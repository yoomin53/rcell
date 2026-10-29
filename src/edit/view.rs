// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 숨기기·그룹(개요)·틀 고정·시트 탭 색 (보기 대조 실험 — docs/editing.md 15절).
//!
//! - 행·열 숨기기는 숨김 표시만 바꾼다. 열 정보가 없는 열은 시트 기본 너비로 열 정보를 만든다.
//! - 활성 시트를 숨기면 다음 보이는 시트(없으면 앞 시트)가 활성·선택된다. 마지막 보이는 시트는 숨길 수 없다.
//! - 틀 고정은 칸의 왼쪽 위로 나눈다 (`xSplit` = 칸 열 번호, `ySplit` = 칸 행 번호, 오른쪽 아래 첫 칸 = 그 칸).
//!   한셀은 선택을 그 칸으로 두고 선택을 나뉜 칸 쪽(`bottomRight` …)에 둔다.
//! - 그룹은 행·열의 개요 수준을 1 올린다 (7까지), 그룹 해제는 1 내린다.

use super::err;
use crate::model::{CellRef, ColInfo, Pane, RowInfo, Workbook};

fn row_info(sheet: &mut crate::model::Sheet, r: u32) -> &mut RowInfo {
    let i = match sheet.rows.binary_search_by_key(&(r + 1), |x| x.index) {
        Ok(i) => i,
        Err(i) => {
            sheet.rows.insert(
                i,
                RowInfo {
                    index: r + 1,
                    ..Default::default()
                },
            );
            i
        }
    };
    &mut sheet.rows[i]
}

/// 열 정보를 열마다 나눠 `f` 를 적용한다 (열 정보가 없는 열은 기본 너비로 만든다)
fn each_col(wb: &mut Workbook, si: usize, c0: u32, c1: u32, f: &dyn Fn(&mut ColInfo)) {
    let default_w = super::structure::default_col_width(wb, si);
    let sheet = &mut wb.sheets[si];
    let (a, b) = (c0 + 1, c1.min(super::refs::MAX_COLS - 1) + 1);
    let mut out: Vec<ColInfo> = Vec::new();
    let mut covered: Vec<u32> = Vec::new();
    for c in std::mem::take(&mut sheet.cols) {
        if c.max < a || c.min > b {
            out.push(c);
            continue;
        }
        if c.min < a {
            out.push(ColInfo {
                max: a - 1,
                ..c.clone()
            });
        }
        if c.max > b {
            out.push(ColInfo {
                min: b + 1,
                ..c.clone()
            });
        }
        let mut mid = ColInfo {
            min: c.min.max(a),
            max: c.max.min(b),
            ..c.clone()
        };
        covered.extend(mid.min..=mid.max);
        f(&mut mid);
        out.push(mid);
    }
    // 열 정보가 없던 열 (이어진 것끼리 하나로)
    let mut k = a;
    while k <= b {
        if covered.contains(&k) {
            k += 1;
            continue;
        }
        let start = k;
        while k <= b && !covered.contains(&k) {
            k += 1;
        }
        let mut info = ColInfo {
            min: start,
            max: k - 1,
            width: Some(default_w),
            ..Default::default()
        };
        f(&mut info);
        out.push(info);
    }
    out.sort_by_key(|c| c.min);
    sheet.cols = out;
}

/// 행 숨기기·보이기 (0부터)
pub fn hide_rows(
    wb: &mut Workbook,
    si: usize,
    r0: u32,
    r1: u32,
    hidden: bool,
) -> crate::Result<()> {
    // 보호된 시트는 행 서식이 허용됐을 때만 (한셀)
    if si < wb.sheets.len() {
        super::guard_action(wb, si, "formatRows")?;
    }
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    for r in r0..=r1.min(r0 + 100_000) {
        if !hidden
            && sheet
                .rows
                .binary_search_by_key(&(r + 1), |x| x.index)
                .is_err()
        {
            continue;
        }
        row_info(sheet, r).hidden = hidden;
    }
    Ok(())
}

/// 열 숨기기·보이기 (0부터)
pub fn hide_cols(
    wb: &mut Workbook,
    si: usize,
    c0: u32,
    c1: u32,
    hidden: bool,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    super::guard_action(wb, si, "formatColumns")?;
    if !hidden {
        each_col(wb, si, c0, c1, &|c| c.hidden = false);
        // 너비만 기본값인 빈 열 정보는 없앤다
        let dw = super::structure::default_col_width(wb, si);
        wb.sheets[si].cols.retain(|c| {
            !(c.min > c0
                && c.max <= c1 + 1
                && !c.hidden
                && c.style.is_none()
                && c.outline_level == 0
                && !c.custom_width
                && c.width.is_none_or(|w| (w - dw).abs() < 1e-9))
        });
        return Ok(());
    }
    each_col(wb, si, c0, c1, &|c| c.hidden = true);
    Ok(())
}

/// 시트 숨기기·보이기. 활성 시트를 숨기면 다음 보이는 시트가 활성 시트가 된다
pub fn hide_sheet(wb: &mut Workbook, si: usize, hidden: bool) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    if !hidden {
        wb.sheets[si].state = crate::model::SheetState::Visible;
        return Ok(());
    }
    let visible = wb
        .sheets
        .iter()
        .filter(|s| s.state == crate::model::SheetState::Visible)
        .count();
    if wb.sheets[si].state == crate::model::SheetState::Visible && visible <= 1 {
        return Err(err("마지막 보이는 시트는 숨길 수 없습니다"));
    }
    wb.sheets[si].state = crate::model::SheetState::Hidden;
    let active = wb.view.as_ref().map_or(0, |v| v.active_tab as usize);
    if active == si || wb.sheets[si].view.tab_selected {
        let next = (si + 1..wb.sheets.len())
            .chain((0..si).rev())
            .find(|&i| wb.sheets[i].state == crate::model::SheetState::Visible);
        if let Some(n) = next {
            wb.sheets[si].view.tab_selected = false;
            super::select_sheet(wb, n);
        }
    }
    Ok(())
}

/// 틀 고정: `cell` 의 왼쪽·위를 고정한다 (A1 이면 풀기)
pub fn freeze(wb: &mut Workbook, si: usize, cell: CellRef) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    if cell.row == 0 && cell.col == 0 {
        sheet.view.pane = None;
        return Ok(());
    }
    let a1 = super::refs::Area::rect(cell.row, cell.col, cell.row, cell.col).text();
    let active = match (cell.row > 0, cell.col > 0) {
        (true, true) => "bottomRight",
        (true, false) => "bottomLeft",
        _ => "topRight",
    };
    sheet.view.pane = Some(Pane {
        x_split: cell.col as f64,
        y_split: cell.row as f64,
        top_left_cell: Some(a1.clone()),
        active_pane: Some(active.into()),
        state: Some("frozen".into()),
    });
    sheet.view.active_cell = Some(a1.clone());
    sheet.view.sqref = Some(a1);
    Ok(())
}

/// 틀 고정 풀기
pub fn unfreeze(wb: &mut Workbook, si: usize) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    sheet.view.pane = None;
    Ok(())
}

/// 시트 탭 색 (`RRGGBB`, 빈 글이면 없앤다)
pub fn tab_color(wb: &mut Workbook, si: usize, color: &str) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let c = color.trim().trim_start_matches('#');
    if c.is_empty() {
        sheet.tab_color = None;
        return Ok(());
    }
    if c.len() != 6 || !c.chars().all(|x| x.is_ascii_hexdigit()) {
        return Err(err(format!("색이 아닙니다: {color}")));
    }
    sheet.tab_color = Some(crate::model::Color {
        rgb: Some(format!("FF{}", c.to_ascii_uppercase())),
        ..Default::default()
    });
    Ok(())
}

/// 행 그룹 (개요 수준 +1, 해제면 −1)
pub fn group_rows(
    wb: &mut Workbook,
    si: usize,
    r0: u32,
    r1: u32,
    group: bool,
) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    for r in r0..=r1.min(r0 + 100_000) {
        if !group
            && sheet
                .rows
                .binary_search_by_key(&(r + 1), |x| x.index)
                .is_err()
        {
            continue;
        }
        let info = row_info(sheet, r);
        info.outline_level = if group {
            (info.outline_level + 1).min(7)
        } else {
            info.outline_level.saturating_sub(1)
        };
    }
    Ok(())
}

/// 열 그룹 (개요 수준 +1, 해제면 −1)
pub fn group_cols(
    wb: &mut Workbook,
    si: usize,
    c0: u32,
    c1: u32,
    group: bool,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    each_col(wb, si, c0, c1, &|c| {
        c.outline_level = if group {
            (c.outline_level + 1).min(7)
        } else {
            c.outline_level.saturating_sub(1)
        };
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeze_like_hcell() {
        let mut wb = Workbook {
            sheets: vec![crate::model::Sheet::default()],
            ..Default::default()
        };
        freeze(&mut wb, 0, CellRef::parse("B3").unwrap()).unwrap();
        let p = wb.sheets[0].view.pane.clone().unwrap();
        assert_eq!((p.x_split, p.y_split), (1.0, 2.0));
        assert_eq!(p.top_left_cell.as_deref(), Some("B3"));
        assert_eq!(p.active_pane.as_deref(), Some("bottomRight"));
        freeze(&mut wb, 0, CellRef::parse("A4").unwrap()).unwrap();
        assert_eq!(
            wb.sheets[0]
                .view
                .pane
                .as_ref()
                .unwrap()
                .active_pane
                .as_deref(),
            Some("bottomLeft")
        );
        unfreeze(&mut wb, 0).unwrap();
        assert!(wb.sheets[0].view.pane.is_none());
    }
}
