// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 셀 삽입·삭제 — 아래·오른쪽으로 밀기, 위·왼쪽으로 당기기 (셀 삽입·삭제 대조 실험 — docs/editing.md 15절).
//!
//! 범위가 걸친 열(오른쪽·왼쪽이면 행)의 띠만 행(열) 삽입·삭제처럼 움직인다.
//! - 수식 참조: 엇갈린 축의 범위가 띠 안에 모두 든 참조만 행·열 편집처럼 바뀐다 (`SUM(A3:D6)` 처럼 띠에 일부만 걸친
//!   참조는 그대로). 지운 칸만 가리키면 `#REF!`.
//! - 삽입한 칸은 앞 칸(위·왼쪽)의 서식을 받는다.
//! - 병합: 띠 안에 모두 들면 함께 움직이고, 띠에 일부만 걸쳐 움직이는 자리에 있으면 풀린다.
//! - 조건부 서식·유효성 범위: 삽입은 범위의 두 모서리를 칸처럼 따로 옮기고 (한셀 — 그래서 `C3:D8` 이 `E3:D8` 이
//!   되기도 한다), 삭제는 띠 밖의 몫은 그대로 두고 띠 안의 몫만 줄인다 (조각은 첫 행 차례).
//! - 메모(상자도 칸 수만큼)·하이퍼링크는 칸을 따라간다.

use super::err;
use super::refs::{self, Area, Shift};
use crate::model::{CellRange, CellRef, Workbook};

/// 미는 방향
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShiftDir {
    Down,
    Right,
    Up,
    Left,
}

/// 행·열 편집과 띠
fn op_of(area: CellRange, dir: ShiftDir) -> (Shift, (u32, u32)) {
    let (r0, c0, r1, c1) = (area.start.row, area.start.col, area.end.row, area.end.col);
    match dir {
        ShiftDir::Down => (
            Shift::InsRows {
                at: r0,
                n: r1 - r0 + 1,
            },
            (c0, c1),
        ),
        ShiftDir::Up => (
            Shift::DelRows {
                at: r0,
                n: r1 - r0 + 1,
            },
            (c0, c1),
        ),
        ShiftDir::Right => (
            Shift::InsCols {
                at: c0,
                n: c1 - c0 + 1,
            },
            (r0, r1),
        ),
        ShiftDir::Left => (
            Shift::DelCols {
                at: c0,
                n: c1 - c0 + 1,
            },
            (r0, r1),
        ),
    }
}

/// 셀 삽입(`Down`·`Right`) 또는 삭제(`Up`·`Left`)
pub fn shift_cells(
    wb: &mut Workbook,
    si: usize,
    area: CellRange,
    dir: ShiftDir,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let (op, band) = op_of(area, dir);
    let rows = op.on_rows();
    let insert = matches!(dir, ShiftDir::Down | ShiftDir::Right);
    let in_band = |r: CellRef| {
        let x = if rows { r.col } else { r.row };
        x >= band.0 && x <= band.1
    };
    let main = |r: CellRef| if rows { r.row } else { r.col };
    let at = main(area.start);
    {
        let sheet = &wb.sheets[si];
        // 띠 밖으로 밀려 나갈 값, 배열 수식의 일부
        if insert {
            let n = if rows {
                area.end.row - area.start.row + 1
            } else {
                area.end.col - area.start.col + 1
            };
            let max = if rows { refs::MAX_ROWS } else { refs::MAX_COLS };
            if sheet.cells.iter().any(|c| {
                in_band(c.r)
                    && main(c.r) >= at
                    && main(c.r) + n >= max
                    && (!c.value.is_empty() || c.formula.is_some())
            }) {
                return Err(err("시트 끝을 넘는 셀이 생겨 삽입할 수 없습니다"));
            }
        }
        for c in &sheet.cells {
            if let Some(f) = &c.formula
                && matches!(
                    f.kind,
                    crate::model::FormulaKind::Array | crate::model::FormulaKind::DataTable
                )
                && let Some(a) = f.range.as_deref().and_then(CellRange::parse)
                && a.start != a.end
                && in_band(a.start) | in_band(a.end)
                && main(a.end) >= at
            {
                return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
            }
        }
    }
    super::guard_cells(wb, si, area)?;
    let target = wb.sheets[si].name.clone();
    // 1. 수식·이름·차트
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    for (k, sh) in wb.sheets.iter_mut().enumerate() {
        let here = names[k].clone();
        for c in &mut sh.cells {
            if let Some(f) = &mut c.formula
                && !f.text.is_empty()
            {
                f.text = refs::shift_formula_band(&f.text, &here, &target, op, band);
            }
        }
        super::structure::unshare(sh);
    }
    for d in &mut wb.defined_names {
        let here = d
            .local_sheet_id
            .and_then(|i| names.get(i as usize))
            .cloned()
            .unwrap_or_default();
        d.formula = refs::shift_formula_band(&d.formula, &here, &target, op, band);
    }
    super::structure::map_all_charts(wb, &|x| refs::shift_formula_band(x, "", &target, op, band));
    // 2. 셀 (삽입한 칸은 앞 칸의 서식)
    let inherit: Vec<(CellRef, u32)> = if insert && at > 0 {
        let sheet = &wb.sheets[si];
        let n = if rows {
            area.end.row - area.start.row + 1
        } else {
            area.end.col - area.start.col + 1
        };
        let mut v = Vec::new();
        for x in band.0..=band.1 {
            let prev = if rows {
                CellRef {
                    row: at - 1,
                    col: x,
                }
            } else {
                CellRef {
                    row: x,
                    col: at - 1,
                }
            };
            let st = super::style_at(sheet, prev);
            if st == 0 {
                continue;
            }
            for k in 0..n {
                let r = if rows {
                    CellRef {
                        row: at + k,
                        col: x,
                    }
                } else {
                    CellRef {
                        row: x,
                        col: at + k,
                    }
                };
                v.push((r, st));
            }
        }
        v
    } else {
        Vec::new()
    };
    let boxes_before = super::structure::comment_geometry(wb, si, 0);
    // 셀 기록 없이 행·열 서식으로만 보이던 띠 안의 칸: 옮긴 자리에 그 서식의 셀을 만든다 (한셀 — 열 서식이 있는 행에
    // 셀을 오른쪽으로 밀면 옮겨진 칸이 원래 열의 서식을 지닌다)
    let carried: Vec<(CellRef, u32)> = {
        let sheet = &wb.sheets[si];
        let last = if rows {
            sheet
                .rows
                .iter()
                .filter(|r| r.style.is_some())
                .map(|r| r.index - 1)
                .max()
        } else {
            sheet
                .cols
                .iter()
                .filter(|c| c.style.is_some())
                .map(|c| c.max - 1)
                .max()
        };
        let mut v = Vec::new();
        if let Some(last) = last
            && last >= at
            && (last - at) as u64 * (band.1 - band.0 + 1) as u64 <= 200_000
        {
            for m in at..=last {
                for x in band.0..=band.1 {
                    let r = if rows {
                        CellRef { row: m, col: x }
                    } else {
                        CellRef { row: x, col: m }
                    };
                    if sheet.cell(r).is_some() {
                        continue;
                    }
                    let Some(nm) = op.point(rows, m) else {
                        continue;
                    };
                    let to = if rows {
                        CellRef { row: nm, col: x }
                    } else {
                        CellRef { row: x, col: nm }
                    };
                    let st = sheet.effective_style(r.row, r.col);
                    if st != sheet.effective_style(to.row, to.col) {
                        v.push((to, st));
                    }
                }
            }
        }
        v
    };
    let sheet = &mut wb.sheets[si];
    let mut cells = Vec::with_capacity(sheet.cells.len());
    for mut c in std::mem::take(&mut sheet.cells) {
        if in_band(c.r) {
            let Some(m) = op.point(rows, main(c.r)) else {
                continue;
            };
            if rows {
                c.r.row = m;
            } else {
                c.r.col = m;
            }
            if let Some(f) = &mut c.formula
                && let Some(rg) = f.range.clone()
                && let Some(a) = refs::parse_area(&rg)
            {
                f.range = op.area(a, false).map(|b| b.text());
            }
        }
        cells.push(c);
    }
    for (r, st) in carried {
        if !cells.iter().any(|c| c.r == r) {
            cells.push(crate::model::Cell {
                r,
                style: st,
                value: crate::model::CellValue::Empty,
                formula: None,
            });
        }
    }
    for (r, st) in inherit {
        cells.push(crate::model::Cell {
            r,
            style: st,
            value: crate::model::CellValue::Empty,
            formula: None,
        });
    }
    cells.sort_by_key(|c| (c.r.row, c.r.col));
    cells.dedup_by_key(|c| (c.r.row, c.r.col));
    sheet.cells = cells;
    // 3. 병합
    let zone_hit = |m: &CellRange| main(m.end) >= at;
    let mut merges = Vec::new();
    for m in std::mem::take(&mut sheet.merges) {
        let inside = in_band(m.start) && in_band(m.end);
        let crosses = !inside && {
            let (a, b) = if rows {
                (m.start.col, m.end.col)
            } else {
                (m.start.row, m.end.row)
            };
            !(b < band.0 || a > band.1)
        };
        if crosses && zone_hit(&m) {
            continue;
        }
        if inside {
            let (Some(a), Some(b)) = (op.point(rows, main(m.start)), op.point(rows, main(m.end)))
            else {
                continue;
            };
            let mut n = m;
            if rows {
                n.start.row = a;
                n.end.row = b;
            } else {
                n.start.col = a;
                n.end.col = b;
            }
            if n.start != n.end {
                merges.push(n);
            }
            continue;
        }
        merges.push(m);
    }
    sheet.merges = merges;
    // 4. 조건부 서식·유효성 범위
    let fix = |sq: &str| -> String { band_sqref(sq, op, band, insert, area) };
    for cf in &mut sheet.conditional_formats {
        cf.sqref = fix(&cf.sqref);
    }
    sheet.conditional_formats.retain(|c| !c.sqref.is_empty());
    for dv in &mut sheet.data_validations {
        dv.sqref = fix(&dv.sqref);
    }
    sheet.data_validations.retain(|d| !d.sqref.is_empty());
    // 도형: 시작 칸이 띠 안에서 움직이는 자리에 있으면 행·열 편집처럼 옮긴다
    for sh in &mut sheet.shapes {
        let f = sh.anchor.from;
        let fr = CellRef {
            row: f.row,
            col: f.col,
        };
        if in_band(fr) && main(fr) >= at {
            super::structure::shift_shape(sh, op);
        }
    }
    // 5. 하이퍼링크·메모
    sheet.hyperlinks.retain_mut(|h| {
        let Some(rg) = CellRange::parse(&h.range) else {
            return true;
        };
        if !(in_band(rg.start) && in_band(rg.end)) {
            return true;
        }
        match (op.point(rows, main(rg.start)), op.point(rows, main(rg.end))) {
            (Some(a), Some(b)) => {
                let mut n = rg;
                if rows {
                    n.start.row = a;
                    n.end.row = b;
                } else {
                    n.start.col = a;
                    n.end.col = b;
                }
                h.range = Area::rect(n.start.row, n.start.col, n.end.row, n.end.col).text();
                true
            }
            _ => false,
        }
    });
    let mut moved: Vec<usize> = Vec::new();
    sheet
        .comments
        .retain(|c| !in_band(c.cell) || op.point(rows, main(c.cell)).is_some());
    for (i, c) in sheet.comments.iter_mut().enumerate() {
        if !in_band(c.cell) {
            continue;
        }
        let Some(m) = op.point(rows, main(c.cell)) else {
            continue;
        };
        if m == main(c.cell) {
            continue;
        }
        if rows {
            c.cell.row = m;
        } else {
            c.cell.col = m;
        }
        moved.push(i);
    }
    super::structure::update_dimension(sheet);
    // 칸이 옮겨진 메모의 상자: 행·열 삽입·삭제와 같은 규칙 (메모 칸의 열 + 14px, 두 행 위 + 10px, 크기 그대로)
    if let Some((old, ext)) = boxes_before
        && !moved.is_empty()
    {
        let new = crate::render::sheet_geometry(wb, &wb.sheets[si], ext.0 + 64, ext.1 + 32);
        let sheet = &mut wb.sheets[si];
        for i in moved {
            let c = &mut sheet.comments[i];
            let cell = c.cell;
            if let Some(s) = &mut c.shape {
                s.anchor = super::structure::moved_comment_box(&old, &new, s.anchor, cell);
            }
        }
    }
    Ok(())
}

/// 조건부 서식·유효성 범위 (삽입은 모서리를 따로, 삭제는 띠 안의 몫만)
fn band_sqref(sq: &str, op: Shift, band: (u32, u32), insert: bool, area: CellRange) -> String {
    let rows = op.on_rows();
    let in_band = |x: u32| x >= band.0 && x <= band.1;
    let mut out: Vec<(u32, String)> = Vec::new();
    for a in refs::parse_sqref(sq) {
        let (r0, c0, r1, c1) = a.bounds();
        if insert {
            let corner = |r: u32, c: u32| -> (u32, u32) {
                let cross = if rows { c } else { r };
                if !in_band(cross) {
                    return (r, c);
                }
                if rows {
                    (op.point(true, r).unwrap_or(r), c)
                } else {
                    (r, op.point(false, c).unwrap_or(c))
                }
            };
            let (a0, b0) = corner(r0, c0);
            let (a1, b1) = corner(r1, c1);
            let text = if a0 == a1 && b0 == b1 {
                Area::rect(a0, b0, a0, b0).text()
            } else {
                // 모서리가 뒤집혀도 그대로 적는다 (한셀)
                format!(
                    "{}{}:{}{}",
                    crate::model::CellRef::col_name(b0),
                    a0 + 1,
                    crate::model::CellRef::col_name(b1),
                    a1 + 1
                )
            };
            out.push((a0.min(a1), text));
            continue;
        }
        // 삭제: 움직이는 자리(띠 × 지운 곳부터 끝까지)를 뺀 몫 + 그 안의 몫을 줄인 것
        let zone = if rows {
            (area.start.row, band.0, refs::MAX_ROWS - 1, band.1)
        } else {
            (band.0, area.start.col, band.1, refs::MAX_COLS - 1)
        };
        let rect = (r0, c0, r1, c1);
        let overl = !(rect.2 < zone.0 || zone.2 < rect.0 || rect.3 < zone.1 || zone.3 < rect.1);
        if !overl {
            out.push((r0, a.text()));
            continue;
        }
        for p in super::clip::subtract_rect(rect, zone) {
            out.push((p.0, Area::rect(p.0, p.1, p.2, p.3).text()));
        }
        let inner = (
            rect.0.max(zone.0),
            rect.1.max(zone.1),
            rect.2.min(zone.2),
            rect.3.min(zone.3),
        );
        let moved = if rows {
            op.span(true, inner.0, inner.2, false)
                .map(|(x, y)| (x, inner.1, y, inner.3))
        } else {
            op.span(false, inner.1, inner.3, false)
                .map(|(x, y)| (inner.0, x, inner.2, y))
        };
        if let Some(m) = moved {
            out.push((m.0, Area::rect(m.0, m.1, m.2, m.3).text()));
        }
    }
    if !insert {
        out.sort_by_key(|x| x.0);
    }
    out.into_iter().map(|x| x.1).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rg(s: &str) -> CellRange {
        CellRange::parse(s).unwrap()
    }

    #[test]
    fn band_refs_like_hcell() {
        // 셀 삽입·삭제 대조 실험: B3:C4 아래로 삽입
        let (op, band) = op_of(rg("B3:C4"), ShiftDir::Down);
        let f = |t: &str| refs::shift_formula_band(t, "Sheet1", "Sheet1", op, band);
        assert_eq!(f("B5"), "B7");
        assert_eq!(f("SUM(B1:B10)"), "SUM(B1:B12)");
        assert_eq!(f("SUM(B3:C6)"), "SUM(B5:C8)");
        assert_eq!(f("SUM(A3:D6)"), "SUM(A3:D6)");
        assert_eq!(f("SUM(5:5)"), "SUM(5:5)");
        // 위로 당기기: 지운 칸만 가리키면 #REF!
        let (op, band) = op_of(rg("B3:C4"), ShiftDir::Up);
        let f = |t: &str| refs::shift_formula_band(t, "Sheet1", "Sheet1", op, band);
        assert_eq!(f("SUM(B3:C4)"), "SUM(#REF!)");
        assert_eq!(f("SUM(B2:B4)"), "SUM(B2:B2)");
        assert_eq!(f("B5"), "B3");
    }

    #[test]
    fn band_sqref_like_hcell() {
        // 삽입은 모서리를 따로 (오른쪽으로 밀면 C3:D8 → E3:D8), 삭제는 띠 안의 몫만
        let a = rg("B3:C4");
        let (op, band) = op_of(a, ShiftDir::Down);
        assert_eq!(band_sqref("C3:D8", op, band, true, a), "C5:D8");
        assert_eq!(band_sqref("B1:B10", op, band, true, a), "B1:B12");
        let (op, band) = op_of(a, ShiftDir::Right);
        assert_eq!(band_sqref("C3:D8", op, band, true, a), "E3:D8");
        let (op, band) = op_of(a, ShiftDir::Up);
        assert_eq!(band_sqref("C3:D8", op, band, false, a), "D3:D8 C3:C6");
        let (op, band) = op_of(a, ShiftDir::Left);
        assert_eq!(band_sqref("B1:B10", op, band, false, a), "B1:B2 B5:B10");
        assert_eq!(band_sqref("C3:D8", op, band, false, a), "B3:B4 C5:D8");
    }
}
