// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀 화면의 글자 폭 (docs/format/screen.md 4절, 글자 폭 실험).
//!
//! 한셀은 글자마다 정수 px 폭으로 글을 늘어놓는다. 맑은 고딕·Segoe UI·Consolas 는 폭 = round(설계 폭(em) × ppem),
//! ppem = round(크기 pt × 4/3) 가 ASCII 94자 × 6 크기(9–16pt) 모두와 같다 (보통·굵게). 한글·한자 등 전각은 1em.
//! Calibri·Arial·Times New Roman 은 힌트 폭이 섞여 있어 크기마다 잰 표(`advance_hint.rs`)를 쓴다. 굴림·돋움·바탕은
//! 브라우저 폭이 한셀 폭과 같다.
//! 브라우저는 소수 폭으로 늘어놓으므로 (11pt 한글 14.67px — 한셀 15px) 글 한 줄이 한셀보다 짧다 — [`segments`] 가
//! 글자마다 자간을 붙여 정수 자리에 놓는다.

use super::advance_data::{FONT_EMS, FontEm};
use super::advance_hint::{HINTED, Hinted};

fn norm(name: Option<&str>) -> &str {
    name.unwrap_or(super::metrics::DEFAULT_FONT).trim()
}

/// 글꼴 이름의 설계 폭 표 (없으면 None)
pub fn table(name: Option<&str>) -> Option<&'static FontEm> {
    let n = norm(name);
    FONT_EMS
        .iter()
        .find(|t| t.names.iter().any(|x| x.eq_ignore_ascii_case(n)))
}

fn hinted(name: Option<&str>) -> Option<&'static Hinted> {
    let n = norm(name);
    HINTED
        .iter()
        .find(|t| t.names.iter().any(|x| x.eq_ignore_ascii_case(n)))
}

/// 크기(pt) → ppem
pub fn ppem(size_pt: f64) -> f64 {
    (size_pt * 4.0 / 3.0).round().max(1.0)
}

/// 설계 폭 식이 한셀 화면과 모두 같은 글꼴인가 (맑은 고딕·Segoe UI·Consolas)
pub fn is_exact(name: Option<&str>) -> bool {
    table(name).is_some_and(|t| {
        t.names
            .iter()
            .any(|n| matches!(*n, "맑은 고딕" | "Segoe UI" | "Consolas"))
    })
}

/// 글자 하나의 설계 폭 (em): ASCII 는 표, 전각은 1. 그 밖은 None
fn char_em(t: &FontEm, bold: bool, c: char) -> Option<f64> {
    let u = c as u32;
    if (0x20..=0x7E).contains(&u) {
        let tab = if bold { t.bold } else { t.regular };
        return Some(f64::from(tab[(u - 0x20) as usize]) / 10000.0);
    }
    super::metrics::is_wide(c).then_some(1.0)
}

/// 힌트 표의 칸 번호: ASCII 는 0x20.., 전각은 95('한')
fn hint_slot(c: char) -> Option<usize> {
    let u = c as u32;
    if (0x20..=0x7E).contains(&u) {
        Some((u - 0x20) as usize)
    } else {
        super::metrics::is_wide(c).then_some(95)
    }
}

/// 글꼴 하나의 폭 표 묶음 (설계 폭 식, 크기별 힌트 표)
#[derive(Debug, Clone, Copy)]
pub struct AdvFont {
    em: Option<&'static FontEm>,
    hint: Option<&'static Hinted>,
}

/// 글꼴 이름의 폭 표 묶음. 둘 다 없으면 None
pub fn font(name: Option<&str>) -> Option<AdvFont> {
    let (em, hint) = (table(name), hinted(name));
    (em.is_some() || hint.is_some()).then_some(AdvFont { em, hint })
}

impl AdvFont {
    /// 글자 하나의 (한셀 폭, 브라우저 폭) px. 표에 없으면 None
    pub fn widths(&self, size_pt: f64, bold: bool, c: char) -> Option<(f64, f64)> {
        let px = size_pt * 96.0 / 72.0;
        // 힌트 폭 표: 크기가 표에 있을 때
        if let Some(h) = self.hint
            && size_pt.fract() == 0.0
            && let Some(i) = h.sizes.iter().position(|&s| f64::from(s) == size_pt)
            && let Some(slot) = hint_slot(c)
        {
            let hp = f64::from(if bold { h.bold } else { h.regular }[i][slot]);
            let nat = if bold { h.nat_bold } else { h.nat_regular };
            let np = nat.get(i).map_or(hp, |row| f64::from(row[slot]) / 100.0);
            return Some((hp, np));
        }
        // 설계 폭 식
        let em = char_em(self.em?, bold, c)?;
        Some(((em * ppem(size_pt)).round(), em * px))
    }
}

/// 글자 하나의 (한셀 폭, 브라우저 폭) px. 표에 없으면 None
pub fn widths(name: Option<&str>, size_pt: f64, bold: bool, c: char) -> Option<(f64, f64)> {
    font(name)?.widths(size_pt, bold, c)
}

/// 한셀의 글자 폭 (px, 정수). 표에 없는 글자는 None
#[cfg(test)]
pub fn hcell_px(name: Option<&str>, size_pt: f64, bold: bool, c: char) -> Option<f64> {
    widths(name, size_pt, bold, c).map(|w| w.0)
}

/// 글 덩이를 글자 폭이 정수가 되도록 자간을 붙인 조각들로 나눈다: (자간 px, 글) 목록. 같은 자간은 한 조각으로 묶는다.
/// 보정할 것이 없으면(표에 없는 글꼴, 브라우저 폭이 한셀 폭과 같은 글꼴) None
pub fn segments(
    name: Option<&str>,
    size_pt: f64,
    bold: bool,
    text: &str,
) -> Option<Vec<(f64, String)>> {
    let mut out: Vec<(i32, String)> = Vec::new();
    let mut any = false;
    for c in text.chars() {
        let q =
            widths(name, size_pt, bold, c).map_or(0, |(h, n)| ((h - n) * 1000.0).round() as i32);
        any |= q != 0;
        match out.last_mut() {
            Some((lq, buf)) if *lq == q => buf.push(c),
            _ => out.push((q, c.to_string())),
        }
    }
    any.then(|| {
        out.into_iter()
            .map(|(q, s)| (f64::from(q) / 1000.0, s))
            .collect()
    })
}

/// 한 글 덩이의 평균 자간 보정 (px) — 차트 글자(SVG `letter-spacing`)용. 보정할 것이 없으면 None
pub fn spacing(name: Option<&str>, size_pt: f64, bold: bool, text: &str) -> Option<f64> {
    let (mut diff, mut n) = (0.0, 0usize);
    for c in text.chars().filter(|c| *c != '\n' && *c != '\r') {
        n += 1;
        if let Some((h, nat)) = widths(name, size_pt, bold, c) {
            diff += h - nat;
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
        let w = |c| hcell_px(Some("맑은 고딕"), 11.0, false, c).unwrap();
        assert_eq!(
            (w('0'), w('a'), w('i'), w('W'), w('한')),
            (8.0, 8.0, 4.0, 14.0, 15.0)
        );
        // 9pt (ppem 12): 0 → 7, a → 6
        let w9 = |c| hcell_px(Some("맑은 고딕"), 9.0, false, c).unwrap();
        assert_eq!((w9('0'), w9('a'), w9('W')), (7.0, 6.0, 11.0));
        assert!(hcell_px(Some("없는 글꼴"), 11.0, false, 'a').is_none());
    }

    #[test]
    fn spacing_matches_total_width() {
        // 한글 12자 11pt: 한셀 15 × 12 = 180, 브라우저 14.667 × 12 = 176 → 글자마다 +1/3px
        let s = spacing(Some("맑은 고딕"), 11.0, false, "가나다라마바사아자차카타").unwrap();
        assert!((s - 1.0 / 3.0).abs() < 1e-9);
    }
}
