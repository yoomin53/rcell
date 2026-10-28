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
    let mut ops = Vec::new();
    if frame.rows.is_empty() || frame.cols.is_empty() {
        return ops;
    }
    let mono = frame.black_and_white;
    let max_r = frame.rows.iter().map(|r| r.0).max().unwrap_or(0);
    let max_c = frame.cols.iter().map(|c| c.0).max().unwrap_or(0);
    // 병합·넘침을 볼 수 있게 조금 넓게
    let mut lay_r = max_r + 1;
    let mut lay_c = max_c + 1;
    for m in &sheet.merges {
        lay_r = lay_r.max(m.end.row + 1);
        lay_c = lay_c.max(m.end.col + 1);
    }
    let lay_c = (lay_c + 32).min(16_384);
    let ctx = Views::new(wb, sheet, lay_r, lay_c);
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
        for x in xs {
            ops.push(Op::Line {
                x0: x - SHIFT,
                y0: t - SHIFT,
                x1: x - SHIFT,
                y1: bb - SHIFT,
                width: 6.0,
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
                width: 6.0,
                color: BLACK,
                dash: Vec::new(),
            });
        }
    }
    // 3. 글자
    let theme = &ctx.theme;
    for &(r, c) in &draw {
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
        let indent = v.indent as f64 * INDENT_PX * px;
        let w = |s: &str, sz: f64| measure.width(&key, sz, s) * stretch;
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
        let first = match v.v {
            VAlign::Top => y0 + line / 2.0 + kt * size + ct * k99,
            VAlign::Bottom => y1 - line / 2.0 + kb * size - cb * k99 - (n - 1.0) * line,
            VAlign::Center => (y0 + y1) / 2.0 + km * size + cm * k99 - (n - 1.0) * line / 2.0,
        };
        let need_clip = !(v.wrap || ol > 0 || or > 0);
        if need_clip {
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
            let x = match v.h {
                HAlign::Right => bx1 - 4.2 * px - indent - tw,
                HAlign::Center => (bx0 + bx1) / 2.0 - 1.3 * px - tw / 2.0,
                HAlign::CenterContinuous => (x0 + bx1) / 2.0 - 1.3 * px - tw / 2.0,
                _ => bx0 + pad_l + indent,
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
        if need_clip {
            ops.push(Op::Unclip);
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
                ops.push(Op::Image {
                    x0: x,
                    y0: y,
                    x1: x + w,
                    y1: y + h,
                    data: std::sync::Arc::new(p.data.clone()),
                });
            }
        }
        ShapeKind::Chart => {
            // 차트는 인쇄 크기의 px 로 다시 짠다 (한셀 PDF: 배율 34% 차트의 눈금 간격 수가 인쇄 크기를 따른다)
            if let Some(ch) = &sh.chart {
                let svg = crate::chart_svg::chart_svg(ch, w / 6.25, h / 6.25, theme, &|c| {
                    super::draw::draw_color(c, theme)
                });
                let m = box_matrix(x, y, w, h, 6.25, rot, fh, fv);
                super::svgops::svg_ops(&svg, m, &mw, ops);
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
    let ax = Axis::new(&frame.cols, &frame.col_dev);
    let ay = Axis::new(&frame.rows, &frame.row_dev);
    let area = (
        frame.cols.iter().map(|c| c.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.rows.iter().map(|r| r.1).fold(f64::MAX, f64::min) - SHIFT,
        frame.cols.iter().map(|c| c.2).fold(f64::MIN, f64::max) - SHIFT,
        frame.rows.iter().map(|r| r.2).fold(f64::MIN, f64::max) - SHIFT,
    );
    let k = frame.px;
    let at = |m: &crate::model::Marker| -> Option<(f64, f64)> {
        let x = ax.start(m.col)? + (m.col_off as f64 / EMU * k).min(ax.size(m.col)).max(0.0);
        let y = ay.start(m.row)? + (m.row_off as f64 / EMU * k).min(ay.size(m.row)).max(0.0);
        Some((x, y))
    };
    let mut inner = Vec::new();
    for sh in sheet
        .shapes
        .iter()
        .filter(|s| !s.hidden && s.prints_with_sheet != Some(false))
    {
        let a = &sh.anchor;
        let first = match (a.kind, a.pos) {
            (crate::model::AnchorKind::Absolute, Some((px, py))) => ax
                .start(0)
                .zip(ay.start(0))
                .map(|(x0, y0)| (x0 + px as f64 / EMU * k, y0 + py as f64 / EMU * k)),
            _ => at(&a.from),
        };
        let Some((x1, y1)) = first else { continue };
        let second = match (&a.to, a.ext) {
            (Some(t), _) if a.kind == crate::model::AnchorKind::TwoCell => at(t),
            (_, Some((cx, cy))) => Some((x1 + cx as f64 / EMU * k, y1 + cy as f64 / EMU * k)),
            (Some(t), None) => at(t),
            _ => Some((x1, y1)),
        };
        let Some((x2, y2)) = second else { continue };
        // 한셀 화면처럼 경계 1px 앞 (선 가운데)
        let (x1, y1, x2, y2) = (x1 - SHIFT, y1 - SHIFT, x2 - SHIFT, y2 - SHIFT);
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
}
