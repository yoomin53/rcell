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
                && matches!(c.name.as_str(), "sp" | "cxnSp" | "pic" | "graphicFrame" | "grpSp")
        })
        .cloned()
}

/// spPr (또는 grpSpPr)
pub fn sp_pr(obj: &Element) -> Option<&Element> {
    obj.child_ns(XDR, "spPr").or_else(|| obj.child_ns(XDR, "grpSpPr"))
}

/// `a:avLst` 의 조정값 (`val n` 만)
fn adjusts(geom: &Element) -> HashMap<String, f64> {
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

/// 미리 정의된 모양 → SVG 경로 (w×h px 상자). 모르는 모양은 None (사각형으로 그린다)
pub fn preset_path(prst: &str, w: f64, h: f64, adj: &HashMap<String, f64>) -> Option<String> {
    let ss = w.min(h);
    let a = |n: &str, d: f64| adj.get(n).copied().unwrap_or(d) / 100_000.0;
    let poly = |pts: &[(f64, f64)]| -> String {
        let mut s = String::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            let _ = write!(s, "{}{},{}", if i == 0 { "M" } else { "L" }, f(*x), f(*y));
        }
        s.push('Z');
        s
    };
    Some(match prst {
        "rect" | "flowChartProcess" | "flowChartPredefinedProcess" | "flowChartInternalStorage" => {
            poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)])
        }
        "roundRect" | "flowChartAlternateProcess" | "flowChartTerminator" => {
            let r = if prst == "flowChartTerminator" {
                h / 2.0
            } else {
                ss * a("adj", 16667.0)
            }
            .min(w / 2.0)
            .min(h / 2.0);
            format!(
                "M{r},0H{}A{r},{r} 0 0 1 {},{r}V{}A{r},{r} 0 0 1 {},{}H{r}A{r},{r} 0 0 1 0,{}V{r}A{r},{r} 0 0 1 {r},0Z",
                f(w - r),
                f(w),
                f(h - r),
                f(w - r),
                f(h),
                f(h - r),
                r = f(r)
            )
        }
        "ellipse" | "flowChartConnector" => format!(
            "M0,{}A{},{} 0 1 1 {},{}A{},{} 0 1 1 0,{}Z",
            f(h / 2.0),
            f(w / 2.0),
            f(h / 2.0),
            f(w),
            f(h / 2.0),
            f(w / 2.0),
            f(h / 2.0),
            f(h / 2.0)
        ),
        "triangle" | "flowChartExtract" => {
            poly(&[(0.0, h), (w * a("adj", 50000.0), 0.0), (w, h)])
        }
        "flowChartMerge" => poly(&[(0.0, 0.0), (w, 0.0), (w / 2.0, h)]),
        "rtTriangle" => poly(&[(0.0, 0.0), (0.0, h), (w, h)]),
        "diamond" | "flowChartDecision" => {
            poly(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)])
        }
        "parallelogram" | "flowChartInputOutput" => {
            let x = if prst == "parallelogram" { ss * a("adj", 25000.0) } else { w / 5.0 };
            poly(&[(x, 0.0), (w, 0.0), (w - x, h), (0.0, h)])
        }
        "trapezoid" | "flowChartManualOperation" => {
            let x = if prst == "trapezoid" { ss * a("adj", 25000.0) } else { w / 5.0 };
            if prst == "trapezoid" {
                poly(&[(0.0, h), (x, 0.0), (w - x, 0.0), (w, h)])
            } else {
                poly(&[(0.0, 0.0), (w, 0.0), (w - x, h), (x, h)])
            }
        }
        "pentagon" | "homePlate" if prst == "homePlate" => {
            let x = (ss * a("adj", 50000.0)).min(w);
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h)])
        }
        "pentagon" => poly(&[
            (w / 2.0, 0.0),
            (w, h * 0.3820),
            (w * 0.8090, h),
            (w * 0.1910, h),
            (0.0, h * 0.3820),
        ]),
        "chevron" => {
            let x = (ss * a("adj", 50000.0)).min(w);
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h), (x, h / 2.0)])
        }
        "hexagon" | "flowChartPreparation" => {
            let x = if prst == "hexagon" { ss * a("adj", 25000.0) } else { w / 5.0 };
            poly(&[(0.0, h / 2.0), (x, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (x, h)])
        }
        "octagon" => {
            let x = ss * a("adj", 29289.0);
            poly(&[
                (x, 0.0),
                (w - x, 0.0),
                (w, x),
                (w, h - x),
                (w - x, h),
                (x, h),
                (0.0, h - x),
                (0.0, x),
            ])
        }
        "plus" | "flowChartOr" if prst == "plus" => {
            let x = ss * a("adj", 25000.0);
            poly(&[
                (x, 0.0),
                (w - x, 0.0),
                (w - x, x),
                (w, x),
                (w, h - x),
                (w - x, h - x),
                (w - x, h),
                (x, h),
                (x, h - x),
                (0.0, h - x),
                (0.0, x),
                (x, x),
            ])
        }
        "star5" => {
            let r_in = a("adj", 19098.0) / 0.5;
            let mut pts = Vec::new();
            for i in 0..10 {
                let ang = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                let r = if i % 2 == 0 { 1.0 } else { r_in };
                pts.push((w / 2.0 + w / 2.0 * r * ang.cos(), h / 2.0 + h / 2.0 * r * ang.sin()));
            }
            poly(&pts)
        }
        "rightArrow" | "leftArrow" | "upArrow" | "downArrow" => {
            let (len, thick) = if matches!(prst, "rightArrow" | "leftArrow") { (w, h) } else { (h, w) };
            let shaft = thick * a("adj1", 50000.0);
            let head = (ss * a("adj2", 50000.0)).min(len);
            let (y1, y2) = ((thick - shaft) / 2.0, (thick + shaft) / 2.0);
            let x2 = len - head;
            // 오른쪽 화살표를 기준으로 만들고 방향에 맞게 돌린다
            let base = [
                (0.0, y1),
                (x2, y1),
                (x2, 0.0),
                (len, thick / 2.0),
                (x2, thick),
                (x2, y2),
                (0.0, y2),
            ];
            let pts: Vec<(f64, f64)> = base
                .iter()
                .map(|&(x, y)| match prst {
                    "leftArrow" => (len - x, y),
                    "downArrow" => (y, x),
                    "upArrow" => (y, len - x),
                    _ => (x, y),
                })
                .collect();
            poly(&pts)
        }
        "flowChartDocument" => format!(
            "M0,0H{}V{}C{},{} {},{} 0,{}Z",
            f(w),
            f(h * 0.8),
            f(w * 0.5),
            f(h * 0.6),
            f(w * 0.5),
            f(h * 1.1),
            f(h * 0.85)
        ),
        "can" | "flowChartMagneticDisk" => {
            let ry = (ss * a("adj", 25000.0) / 2.0).min(h / 2.0);
            format!(
                "M0,{ry}A{},{ry} 0 0 1 {},{ry}V{}A{},{ry} 0 0 1 0,{}ZM0,{ry}A{},{ry} 0 0 0 {},{ry}",
                f(w / 2.0),
                f(w),
                f(h - ry),
                f(w / 2.0),
                f(h - ry),
                f(w / 2.0),
                f(w),
                ry = f(ry)
            )
        }
        _ => return None,
    })
}

/// 수식 이름 → 값 (avLst/gdLst 의 `val n` 과 도형 크기 w·h)
fn guide(name: &str, gd: &HashMap<String, f64>, pw: f64, ph: f64) -> f64 {
    if let Ok(v) = name.trim().parse::<f64>() {
        return v;
    }
    match name {
        "w" | "r" => pw,
        "h" | "b" => ph,
        "l" | "t" => 0.0,
        "hc" => pw / 2.0,
        "vc" => ph / 2.0,
        "wd2" => pw / 2.0,
        "hd2" => ph / 2.0,
        _ => gd.get(name).copied().unwrap_or(0.0),
    }
}

/// 사용자 정의 모양의 경로들 → (SVG 경로, 채움, 선)
pub fn custom_paths(geom: &Element, w: f64, h: f64) -> Vec<(String, bool, bool)> {
    let mut gd = adjusts(geom);
    if let Some(l) = geom.child_ns(A, "gdLst") {
        for g in l.elements().filter(|g| g.is(A, "gd")) {
            if let (Some(n), Some(fm)) = (g.attr("name"), g.attr("fmla"))
                && let Some(v) = fm.strip_prefix("val ").and_then(|v| v.trim().parse().ok())
            {
                gd.insert(n.to_string(), v);
            }
        }
    }
    let mut out = Vec::new();
    let Some(list) = geom.child_ns(A, "pathLst") else {
        return out;
    };
    for p in list.elements().filter(|p| p.is(A, "path")) {
        // 경로 좌표계: w/h 속성 (없으면 EMU 로 본다)
        let pw = p.attr_f64("w").filter(|v| *v > 0.0);
        let ph = p.attr_f64("h").filter(|v| *v > 0.0);
        let sx = pw.map_or(1.0 / crate::units::EMU_PER_PX as f64, |v| w / v);
        let sy = ph.map_or(1.0 / crate::units::EMU_PER_PX as f64, |v| h / v);
        let (pw0, ph0) = (pw.unwrap_or(w / sx), ph.unwrap_or(h / sy));
        let pt = |e: &Element| -> (f64, f64) {
            let x = guide(e.attr("x").unwrap_or("0"), &gd, pw0, ph0);
            let y = guide(e.attr("y").unwrap_or("0"), &gd, pw0, ph0);
            (x * sx, y * sy)
        };
        let mut d = String::new();
        let mut cur = (0.0f64, 0.0f64);
        for c in p.elements() {
            let pts: Vec<(f64, f64)> = c.elements().filter(|e| e.is(A, "pt")).map(pt).collect();
            match c.name.as_str() {
                "moveTo" if !pts.is_empty() => {
                    cur = pts[0];
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
                    let _ = write!(d, "Q{},{} {},{}", f(pts[0].0), f(pts[0].1), f(cur.0), f(cur.1));
                }
                "arcTo" => {
                    // 현재 점에서 시작하는 타원 호: wR·hR 반지름, stAng 시작각, swAng 휘는 각 (1/60000 도)
                    let wr = guide(c.attr("wR").unwrap_or("0"), &gd, pw0, ph0) * sx;
                    let hr = guide(c.attr("hR").unwrap_or("0"), &gd, pw0, ph0) * sy;
                    let st = guide(c.attr("stAng").unwrap_or("0"), &gd, pw0, ph0) / 60000.0;
                    let sw = guide(c.attr("swAng").unwrap_or("0"), &gd, pw0, ph0) / 60000.0;
                    let (s0, s1) = (st.to_radians(), (st + sw).to_radians());
                    let cx = cur.0 - wr * s0.cos();
                    let cy = cur.1 - hr * s0.sin();
                    let end = (cx + wr * s1.cos(), cy + hr * s1.sin());
                    let large = if sw.abs() > 180.0 { 1 } else { 0 };
                    let sweep = if sw > 0.0 { 1 } else { 0 };
                    let _ = write!(
                        d,
                        "A{},{} 0 {large} {sweep} {},{}",
                        f(wr),
                        f(hr),
                        f(end.0),
                        f(end.1)
                    );
                    cur = end;
                }
                "close" => d.push('Z'),
                _ => {}
            }
        }
        if d.is_empty() {
            continue;
        }
        let fill = p.attr("fill") != Some("none");
        let stroke = p.attr_bool("stroke").unwrap_or(true);
        out.push((d, fill, stroke));
    }
    out
}

/// 모양 요소 (prstGeom/custGeom) 와 조정값
pub fn geometry(obj: &Element) -> Option<&Element> {
    let p = sp_pr(obj)?;
    p.child_ns(A, "custGeom").or_else(|| p.child_ns(A, "prstGeom"))
}

/// 도형의 경로들 (w×h px). 원본이 없거나 모르는 모양이면 None
pub fn paths(obj: &Element, w: f64, h: f64) -> Option<Vec<(String, bool, bool)>> {
    let g = geometry(obj)?;
    if g.is(A, "custGeom") {
        let p = custom_paths(g, w, h);
        return (!p.is_empty()).then_some(p);
    }
    let prst = g.attr("prst")?;
    preset_path(prst, w, h, &adjusts(g)).map(|d| vec![(d, true, true)])
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

/// 선 끝 모양 (`a:headEnd`/`a:tailEnd` 의 type, `none` 이면 None)
pub fn line_end(obj: &Element, which: &str) -> Option<String> {
    let ln = sp_pr(obj)?.child_ns(A, "ln")?;
    ln.child_ns(A, which)
        .and_then(|e| e.attr("type"))
        .filter(|t| *t != "none")
        .map(str::to_string)
}

/// 그라데이션 채우기 (`a:gradFill`): (각도°, [(위치 0–1, 색 요소)])
pub fn grad_fill(obj: &Element) -> Option<(f64, Vec<(f64, Element)>)> {
    let g = sp_pr(obj)?.child_ns(A, "gradFill")?;
    let stops: Vec<(f64, Element)> = g
        .child_ns(A, "gsLst")?
        .elements()
        .filter(|e| e.is(A, "gs"))
        .filter_map(|gs| {
            let pos = gs.attr_f64("pos").unwrap_or(0.0) / 100_000.0;
            Some((pos, gs.elements().next()?.clone()))
        })
        .collect();
    let ang = g
        .child_ns(A, "lin")
        .and_then(|l| l.attr_f64("ang"))
        .unwrap_or(5_400_000.0)
        / 60000.0;
    (!stops.is_empty()).then_some((ang, stops))
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
        assert_eq!(p[0].0, "M0,100L100,0L200,100Z");
        assert!(p[0].1 && p[0].2);
    }

    #[test]
    fn presets() {
        let none = HashMap::new();
        assert_eq!(
            preset_path("diamond", 100.0, 50.0, &none).unwrap(),
            "M50,0L100,25L50,50L0,25Z"
        );
        assert!(preset_path("rightArrow", 100.0, 40.0, &none).unwrap().starts_with("M0,10L80,10"));
        assert!(preset_path("cloudCallout", 10.0, 10.0, &none).is_none());
    }
}
