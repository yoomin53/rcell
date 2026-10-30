// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 칸 하나의 글자: 정렬·넘침·줄 바꿈·줄여 맞춤·채움·회전·세로 쓰기·들여쓰기, 글꼴 CSS
//! (한셀 화면 갈무리 대조 실험, docs/format/screen.md).
//!
//! 한 줄 글자는 칸(넘치면 넘친 범위)을 잘라 보이는 상자 안에 절대 위치로 놓는다: 왼쪽 맞춤은 왼쪽 여백에서,
//! 오른쪽 맞춤은 오른쪽 여백에서, 가운데는 칸 가운데에서 시작해 넘치는 쪽으로 자란다. 그래서 오른쪽 맞춤
//! 글자가 칸보다 넓고 왼쪽이 막혀 있으면 한셀처럼 글자의 끝이 보인다.

use std::fmt::Write as _;

use super::layout::{PAD, PAD_R};
use super::metrics::{self, FontMetrics};
use super::paint;
use super::{HAlign, INDENT_PX, VAlign};
use crate::model::{CellValue, Font, RichText};
use crate::theme::{self, Theme};
use crate::writer::esc;

/// 아이콘이 차지하는 왼쪽 폭 (px)
const ICON_W: f64 = 19.0;

/// 데이터 막대 (한셀 화면: 직각삼각형 — x14 데이터 막대 실험). 칸 안쪽 [1, 1 + (w − 4)) 이 막대 영역이고 축은
/// 그 영역의 비율 자리 픽셀, 막대는 축 오른쪽 경계에서 시작한다. 위·아래는 3px 씩 띄우고 (행 높이 16·22·27·40·60px
/// 모두), 테두리는 세로선이 축 다음 픽셀, 위·아래는 1px 바깥. 삼각형의 세로 변은 양수 막대면 왼쪽, 음수면 오른쪽
/// (오른쪽에서 왼쪽 방향은 자리만 좌우로 뒤집고 모양은 그대로). 그라데이션은 세로 변의 색에서 끝의 흰색까지.
/// 축은 칸 높이 전체(아래 격자선 포함)에 2px 점·1px 틈 점선
fn data_bar_svg(b: &crate::cf_eval::BarDraw, w: u32, h: u32) -> String {
    let iw = w.saturating_sub(1) as f64;
    let area = (w as f64 - 4.0).max(1.0);
    let (top, bot) = (3.0, (h as f64 - 3.0).max(4.0));
    let ax = b.axis.as_ref().map(|(a, _)| (1.0 + area * a).floor());
    // 왼쪽 → 오른쪽 기준의 시작(축 쪽)·끝
    let (xs, xe) = match ax {
        Some(ax) => {
            let xs = ax + 1.0;
            if b.leftward {
                (xs, xs - (xs - 1.0) * b.len)
            } else {
                (xs, xs + (1.0 + area - xs) * b.len)
            }
        }
        None => (1.0, 1.0 + area * b.len),
    };
    let flip = |x: f64| if b.rtl { iw - x } else { x };
    let mut g = String::new();
    if (xe - xs).abs() >= 0.5 {
        let (p, q) = (flip(xs).min(flip(xe)), flip(xs).max(flip(xe)));
        // 세로 변: 양수 막대는 왼쪽, 음수(왼쪽 기준에서 왼쪽으로 자라는) 막대는 오른쪽
        let (v, t) = if b.leftward { (q, p) } else { (p, q) };
        let fill = if b.gradient {
            let id = format!(
                "rcdb{}{}",
                b.fill.trim_start_matches('#'),
                if b.leftward { "l" } else { "r" }
            );
            let (x1, x2) = if b.leftward { (1, 0) } else { (0, 1) };
            let _ = write!(
                g,
                "<defs><linearGradient id=\"{id}\" x1=\"{x1}\" y1=\"0\" x2=\"{x2}\" y2=\"0\"><stop offset=\"0\" stop-color=\"{}\"/><stop offset=\"1\" stop-color=\"#FFFFFF\"/></linearGradient></defs>",
                b.fill
            );
            format!("url(#{id})")
        } else {
            b.fill.clone()
        };
        let _ = write!(
            g,
            "<path d=\"M{v:.1},{top}V{bot}H{t:.1}Z\" fill=\"{fill}\"/>"
        );
        if let Some(bc) = &b.border {
            let _ = write!(
                g,
                "<path d=\"M{:.1},{:.1}V{:.1}H{t:.1}Z\" fill=\"none\" stroke=\"{bc}\" stroke-width=\"1\"/>",
                v + 0.5,
                top - 0.5,
                bot + 0.5
            );
        }
    }
    if let (Some((_, c)), Some(ax)) = (&b.axis, ax) {
        // 오른쪽에서 왼쪽 막대의 축은 뒤집은 자리보다 2px 왼쪽 (x14 실험의 rightToLeft 열: 칸 폭 82 에서 77 번째 픽셀)
        let x = if b.rtl {
            flip(ax + 0.5) - 2.0
        } else {
            ax + 0.5
        };
        let _ = write!(
            g,
            "<path d=\"M{x:.1},0V{h}\" stroke=\"{c}\" stroke-width=\"1\" stroke-dasharray=\"2 1\"/>"
        );
    }
    format!("<svg class=\"rc-bar\" width=\"{iw:.0}\" height=\"{h}\">{g}</svg>")
}

/// 칸 하나를 그리는 데 필요한 것
pub(super) struct CellView<'a> {
    pub value: CellValue,
    /// 표시 글자 (숫자는 칸에 맞춘 뒤)
    pub text: String,
    /// 서식 있는 글 (공유 문자열의 조각)
    pub rich: Option<&'a RichText>,
    pub font: Option<&'a Font>,
    pub m: FontMetrics,
    /// 서식 코드의 색 ([Red] 등)
    pub code_color: Option<String>,
    pub h: HAlign,
    pub v: VAlign,
    pub wrap: bool,
    pub shrink: bool,
    pub rotation: i32,
    /// 서식의 `_x` 빈 자리 (글의 바이트 위치, x)
    pub pads: Vec<(usize, char)>,
    /// 서식의 `*x` 채움 (글의 바이트 위치, x)
    pub fill_char: Option<(usize, char)>,
    pub indent: u32,
    pub fill: Option<paint::FillPaint>,
    pub borders: paint::CellBorders,
}

impl CellView<'_> {
    pub fn has_value(&self) -> bool {
        !self.value.is_empty()
    }

    /// 이웃 칸으로 넘칠 수 있는 글자인가 (글만, 한 줄, 돌리지 않음)
    pub fn may_overflow(&self) -> bool {
        matches!(self.value, CellValue::Text(_))
            && !self.wrap
            && !self.shrink
            && self.rotation == 0
            && matches!(
                self.h,
                HAlign::Left | HAlign::Right | HAlign::Center | HAlign::CenterContinuous
            )
    }

    /// 한 줄 글자 폭 (px, 장평 포함)
    pub fn text_px(&self) -> f64 {
        self.m.text_width(&self.text) * ratio_of(self.font).unwrap_or(1.0)
    }
}

/// 서식 코드 색 이름 → CSS
pub(super) fn code_color(c: &str) -> String {
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

/// 글꼴 → CSS (가족·크기·굵게·기울임·밑줄·취소선·첨자·색·자간. 장평은 따로)
pub(super) fn font_css(f: Option<&Font>, theme: &Theme, color: Option<&str>) -> String {
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
        if f.underline
            .as_deref()
            .is_some_and(|u| u.starts_with("double"))
            || f.double_strike
        {
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
pub(super) fn font_family(n: &str) -> String {
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
    let n = n.replace(['\'', '"', ';', '<', '>'], "");
    match alias {
        Some(a) => format!("'{n}','{a}',sans-serif"),
        None => format!("'{n}','맑은 고딕','Malgun Gothic',sans-serif"),
    }
}

pub(super) fn ratio_of(f: Option<&Font>) -> Option<f64> {
    f.and_then(|f| f.hancom.as_ref())
        .and_then(|h| h.ratio)
        .filter(|&r| r != 100 && r > 0)
        .map(|r| r as f64 / 100.0)
}

/// 글 (서식 있는 글이면 조각마다 글꼴)
/// 글 덩이의 HTML: 한셀의 정수 글자 폭이 되도록 글자마다(같은 자간끼리 묶어) 자간을 붙인다 (advance.rs). `.rc-t>span` 의
/// 절대 위치 규칙에 걸리지 않게 서식 없는 인라인 요소 `<data>` 를 쓴다
fn spaced_html(name: Option<&str>, size_pt: f64, bold: bool, text: &str) -> String {
    match super::advance::segments(name, size_pt, bold, text) {
        Some(segs) => {
            let mut s = String::new();
            for (ls, t) in segs {
                if ls == 0.0 {
                    s.push_str(&esc(&t));
                } else {
                    let _ = write!(
                        s,
                        "<data style=\"letter-spacing:{ls:.3}px\">{}</data>",
                        esc(&t)
                    );
                }
            }
            s
        }
        None => esc(text),
    }
}

fn rich_html(v: &CellView, text: &str, theme: &Theme, cf_bold: bool) -> String {
    let mut s = String::new();
    // 돌린 글은 정수 폭에 맞추지 않는다
    let spaced = |name: Option<&str>, size: f64, bold: bool, t: &str| {
        if v.rotation == 0 {
            spaced_html(name, size, bold, t)
        } else {
            esc(t)
        }
    };
    match v.rich {
        Some(rt) if text == rt.text => {
            for run in &rt.runs {
                let mut css = run
                    .font
                    .as_ref()
                    .map(|f| font_css(Some(f), theme, None))
                    .unwrap_or_default();
                // 글자 폭: 조각마다 한셀의 정수 폭에 맞춘 자간 (advance.rs)
                let html = {
                    let f = run.font.as_ref();
                    let has_sp = f
                        .and_then(|f| f.hancom.as_ref())
                        .and_then(|h| h.spacing)
                        .is_some_and(|s| s != 0);
                    let name = f
                        .and_then(|f| f.name.as_deref())
                        .or_else(|| v.font.and_then(|c| c.name.as_deref()));
                    let size = f.and_then(|f| f.size).unwrap_or_else(|| v.m.size_pt());
                    let bold = f.map_or_else(|| v.font.is_some_and(|c| c.bold), |f| f.bold);
                    // 줄 높이는 조각 글꼴의 한 줄 높이 — 크기가 다른 조각이 섞인 줄은 가장 큰 조각의 높이가 줄 높이가 된다
                    // (한셀 화면: 20pt 조각이 낀 칸의 글 기준선이 가운데 맞춤에서 행 아래 − 글꼴 아래 폭)
                    if f.is_some() {
                        let m = metrics::FontMetrics::new(name, Some(size), bold);
                        let _ = write!(css, "line-height:{:.2}px;", m.row_pt * 4.0 / 3.0);
                    }
                    if has_sp || f.is_some_and(|f| f.vert_align.is_some()) {
                        esc(&run.text)
                    } else {
                        spaced(name, size, bold, &run.text)
                    }
                };
                // 조각의 위·아래 첨자도 칸 전체와 같게: 60% 글꼴을 기준선에서 올리거나 내린다
                if let Some(f) = run.font.as_ref()
                    && let Some(va) = f.vert_align.as_deref()
                {
                    let size = f
                        .size
                        .or_else(|| v.font.and_then(|c| c.size))
                        .unwrap_or(metrics::DEFAULT_SIZE);
                    let dy = match va {
                        "superscript" => -0.45,
                        "subscript" => 0.19,
                        _ => 0.0,
                    } * size
                        * 4.0
                        / 3.0;
                    if dy != 0.0 {
                        css = css
                            .replace("vertical-align:super;font-size:smaller;", "")
                            .replace("vertical-align:sub;font-size:smaller;", "");
                        let _ = write!(
                            css,
                            "font-size:{:.2}pt;position:relative;top:{dy:.1}px;",
                            size * 0.6
                        );
                    }
                }
                if css.is_empty() {
                    s.push_str(&html);
                } else {
                    let _ = write!(s, "<span style=\"{css}\">{html}</span>");
                }
            }
        }
        _ => {
            let has_sp = v
                .font
                .and_then(|f| f.hancom.as_ref())
                .and_then(|h| h.spacing)
                .is_some_and(|s| s != 0);
            if has_sp {
                s.push_str(&esc(text));
            } else {
                let bold = v.font.is_some_and(|f| f.bold);
                s.push_str(&spaced(
                    v.font.and_then(|f| f.name.as_deref()),
                    v.m.size_pt(),
                    bold || cf_bold,
                    text,
                ));
            }
        }
    }
    s
}

/// 칸 안 글자 (`.rc-t` 상자). `w`×`h` 는 칸(병합이면 병합 영역)의 크기(px, 오른쪽·아래 격자선 포함),
/// `overflow` 는 글자가 넘쳐 쓰는 왼쪽·오른쪽 이웃 폭(px)
pub(super) fn cell_content(
    v: &CellView,
    cf: Option<&crate::cf_eval::CfStyle>,
    theme: &Theme,
    w: u32,
    h: u32,
    overflow: (u32, u32),
) -> String {
    let mut out = String::new();
    // 데이터 막대 (조건부 서식): 글자 뒤 칸 안쪽
    if let Some(b) = cf.and_then(|c| c.bar.as_ref()) {
        out.push_str(&data_bar_svg(b, w, h));
    }
    let mut text = v.text.clone();
    if cf.is_some_and(|c| c.hide_value) {
        text.clear();
    }
    // 아이콘: 칸 왼쪽 3px, 세로는 글자 맞춤대로 (한셀 화면 — 가운데면 (높이 − 16)/2, 아이콘 실험)
    let has_icon = match cf.and_then(|c| c.icon.as_ref()) {
        Some((set, idx, _)) => {
            let bh = h.saturating_sub(1) as f64;
            let top = match v.v {
                VAlign::Top => 2.0,
                VAlign::Center => ((bh - 16.0) / 2.0).round(),
                VAlign::Bottom => bh - 18.0,
            }
            .max(0.0);
            out.push_str(&super::icons::icon_svg(set, *idx, top));
            true
        }
        None => false,
    };
    if text.is_empty() {
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
    // 줄 높이 = 글꼴의 한 줄 높이 (행 자동 높이와 같은 값)
    let line = v.m.row_pt * 4.0 / 3.0;
    let _ = write!(fcss, "line-height:{line:.2}px;");
    // 왼쪽 맞춤 글자는 아이콘 뒤에서 (3px + 16px)
    let indent = v.indent as f64 * INDENT_PX
        + if has_icon
            && !matches!(
                v.h,
                HAlign::Right | HAlign::Center | HAlign::CenterContinuous
            )
        {
            ICON_W
        } else {
            0.0
        };
    let ratio = ratio_of(v.font);
    let scaled = |inner: String, origin: &str| match ratio {
        Some(r) => format!(
            "<span style=\"display:inline-block;transform:scaleX({r:.3});transform-origin:{origin}\">{inner}</span>"
        ),
        None => inner,
    };
    // 보이는 칸 상자 (격자선 제외)
    let (bw, bh) = (
        w.saturating_sub(1).max(1) as f64,
        h.saturating_sub(1).max(1) as f64,
    );
    let vpos = match v.v {
        VAlign::Top => "top:0".to_string(),
        VAlign::Center => format!("top:{:.1}px", bh / 2.0),
        VAlign::Bottom => "bottom:0".to_string(),
    };
    let vshift = if v.v == VAlign::Center { "-50%" } else { "0" };
    let mut body = rich_html(v, &text, theme, cf.is_some_and(|c| c.bold == Some(true)));
    // 칸 전체 위·아래 첨자: 60% 글꼴을 기준선에서 올리거나 내린다 (한셀 화면 — 글꼴 실험의 11pt: 위 첨자 잉크
    // 1–9px, 아래 첨자 10–17px, 보통 글 3–16px)
    if let Some(va) = v.font.and_then(|f| f.vert_align.as_deref()) {
        let px = v.font.and_then(|f| f.size).unwrap_or(metrics::DEFAULT_SIZE) * 4.0 / 3.0;
        let dy = match va {
            "superscript" => -0.45 * px,
            "subscript" => 0.19 * px,
            _ => 0.0,
        };
        if dy != 0.0 {
            fcss = fcss.replace("vertical-align:super;font-size:smaller;", "");
            fcss = fcss.replace("vertical-align:sub;font-size:smaller;", "");
            body = format!(
                "<span style=\"font-size:60%;position:relative;top:{dy:.1}px\">{body}</span>"
            );
        }
    }

    // 돌린 글자·세로 쓰기
    if v.rotation != 0 {
        return out + &rotated(v, &fcss, &body, bw, bh, line);
    }
    // 균등 분할 글: 들여쓰기를 양쪽에 두고, 넘치면 줄을 바꿔 줄마다 글자 사이(공백도 한 글자)를 고르게 벌린다
    // (한셀 화면 — 들여쓰기 실험). 한 글자 줄은 가운데
    if v.h == HAlign::Distributed && matches!(v.value, CellValue::Text(_)) && !v.shrink {
        let ind = v.indent as f64 * INDENT_PX;
        let avail = (bw - PAD - PAD_R - 2.0 * ind).max(1.0);
        let k = ratio.unwrap_or(1.0);
        let jc = match v.v {
            VAlign::Top => "flex-start",
            VAlign::Center => "center",
            VAlign::Bottom => "flex-end",
        };
        let mut inner = String::new();
        for l in super::layout::wrap_split(&text, avail / k, &v.m) {
            let n = l.chars().count();
            let tw = v.m.text_width(&l) * k;
            let (gap, lead) = match n {
                0 => (0.0, 0.0),
                1 => (0.0, ((avail - tw) / 2.0).max(0.0)),
                _ => (((avail - tw) / (n - 1) as f64).max(0.0), 0.0),
            };
            let _ = write!(
                inner,
                "<div style=\"padding-left:{:.1}px;letter-spacing:{gap:.2}px\">{}</div>",
                PAD + ind + lead,
                scaled(esc(&l), "left")
            );
        }
        let _ = write!(
            out,
            "<div class=\"rc-t rc-w\" style=\"{fcss}width:{bw:.0}px;height:{bh:.0}px;justify-content:{jc};padding:0\">{inner}</div>"
        );
        return out;
    }
    // 여러 줄 (줄 바꿈·양쪽 맞춤)
    if v.wrap {
        let ta = match v.h {
            HAlign::Right => "right",
            HAlign::Center | HAlign::CenterContinuous => "center",
            HAlign::Justify | HAlign::Distributed => "justify",
            _ => "left",
        };
        let jc = match v.v {
            VAlign::Top => "flex-start",
            VAlign::Center => "center",
            VAlign::Bottom => "flex-end",
        };
        let mut css = fcss;
        let _ = write!(
            css,
            "width:{bw:.0}px;height:{bh:.0}px;justify-content:{jc};text-align:{ta};white-space:pre-wrap;overflow-wrap:anywhere;"
        );
        if v.h == HAlign::Distributed {
            css.push_str("text-align-last:justify;");
        }
        match v.h {
            HAlign::Right if indent > 0.0 => {
                let _ = write!(css, "padding-right:{:.1}px;", PAD_R + indent);
            }
            HAlign::Left if indent > 0.0 => {
                let _ = write!(css, "padding-left:{:.1}px;", PAD + indent);
            }
            _ => {}
        }
        let _ = write!(
            out,
            "<div class=\"rc-t rc-w\" style=\"{css}\"><span style=\"white-space:inherit\">{}</span></div>",
            scaled(body, "left")
        );
        return out;
    }

    let avail = (bw - PAD - PAD_R - indent).max(1.0);
    // 서식의 `*x` 채움: 앞 글은 왼쪽, 뒤 글은 오른쪽 끝에 붙이고 사이를 x 로 채운다 (맞춤과 상관없이 —
    // 표시 형식 실험 `0*-`·`@*.`·회계 서식). `_x` 는 x 폭의 보이지 않는 글자
    if let Some((at, fc)) = v
        .fill_char
        .filter(|&(at, _)| at <= text.len() && v.text == text)
    {
        let (l, r) = text.split_at(at);
        let lp: Vec<(usize, char)> = v.pads.iter().copied().filter(|p| p.0 <= at).collect();
        let rp: Vec<(usize, char)> = v
            .pads
            .iter()
            .filter(|p| p.0 > at)
            .map(|&(p, c)| (p - at, c))
            .collect();
        let n = (avail / v.m.char_width(fc).max(1.0)).ceil() as usize + 1;
        let filler = esc(&fc.to_string().repeat(n));
        let _ = write!(
            out,
            "<div class=\"rc-t\" style=\"left:0;width:{bw:.0}px;height:{bh:.0}px;{fcss}\"><span style=\"left:{:.1}px;width:{avail:.1}px;{vpos};transform:translateY({vshift});display:flex\"><span>{}</span><span style=\"flex:1 1 0;overflow:hidden\">{filler}</span><span>{}</span></span></div>",
            PAD + indent,
            with_pads(l, &lp),
            with_pads(r, &rp)
        );
        return out;
    }
    let mut body = if v.pads.is_empty() || v.rich.is_some() || v.text != text {
        body
    } else {
        with_pads(&text, &v.pads)
    };
    // 줄여 맞춤: 글꼴을 칸 너비에 맞게 줄인다
    if v.shrink {
        let tw = v.text_px();
        if tw > avail {
            let k = (avail / tw).max(0.05);
            let size = v.font.and_then(|f| f.size).unwrap_or(metrics::DEFAULT_SIZE) * k;
            let _ = write!(fcss, "font-size:{size:.2}pt;");
        }
    }
    // 채움: 칸 너비만큼 글자를 되풀이한다
    if v.h == HAlign::Fill && matches!(v.value, CellValue::Text(_)) {
        let tw = v.m.text_width(&text).max(1.0);
        let n = ((avail / tw).floor() as usize).max(1);
        body = esc(&text.repeat(n));
    }
    // 균등 분할(한 줄): 글자 사이를 칸 너비에 맞춰 벌린다
    if matches!(v.h, HAlign::Distributed | HAlign::Justify) {
        let n = text.chars().count();
        let tw = v.text_px();
        let gap = if n > 1 && tw < avail {
            (avail - tw) / (n - 1) as f64
        } else {
            0.0
        };
        let _ = write!(
            out,
            "<div class=\"rc-t\" style=\"left:0;width:{bw:.0}px;height:{bh:.0}px;{fcss}\"><span style=\"left:{PAD}px;{vpos};transform:translateY({vshift});letter-spacing:{gap:.2}px\">{body}</span></div>"
        );
        return out;
    }

    // 한 줄 글자: 보이는 상자(칸 + 넘친 범위) 안에 절대 위치
    let (left, right) = overflow;
    let box_w = bw + left as f64 + right as f64;
    let (pos, tx, origin) = match v.h {
        HAlign::Right => (
            format!("right:{:.1}px", right as f64 + PAD_R + indent),
            "0",
            "right",
        ),
        HAlign::Center | HAlign::CenterContinuous => {
            // 선택 영역 가운데는 이어진 빈 칸까지 한 칸처럼 (넘친 오른쪽 폭이 그 범위)
            let center = if v.h == HAlign::CenterContinuous {
                (bw + right as f64) / 2.0
            } else {
                left as f64 + bw / 2.0
            };
            (format!("left:{center:.1}px"), "-50%", "center")
        }
        _ => (
            format!("left:{:.1}px", left as f64 + PAD + indent),
            "0",
            "left",
        ),
    };
    let ov = if left > 0 || right > 0 { " rc-ov" } else { "" };
    let _ = write!(
        out,
        "<div class=\"rc-t{ov}\" style=\"left:-{left}px;width:{box_w:.0}px;height:{bh:.0}px;{fcss}\"><span style=\"{pos};{vpos};transform:translate({tx},{vshift})\">{}</span></div>",
        scaled(body, origin)
    );
    out
}

/// 서식의 `_x` 빈 자리를 x 폭의 보이지 않는 글자로 넣은 HTML
fn with_pads(text: &str, pads: &[(usize, char)]) -> String {
    let mut s = String::new();
    let mut last = 0;
    for &(p, x) in pads {
        if p < last || p > text.len() || !text.is_char_boundary(p) {
            continue;
        }
        s.push_str(&esc(&text[last..p]));
        let _ = write!(
            s,
            "<span style=\"visibility:hidden\">{}</span>",
            esc(&x.to_string())
        );
        last = p;
    }
    s.push_str(&esc(&text[last..]));
    s
}

/// 세로 쓰기 글자 간격 (px) — 한셀 화면에서 글자 크기 8–28pt 마다 잰 값 (세로 쓰기 간격 실험: 글꼴 8종 × 크기 21).
/// 간격은 글꼴이 아니라 계열만 따른다 — 맑은 고딕과 라틴 글꼴(Arial·Calibri·Times New Roman)은 같은 표, 바탕·굴림·돋움·궁서는
/// 같은 표. 글자 종류(한글·라틴)와도 상관없다. 16pt 맑은 고딕 35.2px 는 줄 높이(31.2px)보다 크고 17pt(33.2px)보다도 크다
const VERT_PITCH: [f64; 21] = [
    16.2, 16.8, 18.4, 21.8, 25.4, 26.6, 28.6, 30.8, 35.2, 33.2, 35.2, 37.0, 38.8, 40.8, 42.8, 44.6,
    46.4, 48.2, 50.4, 52.25, 54.0,
];
const VERT_PITCH_KR: [f64; 21] = [
    14.6, 15.6, 16.6, 18.2, 19.2, 24.0, 25.6, 26.0, 27.6, 30.2, 30.2, 31.8, 33.4, 35.0, 36.6, 38.4,
    39.8, 41.6, 43.0, 44.8, 46.2,
];

/// 글꼴·크기(pt)의 세로 쓰기 글자 간격 — 표 사이 크기는 선형 보간, 밖은 비례
fn vert_pitch(font: &str, size: f64) -> f64 {
    let kr = matches!(
        font.trim_end_matches('체'),
        "바탕" | "굴림" | "돋움" | "궁서" | "Batang" | "Gulim" | "Dotum" | "Gungsuh"
    ) || matches!(font, "BatangChe" | "GulimChe" | "DotumChe" | "GungsuhChe");
    let t = if kr { &VERT_PITCH_KR } else { &VERT_PITCH };
    if size <= 8.0 {
        return t[0] * size / 8.0;
    }
    if size >= 28.0 {
        return t[20] * size / 28.0;
    }
    let i = (size - 8.0).floor() as usize;
    let f = size - 8.0 - i as f64;
    t[i] + (t[i + 1] - t[i]) * f
}

/// 돌린 글자의 여백 (px): 왼쪽, 오른쪽, 위, 아래 (회전 실험 — 10 각도 × 맞춤 12가지)
const ROT_PAD: (f64, f64, f64, f64) = (2.0, 2.0, 2.0, 2.0);
/// 세로 쓰기 여백 (px): 왼쪽, 오른쪽, 위, 아래
const VERT_PAD: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, -1.5);

/// 돌린 글자 (1–90: 반시계, 91–180: 시계 방향 rot−90), 세로 쓰기 (255).
/// 한셀 화면 (회전 실험): 글 한 줄 상자(글 폭 × 줄 높이)를 돌린 경계 상자를 칸 맞춤대로 놓는다. 세로 쓰기는 글자마다
/// 줄 높이 한 칸(폭도 줄 높이)을 위에서 아래로 쌓고, 칸을 넘치면 잘린다
fn rotated(v: &CellView, fcss: &str, body: &str, bw: f64, bh: f64, line: f64) -> String {
    let frame = format!("width:{bw:.0}px;height:{bh:.0}px;{fcss}");
    if v.rotation == 255 {
        let (pl, pr, pt, pb) = VERT_PAD;
        let chars: Vec<char> = match v.rich {
            Some(rt) if rt.text == v.text => rt.text.chars().collect(),
            _ => v.text.chars().collect(),
        };
        let n = chars.len().max(1) as f64;
        let col_w = line;
        // 글자 사이 간격은 글자 크기의 표 값 (줄 높이와 다르다 — 16pt 맑은 고딕 35.2px, 줄 높이 31.2px)
        let pitch = v
            .font
            .map(|f| {
                vert_pitch(
                    f.name.as_deref().unwrap_or(metrics::DEFAULT_FONT),
                    f.size.unwrap_or(metrics::DEFAULT_SIZE),
                )
            })
            .unwrap_or(line);
        let x = match v.h {
            HAlign::Right => bw - pr - col_w,
            HAlign::Center | HAlign::CenterContinuous | HAlign::Distributed | HAlign::Justify => {
                (bw - col_w) / 2.0 + 1.0
            }
            _ => pl,
        };
        // 간격이 줄 높이보다 큰 만큼은 글자 위에 얹힌다 (16pt: 글자가 한셀에서 2px 아래)
        let extra = (pitch - line).max(0.0) / 2.0;
        let y = extra
            + match v.v {
                VAlign::Top => pt,
                VAlign::Center => (bh - n * pitch) / 2.0 + 3.0,
                VAlign::Bottom => bh - pb - n * pitch,
            };
        let mut s = String::new();
        for ch in chars {
            let _ = write!(s, "<span>{}</span>", esc(&ch.to_string()));
        }
        return format!(
            "<div class=\"rc-t\" style=\"{frame}\"><span class=\"rc-vert\" style=\"left:{x:.1}px;top:{y:.1}px;width:{col_w:.1}px;line-height:{pitch:.2}px\">{s}</span></div>"
        );
    }
    let deg = if v.rotation > 90 {
        -(v.rotation - 90)
    } else {
        v.rotation
    } as f64;
    let (pl, pr, pt, pb) = ROT_PAD;
    let w = v.text_px();
    let th = deg.to_radians();
    let (c, s) = (th.cos().abs(), th.sin().abs());
    let (bbw, bbh) = (w * c + line * s, w * s + line * c);
    // 가로: 상자가 칸보다 넓어도 맞춤대로 놓고 (오른쪽은 칸 오른쪽 끝에 맞춰 왼쪽으로 삐져나간다) 가로로는 자르지 않는다
    // (이웃 칸이 비었으면 글이 그쪽으로 보인다 — 한셀 화면, 칸 폭 86px 에 상자 178px)
    let x0 = match v.h {
        HAlign::Right => bw - pr - bbw,
        HAlign::Center | HAlign::CenterContinuous | HAlign::Distributed | HAlign::Justify => {
            (bw - bbw) / 2.0
        }
        _ => pl,
    };
    // 세로: 상자가 칸보다 크면 칸 높이만큼만 보인다 (위·아래로 자른다). 보이는 쪽은 한셀 화면에서 잰 값 (회전 실험 — 글 11 개,
    // 칸 높이 27–96px, 각도 ±30·45·60·90). 글이 시작하는 쪽(반시계는 아래, 시계는 위)에 맞춘 글은 그 끝에 붙는다.
    // 반대쪽에 맞춘 글과 가운데 맞춘 글은 가운데에서 시작 쪽으로 t 만큼 간다 — 각도 30·45·60·90 에서
    // 반대쪽 0·.4·.8·1, 가운데 .6(시계 .5)·.9(.7)·1(.85)·1
    let tall = bbh > bh - pt - pb;
    let ccw = deg > 0.0;
    let at_start = if ccw {
        v.v == VAlign::Bottom
    } else {
        v.v == VAlign::Top
    };
    let t = if !tall {
        0.0
    } else if at_start {
        1.0
    } else {
        let table = if v.v == VAlign::Center {
            if ccw {
                [0.6, 0.9, 1.0, 1.0]
            } else {
                [0.5, 0.7, 0.85, 1.0]
            }
        } else {
            [0.0, 0.4, 0.8, 1.0]
        };
        let d = deg.abs().clamp(30.0, 90.0);
        let (i, f) = if d <= 45.0 {
            (0, (d - 30.0) / 15.0)
        } else if d <= 60.0 {
            (1, (d - 45.0) / 15.0)
        } else {
            (2, (d - 60.0) / 30.0)
        };
        table[i] + (table[i + 1] - table[i]) * f
    };
    // 한셀 글자 자리와의 차이를 줄 때 쓸 세로 맞춤
    let va = if !tall {
        v.v
    } else if t > 0.5 {
        if ccw { VAlign::Bottom } else { VAlign::Top }
    } else {
        VAlign::Center
    };
    // 한셀 글자 자리와의 차이 (회전 실험의 잉크 상자 — 맑은 고딕 9·11pt, 바탕·Arial 11pt): 90도가 아닌 각도는
    // 경계 상자보다 위쪽, 시계 방향은 더 위
    let dy = match (deg as i32, va) {
        (90, VAlign::Top) => 0.0,
        (90, VAlign::Center) => -1.5,
        (90, VAlign::Bottom) => -3.0,
        (-90, VAlign::Top) => 2.0,
        (-90, VAlign::Center) => 1.5,
        (-90, VAlign::Bottom) => -2.5,
        (d, VAlign::Top) if d > 0 => -3.5,
        (d, VAlign::Center) if d > 0 => -1.0,
        (d, VAlign::Bottom) if d > 0 => 2.0,
        (_, VAlign::Top) => -5.0,
        (_, VAlign::Center) => -2.0,
        (_, VAlign::Bottom) => -1.5,
    };
    let (yc, yb) = ((bh - bbh) / 2.0, bh - pb - bbh);
    let y0 = dy
        + if tall {
            // 가운데에서 시작 쪽 끝(반시계 아래, 시계 위)으로 t 만큼
            yc + t * (if ccw { yb } else { pt } - yc)
        } else {
            match va {
                VAlign::Top => pt,
                VAlign::Center => yc,
                VAlign::Bottom => yb,
            }
        };
    let (cx, cy) = (x0 + bbw / 2.0, y0 + bbh / 2.0);
    // 칸보다 넓은 상자는 위·아래로만 자른다 (옆 칸으로 삐져나간다)
    let frame = if bbw > bw - pl - pr {
        format!("{frame}overflow:visible;clip-path:inset(0 -3000px);")
    } else {
        frame
    };
    format!(
        "<div class=\"rc-t\" style=\"{frame}\"><span class=\"rc-rot\" style=\"left:{cx:.2}px;top:{cy:.2}px;line-height:{line:.2}px;transform:translate(-50%,-50%) rotate({}deg)\">{body}</span></div>",
        -deg
    )
}
