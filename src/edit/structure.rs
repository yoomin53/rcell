// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 행·열 삽입·삭제를 문서 전체에 반영한다: 셀, 모든 시트의 수식, 이름, 병합, 조건부 서식·유효성 (범위와
//! 수식), 하이퍼링크, 메모(상자 자리 포함), 도형·차트 앵커(원본 XML 도), 차트 계열 참조(원본 차트 파트도),
//! 표·자동 필터, 행·열 정보, 페이지 나누기, 배열 수식·데이터 표 범위.
//!
//! 삽입한 행·열은 바로 위 행(왼쪽 열)의 서식을 물려받는다 (한셀 대조 실험 — docs/editing.md 3절):
//! 행 정보(높이·행 서식)를 복사하고, 서식이 있는 칸은 빈 칸으로 복사한다. 열은 열 정보를 복사하되 (없으면
//! 기본 너비를 명시한 열 정보를 만든다) 왼쪽 열에 열 서식이 있으면 행 서식이 있는 행의 칸만 복사한다.
//! 조건부 서식·유효성 범위는 바로 위(왼쪽)에서 끝나도 넓어진다.

use super::refs::{self, Area, Shift};
use crate::model::{
    Cell, CellRef, CellValue, ColInfo, FormulaKind, Marker, RowInfo, Shape, Sheet, Workbook,
};

/// 수식의 공유 표시를 지운다 (셀마다 원문이 있으므로 일반 수식으로 쓰면 된다)
pub(crate) fn unshare(sheet: &mut Sheet) {
    for c in &mut sheet.cells {
        if let Some(f) = &mut c.formula
            && f.kind == FormulaKind::Shared
        {
            f.kind = FormulaKind::Normal;
            f.shared_index = None;
            f.range = None;
        }
    }
}

fn area_of(s: &str) -> Option<Area> {
    refs::parse_area(s)
}

/// 범위 글 하나 (`A1:B2`) 를 옮긴다. 모두 지워지면 None
fn shift_range_text(s: &str, op: Shift, adjacent: bool) -> Option<String> {
    match area_of(s) {
        Some(a) => op.area(a, adjacent).map(|b| b.text()),
        None => Some(s.to_string()),
    }
}

/// sqref (공백으로 구분한 범위들)
fn shift_sqref(s: &str, op: Shift, adjacent: bool) -> Vec<Area> {
    refs::parse_sqref(s)
        .into_iter()
        .filter_map(|a| op.area(a, adjacent))
        .collect()
}

fn top_left(list: &[Area]) -> Option<(u32, u32)> {
    list.first().map(|a| {
        let (r0, c0, _, _) = a.bounds();
        (r0, c0)
    })
}

fn marker_shift(m: &mut Marker, op: Shift) {
    let rows = op.on_rows();
    let v = if rows { m.row } else { m.col };
    let (nv, reset) = match op.point(rows, v) {
        Some(n) => (n, false),
        // 지운 행·열 안의 표지는 지운 자리의 시작으로
        None => match op {
            Shift::DelRows { at, .. } | Shift::DelCols { at, .. } => (at, true),
            _ => (v, false),
        },
    };
    if rows {
        m.row = nv;
        if reset {
            m.row_off = 0;
        }
    } else {
        m.col = nv;
        if reset {
            m.col_off = 0;
        }
    }
}

fn marker_xml(prefix: &str, tag: &str, m: &Marker) -> String {
    format!(
        "<{prefix}{tag}><{prefix}col>{}</{prefix}col><{prefix}colOff>{}</{prefix}colOff><{prefix}row>{}</{prefix}row><{prefix}rowOff>{}</{prefix}rowOff></{prefix}{tag}>",
        m.col, m.col_off, m.row, m.row_off
    )
}

/// 원본 앵커 XML 의 `from`/`to` 표지를 바꾼다 (`mc:AlternateContent` 안의 두 갈래 모두)
pub(crate) fn patch_anchor_xml(raw: &str, from: &Marker, to: Option<&Marker>) -> String {
    let mut out = raw.to_string();
    for (tag, m) in [("from", Some(from)), ("to", to)] {
        let Some(m) = m else { continue };
        let mut pos = 0;
        while let Some(k) = out[pos..].find(&format!(":{tag}>")).map(|k| k + pos) {
            // 여는 태그 `<xdr:from>` 인지 (닫는 태그 `</xdr:from>` 가 아니라)
            let open = out[..k].rfind('<').unwrap_or(0);
            let name = &out[open + 1..k];
            if name.starts_with('/') || name.contains(|c: char| c.is_whitespace() || c == '>') {
                pos = k + 1;
                continue;
            }
            let prefix = format!("{name}:");
            let close = format!("</{prefix}{tag}>");
            let Some(end) = out[k..].find(&close).map(|e| e + k + close.len()) else {
                break;
            };
            let new = marker_xml(&prefix, tag, m);
            out.replace_range(open..end, &new);
            pos = open + new.len();
        }
    }
    out
}

/// 열 너비·행 높이가 바뀐 칸 안의 도형 표지 오프셋을 같은 비율로 늘이거나 줄인다 — 한셀은 오프셋을 px 로 올리고
/// 칸 크기(px)에서 1px 을 뺀 길이의 비율로 옮겨 px 로 반올림한다 (열 너비·행 높이 대조 실험: 28px 열을 131px 로 넓히면 25px 표지가
/// 25 × 130/27 → 120px, 40px 행을 60px 로 높이면 25px 표지가 38px). `first` 부터 `old`·`new` (px) 개수만큼의
/// 행(`rows`) 또는 열
pub(crate) fn scale_offsets(sheet: &mut Sheet, rows: bool, first: u32, old: &[f64], new: &[f64]) {
    let scale = |m: &mut Marker| -> bool {
        let idx = if rows { m.row } else { m.col };
        let Some(k) = idx.checked_sub(first).map(|k| k as usize) else {
            return false;
        };
        let (Some(&o), Some(&n)) = (old.get(k), new.get(k)) else {
            return false;
        };
        if o <= 1.0 || o == n {
            return false;
        }
        const EMU: i64 = crate::units::EMU_PER_PX;
        let off = if rows { &mut m.row_off } else { &mut m.col_off };
        // 1px 보다 작은 EMU 는 한셀처럼 올려 px 로 (12345 EMU → 2px) 한 뒤 비율로 옮긴다
        let px = ((*off).max(0) as u64).div_ceil(EMU as u64) as f64;
        let v = (px * (n - 1.0).max(0.0) / (o - 1.0)).round() as i64 * EMU;
        let changed = v != *off;
        *off = v;
        changed
    };
    for sh in &mut sheet.shapes {
        let a = &mut sh.anchor;
        if a.kind == crate::model::AnchorKind::Absolute {
            continue;
        }
        let mut changed = scale(&mut a.from);
        if let Some(to) = &mut a.to {
            changed |= scale(to);
        }
        if changed && let Some(raw) = &sh.raw {
            sh.raw = Some(patch_anchor_xml(raw, &a.from, a.to.as_ref()));
        }
    }
}

/// `editAs="oneCell"` 도형(칸과 함께 옮기되 크기는 그대로)의 크기 (EMU). 열 너비·행 높이를 바꾸기 전에 잰다
pub(crate) fn one_cell_sizes(
    sheet: &Sheet,
    g: &crate::render::Geometry,
) -> Vec<Option<(i64, i64)>> {
    sheet
        .shapes
        .iter()
        .map(|sh| {
            let a = &sh.anchor;
            // 한셀은 그림만 크기를 지킨다 (글상자 도형은 끝 표지를 그대로 둔다 — 편집 대조 실험)
            if a.kind != crate::model::AnchorKind::TwoCell
                || a.edit_as.as_deref() != Some("oneCell")
                || sh.kind != crate::model::ShapeKind::Picture
            {
                return None;
            }
            let to = a.to?;
            let (fx, fy) = marker_abs(g, &a.from);
            let (tx, ty) = marker_abs(g, &to);
            Some((tx - fx, ty - fy))
        })
        .collect()
}

/// 열 너비·행 높이를 바꾼 뒤 `editAs="oneCell"` 도형의 끝 표지를 크기에 맞춰 다시 정한다 — 한셀은 그림의 크기를
/// 그대로 둔다 (4행을 높이면 그림 끝이 8행+10px 에서 7행+21px 로, 편집 대조 실험)
pub(crate) fn keep_one_cell_sizes(
    sheet: &mut Sheet,
    sizes: &[Option<(i64, i64)>],
    g: &crate::render::Geometry,
) {
    const EMU: i64 = crate::units::EMU_PER_PX;
    let locate = |list: &[(u32, u32)], pos: i64| -> (u32, i64) {
        for (i, &(l, w)) in list.iter().enumerate() {
            let (a, b) = (l as i64 * EMU, (l + w) as i64 * EMU);
            if pos >= a && pos < b {
                return (i as u32, pos - a);
            }
        }
        match list.last() {
            Some(&(l, _)) => ((list.len() - 1) as u32, pos - l as i64 * EMU),
            None => (0, pos),
        }
    };
    for (sh, size) in sheet.shapes.iter_mut().zip(sizes) {
        let Some((w, h)) = *size else { continue };
        let (fx, fy) = marker_abs(g, &sh.anchor.from);
        let (col, col_off) = locate(&g.cols, fx + w);
        let (row, row_off) = locate(&g.rows, fy + h);
        let to = Marker {
            col,
            col_off,
            row,
            row_off,
        };
        if sh.anchor.to != Some(to) {
            sh.anchor.to = Some(to);
            if let Some(raw) = &sh.raw {
                sh.raw = Some(patch_anchor_xml(
                    raw,
                    &sh.anchor.from,
                    sh.anchor.to.as_ref(),
                ));
            }
        }
    }
}

/// 행·열을 숨긴 뒤 크기를 지키는 그림의 시작 표지: 칸 첫머리(오프셋 0)에서 시작하고 바로 앞 칸들이 숨겨져(폭 0)
/// 있으면 그 맨 앞 숨긴 칸으로 옮긴다 (한셀 — G2 의 그림은 F 열을 숨기면 F2+0 에서 시작)
pub(crate) fn snap_from_hidden(
    sheet: &mut Sheet,
    sizes: &[Option<(i64, i64)>],
    g: &crate::render::Geometry,
) {
    let back = |list: &[(u32, u32)], mut i: u32| -> u32 {
        while i > 0 && list.get(i as usize - 1).is_some_and(|x| x.1 == 0) {
            i -= 1;
        }
        i
    };
    for (sh, size) in sheet.shapes.iter_mut().zip(sizes) {
        if size.is_none() {
            continue;
        }
        let mut from = sh.anchor.from;
        if from.col_off == 0 {
            from.col = back(&g.cols, from.col);
        }
        if from.row_off == 0 {
            from.row = back(&g.rows, from.row);
        }
        if from != sh.anchor.from {
            sh.anchor.from = from;
            if let Some(raw) = &sh.raw {
                sh.raw = Some(patch_anchor_xml(
                    raw,
                    &sh.anchor.from,
                    sh.anchor.to.as_ref(),
                ));
            }
        }
    }
}

/// 열 너비·행 높이를 바꾼 뒤 메모 상자를 화면의 같은 자리에 둔다 — 셀과 함께 옮기지 않는 메모 상자(한셀 기본)는
/// 절대 자리를 지킨다 (4행을 높이면 상자 위가 7행+10px 에서 5행+21px 로, 편집 대조 실험)
pub(crate) fn keep_comment_boxes(
    sheet: &mut Sheet,
    old: &crate::render::Geometry,
    new: &crate::render::Geometry,
) {
    let abs = |list: &[(u32, u32)], i: u32, off: u32| -> i64 {
        list.get(i as usize).map_or(0, |p| p.0 as i64) + off as i64
    };
    let locate = |list: &[(u32, u32)], pos: i64| -> (u32, u32) {
        for (i, &(l, w)) in list.iter().enumerate() {
            if pos >= l as i64 && pos < (l + w) as i64 {
                return (i as u32, (pos - l as i64) as u32);
            }
        }
        match list.last() {
            Some(&(l, _)) => ((list.len() - 1) as u32, (pos - l as i64).max(0) as u32),
            None => (0, pos.max(0) as u32),
        }
    };
    for c in &mut sheet.comments {
        let Some(s) = &mut c.shape else { continue };
        if s.move_with_cells {
            continue;
        }
        let a = s.anchor;
        let (lc, lo) = locate(&new.cols, abs(&old.cols, a[0], a[1]));
        let (tr, to) = locate(&new.rows, abs(&old.rows, a[2], a[3]));
        let (rc, ro) = locate(&new.cols, abs(&old.cols, a[4], a[5]));
        let (br, bo) = locate(&new.rows, abs(&old.rows, a[6], a[7]));
        s.anchor = [lc, lo, tr, to, rc, ro, br, bo];
    }
}

/// 화면 px 자리 → (칸, 칸 안 px)
pub(crate) fn locate_px(list: &[(u32, u32)], pos: i64) -> (u32, u32) {
    for (i, &(l, w)) in list.iter().enumerate() {
        if pos >= l as i64 && pos < (l + w) as i64 {
            return (i as u32, (pos - l as i64) as u32);
        }
    }
    match list.last() {
        Some(&(l, _)) => ((list.len() - 1) as u32, (pos - l as i64).max(0) as u32),
        None => (0, pos.max(0) as u32),
    }
}

/// (칸, 칸 안 px) → 화면 px (칸 안 px 는 칸 너비까지)
pub(crate) fn at_px(list: &[(u32, u32)], i: u32, off: u32) -> i64 {
    match list.get(i as usize) {
        Some(&(l, w)) => l as i64 + off.min(w) as i64,
        None => list.last().map_or(0, |&(l, w)| (l + w) as i64) + off as i64,
    }
}

/// 메모 상자를 다시 정할 때 쓰는 배치 (상자가 있는 메모가 없으면 None)
pub(crate) fn comment_geometry(
    wb: &Workbook,
    si: usize,
    extra: u32,
) -> Option<(crate::render::Geometry, (u32, u32))> {
    let sheet = &wb.sheets[si];
    if !sheet.comments.iter().any(|c| c.shape.is_some()) {
        return None;
    }
    let (r, c) = comments_extent(sheet);
    let (cr, cc) = sheet
        .comments
        .iter()
        .fold((0, 0), |(r, c), x| (r.max(x.cell.row), c.max(x.cell.col)));
    let ext = (r.max(cr) + 64 + extra, c.max(cc) + 32 + extra);
    Some((crate::render::sheet_geometry(wb, sheet, ext.0, ext.1), ext))
}

/// 메모 상자 앵커의 열 배치: 한셀 VML 앵커의 열 px 는 화면 너비보다 1 작다 (기본 너비 54pt = 72px 열이 71)
pub(crate) fn anchor_cols(g: &crate::render::Geometry) -> Vec<(u32, u32)> {
    let mut x = 0;
    g.cols
        .iter()
        .map(|&(_, w)| {
            let w = w.saturating_sub(1);
            let c = (x, w);
            x += w;
            c
        })
        .collect()
}

/// 행·열 삽입·삭제로 칸이 옮겨진 메모의 상자 (메모 상자 대조 실험). 한셀은 상자 왼쪽 위를 메모 칸의 열 + 14px,
/// 두 행 위 + 10px 에 다시 두고 크기(px)는 그대로 둔다 — 기본 자리(오른쪽 열, 한 행 위)의 상자는 처음 옮길 때
/// 한 열 왼쪽·한 행 위로 간다. 다만 옛 상자 왼쪽이 A 열에 있으면 그 왼쪽을 A 열 너비 − 1px 만큼 오른쪽에서 잰다:
/// A 열 메모의 상자는 칸이 옮겨질 때마다 그만큼 좁아진다 (A10 메모는 행 삽입 세 번이면 오른쪽 끝이 A 열 처음).
/// 열 너비가 기본이 아닌 열에서는 한셀의 오프셋이 1px 다를 수 있다 (docs/editing.md 11절)
pub(crate) fn moved_comment_box(
    old: &crate::render::Geometry,
    new: &crate::render::Geometry,
    a: [u32; 8],
    cell: CellRef,
    cap: bool,
) -> [u32; 8] {
    let (oc, nc) = (anchor_cols(old), anchor_cols(new));
    let left = if a[0] == 0 {
        oc.first().map_or(0, |c| c.1 as i64 - 1) + a[1] as i64
    } else {
        at_px(&oc, a[0], a[1])
    };
    let width = at_px(&oc, a[4], a[5]) - left;
    let height = at_px(&old.rows, a[6], a[7]) - at_px(&old.rows, a[2], a[3]);
    let (lc, lo) = (cell.col, 14);
    let (tr, to) = (up_visible(&new.rows, cell.row, 2), 10);
    // A 열에서 시작한 상자의 오른쪽 끝은 새 시작 열의 오른쪽 가장자리 − 1px 를 넘지 못한다 (한셀 — 옛 끝이 69 든 70 이든
    // 옮기면 `70`, 다시 옮기면 `0`; 대조 실험 cb1·cb2)
    let mut end = (at_px(&nc, lc, lo) + width).max(0);
    if cap
        && a[0] == 0
        && let Some(&(l, w)) = nc.get(lc as usize)
    {
        end = end.min(l as i64 + w as i64 - 1);
    }
    let (rc, ro) = locate_end_col(&nc, end);
    let (br, bo) = locate_px(&new.rows, (at_px(&new.rows, tr, to) + height).max(0));
    [lc, lo, tr, to, rc, ro, br, bo]
}

/// 끝 열 자리: 열 경계에 닿으면 앞 열의 끝(열 너비)으로 적는다 (한셀 — 끝이 C 열 너비에 닿으면 `2, 70`, 행은 다음 행 처음, cb1)
fn locate_end_col(list: &[(u32, u32)], pos: i64) -> (u32, u32) {
    for (i, &(l, w)) in list.iter().enumerate() {
        if pos > l as i64 && pos <= (l + w) as i64 {
            return (i as u32, (pos - l as i64) as u32);
        }
    }
    locate_px(list, pos)
}

/// 메모 상자 끝 제한(A 열에서 시작한 상자)을 쓰는가: 화면 배율이 100 일 때만 맞는다 (다른 배율은 1px 씩 달라진다, docs/editing.md 11절)
pub(crate) fn box_cap(sheet: &Sheet) -> bool {
    sheet.view.zoom_scale.unwrap_or(100) == 100
}

/// 보이는 행 `n` 개만큼 위의 행 (숨긴 행은 세지 않고 건너뛴다 — 한셀의 메모 상자 자리, 대조 실험 cb1)
pub(crate) fn up_visible(rows: &[(u32, u32)], row: u32, n: u32) -> u32 {
    let (mut r, mut k) = (row, 0);
    while r > 0 && k < n {
        r -= 1;
        if rows.get(r as usize).is_none_or(|x| x.1 > 0) {
            k += 1;
        }
    }
    r
}

/// 메모 상자가 닿는 가장 큰 행·열
pub(crate) fn comments_extent(sheet: &Sheet) -> (u32, u32) {
    let mut r = 0;
    let mut c = 0;
    for x in &sheet.comments {
        if let Some(s) = &x.shape {
            r = r.max(s.anchor[6]).max(s.anchor[2]);
            c = c.max(s.anchor[4]).max(s.anchor[0]);
        }
    }
    (r, c)
}

/// 표지의 시트 안 절대 자리 (EMU)
fn marker_abs(g: &crate::render::Geometry, m: &Marker) -> (i64, i64) {
    const EMU: i64 = crate::units::EMU_PER_PX;
    let x = g.cols.get(m.col as usize).map_or(0, |c| c.0 as i64) * EMU + m.col_off;
    let y = g.rows.get(m.row as usize).map_or(0, |r| r.0 as i64) * EMU + m.row_off;
    (x, y)
}

/// 도형 표지가 닿는 가장 큰 행·열 (배치를 잴 범위)
pub(crate) fn shapes_extent(sheet: &Sheet) -> (u32, u32) {
    let mut r = 0;
    let mut c = 0;
    for sh in &sheet.shapes {
        for m in [Some(sh.anchor.from), sh.anchor.to].into_iter().flatten() {
            r = r.max(m.row);
            c = c.max(m.col);
        }
    }
    (r, c)
}

/// 두 칸 앵커 도형이 지운 행·열 안에 모두 드는가 (끝 표지가 지운 범위 바로 다음 칸의 처음이어도 든다)
fn inside_deleted(sh: &Shape, op: Shift) -> bool {
    use crate::model::AnchorKind;
    let a = &sh.anchor;
    if a.kind != AnchorKind::TwoCell || matches!(a.edit_as.as_deref(), Some("oneCell" | "absolute"))
    {
        return false;
    }
    let Some(to) = a.to else { return false };
    match op {
        Shift::DelRows { at, n } => {
            a.from.row >= at && (to.row < at + n || (to.row == at + n && to.row_off == 0))
        }
        Shift::DelCols { at, n } => {
            a.from.col >= at && (to.col < at + n || (to.col == at + n && to.col_off == 0))
        }
        _ => false,
    }
}

/// 도형 앵커를 옮긴다: 두 칸 앵커(기본)는 양 끝을 따로, `oneCell` 은 시작 칸만큼 함께, `absolute` 는 그대로
pub(crate) fn shift_shape(sh: &mut Shape, op: Shift) {
    use crate::model::AnchorKind;
    let a = &mut sh.anchor;
    if a.kind == AnchorKind::Absolute || a.edit_as.as_deref() == Some("absolute") {
        return;
    }
    let before = a.from;
    marker_shift(&mut a.from, op);
    if let Some(to) = &mut a.to {
        if a.edit_as.as_deref() == Some("oneCell") {
            let dr = a.from.row as i64 - before.row as i64;
            let dc = a.from.col as i64 - before.col as i64;
            to.row = (to.row as i64 + dr).max(0) as u32;
            to.col = (to.col as i64 + dc).max(0) as u32;
        } else {
            marker_shift(to, op);
        }
    }
    if let Some(raw) = &sh.raw {
        sh.raw = Some(patch_anchor_xml(raw, &a.from, a.to.as_ref()));
    }
}

/// 차트 파트 원본 XML 의 `<c:f>` 수식을 모두 `f` 로 바꾼다
pub(crate) fn patch_chart_formulas(data: &[u8], f: &dyn Fn(&str) -> String) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(data).ok()?;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut changed = false;
    while let Some(k) = rest.find("<c:f>") {
        out.push_str(&rest[..k + 5]);
        rest = &rest[k + 5..];
        let Some(e) = rest.find("</c:f>") else { break };
        let old = unescape(&rest[..e]);
        let new = f(&old);
        if new != old {
            changed = true;
            out.push_str(&crate::writer::esc(&new));
        } else {
            out.push_str(&rest[..e]);
        }
        rest = &rest[e..];
    }
    out.push_str(rest);
    changed.then(|| out.into_bytes())
}

/// XML 글의 엔티티를 푼다
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(k) = rest.find('&') {
        out.push_str(&rest[..k]);
        rest = &rest[k..];
        let Some(e) = rest.find(';') else { break };
        let ent = &rest[1..e];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\x27'),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 차트 모델의 참조 수식을 모두 바꾼다
pub(crate) fn map_chart(chart: &mut crate::model::Chart, f: &dyn Fn(&str) -> String) {
    use crate::model::{ChartData, ChartTx};
    let fix = |d: &mut Option<ChartData>| {
        if let Some(d) = d
            && let Some(x) = &mut d.formula
        {
            *x = f(x);
        }
    };
    let fix_tx = |t: &mut Option<ChartTx>| {
        if let Some(ChartTx::Ref(d)) = t
            && let Some(x) = &mut d.formula
        {
            *x = f(x);
        }
    };
    if let Some(t) = &mut chart.title_full {
        fix_tx(&mut t.tx);
    }
    for ax in &mut chart.axes {
        if let Some(t) = &mut ax.title {
            fix_tx(&mut t.tx);
        }
    }
    let series = chart.series.iter_mut().chain(
        chart
            .more_groups
            .iter_mut()
            .flat_map(|g| g.series.iter_mut()),
    );
    for s in series {
        fix_tx(&mut s.tx);
        fix(&mut s.cat);
        fix(&mut s.val);
        fix(&mut s.bubble_size);
        s.refresh_summary();
    }
}

/// 모든 차트(모델과 원본 차트 파트)의 참조를 바꾼다
pub(crate) fn map_all_charts(wb: &mut Workbook, f: &dyn Fn(&str) -> String) {
    fn walk(list: &mut [Shape], f: &dyn Fn(&str) -> String, parts: &mut Vec<String>) {
        for s in list {
            if let Some(c) = &mut s.chart {
                map_chart(c, f);
            }
            if let Some(t) = &s.target {
                parts.push(t.clone());
            }
            walk(&mut s.children, f, parts);
        }
    }
    let mut parts = Vec::new();
    for sh in &mut wb.sheets {
        walk(&mut sh.shapes, f, &mut parts);
    }
    for p in &mut wb.raw_parts {
        if parts.contains(&p.name)
            && p.name.contains("charts/")
            && let Some(d) = patch_chart_formulas(&p.data, f)
        {
            p.data = d;
        }
    }
}

/// 수식이 있는 모든 곳(셀·이름·조건부 서식·유효성·차트)에 `f(수식, 수식이 있는 시트)` 를 적용한다.
/// 조건부 서식·유효성은 범위가 바뀌지 않는 편집(시트 이름 바꾸기·삭제)에만 쓴다
pub(crate) fn map_all_formulas(wb: &mut Workbook, f: &dyn Fn(&str, &str) -> String) {
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    for (si, sh) in wb.sheets.iter_mut().enumerate() {
        let here = names[si].as_str();
        let mut changed = false;
        for c in &mut sh.cells {
            if let Some(fm) = &mut c.formula
                && !fm.text.is_empty()
            {
                let t = f(&fm.text, here);
                if t != fm.text {
                    fm.text = t;
                    changed = true;
                }
            }
        }
        if changed {
            unshare(sh);
        }
        for cf in &mut sh.conditional_formats {
            for r in &mut cf.rules {
                for x in &mut r.formulas {
                    *x = f(x, here);
                }
            }
        }
        for dv in &mut sh.data_validations {
            for x in [&mut dv.formula1, &mut dv.formula2].into_iter().flatten() {
                *x = f(x, here);
            }
        }
        for h in &mut sh.hyperlinks {
            if let Some(l) = &mut h.location {
                *l = f(l, here);
            }
        }
    }
    for d in &mut wb.defined_names {
        let here = d
            .local_sheet_id
            .and_then(|i| names.get(i as usize))
            .map(String::as_str)
            .unwrap_or("");
        d.formula = f(&d.formula, here);
    }
    map_all_charts(wb, &|x| f(x, ""));
}

/// 표의 원본 파트를 버리고 모델로 쓰게 한다 (범위가 바뀐 표)
fn detach_table(wb_parts: &mut Vec<crate::model::RawPart>, t: &mut crate::model::Table) {
    if let Some(p) = t.part.take() {
        let rels = crate::package::rels_part_for(&p);
        wb_parts.retain(|x| x.name != p && x.name != rels);
    }
}

/// 시트 끝을 넘어 밀려나는 값이 있는가 (한셀은 이런 삽입을 거부한다)
fn would_push_out(sheet: &Sheet, op: Shift) -> bool {
    match op {
        Shift::InsRows { at, n } => sheet.cells.iter().any(|c| {
            c.r.row >= at
                && c.r.row + n >= refs::MAX_ROWS
                && (!c.value.is_empty() || c.formula.is_some())
        }),
        Shift::InsCols { at, n } => sheet.cells.iter().any(|c| {
            c.r.col >= at
                && c.r.col + n >= refs::MAX_COLS
                && (!c.value.is_empty() || c.formula.is_some())
        }),
        _ => false,
    }
}

/// 열 정보의 열 서식 (0부터 센 열, 열 정보가 없으면 0)
fn col_style(sheet: &Sheet, c: u32) -> u32 {
    sheet
        .cols
        .iter()
        .find(|ci| ci.min <= c + 1 && c < ci.max)
        .and_then(|ci| ci.style)
        .unwrap_or(0)
}

/// 열을 넣거나 지울 때 행 서식이 있는 행에 만드는 셀 (옮기기 전 좌표). 한셀은 마지막 셀이 있는 행(지울 열의 셀은
/// 빼고)까지, 행 서식이 있는 행마다 열 정보가 옮겨지는 칸에 그 칸의 열 서식(열 정보가 없으면 0)으로 셀을 만든다
/// — 삽입은 삽입 위치부터 열 정보가 있는 열과 그 n 칸 앞 열, 삭제는 지운 범위 뒤의 열 정보가 있는 열과 그 n 칸 뒤
/// 열 (열 삽입·삭제 대조 실험: E:J 열 서식, C 열 삽입 → 옛 D:J, G 열에 3칸 삽입 → 옛 D:J, C:D 삭제 → 옛 E:L).
/// 행 서식과 같은 서식, 이미 있는 셀은 그대로 둔다. 그래서 그 칸들은 행 서식 대신 열 서식으로 보인다
fn row_style_cells(sheet: &Sheet, op: Shift) -> Vec<Cell> {
    let (at, n, ins) = match op {
        Shift::InsCols { at, n } => (at, n, true),
        Shift::DelCols { at, n } => (at, n, false),
        _ => return Vec::new(),
    };
    let max = refs::MAX_COLS - 1;
    let last = sheet
        .cells
        .iter()
        .filter(|c| ins || c.r.col < at || c.r.col >= at + n)
        .map(|c| c.r.row)
        .max();
    let Some(last) = last else {
        return Vec::new();
    };
    let styled: Vec<(u32, u32)> = sheet
        .rows
        .iter()
        .filter_map(|r| Some((r.index - 1, r.style?)))
        .filter(|&(i, _)| i <= last)
        .collect();
    if styled.is_empty() {
        return Vec::new();
    }
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for ci in &sheet.cols {
        let (a, b) = (ci.min.max(1) - 1, (ci.max.max(1) - 1).min(max));
        if ins {
            if b < at {
                continue;
            }
            let a = a.max(at);
            spans.push((a, b));
            if b >= n {
                spans.push((a.max(n) - n, b - n));
            }
        } else {
            let a = a.max(at + n);
            if a > b {
                continue;
            }
            spans.push((a, b));
            if a + n <= max {
                spans.push((a + n, (b + n).min(max)));
            }
        }
    }
    spans.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (a, b) in spans {
        match merged.last_mut() {
            Some(l) if a <= l.1 + 1 => l.1 = l.1.max(b),
            _ => merged.push((a, b)),
        }
    }
    let cols: Vec<(u32, u32)> = merged
        .iter()
        .flat_map(|&(a, b)| a..=b)
        .map(|c| (c, col_style(sheet, c)))
        .collect();
    let mut out = Vec::new();
    for (row, s) in styled {
        for &(col, cs) in &cols {
            if cs != s && sheet.cell(CellRef { row, col }).is_none() {
                out.push(Cell {
                    r: CellRef { row, col },
                    style: cs,
                    value: CellValue::Empty,
                    formula: None,
                });
            }
        }
    }
    out
}

/// 삽입한 행·열이 물려받는 서식: 앞 칸의 서식에서 맞은편 칸(삽입 위치에 있던 칸)과 같은 선이 아닌 변은 지운다
/// (지운 변은 기본 테두리의 그 변). 한셀 대조 실험: 아래 선이 있는 B2 아래에 행을 넣으면 옛 B3 에도 같은 아래 선이
/// 있을 때만 새 B3 이 아래 선을 받는다. 열도 같다
fn inherited_style(
    styles: &mut crate::model::Styles,
    cache: &mut std::collections::HashMap<(u32, u32), u32>,
    src: u32,
    far: u32,
) -> u32 {
    if let Some(&s) = cache.get(&(src, far)) {
        return s;
    }
    let border = |st: &crate::model::Styles, i: u32| {
        st.xf(i)
            .and_then(|x| st.borders.get(x.border_id as usize))
            .cloned()
            .unwrap_or_default()
    };
    let mut b = border(styles, src);
    let fb = border(styles, far);
    let base = styles.borders.first().cloned().unwrap_or_default();
    let keep = |own: &Option<crate::model::BorderEdge>, far: &Option<crate::model::BorderEdge>| {
        !super::is_line(own.as_ref())
            || matches!((own, far), (Some(x), Some(y)) if super::is_line(Some(y)) && super::same_line(x, y))
    };
    let mut changed = false;
    for (own, far, def) in [
        (&mut b.left, &fb.left, &base.left),
        (&mut b.right, &fb.right, &base.right),
        (&mut b.top, &fb.top, &base.top),
        (&mut b.bottom, &fb.bottom, &base.bottom),
    ] {
        if !keep(own, far) {
            *own = def.clone();
            changed = true;
        }
    }
    if !keep(&b.diagonal, &fb.diagonal)
        || (super::is_line(b.diagonal.as_ref())
            && (b.diagonal_up != fb.diagonal_up || b.diagonal_down != fb.diagonal_down))
    {
        b.diagonal = base.diagonal.clone();
        b.diagonal_up = false;
        b.diagonal_down = false;
        changed = true;
    }
    let out = match styles.xf(src).cloned() {
        Some(mut xf) if changed => {
            xf.border_id = super::style::intern_border(styles, b);
            super::style::intern_xf(styles, xf)
        }
        _ => src,
    };
    cache.insert((src, far), out);
    out
}

/// 삽입한 행·열에 만들 셀 (옮긴 뒤 좌표, 서식): 앞 행(열)의 셀마다 물려받는 서식을 정하고, 새 칸이 셀 없이도
/// 그 서식으로 보이면(행 서식 → 열 서식) 만들지 않는다. `sheet` 는 옮기기 전
fn inherited_cells(
    styles: &mut crate::model::Styles,
    sheet: &Sheet,
    op: Shift,
) -> Vec<(u32, u32, u32)> {
    let (at, n, on_rows) = match op {
        Shift::InsRows { at, n } if at > 0 => (at, n, true),
        Shift::InsCols { at, n } if at > 0 => (at, n, false),
        _ => return Vec::new(),
    };
    let row_style: std::collections::HashMap<u32, u32> = sheet
        .rows
        .iter()
        .filter_map(|r| Some((r.index - 1, r.style?)))
        .collect();
    let src = at - 1;
    let mut cache = std::collections::HashMap::new();
    let mut out = Vec::new();
    for c in &sheet.cells {
        let (line, other) = if on_rows {
            (c.r.row, c.r.col)
        } else {
            (c.r.col, c.r.row)
        };
        if line != src {
            continue;
        }
        let far = if on_rows {
            CellRef {
                row: at,
                col: other,
            }
        } else {
            CellRef {
                row: other,
                col: at,
            }
        };
        let st = inherited_style(styles, &mut cache, c.style, super::style_at(sheet, far));
        // 새 행은 앞 행의 행 정보를, 새 열은 앞 열의 열 정보를 받는다
        let row = if on_rows { src } else { other };
        let default = row_style
            .get(&row)
            .copied()
            .unwrap_or_else(|| col_style(sheet, if on_rows { other } else { src }));
        if st == default {
            continue;
        }
        let end = (at + n).min(if on_rows {
            refs::MAX_ROWS
        } else {
            refs::MAX_COLS
        });
        for k in at..end {
            out.push(if on_rows {
                (k, other, st)
            } else {
                (other, k, st)
            });
        }
    }
    out
}

/// 서식의 변 몇 개를 바꾼 서식 (선이 없으면 기본 테두리의 그 변)
fn with_sides(
    styles: &mut crate::model::Styles,
    style: u32,
    sides: &[(super::Side, Option<crate::model::BorderEdge>)],
) -> u32 {
    let Some(mut xf) = styles.xf(style).cloned() else {
        return style;
    };
    let mut b = styles
        .borders
        .get(xf.border_id as usize)
        .cloned()
        .unwrap_or_default();
    let base = styles.borders.first().cloned().unwrap_or_default();
    let before = format!("{b:?}");
    for (side, line) in sides {
        let (slot, def) = match side {
            super::Side::Left => (&mut b.left, &base.left),
            super::Side::Right => (&mut b.right, &base.right),
            super::Side::Top => (&mut b.top, &base.top),
            super::Side::Bottom => (&mut b.bottom, &base.bottom),
        };
        *slot = line.clone().or_else(|| def.clone());
    }
    if format!("{b:?}") == before {
        return style;
    }
    xf.border_id = super::style::intern_border(styles, b);
    super::style::intern_xf(styles, xf)
}

/// 선의 굵기 순위 (가는 선 2, 보통 3, 굵은 선·이중선 4)
fn line_weight(e: &crate::model::BorderEdge) -> u8 {
    match e.style.as_deref() {
        Some("hair") => 1,
        Some("medium" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" | "slantDashDot") => {
            3
        }
        Some("thick" | "double") => 4,
        _ => 2,
    }
}

/// 칸 서식의 한 변의 선 (선이 없으면 None)
fn edge_of(
    sheet: &Sheet,
    styles: &crate::model::Styles,
    at: CellRef,
    side: super::Side,
) -> Option<crate::model::BorderEdge> {
    styles
        .xf(super::style_at(sheet, at))
        .and_then(|x| styles.borders.get(x.border_id as usize))
        .and_then(|b| side.of(b))
        .filter(|e| super::is_line(Some(e)))
        .cloned()
}

/// 이웃 칸의 맞닿은 선
fn facing(
    sheet: &Sheet,
    styles: &crate::model::Styles,
    at: CellRef,
    side: super::Side,
) -> Option<crate::model::BorderEdge> {
    side.step(at)
        .and_then(|q| edge_of(sheet, styles, q, side.opposite()))
}

/// 후보 선 가운데 가장 굵은 것 (같은 굵기면 앞의 것)
fn pick_line(
    cands: impl IntoIterator<Item = Option<crate::model::BorderEdge>>,
) -> Option<crate::model::BorderEdge> {
    let mut best: Option<crate::model::BorderEdge> = None;
    for c in cands.into_iter().flatten() {
        if best
            .as_ref()
            .is_none_or(|b| line_weight(&c) > line_weight(b))
        {
            best = Some(c);
        }
    }
    best
}

/// 행·열을 지워 병합이 줄 때 한셀이 고치는 칸 (옮기기 전 좌표): 칸마다 새 서식, 그리고 왼쪽 위 칸의 값을 옮길 자리.
/// 병합 대조 실험 (B2:C2 병합에서 열 하나를 지우는 경우 등). 변에 적는 선은 후보 가운데 가장 굵은 선(같으면 앞의
/// 후보)이고, 지운 쪽 변은 열을 지우면 왼쪽, 행을 지우면 위다:
/// - 한 칸만 남으면 그 칸은 왼쪽 위 칸의 서식을 받고, 열을 지우면 왼쪽·위·아래 변, 행을 지우면 위·왼쪽·오른쪽 변을
///   다시 적는다. 후보는 왼쪽 위 칸이 남으면 (그 칸의 선, 이웃의 맞닿은 선), 지웠으면 (이웃의 맞닿은 선, 남는 칸의
///   선)이고, 지웠을 때는 지운 자리 앞 이웃의 맞닿은 선이 새 선과 다르면 지운다
/// - 여러 칸이 남고 왼쪽 위 칸을 지웠으면 값은 남는 첫 칸으로 가고, 병합의 칸 서식은 병합의 처음부터 남는 칸 수만큼
///   그 자리에 남는다 (B2:D2 에서 B 열을 지우면 새 B2:C2 는 옛 B2·C2 의 서식). 둘레의 변(지운 쪽 변 포함)에는 (이웃의
///   맞닿은 선, 남는 칸의 선)을 적는다
/// - 왼쪽 위 칸이 남고 여러 칸이 남으면 그대로 둔다
type MergeFix = (Vec<(CellRef, u32)>, Vec<(CellRef, CellRef)>);
fn merge_fixes(styles: &mut crate::model::Styles, sheet: &Sheet, op: Shift) -> MergeFix {
    use super::Side;
    let (at, n, rows) = match op {
        Shift::DelRows { at, n } => (at, n, true),
        Shift::DelCols { at, n } => (at, n, false),
        _ => return (Vec::new(), Vec::new()),
    };
    let gone = |v: u32| v >= at && v < at + n;
    let pos = |line: u32, other: u32| {
        if rows {
            CellRef {
                row: line,
                col: other,
            }
        } else {
            CellRef {
                row: other,
                col: line,
            }
        }
    };
    // 지운 쪽 변, 맞은편 변, 다른 축의 두 변
    let (low, high, side_a, side_b) = if rows {
        (Side::Top, Side::Bottom, Side::Left, Side::Right)
    } else {
        (Side::Left, Side::Right, Side::Top, Side::Bottom)
    };
    let mut sets = Vec::new();
    let mut moves = Vec::new();
    for m in &sheet.merges {
        let (lo, hi, olo, ohi) = if rows {
            (m.start.row, m.end.row, m.start.col, m.end.col)
        } else {
            (m.start.col, m.end.col, m.start.row, m.end.row)
        };
        let keep: Vec<u32> = (lo..=hi).filter(|&v| !gone(v)).collect();
        if keep.len() as u32 == hi - lo + 1 || keep.is_empty() {
            continue;
        }
        let anchor_gone = gone(lo);
        let first = keep[0];
        if keep.len() == 1 && olo == ohi {
            let s = pos(first, olo);
            let base = super::style_at(sheet, m.start);
            let mut lines = Vec::new();
            for sd in [low, side_a, side_b] {
                let line = if anchor_gone {
                    pick_line([facing(sheet, styles, s, sd), edge_of(sheet, styles, s, sd)])
                } else {
                    pick_line([
                        edge_of(sheet, styles, m.start, sd),
                        facing(sheet, styles, s, sd),
                    ])
                };
                lines.push((sd, line));
            }
            let st = with_sides(styles, base, &lines);
            sets.push((s, st));
            if anchor_gone {
                moves.push((m.start, s));
                if at > 0 {
                    let nb = pos(at - 1, olo);
                    let face = low.opposite();
                    let their = edge_of(sheet, styles, nb, face);
                    let same = match (&lines[0].1, &their) {
                        (Some(a), Some(b)) => super::same_line(a, b),
                        _ => false,
                    };
                    if their.is_some() && !same {
                        let nst = super::style_at(sheet, nb);
                        sets.push((nb, with_sides(styles, nst, &[(face, None)])));
                    }
                }
            }
        } else if anchor_gone {
            moves.push((m.start, pos(first, olo)));
            for (k, &v) in keep.iter().enumerate() {
                for o in olo..=ohi {
                    let p = pos(v, o);
                    let src = pos(lo + k as u32, o);
                    let mut lines = Vec::new();
                    if v == first {
                        lines.push((
                            low,
                            pick_line([
                                facing(sheet, styles, p, low),
                                edge_of(sheet, styles, p, low),
                            ]),
                        ));
                    }
                    for (sd, on) in [(side_a, o == olo), (side_b, o == ohi), (high, v == hi)] {
                        if on {
                            let line = pick_line([
                                facing(sheet, styles, p, sd),
                                edge_of(sheet, styles, p, sd),
                            ]);
                            lines.push((sd, line));
                        }
                    }
                    let st = with_sides(styles, super::style_at(sheet, src), &lines);
                    if st != super::style_at(sheet, p) {
                        sets.push((p, st));
                    }
                }
            }
        }
    }
    (sets, moves)
}

/// 행·열 삽입·삭제
pub fn shift(wb: &mut Workbook, si: usize, op: Shift) -> crate::Result<()> {
    let err = |m: &str| crate::Error::Edit(m.to_string());
    let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
    if would_push_out(sheet, op) {
        return Err(err("시트 끝을 넘는 셀이 생겨 삽입할 수 없습니다"));
    }
    // 배열 수식의 일부만 지우거나 그 안에 끼워 넣을 수 없다
    for c in &sheet.cells {
        if let Some(f) = &c.formula
            && matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable)
            && let Some(a) = f.range.as_deref().and_then(refs::parse_area)
        {
            let (r0, c0, r1, c1) = a.bounds();
            let (lo, hi) = if op.on_rows() { (r0, r1) } else { (c0, c1) };
            let broken = match op {
                Shift::InsRows { at, .. } | Shift::InsCols { at, .. } => lo < at && at <= hi,
                Shift::DelRows { at, n } | Shift::DelCols { at, n } => {
                    let end = at + n - 1;
                    let hits = !(hi < at || lo > end);
                    hits && !(lo >= at && hi <= end)
                }
            };
            if broken {
                return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
            }
        }
    }
    let target = sheet.name.clone();

    // 1. 모든 시트의 수식, 이름, 차트
    let names: Vec<String> = wb.sheets.iter().map(|s| s.name.clone()).collect();
    for (k, sh) in wb.sheets.iter_mut().enumerate() {
        let here = names[k].clone();
        let mut changed = false;
        for c in &mut sh.cells {
            if let Some(f) = &mut c.formula
                && !f.text.is_empty()
            {
                let t = refs::shift_formula(&f.text, &here, &target, op);
                if t != f.text {
                    f.text = t;
                    changed = true;
                }
            }
        }
        if changed || k == si {
            unshare(sh);
        }
        // 다른 시트의 조건부 서식·유효성 수식도 이 시트를 가리킬 수 있다
        if k != si {
            for cf in &mut sh.conditional_formats {
                for r in &mut cf.rules {
                    for x in &mut r.formulas {
                        *x = refs::shift_formula(x, &here, &target, op);
                    }
                }
            }
            for dv in &mut sh.data_validations {
                for x in [&mut dv.formula1, &mut dv.formula2].into_iter().flatten() {
                    *x = refs::shift_formula(x, &here, &target, op);
                }
            }
            for h in &mut sh.hyperlinks {
                if let Some(l) = &mut h.location {
                    *l = refs::shift_formula(l, &here, &target, op);
                }
            }
        }
    }
    for d in &mut wb.defined_names {
        let here = d
            .local_sheet_id
            .and_then(|i| names.get(i as usize))
            .cloned()
            .unwrap_or_default();
        d.formula = refs::shift_formula(&d.formula, &here, &target, op);
    }
    map_all_charts(wb, &|x| refs::shift_formula(x, "", &target, op));

    let default_width = default_col_width(wb, si);
    let boxes_before = comment_geometry(wb, si, 0);
    let (raw_parts, styles, sheet) = (&mut wb.raw_parts, &mut wb.styles, &mut wb.sheets[si]);
    let rows = op.on_rows();

    // 2. 셀. 열을 옮기면 행 서식이 있는 행에 열 서식 셀을 먼저 만들고, 삽입한 행·열이 물려받을 서식을 정한다
    let made = row_style_cells(sheet, op);
    if !made.is_empty() {
        sheet.cells.extend(made);
        sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    }
    let (sets, moves) = merge_fixes(styles, sheet, op);
    for (from, to) in moves {
        let Some(a) = sheet.cell(from).cloned() else {
            continue;
        };
        if a.value.is_empty() && a.formula.is_none() {
            continue;
        }
        let c = super::cell_mut(sheet, to);
        c.value = a.value;
        c.formula = a.formula;
    }
    for (r, st) in sets {
        super::cell_mut(sheet, r).style = st;
    }
    let inherit = inherited_cells(styles, sheet, op);
    let mut cells: Vec<Cell> = Vec::with_capacity(sheet.cells.len());
    for mut c in std::mem::take(&mut sheet.cells) {
        let (Some(row), Some(col)) = (op.point(true, c.r.row), op.point(false, c.r.col)) else {
            continue;
        };
        // 시트 끝에 머문 칸 (빈 서식 칸만 — 값은 위에서 막았다)
        let moved_out = match op {
            Shift::InsRows { at, n } => c.r.row >= at && c.r.row + n >= refs::MAX_ROWS,
            Shift::InsCols { at, n } => c.r.col >= at && c.r.col + n >= refs::MAX_COLS,
            _ => false,
        };
        if moved_out {
            continue;
        }
        c.r = CellRef { row, col };
        if let Some(f) = &mut c.formula {
            if let Some(rg) = &f.range
                && matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable)
            {
                f.range = shift_range_text(rg, op, false);
            }
            if let Some(dt) = &mut f.data_table {
                for x in [&mut dt.r1, &mut dt.r2].into_iter().flatten() {
                    if let Some(a) = area_of(x) {
                        match op.area(a, false) {
                            Some(b) => *x = b.text(),
                            None => {
                                *x = "#REF!".into();
                            }
                        }
                    }
                }
            }
        }
        cells.push(c);
    }
    // 삽입한 행·열: 앞 행(열)의 서식을 물려받는다 (옮기기 전의 칸으로 정해 둔 것)
    for (r, c, style) in inherit {
        cells.push(Cell {
            r: CellRef { row: r, col: c },
            style,
            value: CellValue::Empty,
            formula: None,
        });
    }
    cells.sort_by_key(|c| (c.r.row, c.r.col));
    cells.dedup_by_key(|c| (c.r.row, c.r.col));
    sheet.cells = cells;

    // 3. 행·열 정보
    if rows {
        let mut out: Vec<RowInfo> = Vec::with_capacity(sheet.rows.len());
        let mut copy_from: Option<RowInfo> = None;
        for r in std::mem::take(&mut sheet.rows) {
            let i = r.index - 1;
            if let Shift::InsRows { at, .. } = op
                && at > 0
                && i == at - 1
            {
                copy_from = Some(r.clone());
            }
            if let Some(n) = op.point(true, i) {
                out.push(RowInfo { index: n + 1, ..r });
            }
        }
        if let (Shift::InsRows { at, n }, Some(src)) = (op, copy_from) {
            for k in at..(at + n).min(refs::MAX_ROWS) {
                out.push(RowInfo {
                    index: k + 1,
                    hidden: false,
                    collapsed: false,
                    ..src.clone()
                });
            }
        }
        out.sort_by_key(|r| r.index);
        out.dedup_by_key(|r| r.index);
        sheet.rows = out;
    } else {
        // 삽입한 열은 왼쪽 열의 열 정보를 너비를 명시해(customWidth) 받는다 — 왼쪽 열을 덮는 정보를 넓히지 않고
        // 따로 둔다 (숨김·접힘은 받지 않는다). 왼쪽 열 정보가 없거나 맨 앞에 넣으면 기본 너비를 명시한 열 정보
        let mut out: Vec<ColInfo> = Vec::with_capacity(sheet.cols.len() + 2);
        let mut new_cols: Option<ColInfo> = None;
        for c in std::mem::take(&mut sheet.cols) {
            let (a, b) = (c.min - 1, c.max - 1);
            if let Shift::InsCols { at, n } = op {
                if at > 0 && (a..=b).contains(&(at - 1)) {
                    new_cols = Some(ColInfo {
                        min: at + 1,
                        max: (at + n).min(refs::MAX_COLS),
                        hidden: false,
                        collapsed: false,
                        custom_width: true,
                        ..c.clone()
                    });
                }
                // 삽입 위치를 품은 정보는 둘로 나뉜다
                if a < at && at <= b {
                    out.push(ColInfo {
                        min: a + 1,
                        max: at,
                        ..c.clone()
                    });
                    if at + n < refs::MAX_COLS {
                        out.push(ColInfo {
                            min: at + n + 1,
                            max: (b + n + 1).min(refs::MAX_COLS),
                            ..c
                        });
                    }
                    continue;
                }
            }
            if let Some((x, y)) = op.span(false, a, b, false) {
                out.push(ColInfo {
                    min: x + 1,
                    max: y + 1,
                    ..c
                });
            }
        }
        if let Shift::InsCols { at, n } = op {
            out.push(new_cols.unwrap_or(ColInfo {
                min: at + 1,
                max: (at + n).min(refs::MAX_COLS),
                width: Some(default_width),
                style: None,
                hidden: false,
                custom_width: true,
                outline_level: 0,
                collapsed: false,
            }));
        }
        out.sort_by_key(|c| c.min);
        sheet.cols = out;
    }

    // 4. 병합 (한 칸만 남으면 푼다)
    sheet.merges = std::mem::take(&mut sheet.merges)
        .into_iter()
        .filter_map(|m| {
            let a = Area::rect(m.start.row, m.start.col, m.end.row, m.end.col);
            let (r0, c0, r1, c1) = op.area(a, false)?.bounds();
            (r0 != r1 || c0 != c1).then_some(crate::model::CellRange {
                start: CellRef { row: r0, col: c0 },
                end: CellRef { row: r1, col: c1 },
            })
        })
        .collect();

    // 5. 조건부 서식·유효성: 범위는 붙은 범위까지 넓히고, 수식은 새 범위의 왼쪽 위 칸에 맞춘다
    let anchor_from = |tl: (u32, u32)| {
        (
            op.inverse(true, tl.0).unwrap_or(tl.0),
            op.inverse(false, tl.1).unwrap_or(tl.1),
        )
    };
    sheet.conditional_formats = std::mem::take(&mut sheet.conditional_formats)
        .into_iter()
        .filter_map(|mut cf| {
            let old = refs::parse_sqref(&cf.sqref);
            let new = shift_sqref(&cf.sqref, op, true);
            let (otl, ntl) = (top_left(&old)?, top_left(&new)?);
            let from = anchor_from(ntl);
            for r in &mut cf.rules {
                for f in &mut r.formulas {
                    *f = refs::shift_anchored(f, &target, &target, op, otl, from);
                }
            }
            cf.sqref = refs::sqref_text(&new);
            Some(cf)
        })
        .collect();
    sheet.data_validations = std::mem::take(&mut sheet.data_validations)
        .into_iter()
        .filter_map(|mut dv| {
            let old = refs::parse_sqref(&dv.sqref);
            let new = shift_sqref(&dv.sqref, op, true);
            let (otl, ntl) = (top_left(&old)?, top_left(&new)?);
            let from = anchor_from(ntl);
            for f in [&mut dv.formula1, &mut dv.formula2].into_iter().flatten() {
                *f = refs::shift_anchored(f, &target, &target, op, otl, from);
            }
            dv.sqref = refs::sqref_text(&new);
            Some(dv)
        })
        .collect();

    // 6. 하이퍼링크·메모
    sheet.hyperlinks = std::mem::take(&mut sheet.hyperlinks)
        .into_iter()
        .filter_map(|mut h| {
            h.range = shift_range_text(&h.range, op, false)?;
            if let Some(l) = &mut h.location {
                *l = refs::shift_formula(l, &target, &target, op);
            }
            Some(h)
        })
        .collect();
    // 자리가 바뀐 메모 (새 목록의 번호, 옛 상자 앵커) — 상자는 마지막에 새 배치로 다시 정한다
    let mut moved_boxes: Vec<(usize, [u32; 8])> = Vec::new();
    let mut comments = Vec::with_capacity(sheet.comments.len());
    for mut c in std::mem::take(&mut sheet.comments) {
        let (Some(row), Some(col)) = (op.point(true, c.cell.row), op.point(false, c.cell.col))
        else {
            continue;
        };
        let (dr, dc) = (
            row as i64 - c.cell.row as i64,
            col as i64 - c.cell.col as i64,
        );
        c.cell = CellRef { row, col };
        if let Some(s) = &mut c.shape {
            let a = &mut s.anchor;
            if dr != 0 || dc != 0 {
                moved_boxes.push((comments.len(), *a));
            }
            for k in [0usize, 4] {
                a[k] = (a[k] as i64 + dc).max(0) as u32;
                a[k + 2] = (a[k + 2] as i64 + dr).max(0) as u32;
            }
        }
        comments.push(c);
    }
    sheet.comments = comments;

    // 7. 도형·차트 앵커. 지운 행·열 안에 모두 든 두 칸 앵커 도형은 함께 지운다 (한셀)
    sheet.shapes.retain(|s| !inside_deleted(s, op));
    for s in &mut sheet.shapes {
        shift_shape(s, op);
    }

    // 8. 표·자동 필터
    let mut tables = Vec::with_capacity(sheet.tables.len());
    for mut t in std::mem::take(&mut sheet.tables) {
        let Some(a) = area_of(&t.range) else {
            tables.push(t);
            continue;
        };
        let Some(b) = op.area(a, false) else {
            detach_table(raw_parts, &mut t);
            continue;
        };
        if b != a {
            let (_, c0, _, c1) = a.bounds();
            let (_, d0, _, d1) = b.bounds();
            if !rows && (d1 - d0) != (c1 - c0) {
                // 표 안에 열을 넣거나 지웠다: 열 목록을 맞춘다
                match op {
                    Shift::InsCols { at, n } => {
                        let k = (at - c0) as usize;
                        let mut next = t.columns.iter().map(|c| c.id).max().unwrap_or(0) + 1;
                        for j in 0..n as usize {
                            let mut name = format!("열{}", next);
                            while t.columns.iter().any(|c| c.name == name) {
                                next += 1;
                                name = format!("열{}", next);
                            }
                            t.columns.insert(
                                (k + j).min(t.columns.len()),
                                crate::model::TableColumn {
                                    id: next,
                                    name,
                                    ..Default::default()
                                },
                            );
                            next += 1;
                        }
                    }
                    Shift::DelCols { at, n } => {
                        let lo = at.max(c0) - c0;
                        let hi = (at + n - 1).min(c1) - c0;
                        t.columns
                            .drain(lo as usize..=(hi as usize).min(t.columns.len() - 1));
                    }
                    _ => {}
                }
            }
            t.range = b.text();
            if let Some(af) = &mut t.auto_filter
                && let Some(r) = shift_range_text(&af.range, op, false)
            {
                af.range = r;
            }
            detach_table(raw_parts, &mut t);
        }
        tables.push(t);
    }
    sheet.tables = tables;
    // 필터 범위의 첫 칸(머리글)이 옮겨지면 한셀은 필터를 다시 적용한다 (마지막에)
    let mut filter_moved = false;
    if let Some(af) = &mut sheet.auto_filter {
        let start = |r: &str| area_of(r).map(|a| (a.bounds().0, a.bounds().1));
        let before = start(&af.range);
        match shift_range_text(&af.range, op, false) {
            Some(r) => {
                filter_moved = before.is_some() && start(&r) != before;
                af.range = r;
            }
            None => sheet.auto_filter = None,
        }
    }

    // 9·10. 틀 고정과 페이지 나누기
    shift_pane_breaks(sheet, op);

    update_dimension(sheet);

    // 11. 필터 다시 적용
    if filter_moved {
        super::filter::reapply(wb, si);
    }

    // 12. 자리가 바뀐 메모 상자
    if let Some((old, ext)) = boxes_before
        && !moved_boxes.is_empty()
    {
        let n = match op {
            Shift::InsRows { n, .. }
            | Shift::DelRows { n, .. }
            | Shift::InsCols { n, .. }
            | Shift::DelCols { n, .. } => n.min(1024),
        };
        let new = crate::render::sheet_geometry(wb, &wb.sheets[si], ext.0 + n, ext.1 + n);
        let sheet = &mut wb.sheets[si];
        let cap = box_cap(sheet);
        for (i, a) in moved_boxes {
            let c = &mut sheet.comments[i];
            let cell = c.cell;
            if let Some(s) = &mut c.shape {
                s.anchor = moved_comment_box(&old, &new, a, cell, cap);
            }
        }
    }
    Ok(())
}

/// 행·열 편집에 따른 틀 고정 범위와 페이지 나누기 (셀 삽입도 같이 쓴다)
pub(crate) fn shift_pane_breaks(sheet: &mut Sheet, op: Shift) {
    let rows = op.on_rows();
    // 9. 틀 고정: 고정한 행·열 안에 넣거나 지우면 고정 범위가 늘거나 준다 (한셀 — 왼쪽 위 칸은 그대로)
    if let Some(p) = &mut sheet.view.pane
        && p.state.as_deref().is_some_and(|s| s.starts_with("frozen"))
    {
        let split = if rows { &mut p.y_split } else { &mut p.x_split };
        let n_frozen = *split as u32;
        match op {
            Shift::InsRows { at, n } | Shift::InsCols { at, n } if at < n_frozen => {
                *split = (n_frozen + n) as f64;
            }
            Shift::DelRows { at, n } | Shift::DelCols { at, n } if at < n_frozen => {
                *split = (n_frozen - n.min(n_frozen - at)) as f64;
            }
            _ => {}
        }
    }

    // 10. 페이지 나누기 (페이지 나누기 대조 실험): 여러 행·열을 넣거나 지우면 한셀은 범위의 첫 자리와 끝 자리의
    // 나누기를 그대로 두고 (E:G 삭제 → E·G 앞 나누기는 그대로, F 앞 나누기는 없어진다), 나머지는 하나일 때와 같이
    // 옮긴다. 차례를 바꾸거나 겹친 것을 합치지 않는다
    let (at, n) = match op {
        Shift::InsRows { at, n }
        | Shift::DelRows { at, n }
        | Shift::InsCols { at, n }
        | Shift::DelCols { at, n } => (at, n),
    };
    let breaks = if rows {
        &mut sheet.page.row_breaks
    } else {
        &mut sheet.page.col_breaks
    };
    *breaks = breaks
        .iter()
        .filter_map(|&b| {
            if n >= 2 && (b == at || b == at + n - 1) {
                Some(b)
            } else {
                op.point(rows, b)
            }
        })
        .collect();
}

/// 새 열의 기본 너비 (글자 수): 시트의 기본 너비, 없으면 한셀 규칙 (첫 글꼴의 숫자 폭)
pub(crate) fn default_col_width(wb: &Workbook, si: usize) -> f64 {
    let k = crate::render::book_metrics(wb).k;
    crate::render::default_col_width(&wb.sheets[si], k)
}

/// 시트의 쓰는 범위 (`dimension`)를 셀에 맞춘다
pub(crate) fn update_dimension(sheet: &mut Sheet) {
    let mut it = sheet.cells.iter();
    let Some(first) = it.next() else {
        sheet.dimension = Some("A1".into());
        return;
    };
    let (mut r0, mut c0, mut r1, mut c1) = (first.r.row, first.r.col, first.r.row, first.r.col);
    for c in it {
        r0 = r0.min(c.r.row);
        c0 = c0.min(c.r.col);
        r1 = r1.max(c.r.row);
        c1 = c1.max(c.r.col);
    }
    sheet.dimension = Some(Area::rect(r0, c0, r1, c1).text());
}
