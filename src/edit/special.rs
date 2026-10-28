// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 선택하여 붙이기 (선택하여 붙이기 대조 실험 — docs/editing.md 16절).
//!
//! - 값: 수식은 계산한 값으로, 빈 칸은 대상을 지운다. 대상 서식은 그대로.
//! - 서식: 서식(조건부 서식·병합 포함 — 붙이기 규칙)만, 대상 값은 그대로.
//! - 수식: 수식(상대 참조는 복사처럼)과 값, 서식은 그대로. 값·숫자 서식 / 수식·숫자 서식은 표시 형식도.
//! - 테두리만 빼고: 붙이기와 같고 대상의 테두리를 남긴다.
//! - 행/열 바꿈: 칸을 뒤집고 수식의 참조도 행·열 차이를 바꾼다 (`A2` 의 `=A1*2` 를 F1 에 붙이면 `=E1*2`, 절대
//!   참조도 — `C2` 의 `=$A$1+1` 을 F3 에 붙이면 `=$E$1+1`).
//! - 빈 셀 무시: 원본의 빈 칸은 대상을 바꾸지 않는다.
//! - 연산(더하기·빼기·곱하기·나누기): 대상 값 ∘ 원본 값. 원본 빈 칸은 0, 원본 글·논리값은 건너뛴다. 대상이나 원본이
//!   수식이면 수식으로 `(대상)∘(원본)` (상수 대상은 괄호 없이). 0 으로 나누면 `#DIV/0!`.
//! - 붙인 범위를 선택한다.

use super::clip::{self, Clip, ClipCell};
use super::err;
use super::refs::{self, Area, Ax, Body};
use crate::model::{CellRange, CellRef, CellValue, Formula, Workbook};

/// 붙일 것 (COM `XlPasteType`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteWhat {
    #[default]
    All,
    Values,
    Formats,
    Formulas,
    AllExceptBorders,
    FormulasAndNumberFormats,
    ValuesAndNumberFormats,
}

/// 연산 (COM `XlPasteSpecialOperation`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PasteOp {
    #[default]
    None,
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// 선택하여 붙이기 설정
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PasteOptions {
    pub what: PasteWhat,
    pub op: PasteOp,
    pub skip_blanks: bool,
    pub transpose: bool,
}

/// 행·열을 바꾼 수식: 상대 참조의 행·열 차이를 바꾼다 (`from` 칸에 있던 수식을 `to` 칸으로)
fn transpose_formula(f: &str, from: CellRef, to: CellRef) -> String {
    // 절대 참조도 같은 규칙으로 옮긴다 (한셀 — `$A$1` 도 바뀐다), `$` 표시는 그대로
    let fix = |row: Ax, col: Ax| -> Option<(Ax, Ax)> {
        let dr = row.n as i64 - from.row as i64;
        let dc = col.n as i64 - from.col as i64;
        let (r, c) = (to.row as i64 + dc, to.col as i64 + dr);
        if r < 0 || c < 0 || r >= refs::MAX_ROWS as i64 || c >= refs::MAX_COLS as i64 {
            return None;
        }
        Some((
            Ax {
                n: r as u32,
                abs: row.abs,
            },
            Ax {
                n: c as u32,
                abs: col.abs,
            },
        ))
    };
    refs::map_refs(f, |r| {
        if r.prefix.as_ref().is_some_and(|p| p.book.is_some()) {
            return None;
        }
        let Body::Area(a) = r.body else { return None };
        let new = match a {
            Area::Cell { col, row } => match fix(row, col) {
                Some((row, col)) => Area::Cell { col, row },
                None => return Some(format!("{}#REF!", r.prefix_raw)),
            },
            Area::Rect { c0, r0, c1, r1 } => {
                let (Some((ra, ca)), Some((rb, cb))) = (fix(r0, c0), fix(r1, c1)) else {
                    return Some(format!("{}#REF!", r.prefix_raw));
                };
                let (r0, r1) = if ra.n <= rb.n { (ra, rb) } else { (rb, ra) };
                let (c0, c1) = if ca.n <= cb.n { (ca, cb) } else { (cb, ca) };
                Area::Rect { c0, r0, c1, r1 }
            }
            _ => return None,
        };
        Some(format!("{}{}", r.prefix_raw, new.text()))
    })
}

/// 복사한 것을 행·열을 바꾼 것으로
fn transpose_clip(c: &Clip, at: CellRef) -> Clip {
    let mut t = Clip {
        sheet: c.sheet,
        origin: at,
        rows: c.cols,
        cols: c.rows,
        ..Default::default()
    };
    for x in &c.cells {
        let from = CellRef {
            row: c.origin.row + x.dr,
            col: c.origin.col + x.dc,
        };
        // 붙일 자리에 맞춘 수식 (붙이기는 원점을 붙일 자리로 두어 다시 옮기지 않는다)
        let to = CellRef {
            row: at.row + x.dc,
            col: at.col + x.dr,
        };
        t.cells.push(ClipCell {
            dr: x.dc,
            dc: x.dr,
            value: x.value.clone(),
            formula: x.formula.as_ref().map(|f| transpose_formula(f, from, to)),
            array: None,
            style: x.style,
        });
    }
    t.cells.sort_by_key(|x| (x.dr, x.dc));
    t.merges = c
        .merges
        .iter()
        .map(|&(a, b, cc, d)| (b, a, d, cc))
        .collect();
    t.comments = c
        .comments
        .iter()
        .map(|m| clip::ClipComment {
            dr: m.dc,
            dc: m.dr,
            comment: m.comment.clone(),
        })
        .collect();
    t
}

/// 숫자 글 (수식에 넣을 상수)
fn num_text(v: f64) -> String {
    crate::model::format_general(v)
}

/// 선택하여 붙이기: `src` 를 `to`(왼쪽 위 칸 또는 채울 범위)에 붙인다. 붙인 범위를 돌려준다
pub fn paste_special(
    wb: &mut Workbook,
    si: usize,
    src: CellRange,
    dsi: usize,
    to: CellRange,
    o: &PasteOptions,
) -> crate::Result<CellRange> {
    if si >= wb.sheets.len() || dsi >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    crate::calc::recalculate(wb);
    let mut c = clip::copy(wb, si, src)?;
    if o.transpose {
        c = transpose_clip(&c, to.start);
    }
    // 서식만 붙이면 메모는 가지 않는다
    if o.what == PasteWhat::Formats {
        c.comments.clear();
    }
    // 붙일 크기 (채울 범위가 배수면 되풀이)
    let (h, w) = (to.end.row - to.start.row + 1, to.end.col - to.start.col + 1);
    let (reps_r, reps_c) = if h % c.rows == 0 && w % c.cols == 0 && (h > 1 || w > 1) {
        (h / c.rows, w / c.cols)
    } else {
        (1, 1)
    };
    let at = to.start;
    let area = CellRange {
        start: at,
        end: CellRef {
            row: at.row + c.rows * reps_r - 1,
            col: at.col + c.cols * reps_c - 1,
        },
    };
    if area.end.row >= refs::MAX_ROWS || area.end.col >= refs::MAX_COLS {
        return Err(err("붙여 넣을 범위가 시트를 넘습니다"));
    }
    super::guard_cells(wb, dsi, area)?;
    // 원본 칸 (붙일 자리마다)
    let mut src_at: std::collections::HashMap<CellRef, (ClipCell, CellRef)> = Default::default();
    for i in 0..reps_r {
        for j in 0..reps_c {
            let base = CellRef {
                row: at.row + i * c.rows,
                col: at.col + j * c.cols,
            };
            for x in &c.cells {
                let r = CellRef {
                    row: base.row + x.dr,
                    col: base.col + x.dc,
                };
                src_at.insert(r, (x.clone(), base));
            }
        }
    }
    // 대상의 지금 칸
    let before: Vec<(CellRef, Option<crate::model::Cell>)> = {
        let sheet = &wb.sheets[dsi];
        let mut v = Vec::new();
        for r in area.start.row..=area.end.row {
            for col in area.start.col..=area.end.col {
                let p = CellRef { row: r, col };
                v.push((p, sheet.cell(p).cloned()));
            }
        }
        v
    };
    let style_before: std::collections::HashMap<CellRef, u32> = before
        .iter()
        .map(|(p, _)| (*p, super::style_at(&wb.sheets[dsi], *p)))
        .collect();
    let formats = matches!(
        o.what,
        PasteWhat::All | PasteWhat::AllExceptBorders | PasteWhat::Formats
    );
    if formats {
        clip::paste_with(wb, dsi, at, &c, Some(area), false)?;
    }
    let with_formulas = matches!(
        o.what,
        PasteWhat::All
            | PasteWhat::AllExceptBorders
            | PasteWhat::Formulas
            | PasteWhat::FormulasAndNumberFormats
    );
    let with_numfmt = matches!(
        o.what,
        PasteWhat::FormulasAndNumberFormats | PasteWhat::ValuesAndNumberFormats
    );
    for (p, old) in &before {
        let sheet = &mut wb.sheets[dsi];
        let src = src_at.get(p);
        let blank = src.is_none_or(|(x, _)| x.value.is_empty() && x.formula.is_none());
        // 빈 셀 무시: 대상 그대로
        if o.skip_blanks && blank {
            restore(sheet, *p, old.clone());
            continue;
        }
        // 원본 수식 (붙인 자리에 맞게)
        let src_formula = src.and_then(|(x, base)| {
            let f = x.formula.as_ref()?;
            let from = CellRef {
                row: c.origin.row + x.dr,
                col: c.origin.col + x.dc,
            };
            let dst = CellRef {
                row: base.row + x.dr,
                col: base.col + x.dc,
            };
            Some(crate::formula_text::shift_references(
                f,
                dst.row as i64 - from.row as i64,
                dst.col as i64 - from.col as i64,
            ))
        });
        let src_value = src.map(|(x, _)| x.value.clone()).unwrap_or_default();
        if o.what == PasteWhat::Formats {
            // 값·수식은 붙이기 전으로
            let style = super::style_at(sheet, *p);
            restore(sheet, *p, old.clone());
            restyle(sheet, *p, style);
            continue;
        }
        // 값·수식
        let (value, formula) = if o.op != PasteOp::None {
            let old_v = old.as_ref().map(|x| x.value.clone()).unwrap_or_default();
            let old_f = old
                .as_ref()
                .and_then(|x| x.formula.as_ref())
                .map(|f| f.text.clone());
            let s_num = match (&src_value, &src_formula) {
                (_, Some(_)) if with_formulas => None,
                (CellValue::Number(v), _) => Some(*v),
                (CellValue::Empty, None) => Some(0.0),
                _ => {
                    // 원본 글·논리값: 대상 그대로
                    let st = super::style_at(sheet, *p);
                    restore(sheet, *p, old.clone());
                    restyle(sheet, *p, st);
                    continue;
                }
            };
            let sym = match o.op {
                PasteOp::Add => "+",
                PasteOp::Subtract => "-",
                PasteOp::Multiply => "*",
                _ => "/",
            };
            let calc = |a: f64, b: f64| match o.op {
                PasteOp::Add => CellValue::Number(a + b),
                PasteOp::Subtract => CellValue::Number(a - b),
                PasteOp::Multiply => CellValue::Number(a * b),
                _ if b == 0.0 => CellValue::Error("#DIV/0!".into()),
                _ => CellValue::Number(a / b),
            };
            let rhs = match (s_num, &src_formula) {
                (Some(v), _) => num_text(v),
                (None, Some(f)) => format!("({f})"),
                _ => String::new(),
            };
            match (&old_f, &old_v, s_num) {
                (Some(f), _, _) => (CellValue::Empty, Some(format!("({f}){sym}{rhs}"))),
                (None, CellValue::Number(a), Some(b)) => (calc(*a, b), None),
                // 빈 대상에 빈 원본: 그대로 (한셀)
                (None, CellValue::Empty, Some(_)) if blank => {
                    let st = super::style_at(sheet, *p);
                    restore(sheet, *p, old.clone());
                    restyle(sheet, *p, st);
                    continue;
                }
                (None, CellValue::Empty, Some(b)) => (calc(0.0, b), None),
                (None, CellValue::Number(a), None) => (
                    CellValue::Empty,
                    Some(format!("{}{sym}{rhs}", num_text(*a))),
                ),
                (None, CellValue::Empty, None) => (CellValue::Empty, Some(format!("0{sym}{rhs}"))),
                _ => {
                    // 대상이 글이면 그대로
                    let st = super::style_at(sheet, *p);
                    restore(sheet, *p, old.clone());
                    restyle(sheet, *p, st);
                    continue;
                }
            }
        } else if with_formulas {
            match &src_formula {
                Some(f) => (CellValue::Empty, Some(f.clone())),
                None => (src_value.clone(), None),
            }
        } else {
            (src_value.clone(), None)
        };
        let keep_style = if formats {
            super::style_at(sheet, *p)
        } else {
            style_before[p]
        };
        if value.is_empty() && formula.is_none() && !formats {
            // 빈 칸을 붙였다: 내용만 지운다
            if let Some(cell) = sheet.cells.iter_mut().find(|x| x.r == *p) {
                cell.value = CellValue::Empty;
                cell.formula = None;
            }
            continue;
        }
        let cell = super::cell_mut(sheet, *p);
        cell.value = value;
        cell.formula = formula.map(|text| Formula {
            text,
            ..Default::default()
        });
        cell.style = keep_style;
    }
    // 숫자 서식 / 테두리
    if with_numfmt || o.what == PasteWhat::AllExceptBorders {
        for (p, _) in &before {
            let Some((x, _)) = src_at.get(p) else {
                continue;
            };
            let cur = super::style_at(&wb.sheets[dsi], *p);
            let mut xf = wb.styles.xf(cur).cloned().unwrap_or_default();
            if with_numfmt {
                xf.num_fmt_id = wb.styles.xf(x.style).map_or(0, |s| s.num_fmt_id);
            } else {
                xf.border_id = wb.styles.xf(style_before[p]).map_or(0, |s| s.border_id);
            }
            let n = super::style::intern_xf(&mut wb.styles, xf);
            if n != cur {
                super::cell_mut(&mut wb.sheets[dsi], *p).style = n;
            }
        }
    }
    // 붙인 범위를 선택한다
    let sheet = &mut wb.sheets[dsi];
    sheet.view.active_cell = Some(Area::rect(at.row, at.col, at.row, at.col).text());
    sheet.view.sqref =
        Some(Area::rect(area.start.row, area.start.col, area.end.row, area.end.col).text());
    super::structure::update_dimension(sheet);
    Ok(area)
}

/// 칸을 붙이기 전으로 되돌린다
fn restore(sheet: &mut crate::model::Sheet, p: CellRef, old: Option<crate::model::Cell>) {
    sheet.cells.retain(|x| x.r != p);
    if let Some(c) = old {
        let i = sheet
            .cells
            .partition_point(|x| (x.r.row, x.r.col) < (p.row, p.col));
        sheet.cells.insert(i, c);
    }
}

/// 칸의 서식을 `st` 로 (셀 기록 없이도 그 서식으로 보이면 기록을 만들지 않는다)
fn restyle(sheet: &mut crate::model::Sheet, p: CellRef, st: u32) {
    if super::style_at(sheet, p) != st {
        super::cell_mut(sheet, p).style = st;
    }
}
