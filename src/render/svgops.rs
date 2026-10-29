// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! rcell 이 그린 SVG(차트·도형 모양)를 인쇄 그리기 목록([`Op`])으로 옮긴다. 렌더러가 쓰는 요소만 읽는다:
//! `svg` `g` `rect` `path` `line` `polyline` `polygon` `circle` `ellipse` `text`, 변환(`translate` `scale` `rotate`
//! `matrix`), 채우기·선·점선·불투명도, `clipPath` 의 사각형. 그라데이션·무늬는 첫 색으로, 불투명도는 흰 바탕과 섞는다.

use std::collections::HashMap;

use super::page::{FontKey, Op, Seg};
use crate::xml::{Element, Node};

/// 2차원 변환 [a b c d e f] (x' = a·x + c·y + e)
type Mat = [f64; 6];

const ID: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn mul(m: Mat, n: Mat) -> Mat {
    // 먼저 n, 다음 m
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

fn apply(m: Mat, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// 숫자 목록 (쉼표·공백·부호로 나뉜)
fn numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' {
            let start = i;
            i += 1;
            let mut dot = c == b'.';
            while i < b.len() {
                let d = b[i];
                if d.is_ascii_digit() {
                    i += 1;
                } else if d == b'.' && !dot {
                    dot = true;
                    i += 1;
                } else if (d == b'e' || d == b'E') && i + 1 < b.len() {
                    i += 1;
                    if b[i] == b'-' || b[i] == b'+' {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            if let Ok(v) = s[start..i].parse() {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn transform(s: &str) -> Mat {
    let mut m = ID;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let Some(close) = rest[open..].find(')') else {
            break;
        };
        let args = numbers(&rest[open + 1..open + close]);
        let t = match (name, args.as_slice()) {
            ("translate", [x]) => [1.0, 0.0, 0.0, 1.0, *x, 0.0],
            ("translate", [x, y, ..]) => [1.0, 0.0, 0.0, 1.0, *x, *y],
            ("scale", [s]) => [*s, 0.0, 0.0, *s, 0.0, 0.0],
            ("scale", [sx, sy, ..]) => [*sx, 0.0, 0.0, *sy, 0.0, 0.0],
            ("rotate", [a, rest @ ..]) => {
                let (s, c) = a.to_radians().sin_cos();
                let r = [c, s, -s, c, 0.0, 0.0];
                if let [cx, cy] = rest {
                    mul(
                        mul([1.0, 0.0, 0.0, 1.0, *cx, *cy], r),
                        [1.0, 0.0, 0.0, 1.0, -cx, -cy],
                    )
                } else {
                    r
                }
            }
            ("matrix", [a, b, c, d, e, f]) => [*a, *b, *c, *d, *e, *f],
            _ => ID,
        };
        m = mul(m, t);
        rest = &rest[open + close + 1..];
    }
    m
}

/// SVG 경로 → 조각 (절대 좌표, 변환 전)
pub fn path_segs(d: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    // 명령과 숫자 토막
    let mut toks: Vec<(char, Vec<f64>)> = Vec::new();
    let mut cur: Option<(char, String)> = None;
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' && ch != 'E' {
            if let Some((c, a)) = cur.take() {
                toks.push((c, numbers(&a)));
            }
            cur = Some((ch, String::new()));
        } else if let Some((_, a)) = cur.as_mut() {
            a.push(ch);
        }
    }
    if let Some((c, a)) = cur {
        toks.push((c, numbers(&a)));
    }
    let (mut x, mut y) = (0.0, 0.0);
    let (mut sx, mut sy) = (0.0, 0.0);
    // 직전 곡선의 제어점 (S·T 반사용)
    let mut last_c: Option<(f64, f64)> = None;
    let mut last_q: Option<(f64, f64)> = None;
    for (cmd, a) in toks {
        let rel = cmd.is_ascii_lowercase();
        let up = cmd.to_ascii_uppercase();
        let n = match up {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => 0,
        };
        if up == 'Z' {
            out.push(Seg::Close);
            x = sx;
            y = sy;
            last_c = None;
            last_q = None;
            continue;
        }
        if n == 0 || a.len() < n {
            continue;
        }
        for (k, p) in a.chunks(n).enumerate() {
            if p.len() < n {
                break;
            }
            let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };
            match up {
                'M' if k == 0 => {
                    x = ox + p[0];
                    y = oy + p[1];
                    sx = x;
                    sy = y;
                    out.push(Seg::Move(x, y));
                    last_c = None;
                    last_q = None;
                }
                'M' | 'L' => {
                    x = ox + p[0];
                    y = oy + p[1];
                    out.push(Seg::Line(x, y));
                    last_c = None;
                    last_q = None;
                }
                'H' => {
                    x = if rel { x + p[0] } else { p[0] };
                    out.push(Seg::Line(x, y));
                    last_c = None;
                    last_q = None;
                }
                'V' => {
                    y = if rel { y + p[0] } else { p[0] };
                    out.push(Seg::Line(x, y));
                    last_c = None;
                    last_q = None;
                }
                'C' => {
                    let (x1, y1, x2, y2) = (ox + p[0], oy + p[1], ox + p[2], oy + p[3]);
                    x = ox + p[4];
                    y = oy + p[5];
                    out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                    last_c = Some((x2, y2));
                    last_q = None;
                }
                'S' => {
                    let (x1, y1) = last_c.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
                    let (x2, y2) = (ox + p[0], oy + p[1]);
                    x = ox + p[2];
                    y = oy + p[3];
                    out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                    last_c = Some((x2, y2));
                    last_q = None;
                }
                'Q' | 'T' => {
                    let (qx, qy, ex, ey) = if up == 'Q' {
                        (ox + p[0], oy + p[1], ox + p[2], oy + p[3])
                    } else {
                        let (qx, qy) =
                            last_q.map_or((x, y), |(cx, cy)| (2.0 * x - cx, 2.0 * y - cy));
                        (qx, qy, ox + p[0], oy + p[1])
                    };
                    out.push(Seg::Cubic(
                        x + 2.0 / 3.0 * (qx - x),
                        y + 2.0 / 3.0 * (qy - y),
                        ex + 2.0 / 3.0 * (qx - ex),
                        ey + 2.0 / 3.0 * (qy - ey),
                        ex,
                        ey,
                    ));
                    x = ex;
                    y = ey;
                    last_q = Some((qx, qy));
                    last_c = None;
                }
                'A' => {
                    let (ex, ey) = (ox + p[5], oy + p[6]);
                    arc(
                        &mut out,
                        (x, y),
                        p[0],
                        p[1],
                        p[2],
                        p[3] != 0.0,
                        p[4] != 0.0,
                        (ex, ey),
                    );
                    x = ex;
                    y = ey;
                    last_c = None;
                    last_q = None;
                }
                _ => {}
            }
        }
    }
    out
}

/// SVG 호 → 3차 베지어 (끝점 매개변수화, SVG 1.1 부록 F.6)
#[allow(clippy::too_many_arguments)]
fn arc(
    out: &mut Vec<Seg>,
    (x1, y1): (f64, f64),
    rx: f64,
    ry: f64,
    phi: f64,
    large: bool,
    sweep: bool,
    (x2, y2): (f64, f64),
) {
    if (x1 - x2).abs() < 1e-9 && (y1 - y2).abs() < 1e-9 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-9 || ry < 1e-9 {
        out.push(Seg::Line(x2, y2));
        return;
    }
    let (sp, cp) = phi.to_radians().sin_cos();
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = cp * dx + sp * dy;
    let y1p = -sp * dx + cp * dy;
    let lam = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lam > 1.0 {
        rx *= lam.sqrt();
        ry *= lam.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut co = (num / den).max(0.0).sqrt();
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1p / ry;
    let cyp = -co * ry * x1p / rx;
    let cx = cp * cxp - sp * cyp + (x1 + x2) / 2.0;
    let cy = sp * cxp + cp * cyp + (y1 + y2) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let t1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dt = ang(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f64::consts::TAU;
    }
    let n = (dt.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dt / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| {
        let (s, c) = t.sin_cos();
        (
            cx + rx * c * cp - ry * s * sp,
            cy + rx * c * sp + ry * s * cp,
        )
    };
    let der = |t: f64| {
        let (s, c) = t.sin_cos();
        (-rx * s * cp - ry * c * sp, -rx * s * sp + ry * c * cp)
    };
    for i in 0..n {
        let a = t1 + step * i as f64;
        let b = a + step;
        let (p0x, p0y) = pt(a);
        let (p3x, p3y) = pt(b);
        let (d0x, d0y) = der(a);
        let (d3x, d3y) = der(b);
        out.push(Seg::Cubic(
            p0x + k * d0x,
            p0y + k * d0y,
            p3x - k * d3x,
            p3y - k * d3y,
            p3x,
            p3y,
        ));
    }
}

/// CSS 색 → RGB (불투명도는 흰 바탕과 섞는다)
fn css_color(v: &str, alpha: f64, grads: &HashMap<String, String>) -> Option<[u8; 3]> {
    let v = v.trim();
    let base: [u8; 3] = if let Some(id) = v.strip_prefix("url(#").and_then(|r| r.strip_suffix(')'))
    {
        return css_color(grads.get(id)?, alpha, grads);
    } else if let Some(h) = v.strip_prefix('#') {
        let h: String = if h.len() == 3 {
            h.chars().flat_map(|c| [c, c]).collect()
        } else {
            h.chars().take(6).collect()
        };
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        [p(0)?, p(2)?, p(4)?]
    } else if let Some(r) = v.strip_prefix("rgb(") {
        let n = numbers(r);
        [*n.first()? as u8, *n.get(1)? as u8, *n.get(2)? as u8]
    } else {
        match v {
            "none" | "transparent" | "" => return None,
            "white" => [255, 255, 255],
            "black" => [0, 0, 0],
            "red" => [255, 0, 0],
            "gray" | "grey" => [128, 128, 128],
            _ => [0, 0, 0],
        }
    };
    let a = alpha.clamp(0.0, 1.0);
    if a <= 0.001 {
        return None;
    }
    Some(base.map(|c| (c as f64 * a + 255.0 * (1.0 - a)).round() as u8))
}

/// 물려받는 그리기 상태
#[derive(Clone)]
struct Style {
    fill: String,
    stroke: String,
    stroke_width: f64,
    dash: Vec<f64>,
    opacity: f64,
    fill_opacity: f64,
    stroke_opacity: f64,
    font_family: String,
    font_size: f64,
    bold: bool,
    italic: bool,
    anchor: String,
    even_odd: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: "#000000".into(),
            stroke: "none".into(),
            stroke_width: 1.0,
            dash: Vec::new(),
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            font_family: "맑은 고딕".into(),
            font_size: 16.0,
            bold: false,
            italic: false,
            anchor: "start".into(),
            even_odd: false,
        }
    }
}

fn first_family(v: &str) -> String {
    v.split(',')
        .map(|s| s.trim().trim_matches(['\'', '"']).to_string())
        .find(|s| !s.is_empty() && s != "sans-serif" && s != "serif")
        .unwrap_or_else(|| "맑은 고딕".into())
}

fn apply_attrs(st: &mut Style, e: &Element) {
    let mut set = |k: &str, v: &str| match k {
        "fill" => st.fill = v.to_string(),
        "stroke" => st.stroke = v.to_string(),
        "stroke-width" => st.stroke_width = numbers(v).first().copied().unwrap_or(1.0),
        "stroke-dasharray" => {
            st.dash = if v.trim() == "none" {
                Vec::new()
            } else {
                numbers(v)
            }
        }
        "opacity" => st.opacity *= v.trim().parse().unwrap_or(1.0),
        "fill-opacity" => st.fill_opacity = v.trim().parse().unwrap_or(1.0),
        "stroke-opacity" => st.stroke_opacity = v.trim().parse().unwrap_or(1.0),
        "font-family" => st.font_family = first_family(v),
        "font-size" => st.font_size = numbers(v).first().copied().unwrap_or(16.0),
        "font-weight" => {
            st.bold = v.contains("bold") || numbers(v).first().is_some_and(|w| *w >= 600.0)
        }
        "font-style" => st.italic = v.contains("italic"),
        "text-anchor" => st.anchor = v.trim().to_string(),
        "fill-rule" => st.even_odd = v.trim() == "evenodd",
        _ => {}
    };
    for a in &e.attrs {
        set(&a.name, &a.value);
    }
    if let Some(style) = e.attr("style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                set(k.trim(), v.trim());
            }
        }
    }
}

/// 그라데이션 id → 첫 정지점 색, clipPath id → 사각형
fn collect_defs(
    e: &Element,
    grads: &mut HashMap<String, String>,
    clips: &mut HashMap<String, [f64; 4]>,
) {
    for c in e.elements() {
        match c.name.as_str() {
            "linearGradient" | "radialGradient" | "pattern" => {
                if let Some(id) = c.attr("id") {
                    let mut col = None;
                    c.walk(&mut |x| {
                        if col.is_none() {
                            col = x
                                .attr("stop-color")
                                .or_else(|| x.attr("fill").filter(|f| f.starts_with('#')))
                                .map(str::to_string);
                        }
                    });
                    if let Some(col) = col {
                        grads.insert(id.to_string(), col);
                    }
                }
            }
            "clipPath" => {
                if let (Some(id), Some(r)) = (c.attr("id"), c.elements().find(|x| x.name == "rect"))
                {
                    let n = |k: &str| r.attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                    clips.insert(id.to_string(), [n("x"), n("y"), n("width"), n("height")]);
                }
            }
            _ => collect_defs(c, grads, clips),
        }
    }
}

/// 변환을 조각들에 적용
fn map_segs(segs: &[Seg], m: Mat) -> Vec<Seg> {
    segs.iter()
        .map(|s| match *s {
            Seg::Move(x, y) => {
                let (a, b) = apply(m, x, y);
                Seg::Move(a, b)
            }
            Seg::Line(x, y) => {
                let (a, b) = apply(m, x, y);
                Seg::Line(a, b)
            }
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                let (a1, b1) = apply(m, x1, y1);
                let (a2, b2) = apply(m, x2, y2);
                let (a, b) = apply(m, x, y);
                Seg::Cubic(a1, b1, a2, b2, a, b)
            }
            Seg::Close => Seg::Close,
        })
        .collect()
}

/// 글자 폭 재기 (가운데·끝 맞춤용)
pub type Measure<'a> = &'a dyn Fn(&FontKey, f64, &str) -> f64;

struct Ctx<'a> {
    grads: HashMap<String, String>,
    clips: HashMap<String, [f64; 4]>,
    measure: Measure<'a>,
    /// 선 굵기 배수 (px → 장치 단위)
    unit: f64,
}

fn node(cx: &Ctx, e: &Element, m: Mat, st: &Style, ops: &mut Vec<Op>) {
    let mut st = st.clone();
    apply_attrs(&mut st, e);
    let m = match e.attr("transform") {
        Some(t) => mul(m, transform(t)),
        None => m,
    };
    let n = |k: &str| {
        e.attr(k)
            .and_then(|v| numbers(v).first().copied())
            .unwrap_or(0.0)
    };
    let shape = |segs: Vec<Seg>, ops: &mut Vec<Op>, closed_fill: bool| {
        let fill = if closed_fill {
            css_color(&st.fill, st.opacity * st.fill_opacity, &cx.grads)
        } else {
            None
        };
        let stroke = css_color(&st.stroke, st.opacity * st.stroke_opacity, &cx.grads);
        if fill.is_none() && stroke.is_none() {
            return;
        }
        // 선 굵기는 변환의 크기만큼
        let scale = (m[0] * m[3] - m[1] * m[2]).abs().sqrt();
        ops.push(Op::Path {
            segs: map_segs(&segs, m),
            fill,
            stroke: stroke.map(|c| {
                (
                    st.stroke_width * scale,
                    c,
                    st.dash.iter().map(|d| d * scale).collect(),
                )
            }),
            even_odd: st.even_odd,
        });
        let _ = cx.unit;
    };
    match e.name.as_str() {
        "defs" | "clipPath" | "linearGradient" | "radialGradient" | "pattern" | "marker"
        | "filter" | "mask" | "title" | "style" => {}
        "rect" => {
            let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
            if w > 0.0 && h > 0.0 {
                shape(
                    vec![
                        Seg::Move(x, y),
                        Seg::Line(x + w, y),
                        Seg::Line(x + w, y + h),
                        Seg::Line(x, y + h),
                        Seg::Close,
                    ],
                    ops,
                    true,
                );
            }
        }
        "line" => shape(
            vec![Seg::Move(n("x1"), n("y1")), Seg::Line(n("x2"), n("y2"))],
            ops,
            false,
        ),
        "polyline" | "polygon" => {
            let p = numbers(e.attr("points").unwrap_or(""));
            let mut segs: Vec<Seg> = p
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    if i == 0 {
                        Seg::Move(c[0], c[1])
                    } else {
                        Seg::Line(c[0], c[1])
                    }
                })
                .collect();
            if e.name == "polygon" {
                segs.push(Seg::Close);
            }
            shape(segs, ops, e.name == "polygon" || st.fill != "none");
        }
        "circle" | "ellipse" => {
            let (cxv, cyv) = (n("cx"), n("cy"));
            let (rx, ry) = if e.name == "circle" {
                (n("r"), n("r"))
            } else {
                (n("rx"), n("ry"))
            };
            let d = format!(
                "M{} {}A{rx} {ry} 0 1 0 {} {}A{rx} {ry} 0 1 0 {} {}Z",
                cxv - rx,
                cyv,
                cxv + rx,
                cyv,
                cxv - rx,
                cyv
            );
            shape(path_segs(&d), ops, true);
        }
        "path" => {
            if let Some(d) = e.attr("d") {
                shape(path_segs(d), ops, true);
            }
        }
        "text" => {
            let text: String = e.text();
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let Some(color) = css_color(&st.fill, st.opacity * st.fill_opacity, &cx.grads) else {
                return;
            };
            let key = FontKey {
                name: st.font_family.clone(),
                bold: st.bold,
                italic: st.italic,
            };
            // 크기·자리: 변환의 크기 (회전은 각도로)
            let scale = (m[0] * m[0] + m[1] * m[1]).sqrt();
            let size = st.font_size * scale;
            let angle = m[1].atan2(m[0]).to_degrees();
            let w = (cx.measure)(&key, size, &text);
            let shift = match st.anchor.as_str() {
                "middle" => w / 2.0,
                "end" => w,
                _ => 0.0,
            };
            let (x, y) = apply(m, n("x"), n("y"));
            let (c, s) = (angle.to_radians().cos(), angle.to_radians().sin());
            ops.push(Op::Text {
                x: x - shift * c,
                y: y - shift * s,
                size,
                font: key,
                color,
                text,
                underline: false,
                strike: false,
                stretch: 1.0,
                angle,
            });
        }
        _ => {
            // svg, g, a …: 자식들 (clip-path 는 사각형으로)
            let clip = e
                .attr("clip-path")
                .and_then(|v| v.strip_prefix("url(#"))
                .and_then(|v| v.strip_suffix(')'))
                .and_then(|id| cx.clips.get(id));
            if let Some(r) = clip {
                let (x0, y0) = apply(m, r[0], r[1]);
                let (x1, y1) = apply(m, r[0] + r[2], r[1] + r[3]);
                ops.push(Op::Clip {
                    x0: x0.min(x1),
                    y0: y0.min(y1),
                    x1: x0.max(x1),
                    y1: y0.max(y1),
                });
            }
            for ch in &e.children {
                if let Node::Element(c) = ch {
                    node(cx, c, m, &st, ops);
                }
            }
            if clip.is_some() {
                ops.push(Op::Unclip);
            }
        }
    }
}

/// SVG 글(요소 하나 또는 여러 개)을 `m` 변환으로 옮겨 그리기 목록에 더한다. `unit`: px → 장치 단위
pub fn svg_ops(svg: &str, m: [f64; 6], measure: Measure, ops: &mut Vec<Op>) {
    let Ok(root) = crate::xml::parse_raw(&format!("<r>{svg}</r>"), "svg") else {
        return;
    };
    let mut grads = HashMap::new();
    let mut clips = HashMap::new();
    collect_defs(&root, &mut grads, &mut clips);
    let cx = Ctx {
        grads,
        clips,
        measure,
        unit: 1.0,
    };
    node(&cx, &root, m, &Style::default(), ops);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_and_transforms() {
        let s = path_segs("M0 0L10 0l0 10H0z");
        assert_eq!(
            s,
            vec![
                Seg::Move(0.0, 0.0),
                Seg::Line(10.0, 0.0),
                Seg::Line(10.0, 10.0),
                Seg::Line(0.0, 10.0),
                Seg::Close
            ]
        );
        let m = transform("translate(5,6) scale(2)");
        assert_eq!(apply(m, 1.0, 1.0), (7.0, 8.0));
        // 반원 호는 베지어 둘
        let a = path_segs("M0 0A5 5 0 0 1 10 0");
        assert_eq!(a.iter().filter(|s| matches!(s, Seg::Cubic(..))).count(), 2);
        assert_eq!(
            css_color("#ff0000", 0.5, &HashMap::new()),
            Some([255, 128, 128])
        );
    }
}
