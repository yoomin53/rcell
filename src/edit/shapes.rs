// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 도형·그림·차트 옮기기·크기 바꾸기·지우기 (한셀 COM `Shape.Left/Top/Width/Height`·`Shape.Delete` 대조 실험 —
//! docs/editing.md 19절).
//!
//! - 위치·크기는 포인트(시트 왼쪽 위 기준)이고 화면 px 는 ×96/72 다. 새 왼쪽 위·오른쪽 아래 표지는 시트 배치(열 너비·행
//!   높이)에서 px 자리로 찾아 `칸 + 오프셋(EMU)` 으로 쓴다. 한셀은 포인트 값의 px(×96/72) 정수 부분에 1 px 를 더한다. 값을 주지
//!   않은 것은 그대로 (크기는 그대로 두고 옮긴다).
//! - 지우면 그 도형만 없어지고 다른 도형의 번호·이름은 그대로.

use super::err;
use crate::model::{AnchorKind, Marker, Shape, Workbook};
use serde::Deserialize;

const EMU: i64 = crate::units::EMU_PER_PX;

/// 도형 지정: 번호(1부터, 시트의 도형 차례) 또는 이름
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ShapeSel {
    Index(usize),
    Name(String),
}

fn find(wb: &Workbook, si: usize, sel: &ShapeSel) -> crate::Result<usize> {
    let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
    match sel {
        ShapeSel::Index(n) => (*n >= 1 && *n <= sheet.shapes.len())
            .then(|| n - 1)
            .ok_or_else(|| err(format!("도형 번호가 없습니다: {n}"))),
        ShapeSel::Name(n) => sheet
            .shapes
            .iter()
            .position(|s| &s.name == n)
            .ok_or_else(|| err(format!("도형 이름이 없습니다: {n}"))),
    }
}

/// 도형 지우기
pub fn delete_shape(wb: &mut Workbook, si: usize, sel: &ShapeSel) -> crate::Result<()> {
    super::guard_action(wb, si, "objects")?;
    let i = find(wb, si, sel)?;
    wb.sheets[si].shapes.remove(i);
    Ok(())
}

fn marker_abs(g: &crate::render::Geometry, m: &Marker) -> (i64, i64) {
    let x = g.cols.get(m.col as usize).map_or(0, |c| c.0 as i64) * EMU + m.col_off;
    let y = g.rows.get(m.row as usize).map_or(0, |r| r.0 as i64) * EMU + m.row_off;
    (x, y)
}

/// 시트 안 자리(EMU) → 표지
fn marker_at(g: &crate::render::Geometry, x: i64, y: i64) -> Marker {
    let (col, _) = super::structure::locate_px(&g.cols, x.max(0) / EMU);
    let (row, _) = super::structure::locate_px(&g.rows, y.max(0) / EMU);
    let cs = g.cols.get(col as usize).map_or(0, |c| c.0 as i64) * EMU;
    let rs = g.rows.get(row as usize).map_or(0, |r| r.0 as i64) * EMU;
    Marker {
        col,
        col_off: (x.max(0) - cs).max(0),
        row,
        row_off: (y.max(0) - rs).max(0),
    }
}

/// 포인트 → EMU: 한셀은 px(×96/72)의 정수 부분에 1 을 더한다 (`Left` 50pt → 67px, 55pt → 74px, 300pt → 401px; 너비도 같다)
fn pt_emu(pt: f64) -> i64 {
    ((pt * 96.0 / 72.0).floor() as i64 + 1) * EMU
}

/// 도형 옮기기·크기 바꾸기 (포인트). 주지 않은 값은 그대로
pub fn move_shape(
    wb: &mut Workbook,
    si: usize,
    sel: &ShapeSel,
    left: Option<f64>,
    top: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
) -> crate::Result<()> {
    super::guard_action(wb, si, "objects")?;
    let i = find(wb, si, sel)?;
    let (rows, cols) = {
        let s = &wb.sheets[si].shapes[i];
        let end = s.anchor.to.unwrap_or(s.anchor.from);
        (
            s.anchor.from.row.max(end.row) + 200,
            s.anchor.from.col.max(end.col) + 64,
        )
    };
    let g = crate::render::sheet_geometry(wb, &wb.sheets[si], rows, cols);
    let sh: &mut Shape = &mut wb.sheets[si].shapes[i];
    let a = &mut sh.anchor;
    let (x0, y0) = match a.kind {
        AnchorKind::Absolute => a.pos.unwrap_or((0, 0)),
        _ => marker_abs(&g, &a.from),
    };
    let (w0, h0) = match (a.kind, a.to) {
        (AnchorKind::TwoCell, Some(t)) => {
            let (x1, y1) = marker_abs(&g, &t);
            (x1 - x0, y1 - y0)
        }
        _ => a.ext.unwrap_or((0, 0)),
    };
    let (x, y) = (left.map_or(x0, pt_emu), top.map_or(y0, pt_emu));
    let (w, h) = (width.map_or(w0, pt_emu), height.map_or(h0, pt_emu));
    match a.kind {
        AnchorKind::TwoCell => {
            a.from = marker_at(&g, x, y);
            a.to = Some(marker_at(&g, x + w, y + h));
        }
        AnchorKind::OneCell => {
            a.from = marker_at(&g, x, y);
            a.ext = Some((w, h));
        }
        AnchorKind::Absolute => {
            a.pos = Some((x, y));
            a.ext = Some((w, h));
        }
    }
    let (from, to, ext) = (a.from, a.to, a.ext);
    if let Some(raw) = &sh.raw {
        let mut r = super::structure::patch_anchor_xml(raw, &from, to.as_ref());
        if sh.anchor.kind != AnchorKind::TwoCell
            && let Some((cx, cy)) = ext
            && let Some(p) = r.find("<xdr:ext ")
            && let Some(q) = r[p..].find("/>")
        {
            r.replace_range(p..p + q + 2, &format!("<xdr:ext cx=\"{cx}\" cy=\"{cy}\"/>"));
        }
        sh.raw = Some(r);
    }
    Ok(())
}
