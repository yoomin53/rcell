// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 도형 윤곽을 SVG 경로로 (사용자 정의 모양 `a:custGeom`, 자주 쓰는 미리 정의된 모양).
//!
//! 문서 모델은 모양 이름만 담으므로 경로·조정값·스타일 참조(`xdr:style`)·화살표 끝은 도형의
//! 원본 앵커 XML(`Shape::raw`)에서 다시 읽는다 (그리기 전용).

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::xml::Element;

pub const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

/// 앵커 원본 XML 에서 개체 요소(sp/cxnSp/pic/graphicFrame/grpSp)를 꺼낸다
pub fn object_element(raw: &str) -> Option<Element> {
    let root = crate::xml::parse(raw, "drawing").ok()?;
    root.elements()
        .find(|c| {
            c.ns.as_deref() == Some(XDR)
                && matches!(
                    c.name.as_str(),
                    "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp"
                )
        })
        .cloned()
}

/// spPr (또는 grpSpPr)
pub fn sp_pr(obj: &Element) -> Option<&Element> {
    obj.child_ns(XDR, "spPr")
        .or_else(|| obj.child_ns(XDR, "grpSpPr"))
}

/// `a:avLst` 의 조정값 (`val n` 만)
pub(super) fn adjusts(geom: &Element) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    if let Some(av) = geom.child_ns(A, "avLst") {
        for gd in av.elements().filter(|g| g.is(A, "gd")) {
            if let (Some(n), Some(f)) = (gd.attr("name"), gd.attr("fmla"))
                && let Some(v) = f.strip_prefix("val ").and_then(|v| v.trim().parse().ok())
            {
                m.insert(n.to_string(), v);
            }
        }
    }
    m
}

fn f(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// 경로 하나의 채움 (DrawingML 경로의 fill 속성): 보통, 없음, 어둡게·덜 어둡게·밝게·덜 밝게
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathFill {
    Norm,
    None,
    Darken,
    DarkenLess,
    Lighten,
    LightenLess,
}

impl PathFill {
    pub fn parse(v: Option<&str>) -> PathFill {
        match v {
            Some("none") => PathFill::None,
            Some("darken") => PathFill::Darken,
            Some("darkenLess") => PathFill::DarkenLess,
            Some("lighten") => PathFill::Lighten,
            Some("lightenLess") => PathFill::LightenLess,
            _ => PathFill::Norm,
        }
    }

    /// 채움 위에 덧씌울 반투명 색 (도형 모양 실험의 한셀 화면: 어둡게 = 검정 50%, 덜 어둡게 25%, 밝게 = 흰색 30%,
    /// 덜 밝게 15% — 6182D6 이 31416B·4962A1·90A7E2·7995DC)
    pub fn overlay(self) -> Option<(&'static str, f64)> {
        match self {
            PathFill::Darken => Some(("#000000", 0.5)),
            PathFill::DarkenLess => Some(("#000000", 0.25)),
            PathFill::Lighten => Some(("#FFFFFF", 0.3)),
            PathFill::LightenLess => Some(("#FFFFFF", 0.15)),
            _ => None,
        }
    }
}

/// SVG 경로 하나 + 채움 + 선
#[derive(Debug, Clone, PartialEq)]
pub struct PathSpec {
    pub d: String,
    pub fill: PathFill,
    pub stroke: bool,
}

impl PathSpec {
    fn new(d: String) -> PathSpec {
        PathSpec {
            d,
            fill: PathFill::Norm,
            stroke: true,
        }
    }
    fn with(d: String, fill: PathFill, stroke: bool) -> PathSpec {
        PathSpec { d, fill, stroke }
    }
}

fn poly(pts: &[(f64, f64)]) -> String {
    let mut s = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(s, "{}{},{}", if i == 0 { "M" } else { "L" }, f(*x), f(*y));
    }
    s.push('Z');
    s
}

fn open(pts: &[(f64, f64)]) -> String {
    let mut s = poly(pts);
    s.pop();
    s
}

/// 타원 (중심, 반지름) 하나를 닫힌 경로로
fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64) -> String {
    format!(
        "M{},{}A{},{} 0 1 1 {},{}A{},{} 0 1 1 {},{}Z",
        f(cx - rx),
        f(cy),
        f(rx),
        f(ry),
        f(cx + rx),
        f(cy),
        f(rx),
        f(ry),
        f(cx - rx),
        f(cy)
    )
}

/// 모서리를 둥글린 사각형 (모서리마다 반지름: 왼위·오른위·오른아래·왼아래)
fn round_rect(x: f64, y: f64, w: f64, h: f64, r: [f64; 4]) -> String {
    let m = (w / 2.0).min(h / 2.0);
    let [a, b, c, d] = r.map(|v| v.clamp(0.0, m));
    let arc = |rr: f64, ex: f64, ey: f64| {
        if rr > 0.0 {
            format!("A{},{} 0 0 1 {},{}", f(rr), f(rr), f(ex), f(ey))
        } else {
            format!("L{},{}", f(ex), f(ey))
        }
    };
    format!(
        "M{},{}H{}{}V{}{}H{}{}V{}{}Z",
        f(x + a),
        f(y),
        f(x + w - b),
        arc(b, x + w, y + b),
        f(y + h - c),
        arc(c, x + w - c, y + h),
        f(x + d),
        arc(d, x, y + h - d),
        f(y + a),
        arc(a, x + a, y)
    )
}

/// 정다각형·별: n 꼭짓점 (별이면 2n), 타원(w/2, h/2)에 내접, 위 꼭짓점부터
fn star(w: f64, h: f64, n: usize, inner: Option<f64>) -> String {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let k = if inner.is_some() { 2 * n } else { n };
    let pts: Vec<(f64, f64)> = (0..k)
        .map(|i| {
            let ang = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / k as f64;
            let r = match inner {
                Some(ri) if i % 2 == 1 => ri,
                _ => 1.0,
            };
            (cx + cx * r * ang.cos(), cy + cy * r * ang.sin())
        })
        .collect();
    poly(&pts)
}

/// 미리 정의된 모양 → 경로들 (w×h px 상자). 조정값은 1/100000 단위 비율. 모르는 모양은 None
/// (그리는 쪽이 사각형으로 그린다). 모양은 ECMA-376 의 이름과 기본 조정값을 따라 직접 그린 근사다
pub fn preset_paths(
    prst: &str,
    w: f64,
    h: f64,
    adj: &HashMap<String, f64>,
) -> Option<Vec<PathSpec>> {
    let ss = w.min(h);
    let a = |n: &str, d: f64| adj.get(n).copied().unwrap_or(d) / 100_000.0;
    let one = |d: String| Some(vec![PathSpec::new(d)]);
    let (hc, vc) = (w / 2.0, h / 2.0);
    match prst {
        "rect" | "flowChartProcess" | "textBox" => {
            one(poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]))
        }
        "flowChartPredefinedProcess" => {
            let x = w / 8.0;
            Some(vec![
                PathSpec::new(poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])),
                PathSpec::with(
                    format!("M{},0V{}M{},0V{}", f(x), f(h), f(w - x), f(h)),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "flowChartInternalStorage" => {
            let (x, y) = (w / 8.0, h / 8.0);
            Some(vec![
                PathSpec::new(poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])),
                PathSpec::with(
                    format!("M{},0V{}M0,{}H{}", f(x), f(h), f(y), f(w)),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "roundRect" | "flowChartAlternateProcess" | "flowChartTerminator" => {
            let r = match prst {
                "flowChartTerminator" => h / 2.0,
                "flowChartAlternateProcess" => ss / 6.0,
                _ => ss * a("adj", 16667.0),
            };
            one(round_rect(0.0, 0.0, w, h, [r; 4]))
        }
        "round1Rect" => one(round_rect(
            0.0,
            0.0,
            w,
            h,
            [0.0, ss * a("adj", 16667.0), 0.0, 0.0],
        )),
        "round2SameRect" => {
            let (t, b) = (ss * a("adj1", 16667.0), ss * a("adj2", 0.0));
            one(round_rect(0.0, 0.0, w, h, [t, t, b, b]))
        }
        "round2DiagRect" => {
            let (p, q) = (ss * a("adj1", 16667.0), ss * a("adj2", 0.0));
            one(round_rect(0.0, 0.0, w, h, [p, q, p, q]))
        }
        "snip1Rect" => {
            let x = ss * a("adj", 16667.0);
            one(poly(&[(0.0, 0.0), (w - x, 0.0), (w, x), (w, h), (0.0, h)]))
        }
        "snip2SameRect" => {
            let x = ss * a("adj1", 16667.0);
            let y = ss * a("adj2", 0.0);
            one(poly(&[
                (x, 0.0),
                (w - x, 0.0),
                (w, x),
                (w, h - y),
                (w - y, h),
                (y, h),
                (0.0, h - y),
                (0.0, x),
            ]))
        }
        "snipRoundRect" => {
            let r = ss * a("adj1", 16667.0);
            let x = ss * a("adj2", 16667.0);
            one(format!(
                "M{},0H{}L{},{}V{}H0V{}A{},{} 0 0 1 {},0Z",
                f(r),
                f(w - x),
                f(w),
                f(x),
                f(h),
                f(r),
                f(r),
                f(r),
                f(r)
            ))
        }
        "ellipse" | "flowChartConnector" => one(ellipse(hc, vc, hc, vc)),
        "triangle" | "flowChartExtract" => {
            let x = if prst == "triangle" {
                w * a("adj", 50000.0)
            } else {
                hc
            };
            one(poly(&[(0.0, h), (x, 0.0), (w, h)]))
        }
        "flowChartMerge" => one(poly(&[(0.0, 0.0), (w, 0.0), (hc, h)])),
        "rtTriangle" => one(poly(&[(0.0, 0.0), (0.0, h), (w, h)])),
        "diamond" | "flowChartDecision" => one(poly(&[(hc, 0.0), (w, vc), (hc, h), (0.0, vc)])),
        "flowChartSort" => Some(vec![
            PathSpec::new(poly(&[(hc, 0.0), (w, vc), (hc, h), (0.0, vc)])),
            PathSpec::with(format!("M0,{}H{}", f(vc), f(w)), PathFill::None, true),
        ]),
        "flowChartCollate" => one(poly(&[(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)])),
        "parallelogram" | "flowChartInputOutput" => {
            let x = if prst == "parallelogram" {
                ss * a("adj", 25000.0)
            } else {
                w / 5.0
            };
            one(poly(&[(x, 0.0), (w, 0.0), (w - x, h), (0.0, h)]))
        }
        "trapezoid" => {
            let x = ss * a("adj", 25000.0);
            one(poly(&[(0.0, h), (x, 0.0), (w - x, 0.0), (w, h)]))
        }
        "flowChartManualOperation" => {
            let x = w / 5.0;
            one(poly(&[(0.0, 0.0), (w, 0.0), (w - x, h), (x, h)]))
        }
        "flowChartManualInput" => one(poly(&[(0.0, h / 5.0), (w, 0.0), (w, h), (0.0, h)])),
        "flowChartPunchedCard" => one(poly(&[
            (w / 5.0, 0.0),
            (w, 0.0),
            (w, h),
            (0.0, h),
            (0.0, h / 5.0),
        ])),
        "flowChartOffpageConnector" => one(poly(&[
            (0.0, 0.0),
            (w, 0.0),
            (w, h * 0.8),
            (hc, h),
            (0.0, h * 0.8),
        ])),
        "flowChartDelay" => one(format!(
            "M0,0H{}A{},{} 0 0 1 {},{}H0Z",
            f(hc),
            f(hc),
            f(vc),
            f(hc),
            f(h)
        )),
        "flowChartDisplay" => one(format!(
            "M0,{}L{},0H{}A{},{} 0 0 1 {},{}H{}Z",
            f(vc),
            f(w / 6.0),
            f(w * 5.0 / 6.0),
            f(w / 6.0),
            f(vc),
            f(w * 5.0 / 6.0),
            f(h),
            f(w / 6.0)
        )),
        "flowChartOnlineStorage" => one(format!(
            "M{},0H{}A{},{} 0 0 0 {},{}H{}A{},{} 0 0 1 {},0Z",
            f(w / 6.0),
            f(w),
            f(w / 6.0),
            f(vc),
            f(w),
            f(h),
            f(w / 6.0),
            f(w / 6.0),
            f(vc),
            f(w / 6.0)
        )),
        "flowChartSummingJunction" | "flowChartOr" => {
            let (dx, dy) = (
                hc * std::f64::consts::FRAC_1_SQRT_2,
                vc * std::f64::consts::FRAC_1_SQRT_2,
            );
            let cross = if prst == "flowChartOr" {
                format!("M{},0V{}M0,{}H{}", f(hc), f(h), f(vc), f(w))
            } else {
                format!(
                    "M{},{}L{},{}M{},{}L{},{}",
                    f(hc - dx),
                    f(vc - dy),
                    f(hc + dx),
                    f(vc + dy),
                    f(hc + dx),
                    f(vc - dy),
                    f(hc - dx),
                    f(vc + dy)
                )
            };
            Some(vec![
                PathSpec::new(ellipse(hc, vc, hc, vc)),
                PathSpec::with(cross, PathFill::None, true),
            ])
        }
        "flowChartPunchedTape" => one(format!(
            "M0,{}Q{},{} {},{}T{},{}V{}Q{},{} {},{}T0,{}Z",
            f(h * 0.1),
            f(w * 0.25),
            f(h * 0.3),
            f(hc),
            f(h * 0.1),
            f(w),
            f(h * 0.1),
            f(h * 0.9),
            f(w * 0.75),
            f(h * 0.7),
            f(hc),
            f(h * 0.9),
            f(h * 0.9)
        )),
        "flowChartDocument" => one(format!(
            "M0,0H{}V{}C{},{} {},{} 0,{}Z",
            f(w),
            f(h * 0.8),
            f(w * 0.5),
            f(h * 0.6),
            f(w * 0.5),
            f(h * 1.1),
            f(h * 0.85)
        )),
        "flowChartMultidocument" => {
            let (dx, dy) = (w * 0.1, h * 0.1);
            let doc = |x: f64, y: f64, ww: f64, hh: f64| {
                format!(
                    "M{},{}H{}V{}C{},{} {},{} {},{}Z",
                    f(x),
                    f(y),
                    f(x + ww),
                    f(y + hh * 0.8),
                    f(x + ww * 0.5),
                    f(y + hh * 0.6),
                    f(x + ww * 0.5),
                    f(y + hh * 1.1),
                    f(x),
                    f(y + hh * 0.85)
                )
            };
            Some(vec![
                PathSpec::new(doc(2.0 * dx, 0.0, w - 2.0 * dx, h - 2.0 * dy)),
                PathSpec::new(doc(dx, dy, w - 2.0 * dx, h - 2.0 * dy)),
                PathSpec::new(doc(0.0, 2.0 * dy, w - 2.0 * dx, h - 2.0 * dy)),
            ])
        }
        "homePlate" => {
            let x = (ss * a("adj", 50000.0)).min(w);
            one(poly(&[
                (0.0, 0.0),
                (w - x, 0.0),
                (w, vc),
                (w - x, h),
                (0.0, h),
            ]))
        }
        "pentagon" => one(poly(&[
            (hc, 0.0),
            (w, h * 0.3820),
            (w * 0.8090, h),
            (w * 0.1910, h),
            (0.0, h * 0.3820),
        ])),
        "chevron" => {
            let x = (ss * a("adj", 50000.0)).min(w);
            one(poly(&[
                (0.0, 0.0),
                (w - x, 0.0),
                (w, vc),
                (w - x, h),
                (0.0, h),
                (x, vc),
            ]))
        }
        "hexagon" | "flowChartPreparation" => {
            let x = if prst == "hexagon" {
                ss * a("adj", 25000.0)
            } else {
                w / 5.0
            };
            one(poly(&[
                (0.0, vc),
                (x, 0.0),
                (w - x, 0.0),
                (w, vc),
                (w - x, h),
                (x, h),
            ]))
        }
        "octagon" => {
            let x = ss * a("adj", 29289.0);
            one(poly(&[
                (x, 0.0),
                (w - x, 0.0),
                (w, x),
                (w, h - x),
                (w - x, h),
                (x, h),
                (0.0, h - x),
                (0.0, x),
            ]))
        }
        "heptagon" => one(star(w, h, 7, None)),
        "decagon" => one(star(w, h, 10, None)),
        "dodecagon" => one(star(w, h, 12, None)),
        "plus" | "mathPlus" => {
            let x = ss * a("adj", if prst == "plus" { 25000.0 } else { 23520.0 });
            let (x0, y0) = if prst == "plus" {
                (x, x)
            } else {
                (hc - x / 2.0, vc - x / 2.0)
            };
            let (x1, y1) = if prst == "plus" {
                (w - x, h - x)
            } else {
                (hc + x / 2.0, vc + x / 2.0)
            };
            one(poly(&[
                (x0, 0.0),
                (x1, 0.0),
                (x1, y0),
                (w, y0),
                (w, y1),
                (x1, y1),
                (x1, h),
                (x0, h),
                (x0, y1),
                (0.0, y1),
                (0.0, y0),
                (x0, y0),
            ]))
        }
        "mathMinus" => {
            let t = h * a("adj1", 23520.0);
            one(poly(&[
                (0.0, vc - t / 2.0),
                (w, vc - t / 2.0),
                (w, vc + t / 2.0),
                (0.0, vc + t / 2.0),
            ]))
        }
        "mathEqual" => {
            let t = h * a("adj1", 23520.0);
            let g = h * a("adj2", 11760.0);
            Some(vec![
                PathSpec::new(poly(&[
                    (0.0, vc - g / 2.0 - t),
                    (w, vc - g / 2.0 - t),
                    (w, vc - g / 2.0),
                    (0.0, vc - g / 2.0),
                ])),
                PathSpec::new(poly(&[
                    (0.0, vc + g / 2.0),
                    (w, vc + g / 2.0),
                    (w, vc + g / 2.0 + t),
                    (0.0, vc + g / 2.0 + t),
                ])),
            ])
        }
        "mathMultiply" => {
            let t = ss * a("adj1", 23520.0) / 2.0;
            let (dx, dy) = (t, t);
            one(poly(&[
                (dx, 0.0),
                (hc, vc - dy),
                (w - dx, 0.0),
                (w, dy),
                (hc + dx, vc),
                (w, h - dy),
                (w - dx, h),
                (hc, vc + dy),
                (dx, h),
                (0.0, h - dy),
                (hc - dx, vc),
                (0.0, dy),
            ]))
        }
        "star4" | "star5" | "star6" | "star7" | "star8" | "star10" | "star12" | "star16"
        | "star24" | "star32" => {
            let n: usize = prst[4..].parse().unwrap_or(5);
            let d = match n {
                4 => 12500.0,
                5 => 19098.0,
                6 => 28868.0,
                7 => 34601.0,
                10 => 42533.0,
                _ => 37500.0,
            };
            one(star(w, h, n, Some(a("adj", d) * 2.0)))
        }
        "rightArrow" | "leftArrow" | "upArrow" | "downArrow" | "leftRightArrow" | "upDownArrow"
        | "notchedRightArrow" => {
            let horiz = matches!(
                prst,
                "rightArrow" | "leftArrow" | "leftRightArrow" | "notchedRightArrow"
            );
            let (len, thick) = if horiz { (w, h) } else { (h, w) };
            let shaft = thick * a("adj1", 50000.0);
            let both = matches!(prst, "leftRightArrow" | "upDownArrow");
            let head = (ss * a("adj2", 50000.0)).min(if both { len / 2.0 } else { len });
            let (y1, y2) = ((thick - shaft) / 2.0, (thick + shaft) / 2.0);
            let x2 = len - head;
            // 오른쪽 화살표를 기준으로 만들고 방향에 맞게 돌린다
            let mut base = vec![];
            if both {
                base.extend([(0.0, thick / 2.0), (head, 0.0), (head, y1)]);
            } else {
                base.push((0.0, y1));
            }
            base.extend([
                (x2, y1),
                (x2, 0.0),
                (len, thick / 2.0),
                (x2, thick),
                (x2, y2),
            ]);
            if both {
                base.extend([(head, y2), (head, thick)]);
            } else {
                base.push((0.0, y2));
                if prst == "notchedRightArrow" {
                    base.push((head * shaft / thick, thick / 2.0));
                }
            }
            let pts: Vec<(f64, f64)> = base
                .iter()
                .map(|&(x, y)| match prst {
                    "leftArrow" => (len - x, y),
                    "downArrow" | "upDownArrow" => (y, x),
                    "upArrow" => (y, len - x),
                    _ => (x, y),
                })
                .collect();
            one(poly(&pts))
        }
        "can" | "flowChartMagneticDisk" => {
            let ry = if prst == "can" {
                ss * a("adj", 25000.0) / 2.0
            } else {
                h / 6.0
            }
            .min(vc);
            Some(vec![
                PathSpec::with(
                    format!(
                        "M0,{ry}A{},{ry} 0 0 1 {},{ry}V{}A{},{ry} 0 0 1 0,{}Z",
                        f(hc),
                        f(w),
                        f(h - ry),
                        f(hc),
                        f(h - ry),
                        ry = f(ry)
                    ),
                    PathFill::Norm,
                    false,
                ),
                // 자기 디스크의 윗면은 밝게 칠하지 않는다 (한셀 화면 — 원기둥만)
                PathSpec::with(
                    ellipse(hc, ry, hc, ry),
                    if prst == "can" {
                        PathFill::Lighten
                    } else {
                        PathFill::Norm
                    },
                    false,
                ),
                PathSpec::with(
                    format!(
                        "M0,{ry}A{},{ry} 0 0 1 {},{ry}A{},{ry} 0 0 1 0,{ry}V{}A{},{ry} 0 0 0 {},{}V{ry}",
                        f(hc),
                        f(w),
                        f(hc),
                        f(h - ry),
                        f(hc),
                        f(w),
                        f(h - ry),
                        ry = f(ry)
                    ),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "cube" => {
            let d = ss * a("adj", 25000.0);
            Some(vec![
                PathSpec::with(
                    poly(&[(0.0, d), (w - d, d), (w - d, h), (0.0, h)]),
                    PathFill::Norm,
                    false,
                ),
                PathSpec::with(
                    poly(&[(0.0, d), (d, 0.0), (w, 0.0), (w - d, d)]),
                    PathFill::LightenLess,
                    false,
                ),
                PathSpec::with(
                    poly(&[(w - d, d), (w, 0.0), (w, h - d), (w - d, h)]),
                    PathFill::DarkenLess,
                    false,
                ),
                PathSpec::with(
                    format!(
                        "{}M0,{}L{},0M{},{}L{},0",
                        poly(&[
                            (0.0, d),
                            (d, 0.0),
                            (w, 0.0),
                            (w, h - d),
                            (w - d, h),
                            (0.0, h)
                        ]),
                        f(d),
                        f(d),
                        f(w - d),
                        f(d),
                        f(w)
                    ) + &format!("M0,{}H{}V{}", f(d), f(w - d), f(h)),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "bevel" => {
            let d = ss * a("adj", 12500.0);
            Some(vec![
                PathSpec::with(
                    poly(&[(d, d), (w - d, d), (w - d, h - d), (d, h - d)]),
                    PathFill::Norm,
                    false,
                ),
                PathSpec::with(
                    poly(&[(0.0, 0.0), (w, 0.0), (w - d, d), (d, d)]),
                    PathFill::LightenLess,
                    false,
                ),
                PathSpec::with(
                    poly(&[(0.0, 0.0), (d, d), (d, h - d), (0.0, h)]),
                    PathFill::Lighten,
                    false,
                ),
                PathSpec::with(
                    poly(&[(w, 0.0), (w, h), (w - d, h - d), (w - d, d)]),
                    PathFill::DarkenLess,
                    false,
                ),
                PathSpec::with(
                    poly(&[(0.0, h), (d, h - d), (w - d, h - d), (w, h)]),
                    PathFill::Darken,
                    false,
                ),
                PathSpec::with(
                    poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])
                        + &poly(&[(d, d), (w - d, d), (w - d, h - d), (d, h - d)])
                        + &format!(
                            "M0,0L{},{}M{},0L{},{}M0,{}L{},{}M{},{}L{},{}",
                            f(d),
                            f(d),
                            f(w),
                            f(w - d),
                            f(d),
                            f(h),
                            f(d),
                            f(h - d),
                            f(w),
                            f(h),
                            f(w - d),
                            f(h - d)
                        ),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "foldedCorner" => {
            let d = ss * a("adj", 16667.0);
            Some(vec![
                PathSpec::with(
                    poly(&[(0.0, 0.0), (w, 0.0), (w, h - d), (w - d, h), (0.0, h)]),
                    PathFill::Norm,
                    false,
                ),
                PathSpec::with(
                    format!(
                        "M{},{}L{},{}L{},{}Z",
                        f(w - d),
                        f(h),
                        f(w - d * 0.8),
                        f(h - d * 0.8),
                        f(w),
                        f(h - d)
                    ),
                    PathFill::DarkenLess,
                    false,
                ),
                PathSpec::with(
                    format!(
                        "M{},{}L{},{}L{},{}L{},{}L0,{}V0H{}V{}",
                        f(w - d),
                        f(h),
                        f(w - d * 0.8),
                        f(h - d * 0.8),
                        f(w),
                        f(h - d),
                        f(w - d),
                        f(h),
                        f(h),
                        f(w),
                        f(h - d)
                    ),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "frame" => {
            let d = ss * a("adj1", 12500.0);
            one(poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])
                + &poly(&[(d, d), (d, h - d), (w - d, h - d), (w - d, d)]))
        }
        "donut" => {
            let d = ss * a("adj", 25000.0);
            one(ellipse(hc, vc, hc, vc)
                + &format!(
                    "M{},{}A{},{} 0 1 0 {},{}A{},{} 0 1 0 {},{}Z",
                    f(d),
                    f(vc),
                    f(hc - d),
                    f(vc - d),
                    f(w - d),
                    f(vc),
                    f(hc - d),
                    f(vc - d),
                    f(d),
                    f(vc)
                ))
        }
        "noSmoking" => {
            let d = ss * a("adj", 18750.0);
            let (rx, ry) = (hc - d, vc - d);
            let k = std::f64::consts::FRAC_1_SQRT_2;
            one(ellipse(hc, vc, hc, vc)
                + &format!(
                    "M{},{}A{},{} 0 0 0 {},{}ZM{},{}A{},{} 0 0 0 {},{}Z",
                    f(hc - rx * k + d * 0.5),
                    f(vc - ry * k - d * 0.5),
                    f(rx),
                    f(ry),
                    f(hc + rx * k + d * 0.5),
                    f(vc + ry * k - d * 0.5),
                    f(hc + rx * k - d * 0.5),
                    f(vc + ry * k + d * 0.5),
                    f(rx),
                    f(ry),
                    f(hc - rx * k - d * 0.5),
                    f(vc - ry * k + d * 0.5)
                ))
        }
        "plaque" => {
            let r = ss * a("adj", 16667.0);
            one(format!(
                "M{r},0H{}A{r},{r} 0 0 0 {},{r}V{}A{r},{r} 0 0 0 {},{}H{r}A{r},{r} 0 0 0 0,{}V{r}A{r},{r} 0 0 0 {r},0Z",
                f(w - r),
                f(w),
                f(h - r),
                f(w - r),
                f(h),
                f(h - r),
                r = f(r)
            ))
        }
        "teardrop" => {
            let k = a("adj", 100000.0);
            one(format!(
                "M0,{}A{},{} 0 0 1 {},0L{},{}A{},{} 0 1 1 0,{}Z",
                f(vc),
                f(hc),
                f(vc),
                f(hc),
                f(hc + hc * k),
                f(vc - vc * k),
                f(hc),
                f(vc),
                f(vc)
            ))
        }
        "heart" => one(format!(
            "M{},{}C{},{} {},{} {},{}C{},{} {},{} {},{}C{},{} {},{} {},{}C{},{} {},{} {},{}Z",
            f(hc),
            f(h * 0.25),
            f(hc + w * 0.15),
            f(-h * 0.1),
            f(w * 1.05),
            f(h * 0.05),
            f(w * 0.95),
            f(h * 0.4),
            f(w * 0.9),
            f(h * 0.6),
            f(hc + w * 0.2),
            f(h * 0.75),
            f(hc),
            f(h),
            f(hc - w * 0.2),
            f(h * 0.75),
            f(w * 0.1),
            f(h * 0.6),
            f(w * 0.05),
            f(h * 0.4),
            f(-w * 0.05),
            f(h * 0.05),
            f(hc - w * 0.15),
            f(-h * 0.1),
            f(hc),
            f(h * 0.25)
        )),
        "smileyFace" => {
            let k = a("adj", 4653.0);
            let smile_y = h * 0.75 + h * k;
            Some(vec![
                PathSpec::new(ellipse(hc, vc, hc, vc)),
                PathSpec::with(
                    ellipse(w * 0.35, h * 0.37, w * 0.05, h * 0.05)
                        + &ellipse(w * 0.65, h * 0.37, w * 0.05, h * 0.05),
                    PathFill::DarkenLess,
                    true,
                ),
                PathSpec::with(
                    format!(
                        "M{},{}Q{},{} {},{}",
                        f(w * 0.25),
                        f(h * 0.68),
                        f(hc),
                        f(smile_y + h * 0.08),
                        f(w * 0.75),
                        f(h * 0.68)
                    ),
                    PathFill::None,
                    true,
                ),
            ])
        }
        "lightningBolt" => {
            const P: [(f64, f64); 11] = [
                (8458.0, 0.0),
                (12960.0, 6176.0),
                (11053.0, 6862.0),
                (16564.0, 11853.0),
                (14353.0, 12825.0),
                (21600.0, 21600.0),
                (10533.0, 14290.0),
                (12290.0, 13563.0),
                (5333.0, 8508.0),
                (7305.0, 7811.0),
                (0.0, 3923.0),
            ];
            let pts: Vec<(f64, f64)> = P
                .iter()
                .map(|&(x, y)| (x / 21600.0 * w, y / 21600.0 * h))
                .collect();
            one(poly(&pts))
        }
        "moon" => {
            let k = a("adj", 50000.0);
            one(format!(
                "M{},0A{},{} 0 0 0 {},{}A{},{} 0 0 1 {},0Z",
                f(w),
                f(w),
                f(vc),
                f(w),
                f(h),
                f(w * (1.0 - k)),
                f(vc),
                f(w)
            ))
        }
        "pie" | "chord" | "arc" | "blockArc" => {
            let (st, en) = match prst {
                "arc" => (a("adj1", 16_200_000.0), a("adj2", 0.0)),
                "blockArc" => (a("adj1", 10_800_000.0), a("adj2", 0.0)),
                "chord" => (a("adj1", 2_700_000.0), a("adj2", 16_200_000.0)),
                _ => (a("adj1", 0.0), a("adj2", 16_200_000.0)),
            };
            // 각도 (1/60000 도 → 도). 조정값 a() 는 1/100000 로 나눴으므로 되돌린다
            let (st, en) = (st * 100_000.0 / 60_000.0, en * 100_000.0 / 60_000.0);
            let mut sw = (en - st).rem_euclid(360.0);
            if sw == 0.0 {
                sw = 360.0;
            }
            let pt = |deg: f64, rx: f64, ry: f64| {
                let t = ellipse_param(rx, ry, deg);
                (hc + rx * t.cos(), vc + ry * t.sin())
            };
            let (p0, p1) = (pt(st, hc, vc), pt(st + sw, hc, vc));
            let large = if sw > 180.0 { 1 } else { 0 };
            let arc = format!("A{},{} 0 {large} 1 {},{}", f(hc), f(vc), f(p1.0), f(p1.1));
            match prst {
                "arc" => Some(vec![PathSpec::with(
                    format!("M{},{}{arc}", f(p0.0), f(p0.1)),
                    PathFill::None,
                    true,
                )]),
                "chord" => one(format!("M{},{}{arc}Z", f(p0.0), f(p0.1))),
                "blockArc" => {
                    let d = ss * a("adj3", 25000.0);
                    let (rx, ry) = (hc - d, vc - d);
                    let (q1, q0) = (pt(st + sw, rx, ry), pt(st, rx, ry));
                    one(format!(
                        "M{},{}{arc}L{},{}A{},{} 0 {large} 0 {},{}Z",
                        f(p0.0),
                        f(p0.1),
                        f(q1.0),
                        f(q1.1),
                        f(rx),
                        f(ry),
                        f(q0.0),
                        f(q0.1)
                    ))
                }
                _ => one(format!(
                    "M{},{}L{},{}{arc}Z",
                    f(hc),
                    f(vc),
                    f(p0.0),
                    f(p0.1)
                )),
            }
        }
        "nonIsoscelesTrapezoid" => {
            let (x2, dx3) = (ss * a("adj1", 25000.0), ss * a("adj2", 25000.0));
            one(poly(&[(0.0, h), (x2, 0.0), (w - dx3, 0.0), (w, h)]))
        }
        "diagStripe" => {
            let k = a("adj", 50000.0);
            one(poly(&[(0.0, h * k), (w * k, 0.0), (w, 0.0), (0.0, h)]))
        }
        "snip2DiagRect" => {
            let (l, r) = (ss * a("adj1", 0.0), ss * a("adj2", 16667.0));
            one(poly(&[
                (l, 0.0),
                (w - r, 0.0),
                (w, r),
                (w, h - l),
                (w - l, h),
                (r, h),
                (0.0, h - r),
                (0.0, l),
            ]))
        }
        "halfFrame" => {
            let x1 = ss * a("adj2", 33333.0).clamp(0.0, w / ss);
            let g2 = h - h * x1 / w;
            let y1 = ss * a("adj1", 33333.0).clamp(0.0, g2 / ss);
            let (x2, y2) = (w - y1 * w / h, h - x1 * h / w);
            one(poly(&[
                (0.0, 0.0),
                (w, 0.0),
                (x2, y1),
                (x1, y1),
                (x1, y2),
                (0.0, h),
            ]))
        }
        "flowChartOfflineStorage" => Some(vec![
            PathSpec::new(poly(&[(0.0, 0.0), (w, 0.0), (hc, h)])),
            PathSpec::with(
                format!("M{},{}H{}", f(w * 0.4), f(h * 0.8), f(w * 0.6)),
                PathFill::None,
                true,
            ),
        ]),
        // 동작 단추: 사각형 + 어둡게 칠한 그림 (빈 단추·앞·뒤·처음·끝 — 도형 모양 실험 4번 시트)
        "actionButtonBlank" => one(poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])),
        "actionButtonBackPrevious"
        | "actionButtonForwardNext"
        | "actionButtonBeginning"
        | "actionButtonEnd" => {
            let dx2 = ss * 3.0 / 8.0;
            let (g9, g10, g11, g12) = (vc - dx2, vc + dx2, hc - dx2, hc + dx2);
            let g13 = ss * 3.0 / 4.0;
            let mut icon = Vec::new();
            match prst {
                "actionButtonBackPrevious" => icon.push(poly(&[(g11, vc), (g12, g9), (g12, g10)])),
                "actionButtonForwardNext" => icon.push(poly(&[(g12, vc), (g11, g9), (g11, g10)])),
                "actionButtonBeginning" => {
                    let (g16, g17) = (g11 + g13 / 8.0, g11 + g13 / 4.0);
                    icon.push(poly(&[(g17, vc), (g12, g9), (g12, g10)]));
                    icon.push(poly(&[(g11, g9), (g16, g9), (g16, g10), (g11, g10)]));
                }
                _ => {
                    let (g16, g17) = (g11 + g13 * 3.0 / 4.0, g11 + g13 * 7.0 / 8.0);
                    icon.push(poly(&[(g16, vc), (g11, g9), (g11, g10)]));
                    icon.push(poly(&[(g17, g9), (g12, g9), (g12, g10), (g17, g10)]));
                }
            }
            let mut v = vec![PathSpec::new(poly(&[
                (0.0, 0.0),
                (w, 0.0),
                (w, h),
                (0.0, h),
            ]))];
            v.extend(
                icon.into_iter()
                    .map(|d| PathSpec::with(d, PathFill::Darken, true)),
            );
            Some(v)
        }
        // 선 설명선: 사각형 + 꺾인 지시선 (adj 홀수 = y, 짝수 = x — 상자 기준 비율). border 는 사각형 테두리,
        // accent 는 첫 점 x 의 세로 강조선 (도형 모양 실험)
        "callout1"
        | "callout2"
        | "callout3"
        | "borderCallout1"
        | "borderCallout2"
        | "borderCallout3"
        | "accentCallout1"
        | "accentCallout2"
        | "accentCallout3"
        | "accentBorderCallout1"
        | "accentBorderCallout2"
        | "accentBorderCallout3" => {
            let def: &[f64] = match prst.as_bytes().last() {
                Some(b'1') => &[18750.0, -8333.0, 112500.0, -38333.0],
                Some(b'2') => &[18750.0, -8333.0, 18750.0, -16667.0, 112500.0, -46667.0],
                _ => &[
                    18750.0, -8333.0, 18750.0, -16667.0, 100000.0, -16667.0, 112963.0, -8333.0,
                ],
            };
            let pts: Vec<(f64, f64)> = def
                .chunks(2)
                .enumerate()
                .map(|(k, d)| {
                    let y = h * a(&format!("adj{}", 2 * k + 1), d[0]);
                    let x = w * a(&format!("adj{}", 2 * k + 2), d[1]);
                    (x, y)
                })
                .collect();
            let border = prst.contains("order");
            let mut v = vec![PathSpec::with(
                poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]),
                PathFill::Norm,
                border,
            )];
            if prst.starts_with("accent") {
                v.push(PathSpec::with(
                    format!("M{},0V{}", f(pts[0].0), f(h)),
                    PathFill::None,
                    true,
                ));
            }
            let mut d = String::new();
            for (i, (x, y)) in pts.iter().enumerate() {
                let _ = write!(d, "{}{},{}", if i == 0 { 'M' } else { 'L' }, f(*x), f(*y));
            }
            v.push(PathSpec::with(d, PathFill::None, true));
            Some(v)
        }
        "wedgeRectCallout" | "wedgeRoundRectCallout" | "wedgeEllipseCallout" => {
            let tx = hc + w * a("adj1", -20833.0);
            let ty = vc + h * a("adj2", 62500.0);
            if prst == "wedgeEllipseCallout" {
                // 꼬리 밑변: 끝을 향한 방향의 양쪽 ±10도
                let ang = (ty - vc).atan2(tx - hc);
                let p = |da: f64| (hc + hc * (ang + da).cos(), vc + vc * (ang + da).sin());
                let (b0, b1) = (p(-0.17), p(0.17));
                return one(format!(
                    "M{},{}L{},{}L{},{}A{},{} 0 1 1 {},{}Z",
                    f(b0.0),
                    f(b0.1),
                    f(tx),
                    f(ty),
                    f(b1.0),
                    f(b1.1),
                    f(hc),
                    f(vc),
                    f(b0.0),
                    f(b0.1)
                ));
            }
            let r = if prst == "wedgeRoundRectCallout" {
                ss * a("adj3", 16667.0)
            } else {
                0.0
            };
            // 꼬리가 나가는 변: 끝이 칸 밖으로 더 멀리 나간 쪽
            let (dx, dy) = ((tx - hc) / w, (ty - vc) / h);
            let inside = tx >= 0.0 && tx <= w && ty >= 0.0 && ty <= h;
            if inside {
                return one(round_rect(0.0, 0.0, w, h, [r; 4]));
            }
            let mut pts: Vec<(f64, f64)> = vec![(0.0, 0.0)];
            if dy < 0.0 && dy.abs() >= dx.abs() {
                pts.extend([(w * 5.0 / 12.0, 0.0), (tx, ty), (w * 7.0 / 12.0, 0.0)]);
            }
            pts.push((w, 0.0));
            if dx > 0.0 && dx.abs() > dy.abs() {
                pts.extend([(w, h * 5.0 / 12.0), (tx, ty), (w, h * 7.0 / 12.0)]);
            }
            pts.push((w, h));
            if dy > 0.0 && dy.abs() >= dx.abs() {
                pts.extend([(w * 7.0 / 12.0, h), (tx, ty), (w * 5.0 / 12.0, h)]);
            }
            pts.push((0.0, h));
            if dx < 0.0 && dx.abs() > dy.abs() {
                pts.extend([(0.0, h * 7.0 / 12.0), (tx, ty), (0.0, h * 5.0 / 12.0)]);
            }
            one(poly(&pts))
        }
        "wave" | "doubleWave" => {
            let amp = h * a("adj1", if prst == "wave" { 12500.0 } else { 6250.0 });
            let n = if prst == "wave" { 1.0 } else { 2.0 };
            let seg = w / (2.0 * n);
            let mut d = format!("M0,{}", f(amp));
            let mut x = 0.0;
            let mut up = true;
            while x < w - 0.01 {
                let _ = write!(
                    d,
                    "Q{},{} {},{}",
                    f(x + seg / 2.0),
                    f(if up { amp * 3.0 } else { -amp }),
                    f(x + seg),
                    f(amp)
                );
                x += seg;
                up = !up;
            }
            let _ = write!(d, "L{},{}", f(w), f(h - amp));
            let mut x = w;
            let mut up = n == 1.0;
            while x > 0.01 {
                let _ = write!(
                    d,
                    "Q{},{} {},{}",
                    f(x - seg / 2.0),
                    f(if up { h - amp * 3.0 } else { h + amp }),
                    f(x - seg),
                    f(h - amp)
                );
                x -= seg;
                up = !up;
            }
            d.push('Z');
            one(d)
        }
        "leftBracket" | "rightBracket" | "leftBrace" | "rightBrace" | "bracketPair"
        | "bracePair" => {
            let r = ss * a("adj", 8333.0);
            let lb = format!(
                "M{},0A{},{} 0 0 0 0,{}V{}A{},{} 0 0 0 {},{}",
                f(w.min(r * 2.0)),
                f(r),
                f(r),
                f(r),
                f(h - r),
                f(r),
                f(r),
                f(w.min(r * 2.0)),
                f(h)
            );
            let d = match prst {
                "leftBracket" => format!(
                    "M{},0A{},{} 0 0 0 0,{}V{}A{},{} 0 0 0 {},{}",
                    f(w),
                    f(w),
                    f(r),
                    f(r),
                    f(h - r),
                    f(w),
                    f(r),
                    f(w),
                    f(h)
                ),
                "rightBracket" => format!(
                    "M0,0A{},{} 0 0 1 {},{}V{}A{},{} 0 0 1 0,{}",
                    f(w),
                    f(r),
                    f(w),
                    f(r),
                    f(h - r),
                    f(w),
                    f(r),
                    f(h)
                ),
                "leftBrace" => format!(
                    "M{},0Q{},0 {},{}V{}Q{},{} 0,{}Q{},{} {},{}V{}Q{},{} {},{}",
                    f(w),
                    f(hc),
                    f(hc),
                    f(r),
                    f(vc - r),
                    f(hc),
                    f(vc),
                    f(vc),
                    f(hc),
                    f(vc),
                    f(hc),
                    f(vc + r),
                    f(h - r),
                    f(hc),
                    f(h),
                    f(w),
                    f(h)
                ),
                "rightBrace" => format!(
                    "M0,0Q{},0 {},{}V{}Q{},{} {},{}Q{},{} {},{}V{}Q{},{} 0,{}",
                    f(hc),
                    f(hc),
                    f(r),
                    f(vc - r),
                    f(hc),
                    f(vc),
                    f(w),
                    f(vc),
                    f(hc),
                    f(vc),
                    f(hc),
                    f(vc + r),
                    f(h - r),
                    f(hc),
                    f(h),
                    f(h)
                ),
                "bracketPair" => format!(
                    "{lb}M{},0A{},{} 0 0 1 {},{}V{}A{},{} 0 0 1 {},{}",
                    f(w - (r * 2.0).min(w)),
                    f(r),
                    f(r),
                    f(w),
                    f(r),
                    f(h - r),
                    f(r),
                    f(r),
                    f(w - (r * 2.0).min(w)),
                    f(h)
                ),
                _ => format!(
                    "M{},0Q0,0 0,{}V{}Q0,{} {},{}M{},0Q{},0 {},{}V{}Q{},{} {},{}",
                    f(r * 2.0),
                    f(r * 2.0),
                    f(h - r * 2.0),
                    f(h),
                    f(r * 2.0),
                    f(h),
                    f(w - r * 2.0),
                    f(w),
                    f(w),
                    f(r * 2.0),
                    f(h - r * 2.0),
                    f(w),
                    f(h),
                    f(w - r * 2.0),
                    f(h)
                ),
            };
            Some(vec![PathSpec::with(d, PathFill::None, true)])
        }
        "line" | "straightConnector1" => Some(vec![PathSpec::with(
            format!("M0,0L{},{}", f(w), f(h)),
            PathFill::None,
            true,
        )]),
        "bentConnector2" => Some(vec![PathSpec::with(
            open(&[(0.0, 0.0), (w, 0.0), (w, h)]),
            PathFill::None,
            true,
        )]),
        "bentConnector3" => {
            let x = w * a("adj1", 50000.0);
            Some(vec![PathSpec::with(
                open(&[(0.0, 0.0), (x, 0.0), (x, h), (w, h)]),
                PathFill::None,
                true,
            )])
        }
        "curvedConnector3" => {
            let x = w * a("adj1", 50000.0);
            Some(vec![PathSpec::with(
                format!(
                    "M0,0C{},0 {},{} {},{}S{},{} {},{}",
                    f(x / 2.0),
                    f(x),
                    f(h / 4.0),
                    f(x),
                    f(vc),
                    f(x + (w - x) / 2.0),
                    f(h),
                    f(w),
                    f(h)
                ),
                PathFill::None,
                true,
            )])
        }
        "cloud" | "cloudCallout" => one(ellipse(hc, vc, hc, vc)),
        _ => None,
    }
}

/// 미리 정의된 모양 → SVG 경로 하나 (경로들을 이어 붙인다). 모르는 모양은 None
pub fn preset_path(prst: &str, w: f64, h: f64, adj: &HashMap<String, f64>) -> Option<String> {
    preset_paths(prst, w, h, adj).map(|v| v.into_iter().map(|p| p.d).collect())
}

/// 도형 수식 계산 (ECMA-376 1부 20.1.9 `a:gd` 의 fmla): 도형 크기 w·h 와 미리 정해진 이름(hc, ss, cd4 …),
/// 조정값(avLst), 앞서 정한 수식(gdLst)을 쓴다. 각도는 1/60000 도
pub(super) struct Guides {
    vals: HashMap<String, f64>,
    w: f64,
    h: f64,
}

impl Guides {
    pub fn new(w: f64, h: f64) -> Guides {
        Guides {
            vals: HashMap::new(),
            w,
            h,
        }
    }

    /// 수식 목록(avLst 또는 gdLst)을 차례로 정한다
    pub fn define_list(&mut self, list: Option<&Element>) {
        let Some(list) = list else { return };
        for g in list.elements().filter(|g| g.is(A, "gd")) {
            if let (Some(n), Some(fm)) = (g.attr("name"), g.attr("fmla")) {
                let v = self.eval(fm);
                self.vals.insert(n.to_string(), v);
            }
        }
    }

    fn builtin(&self, name: &str) -> Option<f64> {
        let (w, h) = (self.w, self.h);
        let ss = w.min(h);
        Some(match name {
            "w" | "r" => w,
            "h" | "b" => h,
            "l" | "t" => 0.0,
            "hc" => w / 2.0,
            "vc" => h / 2.0,
            "ss" => ss,
            "ls" => w.max(h),
            "cd2" => 10_800_000.0,
            "cd4" => 5_400_000.0,
            "cd8" => 2_700_000.0,
            "3cd4" => 16_200_000.0,
            "3cd8" => 8_100_000.0,
            "5cd8" => 13_500_000.0,
            "7cd8" => 18_900_000.0,
            _ => {
                // wd2 … wd32, hd2 …, ssd2 … (나누기)
                let (base, n) = if let Some(n) = name.strip_prefix("ssd") {
                    (ss, n)
                } else if let Some(n) = name.strip_prefix("wd") {
                    (w, n)
                } else {
                    let n = name.strip_prefix("hd")?;
                    (h, n)
                };
                base / n.parse::<f64>().ok().filter(|d| *d > 0.0)?
            }
        })
    }

    /// 값 하나: 숫자, 정한 수식, 미리 정해진 이름
    pub fn get(&self, tok: &str) -> f64 {
        let t = tok.trim();
        if let Ok(v) = t.parse::<f64>() {
            return v;
        }
        self.vals
            .get(t)
            .copied()
            .or_else(|| self.builtin(t))
            .unwrap_or(0.0)
    }

    pub fn eval(&self, fmla: &str) -> f64 {
        let p: Vec<&str> = fmla.split_whitespace().collect();
        let a = |i: usize| p.get(i).map_or(0.0, |t| self.get(t));
        let ang = |v: f64| (v / 60000.0).to_radians();
        match p.first().copied().unwrap_or("") {
            "val" => a(1),
            "*/" => {
                let z = a(3);
                if z == 0.0 { 0.0 } else { a(1) * a(2) / z }
            }
            "+-" => a(1) + a(2) - a(3),
            "+/" => {
                let z = a(3);
                if z == 0.0 { 0.0 } else { (a(1) + a(2)) / z }
            }
            "?:" => {
                if a(1) > 0.0 {
                    a(2)
                } else {
                    a(3)
                }
            }
            "abs" => a(1).abs(),
            "at2" => a(2).atan2(a(1)).to_degrees() * 60000.0,
            "cat2" => a(1) * a(3).atan2(a(2)).cos(),
            "sat2" => a(1) * a(3).atan2(a(2)).sin(),
            "cos" => a(1) * ang(a(2)).cos(),
            "sin" => a(1) * ang(a(2)).sin(),
            "tan" => a(1) * ang(a(2)).tan(),
            "max" => a(1).max(a(2)),
            "min" => a(1).min(a(2)),
            "mod" => (a(1) * a(1) + a(2) * a(2) + a(3) * a(3)).sqrt(),
            "pin" => {
                let (lo, v, hi) = (a(1), a(2), a(3));
                if v < lo {
                    lo
                } else if v > hi {
                    hi
                } else {
                    v
                }
            }
            "sqrt" => a(1).max(0.0).sqrt(),
            _ => 0.0,
        }
    }
}

/// 사용자 정의 모양의 경로들 → (SVG 경로, 채움, 선). w×h 는 그릴 크기(px) — 수식은 도형 좌표(EMU)로,
/// 경로 점은 경로의 w·h 좌표(없으면 도형 좌표)로 계산해 그릴 크기로 늘인다
pub fn custom_paths(geom: &Element, w: f64, h: f64) -> Vec<PathSpec> {
    let emu = crate::units::EMU_PER_PX as f64;
    let mut gd = Guides::new(w * emu, h * emu);
    gd.define_list(geom.child_ns(A, "avLst"));
    gd.define_list(geom.child_ns(A, "gdLst"));
    let Some(list) = geom.child_ns(A, "pathLst") else {
        return Vec::new();
    };
    paths_of(list, &gd, w, h, w * emu, h * emu)
}

/// 경로 목록(a:pathLst) → SVG. 경로에 w·h 가 없으면 (dw, dh) 좌표계
pub(super) fn paths_of(
    list: &Element,
    gd: &Guides,
    w: f64,
    h: f64,
    dw: f64,
    dh: f64,
) -> Vec<PathSpec> {
    let mut out = Vec::new();
    for p in list.elements().filter(|p| p.is(A, "path")) {
        let pw = p.attr_f64("w").filter(|v| *v > 0.0).unwrap_or(dw);
        let ph = p.attr_f64("h").filter(|v| *v > 0.0).unwrap_or(dh);
        let (sx, sy) = (w / pw, h / ph);
        let pt = |e: &Element| -> (f64, f64) {
            (
                gd.get(e.attr("x").unwrap_or("0")) * sx,
                gd.get(e.attr("y").unwrap_or("0")) * sy,
            )
        };
        let mut d = String::new();
        let mut cur = (0.0f64, 0.0f64);
        let mut start = cur;
        for c in p.elements() {
            let pts: Vec<(f64, f64)> = c.elements().filter(|e| e.is(A, "pt")).map(pt).collect();
            match c.name.as_str() {
                "moveTo" if !pts.is_empty() => {
                    cur = pts[0];
                    start = cur;
                    let _ = write!(d, "M{},{}", f(cur.0), f(cur.1));
                }
                "lnTo" if !pts.is_empty() => {
                    cur = pts[0];
                    let _ = write!(d, "L{},{}", f(cur.0), f(cur.1));
                }
                "cubicBezTo" if pts.len() == 3 => {
                    cur = pts[2];
                    let _ = write!(
                        d,
                        "C{},{} {},{} {},{}",
                        f(pts[0].0),
                        f(pts[0].1),
                        f(pts[1].0),
                        f(pts[1].1),
                        f(cur.0),
                        f(cur.1)
                    );
                }
                "quadBezTo" if pts.len() == 2 => {
                    cur = pts[1];
                    let _ = write!(
                        d,
                        "Q{},{} {},{}",
                        f(pts[0].0),
                        f(pts[0].1),
                        f(cur.0),
                        f(cur.1)
                    );
                }
                "arcTo" => {
                    // 현재 점에서 시작하는 타원 호: wR·hR 반지름, stAng 시작각, swAng 휘는 각 (1/60000 도)
                    let wr = gd.get(c.attr("wR").unwrap_or("0")) * sx;
                    let hr = gd.get(c.attr("hR").unwrap_or("0")) * sy;
                    let st = gd.get(c.attr("stAng").unwrap_or("0")) / 60000.0;
                    let sw = gd.get(c.attr("swAng").unwrap_or("0")) / 60000.0;
                    d.push_str(&arc_svg(cur, wr, hr, st, sw));
                    cur = arc_end(cur, wr, hr, st, sw);
                }
                "close" => {
                    d.push('Z');
                    cur = start;
                }
                _ => {}
            }
        }
        if d.is_empty() {
            continue;
        }
        out.push(PathSpec::with(
            d,
            PathFill::parse(p.attr("fill")),
            p.attr_bool("stroke").unwrap_or(true),
        ));
    }
    out
}

/// 타원 위의 각도: DrawingML 의 각도는 실제 방향이다 — 점 = 중심 + (wr·cos t, hr·sin t) 의 t 로 바꾼다
fn ellipse_param(wr: f64, hr: f64, deg: f64) -> f64 {
    let a = deg.to_radians();
    (wr * a.sin()).atan2(hr * a.cos())
}

/// 호의 끝점 (현재 점이 시작각 위에 있다)
fn arc_end(cur: (f64, f64), wr: f64, hr: f64, st: f64, sw: f64) -> (f64, f64) {
    let t0 = ellipse_param(wr, hr, st);
    let t1 = ellipse_param(wr, hr, st + sw);
    let (cx, cy) = (cur.0 - wr * t0.cos(), cur.1 - hr * t0.sin());
    (cx + wr * t1.cos(), cy + hr * t1.sin())
}

/// 호 하나를 SVG 로 (거의 한 바퀴면 반씩 나눈다)
fn arc_svg(cur: (f64, f64), wr: f64, hr: f64, st: f64, sw: f64) -> String {
    if sw.abs() >= 359.9 {
        let half = sw / 2.0;
        let mid = arc_end(cur, wr, hr, st, half);
        return arc_svg(cur, wr, hr, st, half) + &arc_svg(mid, wr, hr, st + half, half);
    }
    let end = arc_end(cur, wr, hr, st, sw);
    // 각도 방향의 큰 호 판단은 매개변수 각도로
    let dt = ellipse_param(wr, hr, st + sw) - ellipse_param(wr, hr, st);
    let dt = if sw > 0.0 {
        dt.rem_euclid(std::f64::consts::TAU)
    } else {
        -(-dt).rem_euclid(std::f64::consts::TAU)
    };
    let large = if dt.abs() > std::f64::consts::PI {
        1
    } else {
        0
    };
    let sweep = if sw > 0.0 { 1 } else { 0 };
    format!(
        "A{},{} 0 {large} {sweep} {},{}",
        f(wr.abs()),
        f(hr.abs()),
        f(end.0),
        f(end.1)
    )
}

/// 모양 요소 (prstGeom/custGeom) 와 조정값
pub fn geometry(obj: &Element) -> Option<&Element> {
    let p = sp_pr(obj)?;
    p.child_ns(A, "custGeom")
        .or_else(|| p.child_ns(A, "prstGeom"))
}

/// 도형의 경로들 (w×h px). 원본이 없거나 모르는 모양이면 None
pub fn paths(obj: &Element, w: f64, h: f64) -> Option<Vec<PathSpec>> {
    let g = geometry(obj)?;
    if g.is(A, "custGeom") {
        let p = custom_paths(g, w, h);
        return (!p.is_empty()).then_some(p);
    }
    let prst = g.attr("prst")?;
    preset_paths(prst, w, h, &adjusts(g))
}

/// `xdr:style` 의 채우기·선 참조 색 (idx 0 = 없음)
pub fn style_ref<'a>(obj: &'a Element, name: &str) -> Option<&'a Element> {
    let st = obj.child_ns(XDR, "style")?;
    let r = st.child_ns(A, name)?;
    if r.attr_u32("idx") == Some(0) {
        return None;
    }
    r.elements().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_geometry_triangle() {
        let xml = r#"<a:custGeom xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:avLst/><a:gdLst/><a:rect l="0" t="0" r="r" b="b"/><a:pathLst><a:path w="100" h="50"><a:moveTo><a:pt x="0" y="50"/></a:moveTo><a:lnTo><a:pt x="50" y="0"/></a:lnTo><a:lnTo><a:pt x="100" y="50"/></a:lnTo><a:close/></a:path></a:pathLst></a:custGeom>"#;
        let g = crate::xml::parse(xml, "t").unwrap();
        let p = custom_paths(&g, 200.0, 100.0);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].d, "M0,100L100,0L200,100Z");
        assert!(p[0].fill == PathFill::Norm && p[0].stroke);
    }

    #[test]
    fn presets() {
        let none = HashMap::new();
        assert_eq!(
            preset_path("diamond", 100.0, 50.0, &none).unwrap(),
            "M50,0L100,25L50,50L0,25Z"
        );
        assert!(
            preset_path("rightArrow", 100.0, 40.0, &none)
                .unwrap()
                .starts_with("M0,10L80,10")
        );
        assert!(preset_path("unknownShape", 10.0, 10.0, &none).is_none());
        // 별 5개 꼭짓점: 위 꼭짓점에서 시작
        assert!(
            preset_path("star5", 100.0, 100.0, &none)
                .unwrap()
                .starts_with("M50,0")
        );
        assert_eq!(preset_paths("cube", 100.0, 100.0, &none).unwrap().len(), 4);
    }
}
