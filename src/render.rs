// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 시트를 HTML 로 그린다 (명령줄 `rcell html` 과 웹 뷰어가 함께 쓴다).
//!
//! 한셀 화면과 같게 그리는 것이 목표다 (한셀 COM·화면 갈무리 대조 실험, docs/format/screen.md):
//! - 열 너비는 글꼴 0 의 숫자 폭 K 로 w·K px 를 열마다 반올림, 행 높이는 1/20 pt 로 잘라 행마다
//!   반올림한 px. 직접 정하지 않은 행 높이는 그 행 글꼴들로 맞춘다 ([`layout`]).
//! - 격자선(#D9D9D9)은 칸의 오른쪽·아래 끝 픽셀. 채운 칸은 격자선 자리까지 덮는다.
//! - 테두리·대각선은 칸마다 SVG 로 경계 픽셀에 그린다 ([`paint`]). 무늬 18종·그라데이션.
//! - 글자는 빈 이웃 칸으로 넘친다 (왼쪽·오른쪽·가운데, 값이 있는 칸에서 멈춤, 넘친 자리의 세로선은
//!   지운다). 숫자는 칸에 맞게 줄이거나 `#` 로 채운다 ([`numfit`], [`cell`]).
//! - 도형은 미리 정의된 모양과 사용자 정의 경로를 SVG 로 ([`shape`], [`draw`] — 그림자·화살표·겹선),
//!   스파크라인은 칸 안 SVG ([`sparkline`]), 차트는 [`crate::chart_svg`].
//! - 조건부 서식: 데이터 막대는 한셀처럼 삼각형([`cell`]), 아이콘은 16px SVG ([`icons`]). 보이는 메모는 상자와
//!   화살표 ([`note`]).

mod cell;
mod draw;
mod icons;
mod layout;
mod metrics;
mod note;
mod numfit;
mod paint;
mod shape;
mod sparkline;
mod table;

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

pub use layout::{book_metrics, default_col_px, default_col_width};
pub use metrics::{FontMetrics, col_px, default_col_chars, is_wide, row_px};
pub use numfit::{general, general_fit};
pub use sparkline::{Extras, SparklineGroup};

/// DrawingML 미리 정의된 점선 → SVG stroke-dasharray (선 굵기 px 배수). 차트도 쓴다
pub fn dash_pattern(d: &str, px: f64) -> Option<String> {
    draw::dash_array(d, px)
}

use crate::model::{Alignment, CellRef, CellValue, RichText, Sheet, Workbook, Xf};
use crate::theme::{self, Theme};
use crate::writer::esc;
use cell::CellView;

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
    /// 빈 칸으로라도 채울 최소 크기 (px, 너비·높이). 한셀 창처럼 쓰는 범위 밖도 격자를 보인다. (0, 0) 이면
    /// 쓰는 범위(값·서식·병합·개체, 넘치는 글자)까지만
    pub fill: (u32, u32),
    /// 적어도 그릴 행·열 수 (웹 편집기: 선택한 칸까지)
    pub min_cells: (u32, u32),
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_rows: 2000,
            max_cols: 200,
            headers: true,
            active_sheet: None,
            fill: (1280, 640),
            min_cells: (0, 0),
        }
    }
}

/// 들여쓰기 한 단계 (px): 한셀 1134 HWPUNIT = 4mm (hs:parashape leftMargin)
pub(crate) const INDENT_PX: f64 = 1134.0 / 7200.0 * 96.0;

/// 들여쓰기 단계: `indent` 가 없으면 문단 모양 여백 (한셀은 균등 분할의 들여쓰기를 `hs:parashape` 의
/// leftMargin·rightMargin 에만 적는다 — 들여쓰기 실험)
pub(crate) fn indent_level(xf: Option<&Xf>) -> u32 {
    let a = xf.and_then(|x| x.alignment.as_ref());
    if let Some(i) = a.and_then(|a| a.indent) {
        return i;
    }
    if a.and_then(|a| a.horizontal.as_deref()) != Some("distributed") {
        return 0;
    }
    xf.and_then(|x| x.hancom.as_ref())
        .and_then(|h| h.parashape.as_ref())
        .and_then(|p| p.left_margin)
        .filter(|&m| m > 0)
        .map_or(0, |m| (m as f64 / 1134.0).round() as u32)
}

/// 칸에 들어가는 숫자의 폭 (px): 칸 너비 − 4 (좁은 칸 숫자 대조 실험)
const NUM_SLACK: f64 = 4.0;

// ---------------------------------------------------------------------------
// 배치 (테스트·대조 도구용 공개 API)
// ---------------------------------------------------------------------------

/// 화면 배치 (시트 원점 = A1 왼쪽 위). 숨긴 열·행은 너비·높이 0
#[derive(Debug, Clone, Default)]
pub struct Geometry {
    /// 열(0부터) → (왼쪽, 너비) 화면 px
    pub cols: Vec<(u32, u32)>,
    /// 행(0부터) → (위, 높이) 화면 px
    pub rows: Vec<(u32, u32)>,
    /// 열 너비 (한셀 안의 값, px 소수) — COM `Range.Width`(pt) × 4/3 과 같다
    pub col_px: Vec<f64>,
    /// 행 높이 (pt) — COM `Range.Height` 와 같다
    pub row_pt: Vec<f64>,
}

/// 시트의 화면 배치 (행 `max_rows`, 열 `max_cols` 개). 한셀 화면과 같은 규칙으로 계산한다
pub fn sheet_geometry(wb: &Workbook, sheet: &Sheet, max_rows: u32, max_cols: u32) -> Geometry {
    let lay = layout::layout(wb, sheet, max_rows, max_cols);
    Geometry {
        cols: (0..max_cols)
            .map(|c| (lay.col_x(c), lay.col_w(c)))
            .collect(),
        rows: (0..max_rows)
            .map(|r| (lay.row_y(r), lay.row_h(r)))
            .collect(),
        col_px: lay.col_exact.clone(),
        row_pt: lay.row_pt.clone(),
    }
}

// ---------------------------------------------------------------------------
// 칸 서식
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

fn h_align(a: Option<&Alignment>, v: &CellValue) -> HAlign {
    match a.and_then(|a| a.horizontal.as_deref()).unwrap_or("general") {
        "left" => HAlign::Left,
        "center" => HAlign::Center,
        "right" => HAlign::Right,
        "justify" => HAlign::Justify,
        "distributed" => HAlign::Distributed,
        "fill" => HAlign::Fill,
        "centerContinuous" => HAlign::CenterContinuous,
        // 일반 + 돌린 글자 (한셀 화면, 회전 실험): 반시계 1–89 는 왼쪽, 90 은 오른쪽, 시계(91–179)는 오른쪽,
        // 시계 90(180)은 왼쪽, 세로 쓰기는 가운데 — 글·숫자 모두
        _ if a.and_then(|a| a.text_rotation).is_some_and(|r| r != 0) => {
            match a.and_then(|a| a.text_rotation).unwrap_or(0) {
                1..=89 | 180 => HAlign::Left,
                255 => HAlign::Center,
                _ => HAlign::Right,
            }
        }
        // 일반: 숫자는 오른쪽, 논리값·오류는 가운데, 글은 왼쪽
        _ => match v {
            CellValue::Number(_) | CellValue::Date(_) => HAlign::Right,
            CellValue::Bool(_) | CellValue::Error(_) => HAlign::Center,
            _ => HAlign::Left,
        },
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
        down: if b.diagonal_down {
            e(b.diagonal.as_ref())
        } else {
            None
        },
        up: if b.diagonal_up {
            e(b.diagonal.as_ref())
        } else {
            None
        },
    }
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

/// 넘친 글자: 칸 → (왼쪽 이웃 폭, 오른쪽 이웃 폭) px, 지울 세로 경계(칸의 오른쪽), 가장 오른쪽 열
struct Overflow {
    span: HashMap<(u32, u32), (u32, u32)>,
    no_vline: HashSet<(u32, u32)>,
    max_col: u32,
}

/// 넘치는 글자의 범위를 정한다 (값이 있는 칸·병합·스파크라인에서 멈춘다)
fn overflow_spans(
    lay: &layout::Layout,
    views: &HashMap<(u32, u32), CellView>,
    blocked: &dyn Fn(u32, u32) -> bool,
    ncols: u32,
) -> Overflow {
    let mut out = Overflow {
        span: HashMap::new(),
        no_vline: HashSet::new(),
        max_col: 0,
    };
    let next = |c: u32| ((c + 1)..ncols).find(|&k| lay.col_w(k) > 0);
    let prev = |c: u32| (0..c).rev().find(|&k| lay.col_w(k) > 0);
    for (&(r, c), v) in views {
        if !v.may_overflow() || lay.span.contains_key(&(r, c)) {
            continue;
        }
        // 선택 영역 가운데: 오른쪽으로 같은 정렬의 빈 칸까지를 한 칸처럼
        if v.h == HAlign::CenterContinuous {
            let mut right = 0;
            let mut k = c;
            while let Some(n) = next(k) {
                let same = views
                    .get(&(r, n))
                    .is_some_and(|x| x.h == HAlign::CenterContinuous && !x.has_value());
                if !same || blocked(r, n) {
                    break;
                }
                right += lay.col_w(n);
                out.no_vline.insert((r, k));
                k = n;
            }
            if right > 0 {
                out.span.insert((r, c), (0, right));
                out.max_col = out.max_col.max(k);
            }
            // 이어진 범위보다 넓으면 그 바깥으로도 넘친다 (아래에서 다시 보지 않는다)
            continue;
        }
        let (cw, _) = lay.cell_size(r, c);
        let indent = v.indent as f64 * INDENT_PX;
        let need = v.text_px() + indent + layout::PAD + layout::PAD_R + 1.0;
        if need <= cw as f64 {
            continue;
        }
        let (go_left, go_right) = match v.h {
            HAlign::Left => (false, true),
            HAlign::Right => (true, false),
            _ => (true, true),
        };
        let extra = need - cw as f64;
        // 가운데는 양쪽으로 반씩
        let want = if go_left && go_right {
            extra / 2.0
        } else {
            extra
        };
        let mut walk = |forward: bool| -> u32 {
            let mut got = 0u32;
            let mut k = c;
            loop {
                let n = if forward { next(k) } else { prev(k) };
                let Some(n) = n else { break };
                if blocked(r, n) || got as f64 >= want {
                    break;
                }
                out.no_vline.insert(if forward { (r, k) } else { (r, n) });
                got += lay.col_w(n);
                k = n;
            }
            if forward {
                out.max_col = out.max_col.max(k);
            }
            got
        };
        let left = if go_left { walk(false) } else { 0 };
        let right = if go_right { walk(true) } else { 0 };
        if left > 0 || right > 0 {
            out.span.insert((r, c), (left, right));
        }
    }
    out
}

/// 시트 하나를 `<table>` 로 (원본에서 다시 읽은 스파크라인 포함)
pub fn sheet_table_with(wb: &Workbook, sheet: &Sheet, opts: Options, extras: &Extras) -> String {
    sheet_table_grid(wb, sheet, opts, extras).0
}

/// 그린 표의 칸 배치 (웹 편집기가 마우스 자리 → 칸, 선택 상자 자리를 정하는 데 쓴다). 좌표는 표 왼쪽 위
/// (머리글 모서리) 기준 px, 확대율 적용 전
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GridInfo {
    pub header_w: u32,
    pub header_h: u32,
    /// 그린 열: (열 번호, 왼쪽, 너비) — 숨긴 열은 없다
    pub cols: Vec<(u32, u32, u32)>,
    /// 그린 행: (행 번호, 위, 높이)
    pub rows: Vec<(u32, u32, u32)>,
    /// 확대율 (%)
    pub zoom: u32,
    /// 틀 고정 칸 (이 칸의 위·왼쪽이 고정된다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen: Option<(u32, u32)>,
}

/// [`sheet_table_with`] + 칸 배치
pub fn sheet_table_grid(
    wb: &Workbook,
    sheet: &Sheet,
    opts: Options,
    extras: &Extras,
) -> (String, GridInfo) {
    let theme = wb.theme.clone().unwrap_or_default();
    let st = &wb.styles;
    // 배치: 쓰는 범위 + 넘침·채우기 여유
    let (used_r, used_c) = layout::used_extent(sheet);
    let (used_r, used_c) = (used_r.max(opts.min_cells.0), used_c.max(opts.min_cells.1));
    let slack_c = 64.max(opts.fill.0 / 8);
    let slack_r = 64.max(opts.fill.1 / 12);
    let lay_r = (used_r + slack_r).min(opts.max_rows);
    let lay_c = (used_c + slack_c).min(opts.max_cols);
    let lay = layout::layout(wb, sheet, lay_r, lay_c);
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
    // 표 스타일 (칸 → 덧입힐 서식, 칸 자체의 서식과 조건부 서식이 이긴다)
    let tlooks = table::sheet_looks(sheet, st, &theme);
    // 메모: 셀 오른쪽 위 빨간 삼각형. 숨긴 메모는 마우스를 올리면 작성자·내용, 보이는 메모는 상자 (note.rs)
    let notes: HashMap<(u32, u32), (String, bool)> = sheet
        .comments
        .iter()
        .map(|c| {
            let who = if c.author.is_empty() {
                String::new()
            } else {
                format!("{}:\n", c.author)
            };
            (
                (c.cell.row, c.cell.col),
                (format!("{who}{}", c.text.text), c.visible),
            )
        })
        .collect();
    // 서식 있는 글: 공유 문자열 중 조각이 둘 이상인 것
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
    let spark_groups = extras
        .sparklines
        .get(&sheet.part)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let sparks = sparkline::sheet_items(wb, sheet_index, spark_groups);

    // ---- 1. 칸마다 그릴 내용: 셀 레코드가 있는 칸 (행·열 서식만 있는 칸은 그릴 범위를 정한 뒤에)
    let mut views: HashMap<(u32, u32), CellView> = HashMap::new();
    let mut want: Vec<(u32, u32)> = sheet
        .cells
        .iter()
        .filter(|c| c.r.row < lay_r && c.r.col < lay_c)
        .map(|c| (c.r.row, c.r.col))
        .collect();
    want.extend(cfx.keys().copied());
    want.extend(
        tlooks
            .keys()
            .copied()
            .filter(|&(r, c)| r < lay_r && c < lay_c),
    );
    want.sort_unstable();
    want.dedup();
    let make_view = |r: u32, c: u32| -> Option<CellView> {
        if lay.covered.contains(&(r, c)) || lay.row_h(r) == 0 || lay.col_w(c) == 0 {
            return None;
        }
        let cell = cells.get(&(r, c));
        let style = cell
            .map(|x| x.style)
            .unwrap_or_else(|| sheet.effective_style(r, c));
        let cf = cfx.get(&(r, c));
        let tl = tlooks.get(&(r, c));
        if cell.is_none() && style == 0 && cf.is_none() && tl.is_none() {
            return None;
        }
        let formula = cell
            .and_then(|x| x.formula.as_ref())
            .filter(|f| !f.text.is_empty());
        // 수식 표시(showFormulas)면 값 대신 수식, 0 값 숨김(showZeros=0)이면 0 을 비운다
        let value = match (cell.map(|x| &x.value), formula) {
            (_, Some(f)) if view.show_formulas => CellValue::Text(format!("={}", f.text)),
            (Some(CellValue::Number(n)), _) if *n == 0.0 && !view.show_zeros => CellValue::Empty,
            (Some(v), _) => v.clone(),
            (None, _) => CellValue::Empty,
        };
        let xf = st.xf(style);
        let align = xf.and_then(|x| x.alignment.as_ref());
        let font = xf.and_then(|x| st.fonts.get(x.font_id as usize));
        let m = layout::font_metrics(wb, font);
        let h = h_align(align, &value);
        let (w_px, _) = lay.cell_size(r, c);
        let origin = (lay.col_x(c), lay.row_y(r));
        let fill = xf
            .and_then(|x| st.fills.get(x.fill_id as usize))
            .and_then(|f| paint::fill_css(f, &theme, origin));
        let code = xf
            .map(|x| crate::numfmt::code_for(st, x.num_fmt_id))
            .unwrap_or_else(|| "General".into());
        let shown = crate::numfmt::format_layout(&value, &code, wb.date1904);
        let (shown_text, shown_pads, shown_fill) =
            (shown.text.clone(), shown.pads.clone(), shown.fill);
        // 글 구역(@)으로 보인 숫자는 일반 맞춤에서 글처럼 왼쪽
        let h = if shown.as_text
            && h == HAlign::Right
            && align
                .and_then(|a| a.horizontal.as_deref())
                .is_none_or(|x| x == "general")
            && align.and_then(|a| a.text_rotation).unwrap_or(0) == 0
        {
            HAlign::Left
        } else {
            h
        };
        // 양쪽 맞춤·균등 분할은 줄을 바꾼다
        let wrap = align.is_some_and(|a| a.wrap_text)
            || ((matches!(h, HAlign::Justify | HAlign::Distributed)
                || align.is_some_and(|a| {
                    matches!(a.vertical.as_deref(), Some("justify" | "distributed"))
                }))
                && matches!(value, CellValue::Text(_)));
        let shrink = align.is_some_and(|a| a.shrink_to_fit);
        let indent = indent_level(xf);
        // 숫자: 일반 서식은 칸에 맞춰 줄이고, 다른 서식은 넘치면 # 로 채운다
        let avail = w_px as f64 - NUM_SLACK - indent as f64 * INDENT_PX;
        let text = if shown.overflow {
            numfit::hashes(avail, &m)
        } else if let CellValue::Number(n) = value {
            let general = code.eq_ignore_ascii_case("general");
            if shrink || wrap {
                if general {
                    numfit::general(n)
                } else {
                    shown.text
                }
            } else if general {
                numfit::general_fit(n, avail, &m)
            } else if m.text_width(&shown.text) > avail {
                numfit::hashes(avail, &m)
            } else {
                shown.text
            }
        } else {
            shown.text
        };
        // 서식의 빈 자리·채움은 글을 그대로 쓸 때만
        let (pads, fill_char) = if text == shown_text {
            (shown_pads, shown_fill)
        } else {
            (Vec::new(), None)
        };
        let hashed = !text.is_empty()
            && text.bytes().all(|b| b == b'#')
            && !matches!(value, CellValue::Text(_));
        let rich = match &value {
            CellValue::Text(t) => rich.get(t.as_str()).copied(),
            _ => None,
        };
        // 조건부 서식 채우기는 칸 채우기를 대신한다
        let fill = cf
            .and_then(|cf| {
                cf.scale
                    .clone()
                    .or_else(|| cf.fill.as_ref().and_then(|c| theme::resolve(c, &theme)))
            })
            .map(|c| paint::FillPaint {
                css: format!("background:{c};"),
                color: Some(c),
                tiled: false,
            })
            .or(fill)
            .or_else(|| {
                tl.and_then(|t| t.fill.clone()).map(|c| paint::FillPaint {
                    css: format!("background:{c};"),
                    color: Some(c),
                    tiled: false,
                })
            });
        // 표 스타일 테두리는 칸 자체의 테두리가 없는 변에만
        let mut borders = borders_of(xf, wb, &theme);
        if let Some(t) = tl {
            let fill_in = |slot: &mut Option<paint::Edge>, v: &Option<paint::Edge>| {
                if slot.is_none() {
                    slot.clone_from(v);
                }
            };
            fill_in(&mut borders.left, &t.borders.left);
            fill_in(&mut borders.right, &t.borders.right);
            fill_in(&mut borders.top, &t.borders.top);
            fill_in(&mut borders.bottom, &t.borders.bottom);
        }
        Some(CellView {
            value,
            text,
            rich,
            font,
            m,
            code_color: shown.color.as_deref().map(cell::code_color),
            // 넘쳐 # 로 채운 칸은 왼쪽부터 (한셀 화면 — dbnum 실험의 오른쪽 맞춤 숫자도 # 는 왼쪽 2px 에서 시작)
            h: if hashed { HAlign::Left } else { h },
            v: v_align(align),
            wrap,
            shrink,
            rotation: align.and_then(|a| a.text_rotation).unwrap_or(0),
            pads,
            fill_char,
            indent,
            fill,
            borders,
        })
    };
    for (r, c) in want {
        if let Some(v) = make_view(r, c) {
            views.insert((r, c), v);
        }
    }

    // ---- 2. 넘치는 글자
    let blocked = |r: u32, c: u32| -> bool {
        lay.covered.contains(&(r, c))
            || lay.span.contains_key(&(r, c))
            || views.get(&(r, c)).is_some_and(|v| v.has_value())
            || cells.get(&(r, c)).is_some_and(|x| x.formula.is_some())
            || sparks.contains_key(&(r, c))
    };
    let ov = overflow_spans(&lay, &views, &blocked, lay_c);

    // ---- 3. 그릴 범위: 쓰는 범위 + 넘친 글자 + 최소 크기
    let mut ncols = used_c.max(ov.max_col + 1).min(lay_c);
    while ncols < lay_c && lay.col_x(ncols) < opts.fill.0 {
        ncols += 1;
    }
    let mut nrows = used_r.min(lay_r);
    while nrows < lay_r && lay.row_y(nrows) < opts.fill.1 {
        nrows += 1;
    }
    // 행·열 서식만 있는 칸 (채우기·테두리) — 그릴 범위 안에서
    let styled_rows: Vec<u32> = sheet
        .rows
        .iter()
        .filter(|r| r.style.is_some_and(|s| s != 0) && r.index >= 1 && r.index - 1 < nrows)
        .map(|r| r.index - 1)
        .collect();
    let styled_cols: Vec<u32> = sheet
        .cols
        .iter()
        .filter(|c| c.style.is_some_and(|s| s != 0))
        .flat_map(|c| c.min.max(1) - 1..c.max.min(ncols))
        .collect();
    for &r in &styled_rows {
        for c in 0..ncols {
            if !views.contains_key(&(r, c))
                && let Some(v) = make_view(r, c)
            {
                views.insert((r, c), v);
            }
        }
    }
    for &c in &styled_cols {
        for r in 0..nrows {
            if !views.contains_key(&(r, c))
                && let Some(v) = make_view(r, c)
            {
                views.insert((r, c), v);
            }
        }
    }
    let cols = lay.cols(ncols);
    let rows = lay.rows(nrows);
    let next_col: HashMap<u32, u32> = cols.windows(2).map(|w| (w[0], w[1])).collect();
    let prev_col: HashMap<u32, u32> = cols.windows(2).map(|w| (w[1], w[0])).collect();
    let next_row: HashMap<u32, u32> = rows.windows(2).map(|w| (w[0], w[1])).collect();

    // ---- 4. 격자선 자리: 칸의 오른쪽·아래 경계 픽셀 색
    // 채운 이웃이 있으면 그 색이 덮고, 넘친 글자가 지나가면 지운다
    let edge_color = |r: u32, c: u32, right: bool| -> Option<String> {
        let me = views.get(&(r, c));
        if right && ov.no_vline.contains(&(r, c)) {
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
        // 무늬·그라데이션 칸은 자기 격자선 자리까지 무늬를 잇는다 (테두리를 비워 배경이 보이게)
        if me.and_then(|v| v.fill.as_ref()).is_some_and(|f| f.tiled) {
            return Some("transparent".into());
        }
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
        let digits = rows.last().map_or(1, |r| (r + 1).to_string().len()) as u32;
        HEADER_COL_PX.max(digits * 7 + 11)
    } else {
        0
    };
    let mut sticky_top: HashMap<u32, u32> = HashMap::new();
    let mut sticky_left: HashMap<u32, u32> = HashMap::new();
    if let Some(f) = freeze {
        let mut y = header_h;
        for &r in rows.iter().filter(|&&r| r < f.row) {
            sticky_top.insert(r, y);
            y += lay.row_h(r);
        }
        let mut x = header_w;
        for &c in cols.iter().filter(|&&c| c < f.col) {
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

    let zoom_pct = view
        .zoom_scale
        .filter(|&z| (10..=400).contains(&z))
        .unwrap_or(100);
    let mut info = GridInfo {
        header_w,
        header_h,
        zoom: zoom_pct,
        frozen: freeze.map(|f| (f.row, f.col)),
        ..Default::default()
    };
    let mut x = header_w;
    for &c in &cols {
        info.cols.push((c, x, lay.col_w(c)));
        x += lay.col_w(c);
    }
    let mut y = header_h;
    for &r in &rows {
        info.rows.push((r, y, lay.row_h(r)));
        y += lay.row_h(r);
    }
    let mut s = String::new();
    let zoom = view
        .zoom_scale
        .filter(|&z| z != 100 && (10..=400).contains(&z))
        .map(|z| format!("zoom:{z}%"))
        .unwrap_or_default();
    // 확대율 (한셀 화면 — 테두리 확대 실험의 50·75·125·150·200%): 열·행마다 100% 의 px 에 확대율을 곱해 반올림한 폭
    // (54px → 75% 41, 125% 68; 행 22px → 75% 17), 격자선은 늘 1px. CSS zoom 아래에서 그렇게 보이도록 폭·높이는
    // 반올림한 px ÷ 확대율, 격자선 굵기는 1 ÷ 확대율로 쓴다
    let zf = zoom_pct as f64 / 100.0;
    let zpx = |v: u32| -> String {
        if zoom_pct == 100 {
            format!("{v}")
        } else {
            format!("{:.4}", (v as f64 * zf).round() / zf)
        }
    };
    // 표와 그리기 개체를 같은 좌표계(왼쪽 위 = 머리글 모서리)에 둔다
    let _ = write!(s, "<div class=\"rc-canvas\" style=\"{zoom}\">");
    if zoom_pct != 100 {
        let _ = write!(
            s,
            "<style>table.rc-sheet.rc-z{zoom_pct} td{{border-right-width:{w:.4}px;border-bottom-width:{w:.4}px}}</style>",
            w = 1.0 / zf
        );
    }
    let _ = write!(
        s,
        "<table class=\"rc-sheet{}{}\">",
        if grid { " rc-grid" } else { "" },
        if zoom_pct != 100 {
            format!(" rc-z{zoom_pct}")
        } else {
            String::new()
        }
    );
    s.push_str("<colgroup>");
    if headers {
        let _ = write!(s, "<col style=\"width:{header_w}px\">");
    }
    for &c in &cols {
        let _ = write!(
            s,
            "<col data-c=\"{c}\" style=\"width:{}px\">",
            zpx(lay.col_w(c))
        );
    }
    s.push_str("</colgroup>");
    if headers {
        let _ = write!(
            s,
            "<thead><tr style=\"height:{header_h}px\"><th class=\"rc-corner\"></th>"
        );
        for c in &cols {
            let st = sticky_left
                .get(c)
                .map(|l| format!(" style=\"left:{l}px;z-index:4\""))
                .unwrap_or_default();
            let _ = write!(s, "<th{st}>{}</th>", CellRef::col_name(*c));
        }
        s.push_str("</tr></thead>");
    }
    s.push_str("<tbody>");
    for &r in &rows {
        let _ = write!(s, "<tr data-r=\"{r}\" style=\"height:{}px\">", lay.row_h(r));
        if headers {
            let st = sticky_top
                .get(&r)
                .map(|t| format!(" style=\"top:{t}px;z-index:4\""))
                .unwrap_or_default();
            let _ = write!(s, "<th{st}>{}</th>", r + 1);
        }
        for &c in &cols {
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
            match notes.get(&(r, c)) {
                Some((_, true)) => attrs.push_str(" class=\"rc-note rc-note-v\""),
                Some((n, false)) => {
                    let _ = write!(attrs, " class=\"rc-note\" data-note=\"{}\"", esc(n));
                }
                None => {}
            }
            let (w, h) = lay.cell_size(r, c);
            let mut css = sticky(r, Some(c));
            // 병합 칸의 오른쪽·아래 경계는 병합 끝 칸 기준 (보이는 마지막 행·열)
            let (er, ec) = lay.merge_end.get(&(r, c)).copied().unwrap_or((r, c));
            let ec = (c..=ec).rev().find(|&k| lay.col_w(k) > 0).unwrap_or(c);
            let er = (r..=er).rev().find(|&k| lay.row_h(k) > 0).unwrap_or(r);
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
                // 표 스타일의 글꼴 색·굵게 (조건부 서식이 이긴다)
                let merged = tlooks
                    .get(&(r, c))
                    .filter(|t| t.font_color.is_some() || t.bold)
                    .map(|t| {
                        let mut m = cf.cloned().unwrap_or_default();
                        if m.font_color.is_none() {
                            m.font_color = t.font_color.as_ref().map(|c| crate::model::Color {
                                rgb: Some(format!("FF{}", c.trim_start_matches('#'))),
                                ..Default::default()
                            });
                        }
                        if t.bold && m.bold.is_none() {
                            m.bold = Some(true);
                        }
                        m
                    });
                let span = ov.span.get(&(r, c)).copied().unwrap_or((0, 0));
                inner = cell::cell_content(v, merged.as_ref().or(cf), &theme, w, h, span);
            }
            if tlooks.get(&(r, c)).is_some_and(|t| t.handle) {
                inner.push_str(&table::handle_svg(w, h));
            }
            if tlooks.get(&(r, c)).is_some_and(|t| t.button) {
                inner.push_str(&table::button_svg(w, h));
            }
            // 테두리: 병합 영역은 가장자리 칸들의 테두리 (위·왼쪽은 왼쪽 위 칸, 오른쪽은 오른쪽 위 칸,
            // 아래는 왼쪽 아래 칸). 넘친 글자가 지나가는 세로 경계는 지운다 (한셀 화면: 위·아래 선은 남는다)
            let style_at = |rr: u32, cc: u32| {
                cells
                    .get(&(rr, cc))
                    .map(|x| x.style)
                    .unwrap_or_else(|| sheet.effective_style(rr, cc))
            };
            let mut b = match lay.merge_end.get(&(r, c)) {
                Some(&(r1, c1)) => {
                    let at = |rr, cc| borders_of(st.xf(style_at(rr, cc)), wb, &theme);
                    let (tl, tr, bl) = (at(r, c), at(r, c1), at(r1, c));
                    paint::CellBorders {
                        left: tl.left,
                        top: tl.top,
                        right: tr.right,
                        bottom: bl.bottom,
                        down: tl.down,
                        up: tl.up,
                    }
                }
                None => views
                    .get(&(r, c))
                    .map(|v| v.borders.clone())
                    .unwrap_or_default(),
            };
            if ov.no_vline.contains(&(r, ec)) {
                b.right = None;
            }
            if prev_col
                .get(&c)
                .is_some_and(|&p| ov.no_vline.contains(&(r, p)))
            {
                b.left = None;
            }
            if !b.is_empty() {
                let gap = views
                    .get(&(r, c))
                    .and_then(|v| v.fill.as_ref())
                    .and_then(|f| f.color.clone())
                    .unwrap_or_else(|| "#FFFFFF".into());
                inner.push_str(&paint::borders_svg(&b, w, h, &gap));
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
    s.push_str(&draw::shapes_html(
        wb, sheet, &lay, header_w, header_h, &theme,
    ));
    s.push_str(&note::notes_html(sheet, &lay, header_w, header_h, &theme));
    s.push_str("</div>");
    (s, info)
}

/// 뷰어 공통 CSS. 칸은 오른쪽·아래 1px 가 격자선 자리다 (한셀 화면과 같다)
pub const CSS: &str = "\
.rc-book{font-family:'맑은 고딕','Malgun Gothic',sans-serif;color:#000}\
.rc-tabs{display:flex;gap:2px;flex-wrap:wrap;margin:8px 0;padding:0;list-style:none}\
.rc-tabs button{border:1px solid #c8c8c8;background:#f3f3f3;padding:4px 12px;cursor:pointer;font:inherit}\
.rc-tabs button[aria-selected=true]{background:#fff;border-bottom-color:#fff;font-weight:bold}\
.rc-wrap{overflow:auto;max-height:80vh;border:1px solid #c8c8c8;background:#fff}\
table.rc-sheet{border-collapse:separate;border-spacing:0;table-layout:fixed;background:#fff;width:max-content}\
table.rc-sheet td{position:relative;box-sizing:border-box;padding:0;border:0 solid transparent;border-right-width:1px;border-bottom-width:1px;font-size:11pt;vertical-align:top;background-clip:border-box;overflow:visible}\
table.rc-grid td{border-right-color:#D9D9D9;border-bottom-color:#D9D9D9}\
table.rc-sheet th{box-sizing:border-box;background:#F5F5F5;border:0 solid #CCCCCC;border-right-width:1px;border-bottom-width:1px;padding:0;font-weight:normal;font-size:9pt;color:#333;position:sticky;top:0;z-index:3;overflow:hidden;white-space:nowrap}\
table.rc-sheet thead th.rc-corner{left:0;z-index:5}\
table.rc-sheet tbody th{position:sticky;left:0;text-align:center}\
.rc-t{position:absolute;left:0;top:0;overflow:hidden;box-sizing:border-box;white-space:pre;z-index:1}\
.rc-t>span{position:absolute;white-space:pre;text-decoration:inherit}\
.rc-w,.rc-flex{display:flex;flex-direction:column;padding:0 2px}\
.rc-w>span,.rc-flex>span{position:static}\
.rc-ov{z-index:2}\
.rc-rot{position:absolute;white-space:pre}\
.rc-vert{position:absolute;display:flex;flex-direction:column;align-items:center;white-space:pre}\
.rc-vert>span{display:block}\
.rc-bd{position:absolute;left:-2px;top:-2px;pointer-events:none;overflow:visible;z-index:3}\
.rc-spark{position:absolute;left:0;top:0;pointer-events:none}\
.rc-bar{position:absolute;left:0;top:0;overflow:visible;pointer-events:none}\
.rc-hidden{display:none}\
.rc-canvas{position:relative;display:inline-block}\
.rc-icon{position:absolute;left:3px;z-index:1;overflow:visible}\
.rc-note-box,.rc-note-line{position:absolute;overflow:visible;z-index:6;pointer-events:none}\
.rc-note-text{position:absolute;z-index:7;display:flex;flex-direction:column;overflow:hidden;white-space:pre-wrap;overflow-wrap:anywhere;font:9pt '돋움',Dotum,sans-serif;line-height:15px;color:#000;pointer-events:none}\
table.rc-sheet td.rc-note-v:hover::before{content:none}\
.rc-shape{position:absolute;box-sizing:border-box;overflow:hidden;pointer-events:auto;z-index:4}\
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
