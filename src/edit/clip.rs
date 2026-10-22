// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 복사·붙여넣기, 잘라 붙이기(셀 옮기기), 아래로·오른쪽으로 채우기.
//!
//! - 복사해 붙인 수식의 상대 참조는 옮긴 만큼 바뀐다 (시트 밖으로 나가면 `#REF!`). 서식·병합도 함께 간다.
//! - 잘라 붙이면 옮긴 셀을 가리키던 참조가 모두 새 자리를 따라간다 (원본 범위 안에 모두 든 참조만 —
//!   한셀 대조 실험: `SUM(A1:A10)` 은 그대로, `SUM(A5:A6)` 은 `SUM(Q5:Q6)`). 조건부 서식·유효성도 옮긴 칸의
//!   몫을 떼어 새 자리로 옮긴다 (`B1:B10` 에서 B5:B6 을 옮기면 `B1:B4 B7:B10` + `R5:R6`).

use super::refs::{self, Area, Move};
use super::structure::{map_all_charts, unshare, update_dimension};
use crate::model::{
    Cell, CellRange, CellRef, CellValue, ConditionalFormat, DataValidation, DefinedName, Formula,
    FormulaKind, Sheet, Workbook,
};

/// 복사한 칸 하나
#[derive(Debug, Clone, PartialEq)]
pub struct ClipCell {
    /// 원본 범위 안의 자리
    pub dr: u32,
    pub dc: u32,
    pub value: CellValue,
    /// 수식 원문 (`=` 없이)
    pub formula: Option<String>,
    /// 배열 수식이면 그 크기 (행, 열)
    pub array: Option<(u32, u32)>,
    pub style: u32,
}

/// 복사한 범위
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    pub sheet: usize,
    pub origin: CellRef,
    pub rows: u32,
    pub cols: u32,
    /// 값·수식·서식이 있는 칸 (없는 칸은 빈 기본 서식)
    pub cells: Vec<ClipCell>,
    /// 범위 안의 병합 (원본 범위 기준 (첫 행, 첫 열, 끝 행, 끝 열))
    pub merges: Vec<(u32, u32, u32, u32)>,
    /// 범위에 걸친 조건부 서식: 원본 범위 기준 조각들과, 첫 조각의 왼쪽 위 칸(원본 좌표)에 맞춘 수식
    pub cfs: Vec<(Vec<(u32, u32, u32, u32)>, ConditionalFormat)>,
    /// 범위에 걸친 데이터 유효성 (조건부 서식과 같은 꼴)
    pub dvs: Vec<(Vec<(u32, u32, u32, u32)>, DataValidation)>,
}

fn err(m: &str) -> crate::Error {
    crate::Error::Edit(m.to_string())
}

/// 범위의 칸들을 복사한다
pub fn copy(wb: &Workbook, si: usize, range: CellRange) -> crate::Result<Clip> {
    let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    let mut clip = Clip {
        sheet: si,
        origin: range.start,
        rows: r1 - r0 + 1,
        cols: c1 - c0 + 1,
        ..Default::default()
    };
    let lo = sheet.cells.partition_point(|c| c.r.row < r0);
    let mut have = std::collections::HashSet::new();
    for c in &sheet.cells[lo..] {
        if c.r.row > r1 {
            break;
        }
        if c.r.col < c0 || c.r.col > c1 {
            continue;
        }
        have.insert((c.r.row, c.r.col));
        let mut formula = c.formula.as_ref().map(|f| f.text.clone());
        let mut array = None;
        if let Some(f) = &c.formula {
            match f.kind {
                FormulaKind::Array => {
                    // 범위 안에 모두 들면 배열 수식째, 아니면 값만
                    match f.range.as_deref().and_then(CellRange::parse) {
                        Some(a) if a.start == c.r && a.end.row <= r1 && a.end.col <= c1 => {
                            array =
                                Some((a.end.row - a.start.row + 1, a.end.col - a.start.col + 1));
                        }
                        _ => formula = None,
                    }
                }
                FormulaKind::DataTable => formula = None,
                _ => {}
            }
            if f.text.is_empty() {
                formula = None;
            }
        }
        clip.cells.push(ClipCell {
            dr: c.r.row - r0,
            dc: c.r.col - c0,
            value: c.value.clone(),
            formula,
            array,
            style: c.style,
        });
    }
    // 셀 레코드가 없는 칸의 행·열 서식 (붙인 곳에 서식으로 간다)
    let styled_rows = sheet.rows.iter().any(|r| r.style.is_some_and(|s| s != 0));
    let styled_cols = sheet.cols.iter().any(|c| c.style.is_some_and(|s| s != 0));
    if (styled_rows || styled_cols) && (clip.rows as u64 * clip.cols as u64) <= 1_000_000 {
        for r in r0..=r1 {
            for c in c0..=c1 {
                if have.contains(&(r, c)) {
                    continue;
                }
                let st = sheet.effective_style(r, c);
                if st != 0 {
                    clip.cells.push(ClipCell {
                        dr: r - r0,
                        dc: c - c0,
                        value: CellValue::Empty,
                        formula: None,
                        array: None,
                        style: st,
                    });
                }
            }
        }
        clip.cells.sort_by_key(|c| (c.dr, c.dc));
    }
    // 조건부 서식·유효성 조각: 수식은 첫 조각의 왼쪽 위 칸(원본 좌표)에 맞춰 둔다
    let area = (r0, c0, r1, c1);
    let pieces = |sqref: &str| -> Option<((i64, i64), Vec<(u32, u32, u32, u32)>)> {
        let areas = refs::parse_sqref(sqref);
        let tl = areas.first().map(|a| {
            let b = a.bounds();
            (b.0, b.1)
        })?;
        let cut: Vec<(u32, u32, u32, u32)> = areas
            .iter()
            .filter_map(|a| intersect(a.bounds(), area))
            .map(|x| (x.0 - r0, x.1 - c0, x.2 - r0, x.3 - c0))
            .collect();
        let first = cut.first()?;
        let d = (
            (first.0 + r0) as i64 - tl.0 as i64,
            (first.1 + c0) as i64 - tl.1 as i64,
        );
        Some((d, cut))
    };
    for cf in &sheet.conditional_formats {
        let Some(((dr, dc), cut)) = pieces(&cf.sqref) else {
            continue;
        };
        let mut m = cf.clone();
        for r in &mut m.rules {
            for f in &mut r.formulas {
                *f = crate::formula_text::shift_references(f, dr, dc);
            }
        }
        clip.cfs.push((cut, m));
    }
    for dv in &sheet.data_validations {
        let Some(((dr, dc), cut)) = pieces(&dv.sqref) else {
            continue;
        };
        let mut m = dv.clone();
        for f in [&mut m.formula1, &mut m.formula2].into_iter().flatten() {
            *f = crate::formula_text::shift_references(f, dr, dc);
        }
        clip.dvs.push((cut, m));
    }
    for m in &sheet.merges {
        if m.start.row >= r0 && m.start.col >= c0 && m.end.row <= r1 && m.end.col <= c1 {
            clip.merges.push((
                m.start.row - r0,
                m.start.col - c0,
                m.end.row - r0,
                m.end.col - c0,
            ));
        }
    }
    Ok(clip)
}

fn overlaps(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    !(a.2 < b.0 || b.2 < a.0 || a.3 < b.1 || b.3 < a.1)
}

fn rect(r: &CellRange) -> (u32, u32, u32, u32) {
    (r.start.row, r.start.col, r.end.row, r.end.col)
}

/// 배열 수식 범위를 일부만 덮는가
fn cuts_array(sheet: &Sheet, area: (u32, u32, u32, u32)) -> bool {
    sheet.cells.iter().any(|c| {
        c.formula.as_ref().is_some_and(|f| {
            matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable)
                && f.range
                    .as_deref()
                    .and_then(CellRange::parse)
                    .is_some_and(|a| {
                        let b = rect(&a);
                        overlaps(b, area)
                            && !(b.0 >= area.0 && b.1 >= area.1 && b.2 <= area.2 && b.3 <= area.3)
                    })
        })
    })
}

/// 범위 안 셀을 지우고 (그 자리의 셀 레코드를 없앤다) 새 셀들을 넣는다
fn replace_cells(sheet: &mut Sheet, area: (u32, u32, u32, u32), new: Vec<Cell>) {
    sheet.cells.retain(|c| {
        !(c.r.row >= area.0 && c.r.row <= area.2 && c.r.col >= area.1 && c.r.col <= area.3)
    });
    sheet.cells.extend(new);
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    sheet.cells.dedup_by_key(|c| (c.r.row, c.r.col));
}

/// 복사한 것을 `at` 에 붙인다. `fill` 이 주어지면 그 범위를 복사한 크기의 배수만큼 되풀이해 채운다
/// (배수가 아니면 한 번만). 붙인 범위를 돌려준다
pub fn paste(
    wb: &mut Workbook,
    si: usize,
    at: CellRef,
    clip: &Clip,
    fill: Option<CellRange>,
) -> crate::Result<CellRange> {
    paste_with(wb, si, at, clip, fill, true)
}

/// 수식이 쓰는 통합 문서 이름을 붙인 시트의 시트 범위 이름으로 복사한다 — 한셀은 이름을 쓰는 수식을 복사해
/// 붙이면 그 이름을 붙인 시트 범위로 하나 더 만든다 (편집 대조 실험: `SUM(MyRange)` 를 같은 시트에 복사)
fn localize_names(wb: &mut Workbook, si: usize, formulas: &[String]) {
    use crate::calc::ast::RefOrName;
    let mut used: Vec<String> = Vec::new();
    for t in formulas {
        let Ok(e) = crate::calc::parse(t) else {
            continue;
        };
        e.visit_refs(&mut |r| {
            if let RefOrName::Name(n) = r
                && n.book.is_none()
                && n.sheet.is_none()
                && !used.iter().any(|u| refs::same_sheet(u, &n.name))
            {
                used.push(n.name.clone());
            }
        });
    }
    for n in used {
        let Some(global) = wb
            .defined_names
            .iter()
            .find(|d| d.local_sheet_id.is_none() && refs::same_sheet(&d.name, &n))
            .cloned()
        else {
            continue;
        };
        let exists = wb
            .defined_names
            .iter()
            .any(|d| d.local_sheet_id == Some(si as u32) && refs::same_sheet(&d.name, &n));
        if !exists {
            wb.defined_names.push(DefinedName {
                local_sheet_id: Some(si as u32),
                ..global
            });
        }
    }
}

/// 조건부 서식·유효성 범위에서 덮어쓰는 자리를 뺀다 (모두 빠지면 없앤다)
fn cut_out(sheet: &mut Sheet, area: (u32, u32, u32, u32)) {
    let rest = |sqref: &str| -> Option<String> {
        let areas = refs::parse_sqref(sqref);
        if !areas.iter().any(|a| overlaps(a.bounds(), area)) {
            return None;
        }
        let rest: Vec<Area> = areas
            .iter()
            .flat_map(|a| subtract(a.bounds(), area))
            .map(rect_area)
            .collect();
        Some(refs::sqref_text(&rest))
    };
    for cf in &mut sheet.conditional_formats {
        if let Some(r) = rest(&cf.sqref) {
            cf.sqref = r;
        }
    }
    sheet.conditional_formats.retain(|cf| !cf.sqref.is_empty());
    for dv in &mut sheet.data_validations {
        if let Some(r) = rest(&dv.sqref) {
            dv.sqref = r;
        }
    }
    sheet.data_validations.retain(|dv| !dv.sqref.is_empty());
}

/// [`paste`] — `localize` 면 수식이 쓰는 이름을 시트 범위로 복사한다 (채우기는 하지 않는다)
pub fn paste_with(
    wb: &mut Workbook,
    si: usize,
    at: CellRef,
    clip: &Clip,
    fill: Option<CellRange>,
    localize: bool,
) -> crate::Result<CellRange> {
    let (mut reps_r, mut reps_c) = (1, 1);
    if let Some(f) = fill {
        let (h, w) = (f.end.row - f.start.row + 1, f.end.col - f.start.col + 1);
        if h % clip.rows == 0 && w % clip.cols == 0 {
            reps_r = h / clip.rows;
            reps_c = w / clip.cols;
        }
    }
    let total_r = clip.rows * reps_r;
    let total_c = clip.cols * reps_c;
    if at.row as u64 + total_r as u64 > refs::MAX_ROWS as u64
        || at.col as u64 + total_c as u64 > refs::MAX_COLS as u64
    {
        return Err(err("붙여 넣을 범위가 시트를 넘습니다"));
    }
    let area = (at.row, at.col, at.row + total_r - 1, at.col + total_c - 1);
    let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
    if cuts_array(sheet, area) {
        return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
    }
    let mut new = Vec::new();
    let mut merges = Vec::new();
    for i in 0..reps_r {
        for j in 0..reps_c {
            let base = CellRef {
                row: at.row + i * clip.rows,
                col: at.col + j * clip.cols,
            };
            let drow = base.row as i64 - clip.origin.row as i64;
            let dcol = base.col as i64 - clip.origin.col as i64;
            for c in &clip.cells {
                let r = CellRef {
                    row: base.row + c.dr,
                    col: base.col + c.dc,
                };
                let formula = c.formula.as_ref().map(|t| {
                    let text = crate::formula_text::shift_references(t, drow, dcol);
                    match c.array {
                        Some((h, w)) => Formula {
                            text,
                            kind: FormulaKind::Array,
                            range: Some(
                                Area::rect(r.row, r.col, r.row + h - 1, r.col + w - 1).text(),
                            ),
                            ..Default::default()
                        },
                        None => Formula {
                            text,
                            ..Default::default()
                        },
                    }
                });
                new.push(Cell {
                    r,
                    style: c.style,
                    value: if formula.is_some() {
                        CellValue::Empty
                    } else {
                        c.value.clone()
                    },
                    formula,
                });
            }
            for &(a, b, c, d) in &clip.merges {
                merges.push(CellRange {
                    start: CellRef {
                        row: base.row + a,
                        col: base.col + b,
                    },
                    end: CellRef {
                        row: base.row + c,
                        col: base.col + d,
                    },
                });
            }
        }
    }
    // 조건부 서식·유효성: 덮어쓴 자리의 것은 떼고, 복사한 조각을 옮겨 붙인다
    let mut cfs = Vec::new();
    let mut dvs = Vec::new();
    for i in 0..reps_r {
        for j in 0..reps_c {
            let drow = (at.row + i * clip.rows) as i64 - clip.origin.row as i64;
            let dcol = (at.col + j * clip.cols) as i64 - clip.origin.col as i64;
            let place = |cut: &[(u32, u32, u32, u32)]| -> String {
                let list: Vec<Area> = cut
                    .iter()
                    .filter_map(|&(a, b, c, d)| {
                        Area::rect(
                            a + clip.origin.row,
                            b + clip.origin.col,
                            c + clip.origin.row,
                            d + clip.origin.col,
                        )
                        .offset(drow, dcol)
                    })
                    .collect();
                refs::sqref_text(&list)
            };
            for (cut, cf) in &clip.cfs {
                let mut m = cf.clone();
                for r in &mut m.rules {
                    for f in &mut r.formulas {
                        *f = crate::formula_text::shift_references(f, drow, dcol);
                    }
                }
                m.sqref = place(cut);
                cfs.push(m);
            }
            for (cut, dv) in &clip.dvs {
                let mut m = dv.clone();
                for f in [&mut m.formula1, &mut m.formula2].into_iter().flatten() {
                    *f = crate::formula_text::shift_references(f, drow, dcol);
                }
                m.sqref = place(cut);
                dvs.push(m);
            }
        }
    }
    if localize {
        let texts: Vec<String> = new
            .iter()
            .filter_map(|c| c.formula.as_ref().map(|f| f.text.clone()))
            .collect();
        localize_names(wb, si, &texts);
    }
    let sheet = &mut wb.sheets[si];
    sheet.merges.retain(|m| !overlaps(rect(m), area));
    sheet.merges.extend(merges);
    cut_out(sheet, area);
    sheet.conditional_formats.extend(cfs);
    sheet.data_validations.extend(dvs);
    replace_cells(sheet, area, new);
    unshare(sheet);
    update_dimension(sheet);
    Ok(CellRange {
        start: at,
        end: CellRef {
            row: area.2,
            col: area.3,
        },
    })
}

/// 아래로 채우기 (첫 행을 나머지 행에) / 오른쪽으로 채우기 (첫 열을 나머지 열에)
pub fn fill(wb: &mut Workbook, si: usize, range: CellRange, down: bool) -> crate::Result<()> {
    let (r0, c0, r1, c1) = rect(&range);
    if (down && r1 == r0) || (!down && c1 == c0) {
        return Ok(());
    }
    let src = if down {
        CellRange {
            start: range.start,
            end: CellRef { row: r0, col: c1 },
        }
    } else {
        CellRange {
            start: range.start,
            end: CellRef { row: r1, col: c0 },
        }
    };
    let clip = copy(wb, si, src)?;
    let target = if down {
        CellRange {
            start: CellRef {
                row: r0 + 1,
                col: c0,
            },
            end: range.end,
        }
    } else {
        CellRange {
            start: CellRef {
                row: r0,
                col: c0 + 1,
            },
            end: range.end,
        }
    };
    paste_with(wb, si, target.start, &clip, Some(target), false).map(|_| ())
}

/// 직사각형에서 구멍을 뺀 조각들 (위, 왼쪽, 오른쪽, 아래 순)
fn subtract(a: (u32, u32, u32, u32), hole: (u32, u32, u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    if !overlaps(a, hole) {
        return vec![a];
    }
    let mut out = Vec::new();
    let (r0, c0, r1, c1) = a;
    let (hr0, hc0, hr1, hc1) = (
        hole.0.max(r0),
        hole.1.max(c0),
        hole.2.min(r1),
        hole.3.min(c1),
    );
    if hr0 > r0 {
        out.push((r0, c0, hr0 - 1, c1));
    }
    if hc0 > c0 {
        out.push((hr0, c0, hr1, hc0 - 1));
    }
    if hc1 < c1 {
        out.push((hr0, hc1 + 1, hr1, c1));
    }
    if hr1 < r1 {
        out.push((hr1 + 1, c0, r1, c1));
    }
    out
}

fn intersect(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
    overlaps(a, b).then(|| (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3)))
}

fn rect_area(r: (u32, u32, u32, u32)) -> Area {
    Area::rect(r.0, r.1, r.2, r.3)
}

/// 잘라 붙이기: `src` 의 셀을 `dst_sheet` 의 `at` 으로 옮긴다
pub fn move_cells(
    wb: &mut Workbook,
    si: usize,
    src: CellRange,
    dsi: usize,
    at: CellRef,
) -> crate::Result<()> {
    let s = rect(&src);
    let (h, w) = (s.2 - s.0, s.3 - s.1);
    if at.row as u64 + h as u64 >= refs::MAX_ROWS as u64
        || at.col as u64 + w as u64 >= refs::MAX_COLS as u64
    {
        return Err(err("옮길 자리가 시트를 넘습니다"));
    }
    let d = (at.row, at.col, at.row + h, at.col + w);
    if si == dsi && s == d {
        return Ok(());
    }
    if wb.sheets.get(si).is_none() || wb.sheets.get(dsi).is_none() {
        return Err(err("시트가 없습니다"));
    }
    if cuts_array(&wb.sheets[si], s) || cuts_array(&wb.sheets[dsi], d) {
        return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
    }
    let src_name = wb.sheets[si].name.clone();
    let dst_name = wb.sheets[dsi].name.clone();
    let mv = Move {
        src_sheet: &src_name,
        dst_sheet: &dst_name,
        src: s,
        drow: at.row as i64 - s.0 as i64,
        dcol: at.col as i64 - s.1 as i64,
    };
    let names: Vec<String> = wb.sheets.iter().map(|x| x.name.clone()).collect();

    // 1. 옮길 셀을 떼어 낸다 (참조를 고치기 전 원문 — 고치는 것은 아래에서 함께)
    let taken: Vec<Cell> = {
        let sheet = &mut wb.sheets[si];
        let mut taken = Vec::new();
        sheet.cells.retain(|c| {
            let inside = c.r.row >= s.0 && c.r.row <= s.2 && c.r.col >= s.1 && c.r.col <= s.3;
            if inside {
                taken.push(c.clone());
            }
            !inside
        });
        taken
    };

    // 2. 문서 전체의 참조 (옮긴 셀의 수식 포함 — 그 수식은 원본 시트에 있던 것으로 본다)
    let fix = |f: &str, here: &str| refs::move_formula(f, here, &mv);
    for (k, sh) in wb.sheets.iter_mut().enumerate() {
        let here = names[k].as_str();
        let mut changed = false;
        for c in &mut sh.cells {
            if let Some(fm) = &mut c.formula
                && !fm.text.is_empty()
            {
                let t = fix(&fm.text, here);
                if t != fm.text {
                    fm.text = t;
                    changed = true;
                }
            }
        }
        if changed {
            unshare(sh);
        }
        for h in &mut sh.hyperlinks {
            if let Some(l) = &mut h.location {
                *l = fix(l, here);
            }
        }
    }
    for d in &mut wb.defined_names {
        let here = d
            .local_sheet_id
            .and_then(|i| names.get(i as usize))
            .map(String::as_str)
            .unwrap_or("");
        d.formula = fix(&d.formula, here);
    }
    map_all_charts(wb, &|x| refs::move_formula(x, "", &mv));
    let moved: Vec<Cell> = taken
        .into_iter()
        .map(|mut c| {
            if let Some(f) = &mut c.formula {
                if !f.text.is_empty() {
                    f.text = refs::move_formula(&f.text, &src_name, &mv);
                    if si != dsi {
                        // 다른 시트로 옮긴 수식: 접두 없는 참조는 원래 시트를 가리키게
                        f.text = qualify(&f.text, &src_name);
                    }
                }
                if f.kind == FormulaKind::Shared {
                    f.kind = FormulaKind::Normal;
                    f.shared_index = None;
                    f.range = None;
                }
                if let Some(rg) = f.range.as_deref().and_then(refs::parse_area) {
                    f.range = rg.offset(mv.drow, mv.dcol).map(|a| a.text());
                }
            }
            c.r = CellRef {
                row: (c.r.row as i64 + mv.drow) as u32,
                col: (c.r.col as i64 + mv.dcol) as u32,
            };
            c
        })
        .collect();

    // 3. 조건부 서식·유효성: 옮긴 칸의 몫을 떼어 새 자리로 (수식은 옮긴 칸 기준으로 바꾼다)
    let mut moved_cf = Vec::new();
    let mut moved_dv = Vec::new();
    {
        let sheet = &mut wb.sheets[si];
        let here = src_name.clone();
        let mut keep_cf = Vec::new();
        for mut cf in std::mem::take(&mut sheet.conditional_formats) {
            let areas = refs::parse_sqref(&cf.sqref);
            let Some(tl) = areas.first().map(|a| {
                let b = a.bounds();
                (b.0, b.1)
            }) else {
                keep_cf.push(cf);
                continue;
            };
            let mut rest = Vec::new();
            let mut cut = Vec::new();
            for a in &areas {
                let b = a.bounds();
                rest.extend(subtract(b, s));
                if let Some(x) = intersect(b, s) {
                    cut.push(x);
                }
            }
            if let Some(first) = cut.first() {
                let mut m = cf.clone();
                let (dr, dc) = (first.0 as i64 - tl.0 as i64, first.1 as i64 - tl.1 as i64);
                for r in &mut m.rules {
                    for f in &mut r.formulas {
                        let anchored = crate::formula_text::shift_references(f, dr, dc);
                        *f = refs::move_formula(&anchored, &here, &mv);
                    }
                }
                let list: Vec<Area> = cut
                    .iter()
                    .filter_map(|&x| rect_area(x).offset(mv.drow, mv.dcol))
                    .collect();
                m.sqref = refs::sqref_text(&list);
                moved_cf.push(m);
            }
            if rest.is_empty() {
                continue;
            }
            for r in &mut cf.rules {
                for f in &mut r.formulas {
                    *f = refs::move_formula(f, &here, &mv);
                }
            }
            cf.sqref = refs::sqref_text(&rest.iter().map(|&x| rect_area(x)).collect::<Vec<_>>());
            keep_cf.push(cf);
        }
        sheet.conditional_formats = keep_cf;
        let mut keep_dv = Vec::new();
        for mut dv in std::mem::take(&mut sheet.data_validations) {
            let areas = refs::parse_sqref(&dv.sqref);
            let mut rest = Vec::new();
            let mut cut = Vec::new();
            for a in &areas {
                let b = a.bounds();
                rest.extend(subtract(b, s));
                if let Some(x) = intersect(b, s) {
                    cut.push(x);
                }
            }
            let tl = areas.first().map(|a| {
                let b = a.bounds();
                (b.0, b.1)
            });
            if let (Some(first), Some(tl)) = (cut.first(), tl) {
                let mut m = dv.clone();
                let (dr, dc) = (first.0 as i64 - tl.0 as i64, first.1 as i64 - tl.1 as i64);
                for f in [&mut m.formula1, &mut m.formula2].into_iter().flatten() {
                    let anchored = crate::formula_text::shift_references(f, dr, dc);
                    *f = refs::move_formula(&anchored, &here, &mv);
                }
                let list: Vec<Area> = cut
                    .iter()
                    .filter_map(|&x| rect_area(x).offset(mv.drow, mv.dcol))
                    .collect();
                m.sqref = refs::sqref_text(&list);
                moved_dv.push(m);
            }
            if !rest.is_empty() {
                for f in [&mut dv.formula1, &mut dv.formula2].into_iter().flatten() {
                    *f = refs::move_formula(f, &here, &mv);
                }
                dv.sqref =
                    refs::sqref_text(&rest.iter().map(|&x| rect_area(x)).collect::<Vec<_>>());
                keep_dv.push(dv);
            }
        }
        sheet.data_validations = keep_dv;
    }

    // 4. 병합·메모·하이퍼링크를 옮긴다
    let (mut merges, mut comments, mut links) = (Vec::new(), Vec::new(), Vec::new());
    {
        let sheet = &mut wb.sheets[si];
        sheet.merges.retain(|m| {
            let r = rect(m);
            let inside = r.0 >= s.0 && r.1 >= s.1 && r.2 <= s.2 && r.3 <= s.3;
            if inside {
                merges.push(*m);
            }
            !inside
        });
        sheet.comments.retain(|c| {
            let inside =
                c.cell.row >= s.0 && c.cell.row <= s.2 && c.cell.col >= s.1 && c.cell.col <= s.3;
            if inside {
                comments.push(c.clone());
            }
            !inside
        });
        sheet.hyperlinks.retain(|h| {
            let inside = CellRange::parse(&h.range).is_some_and(|r| {
                let r = rect(&r);
                r.0 >= s.0 && r.1 >= s.1 && r.2 <= s.2 && r.3 <= s.3
            });
            if inside {
                links.push(h.clone());
            }
            !inside
        });
        update_dimension(sheet);
    }

    // 5. 대상 자리에 넣는다 (덮어쓴 칸의 셀·병합·메모·조건부 서식·유효성은 없어진다)
    let dst = &mut wb.sheets[dsi];
    dst.merges.retain(|m| !overlaps(rect(m), d));
    dst.comments.retain(|c| {
        !(c.cell.row >= d.0 && c.cell.row <= d.2 && c.cell.col >= d.1 && c.cell.col <= d.3)
    });
    dst.hyperlinks
        .retain(|h| !CellRange::parse(&h.range).is_some_and(|r| overlaps(rect(&r), d)));
    cut_out(dst, d);
    dst.conditional_formats.extend(moved_cf);
    dst.data_validations.extend(moved_dv);
    for m in merges {
        dst.merges.push(CellRange {
            start: CellRef {
                row: (m.start.row as i64 + mv.drow) as u32,
                col: (m.start.col as i64 + mv.dcol) as u32,
            },
            end: CellRef {
                row: (m.end.row as i64 + mv.drow) as u32,
                col: (m.end.col as i64 + mv.dcol) as u32,
            },
        });
    }
    for mut c in comments {
        c.cell = CellRef {
            row: (c.cell.row as i64 + mv.drow) as u32,
            col: (c.cell.col as i64 + mv.dcol) as u32,
        };
        if let Some(sh) = &mut c.shape {
            for k in [0usize, 4] {
                sh.anchor[k] = (sh.anchor[k] as i64 + mv.dcol).max(0) as u32;
                sh.anchor[k + 2] = (sh.anchor[k + 2] as i64 + mv.drow).max(0) as u32;
            }
        }
        dst.comments.push(c);
    }
    for mut h in links {
        if let Some(a) = refs::parse_area(&h.range).and_then(|a| a.offset(mv.drow, mv.dcol)) {
            h.range = a.text();
        }
        dst.hyperlinks.push(h);
    }
    replace_cells(dst, d, moved);
    unshare(dst);
    update_dimension(dst);
    Ok(())
}

/// 접두 없는 참조에 시트 이름을 붙인다 (다른 시트로 옮긴 수식)
fn qualify(f: &str, sheet: &str) -> String {
    refs::map_refs(f, |r| {
        if r.prefix.is_some() {
            return None;
        }
        Some(format!("{}{}", refs::prefix_text(sheet, None), r.raw))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_pieces_like_hcell() {
        // B1:B10 에서 B5:B6 을 빼면 B1:B4, B7:B10
        let pieces = subtract((0, 1, 9, 1), (4, 0, 5, 1));
        let text: Vec<String> = pieces.iter().map(|&p| rect_area(p).text()).collect();
        assert_eq!(text, ["B1:B4", "B7:B10"]);
    }
}
