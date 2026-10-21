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
        if f
            .underline
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
    if let Some(cf) = cf
        && let Some((len, c)) = &cf.bar
        && let Some(c) = theme::resolve(c, theme)
    {
        let p = (len * 100.0).clamp(0.0, 100.0);
        let _ = write!(
            out,
            "<div class=\"rc-bar\" style=\"width:calc({p:.1}% - 3px);background:linear-gradient(to right,{c},{c}33)\"></div>"
        );
    }
    let mut text = v.text.clone();
    if cf.is_some_and(|c| c.hide_value) {
        text.clear();
    }
    let icon = cf.and_then(|c| c.icon.as_ref()).map(|(set, level, n)| {
        let (g, color) = super::icon_glyph(set, *level, *n);
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
    // 줄 높이 = 글꼴의 한 줄 높이 (행 자동 높이와 같은 값)
    let line = v.m.row_pt * 4.0 / 3.0;
    let _ = write!(fcss, "line-height:{line:.2}px;");
    let indent = v.indent as f64 * INDENT_PX;
    let ratio = ratio_of(v.font);
    let scaled = |inner: String, origin: &str| match ratio {
        Some(r) => format!(
            "<span style=\"display:inline-block;transform:scaleX({r:.3});transform-origin:{origin}\">{inner}</span>"
        ),
        None => inner,
    };
    // 보이는 칸 상자 (격자선 제외)
    let (bw, bh) = (w.saturating_sub(1).max(1) as f64, h.saturating_sub(1).max(1) as f64);
    let vpos = match v.v {
        VAlign::Top => "top:0".to_string(),
        VAlign::Center => format!("top:{:.1}px", bh / 2.0),
        VAlign::Bottom => "bottom:0".to_string(),
    };
    let vshift = if v.v == VAlign::Center { "-50%" } else { "0" };
    let body = rich_html(v, &text, theme, icon.as_deref());

    // 돌린 글자·세로 쓰기
    if v.rotation != 0 {
        return out + &rotated(v, &fcss, &body, bw, bh, line);
    }
    // 여러 줄 (줄 바꿈·양쪽 맞춤·균등 분할)
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
            "<div class=\"rc-t rc-w\" style=\"{css}\">{}</div>",
            scaled(body, "left")
        );
        return out;
    }

    let avail = (bw - PAD - PAD_R - indent).max(1.0);
    let mut body = body;
    // 줄여 맞춤: 글꼴을 칸 너비에 맞게 줄인다
    if v.shrink {
        let tw = v.text_px();
        if tw > avail {
            let k = (avail / tw).max(0.05);
            let size = v
                .font
                .and_then(|f| f.size)
                .unwrap_or(metrics::DEFAULT_SIZE)
                * k;
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

/// 돌린 글자 (1–90: 반시계, 91–180: 시계 방향 rot−90), 세로 쓰기 (255)
fn rotated(v: &CellView, fcss: &str, body: &str, bw: f64, bh: f64, line: f64) -> String {
    let ai = match v.h {
        HAlign::Left => "flex-start",
        HAlign::Center | HAlign::CenterContinuous => "center",
        HAlign::Right => "flex-end",
        _ => "center",
    };
    let jc = match v.v {
        VAlign::Top => "flex-start",
        VAlign::Center => "center",
        VAlign::Bottom => "flex-end",
    };
    let frame = format!("width:{bw:.0}px;height:{bh:.0}px;{fcss}");
    if v.rotation == 255 {
        // 세로 쓰기: 글자를 위에서 아래로 쌓는다
        return format!(
            "<div class=\"rc-t rc-flex\" style=\"{frame}justify-content:{jc};align-items:{ai}\"><span class=\"rc-vert\" style=\"line-height:{line:.2}px\">{body}</span></div>"
        );
    }
    let deg = if v.rotation > 90 {
        -(v.rotation - 90)
    } else {
        v.rotation
    };
    if deg.abs() == 90 {
        // 90도: 아래에서 위로(반시계) 또는 위에서 아래로(시계)
        let wm = if deg > 0 {
            "writing-mode:vertical-rl;transform:rotate(180deg)"
        } else {
            "writing-mode:vertical-rl"
        };
        let jc90 = match (v.v, deg > 0) {
            (VAlign::Top, _) => "flex-start",
            (VAlign::Center, _) => "center",
            (VAlign::Bottom, _) => "flex-end",
        };
        return format!(
            "<div class=\"rc-t rc-flex\" style=\"{frame}justify-content:{jc90};align-items:{ai};padding:{PAD}px 0\"><span style=\"{wm};line-height:{line:.2}px;white-space:pre\">{body}</span></div>"
        );
    }
    // 그 밖의 각도: 글자 기준선의 시작점을 축으로 돌린다. 반시계면 왼쪽 아래, 시계면 왼쪽 위에서 시작
    let (x, y, origin, ty) = if deg > 0 {
        (PAD, bh - PAD, "left bottom", "-100%")
    } else {
        (PAD, PAD, "left top", "0")
    };
    format!(
        "<div class=\"rc-t\" style=\"{frame}\"><span class=\"rc-rot\" style=\"left:{x:.1}px;top:{y:.1}px;transform-origin:{origin};transform:translateY({ty}) rotate({}deg)\">{body}</span></div>",
        -deg
    )
}
