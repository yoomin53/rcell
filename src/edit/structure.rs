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
    Cell, CellRef, CellValue, ColInfo, Comment, FormulaKind, Marker, RowInfo, Shape, Sheet,
    Workbook,
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

/// 도형 앵커를 옮긴다: 두 칸 앵커(기본)는 양 끝을 따로, `oneCell` 은 시작 칸만큼 함께, `absolute` 는 그대로
fn shift_shape(sh: &mut Shape, op: Shift) {
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
    let (raw_parts, sheet) = (&mut wb.raw_parts, &mut wb.sheets[si]);
    let rows = op.on_rows();

    // 2. 셀
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
    // 삽입한 행·열: 앞 행(열)의 서식을 물려받는다
    match op {
        Shift::InsRows { at, n } if at > 0 => {
            let src = at - 1;
            let styled: Vec<(u32, u32)> = cells
                .iter()
                .filter(|c| c.r.row == src && c.style != 0)
                .map(|c| (c.r.col, c.style))
                .collect();
            for r in at..(at + n).min(refs::MAX_ROWS) {
                for &(col, style) in &styled {
                    cells.push(Cell {
                        r: CellRef { row: r, col },
                        style,
                        value: CellValue::Empty,
                        formula: None,
                    });
                }
            }
        }
        Shift::InsCols { at, n } if at > 0 => {
            let src = at - 1;
            let col_styled = sheet
                .cols
                .iter()
                .any(|c| (c.min..=c.max).contains(&at) && c.style.is_some_and(|s| s != 0));
            let row_styled: std::collections::HashSet<u32> = sheet
                .rows
                .iter()
                .filter(|r| r.style.is_some())
                .map(|r| r.index - 1)
                .collect();
            let styled: Vec<(u32, u32)> = cells
                .iter()
                .filter(|c| c.r.col == src && c.style != 0)
                .filter(|c| !col_styled || row_styled.contains(&c.r.row))
                .map(|c| (c.r.row, c.style))
                .collect();
            for col in at..(at + n).min(refs::MAX_COLS) {
                for &(row, style) in &styled {
                    cells.push(Cell {
                        r: CellRef { row, col },
                        style,
                        value: CellValue::Empty,
                        formula: None,
                    });
                }
            }
        }
        _ => {}
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
        let mut out: Vec<ColInfo> = Vec::with_capacity(sheet.cols.len() + 1);
        let mut covered = false;
        for c in std::mem::take(&mut sheet.cols) {
            let (a, b) = (c.min - 1, c.max - 1);
            // 왼쪽 열을 덮는 정보는 삽입한 열까지 넓어진다
            if let Shift::InsCols { at, .. } = op
                && at > 0
                && (a..=b).contains(&(at - 1))
            {
                covered = true;
            }
            let adjacent = matches!(op, Shift::InsCols { .. });
            if let Some((x, y)) = op.span(false, a, b, adjacent) {
                out.push(ColInfo {
                    min: x + 1,
                    max: y + 1,
                    ..c
                });
            }
        }
        if let Shift::InsCols { at, n } = op
            && at > 0
            && !covered
        {
            out.push(ColInfo {
                min: at + 1,
                max: (at + n).min(refs::MAX_COLS),
                width: Some(default_width),
                style: None,
                hidden: false,
                custom_width: true,
                outline_level: 0,
                collapsed: false,
            });
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
    sheet.comments = std::mem::take(&mut sheet.comments)
        .into_iter()
        .filter_map(|mut c: Comment| {
            let row = op.point(true, c.cell.row)?;
            let col = op.point(false, c.cell.col)?;
            let (dr, dc) = (
                row as i64 - c.cell.row as i64,
                col as i64 - c.cell.col as i64,
            );
            c.cell = CellRef { row, col };
            if let Some(s) = &mut c.shape {
                let a = &mut s.anchor;
                for k in [0usize, 4] {
                    a[k] = (a[k] as i64 + dc).max(0) as u32;
                    a[k + 2] = (a[k + 2] as i64 + dr).max(0) as u32;
                }
            }
            Some(c)
        })
        .collect();

    // 7. 도형·차트 앵커
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
    if let Some(af) = &mut sheet.auto_filter {
        match shift_range_text(&af.range, op, false) {
            Some(r) => af.range = r,
            None => sheet.auto_filter = None,
        }
    }

    // 9. 페이지 나누기
    let breaks = if rows {
        &mut sheet.page.row_breaks
    } else {
        &mut sheet.page.col_breaks
    };
    *breaks = breaks.iter().filter_map(|&b| op.point(rows, b)).collect();
    breaks.dedup();

    update_dimension(sheet);
    Ok(())
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
