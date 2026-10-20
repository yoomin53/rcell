// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 메모: `xl/commentsN.xml` (작성자·내용) + VML 그리기 (표시 여부).
//!
//! 한셀도 Excel 과 같은 구조로 저장한다 (관측). VML 도형의 `x:ClientData ObjectType="Note"` 가
//! `x:Row`/`x:Column` 으로 메모 셀을 가리키고, `x:Visible` 이 있으면 항상 표시다.

use crate::model::{CellRange, CellRef, Comment};
use crate::ns;
use crate::parser::shared_strings::parse_rich_text;
use crate::xml::Element;

/// `x:comments` → 메모 목록 (표시 여부는 [`apply_vml`] 이 채운다)
pub fn parse_comments(root: &Element) -> Vec<Comment> {
    let authors: Vec<String> = root
        .sml_child("authors")
        .map(|a| a.sml_children("author").map(|e| e.text()).collect())
        .unwrap_or_default();
    let Some(list) = root.sml_child("commentList") else {
        return Vec::new();
    };
    list.sml_children("comment")
        .filter_map(|c| {
            // 한셀은 ref 를 "K8:K8" 처럼 범위로 쓴다
            let cell = c.attr("ref").and_then(|r| {
                CellRange::parse(r)
                    .map(|rg| rg.start)
                    .or_else(|| CellRef::parse(r))
            })?;
            Some(Comment {
                cell,
                author: c
                    .attr_u32("authorId")
                    .and_then(|i| authors.get(i as usize))
                    .cloned()
                    .unwrap_or_default(),
                text: c.sml_child("text").map(parse_rich_text).unwrap_or_default(),
                visible: false,
            })
        })
        .collect()
}

/// VML 의 메모 도형에서 표시 여부를 읽어 같은 셀의 메모에 넣는다.
pub fn apply_vml(vml: &Element, comments: &mut [Comment]) {
    vml.walk(&mut |e| {
        if !e.is(ns::VML, "shape") {
            return;
        }
        let Some(cd) = e.child_ns(ns::VML_EXCEL, "ClientData") else {
            return;
        };
        if cd.attr("ObjectType") != Some("Note") {
            return;
        }
        let num = |name: &str| {
            cd.child_ns(ns::VML_EXCEL, name)
                .and_then(|x| x.text().trim().parse::<u32>().ok())
        };
        let (Some(row), Some(col)) = (num("Row"), num("Column")) else {
            return;
        };
        let style_visible = e
            .attr("style")
            .is_some_and(|s| s.replace(' ', "").contains("visibility:visible"));
        let visible = cd.child_ns(ns::VML_EXCEL, "Visible").is_some() || style_visible;
        if let Some(c) = comments.iter_mut().find(|c| c.cell == CellRef { row, col }) {
            c.visible = visible;
        }
    });
}
