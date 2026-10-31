// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! `.ods` 의 도형·그림·차트·묶음 (한셀 `.ods` 저장·열기 꼴 — docs/format/other-formats.md 17절).
//!
//! 개체는 시작 칸 안에 들어간다: 글 상자 도형은 `draw:frame` + `draw:text-box`, 타원 따위는 `draw:custom-shape`,
//! 연결선은 `draw:connector`, 그림은 `draw:image` (`Pictures/`), 차트는 `draw:object` 가 가리키는 하위 문서
//! `Object N/`, 묶음은 `draw:g`. `svg:x`·`svg:y` 는 시작 칸 안의 오프셋, `table:end-*` 는 끝 칸과 그 안의 오프셋.
//! 한셀의 차트 하위 문서는 계열 이름 칸·항목 범위만 담고 (막대·꺾은선·원형은 값도 빠진다) 읽을 때 이름은 지역 표 머리에서 가져온다.

use std::collections::HashMap;

use crate::model::{
    Anchor, AnchorKind, Chart, ChartAxis, ChartCache, ChartColor, ChartData, ChartFill,
    ChartGroupProps, ChartLine, ChartPoint, ChartSeries, ChartShapeProps, ChartTx, DrawColor,
    DrawingPara, DrawingRun, Marker, Outline, Paint, RawPart, Shape, ShapeKind, ShapeShadow,
    TextBody, Workbook, Xfrm,
};
use crate::ods_geom::GEOMS;
use crate::theme::Theme;
use crate::xml::Element;

const EMU_CM: f64 = 360_000.0;
const NS_DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
const NS_SVG: &str = "urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0";
const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_XLINK: &str = "http://www.w3.org/1999/xlink";
const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";
const NS_CHART: &str = "urn:oasis:names:tc:opendocument:xmlns:chart:1.0";
const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_FO: &str = "urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// EMU → `1.5cm` (소수 여섯째 자리까지, 끝 0 없이)
fn cm(emu: i64) -> String {
    let s = format!("{:.6}", emu as f64 / EMU_CM);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("{}cm", if s.is_empty() || s == "-0" { "0" } else { s })
}

/// 칸 안 오프셋 (cm 글) → EMU: 한셀은 96dpi 픽셀로 올려 읽는다 (`0.291042cm` → 12px — 한셀 .ods 열기 대조)
fn offset_emu(v: &str) -> Option<i64> {
    let v = v.trim();
    let cm = match v.strip_suffix("cm") {
        Some(n) => n.trim().parse::<f64>().ok()?,
        None => emu(v)? as f64 / EMU_CM,
    };
    let px = (cm * 96.0 / 2.54 - 1e-9).ceil().max(0.0);
    Some(px as i64 * 9_525)
}

/// 길이 (`1.5cm`·`3pt`·`2mm`·`1in`) → EMU
fn emu(v: &str) -> Option<i64> {
    let v = v.trim();
    let at = v.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(v.len());
    let (n, unit) = v.split_at(at);
    let n: f64 = n.trim().parse().ok()?;
    let k = match unit {
        "cm" | "" => EMU_CM,
        "mm" => EMU_CM / 10.0,
        "in" => 914_400.0,
        "pt" => 12_700.0,
        "px" => 9_525.0,
        _ => return None,
    };
    Some((n * k).round() as i64)
}

fn col_name(mut c: u32) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s
}

fn attr<'a>(e: &'a Element, ns: &str, n: &str) -> Option<&'a str> {
    e.attr_ns(ns, n)
}

/// 점선 이름 (한셀 저장 관찰): OOXML `prstDash` → ODF `draw:stroke-dash`
const DASHES: &[(&str, &str)] = &[
    ("dash", "Dashed"),
    ("lgDash", "LongDash"),
    ("dashDot", "DashDot"),
    ("lgDashDot", "LongDashDot"),
    ("lgDashDotDot", "LongDashDotDot"),
    ("sysDash", "SysDash"),
    ("sysDot", "SysDot"),
    ("dot", "SysDot"),
    ("sysDashDot", "LongDashDot"),
    ("sysDashDotDot", "LongDashDotDot"),
];

/// 쓰기에서 모으는 것: 그림 파일, 차트 하위 문서, 자동 스타일
pub(crate) struct Draw {
    /// (`Pictures/imageN.ext`, 바이트, 미디어 형식)
    pub media: Vec<(String, Vec<u8>, String)>,
    /// (`Object N`, content.xml, styles.xml)
    pub objects: Vec<(String, String, String)>,
    /// 도형 그래픽·문단·글자 자동 스타일 (content.xml)
    pub auto: String,
    /// styles.xml 에 둘 점선 정의 (`draw:stroke-dash`)
    pub dashes: Vec<&'static str>,
    theme: Theme,
    /// 테마를 가리키는 글꼴 이름(`+mn-ea` …)을 한셀이 바꾸는 글꼴: 테마 제목 글꼴의 영문 이름
    theme_font: String,
    gr: u32,
    p: u32,
    t: u32,
    drawings: u32,
}

/// 테마 파트 원문에서 `a:majorFont/a:latin` 의 글꼴 이름
fn major_latin(theme: &Theme) -> Option<String> {
    let xml = std::str::from_utf8(theme.source.as_deref()?).ok()?;
    let root = crate::xml::parse_raw(xml, "theme").ok()?;
    let mut out = None;
    root.walk(&mut |e| {
        if out.is_none()
            && e.name == "majorFont"
            && let Some(l) = e.elements().find(|c| c.name == "latin")
        {
            out = l
                .attr("typeface")
                .filter(|t| !t.is_empty())
                .map(str::to_string);
        }
    });
    out
}

impl Draw {
    pub fn new(theme: Theme) -> Self {
        Draw {
            theme_font: major_latin(&theme).unwrap_or_else(|| "Cambria".into()),
            media: Vec::new(),
            objects: Vec::new(),
            auto: String::new(),
            dashes: Vec::new(),
            theme,
            gr: 0,
            p: 0,
            t: 0,
            drawings: 0,
        }
    }

    /// styles.xml 의 점선 정의
    pub fn dash_defs(&self) -> String {
        self.dashes
            .iter()
            .map(|n| match *n {
                "SysDot" => r#"<draw:stroke-dash draw:name="SysDot" draw:style="rect" draw:dots1="1"/>"#,
                "Dashed" => r#"<draw:stroke-dash draw:name="Dashed" draw:style="rect" draw:dots2="1"/>"#,
                "LongDash" => r#"<draw:stroke-dash draw:name="LongDash" draw:style="rect" draw:dots2="1"/>"#,
                "DashDot" => r#"<draw:stroke-dash draw:name="DashDot" draw:style="rect" draw:dots1="1" draw:dots2="1"/>"#,
                "LongDashDot" => r#"<draw:stroke-dash draw:name="LongDashDot" draw:style="rect" draw:dots1="1" draw:dots2="1"/>"#,
                "LongDashDotDot" => r#"<draw:stroke-dash draw:name="LongDashDotDot" draw:style="rect" draw:dots1="1" draw:dots2="2"/>"#,
                _ => r#"<draw:stroke-dash draw:name="SysDash" draw:style="round" draw:dots2="1"/>"#,
            })
            .collect()
    }

    fn color(&self, c: &DrawColor) -> Option<String> {
        crate::render::draw::draw_color(c, &self.theme)
    }

    fn alpha(c: &DrawColor) -> Option<i64> {
        c.mods.iter().find(|(k, _)| k == "alpha").map(|m| m.1)
    }

    /// 도형 그래픽 자동 스타일 (한셀 저장 꼴). 이름을 돌려준다. 속성은 원본 개체가 가진 것만 쓴다 (한셀 저장 대조)
    fn graphic_style(&mut self, s: &Shape, text_empty: bool) -> String {
        self.gr += 1;
        let name = format!("gr{}", self.gr);
        let profile = profile(s);
        let full = profile == Profile::Full;
        let line = s.line.as_ref();
        let stroke_color = match line.and_then(|l| l.fill.as_ref()) {
            Some(Paint::Solid(c)) => self.color(c),
            _ => None,
        };
        let dash = line
            .and_then(|l| l.dash.as_deref())
            .and_then(|d| DASHES.iter().find(|x| x.0 == d))
            .map(|x| x.1);
        let mut p = String::new();
        // 선: 스타일 참조만 있는 도형(선 지정 없음)은 스타일의 선, 선 없음은 한셀 기본 선 값(0.5pt, #263E5B)
        let inherits = full && line.is_none();
        match (&stroke_color, dash) {
            (Some(_), Some(d)) => {
                if !self.dashes.contains(&d) {
                    self.dashes.push(d);
                }
                p.push_str(&format!(r#"draw:stroke="dash" draw:stroke-dash="{d}""#));
            }
            (Some(_), None) => p.push_str(r#"draw:stroke="solid""#),
            _ if inherits => p.push_str(r#"draw:stroke="solid""#),
            _ => p.push_str(r#"draw:stroke="none""#),
        }
        match &stroke_color {
            Some(c) => {
                let w = line.and_then(|l| l.width).unwrap_or(9525);
                p.push_str(&format!(
                    r#" svg:stroke-width="{}" svg:stroke-color="{c}""#,
                    cm(w)
                ));
            }
            None if full => p.push_str(r##" svg:stroke-width="0.5pt" svg:stroke-color="#263E5B""##),
            None => {}
        }
        if let Some(j) = line.and_then(|l| l.join.as_deref()) {
            let j = match j {
                "bevel" => "bevel",
                "miter" => "miter",
                _ => "round",
            };
            p.push_str(&format!(r#" draw:stroke-linejoin="{j}""#));
        }
        if let Some(c) = line.and_then(|l| l.cap.as_deref()) {
            let c = match c {
                "rnd" => "round",
                "sq" => "square",
                _ => "butt",
            };
            p.push_str(&format!(r#" svg:stroke-linecap="{c}""#));
        }
        let body = s.text.as_ref();
        let raw = s.raw.as_deref().unwrap_or("");
        if full || profile == Profile::TextBox {
            let grow = profile == Profile::TextBox && raw.contains("spAutoFit");
            p.push_str(&format!(r#" draw:auto-grow-height="{grow}""#));
        }
        if profile != Profile::Custom {
            match body.and_then(|t| t.anchor.as_deref()) {
                Some("ctr") => p.push_str(r#" draw:textarea-vertical-align="middle""#),
                Some("b") => p.push_str(r#" draw:textarea-vertical-align="bottom""#),
                _ => {}
            }
            let wrap = if body.and_then(|t| t.wrap.as_deref()) == Some("none") {
                "no-wrap"
            } else {
                "wrap"
            };
            p.push_str(&format!(r#" fo:wrap-option="{wrap}""#));
        }
        if let Some(sh) = &s.shadow {
            // 한셀은 아주 작은 그림자는 간격 0 으로 쓴다 (서식 문서 25개 모두 0cm)
            let (dx, dy) = if sh.dist <= 20_000 {
                (0.0, 0.0)
            } else {
                let a = (sh.dir as f64 / 60000.0).to_radians();
                (sh.dist as f64 * a.cos(), sh.dist as f64 * a.sin())
            };
            let col = self.color(&sh.color).unwrap_or_else(|| "#000000".into());
            p.push_str(&format!(
                r#" draw:shadow="visible" draw:shadow-offset-x="{}" draw:shadow-offset-y="{}" draw:shadow-color="{col}""#,
                cm(dx.round() as i64),
                cm(dy.round() as i64)
            ));
            if let Some(a) = Self::alpha(&sh.color) {
                p.push_str(&format!(r#" draw:shadow-opacity="{}%""#, a / 1000));
            }
        }
        if full {
            p.push_str(r#" fo:min-width="3.5cm" fo:min-height="3.95cm""#);
        }
        // 안쪽 여백: 정해 두었으면 그 값, 글이 없으면 한셀 기본 여백
        let ins = body.map(|t| t.insets).unwrap_or_default();
        if profile != Profile::Custom {
            if ins.iter().any(Option::is_some) {
                let v = |i: usize, d: i64| cm(ins[i].unwrap_or(d));
                p.push_str(&format!(
                    r#" fo:padding-top="{}" fo:padding-bottom="{}" fo:padding-left="{}" fo:padding-right="{}""#,
                    v(1, 45720),
                    v(3, 45720),
                    v(0, 91440),
                    v(2, 91440)
                ));
            } else if text_empty && full {
                p.push_str(r#" fo:padding-top="0.130528cm" fo:padding-bottom="0.130528cm" fo:padding-left="0.250472cm" fo:padding-right="0.250472cm""#);
            }
        }
        let (fill, fill_color, opacity) = match &s.fill {
            Some(Paint::Solid(c)) => match self.color(c) {
                Some(h) => ("solid", Some(h), Self::alpha(c)),
                None => ("none", None, None),
            },
            // 채우기 지정이 없으면 스타일의 채우기 (강조 1)
            None if full => ("solid", Some("#4F81BD".to_string()), None),
            _ => ("none", None, None),
        };
        p.push_str(&format!(r#" draw:fill="{fill}""#));
        match fill_color {
            Some(c) => p.push_str(&format!(r#" draw:fill-color="{c}""#)),
            None if full => p.push_str(r##" draw:fill-color="#4F81BD""##),
            None => {}
        }
        if let Some(a) = opacity {
            p.push_str(&format!(r#" draw:opacity="{}%""#, a / 1000));
        }
        if profile == Profile::TextBox {
            p.push_str(r#" style:shrink-to-fit="false""#);
        }
        let vert = if body.and_then(|t| t.vert.as_deref()) == Some("vert") {
            r#"<style:paragraph-properties style:writing-mode="tb-rl"/>"#
        } else {
            ""
        };
        self.auto.push_str(&format!(
            r#"<style:style style:name="{name}" style:family="graphic"><style:graphic-properties {p}/>{vert}</style:style>"#
        ));
        name
    }

    fn picture_style(&mut self, s: &Shape) -> String {
        self.gr += 1;
        let name = format!("gr{}", self.gr);
        let line = s.line.as_ref().and_then(|l| match &l.fill {
            Some(Paint::Solid(c)) => self.color(c).map(|c| (c, l.width.unwrap_or(9525))),
            _ => None,
        });
        let p = match line {
            Some((c, w)) => format!(
                r#"draw:stroke="solid" svg:stroke-width="{}" svg:stroke-color="{c}" draw:fill="none""#,
                cm(w)
            ),
            None => r#"draw:stroke="none" draw:fill="none""#.to_string(),
        };
        self.auto.push_str(&format!(
            r#"<style:style style:name="{name}" style:family="graphic"><style:graphic-properties {p}/></style:style>"#
        ));
        name
    }

    fn connector_style(&mut self, s: &Shape) -> String {
        self.gr += 1;
        let name = format!("gr{}", self.gr);
        let line = s.line.as_ref();
        let color = line
            .and_then(|l| match &l.fill {
                Some(Paint::Solid(c)) => self.color(c),
                _ => None,
            })
            .unwrap_or_else(|| "#000000".into());
        let w = line.and_then(|l| l.width).unwrap_or(9525);
        let mut p = format!(
            r#"draw:stroke="solid" svg:stroke-width="{}" svg:stroke-color="{color}""#,
            cm(w)
        );
        if line
            .and_then(|l| l.tail.as_ref())
            .and_then(|e| e.kind.as_deref())
            .is_some_and(|k| k != "none")
        {
            p.push_str(r#" draw:marker-end="Arrow""#);
        }
        p.push_str(r#" draw:fill="none""#);
        self.auto.push_str(&format!(
            r#"<style:style style:name="{name}" style:family="graphic"><style:graphic-properties {p}/></style:style>"#
        ));
        name
    }

    /// 글 상자 안 문단·글자 (자동 스타일 P·T)
    fn text_xml(&mut self, tb: Option<&TextBody>, center: bool) -> String {
        let Some(tb) = tb else {
            return String::new();
        };
        let mut out = String::new();
        for para in &tb.paragraphs {
            self.p += 1;
            let align = match para.align.as_deref() {
                Some("ctr") => "center",
                Some("r") => "end",
                Some("just") | Some("dist") => "justify",
                Some("l") => "start",
                // 정렬이 없으면 한셀은 글 상자(txBox)는 가운데, 그 밖은 왼쪽으로 쓴다 (한셀 저장 대조)
                _ if center => "center",
                _ => "start",
            };
            let pn = format!("P{}", self.p);
            self.auto.push_str(&format!(
                r#"<style:style style:name="{pn}" style:family="paragraph"><style:paragraph-properties fo:text-align="{align}"/></style:style>"#
            ));
            if para.runs.is_empty() {
                out.push_str(&format!(r#"<text:p text:style-name="{pn}"/>"#));
                continue;
            }
            out.push_str(&format!(r#"<text:p text:style-name="{pn}">"#));
            for r in &para.runs {
                self.t += 1;
                let tn = format!("T{}", self.t);
                let st = self.run_style(r);
                self.auto.push_str(&format!(
                    r#"<style:style style:name="{tn}" style:family="text"><style:text-properties {st}/></style:style>"#
                ));
                let keep =
                    r.text.starts_with(' ') || r.text.ends_with(' ') || r.text.contains("  ");
                let text = esc(&r.text).replace('\n', "<text:line-break/>");
                out.push_str(&format!(
                    r#"<text:span text:style-name="{tn}"{}>{text}</text:span>"#,
                    if keep { r#" xml:space="preserve""# } else { "" }
                ));
            }
            out.push_str("</text:p>");
        }
        out
    }

    fn run_style(&self, r: &DrawingRun) -> String {
        let mut st = String::new();
        match r.color.as_ref().and_then(|c| self.color(c)) {
            Some(c) => st.push_str(&format!(r#"fo:color="{c}""#)),
            None => st.push_str(r#"style:use-window-font-color="true""#),
        }
        let latin = r
            .latin
            .as_deref()
            .filter(|f| !f.is_empty() && !f.starts_with('+'));
        let asian =
            r.ea.as_deref()
                .filter(|f| !f.is_empty() && !f.starts_with('+'))
                .or(latin);
        let theme_font = Some(self.theme_font.as_str());
        if let Some(f) = latin.or(theme_font) {
            let a = esc(asian.or(theme_font).unwrap_or(f));
            let f = esc(f);
            // 한컴 글꼴은 고딕 계열(modern)로, 복합 문자 글꼴도 지정한다 (한셀 저장 관찰)
            let modern = f.starts_with("함초롬");
            if latin.is_some() {
                st.push_str(&format!(r#" style:font-name="{f}""#));
            }
            st.push_str(&format!(
                r#" fo:font-family="{f}" style:font-family-asian="{a}""#
            ));
            if modern {
                st.push_str(&format!(r#" style:font-family-complex="{f}""#));
            }
            if latin.is_none() {
                st.push_str(r#" style:font-family-generic="roman""#);
            }
            st.push_str(&format!(
                r#" style:font-family-generic-asian="{}""#,
                if modern { "modern" } else { "roman" }
            ));
            if modern {
                st.push_str(r#" style:font-family-generic-complex="modern""#);
            }
        }
        let size = r
            .size
            .map_or("10".to_string(), |s| format!("{}", f64::from(s) / 100.0));
        st.push_str(&format!(
            r#" fo:font-size="{size}pt" style:font-size-asian="{size}pt" style:font-size-complex="{size}pt""#
        ));
        if r.italic == Some(true) {
            st.push_str(r#" fo:font-style="italic" style:font-style-asian="italic""#);
        }
        let underline = r.underline.as_deref().filter(|u| *u != "none");
        if let Some(u) = underline {
            let ty = if u == "dbl" { "double" } else { "single" };
            st.push_str(&format!(
                r#" style:text-underline-style="solid" style:text-underline-type="{ty}" style:text-underline-color="font-color""#
            ));
        }
        if r.bold == Some(true) {
            st.push_str(r#" fo:font-weight="bold" style:font-weight-asian="bold""#);
        }
        if underline.is_some() {
            st.push_str(r#" style:text-underline-mode="continuous""#);
        }
        st
    }
}

/// 도형 스타일의 꼴. 한셀은 원본 개체가 가진 속성만 쓴다: 스타일 참조(`xdr:style`)가 있으면 전부(Full),
/// 글 상자(`txBox`)는 TextBox, 그 밖은 Minimal, 경로가 없는 사용자 정의 도형은 스타일만 쓰고 개체는 쓰지 않는다 (Custom)
#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Full,
    TextBox,
    Minimal,
    Custom,
}

fn profile(s: &Shape) -> Profile {
    if s.kind == ShapeKind::Shape && matches!(s.geometry.as_deref(), None | Some("custom")) {
        return Profile::Custom;
    }
    match s.raw.as_deref() {
        Some(r) if r.contains("<xdr:style") => Profile::Full,
        Some(r) if r.contains("txBox=\"1\"") => Profile::TextBox,
        Some(_) => Profile::Minimal,
        None => Profile::Full,
    }
}

/// 시트의 도형 → (행, 열, `draw:frame`·`draw:g` 글). 시작 칸 안에 쓴다. 스타일 번호는 그리기 차례(앞 도형부터)
pub(crate) fn frames(wb: &Workbook, si: usize, d: &mut Draw) -> Vec<(u32, u32, String)> {
    let sheet = &wb.sheets[si];
    if sheet.shapes.is_empty() {
        return Vec::new();
    }
    let drawing = d.drawings;
    d.drawings += 1;
    let mut out = Vec::new();
    for s in &sheet.shapes {
        let host = s.anchor.from;
        let (row, col) = if s.anchor.kind == AnchorKind::Absolute {
            (0, 0)
        } else {
            (host.row, host.col)
        };
        if let Some(x) = shape_xml(wb, si, drawing, s, false, d) {
            out.push((row, col, x));
        }
    }
    out
}

fn end_attrs(wb: &Workbook, si: usize, a: &Anchor) -> String {
    let Some(t) = a.to else {
        return String::new();
    };
    format!(
        r#" table:end-cell-address="{}.{}{}" table:end-x="{}" table:end-y="{}""#,
        sheet_ref(&wb.sheets[si].name),
        col_name(t.col),
        t.row + 1,
        cm(t.col_off),
        cm(t.row_off)
    )
}

/// 시트 이름 (공백·기호가 있으면 따옴표)
pub(crate) fn sheet_ref(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        esc(name)
    } else {
        esc(&format!("'{}'", name.replace('\'', "''")))
    }
}

fn shape_xml(
    wb: &Workbook,
    si: usize,
    drawing: u32,
    s: &Shape,
    child: bool,
    d: &mut Draw,
) -> Option<String> {
    let sheet = &wb.sheets[si];
    let mdw = crate::units::max_digit_width(wb);
    let a = &s.anchor;
    // 크기: 도형의 xfrm → 앵커. 그림은 두 칸 앵커의 칸 사이 크기 (한셀 저장 대조: xfrm 이 아니라 앵커가 정한다)
    let by_cells = s.kind == ShapeKind::Picture && a.to.is_some() && !child;
    let (w, h) = match (&s.xfrm, a.ext) {
        _ if by_cells => {
            let r = crate::writer::anchor_rect(sheet, mdw, a);
            (r.2, r.3)
        }
        (Some(x), _) if x.cx > 0 || x.cy > 0 => (x.cx, x.cy),
        (_, Some((cx, cy))) => (cx, cy),
        _ => {
            let r = crate::writer::anchor_rect(sheet, mdw, a);
            (r.2, r.3)
        }
    };
    let (x, y) = if child {
        s.xfrm.as_ref().map_or((0, 0), |x| (x.x, x.y))
    } else if a.kind == AnchorKind::Absolute {
        a.pos.unwrap_or((0, 0))
    } else {
        (a.from.col_off, a.from.row_off)
    };
    let pos = format!(
        r#" svg:width="{}" svg:height="{}" svg:x="{}" svg:y="{}""#,
        cm(w),
        cm(h),
        cm(x),
        cm(y)
    );
    let end = if child {
        String::new()
    } else {
        end_attrs(wb, si, a)
    };
    let id = format!("drawing{drawing}.xml-{}", s.id);
    match s.kind {
        ShapeKind::Group => {
            let kids: String = s
                .children
                .iter()
                .filter_map(|c| shape_xml(wb, si, drawing, c, true, d))
                .collect();
            let gx = if !child && a.from.col_off != 0 {
                format!(r#" svg:x="{}""#, cm(a.from.col_off))
            } else {
                String::new()
            };
            let gy = if child {
                String::new()
            } else {
                format!(r#" svg:y="{}""#, cm(a.from.row_off))
            };
            Some(format!("<draw:g{gx}{gy}{end}>{kids}</draw:g>"))
        }
        ShapeKind::Chart => {
            // 한셀은 제목이 있는 차트를 .ods 에 쓰지 못한다 (제목이 있는 차트 24개가 모두 빠짐)
            let ch = s.chart.as_ref().filter(|c| c.title.is_none())?;
            let n = d.objects.len() + 1;
            let dir = format!("Object {n}");
            let (content, styles) = chart_doc(wb, si, ch, w, h);
            d.objects.push((dir.clone(), content, styles));
            Some(format!(
                r#"<draw:frame{pos}{end}><draw:object xlink:href="{dir}/" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/><svg:title/><svg:desc/></draw:frame>"#
            ))
        }
        ShapeKind::Picture => {
            let part = s
                .target
                .as_ref()
                .and_then(|t| wb.raw_parts.iter().find(|p| &p.name == t))?;
            let info = crate::image::info(&part.data);
            let (ext, mime) = crate::image::ext_and_type(info.map_or("png", |i| i.format));
            let href = format!("Pictures/image{}.{ext}", d.media.len());
            d.media
                .push((href.clone(), part.data.clone(), mime.to_string()));
            let st = d.picture_style(s);
            Some(format!(
                r#"<draw:frame draw:name="{}" draw:style-name="{st}"{pos} draw:id="{id}"{end}><draw:image xlink:href="{href}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>"#,
                esc(&s.name)
            ))
        }
        ShapeKind::Connector => {
            let (rx, ry, rw, rh) = if a.kind == AnchorKind::Absolute {
                let (px, py) = a.pos.unwrap_or((0, 0));
                (px, py, w, h)
            } else {
                crate::writer::anchor_rect(sheet, mdw, a)
            };
            let (fh, fv) = s
                .xfrm
                .as_ref()
                .map_or((false, false), |x| (x.flip_h, x.flip_v));
            let (x1, x2) = if fh { (rx + rw, rx) } else { (rx, rx + rw) };
            let (y1, y2) = if fv { (ry + rh, ry) } else { (ry, ry + rh) };
            let ty = match s.geometry.as_deref() {
                Some(g) if g.starts_with("bentConnector") => "standard",
                Some(g) if g.starts_with("curvedConnector") => "curve",
                _ => "line",
            };
            let st = d.connector_style(s);
            Some(format!(
                r#"<draw:connector draw:type="{ty}" svg:x1="{}" svg:y1="{}" svg:x2="{}" svg:y2="{}" draw:id="{id}" draw:style-name="{st}" draw:name="{}"{end}/>"#,
                cm(x1),
                cm(y1),
                cm(x2),
                cm(y2),
                esc(&s.name)
            ))
        }
        _ => {
            let empty = s.text.as_ref().is_none_or(|t| t.paragraphs.is_empty());
            let st = d.graphic_style(s, empty);
            if profile(s) == Profile::Custom {
                return None;
            }
            let text = d.text_xml(s.text.as_ref(), profile(s) == Profile::TextBox);
            let geom = s
                .geometry
                .as_deref()
                .and_then(|g| GEOMS.iter().find(|x| x.0 == g));
            match geom {
                Some((_, _, xml)) => Some(format!(
                    r#"<draw:custom-shape svg:x="{}" svg:y="{}" svg:width="{}" svg:height="{}" draw:id="{id}" draw:layer="layout" draw:style-name="{st}" draw:name="{}"{end}>{text}{xml}</draw:custom-shape>"#,
                    cm(x),
                    cm(y),
                    cm(w),
                    cm(h),
                    esc(&s.name)
                )),
                None => Some(format!(
                    r#"<draw:frame draw:name="{}" draw:style-name="{st}"{pos} draw:id="{id}"{end}><draw:text-box>{text}</draw:text-box></draw:frame>"#,
                    esc(&s.name)
                )),
            }
        }
    }
}

/// 셀 참조 `시트!$C$1:$C$6` → 차트 하위 문서의 주소. 영문 시트 이름은 한셀이 `local-table.$C$1:.$C$6` 로,
/// 그 밖의 이름은 `서식!$E$6:.$P$6` 로 쓴다 (한셀 저장 관찰)
fn local_ref(f: &str) -> String {
    let (sheet, cells) = f.rsplit_once('!').unwrap_or(("", f));
    let sheet = sheet.trim_matches('\'');
    let abs = |c: &str| {
        let c = c.replace('$', "");
        let col: String = c
            .chars()
            .take_while(|ch| ch.is_ascii_alphabetic())
            .collect();
        let row: String = c
            .chars()
            .skip_while(|ch| ch.is_ascii_alphabetic())
            .collect();
        format!("${col}${row}")
    };
    let local = sheet.is_empty() || sheet.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let head = if local {
        "local-table.".to_string()
    } else {
        format!("{sheet}!")
    };
    match cells.split_once(':') {
        Some((a, b)) => format!("{head}{}:.{}", abs(a), abs(b)),
        None => format!("{head}{}", abs(cells)),
    }
}

fn chart_class(ch: &Chart) -> &'static str {
    match ch.kind.as_str() {
        "line" | "line3D" => "chart:line",
        "pie" | "pie3D" | "ofPie" => "chart:circle",
        "doughnut" => "chart:ring",
        "area" | "area3D" => "chart:area",
        "scatter" => "chart:scatter",
        "radar" => "chart:radar",
        "bubble" => "chart:bubble",
        "stock" => "chart:stock",
        _ => "chart:bar",
    }
}

const PALETTE: [&str; 8] = [
    "6182D6", "FF843A", "9ACE5B", "FFCE3A", "A07CD6", "5FC0C8", "D66161", "8C8C8C",
];

/// 차트 하위 문서 (content.xml, styles.xml) — 한셀이 쓰는 꼴: 종류, 범례, 항목 축의 범위, 계열의 이름 칸, 지역 표
fn chart_doc(wb: &Workbook, _si: usize, ch: &Chart, w: i64, h: i64) -> (String, String) {
    let ns = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:dr3d="urn:oasis:names:tc:opendocument:xmlns:dr3d:1.0" xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2""#;
    let theme = wb.theme.clone().unwrap_or_default();
    let class = chart_class(ch);
    let pie = matches!(class, "chart:circle" | "chart:ring");
    let scatter = class == "chart:scatter";
    let with_values = matches!(class, "chart:area" | "chart:scatter");
    let three_d = ch.three_d;
    let bar = class == "chart:bar";
    let stacked = matches!(ch.grouping.as_deref(), Some("stacked" | "percentStacked"));
    let percent = ch.grouping.as_deref() == Some("percentStacked");
    let horizontal = bar && ch.bar_dir.as_deref() == Some("bar");
    // 이름이 글자(`c:v`)인 계열은 한셀이 .ods 에 쓰지 않는다 (차트 `chart_litname`: 계열 없이 항목만)
    let series: Vec<&ChartSeries> = ch
        .series
        .iter()
        .filter(|s| !matches!(s.tx, Some(ChartTx::Value(_))))
        .collect();
    let mut styles: Vec<String> = Vec::new();
    let mut add = |s: String| -> String {
        styles.push(s);
        format!("Chart{}", styles.len())
    };
    let none_gfx = r#"<style:graphic-properties draw:stroke="none" draw:fill="none"/>"#;
    let solid_gfx = r#"<style:graphic-properties draw:stroke="solid" draw:fill="none"/>"#;
    let text10 = r#"<style:text-properties fo:font-size="10pt" style:font-size-asian="10pt" style:font-size-complex="10pt"/>"#;
    let legend_style = ch.legend.is_some().then(|| {
        add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid"/>{none_gfx}{text10}</style:style>"#
        ))
    });
    let mut plot_props = String::from(r#"chart:deep="false""#);
    if three_d {
        plot_props = String::from(r#"chart:three-dimensional="true" chart:deep="true""#);
    }
    if horizontal {
        plot_props.push_str(r#" chart:vertical="true""#);
    }
    if class == "chart:line" {
        plot_props.push_str(r#" chart:symbol-type="automatic""#);
    }
    if scatter {
        plot_props.push_str(r#" chart:symbol-type="automatic" chart:symbol-name="diamond""#);
    }
    if bar && !three_d {
        plot_props.push_str(&format!(
            r#" chart:overlap="{}""#,
            if stacked { 100 } else { 0 }
        ));
    }
    plot_props.push_str(r#" chart:solid-type="cuboid""#);
    if !pie && !scatter {
        plot_props.push_str(&format!(
            r#" chart:stacked="false" chart:percentage="{percent}""#
        ));
    }
    if three_d && !pie {
        plot_props.push_str(r#" chart:right-angled-axes="true""#);
    }
    // 그림 영역 테두리: 선 속성(꺾임·끝)이 원본에 있을 때만 쓴다
    let plot_line = ch.plot_sp_pr.as_ref().and_then(|p| p.line.as_ref());
    let mut plot_gfx = String::from(r#"draw:stroke="none""#);
    if plot_line.is_some_and(|l| l.join.is_some()) {
        plot_gfx.push_str(r#" draw:stroke-linejoin="round""#);
    }
    if plot_line.is_some_and(|l| l.cap.is_some()) {
        plot_gfx.push_str(r#" svg:stroke-linecap="butt""#);
    }
    let plot_style = add(format!(
        r#"<style:style style:name="@" style:family="chart"><style:chart-properties {plot_props}/><style:graphic-properties {plot_gfx} draw:fill="none"/></style:style>"#
    ));
    // 계열 스타일 (분산형은 축보다 앞)
    let mut series_styles: Vec<String> = Vec::new();
    let mut make_series = |add: &mut dyn FnMut(String) -> String| {
        for (i, s) in series.iter().enumerate() {
            let default = format!("#{}", PALETTE[i % PALETTE.len()]);
            let draw = |c: &ChartColor| crate::render::draw::draw_color(&c.to_draw(), &theme);
            // 서식이 있으면 그 채우기·선 (선이 없으면 선 없음), 없으면 기본 색에 선·채우기·불투명 100%
            let (fill, stroke) = match &s.sp_pr {
                Some(p) => (
                    match &p.fill {
                        Some(ChartFill::Solid(c)) => draw(c),
                        _ => None,
                    },
                    match p.line.as_ref().and_then(|l| l.fill.as_ref()) {
                        Some(ChartFill::Solid(c)) => draw(c),
                        _ => None,
                    },
                ),
                None => (Some(default.clone()), Some(default.clone())),
            };
            let default_look = s.sp_pr.is_none();
            let props = if scatter {
                r#"chart:symbol-type="automatic" chart:solid-type="cuboid""#
            } else if pie {
                r#"chart:solid-type="cuboid""#
            } else {
                r#"chart:solid-type="cuboid" chart:stacked="false" chart:percentage="false""#
            };
            let mut gfx = match &stroke {
                Some(c) => format!(r#"draw:stroke="solid" svg:stroke-color="{c}""#),
                None => r#"draw:stroke="none""#.to_string(),
            };
            match &fill {
                Some(c) => gfx.push_str(&format!(r#" draw:fill="solid" draw:fill-color="{c}""#)),
                None => gfx.push_str(r#" draw:fill="none""#),
            }
            if default_look {
                gfx.push_str(r#" draw:opacity="100%""#);
            }
            series_styles.push(add(format!(
                r#"<style:style style:name="@" style:family="chart"><style:chart-properties {props}/><style:graphic-properties {gfx}/></style:style>"#
            )));
        }
    };
    if scatter {
        make_series(&mut add);
    }
    let (x_style, y_style) = if pie {
        let s = add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid" chart:display-label="false" chart:reverse-direction="true"/>{none_gfx}</style:style>"#
        ));
        (s.clone(), s)
    } else if scatter {
        let a = add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid" chart:link-data-style-to-source="true" chart:display-label="true"/>{solid_gfx}</style:style>"#
        ));
        let b = add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid" chart:link-data-style-to-source="true" chart:display-label="true"/>{solid_gfx}</style:style>"#
        ));
        (a, b)
    } else {
        // 항목 축의 표시 형식이 있으면 원본 연결 여부를 쓴다
        let link = ch
            .axes
            .iter()
            .find(|a| a.kind == "catAx" || a.kind == "dateAx")
            .and_then(|a| a.num_fmt.as_ref())
            .and_then(|n| n.source_linked)
            .map(|v| format!(r#" chart:link-data-style-to-source="{v}""#))
            .unwrap_or_default();
        let a = add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:gap-width="150" chart:overlap="0" chart:solid-type="cuboid"{link} chart:display-label="true"/>{solid_gfx}{text10}</style:style>"#
        ));
        let b = add(format!(
            r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid" chart:link-data-style-to-source="true" chart:display-label="true"/>{solid_gfx}</style:style>"#
        ));
        (a, b)
    };
    if !scatter {
        make_series(&mut add);
    }
    let wall_style = add(format!(
        r#"<style:style style:name="@" style:family="chart"><style:chart-properties chart:solid-type="cuboid"/>{none_gfx}</style:style>"#
    ));
    let legend_el = match (&ch.legend, &legend_style) {
        (Some(pos), Some(st)) => {
            let p = match pos.as_str() {
                "b" => "bottom",
                "l" => "start",
                "t" | "tr" => "top",
                _ => "end",
            };
            format!(
                r#"<chart:legend chart:legend-position="{p}" style:legend-expansion="wide" style:legend-expansion-aspect-ratio="0" chart:style-name="{st}"/>"#
            )
        }
        _ => String::new(),
    };
    let cat_ref = ch.series.first().and_then(|s| s.cat_ref.as_deref());
    let cats_el = match (scatter, cat_ref) {
        (false, Some(r)) => format!(
            r#"<chart:categories table:cell-range-address="{}"/>"#,
            esc(&local_ref(r))
        ),
        _ => String::new(),
    };
    let z_axis = if three_d && !pie {
        format!(
            r#"<chart:axis chart:dimension="z" chart:name="primary-z" chart:style-name="{x_style}"/>"#
        )
    } else {
        String::new()
    };
    let axes = if scatter {
        format!(
            r#"<chart:axis chart:dimension="x" chart:name="primary-x" chart:style-name="{x_style}"/><chart:axis chart:dimension="y" chart:name="primary-y" chart:style-name="{y_style}"/>"#
        )
    } else {
        format!(
            r#"<chart:axis chart:dimension="x" chart:name="primary-x" chart:style-name="{x_style}">{cats_el}</chart:axis><chart:axis chart:dimension="y" chart:name="primary-y" chart:style-name="{y_style}"/>{z_axis}"#
        )
    };
    let series_el: String = series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let label = s
                .name_ref
                .as_deref()
                .map(|r| format!(r#" chart:label-cell-address="{}""#, esc(&local_ref(r))))
                .unwrap_or_default();
            let values = match (with_values, s.val_ref.as_deref()) {
                (true, Some(r)) => format!(
                    r#" chart:values-cell-range-address="{}""#,
                    esc(&local_ref(r))
                ),
                _ => String::new(),
            };
            let st = &series_styles[i];
            if values.is_empty() {
                format!(r#"<chart:series{label} chart:style-name="{st}"/>"#)
            } else {
                format!(
                    r#"<chart:series{values}{label} chart:style-name="{st}"><chart:data-point chart:repeated="{}"/></chart:series>"#,
                    s.values.len().max(s.categories.len())
                )
            }
        })
        .collect();
    // 지역 표: 머리 행 (빈 칸 + 계열 이름) + 행마다 항목 값 (+ 값 칸)
    let head: String =
        std::iter::once("<table:table-cell><text:p/></table:table-cell>".to_string())
            .chain(series.iter().map(|s| {
                format!(
                    r#"<table:table-cell office:value-type="string"><text:p>{}</text:p></table:table-cell>"#,
                    esc(s.name.as_deref().unwrap_or(""))
                )
            }))
            .collect();
    let cats = ch
        .series
        .first()
        .map(|s| s.categories.clone())
        .unwrap_or_default();
    let cats = if cats.is_empty() {
        series
            .first()
            .map(|s| s.categories.clone())
            .unwrap_or_default()
    } else {
        cats
    };
    let cell = |t: &str| match t.parse::<f64>() {
        Ok(v) if t.trim() == t && !t.is_empty() => format!(
            r#"<table:table-cell office:value-type="float" office:value="{v}"><text:p>{}</text:p></table:table-cell>"#,
            esc(t)
        ),
        _ => format!(
            r#"<table:table-cell office:value-type="string"><text:p>{}</text:p></table:table-cell>"#,
            esc(t)
        ),
    };
    let nrows = if with_values {
        cats.len()
            .max(series.iter().map(|s| s.values.len()).max().unwrap_or(0))
    } else {
        cats.len()
    };
    let rows: String = (0..nrows)
        .map(|r| {
            let mut row = cell(cats.get(r).map_or("", |s| s.as_str()));
            if with_values {
                for s in &series {
                    match s.values.get(r).copied().flatten() {
                        Some(v) => row.push_str(&format!(
                            r#"<table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell>"#
                        )),
                        None => row.push_str("<table:table-cell/>"),
                    }
                }
            }
            format!("<table:table-row>{row}</table:table-row>")
        })
        .collect();
    let dr3d = if three_d && !pie {
        r#" dr3d:vrp="(17634.621837378301 10271.482381764699 24594.863908273899 )" dr3d:vpn="(0.416199821709347 0.173649045905254 0.892537795986984)" dr3d:vup="(-0.0733876362771618 0.984807599917971 -0.157379306090273)" dr3d:projection="perspective" dr3d:distance="4.2cm" dr3d:focal-length="8cm" dr3d:transform="matrix (0.99564446382742067 -2.9808720977666348E-2 -8.833765790127851E-2 3.6833428844505495E-2 0.99619477459360894 7.8989047291030706E-2 8.5646950730101257E-2 -8.1898786474914415E-2 0.99295376962100956 0cm 0cm 0cm)""#
    } else {
        ""
    };
    let light = if three_d && !pie {
        r#"<dr3d:light dr3d:direction=""/>"#
    } else {
        ""
    };
    let autos: String = styles
        .iter()
        .enumerate()
        .map(|(i, s)| s.replacen("name=\"@\"", &format!("name=\"Chart{}\"", i + 1), 1))
        .collect();
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {ns} office:version="1.2"><office:automatic-styles>{autos}</office:automatic-styles><office:body><office:chart><chart:chart chart:class="{class}" svg:width="{}" svg:height="{}">{legend_el}<chart:plot-area chart:style-name="{plot_style}" chart:data-source-has-labels="both"{dr3d}>{axes}{light}{series_el}<chart:wall chart:style-name="{wall_style}"/></chart:plot-area><table:table table:name="local-table"><table:table-header-columns><table:table-column/></table:table-header-columns><table:table-columns><table:table-column table:number-columns-repeated="{}"/></table:table-columns><table:table-header-rows><table:table-row>{head}</table:table-row></table:table-header-rows><table:table-rows>{rows}</table:table-rows></table:table></chart:chart></office:chart></office:body></office:document-content>"#,
        cm(w),
        cm(h),
        series.len()
    );
    let st = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {ns} office:version="1.2"><office:styles/><office:automatic-styles/></office:document-styles>"#
    );
    (content, st)
}

// ---------------------------------------------------------------------------
// 읽기
// ---------------------------------------------------------------------------

type Props = Vec<(String, String)>;

/// 자동 스타일: 그래픽(이름 → 속성), 문단(이름 → 정렬), 글자(이름 → 속성)
#[derive(Default)]
pub(crate) struct Styles {
    graphic: HashMap<String, GfxProps>,
    para: HashMap<String, String>,
    text: HashMap<String, Props>,
}

#[derive(Default, Clone)]
struct GfxProps {
    attrs: Props,
    vertical: bool,
}

impl GfxProps {
    fn get(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|a| a.0 == k).map(|a| a.1.as_str())
    }
}

/// 요소의 속성 → (접두사 붙인 이름, 값)
fn flat_attrs(p: &Element) -> Props {
    p.attrs
        .iter()
        .map(|a| {
            let pre = match a.ns.as_deref() {
                Some(NS_DRAW) => "draw:",
                Some(NS_SVG) => "svg:",
                Some(NS_FO) => "fo:",
                Some(NS_STYLE) => "style:",
                Some(NS_CHART) => "chart:",
                _ => "",
            };
            (format!("{pre}{}", a.name), a.value.clone())
        })
        .collect()
}

impl Styles {
    /// content.xml 루트의 `office:automatic-styles` 에서 모은다
    pub fn new(root: &Element) -> Self {
        let mut s = Styles::default();
        let Some(auto) = root.child_ns(NS_OFFICE, "automatic-styles") else {
            return s;
        };
        for st in auto.elements().filter(|e| e.is(NS_STYLE, "style")) {
            let (Some(name), Some(family)) =
                (attr(st, NS_STYLE, "name"), attr(st, NS_STYLE, "family"))
            else {
                continue;
            };
            let props = |tag: &str| {
                st.child_ns(NS_STYLE, tag)
                    .map(flat_attrs)
                    .unwrap_or_default()
            };
            match family {
                "graphic" => {
                    let vertical = st
                        .child_ns(NS_STYLE, "paragraph-properties")
                        .and_then(|p| attr(p, NS_STYLE, "writing-mode"))
                        .is_some_and(|m| m.starts_with("tb"));
                    s.graphic.insert(
                        name.to_string(),
                        GfxProps {
                            attrs: props("graphic-properties"),
                            vertical,
                        },
                    );
                }
                "paragraph" => {
                    let align = props("paragraph-properties")
                        .into_iter()
                        .find(|a| a.0 == "fo:text-align")
                        .map(|a| a.1)
                        .unwrap_or_default();
                    s.para.insert(name.to_string(), align);
                }
                "text" => {
                    s.text.insert(name.to_string(), props("text-properties"));
                }
                _ => {}
            }
        }
        s
    }
}

/// 읽기에서 쓰는 것: 압축 파일 바이트, 시트 이름, 스타일, 마지막으로 준 번호
pub(crate) struct ReadCtx<'a> {
    pub bytes: &'a [u8],
    pub sheet: &'a str,
    pub next_id: u32,
    pub styles: &'a Styles,
}

/// 칸 하나 안의 `draw:frame`·`draw:g`·`draw:custom-shape`·`draw:connector` → 도형들 (시작 칸 (row, col))
pub(crate) fn read_frames(
    cell: &Element,
    (row, col): (u32, u32),
    cx: &mut ReadCtx,
    parts: &mut Vec<RawPart>,
) -> Vec<Shape> {
    cell.elements()
        .filter_map(|e| read_shape(e, (row, col), cx, parts, false))
        .collect()
}

/// `Sheet1.N17` · `'내 시트'.F25` → (열, 행) (0부터)
fn end_cell(a: &str) -> Option<(u32, u32)> {
    let cell = a.rsplit('.').next()?.replace('$', "");
    let col: String = cell
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let row: u32 = cell[col.len()..].parse().ok()?;
    let c = col
        .to_ascii_uppercase()
        .bytes()
        .fold(0u32, |n, b| n * 26 + u32::from(b - b'A') + 1);
    (c > 0 && row > 0).then(|| (c - 1, row - 1))
}

fn hex_color(v: &str) -> Option<DrawColor> {
    let h = v.strip_prefix('#')?;
    (h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| DrawColor {
        value: format!("srgb:{}", h.to_ascii_uppercase()),
        mods: Vec::new(),
    })
}

fn pct(v: &str) -> Option<i64> {
    v.trim()
        .trim_end_matches('%')
        .parse::<f64>()
        .ok()
        .map(|p| (p * 1000.0).round() as i64)
}

fn read_shape(
    e: &Element,
    (row, col): (u32, u32),
    cx: &mut ReadCtx,
    parts: &mut Vec<RawPart>,
    child: bool,
) -> Option<Shape> {
    let kind = ["g", "frame", "custom-shape", "connector"]
        .into_iter()
        .find(|k| e.is(NS_DRAW, k))?;
    let len = |n: &str| attr(e, NS_SVG, n).and_then(emu);
    let (mut x, mut y) = (len("x").unwrap_or(0), len("y").unwrap_or(0));
    let (mut w, mut h) = (len("width").unwrap_or(0), len("height").unwrap_or(0));
    let mut flip = (false, false);
    let mut absolute = None;
    if kind == "connector" {
        let (x1, y1, x2, y2) = (
            len("x1").unwrap_or(0),
            len("y1").unwrap_or(0),
            len("x2").unwrap_or(0),
            len("y2").unwrap_or(0),
        );
        x = x1.min(x2);
        y = y1.min(y2);
        w = (x2 - x1).abs();
        h = (y2 - y1).abs();
        flip = (x2 < x1, y2 < y1);
        absolute = Some((x, y));
    }
    let mut anchor = Anchor {
        kind: AnchorKind::TwoCell,
        edit_as: None,
        from: Marker {
            col,
            col_off: if child || absolute.is_some() {
                0
            } else {
                attr(e, NS_SVG, "x").and_then(offset_emu).unwrap_or(0)
            },
            row,
            row_off: if child || absolute.is_some() {
                0
            } else {
                attr(e, NS_SVG, "y").and_then(offset_emu).unwrap_or(0)
            },
        },
        to: None,
        ext: Some((w, h)),
        pos: None,
    };
    if let Some(p) = absolute.filter(|_| !child) {
        // 연결선은 시트 안 절대 위치
        anchor.kind = AnchorKind::Absolute;
        anchor.pos = Some(p);
    } else if !child
        && let Some(a) = attr(e, NS_TABLE, "end-cell-address")
        && let Some((c, r)) = end_cell(a)
    {
        anchor.to = Some(Marker {
            col: c,
            col_off: attr(e, NS_TABLE, "end-x").and_then(offset_emu).unwrap_or(0),
            row: r,
            row_off: attr(e, NS_TABLE, "end-y").and_then(offset_emu).unwrap_or(0),
        });
        anchor.ext = None;
    } else if !child {
        anchor.kind = AnchorKind::OneCell;
    }
    cx.next_id += 1;
    let mut s = Shape {
        anchor,
        id: cx.next_id,
        name: attr(e, NS_DRAW, "name").unwrap_or("").to_string(),
        xfrm: Some(Xfrm {
            x: if child { x } else { 0 },
            y: if child { y } else { 0 },
            cx: w,
            cy: h,
            flip_h: flip.0,
            flip_v: flip.1,
            ..Default::default()
        }),
        locks_with_sheet: Some(true),
        prints_with_sheet: Some(true),
        ..Default::default()
    };
    if kind == "g" {
        s.kind = ShapeKind::Group;
        let kids: Vec<Shape> = e
            .elements()
            .filter_map(|c| read_shape(c, (row, col), cx, parts, true))
            .collect();
        // 묶음 크기: 자식을 모두 덮는 상자 (한셀 .ods 의 draw:g 에는 크기가 없다)
        let (mut l, mut t, mut r, mut b) = (i64::MAX, i64::MAX, 0i64, 0i64);
        for k in &kids {
            if let Some(x) = &k.xfrm {
                l = l.min(x.x);
                t = t.min(x.y);
                r = r.max(x.x + x.cx);
                b = b.max(x.y + x.cy);
            }
        }
        if kids.is_empty() {
            (l, t) = (0, 0);
        }
        s.xfrm = Some(Xfrm {
            x: l,
            y: t,
            cx: (r - l).max(0),
            cy: (b - t).max(0),
            ..Default::default()
        });
        s.name.clear();
        // 묶음의 시작 칸 안 위치는 `svg:x`·`svg:y`
        if !child {
            s.anchor.from.col_off = attr(e, NS_SVG, "x").and_then(emu).unwrap_or(0);
            s.anchor.from.row_off = attr(e, NS_SVG, "y").and_then(emu).unwrap_or(0);
        }
        s.children = kids;
        return Some(s);
    }
    if kind == "connector" {
        s.kind = ShapeKind::Connector;
        s.geometry = Some(
            match attr(e, NS_DRAW, "type") {
                Some("standard") => "bentConnector3",
                Some("curve") => "curvedConnector3",
                _ => "line",
            }
            .into(),
        );
        apply_graphic(&mut s, e, cx);
        return Some(s);
    }
    if let Some(img) = e.child_ns(NS_DRAW, "image") {
        let href = attr(img, NS_XLINK, "href")?;
        let data = crate::container::read_entry(cx.bytes, href).ok()?;
        let ext = href.rsplit('.').next().unwrap_or("png");
        let name = format!("xl/media/ods{}.{ext}", parts.len() + 1);
        let mime = match ext {
            "jpeg" | "jpg" => "image/jpeg",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "emf" => "image/x-emf",
            "wmf" => "image/x-wmf",
            _ => "image/png",
        };
        parts.push(RawPart {
            name: name.clone(),
            content_type: Some(mime.into()),
            data,
        });
        s.kind = ShapeKind::Picture;
        s.target = Some(name);
        apply_graphic(&mut s, e, cx);
        return Some(s);
    }
    if let Some(obj) = e.child_ns(NS_DRAW, "object") {
        let href = attr(obj, NS_XLINK, "href")?
            .trim_end_matches('/')
            .trim_start_matches("./");
        let doc = crate::container::read_entry(cx.bytes, &format!("{href}/content.xml")).ok()?;
        let doc = String::from_utf8(doc).ok()?;
        s.kind = ShapeKind::Chart;
        s.chart = Some(read_chart(&doc, cx.sheet)?);
        s.name.clear();
        return Some(s);
    }
    // 글 상자·사용자 정의 도형
    s.kind = ShapeKind::Shape;
    s.geometry = Some("rect".into());
    if kind == "custom-shape"
        && let Some(g) = e.child_ns(NS_DRAW, "enhanced-geometry")
        && let Some(t) = attr(g, NS_DRAW, "type")
        && let Some(p) = GEOMS.iter().find(|x| x.1 == t)
    {
        s.geometry = Some(p.0.into());
    }
    let holder = e.child_ns(NS_DRAW, "text-box").unwrap_or(e);
    let paras: Vec<DrawingPara> = holder
        .elements()
        .filter(|p| p.is(NS_TEXT, "p"))
        .map(|p| read_para(p, cx))
        .collect();
    apply_graphic(&mut s, e, cx);
    let mut tb = s.text.take().unwrap_or_default();
    tb.paragraphs = paras;
    s.text = Some(tb);
    Some(s)
}

fn read_para(p: &Element, cx: &ReadCtx) -> DrawingPara {
    let align = p
        .attr_ns(NS_TEXT, "style-name")
        .and_then(|n| cx.styles.para.get(n))
        .and_then(|a| match a.as_str() {
            "center" => Some("ctr"),
            "end" | "right" => Some("r"),
            "justify" => Some("just"),
            "start" | "left" => Some("l"),
            _ => None,
        })
        .map(str::to_string);
    let mut runs = Vec::new();
    let mut pending = String::new();
    let flush = |pending: &mut String, runs: &mut Vec<DrawingRun>| {
        if !pending.is_empty() {
            runs.push(DrawingRun {
                text: std::mem::take(pending),
                ..Default::default()
            });
        }
    };
    for n in &p.children {
        match n {
            crate::xml::Node::Text(t) => pending.push_str(t),
            crate::xml::Node::Element(e) if e.is(NS_TEXT, "span") => {
                flush(&mut pending, &mut runs);
                let mut r = run_props(e.attr_ns(NS_TEXT, "style-name"), cx);
                r.text = e.text();
                runs.push(r);
            }
            crate::xml::Node::Element(e) if e.is(NS_TEXT, "line-break") => pending.push('\n'),
            crate::xml::Node::Element(e) => pending.push_str(&e.text()),
        }
    }
    flush(&mut pending, &mut runs);
    DrawingPara { align, runs }
}

fn run_props(style: Option<&str>, cx: &ReadCtx) -> DrawingRun {
    let mut r = DrawingRun::default();
    let Some(props) = style.and_then(|n| cx.styles.text.get(n)) else {
        return r;
    };
    let get = |k: &str| props.iter().find(|a| a.0 == k).map(|a| a.1.as_str());
    r.color = get("fo:color").and_then(hex_color);
    r.latin = get("style:font-name")
        .or(get("fo:font-family"))
        .map(str::to_string);
    r.ea = get("style:font-family-asian").map(str::to_string);
    r.size = get("fo:font-size")
        .and_then(crate::ods::length_pt)
        .map(|pt| (pt * 100.0).round() as u32);
    if get("fo:font-weight") == Some("bold") {
        r.bold = Some(true);
    }
    if get("fo:font-style") == Some("italic") {
        r.italic = Some(true);
    }
    if get("style:text-underline-style").is_some_and(|u| u != "none") {
        r.underline = Some(
            if get("style:text-underline-type") == Some("double") {
                "dbl"
            } else {
                "sng"
            }
            .into(),
        );
    }
    r
}

/// 그래픽 스타일 → 채우기·선·그림자·글 상자 속성
fn apply_graphic(s: &mut Shape, e: &Element, cx: &ReadCtx) {
    let Some(g) = attr(e, NS_DRAW, "style-name").and_then(|n| cx.styles.graphic.get(n)) else {
        return;
    };
    // 선
    // (한셀이 읽은 선: 두께만 있고 끝·겹선·이음은 비어 있다)
    let mut line = Outline {
        dash: Some("solid".into()),
        ..Default::default()
    };
    match g.get("draw:stroke") {
        Some("solid") | Some("dash") => {
            line.width = Some(g.get("svg:stroke-width").and_then(emu).unwrap_or(9525));
            let c = g.get("svg:stroke-color").and_then(hex_color);
            line.fill = Some(c.map_or(Paint::None, Paint::Solid));
            if g.get("draw:stroke") == Some("dash") {
                let n = g.get("draw:stroke-dash").unwrap_or("Dashed");
                line.dash = Some(
                    DASHES
                        .iter()
                        .find(|x| x.1 == n)
                        .map_or("dash", |x| x.0)
                        .to_string(),
                );
            }
        }
        _ => line.fill = Some(Paint::None),
    }
    if g.get("draw:marker-end").is_some() {
        line.tail = Some(crate::model::LineEnd {
            kind: Some("triangle".into()),
            w: Some("med".into()),
            len: Some("med".into()),
        });
    }
    s.line = Some(line);
    // 채우기 (연결선·그림은 채우지 않는다)
    if s.kind != ShapeKind::Connector && s.kind != ShapeKind::Picture {
        s.fill = Some(match g.get("draw:fill") {
            Some("solid") => match g.get("draw:fill-color").and_then(hex_color) {
                Some(mut c) => {
                    if let Some(a) = g.get("draw:opacity").and_then(pct) {
                        c.mods.push(("alpha".into(), a));
                    }
                    Paint::Solid(c)
                }
                None => Paint::None,
            },
            Some("gradient") | Some("hatch") | Some("bitmap") => Paint::Other("gradFill".into()),
            _ => Paint::None,
        });
    }
    // 그림자
    if g.get("draw:shadow") == Some("visible") {
        let dx = g.get("draw:shadow-offset-x").and_then(emu).unwrap_or(0) as f64;
        let dy = g.get("draw:shadow-offset-y").and_then(emu).unwrap_or(0) as f64;
        let mut color = g
            .get("draw:shadow-color")
            .and_then(hex_color)
            .unwrap_or(DrawColor {
                value: "srgb:000000".into(),
                mods: Vec::new(),
            });
        if let Some(a) = g.get("draw:shadow-opacity").and_then(pct) {
            color.mods.push(("alpha".into(), a));
        }
        let dist = dx.hypot(dy).round() as i64;
        let dir = (dy.atan2(dx).to_degrees() * 60000.0).round() as i64;
        s.shadow = Some(ShapeShadow {
            dist,
            dir: dir.rem_euclid(21_600_000),
            blur: 0,
            align: Some("tl".into()),
            rot_with_shape: false,
            color,
        });
    }
    // 글 상자 속성: 여백·세로 맞춤·줄 바꿈·세로 쓰기
    if s.kind == ShapeKind::Shape {
        let pad = |k: &str| g.get(k).and_then(emu);
        let mut tb = TextBody {
            wrap: Some(
                if g.get("fo:wrap-option") == Some("no-wrap") {
                    "none"
                } else {
                    "square"
                }
                .into(),
            ),
            ..Default::default()
        };
        tb.anchor = match g.get("draw:textarea-vertical-align") {
            Some("middle") => Some("ctr".into()),
            Some("bottom") => Some("b".into()),
            _ => None,
        };
        tb.insets = [
            pad("fo:padding-left"),
            pad("fo:padding-top"),
            pad("fo:padding-right"),
            pad("fo:padding-bottom"),
        ];
        if g.vertical {
            tb.vert = Some("vert".into());
        }
        s.text = Some(tb);
    }
}

fn series_color(style: Option<&str>, styles: &HashMap<String, Props>) -> Option<DrawColor> {
    let props = styles.get(style?)?;
    let get = |k: &str| props.iter().find(|a| a.0 == k).map(|a| a.1.as_str());
    get("draw:fill-color")
        .or_else(|| get("svg:stroke-color"))
        .and_then(hex_color)
}

/// 차트 하위 문서 → 차트 모델: 종류, 범례, 항목 범위, 계열 이름 (지역 표 머리)
fn read_chart(doc: &str, sheet: &str) -> Option<Chart> {
    let root = crate::xml::parse_raw(doc, "ods chart").ok()?;
    let sheet_ref = |a: &str| -> String {
        let q = if sheet.chars().all(|c| c.is_alphanumeric() || c == '_') {
            sheet.to_string()
        } else {
            format!("'{}'", sheet.replace('\'', "''"))
        };
        match a.strip_prefix("local-table.") {
            Some(cells) => format!("{q}!{}", cells.replace(":.", ":")),
            None => a.replace(":.", ":"),
        }
    };
    // 계열·차트 스타일 (이름 → 속성)
    let mut styles: HashMap<String, Props> = HashMap::new();
    let mut chart_props: HashMap<String, Props> = HashMap::new();
    if let Some(auto) = root.child_ns(NS_OFFICE, "automatic-styles") {
        for st in auto.elements().filter(|e| e.is(NS_STYLE, "style")) {
            let Some(name) = attr(st, NS_STYLE, "name") else {
                continue;
            };
            let flat = |tag: &str| {
                st.child_ns(NS_STYLE, tag)
                    .map(flat_attrs)
                    .unwrap_or_default()
            };
            styles.insert(name.to_string(), flat("graphic-properties"));
            chart_props.insert(name.to_string(), flat("chart-properties"));
        }
    }
    let mut ch = Chart::default();
    let mut found = false;
    let mut cat: Option<String> = None;
    type Ser = (Option<String>, Option<String>, Option<String>);
    let mut sers: Vec<Ser> = Vec::new();
    let mut cats_vals: Vec<String> = Vec::new();
    let mut values: Vec<Vec<Option<f64>>> = Vec::new();
    let mut headers: Vec<String> = Vec::new();
    let mut plot_style: Option<String> = None;
    let mut class = String::new();
    root.walk(&mut |e| {
        if e.is(NS_CHART, "chart") {
            found = true;
            class = attr(e, NS_CHART, "class")
                .unwrap_or("chart:bar")
                .to_string();
        } else if e.is(NS_CHART, "plot-area") {
            plot_style = attr(e, NS_CHART, "style-name").map(str::to_string);
        } else if e.is(NS_CHART, "legend") {
            ch.legend = Some(
                match attr(e, NS_CHART, "legend-position") {
                    Some("bottom") => "b",
                    Some("start") => "l",
                    Some("top") => "t",
                    _ => "r",
                }
                .into(),
            );
        } else if e.is(NS_CHART, "categories") {
            cat = attr(e, NS_TABLE, "cell-range-address").map(sheet_ref);
        } else if e.is(NS_CHART, "series") {
            sers.push((
                attr(e, NS_CHART, "label-cell-address").map(sheet_ref),
                attr(e, NS_CHART, "values-cell-range-address").map(sheet_ref),
                attr(e, NS_CHART, "style-name").map(str::to_string),
            ));
        } else if e.is(NS_TABLE, "table-header-rows") {
            let mut out = Vec::new();
            e.walk(&mut |c| {
                if c.is(NS_TABLE, "table-cell") {
                    out.push(c.text());
                }
            });
            headers = out;
        } else if e.is(NS_TABLE, "table-rows") {
            for r in e.elements().filter(|r| r.is(NS_TABLE, "table-row")) {
                let cells: Vec<&Element> = r
                    .elements()
                    .filter(|c| c.is(NS_TABLE, "table-cell"))
                    .collect();
                cats_vals.push(cells.first().map(|c| c.text()).unwrap_or_default());
                for (k, c) in cells.iter().skip(1).enumerate() {
                    if values.len() <= k {
                        values.resize(k + 1, Vec::new());
                    }
                    values[k].push(attr(c, NS_OFFICE, "value").and_then(|v| v.parse().ok()));
                }
            }
        }
    });
    if !found {
        return None;
    }
    let props = plot_style.as_ref().and_then(|n| chart_props.get(n));
    let pget = |k: &str| props.and_then(|p| p.iter().find(|a| a.0 == k).map(|a| a.1.as_str()));
    ch.kind = match class.as_str() {
        "chart:line" => "line",
        "chart:circle" => "pie",
        "chart:ring" => "doughnut",
        "chart:area" => "area",
        "chart:scatter" => "scatter",
        "chart:radar" => "radar",
        "chart:bubble" => "bubble",
        _ => "bar",
    }
    .to_string();
    ch.three_d = pget("chart:three-dimensional") == Some("true");
    let pie = ch.kind == "pie" || ch.kind == "doughnut";
    if ch.kind == "bar" {
        ch.bar_dir = Some(
            if pget("chart:vertical") == Some("true") {
                "bar"
            } else {
                "col"
            }
            .into(),
        );
    }
    if matches!(ch.kind.as_str(), "bar" | "line" | "area") {
        ch.grouping = Some(
            if pget("chart:percentage") == Some("true") {
                "percentStacked"
            } else if pget("chart:stacked") == Some("true")
                || (ch.kind == "bar" && pget("chart:overlap") == Some("100"))
            {
                "stacked"
            } else if ch.kind == "bar" {
                "clustered"
            } else {
                "standard"
            }
            .into(),
        );
    }
    ch.vary_colors = pie;
    ch.auto_title_deleted = Some(true);
    ch.plot_vis_only = Some(true);
    ch.disp_blanks_as = Some("gap".into());
    for (i, (label, vals, st)) in sers.iter().enumerate() {
        let name = headers.get(i + 1).cloned().unwrap_or_default();
        let mut s = ChartSeries {
            idx: Some(i as u32),
            order: Some(i as u32),
            ..Default::default()
        };
        // 이름은 지역 표 머리의 글 (한셀은 글자로만 읽지만, 이름 칸이 있으면 참조도 둔다)
        s.tx = Some(match label {
            Some(l) => ChartTx::Ref(ChartData {
                kind: "strRef".into(),
                formula: Some(l.clone()),
                cache: Some(ChartCache {
                    format_code: None,
                    pt_count: Some(1),
                    points: vec![ChartPoint {
                        idx: 0,
                        value: name,
                        ..Default::default()
                    }],
                }),
            }),
            None => ChartTx::Value(name),
        });
        if let Some(c) = &cat
            && ch.kind != "scatter"
        {
            s.cat = Some(ChartData {
                kind: "strRef".into(),
                formula: Some(c.clone()),
                cache: Some(ChartCache {
                    format_code: None,
                    pt_count: Some(cats_vals.len() as u32),
                    points: cats_vals
                        .iter()
                        .enumerate()
                        .map(|(k, v)| ChartPoint {
                            idx: k as u32,
                            value: v.clone(),
                            ..Default::default()
                        })
                        .collect(),
                }),
            });
        }
        if let Some(v) = vals {
            let col = values.get(i).cloned().unwrap_or_default();
            s.val = Some(ChartData {
                kind: "numRef".into(),
                formula: Some(v.clone()),
                cache: Some(ChartCache {
                    format_code: Some("General".into()),
                    pt_count: Some(col.len() as u32),
                    points: col
                        .iter()
                        .enumerate()
                        .filter_map(|(k, x)| {
                            x.map(|x| ChartPoint {
                                idx: k as u32,
                                value: format!("{x}"),
                                ..Default::default()
                            })
                        })
                        .collect(),
                }),
            });
        }
        if let Some(c) = series_color(st.as_deref(), &styles) {
            let fill = ChartFill::Solid(ChartColor {
                value: c.value.clone(),
                ..Default::default()
            });
            s.sp_pr = Some(ChartShapeProps {
                fill: Some(fill.clone()),
                line: Some(ChartLine {
                    width: Some(6479),
                    fill: Some(fill),
                    dash: Some("solid".into()),
                    ..Default::default()
                }),
            });
        }
        s.invert_if_negative = (ch.kind == "bar").then_some(false);
        s.refresh_summary();
        ch.series.push(s);
    }
    // 축 (원형은 없다)
    let mut group = ChartGroupProps::default();
    if !pie {
        let xk = if ch.kind == "scatter" {
            "valAx"
        } else {
            "catAx"
        };
        let axis = |kind: &str, id: u32, cross: u32, pos: &str| ChartAxis {
            kind: kind.into(),
            id,
            delete: Some(false),
            ax_pos: Some(pos.into()),
            cross_ax: Some(cross),
            crosses: Some("autoZero".into()),
            ..Default::default()
        };
        ch.axes = vec![axis(xk, 0, 1, "b"), axis("valAx", 1, 0, "l")];
        group.ax_ids = vec![0, 1];
    }
    if ch.kind == "bar" {
        group.gap_width = Some(150);
        group.overlap = Some(if ch.grouping.as_deref() == Some("clustered") {
            0
        } else {
            100
        });
    }
    ch.group = group;
    ch.refresh_summary();
    Some(ch)
}

/// 읽은 도형의 시트 안 절대 위치 (`xfrm` 의 x·y): 앞 열·행의 크기 + 시작 칸 안 오프셋. 묶음 안 자식은 이미 시트 좌표다
pub(crate) fn place(sheet: &mut crate::model::Sheet, mdw: u32) {
    let rects: Vec<(i64, i64)> = sheet
        .shapes
        .iter()
        .map(|s| {
            let r = crate::writer::anchor_rect(sheet, mdw, &s.anchor);
            (r.0, r.1)
        })
        .collect();
    for (s, (x, y)) in sheet.shapes.iter_mut().zip(rects) {
        if s.anchor.kind == AnchorKind::Absolute {
            continue;
        }
        if let Some(xf) = s.xfrm.as_mut() {
            let (dx, dy) = (x - xf.x, y - xf.y);
            if s.kind == ShapeKind::Group {
                // 묶음: 자식 상자의 왼쪽 위가 묶음 위치 — 자식을 같이 옮긴다
                xf.x = x;
                xf.y = y;
                shift(&mut s.children, dx, dy);
            } else {
                xf.x = x;
                xf.y = y;
            }
        }
    }
}

fn shift(kids: &mut [Shape], dx: i64, dy: i64) {
    for k in kids {
        if let Some(x) = k.xfrm.as_mut() {
            x.x += dx;
            x.y += dy;
        }
        shift(&mut k.children, dx, dy);
    }
}
