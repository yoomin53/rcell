// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 인쇄 페이지 한 장의 칸(채우기·격자선·테두리·글자)을 장치 단위(600 dpi, 위가 0) 그리기 목록으로
//! (한셀 PDF 대조 실험, docs/format/print.md). 화면 표와 같은 칸 내용([`super::Views`])을 쓰고, 자리만 인쇄
//! 규칙으로 정한다:
//! - 칸 상자는 행·열 경계에서 장치 6 만큼 왼쪽·위 (화면처럼 칸의 마지막 픽셀이 격자선 자리). 채우기는 그 상자,
//!   가는 선·중간 선은 경계 − 6 이 선 가운데, 굵은 선은 경계가 가운데. 굵기는 장치 6·13·19.
//! - 글자: 왼쪽 맞춤은 경계 + 1px, 오른쪽 맞춤은 끝이 경계 − 4.2px, 가운데는 칸 가운데 − 1.3px. 세로는 화면
//!   줄 높이표의 한 줄 높이 L 과 글꼴 크기로: 위 맞춤 기준선 = 위 + L/2 + k·크기 + c (글꼴 무리마다 k·c).

use std::collections::{HashMap, HashSet};

use super::cell::CellView;
use super::{HAlign, INDENT_PX, VAlign, Views, overflow_spans, paint};
use crate::model::{CellValue, Sheet, Workbook};
use crate::theme;

/// 그릴 글꼴 (이름·굵게·기울임)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontKey {
    pub name: String,
    pub bold: bool,
    pub italic: bool,
}

/// 글자 폭 재기 (PDF 쪽이 실제 글꼴 파일로 잰다)
pub trait TextMeasure {
    /// `text` 를 `size`(장치 단위 글꼴 크기)로 쓴 폭 (장치 단위)
    fn width(&self, font: &FontKey, size: f64, text: &str) -> f64;
    /// 글꼴의 오름·내림 (em 의 비율) — 메타파일 글자의 기준선
    fn ascent_descent(&self, _font: &FontKey) -> (f64, f64) {
        (0.88, 0.12)
    }
    /// 글꼴 높이 `ppem`(장치 px) 에서 GDI 가 돌려주는 오름·내림 (장치 px, 내림은 양수) — 글꼴에 VDMX 표가 있으면 그 값,
    /// 없으면 비율을 반올림
    fn device_ascent_descent(&self, font: &FontKey, ppem: u32) -> (f64, f64) {
        let (a, d) = self.ascent_descent(font);
        ((a * ppem as f64).round(), (d * ppem as f64).round())
    }
}

/// 경로 조각 (장치 단위)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// 그리기 하나 (장치 단위, 위가 0). 색은 RGB
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// 경로: 채우기, 선 (굵기, 색, 점선)
    Path {
        segs: Vec<Seg>,
        fill: Option<[u8; 3]>,
        stroke: Option<(f64, [u8; 3], Vec<f64>)>,
        even_odd: bool,
        /// 선 끝·꺾임 (PDF 의 J·j). 없으면 PDF 기본 (평평·뾰족)
        caps: Option<(u8, u8)>,
        /// 빗금 채우기: (무늬, 빗금 색, 바탕색) — PDF 는 페이지에 맞춘 8×8 Tiling 무늬. 이때 `fill` 은 None
        hatch: Option<super::metafile::Hatch>,
    },
    /// 그림 (PNG·JPEG 바이트)
    Image {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        data: std::sync::Arc<Vec<u8>>,
    },
    Rect {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        color: [u8; 3],
    },
    Line {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        width: f64,
        color: [u8; 3],
        /// 점선 무늬 (장치 단위 길이들) — 비면 실선
        dash: Vec<f64>,
    },
    /// 한 줄 글자. (x, y) 는 기준선 왼쪽 끝
    Text {
        x: f64,
        y: f64,
        size: f64,
        font: FontKey,
        color: [u8; 3],
        text: String,
        underline: bool,
        strike: bool,
        /// 가로 장평 (1 = 그대로)
        stretch: f64,
        /// 기준선 왼쪽 끝을 중심으로 돌린 각도 (도, 시계 방향 — 장치 좌표는 아래가 +)
        angle: f64,
    },
    /// 상자 안을 장치 해상도 그림으로 그릴 묶음 (차트 — 한셀 PDF 는 차트를 비트맵으로 인쇄한다). PDF 가 그림으로 못 그리면
    /// 안의 ops 를 벡터로 그린다
    Raster {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        ops: Vec<Op>,
    },
    /// 사각형으로 자르기 시작 / 끝
    Clip {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Unclip,
}

/// 오류 값 인쇄 (`pageSetup/@errors`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorPrint {
    #[default]
    Displayed,
    Blank,
    Dash,
    Na,
}

/// 페이지 한 장의 자리 (장치 단위)
#[derive(Debug, Clone, Default)]
pub struct PageFrame {
    /// 그릴 열: (열 번호, 왼쪽 경계, 오른쪽 경계) — 반복 열이 먼저
    pub cols: Vec<(u32, f64, f64)>,
    /// 그릴 행: (행 번호, 위 경계, 아래 경계) — 반복 행이 먼저
    pub rows: Vec<(u32, f64, f64)>,
    /// 모든 행·열의 인쇄 크기 (장치 단위) — 페이지 밖으로 이어지는 병합 칸의 자리를 잴 때
    pub row_dev: Vec<f64>,
    pub col_dev: Vec<f64>,
    /// 화면 1px 의 장치 크기 (칸 안 여백·점선 무늬): 6.25 × 배율
    pub px: f64,
    /// 배율 (%) — 글꼴 크기 = round(pt × 25/3 × z/100)
    pub zoom: u32,
    /// 페이지 원점 (장치 단위)
    pub origin_x: f64,
    /// 모든 열·행의 화면 px 누적 시작 자리 (번호 = 위치, 끝에 전체) — 그리기 개체는 칸 경계가 아니라 시트의 절대 px 에
    /// 배율을 곱한 자리에 놓인다 (한셀 PDF 그림·도형 실험)
    pub col_abs: Vec<f64>,
    pub row_abs: Vec<f64>,
    pub gridlines: bool,
    pub black_and_white: bool,
    pub draft: bool,
    pub errors: ErrorPrint,
}

impl PageFrame {
    /// 글꼴 크기 pt → 장치 단위 (한셀 PDF 의 Tf: 정수로 반올림)
    pub fn font_size(&self, pt: f64) -> f64 {
        (pt * 25.0 / 3.0 * self.zoom as f64 / 100.0 + 0.5)
            .floor()
            .max(1.0)
    }
}

const BLACK: [u8; 3] = [0, 0, 0];

/// `#RRGGBB` → RGB
fn rgb(css: &str) -> Option<[u8; 3]> {
    let h = css.trim().trim_start_matches('#');
    if h.len() < 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// 흑백 인쇄: 글자·선은 검정, 채우기는 흰색 (한셀 bw 실험)
fn bw(c: [u8; 3], on: bool) -> [u8; 3] {
    if on { BLACK } else { c }
}

/// 글꼴 무리별 세로 자리 상수 (위 맞춤 c·k, 아래 맞춤 c·k, 가운데 c·k): 장치 단위, 배율 99 에서 잰 값.
/// 같은 줄 높이표를 쓰는 글꼴은 글꼴 파일과 상관없이 같은 자리 (맑은 고딕·Arial·Times New Roman 이 같고,
/// 굴림·돋움·바탕·궁서가 같다 — 글꼴 9종 × 크기 10 가지 대조 실험)
fn v_consts(name: &str) -> [(f64, f64); 3] {
    let base = name.trim_end_matches('체');
    match base {
        "굴림" | "돋움" | "바탕" | "궁서" | "Gulim" | "Dotum" | "Batang" | "Gungsuh"
        | "GulimChe" | "DotumChe" | "BatangChe" | "GungsuhChe" => {
            [(-10.2, 0.415), (23.9, 0.434), (-13.6, 0.423)]
        }
        "함초롬돋움" | "함초롬바탕" | "HCR Dotum" | "HCR Batang" => {
            [(-9.4, 0.386), (21.4, 0.394), (-12.4, 0.392)]
        }
        _ => [(-9.0, 0.378), (24.8, 0.425), (-11.5, 0.386)],
    }
}

/// 장치 단위 위치를 행·열 번호로 찾는 표
struct Axis {
    /// 그린 것: 번호 → (시작, 끝)
    pos: HashMap<u32, (f64, f64)>,
    order: Vec<u32>,
    dev: Vec<f64>,
}

impl Axis {
    fn new(list: &[(u32, f64, f64)], dev: &[f64]) -> Axis {
        Axis {
            pos: list.iter().map(|&(i, a, b)| (i, (a, b))).collect(),
            order: list.iter().map(|x| x.0).collect(),
            dev: dev.to_vec(),
        }
    }

    fn size(&self, i: u32) -> f64 {
        self.dev.get(i as usize).copied().unwrap_or(0.0)
    }

    /// i 의 시작 경계 — 그리지 않은 것은 가장 가까운 그린 것에서 크기를 더하거나 빼서
    fn start(&self, i: u32) -> Option<f64> {
        if let Some(p) = self.pos.get(&i) {
            return Some(p.0);
        }
        // 뒤쪽에 그린 것이 있으면 그 시작에서 뺀다
        if let Some(&j) = self.order.iter().filter(|&&j| j > i).min() {
            let gap: f64 = (i..j).map(|k| self.size(k)).sum();
            return Some(self.pos[&j].0 - gap);
        }
        let &j = self.order.iter().filter(|&&j| j < i).max()?;
        let gap: f64 = (j..i).map(|k| self.size(k)).sum();
        Some(self.pos[&j].0 + gap)
    }

    /// i..=e 의 (시작, 끝)
    fn span(&self, i: u32, e: u32) -> Option<(f64, f64)> {
        let a = self.start(i)?;
        let b = match self.pos.get(&e) {
            Some(p) => p.1,
            None => self.start(e)? + self.size(e),
        };
        Some((a, b))
    }
}

/// 오른쪽(`right`)·가운데 맞춤 글자의 시작 — 칸의 인쇄 시작에서 인쇄 px (글자 폭 실험, 배율 50–200·열 너비 1px 씩).
/// 화면 배치(배율 100)에서 오른쪽 = 너비 − 3 − 들여쓰기 − round(W), 가운데 = floor(너비/2) − floor(W/2) (W = 글꼴 advance ×
/// pt·4/3 의 합, px). 배율 100 이면 그대로, 아니면 round(x·z − 1.75) (가운데 1.5) — 배율에 따라 1px 어긋나는 것이 남는다
pub(crate) fn aligned_px(right: bool, cell_w: f64, text_w: f64, indent: f64, zoom: u32) -> f64 {
    let (x, k) = if right {
        (cell_w - 3.0 - indent - text_w.round(), 1.75)
    } else {
        ((cell_w / 2.0).floor() - (text_w / 2.0).floor(), 1.5)
    };
    if zoom == 100 {
        x
    } else {
        (x * zoom as f64 / 100.0 - k).round()
    }
}

/// 칸 상자가 경계에서 들어가는 거리 (장치 단위): 화면에서 칸의 마지막 픽셀이 격자선 자리인 것과 같다
const SHIFT: f64 = 6.0;

/// 선 굵기, 경계에서 선 가운데까지, 이중선, 점선 무늬 (장치 단위, 배율 99 의 한셀 PDF 테두리 실험 — 13 가지)
fn line_geom(style: &str) -> (f64, f64, bool, &'static [f64]) {
    match style {
        "medium" => (13.0, -SHIFT, false, &[]),
        "thick" => (19.0, 0.0, false, &[]),
        "double" => (6.0, -SHIFT, true, &[]),
        "hair" => (1.0, -SHIFT, false, &[]),
        // 한셀은 dashed 를 짧은 점, dotted 를 긴 점으로 그린다
        "dashed" => (6.0, -SHIFT, false, &[9.0, 13.0]),
        "dotted" => (6.0, -SHIFT, false, &[18.0, 10.0]),
        "dashDot" => (6.0, -SHIFT, false, &[64.0, 16.0, 18.0, 16.0]),
        "dashDotDot" => (6.0, -SHIFT, false, &[64.0, 16.0, 18.0, 16.0, 18.0, 16.0]),
        "mediumDashed" => (13.0, -SHIFT, false, &[37.0, 15.0]),
        "mediumDashDot" => (13.0, -SHIFT, false, &[73.0, 16.0, 18.0, 16.0]),
        "mediumDashDotDot" => (13.0, -SHIFT, false, &[73.0, 16.0, 18.0, 16.0, 18.0, 16.0]),
        "slantDashDot" => (13.0, -SHIFT, false, &[73.0, 16.0, 18.0, 16.0]),
        _ => (6.0, -SHIFT, false, &[]),
    }
}

/// 테두리 한 변을 선으로 (가로면 y 고정)
#[allow(clippy::too_many_arguments)]
fn edge_ops(
    ops: &mut Vec<Op>,
    e: &paint::Edge,
    horizontal: bool,
    at: f64,
    from: f64,
    to: f64,
    px: f64,
    mono: bool,
) {
    let (width, off, double, pattern) = line_geom(&e.style);
    let color = bw(rgb(&e.color).unwrap_or(BLACK), mono);
    // 무늬는 배율 99 에서 잰 길이 — 배율에 맞춘다
    let dash: Vec<f64> = pattern.iter().map(|v| v * px / 6.1875).collect();
    let mid = at + off;
    // 이중선: 가는 선 둘, 2px 떨어져
    let lines: Vec<f64> = if double {
        vec![mid - 6.0, mid + 6.0]
    } else {
        vec![mid]
    };
    // 선 끝: 칸 상자의 모서리 (경계 − 6) 까지
    let (a, b) = (from - SHIFT, to - SHIFT);
    for m in lines {
        let (x0, y0, x1, y1) = if horizontal {
            (a, m, b, m)
        } else {
            (m, a, m, b)
        };
        ops.push(Op::Line {
            x0,
            y0,
            x1,
            y1,
            width,
            color,
            dash: dash.clone(),
        });
    }
}

/// 줄 바꿈: 폭 `avail` 에 맞춰 한글은 아무 데서나, 영문은 단어 단위로
fn wrap_text(text: &str, avail: f64, w: &dyn Fn(&str) -> f64) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut word = String::new();
        let flush_word = |line: &mut String, word: &mut String, out: &mut Vec<String>| {
            if word.is_empty() {
                return;
            }
            let joined = format!("{line}{word}");
            if !line.is_empty() && w(joined.trim_end()) > avail {
                out.push(line.trim_end().to_string());
                line.clear();
            }
            // 단어 하나가 줄보다 길면 글자 단위로 자른다
            let mut rest = std::mem::take(word);
            while !rest.is_empty() && w(&format!("{line}{rest}")) > avail {
                let mut cut = String::new();
                for ch in rest.chars() {
                    let t = format!("{line}{cut}{ch}");
                    if w(&t) > avail && !(line.is_empty() && cut.is_empty()) {
                        break;
                    }
                    cut.push(ch);
                }
                if cut.is_empty() {
                    break;
                }
                line.push_str(&cut);
                out.push(std::mem::take(line).trim_end().to_string());
                rest = rest[cut.len()..].to_string();
            }
            line.push_str(&rest);
        };
        for ch in para.chars() {
            if ch == ' ' || super::metrics::is_wide(ch) {
                flush_word(&mut line, &mut word, &mut out);
                if ch == ' ' {
                    line.push(ch);
                } else {
                    let t = format!("{line}{ch}");
                    if !line.trim_end().is_empty() && w(t.trim_end()) > avail {
                        out.push(line.trim_end().to_string());
                        line.clear();
                    }
                    line.push(ch);
                }
            } else {
                word.push(ch);
            }
        }
        flush_word(&mut line, &mut word, &mut out);
        out.push(line.trim_end().to_string());
    }
    out
}

/// 인쇄 페이지 한 장의 칸들을 그린다
pub fn page_ops(
    wb: &Workbook,
    sheet: &Sheet,
    frame: &PageFrame,
    measure: &dyn TextMeasure,
) -> Vec<Op> {
    let max_r = frame.rows.iter().map(|r| r.0).max().unwrap_or(0);
    let max_c = frame.cols.iter().map(|c| c.0).max().unwrap_or(0);
    SheetPainter::new(wb, sheet, max_r, max_c).page_ops(frame, measure)
}

/// 시트 하나의 인쇄 페이지들을 그린다 (배치·조건부 서식 결과를 페이지마다 다시 짓지 않게 한 번만)
pub struct SheetPainter<'a> {
    wb: &'a Workbook,
    sheet: &'a Sheet,
    ctx: Views<'a>,
    lay_c: u32,
}

impl<'a> SheetPainter<'a> {
    /// 행 0..=max_r, 열 0..=max_c 를 그릴 수 있는 배치 (병합·넘치는 글자 자리까지 넓힌다)
    pub fn new(wb: &'a Workbook, sheet: &'a Sheet, max_r: u32, max_c: u32) -> SheetPainter<'a> {
        let mut lay_r = max_r + 1;
        let mut lay_c = max_c + 1;
        for m in &sheet.merges {
            lay_r = lay_r.max(m.end.row + 1);
            lay_c = lay_c.max(m.end.col + 1);
        }
        let lay_c = (lay_c + 32).min(16_384);
        SheetPainter {
            wb,
            sheet,
            ctx: Views::new(wb, sheet, lay_r, lay_c),
            lay_c,
        }
    }

    /// 페이지 한 장의 칸들
    pub fn page_ops(&self, frame: &PageFrame, measure: &dyn TextMeasure) -> Vec<Op> {
        let (wb, sheet, ctx, lay_c) = (self.wb, self.sheet, &self.ctx, self.lay_c);
        page_body(wb, sheet, ctx, lay_c, frame, measure)
    }
}

fn page_body(
    wb: &Workbook,
    sheet: &Sheet,
    ctx: &Views,
    lay_c: u32,
    frame: &PageFrame,
    measure: &dyn TextMeasure,
) -> Vec<Op> {
    let mut ops = Vec::new();
    if frame.rows.is_empty() || frame.cols.is_empty() {
        return ops;
    }
    let mono = frame.black_and_white;
    let lay = &ctx.lay;
    let rows_set: HashSet<u32> = frame.rows.iter().map(|r| r.0).collect();
    let cols_set: HashSet<u32> = frame.cols.iter().map(|c| c.0).collect();
    let ax = Axis::new(&frame.cols, &frame.col_dev);
    let ay = Axis::new(&frame.rows, &frame.row_dev);

    // 페이지에 찍히는 영역 (자르기 상자)
    let area = (
        frame.cols.iter().map(|c| c.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.rows.iter().map(|r| r.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.cols.iter().map(|c| c.2).fold(f64::MIN, f64::max) - SHIFT,
        frame.rows.iter().map(|r| r.2).fold(f64::MIN, f64::max) - SHIFT,
    );

    // 칸 내용: 그리는 행의 모든 셀 (넘침이 이웃 값에서 멈추게) + 그리는 칸의 행·열 서식
    let mut views: HashMap<(u32, u32), CellView> = HashMap::new();
    for c in &sheet.cells {
        if rows_set.contains(&c.r.row)
            && c.r.col < lay_c
            && let Some(v) = ctx.view(c.r.row, c.r.col)
        {
            views.insert((c.r.row, c.r.col), v);
        }
    }
    for &(r, _, _) in &frame.rows {
        for &(c, _, _) in &frame.cols {
            if !views.contains_key(&(r, c))
                && let Some(v) = ctx.view(r, c)
            {
                views.insert((r, c), v);
            }
        }
    }
    // 병합 시작 칸이 페이지 밖이어도 병합이 페이지에 걸치면 그린다
    let mut merge_starts: Vec<(u32, u32)> = Vec::new();
    for m in &sheet.merges {
        let on_rows = (m.start.row..=m.end.row).any(|r| rows_set.contains(&r));
        let on_cols = (m.start.col..=m.end.col).any(|c| cols_set.contains(&c));
        if on_rows && on_cols {
            merge_starts.push((m.start.row, m.start.col));
            if !views.contains_key(&(m.start.row, m.start.col))
                && let Some(v) = ctx.view(m.start.row, m.start.col)
            {
                views.insert((m.start.row, m.start.col), v);
            }
        }
    }
    let blocked = |r: u32, c: u32| -> bool {
        lay.covered.contains(&(r, c))
            || lay.span.contains_key(&(r, c))
            || views.get(&(r, c)).is_some_and(|v| v.has_value())
            || ctx.cells.get(&(r, c)).is_some_and(|x| x.formula.is_some())
    };
    let ov = overflow_spans(lay, &views, &blocked, lay_c);

    // 그릴 칸: 페이지의 (행, 열) 중 병합에 덮이지 않은 것 + 페이지에 걸친 병합의 시작 칸
    let mut draw: Vec<(u32, u32)> = Vec::new();
    for &(r, _, _) in &frame.rows {
        for &(c, _, _) in &frame.cols {
            if !lay.covered.contains(&(r, c)) {
                draw.push((r, c));
            }
        }
    }
    // 페이지 밖 칸에서 넘쳐 들어오는 글자 (한셀은 넘친 글자를 옆 페이지에 이어 찍는다)
    let mut spill: Vec<(u32, u32)> = Vec::new();
    for (&(r, c), &(ol, or)) in &ov.span {
        if !rows_set.contains(&r) || cols_set.contains(&c) {
            continue;
        }
        let reach = |forward: bool, want: u32| -> bool {
            let (mut acc, mut k) = (0, c);
            while acc < want {
                k = match (forward, k.checked_sub(1)) {
                    (true, _) if k + 1 < lay_c => k + 1,
                    (false, Some(p)) => p,
                    _ => break,
                };
                if cols_set.contains(&k) {
                    return true;
                }
                acc += lay.col_w(k);
            }
            false
        };
        if reach(true, or) || reach(false, ol) {
            spill.push((r, c));
        }
    }
    spill.sort_unstable();
    for m in merge_starts {
        if !draw.contains(&m) {
            draw.push(m);
        }
    }
    // 칸 상자 (경계, 장치 단위) — 병합이면 병합 전체
    let cell_rect = |r: u32, c: u32| -> Option<(f64, f64, f64, f64)> {
        let (er, ec) = lay.merge_end.get(&(r, c)).copied().unwrap_or((r, c));
        let (x0, x1) = ax.span(c, ec)?;
        let (y0, y1) = ay.span(r, er)?;
        Some((x0, y0, x1, y1))
    };

    ops.push(Op::Clip {
        x0: area.0,
        y0: area.1,
        x1: area.2,
        y1: area.3,
    });
    // 1. 채우기 (초안 인쇄·흑백 인쇄는 채우지 않는다)
    if !frame.draft && !mono {
        for &(r, c) in &draw {
            let Some(v) = views.get(&(r, c)) else {
                continue;
            };
            let Some(color) = v
                .fill
                .as_ref()
                .and_then(|f| f.color.as_deref())
                .and_then(rgb)
            else {
                continue;
            };
            let Some((x0, y0, x1, y1)) = cell_rect(r, c) else {
                continue;
            };
            ops.push(Op::Rect {
                x0: x0 - SHIFT,
                y0: y0 - SHIFT,
                x1: x1 - SHIFT,
                y1: y1 - SHIFT,
                color,
            });
        }
    }
    // 2. 격자선 (인쇄 옵션): 그린 행·열의 모든 경계
    if frame.gridlines {
        let (l, t) = (
            frame.cols.first().map_or(0.0, |c| c.1),
            frame.rows.first().map_or(0.0, |r| r.1),
        );
        let (rr, bb) = (
            frame.cols.last().map_or(0.0, |c| c.2),
            frame.rows.last().map_or(0.0, |r| r.2),
        );
        let mut xs: Vec<f64> = frame.cols.iter().map(|c| c.1).collect();
        xs.push(rr);
        let mut ys: Vec<f64> = frame.rows.iter().map(|r| r.1).collect();
        ys.push(bb);
        // 한셀 PDF 의 인쇄 격자선은 장치 1 굵기 (0.12pt)
        for x in xs {
            ops.push(Op::Line {
                x0: x - SHIFT,
                y0: t - SHIFT,
                x1: x - SHIFT,
                y1: bb - SHIFT,
                width: 1.0,
                color: BLACK,
                dash: Vec::new(),
            });
        }
        for y in ys {
            ops.push(Op::Line {
                x0: l - SHIFT,
                y0: y - SHIFT,
                x1: rr - SHIFT,
                y1: y - SHIFT,
                width: 1.0,
                color: BLACK,
                dash: Vec::new(),
            });
        }
    }
    // 3. 글자
    let theme = &ctx.theme;
    for &(r, c) in draw.iter().chain(&spill) {
        let Some(v) = views.get(&(r, c)) else {
            continue;
        };
        let mut text = v.text.clone();
        if let CellValue::Error(e) = &v.value {
            text = match frame.errors {
                ErrorPrint::Displayed => e.clone(),
                ErrorPrint::Blank => String::new(),
                ErrorPrint::Dash => "--".into(),
                ErrorPrint::Na => "#N/A".into(),
            };
        }
        let cf = ctx.cfx.get(&(r, c));
        if cf.is_some_and(|c| c.hide_value) || text.is_empty() {
            continue;
        }
        let Some((x0, y0, x1, y1)) = cell_rect(r, c) else {
            continue;
        };
        let f = v.font;
        let name = f
            .and_then(|f| f.name.clone())
            .unwrap_or_else(|| super::metrics::DEFAULT_FONT.to_string());
        let pt = f
            .and_then(|f| f.size)
            .unwrap_or(super::metrics::DEFAULT_SIZE);
        let bold = cf.and_then(|c| c.bold).unwrap_or(f.is_some_and(|f| f.bold));
        let italic = cf
            .and_then(|c| c.italic)
            .unwrap_or(f.is_some_and(|f| f.italic));
        let key = FontKey {
            name: name.clone(),
            bold,
            italic,
        };
        let color = cf
            .and_then(|c| c.font_color.as_ref())
            .and_then(|c| theme::resolve(c, theme))
            .or_else(|| v.code_color.clone())
            .or_else(|| {
                f.and_then(|f| f.color.as_ref())
                    .and_then(|c| theme::resolve(c, theme))
            })
            .and_then(|c| rgb(&c))
            .unwrap_or(BLACK);
        let color = bw(color, mono);
        let underline = cf
            .and_then(|c| c.underline)
            .unwrap_or(f.is_some_and(|f| f.underline.as_deref().is_some_and(|u| u != "none")));
        let strike = cf
            .and_then(|c| c.strike)
            .unwrap_or(f.is_some_and(|f| f.strike || f.double_strike));
        let stretch = super::cell::ratio_of(f).unwrap_or(1.0);
        let mut size = frame.font_size(pt);
        let px = frame.px;
        let pad_l = (px + 0.5).floor();
        // 왼쪽 맞춤 글자 자리: 열의 인쇄 왼쪽 경계 + pad px (한셀 PDF 열 너비 3.3·5·8.43 글자 × 배율 50–200 실험)
        let left_at = |col: u32, extra: f64| -> Option<f64> {
            // 열의 인쇄 왼쪽 경계에서 pad 만큼 (배율 100 = 키 큰 모드 또는 1:1 은 2px 그대로)
            let b = ax.start(col)?;
            let pad = if frame.zoom == 100 {
                2.0 + extra
            } else {
                // 반은 내림 (75% 의 1.5 → 1): 배율 50·75·99·125·150·200 의 첫 열 자리 0·0·1·1·2·3px
                ((2.0 + extra) * frame.zoom as f64 / 100.0 - 0.5).ceil() - 1.0
            };
            Some((b + pad * 6.25).round())
        };
        let indent = v.indent as f64 * INDENT_PX * px;
        let w = |s: &str, sz: f64| measure.width(&key, sz, s) * stretch;
        // 오른쪽·가운데 맞춤: 화면 글자 폭 (px) 으로 [`aligned_px`]
        let scr_w = |s: &str| measure.width(&key, pt * 4.0 / 3.0, s) * stretch;
        let ec = lay.merge_end.get(&(r, c)).map_or(c, |m| m.1);
        let cell_w100 = cols_set
            .contains(&c)
            .then(|| (c..=ec).map(|k| lay.col_w(k) as f64).sum::<f64>());
        let lp = ((x0 - frame.origin_x) / 6.25).round();
        let to_dev = |xp: f64| (frame.origin_x + (lp + xp) * 6.25).round();
        // 넘친 범위 (화면 px → 이어진 열의 인쇄 자리)
        let (ol, or) = ov.span.get(&(r, c)).copied().unwrap_or((0, 0));
        let mut bx0 = x0;
        let mut bx1 = x1;
        if or > 0 {
            let mut acc = 0;
            let mut k = lay.merge_end.get(&(r, c)).map_or(c, |m| m.1);
            while acc < or && k + 1 < lay_c {
                k += 1;
                acc += lay.col_w(k);
            }
            if let Some((_, e)) = ax.span(c, k) {
                bx1 = e;
            }
        }
        if ol > 0 {
            let mut acc = 0;
            let mut k = c;
            while acc < ol && k > 0 {
                k -= 1;
                acc += lay.col_w(k);
            }
            if let Some(s) = ax.start(k) {
                bx0 = s;
            }
        }
        // 줄여 맞춤
        let avail = x1 - x0 - pad_l - 4.2 * px - indent;
        if v.shrink {
            let tw = w(&text, size);
            if tw > avail && tw > 0.0 {
                size = (size * avail / tw).max(1.0);
            }
        }
        let l_pt = v.m.row_pt;
        let line = l_pt * 25.0 / 3.0 * frame.zoom as f64 / 100.0;
        let k99 = frame.zoom as f64 / 99.0;
        let [(ct, kt), (cb, kb), (cm, km)] = v_consts(&name);
        let lines: Vec<String> = if v.wrap {
            wrap_text(&text, avail.max(px), &|s| w(s, size))
        } else {
            vec![text.clone()]
        };
        let n = lines.len() as f64;
        // 첫 줄 기준선
        let top = y0 + line / 2.0 + kt * size + ct * k99;
        // 화면에서 글이 행에 들어가면 (행 px ≥ 줄 수 × 줄 높이 px) 아래·가운데 맞춤도 위 맞춤 자리보다 올라가지 않는다
        // (22px 행의 숫자, 배율 97). 들어가지 않으면 아래에 붙인 채 위로 넘친다 (행 6–16px 의 숫자 — 행 4.5–12pt 실험)
        let er = lay.merge_end.get(&(r, c)).map_or(r, |m| m.0);
        let screen_h: u32 = (r..=er).map(|k| lay.row_h(k)).sum();
        let fits = screen_h as f64 + 0.5 >= n * l_pt * 4.0 / 3.0;
        let floor = if fits { top } else { f64::MIN };
        let first = match v.v {
            VAlign::Top => top,
            VAlign::Bottom => {
                (y1 - line / 2.0 + kb * size - cb * k99 - (n - 1.0) * line).max(floor)
            }
            VAlign::Center => {
                ((y0 + y1) / 2.0 + km * size + cm * k99 - (n - 1.0) * line / 2.0).max(floor)
            }
        };
        let need_clip = !(v.wrap || ol > 0 || or > 0);
        // 페이지 안 칸의 넘친 글자는 페이지 영역 밖(여백)까지 넘친 범위만큼 찍힌다 — 한셀: B1 의 긴 글자가 1 페이지
        // 오른쪽 여백을 지나 종이 끝까지. 옆 페이지로 넘쳐 들어온 글자는 페이지 영역에서 자른다
        let escape =
            !need_clip && cols_set.contains(&c) && (bx1 - SHIFT > area.2 || bx0 - SHIFT < area.0);
        if escape {
            ops.push(Op::Unclip);
        }
        if need_clip || escape {
            ops.push(Op::Clip {
                x0: bx0 - SHIFT,
                y0: y0 - SHIFT,
                x1: bx1 - SHIFT,
                y1: y1 - SHIFT,
            });
        }
        for (i, ln) in lines.iter().enumerate() {
            if ln.is_empty() {
                continue;
            }
            let tw = w(ln, size);
            let x = match (v.h, cell_w100) {
                (HAlign::Right | HAlign::Center, Some(cw)) => to_dev(aligned_px(
                    v.h == HAlign::Right,
                    cw,
                    scr_w(ln),
                    v.indent as f64 * INDENT_PX,
                    frame.zoom,
                )),
                (HAlign::Right, _) => bx1 - 4.2 * px - indent - tw,
                (HAlign::Center, _) => (bx0 + bx1) / 2.0 - 1.3 * px - tw / 2.0,
                (HAlign::CenterContinuous, _) => (x0 + bx1) / 2.0 - 1.3 * px - tw / 2.0,
                _ => {
                    let start = if ol > 0 { None } else { Some(c) };
                    start
                        .and_then(|col| left_at(col, v.indent as f64 * INDENT_PX))
                        .unwrap_or(bx0 + pad_l + indent)
                }
            };
            ops.push(Op::Text {
                x,
                y: first + i as f64 * line,
                size,
                font: key.clone(),
                color,
                text: ln.clone(),
                underline,
                strike,
                stretch,
                angle: 0.0,
            });
        }
        if need_clip || escape {
            ops.push(Op::Unclip);
        }
        if escape {
            ops.push(Op::Clip {
                x0: area.0,
                y0: area.1,
                x1: area.2,
                y1: area.3,
            });
        }
    }
    // 4. 테두리 (병합은 가장자리 칸들의 테두리, 넘친 글자가 지나가는 세로 경계는 지운다)
    let style_at = |rr: u32, cc: u32| {
        ctx.cells
            .get(&(rr, cc))
            .map(|x| x.style)
            .unwrap_or_else(|| sheet.effective_style(rr, cc))
    };
    for &(r, c) in &draw {
        let Some((x0, y0, x1, y1)) = cell_rect(r, c) else {
            continue;
        };
        let mut b = match lay.merge_end.get(&(r, c)) {
            Some(&(r1, c1)) => {
                let at = |rr, cc| super::borders_of(wb.styles.xf(style_at(rr, cc)), wb, theme);
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
        let ec = lay.merge_end.get(&(r, c)).map_or(c, |m| m.1);
        if ov.no_vline.contains(&(r, ec)) {
            b.right = None;
        }
        if c > 0 && ov.no_vline.contains(&(r, c - 1)) {
            b.left = None;
        }
        let px = frame.px;
        if let Some(e) = &b.top {
            edge_ops(&mut ops, e, true, y0, x0, x1, px, mono);
        }
        if let Some(e) = &b.bottom {
            edge_ops(&mut ops, e, true, y1, x0, x1, px, mono);
        }
        if let Some(e) = &b.left {
            edge_ops(&mut ops, e, false, x0, y0, y1, px, mono);
        }
        if let Some(e) = &b.right {
            edge_ops(&mut ops, e, false, x1, y0, y1, px, mono);
        }
        for (e, up) in [(&b.down, false), (&b.up, true)] {
            let Some(e) = e else { continue };
            let (width, _, _, _) = line_geom(&e.style);
            let (ya, yb) = if up {
                (y1 - SHIFT, y0 - SHIFT)
            } else {
                (y0 - SHIFT, y1 - SHIFT)
            };
            ops.push(Op::Line {
                x0: x0 - SHIFT,
                y0: ya,
                x1: x1 - SHIFT,
                y1: yb,
                width,
                color: bw(rgb(&e.color).unwrap_or(BLACK), mono),
                dash: Vec::new(),
            });
        }
    }
    ops.push(Op::Unclip);
    ops
}

// ---------------------------------------------------------------------------
// 그리기 개체 (도형·글상자·그림·차트·그룹)
// ---------------------------------------------------------------------------

/// EMU → 화면 px
const EMU: f64 = crate::units::EMU_PER_PX as f64;

/// `html` 안의 맨 바깥 `<svg …>…</svg>` 조각들
fn svg_blocks(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = html[i..].find("<svg") {
        let start = i + off;
        let mut depth = 0;
        let mut j = start;
        let mut end = None;
        while j < html.len() {
            if html[j..].starts_with("<svg") {
                depth += 1;
                j += 4;
            } else if html[j..].starts_with("</svg>") {
                depth -= 1;
                j += 6;
                if depth == 0 {
                    end = Some(j);
                    break;
                }
            } else {
                j += html[j..].chars().next().map_or(1, char::len_utf8);
            }
        }
        let Some(e) = end else { break };
        out.push(&html[start..e]);
        i = e;
    }
    out
}

/// 상자 (x, y, w, h — 장치 단위)를 도는 변환: 가운데를 중심으로 회전·뒤집기, 그다음 px → 장치 단위 (k 배)
#[allow(clippy::too_many_arguments)]
fn box_matrix(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    k: f64,
    rot: f64,
    flip_h: bool,
    flip_v: bool,
) -> [f64; 6] {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (s, c) = rot.to_radians().sin_cos();
    let fx = if flip_h { -1.0 } else { 1.0 };
    let fy = if flip_v { -1.0 } else { 1.0 };
    // p(px) → (p·k − 반 크기)·뒤집기 → 회전 → + 가운데
    let (a, b, cc, d) = (c * fx * k, s * fx * k, -s * fy * k, c * fy * k);
    let (ox, oy) = (-w / 2.0 * fx, -h / 2.0 * fy);
    let (ox, oy) = (c * ox - s * oy, s * ox + c * oy);
    [a, b, cc, d, cx + ox, cy + oy]
}

/// 도형 글 조각
struct TRun {
    text: String,
    key: FontKey,
    size_pt: f64,
    color: [u8; 3],
    under: bool,
}

/// 도형 글 문단 (간격은 pt)
struct TPara {
    align: String,
    size_pt: f64,
    /// 줄 높이 (pt). None = 1.2 × 크기
    line_pt: Option<f64>,
    before_pt: f64,
    after_pt: f64,
    /// 왼쪽 들여쓰기 (화면 px)
    indent_px: f64,
    /// 첫 줄을 올리는 거리 (pt) — 화면 규칙과 같게 함초롬 글꼴의 첫 문단 0.12 em
    lift_pt: f64,
    runs: Vec<TRun>,
}

/// 도형 글 전체
struct TBody {
    anchor: String,
    wrap: bool,
    /// 안쪽 여백 왼·위·오른·아래 (EMU)
    insets: [f64; 4],
    paras: Vec<TPara>,
}

/// 모델의 글상자 글 (원본 XML 이 없을 때)
fn model_body(t: &crate::model::TextBody, theme: &crate::theme::Theme) -> TBody {
    let mut paras = Vec::new();
    for p in &t.paragraphs {
        let size_pt = p
            .runs
            .iter()
            .filter_map(|r| r.size)
            .max()
            .map_or(11.0, |z| z as f64 / 100.0);
        let runs = p
            .runs
            .iter()
            .map(|run| TRun {
                text: run.text.clone(),
                key: FontKey {
                    name: run
                        .ea
                        .as_deref()
                        .or(run.latin.as_deref())
                        .and_then(|n| super::draw::theme_font(n, theme))
                        .unwrap_or("맑은 고딕")
                        .to_string(),
                    bold: run.bold.unwrap_or(false),
                    italic: run.italic.unwrap_or(false),
                },
                size_pt: run.size.map_or(size_pt, |z| z as f64 / 100.0),
                color: run
                    .color
                    .as_ref()
                    .and_then(|c| super::draw::draw_color(c, theme))
                    .and_then(|c| rgb(&c))
                    .unwrap_or(BLACK),
                under: run.underline.as_deref().is_some_and(|u| u != "none"),
            })
            .collect();
        paras.push(TPara {
            align: p.align.clone().unwrap_or_else(|| "l".into()),
            size_pt,
            line_pt: None,
            before_pt: 0.0,
            after_pt: 0.0,
            indent_px: 0.0,
            lift_pt: 0.0,
            runs,
        });
    }
    TBody {
        anchor: t.anchor.clone().unwrap_or_else(|| "t".into()),
        wrap: t.wrap.as_deref() != Some("none"),
        insets: [
            t.insets[0].unwrap_or(91440) as f64,
            t.insets[1].unwrap_or(45720) as f64,
            t.insets[2].unwrap_or(91440) as f64,
            t.insets[3].unwrap_or(45720) as f64,
        ],
        paras,
    }
}

/// 원본 txBody (화면 렌더러 [`super::draw`] 의 txbody_html 과 같은 규칙: 빈 문단, 줄·문단 간격, 글꼴 줄이기)
fn raw_body(tx: &crate::xml::Element, theme: &crate::theme::Theme) -> TBody {
    use super::shape::A;
    let body = tx.child_ns(A, "bodyPr");
    let ins = |n: &str, d: f64| body.and_then(|b| b.attr_f64(n)).unwrap_or(d);
    let scale = body
        .and_then(|b| b.child_ns(A, "normAutofit"))
        .and_then(|a| a.attr_f64("fontScale"))
        .map_or(1.0, |v| v / 100_000.0);
    let lvl1 = tx
        .child_ns(A, "lstStyle")
        .and_then(|l| l.child_ns(A, "lvl1pPr"))
        .and_then(|p| p.child_ns(A, "defRPr"));
    let mut paras = Vec::new();
    let mut first = true;
    for p in tx.elements().filter(|e| e.is(A, "p")) {
        let ppr = p.child_ns(A, "pPr");
        let mut size: Option<f64> = None;
        for r in p
            .elements()
            .filter(|e| e.is(A, "r") || e.is(A, "endParaRPr"))
        {
            let rp = if r.is(A, "r") {
                r.child_ns(A, "rPr")
            } else {
                Some(r)
            };
            if let Some(z) = rp.and_then(|x| x.attr_f64("sz")) {
                size = Some(size.map_or(z / 100.0, |s: f64| s.max(z / 100.0)));
            }
        }
        let size = size.or_else(|| lvl1.and_then(|d| d.attr_f64("sz")).map(|v| v / 100.0));
        let has_text = p
            .elements()
            .any(|e| e.is(A, "r") || e.is(A, "fld") || e.is(A, "br"));
        if size.is_none() && !has_text {
            continue;
        }
        let size = size.unwrap_or(11.0) * scale;
        let lnspc = ppr.and_then(|x| x.child_ns(A, "lnSpc"));
        let line_pt = match lnspc.and_then(|l| l.child_ns(A, "spcPct")) {
            Some(pct) => Some(1.2 * pct.attr_f64("val").unwrap_or(100_000.0) / 100_000.0 * size),
            None => lnspc
                .and_then(|l| l.child_ns(A, "spcPts"))
                .map(|pts| pts.attr_f64("val").unwrap_or(0.0) / 100.0),
        };
        let space = |n: &str| -> f64 {
            let Some(sp) = ppr.and_then(|x| x.child_ns(A, n)) else {
                return 0.0;
            };
            if let Some(p) = sp.child_ns(A, "spcPts") {
                p.attr_f64("val").unwrap_or(0.0) / 100.0
            } else if let Some(p) = sp.child_ns(A, "spcPct") {
                p.attr_f64("val").unwrap_or(0.0) / 100_000.0 * size * 1.2
            } else {
                0.0
            }
        };
        let face_of = |rp: Option<&crate::xml::Element>| {
            rp.and_then(|r| r.child_ns(A, "ea").or_else(|| r.child_ns(A, "latin")))
                .and_then(|f| f.attr("typeface"))
                .and_then(|f| super::draw::theme_font(f, theme))
                .map(str::to_string)
        };
        let para_face = face_of(
            p.elements()
                .find(|e| e.is(A, "r"))
                .and_then(|r| r.child_ns(A, "rPr"))
                .or(lvl1),
        );
        let lift = if first
            && para_face
                .as_deref()
                .is_some_and(|f| f.starts_with("함초롬"))
        {
            0.12 * size
        } else {
            0.0
        };
        first = false;
        let mut runs = Vec::new();
        if let Some(bu) = ppr.and_then(|x| x.child_ns(A, "buChar")) {
            runs.push(TRun {
                text: format!("{} ", bu.attr("char").unwrap_or("•")),
                key: FontKey {
                    name: para_face.clone().unwrap_or_else(|| "맑은 고딕".into()),
                    bold: false,
                    italic: false,
                },
                size_pt: size,
                color: BLACK,
                under: false,
            });
        }
        for r in p.elements() {
            if r.is(A, "br") {
                if let Some(last) = runs.last_mut() {
                    last.text.push('\n');
                }
                continue;
            }
            if !(r.is(A, "r") || r.is(A, "fld")) {
                continue;
            }
            let rp = r.child_ns(A, "rPr").or(lvl1);
            let text = r.child_ns(A, "t").map(|t| t.text()).unwrap_or_default();
            let color = rp
                .and_then(|x| x.child_ns(A, "solidFill"))
                .and_then(|f| f.elements().next())
                .map(crate::parser::drawing::color)
                .and_then(|c| super::draw::draw_color(&c, theme))
                .and_then(|c| rgb(&c))
                .unwrap_or(BLACK);
            runs.push(TRun {
                text,
                key: FontKey {
                    name: face_of(rp)
                        .or_else(|| para_face.clone())
                        .unwrap_or_else(|| "맑은 고딕".into()),
                    bold: rp.and_then(|x| x.attr_bool("b")).unwrap_or(false),
                    italic: rp.and_then(|x| x.attr_bool("i")).unwrap_or(false),
                },
                size_pt: rp
                    .and_then(|x| x.attr_f64("sz"))
                    .map_or(size, |z| z / 100.0 * scale),
                color,
                under: rp.and_then(|x| x.attr("u")).is_some_and(|u| u != "none"),
            });
        }
        paras.push(TPara {
            align: ppr.and_then(|x| x.attr("algn")).unwrap_or("l").to_string(),
            size_pt: size,
            line_pt,
            before_pt: space("spcBef"),
            after_pt: space("spcAft"),
            indent_px: ppr
                .and_then(|x| x.attr_f64("marL"))
                .map_or(0.0, |v| v / EMU),
            lift_pt: lift,
            runs,
        });
    }
    TBody {
        anchor: body
            .and_then(|b| b.attr("anchor"))
            .unwrap_or("t")
            .to_string(),
        wrap: body.and_then(|b| b.attr("wrap")) != Some("none"),
        insets: [
            ins("lIns", 91440.0),
            ins("tIns", 45720.0),
            ins("rIns", 91440.0),
            ins("bIns", 45720.0),
        ],
        paras,
    }
}

/// 도형 글을 상자 (x, y, w, h — 장치 단위) 안에: 안쪽 여백, 줄 바꿈, 가로·세로 맞춤
#[allow(clippy::too_many_arguments)]
fn shape_text(
    t: &TBody,
    (x, y, w, h): (f64, f64, f64, f64),
    k: f64,
    zoom: f64,
    measure: &dyn TextMeasure,
    mono: bool,
    ops: &mut Vec<Op>,
) {
    let dev_pt = 25.0 / 3.0 * zoom;
    let ins = |v: f64| v / EMU * k;
    let (bx0, by0, bx1, by1) = (
        x + ins(t.insets[0]),
        y + ins(t.insets[1]),
        x + w - ins(t.insets[2]),
        y + h - ins(t.insets[3]),
    );
    // (줄 조각들, 줄 높이, 맞춤, 들여쓰기, 줄 위 여백)
    type Piece = (String, FontKey, f64, [u8; 3], bool);
    let mut lines: Vec<(Vec<Piece>, f64, String, f64, f64)> = Vec::new();
    for (pi, p) in t.paras.iter().enumerate() {
        let lh = p.line_pt.unwrap_or(1.2 * p.size_pt) * dev_pt;
        let indent = p.indent_px * k;
        let avail = (bx1 - bx0 - indent).max(k);
        let mut before = p.before_pt * dev_pt - p.lift_pt * dev_pt;
        if pi > 0 {
            before += t.paras[pi - 1].after_pt * dev_pt;
        }
        let mut cur: Vec<Piece> = Vec::new();
        let mut used = 0.0;
        let push_line =
            |cur: &mut Vec<Piece>,
             before: &mut f64,
             lines: &mut Vec<(Vec<Piece>, f64, String, f64, f64)>| {
                lines.push((std::mem::take(cur), lh, p.align.clone(), indent, *before));
                *before = 0.0;
            };
        for run in &p.runs {
            let size = (run.size_pt * dev_pt).max(1.0);
            let color = bw(run.color, mono);
            // 낱말(끝 공백 포함)·전각 글자 단위
            let mut tokens: Vec<String> = Vec::new();
            for ch in run.text.chars() {
                if ch == '\n' || ch == '\u{b}' {
                    tokens.push("\n".into());
                    continue;
                }
                let wide = super::metrics::is_wide(ch);
                match tokens.last_mut() {
                    Some(tk)
                        if !wide
                            && tk != "\n"
                            && !tk.ends_with(' ')
                            && !tk.chars().any(super::metrics::is_wide) =>
                    {
                        tk.push(ch)
                    }
                    _ => tokens.push(ch.to_string()),
                }
            }
            for tok in tokens {
                if tok == "\n" {
                    push_line(&mut cur, &mut before, &mut lines);
                    used = 0.0;
                    continue;
                }
                let tw = measure.width(&run.key, size, &tok);
                if t.wrap && used + tw > avail && used > 0.0 {
                    push_line(&mut cur, &mut before, &mut lines);
                    used = 0.0;
                    if tok.trim().is_empty() {
                        continue;
                    }
                }
                used += tw;
                match cur.last_mut() {
                    Some(last)
                        if last.1 == run.key
                            && last.2 == size
                            && last.3 == color
                            && last.4 == run.under =>
                    {
                        last.0.push_str(&tok)
                    }
                    _ => cur.push((tok, run.key.clone(), size, color, run.under)),
                }
            }
        }
        push_line(&mut cur, &mut before, &mut lines);
    }
    let total: f64 = lines.iter().map(|l| l.1 + l.4).sum();
    let mut ly = match t.anchor.as_str() {
        "ctr" => (by0 + by1) / 2.0 - total / 2.0,
        "b" => by1 - total,
        _ => by0,
    };
    for (pieces, lh, align, indent, before) in lines {
        ly += before;
        let widths: Vec<f64> = pieces
            .iter()
            .map(|(s, key, size, _, _)| measure.width(key, *size, s.trim_end()))
            .collect();
        let tw: f64 = widths.iter().sum();
        let mut lx = match align.as_str() {
            "ctr" => (bx0 + indent + bx1) / 2.0 - tw / 2.0,
            "r" => bx1 - tw,
            _ => bx0 + indent,
        };
        // 기준선: 줄 상자(1.2 em)의 반 줄 간격 + 오름 ≈ 위에서 1.02 em (맑은 고딕·함초롬)
        let size_max = pieces.iter().map(|p| p.2).fold(0.0, f64::max);
        let base = ly + (lh - 1.2 * size_max) / 2.0 + 1.02 * size_max;
        for ((s, key, size, color, under), w) in pieces.into_iter().zip(widths) {
            ops.push(Op::Text {
                x: lx,
                y: base,
                size,
                font: key,
                color,
                text: s,
                underline: under,
                strike: false,
                stretch: 1.0,
                angle: 0.0,
            });
            lx += w;
        }
        ly += lh;
    }
}

/// 개체 종류 — 한셀 PDF 에서 자리 규칙이 다르다
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObjKind {
    Picture,
    Shape,
    /// 차트 (한셀은 비트맵으로 찍는다)
    Chart,
}

/// 개체의 왼쪽(위) 자리 (장치 단위). 한셀 PDF 그림·도형 실험 (배율 60–150, 개체 14 개, 장치 0.5 안에 모두 같다): 시트의 절대
/// 화면 px 에 배율을 곱한 자리 `c = 원점 + px × k` (k = 6.25 × z/100) 에서 그림은 floor(c + 0.25) − 8, 도형은
/// floor(c − 8.5) + 0.5. 칸 경계(배율에 맞춰 반올림한 행·열)와 상관없다. (앵커 오프셋이 1 EMU 모자라 정수 경계에서
/// 틀리지 않게 0.001 을 더한다). 차트 비트맵은 개체 상자보다 반 px 씩 넓다: round(c − 8.5 − k/2) (배율 60·75·99·150,
/// 앵커 둘에서 장치 1.5 안)
pub(crate) fn object_origin(origin: f64, px: f64, k: f64, kind: ObjKind) -> f64 {
    let c = origin + px * k + 0.001;
    match kind {
        ObjKind::Picture => (c + 0.25).floor() - 8.0,
        ObjKind::Shape => (c - 8.5).floor() + 0.5,
        ObjKind::Chart => (c - 8.5 - k / 2.0).round(),
    }
}

/// 개체의 크기 (장치 단위): ceil(px 크기 × k). 차트는 round((px 크기 + 1) × k + 1)
pub(crate) fn object_size(px: f64, k: f64, kind: ObjKind) -> f64 {
    match kind {
        ObjKind::Chart => ((px + 1.0) * k + 1.0).round(),
        _ => (px * k - 0.001).ceil(),
    }
}

/// 메타파일(EMF·WMF)을 그림 상자 `m` (그림 px → 장치, [`box_matrix`]) 에 그린다. 틀 전체가 상자에 늘어난다.
/// 펜 굵기와 점선은 가로 배율, 글자는 가로·세로 배율을 따로 (한셀 PDF 와 같다)
fn metafile_ops(
    mf: &super::metafile::Metafile,
    m: [f64; 6],
    (wp, hp): (f64, f64),
    rot: f64,
    measure: &dyn TextMeasure,
    ops: &mut Vec<Op>,
) {
    use super::metafile::Item;
    if mf.width <= 0.0 || mf.height <= 0.0 {
        return;
    }
    let (ax, ay) = (wp / mf.width, hp / mf.height);
    let f = |u: f64, v: f64| {
        let (u, v) = (u * ax, v * ay);
        (m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5])
    };
    // 장치 단위 배율 (가로·세로)
    let sx = (m[0] * m[0] + m[1] * m[1]).sqrt() * ax;
    let sy = (m[2] * m[2] + m[3] * m[3]).sqrt() * ay;
    // 그림 상자로 자르기 (한셀 PDF 의 자르기는 상자보다 장치 2.75 단위씩 넓다)
    let corners = [
        f(0.0, 0.0),
        f(mf.width, 0.0),
        f(mf.width, mf.height),
        f(0.0, mf.height),
    ];
    let (x0, x1) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |a, c| (a.0.min(c.0), a.1.max(c.0)));
    let (y0, y1) = corners
        .iter()
        .fold((f64::MAX, f64::MIN), |a, c| (a.0.min(c.1), a.1.max(c.1)));
    ops.push(Op::Clip {
        x0: x0 - 2.75,
        y0: y0 - 2.75,
        x1: x1 + 2.75,
        y1: y1 + 2.75,
    });
    for it in &mf.items {
        match it {
            Item::Path {
                segs,
                fill,
                stroke,
                even_odd,
                caps,
                hatch,
            } => {
                let segs = segs
                    .iter()
                    .map(|s| match *s {
                        Seg::Move(a, b) => {
                            let p = f(a, b);
                            Seg::Move(p.0, p.1)
                        }
                        Seg::Line(a, b) => {
                            let p = f(a, b);
                            Seg::Line(p.0, p.1)
                        }
                        Seg::Cubic(a, b, c, d, e, g) => {
                            let (p, q, r) = (f(a, b), f(c, d), f(e, g));
                            Seg::Cubic(p.0, p.1, q.0, q.1, r.0, r.1)
                        }
                        Seg::Close => Seg::Close,
                    })
                    .collect();
                ops.push(Op::Path {
                    segs,
                    fill: *fill,
                    stroke: stroke
                        .as_ref()
                        .map(|(w, c, d)| (w * sx, *c, d.iter().map(|v| v * sx).collect())),
                    even_odd: *even_odd,
                    caps: Some(*caps),
                    hatch: *hatch,
                });
            }
            Item::Text {
                x,
                y,
                halign,
                valign,
                em,
                name,
                bold,
                italic,
                underline,
                strike,
                color,
                text,
                angle,
            } => {
                let key = FontKey {
                    name: name.clone(),
                    bold: *bold,
                    italic: *italic,
                };
                let (asc, desc) = measure.ascent_descent(&key);
                // 기준선까지: 위 맞춤은 오름, 아래 맞춤은 내림만큼. 한셀은 장치 글꼴 크기(em × 장치 배율)에서 GDI 가 돌려주는
                // 오름·셀 높이를 각각 논리 단위로 반올림하고 내림 = 높이 − 오름 (Arial 15px: 장치 91 → 오름 81, 높이 101 →
                // 13, 17 → 내림 4. 비율 반올림은 14·3). WMF 도 한셀이 EMF 로 바꿔 넣으므로 같다
                let off = if *valign != 1 && sy > 0.0 {
                    let ppem = (em * sy).round().max(1.0) as u32;
                    let (ad, dd) = measure.device_ascent_descent(&key, ppem);
                    let a = (ad / sy).round();
                    if *valign == 0 {
                        a
                    } else {
                        -(((ad + dd) / sy).round() - a)
                    }
                } else {
                    match *valign {
                        0 => (asc * em).round(),
                        2 => -(desc * em).round(),
                        _ => 0.0,
                    }
                };
                let (gs, gc) = angle.to_radians().sin_cos();
                let (bx, by) = (x - off * gs, y + off * gc);
                let (px, py) = f(bx, by);
                let th = (angle + rot).to_radians();
                let (sn, cs) = th.sin_cos();
                let size = em * sy;
                let stretch = if sy > 0.0 { sx / sy } else { 1.0 };
                let tw = measure.width(&key, size, text) * stretch;
                let shift = match halign {
                    1 => tw / 2.0,
                    2 => tw,
                    _ => 0.0,
                };
                ops.push(Op::Text {
                    x: px - shift * cs,
                    y: py - shift * sn,
                    size,
                    font: key,
                    color: *color,
                    text: text.clone(),
                    underline: *underline,
                    strike: *strike,
                    stretch,
                    angle: th.to_degrees(),
                });
            }
            Item::Clip { x0, y0, x1, y1 } => {
                let (a, b) = (f(*x0, *y0), f(*x1, *y1));
                ops.push(Op::Clip {
                    x0: a.0.min(b.0),
                    y0: a.1.min(b.1),
                    x1: a.0.max(b.0),
                    y1: a.1.max(b.1),
                });
            }
            Item::Unclip => ops.push(Op::Unclip),
            Item::Image { x, y, w, h, bmp } => {
                let (a, b) = (f(*x, *y), f(x + w, y + h));
                ops.push(Op::Image {
                    x0: a.0.min(b.0),
                    y0: a.1.min(b.1),
                    x1: a.0.max(b.0),
                    y1: a.1.max(b.1),
                    data: std::sync::Arc::new(bmp.clone()),
                });
            }
        }
    }
    ops.push(Op::Unclip);
}

/// 개체 하나 (그룹이면 자식들을 그룹 상자 안에)
#[allow(clippy::too_many_arguments)]
fn object_ops(
    wb: &Workbook,
    sh: &crate::model::Shape,
    obj: Option<&crate::xml::Element>,
    (x, y, w, h): (f64, f64, f64, f64),
    frame: &PageFrame,
    theme: &crate::theme::Theme,
    measure: &dyn TextMeasure,
    ops: &mut Vec<Op>,
) {
    use crate::model::ShapeKind;
    let k = frame.px;
    let (wp, hp) = (w / k, h / k);
    let (rot, fh, fv) = sh.xfrm.as_ref().map_or((0.0, false, false), |xf| {
        (
            xf.rot as f64 / 60000.0,
            xf.flip_h && sh.kind != ShapeKind::Connector,
            xf.flip_v && sh.kind != ShapeKind::Connector,
        )
    });
    let m = box_matrix(x, y, w, h, k, rot, fh, fv);
    let mw = |key: &FontKey, size: f64, s: &str| measure.width(key, size, s);
    match sh.kind {
        ShapeKind::Group => {
            let xf = obj
                .and_then(super::shape::sp_pr)
                .and_then(|p| p.child_ns(super::shape::A, "xfrm"));
            let num = |e: Option<&crate::xml::Element>, n: &str| {
                e.and_then(|e| e.attr(n))
                    .and_then(|v| v.parse::<f64>().ok())
            };
            let ch_off = xf.and_then(|x| x.child_ns(super::shape::A, "chOff"));
            let ch_ext = xf.and_then(|x| x.child_ns(super::shape::A, "chExt"));
            let (cx0, cy0) = (
                num(ch_off, "x").unwrap_or(0.0),
                num(ch_off, "y").unwrap_or(0.0),
            );
            let (cw, chh) = (
                num(ch_ext, "cx").filter(|v| *v > 0.0),
                num(ch_ext, "cy").filter(|v| *v > 0.0),
            );
            let raw_kids: Vec<&crate::xml::Element> = obj
                .map(|o| {
                    o.elements()
                        .filter(|c| {
                            c.ns.as_deref() == Some(super::shape::XDR)
                                && matches!(
                                    c.name.as_str(),
                                    "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                                )
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (i, kid) in sh.children.iter().enumerate().filter(|(_, c)| !c.hidden) {
                let Some(kx) = kid.xfrm.as_ref() else {
                    continue;
                };
                let sx = cw.map_or(k / EMU, |cw| w / cw);
                let sy = chh.map_or(k / EMU, |ch| h / ch);
                let r = (
                    x + (kx.x as f64 - cx0) * sx,
                    y + (kx.y as f64 - cy0) * sy,
                    kx.cx as f64 * sx,
                    kx.cy as f64 * sy,
                );
                object_ops(
                    wb,
                    kid,
                    raw_kids.get(i).copied(),
                    r,
                    frame,
                    theme,
                    measure,
                    ops,
                );
            }
        }
        ShapeKind::Picture => {
            let img = sh
                .target
                .as_ref()
                .and_then(|t| wb.raw_parts.iter().find(|p| &p.name == t));
            if let Some(p) = img {
                if let Some(mf) = super::metafile::parse(&p.data) {
                    metafile_ops(&mf, m, (wp, hp), rot, measure, ops);
                } else {
                    ops.push(Op::Image {
                        x0: x,
                        y0: y,
                        x1: x + w,
                        y1: y + h,
                        data: std::sync::Arc::new(p.data.clone()),
                    });
                }
            }
        }
        ShapeKind::Chart => {
            // 차트는 인쇄 크기의 px 로 다시 짠다 (한셀 PDF: 배율 34% 차트의 눈금 간격 수가 인쇄 크기를 따른다)
            if let Some(ch) = &sh.chart {
                let svg = crate::chart_svg::chart_svg(ch, w / 6.25, h / 6.25, theme, &|c| {
                    super::draw::draw_color(c, theme)
                });
                let m = box_matrix(x, y, w, h, 6.25, rot, fh, fv);
                // 한셀은 레거시 차트(VtChart 에서 읽은 것)는 벡터로, 새 차트만 비트맵으로 찍는다 (레거시 판 PDF 에 글자 `Tf` 가 있다)
                if rot == 0.0 && !fh && !fv && ch.legacy.is_none() {
                    let mut sub = Vec::new();
                    super::svgops::svg_ops(&svg, m, &mw, &mut sub);
                    ops.push(Op::Raster {
                        x0: x,
                        y0: y,
                        x1: x + w,
                        y1: y + h,
                        ops: sub,
                    });
                } else {
                    super::svgops::svg_ops(&svg, m, &mw, ops);
                }
            }
        }
        _ => {
            let html = super::draw::shape_body(wb, sh, obj, wp, hp, theme);
            for b in svg_blocks(&html) {
                super::svgops::svg_ops(b, m, &mw, ops);
            }
            let body = obj
                .and_then(|o| o.child_ns(super::shape::XDR, "txBody"))
                .map(|tx| raw_body(tx, theme))
                .or_else(|| sh.text.as_ref().map(|t| model_body(t, theme)));
            if let Some(t) = body {
                shape_text(
                    &t,
                    (x, y, w, h),
                    k,
                    frame.zoom as f64 / 100.0,
                    measure,
                    frame.black_and_white,
                    ops,
                );
            }
        }
    }
}

/// 개체 하나를 (0, 0) 에서 시작하는 w × h (px) 상자 안에 그린 목록 (웹 페이지의 도형·차트 그림용). 장치 단위는 1px = 6.25.
pub fn shape_ops(
    wb: &Workbook,
    sh: &crate::model::Shape,
    w: f64,
    h: f64,
    measure: &dyn TextMeasure,
) -> Vec<Op> {
    let frame = PageFrame {
        px: 6.25,
        zoom: 100,
        ..Default::default()
    };
    let theme = wb.theme.clone().unwrap_or_default();
    let obj = sh.raw.as_deref().and_then(super::shape::object_element);
    let mut ops = Vec::new();
    object_ops(
        wb,
        sh,
        obj.as_ref(),
        (0.0, 0.0, w * 6.25, h * 6.25),
        &frame,
        &theme,
        measure,
        &mut ops,
    );
    ops
}

/// 바깥 그림자: (가로, 세로 이동 px — 정수로 반올림, 색). 흐림 없는 그림자만 (웹 페이지 그림용)
pub fn shape_shadow(wb: &Workbook, sh: &crate::model::Shape) -> Option<(i32, i32, [u8; 3])> {
    let s = sh.shadow.as_ref()?;
    let theme = wb.theme.clone().unwrap_or_default();
    let (r, g, b) = super::draw::draw_rgb(&s.color, &theme).unwrap_or((0, 0, 0));
    let dist = s.dist as f64 / 9525.0;
    let ang = (s.dir as f64 / 60000.0).to_radians();
    let (dx, dy) = (
        (dist * ang.cos()).trunc() as i32,
        (dist * ang.sin()).trunc() as i32,
    );
    Some((dx, dy, [r, g, b]))
}

/// 페이지에 걸친 그리기 개체들을 그린다 (페이지 영역으로 자른다)
pub fn objects_ops(
    wb: &Workbook,
    sheet: &Sheet,
    frame: &PageFrame,
    measure: &dyn TextMeasure,
) -> Vec<Op> {
    let mut ops = Vec::new();
    if frame.rows.is_empty() || frame.cols.is_empty() || sheet.shapes.is_empty() {
        return ops;
    }
    let theme = wb.theme.clone().unwrap_or_default();

    let area = (
        frame.cols.iter().map(|c| c.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.rows.iter().map(|r| r.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.cols.iter().map(|c| c.2).fold(f64::MIN, f64::max) - SHIFT,
        frame.rows.iter().map(|r| r.2).fold(f64::MIN, f64::max) - SHIFT,
    );
    let k = frame.px;
    // 시트의 절대 화면 px (칸 시작 + 오프셋), 페이지의 첫 열·행 시작을 0 으로
    let (Some(fc), Some(fr)) = (frame.cols.first(), frame.rows.first()) else {
        return ops;
    };
    let start = |v: &[f64], i: u32, dflt: f64| -> f64 {
        v.get(i as usize).copied().unwrap_or_else(|| {
            let n = v.len().saturating_sub(1);
            v.last().copied().unwrap_or(0.0) + (i as usize - n) as f64 * dflt
        })
    };
    let (x_page0, y_page0) = (
        start(&frame.col_abs, fc.0, 64.0),
        start(&frame.row_abs, fr.0, 20.0),
    );
    let at = |m: &crate::model::Marker| -> Option<(f64, f64)> {
        Some((
            start(&frame.col_abs, m.col, 64.0) + m.col_off as f64 / EMU - x_page0,
            start(&frame.row_abs, m.row, 20.0) + m.row_off as f64 / EMU - y_page0,
        ))
    };
    let mut inner = Vec::new();
    for sh in sheet
        .shapes
        .iter()
        .filter(|s| !s.hidden && s.prints_with_sheet != Some(false))
    {
        let a = &sh.anchor;
        let first = match (a.kind, a.pos) {
            (crate::model::AnchorKind::Absolute, Some((px, py))) => Some((
                start(&frame.col_abs, 0, 64.0) + px as f64 / EMU - x_page0,
                start(&frame.row_abs, 0, 20.0) + py as f64 / EMU - y_page0,
            )),
            _ => at(&a.from),
        };
        let Some((x1, y1)) = first else { continue };
        let second = match (&a.to, a.ext) {
            (Some(t), _) if a.kind == crate::model::AnchorKind::TwoCell => at(t),
            (_, Some((cx, cy))) => Some((x1 + cx as f64 / EMU, y1 + cy as f64 / EMU)),
            (Some(t), None) => at(t),
            _ => Some((x1, y1)),
        };
        let Some((x2, y2)) = second else { continue };
        let kind = match sh.kind {
            crate::model::ShapeKind::Picture => ObjKind::Picture,
            crate::model::ShapeKind::Chart => ObjKind::Chart,
            _ => ObjKind::Shape,
        };
        let (ox, oy) = (frame.origin_x, fr.1);
        let (left, top) = (
            object_origin(ox, x1, k, kind),
            object_origin(oy, y1, k, kind),
        );
        let (wd, hd) = (object_size(x2 - x1, k, kind), object_size(y2 - y1, k, kind));
        let (x1, y1, x2, y2) = (left, top, left + wd, top + hd);
        if x2 < area.0 || x1 > area.2 || y2 < area.1 || y1 > area.3 {
            continue;
        }
        let obj = sh.raw.as_deref().and_then(super::shape::object_element);
        object_ops(
            wb,
            sh,
            obj.as_ref(),
            (x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)),
            frame,
            &theme,
            measure,
            &mut inner,
        );
    }
    if !inner.is_empty() {
        ops.push(Op::Clip {
            x0: area.0,
            y0: area.1,
            x1: area.2,
            y1: area.3,
        });
        ops.extend(inner);
        ops.push(Op::Unclip);
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_hangul_anywhere_and_words_whole() {
        let w = |s: &str| s.chars().count() as f64 * 10.0;
        assert_eq!(wrap_text("가나다라마바", 35.0, &w), ["가나다", "라마바"]);
        assert_eq!(wrap_text("abc def", 45.0, &w), ["abc", "def"]);
        assert_eq!(wrap_text("가\n나", 100.0, &w), ["가", "나"]);
    }

    /// 메타파일 글자의 기준선: 한셀은 장치 글꼴 크기의 GDI 오름·셀 높이를 논리 단위로 반올림한다. Arial 15px (장치 배율 6.049 →
    /// 장치 91px): 오름 81·내림 20 → 오름 13, 높이 101 → 17, 내림 4 (비율 반올림이면 14·3)
    #[test]
    fn metafile_text_baseline_uses_device_metrics() {
        use super::super::metafile::{Item, Metafile};
        struct Arial;
        impl TextMeasure for Arial {
            fn width(&self, _: &FontKey, size: f64, text: &str) -> f64 {
                size * text.chars().count() as f64
            }
            fn ascent_descent(&self, _: &FontKey) -> (f64, f64) {
                (0.9053, 0.2119)
            }
            fn device_ascent_descent(&self, _: &FontKey, ppem: u32) -> (f64, f64) {
                if ppem == 91 {
                    (81.0, 20.0)
                } else {
                    (0.9053 * ppem as f64, 0.2119 * ppem as f64)
                }
            }
        }
        let text = |valign: u8| Item::Text {
            x: 20.0,
            y: 100.0,
            halign: 0,
            valign,
            em: 15.0,
            name: "Arial".into(),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: [0, 0, 0],
            text: "H".into(),
            angle: 0.0,
        };
        let mf = Metafile {
            width: 400.0,
            height: 1000.0,
            items: vec![text(0), text(1), text(2)],
        };
        let s = 6.049;
        let mut ops = Vec::new();
        metafile_ops(
            &mf,
            [s, 0.0, 0.0, s, 0.0, 0.0],
            (400.0, 1000.0),
            0.0,
            &Arial,
            &mut ops,
        );
        let ys: Vec<f64> = ops
            .iter()
            .filter_map(|o| match o {
                Op::Text { y, .. } => Some((y / s).round()),
                _ => None,
            })
            .collect();
        assert_eq!(ys, [113.0, 100.0, 96.0]);
    }

    /// 한셀 PDF 에서 잰 그림·도형 상자 (원점 449·591, 배율 99 = k 6.1875): 그림 off (133, 200) ext (200, 400) → 왼쪽 1264,
    /// 위 …; 그림 off (17, 22) ext (102, 44) → 546, 크기 632 × 273; 도형 off (134, 200) ext (200, 400) → 1269.5, 크기 1238 ×
    /// 2475; 도형 off (53, 53) ext (401, 300) → 768.5, 2482 × 1857
    #[test]
    fn object_boxes() {
        let k = 6.25 * 0.99;
        assert_eq!(object_origin(449.0, 133.0, k, ObjKind::Picture), 1264.0);
        assert_eq!(object_origin(449.0, 17.0, k, ObjKind::Picture), 546.0);
        assert_eq!(object_origin(449.0, 265.0, k, ObjKind::Picture), 2080.0);
        assert_eq!(object_size(102.0, k, ObjKind::Shape), 632.0);
        assert_eq!(object_size(44.0, k, ObjKind::Shape), 273.0);
        assert_eq!(object_origin(449.0, 134.0, k, ObjKind::Shape), 1269.5);
        assert_eq!(object_origin(449.0, 53.0, k, ObjKind::Shape), 768.5);
        assert_eq!(object_origin(449.0, 17.0, k, ObjKind::Shape), 545.5);
        assert_eq!(object_size(200.0, k, ObjKind::Shape), 1238.0);
        assert_eq!(object_size(401.0, k, ObjKind::Shape), 2482.0);
        assert_eq!(object_size(300.0, k, ObjKind::Shape), 1857.0);
        // 앵커 오프셋이 1 EMU 모자라도 (52 − 1/9525 px) 한셀과 같은 칸에 놓인다
        assert_eq!(
            object_origin(449.0, 52.0 - 1.0 / 9525.0, k, ObjKind::Picture),
            763.0
        );
    }

    /// 한셀 PDF 에서 잰 글자 시작 (맑은 고딕 11pt: 숫자 8.078px, 한글 14.667px, "AAAA" 38.61px)
    #[test]
    fn aligned_text_start() {
        let digit = 1128.0 / 2048.0 * 11.0 * 4.0 / 3.0;
        // 배율 100 (두 번째 시트): 143px 열의 오른쪽 맞춤 "1" 은 132, "가가가" 는 96, 가운데 "1" 은 67
        assert_eq!(aligned_px(true, 143.0, digit, 0.0, 100), 132.0);
        assert_eq!(aligned_px(true, 143.0, 44.0, 0.0, 100), 96.0);
        assert_eq!(aligned_px(false, 143.0, digit, 0.0, 100), 67.0);
        assert_eq!(aligned_px(false, 144.0, 38.61, 0.0, 100), 53.0);
        // 배율 99: 182px 열의 "1234567890" 은 95, "1111" 은 144
        assert_eq!(aligned_px(true, 182.0, digit * 10.0, 0.0, 99), 95.0);
        assert_eq!(aligned_px(true, 182.0, digit * 4.0, 0.0, 99), 144.0);
        // 배율 150: 142px 열의 가운데 "AAAA" 는 77
        assert_eq!(aligned_px(false, 142.0, 38.61, 0.0, 150), 77.0);
    }
}
