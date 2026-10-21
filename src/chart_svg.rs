// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 차트를 SVG 로 그린다 (뷰어). 한셀 화면을 따른다 (차트 갈무리 대조, docs/format/screen.md 6절):
//!
//! - 지정이 없을 때의 모양: 차트 영역 흰 바탕 + `#868686` 1px 테두리, 축 선 `#868686` 1px, 주 눈금 바깥 5px,
//!   글자 검정 9pt, 제목 14pt 굵게. 계열 색은 지정 색 → 테마 강조 1–6 (여섯 넘으면 어둡게·밝게 바꿔 돈다)
//! - 값 축 자동 범위: 0 을 포함하고, 최댓값에 5% 여유를 두어 주 단위로 올린다 (60 → 70, 130 → 140)
//! - 묶음: 막대(세로·가로, 묶은·누적·100% 누적, 간격·겹침, 음수 반전), 꺾은선(표식 — 자동이면 계열마다
//!   마름모·네모·세모 …, 부드러운 선, 누적), 영역, 원형·도넛(첫 조각 각도, 구멍, 쪼개기), 분산형, 거품형, 방사형,
//!   주식형(고저선), 혼합 차트와 보조 축
//! - 축: 범위·주 단위·거꾸로·로그, 교차 위치, 눈금 모양, 레이블 위치·표시 형식, 축 선·눈금선 서식, 축 제목.
//!   데이터 레이블(값·항목·계열·백분율), 범례(위치·순서·직접 배치), 그림 영역 직접 배치
//! - 3차원 차트는 평면으로 그린다

use std::fmt::Write as _;

use crate::model::{
    Chart, ChartAxis, ChartDataLabels, ChartFill, ChartFont, ChartGroupProps, ChartLayout,
    ChartLine, ChartMarker, ChartRich, ChartSeries, ChartShapeProps, DrawColor,
};
use crate::theme::Theme;
use crate::writer::esc;

type ColorFn<'a> = &'a dyn Fn(&DrawColor) -> Option<String>;

/// 축 선·눈금의 기본 색 (한셀 화면)
const AXIS_GRAY: &str = "#868686";
/// 기본 글자 크기 (pt)
const BASE_PT: f64 = 9.0;
/// 꺾은선 기본 굵기 (px, 2.25pt)
const LINE_PX: f64 = 3.0;
/// 차트 영역 안쪽 여백 (px, 왼쪽·오른쪽·위·아래 — 한셀 화면: 값 레이블이 틀에서 12.5px (craft-charts 15개 모두
/// 글자 잉크가 틀 +13px), 그림 영역 오른쪽 끝이 8px, 맨 위 눈금 레이블이 16px, 항목 레이블 아래가 16px)
const PAD: (f64, f64, f64, f64) = (12.5, 8.0, 16.0, 16.0);
/// 값 레이블 오른쪽 끝 → 축 (px)
const VAL_GAP: f64 = 14.0;
/// 범례: 줄 간격, 표지 크기, 표지 → 글, 글 끝 → 틀, 그림 영역 → 표지 (px).
/// 선 표지(꺾은선·분산형)는 16px 선, 선 끝 → 글 1.5px, 글 끝 → 틀 11.5px, 그림 영역 → 선 9.5px (chart_line·chart_scatter)
const LEG_ROW: f64 = 18.0;
const LEG_KEY: f64 = 8.0;
const LEG_KEY_GAP: f64 = 5.0;
const LEG_RIGHT: f64 = 12.5;
const LEG_PLOT_GAP: f64 = 15.0;
const LEG_LINE: (f64, f64, f64, f64) = (16.0, 1.5, 11.5, 9.5);

/// 차트 하나를 `w × h` 픽셀 SVG 로.
pub fn chart_svg(ch: &Chart, w: f64, h: f64, theme: &Theme, color: ColorFn) -> String {
    let cx = Ctx { theme, color };
    let mut out = Svg::new(w, h);
    // 차트 영역
    let area = cx.shape(ch.sp_pr.as_ref(), Some("#FFFFFF"), Some((AXIS_GRAY, 1.0)));
    out.rect(0.5, 0.5, w - 1.0, h - 1.0, &area);
    let base_font = cx.font(ch.tx_pr.as_ref(), None, BASE_PT, false);
    // 안쪽 영역 (그림 영역 + 축 레이블)
    let mut inner = Rect {
        x: PAD.0,
        y: PAD.2,
        w: w - PAD.0 - PAD.1,
        h: h - PAD.2 - PAD.3,
    };

    // 제목: 위 가운데 (한셀 화면: 14pt 굵게, 글자 윗변이 틀에서 14px, 그 아래 그림 영역)
    if let Some((text, font)) = title_of(ch, &cx, &base_font) {
        let lines: Vec<&str> = text.split('\n').collect();
        let lh = font.px() * 1.25;
        let manual = ch
            .title_full
            .as_ref()
            .and_then(|t| t.layout.as_ref())
            .and_then(manual_xy);
        let (tx, ty) = match manual {
            Some((x, y)) => (x * w, y * h),
            None => (w / 2.0, 10.0),
        };
        for (i, l) in lines.iter().enumerate() {
            out.text(
                tx,
                ty + lh * i as f64 + font.px() * 0.95,
                l,
                &font,
                "middle",
            );
        }
        if ch.title_full.as_ref().and_then(|t| t.overlay) != Some(true) {
            let bottom = ty + lh * lines.len() as f64 + 8.0;
            inner.h -= bottom - inner.y;
            inner.y = bottom;
        }
    }

    let groups = groups_of(ch);
    let pie_like = groups
        .first()
        .is_some_and(|g| matches!(g.kind, "pie" | "doughnut"));

    // 범례
    if let Some(pos) = ch.legend.as_deref() {
        let font = cx.font(ch.legend_props.tx_pr.as_ref(), None, base_font.size, false);
        let items = legend_items(ch, &groups, &cx, pie_like);
        if !items.is_empty() {
            inner = draw_legend(&mut out, ch, pos, &items, &font, inner, w, h, &cx);
        }
    }

    if pie_like {
        draw_pies(&mut out, ch, &groups, &cx, &base_font, inner);
    } else if groups.first().is_some_and(|g| g.kind == "radar") {
        draw_radar(&mut out, ch, &groups, &cx, &base_font, inner);
    } else if !groups.is_empty() {
        draw_axis_chart(&mut out, ch, &groups, &cx, &base_font, inner, w, h);
    }
    out.finish()
}

// ---------------------------------------------------------------------------
// SVG 쓰기
// ---------------------------------------------------------------------------

struct Svg {
    s: String,
    defs: String,
    clip: usize,
}

impl Svg {
    fn new(w: f64, h: f64) -> Svg {
        Svg {
            s: format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.1} {h:.1}\" font-family=\"'맑은 고딕','Malgun Gothic',sans-serif\">"
            ),
            defs: String::new(),
            clip: 0,
        }
    }
    fn finish(mut self) -> String {
        if !self.defs.is_empty() {
            let d = format!("<defs>{}</defs>", self.defs);
            let at = self.s.find('>').map_or(0, |i| i + 1);
            self.s.insert_str(at, &d);
        }
        self.s.push_str("</svg>");
        self.s
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, st: &Style) {
        if st.is_empty() || w <= 0.0 || h <= 0.0 {
            return;
        }
        let _ = write!(
            self.s,
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\"{}/>",
            st.attrs()
        );
    }
    fn path(&mut self, d: &str, st: &Style) {
        if st.is_empty() || d.is_empty() {
            return;
        }
        let _ = write!(self.s, "<path d=\"{d}\"{}/>", st.attrs());
    }
    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, st: &Stroke) {
        let _ = write!(
            self.s,
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\"{}/>",
            st.attrs()
        );
    }
    fn text(&mut self, x: f64, y: f64, t: &str, f: &Font, anchor: &str) {
        if t.is_empty() {
            return;
        }
        let _ = write!(
            self.s,
            "<text x=\"{x:.1}\" y=\"{y:.1}\" text-anchor=\"{anchor}\"{}>{}</text>",
            f.attrs(),
            esc(t)
        );
    }
    fn text_rot(&mut self, x: f64, y: f64, t: &str, f: &Font, deg: f64) {
        let _ = write!(
            self.s,
            "<text x=\"{x:.1}\" y=\"{y:.1}\" text-anchor=\"middle\" transform=\"rotate({deg:.0} {x:.1} {y:.1})\"{}>{}</text>",
            f.attrs(),
            esc(t)
        );
    }
    /// 그림 영역 안으로 자르기 시작 (끝은 `end_clip`)
    fn begin_clip(&mut self, r: &Rect) {
        self.clip += 1;
        let id = format!("rcc{}x{:.0}{:.0}", self.clip, r.x * 3.0, r.y * 7.0);
        let _ = write!(
            self.defs,
            "<clipPath id=\"{id}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\"/></clipPath>",
            r.x - 0.5,
            r.y - 0.5,
            r.w + 1.0,
            r.h + 1.0
        );
        let _ = write!(self.s, "<g clip-path=\"url(#{id})\">");
    }
    fn end_clip(&mut self) {
        self.s.push_str("</g>");
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Debug, Clone, Default)]
struct Stroke {
    color: String,
    width: f64,
    dash: Option<String>,
}

impl Stroke {
    fn new(color: &str, width: f64) -> Stroke {
        Stroke {
            color: color.into(),
            width,
            dash: None,
        }
    }
    fn attrs(&self) -> String {
        let mut a = format!(
            " stroke=\"{}\" stroke-width=\"{:.2}\" fill=\"none\"",
            self.color, self.width
        );
        if let Some(d) = &self.dash {
            let _ = write!(a, " stroke-dasharray=\"{d}\"");
        }
        a
    }
}

#[derive(Debug, Clone, Default)]
struct Style {
    fill: Option<(String, f64)>,
    stroke: Option<Stroke>,
}

impl Style {
    fn is_empty(&self) -> bool {
        self.fill.is_none() && self.stroke.is_none()
    }
    fn fill(c: &str) -> Style {
        Style {
            fill: Some((c.into(), 1.0)),
            stroke: None,
        }
    }
    fn attrs(&self) -> String {
        let mut a = match &self.fill {
            Some((c, o)) if *o < 1.0 => format!(" fill=\"{c}\" fill-opacity=\"{o:.3}\""),
            Some((c, _)) => format!(" fill=\"{c}\""),
            None => " fill=\"none\"".to_string(),
        };
        if let Some(s) = &self.stroke {
            let _ = write!(a, " stroke=\"{}\" stroke-width=\"{:.2}\"", s.color, s.width);
            if let Some(d) = &s.dash {
                let _ = write!(a, " stroke-dasharray=\"{d}\"");
            }
        }
        a
    }
}

#[derive(Debug, Clone)]
struct Font {
    /// pt
    size: f64,
    bold: bool,
    italic: bool,
    color: String,
    family: Option<String>,
    /// 1/60000 도 (`a:bodyPr/@rot`)
    rot: Option<i64>,
}

impl Font {
    fn px(&self) -> f64 {
        self.size * 96.0 / 72.0
    }
    fn attrs(&self) -> String {
        let mut a = format!(" font-size=\"{:.1}px\" fill=\"{}\"", self.px(), self.color);
        if self.bold {
            a.push_str(" font-weight=\"bold\"");
        }
        if self.italic {
            a.push_str(" font-style=\"italic\"");
        }
        if let Some(f) = &self.family {
            let _ = write!(
                a,
                " font-family=\"'{}','맑은 고딕','Malgun Gothic',sans-serif\"",
                f.replace(['\'', '"', '<', '>', '&'], "")
            );
        }
        a
    }
    /// 글자열 폭 어림 (px): 한글·한자 1em, 숫자 0.5em (한셀 축 배치: 9pt 숫자 하나가 6px — chart_range12 의 100 이 70 보다 6px 넓다), 그 밖 0.5em
    fn width(&self, t: &str) -> f64 {
        let em = self.px() * if self.bold { 1.05 } else { 1.0 };
        t.chars()
            .map(|c| {
                if crate::render::is_wide(c) {
                    1.0
                } else if c.is_ascii_digit() {
                    0.5
                } else if c == ' ' {
                    0.3
                } else if c.is_ascii_uppercase() {
                    0.64
                } else {
                    0.5
                }
            })
            .sum::<f64>()
            * em
    }
}

// ---------------------------------------------------------------------------
// 서식 해석
// ---------------------------------------------------------------------------

struct Ctx<'a> {
    theme: &'a Theme,
    color: ColorFn<'a>,
}

impl Ctx<'_> {
    fn chart_color(&self, c: &crate::model::ChartColor) -> Option<(String, f64)> {
        let alpha = c
            .mods
            .iter()
            .find(|(m, _)| m == "alpha")
            .map_or(1.0, |(_, v)| (*v as f64 / 100_000.0).clamp(0.0, 1.0));
        (self.color)(&c.to_draw()).map(|s| (s, alpha))
    }

    fn fill(&self, f: &ChartFill) -> Option<Option<(String, f64)>> {
        match f {
            ChartFill::None => Some(None),
            ChartFill::Solid(c) => Some(self.chart_color(c)),
            // 그라데이션·무늬: 첫 색으로 칠한다
            ChartFill::Other(xml) => Some(
                first_color_in(xml)
                    .and_then(|c| (self.color)(&c))
                    .map(|c| (c, 1.0)),
            ),
        }
    }

    fn stroke(&self, l: &ChartLine) -> Option<Option<Stroke>> {
        let color = match l.fill.as_ref()? {
            ChartFill::None => return Some(None),
            // 한셀 차트 화면은 미리 정한 색(a:prstClr)의 선을 그리지 않는다 (레거시 차트의 범례 테두리 prstClr black 이
            // 한셀 화면에 없다 — craft-charts 레거시 15개)
            ChartFill::Solid(c) if c.value.starts_with("prst:") => return Some(None),
            f => self.fill(f)?.map(|c| c.0)?,
        };
        let width = l.width.map_or(1.0, |w| (w as f64 / 9525.0).max(0.75));
        let dash = l
            .dash
            .as_deref()
            .and_then(|d| crate::render::dash_pattern(d, width));
        Some(Some(Stroke { color, width, dash }))
    }

    /// 도형 속성 → 채우기·선. 지정이 없으면 기본값
    fn shape(
        &self,
        sp: Option<&ChartShapeProps>,
        fill: Option<&str>,
        line: Option<(&str, f64)>,
    ) -> Style {
        let f = match sp.and_then(|p| p.fill.as_ref()).and_then(|f| self.fill(f)) {
            Some(v) => v,
            None => fill.map(|c| (c.to_string(), 1.0)),
        };
        let l = match sp
            .and_then(|p| p.line.as_ref())
            .and_then(|l| self.stroke(l))
        {
            Some(v) => v,
            None => line.map(|(c, w)| Stroke::new(c, w)),
        };
        Style { fill: f, stroke: l }
    }

    /// 선 서식만 (눈금선·축 선)
    fn line_of(
        &self,
        sp: Option<&ChartShapeProps>,
        default: Option<(&str, f64)>,
    ) -> Option<Stroke> {
        match sp
            .and_then(|p| p.line.as_ref())
            .and_then(|l| self.stroke(l))
        {
            Some(v) => v,
            None => default.map(|(c, w)| Stroke::new(c, w)),
        }
    }

    /// 글 서식: 글 상자(c:txPr/c:rich) 의 첫 문단 기본 글꼴 → 첫 조각 글꼴 → 기본값
    fn font(&self, rich: Option<&ChartRich>, parent: Option<&Font>, size: f64, bold: bool) -> Font {
        let mut f = Font {
            size: parent.map_or(size, |p| p.size),
            bold: parent.map_or(bold, |p| p.bold),
            italic: parent.is_some_and(|p| p.italic),
            color: parent.map_or_else(|| "#000000".to_string(), |p| p.color.clone()),
            family: parent.and_then(|p| p.family.clone()),
            rot: None,
        };
        if parent.is_none() {
            f.size = size;
            f.bold = bold;
        }
        let Some(r) = rich else { return f };
        f.rot = r.rot;
        let src: Option<&ChartFont> =
            r.paras
                .first()
                .and_then(|p| p.def_font.as_ref())
                .or_else(|| {
                    r.paras
                        .first()
                        .and_then(|p| p.runs.first())
                        .and_then(|x| x.font.as_ref())
                });
        if let Some(cf) = src {
            self.apply_font(&mut f, cf);
        }
        f
    }

    fn apply_font(&self, f: &mut Font, cf: &ChartFont) {
        if let Some(z) = cf.size {
            f.size = z as f64 / 100.0;
        }
        if let Some(b) = cf.bold {
            f.bold = b;
        }
        if let Some(i) = cf.italic {
            f.italic = i;
        }
        if let Some(c) = cf.fill.as_ref().and_then(|x| self.fill(x)).flatten() {
            f.color = c.0;
        }
        if let Some(n) = cf
            .ea
            .as_ref()
            .or(cf.latin.as_ref())
            .filter(|n| !n.is_empty() && !n.starts_with('+'))
        {
            f.family = Some(n.clone());
        }
    }

    /// 계열 색: 지정 색 → 테마 강조 1–6 (6 개마다 어둡게·밝게)
    fn palette(&self, i: usize) -> String {
        let base = self
            .theme
            .colors
            .get(4 + i % 6)
            .map(|c| c[c.len().saturating_sub(6)..].to_string())
            .unwrap_or_else(|| "4472C4".into());
        let tier = i / 6;
        if tier == 0 {
            return format!("#{}", base.to_ascii_uppercase());
        }
        let mods: Vec<(String, i64)> = match tier % 3 {
            1 => vec![("lumMod".into(), 60000)],
            2 => vec![("lumMod".into(), 80000), ("lumOff".into(), 20000)],
            _ => vec![("lumMod".into(), 50000)],
        };
        (self.color)(&DrawColor {
            value: format!("srgb:{base}"),
            mods,
        })
        .unwrap_or_else(|| format!("#{base}"))
    }
}

/// 원본 XML 조각(그라데이션 등)에서 첫 색 요소
fn first_color_in(xml: &str) -> Option<DrawColor> {
    let root = crate::xml::parse(xml, "fill").ok()?;
    let mut found = None;
    root.walk(&mut |e| {
        if found.is_none()
            && matches!(
                e.name.as_str(),
                "srgbClr" | "schemeClr" | "sysClr" | "prstClr"
            )
        {
            found = Some(crate::parser::drawing::color(e));
        }
    });
    found
}

/// 직접 배치(manualLayout)의 x·y 비율 (edge 모드만)
fn manual_xy(l: &ChartLayout) -> Option<(f64, f64)> {
    let m = l.manual.as_ref()?;
    Some((m.x?, m.y?))
}

/// 직접 배치의 사각형 (차트 크기에 대한 비율 → px)
fn manual_rect(l: Option<&ChartLayout>, w: f64, h: f64) -> Option<(Rect, bool)> {
    let m = l?.manual.as_ref()?;
    let (x, y, ww, hh) = (m.x?, m.y?, m.w?, m.h?);
    let inner = m.layout_target.as_deref() == Some("inner");
    Some((
        Rect {
            x: x * w,
            y: y * h,
            w: ww * w,
            h: hh * h,
        },
        inner,
    ))
}

/// 제목 글과 글꼴. 제목 요소가 있는데 글이 없으면 계열 하나일 때 그 이름 (자동 제목)
fn title_of(ch: &Chart, cx: &Ctx, base: &Font) -> Option<(String, Font)> {
    if ch.auto_title_deleted == Some(true) && ch.title.is_none() {
        return None;
    }
    let t = ch.title_full.as_ref()?;
    let text = ch.title.clone().or_else(|| {
        let all: Vec<&ChartSeries> = ch
            .series
            .iter()
            .chain(ch.more_groups.iter().flat_map(|g| g.series.iter()))
            .collect();
        if all.len() == 1 {
            all[0].name.clone()
        } else {
            Some("차트 제목".into())
        }
    })?;
    let mut font = cx.font(t.tx_pr.as_ref(), None, 14.0, true);
    font.family = font.family.or_else(|| base.family.clone());
    if let Some(crate::model::ChartTx::Rich(r)) = &t.tx {
        // 서식 있는 제목: 첫 조각의 글꼴
        if let Some(cf) = r.paras.first().and_then(|p| {
            p.runs
                .first()
                .and_then(|r| r.font.as_ref())
                .or(p.def_font.as_ref())
        }) {
            cx.apply_font(&mut font, cf);
        }
    }
    Some((text, font))
}

// ---------------------------------------------------------------------------
// 묶음
// ---------------------------------------------------------------------------

struct Group<'a> {
    kind: &'a str,
    three_d: bool,
    bar_dir: Option<&'a str>,
    grouping: &'a str,
    vary_colors: bool,
    props: &'a ChartGroupProps,
    series: &'a [ChartSeries],
    /// 이 묶음보다 앞선 묶음들의 계열 수 (색 순서)
    offset: usize,
}

fn groups_of(ch: &Chart) -> Vec<Group<'_>> {
    let mut out = Vec::new();
    if !ch.kind.is_empty() {
        out.push(Group {
            kind: ch.kind.as_str(),
            three_d: ch.three_d,
            bar_dir: ch.bar_dir.as_deref(),
            grouping: ch.grouping.as_deref().unwrap_or("clustered"),
            vary_colors: ch.vary_colors,
            props: &ch.group,
            series: &ch.series,
            offset: 0,
        });
    }
    let mut off = ch.series.len();
    for g in &ch.more_groups {
        out.push(Group {
            kind: g.kind.as_str(),
            three_d: g.three_d,
            bar_dir: g.bar_dir.as_deref(),
            grouping: g.grouping.as_deref().unwrap_or("clustered"),
            vary_colors: g.vary_colors,
            props: &g.props,
            series: &g.series,
            offset: off,
        });
        off += g.series.len();
    }
    out
}

/// 계열 하나의 기본 색 (지정 → 테마 순서)
fn series_color(s: &ChartSeries, i: usize, cx: &Ctx) -> String {
    if let Some(sp) = &s.sp_pr {
        if let Some(c) = sp.fill.as_ref().and_then(|f| cx.fill(f)).flatten() {
            return c.0;
        }
        if let Some(Some(st)) = sp.line.as_ref().and_then(|l| cx.stroke(l)) {
            return st.color;
        }
    }
    cx.palette(s.idx.map_or(i, |v| v as usize))
}

/// 요소 하나의 채우기 (데이터 요소 서식 → 계열 서식 → 색)
fn point_style(s: &ChartSeries, i: usize, pt: usize, cx: &Ctx, base: &str) -> Style {
    let dpt = s.d_pts.iter().find(|d| d.idx as usize == pt);
    let sp = dpt.and_then(|d| d.sp_pr.as_ref()).or(s.sp_pr.as_ref());
    let _ = i;
    cx.shape(sp, Some(base), None)
}

/// 계열 선 서식 (꺾은선·분산형): 지정 → 기본 굵기와 계열 색
fn series_stroke(s: &ChartSeries, color: &str, cx: &Ctx) -> Option<Stroke> {
    match s
        .sp_pr
        .as_ref()
        .and_then(|p| p.line.as_ref())
        .and_then(|l| cx.stroke(l))
    {
        Some(v) => v,
        None => Some(Stroke::new(color, LINE_PX)),
    }
}

/// 계열 값 (없는 점은 None)
fn values(s: &ChartSeries) -> &[Option<f64>] {
    &s.values
}

/// 분산형·거품형의 x 값: 항목이 숫자면 그 값, 아니면 1, 2, 3 …
fn x_values(s: &ChartSeries) -> Vec<f64> {
    let n = s.values.len().max(s.categories.len());
    let nums: Vec<Option<f64>> = s
        .categories
        .iter()
        .map(|c| c.trim().parse::<f64>().ok())
        .collect();
    if !nums.is_empty() && nums.iter().all(Option::is_some) {
        nums.into_iter().flatten().collect()
    } else {
        (1..=n).map(|i| i as f64).collect()
    }
}

// ---------------------------------------------------------------------------
// 범례
// ---------------------------------------------------------------------------

/// 범례 항목: (이름, 칠, 선, 표식)
struct LegendItem {
    name: String,
    fill: Option<String>,
    stroke: Option<Stroke>,
    marker: Option<String>,
}

fn legend_items(ch: &Chart, groups: &[Group], cx: &Ctx, pie_like: bool) -> Vec<LegendItem> {
    let mut items = Vec::new();
    if pie_like || (groups.len() == 1 && groups[0].vary_colors && groups[0].series.len() == 1) {
        // 원형(또는 요소마다 색): 항목마다
        if let Some(s) = groups.first().and_then(|g| g.series.first()) {
            let n = s.values.len().max(s.categories.len());
            for k in 0..n {
                let base = cx.palette(k);
                let st = point_style(s, 0, k, cx, &base);
                let name = s
                    .categories
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| (k + 1).to_string());
                items.push(LegendItem {
                    name,
                    fill: st.fill.map(|f| f.0).or(Some(base)),
                    stroke: None,
                    marker: None,
                });
            }
        }
        return items;
    }
    for g in groups {
        for (i, s) in g.series.iter().enumerate() {
            let k = g.offset + i;
            let color = series_color(s, k, cx);
            let name = s.name.clone().unwrap_or_else(|| format!("계열{}", k + 1));
            let item = match g.kind {
                "line" | "scatter" | "radar" => {
                    let st = series_stroke(s, &color, cx);
                    let marker = marker_of(s, g, k).map(|(m, _)| m);
                    LegendItem {
                        name,
                        fill: None,
                        stroke: st,
                        marker,
                    }
                }
                _ => {
                    let st = cx.shape(s.sp_pr.as_ref(), Some(&color), None);
                    LegendItem {
                        name,
                        fill: st.fill.map(|f| f.0),
                        stroke: st.stroke,
                        marker: None,
                    }
                }
            };
            items.push(item);
        }
    }
    // 누적 막대는 쌓인 순서(위가 먼저)로 보인다 (한셀 화면)
    let stacked_bar = groups.len() == 1
        && groups[0].kind == "bar"
        && groups[0].bar_dir != Some("bar")
        && matches!(groups[0].grouping, "stacked" | "percentStacked");
    if stacked_bar && matches!(ch.legend.as_deref(), Some("r" | "l" | "tr")) {
        items.reverse();
    }
    items
}

#[allow(clippy::too_many_arguments)]
fn draw_legend(
    out: &mut Svg,
    ch: &Chart,
    pos: &str,
    items: &[LegendItem],
    font: &Font,
    inner: Rect,
    w: f64,
    h: f64,
    cx: &Ctx,
) -> Rect {
    let fpx = font.px();
    let row = LEG_ROW * font.size / BASE_PT;
    let key = LEG_KEY;
    let has_line = items.iter().any(|i| i.stroke.is_some() && i.fill.is_none());
    let (kw, key_gap, right, plot_gap) = if has_line {
        LEG_LINE
    } else {
        (key, LEG_KEY_GAP, LEG_RIGHT, LEG_PLOT_GAP)
    };
    let item_w = |it: &LegendItem| kw + key_gap + font.width(&it.name);
    let vertical = matches!(pos, "r" | "l" | "tr");
    // 상자: 세로면 항목 줄들, 가로면 한 줄
    let (bw, bh) = if vertical {
        (
            items.iter().map(item_w).fold(0.0, f64::max),
            row * items.len() as f64,
        )
    } else {
        (
            items.iter().map(|i| item_w(i) + 16.0).sum::<f64>() - 16.0,
            row,
        )
    };
    let mut rest = inner;
    let (bx, by) = match manual_rect(ch.legend_props.layout.as_ref(), w, h) {
        Some((r, _)) => (r.x + 4.0, r.y + 2.0),
        None => match pos {
            "l" => {
                rest.x = inner.x + bw + plot_gap;
                rest.w -= bw + plot_gap;
                (inner.x, (h - bh) / 2.0)
            }
            "t" => {
                rest.y += bh + 12.0;
                rest.h -= bh + 12.0;
                ((w - bw) / 2.0, inner.y)
            }
            "b" => {
                // 한셀 화면: 범례 가운데가 틀 아래에서 17.5px, 그림 영역은 28px 위로
                rest.h -= 28.0 * font.size / BASE_PT;
                ((w - bw) / 2.0, h - 17.5 - bh / 2.0)
            }
            "tr" => {
                let x = w - right - bw;
                rest.w = x - plot_gap - inner.x;
                (x, inner.y)
            }
            _ => {
                // 오른쪽: 글 끝이 틀에서 11px, 세로 가운데는 틀 가운데, 그림 영역은 표지 15px 앞까지
                let x = w - right - bw;
                rest.w = x - plot_gap - inner.x;
                (x, (h - bh) / 2.0)
            }
        },
    };
    if ch.legend_props.overlay == Some(true) {
        rest = inner;
    }
    let frame = cx.shape(ch.legend_props.sp_pr.as_ref(), None, None);
    out.rect(bx - 4.0, by - 2.0, bw + 8.0, bh + 4.0, &frame);
    let mut x = bx;
    let mut y = by;
    for it in items {
        let cy = y + row / 2.0;
        if let Some(f) = &it.fill {
            let st = Style {
                fill: Some((f.clone(), 1.0)),
                stroke: it.stroke.clone(),
            };
            let kx = x + (kw - key) / 2.0;
            // 표지는 글 가운데보다 1px 아래 (한셀 화면)
            out.rect(
                kx.round() + 0.5,
                (cy - key / 2.0 + 1.0).round() + 0.5,
                key,
                key,
                &st,
            );
        } else if let Some(st) = &it.stroke {
            out.line(x, cy.round() + 0.5, x + kw, cy.round() + 0.5, st);
        }
        if let Some(m) = &it.marker {
            let c = it
                .stroke
                .as_ref()
                .map_or_else(|| "#000000".to_string(), |s| s.color.clone());
            draw_marker(out, m, x + kw / 2.0, cy, 7.0, &Style::fill(&c));
        }
        out.text(x + kw + key_gap, cy + fpx * 0.35, &it.name, font, "start");
        if vertical {
            y += row;
        } else {
            x += item_w(it) + 16.0;
        }
    }
    rest
}

// ---------------------------------------------------------------------------
// 표식
// ---------------------------------------------------------------------------

/// 계열의 표식 (모양, 크기 px). 꺾은선 묶음에 표식이 꺼져 있거나 `none` 이면 None.
/// 자동이면 계열 번호마다 마름모 → 네모 → 세모 → × → 별 → 원 → +
fn marker_of(s: &ChartSeries, g: &Group, k: usize) -> Option<(String, f64)> {
    const AUTO: [&str; 7] = [
        "diamond", "square", "triangle", "x", "star", "circle", "plus",
    ];
    let m: Option<&ChartMarker> = s.marker.as_ref();
    let sym = m.and_then(|m| m.symbol.as_deref());
    let size = m.and_then(|m| m.size).unwrap_or(7) as f64 * 1.1;
    match (g.kind, sym) {
        (_, Some("none")) => None,
        ("scatter", None) => match g.props.scatter_style.as_deref() {
            Some("line") | Some("smooth") => None,
            _ => Some((AUTO[k % AUTO.len()].to_string(), size)),
        },
        ("radar", None) => match g.props.radar_style.as_deref() {
            Some("marker") => Some((AUTO[k % AUTO.len()].to_string(), size)),
            _ => None,
        },
        ("line", None) if g.props.marker == Some(false) => None,
        (_, None) | (_, Some("auto")) => Some((AUTO[k % AUTO.len()].to_string(), size)),
        (_, Some(v)) => Some((v.to_string(), size)),
    }
}

fn draw_marker(out: &mut Svg, sym: &str, x: f64, y: f64, size: f64, st: &Style) {
    let r = size / 2.0;
    let d = match sym {
        "square" => format!("M{:.1},{:.1}h{size:.1}v{size:.1}h-{size:.1}Z", x - r, y - r),
        "diamond" => format!(
            "M{x:.1},{:.1}L{:.1},{y:.1}L{x:.1},{:.1}L{:.1},{y:.1}Z",
            y - r,
            x + r,
            y + r,
            x - r
        ),
        "triangle" => format!(
            "M{x:.1},{:.1}L{:.1},{:.1}L{:.1},{:.1}Z",
            y - r,
            x + r,
            y + r,
            x - r,
            y + r
        ),
        "circle" => format!(
            "M{:.1},{y:.1}A{r:.1},{r:.1} 0 1 1 {:.1},{y:.1}A{r:.1},{r:.1} 0 1 1 {:.1},{y:.1}Z",
            x - r,
            x + r,
            x - r
        ),
        "x" | "star" | "plus" | "dash" | "dot" => {
            let c = st
                .fill
                .as_ref()
                .map_or_else(|| "#000000".to_string(), |f| f.0.clone());
            let k = Stroke::new(&c, 1.5);
            match sym {
                "x" => {
                    out.line(x - r, y - r, x + r, y + r, &k);
                    out.line(x - r, y + r, x + r, y - r, &k);
                }
                "plus" => {
                    out.line(x - r, y, x + r, y, &k);
                    out.line(x, y - r, x, y + r, &k);
                }
                "star" => {
                    out.line(x - r, y - r, x + r, y + r, &k);
                    out.line(x - r, y + r, x + r, y - r, &k);
                    out.line(x, y - r, x, y + r, &k);
                }
                "dash" => out.line(x - r, y, x + r, y, &Stroke::new(&c, 2.0)),
                _ => out.rect(x - 1.0, y - 1.0, 2.0, 2.0, &Style::fill(&c)),
            }
            return;
        }
        _ => format!("M{:.1},{:.1}h{size:.1}v{size:.1}h-{size:.1}Z", x - r, y - r),
    };
    out.path(&d, st);
}

/// 표식 칠: 표식 서식 → 계열 색
fn marker_style(s: &ChartSeries, color: &str, cx: &Ctx) -> Style {
    let sp = s.marker.as_ref().and_then(|m| m.sp_pr.as_ref());
    cx.shape(sp, Some(color), Some((color, 1.0)))
}

// ---------------------------------------------------------------------------
// 축과 눈금
// ---------------------------------------------------------------------------

/// 값 축의 눈금: 범위와 주 단위
#[derive(Debug, Clone, Copy)]
struct Scale {
    min: f64,
    max: f64,
    major: f64,
    log: Option<f64>,
    reverse: bool,
}

impl Scale {
    /// 값 → 0..1 (거꾸로면 뒤집는다)
    fn t(&self, v: f64) -> f64 {
        let t = match self.log {
            Some(b) => {
                let f = |x: f64| x.max(f64::MIN_POSITIVE).log(b);
                (f(v) - f(self.min)) / (f(self.max) - f(self.min))
            }
            None => (v - self.min) / (self.max - self.min),
        };
        if self.reverse { 1.0 - t } else { t }
    }
    fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if let Some(b) = self.log {
            let mut v = self.min;
            while v <= self.max * 1.000001 && out.len() < 100 {
                out.push(v);
                v *= b;
            }
            return out;
        }
        if self.major <= 0.0 {
            return vec![self.min, self.max];
        }
        let mut v = self.min;
        while v <= self.max + self.major * 1e-9 && out.len() < 200 {
            out.push(if v.abs() < self.major * 1e-9 { 0.0 } else { v });
            v += self.major;
        }
        out
    }
}

/// 보기 좋은 주 단위 (1·2·5 × 10^n), 눈금 수가 `n` 개 남짓이 되게
fn nice_unit(range: f64, n: f64) -> f64 {
    if range <= 0.0 || !range.is_finite() {
        return 1.0;
    }
    let raw = range / n;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    mag * if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    }
}

/// 자동 범위: 0 을 포함하고, 끝에 `room` 비율 여유(2차원 5%, 3차원 0 — chart_col3d 의 20~60 이 0~60)를 두어 주 단위로 맞춘다. 축 지정값이 이긴다
fn make_scale(
    lo: f64,
    hi: f64,
    axis: Option<&ChartAxis>,
    len_px: f64,
    zero_based: bool,
    room: f64,
) -> Scale {
    let sc = axis.map(|a| &a.scaling);
    let reverse = sc.and_then(|s| s.orientation.as_deref()) == Some("maxMin");
    if let Some(b) = sc.and_then(|s| s.log_base).filter(|b| *b > 1.0) {
        let lo = lo.max(f64::MIN_POSITIVE);
        let min = sc
            .and_then(|s| s.min)
            .unwrap_or_else(|| b.powf(lo.log(b).floor()));
        let max = sc
            .and_then(|s| s.max)
            .unwrap_or_else(|| b.powf(hi.max(lo * b).log(b).ceil()));
        return Scale {
            min,
            max,
            major: b,
            log: Some(b),
            reverse,
        };
    }
    let (mut lo, mut hi) = (lo, hi);
    // 0 에서 먼 값만 있으면(폭이 끝값의 1/6 이하) 0 을 넣지 않는다 — 분산형 20~60 이 0~70 (chart_scatter)
    let zero_based =
        zero_based || (lo >= 0.0 && hi - lo > hi / 6.0) || (hi <= 0.0 && hi - lo > -lo / 6.0);
    if zero_based {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    let fixed_min = sc.and_then(|s| s.min);
    let fixed_max = sc.and_then(|s| s.max);
    let lo = fixed_min.unwrap_or(lo);
    let hi = fixed_max.unwrap_or(hi);
    // 눈금 사이가 40px 남짓
    let target = (len_px / 40.0).clamp(2.0, 10.0);
    // 주 단위는 여유를 빼고 고른다 (0~130 → 20 단위 0~140)
    let span = hi - lo;
    let major = axis
        .and_then(|a| a.major_unit)
        .filter(|u| *u > 0.0)
        .unwrap_or_else(|| nice_unit(span, target));
    let min = fixed_min.unwrap_or_else(|| {
        if lo >= 0.0 && zero_based {
            0.0
        } else {
            ((lo - (hi - lo) * room) / major).floor() * major
        }
    });
    let max = fixed_max.unwrap_or_else(|| {
        let m = ((hi + (hi - lo) * room) / major).ceil() * major;
        if hi <= 0.0 && zero_based { 0.0 } else { m }
    });
    Scale {
        min,
        max: if max > min { max } else { min + major },
        major,
        log: None,
        reverse,
    }
}

/// 눈금 레이블 글: 축 표시 형식 (원본 연결이면 계열 캐시의 형식) → 일반
fn tick_label(v: f64, axis: Option<&ChartAxis>, linked: Option<&str>, percent: bool) -> String {
    if percent && axis.and_then(|a| a.num_fmt.as_ref()).is_none() {
        return format!("{:.0}%", v * 100.0);
    }
    // 원본 연결 형식의 G/표준 은 일반 서식으로, 축에 정한 G/표준 은 표시 형식 그대로 (연호 G + 글자 — 한셀 화면의
    // 레거시 차트 값 축 레이블이 "/표준", craft-charts 레거시)
    let (code, from_link) = match axis.and_then(|a| a.num_fmt.as_ref()) {
        Some(f) if f.source_linked == Some(true) => {
            (linked.unwrap_or(&f.format_code).to_string(), true)
        }
        Some(f) => (f.format_code.clone(), false),
        None => (linked.unwrap_or("General").to_string(), true),
    };
    let code = if code.is_empty() || (from_link && code.eq_ignore_ascii_case("G/표준")) {
        "General".to_string()
    } else {
        code
    };
    let v = if v.abs() < 1e-12 { 0.0 } else { v };
    let shown = crate::numfmt::format(&crate::model::CellValue::Number(v), &code, false);
    if shown.text.is_empty() {
        crate::model::format_general(v)
    } else {
        shown.text
    }
}

/// 축 찾기 (번호)
fn axis_by_id(ch: &Chart, id: u32) -> Option<&ChartAxis> {
    ch.axes.iter().find(|a| a.id == id)
}

/// 묶음의 (항목 축, 값 축). 분산형·거품형은 (x 값 축, y 값 축)
fn group_axes<'a>(ch: &'a Chart, g: &Group) -> (Option<&'a ChartAxis>, Option<&'a ChartAxis>) {
    let ids = &g.props.ax_ids;
    let found: Vec<&ChartAxis> = ids.iter().filter_map(|&i| axis_by_id(ch, i)).collect();
    if matches!(g.kind, "scatter" | "bubble") {
        let mut vals = found.iter().filter(|a| a.kind == "valAx");
        let x = vals.next().copied();
        let y = vals.next().copied();
        return (x, y);
    }
    let cat = found
        .iter()
        .find(|a| matches!(a.kind.as_str(), "catAx" | "dateAx"))
        .copied()
        .or_else(|| {
            ch.axes
                .iter()
                .find(|a| matches!(a.kind.as_str(), "catAx" | "dateAx"))
        });
    let val = found
        .iter()
        .find(|a| a.kind == "valAx")
        .copied()
        .or_else(|| ch.axes.iter().find(|a| a.kind == "valAx"));
    (cat, val)
}

fn axis_visible(a: Option<&ChartAxis>) -> bool {
    a.is_none_or(|a| a.delete != Some(true))
}

fn labels_on(a: Option<&ChartAxis>) -> bool {
    axis_visible(a) && a.and_then(|a| a.tick_lbl_pos.as_deref()) != Some("none")
}

/// 값 축 하나의 범위를 정할 값들 (누적이면 항목마다 합)
fn value_range(groups: &[&Group]) -> (f64, f64, bool) {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut percent = false;
    for g in groups {
        let stacked = matches!(g.grouping, "stacked" | "percentStacked") && g.kind != "scatter";
        if g.grouping == "percentStacked" {
            percent = true;
        }
        let n = g.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        if stacked {
            for k in 0..n {
                let (mut pos, mut neg) = (0.0, 0.0);
                let mut total = 0.0;
                for s in g.series {
                    let v = s.values.get(k).copied().flatten().unwrap_or(0.0);
                    total += v.abs();
                    if v >= 0.0 { pos += v } else { neg += v }
                }
                if g.grouping == "percentStacked" && total > 0.0 {
                    pos /= total;
                    neg /= total;
                }
                lo = lo.min(neg);
                hi = hi.max(pos);
            }
        } else {
            for s in g.series {
                for v in values(s).iter().flatten() {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
                if g.kind == "stock" || g.props.hi_low_lines.is_some() {
                    continue;
                }
            }
        }
    }
    if !lo.is_finite() {
        (0.0, 1.0, percent)
    } else {
        (lo, hi, percent)
    }
}

// ---------------------------------------------------------------------------
// 축 차트 (막대·꺾은선·영역·분산형·거품형·주식형)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn draw_axis_chart(
    out: &mut Svg,
    ch: &Chart,
    groups: &[Group],
    cx: &Ctx,
    base: &Font,
    inner: Rect,
    w: f64,
    h: f64,
) {
    let first = &groups[0];
    let horizontal = first.kind == "bar" && first.bar_dir == Some("bar");
    let xy = matches!(first.kind, "scatter" | "bubble");
    let (cat_ax, val_ax) = group_axes(ch, first);
    // 보조 값 축: 첫 묶음과 다른 값 축을 쓰는 묶음
    let secondary: Vec<&Group> = groups
        .iter()
        .skip(1)
        .filter(|g| {
            let (_, v) = group_axes(ch, g);
            v.is_some() && val_ax.is_some() && v.map(|a| a.id) != val_ax.map(|a| a.id)
        })
        .collect();
    let primary: Vec<&Group> = groups
        .iter()
        .filter(|g| !secondary.iter().any(|s| std::ptr::eq(*s, *g)))
        .collect();
    let sec_ax = secondary.first().and_then(|g| group_axes(ch, g).1);

    // 항목
    let n_cat = groups
        .iter()
        .flat_map(|g| g.series.iter())
        .map(|s| s.values.len().max(s.categories.len()))
        .max()
        .unwrap_or(0)
        .max(1);
    let cats: Vec<String> = groups
        .iter()
        .flat_map(|g| g.series.iter())
        .find(|s| !s.categories.is_empty())
        .map(|s| s.categories.clone())
        .unwrap_or_else(|| (1..=n_cat).map(|i| i.to_string()).collect());
    let cat_fmt = groups
        .iter()
        .flat_map(|g| g.series.iter())
        .find_map(|s| s.cat.as_ref())
        .and_then(|d| d.cache.as_ref())
        .and_then(|c| c.format_code.clone());
    let val_fmt = first
        .series
        .iter()
        .find_map(|s| s.val.as_ref())
        .and_then(|d| d.cache.as_ref())
        .and_then(|c| c.format_code.clone());

    // 글꼴
    let cat_font = cx.font(
        cat_ax.and_then(|a| a.tx_pr.as_ref()),
        Some(base),
        base.size,
        false,
    );
    let val_font = cx.font(
        val_ax.and_then(|a| a.tx_pr.as_ref()),
        Some(base),
        base.size,
        false,
    );

    // 그림 영역: 직접 배치 또는 축 레이블 자리를 빼고
    let (vlo, vhi, percent) = value_range(&primary);
    let room = if first.three_d { 0.0 } else { 0.05 };
    let probe = make_scale(vlo, vhi, val_ax, inner.h.max(inner.w) * 0.8, !xy, room);
    let val_labels: Vec<String> = probe
        .ticks()
        .iter()
        .map(|v| tick_label(*v, val_ax, val_fmt.as_deref(), percent))
        .collect();
    let val_label_w = if labels_on(val_ax) {
        val_labels
            .iter()
            .map(|l| val_font.width(l))
            .fold(0.0, f64::max)
            + VAL_GAP
    } else {
        0.0
    };
    let sec_label_w = if sec_ax.is_some() && labels_on(sec_ax) {
        val_label_w
    } else {
        0.0
    };
    let cat_label_h = if labels_on(cat_ax) || xy {
        cat_font.px() * 1.1 + VAL_GAP
    } else {
        0.0
    };
    let axis_title = |a: Option<&ChartAxis>| -> Option<(String, Font)> {
        let t = a?.title.as_ref()?;
        let text = t.plain_text()?;
        let mut f = cx.font(t.tx_pr.as_ref(), Some(base), 10.0, true);
        if let Some(crate::model::ChartTx::Rich(r)) = &t.tx {
            if let Some(cf) = r.paras.first().and_then(|p| {
                p.runs
                    .first()
                    .and_then(|r| r.font.as_ref())
                    .or(p.def_font.as_ref())
            }) {
                cx.apply_font(&mut f, cf);
            }
            f.rot = r.rot;
        }
        Some((text, f))
    };
    let cat_title = axis_title(cat_ax);
    let val_title = axis_title(val_ax);
    let mut plot = match manual_rect(ch.plot_layout.as_ref(), w, h) {
        Some((r, true)) => r,
        Some((r, false)) => Rect {
            x: r.x + val_label_w,
            y: r.y,
            w: r.w - val_label_w - sec_label_w,
            h: r.h - cat_label_h,
        },
        None => {
            // 한셀 화면: 왼쪽은 레이블 + 14px, 위는 눈금 레이블 반 줄, 아래는 항목 레이블 한 줄 + 14px
            let tl = val_title.as_ref().map_or(0.0, |(_, f)| f.px() * 1.4);
            let tb = cat_title.as_ref().map_or(0.0, |(_, f)| f.px() * 1.4);
            let (left, bottom, top_half) = if horizontal {
                (
                    if labels_on(cat_ax) {
                        cats.iter().map(|c| cat_font.width(c)).fold(0.0, f64::max) + VAL_GAP
                    } else {
                        0.0
                    },
                    val_font.px() * 1.1 + VAL_GAP,
                    // 한셀 화면 (chart_barh): 그림 영역 위가 틀에서 8px
                    8.0 - PAD.2,
                )
            } else {
                (val_label_w, cat_label_h, val_font.px() * 0.35)
            };
            Rect {
                x: inner.x + left + tl,
                y: inner.y + top_half,
                w: inner.w - left - tl - sec_label_w,
                h: inner.h - top_half - bottom - tb,
            }
        }
    };
    // 가로 막대·분산형: 값 레이블이 눈금 가운데에 오므로 마지막 레이블의 반을 오른쪽에 남긴다
    if (horizontal || xy)
        && ch
            .plot_layout
            .as_ref()
            .and_then(|l| l.manual.as_ref())
            .is_none()
    {
        let last = if horizontal {
            val_labels.last().map_or(0.0, |l| val_font.width(l))
        } else {
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for g in groups {
                for s in g.series {
                    for x in x_values(s) {
                        lo = lo.min(x);
                        hi = hi.max(x);
                    }
                }
            }
            let xs = make_scale(lo.min(hi), hi.max(lo), cat_ax, plot.w, true, room);
            xs.ticks().last().map_or(0.0, |v| {
                cat_font.width(&tick_label(*v, cat_ax, cat_fmt.as_deref(), false))
            })
        };
        // 한셀 화면: chart_barh·chart_scatter 모두 세로 막대·꺾은선보다 10.5px 좁다 (마지막 레이블 70·10)
        plot.w -= last / 2.0 + 4.5;
    }
    // 3차원: 깊이 자리만큼 앞면 그림 영역이 작다 (chart_col3d: 2차원 415×267 보다 왼쪽 +9, 오른쪽 −38, 위 +15, 아래 −2 px)
    if first.three_d
        && !xy
        && ch
            .plot_layout
            .as_ref()
            .and_then(|l| l.manual.as_ref())
            .is_none()
    {
        let (l, r, t, b) = (
            plot.w * 0.022,
            plot.w * 0.092,
            plot.h * 0.056,
            plot.h * 0.008,
        );
        plot.x += l;
        plot.w -= l + r;
        plot.y += t;
        plot.h -= t + b;
    }
    plot.w = plot.w.max(10.0);
    plot.h = plot.h.max(10.0);

    // 값 축 범위 (그림 영역 크기로 다시)
    let vlen = if horizontal { plot.w } else { plot.h };
    let scale = make_scale(vlo, vhi, val_ax, vlen, !xy, room);
    let sec_scale = sec_ax.map(|a| {
        let (lo, hi, _) = value_range(&secondary);
        make_scale(lo, hi, Some(a), vlen, true, room)
    });
    // x 값 축 (분산형·거품형)
    let xscale = if xy {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for g in groups {
            for s in g.series {
                for x in x_values(s) {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
        if !lo.is_finite() {
            lo = 0.0;
            hi = 1.0;
        }
        Some(make_scale(lo, hi, cat_ax, plot.w, true, room))
    } else {
        None
    };

    // 그림 영역 바탕
    let plot_style = cx.shape(ch.plot_sp_pr.as_ref(), None, None);
    out.rect(plot.x, plot.y, plot.w, plot.h, &plot_style);

    // 값 → 화면 좌표
    let vpos = |s: &Scale, v: f64| -> f64 {
        let t = s.t(v);
        if horizontal {
            plot.x + t * plot.w
        } else {
            plot.y + plot.h - t * plot.h
        }
    };
    // 항목 띠 (between: 띠 가운데, midCat: 눈금 위)
    let between =
        val_ax.and_then(|a| a.cross_between.as_deref()) != Some("midCat") || first.kind == "bar";
    let cat_reverse = cat_ax.and_then(|a| a.scaling.orientation.as_deref()) == Some("maxMin");
    let band_len = if horizontal { plot.h } else { plot.w };
    let band = if between {
        band_len / n_cat as f64
    } else {
        band_len / (n_cat.max(2) - 1) as f64
    };
    let cat_mid = |k: usize| -> f64 {
        let k = if cat_reverse { n_cat - 1 - k } else { k };
        let off = if between {
            band * (k as f64 + 0.5)
        } else {
            band * k as f64
        };
        if horizontal {
            // 가로 막대: 첫 항목이 아래
            plot.y + plot.h - off
        } else {
            plot.x + off
        }
    };

    // 눈금선
    let grid = |a: Option<&ChartAxis>, major: bool| -> Option<Stroke> {
        let a = a?;
        let l = if major {
            a.major_gridlines.as_ref()
        } else {
            a.minor_gridlines.as_ref()
        }?;
        cx.line_of(l.sp_pr.as_ref(), Some(("#D9D9D9", 1.0)))
    };
    if let Some(gs) = grid(val_ax, false) {
        let minor = val_ax
            .and_then(|a| a.minor_unit)
            .unwrap_or(scale.major / 5.0);
        if minor > 0.0 && scale.log.is_none() {
            let mut v = scale.min;
            while v <= scale.max && (v - scale.min) / minor < 500.0 {
                let p = vpos(&scale, v);
                grid_line(out, horizontal, p, &plot, &gs);
                v += minor;
            }
        }
    }
    if let Some(gs) = grid(val_ax, true) {
        for v in scale.ticks() {
            let p = vpos(&scale, v);
            grid_line(out, horizontal, p, &plot, &gs);
        }
    }
    if let Some(gs) = grid(cat_ax, true) {
        if let Some(xs) = &xscale {
            for v in xs.ticks() {
                let p = plot.x + xs.t(v) * plot.w;
                grid_line(out, true, p, &plot, &gs);
            }
        } else {
            for k in 0..=n_cat {
                let off = band * k as f64;
                let p = if horizontal {
                    plot.y + off
                } else {
                    plot.x + off
                };
                grid_line(out, !horizontal, p, &plot, &gs);
            }
        }
    }

    // 계열
    out.begin_clip(&plot);
    for g in groups {
        let sc = if secondary.iter().any(|s| std::ptr::eq(*s, g)) {
            sec_scale.as_ref().unwrap_or(&scale)
        } else {
            &scale
        };
        match g.kind {
            "bar" => draw_bars(
                out, g, cx, sc, &plot, horizontal, n_cat, band, &cat_mid, &vpos,
            ),
            "line" | "stock" => draw_lines(out, g, cx, sc, n_cat, &cat_mid, &vpos, false),
            "area" => draw_areas(out, g, cx, sc, n_cat, &cat_mid, &vpos, &plot),
            "scatter" | "bubble" => {
                if let Some(xs) = &xscale {
                    let xpos = |v: f64| plot.x + xs.t(v) * plot.w;
                    draw_xy(out, g, cx, sc, &xpos, &vpos, &plot);
                }
            }
            _ => draw_lines(out, g, cx, sc, n_cat, &cat_mid, &vpos, false),
        }
    }
    out.end_clip();

    // 축 선과 눈금, 레이블
    let ax_line = |a: Option<&ChartAxis>| {
        cx.line_of(a.and_then(|a| a.sp_pr.as_ref()), Some((AXIS_GRAY, 1.0)))
    };
    let tick_len = |a: Option<&ChartAxis>| match a.and_then(|a| a.major_tick_mark.as_deref()) {
        Some("none") => (0.0, 0.0),
        Some("in") => (-5.0, 0.0),
        Some("cross") => (-5.0, 5.0),
        _ => (0.0, 5.0),
    };
    // 값 축이 항목 축과 만나는 자리 (항목 축 선의 위치)
    let cross_v = match cat_ax.and_then(|a| a.crosses.as_deref()) {
        Some("max") => scale.max,
        Some("min") => scale.min,
        _ => cat_ax
            .and_then(|a| a.crosses_at)
            .unwrap_or_else(|| 0.0f64.clamp(scale.min, scale.max)),
    };
    let cross_p = vpos(
        &scale,
        if scale.log.is_some() {
            scale.min
        } else {
            cross_v
        },
    );
    // 값 축 (세로 차트는 왼쪽)
    if axis_visible(val_ax) {
        let st = ax_line(val_ax);
        let (ti, to) = tick_len(val_ax);
        let right = val_ax.and_then(|a| a.ax_pos.as_deref()) == Some("r");
        if horizontal {
            let y = plot.y + plot.h;
            if let Some(st) = &st {
                out.line(plot.x, y + 0.5, plot.x + plot.w, y + 0.5, st);
            }
            for v in scale.ticks() {
                let p = vpos(&scale, v).round() + 0.5;
                if let Some(st) = &st
                    && (ti != 0.0 || to != 0.0)
                {
                    out.line(p, y + ti, p, y + to, st);
                }
                if labels_on(val_ax) {
                    let t = tick_label(v, val_ax, val_fmt.as_deref(), percent);
                    out.text(
                        p,
                        y + 22.5 * val_font.size / BASE_PT + val_font.px() * 0.35,
                        &t,
                        &val_font,
                        "middle",
                    );
                }
            }
        } else {
            let x = if right { plot.x + plot.w } else { plot.x };
            let x = x.round() + 0.5;
            if let Some(st) = &st {
                out.line(x, plot.y, x, plot.y + plot.h, st);
            }
            for v in scale.ticks() {
                let p = vpos(&scale, v).round() + 0.5;
                if let Some(st) = &st
                    && (ti != 0.0 || to != 0.0)
                {
                    if right {
                        out.line(x - ti, p, x + to, p, st);
                    } else {
                        out.line(x + ti, p, x - to, p, st);
                    }
                }
                if labels_on(val_ax) {
                    let t = tick_label(v, val_ax, val_fmt.as_deref(), percent);
                    if right {
                        out.text(
                            x + VAL_GAP,
                            p + val_font.px() * 0.35,
                            &t,
                            &val_font,
                            "start",
                        );
                    } else {
                        out.text(x - VAL_GAP, p + val_font.px() * 0.35, &t, &val_font, "end");
                    }
                }
            }
        }
    }
    // 보조 값 축 (오른쪽)
    if let (Some(sa), Some(ss)) = (sec_ax, sec_scale.as_ref())
        && axis_visible(Some(sa))
    {
        let st = ax_line(Some(sa));
        let x = (plot.x + plot.w).round() + 0.5;
        if let Some(st) = &st {
            out.line(x, plot.y, x, plot.y + plot.h, st);
        }
        let font = cx.font(sa.tx_pr.as_ref(), Some(base), base.size, false);
        let fmt = secondary
            .first()
            .and_then(|g| g.series.iter().find_map(|s| s.val.as_ref()))
            .and_then(|d| d.cache.as_ref())
            .and_then(|c| c.format_code.clone());
        for v in ss.ticks() {
            let p = vpos(ss, v).round() + 0.5;
            if let Some(st) = &st {
                out.line(x, p, x + 5.0, p, st);
            }
            if labels_on(Some(sa)) {
                let t = tick_label(v, Some(sa), fmt.as_deref(), false);
                out.text(x + 8.0, p + font.px() * 0.35, &t, &font, "start");
            }
        }
    }
    // 항목 축 (또는 x 값 축)
    if axis_visible(cat_ax) {
        let st = ax_line(cat_ax);
        let (ti, to) = tick_len(cat_ax);
        if let Some(xs) = &xscale {
            let y = cross_p.round() + 0.5;
            if let Some(st) = &st {
                out.line(plot.x, y, plot.x + plot.w, y, st);
            }
            for v in xs.ticks() {
                let p = (plot.x + xs.t(v) * plot.w).round() + 0.5;
                if let Some(st) = &st {
                    out.line(p, y + ti, p, y + to, st);
                }
                if labels_on(cat_ax) {
                    let t = tick_label(v, cat_ax, cat_fmt.as_deref(), false);
                    out.text(
                        p,
                        plot.y + plot.h + 22.5 * cat_font.size / BASE_PT + cat_font.px() * 0.35,
                        &t,
                        &cat_font,
                        "middle",
                    );
                }
            }
        } else if horizontal {
            let x = cross_p.round() + 0.5;
            if let Some(st) = &st {
                out.line(x, plot.y, x, plot.y + plot.h, st);
            }
            for k in 0..=n_cat {
                let p = (plot.y + band * k as f64).round() + 0.5;
                if let Some(st) = &st {
                    out.line(x + ti, p, x - to, p, st);
                }
            }
            if labels_on(cat_ax) {
                for (k, c) in cats.iter().enumerate().take(n_cat) {
                    let t = cat_text(c, cat_fmt.as_deref());
                    out.text(
                        plot.x - VAL_GAP,
                        cat_mid(k) + cat_font.px() * 0.35,
                        &t,
                        &cat_font,
                        "end",
                    );
                }
            }
        } else {
            let y = cross_p.round() + 0.5;
            if let Some(st) = &st {
                out.line(plot.x, y, plot.x + plot.w, y, st);
            }
            let marks = if between { n_cat + 1 } else { n_cat };
            for k in 0..marks {
                let p = (plot.x + band * k as f64).round() + 0.5;
                if let Some(st) = &st
                    && (ti != 0.0 || to != 0.0)
                {
                    out.line(p, y + ti, p, y + to, st);
                }
            }
            if labels_on(cat_ax) {
                // 레이블이 겹치면 건너뛴다
                let skip = cat_ax
                    .and_then(|a| a.tick_lbl_skip)
                    .map(|s| s.max(1) as usize)
                    .unwrap_or_else(|| {
                        let widest = cats.iter().map(|c| cat_font.width(c)).fold(0.0, f64::max);
                        ((widest + 6.0) / band).ceil().max(1.0) as usize
                    });
                // 항목 레이블 가운데가 축 아래 22.5px (한셀 화면, 9pt)
                let ly = plot.y + plot.h + 22.5 * cat_font.size / BASE_PT + cat_font.px() * 0.35;
                for (k, c) in cats.iter().enumerate().take(n_cat) {
                    if k % skip != 0 {
                        continue;
                    }
                    let t = cat_text(c, cat_fmt.as_deref());
                    out.text(cat_mid(k), ly, &t, &cat_font, "middle");
                }
            }
        }
    }
    // 축 제목
    if let Some((t, f)) = val_title {
        if horizontal {
            out.text(
                plot.x + plot.w / 2.0,
                plot.y + plot.h + val_font.px() * 1.3 + 8.0 + f.px(),
                &t,
                &f,
                "middle",
            );
        } else {
            let x = plot.x - val_label_w - f.px() * 0.6;
            let y = plot.y + plot.h / 2.0;
            if f.rot == Some(0) {
                out.text(x, y, &t, &f, "middle");
            } else {
                out.text_rot(x, y, &t, &f, -90.0);
            }
        }
    }
    if let Some((t, f)) = cat_title {
        if horizontal {
            let x = plot.x
                - 10.0
                - cats.iter().map(|c| cat_font.width(c)).fold(0.0, f64::max)
                - f.px() * 0.6;
            out.text_rot(x, plot.y + plot.h / 2.0, &t, &f, -90.0);
        } else {
            out.text(
                plot.x + plot.w / 2.0,
                plot.y + plot.h + cat_label_h + f.px(),
                &t,
                &f,
                "middle",
            );
        }
    }

    // 데이터 레이블
    for g in groups {
        let sc = if secondary.iter().any(|s| std::ptr::eq(*s, g)) {
            sec_scale.as_ref().unwrap_or(&scale)
        } else {
            &scale
        };
        draw_axis_labels(
            out,
            ch,
            g,
            cx,
            base,
            sc,
            n_cat,
            band,
            horizontal,
            &cat_mid,
            &vpos,
            xscale.as_ref(),
            &plot,
        );
    }
}

fn grid_line(out: &mut Svg, vertical: bool, p: f64, plot: &Rect, st: &Stroke) {
    let p = p.round() + 0.5;
    if vertical {
        out.line(p, plot.y, p, plot.y + plot.h, st);
    } else {
        out.line(plot.x, p, plot.x + plot.w, p, st);
    }
}

/// 항목 레이블 글 (숫자 항목은 형식대로)
fn cat_text(c: &str, fmt: Option<&str>) -> String {
    match (c.trim().parse::<f64>(), fmt) {
        (Ok(v), Some(f))
            if !f.is_empty()
                && !f.eq_ignore_ascii_case("G/표준")
                && !f.eq_ignore_ascii_case("general") =>
        {
            crate::numfmt::format(&crate::model::CellValue::Number(v), f, false).text
        }
        _ => c.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_bars(
    out: &mut Svg,
    g: &Group,
    cx: &Ctx,
    sc: &Scale,
    plot: &Rect,
    horizontal: bool,
    n_cat: usize,
    band: f64,
    cat_mid: &dyn Fn(usize) -> f64,
    vpos: &dyn Fn(&Scale, f64) -> f64,
) {
    let stacked = matches!(g.grouping, "stacked" | "percentStacked");
    let percent = g.grouping == "percentStacked";
    let ns = g.series.len().max(1);
    let gap = g.props.gap_width.unwrap_or(150) as f64 / 100.0;
    let overlap = g
        .props
        .overlap
        .map_or(if stacked { 1.0 } else { 0.0 }, |o| o as f64 / 100.0);
    // 띠 = 막대들 + 간격(막대 폭 × gap). 겹침이 있으면 막대 폭 × (1 − overlap) 씩 옮긴다
    let slots = if stacked {
        1.0
    } else {
        ns as f64 - (ns as f64 - 1.0) * overlap
    };
    let bw = band / (slots + gap);
    let step = bw * (1.0 - overlap);
    let base_v = 0.0f64.clamp(sc.min, sc.max);
    let base_v = if sc.log.is_some() { sc.min } else { base_v };
    let base = vpos(sc, base_v);
    let _ = plot;
    for k in 0..n_cat {
        let total: f64 = g
            .series
            .iter()
            .map(|s| s.values.get(k).copied().flatten().unwrap_or(0.0).abs())
            .sum();
        let (mut pos, mut neg) = (0.0, 0.0);
        let group_w = if stacked {
            bw
        } else {
            bw + step * (ns as f64 - 1.0)
        };
        let start = cat_mid(k)
            - if horizontal {
                -group_w / 2.0
            } else {
                group_w / 2.0
            };
        for (i, s) in g.series.iter().enumerate() {
            let Some(mut v) = s.values.get(k).copied().flatten() else {
                continue;
            };
            if percent && total > 0.0 {
                v /= total;
            }
            let (a, b) = if stacked {
                if v >= 0.0 {
                    pos += v;
                    (pos - v, pos)
                } else {
                    neg += v;
                    (neg - v, neg)
                }
            } else {
                (base_v, v)
            };
            let (p0, p1) = (if stacked { vpos(sc, a) } else { base }, vpos(sc, b));
            let color = if g.vary_colors && g.series.len() == 1 {
                cx.palette(k)
            } else {
                series_color(s, g.offset + i, cx)
            };
            let mut st = point_style(s, i, k, cx, &color);
            if v < 0.0 && s.invert_if_negative == Some(true) {
                st.fill = Some(("#FFFFFF".into(), 1.0));
                st.stroke.get_or_insert(Stroke::new(&color, 1.0));
            }
            let off = if stacked { 0.0 } else { step * i as f64 };
            let (rx, ry, rw, rh) = if horizontal {
                (p0.min(p1), start - off - bw, (p1 - p0).abs(), bw)
            } else {
                (start + off, p0.min(p1), bw, (p1 - p0).abs())
            };
            if g.three_d {
                // 한셀 화면 (chart_col3d, 막대 폭 30px): 앞면은 축 자리에서 오른쪽 6px·위 3~4px, 깊이는 오른쪽 8px·위 7px
                let (sx, sy) = (bw * 0.2, bw * 0.12);
                out.rect(rx + sx, ry - sy, rw, rh, &st);
                bar_depth(out, rx + sx, ry - sy, rw, rh, bw, &st, &color);
            } else {
                out.rect(rx, ry, rw, rh, &st);
            }
        }
    }
}

/// 3차원 막대의 윗면·옆면. 한셀 화면에서 윗면은 앞면 색 × 0.72, 옆면은 × 0.485 (chart_col3d: #6182D6 → #465E9A·#2F3F68)
#[allow(clippy::too_many_arguments)]
fn bar_depth(out: &mut Svg, x: f64, y: f64, w: f64, h: f64, bw: f64, st: &Style, color: &str) {
    let (dx, dy) = (bw * 0.267, bw * 0.233);
    let front = st.fill.as_ref().map_or(color, |f| f.0.as_str());
    let top = format!(
        "M{x:.1},{y:.1}L{:.1},{:.1}L{:.1},{:.1}L{:.1},{y:.1}Z",
        x + dx,
        y - dy,
        x + w + dx,
        y - dy,
        x + w
    );
    let side = format!(
        "M{:.1},{y:.1}L{:.1},{:.1}L{:.1},{:.1}L{:.1},{:.1}Z",
        x + w,
        x + w + dx,
        y - dy,
        x + w + dx,
        y + h - dy,
        x + w,
        y + h
    );
    out.path(&top, &Style::fill(&scale_rgb(front, 0.72)));
    out.path(&side, &Style::fill(&scale_rgb(front, 0.485)));
}

/// `#RRGGBB` 의 각 성분에 `k` 를 곱한다 (형식이 다르면 그대로)
fn scale_rgb(c: &str, k: f64) -> String {
    let h = c.trim_start_matches('#');
    match h.get(..6).map(|s| u32::from_str_radix(s, 16)) {
        Some(Ok(v)) => {
            let f = |s: u32| (((v >> s) & 0xff) as f64 * k).round().clamp(0.0, 255.0) as u32;
            format!("#{:02X}{:02X}{:02X}", f(16), f(8), f(0))
        }
        _ => c.to_string(),
    }
}

/// 꺾은선 (주식형도): 계열마다 선과 표식, 누적이면 쌓는다. 고저선은 항목마다 최고·최저를 잇는다
#[allow(clippy::too_many_arguments)]
fn draw_lines(
    out: &mut Svg,
    g: &Group,
    cx: &Ctx,
    sc: &Scale,
    n_cat: usize,
    cat_mid: &dyn Fn(usize) -> f64,
    vpos: &dyn Fn(&Scale, f64) -> f64,
    _area: bool,
) {
    let stacked = matches!(g.grouping, "stacked" | "percentStacked");
    let percent = g.grouping == "percentStacked";
    let totals: Vec<f64> = (0..n_cat)
        .map(|k| {
            g.series
                .iter()
                .map(|s| s.values.get(k).copied().flatten().unwrap_or(0.0).abs())
                .sum()
        })
        .collect();
    // 고저선
    if let Some(hl) = &g.props.hi_low_lines {
        let st = cx.line_of(hl.sp_pr.as_ref(), Some(("#000000", 1.0)));
        if let Some(st) = st {
            for k in 0..n_cat {
                let vs: Vec<f64> = g
                    .series
                    .iter()
                    .filter_map(|s| s.values.get(k).copied().flatten())
                    .collect();
                if vs.len() >= 2 {
                    let lo = vs.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = vs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let x = cat_mid(k).round() + 0.5;
                    out.line(x, vpos(sc, lo), x, vpos(sc, hi), &st);
                }
            }
        }
    }
    let mut acc = vec![0.0; n_cat];
    for (i, s) in g.series.iter().enumerate() {
        let k0 = g.offset + i;
        let color = series_color(s, k0, cx);
        let pts: Vec<Option<(f64, f64)>> = (0..n_cat)
            .map(|k| {
                let mut v = s.values.get(k).copied().flatten()?;
                if percent && totals[k] > 0.0 {
                    v /= totals[k];
                }
                if stacked {
                    acc[k] += v;
                    v = acc[k];
                }
                Some((cat_mid(k), vpos(sc, v)))
            })
            .collect();
        let smooth = s.smooth == Some(true) || g.props.smooth == Some(true) && s.smooth.is_none();
        // 주식형 계열은 보통 선이 없다 (ln noFill)
        if let Some(st) = series_stroke(s, &color, cx) {
            let d = polyline(&pts, smooth);
            out.path(
                &d,
                &Style {
                    fill: None,
                    stroke: Some(st),
                },
            );
        }
        if let Some((sym, size)) = marker_of(s, g, k0) {
            let ms = marker_style(s, &color, cx);
            for (k, p) in pts.iter().enumerate() {
                if let Some((x, y)) = p {
                    let dpt = s.d_pts.iter().find(|d| d.idx as usize == k);
                    let st = dpt
                        .and_then(|d| d.marker.as_ref())
                        .and_then(|m| m.sp_pr.as_ref())
                        .map(|sp| cx.shape(Some(sp), Some(&color), None))
                        .unwrap_or_else(|| ms.clone());
                    draw_marker(out, &sym, *x, *y, size, &st);
                }
            }
        }
    }
}

/// 점들 → 경로 (빈 점에서 끊는다). 부드러운 선은 카디널 곡선
fn polyline(pts: &[Option<(f64, f64)>], smooth: bool) -> String {
    let mut d = String::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    let flush = |run: &mut Vec<(f64, f64)>, d: &mut String| {
        if run.is_empty() {
            return;
        }
        let _ = write!(d, "M{:.1},{:.1}", run[0].0, run[0].1);
        if smooth && run.len() > 2 {
            for i in 0..run.len() - 1 {
                let p0 = if i == 0 { run[0] } else { run[i - 1] };
                let (p1, p2) = (run[i], run[i + 1]);
                let p3 = if i + 2 < run.len() { run[i + 2] } else { p2 };
                let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
                let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
                let _ = write!(
                    d,
                    "C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
                    c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
                );
            }
        } else {
            for p in &run[1..] {
                let _ = write!(d, "L{:.1},{:.1}", p.0, p.1);
            }
        }
        run.clear();
    };
    for p in pts {
        match p {
            Some(p) => run.push(*p),
            None => flush(&mut run, &mut d),
        }
    }
    flush(&mut run, &mut d);
    d
}

#[allow(clippy::too_many_arguments)]
fn draw_areas(
    out: &mut Svg,
    g: &Group,
    cx: &Ctx,
    sc: &Scale,
    n_cat: usize,
    cat_mid: &dyn Fn(usize) -> f64,
    vpos: &dyn Fn(&Scale, f64) -> f64,
    plot: &Rect,
) {
    let stacked = matches!(g.grouping, "stacked" | "percentStacked");
    let percent = g.grouping == "percentStacked";
    let totals: Vec<f64> = (0..n_cat)
        .map(|k| {
            g.series
                .iter()
                .map(|s| s.values.get(k).copied().flatten().unwrap_or(0.0).abs())
                .sum()
        })
        .collect();
    let base_v = 0.0f64.clamp(sc.min, sc.max);
    let mut acc = vec![0.0; n_cat];
    let _ = plot;
    // 누적이 아니면 뒤 계열이 앞을 가린다: 그리는 순서는 계열 순서 그대로
    for (i, s) in g.series.iter().enumerate() {
        let color = series_color(s, g.offset + i, cx);
        let st = cx.shape(s.sp_pr.as_ref(), Some(&color), None);
        let below = acc.clone();
        let mut top = Vec::with_capacity(n_cat);
        for k in 0..n_cat {
            let mut v = s.values.get(k).copied().flatten().unwrap_or(0.0);
            if percent && totals[k] > 0.0 {
                v /= totals[k];
            }
            if stacked {
                acc[k] += v;
                v = acc[k];
            }
            top.push((cat_mid(k), vpos(sc, v)));
        }
        let mut d = String::new();
        for (k, (x, y)) in top.iter().enumerate() {
            let _ = write!(d, "{}{x:.1},{y:.1}", if k == 0 { "M" } else { "L" });
        }
        for k in (0..n_cat).rev() {
            let b = if stacked { below[k] } else { base_v };
            let _ = write!(d, "L{:.1},{:.1}", cat_mid(k), vpos(sc, b));
        }
        d.push('Z');
        out.path(&d, &st);
    }
}

/// 분산형·거품형
fn draw_xy(
    out: &mut Svg,
    g: &Group,
    cx: &Ctx,
    sc: &Scale,
    xpos: &dyn Fn(f64) -> f64,
    vpos: &dyn Fn(&Scale, f64) -> f64,
    plot: &Rect,
) {
    let bubble = g.kind == "bubble";
    let max_size = g
        .series
        .iter()
        .filter_map(|s| s.bubble_size.as_ref())
        .flat_map(|d| d.dense())
        .filter_map(|v| v.and_then(|t| t.trim().parse::<f64>().ok()))
        .fold(0.0, f64::max);
    let scale_pct = g.props.bubble_scale.unwrap_or(100) as f64 / 100.0;
    for (i, s) in g.series.iter().enumerate() {
        let k0 = g.offset + i;
        let color = series_color(s, k0, cx);
        let xs = x_values(s);
        let pts: Vec<Option<(f64, f64)>> = s
            .values
            .iter()
            .enumerate()
            .map(|(k, v)| Some((xpos(*xs.get(k)?), vpos(sc, (*v)?))))
            .collect();
        if bubble {
            let sizes: Vec<f64> = s
                .bubble_size
                .as_ref()
                .map(|d| {
                    d.dense()
                        .into_iter()
                        .map(|v| v.and_then(|t| t.trim().parse().ok()).unwrap_or(0.0))
                        .collect()
                })
                .unwrap_or_default();
            let rmax = plot.w.min(plot.h) * 0.125 * scale_pct;
            for (k, p) in pts.iter().enumerate() {
                if let Some((x, y)) = p {
                    let sz = sizes.get(k).copied().unwrap_or(0.0).max(0.0);
                    let r = if max_size > 0.0 {
                        rmax * (sz / max_size).sqrt()
                    } else {
                        0.0
                    };
                    let st = cx.shape(s.sp_pr.as_ref(), Some(&color), None);
                    draw_marker(
                        out,
                        "circle",
                        *x,
                        *y,
                        r * 2.0,
                        &Style {
                            fill: st.fill.map(|(c, a)| (c, a.min(0.75))),
                            stroke: st.stroke,
                        },
                    );
                }
            }
            continue;
        }
        let style = g.props.scatter_style.as_deref().unwrap_or("lineMarker");
        let smooth = matches!(style, "smooth" | "smoothMarker") || s.smooth == Some(true);
        let line_off = style == "marker"
            || s.sp_pr
                .as_ref()
                .and_then(|p| p.line.as_ref())
                .and_then(|l| l.fill.as_ref())
                .is_some_and(|f| matches!(f, ChartFill::None));
        if !line_off && let Some(st) = series_stroke(s, &color, cx) {
            out.path(
                &polyline(&pts, smooth),
                &Style {
                    fill: None,
                    stroke: Some(st),
                },
            );
        }
        if let Some((sym, size)) = marker_of(s, g, k0) {
            let ms = marker_style(s, &color, cx);
            for (x, y) in pts.iter().flatten() {
                draw_marker(out, &sym, *x, *y, size, &ms);
            }
        }
    }
}

/// 데이터 레이블 (계열 → 묶음 설정)
fn labels_of<'a>(s: &'a ChartSeries, g: &'a Group) -> Option<&'a ChartDataLabels> {
    let d = s.d_lbls.as_ref().or(g.props.d_lbls.as_ref())?;
    if d.delete == Some(true) {
        return None;
    }
    let any = [
        d.show_val,
        d.show_cat_name,
        d.show_ser_name,
        d.show_percent,
        d.show_bubble_size,
    ]
    .contains(&Some(true));
    any.then_some(d)
}

fn label_text(d: &ChartDataLabels, s: &ChartSeries, k: usize, v: f64, pct: Option<f64>) -> String {
    let mut parts = Vec::new();
    if d.show_ser_name == Some(true) {
        parts.push(s.name.clone().unwrap_or_default());
    }
    if d.show_cat_name == Some(true) {
        parts.push(
            s.categories
                .get(k)
                .cloned()
                .unwrap_or_else(|| (k + 1).to_string()),
        );
    }
    if d.show_val == Some(true) {
        let code = d
            .num_fmt
            .as_ref()
            .filter(|f| f.source_linked != Some(true))
            .map(|f| f.format_code.clone())
            .or_else(|| {
                s.val
                    .as_ref()
                    .and_then(|x| x.cache.as_ref())
                    .and_then(|c| c.format_code.clone())
            })
            .filter(|c| !c.eq_ignore_ascii_case("G/표준"))
            .unwrap_or_else(|| "General".into());
        parts.push(crate::numfmt::format(&crate::model::CellValue::Number(v), &code, false).text);
    }
    if d.show_percent == Some(true)
        && let Some(p) = pct
    {
        parts.push(format!("{:.0}%", p * 100.0));
    }
    let sep = d.separator.clone().unwrap_or_else(|| ", ".into());
    parts.join(&sep)
}

#[allow(clippy::too_many_arguments)]
fn draw_axis_labels(
    out: &mut Svg,
    _ch: &Chart,
    g: &Group,
    cx: &Ctx,
    base: &Font,
    sc: &Scale,
    n_cat: usize,
    band: f64,
    horizontal: bool,
    cat_mid: &dyn Fn(usize) -> f64,
    vpos: &dyn Fn(&Scale, f64) -> f64,
    xscale: Option<&Scale>,
    plot: &Rect,
) {
    let stacked = matches!(g.grouping, "stacked" | "percentStacked");
    let ns = g.series.len().max(1);
    let gap = g.props.gap_width.unwrap_or(150) as f64 / 100.0;
    let bw = if stacked {
        band / (1.0 + gap)
    } else {
        band / (ns as f64 + gap)
    };
    let mut acc = vec![0.0; n_cat];
    for (i, s) in g.series.iter().enumerate() {
        let Some(d) = labels_of(s, g) else {
            if stacked {
                for (k, a) in acc.iter_mut().enumerate() {
                    *a += s.values.get(k).copied().flatten().unwrap_or(0.0);
                }
            }
            continue;
        };
        let font = cx.font(d.tx_pr.as_ref(), Some(base), base.size, false);
        let xs = x_values(s);
        for (k, v) in s.values.iter().enumerate().take(n_cat) {
            let Some(v) = *v else { continue };
            let t = label_text(d, s, k, v, None);
            let (x, y) = if let Some(xsc) = xscale {
                let Some(xv) = xs.get(k) else { continue };
                (plot.x + xsc.t(*xv) * plot.w, vpos(sc, v) - 6.0)
            } else if g.kind == "bar" {
                let top = if stacked {
                    acc[k] += v;
                    acc[k]
                } else {
                    v
                };
                let off = if stacked {
                    0.0
                } else {
                    bw * (i as f64 - (ns as f64 - 1.0) / 2.0)
                };
                let p = vpos(sc, top);
                let pos = d
                    .d_lbl_pos
                    .as_deref()
                    .unwrap_or(if stacked { "ctr" } else { "outEnd" });
                if horizontal {
                    let y = cat_mid(k) - off + font.px() * 0.35;
                    let x = match pos {
                        "ctr" => (p + vpos(sc, if stacked { top - v } else { 0.0 })) / 2.0,
                        "inEnd" => p - font.width(&t) / 2.0 - 3.0,
                        _ => p + font.width(&t) / 2.0 + 3.0,
                    };
                    (x, y)
                } else {
                    let x = cat_mid(k) + off;
                    let y = match pos {
                        "ctr" => {
                            (p + vpos(sc, if stacked { top - v } else { 0.0 })) / 2.0
                                + font.px() * 0.35
                        }
                        "inEnd" => p + font.px() + 2.0,
                        "inBase" => vpos(sc, 0.0) - 3.0,
                        _ => p - 4.0,
                    };
                    (x, y)
                }
            } else {
                let vv = if stacked {
                    acc[k] += v;
                    acc[k]
                } else {
                    v
                };
                let p = vpos(sc, vv);
                let pos = d.d_lbl_pos.as_deref().unwrap_or("r");
                match pos {
                    "t" => (cat_mid(k), p - 6.0),
                    "b" => (cat_mid(k), p + font.px() + 4.0),
                    "l" => (
                        cat_mid(k) - font.width(&t) / 2.0 - 6.0,
                        p + font.px() * 0.35,
                    ),
                    "ctr" => (cat_mid(k), p + font.px() * 0.35),
                    _ => (
                        cat_mid(k) + font.width(&t) / 2.0 + 6.0,
                        p + font.px() * 0.35,
                    ),
                }
            };
            out.text(x, y, &t, &font, "middle");
        }
    }
}

// ---------------------------------------------------------------------------
// 원형·도넛
// ---------------------------------------------------------------------------

fn draw_pies(out: &mut Svg, ch: &Chart, groups: &[Group], cx: &Ctx, base: &Font, inner: Rect) {
    let g = &groups[0];
    let doughnut = g.kind == "doughnut";
    // 한셀 화면: 원은 틀 위·아래에서 13px 안쪽까지 (안쪽 영역보다 위·아래로 3px 크다)
    let plot = match manual_rect(
        ch.plot_layout.as_ref(),
        inner.x + inner.w + PAD.1,
        inner.y + inner.h + PAD.3,
    ) {
        Some((r, _)) => r,
        None => Rect {
            x: inner.x,
            y: inner.y - 3.0,
            w: inner.w,
            h: inner.h + 6.0,
        },
    };
    let (cx0, cy0) = (plot.x + plot.w / 2.0, plot.y + plot.h / 2.0);
    let r_out = (plot.w.min(plot.h) / 2.0).max(4.0);
    let first_ang = g.props.first_slice_ang.unwrap_or(0) as f64;
    let rings: Vec<&ChartSeries> = if doughnut {
        g.series.iter().collect()
    } else {
        g.series.iter().take(1).collect()
    };
    let hole = if doughnut {
        g.props.hole_size.unwrap_or(50).clamp(10, 90) as f64 / 100.0
    } else {
        0.0
    };
    let ring_w = (r_out * (1.0 - hole)) / rings.len().max(1) as f64;
    for (ri, s) in rings.iter().enumerate() {
        let vals: Vec<f64> = s.values.iter().map(|v| v.unwrap_or(0.0).abs()).collect();
        let total: f64 = vals.iter().sum();
        let r1 = r_out - ring_w * ri as f64;
        let r0 = if doughnut { r1 - ring_w } else { 0.0 };
        if total <= 0.0 {
            continue;
        }
        let mut a0 = (first_ang - 90.0).to_radians();
        for (k, v) in vals.iter().enumerate() {
            let a1 = a0 + v / total * std::f64::consts::TAU;
            let base_c = cx.palette(k);
            let st0 = point_style(s, 0, k, cx, &base_c);
            // 조각 경계선은 지정이 있을 때만 (한셀 기본은 없다)
            let st = Style {
                fill: st0.fill.or(Some((base_c, 1.0))),
                stroke: st0.stroke,
            };
            // 쪼개기: 조각 가운데 방향으로 반지름 × explosion%
            let ex = s
                .d_pts
                .iter()
                .find(|d| d.idx as usize == k)
                .and_then(|d| d.explosion)
                .or(s.explosion)
                .unwrap_or(0) as f64
                / 100.0
                * r1;
            let mid = (a0 + a1) / 2.0;
            let (ox, oy) = (cx0 + ex * mid.cos(), cy0 + ex * mid.sin());
            let large = if a1 - a0 > std::f64::consts::PI { 1 } else { 0 };
            let p = |r: f64, a: f64| (ox + r * a.cos(), oy + r * a.sin());
            let d = if (a1 - a0) >= std::f64::consts::TAU - 1e-9 {
                // 한 조각이 전부
                if r0 > 0.0 {
                    format!(
                        "M{:.1},{:.1}A{r1:.1},{r1:.1} 0 1 1 {:.1},{:.1}A{r1:.1},{r1:.1} 0 1 1 {:.1},{:.1}ZM{:.1},{:.1}A{r0:.1},{r0:.1} 0 1 0 {:.1},{:.1}A{r0:.1},{r0:.1} 0 1 0 {:.1},{:.1}Z",
                        ox - r1,
                        oy,
                        ox + r1,
                        oy,
                        ox - r1,
                        oy,
                        ox - r0,
                        oy,
                        ox + r0,
                        oy,
                        ox - r0,
                        oy
                    )
                } else {
                    format!(
                        "M{:.1},{:.1}A{r1:.1},{r1:.1} 0 1 1 {:.1},{:.1}A{r1:.1},{r1:.1} 0 1 1 {:.1},{:.1}Z",
                        ox - r1,
                        oy,
                        ox + r1,
                        oy,
                        ox - r1,
                        oy
                    )
                }
            } else {
                let (s0, s1) = (p(r1, a0), p(r1, a1));
                if r0 > 0.0 {
                    let (e0, e1) = (p(r0, a1), p(r0, a0));
                    format!(
                        "M{:.1},{:.1}A{r1:.1},{r1:.1} 0 {large} 1 {:.1},{:.1}L{:.1},{:.1}A{r0:.1},{r0:.1} 0 {large} 0 {:.1},{:.1}Z",
                        s0.0, s0.1, s1.0, s1.1, e0.0, e0.1, e1.0, e1.1
                    )
                } else {
                    format!(
                        "M{ox:.1},{oy:.1}L{:.1},{:.1}A{r1:.1},{r1:.1} 0 {large} 1 {:.1},{:.1}Z",
                        s0.0, s0.1, s1.0, s1.1
                    )
                }
            };
            out.path(&d, &st);
            // 데이터 레이블
            if let Some(dl) = labels_of(s, g) {
                let font = cx.font(dl.tx_pr.as_ref(), Some(base), base.size, false);
                let t = label_text(dl, s, k, *v, Some(v / total));
                let pos = dl.d_lbl_pos.as_deref().unwrap_or("bestFit");
                let rr = match pos {
                    "outEnd" => r1 + font.px() * 1.2,
                    "inEnd" => r1 * 0.8,
                    "ctr" => (r0 + r1) / 2.0,
                    _ => {
                        if doughnut {
                            (r0 + r1) / 2.0
                        } else {
                            r1 * 0.65
                        }
                    }
                };
                let (lx, ly) = p(rr, mid);
                out.text(lx, ly + font.px() * 0.35, &t, &font, "middle");
            }
            a0 = a1;
        }
    }
}

// ---------------------------------------------------------------------------
// 방사형
// ---------------------------------------------------------------------------

fn draw_radar(out: &mut Svg, ch: &Chart, groups: &[Group], cx: &Ctx, base: &Font, inner: Rect) {
    let g = &groups[0];
    let n = g
        .series
        .iter()
        .map(|s| s.values.len().max(s.categories.len()))
        .max()
        .unwrap_or(0)
        .max(3);
    let cats: Vec<String> = g
        .series
        .iter()
        .find(|s| !s.categories.is_empty())
        .map(|s| s.categories.clone())
        .unwrap_or_else(|| (1..=n).map(|i| i.to_string()).collect());
    let font = cx.font(None, Some(base), base.size, false);
    let label_r = font.px() * 2.5;
    let r = ((inner.w.min(inner.h)) / 2.0 - label_r).max(10.0);
    let (cx0, cy0) = (inner.x + inner.w / 2.0, inner.y + inner.h / 2.0);
    let (lo, hi, _) = value_range(&[g]);
    let (_, val_ax) = group_axes(ch, g);
    let sc = make_scale(lo, hi, val_ax, r, true, 0.05);
    let ang = |k: usize| -std::f64::consts::FRAC_PI_2 + k as f64 * std::f64::consts::TAU / n as f64;
    let pt = |k: usize, v: f64| {
        let rr = sc.t(v).clamp(0.0, 1.0) * r;
        (cx0 + rr * ang(k).cos(), cy0 + rr * ang(k).sin())
    };
    // 눈금 다각형과 살
    let grid = cx
        .line_of(
            val_ax
                .and_then(|a| a.major_gridlines.as_ref())
                .and_then(|l| l.sp_pr.as_ref()),
            Some(("#D9D9D9", 1.0)),
        )
        .unwrap_or_else(|| Stroke::new("#D9D9D9", 1.0));
    for v in sc.ticks() {
        let mut d = String::new();
        for k in 0..n {
            let (x, y) = pt(k, v);
            let _ = write!(d, "{}{x:.1},{y:.1}", if k == 0 { "M" } else { "L" });
        }
        d.push('Z');
        out.path(
            &d,
            &Style {
                fill: None,
                stroke: Some(grid.clone()),
            },
        );
        let (x, y) = pt(0, v);
        out.text(
            x - 4.0,
            y + font.px() * 0.35,
            &tick_label(v, val_ax, None, false),
            &font,
            "end",
        );
    }
    let spoke = Stroke::new(AXIS_GRAY, 1.0);
    for (k, c) in cats.iter().enumerate().take(n) {
        let (x, y) = (cx0 + r * ang(k).cos(), cy0 + r * ang(k).sin());
        out.line(cx0, cy0, x, y, &spoke);
        let (lx, ly) = (
            cx0 + (r + label_r * 0.5) * ang(k).cos(),
            cy0 + (r + label_r * 0.5) * ang(k).sin(),
        );
        out.text(lx, ly + font.px() * 0.35, c, &font, "middle");
    }
    let filled = g.props.radar_style.as_deref() == Some("filled");
    for (i, s) in g.series.iter().enumerate() {
        let color = series_color(s, g.offset + i, cx);
        let mut d = String::new();
        let mut pts = Vec::new();
        for k in 0..n {
            let v = s.values.get(k).copied().flatten().unwrap_or(0.0);
            let (x, y) = pt(k, v);
            pts.push((x, y));
            let _ = write!(d, "{}{x:.1},{y:.1}", if k == 0 { "M" } else { "L" });
        }
        d.push('Z');
        let st = if filled {
            let mut st = cx.shape(s.sp_pr.as_ref(), Some(&color), None);
            if let Some(f) = &mut st.fill {
                f.1 = f.1.min(0.8);
            }
            st
        } else {
            Style {
                fill: None,
                stroke: series_stroke(s, &color, cx),
            }
        };
        out.path(&d, &st);
        if !filled && let Some((sym, size)) = marker_of(s, g, g.offset + i) {
            let ms = marker_style(s, &color, cx);
            for (x, y) in pts {
                draw_marker(out, &sym, x, y, size, &ms);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_units() {
        assert_eq!(nice_unit(1.0, 5.0), 0.2);
        assert_eq!(nice_unit(87.0, 5.0), 20.0);
        assert_eq!(nice_unit(150.0, 5.0), 50.0);
    }

    #[test]
    fn auto_scale_has_headroom_like_hancell() {
        // 한셀 화면: 최댓값 60 → 70, 130 → 140 (그림 영역 높이 270px 남짓)
        let s = make_scale(20.0, 60.0, None, 270.0, true, 0.05);
        assert_eq!((s.min, s.max, s.major), (0.0, 70.0, 10.0));
        let s = make_scale(0.0, 130.0, None, 270.0, true, 0.05);
        assert_eq!((s.min, s.max, s.major), (0.0, 140.0, 20.0));
    }
}
