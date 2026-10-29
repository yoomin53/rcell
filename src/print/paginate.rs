// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 페이지 나누기와 인쇄 배율 (한셀 COM `HPageBreaks`·`VPageBreaks`·`Pages.Count` 와 PDF 대조, docs/format/print.md).
//!
//! 한셀은 두 단계로 나눈다:
//! 1. 페이지 나누기: 행·열의 화면 px 에 배율을 곱해 쌓는다. 배율 지정이면 쓸 수 있는 자리는 1/100 인치 단위
//!    round((용지 − 여백)/0.72pt), 페이지에 맞추기면 화면 px 단위 (용지 − 여백)/0.75pt.
//! 2. 인쇄 배율 z (%): 행·열을 하나씩 round(px × z/100) 화면 px 로 줄였을 때 모든 페이지가 (용지 − 여백)/0.75pt
//!    안에 들어갈 때까지 1 씩 줄인다. 배율 100 은 99 부터 (내용이 아주 작으면 세로만 6.25 배로 늘리는 한셀 동작을 따른다).

use super::paper::{Paper, paper};
use crate::model::{CellRef, PageLayout, Sheet, Workbook};
use crate::render;

/// 인쇄 배율 지정
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scaling {
    /// 확대/축소 % (10–400)
    Zoom(u32),
    /// 페이지에 맞추기: 가로·세로 페이지 수 (0 = 제한 없음)
    Fit { wide: u32, tall: u32 },
}

/// 인쇄 범위 하나 (0부터, 끝 포함)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

/// 시트의 인쇄 설정을 한셀 기본값으로 채운 것 (단위 pt)
#[derive(Debug, Clone)]
pub struct Settings {
    /// 방향을 반영한 용지 (가로 방향이면 너비·높이를 바꾼다)
    pub paper: Paper,
    /// 여백 (pt)
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
    pub scaling: Scaling,
    pub center_h: bool,
    pub center_v: bool,
    /// 왼쪽에서 오른쪽 먼저 (`overThenDown`)
    pub over_then_down: bool,
    pub areas: Vec<Area>,
    /// 반복할 행·열 (0부터, 끝 포함)
    pub title_rows: Option<(u32, u32)>,
    pub title_cols: Option<(u32, u32)>,
    /// 수동 페이지 나누기: 새 페이지의 첫 행·열 (0부터)
    pub row_breaks: Vec<u32>,
    pub col_breaks: Vec<u32>,
    pub first_page_number: Option<u32>,
}

/// 페이지 하나
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    /// 인쇄 범위 번호 (0부터)
    pub area: usize,
    /// 이 페이지에 찍는 보이는 행·열 (0부터, 반복 행·열 제외)
    pub rows: Vec<u32>,
    pub cols: Vec<u32>,
    /// 위에 반복하는 행, 왼쪽에 반복하는 열
    pub title_rows: Vec<u32>,
    pub title_cols: Vec<u32>,
}

/// 시트의 인쇄 배치
#[derive(Debug, Clone)]
pub struct SheetPrint {
    pub settings: Settings,
    /// 인쇄 순서대로
    pub pages: Vec<Page>,
    /// 페이지 나누기에 쓴 배율 (%) — 페이지에 맞추기면 한셀이 계산해 `pageSetup/@scale` 에 저장하는 값
    pub page_scale: u32,
    /// 그릴 때의 배율 (%) — 행·열은 round(px × z/100) 화면 px
    pub print_scale: u32,
    /// 내용이 아주 작아 세로를 6.25 배로 그리는 한셀 동작 (배율 100, 가로는 1:1)
    pub tall: bool,
    /// COM `HPageBreaks`: 새 페이지의 첫 행 (1부터)과 수동 여부
    pub h_breaks: Vec<(u32, bool)>,
    /// COM `VPageBreaks`: 새 페이지의 첫 열 (1부터)과 수동 여부
    pub v_breaks: Vec<(u32, bool)>,
    /// 행·열의 화면 px (0부터, 숨김은 0)
    pub row_px: Vec<u32>,
    pub col_px: Vec<u32>,
}

impl SheetPrint {
    /// 행 하나의 인쇄 높이 (화면 px, 배율 반영). 키 큰 모드는 6.25 배
    pub fn row_print_px(&self, r: u32) -> f64 {
        let px = self.row_px.get(r as usize).copied().unwrap_or(0);
        if self.tall {
            px as f64 * TALL
        } else {
            scaled(px, self.print_scale) as f64
        }
    }
    /// 열 하나의 인쇄 너비 (화면 px, 배율 반영)
    pub fn col_print_px(&self, c: u32) -> f64 {
        let px = self.col_px.get(c as usize).copied().unwrap_or(0);
        if self.tall {
            px as f64
        } else {
            scaled(px, self.print_scale) as f64
        }
    }
}

/// 키 큰 모드의 세로 배수 (600 dpi / 96 dpi)
const TALL: f64 = 6.25;

/// round(px × z / 100), 반은 올림
fn scaled(px: u32, z: u32) -> u32 {
    (px * z + 50) / 100
}

/// 1/100 인치 (pt)
const HUNDREDTH: f64 = 0.72;
/// 화면 1px (pt)
const PX: f64 = 0.75;

/// 인쇄 이름(`_xlnm.Print_Area`·`_xlnm.Print_Titles`)의 식 → 범위들. 외부 문서·`#REF!` 는 버린다 (한셀은 외부 문서를
/// 가리키는 인쇄 영역을 쓰지 않는다 — 서식 파일들의 `'[1]시트'!$B$3:$H$46`)
/// 이름 식의 범위 하나: (행0, 열0, 행1, 열1), 0부터. 행 전체(`$1:$3`)는 열이, 열 전체는 행이 없다
type NameRange = (Option<u32>, Option<u32>, Option<u32>, Option<u32>);

fn name_ranges(formula: &str) -> Vec<NameRange> {
    let mut out = Vec::new();
    for part in split_top(formula) {
        let p = part.trim();
        if p.contains('[') || p.contains("#REF") {
            continue;
        }
        let r = p.rsplit_once('!').map_or(p, |(_, r)| r);
        let (a, b) = r.split_once(':').unwrap_or((r, r));
        let (Some(a), Some(b)) = (ref_parts(a), ref_parts(b)) else {
            continue;
        };
        // (행0, 열0, 행1, 열1): 없으면 전체
        out.push((a.0, a.1, b.0, b.1));
    }
    out
}

/// 쉼표로 나누되 작은따옴표 안의 쉼표는 두기
fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut q = false;
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '\'' => q = !q,
            ',' if !q => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// `$B$3` / `$3` / `$B` → (행, 열) 0부터
fn ref_parts(s: &str) -> Option<(Option<u32>, Option<u32>)> {
    let s = s.trim().replace('$', "");
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        let r: u32 = s.parse().ok()?;
        return (r >= 1).then_some((Some(r - 1), None));
    }
    if s.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut col = 0u32;
        for ch in s.chars() {
            col = col * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
        }
        return (col >= 1).then_some((None, Some(col - 1)));
    }
    let c = CellRef::parse(&s)?;
    Some((Some(c.row), Some(c.col)))
}

/// 빈 칸의 서식이 인쇄 범위에 드는지: 테두리, 채우기, 선택 영역의 가운데로(`centerContinuous` — 서식 파일
/// Retirement_Budget 의 제목 줄 G2:H2 까지 인쇄 범위다)
fn visible_style(wb: &Workbook, style: u32) -> bool {
    let Some(x) = wb.styles.xf(style) else {
        return false;
    };
    if x.alignment.as_ref().and_then(|a| a.horizontal.as_deref()) == Some("centerContinuous") {
        return true;
    }
    let fill = wb.styles.fills.get(x.fill_id as usize).is_some_and(|f| {
        f.gradient.is_some()
            || f.pattern_type
                .as_deref()
                .is_some_and(|p| p != "none" && !p.is_empty())
    });
    let border = wb
        .styles
        .borders
        .get(x.border_id as usize)
        .is_some_and(|b| {
            let e = |e: &Option<crate::model::BorderEdge>| {
                e.as_ref()
                    .and_then(|e| e.style.as_deref())
                    .is_some_and(|s| s != "none")
            };
            e(&b.left)
                || e(&b.right)
                || e(&b.top)
                || e(&b.bottom)
                || ((b.diagonal_up || b.diagonal_down) && e(&b.diagonal))
        });
    fill || border
}

/// 인쇄 범위의 끝 (행, 열 — 0부터): 값이 있거나 테두리·채우기가 있는 칸, 그리기 개체, 넘친 글자. 없으면 None.
/// 한셀의 인쇄 범위는 늘 A1 에서 시작하고, 보이지 않는 서식(글꼴·표시 형식만)의 빈 칸은 넣지 않는다
/// (서식 파일 Basic_Invoice 의 P15 — 쓰는 범위 B2:P49 이지만 인쇄는 A:I)
pub fn used_end(wb: &Workbook, sheet: &Sheet) -> Option<(u32, u32)> {
    let mut end: Option<(u32, u32)> = None;
    let mut add = |r: u32, c: u32| {
        end = Some(match end {
            Some((a, b)) => (a.max(r), b.max(c)),
            None => (r, c),
        });
    };
    for c in &sheet.cells {
        if !c.value.is_empty() || c.formula.is_some() || visible_style(wb, c.style) {
            add(c.r.row, c.r.col);
        }
    }
    for s in sheet.shapes.iter().filter(|s| !s.hidden) {
        let to = s.anchor.to.unwrap_or(s.anchor.from);
        add(to.row, to.col);
    }
    // 넘친 글자가 닿는 열까지
    let (r, c) = end?;
    let c = render::overflow_end_col(wb, sheet, r).map_or(c, |o| o.max(c));
    Some((r, c))
}

/// 시트의 인쇄 설정 (없는 값은 한셀 기본: A4, 여백 왼쪽·오른쪽 19mm·위·아래 25mm·머리글 13mm)
pub fn settings(wb: &Workbook, sheet_index: usize) -> Settings {
    let sheet = &wb.sheets[sheet_index];
    let page: &PageLayout = &sheet.page;
    let setup = page.setup.as_ref();
    let mut p = paper(setup.and_then(|s| s.paper_size));
    if setup.and_then(|s| s.orientation.as_deref()) == Some("landscape") {
        p = Paper {
            width: p.height,
            height: p.width,
        };
    }
    let m = page.margins.as_ref();
    let pt = |v: Option<f64>, d: f64| v.map_or(d, |x| x * 72.0);
    let scaling = if page.fit_to_page == Some(true) {
        Scaling::Fit {
            wide: setup.and_then(|s| s.fit_to_width).unwrap_or(1),
            tall: setup.and_then(|s| s.fit_to_height).unwrap_or(1),
        }
    } else {
        Scaling::Zoom(setup.and_then(|s| s.scale).unwrap_or(100).clamp(10, 400))
    };
    let po = page.print_options.as_ref();

    let mut areas = Vec::new();
    let mut title_rows = None;
    let mut title_cols = None;
    for n in wb
        .defined_names
        .iter()
        .filter(|n| n.local_sheet_id == Some(sheet_index as u32))
    {
        match n.name.as_str() {
            "_xlnm.Print_Area" => {
                for (r0, c0, r1, c1) in name_ranges(&n.formula) {
                    areas.push(Area {
                        r0: r0.unwrap_or(0),
                        c0: c0.unwrap_or(0),
                        r1: r1.unwrap_or(1_048_575),
                        c1: c1.unwrap_or(16_383),
                    });
                }
            }
            "_xlnm.Print_Titles" => {
                for (r0, c0, r1, c1) in name_ranges(&n.formula) {
                    match (r0, c0, r1, c1) {
                        (Some(a), None, Some(b), None) => title_rows = Some((a.min(b), a.max(b))),
                        (None, Some(a), None, Some(b)) => title_cols = Some((a.min(b), a.max(b))),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if areas.is_empty() {
        let (r1, c1) = used_end(wb, sheet).unwrap_or((0, 0));
        areas.push(Area {
            r0: 0,
            c0: 0,
            r1,
            c1,
        });
    }
    Settings {
        paper: p,
        left: pt(m.map(|m| m.left), 53.86),
        right: pt(m.map(|m| m.right), 53.86),
        top: pt(m.map(|m| m.top), 70.87),
        bottom: pt(m.map(|m| m.bottom), 70.87),
        header: pt(m.map(|m| m.header), 36.85),
        footer: pt(m.map(|m| m.footer), 36.85),
        scaling,
        center_h: po.is_some_and(|o| o.horizontal_centered),
        center_v: po.is_some_and(|o| o.vertical_centered),
        over_then_down: setup.and_then(|s| s.page_order.as_deref()) == Some("overThenDown"),
        areas,
        title_rows,
        title_cols,
        row_breaks: page.row_breaks.clone(),
        col_breaks: page.col_breaks.clone(),
        first_page_number: setup
            .filter(|s| s.use_first_page_number)
            .and_then(|s| s.first_page_number),
    }
}

/// 한 방향으로 나눈 띠들 (보이는 행 또는 열 목록)
#[derive(Debug, Clone)]
struct Bands {
    bands: Vec<Vec<u32>>,
    /// 띠마다 반복 제목을 붙이는지
    titled: Vec<bool>,
}

/// 한 방향 나누기. `px`: 화면 px (숨김 0), `f`: 배율, `avail`: 쓸 수 있는 자리(px 단위), `title`: 반복 범위
fn split(
    range: (u32, u32),
    px: &[f64],
    f: f64,
    avail: f64,
    manual: &[u32],
    title: Option<(u32, u32)>,
) -> Bands {
    let title_px: f64 = title.map_or(0.0, |(a, b)| {
        (a..=b)
            .map(|i| px.get(i as usize).copied().unwrap_or(0.0) * f)
            .sum()
    });
    // 반복 제목을 붙이지 않고 i 에서 시작한 띠가 제목 행·열을 담는지 (담으면 그 페이지에는 제목을 되풀이하지 않는다
    // — 한셀: 제목이 100~102 행이면 1·2 페이지에도 제목을 붙이고, 100 행이 든 페이지만 빼다)
    let holds_title = |start: u32| -> bool {
        let Some((ta, tb)) = title else { return false };
        let mut used = 0.0;
        for i in start..=range.1 {
            let h = px.get(i as usize).copied().unwrap_or(0.0) * f;
            if h <= 0.0 {
                continue;
            }
            if i != start && (manual.contains(&i) || used + h > avail + 1e-9) {
                break;
            }
            if (ta..=tb).contains(&i) {
                return true;
            }
            used += h;
        }
        false
    };
    let mut bands: Vec<Vec<u32>> = Vec::new();
    let mut titled = Vec::new();
    let mut cur: Vec<u32> = Vec::new();
    let mut used = 0.0;
    let mut cur_titled = false;
    for i in range.0..=range.1 {
        let h = px.get(i as usize).copied().unwrap_or(0.0);
        if h <= 0.0 {
            continue;
        }
        let hh = h * f;
        let forced = manual.contains(&i) && i != range.0;
        if !cur.is_empty() && (forced || used + hh > avail + 1e-9) {
            bands.push(std::mem::take(&mut cur));
            titled.push(cur_titled);
            used = 0.0;
        }
        if cur.is_empty() {
            cur_titled = title.is_some() && !holds_title(i);
            if cur_titled {
                used = title_px;
            }
        }
        cur.push(i);
        used += hh;
    }
    if !cur.is_empty() || bands.is_empty() {
        bands.push(cur);
        titled.push(cur_titled);
    }
    Bands { bands, titled }
}

/// 한 인쇄 범위의 페이지 나누기 (가로 띠, 세로 띠)
fn split_area(
    s: &Settings,
    a: &Area,
    row_px: &[f64],
    col_px: &[f64],
    f: f64,
    avail: (f64, f64),
) -> (Bands, Bands) {
    // 페이지에 맞추기면 페이지 수를 정한 방향의 수동 나누기는 쓰지 않는다 (한셀: 1×3 맞추기의 20 행 나누기는
    // 사라지고, 세로 제한이 없으면 남는다)
    let (no_rows, no_cols) = match s.scaling {
        Scaling::Fit { wide, tall } => (tall > 0, wide > 0),
        Scaling::Zoom(_) => (false, false),
    };
    let rows = split(
        (a.r0, a.r1),
        row_px,
        f,
        avail.1,
        if no_rows { &[] } else { &s.row_breaks },
        s.title_rows,
    );
    let cols = split(
        (a.c0, a.c1),
        col_px,
        f,
        avail.0,
        if no_cols { &[] } else { &s.col_breaks },
        s.title_cols,
    );
    (rows, cols)
}

/// (용지 − 여백) 의 너비·높이 (pt)
fn printable(s: &Settings) -> (f64, f64) {
    (
        s.paper.width - s.left - s.right,
        s.paper.height - s.top - s.bottom,
    )
}

/// 시트의 인쇄 배치를 계산한다
pub fn sheet_print(wb: &Workbook, sheet_index: usize) -> SheetPrint {
    let s = settings(wb, sheet_index);
    let sheet = &wb.sheets[sheet_index];
    let max_r = s
        .areas
        .iter()
        .map(|a| a.r1)
        .chain(s.title_rows.map(|t| t.1))
        .max()
        .unwrap_or(0)
        .min(1_048_575);
    let max_c = s
        .areas
        .iter()
        .map(|a| a.c1)
        .chain(s.title_cols.map(|t| t.1))
        .max()
        .unwrap_or(0)
        .min(16_383);
    // 전체 행·열을 가리키는 영역은 쓰는 범위로 줄인다
    let (ur, uc) = if max_r >= 1_048_575 || max_c >= 16_383 {
        used_end(wb, sheet).unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    let max_r = if max_r >= 1_048_575 { ur } else { max_r };
    let max_c = if max_c >= 16_383 { uc } else { max_c };
    let mut s = s;
    for a in &mut s.areas {
        a.r1 = a.r1.min(max_r);
        a.c1 = a.c1.min(max_c);
    }
    let geo = render::sheet_geometry(wb, sheet, max_r + 1, max_c + 1);
    let row_px: Vec<u32> = geo.rows.iter().map(|r| r.1).collect();
    let col_px: Vec<u32> = geo.cols.iter().map(|c| c.1).collect();
    // 배율 지정의 페이지 나누기는 화면 px (행마다 반올림), 페이지에 맞추기는 한셀 안의 크기 (COM Height·Width) —
    // 서식 파일 Basic_Invoice 서식 시트: 행 높이 합 817.45pt 로 85%, 반올림한 px 합(1096)이면 84%
    let row_round: Vec<f64> = row_px.iter().map(|&v| v as f64).collect();
    let col_round: Vec<f64> = col_px.iter().map(|&v| v as f64).collect();
    // 직접 정한 높이(customHeight)는 pt 그대로, 자동 높이 행은 화면 px (CM_and_BreakEven_Analysis: 빈 자동 행
    // 13.2pt 를 18px 로 세야 33%)
    let custom: std::collections::HashSet<u32> = sheet
        .rows
        .iter()
        .filter(|r| r.custom_height && r.index >= 1)
        .map(|r| r.index - 1)
        .collect();
    let row_exact: Vec<f64> = geo
        .row_pt
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if custom.contains(&(i as u32)) {
                v / PX
            } else {
                row_px[i] as f64
            }
        })
        .collect();
    let col_exact: Vec<f64> = geo.col_px.clone();

    let (pw, ph) = printable(&s);
    let avail100 = (
        (pw / HUNDREDTH + 0.5).floor(),
        (ph / HUNDREDTH + 0.5).floor(),
    );
    let avail75 = (pw / PX, ph / PX);

    // 1. 페이지 나누기 배율
    let paginate = |z: u32, avail: (f64, f64), exact: bool| -> Vec<(Bands, Bands)> {
        let (rp, cp) = if exact {
            (&row_exact, &col_exact)
        } else {
            (&row_round, &col_round)
        };
        s.areas
            .iter()
            .map(|a| split_area(&s, a, rp, cp, z as f64 / 100.0, avail))
            .collect()
    };
    let (page_scale, split) = match s.scaling {
        Scaling::Zoom(z) => (z, paginate(z, avail100, false)),
        Scaling::Fit { wide, tall } => {
            // 95 부터 1 씩 줄여 25 까지, 그래도 넘치면 12 (페이지에 맞추기 대조 실험)
            let ok = |sp: &[(Bands, Bands)]| {
                sp.iter().all(|(r, c)| {
                    (wide == 0 || c.bands.len() as u32 <= wide)
                        && (tall == 0 || r.bands.len() as u32 <= tall)
                })
            };
            let mut found = None;
            for z in (25..=95).rev().chain([12]) {
                let sp = paginate(z, avail75, true);
                if ok(&sp) {
                    found = Some((z, sp));
                    break;
                }
            }
            found.unwrap_or_else(|| (12, paginate(12, avail75, true)))
        }
    };

    // 페이지 목록
    let mut pages = Vec::new();
    let mut h_breaks = Vec::new();
    let mut v_breaks = Vec::new();
    for (ai, (rb, cb)) in split.iter().enumerate() {
        let a = &s.areas[ai];
        let title_list = |t: Option<(u32, u32)>, px: &[u32]| -> Vec<u32> {
            t.map(|(x, y)| {
                (x..=y)
                    .filter(|&i| px.get(i as usize).copied().unwrap_or(0) > 0)
                    .collect()
            })
            .unwrap_or_default()
        };
        let trows = title_list(s.title_rows, &row_px);
        let tcols = title_list(s.title_cols, &col_px);
        let mut push = |ri: usize, ci: usize| {
            pages.push(Page {
                area: ai,
                rows: rb.bands[ri].clone(),
                cols: cb.bands[ci].clone(),
                title_rows: if rb.titled[ri] {
                    trows.clone()
                } else {
                    Vec::new()
                },
                title_cols: if cb.titled[ci] {
                    tcols.clone()
                } else {
                    Vec::new()
                },
            })
        };
        if s.over_then_down {
            for ri in 0..rb.bands.len() {
                for ci in 0..cb.bands.len() {
                    push(ri, ci);
                }
            }
        } else {
            for ci in 0..cb.bands.len() {
                for ri in 0..rb.bands.len() {
                    push(ri, ci);
                }
            }
        }
        for b in rb.bands.iter().skip(1) {
            if let Some(&r) = b.first() {
                h_breaks.push((r + 1, s.row_breaks.contains(&r)));
            }
        }
        for b in cb.bands.iter().skip(1) {
            if let Some(&c) = b.first() {
                v_breaks.push((c + 1, s.col_breaks.contains(&c)));
            }
        }
        if ai + 1 < split.len() {
            h_breaks.push((a.r1 + 2, false));
            v_breaks.push((a.c1 + 2, false));
        }
    }
    h_breaks.sort();
    h_breaks.dedup_by_key(|b| b.0);
    v_breaks.sort();
    v_breaks.dedup_by_key(|b| b.0);

    // 2. 인쇄 배율
    let sum = |list: &[u32], px: &[u32], z: u32| -> u32 {
        list.iter()
            .map(|&i| scaled(px.get(i as usize).copied().unwrap_or(0), z))
            .sum()
    };
    let fits = |z: u32| {
        pages.iter().all(|p| {
            let h = sum(&p.title_rows, &row_px, z) + sum(&p.rows, &row_px, z);
            let w = sum(&p.title_cols, &col_px, z) + sum(&p.cols, &col_px, z);
            h as f64 <= avail75.1 + 1e-9 && w as f64 <= avail75.0 + 1e-9
        })
    };
    let mut tall = false;
    let mut z = page_scale;
    if matches!(s.scaling, Scaling::Zoom(100)) {
        // 배율 100: 세로 6.25 배로도 들어가면 그렇게 (가로 1:1), 아니면 99 부터
        tall = pages.iter().all(|p| {
            let h: u32 = p
                .title_rows
                .iter()
                .chain(&p.rows)
                .map(|&r| row_px[r as usize])
                .sum();
            let w: u32 = p
                .title_cols
                .iter()
                .chain(&p.cols)
                .map(|&c| col_px[c as usize])
                .sum();
            h as f64 * TALL <= avail75.1 + 1e-9 && w as f64 <= avail75.0 + 1e-9
        }) && pages.iter().any(|p| !p.rows.is_empty());
        z = if tall { 100 } else { 99 };
    }
    if !tall {
        while z > 10 && !fits(z) {
            z -= 1;
        }
    }
    SheetPrint {
        settings: s,
        pages,
        page_scale,
        print_scale: z,
        tall,
        h_breaks,
        v_breaks,
        row_px,
        col_px,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_names() {
        assert_eq!(
            name_ranges("'시트 1'!$B$3:$H$46,'시트 1'!$J$1:$K$2"),
            vec![
                (Some(2), Some(1), Some(45), Some(7)),
                (Some(0), Some(9), Some(1), Some(10))
            ]
        );
        assert_eq!(
            name_ranges("Sheet1!$1:$3,Sheet1!$A:$B"),
            vec![
                (Some(0), None, Some(2), None),
                (None, Some(0), None, Some(1))
            ]
        );
        assert!(name_ranges("'[1]Average Selling Price'!$B$3:$H$46").is_empty());
        assert!(name_ranges("#REF!").is_empty());
    }

    #[test]
    fn scaled_rounds_half_up() {
        assert_eq!(scaled(20, 97), 19);
        assert_eq!(scaled(10, 95), 10);
        assert_eq!(scaled(6, 91), 5);
        assert_eq!(scaled(150, 97), 146);
    }
}
