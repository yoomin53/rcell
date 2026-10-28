// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 메모·하이퍼링크·이름 정의·유효성·조건부 서식 더하기·지우기, 열 너비·행 높이 자동 맞춤
//! (개체 대조 실험 — docs/editing.md 17절).
//!
//! - 새 메모: 글은 `작성자:` 줄 다음에 (굴림 9pt 검정 한 조각), 상자는 메모 칸의 열 + 14px, 두 행 위 + 10px 에
//!   126 × 79 px. 고치면 글 전체를 바꾼다 (작성자 줄 없이). 이름은 쓰지 않은 다음 `메모 N`.
//! - 하이퍼링크: 칸이 비었으면 보일 글(없으면 주소)을 넣고, 글꼴을 하이퍼링크 꼴(파랑 밑줄)로. 지우면 링크와 그
//!   꼴을 함께 없앤다 (값은 남는다).
//! - 이름: 통합 문서 또는 시트 범위. 이름 규칙은 한셀과 같다 (글자·밑줄로 시작, 셀 주소 꼴이 아닌 것).
//! - 열 너비 자동 맞춤: 열의 칸마다 표시된 글의 폭 + 여백으로 (렌더러의 글꼴 측정 — 어림이므로 한셀과 몇 px 다를
//!   수 있다).

use super::err;
use crate::model::{
    CellRange, CellRef, CfRule, Color, Comment, CommentShape, ConditionalFormat, DataValidation,
    DefinedName, Dxf, Fill, Font, Hyperlink, RichText, TextRun, Workbook,
};

/// 메모의 쓰지 않은 다음 이름 번호
fn next_comment_number(sheet: &crate::model::Sheet) -> u32 {
    sheet
        .comments
        .iter()
        .filter_map(|c| {
            c.shape
                .as_ref()
                .and_then(|s| s.name.as_deref())
                .or(c.shape_name.as_deref())
                .and_then(|n| n.strip_prefix("메모 "))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1
}

/// 메모 글 (굴림 9pt 검정 한 조각 — 한셀이 새 메모에 쓰는 것)
fn comment_text(s: &str) -> RichText {
    RichText {
        text: s.to_string(),
        runs: vec![TextRun {
            text: s.to_string(),
            font: Some(Font {
                name: Some("굴림".into()),
                size: Some(9.0),
                color: Some(Color {
                    rgb: Some("FF000000".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }],
    }
}

/// 메모 더하기 (그 칸에 메모가 있으면 바꾼다)
pub fn add_comment(
    wb: &mut Workbook,
    si: usize,
    cell: CellRef,
    author: &str,
    text: &str,
) -> crate::Result<()> {
    let geo = crate::render::sheet_geometry(wb, &wb.sheets[si], cell.row + 12, cell.col + 6);
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    sheet.comments.retain(|c| c.cell != cell);
    let n = next_comment_number(sheet);
    let body = if author.is_empty() {
        text.to_string()
    } else {
        format!("{author}:\n{text}")
    };
    // 기본 자리: 메모 칸의 열 + 14px, 두 행 위 + 10px, 126 × 79 px (한셀 자동화가 새 메모에 준 자리 — 1·2행과
    // 한셀 창의 보이는 범위에 따라 다르기도 하다)
    let cols: Vec<(u32, u32)> = super::structure::anchor_cols(&geo);
    let (lc, lo) = (cell.col, 14);
    let (tr, to) = (cell.row.saturating_sub(2), 10);
    let x = super::structure::at_px(&cols, lc, lo) + 126;
    let y = super::structure::at_px(&geo.rows, tr, to) + 79;
    let (rc, ro) = super::structure::locate_px(&cols, x);
    let (br, bo) = super::structure::locate_px(&geo.rows, y);
    let mut shape = CommentShape::hcell_default([lc, lo, tr, to, rc, ro, br, bo]);
    shape.name = Some(format!("메모 {n}"));
    sheet.comments.push(Comment {
        cell,
        author: author.to_string(),
        text: comment_text(&body),
        visible: false,
        shape: Some(shape),
        shape_name: None,
        z_order: None,
    });
    Ok(())
}

/// 메모 고치기 (글 전체)
pub fn edit_comment(wb: &mut Workbook, si: usize, cell: CellRef, text: &str) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let c = sheet
        .comments
        .iter_mut()
        .find(|c| c.cell == cell)
        .ok_or_else(|| err("메모가 없습니다"))?;
    c.text = comment_text(text);
    Ok(())
}

/// 범위의 메모 지우기
pub fn delete_comments(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    sheet.comments.retain(|c| !range.contains(c.cell));
    Ok(())
}

/// 하이퍼링크 글꼴 (파랑, 밑줄 — 한컴 확장의 밑줄 모양 실선)
fn link_font(base: &Font) -> Font {
    let blue = Color {
        rgb: Some("FF0000FF".into()),
        ..Default::default()
    };
    let mut f = base.clone();
    f.color = Some(blue.clone());
    f.underline = Some("single".into());
    let mut h = f.hancom.clone().unwrap_or_default();
    h.underline_type = Some("1".into());
    h.underline_shape = Some("solid".into());
    h.underline_color = Some(blue);
    f.hancom = Some(h);
    f
}

/// 하이퍼링크 더하기: `address`(바깥 주소) 또는 `location`(문서 안 자리)
pub fn add_hyperlink(
    wb: &mut Workbook,
    si: usize,
    cell: CellRef,
    address: Option<&str>,
    location: Option<&str>,
    tooltip: Option<&str>,
    display: Option<&str>,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let a1 = super::refs::Area::rect(cell.row, cell.col, cell.row, cell.col).text();
    // 비었으면 보일 글 (없으면 주소)
    let empty = wb.sheets[si]
        .cell(cell)
        .is_none_or(|c| c.value.is_empty() && c.formula.is_none());
    if let Some(t) = display.or(address).filter(|_| empty).or(display) {
        let st = super::style_at(&wb.sheets[si], cell);
        let c = super::cell_mut(&mut wb.sheets[si], cell);
        c.value = crate::model::CellValue::Text(t.to_string());
        c.formula = None;
        c.style = st;
    }
    // 글꼴
    let style = super::style_at(&wb.sheets[si], cell);
    let new = super::style::Restyler::default().apply(&mut wb.styles, style, &|st, x| {
        let base = st
            .fonts
            .get(x.font_id as usize)
            .cloned()
            .unwrap_or_default();
        let f = link_font(&base);
        x.font_id = super::style::intern_font(st, f);
        x.apply_font = Some(true);
    });
    super::cell_mut(&mut wb.sheets[si], cell).style = new;
    let sheet = &mut wb.sheets[si];
    sheet
        .hyperlinks
        .retain(|h| CellRange::parse(&h.range).is_none_or(|r| !r.contains(cell)));
    sheet.hyperlinks.push(Hyperlink {
        range: a1,
        location: location.map(str::to_string),
        target: address.filter(|a| !a.is_empty()).map(str::to_string),
        tooltip: tooltip.map(str::to_string),
        display: None,
    });
    super::structure::update_dimension(sheet);
    Ok(())
}

/// 범위의 하이퍼링크 지우기 (그 칸의 서식은 기본으로 — 값은 남는다)
pub fn remove_hyperlinks(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let mut cells = Vec::new();
    sheet
        .hyperlinks
        .retain(|h| match CellRange::parse(&h.range) {
            Some(r) if range.contains(r.start) && range.contains(r.end) => {
                cells.push(r);
                false
            }
            _ => true,
        });
    for r in cells {
        for row in r.start.row..=r.end.row {
            for col in r.start.col..=r.end.col {
                if let Some(c) = sheet.cells.iter_mut().find(|c| c.r == CellRef { row, col }) {
                    c.style = 0;
                }
            }
        }
    }
    Ok(())
}

/// 이름 규칙: 글자·밑줄·역슬래시로 시작, 글자·숫자·밑줄·점, 셀 주소·R1C1 꼴이 아닌 것, 255자까지
pub fn check_name(name: &str) -> crate::Result<()> {
    let bad = || err(format!("이름으로 쓸 수 없습니다: {name}"));
    let mut cs = name.chars();
    let first = cs.next().ok_or_else(bad)?;
    if !(first.is_alphabetic() || first == '_' || first == '\\') {
        return Err(bad());
    }
    if !cs.all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '\\')
        || name.chars().count() > 255
    {
        return Err(bad());
    }
    let up = name.to_ascii_uppercase();
    if CellRef::parse(&up).is_some() || up == "R" || up == "C" {
        return Err(bad());
    }
    Ok(())
}

/// 이름 정의 더하기 (`local` 이면 그 시트 범위). 같은 이름이 있으면 바꾼다
pub fn add_name(
    wb: &mut Workbook,
    si: usize,
    local: Option<usize>,
    name: &str,
    refers_to: &str,
) -> crate::Result<()> {
    check_name(name)?;
    let formula = super::normalize_formula(wb, refers_to.trim_start_matches('='))?;
    // 시트 없는 참조는 그 시트를 붙인다 (한셀 — 이름은 활성 시트의 칸을 가리킨다)
    let sheet_name = wb
        .sheets
        .get(si)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let formula = super::refs::map_refs(&formula, |r| {
        if r.prefix.is_some() {
            return None;
        }
        Some(format!(
            "{}{}",
            super::refs::prefix_text(&sheet_name, None),
            r.raw
        ))
    });
    let lid = local.map(|i| i as u32);
    match wb
        .defined_names
        .iter_mut()
        .find(|d| d.local_sheet_id == lid && super::refs::same_sheet(&d.name, name))
    {
        Some(d) => d.formula = formula,
        None => wb.defined_names.push(DefinedName {
            name: name.to_string(),
            local_sheet_id: lid,
            hidden: false,
            formula,
            comment: None,
        }),
    }
    Ok(())
}

/// 이름 정의 지우기 (`local` 이면 그 시트 범위의 이름)
pub fn delete_name(wb: &mut Workbook, local: Option<usize>, name: &str) -> crate::Result<()> {
    let lid = local.map(|i| i as u32);
    let before = wb.defined_names.len();
    wb.defined_names
        .retain(|d| !(d.local_sheet_id == lid && super::refs::same_sheet(&d.name, name)));
    if wb.defined_names.len() == before {
        return Err(err(format!("이름이 없습니다: {name}")));
    }
    Ok(())
}

/// 유효성 더하기 (범위의 다른 유효성은 그 칸에서 뺀다)
pub fn add_validation(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    dv: DataValidation,
) -> crate::Result<()> {
    delete_validation(wb, si, range)?;
    let sheet = &mut wb.sheets[si];
    let a = super::refs::Area::rect(
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    sheet.data_validations.push(DataValidation {
        sqref: a.text(),
        ..dv
    });
    Ok(())
}

/// 범위의 유효성 지우기
pub fn delete_validation(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let hole = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    for dv in &mut sheet.data_validations {
        let rest: Vec<super::refs::Area> = super::refs::parse_sqref(&dv.sqref)
            .iter()
            .flat_map(|a| super::clip::subtract_rect(a.bounds(), hole))
            .map(|(a, b, c, d)| super::refs::Area::rect(a, b, c, d))
            .collect();
        dv.sqref = super::refs::sqref_text(&rest);
    }
    sheet.data_validations.retain(|d| !d.sqref.is_empty());
    Ok(())
}

/// 조건부 서식 규칙 더하기: 새 규칙은 맨 위 우선순위 (다른 규칙은 하나씩 내려간다)
#[allow(clippy::too_many_arguments)]
pub fn add_cf(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    kind: &str,
    operator: Option<&str>,
    formulas: Vec<String>,
    fill: Option<&str>,
    font_color: Option<&str>,
    bold: bool,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let rgb = |h: &str| Color {
        rgb: Some(format!(
            "FF{}",
            h.trim_start_matches('#').to_ascii_uppercase()
        )),
        ..Default::default()
    };
    let dxf = Dxf {
        font: (font_color.is_some() || bold).then(|| Font {
            bold,
            color: font_color.map(rgb),
            ..Default::default()
        }),
        fill: fill.map(|f| Fill {
            pattern_type: Some("solid".into()),
            fg_color: Some(rgb(f)),
            bg_color: Some(rgb(f)),
            gradient: None,
        }),
        ..Default::default()
    };
    wb.styles.dxfs.push(dxf);
    let dxf_id = (wb.styles.dxfs.len() - 1) as u32;
    let sheet = &mut wb.sheets[si];
    for cf in &mut sheet.conditional_formats {
        for r in &mut cf.rules {
            r.priority += 1;
        }
    }
    let formulas = formulas
        .into_iter()
        .map(|f| f.trim_start_matches('=').to_string())
        .collect();
    sheet.conditional_formats.push(ConditionalFormat {
        sqref: super::refs::Area::rect(
            range.start.row,
            range.start.col,
            range.end.row,
            range.end.col,
        )
        .text(),
        rules: vec![CfRule {
            kind: kind.to_string(),
            priority: 1,
            dxf_id: Some(dxf_id),
            operator: operator.map(str::to_string),
            formulas,
            ..Default::default()
        }],
    });
    Ok(())
}

/// 범위의 조건부 서식 지우기 (범위에서 뺀다)
pub fn delete_cf(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let hole = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    for cf in &mut sheet.conditional_formats {
        let rest: Vec<super::refs::Area> = super::refs::parse_sqref(&cf.sqref)
            .iter()
            .flat_map(|a| super::clip::subtract_rect(a.bounds(), hole))
            .map(|(a, b, c, d)| super::refs::Area::rect(a, b, c, d))
            .collect();
        cf.sqref = super::refs::sqref_text(&rest);
    }
    sheet.conditional_formats.retain(|c| !c.sqref.is_empty());
    Ok(())
}

/// 열 너비 자동 맞춤: 열의 칸마다 표시된 글의 폭 + 여백 8px 의 가장 큰 값 (글이 없는 열은 그대로)
pub fn autofit_cols(wb: &mut Workbook, si: usize, c0: u32, c1: u32) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let k = crate::render::book_metrics(wb).k;
    for col in c0..=c1.min(c0 + 1000) {
        let sheet = &wb.sheets[si];
        let mut best: f64 = 0.0;
        for c in sheet
            .cells
            .iter()
            .filter(|c| c.r.col == col && !c.value.is_empty())
        {
            let xf = wb.styles.xf(c.style);
            let code = xf
                .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
                .unwrap_or_else(|| "General".into());
            let text = crate::numfmt::format(&c.value, &code, wb.date1904).text;
            let font = wb.styles.font_of(c.style);
            let m = crate::render::FontMetrics::new(
                font.and_then(|f| f.name.as_deref()),
                font.and_then(|f| f.size),
                font.is_some_and(|f| f.bold),
            );
            best = best.max(m.text_width(&text) + 8.0);
        }
        if best <= 0.0 {
            continue;
        }
        let stored = (best * 256.0 / k).trunc() / 256.0;
        super::set_col_width(wb, si, col, col, stored)?;
    }
    Ok(())
}

/// 행 높이 자동 맞춤: 높이를 지정하지 않은 행으로 (한셀이 문서를 열 때 글에 맞춘다)
pub fn autofit_rows(wb: &mut Workbook, si: usize, r0: u32, r1: u32) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    for r in &mut sheet.rows {
        let i = r.index - 1;
        if i >= r0 && i <= r1 {
            r.custom_height = false;
            r.height = None;
        }
    }
    Ok(())
}
