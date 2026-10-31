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
    fn with(d: String, fill: PathFill, stroke: bool) -> PathSpec {
        PathSpec { d, fill, stroke }
    }
}

/// 미리 정의된 모양 → 경로들 (w×h px 상자). 조정값은 avLst 의 값(1/100000 단위 비율, 각도는 1/60000 도). 모르는 모양은 None
/// (그리는 쪽이 사각형으로 그린다). 모양은 `shape_guides.rs` 의 가이드 수식으로 — 표준 모양 정의와 같은 구조를 한셀 화면과 모양·가로세로
/// 비율·조정값마다 대조했다
pub fn preset_paths(
    prst: &str,
    w: f64,
    h: f64,
    adj: &HashMap<String, f64>,
) -> Option<Vec<PathSpec>> {
    // 글상자는 사각형
    let prst = if prst == "textBox" { "rect" } else { prst };
    guide_paths(prst, w, h, adj)
}

/// 가이드 수식 모양 (`shape_guides.rs`) → 경로들. 정의는 처음 쓸 때 한 번만 읽는다. 조정값은 avLst 의 기본값 위에 문서의 값을 덮어쓴다
fn guide_paths(prst: &str, w: f64, h: f64, adj: &HashMap<String, f64>) -> Option<Vec<PathSpec>> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<std::sync::Arc<Element>>>>> =
        OnceLock::new();
    let xml = super::shape_guides::definition(prst)?;
    let geom = {
        let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
        cache
            .entry(prst.to_string())
            .or_insert_with(|| {
                crate::xml::parse(&super::shape_guides::expand(xml), "shape_guides")
                    .ok()
                    .map(std::sync::Arc::new)
            })
            .clone()?
    };
    let emu = crate::units::EMU_PER_PX as f64;
    let mut gd = Guides::new(w * emu, h * emu);
    gd.define_list(geom.child_ns(A, "avLst"));
    for (k, v) in adj {
        gd.set(k, *v);
    }
    gd.define_list(geom.child_ns(A, "gdLst"));
    let list = geom.child_ns(A, "pathLst")?;
    let out = paths_of(list, &gd, w, h, w * emu, h * emu);
    (!out.is_empty()).then_some(out)
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

    /// 이미 정한 값(조정값)을 문서의 값으로 바꾼다
    pub fn set(&mut self, name: &str, v: f64) {
        if self.vals.contains_key(name) {
            self.vals.insert(name.to_string(), v);
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
                    // 한셀은 이차 곡선을 제어점 둘이 같은 삼차 곡선으로 그린다 (진짜 이차 곡선보다 제어점 쪽으로 더 휜다)
                    let _ = write!(
                        d,
                        "C{c0},{c1} {c0},{c1} {},{}",
                        f(cur.0),
                        f(cur.1),
                        c0 = f(pts[0].0),
                        c1 = f(pts[0].1)
                    );
                }
                "arcTo" => {
                    // 현재 점에서 시작하는 타원 호: wR·hR 반지름, stAng 시작각, swAng 휘는 각 (1/60000 도)
                    let wr = gd.get(c.attr("wR").unwrap_or("0")) * sx;
                    let hr = gd.get(c.attr("hR").unwrap_or("0")) * sy;
                    let st = gd.get(c.attr("stAng").unwrap_or("0")) / 60000.0;
                    let sw = gd.get(c.attr("swAng").unwrap_or("0")) / 60000.0;
                    d.push_str(&arc_svg(cur, (wr, hr), (sx, sy), st, sw));
                    cur = arc_end(cur, (wr, hr), (sx, sy), st, sw);
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
    fills_then_strokes(out)
}

/// 한셀은 경로마다 칠하고 긋지 않고, 모든 경로의 채우기를 차례로 칠한 뒤 모든 경로의 선을 긋는다 (차트 X·더하기·별: 선만 있는
/// 경로가 채운 경로보다 앞에 정의돼도 선이 위에 보인다). 뒤에 채우기가 더 없으면 경로를 그대로 둔다 (한 경로는 채우기·선을 한 번에)
fn fills_then_strokes(paths: Vec<PathSpec>) -> Vec<PathSpec> {
    let last_fill = paths.iter().rposition(|p| p.fill != PathFill::None);
    let mut out = Vec::with_capacity(paths.len());
    let mut deferred: Vec<PathSpec> = Vec::new();
    for (i, p) in paths.into_iter().enumerate() {
        if last_fill.is_some_and(|l| l > i) {
            // 뒤에 채우기가 더 있다: 선은 미룬다
            if p.stroke {
                deferred.push(PathSpec::with(p.d.clone(), PathFill::None, true));
            }
            if p.fill != PathFill::None {
                out.push(PathSpec::with(p.d, p.fill, false));
            }
        } else if p.fill != PathFill::None && !deferred.is_empty() {
            // 마지막 채우기: 채우고, 미룬 선들, 이 경로의 선
            out.push(PathSpec::with(p.d.clone(), p.fill, false));
            out.append(&mut deferred);
            if p.stroke {
                out.push(PathSpec::with(p.d, PathFill::None, true));
            }
        } else {
            out.append(&mut deferred);
            out.push(p);
        }
    }
    out.append(&mut deferred);
    out
}

/// 타원 위의 각도: DrawingML 의 각도는 실제 방향이다 — 점 = 중심 + (wr·cos t, hr·sin t) 의 t 로 바꾼다
fn ellipse_param(wr: f64, hr: f64, deg: f64) -> f64 {
    let a = deg.to_radians();
    (wr * a.sin()).atan2(hr * a.cos())
}

/// 호의 끝점 (현재 점이 시작각 위에 있다). 반지름 (wr, hr) 은 그릴 크기(px), 각도는 경로 좌표계의 실제 방향이므로
/// 경로 → px 배율 (kx, ky) 로 되돌린 반지름으로 매개변수 각도를 구한다 (경로 크기가 상자와 다르면 원이 늘어난다)
fn arc_end(cur: (f64, f64), r: (f64, f64), k: (f64, f64), st: f64, sw: f64) -> (f64, f64) {
    let (wr, hr) = r;
    let t0 = ellipse_param(wr / k.0, hr / k.1, st);
    let t1 = ellipse_param(wr / k.0, hr / k.1, st + sw);
    let (cx, cy) = (cur.0 - wr * t0.cos(), cur.1 - hr * t0.sin());
    (cx + wr * t1.cos(), cy + hr * t1.sin())
}

/// 호 하나를 SVG 로 (거의 한 바퀴면 반씩 나눈다)
fn arc_svg(cur: (f64, f64), r: (f64, f64), k: (f64, f64), st: f64, sw: f64) -> String {
    if sw.abs() >= 359.9 {
        let half = sw / 2.0;
        let mid = arc_end(cur, r, k, st, half);
        return arc_svg(cur, r, k, st, half) + &arc_svg(mid, r, k, st + half, half);
    }
    let end = arc_end(cur, r, k, st, sw);
    // 각도 방향의 큰 호 판단은 매개변수 각도로
    let (pw, ph) = (r.0 / k.0, r.1 / k.1);
    let dt = ellipse_param(pw, ph, st + sw) - ellipse_param(pw, ph, st);
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
        f(r.0.abs()),
        f(r.1.abs()),
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
        // 표준 모양 정의의 첫 점: 왼쪽 가운데에서 시작해 위·오른쪽·아래로
        assert_eq!(
            preset_path("diamond", 100.0, 50.0, &none).unwrap(),
            "M0,25L50,0L100,25L50,50Z"
        );
        assert!(
            preset_path("rightArrow", 100.0, 40.0, &none)
                .unwrap()
                .starts_with("M0,10L80,10")
        );
        assert!(preset_path("unknownShape", 10.0, 10.0, &none).is_none());
        // 글상자는 사각형
        assert_eq!(
            preset_path("textBox", 10.0, 10.0, &none),
            preset_path("rect", 10.0, 10.0, &none)
        );
        // 조정값이 모양을 바꾼다
        let adjusted = HashMap::from([("adj".to_string(), 30000.0)]);
        assert_ne!(
            preset_paths("star5", 100.0, 100.0, &none).unwrap()[0].d,
            preset_paths("star5", 100.0, 100.0, &adjusted).unwrap()[0].d
        );
        // 표준 모양 187 개 모두: 빈 경로 없음, 좌표는 유한한 수
        let all = include_str!("shape_guides.rs")
            .lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix('"')?
                    .split('"')
                    .next()
                    .map(String::from)
            })
            .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric()) && !n.is_empty())
            .filter(|n| super::super::shape_guides::definition(n).is_some())
            .collect::<Vec<_>>();
        assert!(all.len() >= 187, "{}", all.len());
        for n in &all {
            for (w, h) in [(120.0, 93.0), (53.0, 200.0), (200.0, 53.0)] {
                let ps = preset_paths(n, w, h, &none).unwrap_or_else(|| panic!("{n}"));
                assert!(!ps.is_empty(), "{n}");
                for p in &ps {
                    assert!(!p.d.contains("NaN") && !p.d.contains("inf"), "{n}: {}", p.d);
                }
            }
        }
    }

    #[test]
    fn arc_shapes_follow_the_aspect_ratio() {
        let none = HashMap::new();
        // 굽은 아래 화살표의 호는 정사각형 윤곽을 늘인 것이 아니라 가로세로 비율마다 다른 타원 호다
        let a = preset_paths("curvedDownArrow", 100.0, 100.0, &none).unwrap();
        let b = preset_paths("curvedDownArrow", 200.0, 50.0, &none).unwrap();
        let radii = |p: &[PathSpec]| -> Vec<String> {
            p[0].d
                .split('A')
                .skip(1)
                .filter_map(|s| s.split(' ').next().map(String::from))
                .collect()
        };
        assert!(!radii(&a).is_empty());
        assert_ne!(radii(&a), radii(&b));
        // 한셀은 이차 곡선을 제어점 둘이 같은 삼차 곡선으로 그린다
        let sw = preset_path("swooshArrow", 100.0, 100.0, &none).unwrap();
        assert!(sw.contains('C') && !sw.contains('Q'));
        // 선만 있는 경로가 앞에 정의돼도 선은 채우기 위 (차트 X)
        let x = preset_paths("chartX", 50.0, 50.0, &none).unwrap();
        assert!(x[0].fill != PathFill::None && !x[0].stroke);
        assert!(x.last().unwrap().fill == PathFill::None && x.last().unwrap().stroke);
        // 경로 크기가 상자와 다른 모양(구름)의 호도 늘어난 원: 반지름 비가 상자 비
        let c = preset_paths("cloud", 200.0, 100.0, &none).unwrap();
        assert!(c[0].d.contains('A'));
    }
}
