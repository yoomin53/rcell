// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 찾기·바꾸기 (찾기·바꾸기 대조 실험 — docs/editing.md 14절).
//!
//! - 찾는 글은 셀의 수식 글(수식은 `=…`, 날짜는 `yyyy-mm-dd`, 시각만이면 `12:30:00 PM`, 그 밖의 숫자는 일반 표기 —
//!   백분율도 값 그대로, 논리값은 `TRUE`)이다. 한셀은 와일드카드(`*` `?` `~`)를 쓰지 않고 글자 그대로 찾는다.
//! - 대소문자 구분을 끄면 대소문자를 무시한다. "셀 전체"는 한셀에서 공백으로 나뉜 낱말 전체가 같은 것이다
//!   (`apple pie` 의 `apple` 은 맞고 `apple-pie`·`"apple"` 은 아니다).
//! - 바꾼 글은 셀에 다시 입력한다 (2절의 입력 규칙 — 글자 접두가 있던 칸은 글로). 빈 글이 되면 내용을 지운다.
//! - 값에서 찾기는 표시된 글에서 찾는다 (바꾸기는 언제나 수식 글).

use crate::model::{CellRange, CellRef, CellValue, Workbook};

/// 찾기 설정
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FindOptions {
    pub what: String,
    pub match_case: bool,
    /// 셀 전체 (한셀: 공백으로 나뉜 낱말 전체)
    pub whole: bool,
    /// 표시된 값에서 찾는다 (아니면 수식 글)
    pub in_values: bool,
    /// 모든 시트
    pub all_sheets: bool,
    /// 이 범위 안에서만 (없으면 시트 전체)
    pub range: Option<CellRange>,
}

/// 셀의 찾는 글 (한셀 COM `Range.Formula` 와 같은 꼴)
pub fn formula_text(wb: &Workbook, si: usize, r: CellRef) -> Option<String> {
    let c = wb.sheets.get(si)?.cell(r)?;
    if let Some(f) = c.formula.as_ref().filter(|f| !f.text.is_empty()) {
        return Some(format!("={}", f.text));
    }
    let code = wb
        .styles
        .xf(c.style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    Some(match &c.value {
        CellValue::Empty => return None,
        CellValue::Number(v) => {
            let v = *v;
            let (cat, dated, timed) = super::input::classify(&code);
            match cat {
                super::input::Cat::Date | super::input::Cat::Time
                    if (0.0..2_958_466.0).contains(&v) =>
                {
                    date_time_text(wb, v, dated, timed)
                }
                _ => crate::model::format_general(v),
            }
        }
        v => v.display(),
    })
}

/// 날짜·시각 값이 한셀의 바꾸기에 보이는 글 (날짜·시각 형식의 칸, 한셀 Replace 실험 270가지): 12시간 `yyyy-mm-dd hh:mm:ss
/// AM/PM` (시는 두 자리, 0시 = `12:00:00 AM`). 초는 천 분의 1초로 반올림한 뒤 반올림해 끊고 (자리 올림은 날짜로),
/// 시각 부분이 있는 형식은 1 미만이면 시각만, 그 밖에는 날짜 + 시각. 날짜만 있는 형식은 날짜(0 = `1900-01-00`)에 소수 부분이
/// 있을 때만 시각을 붙인다
fn date_time_text(wb: &Workbook, v: f64, dated: bool, timed: bool) -> String {
    let frac = v - v.floor();
    let ms = (frac * 86_400_000.0).round() as i64;
    let mut secs = (ms + 500) / 1000;
    let mut day = v.floor();
    if secs >= 86_400 {
        secs -= 86_400;
        day += 1.0;
    }
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    let time = format!(
        "{h12:02}:{m:02}:{s:02} {}",
        if h < 12 { "AM" } else { "PM" }
    );
    let date = || {
        crate::calc::date::ymd_from_serial(day, wb.date1904)
            .map(|(y, mo, d)| format!("{y:04}-{mo:02}-{d:02}"))
    };
    let with_date = |t: &str| match date() {
        Some(d) => format!("{d} {t}"),
        None => crate::model::format_general(v),
    };
    if timed && v < 1.0 {
        return time;
    }
    if timed {
        return with_date(&time);
    }
    debug_assert!(dated);
    if frac == 0.0 {
        date().unwrap_or_else(|| crate::model::format_general(v))
    } else {
        with_date(&time)
    }
}

/// 셀의 표시된 글
fn display_text(wb: &Workbook, si: usize, r: CellRef) -> Option<String> {
    let c = wb.sheets.get(si)?.cell(r)?;
    if c.value.is_empty() {
        return None;
    }
    let code = wb
        .styles
        .xf(c.style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    Some(crate::numfmt::format(&c.value, &code, wb.date1904).text)
}

/// `text` 에서 찾은 자리들 (바이트 위치, 길이)
fn matches(text: &str, o: &FindOptions) -> Vec<(usize, usize)> {
    if o.what.is_empty() {
        return Vec::new();
    }
    let fold = |s: &str| {
        if o.match_case {
            s.to_string()
        } else {
            s.to_lowercase()
        }
    };
    let (hay, needle) = (fold(text), fold(&o.what));
    // 소문자로 바꾸면 길이가 바뀌는 글자는 드물다 — 바뀌면 대소문자를 그대로 비교한다
    let (hay, needle) = if hay.len() == text.len() && needle.len() == o.what.len() {
        (hay, needle)
    } else {
        (text.to_string(), o.what.clone())
    };
    let mut out = Vec::new();
    if o.whole {
        let mut pos = 0;
        for tok in hay.split(' ') {
            if tok == needle {
                out.push((pos, tok.len()));
            }
            pos += tok.len() + 1;
        }
        return out;
    }
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        out.push((from + i, needle.len()));
        from += i + needle.len().max(1);
        if from > hay.len() {
            break;
        }
    }
    out
}

/// 찾을 칸들 (시트마다 셀 기록 차례 — 행 우선)
fn targets(wb: &Workbook, si: usize, o: &FindOptions) -> Vec<(usize, CellRef)> {
    let sheets: Vec<usize> = if o.all_sheets {
        (0..wb.sheets.len()).collect()
    } else {
        vec![si]
    };
    let mut v = Vec::new();
    for s in sheets {
        let Some(sheet) = wb.sheets.get(s) else {
            continue;
        };
        for c in &sheet.cells {
            if o.range.is_none_or(|rg| rg.contains(c.r)) {
                v.push((s, c.r));
            }
        }
    }
    v
}

/// 찾기: 찾은 칸 (시트 번호, 칸) — 시트 차례, 행 우선
pub fn find_all(wb: &Workbook, si: usize, o: &FindOptions) -> Vec<(usize, CellRef)> {
    targets(wb, si, o)
        .into_iter()
        .filter(|&(s, r)| {
            let text = if o.in_values {
                display_text(wb, s, r)
            } else {
                formula_text(wb, s, r)
            };
            text.is_some_and(|t| !matches(&t, o).is_empty())
        })
        .collect()
}

/// 바꾸기: 찾은 글을 `with` 로 바꿔 다시 입력한다. 바꾼 칸 수를 돌려준다 (입력할 수 없는 칸 — 문법이 틀린
/// 수식 … — 은 건너뛴다)
pub fn replace_all(
    wb: &mut Workbook,
    si: usize,
    o: &FindOptions,
    with: &str,
) -> crate::Result<usize> {
    let mut n = 0;
    for (s, r) in targets(wb, si, o) {
        let Some(text) = formula_text(wb, s, r) else {
            continue;
        };
        let found = matches(&text, o);
        if found.is_empty() {
            continue;
        }
        let mut new = String::new();
        let mut last = 0;
        for (i, len) in found {
            new.push_str(&text[last..i]);
            new.push_str(with);
            last = i + len;
        }
        new.push_str(&text[last..]);
        // 글자 접두가 있던 글은 글로 (작은따옴표를 붙여 입력)
        let quoted = wb.sheets[s]
            .cell(r)
            .and_then(|c| wb.styles.xf(c.style))
            .is_some_and(|x| x.quote_prefix);
        let input = if quoted && !new.is_empty() {
            format!("'{new}")
        } else {
            new
        };
        // 백분율 칸에 숫자 글을 넣으면 100 으로 나누지 않는다 (한셀 바꾸기 — 값 그대로)
        let percent = wb.sheets[s].cell(r).is_some_and(|c| {
            c.formula.is_none()
                && wb.styles.xf(c.style).is_some_and(|x| {
                    super::input::category(&crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
                        == super::input::Cat::Percent
                })
        });
        if percent && let Ok(v) = input.trim().parse::<f64>() {
            let c = super::cell_mut(&mut wb.sheets[s], r);
            c.value = CellValue::Number(v);
            n += 1;
            continue;
        }
        if super::set_input(wb, s, r, &input).is_ok() {
            n += 1;
            // 바꿔서 새로 고른 수식은 처음 값이 0 이다 — 순환에 들면 한셀은 그 0 을 그대로 둔다 (바꾸기 대조 실험:
            // `IF(SUM(G9:G11),…,"")` 가 `SUM(G9:G77)` 로 바뀌어 자기 칸을 품으면 0)
            if let Some(c) = wb.sheets[s].cells.iter_mut().find(|c| c.r == r)
                && c.formula.is_some()
            {
                c.value = CellValue::Number(0.0);
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(what: &str, whole: bool, case: bool) -> FindOptions {
        FindOptions {
            what: what.into(),
            whole,
            match_case: case,
            ..Default::default()
        }
    }

    // 한셀 Replace 가 보는 날짜·시각 글 (날짜·시각 형식 칸 대조 실험 160가지에서 고른 것)
    #[test]
    fn date_time_text_like_hcell() {
        let wb = Workbook::default();
        let t = |v: f64, dated: bool, timed: bool| date_time_text(&wb, v, dated, timed);
        // 시각 부분이 있는 형식: 1 미만이면 시각만, 아니면 날짜 + 시각 (0시 = 12:00:00 AM, 시는 두 자리)
        assert_eq!(t(45000.5, true, true), "2023-03-15 12:00:00 PM");
        assert_eq!(t(45000.0, true, true), "2023-03-15 12:00:00 AM");
        assert_eq!(t(1234.5678, true, true), "1903-05-18 01:37:38 PM");
        assert_eq!(t(0.5, true, true), "12:00:00 PM");
        assert_eq!(t(0.04, false, true), "12:57:36 AM");
        assert_eq!(t(0.378_472_222_222_222, false, true), "09:05:00 AM");
        // 초는 반올림, 자리 올림은 날짜로 (시각만이면 날짜 없이)
        assert_eq!(t(0.999_999_99, false, true), "12:00:00 AM");
        assert_eq!(t(45_000.999_999_99, true, true), "2023-03-16 12:00:00 AM");
        assert_eq!(t(45_000.000_005_787, false, true), "2023-03-15 12:00:01 AM");
        // 날짜만 있는 형식: 정수면 날짜만 (0 = 1900-01-00), 소수 부분이 있으면 시각도
        assert_eq!(t(45000.0, true, false), "2023-03-15");
        assert_eq!(t(0.0, true, false), "1900-01-00");
        assert_eq!(t(60.0, true, false), "1900-02-29");
        assert_eq!(t(59.5, true, false), "1900-02-28 12:00:00 PM");
        assert_eq!(
            t(0.378_472_222_222_222, true, false),
            "1900-01-00 09:05:00 AM"
        );
    }

    #[test]
    fn whole_is_a_space_separated_word_like_hcell() {
        let o = opt("apple", true, false);
        assert_eq!(matches("apple pie", &o).len(), 1);
        assert_eq!(matches("an Apple a day", &o).len(), 1);
        assert!(matches("apple-pie", &o).is_empty());
        assert!(matches("applesauce", &o).is_empty());
        assert!(matches("=\"apple\"", &o).is_empty());
        let o = opt("apple", false, true);
        assert!(matches("APPLE", &o).is_empty());
        assert_eq!(matches("pineapple apple", &o).len(), 2);
        // 와일드카드 없이 글자 그대로
        assert!(matches("Apple pie", &opt("*pie", false, false)).is_empty());
        assert_eq!(matches("a?b", &opt("a?b", false, false)).len(), 1);
    }
}

/// 이동 옵션(`Go To Special`)의 칸 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKind {
    /// 상수 (수식이 아닌 값)
    Constants,
    /// 수식
    Formulas,
    /// 쓰는 범위 안의 빈 칸
    Blanks,
    /// 메모가 있는 칸
    Comments,
    /// 오류 값 (상수·수식)
    Errors,
    /// 마지막 칸 (쓰는 범위의 오른쪽 아래)
    LastCell,
}

/// 이동 옵션: 시트에서 그 종류의 칸 (행 우선)
pub fn special_cells(wb: &Workbook, si: usize, kind: SpecialKind) -> Vec<CellRef> {
    let Some(sheet) = wb.sheets.get(si) else {
        return Vec::new();
    };
    let has_value = |c: &crate::model::Cell| !c.value.is_empty() || c.formula.is_some();
    let mut v: Vec<CellRef> = match kind {
        SpecialKind::Constants => sheet
            .cells
            .iter()
            .filter(|c| c.formula.is_none() && !c.value.is_empty())
            .map(|c| c.r)
            .collect(),
        SpecialKind::Formulas => sheet
            .cells
            .iter()
            .filter(|c| c.formula.as_ref().is_some_and(|f| !f.text.is_empty()))
            .map(|c| c.r)
            .collect(),
        SpecialKind::Errors => sheet
            .cells
            .iter()
            .filter(|c| matches!(c.value, CellValue::Error(_)))
            .map(|c| c.r)
            .collect(),
        SpecialKind::Comments => sheet.comments.iter().map(|c| c.cell).collect(),
        SpecialKind::Blanks | SpecialKind::LastCell => {
            let filled: Vec<&crate::model::Cell> =
                sheet.cells.iter().filter(|c| has_value(c)).collect();
            let Some(r1) = filled.iter().map(|c| c.r.row).max() else {
                return Vec::new();
            };
            let c1 = filled.iter().map(|c| c.r.col).max().unwrap_or(0);
            if kind == SpecialKind::LastCell {
                return vec![CellRef { row: r1, col: c1 }];
            }
            let r0 = filled.iter().map(|c| c.r.row).min().unwrap_or(0);
            let c0 = filled.iter().map(|c| c.r.col).min().unwrap_or(0);
            let set: std::collections::HashSet<(u32, u32)> =
                filled.iter().map(|c| (c.r.row, c.r.col)).collect();
            let mut out = Vec::new();
            for r in r0..=r1.min(r0 + 100_000) {
                for c in c0..=c1 {
                    if !set.contains(&(r, c)) {
                        out.push(CellRef { row: r, col: c });
                    }
                }
            }
            out
        }
    };
    v.sort_by_key(|r| (r.row, r.col));
    v
}
