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
            match super::input::category(&code) {
                super::input::Cat::Date if (1.0..2_958_466.0).contains(&v) => {
                    super::input_text(wb, si, r)
                }
                // 시각만: 12시간 표기 (`12:30:00 PM`)
                super::input::Cat::Time if (0.0..1.0).contains(&v) => {
                    let (h, m, s) = crate::calc::date::hms_from_serial(v);
                    let (h12, ap) = match h {
                        0 => (12, "AM"),
                        1..=11 => (h, "AM"),
                        12 => (12, "PM"),
                        _ => (h - 12, "PM"),
                    };
                    format!("{h12}:{m:02}:{s:02} {ap}")
                }
                _ => crate::model::format_general(v),
            }
        }
        v => v.display(),
    })
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
