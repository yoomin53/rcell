// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! `xl/sharedStrings.xml` 파서.

use crate::model::{RichText, TextRun};
use crate::parser::styles::parse_font;
use crate::xml::Element;

pub fn parse_shared_strings(root: &Element) -> Vec<RichText> {
    root.sml_children("si").map(parse_rich_text).collect()
}

/// `x:si` 또는 `x:is`(인라인 문자열). 윗주(`x:rPh`)는 본문에서 제외한다.
pub fn parse_rich_text(e: &Element) -> RichText {
    let mut rt = RichText::default();
    for c in e.elements() {
        if c.is_sml("t") {
            rt.text.push_str(&crate::text::xstring_decode(&c.text()));
        } else if c.is_sml("r") {
            let text: String = c
                .sml_children("t")
                .map(|t| crate::text::xstring_decode(&t.text()).into_owned())
                .collect();
            rt.text.push_str(&text);
            rt.runs.push(TextRun {
                text,
                font: c.sml_child("rPr").map(parse_font),
            });
        }
    }
    rt
}
