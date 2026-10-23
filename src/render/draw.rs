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
use crate::xml::Element;

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
    let col_off = (m.col_off as f64 / EMU)
        .min(lay.col_w(m.col) as f64)
        .max(0.0);
    let row_off = (m.row_off as f64 / EMU)
        .min(lay.row_h(m.row) as f64)
        .max(0.0);
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
        // 한셀 화면의 개체 자리 (앵커 대조 실험): 도형의 1px 선은 칸 경계보다 1px 왼쪽·위 픽셀
        // (선 가운데를 −0.5px 에), 그림은 1px 왼쪽·위, 차트 틀은 왼쪽 2px·위 1px 바깥에서 오른쪽·아래
        // 경계의 1px 안쪽까지 (틀 크기 +1)
        let r = match sh.kind {
            crate::model::ShapeKind::Chart => (
                x1.round() - 2.0,
                y1.round() - 1.0,
                w.round() + 1.0,
                h.round() + 1.0,
            ),
            crate::model::ShapeKind::Picture => (x1 - 1.0, y1 - 1.0, w, h),
            _ => (x1 - 0.5, y1 - 0.5, w, h),
        };
        let obj = sh.raw.as_deref().and_then(shape::object_element);
        s.push_str(&object_html(wb, sh, obj.as_ref(), r, theme));
    }
    s
}

/// 개체 하나 (그룹이면 자식들을 그룹 좌표계로 옮겨 그린다)
fn object_html(
    wb: &Workbook,
    sh: &Shape,
    obj: Option<&Element>,
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
            e.and_then(|e| e.attr(n))
                .and_then(|v| v.parse::<f64>().ok())
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

/// DrawingML 색 요소 → (CSS 색, 불투명도)
fn color_el(e: &Element, theme: &Theme) -> Option<(String, f64)> {
    let c = crate::parser::drawing::color(e);
    let alpha = c
        .mods
        .iter()
        .find(|(m, _)| m == "alpha")
        .map_or(1.0, |(_, v)| (*v as f64 / 100_000.0).clamp(0.0, 1.0));
    Some((draw_color(&c, theme)?, alpha))
}

/// 채우기 요소(solidFill/gradFill/pattFill/blipFill/noFill)가 든 부모에서 SVG 채우기 값을 만든다.
/// 그라데이션·무늬·그림은 defs 에 정의를 넣고 url(#id) 를 돌려준다. 채우기 요소가 없으면 None
fn fill_from(
    parent: &Element,
    theme: &Theme,
    defs: &mut String,
    id: &str,
    wb: &Workbook,
    sh: &Shape,
) -> Option<Option<(String, f64)>> {
    for c in parent
        .elements()
        .filter(|c| c.ns.as_deref() == Some(shape::A))
    {
        match c.name.as_str() {
            "noFill" => return Some(None),
            "solidFill" => return Some(c.elements().next().and_then(|e| color_el(e, theme))),
            "gradFill" => return Some(gradient_def(c, theme, defs, id).map(|u| (u, 1.0))),
            "pattFill" => return Some(pattern_def(c, theme, defs, id).map(|u| (u, 1.0))),
            "blipFill" => return Some(image_def(c, wb, sh, defs, id).map(|u| (u, 1.0))),
            "grpFill" => return Some(None),
            _ => {}
        }
    }
    None
}

/// a:gradFill → linearGradient/radialGradient
fn gradient_def(g: &Element, theme: &Theme, defs: &mut String, id: &str) -> Option<String> {
    let stops: Vec<(f64, String, f64)> = g
        .child_ns(shape::A, "gsLst")?
        .elements()
        .filter(|e| e.is(shape::A, "gs"))
        .filter_map(|gs| {
            let pos = gs.attr_f64("pos").unwrap_or(0.0) / 100_000.0;
            let (c, a) = color_el(gs.elements().next()?, theme)?;
            Some((pos, c, a))
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    let stop_xml: String = stops
        .iter()
        .map(|(p, c, a)| {
            if *a < 1.0 {
                format!("<stop offset=\"{p:.4}\" stop-color=\"{c}\" stop-opacity=\"{a:.3}\"/>")
            } else {
                format!("<stop offset=\"{p:.4}\" stop-color=\"{c}\"/>")
            }
        })
        .collect();
    if let Some(path) = g.child_ns(shape::A, "path") {
        // 원·사각·모양 경로 그라데이션: 채우는 사각형(fillToRect)의 가운데에서 바깥으로
        let r = path.child_ns(shape::A, "fillToRect");
        let v = |n: &str| r.and_then(|r| r.attr_f64(n)).unwrap_or(50_000.0) / 100_000.0;
        let (cx, cy) = ((v("l") + 1.0 - v("r")) / 2.0, (v("t") + 1.0 - v("b")) / 2.0);
        let _ = write!(
            defs,
            "<radialGradient id=\"{id}\" cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"0.71\">{stop_xml}</radialGradient>"
        );
    } else {
        let ang = g
            .child_ns(shape::A, "lin")
            .and_then(|l| l.attr_f64("ang"))
            .unwrap_or(5_400_000.0)
            / 60000.0;
        let th = ang.to_radians();
        let (c, s) = (th.cos(), th.sin());
        let d = c.abs() + s.abs();
        let _ = write!(
            defs,
            "<linearGradient id=\"{id}\" x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\">{stop_xml}</linearGradient>",
            0.5 - c * d / 2.0,
            0.5 - s * d / 2.0,
            0.5 + c * d / 2.0,
            0.5 + s * d / 2.0
        );
    }
    Some(format!("url(#{id})"))
}

/// a:pattFill → 8×8 무늬 (전경 픽셀 판정은 무늬 이름의 모양을 따라 직접 그린 근사)
fn pattern_def(p: &Element, theme: &Theme, defs: &mut String, id: &str) -> Option<String> {
    let col = |n: &str, d: &str| {
        p.child_ns(shape::A, n)
            .and_then(|c| c.elements().next())
            .and_then(|e| color_el(e, theme))
            .map_or_else(|| d.to_string(), |c| c.0)
    };
    let (fg, bg) = (col("fgClr", "#000000"), col("bgClr", "#FFFFFF"));
    let prst = p.attr("prst").unwrap_or("pct5");
    let on = |x: u32, y: u32| -> bool {
        let pct = |n: u32| (x * 5 + y * 3 + (x ^ y) * 7) % 100 < n;
        match prst {
            "pct5" => pct(5),
            "pct10" => pct(10),
            "pct20" => pct(20),
            "pct25" => pct(25),
            "pct30" => pct(30),
            "pct40" => pct(40),
            "pct50" => (x + y).is_multiple_of(2),
            "pct60" => pct(60),
            "pct70" => pct(70),
            "pct75" => pct(75),
            "pct80" => pct(80),
            "pct90" => pct(90),
            "horz" | "ltHorz" | "narHorz" => y.is_multiple_of(4),
            "dkHorz" => y % 4 < 2,
            "vert" | "ltVert" | "narVert" => x.is_multiple_of(4),
            "dkVert" => x % 4 < 2,
            "dnDiag" | "ltDnDiag" | "wdDnDiag" => (x + 8 - y).is_multiple_of(4),
            "dkDnDiag" => (x + 8 - y) % 4 < 2,
            "upDiag" | "ltUpDiag" | "wdUpDiag" => (x + y).is_multiple_of(4),
            "dkUpDiag" => (x + y) % 4 < 2,
            "smGrid" | "cross" => x.is_multiple_of(4) || y.is_multiple_of(4),
            "lgGrid" => x == 0 || y == 0,
            "dotGrid" => {
                x.is_multiple_of(4) && y.is_multiple_of(2)
                    || y.is_multiple_of(4) && x.is_multiple_of(2)
            }
            "smCheck" => (x / 2 + y / 2).is_multiple_of(2),
            "lgCheck" => (x / 4 + y / 4).is_multiple_of(2),
            "diagCross" | "trellis" => (x + y).is_multiple_of(4) || (x + 8 - y).is_multiple_of(4),
            "dashHorz" => y.is_multiple_of(4) && x % 8 < 4,
            "dashVert" => x.is_multiple_of(4) && y % 8 < 4,
            "dashDnDiag" => (x + 8 - y).is_multiple_of(4) && y % 8 < 4,
            "dashUpDiag" => (x + y).is_multiple_of(4) && y % 8 < 4,
            "zigZag" | "wave" => y == [0, 1, 2, 3, 3, 2, 1, 0][x as usize % 8],
            "solidDmnd" | "openDmnd" | "dotDmnd" => {
                (x as i32 - 4).abs() + (y as i32 - 4).abs() == 3
            }
            _ => pct(50),
        }
    };
    let _ = write!(
        defs,
        "<pattern id=\"{id}\" width=\"8\" height=\"8\" patternUnits=\"userSpaceOnUse\"><rect width=\"8\" height=\"8\" fill=\"{bg}\"/><path fill=\"{fg}\" d=\""
    );
    for y in 0..8 {
        for x in 0..8 {
            if on(x, y) {
                let _ = write!(defs, "M{x} {y}h1v1h-1z");
            }
        }
    }
    defs.push_str("\"/></pattern>");
    Some(format!("url(#{id})"))
}

/// a:blipFill → 도형을 채우는 그림 (늘이기)
fn image_def(
    b: &Element,
    wb: &Workbook,
    sh: &Shape,
    defs: &mut String,
    id: &str,
) -> Option<String> {
    let blip = b.child_ns(shape::A, "blip")?;
    let rid = blip
        .attrs
        .iter()
        .find(|a| a.name == "embed")
        .map(|a| a.value.as_str())?;
    let target = sh
        .raw_rels
        .iter()
        .find(|r| r.0 == rid)
        .map(|r| r.2.as_str())?;
    let part = wb.raw_parts.iter().find(|p| p.name == target)?;
    let ct = part.content_type.as_deref().unwrap_or("image/png");
    let _ = write!(
        defs,
        "<pattern id=\"{id}\" width=\"1\" height=\"1\" patternContentUnits=\"objectBoundingBox\"><image width=\"1\" height=\"1\" preserveAspectRatio=\"none\" href=\"data:{ct};base64,{}\"/></pattern>",
        base64(&part.data)
    );
    Some(format!("url(#{id})"))
}

/// 채우기 (원본 spPr → 모델 → 스타일 참조): (SVG 채우기 값, 불투명도)
fn shape_fill(
    wb: &Workbook,
    sh: &Shape,
    obj: Option<&Element>,
    theme: &Theme,
    defs: &mut String,
    id: &str,
) -> Option<(String, f64)> {
    if let Some(p) = obj.and_then(shape::sp_pr)
        && let Some(f) = fill_from(p, theme, defs, id, wb, sh)
    {
        return f;
    }
    match &sh.fill {
        Some(Paint::Solid(c)) => draw_color(c, theme).map(|c| (c, 1.0)),
        Some(Paint::None) => None,
        Some(Paint::Other(_)) => Some(("#D9D9D9".into(), 1.0)),
        None => obj
            .and_then(|o| shape::style_ref(o, "fillRef"))
            .and_then(|e| color_el(e, theme)),
    }
}

/// 선
struct Line {
    color: String,
    alpha: f64,
    px: f64,
    dash: Option<String>,
    cap: &'static str,
    join: &'static str,
    head: Option<(String, String, String)>,
    tail: Option<(String, String, String)>,
    /// 겹선 (cmpd: dbl, thickThin, thinThick, tri). 없으면 한 줄
    compound: Option<String>,
}

/// 선 (원본 a:ln → 모델 → 스타일 참조). 선이 없으면 None
fn shape_line(sh: &Shape, obj: Option<&Element>, theme: &Theme) -> Option<Line> {
    let ln = obj
        .and_then(shape::sp_pr)
        .and_then(|p| p.child_ns(shape::A, "ln"));
    let style_color = || {
        obj.and_then(|o| shape::style_ref(o, "lnRef"))
            .and_then(|e| color_el(e, theme))
    };
    let (color, alpha) = match ln {
        Some(ln) => {
            let mut found = None;
            for c in ln.elements().filter(|c| c.ns.as_deref() == Some(shape::A)) {
                match c.name.as_str() {
                    "noFill" => return None,
                    "solidFill" => found = c.elements().next().and_then(|e| color_el(e, theme)),
                    "gradFill" => {
                        // 선 그라데이션은 첫 정지점 색으로
                        found = c
                            .child_ns(shape::A, "gsLst")
                            .and_then(|l| l.elements().next())
                            .and_then(|gs| gs.elements().next())
                            .and_then(|e| color_el(e, theme))
                    }
                    _ => {}
                }
            }
            found.or_else(style_color)?
        }
        None => match &sh.line {
            Some(l) => match &l.fill {
                Some(Paint::Solid(c)) => (draw_color(c, theme)?, 1.0),
                Some(Paint::None) => return None,
                Some(Paint::Other(_)) => ("#000000".into(), 1.0),
                None => style_color()?,
            },
            None => style_color()?,
        },
    };
    let width = ln
        .and_then(|l| l.attr("w"))
        .and_then(|v| v.parse::<i64>().ok())
        .or(sh.line.as_ref().and_then(|l| l.width));
    // 굵기 0 은 가장 가는 선 (1px)
    let px = (width.unwrap_or(9525) as f64 / EMU).max(1.0);
    let dash = ln
        .and_then(|l| l.child_ns(shape::A, "prstDash"))
        .and_then(|d| d.attr("val"))
        .map(str::to_string)
        .or(sh.line.as_ref().and_then(|l| l.dash.clone()))
        .and_then(|d| dash_array(&d, px));
    let model = sh.line.as_ref();
    let cap = match ln
        .and_then(|l| l.attr("cap"))
        .or(model.and_then(|l| l.cap.as_deref()))
    {
        Some("rnd") => "round",
        Some("sq") => "square",
        _ => "butt",
    };
    // 이음: 지정이 없으면 둥글게 (한셀 화면 — 도형 실험의 6pt 겹선 사각형 모서리가 둥글다)
    let model_join = model.and_then(|l| l.join.as_deref());
    let has = |n: &str| ln.and_then(|l| l.child_ns(shape::A, n)).is_some();
    let join = if has("bevel") || (ln.is_none() && model_join == Some("bevel")) {
        "bevel"
    } else if has("miter") || (ln.is_none() && model_join == Some("miter")) {
        "miter"
    } else {
        "round"
    };
    let end = |which: &str| {
        let from_xml = ln.and_then(|l| l.child_ns(shape::A, which)).map(|e| {
            (
                e.attr("type").map(str::to_string),
                e.attr("w").map(str::to_string),
                e.attr("len").map(str::to_string),
            )
        });
        let from_model = model
            .and_then(|l| {
                if which == "headEnd" {
                    l.head.as_ref()
                } else {
                    l.tail.as_ref()
                }
            })
            .map(|e| (e.kind.clone(), e.w.clone(), e.len.clone()));
        let (t, w, len) = from_xml.or(from_model)?;
        let t = t.filter(|t| t != "none")?;
        Some((
            t,
            w.unwrap_or_else(|| "med".into()),
            len.unwrap_or_else(|| "med".into()),
        ))
    };
    let compound = ln
        .and_then(|l| l.attr("cmpd"))
        .map(str::to_string)
        .or_else(|| model.and_then(|l| l.compound.clone()))
        .filter(|c| c != "sng");
    Some(Line {
        color,
        alpha,
        px,
        dash,
        cap,
        join,
        head: end("headEnd"),
        tail: end("tailEnd"),
        compound,
    })
}

/// DrawingML 미리 정의된 점선 (a:prstDash) → SVG stroke-dasharray (선 굵기 배수)
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

/// 선 끝 모양 → SVG marker. 크기는 선 굵기 배수 (sm 2, med 3, lg 5) — 한셀 화면은 2pt 보다 가는 선도 2pt 로 본다
/// (도형 실험: 1pt 선의 lg 삼각형이 13px, 3pt 선의 med 가 12px). 끝(삼각형 꼭짓점)이 선 끝에, 원·마름모는 가운데가
fn marker_def(defs: &mut String, id: &str, end: &(String, String, String), color: &str, px: f64) {
    let base = px.max(8.0 / 3.0);
    let k = |v: &str| match v {
        "sm" => 2.0,
        "lg" => 5.0,
        _ => 3.0,
    } * base;
    let (mw, ml) = (k(&end.1), k(&end.2));
    let shape = match end.0.as_str() {
        "oval" => "<ellipse cx=\"5\" cy=\"5\" rx=\"5\" ry=\"5\"/>".to_string(),
        "diamond" => "<path d=\"M0,5L5,0L10,5L5,10Z\"/>".to_string(),
        "stealth" => "<path d=\"M0,0L10,5L0,10L3,5Z\"/>".to_string(),
        "arrow" => "<path d=\"M0,0L10,5L0,10\" fill=\"none\" stroke-width=\"SW\" stroke=\"COL\"/>"
            .to_string(),
        _ => "<path d=\"M0,0L10,5L0,10Z\"/>".to_string(),
    }
    .replace("COL", color)
    .replace("SW", &format!("{:.2}", (px * 10.0 / ml.max(1.0)).max(1.0)));
    let _ = write!(
        defs,
        "<marker id=\"{id}\" viewBox=\"0 0 10 10\" refX=\"{}\" refY=\"5\" markerWidth=\"{ml:.1}\" markerHeight=\"{mw:.1}\" markerUnits=\"userSpaceOnUse\" orient=\"auto-start-reverse\" preserveAspectRatio=\"none\" fill=\"{color}\" overflow=\"visible\">{shape}</marker>",
        if matches!(end.0.as_str(), "oval" | "diamond") {
            5
        } else {
            10
        }
    );
}

/// 겹선 마스크: 선 굵기 W 를 바깥 → 안 띠로 나눈다. dbl 은 1:1:1, tri 는 1:1:2:1:1 (가운데가 굵다), thickThin 은
/// 굵은 바깥 3 : 틈 1 : 가는 안 1, thinThick 은 그 반대 (도형 실험의 6pt 사각형: 바깥 4.9px·틈 1.5·안 1.6). 안팎이
/// 있는 닫힌 경로만 안쪽 자르기로 가르고, 열린 경로는 dbl 로 그린다
fn compound_mask(id: &str, line: &Line, paths: &[&str], w: f64, h: f64) -> String {
    let wpx = line.px;
    let closed = !paths.is_empty() && paths.iter().all(|d| d.trim_end().ends_with(['Z', 'z']));
    let kind = match line.compound.as_deref() {
        Some(k @ ("thickThin" | "thinThick")) if closed => k,
        Some("tri") => "tri",
        _ => "dbl",
    };
    // (너비 비율, 흰/검, 안쪽만) — 뒤에 칠한 것이 앞을 덮는다
    let bands: &[(f64, &str, bool)] = match kind {
        "tri" => &[
            (1.0, "#fff", false),
            (2.0 / 3.0, "#000", false),
            (1.0 / 3.0, "#fff", false),
        ],
        // 바깥 [−0.1W, 0.5W] 와 안 [−0.5W, −0.3W] (가운데에서 바깥쪽이 +)
        "thickThin" => &[
            (1.0, "#fff", false),
            (0.6, "#000", true),
            (0.2, "#fff", true),
        ],
        // 바깥 [0.3W, 0.5W] 와 안 [−0.5W, 0.1W]
        "thinThick" => &[
            (1.0, "#fff", false),
            (0.6, "#000", false),
            (0.2, "#fff", false),
            (1.0, "#fff", true),
        ],
        _ => &[(1.0, "#fff", false), (1.0 / 3.0, "#000", false)],
    };
    let mut m = String::new();
    if bands.iter().any(|b| b.2) {
        let _ = write!(m, "<clipPath id=\"{id}i\">");
        for d in paths {
            let _ = write!(m, "<path d=\"{d}\"/>");
        }
        m.push_str("</clipPath>");
    }
    let _ = write!(
        m,
        "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\">",
        -wpx * 2.0 - 2.0,
        -wpx * 2.0 - 2.0,
        w + wpx * 4.0 + 4.0,
        h + wpx * 4.0 + 4.0
    );
    for (k, c, inside) in bands {
        if *inside {
            let _ = write!(m, "<g clip-path=\"url(#{id}i)\">");
        }
        for d in paths {
            let _ = write!(
                m,
                "<path d=\"{d}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"{:.2}\" stroke-linejoin=\"{}\"/>",
                wpx * k,
                line.join
            );
        }
        if *inside {
            m.push_str("</g>");
        }
    }
    m.push_str("</mask>");
    m
}

/// 바깥 그림자 (a:outerShdw) → SVG 필터. 거리·방향으로 옮기고 흐림 반지름의 반을 표준 편차로 (도형 실험의 세 그림자)
fn shadow_filter(sh: &Shape, theme: &Theme, id: &str) -> Option<String> {
    let sd = sh.shadow.as_ref()?;
    let color = draw_color(&sd.color, theme).unwrap_or_else(|| "#000000".into());
    let alpha = sd
        .color
        .mods
        .iter()
        .find(|(k, _)| k == "alpha")
        .map_or(1.0, |(_, v)| *v as f64 / 100_000.0);
    let dist = sd.dist as f64 / EMU;
    let dir = (sd.dir as f64 / 60_000.0).to_radians();
    let (dx, dy) = (dist * dir.cos(), dist * dir.sin());
    let blur = sd.blur as f64 / EMU / 2.0;
    Some(format!(
        "<filter id=\"{id}\" x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\" color-interpolation-filters=\"sRGB\"><feDropShadow dx=\"{dx:.2}\" dy=\"{dy:.2}\" stdDeviation=\"{blur:.2}\" flood-color=\"{color}\" flood-opacity=\"{alpha:.3}\"/></filter>"
    ))
}

/// 선 SVG 속성 (stroke …, 선 끝 marker)
fn stroke_attrs(line: &Line, defs: &mut String, id: &str) -> String {
    let mut a = format!(
        " stroke=\"{}\" stroke-width=\"{:.2}\" stroke-linecap=\"{}\" stroke-linejoin=\"{}\"",
        line.color, line.px, line.cap, line.join
    );
    if line.alpha < 1.0 {
        let _ = write!(a, " stroke-opacity=\"{:.3}\"", line.alpha);
    }
    if let Some(d) = &line.dash {
        let _ = write!(a, " stroke-dasharray=\"{d}\"");
    }
    if let Some(e) = &line.head {
        marker_def(defs, &format!("{id}h"), e, &line.color, line.px);
        let _ = write!(a, " marker-start=\"url(#{id}h)\"");
    }
    if let Some(e) = &line.tail {
        marker_def(defs, &format!("{id}t"), e, &line.color, line.px);
        let _ = write!(a, " marker-end=\"url(#{id}t)\"");
    }
    a
}

/// 개체 하나의 style 나머지와 내용 (style="… 다음부터 </div> 앞까지)
fn shape_body(
    wb: &Workbook,
    sh: &Shape,
    obj: Option<&Element>,
    w: f64,
    h: f64,
    theme: &Theme,
) -> String {
    let mut css = String::new();
    let mut inner = String::new();
    // 문서 안에서 겹치지 않는 SVG id
    let id = format!("s{}x{}", sh.id, (w * 7.0 + h * 13.0) as u64);
    match sh.kind {
        ShapeKind::Chart => match &sh.chart {
            Some(ch) => inner.push_str(&crate::chart_svg::chart_svg(ch, w, h, theme, &|c| {
                draw_color(c, theme)
            })),
            None => inner.push_str("<div class=\"rc-chart\">차트</div>"),
        },
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
            // 그림 테두리
            if let Some(line) = shape_line(sh, obj, theme).filter(|_| {
                obj.and_then(shape::sp_pr)
                    .and_then(|p| p.child_ns(shape::A, "ln"))
                    .is_some()
            }) {
                let _ = write!(
                    css,
                    "outline:{:.1}px solid {};outline-offset:-{:.1}px;",
                    line.px,
                    line.color,
                    line.px / 2.0
                );
            }
        }
        ShapeKind::Connector => {
            let line = shape_line(sh, obj, theme).unwrap_or(Line {
                color: "#000000".into(),
                alpha: 1.0,
                px: 1.0,
                dash: None,
                cap: "butt",
                join: "miter",
                head: None,
                tail: None,
                compound: None,
            });
            let mut defs = String::new();
            let attrs = stroke_attrs(&line, &mut defs, &id);
            // 연결선 모양: 직선 또는 꺾인·굽은 연결선 (뒤집기는 경로를 뒤집어 그린다). 크기가 0 인 a:xfrm(앵커가
            // 크기를 정한다)의 뒤집기는 한셀이 쓰지 않는다 (도형 실험의 flipV 연결선: 왼쪽 위 → 오른쪽 아래 그대로)
            let sized = sh.xfrm.as_ref().is_some_and(|x| x.cx != 0 || x.cy != 0);
            let flip_h = sized && sh.xfrm.as_ref().is_some_and(|x| x.flip_h);
            let flip_v = sized && sh.xfrm.as_ref().is_some_and(|x| x.flip_v);
            let prst = obj
                .and_then(shape::geometry)
                .and_then(|g| g.attr("prst"))
                .or(sh.geometry.as_deref())
                .unwrap_or("straightConnector1");
            let adj = obj
                .and_then(shape::geometry)
                .map(shape::adjusts)
                .unwrap_or_default();
            let d = shape::preset_path(prst, w, h, &adj)
                .unwrap_or_else(|| format!("M0,0L{w:.1},{h:.1}"));
            let tf = match (flip_h, flip_v) {
                (false, false) => String::new(),
                _ => format!(
                    " transform=\"translate({:.1},{:.1}) scale({},{})\"",
                    if flip_h { w } else { 0.0 },
                    if flip_v { h } else { 0.0 },
                    if flip_h { -1 } else { 1 },
                    if flip_v { -1 } else { 1 }
                ),
            };
            // 가로·세로 선은 크기가 0 이다 — SVG 상자는 1px 로 두고 넘쳐 그린다
            let _ = write!(
                inner,
                "<svg class=\"rc-geom\" width=\"{:.1}\" height=\"{:.1}\"><defs>{defs}</defs><path d=\"{d}\" fill=\"none\"{attrs}{tf}/></svg>",
                w.max(1.0),
                h.max(1.0)
            );
            css.push_str("overflow:visible;");
        }
        _ => {
            let mut defs = String::new();
            let fill = shape_fill(wb, sh, obj, theme, &mut defs, &format!("{id}f"));
            let line = shape_line(sh, obj, theme);
            // 모양 경로: 원본 XML 이 있으면 그것으로, 없으면 모델의 모양 이름
            let paths = obj.and_then(|o| shape::paths(o, w, h)).or_else(|| {
                let prst = sh.geometry.as_deref()?;
                shape::preset_paths(prst, w, h, &std::collections::HashMap::new())
            });
            if let Some(paths) = paths.filter(|_| fill.is_some() || line.is_some()) {
                let stroke = line
                    .as_ref()
                    .map(|l| stroke_attrs(l, &mut defs, &id))
                    .unwrap_or_default();
                // 겹선: 채우기와 따로, 마스크를 씌운 선만 한 번 더
                let cmask = line.as_ref().filter(|l| l.compound.is_some()).map(|l| {
                    let strokes: Vec<&str> = paths
                        .iter()
                        .filter(|p| p.stroke)
                        .map(|p| p.d.as_str())
                        .collect();
                    defs.push_str(&compound_mask(&format!("{id}m"), l, &strokes, w, h));
                    format!("{id}m")
                });
                let mut body = String::new();
                for p in &paths {
                    let fill_attr = match (&fill, p.fill) {
                        (Some((f, a)), pf) if pf != shape::PathFill::None => {
                            let mut v = format!(" fill=\"{f}\"");
                            if *a < 1.0 {
                                let _ = write!(v, " fill-opacity=\"{a:.3}\"");
                            }
                            v
                        }
                        _ => " fill=\"none\"".to_string(),
                    };
                    let st = if p.stroke && cmask.is_none() {
                        stroke.as_str()
                    } else {
                        ""
                    };
                    let _ = write!(
                        body,
                        "<path d=\"{}\" fill-rule=\"evenodd\"{fill_attr}{st}/>",
                        p.d
                    );
                    // 어둡게·밝게: 같은 경로에 반투명 검정·흰색을 덧칠한다
                    if fill.is_some()
                        && let Some((c, o)) = p.fill.overlay()
                    {
                        let _ = write!(
                            body,
                            "<path d=\"{}\" fill-rule=\"evenodd\" fill=\"{c}\" fill-opacity=\"{o}\"/>",
                            p.d
                        );
                    }
                    if p.stroke
                        && let Some(m) = &cmask
                    {
                        let _ = write!(
                            body,
                            "<path d=\"{}\" fill=\"none\"{stroke} mask=\"url(#{m})\"/>",
                            p.d
                        );
                    }
                }
                let body = match shadow_filter(sh, theme, &format!("{id}s")) {
                    Some(f) => {
                        defs.push_str(&f);
                        format!("<g filter=\"url(#{id}s)\">{body}</g>")
                    }
                    None => body,
                };
                let _ = write!(
                    inner,
                    "<svg class=\"rc-geom\" width=\"{w:.1}\" height=\"{h:.1}\" viewBox=\"0 0 {w:.1} {h:.1}\"><defs>{defs}</defs>{body}</svg>"
                );
                css.push_str("overflow:visible;");
            }
            // 글: 원본 txBody 가 있으면 그것으로 (줄 간격·문단 간격·테마 글꼴), 없으면 모델
            let tx = obj.and_then(|o| o.child_ns(shape::XDR, "txBody"));
            let font_color = obj
                .and_then(|o| o.child_ns(shape::XDR, "style"))
                .and_then(|st| st.child_ns(shape::A, "fontRef"))
                .and_then(|r| r.elements().next())
                .and_then(|e| color_el(e, theme))
                .map(|c| c.0);
            match tx {
                Some(tx) => inner.push_str(&txbody_html(tx, theme, font_color.as_deref())),
                None => {
                    if let Some(t) = &sh.text {
                        inner.push_str(&text_html(t, theme));
                    }
                }
            }
        }
    }
    format!("{css}\">{inner}")
}

/// 테마 글꼴 참조(+mn-lt, +mn-ea, +mj-lt, +mj-ea …)를 글꼴 이름으로. 동아시아(`-ea`)는 테마의 `a:ea` 글꼴, 그 이름이
/// 비어 있으면 한셀 기본 글꼴 함초롬돋움 (서식 파일 소개 글상자: 10pt 한 줄이 맑은 고딕 403px 이 아니라 함초롬돋움의
/// 387~391px — 글꼴 폭 실험)
fn theme_font<'a>(name: &'a str, theme: &'a Theme) -> Option<&'a str> {
    let ea = |f: &'a Option<String>| {
        f.as_deref()
            .filter(|n| !n.is_empty())
            .or(Some("함초롬돋움"))
    };
    match name {
        "+mn-ea" => ea(&theme.minor_ea),
        "+mj-ea" => ea(&theme.major_ea),
        "+mn-lt" | "+mn-cs" => theme.minor_font.as_deref().or(Some("맑은 고딕")),
        "+mj-lt" | "+mj-cs" => theme.major_font.as_deref().or(Some("맑은 고딕")),
        "" => None,
        n => Some(n),
    }
}

/// 글 조각 서식 (a:rPr / a:defRPr / a:endParaRPr) → CSS. 크기(pt) 도 돌려준다
fn run_css(r: Option<&Element>, theme: &Theme, base_color: Option<&str>) -> (String, Option<f64>) {
    let mut st = String::new();
    let Some(r) = r else {
        if let Some(c) = base_color {
            let _ = write!(st, "color:{c};");
        }
        return (st, None);
    };
    let size = r.attr_f64("sz").map(|v| v / 100.0);
    if let Some(z) = size {
        let _ = write!(st, "font-size:{z}pt;");
    }
    match r.attr_bool("b") {
        Some(true) => st.push_str("font-weight:bold;"),
        Some(false) => st.push_str("font-weight:normal;"),
        None => {}
    }
    if r.attr_bool("i") == Some(true) {
        st.push_str("font-style:italic;");
    }
    let mut deco = Vec::new();
    if r.attr("u").is_some_and(|u| u != "none") {
        deco.push("underline");
    }
    if r.attr("strike").is_some_and(|u| u != "noStrike") {
        deco.push("line-through");
    }
    if !deco.is_empty() {
        let _ = write!(st, "text-decoration:{};", deco.join(" "));
    }
    if let Some(b) = r.attr_f64("baseline").filter(|b| *b != 0.0) {
        st.push_str(if b > 0.0 {
            "vertical-align:super;"
        } else {
            "vertical-align:sub;"
        });
    }
    if let Some(sp) = r.attr_f64("spc").filter(|v| *v != 0.0) {
        let _ = write!(st, "letter-spacing:{:.2}pt;", sp / 100.0);
    }
    let color = r
        .child_ns(shape::A, "solidFill")
        .and_then(|f| f.elements().next())
        .and_then(|e| color_el(e, theme))
        .map(|c| c.0)
        .or_else(|| base_color.map(str::to_string));
    if let Some(c) = color {
        let _ = write!(st, "color:{c};");
    }
    let face = r
        .child_ns(shape::A, "ea")
        .or_else(|| r.child_ns(shape::A, "latin"))
        .and_then(|f| f.attr("typeface"))
        .and_then(|f| theme_font(f, theme));
    if let Some(f) = face {
        let _ = write!(st, "font-family:{};", font_family(f));
    }
    (st, size)
}

/// 원본 txBody → HTML (문단·조각·줄 간격·문단 간격·안쪽 여백·세로 맞춤·세로 글)
fn txbody_html(tx: &Element, theme: &Theme, base_color: Option<&str>) -> String {
    let body = tx.child_ns(shape::A, "bodyPr");
    let ins = |n: &str, d: f64| body.and_then(|b| b.attr_f64(n)).unwrap_or(d) / EMU;
    let justify = match body.and_then(|b| b.attr("anchor")) {
        Some("ctr") => "center",
        Some("b") => "flex-end",
        _ => "flex-start",
    };
    let vert = match body.and_then(|b| b.attr("vert")) {
        Some("vert" | "eaVert" | "wordArtVertRtl") => "writing-mode:vertical-rl;",
        Some("vert270") => "writing-mode:vertical-rl;transform:rotate(180deg);",
        _ => "",
    };
    let wrap = if body.and_then(|b| b.attr("wrap")) == Some("none") {
        "white-space:pre"
    } else {
        "white-space:pre-wrap"
    };
    // 글자 줄이기 (normAutofit fontScale)
    let scale = body
        .and_then(|b| b.child_ns(shape::A, "normAutofit"))
        .and_then(|a| a.attr_f64("fontScale"))
        .map_or(1.0, |v| v / 100_000.0);
    let mut s = format!(
        "<div class=\"rc-text\" style=\"justify-content:{justify};padding:{:.1}px {:.1}px {:.1}px {:.1}px;{wrap};{vert}\">",
        ins("tIns", 45720.0),
        ins("rIns", 91440.0),
        ins("bIns", 45720.0),
        ins("lIns", 91440.0)
    );
    let lst = tx.child_ns(shape::A, "lstStyle");
    let lvl1 = lst
        .and_then(|l| l.child_ns(shape::A, "lvl1pPr"))
        .and_then(|p| p.child_ns(shape::A, "defRPr"));
    let mut first_para = true;
    for p in tx.elements().filter(|e| e.is(shape::A, "p")) {
        let ppr = p.child_ns(shape::A, "pPr");
        let align = match ppr.and_then(|x| x.attr("algn")) {
            Some("ctr") => "center",
            Some("r") => "right",
            Some("just") | Some("dist") => "justify",
            _ => "left",
        };
        // 문단 글꼴 크기: 조각 중 가장 큰 것 (없으면 문단 끝 서식, 기본 11pt)
        let mut size: Option<f64> = None;
        for r in p
            .elements()
            .filter(|e| e.is(shape::A, "r") || e.is(shape::A, "endParaRPr"))
        {
            let rp = if r.is(shape::A, "r") {
                r.child_ns(shape::A, "rPr")
            } else {
                Some(r)
            };
            if let Some(z) = rp.and_then(|x| x.attr_f64("sz")) {
                size = Some(size.map_or(z / 100.0, |s| s.max(z / 100.0)));
            }
        }
        let size = size.or_else(|| lvl1.and_then(|d| d.attr_f64("sz")).map(|v| v / 100.0));
        // 글도 크기도 없는 빈 문단은 한셀 화면에 자리가 없다 (서식 파일 소개 글상자: `<a:endParaRPr>` 만 있는 문단 앞뒤
        // 줄이 16px 간격 그대로 — 크기가 있는 빈 문단은 한 줄)
        let has_text = p
            .elements()
            .any(|e| e.is(shape::A, "r") || e.is(shape::A, "fld") || e.is(shape::A, "br"));
        if size.is_none() && !has_text {
            continue;
        }
        let size = size.unwrap_or(11.0) * scale;
        // 줄 간격: spcPct 100% = 글꼴 크기의 1.2 배, spcPts 는 그대로. px 로 정해 둔다 (대신 쓰는 글꼴의 위아래 폭이
        // 줄을 늘리지 않게 — 한셀은 10pt 한 줄이 16px)
        let lnspc = ppr.and_then(|x| x.child_ns(shape::A, "lnSpc"));
        let line_px = match lnspc.and_then(|l| l.child_ns(shape::A, "spcPct")) {
            Some(pct) => {
                1.2 * pct.attr_f64("val").unwrap_or(100_000.0) / 100_000.0 * size * 4.0 / 3.0
            }
            None => match lnspc.and_then(|l| l.child_ns(shape::A, "spcPts")) {
                Some(pts) => pts.attr_f64("val").unwrap_or(0.0) / 100.0 * 4.0 / 3.0,
                None => 1.2 * size * 4.0 / 3.0,
            },
        };
        let line = format!("{line_px:.2}px");
        let space = |n: &str| -> f64 {
            let Some(sp) = ppr.and_then(|x| x.child_ns(shape::A, n)) else {
                return 0.0;
            };
            if let Some(p) = sp.child_ns(shape::A, "spcPts") {
                p.attr_f64("val").unwrap_or(0.0) / 100.0 * 4.0 / 3.0
            } else if let Some(p) = sp.child_ns(shape::A, "spcPct") {
                p.attr_f64("val").unwrap_or(0.0) / 100_000.0 * size * 1.2 * 4.0 / 3.0
            } else {
                0.0
            }
        };
        let indent = ppr
            .and_then(|x| x.attr_f64("marL"))
            .map_or(0.0, |v| v / EMU);
        let first = ppr
            .and_then(|x| x.attr_f64("indent"))
            .map_or(0.0, |v| v / EMU);
        // 문단의 글꼴은 첫 조각의 글꼴 (줄 상자의 기준이 조각과 같은 글꼴이어야 줄 높이가 늘지 않는다)
        let face = p
            .elements()
            .find(|e| e.is(shape::A, "r"))
            .and_then(|r| r.child_ns(shape::A, "rPr"))
            .or(lvl1)
            .and_then(|r| {
                r.child_ns(shape::A, "ea")
                    .or_else(|| r.child_ns(shape::A, "latin"))
            })
            .and_then(|f| f.attr("typeface"))
            .and_then(|f| theme_font(f, theme))
            .map(|f| format!("font-family:{};", font_family(f)))
            .unwrap_or_default();
        let _ = write!(
            s,
            "<p style=\"text-align:{align};font-size:{size:.2}pt;{face}line-height:{line};margin:{:.1}px 0 {:.1}px {indent:.1}px;text-indent:{first:.1}px\">",
            // 첫 줄의 글자는 한셀이 0.12 em 위에 그린다 (글꼴 폭 실험의 5 글꼴 × 5 크기, 서식 파일 소개 글상자)
            space("spcBef")
                - if first_para {
                    0.12 * size * 4.0 / 3.0
                } else {
                    0.0
                },
            space("spcAft")
        );
        first_para = false;
        // 글머리 기호
        if let Some(bu) = ppr.and_then(|x| x.child_ns(shape::A, "buChar")) {
            let _ = write!(s, "{} ", esc(bu.attr("char").unwrap_or("•")));
        }
        let mut any = false;
        for r in p.elements() {
            if r.is(shape::A, "br") {
                s.push_str("<br>");
                continue;
            }
            if !(r.is(shape::A, "r") || r.is(shape::A, "fld")) {
                continue;
            }
            let t = r
                .child_ns(shape::A, "t")
                .map(|t| t.text())
                .unwrap_or_default();
            let (mut st, _) = run_css(r.child_ns(shape::A, "rPr").or(lvl1), theme, base_color);
            if scale != 1.0
                && let Some(z) = r.child_ns(shape::A, "rPr").and_then(|x| x.attr_f64("sz"))
            {
                let _ = write!(st, "font-size:{:.2}pt;", z / 100.0 * scale);
            }
            let _ = write!(s, "<span style=\"{st}\">{}</span>", esc(&t));
            any = true;
        }
        if !any {
            s.push_str("&nbsp;");
        }
        s.push_str("</p>");
    }
    s.push_str("</div>");
    s
}

/// 모델의 글 (원본 XML 이 없을 때: 레거시·바이너리)
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
        let size = p
            .runs
            .iter()
            .filter_map(|r| r.size)
            .max()
            .map_or(11.0, |z| z as f64 / 100.0);
        let _ = write!(
            s,
            "<p style=\"text-align:{align};font-size:{size}pt;line-height:1.2\">"
        );
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
            if let Some(f) =
                r.ea.as_deref()
                    .or(r.latin.as_deref())
                    .and_then(|f| theme_font(f, theme))
            {
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
