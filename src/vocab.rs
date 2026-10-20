// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한컴 확장(`hs:`) 열거형 값 어휘.
//!
//! 값 이름과 순서는 한셀 2024 의 `HncOOXML.dll` 에 있는 열거형 변환표
//! (`{이름, 정수}` 쌍 배열)에서 확인한 사실이다. 배열의 위치가 한셀 내부 정수값이다.
//! 자세한 근거: `docs/format/cell-2024.md` 8절.

/// `hs:lineSpacingType` (`ST_LineSpacing`) — 줄 간격 종류
pub const LINE_SPACING_TYPE: &[&str] = &["percent", "fixed", "betweenLines", "atLeast"];

/// `hs:breakLatinWord`, `hs:breakNonLatinWord` (`ST_LineBreak`) — 단어 나눔
pub const LINE_BREAK: &[&str] = &["keepWord", "breakWord"];

/// `hs:textAlignment` (`ST_TextAlignment`) — 세로 정렬 기준
pub const TEXT_ALIGNMENT: &[&str] = &["baseLine", "top", "center", "bottom"];

/// `hs:underlineShape`, `hs:strikeoutShape` (`ST_UnderlineShape`) — 선 모양
pub const LINE_SHAPE: &[&str] = &[
    "solid",
    "dash",
    "dot",
    "dashdot",
    "dashdotdot",
    "longdash",
    "circle",
    "doubleslim",
    "slimthick",
    "thickslim",
    "slimthickslim",
];

/// `hs:shadowType` (`ST_ShadowType`) — 그림자 (없음 / 비연속 / 연속)
pub const SHADOW_TYPE: &[&str] = &["none", "drop", "continuous"];

/// `hs:outlineType` (`ST_OutlineType`) — 외곽선. `dashdotdt` 는 한셀 원문 철자 그대로다.
pub const OUTLINE_TYPE: &[&str] = &[
    "none",
    "solid",
    "dot",
    "thick",
    "dash",
    "dashdot",
    "dashdotdt",
];

/// `hs:diacsymMark` (`ST_DiacsymMark`) — 강조점
pub const DIACSYM_MARK: &[&str] = &[
    "none",
    "dot_above",
    "ring_above",
    "tilde",
    "caron",
    "sideout",
    "colon",
    "grave_accent",
    "acute_accent",
    "circumflex_accent",
    "macron",
    "hook_above",
    "dot_below",
];

/// `hs:accountUnderline/@shape` (`ST_UnderlineValues`) — 회계용 밑줄 모양
pub const UNDERLINE_VALUES: &[&str] = &[
    "single",
    "double",
    "singleAccounting",
    "doubleAccounting",
    "none",
];

/// `부모/요소-또는-@속성` 경로에 대응하는 값 어휘. 정수형 속성은 `None`.
pub fn for_path(path: &str) -> Option<&'static [&'static str]> {
    Some(match path {
        "parashape/@lineSpacingType" => LINE_SPACING_TYPE,
        "parashape/@breakLatinWord" | "parashape/@breakNonLatinWord" => LINE_BREAK,
        "parashape/@textAlignment" => TEXT_ALIGNMENT,
        "font/underlineShape"
        | "font/strikeoutShape"
        | "rPr/underlineShape"
        | "rPr/strikeoutShape" => LINE_SHAPE,
        "font/shadowType" | "rPr/shadowType" => SHADOW_TYPE,
        "font/outlineType" | "rPr/outlineType" => OUTLINE_TYPE,
        "font/diacsymMark" | "rPr/diacsymMark" => DIACSYM_MARK,
        "accountUnderline/@shape" => UNDERLINE_VALUES,
        _ => return None,
    })
}

/// 열거 문자열의 한셀 내부 정수값 (배열 위치).
pub fn ordinal(vocab: &[&str], value: &str) -> Option<usize> {
    vocab.iter().position(|v| *v == value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_values_belong_to_vocabularies() {
        // 한셀이 저장한 파일에서 관측한 값
        assert_eq!(ordinal(LINE_SPACING_TYPE, "betweenLines"), Some(2));
        assert_eq!(ordinal(LINE_BREAK, "keepWord"), Some(0));
        assert_eq!(ordinal(TEXT_ALIGNMENT, "baseLine"), Some(0));
        assert_eq!(ordinal(LINE_SHAPE, "doubleslim"), Some(7));
        assert_eq!(ordinal(SHADOW_TYPE, "drop"), Some(1));
        assert_eq!(ordinal(OUTLINE_TYPE, "solid"), Some(1));
        assert_eq!(for_path("font/shadowType"), Some(SHADOW_TYPE));
        assert_eq!(for_path("font/ratio"), None);
    }
}
