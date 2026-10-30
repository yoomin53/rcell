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
//! - 행/열 바꿈: 칸을 뒤집고(조건부 서식은 원본 칸마다 따로, 유효성은 범위를 뒤집어) 수식의 참조도 행·열 차이를 바꾼다 (`A2` 의 `=A1*2` 를 F1 에 붙이면 `=E1*2`, 절대
//!   참조도 — `C2` 의 `=$A$1+1` 을 F3 에 붙이면 `=$E$1+1`).
//! - 빈 셀 무시: 원본의 빈 칸은 대상을 바꾸지 않는다.
//! - 연산(더하기·빼기·곱하기·나누기): 대상 값 ∘ 원본 값. 원본 빈 칸은 0, 원본 글·논리값은 건너뛴다. 대상 논리값은 1·0. 대상이나 원본이
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
    // 유효성: 조각을 뒤집고 수식은 뒤집힌 첫 조각의 왼쪽 위 칸에 맞춘다 (한셀: A1 의 유효성을 AP1 에 바꿔 붙이면
    // 같은 개체에 AP1 이 더해진다)
    t.dvs = c
        .dvs
        .iter()
        .map(|(cut, dv)| {
            let mut m = dv.clone();
            if let Some(f) = cut.first() {
                let dr = (at.row + f.1) as i64 - (c.origin.row + f.0) as i64;
                let dc = (at.col + f.0) as i64 - (c.origin.col + f.1) as i64;
                for x in [&mut m.formula1, &mut m.formula2].into_iter().flatten() {
                    *x = crate::formula_text::shift_references(x, dr, dc);
                }
            }
            let cut = cut.iter().map(|&(a, b, cc, d)| (b, a, d, cc)).collect();
            (cut, m)
        })
        .collect();
    t.dv_src = c.dv_src.clone();
    t
}

/// 행/열 바꿈의 조건부 서식: 한셀은 원본 칸마다(행 차례) 그 칸이 든 묶음들을 바꾼 자리 한 칸에 따로 붙인다 — 칸마다
/// 맨 위 우선순위를 받으므로 마지막 칸의 것이 가장 위다 (데이터 막대 묶음은 붙이기처럼 한 번 더)
fn transposed_cf(c: &Clip, at: CellRef) -> Vec<Vec<crate::model::ConditionalFormat>> {
    let mut out = Vec::new();
    for dr in 0..c.rows {
        for dc in 0..c.cols {
            let mut blocks = Vec::new();
            for (cut, cf) in &c.cfs {
                let Some(first) = cut.first() else { continue };
                if !cut
                    .iter()
                    .any(|&(a, b, cc, d)| dr >= a && dr <= cc && dc >= b && dc <= d)
                {
                    continue;
                }
                // 수식은 첫 조각 왼쪽 위(원본) 기준 → 바꾼 자리 칸 기준
                let to = CellRef {
                    row: at.row + dc,
                    col: at.col + dr,
                };
                let d_row = to.row as i64 - (c.origin.row + first.0) as i64;
                let d_col = to.col as i64 - (c.origin.col + first.1) as i64;
                let mut m = cf.clone();
                for r in &mut m.rules {
                    for f in &mut r.formulas {
                        *f = crate::formula_text::shift_references(f, d_row, d_col);
                    }
                }
                m.sqref = Area::rect(to.row, to.col, to.row, to.col).text();
                blocks.push(m);
            }
            if !blocks.is_empty() {
                out.push(blocks);
            }
        }
    }
    out
}

/// 행/열 바꿔 붙일 때 유효성 범위 목록 (유효성 대조 실험 12가지): 한셀은 원본 칸마다(행 차례) 붙이고 — 칸이 유효성 범위
/// 안이면 바꾼 자리의 한 칸을 목록 끝에 더하고 — 칸마다(범위 밖 칸도) 목록의 차례를 뒤집는다 (범위가 둘 이상이던 유효성만). 끝나면 맞닿은 칸을 하나로
/// 이어 붙인다 (먼저 이은 자리에). 그래서 칸이 홀수 개면 뒤집힌 채, 짝수 개면 원래 차례다
fn transposed_dv_sqref(old: &str, origin: CellRef, rows: u32, cols: u32, at: CellRef) -> String {
    type R = (u32, u32, u32, u32);
    let mut list: Vec<(R, bool)> = refs::parse_sqref(old)
        .iter()
        .map(|a| (a.bounds(), false))
        .collect();
    let inside = |list: &[(R, bool)], r: u32, c: u32| {
        list.iter()
            .any(|&((r0, c0, r1, c1), piece)| !piece && r >= r0 && r <= r1 && c >= c0 && c <= c1)
    };
    let orig: Vec<(R, bool)> = list.clone();
    for dr in 0..rows {
        for dc in 0..cols {
            if inside(&orig, origin.row + dr, origin.col + dc) {
                let (r, c) = (at.row + dc, at.col + dr);
                list.push(((r, c, r, c), true));
            }
            // 범위가 하나뿐이던 유효성은 뒤집지 않는다 (한셀 — 더한 칸이 끝에 이어진다)
            if orig.len() > 1 {
                list.reverse();
            }
        }
    }
    // 더한 칸 중 맞닿은 것은 먼저 이은 자리에 하나로
    let mut out: Vec<(R, bool)> = Vec::new();
    for (rect, piece) in list {
        if piece {
            let mut cur = rect;
            let mut at_i: Option<usize> = None;
            while let Some(i) = out.iter().position(|&(a, p)| {
                p && (super::cfpaste::join(a, cur).is_some()
                    || super::cfpaste::join(cur, a).is_some())
            }) {
                let (a, _) = out.remove(i);
                cur = super::cfpaste::join(a, cur)
                    .or_else(|| super::cfpaste::join(cur, a))
                    .unwrap_or(cur);
                at_i = Some(at_i.map_or(i, |x: usize| x.min(i)));
            }
            match at_i {
                Some(i) => out.insert(i, (cur, true)),
                None => out.push((cur, true)),
            }
        } else {
            out.push((rect, false));
        }
    }
    let areas: Vec<Area> = out
        .iter()
        .map(|&((a, b, c, d), _)| Area::rect(a, b, c, d))
        .collect();
    refs::sqref_text(&areas)
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
    let mut cf_cells = Vec::new();
    // 바꾸기 전 원본의 자리와 크기 (유효성 범위를 칸마다 따라가는 데 쓴다)
    let (src_origin, src_rows, src_cols) = (c.origin, c.rows, c.cols);
    if o.transpose {
        cf_cells = transposed_cf(&c, to.start);
        c = transpose_clip(&c, to.start);
    }
    // 서식만 붙이면 메모·유효성은 가지 않는다 (한셀)
    if o.what == PasteWhat::Formats {
        c.comments.clear();
        c.dvs.clear();
        c.dv_src.clear();
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
    if !formats {
        // 값·수식만 붙여도 복사는 시트의 서식 없는 고유·중복 값 규칙을 고친다 (한셀)
        let crate::model::Workbook { sheets, styles, .. } = wb;
        super::cfpaste::fix_bare(&mut sheets[si], &mut styles.dxfs);
    }
    if formats {
        let dv_before: Vec<String> = wb.sheets[dsi]
            .data_validations
            .iter()
            .map(|d| d.sqref.clone())
            .collect();
        clip::paste_with(wb, dsi, at, &c, Some(area), false)?;
        // 붙이기는 유효성마다 범위 차례를 뒤집는다. 서식만 붙이기는 뒤집지 않고, 행/열 바꿈 붙이기는 범위가 더해진
        // 유효성만 뒤집는다 (한셀 — 바꿔 붙인 범위는 원래 범위 뒤에 더한 뒤 뒤집힌다)
        if o.what == PasteWhat::Formats {
            for (dv, old) in wb.sheets[dsi].data_validations.iter_mut().zip(&dv_before) {
                dv.sqref = old.clone();
            }
        } else if o.transpose {
            let sheet = &mut wb.sheets[dsi];
            for (dv, old) in sheet.data_validations.iter_mut().zip(&dv_before) {
                dv.sqref = transposed_dv_sqref(old, src_origin, src_rows, src_cols, at);
            }
        }
        for blocks in cf_cells {
            super::cfpaste::place_pasted(&mut wb.sheets[dsi], &mut wb.styles.dxfs, blocks);
        }
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
            // 대상 논리값은 수 (TRUE = 1, FALSE = 0 — 한셀: TRUE 에 빈 칸을 더하면 1)
            let old_v = match old_v {
                CellValue::Bool(b) => CellValue::Number(if b { 1.0 } else { 0.0 }),
                v => v,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_refs_like_hcell() {
        let c = |s: &str| CellRef::parse(s).unwrap();
        // A2 의 =A1*2 를 바꿔 붙이면 F1 에서 =E1*2, B2 의 SUM(A1:B1) 은 F2 에서 SUM(E1:E2), 절대 참조도 바뀐다
        assert_eq!(transpose_formula("A1*2", c("A2"), c("F1")), "E1*2");
        assert_eq!(
            transpose_formula("SUM(A1:B1)", c("B2"), c("F2")),
            "SUM(E1:E2)"
        );
        assert_eq!(transpose_formula("$A$1+1", c("C2"), c("F3")), "$E$1+1");
    }
}
