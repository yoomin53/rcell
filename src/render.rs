// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 시트를 HTML 로 그린다 (명령줄 `rcell html` 과 웹 뷰어가 함께 쓴다).
//!
//! 한셀 화면에 가깝게: 열 너비·행 높이, 병합, 숨김, 표시 형식, 글꼴(장평은 가로 축소,
//! 자간은 글자 간격), 채우기, 테두리, 정렬(한셀 기본 세로 정렬은 가운데), 눈금선.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::model::{BorderEdge, CellRef, CellValue, Sheet, Workbook};
use crate::theme::{self, Theme};
use crate::writer::esc;

/// 렌더링 옵션.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// 그릴 최대 행/열 (아주 큰 시트 보호)
    pub max_rows: u32,
    pub max_cols: u32,
    /// 행·열 머리글 (A B C / 1 2 3)
    pub headers: bool,
    /// 처음 보일 시트 (통합 문서 안의 번호). None 이면 선택된 시트 (한셀이 열 때처럼)
    pub active_sheet: Option<usize>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_rows: 2000,
            max_cols: 200,
            headers: true,
            active_sheet: None,
        }
    }
}

const PT_TO_PX: f64 = 96.0 / 72.0;

fn border_css(e: Option<&BorderEdge>, theme: &Theme) -> Option<String> {
    let e = e?;
    let (w, style) = match e.style.as_deref()? {
        "none" => return None,
        "thin" => (1, "solid"),
        "medium" => (2, "solid"),
        "thick" => (3, "solid"),
        "double" => (3, "double"),
        "dashed" | "dashDot" | "dashDotDot" | "slantDashDot" => (1, "dashed"),
        "mediumDashed" | "mediumDashDot" | "mediumDashDotDot" => (2, "dashed"),
        "dotted" | "hair" => (1, "dotted"),
        _ => (1, "solid"),
    };
    let color = e
        .color
        .as_ref()
        .and_then(|c| theme::resolve(c, theme))
        .unwrap_or_else(|| "#000000".into());
    Some(format!("{w}px {style} {color}"))
}

struct Layout {
    rows: Vec<u32>,
    cols: Vec<u32>,
    col_width: HashMap<u32, u32>,
    row_height: HashMap<u32, u32>,
    /// 병합 시작 셀 → (행 수, 열 수)
    span: HashMap<(u32, u32), (u32, u32)>,
    covered: HashSet<(u32, u32)>,
}

fn layout(wb: &Workbook, sheet: &Sheet, opts: Options) -> Layout {
    let mdw = crate::units::max_digit_width(wb);
    let default_col = crate::units::default_col_px(sheet, mdw);
    let default_h = sheet.default_row_height.unwrap_or(16.5);
    let mut max_r = 0;
    let mut max_c = 0;
    for c in &sheet.cells {
        if !c.value.is_empty() || c.style != 0 {
            max_r = max_r.max(c.r.row);
            max_c = max_c.max(c.r.col);
        }
    }
    for m in &sheet.merges {
        max_r = max_r.max(m.end.row);
        max_c = max_c.max(m.end.col);
    }
    // 그리기 개체가 놓인 곳까지 그린다
    for s in sheet.shapes.iter().filter(|s| !s.hidden) {
        let to = s.anchor.to.unwrap_or(s.anchor.from);
        max_r = max_r.max(to.row);
        max_c = max_c.max(to.col);
    }
    let max_r = (max_r + 1).min(opts.max_rows);
    let max_c = (max_c + 1).min(opts.max_cols);

    let mut hidden_cols = HashSet::new();
    let mut col_width = HashMap::new();
    for ci in &sheet.cols {
        for col in ci.min.saturating_sub(1)..ci.max.min(max_c + 1) {
            if ci.hidden {
                hidden_cols.insert(col);
            } else if let Some(w) = ci.width {
                col_width.insert(col, crate::units::col_width_px(w, mdw));
            }
        }
    }
    let mut hidden_rows = HashSet::new();
    let mut row_height = HashMap::new();
    for ri in &sheet.rows {
        let r = ri.index - 1;
        if ri.hidden {
            hidden_rows.insert(r);
        } else if let Some(h) = ri.height {
            row_height.insert(r, (h * PT_TO_PX).round() as u32);
        }
    }
    let cols: Vec<u32> = (0..max_c).filter(|c| !hidden_cols.contains(c)).collect();
    let rows: Vec<u32> = (0..max_r).filter(|r| !hidden_rows.contains(r)).collect();
    for &c in &cols {
        col_width.entry(c).or_insert(default_col);
    }
    for &r in &rows {
        row_height
            .entry(r)
            .or_insert((default_h * PT_TO_PX).round() as u32);
    }

    let mut span = HashMap::new();
    let mut covered = HashSet::new();
    for m in &sheet.merges {
        let rs = (m.start.row..=m.end.row)
            .filter(|r| !hidden_rows.contains(r))
            .count() as u32;
        let cs = (m.start.col..=m.end.col)
            .filter(|c| !hidden_cols.contains(c))
            .count() as u32;
        span.insert((m.start.row, m.start.col), (rs.max(1), cs.max(1)));
        for r in m.start.row..=m.end.row {
            for c in m.start.col..=m.end.col {
                if (r, c) != (m.start.row, m.start.col) {
                    covered.insert((r, c));
                }
            }
        }
    }
    Layout {
        rows,
        cols,
        col_width,
        row_height,
        span,
        covered,
    }
}

/// 셀 하나의 인라인 CSS 와 표시 내용.
fn cell_html(
    wb: &Workbook,
    theme: &Theme,
    style: u32,
    value: &CellValue,
    grid: bool,
) -> (String, String) {
    let st = &wb.styles;
    let xf = st.xf(style);
    let mut css = String::new();
    let font = xf.and_then(|x| st.fonts.get(x.font_id as usize));
    let code = xf
        .map(|x| crate::numfmt::code_for(st, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    let shown = crate::numfmt::format(value, &code, wb.date1904);

    // 글꼴
    if let Some(f) = font {
        if let Some(n) = &f.name {
            let _ = write!(css, "font-family:'{}',sans-serif;", n.replace('\'', ""));
        }
        if let Some(s) = f.size {
            let _ = write!(css, "font-size:{s}pt;");
        }
        if f.bold {
            css.push_str("font-weight:bold;");
        }
        if f.italic {
            css.push_str("font-style:italic;");
        }
        let mut deco = Vec::new();
        if f.underline.is_some() {
            deco.push("underline");
        }
        if f.strike {
            deco.push("line-through");
        }
        if !deco.is_empty() {
            let _ = write!(css, "text-decoration:{};", deco.join(" "));
        }
        if f.underline
            .as_deref()
            .is_some_and(|u| u.starts_with("double"))
        {
            css.push_str("text-decoration-style:double;");
        }
        let color = shown
            .color
            .as_deref()
            .map(|c| match c {
                "Red" => "#FF0000".to_string(),
                "Blue" => "#0000FF".to_string(),
                "Green" => "#00FF00".to_string(),
                "Yellow" => "#FFFF00".to_string(),
                "Magenta" => "#FF00FF".to_string(),
                "Cyan" => "#00FFFF".to_string(),
                "White" => "#FFFFFF".to_string(),
                _ => "#000000".to_string(),
            })
            .or_else(|| f.color.as_ref().and_then(|c| theme::resolve(c, theme)));
        if let Some(c) = color {
            let _ = write!(css, "color:{c};");
        }
        if let Some(h) = &f.hancom
            && let Some(sp) = h.spacing.filter(|&s| s != 0)
        {
            let _ = write!(css, "letter-spacing:{:.3}em;", sp as f64 / 100.0);
        }
    }

    // 채우기
    if let Some(fill) = xf.and_then(|x| st.fills.get(x.fill_id as usize))
        && let Some(pt) = fill.pattern_type.as_deref()
        && pt != "none"
    {
        let c = fill
            .fg_color
            .as_ref()
            .and_then(|c| theme::resolve(c, theme))
            .or_else(|| {
                fill.bg_color
                    .as_ref()
                    .and_then(|c| theme::resolve(c, theme))
            });
        if let Some(c) = c {
            let _ = write!(css, "background-color:{c};");
        }
    }

    // 테두리
    if let Some(b) = xf.and_then(|x| st.borders.get(x.border_id as usize)) {
        for (side, e) in [
            ("left", b.left.as_ref()),
            ("right", b.right.as_ref()),
            ("top", b.top.as_ref()),
            ("bottom", b.bottom.as_ref()),
        ] {
            if let Some(v) = border_css(e, theme) {
                let _ = write!(css, "border-{side}:{v};");
            }
        }
    }
    let _ = grid;

    // 정렬
    let align = xf.and_then(|x| x.alignment.as_ref());
    let h = align
        .and_then(|a| a.horizontal.as_deref())
        .unwrap_or("general");
    let h = match h {
        "general" => match value {
            CellValue::Number(_) | CellValue::Date(_) => "right",
            CellValue::Bool(_) | CellValue::Error(_) => "center",
            _ => "left",
        },
        "centerContinuous" => "center",
        "distributed" => "justify",
        "fill" => "left",
        other => other,
    };
    let _ = write!(css, "text-align:{h};");
    let v = align
        .and_then(|a| a.vertical.as_deref())
        .unwrap_or("center");
    let v = match v {
        "top" => "top",
        "bottom" => "bottom",
        _ => "middle",
    };
    let _ = write!(css, "vertical-align:{v};");
    if align.is_some_and(|a| a.wrap_text) {
        css.push_str("white-space:pre-wrap;word-break:break-all;");
    }
    if let Some(i) = align.and_then(|a| a.indent).filter(|&i| i > 0) {
        let side = if h == "right" { "right" } else { "left" };
        let _ = write!(css, "padding-{side}:{}px;", i * 12);
    }

    // 내용 (장평은 가로 축소)
    let text = if shown.overflow {
        "#".repeat(8)
    } else {
        shown.text
    };
    let mut inner = esc(&text).replace('\n', "<br>");
    if let Some(r) = font
        .and_then(|f| f.hancom.as_ref())
        .and_then(|h| h.ratio)
        .filter(|&r| r != 100 && r > 0)
    {
        let origin = match h {
            "right" => "right",
            "center" => "center",
            _ => "left",
        };
        inner = format!(
            "<span style=\"display:inline-block;transform:scaleX({:.3});transform-origin:{origin}\">{inner}</span>",
            r as f64 / 100.0
        );
    }
    (css, inner)
}

/// 머리글 행 높이·머리글 열 너비 (px). 틀 고정 위치 계산에 쓴다.
const HEADER_ROW_PX: u32 = 20;
const HEADER_COL_PX: u32 = 40;

/// 시트 하나를 `<table>` 로.
pub fn sheet_table(wb: &Workbook, sheet: &Sheet, opts: Options) -> String {
    let theme = wb.theme.clone().unwrap_or_default();
    let lay = layout(wb, sheet, opts);
    let cells: HashMap<(u32, u32), &crate::model::Cell> = sheet
        .cells
        .iter()
        .map(|c| ((c.r.row, c.r.col), c))
        .collect();
    let grid = sheet.show_grid_lines;
    let view = &sheet.view;
    // 조건부 서식 결과 (칸 → 덧입힐 서식)
    let cfx = crate::cf_eval::evaluate(sheet, &wb.styles, &theme);
    // 메모: 셀 오른쪽 위 빨간 삼각형 + 마우스를 올리면 작성자·내용
    let notes: HashMap<(u32, u32), String> = sheet
        .comments
        .iter()
        .map(|c| {
            let who = if c.author.is_empty() {
                String::new()
            } else {
                format!("{}:\n", c.author)
            };
            ((c.cell.row, c.cell.col), format!("{who}{}", c.text.text))
        })
        .collect();

    // 틀 고정: 고정 창 왼쪽 위 칸보다 위의 행, 왼쪽의 열은 스크롤해도 남는다 (position: sticky)
    let freeze = view
        .pane
        .as_ref()
        .filter(|p| p.state.as_deref().is_some_and(|s| s.starts_with("frozen")))
        .and_then(|p| p.top_left_cell.as_deref().and_then(CellRef::parse));
    let header_h = if opts.headers { HEADER_ROW_PX } else { 0 };
    let header_w = if opts.headers { HEADER_COL_PX } else { 0 };
    let mut sticky_top: HashMap<u32, u32> = HashMap::new();
    let mut sticky_left: HashMap<u32, u32> = HashMap::new();
    if let Some(f) = freeze {
        let mut y = header_h;
        for &r in lay.rows.iter().filter(|&&r| r < f.row) {
            sticky_top.insert(r, y);
            y += lay.row_height[&r];
        }
        let mut x = header_w;
        for &c in lay.cols.iter().filter(|&&c| c < f.col) {
            sticky_left.insert(c, x);
            x += lay.col_width[&c];
        }
    }
    let sticky = |r: u32, c: Option<u32>| -> String {
        let top = sticky_top.get(&r);
        let left = c.and_then(|c| sticky_left.get(&c));
        let mut css = String::new();
        if top.is_some() || left.is_some() {
            css.push_str("position:sticky;background:#fff;");
            if let Some(t) = top {
                let _ = write!(css, "top:{t}px;");
            }
            if let Some(l) = left {
                let _ = write!(css, "left:{l}px;");
            }
            let _ = write!(
                css,
                "z-index:{};",
                1 + top.is_some() as u8 + left.is_some() as u8
            );
        }
        css
    };

    let mut s = String::new();
    let zoom = view
        .zoom_scale
        .filter(|&z| z != 100 && (10..=400).contains(&z))
        .map(|z| format!("zoom:{z}%"))
        .unwrap_or_default();
    // 표와 그리기 개체를 같은 좌표계(왼쪽 위 = 머리글 모서리)에 둔다
    let _ = write!(s, "<div class=\"rc-canvas\" style=\"{zoom}\">");
    let _ = write!(
        s,
        "<table class=\"rc-sheet{}\">",
        if grid { " rc-grid" } else { "" }
    );
    s.push_str("<colgroup>");
    if opts.headers {
        let _ = write!(s, "<col style=\"width:{HEADER_COL_PX}px\">");
    }
    for c in &lay.cols {
        let _ = write!(s, "<col style=\"width:{}px\">", lay.col_width[c]);
    }
    s.push_str("</colgroup>");
    if opts.headers {
        s.push_str("<thead><tr><th class=\"rc-corner\"></th>");
        for c in &lay.cols {
            let st = sticky_left
                .get(c)
                .map(|l| format!(" style=\"left:{l}px;z-index:4\""))
                .unwrap_or_default();
            let _ = write!(s, "<th{st}>{}</th>", CellRef::col_name(*c));
        }
        s.push_str("</tr></thead>");
    }
    s.push_str("<tbody>");
    for &r in &lay.rows {
        let _ = write!(s, "<tr style=\"height:{}px\">", lay.row_height[&r]);
        if opts.headers {
            let st = sticky_top
                .get(&r)
                .map(|t| format!(" style=\"top:{t}px;z-index:4\""))
                .unwrap_or_default();
            let _ = write!(s, "<th{st}>{}</th>", r + 1);
        }
        for &c in &lay.cols {
            if lay.covered.contains(&(r, c)) {
                continue;
            }
            let span = lay.span.get(&(r, c));
            let mut attrs = String::new();
            if let Some(&(rs, cs)) = span {
                if rs > 1 {
                    let _ = write!(attrs, " rowspan=\"{rs}\"");
                }
                if cs > 1 {
                    let _ = write!(attrs, " colspan=\"{cs}\"");
                }
            }
            let pin = sticky(r, Some(c));
            if let Some(n) = notes.get(&(r, c)) {
                let _ = write!(attrs, " class=\"rc-note\" data-note=\"{}\"", esc(n));
            }
            match cells.get(&(r, c)) {
                Some(cell) => {
                    let formula = cell.formula.as_ref().filter(|f| !f.text.is_empty());
                    // 수식 표시(showFormulas)면 값 대신 수식, 0 값 숨김(showZeros=0)이면 0 을 비운다
                    let value = match (&cell.value, formula) {
                        (_, Some(f)) if view.show_formulas => {
                            CellValue::Text(format!("={}", f.text))
                        }
                        (CellValue::Number(n), _) if *n == 0.0 && !view.show_zeros => {
                            CellValue::Empty
                        }
                        (v, _) => v.clone(),
                    };
                    let (mut css, mut inner) = cell_html(wb, &theme, cell.style, &value, grid);
                    if let Some(cf) = cfx.get(&(r, c)) {
                        apply_cf(cf, &theme, &mut css, &mut inner);
                    }
                    let title = formula
                        .map(|f| format!(" title=\"={}\"", esc(&f.text)))
                        .unwrap_or_default();
                    let _ = write!(s, "<td{attrs}{title} style=\"{pin}{css}\">{inner}</td>");
                }
                None => {
                    // 셀 레코드가 없어도 행·열 서식(채우기·테두리)은 보인다
                    let st = sheet.effective_style(r, c);
                    let mut css = if st == 0 {
                        String::new()
                    } else {
                        cell_html(wb, &theme, st, &CellValue::Empty, grid).0
                    };
                    if let Some(cf) = cfx.get(&(r, c)) {
                        apply_cf(cf, &theme, &mut css, &mut String::new());
                    }
                    if css.is_empty() && pin.is_empty() {
                        let _ = write!(s, "<td{attrs}></td>");
                    } else {
                        let _ = write!(s, "<td{attrs} style=\"{pin}{css}\"></td>");
                    }
                }
            }
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table>");
    s.push_str(&shapes_html(wb, sheet, &lay, opts, &theme));
    s.push_str("</div>");
    s
}

/// 조건부 서식 결과를 칸의 CSS·내용에 덧입힌다 (뒤에 쓴 CSS 가 이긴다)
fn apply_cf(cf: &crate::cf_eval::CfStyle, theme: &Theme, css: &mut String, inner: &mut String) {
    let col = |c: &crate::model::Color| theme::resolve(c, theme);
    if let Some(c) = cf.scale.clone().or_else(|| cf.fill.as_ref().and_then(col)) {
        let _ = write!(css, "background-color:{c};");
    }
    if let Some(c) = cf.font_color.as_ref().and_then(col) {
        let _ = write!(css, "color:{c};");
    }
    if cf.bold == Some(true) {
        css.push_str("font-weight:bold;");
    }
    if cf.italic == Some(true) {
        css.push_str("font-style:italic;");
    }
    let mut deco = Vec::new();
    if cf.underline == Some(true) {
        deco.push("underline");
    }
    if cf.strike == Some(true) {
        deco.push("line-through");
    }
    if !deco.is_empty() {
        let _ = write!(css, "text-decoration:{};", deco.join(" "));
    }
    if let Some((len, c)) = &cf.bar
        && let Some(c) = col(c)
    {
        let p = (len * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            css,
            "background-image:linear-gradient(to right,{c} 0%,{c}99 {p:.1}%,transparent {p:.1}%);background-size:100% 70%;background-repeat:no-repeat;background-position:left center;"
        );
    }
    if cf.hide_value {
        inner.clear();
    }
    if let Some((set, level, n)) = &cf.icon {
        let (g, color) = icon_glyph(set, *level, *n);
        *inner = format!("<span class=\"rc-icon\" style=\"color:{color}\">{g}</span>{inner}");
    }
}

/// 아이콘 집합의 단계 → (글리프, 색)
fn icon_glyph(set: &str, level: usize, n: usize) -> (&'static str, &'static str) {
    const RYG: [&str; 5] = ["#E5333B", "#F4A700", "#F4D100", "#8DC63F", "#21A366"];
    let pick = |i: usize| -> &'static str {
        // 단계 수에 맞춰 빨강 → 초록 사이에서 고른다
        let k = if n <= 1 { 0 } else { i * 4 / (n - 1) };
        RYG[k.min(4)]
    };
    if set.contains("Arrows") {
        const A3: [&str; 3] = ["↓", "→", "↑"];
        const A4: [&str; 4] = ["↓", "↘", "↗", "↑"];
        const A5: [&str; 5] = ["↓", "↘", "→", "↗", "↑"];
        let g = match n {
            3 => A3[level.min(2)],
            4 => A4[level.min(3)],
            _ => A5[level.min(4)],
        };
        (
            g,
            if set.contains("Gray") {
                "#808080"
            } else {
                pick(level)
            },
        )
    } else if set.contains("Quarters") {
        (["○", "◔", "◑", "◕", "●"][level.min(4)], "#404040")
    } else if set.contains("Rating") || set.contains("Boxes") {
        (
            ["▁", "▃", "▅", "▇", "█"][level * 4 / n.max(2).saturating_sub(1).max(1)],
            "#2F6FB7",
        )
    } else if set.contains("Stars") {
        (["☆", "⯪", "★"][level.min(2)], "#F4B400")
    } else if set.contains("Triangles") {
        (["▼", "▬", "▲"][level.min(2)], pick(level))
    } else if set.contains("Flags") {
        ("⚑", pick(level))
    } else if set.contains("RedToBlack") {
        (
            "●",
            ["#000000", "#808080", "#F08080", "#E5333B"][level.min(3)],
        )
    } else {
        ("●", pick(level))
    }
}

// ---------------------------------------------------------------------------
// 그리기 개체
// ---------------------------------------------------------------------------

/// 앵커 위치(머리글 포함 표 좌표, px). 숨긴 행·열은 0 폭.
fn anchor_px(lay: &Layout, opts: Options, m: &crate::model::Marker) -> (f64, f64) {
    let x0 = if opts.headers { HEADER_COL_PX } else { 0 };
    let y0 = if opts.headers { HEADER_ROW_PX } else { 0 };
    let x: u32 = lay
        .cols
        .iter()
        .take_while(|&&c| c < m.col)
        .map(|c| lay.col_width[c])
        .sum();
    let y: u32 = lay
        .rows
        .iter()
        .take_while(|&&r| r < m.row)
        .map(|r| lay.row_height[r])
        .sum();
    let emu = crate::units::EMU_PER_PX as f64;
    (
        (x0 + x) as f64 + m.col_off as f64 / emu,
        (y0 + y) as f64 + m.row_off as f64 / emu,
    )
}

/// 그리기 개체를 표 위에 절대 위치로 그린다.
fn shapes_html(wb: &Workbook, sheet: &Sheet, lay: &Layout, opts: Options, theme: &Theme) -> String {
    let mut s = String::new();
    for sh in sheet.shapes.iter().filter(|x| !x.hidden) {
        let a = &sh.anchor;
        let (x1, y1) = anchor_px(lay, opts, &a.from);
        let (x2, y2) = match (&a.to, a.ext) {
            (Some(t), _) => anchor_px(lay, opts, t),
            (None, Some((cx, cy))) => {
                let e = crate::units::EMU_PER_PX as f64;
                (x1 + cx as f64 / e, y1 + cy as f64 / e)
            }
            _ => (x1, y1),
        };
        let (w, h) = ((x2 - x1).max(0.0), (y2 - y1).max(0.0));
        let rot = sh
            .xfrm
            .as_ref()
            .filter(|x| x.rot != 0)
            .map(|x| format!("transform:rotate({}deg);", x.rot as f64 / 60000.0))
            .unwrap_or_default();
        let _ = write!(
            s,
            "<div class=\"rc-shape\" title=\"{}\" style=\"left:{x1:.1}px;top:{y1:.1}px;width:{w:.1}px;height:{h:.1}px;{rot}",
            esc(&sh.name)
        );
        s.push_str(&shape_body(wb, sh, w, h, theme));
        s.push_str("</div>");
    }
    s
}

/// 개체 하나의 style 나머지와 내용 (`style="…` 다음부터 `</div>` 앞까지)
fn shape_body(wb: &Workbook, sh: &crate::model::Shape, w: f64, h: f64, theme: &Theme) -> String {
    use crate::model::{Paint, ShapeKind};
    let mut css = String::new();
    let mut inner = String::new();
    match sh.kind {
        ShapeKind::Chart => {
            css.push_str("border:1px solid #bbb;background:#fff;");
            match &sh.chart {
                Some(ch) => inner.push_str(&crate::chart_svg::chart_svg(ch, w, h, theme, &|c| {
                    draw_color(c, theme)
                })),
                None => inner.push_str("<div class=\"rc-chart\">차트</div>"),
            }
        }
        ShapeKind::Picture => {
            let img = sh
                .target
                .as_ref()
                .and_then(|t| wb.raw_parts.iter().find(|p| &p.name == t));
            if let Some(p) = img {
                let ct = p.content_type.as_deref().unwrap_or("image/png");
                let _ = write!(
                    inner,
                    "<img alt=\"\" style=\"width:100%;height:100%\" src=\"data:{ct};base64,{}\">",
                    base64(&p.data)
                );
            }
        }
        ShapeKind::Connector => {
            let stroke = sh
                .line
                .as_ref()
                .and_then(|l| l.fill.as_ref())
                .and_then(|f| match f {
                    Paint::Solid(c) => draw_color(c, theme),
                    _ => None,
                })
                .unwrap_or_else(|| "#000".into());
            let flip_h = sh.xfrm.as_ref().is_some_and(|x| x.flip_h);
            let flip_v = sh.xfrm.as_ref().is_some_and(|x| x.flip_v);
            let (ax, bx) = if flip_h { (w, 0.0) } else { (0.0, w) };
            let (ay, by) = if flip_v { (h, 0.0) } else { (0.0, h) };
            let _ = write!(
                inner,
                "<svg width=\"{w:.1}\" height=\"{h:.1}\" style=\"overflow:visible\"><line x1=\"{ax:.1}\" y1=\"{ay:.1}\" x2=\"{bx:.1}\" y2=\"{by:.1}\" stroke=\"{stroke}\"/></svg>"
            );
        }
        _ => {
            // 모양이 없거나 (경로를 아직 그리지 않는) 사용자 정의 모양이면 채우기·선은 그리지 않는다
            let painted = sh.geometry.as_deref().is_some_and(|g| g != "custom");
            if painted
                && let Some(Paint::Solid(c)) = &sh.fill
                && let Some(col) = draw_color(c, theme)
            {
                let _ = write!(css, "background:{col};");
            }
            if painted
                && let Some(l) = &sh.line
                && let Some(Paint::Solid(c)) = &l.fill
                && let Some(col) = draw_color(c, theme)
            {
                let px =
                    (l.width.unwrap_or(9525) as f64 / crate::units::EMU_PER_PX as f64).max(1.0);
                let style = match l.dash.as_deref() {
                    Some("sysDot" | "dot") => "dotted",
                    Some(d) if d != "solid" => "dashed",
                    _ => "solid",
                };
                let _ = write!(
                    css,
                    "outline:{px:.1}px {style} {col};outline-offset:-{px:.1}px;"
                );
            }
            match sh.geometry.as_deref() {
                Some("ellipse") => css.push_str("border-radius:50%;"),
                Some("roundRect") => css.push_str("border-radius:12%;"),
                _ => {}
            }
            if let Some(t) = &sh.text {
                inner.push_str(&text_html(t, theme));
            }
        }
    }
    format!("{css}\">{inner}")
}

fn text_html(t: &crate::model::TextBody, theme: &Theme) -> String {
    let e = crate::units::EMU_PER_PX as f64;
    let pad = |v: Option<i64>, d: i64| v.unwrap_or(d) as f64 / e;
    let justify = match t.anchor.as_deref() {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    };
    let mut s = format!(
        "<div class=\"rc-text\" style=\"justify-content:{justify};padding:{:.1}px {:.1}px {:.1}px {:.1}px;{}\">",
        pad(t.insets[1], 45720),
        pad(t.insets[2], 91440),
        pad(t.insets[3], 45720),
        pad(t.insets[0], 91440),
        if t.wrap.as_deref() == Some("none") {
            "white-space:pre"
        } else {
            "white-space:pre-wrap"
        }
    );
    for p in &t.paragraphs {
        let align = match p.align.as_deref() {
            Some("ctr") => "center",
            Some("r") => "right",
            Some("just") => "justify",
            _ => "left",
        };
        let _ = write!(s, "<p style=\"text-align:{align}\">");
        if p.runs.is_empty() {
            s.push_str("&nbsp;");
        }
        for r in &p.runs {
            if r.text == "\n" {
                s.push_str("<br>");
                continue;
            }
            let mut st = String::new();
            if let Some(z) = r.size {
                let _ = write!(st, "font-size:{}pt;", z as f64 / 100.0);
            }
            if r.bold == Some(true) {
                st.push_str("font-weight:bold;");
            }
            if r.italic == Some(true) {
                st.push_str("font-style:italic;");
            }
            if r.underline.as_deref().is_some_and(|u| u != "none") {
                st.push_str("text-decoration:underline;");
            }
            if let Some(c) = r.color.as_ref().and_then(|c| draw_color(c, theme)) {
                let _ = write!(st, "color:{c};");
            }
            if let Some(f) = r.ea.as_ref().or(r.latin.as_ref()) {
                let _ = write!(st, "font-family:'{}',sans-serif;", f.replace('\'', ""));
            }
            let _ = write!(s, "<span style=\"{st}\">{}</span>", esc(&r.text));
        }
        s.push_str("</p>");
    }
    s.push_str("</div>");
    s
}

/// DrawingML 색 → CSS (#RRGGBB). 테마 색과 변형(lumMod/lumOff/shade/tint)을 적용한다.
fn draw_color(c: &crate::model::DrawColor, theme: &Theme) -> Option<String> {
    let (kind, val) = c.value.split_once(':')?;
    let hex = |s: &str| -> Option<(f64, f64, f64)> {
        let v = u32::from_str_radix(s.get(s.len().saturating_sub(6)..)?, 16).ok()?;
        Some((
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
        ))
    };
    let mut rgb = match kind {
        "srgb" => hex(val)?,
        "scheme" => {
            let i = match val {
                "dk1" | "tx1" => 0,
                "lt1" | "bg1" => 1,
                "dk2" | "tx2" => 2,
                "lt2" | "bg2" => 3,
                "accent1" => 4,
                "accent2" => 5,
                "accent3" => 6,
                "accent4" => 7,
                "accent5" => 8,
                "accent6" => 9,
                "hlink" => 10,
                "folHlink" => 11,
                _ => return None,
            };
            hex(theme.colors.get(i)?)?
        }
        "sys" => {
            if val.eq_ignore_ascii_case("window") {
                (255.0, 255.0, 255.0)
            } else {
                (0.0, 0.0, 0.0)
            }
        }
        "prst" => match val {
            "white" => (255.0, 255.0, 255.0),
            "red" => (255.0, 0.0, 0.0),
            "green" => (0.0, 128.0, 0.0),
            "blue" => (0.0, 0.0, 255.0),
            "yellow" => (255.0, 255.0, 0.0),
            _ => (0.0, 0.0, 0.0),
        },
        _ => return None,
    };
    for (m, v) in &c.mods {
        let f = *v as f64 / 100_000.0;
        match m.as_str() {
            "shade" => rgb = (rgb.0 * f, rgb.1 * f, rgb.2 * f),
            "tint" => {
                rgb = (
                    255.0 - (255.0 - rgb.0) * f,
                    255.0 - (255.0 - rgb.1) * f,
                    255.0 - (255.0 - rgb.2) * f,
                )
            }
            "lumMod" | "lumOff" => {
                let (h, sat, l) = rgb_to_hsl(rgb);
                let l = if m == "lumMod" {
                    l * f
                } else {
                    (l + f).min(1.0)
                };
                rgb = hsl_to_rgb(h, sat, l);
            }
            _ => {}
        }
    }
    let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Some(format!("#{:02X}{:02X}{:02X}", c(rgb.0), c(rgb.1), c(rgb.2)))
}

fn rgb_to_hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l * 255.0, l * 255.0, l * 255.0);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// 표준 Base64 (그림을 data: URI 로 넣을 때)
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// 뷰어 공통 CSS.
pub const CSS: &str = "\
.rc-book{font-family:'맑은 고딕','Malgun Gothic',sans-serif;color:#000}\
.rc-tabs{display:flex;gap:2px;flex-wrap:wrap;margin:8px 0;padding:0;list-style:none}\
.rc-tabs button{border:1px solid #c8c8c8;background:#f3f3f3;padding:4px 12px;cursor:pointer;font:inherit}\
.rc-tabs button[aria-selected=true]{background:#fff;border-bottom-color:#fff;font-weight:bold}\
.rc-wrap{overflow:auto;max-height:80vh;border:1px solid #c8c8c8;background:#fff}\
table.rc-sheet{border-collapse:collapse;table-layout:fixed;background:#fff}\
table.rc-sheet td{padding:0 3px;overflow:hidden;white-space:nowrap;font-size:11pt;vertical-align:middle}\
table.rc-grid td{border:1px solid #e1e1e1}\
table.rc-sheet th{background:#f0f0f0;border:1px solid #c8c8c8;font-weight:normal;font-size:9pt;color:#444;position:sticky;top:0;z-index:3}\
table.rc-sheet thead th{height:20px;padding:0}\
table.rc-sheet thead th.rc-corner{left:0;z-index:5}\
table.rc-sheet tbody th{position:sticky;left:0;text-align:center}\
.rc-hidden{display:none}\
.rc-canvas{position:relative;display:inline-block}\
.rc-icon{display:inline-block;margin-right:3px;font-weight:normal}\
.rc-shape{position:absolute;box-sizing:border-box;overflow:hidden;pointer-events:auto}\
.rc-text{display:flex;flex-direction:column;box-sizing:border-box;width:100%;height:100%;font-size:11pt;line-height:1.2}\
.rc-text p{margin:0}\
.rc-chart{display:flex;align-items:center;justify-content:center;width:100%;height:100%;color:#888;font-size:10pt}\
table.rc-sheet td.rc-note{position:relative}\
table.rc-sheet td.rc-note::after{content:'';position:absolute;top:0;right:0;border-style:solid;border-width:0 6px 6px 0;border-color:transparent #d00 transparent transparent}\
table.rc-sheet td.rc-note:hover::before{content:attr(data-note);position:absolute;left:100%;top:0;z-index:10;white-space:pre-wrap;min-width:12em;max-width:24em;padding:4px 6px;background:#ffffe1;border:1px solid #000;font-size:9pt;font-weight:normal;font-style:normal;color:#000;text-align:left}";

/// 통합 문서 전체를 HTML 조각으로 (시트 탭 포함). 스크립트 없이도 첫 시트가 보인다.
pub fn workbook_fragment(wb: &Workbook, opts: Options) -> String {
    let visible: Vec<(usize, &Sheet)> = wb
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.state == crate::model::SheetState::Visible)
        .collect();
    // 처음 보일 시트: 지정한 것 → 선택된 탭 → 처음 보이는 시트
    let shown = opts
        .active_sheet
        .filter(|a| visible.iter().any(|(i, _)| i == a))
        .or_else(|| {
            visible
                .iter()
                .find(|(_, s)| s.view.tab_selected)
                .map(|(i, _)| *i)
        })
        .or_else(|| visible.first().map(|(i, _)| *i));
    let mut s = String::from("<div class=\"rc-book\"><div class=\"rc-tabs\" role=\"tablist\">");
    for (i, sh) in &visible {
        let _ = write!(
            s,
            "<button role=\"tab\" aria-selected=\"{}\" data-sheet=\"{i}\">{}</button>",
            Some(*i) == shown,
            esc(&sh.name)
        );
    }
    s.push_str("</div>");
    for (i, sh) in &visible {
        let _ = write!(
            s,
            "<div class=\"rc-wrap{}\" data-sheet=\"{i}\">{}</div>",
            if Some(*i) == shown { "" } else { " rc-hidden" },
            sheet_table(wb, sh, opts)
        );
    }
    s.push_str("</div>");
    s
}

/// 탭 전환 스크립트.
pub const TABS_JS: &str = "document.addEventListener('click',function(e){var b=e.target.closest&&e.target.closest('.rc-tabs button');if(!b)return;var book=b.closest('.rc-book');book.querySelectorAll('.rc-tabs button').forEach(function(x){x.setAttribute('aria-selected',x===b)});book.querySelectorAll('.rc-wrap').forEach(function(w){w.classList.toggle('rc-hidden',w.dataset.sheet!==b.dataset.sheet)})});";

/// 독립 실행 HTML 문서.
pub fn workbook_html(wb: &Workbook, title: &str, opts: Options) -> String {
    format!(
        "<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><meta name=\"generator\" content=\"rcell (yoomin53)\"><style>body{{margin:16px;background:#fafafa}}{CSS}</style></head><body>{}<script>{TABS_JS}</script></body></html>",
        esc(title),
        workbook_fragment(wb, opts)
    )
}
