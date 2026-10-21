// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 화면 배치: 열 너비·행 높이·숨김·병합 (한셀 COM·화면 갈무리 대조 실험, docs/format/screen.md).
//!
//! - 열: 저장된 너비 w(글자) × K(글꼴 0 의 숫자 폭, 정수 px)가 한셀 안의 너비(px, 소수)다. COM
//!   `Range.Width`·`Left` 는 이것을 1/20 pt 로 반올림해 돌려주고, 화면은 열마다 반올림한 정수 px 로 그린다
//!   (머리글 구분선 대조: 32.5 → 33, 32.48 → 32, 12.5 → 13).
//! - 행: 높이는 1/20 pt 로 자른 pt, 화면은 행마다 round(twips/15) px (13.125pt → 17px, 13.2pt → 18px).
//!   직접 정한 높이(customHeight)만 저장 값을 쓰고 나머지는 열 때 다시 맞춘다 (`ht` 만 있고 customHeight 가
//!   없는 행도 다시 맞춘다 — 한셀이 저장한 문서도 같다, 자동 높이 대조 실험). 맞춘 높이 = 그 행의 모든 칸(셀
//!   레코드가 없으면 행 서식 → 열 서식 → 0 번 서식)이 요구하는 높이 중 최대: 글꼴 기본 높이(값 없는 칸도),
//!   위·아래 테두리 굵기, 줄 바꿈 줄 수, 돌린 글자 상자, 문단 모양의 위·아래 간격. 여러 행에 걸친 병합 칸은
//!   뺀다. 행 요소가 없는 행도 같은 규칙이라 기본 높이는 열 서식들로 정해진다
//!   (`sheetFormatPr/@defaultRowHeight` 는 customHeight 가 없으면 무시 — 기본 높이 대조 실험).

use std::collections::{HashMap, HashSet};

use super::metrics::{self, FontMetrics, col_px, default_col_chars, row_px};
use crate::model::{Cell, CellValue, Font, RowInfo, Sheet, Workbook};

/// 칸 안 왼쪽 여백 (px): 한셀 화면에서 왼쪽 맞춤 글자의 잉크가 경계 다음 픽셀부터 2px 안쪽 (보정 문서 갈무리)
pub(super) const PAD: f64 = 2.0;
/// 오른쪽 맞춤 글자의 오른쪽 여백 (px, 격자선 제외): 한셀 화면에서 글자 끝이 격자선 2px 앞
pub(super) const PAD_R: f64 = 2.0;

/// 한 시트에 열 수 있는 열 수 (XFD)
const MAX_COLS: u32 = 16384;

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

pub(super) fn font_metrics(f: Option<&Font>) -> FontMetrics {
    FontMetrics::new(
        f.and_then(|f| f.name.as_deref()),
        f.and_then(effective_size),
        f.is_some_and(|f| f.bold),
    )
}

/// 글꼴 크기 (pt) × 한컴 상대 크기(hs:size, %) — 한셀은 이 크기로 글자를 그리고 행 높이를 맞춘다
/// (글꼴 대조 문서: 20pt·상대 크기 83% 행 → 16.6pt 높이 24.35)
pub(super) fn effective_size(f: &Font) -> Option<f64> {
    let rel = f
        .hancom
        .as_ref()
        .and_then(|h| h.size)
        .filter(|&s| s > 0 && s != 100)
        .map_or(1.0, |s| s as f64 / 100.0);
    f.size.map(|s| s * rel)
}

/// 위·아래 테두리가 행 자동 높이에 더하는 값 (1/20 pt): 보통 굵기 7.5, 굵은 선·이중선 15 (변마다).
/// 왼쪽·오른쪽·대각선과 가는 선(점선 포함)은 더하지 않는다 (테두리 자동 높이 대조 실험)
fn border_twips(wb: &Workbook, style: u32) -> f64 {
    let Some(b) = wb
        .styles
        .xf(style)
        .and_then(|x| wb.styles.borders.get(x.border_id as usize))
    else {
        return 0.0;
    };
    let one = |e: Option<&crate::model::BorderEdge>| match e.and_then(|e| e.style.as_deref()) {
        Some("medium" | "mediumDashed" | "mediumDashDot" | "mediumDashDotDot") => 7.5,
        Some("thick" | "double") => 15.0,
        _ => 0.0,
    };
    one(b.top.as_ref()) + one(b.bottom.as_ref())
}

/// 서식 하나(값 없는 칸)가 행에 요구하는 높이 (1/20 pt): 글꼴 기본 높이 + 테두리
fn style_twips(wb: &Workbook, style: u32) -> f64 {
    let m = font_metrics(wb.styles.font_of(style));
    m.row_pt * 20.0 + m.row_frac + border_twips(wb, style)
}

/// 기본 열 너비 (글자 수). defaultColWidth 가 없으면 한셀 기본(8글자 기준), 정수면 레거시 DEFCOLWIDTH
/// 같은 "기준 너비"로 본다 (한셀이 레거시를 읽은 결과와 같다: 8 → 9.140625)
pub fn default_col_width(sheet: &Sheet, k: f64) -> f64 {
    match sheet.default_col_width {
        None => default_col_chars(8.0, k),
        Some(w) if w.fract() == 0.0 => default_col_chars(w, k),
        Some(w) => w,
    }
}

/// 기본 열 너비 (화면 px)
pub fn default_col_px(sheet: &Sheet, k: f64) -> u32 {
    col_px(default_col_width(sheet, k), k)
}

/// 화면 배치 (시트 원점 = A1 왼쪽 위, px). 숨긴 열·행은 너비·높이 0
pub(super) struct Layout {
    pub nrows: u32,
    pub ncols: u32,
    /// 화면 px (열마다 반올림)
    pub col_w: Vec<u32>,
    pub col_x: Vec<u32>,
    pub row_h: Vec<u32>,
    pub row_y: Vec<u32>,
    /// 한셀 안의 열 너비 (px 소수) — COM `Range.Width` 와 대조
    pub col_exact: Vec<f64>,
    /// 행 높이 (pt) — COM `Range.Height` 와 대조
    pub row_pt: Vec<f64>,
    /// 병합 시작 칸 → (보이는 행 수, 열 수)
    pub span: HashMap<(u32, u32), (u32, u32)>,
    /// 병합 시작 칸 → 끝 칸
    pub merge_end: HashMap<(u32, u32), (u32, u32)>,
    /// 병합에 덮인 칸 (시작 칸 제외)
    pub covered: HashSet<(u32, u32)>,
}

impl Layout {
    pub fn col_w(&self, c: u32) -> u32 {
        self.col_w.get(c as usize).copied().unwrap_or(0)
    }
    pub fn row_h(&self, r: u32) -> u32 {
        self.row_h.get(r as usize).copied().unwrap_or(0)
    }
    pub fn col_x(&self, c: u32) -> u32 {
        self.col_x.get(c as usize).copied().unwrap_or(0)
    }
    pub fn row_y(&self, r: u32) -> u32 {
        self.row_y.get(r as usize).copied().unwrap_or(0)
    }
    /// 보이는 열 (0..n)
    pub fn cols(&self, n: u32) -> Vec<u32> {
        (0..n.min(self.ncols)).filter(|&c| self.col_w(c) > 0).collect()
    }
    pub fn rows(&self, n: u32) -> Vec<u32> {
        (0..n.min(self.nrows)).filter(|&r| self.row_h(r) > 0).collect()
    }
    /// 칸(병합이면 병합 영역 전체)의 너비·높이
    pub fn cell_size(&self, r: u32, c: u32) -> (u32, u32) {
        match self.merge_end.get(&(r, c)) {
            Some(&(r1, c1)) => (
                (c..=c1).map(|k| self.col_w(k)).sum(),
                (r..=r1).map(|k| self.row_h(k)).sum(),
            ),
            None => (self.col_w(c), self.row_h(r)),
        }
    }
}

/// 값·서식이 있는 칸, 병합, 그리기 개체가 차지하는 범위 (행 수, 열 수)
pub(super) fn used_extent(sheet: &Sheet) -> (u32, u32) {
    let mut max_r = 0;
    let mut max_c = 0;
    let mut any = false;
    for c in &sheet.cells {
        if !c.value.is_empty() || c.style != 0 {
            max_r = max_r.max(c.r.row);
            max_c = max_c.max(c.r.col);
            any = true;
        }
    }
    for m in &sheet.merges {
        max_r = max_r.max(m.end.row);
        max_c = max_c.max(m.end.col);
        any = true;
    }
    for s in sheet.shapes.iter().filter(|s| !s.hidden) {
        let to = s.anchor.to.unwrap_or(s.anchor.from);
        max_r = max_r.max(to.row);
        max_c = max_c.max(to.col);
        any = true;
    }
    if any { (max_r + 1, max_c + 1) } else { (0, 0) }
}

/// 칸 값의 표시 글자 (행 높이 맞춤용)
fn display_plain(wb: &Workbook, style: u32, v: &CellValue) -> String {
    let code = wb
        .styles
        .xf(style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    crate::numfmt::format(v, &code, wb.date1904).text
}

/// 글자 줄 수 (줄 바꿈 칸): 너비 `avail` px 에 맞춰 한글은 아무 데서나, 영문은 단어 단위로 끊는다
pub(super) fn wrap_lines(text: &str, avail: f64, m: &FontMetrics) -> u32 {
    let avail = avail.max(m.px);
    let mut lines = 0;
    for para in text.split('\n') {
        lines += 1;
        // x: 줄에 놓인 폭, word: 끊지 않은 영문 단어 폭
        let mut x = 0.0;
        let mut word = 0.0;
        for ch in para.chars() {
            let w = m.char_width(ch);
            if ch == ' ' || metrics::is_wide(ch) {
                if x + word + w > avail && x + word > 0.0 {
                    lines += 1;
                    // 공백은 줄 끝에 남기고, 한글은 다음 줄 첫 글자
                    x = if ch == ' ' { 0.0 } else { w };
                    word = 0.0;
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

/// n 줄 글의 높이 (pt): 1/20 pt 로 반올림한 (1줄 + (n−1)·한 줄) — 자동 높이 대조: 맑은 고딕 11 의
/// 2줄 32.75, 3줄 49.15, 5줄 81.85, 함초롬돋움 10 의 2줄 26.1. 행 높이는 409.5pt 를 넘지 않는다
fn lines_pt(n: u32, m: &FontMetrics) -> f64 {
    if n <= 1 {
        return m.row_pt;
    }
    let h1 = m.row_pt * 20.0 + m.row_frac;
    ((h1 + (n - 1) as f64 * m.line_twips).round() / 20.0).min(MAX_ROW_PT)
}

/// 행 높이 한계 (pt)
const MAX_ROW_PT: f64 = 409.5;

/// 돌린 글자 상자의 높이에 더하는 값 (pt): 각도별로 잰 값 (맑은 고딕 11·20pt, 회전 자동 높이 대조 실험)
fn rotated_base_pt(deg: f64, size: f64) -> f64 {
    // (각도, 11pt, 20pt)
    const T: [(f64, f64, f64); 6] = [
        (0.0, 24.37, 37.59),
        (15.0, 21.39, 32.15),
        (30.0, 18.41, 26.71),
        (45.0, 15.41, 21.24),
        (60.0, 12.70, 16.31),
        (90.0, 8.25, 8.25),
    ];
    let d = deg.clamp(0.0, 90.0);
    let i = T.iter().position(|t| t.0 >= d).unwrap_or(T.len() - 1).max(1);
    let (a, b) = (T[i - 1], T[i]);
    let t = (d - a.0) / (b.0 - a.0);
    let at = |s: f64| {
        let v11 = a.1 + (b.1 - a.1) * t;
        let v20 = a.2 + (b.2 - a.2) * t;
        v11 + (v20 - v11) * (s - 11.0) / 9.0
    };
    at(size).max(0.0)
}

/// 칸 하나가 행에 요구하는 높이 (1/20 pt): 글자 높이(1/20 pt 로 반올림) + 위·아래 테두리
pub(super) fn cell_height_twips(wb: &Workbook, cell: &Cell, col_w_px: u32) -> f64 {
    let m = font_metrics(wb.styles.font_of(cell.style));
    let text = (cell_text_pt(wb, cell, col_w_px) * 20.0).round();
    // 한 줄이면 글꼴 높이의 잘린 아래 자리를 되살린다
    let frac = if text == (m.row_pt * 20.0).round() {
        m.row_frac
    } else {
        0.0
    };
    // 문단 모양의 위·아래 간격 (1/100 pt, hs:parashape prevSpacing/nextSpacing): 값이 있는 칸에만
    // (맞춤 대조 문서: 444 + 555 → 16.4 + 9.99 = 26.4)
    let spacing = if cell.value.is_empty() {
        0.0
    } else {
        wb.styles
            .xf(cell.style)
            .and_then(|x| x.hancom.as_ref())
            .and_then(|h| h.parashape.as_ref())
            .map_or(0.0, |p| {
                (p.prev_spacing.unwrap_or(0).max(0) + p.next_spacing.unwrap_or(0).max(0)) as f64
                    / 100.0
                    * 20.0
            })
    };
    text + frac + spacing + border_twips(wb, cell.style)
}

/// 칸 글자의 높이 (pt)
fn cell_text_pt(wb: &Workbook, cell: &Cell, col_w_px: u32) -> f64 {
    let st = &wb.styles;
    let font = st.font_of(cell.style);
    let m = font_metrics(font);
    let one = m.row_pt;
    if cell.value.is_empty() {
        return one;
    }
    let al = st.xf(cell.style).and_then(|x| x.alignment.as_ref());
    let rot = al.and_then(|a| a.text_rotation).unwrap_or(0);
    let text = display_plain(wb, cell.style, &cell.value);
    // 공유 문자열의 서식 있는 글: 조각 글꼴 중 가장 높은 것
    let rich = match &cell.value {
        CellValue::Text(t) => wb
            .shared_strings
            .iter()
            .find(|s| s.runs.len() > 1 && s.text == *t)
            .map(|s| {
                s.runs
                    .iter()
                    .filter_map(|r| r.font.as_ref())
                    .map(|f| {
                        let mut f = f.clone();
                        if f.name.is_none() {
                            f.name = font.and_then(|x| x.name.clone());
                        }
                        font_metrics(Some(&f)).row_pt
                    })
                    .fold(one, f64::max)
            }),
        _ => None,
    };
    let one = rich.unwrap_or(one);
    let h = if rot == 255 {
        // 세로 쓰기: 글자마다 한 줄
        lines_pt(text.chars().filter(|c| *c != '\n').count() as u32, &m) + 0.75
    } else if rot != 0 {
        let deg = if rot > 90 { rot - 90 } else { rot } as f64;
        // 글자 폭은 글자마다 정수 px
        let tw: f64 = text.chars().map(|c| m.char_width(c).round()).sum();
        let size = font.and_then(effective_size).unwrap_or(metrics::DEFAULT_SIZE);
        // 돌린 글자는 글꼴 기본 높이를 최솟값으로 두지 않는다 (20pt 90도 "0" → 19.5pt)
        return rotated_base_pt(deg, size) + tw * 0.75 * deg.to_radians().sin();
    } else if al.is_some_and(|a| {
        a.wrap_text || matches!(a.horizontal.as_deref(), Some("justify" | "distributed"))
    }) {
        let indent = al.and_then(|a| a.indent).unwrap_or(0) as f64 * super::INDENT_PX;
        let avail = col_w_px as f64 - PAD - PAD_R - 1.0 - indent;
        lines_pt(wrap_lines(&text, avail, &m), &m)
    } else {
        one
    };
    h.max(one)
}

/// 행 높이 맞춤에 쓰는 열 서식의 높이 (1/20 pt): 열 서식이 같은 범위마다 (열 범위, 높이)
struct ColStyleHeights {
    ranges: Vec<(u32, u32, f64)>,
    /// 서식 없는 열이 하나라도 있으면 0 번 서식의 높이
    rest: Option<f64>,
    /// 범위 밖(서식 없는) 열 수
    rest_count: u32,
}

fn col_style_heights(wb: &Workbook, sheet: &Sheet) -> ColStyleHeights {
    let mut ranges = Vec::new();
    let mut covered = 0u32;
    for ci in &sheet.cols {
        let (a, b) = (ci.min.max(1) - 1, ci.max.min(MAX_COLS) - 1);
        if b < a {
            continue;
        }
        covered += b - a + 1;
        let s = ci.style.unwrap_or(0);
        ranges.push((a, b, style_twips(wb, s)));
    }
    let rest_count = MAX_COLS.saturating_sub(covered);
    ColStyleHeights {
        ranges,
        rest: (rest_count > 0).then(|| style_twips(wb, 0)),
        rest_count,
    }
}

impl ColStyleHeights {
    /// 셀 레코드가 없는 열들의 높이 중 최대 (`taken`: 레코드가 있는 열)
    fn max_without(&self, taken: &HashSet<u32>) -> f64 {
        let mut h: f64 = 0.0;
        for &(a, b, v) in &self.ranges {
            let n = b - a + 1;
            let used = taken.iter().filter(|&&c| (a..=b).contains(&c)).count() as u32;
            if used < n {
                h = h.max(v);
            }
        }
        if let Some(v) = self.rest {
            let used = taken
                .iter()
                .filter(|&&c| !self.ranges.iter().any(|&(a, b, _)| (a..=b).contains(&c)))
                .count() as u32;
            if used < self.rest_count {
                h = h.max(v);
            }
        }
        h
    }
}

/// 배치를 계산한다 (행 0..nrows, 열 0..ncols)
pub(super) fn layout(wb: &Workbook, sheet: &Sheet, nrows: u32, ncols: u32) -> Layout {
    let book = book_metrics(wb);
    let k = book.k;
    let default_w = default_col_width(sheet, k);

    // 열
    let mut col_exact = vec![default_w * k; ncols as usize];
    for ci in &sheet.cols {
        let a = ci.min.max(1) - 1;
        for c in a..ci.max.min(ncols) {
            col_exact[c as usize] = if ci.hidden || ci.width == Some(0.0) {
                0.0
            } else {
                ci.width.unwrap_or(default_w) * k
            };
        }
    }
    let col_w: Vec<u32> = col_exact
        .iter()
        .map(|&e| if e > 0.0 { col_px(e / k, k).max(1) } else { 0 })
        .collect();

    // 여러 행에 걸친 병합 칸은 자동 높이에서 빠진다 (한 행 안의 병합은 센다 — 병합 대조 실험)
    let mut merged = HashSet::new();
    for m in sheet.merges.iter().filter(|m| m.end.row > m.start.row) {
        for r in m.start.row..=m.end.row.min(nrows.saturating_sub(1).max(m.start.row)) {
            for c in m.start.col..=m.end.col {
                merged.insert((r, c));
            }
        }
    }

    // 행
    let row_info: HashMap<u32, &RowInfo> = sheet
        .rows
        .iter()
        .filter(|r| r.index >= 1)
        .map(|r| (r.index - 1, r))
        .collect();
    let mut by_row: HashMap<u32, Vec<&Cell>> = HashMap::new();
    for c in &sheet.cells {
        if c.r.row < nrows {
            by_row.entry(c.r.row).or_default().push(c);
        }
    }
    let col_heights = col_style_heights(wb, sheet);
    let none = HashSet::new();
    let base_pt = {
        let h = col_heights.max_without(&none).floor() / 20.0;
        if h > 0.0 { h } else { book.row_pt }
    };
    // 행 요소가 없는 행도 열 서식 글꼴로 맞춘 높이 (defaultRowHeight 는 쓰지 않는다)
    let sheet_default = base_pt;
    let col_screen = |c: u32| col_w.get(c as usize).copied().unwrap_or_else(|| col_px(default_w, k));
    let mut row_pt = Vec::with_capacity(nrows as usize);
    for r in 0..nrows {
        let info = row_info.get(&r);
        let cells = by_row.get(&r).map(Vec::as_slice).unwrap_or(&[]);
        // 저장된 높이와 맞춘 높이 모두 1/20 pt 로 자른다 (글자 높이는 반올림해 둔다: 3줄 49.15, 보통 굵기 아래 선 +7.5 → 13.55)
        let stored = |h: f64| (h * 20.0 + 1e-7).floor() / 20.0;
        let pt = match info {
            Some(i) if i.hidden => 0.0,
            Some(i) if i.custom_height && i.height.is_some() => stored(i.height.unwrap_or(0.0)),
            _ if info.is_none() && cells.is_empty() => sheet_default,
            _ => {
                // 맞춘 높이: 레코드가 있는 칸 + 나머지 칸의 행·열 서식
                let mut h: f64 = 0.0;
                let mut taken = HashSet::new();
                for c in cells {
                    taken.insert(c.r.col);
                    if merged.contains(&(r, c.r.col)) {
                        continue;
                    }
                    h = h.max(cell_height_twips(wb, c, col_screen(c.r.col)));
                }
                let rest = match info.and_then(|i| i.style) {
                    Some(s) if (taken.len() as u32) < MAX_COLS => style_twips(wb, s),
                    Some(_) => 0.0,
                    None => col_heights.max_without(&taken),
                };
                h = h.max(rest);
                if h <= 0.0 { base_pt } else { h.floor() / 20.0 }
            }
        };
        row_pt.push(pt.max(0.0));
    }
    let row_h: Vec<u32> = row_pt.iter().map(|&p| if p > 0.0 { row_px(p) } else { 0 }).collect();

    let mut col_x = Vec::with_capacity(ncols as usize);
    let mut x = 0;
    for &w in &col_w {
        col_x.push(x);
        x += w;
    }
    let mut row_y = Vec::with_capacity(nrows as usize);
    let mut y = 0;
    for &h in &row_h {
        row_y.push(y);
        y += h;
    }

    let mut span = HashMap::new();
    let mut merge_end = HashMap::new();
    let mut covered = HashSet::new();
    let vis_c = |c: u32| col_w.get(c as usize).is_none_or(|&w| w > 0);
    let vis_r = |r: u32| row_h.get(r as usize).is_none_or(|&h| h > 0);
    for m in &sheet.merges {
        let rs = (m.start.row..=m.end.row).filter(|&r| vis_r(r)).count() as u32;
        let cs = (m.start.col..=m.end.col).filter(|&c| vis_c(c)).count() as u32;
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
        nrows,
        ncols,
        col_w,
        col_x,
        row_h,
        row_y,
        col_exact,
        row_pt,
        span,
        merge_end,
        covered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_heights_follow_measurements() {
        // 맑은 고딕 11: 90도 "가나다라" (15px × 4) → 53.25pt, 45도 "가" → 23.35pt
        assert!((rotated_base_pt(90.0, 11.0) + 60.0 * 0.75 - 53.25).abs() < 0.01);
        let h45 = rotated_base_pt(45.0, 11.0) + 15.0 * 0.75 * 45f64.to_radians().sin();
        assert!((h45 - 23.35).abs() < 0.06, "{h45}");
        let m = FontMetrics::new(Some("맑은 고딕"), Some(11.0), false);
        assert!((lines_pt(4, &m) + 0.75 - 66.25).abs() < 0.06);
        assert_eq!((lines_pt(3, &m) * 20.0).round() / 20.0, 49.15);
    }

    #[test]
    fn wrap_counts_hangul_lines() {
        let m = FontMetrics::new(Some("맑은 고딕"), Some(11.0), false);
        // 72px 칸(글자 자리 65px)에 한글 4자씩: 18자 + 공백 → 5줄 (자동 높이 대조 81.85pt)
        assert_eq!(wrap_lines("가나다라마바사아자차카타파하 가나다라", 65.0, &m), 5);
        assert_eq!(wrap_lines("가\n가", 65.0, &m), 2);
    }
}
