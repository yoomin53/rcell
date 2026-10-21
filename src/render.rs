// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 시트를 HTML 로 그린다 (명령줄 `rcell html` 과 웹 뷰어가 함께 쓴다).
//!
//! 한셀 화면과 같게 그리는 것이 목표다 (한셀 COM·화면 갈무리 대조 실험, docs/format/screen.md):
//! - 열 너비는 글꼴 0 의 숫자 폭 K 로 w·K px, 행 높이는 pt → px 를 행마다 반올림 ([`metrics`]).
//!   높이를 정하지 않은 행은 글꼴 크기·줄 바꿈으로 맞춘다.
//! - 격자선(#D9D9D9)은 칸의 오른쪽·아래 끝 픽셀. 채운 칸은 격자선 자리까지 덮는다.
//! - 테두리·대각선은 칸마다 SVG 로 경계 픽셀에 그린다 ([`paint`]). 무늬 18종·그라데이션.
//! - 글자는 빈 이웃 칸으로 넘친다 (왼쪽·오른쪽·가운데, 값이 있는 칸에서 멈춤, 넘친 자리의 세로선은
//!   지운다). 숫자는 칸에 맞게 줄이거나 `#` 로 채운다 ([`numfit`]).
//! - 도형은 미리 정의된 모양과 사용자 정의 경로를 SVG 로 ([`shape`]), 스파크라인은 칸 안 SVG
//!   ([`sparkline`]), 차트는 [`crate::chart_svg`].

mod metrics;
mod numfit;
mod paint;
mod shape;
mod sparkline;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

pub use metrics::{FontMetrics, col_px, default_col_chars, row_px};
pub use numfit::{general, general_fit};
pub use sparkline::{Extras, SparklineGroup};

use crate::model::{
    Alignment, CellRef, CellValue, Font, RichText, RowInfo, Sheet, Workbook, Xf,
};
use crate::theme::{self, Theme};
use crate::writer::esc;

/// 렌더링 옵션.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// 그릴 최대 행/열 (아주 큰 시트 보호)
    pub max_rows: u32,
    pub max_cols: u32,
    /// 행·열 머리글 (A B C / 1 2 3). 시트 보기에서 머리글을 끈 시트는 이 값과 상관없이 그리지 않는다
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

/// 격자선 색 (한셀 화면)
const GRID: &str = "#D9D9D9";
/// 칸 안 여백 (px, 한셀 화면: 글자가 경계에서 3px 안쪽에서 시작)
const PAD: f64 = 3.0;
/// 들여쓰기 한 단계 (px): 한셀 1134 HWPUNIT = 4mm
const INDENT_PX: f64 = 1134.0 / 7200.0 * 96.0;

// ---------------------------------------------------------------------------
// 배치
// ---------------------------------------------------------------------------

/// 화면 배치 (px, 시트 원점 = A1 왼쪽 위). 숨긴 열·행은 너비·높이 0
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    /// 열(0부터) → (왼쪽, 너비)
    pub cols: Vec<(u32, u32)>,
    /// 행(0부터) → (위, 높이)
    pub rows: Vec<(u32, u32)>,
}

struct Layout {
    rows: Vec<u32>,
    cols: Vec<u32>,
    col_width: HashMap<u32, u32>,
    row_height: HashMap<u32, u32>,
    col_x: HashMap<u32, u32>,
    row_y: HashMap<u32, u32>,
    /// 병합 시작 셀 → (보이는 행 수, 열 수)
    span: HashMap<(u32, u32), (u32, u32)>,
    /// 병합 시작 셀 → 끝 셀 (원래 좌표)
    merge_end: HashMap<(u32, u32), (u32, u32)>,
    covered: HashSet<(u32, u32)>,
    /// 기본 글꼴(글꼴 0) 측정값
    book: FontMetrics,
    max_c: u32,
}

impl Layout {
    fn col_w(&self, c: u32) -> u32 {
        self.col_width.get(&c).copied().unwrap_or(0)
    }
    fn row_h(&self, r: u32) -> u32 {
        self.row_height.get(&r).copied().unwrap_or(0)
    }
    /// 칸(병합이면 병합 영역 전체)의 너비·높이
    fn cell_size(&self, r: u32, c: u32) -> (u32, u32) {
        match self.merge_end.get(&(r, c)) {
            Some(&(r1, c1)) => (
                (c..=c1).map(|k| self.col_w(k)).sum(),
                (r..=r1).map(|k| self.row_h(k)).sum(),
            ),
            None => (self.col_w(c), self.row_h(r)),
        }
    }
}

/// 통합 문서의 기본 글꼴(글꼴 0) 측정값. 한셀은 표준 스타일이 아니라 글꼴 목록의 첫 글꼴로
/// 열 너비 단위를 정한다 (대조 실험: 표준 스타일 글꼴만 바꾼 문서의 열 너비는 그대로)
pub fn book_metrics(wb: &Workbook) -> FontMetrics {
    let f = wb.styles.fonts.first();
    FontMetrics::new(
        f.and_then(|f| f.name.as_deref()),
        f.and_then(|f| f.size),
        false,
    )
}

fn font_metrics(f: Option<&Font>) -> FontMetrics {
    FontMetrics::new(
        f.and_then(|f| f.name.as_deref()),
        f.and_then(|f| f.size),
        f.is_some_and(|f| f.bold),
    )
}

/// 기본 열 너비 (px). defaultColWidth 가 없으면 한셀 기본(8글자 기준), 정수면 레거시 DEFCOLWIDTH
/// 같은 "기준 너비"로 본다 (한셀이 레거시를 읽은 결과와 같다: 8 → 9.140625)
pub fn default_col_px(sheet: &Sheet, k: f64) -> u32 {
    match sheet.default_col_width {
        None => col_px(default_col_chars(8.0, k), k),
        Some(w) if w.fract() == 0.0 => col_px(default_col_chars(w, k), k),
        Some(w) => col_px(w, k),
    }
}

/// 글자 줄 수 어림 (줄 바꿈 칸): 너비 `avail` px 에 맞춰 한글은 아무 데서나, 영문은 단어 단위로 끊는다
fn wrap_lines(text: &str, avail: f64, m: &FontMetrics) -> u32 {
    let avail = avail.max(m.px);
    let mut lines = 0;
    for para in text.split('\n') {
        lines += 1;
        let mut x = 0.0;
        let mut word = 0.0;
        for ch in para.chars() {
            let w = m.char_width(ch);
            if ch == ' ' || metrics::is_wide(ch) {
                // 끊을 수 있는 자리
                if x + word + w > avail && x + word > 0.0 {
                    lines += 1;
                    x = if ch == ' ' { 0.0 } else { word + w };
                    word = 0.0;
                    if ch != ' ' {
                        x = w;
                    }
                    continue;
                }
                x += word + w;
                word = 0.0;
            } else {
                word += w;
                if x + word > avail && x > 0.0 {
                    lines += 1;
                    x = 0.0;
                }
                while word > avail {
                    lines += 1;
                    word -= avail;
                }
            }
        }
    }
    lines.max(1)
}

/// 높이를 정하지 않은 행의 자동 높이 (pt): 기본 글꼴과 행 안 글꼴들의 기본 높이 중 최대.
/// 줄 바꿈 칸은 줄 수만큼, 돌린 글자는 돌린 상자 높이 (한셀 대조 실험)
fn autofit_pt(
    wb: &Workbook,
    sheet: &Sheet,
    lay_cols: &HashMap<u32, u32>,
    book: &FontMetrics,
    row: u32,
    cells: &[&crate::model::Cell],
    row_style: Option<u32>,
    merged: &HashSet<(u32, u32)>,
) -> f64 {
    let st = &wb.styles;
    let mut h = book.row_pt;
    if let Some(s) = row_style {
        h = h.max(font_metrics(st.font_of(s)).row_pt);
    }
    for c in cells {
        if merged.contains(&(row, c.r.col)) {
            continue;
        }
        let m = font_metrics(st.font_of(c.style));
        let al = st.xf(c.style).and_then(|x| x.alignment.as_ref());
        let rot = al.and_then(|a| a.text_rotation).unwrap_or(0);
        let text = display_plain(wb, c.style, &c.value);
        let one = m.row_pt;
        let ch = if rot == 255 {
            text.chars().count().max(1) as f64 * one
        } else if rot != 0 {
            let deg = if rot > 90 { rot - 90 } else { rot } as f64;
            let tw = m.text_width(&text) * 0.75;
            tw * deg.to_radians().sin() + one * deg.to_radians().cos()
        } else if al.is_some_and(|a| a.wrap_text || a.horizontal.as_deref() == Some("justify"))
            && !text.is_empty()
        {
            let w = lay_cols.get(&c.r.col).copied().unwrap_or(64) as f64 - 2.0 * PAD - 1.0;
            let n = wrap_lines(&text, w, &m) as f64;
            n * one - (n - 1.0) / 30.0
        } else {
            one
        };
        h = h.max(ch);
    }
    let _ = sheet;
    (h * 20.0).round() / 20.0
}

/// 칸 값의 표시 글자 (줄이기 전, 행 높이 맞춤용)
fn display_plain(wb: &Workbook, style: u32, v: &CellValue) -> String {
    let code = wb
        .styles
        .xf(style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    crate::numfmt::format(v, &code, wb.date1904).text
}

fn layout(wb: &Workbook, sheet: &Sheet, opts: Options) -> Layout {
    let book = book_metrics(wb);
    let k = book.k;
    let default_col = default_col_px(sheet, k);
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
                col_width.insert(col, col_px(w, k));
            }
        }
    }
    let cols: Vec<u32> = (0..max_c).filter(|c| !hidden_cols.contains(c)).collect();
    for &c in &cols {
        col_width.entry(c).or_insert(default_col);
    }
    col_width.retain(|c, _| !hidden_cols.contains(c));

    // 병합 칸은 자동 높이에서 빠진다 (대조 실험)
    let mut merged_cells = HashSet::new();
    for m in &sheet.merges {
        for r in m.start.row..=m.end.row {
            for c in m.start.col..=m.end.col {
                merged_cells.insert((r, c));
            }
        }
    }
    let row_info: HashMap<u32, &RowInfo> = sheet.rows.iter().map(|r| (r.index - 1, r)).collect();
    let mut by_row: HashMap<u32, Vec<&crate::model::Cell>> = HashMap::new();
    for c in &sheet.cells {
        if c.r.row < max_r && (!c.value.is_empty() || c.style != 0) {
            by_row.entry(c.r.row).or_default().push(c);
        }
    }
    // 한셀이 저장한 문서의 행 높이(ht)는 한셀 자신이 맞춘 값이다
    let trusted = wb.is_written_by_hancom();
    let sheet_default = sheet.default_row_height.unwrap_or(book.row_pt);
    let mut row_height = HashMap::new();
    let mut rows = Vec::new();
    for r in 0..max_r {
        let info = row_info.get(&r);
        if info.is_some_and(|i| i.hidden) {
            continue;
        }
        let cells = by_row.get(&r).map(Vec::as_slice).unwrap_or(&[]);
        let pt = match info {
            Some(i) if i.height.is_some() && (i.custom_height || trusted) => i.height.unwrap_or(0.0),
            Some(i) if i.custom_height => sheet_default,
            Some(i) if !trusted || i.height.is_none() && !cells.is_empty() => {
                if trusted && cells.is_empty() {
                    sheet_default
                } else {
                    autofit_pt(wb, sheet, &col_width, &book, r, cells, i.style, &merged_cells)
                }
            }
            None if !cells.is_empty() && !trusted => {
                autofit_pt(wb, sheet, &col_width, &book, r, cells, None, &merged_cells)
            }
            _ => sheet_default,
        };
        rows.push(r);
        row_height.insert(r, row_px(pt));
    }
    let hidden_rows: HashSet<u32> = (0..max_r).filter(|r| !row_height.contains_key(r)).collect();

    let mut col_x = HashMap::new();
    let mut x = 0;
    for &c in &cols {
        col_x.insert(c, x);
        x += col_width[&c];
    }
    let mut row_y = HashMap::new();
    let mut y = 0;
    for &r in &rows {
        row_y.insert(r, y);
        y += row_height[&r];
    }

    let mut span = HashMap::new();
    let mut merge_end = HashMap::new();
    let mut covered = HashSet::new();
    for m in &sheet.merges {
        let rs = (m.start.row..=m.end.row)
            .filter(|r| !hidden_rows.contains(r))
            .count() as u32;
        let cs = (m.start.col..=m.end.col)
            .filter(|c| !hidden_cols.contains(c))
            .count() as u32;
        span.insert((m.start.row, m.start.col), (rs.max(1), cs.max(1)));
        merge_end.insert((m.start.row, m.start.col), (m.end.row, m.end.col));
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
        col_x,
        row_y,
        span,
        merge_end,
        covered,
        book,
        max_c,
    }
}

/// 시트의 화면 배치 (px). 한셀 화면과 같은 규칙으로 계산한다 (테스트·대조 도구용)
pub fn sheet_geometry(wb: &Workbook, sheet: &Sheet, max_rows: u32, max_cols: u32) -> Geometry {
    let lay = layout(
        wb,
        sheet,
        Options {
            max_rows,
            max_cols,
            ..Default::default()
        },
    );
    let mut g = Geometry::default();
    let mut x = 0;
    for c in 0..max_cols.min(lay.max_c.max(max_cols)) {
        let w = lay.col_width.get(&c).copied().unwrap_or(0);
        g.cols.push((x, w));
        x += w;
    }
    let mut y = 0;
    for r in 0..max_rows {
        let h = lay.row_height.get(&r).copied().unwrap_or(0);
        g.rows.push((y, h));
        y += h;
    }
    g
}

// ---------------------------------------------------------------------------
// 칸 하나
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum HAlign {
    Left,
    Center,
    Right,
    Justify,
    Distributed,
    Fill,
    CenterContinuous,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum VAlign {
    Top,
    Center,
    Bottom,
}

/// 칸 하나를 그리는 데 필요한 것
struct CellView<'a> {
    value: CellValue,
    /// 표시 글자 (숫자는 칸에 맞춘 뒤)
    text: String,
    /// 서식 있는 글 (공유 문자열의 런)
    rich: Option<&'a RichText>,
    font: Option<&'a Font>,
    m: FontMetrics,
    /// 서식 코드의 색 ([Red] 등)
    code_color: Option<String>,
    h: HAlign,
    v: VAlign,
    wrap: bool,
    shrink: bool,
    rotation: i32,
    indent: u32,
    fill: Option<paint::FillPaint>,
    borders: paint::CellBorders,
    is_number: bool,
}

impl CellView<'_> {
    fn has_value(&self) -> bool {
        !self.value.is_empty()
    }
}

fn h_align(a: Option<&Alignment>, v: &CellValue) -> (HAlign, bool) {
    match a.and_then(|a| a.horizontal.as_deref()).unwrap_or("general") {
        "left" => (HAlign::Left, false),
        "center" => (HAlign::Center, false),
        "right" => (HAlign::Right, false),
        "justify" => (HAlign::Justify, false),
        "distributed" => (HAlign::Distributed, false),
        "fill" => (HAlign::Fill, false),
        "centerContinuous" => (HAlign::CenterContinuous, false),
        _ => (
            match v {
                CellValue::Number(_) | CellValue::Date(_) => HAlign::Right,
                CellValue::Bool(_) | CellValue::Error(_) => HAlign::Center,
                _ => HAlign::Left,
            },
            true,
        ),
    }
}

fn v_align(a: Option<&Alignment>) -> VAlign {
    // 정렬 요소에 세로 정렬이 없으면 OOXML 기본(아래). 한셀이 저장한 문서는 기본 스타일에
    // vertical="center" 를 적어 둔다 (화면 대조: 세로 정렬을 뺀 칸은 아래에 붙는다)
    match a.and_then(|a| a.vertical.as_deref()).unwrap_or("bottom") {
        "top" => VAlign::Top,
        "center" | "distributed" | "justify" => VAlign::Center,
        _ => VAlign::Bottom,
    }
}

/// 서식 코드 색 이름 → CSS
fn code_color(c: &str) -> String {
    match c {
        "Red" => "#FF0000",
        "Blue" => "#0000FF",
        "Green" => "#00FF00",
        "Yellow" => "#FFFF00",
        "Magenta" => "#FF00FF",
        "Cyan" => "#00FFFF",
        "White" => "#FFFFFF",
        _ => "#000000",
    }
    .to_string()
}

fn borders_of(xf: Option<&Xf>, wb: &Workbook, theme: &Theme) -> paint::CellBorders {
    let Some(b) = xf.and_then(|x| wb.styles.borders.get(x.border_id as usize)) else {
        return Default::default();
    };
    let e = |x| paint::edge(x, theme);
    paint::CellBorders {
        left: e(b.left.as_ref()),
        right: e(b.right.as_ref()),
        top: e(b.top.as_ref()),
        bottom: e(b.bottom.as_ref()),
        down: if b.diagonal_down { e(b.diagonal.as_ref()) } else { None },
        up: if b.diagonal_up { e(b.diagonal.as_ref()) } else { None },
    }
}

/// 글꼴 → CSS (가족·크기·굵게·기울임·밑줄·취소선·색·장평 제외)
fn font_css(f: Option<&Font>, theme: &Theme, color: Option<&str>) -> String {
    let mut css = String::new();
    let Some(f) = f else {
        if let Some(c) = color {
            let _ = write!(css, "color:{c};");
        }
        return css;
    };
    if let Some(n) = &f.name {
        let _ = write!(css, "font-family:{};", font_family(n));
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
    if f.underline.as_deref().is_some_and(|u| u != "none") {
        deco.push("underline");
    }
    if f.strike || f.double_strike {
        deco.push("line-through");
    }
    if !deco.is_empty() {
        let _ = write!(css, "text-decoration:{};", deco.join(" "));
        if f.underline.as_deref().is_some_and(|u| u.starts_with("double")) || f.double_strike {
            css.push_str("text-decoration-style:double;");
        }
    }
    match f.vert_align.as_deref() {
        Some("superscript") => css.push_str("vertical-align:super;font-size:smaller;"),
        Some("subscript") => css.push_str("vertical-align:sub;font-size:smaller;"),
        _ => {}
    }
    let c = color
        .map(str::to_string)
        .or_else(|| f.color.as_ref().and_then(|c| theme::resolve(c, theme)));
    if let Some(c) = c {
        let _ = write!(css, "color:{c};");
    }
    if let Some(h) = &f.hancom
        && let Some(sp) = h.spacing.filter(|&s| s != 0)
    {
        let _ = write!(css, "letter-spacing:{:.3}em;", sp as f64 / 100.0);
    }
    css
}

/// 글꼴 이름 → CSS font-family (한글 이름엔 영문 별칭을 붙인다)
fn font_family(n: &str) -> String {
    let alias = match n {
        "맑은 고딕" => Some("Malgun Gothic"),
        "굴림" => Some("Gulim"),
        "굴림체" => Some("GulimChe"),
        "돋움" => Some("Dotum"),
        "돋움체" => Some("DotumChe"),
        "바탕" => Some("Batang"),
        "바탕체" => Some("BatangChe"),
        "궁서" => Some("Gungsuh"),
        "궁서체" => Some("GungsuhChe"),
        "함초롬돋움" => Some("HCR Dotum"),
        "함초롬바탕" => Some("HCR Batang"),
        "한컴 고딕" => Some("Hancom Gothic"),
        _ => None,
    };
    let n = n.replace('\'', "");
    match alias {
        Some(a) => format!("'{n}','{a}',sans-serif"),
        None => format!("'{n}','맑은 고딕','Malgun Gothic',sans-serif"),
    }
}

fn ratio_of(f: Option<&Font>) -> Option<f64> {
    f.and_then(|f| f.hancom.as_ref())
        .and_then(|h| h.ratio)
        .filter(|&r| r != 100 && r > 0)
        .map(|r| r as f64 / 100.0)
}

// ---------------------------------------------------------------------------
// 시트
// ---------------------------------------------------------------------------

/// 머리글 행 높이·머리글 열 너비 (px, 한셀 화면: 28px + 경계 1px, 31px + 1px)
const HEADER_ROW_PX: u32 = 29;
const HEADER_COL_PX: u32 = 32;

/// 시트 하나를 `<table>` 로.
pub fn sheet_table(wb: &Workbook, sheet: &Sheet, opts: Options) -> String {
    sheet_table_with(wb, sheet, opts, &Extras::default())
}

/// 시트 하나를 `<table>` 로 (원본에서 다시 읽은 스파크라인 포함)
pub fn sheet_table_with(wb: &Workbook, sheet: &Sheet, opts: Options, extras: &Extras) -> String {
    let theme = wb.theme.clone().unwrap_or_default();
    let lay = layout(wb, sheet, opts);
    let st = &wb.styles;
    let cells: HashMap<(u32, u32), &crate::model::Cell> = sheet
        .cells
        .iter()
        .map(|c| ((c.r.row, c.r.col), c))
        .collect();
    let grid = sheet.show_grid_lines;
    let view = &sheet.view;
    let headers = opts.headers && view.show_row_col_headers;
    // 조건부 서식 결과 (칸 → 덧입힐 서식)
    let cfx = crate::cf_eval::evaluate(sheet, st, &theme);
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
    // 서식 있는 글: 공유 문자열 중 런이 있는 것
    let rich: HashMap<&str, &RichText> = wb
        .shared_strings
        .iter()
        .filter(|s| s.runs.len() > 1)
        .map(|s| (s.text.as_str(), s))
        .collect();
    // 스파크라인
    let sheet_index = wb
        .sheets
        .iter()
        .position(|s| std::ptr::eq(s, sheet))
        .unwrap_or(0);
    let spark_groups = extras.sparklines.get(&sheet.part).map(Vec::as_slice).unwrap_or(&[]);
    let sparks = sparkline::sheet_items(wb, sheet_index, spark_groups);

    // ---- 1. 칸마다 그릴 내용
    let mut views: HashMap<(u32, u32), CellView> = HashMap::new();
    for &r in &lay.rows {
        for &c in &lay.cols {
            if lay.covered.contains(&(r, c)) {
                continue;
            }
            let cell = cells.get(&(r, c));
            let style = cell.map(|x| x.style).unwrap_or_else(|| sheet.effective_style(r, c));
            let cf = cfx.get(&(r, c));
            if cell.is_none() && style == 0 && cf.is_none() {
                continue;
            }
            let formula = cell.and_then(|x| x.formula.as_ref()).filter(|f| !f.text.is_empty());
            // 수식 표시(showFormulas)면 값 대신 수식, 0 값 숨김(showZeros=0)이면 0 을 비운다
            let value = match (cell.map(|x| &x.value), formula) {
                (_, Some(f)) if view.show_formulas => CellValue::Text(format!("={}", f.text)),
                (Some(CellValue::Number(n)), _) if *n == 0.0 && !view.show_zeros => {
                    CellValue::Empty
                }
                (Some(v), _) => v.clone(),
                (None, _) => CellValue::Empty,
            };
            let xf = st.xf(style);
            let align = xf.and_then(|x| x.alignment.as_ref());
            let font = xf.and_then(|x| st.fonts.get(x.font_id as usize));
            let m = font_metrics(font);
            let (h, _general) = h_align(align, &value);
            let (w_px, _) = lay.cell_size(r, c);
            let origin = (lay.col_x[&c], lay.row_y[&r]);
            let fill = xf
                .and_then(|x| st.fills.get(x.fill_id as usize))
                .and_then(|f| paint::fill_css(f, &theme, origin));
            let code = xf
                .map(|x| crate::numfmt::code_for(st, x.num_fmt_id))
                .unwrap_or_else(|| "General".into());
            let shown = crate::numfmt::format(&value, &code, wb.date1904);
            let wrap = align.is_some_and(|a| a.wrap_text) || h == HAlign::Justify;
            let shrink = align.is_some_and(|a| a.shrink_to_fit);
            let is_number = matches!(value, CellValue::Number(_));
            // 숫자: 일반 서식은 칸에 맞춰 줄이고, 다른 서식은 넘치면 # 로 채운다
            let text = if shown.overflow {
                numfit::hashes(w_px as f64, &m)
            } else if let CellValue::Number(n) = value {
                let avail = w_px as f64 - align.and_then(|a| a.indent).unwrap_or(0) as f64 * INDENT_PX;
                if shrink || wrap {
                    if code.eq_ignore_ascii_case("general") { numfit::general(n) } else { shown.text }
                } else if code.eq_ignore_ascii_case("general") {
                    numfit::general_fit(n, avail, &m)
                } else if m.text_width(&shown.text) > avail {
                    numfit::hashes(avail, &m)
                } else {
                    shown.text
                }
            } else {
                shown.text
            };
            let rich = match &value {
                CellValue::Text(t) => rich.get(t.as_str()).copied(),
                _ => None,
            };
            views.insert(
                (r, c),
                CellView {
                    value,
                    text,
                    rich,
                    font,
                    m,
                    code_color: shown.color.as_deref().map(code_color),
                    h,
                    v: v_align(align),
                    wrap,
                    shrink,
                    rotation: align.and_then(|a| a.text_rotation).unwrap_or(0),
                    indent: align.and_then(|a| a.indent).unwrap_or(0),
                    fill,
                    borders: borders_of(xf, wb, &theme),
                    is_number,
                },
            );
        }
    }
    // 조건부 서식 채우기는 칸 채우기를 대신한다
    for (k, cf) in &cfx {
        let Some(v) = views.get_mut(k) else { continue };
        let col = |c: &crate::model::Color| theme::resolve(c, &theme);
        if let Some(c) = cf.scale.clone().or_else(|| cf.fill.as_ref().and_then(col)) {
            v.fill = Some(paint::FillPaint {
                css: format!("background:{c};"),
                color: Some(c),
            });
        }
    }

    // ---- 2. 넘치는 글자: 칸 → (왼쪽으로 쓸 수 있는 px, 오른쪽 px), 지울 세로 경계
    let next_col: HashMap<u32, u32> = lay.cols.windows(2).map(|w| (w[0], w[1])).collect();
    let prev_col: HashMap<u32, u32> = lay.cols.windows(2).map(|w| (w[1], w[0])).collect();
    let next_row: HashMap<u32, u32> = lay.rows.windows(2).map(|w| (w[0], w[1])).collect();
    let blocked = |r: u32, c: u32| -> bool {
        lay.covered.contains(&(r, c))
            || lay.span.contains_key(&(r, c))
            || views.get(&(r, c)).is_some_and(|v| v.has_value())
            || cells.get(&(r, c)).is_some_and(|x| x.formula.is_some())
            || sparks.contains_key(&(r, c))
    };
    let mut overflow: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    // 오른쪽 경계를 지울 칸
    let mut no_vline: HashSet<(u32, u32)> = HashSet::new();
    for (&(r, c), v) in &views {
        if !matches!(v.value, CellValue::Text(_)) || v.wrap || v.shrink || v.rotation != 0 {
            continue;
        }
        if lay.span.contains_key(&(r, c)) {
            continue;
        }
        let (cw, _) = lay.cell_size(r, c);
        let need = v.m.text_width(&v.text) * ratio_of(v.font).unwrap_or(1.0)
            + v.indent as f64 * INDENT_PX
            + 2.0 * PAD;
        let (go_left, go_right) = match v.h {
            HAlign::Left => (false, true),
            HAlign::Right => (true, false),
            HAlign::Center => (true, true),
            HAlign::CenterContinuous => (false, false),
            _ => continue,
        };
        // 선택 영역 가운데: 오른쪽으로 같은 정렬의 빈 칸까지를 한 칸처럼
        if v.h == HAlign::CenterContinuous {
            let mut right = 0;
            let mut k = c;
            while let Some(&n) = next_col.get(&k) {
                let same = views.get(&(r, n)).is_some_and(|x| {
                    x.h == HAlign::CenterContinuous && !x.has_value()
                });
                if !same || blocked(r, n) {
                    break;
                }
                right += lay.col_w(n);
                no_vline.insert((r, k));
                k = n;
            }
            if right > 0 {
                overflow.insert((r, c), (0, right));
            }
            continue;
        }
        if need <= cw as f64 {
            continue;
        }
        let extra = need - cw as f64;
        // 가운데는 양쪽으로 반씩
        let want_side = if go_left && go_right { extra / 2.0 } else { extra };
        let mut walk = |forward: bool| -> u32 {
            let mut avail = 0u32;
            let mut covered = 0.0;
            let mut k = c;
            loop {
                let n = if forward { next_col.get(&k) } else { prev_col.get(&k) };
                let Some(&n) = n else { break };
                if blocked(r, n) {
                    break;
                }
                // 실제로 글자가 지나가는 경계만 지운다
                if covered < want_side {
                    no_vline.insert(if forward { (r, k) } else { (r, n) });
                }
                covered += lay.col_w(n) as f64;
                avail += lay.col_w(n);
                k = n;
                if avail > 4000 {
                    break;
                }
            }
            avail
        };
        let left = if go_left { walk(false) } else { 0 };
        let right = if go_right { walk(true) } else { 0 };
        if left > 0 || right > 0 {
            overflow.insert((r, c), (left, right));
        }
    }

    // ---- 3. 격자선 자리: 칸의 오른쪽·아래 경계 픽셀 색
    // 채운 이웃이 있으면 그 색이 덮고, 넘친 글자가 지나가면 지운다
    let edge_color = |r: u32, c: u32, right: bool| -> Option<String> {
        let me = views.get(&(r, c));
        if right && no_vline.contains(&(r, c)) {
            return Some(
                me.and_then(|v| v.fill.as_ref())
                    .and_then(|f| f.color.clone())
                    .unwrap_or_else(|| "transparent".into()),
            );
        }
        let n = if right {
            next_col.get(&c).map(|&n| (r, n))
        } else {
            next_row.get(&r).map(|&n| (n, c))
        };
        if let Some(nb) = n.and_then(|k| views.get(&k)).and_then(|v| v.fill.as_ref()) {
            return nb.color.clone().or(Some("transparent".into()));
        }
        if me.is_some_and(|v| v.fill.is_some()) {
            return Some("transparent".into());
        }
        None
    };

    // 틀 고정: 고정 창 왼쪽 위 칸보다 위의 행, 왼쪽의 열은 스크롤해도 남는다 (position: sticky)
    let freeze = view
        .pane
        .as_ref()
        .filter(|p| p.state.as_deref().is_some_and(|s| s.starts_with("frozen")))
        .and_then(|p| p.top_left_cell.as_deref().and_then(CellRef::parse));
    let header_h = if headers { HEADER_ROW_PX } else { 0 };
    let header_w = if headers {
        // 행 번호 자릿수에 따라 넓어진다
        let digits = lay.rows.last().map_or(1, |r| (r + 1).to_string().len()) as u32;
        HEADER_COL_PX.max(digits * 7 + 11)
    } else {
        0
    };
    let mut sticky_top: HashMap<u32, u32> = HashMap::new();
    let mut sticky_left: HashMap<u32, u32> = HashMap::new();
    if let Some(f) = freeze {
        let mut y = header_h;
        for &r in lay.rows.iter().filter(|&&r| r < f.row) {
            sticky_top.insert(r, y);
            y += lay.row_h(r);
        }
        let mut x = header_w;
        for &c in lay.cols.iter().filter(|&&c| c < f.col) {
            sticky_left.insert(c, x);
            x += lay.col_w(c);
        }
    }
    let sticky = |r: u32, c: Option<u32>| -> String {
        let top = sticky_top.get(&r);
        let left = c.and_then(|c| sticky_left.get(&c));
        let mut css = String::new();
        if top.is_some() || left.is_some() {
            css.push_str("position:sticky;background-color:#fff;");
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
    if headers {
        let _ = write!(s, "<col style=\"width:{header_w}px\">");
    }
    for c in &lay.cols {
        let _ = write!(s, "<col style=\"width:{}px\">", lay.col_w(*c));
    }
    s.push_str("</colgroup>");
    if headers {
        let _ = write!(
            s,
            "<thead><tr style=\"height:{header_h}px\"><th class=\"rc-corner\"></th>"
        );
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
        let _ = write!(s, "<tr style=\"height:{}px\">", lay.row_h(r));
        if headers {
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
            let mut attrs = String::new();
            if let Some(&(rs, cs)) = lay.span.get(&(r, c)) {
                if rs > 1 {
                    let _ = write!(attrs, " rowspan=\"{rs}\"");
                }
                if cs > 1 {
                    let _ = write!(attrs, " colspan=\"{cs}\"");
                }
            }
            if let Some(n) = notes.get(&(r, c)) {
                let _ = write!(attrs, " class=\"rc-note\" data-note=\"{}\"", esc(n));
            }
            let (w, h) = lay.cell_size(r, c);
            let mut css = sticky(r, Some(c));
            // 병합 칸의 오른쪽·아래 경계는 병합 끝 칸 기준
            let (er, ec) = lay.merge_end.get(&(r, c)).copied().unwrap_or((r, c));
            let ec = if lay.col_width.contains_key(&ec) { ec } else { c };
            let er = if lay.row_height.contains_key(&er) { er } else { r };
            if let Some(col) = edge_color(r, ec, true) {
                let _ = write!(css, "border-right-color:{col};");
            }
            if let Some(col) = edge_color(er, c, false) {
                let _ = write!(css, "border-bottom-color:{col};");
            }
            let mut inner = String::new();
            if let Some(v) = views.get(&(r, c)) {
                if let Some(f) = &v.fill {
                    css.push_str(&f.css);
                }
                let cf = cfx.get(&(r, c));
                inner = cell_content(v, cf, &theme, w, h, overflow.get(&(r, c)).copied());
                // 테두리 (넘친 글자가 지나가는 세로 경계는 지운다)
                let mut b = v.borders.clone();
                if no_vline.contains(&(r, c)) {
                    b.right = None;
                }
                if prev_col.get(&c).is_some_and(|&p| no_vline.contains(&(r, p))) {
                    b.left = None;
                }
                if !b.is_empty() {
                    inner.push_str(&paint::borders_svg(&b, w, h));
                }
            }
            if let Some((gi, vals)) = sparks.get(&(r, c))
                && let Some(g) = spark_groups.get(*gi)
            {
                let lohi = sparkline::group_range(&sparks, *gi);
                inner.push_str(&sparkline::svg(
                    g,
                    vals,
                    lohi,
                    w.saturating_sub(1) as f64,
                    h.saturating_sub(1) as f64,
                    &theme,
                ));
            }
            if css.is_empty() {
                let _ = write!(s, "<td{attrs}>{inner}</td>");
            } else {
                let _ = write!(s, "<td{attrs} style=\"{css}\">{inner}</td>");
            }
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table>");
    s.push_str(&shapes_html(wb, sheet, &lay, header_w, header_h, &theme));
    s.push_str("</div>");
    s
}

/// 칸 안 글자 (`.rc-t` 상자). 넘치는 글자는 이웃 칸 위로 펼친 상자 안에 둔다
fn cell_content(
    v: &CellView,
    cf: Option<&crate::cf_eval::CfStyle>,
    theme: &Theme,
    w: u32,
    h: u32,
    overflow: Option<(u32, u32)>,
) -> String {
    let mut out = String::new();
    // 데이터 막대 (조건부 서식): 글자 뒤 칸 안쪽
    if let Some(cf) = cf
        && let Some((len, c)) = &cf.bar
        && let Some(c) = theme::resolve(c, theme)
    {
        let p = (len * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            out,
            "<div class=\"rc-bar\" style=\"width:calc({p:.1}% - 2px);background:linear-gradient(to right,{c},{c}33)\"></div>"
        );
    }
    let mut text = v.text.clone();
    if cf.is_some_and(|c| c.hide_value) {
        text.clear();
    }
    let icon = cf.and_then(|c| c.icon.as_ref()).map(|(set, level, n)| {
        let (g, color) = icon_glyph(set, *level, *n);
        format!("<span class=\"rc-icon\" style=\"color:{color}\">{g}</span>")
    });
    if text.is_empty() && icon.is_none() {
        return out;
    }
    // 글꼴·색 (조건부 서식이 덮는다)
    let color = cf
        .and_then(|c| c.font_color.as_ref())
        .and_then(|c| theme::resolve(c, theme))
        .or_else(|| v.code_color.clone());
    let mut fcss = font_css(v.font, theme, color.as_deref());
    if let Some(cf) = cf {
        if cf.bold == Some(true) {
            fcss.push_str("font-weight:bold;");
        }
        if cf.italic == Some(true) {
            fcss.push_str("font-style:italic;");
        }
        let mut deco = Vec::new();
        if cf.underline == Some(true) {
            deco.push("underline");
        }
        if cf.strike == Some(true) {
            deco.push("line-through");
        }
        if !deco.is_empty() {
            let _ = write!(fcss, "text-decoration:{};", deco.join(" "));
        }
    }
    let line = v.m.row_pt * 4.0 / 3.0;
    let _ = write!(fcss, "line-height:{line:.2}px;");
    let indent = v.indent as f64 * INDENT_PX;
    let body = rich_html(v, &text, theme, icon.as_deref());
    let ratio = ratio_of(v.font);
    let scaled = |inner: String, origin: &str| match ratio {
        Some(r) => format!(
            "<span style=\"display:inline-block;transform:scaleX({r:.3});transform-origin:{origin}\">{inner}</span>"
        ),
        None => inner,
    };
    let vpos = match v.v {
        VAlign::Top => "top:0",
        VAlign::Center => "top:50%",
        VAlign::Bottom => "bottom:0",
    };
    let vshift = if v.v == VAlign::Center { "-50%" } else { "0" };

    // 넘치는 한 줄 글자
    if let Some((left, right)) = overflow {
        let (pos, tx) = match v.h {
            HAlign::Right => (format!("right:{:.1}px", right as f64 + PAD + indent), "0"),
            HAlign::Center | HAlign::CenterContinuous => {
                let center = if v.h == HAlign::CenterContinuous {
                    (w + right) as f64 / 2.0
                } else {
                    left as f64 + w as f64 / 2.0
                };
                (format!("left:{center:.1}px"), "-50%")
            }
            _ => (format!("left:{:.1}px", left as f64 + PAD + indent), "0"),
        };
        let _ = write!(
            out,
            "<div class=\"rc-t rc-ov\" style=\"left:-{left}px;right:-{right}px;{fcss}\"><span style=\"{pos};{vpos};transform:translate({tx},{vshift})\">{}</span></div>",
            scaled(body, "left")
        );
        return out;
    }

    let mut css = fcss;
    let jc = match v.v {
        VAlign::Top => "flex-start",
        VAlign::Center => "center",
        VAlign::Bottom => "flex-end",
    };
    let _ = write!(css, "justify-content:{jc};");
    let (ta, origin) = match v.h {
        HAlign::Right => ("right", "right"),
        HAlign::Center | HAlign::CenterContinuous => ("center", "center"),
        HAlign::Justify => ("justify", "left"),
        HAlign::Distributed => ("justify", "left"),
        _ => ("left", "left"),
    };
    let _ = write!(css, "text-align:{ta};");
    match v.h {
        HAlign::Distributed => css.push_str("text-align-last:justify;padding:0;"),
        HAlign::Right if indent > 0.0 => {
            let _ = write!(css, "padding-right:{:.1}px;", PAD + indent);
        }
        HAlign::Left if indent > 0.0 => {
            let _ = write!(css, "padding-left:{:.1}px;", PAD + indent);
        }
        _ => {}
    }
    let avail = (w as f64 - 2.0 * PAD - indent - 1.0).max(1.0);
    let mut body = body;
    if v.wrap {
        css.push_str("white-space:pre-wrap;overflow-wrap:anywhere;");
    }
    if v.shrink && !v.wrap {
        let tw = v.m.text_width(&text) * ratio.unwrap_or(1.0);
        if tw > avail {
            let k = (avail / tw).max(0.1);
            let size = v.font.and_then(|f| f.size).unwrap_or(metrics::DEFAULT_SIZE) * k;
            let _ = write!(css, "font-size:{size:.2}pt;");
        }
    }
    if v.h == HAlign::Fill && !text.is_empty() && !v.is_number {
        // 칸 너비만큼 글자를 되풀이한다
        let tw = v.m.text_width(&text).max(1.0);
        let n = ((avail / tw).floor() as usize).max(1);
        body = esc(&text.repeat(n));
    }
    match v.rotation {
        0 => {
            let _ = write!(out, "<div class=\"rc-t\" style=\"{css}\">{}</div>", scaled(body, origin));
        }
        255 => {
            // 세로 쓰기: 글자를 위에서 아래로 쌓는다
            let _ = write!(
                out,
                "<div class=\"rc-t\" style=\"{css}align-items:center\"><span style=\"writing-mode:vertical-lr;text-orientation:upright\">{body}</span></div>"
            );
        }
        rot => {
            // 1–90: 반시계, 91–180: 시계 방향 (rot−90)
            let deg = if rot > 90 { -(rot - 90) } else { rot };
            let (x, y, origin) = if deg > 0 {
                (PAD, h as f64 - 1.0 - PAD, "left bottom")
            } else {
                (PAD, PAD, "left top")
            };
            if deg.abs() == 90 {
                let wm = if deg > 0 {
                    "writing-mode:vertical-rl;transform:rotate(180deg)"
                } else {
                    "writing-mode:vertical-rl"
                };
                let ai = match v.h {
                    HAlign::Left => "flex-start",
                    HAlign::Center => "center",
                    _ => "flex-end",
                };
                let jc2 = if deg > 0 { "flex-end" } else { "flex-start" };
                let _ = write!(
                    out,
                    "<div class=\"rc-t\" style=\"{css}justify-content:{jc2};align-items:{ai}\"><span style=\"{wm};line-height:{line:.2}px\">{body}</span></div>"
                );
            } else {
                let _ = write!(
                    out,
                    "<div class=\"rc-t\" style=\"{css}\"><span class=\"rc-rot\" style=\"left:{x:.1}px;top:{y:.1}px;transform-origin:{origin};transform:translateY({}) rotate({}deg)\">{body}</span></div>",
                    if deg > 0 { "-100%" } else { "0" },
                    -deg
                );
            }
        }
    }
    out
}

/// 글 (서식 있는 글이면 런마다 글꼴)
fn rich_html(v: &CellView, text: &str, theme: &Theme, icon: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(i) = icon {
        s.push_str(i);
    }
    match v.rich {
        Some(rt) if text == rt.text => {
            for run in &rt.runs {
                let css = run
                    .font
                    .as_ref()
                    .map(|f| font_css(Some(f), theme, None))
                    .unwrap_or_default();
                if css.is_empty() {
                    s.push_str(&esc(&run.text));
                } else {
                    let _ = write!(s, "<span style=\"{css}\">{}</span>", esc(&run.text));
                }
            }
        }
        _ => s.push_str(&esc(text)),
    }
    s
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
fn anchor_px(lay: &Layout, x0: u32, y0: u32, m: &crate::model::Marker) -> (f64, f64) {
    let x: u32 = lay
        .cols
        .iter()
        .take_while(|&&c| c < m.col)
        .map(|c| lay.col_w(*c))
        .sum();
    let y: u32 = lay
        .rows
        .iter()
        .take_while(|&&r| r < m.row)
        .map(|r| lay.row_h(*r))
        .sum();
    let emu = crate::units::EMU_PER_PX as f64;
    // 숨긴 열·행 안의 오프셋은 0 폭으로 접힌다
    let col_off = if lay.col_width.contains_key(&m.col) {
        (m.col_off as f64 / emu).min(lay.col_w(m.col) as f64)
    } else {
        0.0
    };
    let row_off = if lay.row_height.contains_key(&m.row) {
        (m.row_off as f64 / emu).min(lay.row_h(m.row) as f64)
    } else {
        0.0
    };
    ((x0 + x) as f64 + col_off, (y0 + y) as f64 + row_off)
}

/// 그리기 개체를 표 위에 절대 위치로 그린다.
fn shapes_html(
    wb: &Workbook,
    sheet: &Sheet,
    lay: &Layout,
    x0: u32,
    y0: u32,
    theme: &Theme,
) -> String {
    let mut s = String::new();
    for sh in sheet.shapes.iter().filter(|x| !x.hidden) {
        let a = &sh.anchor;
        let (x1, y1) = match (a.kind, a.pos) {
            (crate::model::AnchorKind::Absolute, Some((x, y))) => {
                let e = crate::units::EMU_PER_PX as f64;
                (x0 as f64 + x as f64 / e, y0 as f64 + y as f64 / e)
            }
            _ => anchor_px(lay, x0, y0, &a.from),
        };
        let (x2, y2) = match (&a.to, a.ext) {
            (Some(t), _) => anchor_px(lay, x0, y0, t),
            (None, Some((cx, cy))) => {
                let e = crate::units::EMU_PER_PX as f64;
                (x1 + cx as f64 / e, y1 + cy as f64 / e)
            }
            _ => (x1, y1),
        };
        let (w, h) = ((x2 - x1).max(0.0), (y2 - y1).max(0.0));
        let obj = sh.raw.as_deref().and_then(shape::object_element);
        s.push_str(&object_html(wb, sh, obj.as_ref(), (x1, y1, w, h), theme));
    }
    s
}

/// 개체 하나 (그룹이면 자식들을 그룹 좌표계로 옮겨 그린다)
fn object_html(
    wb: &Workbook,
    sh: &crate::model::Shape,
    obj: Option<&crate::xml::Element>,
    (x, y, w, h): (f64, f64, f64, f64),
    theme: &Theme,
) -> String {
    let mut s = String::new();
    let mut tf = Vec::new();
    if let Some(xf) = sh.xfrm.as_ref() {
        if xf.rot != 0 {
            tf.push(format!("rotate({}deg)", xf.rot as f64 / 60000.0));
        }
        if sh.kind != crate::model::ShapeKind::Connector {
            if xf.flip_h {
                tf.push("scaleX(-1)".into());
            }
            if xf.flip_v {
                tf.push("scaleY(-1)".into());
            }
        }
    }
    let tf = if tf.is_empty() {
        String::new()
    } else {
        format!("transform:{};", tf.join(" "))
    };
    if sh.kind == crate::model::ShapeKind::Group {
        // 자식 좌표: grpSpPr 의 chOff/chExt → 그룹 상자
        let xf = obj.and_then(shape::sp_pr).and_then(|p| p.child_ns(shape::A, "xfrm"));
        let num = |e: Option<&crate::xml::Element>, n: &str| {
            e.and_then(|e| e.attr(n)).and_then(|v| v.parse::<f64>().ok())
        };
        let ch_off = xf.and_then(|x| x.child_ns(shape::A, "chOff"));
        let ch_ext = xf.and_then(|x| x.child_ns(shape::A, "chExt"));
        let (cx0, cy0) = (num(ch_off, "x").unwrap_or(0.0), num(ch_off, "y").unwrap_or(0.0));
        let (cw, ch) = (
            num(ch_ext, "cx").filter(|v| *v > 0.0),
            num(ch_ext, "cy").filter(|v| *v > 0.0),
        );
        let raw_kids: Vec<&crate::xml::Element> = obj
            .map(|o| {
                o.elements()
                    .filter(|c| {
                        c.ns.as_deref() == Some(shape::XDR)
                            && matches!(c.name.as_str(), "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp")
                    })
                    .collect()
            })
            .unwrap_or_default();
        let _ = write!(
            s,
            "<div class=\"rc-shape rc-group\" title=\"{}\" style=\"left:{x:.1}px;top:{y:.1}px;width:{w:.1}px;height:{h:.1}px;{tf}\">",
            esc(&sh.name)
        );
        for (i, kid) in sh.children.iter().enumerate().filter(|(_, k)| !k.hidden) {
            let Some(kx) = kid.xfrm.as_ref() else { continue };
            let sx = cw.map_or(1.0 / crate::units::EMU_PER_PX as f64, |cw| w / cw);
            let sy = ch.map_or(1.0 / crate::units::EMU_PER_PX as f64, |ch| h / ch);
            let r = (
                (kx.x as f64 - cx0) * sx,
                (kx.y as f64 - cy0) * sy,
                kx.cx as f64 * sx,
                kx.cy as f64 * sy,
            );
            s.push_str(&object_html(wb, kid, raw_kids.get(i).copied(), r, theme));
        }
        s.push_str("</div>");
        return s;
    }
    let _ = write!(
        s,
        "<div class=\"rc-shape\" title=\"{}\" style=\"left:{x:.1}px;top:{y:.1}px;width:{w:.1}px;height:{h:.1}px;{tf}",
        esc(&sh.name)
    );
    s.push_str(&shape_body(wb, sh, obj, w, h, theme));
    s.push_str("</div>");
    s
}

/// 채우기 색 (모델 → 없으면 스타일 참조)
fn shape_fill(
    sh: &crate::model::Shape,
    obj: Option<&crate::xml::Element>,
    theme: &Theme,
    defs: &mut String,
    id: &str,
) -> Option<String> {
    use crate::model::Paint;
    match &sh.fill {
        Some(Paint::Solid(c)) => draw_color(c, theme),
        Some(Paint::None) => None,
        Some(Paint::Other(kind)) if kind == "gradFill" => {
            let (ang, stops) = obj.and_then(shape::grad_fill)?;
            let th = ang.to_radians();
            let (c, s) = (th.cos(), th.sin());
            let d = c.abs() + s.abs();
            let _ = write!(
                defs,
                "<linearGradient id=\"{id}\" x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\">",
                0.5 - c * d / 2.0,
                0.5 - s * d / 2.0,
                0.5 + c * d / 2.0,
                0.5 + s * d / 2.0
            );
            for (p, e) in &stops {
                let col = draw_color(&crate::parser::drawing::color(e), theme)
                    .unwrap_or_else(|| "#FFFFFF".into());
                let _ = write!(defs, "<stop offset=\"{p}\" stop-color=\"{col}\"/>");
            }
            defs.push_str("</linearGradient>");
            Some(format!("url(#{id})"))
        }
        Some(Paint::Other(_)) => Some("#D9D9D9".into()),
        None => obj
            .and_then(|o| shape::style_ref(o, "fillRef"))
            .map(crate::parser::drawing::color)
            .and_then(|c| draw_color(&c, theme)),
    }
}

/// 선 (색, 굵기 px, 점선)
fn shape_line(
    sh: &crate::model::Shape,
    obj: Option<&crate::xml::Element>,
    theme: &Theme,
) -> Option<(String, f64, Option<String>)> {
    use crate::model::Paint;
    let style_color = || {
        obj.and_then(|o| shape::style_ref(o, "lnRef"))
            .map(crate::parser::drawing::color)
            .and_then(|c| draw_color(&c, theme))
    };
    let (color, width, dash) = match &sh.line {
        Some(l) => {
            let c = match &l.fill {
                Some(Paint::Solid(c)) => draw_color(c, theme),
                Some(Paint::None) => return None,
                Some(Paint::Other(_)) => Some("#000000".into()),
                None => style_color(),
            }?;
            (c, l.width, l.dash.clone())
        }
        None => (style_color()?, None, None),
    };
    let px = (width.unwrap_or(9525) as f64 / crate::units::EMU_PER_PX as f64).max(1.0);
    let dash = dash.as_deref().and_then(|d| {
        let unit = px;
        let pat: &[f64] = match d {
            "sysDot" | "dot" => &[1.0, 1.0],
            "sysDash" => &[3.0, 1.0],
            "dash" => &[4.0, 3.0],
            "dashDot" | "sysDashDot" => &[4.0, 3.0, 1.0, 3.0],
            "lgDash" => &[8.0, 3.0],
            "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
            "lgDashDotDot" | "sysDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            _ => return None,
        };
        Some(
            pat.iter()
                .map(|v| format!("{:.1}", v * unit))
                .collect::<Vec<_>>()
                .join(" "),
        )
    });
    Some((color, px, dash))
}

/// 개체 하나의 style 나머지와 내용 (`style="…` 다음부터 `</div>` 앞까지)
fn shape_body(
    wb: &Workbook,
    sh: &crate::model::Shape,
    obj: Option<&crate::xml::Element>,
    w: f64,
    h: f64,
    theme: &Theme,
) -> String {
    use crate::model::ShapeKind;
    let mut css = String::new();
    let mut inner = String::new();
    match sh.kind {
        ShapeKind::Chart => {
            css.push_str("background:#fff;");
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
            let (stroke, px, dash) =
                shape_line(sh, obj, theme).unwrap_or_else(|| ("#000000".into(), 1.0, None));
            let flip_h = sh.xfrm.as_ref().is_some_and(|x| x.flip_h);
            let flip_v = sh.xfrm.as_ref().is_some_and(|x| x.flip_v);
            let (ax, bx) = if flip_h { (w, 0.0) } else { (0.0, w) };
            let (ay, by) = if flip_v { (h, 0.0) } else { (0.0, h) };
            let mut defs = String::new();
            let mut marks = String::new();
            for (which, attr) in [("headEnd", "marker-start"), ("tailEnd", "marker-end")] {
                if obj.and_then(|o| shape::line_end(o, which)).is_some() {
                    let id = format!("m{}{}", sh.id, which);
                    let _ = write!(
                        defs,
                        "<marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"{}\" refY=\"5\" markerWidth=\"4\" markerHeight=\"4\" orient=\"auto-start-reverse\"><path d=\"M0,0L10,5L0,10Z\" fill=\"{stroke}\"/></marker>",
                        if which == "tailEnd" { 9 } else { 1 }
                    );
                    let _ = write!(marks, " {attr}=\"url(#{id})\"");
                }
            }
            let da = dash
                .map(|d| format!(" stroke-dasharray=\"{d}\""))
                .unwrap_or_default();
            let _ = write!(
                inner,
                "<svg width=\"{w:.1}\" height=\"{h:.1}\" style=\"overflow:visible\"><defs>{defs}</defs><line x1=\"{ax:.1}\" y1=\"{ay:.1}\" x2=\"{bx:.1}\" y2=\"{by:.1}\" stroke=\"{stroke}\" stroke-width=\"{px:.2}\"{da}{marks}/></svg>"
            );
            css.push_str("overflow:visible;");
        }
        _ => {
            let mut defs = String::new();
            let fill = shape_fill(sh, obj, theme, &mut defs, &format!("g{}", sh.id));
            let line = shape_line(sh, obj, theme);
            // 모양 경로: 원본 XML 이 있으면 그것으로, 없으면 모델의 모양 이름
            let paths = obj.and_then(|o| shape::paths(o, w, h)).or_else(|| {
                let prst = sh.geometry.as_deref()?;
                shape::preset_path(prst, w, h, &HashMap::new()).map(|d| vec![(d, true, true)])
            });
            if let Some(paths) = paths.filter(|_| fill.is_some() || line.is_some()) {
                let mut svg = format!(
                    "<svg class=\"rc-geom\" width=\"{w:.1}\" height=\"{h:.1}\" viewBox=\"0 0 {w:.1} {h:.1}\"><defs>{defs}</defs>"
                );
                for (d, f, st) in paths {
                    let fill_attr = if f { fill.clone().unwrap_or_else(|| "none".into()) } else { "none".into() };
                    let stroke_attr = match (&line, st) {
                        (Some((c, px, dash)), true) => format!(
                            " stroke=\"{c}\" stroke-width=\"{px:.2}\"{}",
                            dash.as_ref()
                                .map(|d| format!(" stroke-dasharray=\"{d}\""))
                                .unwrap_or_default()
                        ),
                        _ => String::new(),
                    };
                    let _ = write!(svg, "<path d=\"{d}\" fill=\"{fill_attr}\"{stroke_attr}/>");
                }
                svg.push_str("</svg>");
                inner.push_str(&svg);
                css.push_str("overflow:visible;");
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
    let vert = match t.vert.as_deref() {
        Some("vert" | "eaVert" | "wordArtVertRtl") => "writing-mode:vertical-rl;",
        Some("vert270") => "writing-mode:vertical-rl;transform:rotate(180deg);",
        _ => "",
    };
    let mut s = format!(
        "<div class=\"rc-text\" style=\"justify-content:{justify};padding:{:.1}px {:.1}px {:.1}px {:.1}px;{}{vert}\">",
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
            Some("just") | Some("dist") => "justify",
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
                let _ = write!(st, "font-family:{};", font_family(f));
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
                "dk1" | "tx1" => 1,
                "lt1" | "bg1" => 0,
                "dk2" | "tx2" => 3,
                "lt2" | "bg2" => 2,
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

/// 뷰어 공통 CSS. 칸은 오른쪽·아래 1px 가 격자선 자리다 (한셀 화면과 같다)
pub const CSS: &str = "\
.rc-book{font-family:'맑은 고딕','Malgun Gothic',sans-serif;color:#000}\
.rc-tabs{display:flex;gap:2px;flex-wrap:wrap;margin:8px 0;padding:0;list-style:none}\
.rc-tabs button{border:1px solid #c8c8c8;background:#f3f3f3;padding:4px 12px;cursor:pointer;font:inherit}\
.rc-tabs button[aria-selected=true]{background:#fff;border-bottom-color:#fff;font-weight:bold}\
.rc-wrap{overflow:auto;max-height:80vh;border:1px solid #c8c8c8;background:#fff}\
table.rc-sheet{border-collapse:separate;border-spacing:0;table-layout:fixed;background:#fff;width:max-content}\
table.rc-sheet td{position:relative;box-sizing:border-box;padding:0;border:0 solid transparent;border-right-width:1px;border-bottom-width:1px;font-size:11pt;vertical-align:top;background-clip:border-box}\
table.rc-grid td{border-right-color:#D9D9D9;border-bottom-color:#D9D9D9}\
table.rc-sheet th{box-sizing:border-box;background:#F5F5F5;border:0 solid #CCCCCC;border-right-width:1px;border-bottom-width:1px;padding:0;font-weight:normal;font-size:9pt;color:#333;position:sticky;top:0;z-index:3}\
table.rc-sheet thead th.rc-corner{left:0;z-index:5}\
table.rc-sheet tbody th{position:sticky;left:0;text-align:center}\
.rc-t{position:absolute;inset:0;overflow:hidden;display:flex;flex-direction:column;padding:0 3px;box-sizing:border-box;white-space:pre;line-height:normal}\
.rc-ov{overflow:hidden;padding:0;z-index:1}\
.rc-ov>span{position:absolute;white-space:pre}\
.rc-rot{position:absolute;white-space:pre}\
.rc-bd{position:absolute;left:-2px;top:-2px;pointer-events:none;overflow:visible;z-index:2}\
.rc-spark{position:absolute;left:0;top:0;pointer-events:none}\
.rc-bar{position:absolute;left:1px;top:2px;bottom:2px}\
.rc-hidden{display:none}\
.rc-canvas{position:relative;display:inline-block}\
.rc-icon{display:inline-block;margin-right:3px;font-weight:normal}\
.rc-shape{position:absolute;box-sizing:border-box;overflow:hidden;pointer-events:auto;z-index:3}\
.rc-geom{position:absolute;left:0;top:0;overflow:visible}\
.rc-text{position:relative;display:flex;flex-direction:column;box-sizing:border-box;width:100%;height:100%;font-size:11pt;line-height:1.2}\
.rc-text p{margin:0}\
.rc-chart{display:flex;align-items:center;justify-content:center;width:100%;height:100%;color:#888;font-size:10pt}\
table.rc-sheet td.rc-note::after{content:'';position:absolute;top:0;right:0;border-style:solid;border-width:0 6px 6px 0;border-color:transparent #d00 transparent transparent;z-index:2}\
table.rc-sheet td.rc-note:hover::before{content:attr(data-note);position:absolute;left:100%;top:0;z-index:10;white-space:pre-wrap;min-width:12em;max-width:24em;padding:4px 6px;background:#ffffe1;border:1px solid #000;font-size:9pt;font-weight:normal;font-style:normal;color:#000;text-align:left}";

/// 통합 문서 전체를 HTML 조각으로 (시트 탭 포함). 스크립트 없이도 첫 시트가 보인다.
pub fn workbook_fragment(wb: &Workbook, opts: Options) -> String {
    workbook_fragment_with(wb, opts, &Extras::default())
}

/// [`workbook_fragment`] + 원본에서 다시 읽은 그리기 정보 (스파크라인)
pub fn workbook_fragment_with(wb: &Workbook, opts: Options, extras: &Extras) -> String {
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
            sheet_table_with(wb, sh, opts, extras)
        );
    }
    s.push_str("</div>");
    s
}

/// 탭 전환 스크립트.
pub const TABS_JS: &str = "document.addEventListener('click',function(e){var b=e.target.closest&&e.target.closest('.rc-tabs button');if(!b)return;var book=b.closest('.rc-book');book.querySelectorAll('.rc-tabs button').forEach(function(x){x.setAttribute('aria-selected',x===b)});book.querySelectorAll('.rc-wrap').forEach(function(w){w.classList.toggle('rc-hidden',w.dataset.sheet!==b.dataset.sheet)})});";

/// 독립 실행 HTML 문서.
pub fn workbook_html(wb: &Workbook, title: &str, opts: Options) -> String {
    workbook_html_with(wb, title, opts, &Extras::default())
}

/// [`workbook_html`] + 원본에서 다시 읽은 그리기 정보 (스파크라인)
pub fn workbook_html_with(wb: &Workbook, title: &str, opts: Options, extras: &Extras) -> String {
    format!(
        "<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><meta name=\"generator\" content=\"rcell (yoomin53)\"><style>body{{margin:16px;background:#fafafa}}{CSS}</style></head><body>{}<script>{TABS_JS}</script></body></html>",
        esc(title),
        workbook_fragment_with(wb, opts, extras)
    )
}
