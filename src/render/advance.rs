// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀 화면의 글자 폭 (docs/format/screen.md 4절, 글자 폭 실험).
//!
//! 한셀은 글자마다 정수 px 폭으로 글을 늘어놓는다: 폭 = round(설계 폭(em) × ppem), ppem = round(크기 pt × 4/3).
//! 맑은 고딕·Segoe UI·Consolas 는 ASCII 94자 × 6 크기(9–16pt)가 모두 이 식과 같다 (보통·굵게). 한글·한자 등 전각은 1em.
//! 브라우저는 소수 폭으로 늘어놓으므로 (11pt 한글 14.67px — 한셀 15px) 글 한 줄이 한셀보다 짧다 — [`spacing`] 이
//! 글 덩이마다 자간을 보태 길이를 맞춘다.

use super::advance_data::{FONT_EMS, FontEm};

/// 글꼴 이름의 폭 표 (없으면 None — 표에 없는 글꼴은 보정하지 않는다)
pub fn table(name: Option<&str>) -> Option<&'static FontEm> {
    let n = name.unwrap_or(super::metrics::DEFAULT_FONT).trim();
    FONT_EMS
        .iter()
        .find(|t| t.names.iter().any(|x| x.eq_ignore_ascii_case(n)))
}

/// 크기(pt) → ppem
pub fn ppem(size_pt: f64) -> f64 {
    (size_pt * 4.0 / 3.0).round().max(1.0)
}

/// 글자 하나의 설계 폭 (em): ASCII 는 표, 전각은 1. 그 밖은 None
pub fn char_em(t: &FontEm, bold: bool, c: char) -> Option<f64> {
    let u = c as u32;
    if (0x20..=0x7E).contains(&u) {
        let tab = if bold { t.bold } else { t.regular };
        return Some(f64::from(tab[(u - 0x20) as usize]) / 10000.0);
    }
    super::metrics::is_wide(c).then_some(1.0)
}

/// 한셀의 글자 폭 (px, 정수). 표에 없는 글자는 None
pub fn hcell_px(t: &FontEm, bold: bool, size_pt: f64, c: char) -> Option<f64> {
    char_em(t, bold, c).map(|em| (em * ppem(size_pt)).round())
}

/// 한 글 덩이의 자간 보정 (px): 한셀의 정수 폭 합과 브라우저 소수 폭 합의 차 / 글자 수.
/// 표에 없는 글꼴이거나 보정할 글자가 없으면 None
pub fn spacing(name: Option<&str>, size_pt: f64, bold: bool, text: &str) -> Option<f64> {
    let t = table(name)?;
    let px = size_pt * 96.0 / 72.0;
    let (mut diff, mut n) = (0.0, 0usize);
    for c in text.chars().filter(|c| *c != '\n' && *c != '\r') {
        n += 1;
        if let Some(em) = char_em(t, bold, c) {
            diff += (em * ppem(size_pt)).round() - em * px;
        }
    }
    (n > 0 && diff.abs() > 1e-9).then_some(diff / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hancell_integer_advances() {
        // 맑은 고딕 11pt (ppem 15): 숫자 8, a 8, i 4, W 14, 한글 15 (한셀 화면 — 글자 폭 실험)
        let t = table(Some("맑은 고딕")).unwrap();
        let w = |c| hcell_px(t, false, 11.0, c).unwrap();
        assert_eq!(
            (w('0'), w('a'), w('i'), w('W'), w('한')),
            (8.0, 8.0, 4.0, 14.0, 15.0)
        );
        // 9pt (ppem 12): 0 → 7, a → 6
        let w9 = |c| hcell_px(t, false, 9.0, c).unwrap();
        assert_eq!((w9('0'), w9('a'), w9('W')), (7.0, 6.0, 11.0));
        assert!(table(Some("없는 글꼴")).is_none());
    }

    #[test]
    fn spacing_matches_total_width() {
        // 한글 12자 11pt: 한셀 15 × 12 = 180, 브라우저 14.667 × 12 = 176 → 글자마다 +1/3px
        let s = spacing(Some("맑은 고딕"), 11.0, false, "가나다라마바사아자차카타").unwrap();
        assert!((s - 1.0 / 3.0).abs() < 1e-9);
    }
}
