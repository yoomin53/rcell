// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 메모: `xl/commentsN.xml` (작성자·내용) + VML 그리기 (표시 여부).
//!
//! 한셀도 Excel 과 같은 구조로 저장한다 (관측). VML 도형의 `x:ClientData ObjectType="Note"` 가
//! `x:Row`/`x:Column` 으로 메모 셀을 가리키고, `x:Visible` 이 있으면 항상 표시다.

use crate::model::{CellRange, CellRef, Comment, CommentShape};
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
                ..Default::default()
            })
        })
        .collect()
}

/// VML 길이 (`0.75pt`, `2.00pt`, `1px` …) → pt. 단위가 없으면 pt 로 본다
fn vml_pt(v: &str) -> Option<f64> {
    let v = v.trim();
    let (num, unit) = v.split_at(v.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(v.len()));
    let n: f64 = num.trim().parse().ok()?;
    Some(match unit {
        "px" => n * 0.75,
        "in" => n * 72.0,
        "mm" => n * 72.0 / 25.4,
        "cm" => n * 72.0 / 2.54,
        "emu" => n / 12700.0,
        _ => n,
    })
}

/// VML 의 메모 도형에서 표시 여부와 상자(자리·채우기·선·그림자·셀 고정·글 맞춤), 도형 이름(`id`)과
/// 그리기 차례를 읽어 같은 셀의 메모에 넣는다.
pub fn apply_vml(vml: &Element, comments: &mut [Comment]) {
    let mut order = 0u32;
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
        let child = |name: &str| cd.child_ns(ns::VML_EXCEL, name);
        let num = |name: &str| child(name).and_then(|x| x.text().trim().parse::<u32>().ok());
        let (Some(row), Some(col)) = (num("Row"), num("Column")) else {
            return;
        };
        let style_visible = e
            .attr("style")
            .is_some_and(|s| s.replace(' ', "").contains("visibility:visible"));
        let visible = child("Visible").is_some() || style_visible;
        let anchor: Vec<u32> = child("Anchor")
            .map(|a| {
                a.text()
                    .split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        // 색이 없으면 VML 기본값 (채우기 흰색, 선 검정). 두께가 없으면 한셀처럼 0.75 pt — 한셀은 1 pt(VML 기본값) 선을
        // 쓰지 않고 다시 읽을 때 0.75 pt 로 본다 (메모 대조 실험)
        let shape = <[u32; 8]>::try_from(anchor)
            .ok()
            .map(|anchor| CommentShape {
                anchor,
                fill_color: e
                    .attr("fillcolor")
                    .unwrap_or("#ffffff")
                    .to_ascii_lowercase(),
                stroke_color: e
                    .attr("strokecolor")
                    .unwrap_or("#000000")
                    .to_ascii_lowercase(),
                stroke_weight: e.attr("strokeweight").and_then(vml_pt).unwrap_or(0.75),
                shadow: e
                    .child_ns(ns::VML, "shadow")
                    .is_some_and(|s| s.attr("on").is_some_and(|v| matches!(v, "t" | "true"))),
                move_with_cells: child("MoveWithCells").is_none(),
                size_with_cells: child("SizeWithCells").is_none(),
                text_halign: child("TextHAlign")
                    .map(|x| x.text().trim().to_string())
                    .filter(|v| v != "Left"),
                text_valign: child("TextVAlign")
                    .map(|x| x.text().trim().to_string())
                    .filter(|v| v != "Top"),
            });
        if let Some(c) = comments.iter_mut().find(|c| c.cell == CellRef { row, col }) {
            c.visible = visible;
            c.shape = shape;
            c.shape_name = e.attr("id").map(str::to_string);
            c.z_order = Some(order);
        }
        order += 1;
    });
}
