// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 칸 채우기(무늬 18종, 그라데이션)와 테두리를 한셀 화면과 같은 픽셀로 그린다
//! (화면 갈무리 대조 실험, docs/format/screen.md).
//!
//! - 무늬는 칸마다 그 칸의 왼쪽 위(위·왼쪽 격자선 다음 픽셀)에 맞춘 4×4·8×4 타일이다 (테두리 확대 실험: 32px 행의
//!   가로줄 무늬가 행 위에서 2·3 번째 줄). 색을 생략하면 전경 검정, 배경 흰색
//!   (단, 단색 채우기에 전경색이 없으면 흰색).
//! - 그라데이션: 직선은 칸을 단위 정사각형으로 본 각도(45° = 왼쪽 위 → 오른쪽 아래 꼭짓점),
//!   경로는 안쪽 사각형(left/right/top/bottom)에서 바깥으로 퍼지는 사각 그라데이션.
//!   정지점이 셋 이상이면 한셀은 앞의 두 색만 써서 가운데 대칭(색1 → 색2 → 색1)으로 그린다.
//! - 테두리 선은 칸 경계 픽셀(격자선 자리) 위에 그린다. 굵기: 가는 선 1px, 보통 2px(경계와 그 위·왼쪽),
//!   굵은 선 3px(경계 ±1), 이중선은 경계 ±1 의 1px 두 줄. 점선 무늬는 칸의 왼쪽(위쪽) 경계에서 시작한다.

use std::fmt::Write as _;

use crate::model::{BorderEdge, Color, Fill, GradientFill};
use crate::theme::{self, Theme};

/// 무늬 타일: (너비, 높이, 전경 픽셀 판정)
type Tile = (u32, u32, fn(u32, u32) -> bool);

fn pattern_tile(kind: &str) -> Option<Tile> {
    let t: Tile = match kind {
        "mediumGray" => (2, 2, |x, y| (x + y) % 2 == 1),
        "darkGray" => (4, 2, |x, y| !((y == 0 && x == 0) || (y == 1 && x == 2))),
        "lightGray" => (4, 2, |x, y| (y == 0 && x == 0) || (y == 1 && x == 2)),
        "darkHorizontal" => (1, 4, |_, y| y >= 2),
        "darkVertical" => (4, 1, |x, _| x >= 2),
        "darkDown" => (4, 4, |x, y| matches!((x + 4 - y) % 4, 1 | 2)),
        "darkUp" => (4, 4, |x, y| (x + y) % 4 < 2),
        "darkGrid" => (4, 4, |x, y| {
            if y < 2 {
                matches!(x, 1 | 2)
            } else {
                matches!(x, 0 | 3)
            }
        }),
        "darkTrellis" => (4, 4, |x, y| match y {
            1 | 3 => true,
            0 => matches!(x, 1 | 2),
            _ => matches!(x, 0 | 3),
        }),
        "lightHorizontal" => (1, 4, |_, y| y == 1),
        "lightVertical" => (4, 1, |x, _| x == 3),
        "lightDown" => (4, 4, |x, y| (x + 4 - y) % 4 == 2),
        "lightUp" => (4, 4, |x, y| (x + y) % 4 == 1),
        "lightGrid" => (4, 4, |x, y| y == 1 || x == 3),
        "lightTrellis" => (4, 4, |x, y| match y {
            1 | 3 => x % 2 == 1,
            0 => x == 2,
            _ => x == 0,
        }),
        "gray125" => (4, 4, |x, y| (y == 0 && x == 2) || (y == 2 && x == 0)),
        "gray0625" => (8, 4, |x, y| (y == 0 && x == 4) || (y == 2 && x == 0)),
        _ => return None,
    };
    Some(t)
}

/// SVG 를 CSS url() 로 (# 등 몇 글자만 바꾼다). 따옴표는 모두 %27 로 — 결과를 `style="…"` 속성에
/// 그대로 넣으므로 큰따옴표가 있으면 속성이 끊긴다
pub fn svg_url(svg: &str) -> String {
    let mut s = String::with_capacity(svg.len() + 32);
    for c in svg.chars() {
        match c {
            '#' => s.push_str("%23"),
            '"' | '\'' => s.push_str("%27"),
            '<' => s.push_str("%3C"),
            '>' => s.push_str("%3E"),
            '\n' => {}
            c => s.push(c),
        }
    }
    format!("url('data:image/svg+xml,{s}')")
}

/// 채우기를 그린 결과
#[derive(Debug, Clone, Default)]
pub struct FillPaint {
    /// 칸에 넣을 CSS (`background…`)
    pub css: String,
    /// 이웃 격자선 자리를 덮을 대표 색 (#RRGGBB)
    pub color: Option<String>,
    /// 무늬·그라데이션: 칸 자신의 오른쪽·아래 격자선 자리도 같은 무늬로 (한셀 화면 — 무늬가 격자선 자리를 지나간다)
    pub tiled: bool,
}

fn col(c: Option<&Color>, theme: &Theme) -> Option<String> {
    c.and_then(|c| theme::resolve(c, theme))
}

/// 칸 채우기 → CSS. `origin` 은 칸의 시트 안 위치(px) — 무늬를 시트 원점에 맞춘다
pub fn fill_css(fill: &Fill, theme: &Theme, origin: (u32, u32)) -> Option<FillPaint> {
    if let Some(g) = &fill.gradient {
        return gradient_css(g, theme);
    }
    let pt = fill.pattern_type.as_deref()?;
    if pt == "none" {
        return None;
    }
    let fg = col(fill.fg_color.as_ref(), theme);
    let bg = col(fill.bg_color.as_ref(), theme);
    if pt == "solid" {
        let c = fg.unwrap_or_else(|| "#FFFFFF".into());
        return Some(FillPaint {
            css: format!("background:{c};"),
            color: Some(c),
            tiled: false,
        });
    }
    let fg = fg.unwrap_or_else(|| "#000000".into());
    let bg = bg.unwrap_or_else(|| "#FFFFFF".into());
    let Some((w, h, on)) = pattern_tile(pt) else {
        return Some(FillPaint {
            css: format!("background:{fg};"),
            color: Some(fg),
            tiled: false,
        });
    };
    let mut svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' shape-rendering='crispEdges'><rect width='{w}' height='{h}' fill='{bg}'/><path fill='{fg}' d='"
    );
    let mut dark = 0;
    for y in 0..h {
        for x in 0..w {
            if on(x, y) {
                let _ = write!(svg, "M{x} {y}h1v1h-1z");
                dark += 1;
            }
        }
    }
    svg.push_str("'/></svg>");
    let _ = origin;
    let (ox, oy) = (0, 0);
    // 대표 색: 전경이 반 넘게 차지하면 전경, 아니면 배경
    let rep = if dark * 2 > w * h { fg } else { bg.clone() };
    Some(FillPaint {
        css: format!(
            "background:{bg} {} -{ox}px -{oy}px repeat;image-rendering:pixelated;",
            svg_url(&svg)
        ),
        color: Some(rep),
        tiled: true,
    })
}

/// 그라데이션 정지점 → (위치, 색). 한셀: 셋 이상이면 앞 두 색으로 가운데 대칭
fn gradient_stops(g: &GradientFill, theme: &Theme) -> Vec<(f64, String)> {
    let cs: Vec<String> = g
        .stops
        .iter()
        .map(|s| theme::resolve(&s.color, theme).unwrap_or_else(|| "#FFFFFF".into()))
        .collect();
    match cs.len() {
        0 => vec![(0.0, "#FFFFFF".into()), (1.0, "#FFFFFF".into())],
        1 => vec![(0.0, cs[0].clone()), (1.0, cs[0].clone())],
        2 => vec![(0.0, cs[0].clone()), (1.0, cs[1].clone())],
        _ => vec![
            (0.0, cs[0].clone()),
            (0.5, cs[1].clone()),
            (1.0, cs[0].clone()),
        ],
    }
}

fn stops_xml(stops: &[(f64, String)]) -> String {
    stops
        .iter()
        .map(|(p, c)| format!("<stop offset='{p}' stop-color='{c}'/>"))
        .collect()
}

fn gradient_css(g: &GradientFill, theme: &Theme) -> Option<FillPaint> {
    let stops = gradient_stops(g, theme);
    let rep = stops.last().map(|s| s.1.clone());
    let body = if g.path {
        // 사각 경로 그라데이션: 안쪽 사각형(정지점 0) → 칸 가장자리(정지점 1).
        // 네 사다리꼴마다 가장자리에 수직인 직선 그라데이션
        let (l, r, t, b) = (
            g.left.clamp(0.0, 1.0),
            g.right.clamp(0.0, 1.0),
            g.top.clamp(0.0, 1.0),
            g.bottom.clamp(0.0, 1.0),
        );
        let (l, r) = (l.min(r), l.max(r));
        let (t, b) = (t.min(b), t.max(b));
        let s = stops_xml(&stops);
        let c0 = &stops[0].1;
        format!(
            "<defs><linearGradient id='L' gradientUnits='userSpaceOnUse' x1='{l}' y1='0' x2='0' y2='0'>{s}</linearGradient>\
             <linearGradient id='R' gradientUnits='userSpaceOnUse' x1='{r}' y1='0' x2='1' y2='0'>{s}</linearGradient>\
             <linearGradient id='T' gradientUnits='userSpaceOnUse' x1='0' y1='{t}' x2='0' y2='0'>{s}</linearGradient>\
             <linearGradient id='B' gradientUnits='userSpaceOnUse' x1='0' y1='{b}' x2='0' y2='1'>{s}</linearGradient></defs>\
             <path fill='url(#L)' d='M0 0L{l} {t}L{l} {b}L0 1Z'/><path fill='url(#R)' d='M1 0L{r} {t}L{r} {b}L1 1Z'/>\
             <path fill='url(#T)' d='M0 0L1 0L{r} {t}L{l} {t}Z'/><path fill='url(#B)' d='M0 1L1 1L{r} {b}L{l} {b}Z'/>\
             <rect x='{l}' y='{t}' width='{}' height='{}' fill='{c0}'/>",
            r - l,
            b - t
        )
    } else {
        // 칸을 단위 정사각형으로 본 각도: 꼭짓점이 0 과 1 에 오도록
        let th = g.degree.to_radians();
        let (c, s) = (th.cos(), th.sin());
        let d = c.abs() + s.abs();
        let (x1, y1) = (0.5 - c * d / 2.0, 0.5 - s * d / 2.0);
        let (x2, y2) = (0.5 + c * d / 2.0, 0.5 + s * d / 2.0);
        format!(
            "<defs><linearGradient id='g' gradientUnits='userSpaceOnUse' x1='{x1:.4}' y1='{y1:.4}' x2='{x2:.4}' y2='{y2:.4}'>{}</linearGradient></defs><rect width='1' height='1' fill='url(#g)'/>",
            stops_xml(&stops)
        )
    };
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1 1' preserveAspectRatio='none'>{body}</svg>"
    );
    Some(FillPaint {
        css: format!(
            "background:{} {};background-size:100% 100%;",
            rep.clone().unwrap_or_else(|| "#FFFFFF".into()),
            svg_url(&svg)
        ),
        color: rep,
        tiled: true,
    })
}

// ---------------------------------------------------------------------------
// 테두리
// ---------------------------------------------------------------------------

/// 선 종류 → (굵기 px, 점선 무늬 [켜짐, 꺼짐, …], 이중선)
pub fn line_style(style: &str) -> Option<(u32, &'static [u32], bool)> {
    Some(match style {
        "thin" => (1, &[], false),
        "medium" => (2, &[], false),
        "thick" => (3, &[], false),
        "double" => (1, &[], true),
        "hair" => (1, &[1, 1], false),
        "dotted" => (1, &[3, 1], false),
        "dashed" => (1, &[5, 1], false),
        "dashDot" => (1, &[10, 2, 3, 2], false),
        "dashDotDot" => (1, &[10, 2, 3, 2, 3, 2], false),
        "mediumDashed" => (2, &[6, 2], false),
        "mediumDashDot" => (2, &[12, 2, 3, 2], false),
        "mediumDashDotDot" => (2, &[12, 2, 3, 2, 3, 2], false),
        "slantDashDot" => (2, &[10, 1, 1, 1], false),
        _ => return None,
    })
}

/// 테두리 한 변 (스타일, 색)
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub style: String,
    pub color: String,
}

pub fn edge(e: Option<&BorderEdge>, theme: &Theme) -> Option<Edge> {
    let e = e?;
    let style = e.style.as_deref().filter(|s| *s != "none")?;
    line_style(style)?;
    Some(Edge {
        style: style.to_string(),
        color: e
            .color
            .as_ref()
            .and_then(|c| theme::resolve(c, theme))
            .unwrap_or_else(|| "#000000".into()),
    })
}

/// 칸 하나의 테두리 (왼·오른·위·아래, 대각선 ↘·↗)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellBorders {
    pub left: Option<Edge>,
    pub right: Option<Edge>,
    pub top: Option<Edge>,
    pub bottom: Option<Edge>,
    pub down: Option<Edge>,
    pub up: Option<Edge>,
}

impl CellBorders {
    pub fn is_empty(&self) -> bool {
        *self == CellBorders::default()
    }
}

/// 가로선 하나 (y 행, x0..=x1) — 점선이면 무늬를 x0 에서 시작
fn hline(s: &mut String, y: i64, x0: i64, x1: i64, dash: &[u32], color: &str) {
    if x1 < x0 {
        return;
    }
    if dash.is_empty() {
        let _ = write!(
            s,
            "<rect x='{x0}' y='{y}' width='{}' height='1' fill='{color}'/>",
            x1 - x0 + 1
        );
    } else {
        let _ = write!(
            s,
            "<line x1='{x0}' y1='{}' x2='{}' y2='{}' stroke='{color}' stroke-dasharray='{}'/>",
            y as f64 + 0.5,
            x1 + 1,
            y as f64 + 0.5,
            dash.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

fn vline(s: &mut String, x: i64, y0: i64, y1: i64, dash: &[u32], color: &str) {
    if y1 < y0 {
        return;
    }
    if dash.is_empty() {
        let _ = write!(
            s,
            "<rect x='{x}' y='{y0}' width='1' height='{}' fill='{color}'/>",
            y1 - y0 + 1
        );
    } else {
        let _ = write!(
            s,
            "<line x1='{}' y1='{y0}' x2='{}' y2='{}' stroke='{color}' stroke-dasharray='{}'/>",
            x as f64 + 0.5,
            x as f64 + 0.5,
            y1 + 1,
            dash.iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}

/// 경계선 하나의 픽셀 줄 위치 (경계 픽셀 b 기준 오프셋들)
fn offsets(e: &Edge) -> (&'static [i64], &'static [u32]) {
    let (w, dash, double) = line_style(&e.style).unwrap_or((1, &[], false));
    let o: &'static [i64] = if double {
        &[-1, 1]
    } else {
        match w {
            1 => &[0],
            2 => &[-1, 0],
            _ => &[-1, 0, 1],
        }
    };
    (o, dash)
}

/// 칸의 테두리를 그리는 SVG. 칸 박스는 w×h px(오른쪽·아래 끝 픽셀이 격자선 자리),
/// SVG 는 칸 왼쪽 위에서 (-2,-2) 만큼 밖으로 나가 (w+3)×(h+3) 을 덮는다.
/// 이중선의 가운데 틈(경계 픽셀)은 `gap` 색으로 칠한다 (위·아래·왼쪽·오른쪽 차례) — 그 자리를 덮는 칸 채우기, 없으면
/// 흰색 (한셀 화면: 격자선이 보이지 않는다, 표 스타일 실험의 요약 행 위 이중선)
pub fn borders_svg(b: &CellBorders, w: u32, h: u32, gap: &[String; 4]) -> String {
    let (w, h) = (w as i64, h as i64);
    // SVG 좌표의 경계 픽셀: 왼쪽 = 이전 칸의 격자선, 오른쪽 = 이 칸의 마지막 픽셀
    let (bl, br, bt, bb) = (1i64, w + 1, 1i64, h + 1);
    let mut s = String::new();
    if let Some(e) = &b.top {
        let (o, d) = offsets(e);
        if o.len() == 2 && o[0] == -1 && o[1] == 1 {
            hline(&mut s, bt, bl, br, &[], &gap[0]);
        }
        for &k in o {
            hline(&mut s, bt + k, bl, br, d, &e.color);
        }
    }
    if let Some(e) = &b.bottom {
        let (o, d) = offsets(e);
        if o.len() == 2 && o[0] == -1 && o[1] == 1 {
            hline(&mut s, bb, bl, br, &[], &gap[1]);
        }
        for &k in o {
            hline(&mut s, bb + k, bl, br, d, &e.color);
        }
    }
    if let Some(e) = &b.left {
        let (o, d) = offsets(e);
        if o.len() == 2 && o[0] == -1 && o[1] == 1 {
            vline(&mut s, bl, bt, bb, &[], &gap[2]);
        }
        for &k in o {
            vline(&mut s, bl + k, bt, bb, d, &e.color);
        }
    }
    if let Some(e) = &b.right {
        let (o, d) = offsets(e);
        if o.len() == 2 && o[0] == -1 && o[1] == 1 {
            vline(&mut s, br, bt, bb, &[], &gap[3]);
        }
        for &k in o {
            vline(&mut s, br + k, bt, bb, d, &e.color);
        }
    }
    for (e, up) in [(&b.down, false), (&b.up, true)] {
        let Some(e) = e else { continue };
        let (wd, dash, double) = line_style(&e.style).unwrap_or((1, &[], false));
        let (y0, y1) = if up {
            (bb as f64 + 0.5, bt as f64 + 0.5)
        } else {
            (bt as f64 + 0.5, bb as f64 + 0.5)
        };
        let da = if dash.is_empty() {
            String::new()
        } else {
            format!(
                " stroke-dasharray='{}'",
                dash.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        let shifts: &[f64] = if double { &[-1.0, 1.0] } else { &[0.0] };
        for dy in shifts {
            let _ = write!(
                s,
                "<line x1='{}' y1='{}' x2='{}' y2='{}' stroke='{}' stroke-width='{wd}'{da}/>",
                bl as f64 + 0.5,
                y0 + dy,
                br as f64 + 0.5,
                y1 + dy,
                e.color
            );
        }
    }
    format!(
        "<svg class=\"rc-bd\" width=\"{}\" height=\"{}\" shape-rendering=\"crispEdges\">{s}</svg>",
        w + 3,
        h + 3
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_tiles_match_hancell_screen() {
        // 화면 갈무리에서 읽은 첫 줄 (시트 원점 기준, K=전경)
        let row = |p: &str, y: u32, n: u32| -> String {
            let (w, h, on) = pattern_tile(p).unwrap();
            (0..n)
                .map(|x| if on(x % w, y % h) { 'K' } else { 'W' })
                .collect()
        };
        assert_eq!(row("mediumGray", 0, 4), "WKWK");
        assert_eq!(row("darkGray", 0, 4), "WKKK");
        assert_eq!(row("darkGray", 1, 4), "KKWK");
        assert_eq!(row("lightGray", 1, 4), "WWKW");
        assert_eq!(row("darkDown", 0, 4), "WKKW");
        assert_eq!(row("darkUp", 2, 4), "WWKK");
        assert_eq!(row("lightUp", 2, 4), "WWWK");
        assert_eq!(row("lightTrellis", 2, 4), "KWWW");
        assert_eq!(row("gray0625", 2, 8), "KWWWWWWW");
        assert_eq!(row("gray125", 0, 4), "WWKW");
    }

    #[test]
    fn border_pixels() {
        let thick = Edge {
            style: "thick".into(),
            color: "#000000".into(),
        };
        let b = CellBorders {
            bottom: Some(thick),
            ..Default::default()
        };
        // 72×22 칸의 아래 굵은 선: 경계(y=23) ±1, 왼쪽 경계(x=1)부터 오른쪽 경계(x=73)까지
        let white = "#FFFFFF".to_string();
        let s = borders_svg(
            &b,
            72,
            22,
            &[white.clone(), white.clone(), white.clone(), white],
        );
        assert!(s.contains("<rect x='1' y='22' width='73' height='1'"));
        assert!(s.contains("<rect x='1' y='24' width='73' height='1'"));
    }
}
