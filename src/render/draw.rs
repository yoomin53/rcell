// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그리기 개체 (도형·글상자·그림·연결선·그룹·차트)를 표 위에 절대 위치로 그린다.
//!
//! 위치는 앵커의 칸 + 오프셋(EMU)을 화면 배치([`super::layout`])로 옮긴 것. 모양 경로·조정값·스타일
//! 참조·화살표 끝은 원본 앵커 XML 에서 다시 읽는다 ([`super::shape`]).

use std::fmt::Write as _;

use super::cell::font_family;
use super::layout::Layout;
use super::shape;
use crate::model::{DrawColor, Marker, Paint, Shape, ShapeKind, TextBody, Workbook};
use crate::theme::Theme;
use crate::writer::esc;

const EMU: f64 = crate::units::EMU_PER_PX as f64;

/// 앵커 위치 (머리글 포함 표 좌표, px). 숨긴 열·행 안의 오프셋은 0 폭으로 접힌다
fn anchor_px(lay: &Layout, x0: u32, y0: u32, m: &Marker) -> (f64, f64) {
    // 배치 범위 밖의 칸은 마지막 칸 끝 (그리기 범위는 개체가 놓인 곳까지 잡으므로 드물다)
    let x = if m.col < lay.ncols {
        lay.col_x(m.col)
    } else {
        lay.col_x.last().copied().unwrap_or(0) + lay.col_w.last().copied().unwrap_or(0)
    } as f64;
    let y = if m.row < lay.nrows {
        lay.row_y(m.row)
    } else {
        lay.row_y.last().copied().unwrap_or(0) + lay.row_h.last().copied().unwrap_or(0)
    } as f64;
    let col_off = (m.col_off as f64 / EMU).min(lay.col_w(m.col) as f64).max(0.0);
    let row_off = (m.row_off as f64 / EMU).min(lay.row_h(m.row) as f64).max(0.0);
    (x0 as f64 + x + col_off, y0 as f64 + y + row_off)
}

/// 그리기 개체를 표 위에 절대 위치로 그린다
pub(super) fn shapes_html(
    wb: &Workbook,
    sheet: &crate::model::Sheet,
    lay: &Layout,
    x0: u32,
    y0: u32,
    theme: &Theme,
) -> String {
    let mut s = String::new();
    for sh in sheet.shapes.iter().filter(|x| !x.hidden) {
        let a = &sh.anchor;
        let (x1, y1) = match (a.kind, a.pos) {
            (crate::model::AnchorKind::Absolute, Some((x, y))) => {
                (x0 as f64 + x as f64 / EMU, y0 as f64 + y as f64 / EMU)
            }
            _ => anchor_px(lay, x0, y0, &a.from),
        };
        let (x2, y2) = match (&a.to, a.ext) {
            (Some(t), _) if a.kind == crate::model::AnchorKind::TwoCell => {
                anchor_px(lay, x0, y0, t)
            }
            (_, Some((cx, cy))) => (x1 + cx as f64 / EMU, y1 + cy as f64 / EMU),
            (Some(t), None) => anchor_px(lay, x0, y0, t),
            _ => (x1, y1),
        };
        let (w, h) = ((x2 - x1).max(0.0), (y2 - y1).max(0.0));
        let obj = sh.raw.as_deref().and_then(shape::object_element);
        s.push_str(&object_html(wb, sh, obj.as_ref(), (x1, y1, w, h), theme));
    }
    s
}

/// 개체 하나 (그룹이면 자식들을 그룹 좌표계로 옮겨 그린다)
fn object_html(
    wb: &Workbook,
    sh: &Shape,
    obj: Option<&crate::xml::Element>,
    (x, y, w, h): (f64, f64, f64, f64),
    theme: &Theme,
) -> String {
    let mut s = String::new();
    let mut tf = Vec::new();
    if let Some(xf) = sh.xfrm.as_ref() {
        if xf.rot != 0 {
            tf.push(format!("rotate({}deg)", xf.rot as f64 / 60000.0));
        }
        if sh.kind != ShapeKind::Connector {
            if xf.flip_h {
                tf.push("scaleX(-1)".into());
            }
            if xf.flip_v {
                tf.push("scaleY(-1)".into());
            }
        }
    }
    let tf = if tf.is_empty() {
        String::new()
    } else {
        format!("transform:{};", tf.join(" "))
    };
    if sh.kind == ShapeKind::Group {
        // 자식 좌표: grpSpPr 의 chOff/chExt → 그룹 상자
        let xf = obj
            .and_then(shape::sp_pr)
            .and_then(|p| p.child_ns(shape::A, "xfrm"));
        let num = |e: Option<&crate::xml::Element>, n: &str| {
            e.and_then(|e| e.attr(n)).and_then(|v| v.parse::<f64>().ok())
        };
        let ch_off = xf.and_then(|x| x.child_ns(shape::A, "chOff"));
        let ch_ext = xf.and_then(|x| x.child_ns(shape::A, "chExt"));
        let (cx0, cy0) = (
            num(ch_off, "x").unwrap_or(0.0),
            num(ch_off, "y").unwrap_or(0.0),
        );
        let (cw, ch) = (
            num(ch_ext, "cx").filter(|v| *v > 0.0),
            num(ch_ext, "cy").filter(|v| *v > 0.0),
        );
        let raw_kids: Vec<&crate::xml::Element> = obj
            .map(|o| {
                o.elements()
                    .filter(|c| {
                        c.ns.as_deref() == Some(shape::XDR)
                            && matches!(
                                c.name.as_str(),
                                "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                            )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let _ = write!(
            s,
            "<div class=\"rc-shape rc-group\" title=\"{}\" style=\"left:{x:.1}px;top:{y:.1}px;width:{w:.1}px;height:{h:.1}px;{tf}\">",
            esc(&sh.name)
        );
        for (i, kid) in sh.children.iter().enumerate().filter(|(_, k)| !k.hidden) {
            let Some(kx) = kid.xfrm.as_ref() else {
                continue;
            };
            let sx = cw.map_or(1.0 / EMU, |cw| w / cw);
            let sy = ch.map_or(1.0 / EMU, |ch| h / ch);
            let r = (
                (kx.x as f64 - cx0) * sx,
                (kx.y as f64 - cy0) * sy,
                kx.cx as f64 * sx,
                kx.cy as f64 * sy,
            );
            s.push_str(&object_html(wb, kid, raw_kids.get(i).copied(), r, theme));
        }
        s.push_str("</div>");
        return s;
    }
    let _ = write!(
        s,
        "<div class=\"rc-shape\" title=\"{}\" style=\"left:{x:.1}px;top:{y:.1}px;width:{w:.1}px;height:{h:.1}px;{tf}",
        esc(&sh.name)
    );
    s.push_str(&shape_body(wb, sh, obj, w, h, theme));
    s.push_str("</div>");
    s
}

/// 채우기 색 (모델 → 없으면 스타일 참조)
fn shape_fill(
    sh: &Shape,
    obj: Option<&crate::xml::Element>,
    theme: &Theme,
    defs: &mut String,
    id: &str,
) -> Option<String> {
    match &sh.fill {
        Some(Paint::Solid(c)) => draw_color(c, theme),
        Some(Paint::None) => None,
        Some(Paint::Other(kind)) if kind == "gradFill" => {
            let (ang, stops) = obj.and_then(shape::grad_fill)?;
            let th = ang.to_radians();
            let (c, s) = (th.cos(), th.sin());
            let d = c.abs() + s.abs();
            let _ = write!(
                defs,
                "<linearGradient id=\"{id}\" x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\">",
                0.5 - c * d / 2.0,
                0.5 - s * d / 2.0,
                0.5 + c * d / 2.0,
                0.5 + s * d / 2.0
            );
            for (p, e) in &stops {
                let col = draw_color(&crate::parser::drawing::color(e), theme)
                    .unwrap_or_else(|| "#FFFFFF".into());
                let _ = write!(defs, "<stop offset=\"{p}\" stop-color=\"{col}\"/>");
            }
            defs.push_str("</linearGradient>");
            Some(format!("url(#{id})"))
        }
        Some(Paint::Other(_)) => Some("#D9D9D9".into()),
        None => obj
            .and_then(|o| shape::style_ref(o, "fillRef"))
            .map(crate::parser::drawing::color)
            .and_then(|c| draw_color(&c, theme)),
    }
}

/// 선 (색, 굵기 px, 점선)
fn shape_line(
    sh: &Shape,
    obj: Option<&crate::xml::Element>,
    theme: &Theme,
) -> Option<(String, f64, Option<String>)> {
    let style_color = || {
        obj.and_then(|o| shape::style_ref(o, "lnRef"))
            .map(crate::parser::drawing::color)
            .and_then(|c| draw_color(&c, theme))
    };
    let (color, width, dash) = match &sh.line {
        Some(l) => {
            let c = match &l.fill {
                Some(Paint::Solid(c)) => draw_color(c, theme),
                Some(Paint::None) => return None,
                Some(Paint::Other(_)) => Some("#000000".into()),
                None => style_color(),
            }?;
            (c, l.width, l.dash.clone())
        }
        None => (style_color()?, None, None),
    };
    let px = (width.unwrap_or(9525) as f64 / EMU).max(1.0);
    Some((color, px, dash.as_deref().and_then(|d| dash_array(d, px))))
}

/// DrawingML 미리 정의된 점선 (`a:prstDash`) → SVG stroke-dasharray (선 굵기 배수)
pub(super) fn dash_array(d: &str, px: f64) -> Option<String> {
    let pat: &[f64] = match d {
        "sysDot" => &[1.0, 1.0],
        "dot" => &[1.0, 3.0],
        "sysDash" => &[3.0, 1.0],
        "dash" => &[4.0, 3.0],
        "dashDot" | "sysDashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDash" => &[8.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" | "sysDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        _ => return None,
    };
    Some(
        pat.iter()
            .map(|v| format!("{:.1}", v * px))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// 개체 하나의 style 나머지와 내용 (`style="…` 다음부터 `</div>` 앞까지)
fn shape_body(
    wb: &Workbook,
    sh: &Shape,
    obj: Option<&crate::xml::Element>,
    w: f64,
    h: f64,
    theme: &Theme,
) -> String {
    let mut css = String::new();
    let mut inner = String::new();
    match sh.kind {
        ShapeKind::Chart => {
            match &sh.chart {
                Some(ch) => inner.push_str(&crate::chart_svg::chart_svg(ch, w, h, theme, &|c| {
                    draw_color(c, theme)
                })),
                None => inner.push_str("<div class=\"rc-chart\">차트</div>"),
            }
        }
        ShapeKind::Picture => {
            let img = sh
                .target
                .as_ref()
                .and_then(|t| wb.raw_parts.iter().find(|p| &p.name == t));
            if let Some(p) = img {
                let ct = p.content_type.as_deref().unwrap_or("image/png");
                let _ = write!(
                    inner,
                    "<img alt=\"\" style=\"width:100%;height:100%\" src=\"data:{ct};base64,{}\">",
                    base64(&p.data)
                );
            }
        }
        ShapeKind::Connector => {
            let (stroke, px, dash) =
                shape_line(sh, obj, theme).unwrap_or_else(|| ("#000000".into(), 1.0, None));
            let flip_h = sh.xfrm.as_ref().is_some_and(|x| x.flip_h);
            let flip_v = sh.xfrm.as_ref().is_some_and(|x| x.flip_v);
            let (ax, bx) = if flip_h { (w, 0.0) } else { (0.0, w) };
            let (ay, by) = if flip_v { (h, 0.0) } else { (0.0, h) };
            let mut defs = String::new();
            let mut marks = String::new();
            for (which, attr) in [("headEnd", "marker-start"), ("tailEnd", "marker-end")] {
                if obj.and_then(|o| shape::line_end(o, which)).is_some() {
                    let id = format!("m{}{}", sh.id, which);
                    let _ = write!(
                        defs,
                        "<marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"{}\" refY=\"5\" markerWidth=\"4\" markerHeight=\"4\" orient=\"auto-start-reverse\"><path d=\"M0,0L10,5L0,10Z\" fill=\"{stroke}\"/></marker>",
                        if which == "tailEnd" { 9 } else { 1 }
                    );
                    let _ = write!(marks, " {attr}=\"url(#{id})\"");
                }
            }
            let da = dash
                .map(|d| format!(" stroke-dasharray=\"{d}\""))
                .unwrap_or_default();
            let _ = write!(
                inner,
                "<svg width=\"{w:.1}\" height=\"{h:.1}\" style=\"overflow:visible\"><defs>{defs}</defs><line x1=\"{ax:.1}\" y1=\"{ay:.1}\" x2=\"{bx:.1}\" y2=\"{by:.1}\" stroke=\"{stroke}\" stroke-width=\"{px:.2}\"{da}{marks}/></svg>"
            );
            css.push_str("overflow:visible;");
        }
        _ => {
            let mut defs = String::new();
            let fill = shape_fill(sh, obj, theme, &mut defs, &format!("g{}", sh.id));
            let line = shape_line(sh, obj, theme);
            // 모양 경로: 원본 XML 이 있으면 그것으로, 없으면 모델의 모양 이름
            let paths = obj.and_then(|o| shape::paths(o, w, h)).or_else(|| {
                let prst = sh.geometry.as_deref()?;
                shape::preset_path(prst, w, h, &std::collections::HashMap::new())
                    .map(|d| vec![(d, true, true)])
            });
            if let Some(paths) = paths.filter(|_| fill.is_some() || line.is_some()) {
                let mut svg = format!(
                    "<svg class=\"rc-geom\" width=\"{w:.1}\" height=\"{h:.1}\" viewBox=\"0 0 {w:.1} {h:.1}\"><defs>{defs}</defs>"
                );
                for (d, f, st) in paths {
                    let fill_attr = if f {
                        fill.clone().unwrap_or_else(|| "none".into())
                    } else {
                        "none".into()
                    };
                    let stroke_attr = match (&line, st) {
                        (Some((c, px, dash)), true) => format!(
                            " stroke=\"{c}\" stroke-width=\"{px:.2}\"{}",
                            dash.as_ref()
                                .map(|d| format!(" stroke-dasharray=\"{d}\""))
                                .unwrap_or_default()
                        ),
                        _ => String::new(),
                    };
                    let _ = write!(svg, "<path d=\"{d}\" fill=\"{fill_attr}\"{stroke_attr}/>");
                }
                svg.push_str("</svg>");
                inner.push_str(&svg);
                css.push_str("overflow:visible;");
            }
            if let Some(t) = &sh.text {
                inner.push_str(&text_html(t, theme));
            }
        }
    }
    format!("{css}\">{inner}")
}

fn text_html(t: &TextBody, theme: &Theme) -> String {
    let pad = |v: Option<i64>, d: i64| v.unwrap_or(d) as f64 / EMU;
    let justify = match t.anchor.as_deref() {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    };
    let vert = match t.vert.as_deref() {
        Some("vert" | "eaVert" | "wordArtVertRtl") => "writing-mode:vertical-rl;",
        Some("vert270") => "writing-mode:vertical-rl;transform:rotate(180deg);",
        _ => "",
    };
    let mut s = format!(
        "<div class=\"rc-text\" style=\"justify-content:{justify};padding:{:.1}px {:.1}px {:.1}px {:.1}px;{}{vert}\">",
        pad(t.insets[1], 45720),
        pad(t.insets[2], 91440),
        pad(t.insets[3], 45720),
        pad(t.insets[0], 91440),
        if t.wrap.as_deref() == Some("none") {
            "white-space:pre"
        } else {
            "white-space:pre-wrap"
        }
    );
    for p in &t.paragraphs {
        let align = match p.align.as_deref() {
            Some("ctr") => "center",
            Some("r") => "right",
            Some("just") | Some("dist") => "justify",
            _ => "left",
        };
        let _ = write!(s, "<p style=\"text-align:{align}\">");
        if p.runs.is_empty() {
            s.push_str("&nbsp;");
        }
        for r in &p.runs {
            if r.text == "\n" {
                s.push_str("<br>");
                continue;
            }
            let mut st = String::new();
            if let Some(z) = r.size {
                let _ = write!(st, "font-size:{}pt;", z as f64 / 100.0);
            }
            if r.bold == Some(true) {
                st.push_str("font-weight:bold;");
            }
            if r.italic == Some(true) {
                st.push_str("font-style:italic;");
            }
            if r.underline.as_deref().is_some_and(|u| u != "none") {
                st.push_str("text-decoration:underline;");
            }
            if let Some(c) = r.color.as_ref().and_then(|c| draw_color(c, theme)) {
                let _ = write!(st, "color:{c};");
            }
            if let Some(f) = r.ea.as_ref().or(r.latin.as_ref()) {
                let _ = write!(st, "font-family:{};", font_family(f));
            }
            let _ = write!(s, "<span style=\"{st}\">{}</span>", esc(&r.text));
        }
        s.push_str("</p>");
    }
    s.push_str("</div>");
    s
}

/// DrawingML 색 → CSS (#RRGGBB). 테마 색과 변형(lumMod/lumOff/shade/tint/alpha 제외)을 적용한다.
pub(crate) fn draw_color(c: &DrawColor, theme: &Theme) -> Option<String> {
    let (r, g, b) = draw_rgb(c, theme)?;
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

/// DrawingML 색 → RGB
pub(crate) fn draw_rgb(c: &DrawColor, theme: &Theme) -> Option<(u8, u8, u8)> {
    let (kind, val) = c.value.split_once(':')?;
    let hex = |s: &str| -> Option<(f64, f64, f64)> {
        let v = u32::from_str_radix(s.get(s.len().saturating_sub(6)..)?, 16).ok()?;
        Some((
            ((v >> 16) & 0xFF) as f64,
            ((v >> 8) & 0xFF) as f64,
            (v & 0xFF) as f64,
        ))
    };
    let mut rgb = match kind {
        "srgb" => hex(val)?,
        "scheme" => {
            // Theme::colors 순서: 0 lt1, 1 dk1, 2 lt2, 3 dk2, 4–9 accent1–6, 10 hlink, 11 folHlink
            let i = match val {
                "dk1" | "tx1" => 1,
                "lt1" | "bg1" => 0,
                "dk2" | "tx2" => 3,
                "lt2" | "bg2" => 2,
                "accent1" => 4,
                "accent2" => 5,
                "accent3" => 6,
                "accent4" => 7,
                "accent5" => 8,
                "accent6" => 9,
                "hlink" => 10,
                "folHlink" => 11,
                // 차트·도형 스타일의 자리 표시 색: 강조 1 로 본다
                "phClr" => 4,
                _ => return None,
            };
            hex(theme.colors.get(i)?)?
        }
        "sys" => {
            if val.eq_ignore_ascii_case("window") {
                (255.0, 255.0, 255.0)
            } else {
                (0.0, 0.0, 0.0)
            }
        }
        "prst" => match val {
            "white" => (255.0, 255.0, 255.0),
            "red" => (255.0, 0.0, 0.0),
            "green" => (0.0, 128.0, 0.0),
            "blue" => (0.0, 0.0, 255.0),
            "yellow" => (255.0, 255.0, 0.0),
            "gray" | "grey" => (128.0, 128.0, 128.0),
            "silver" => (192.0, 192.0, 192.0),
            "orange" => (255.0, 165.0, 0.0),
            "purple" => (128.0, 0.0, 128.0),
            "cyan" | "aqua" => (0.0, 255.0, 255.0),
            "magenta" | "fuchsia" => (255.0, 0.0, 255.0),
            "navy" => (0.0, 0.0, 128.0),
            "maroon" => (128.0, 0.0, 0.0),
            "olive" => (128.0, 128.0, 0.0),
            "teal" => (0.0, 128.0, 128.0),
            "lime" => (0.0, 255.0, 0.0),
            _ => (0.0, 0.0, 0.0),
        },
        _ => return None,
    };
    for (m, v) in &c.mods {
        let f = *v as f64 / 100_000.0;
        match m.as_str() {
            "shade" => rgb = (rgb.0 * f, rgb.1 * f, rgb.2 * f),
            "tint" => {
                rgb = (
                    255.0 - (255.0 - rgb.0) * f,
                    255.0 - (255.0 - rgb.1) * f,
                    255.0 - (255.0 - rgb.2) * f,
                )
            }
            "lumMod" | "lumOff" => {
                let (h, sat, l) = rgb_to_hsl(rgb);
                let l = if m == "lumMod" {
                    l * f
                } else {
                    (l + f).min(1.0)
                };
                rgb = hsl_to_rgb(h, sat, l);
            }
            "satMod" => {
                let (h, sat, l) = rgb_to_hsl(rgb);
                rgb = hsl_to_rgb(h, (sat * f).min(1.0), l);
            }
            _ => {}
        }
    }
    let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Some((c(rgb.0), c(rgb.1), c(rgb.2)))
}

fn rgb_to_hsl((r, g, b): (f64, f64, f64)) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l * 255.0, l * 255.0, l * 255.0);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        v * 255.0
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// 표준 Base64 (그림을 data: URI 로 넣을 때)
pub(super) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}
