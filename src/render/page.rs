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

/// 그리기 하나 (장치 단위, 위가 0). 색은 RGB
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
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
