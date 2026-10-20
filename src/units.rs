// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 화면 단위 변환 (96 dpi 픽셀, EMU, 포인트).
//!
//! 열 너비는 "글자 수" 로 저장되고 화면 픽셀은 기본 글꼴의 숫자 폭(MDW)으로 정해진다
//! (ECMA-376 18.3.1.13). 한셀도 같은 식을 쓴다 (레거시 그리기 앵커와 대조해 확인, docs/format/cell-legacy.md).

use crate::model::{Sheet, Workbook};

/// 1 픽셀(96 dpi) = 9525 EMU
pub const EMU_PER_PX: i64 = 9525;

/// 기본 글꼴의 숫자 폭 (px). 글꼴 크기로 어림한다 (10pt → 7, 11pt → 8).
pub fn max_digit_width(wb: &Workbook) -> u32 {
    let size = wb.styles.fonts.first().and_then(|f| f.size).unwrap_or(11.0);
    ((size * 96.0 / 72.0 * 0.55).round() as u32).max(5)
}

/// 열 너비(글자 수) → 픽셀: `trunc(((trunc(256·w) + trunc(128/MDW)) / 256) · MDW)`
pub fn col_width_px(width: f64, mdw: u32) -> u32 {
    let m = mdw as f64;
    ((((256.0 * width).trunc() + (128.0 / m).trunc()) / 256.0) * m).trunc() as u32
}

/// 행 높이(pt) → 픽셀
pub fn row_height_px(pt: f64) -> u32 {
    (pt * 4.0 / 3.0).round() as u32
}

/// 시트의 열 하나의 픽셀 너비 (0부터 시작하는 열 번호, 숨긴 열은 0)
pub fn sheet_col_px(sheet: &Sheet, col: u32, mdw: u32) -> u32 {
    match sheet
        .cols
        .iter()
        .find(|c| (c.min..=c.max).contains(&(col + 1)))
    {
        Some(c) if c.hidden => 0,
        Some(c) if c.width.is_some() => col_width_px(c.width.unwrap_or(0.0), mdw),
        _ => default_col_px(sheet, mdw),
    }
}

/// 기본 열 너비 (px). 레거시 DEFCOLWIDTH 처럼 정수 글자 수면 Excel 규칙(여백 5px, 8px 배수)으로.
pub fn default_col_px(sheet: &Sheet, mdw: u32) -> u32 {
    let w = sheet.default_col_width.unwrap_or(8.0);
    if w.fract() == 0.0 {
        let px = w * mdw as f64 + 5.0;
        ((px / 8.0).ceil() * 8.0) as u32
    } else {
        col_width_px(w, mdw)
    }
}

/// 시트의 행 하나의 픽셀 높이 (숨긴 행은 0)
pub fn sheet_row_px(sheet: &Sheet, row: u32) -> u32 {
    match sheet.rows.iter().find(|r| r.index == row + 1) {
        Some(r) if r.hidden => 0,
        Some(r) if r.height.is_some() => row_height_px(r.height.unwrap_or(0.0)),
        _ => row_height_px(sheet.default_row_height.unwrap_or(15.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_column_pixels() {
        // 한셀 레거시 그리기 앵커와 대조한 값 (MDW 7)
        for (w, px) in [
            (90.0, 630),
            (4.0, 28),
            (12.7109375, 89),
            (0.85546875, 6),
            (25.7109375, 180),
            (9.140625, 64),
        ] {
            assert_eq!(col_width_px(w, 7), px, "{w}");
        }
        assert_eq!(row_height_px(12.75), 17);
        assert_eq!(row_height_px(13.2), 18);
    }
}
