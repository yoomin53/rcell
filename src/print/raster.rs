// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그리기 목록([`Op`]) → 비트맵 → PNG. 웹 페이지(`.htm`·`.mht`)의 도형·차트 그림을 만든다 (한셀은 도형·차트를
//! RGBA PNG 로 그려 넣는다). 순수 Rust 의 작은 래스터라이저: 경로 채우기(비제로·짝홀 규칙, 세로 8단 표본 + 가로 정확한
//! 덮임), 선(맞닿은 모서리 접합·점선), 글자(글꼴 윤곽을 채움), 그림(이중선형), 사각형 자르기. 바탕은 투명이다.

use super::pdf::FontSet;
use crate::render::page::{FontKey, Op, Seg};

/// 세로 표본 수 (한 픽셀 줄 안)
const SS: usize = 8;

/// 투명한 RGBA 비트맵 (직선 알파, 0..1)
pub struct Bitmap {
    pub w: u32,
    pub h: u32,
    px: Vec<[f32; 4]>,
}

/// 변환 [a b c d e f]: x' = a·x + c·y + e, y' = b·x + d·y + f
type Mat = [f64; 6];

fn apply(m: &Mat, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

impl Bitmap {
    pub fn new(w: u32, h: u32) -> Bitmap {
        Bitmap {
            w,
            h,
            px: vec![[0.0; 4]; (w as usize) * (h as usize)],
        }
    }

    /// 한 픽셀에 색을 덮임(0..1)만큼 얹는다 (source-over)
    fn blend(&mut self, x: u32, y: u32, c: [u8; 3], cov: f32) {
        if cov <= 0.0 {
            return;
        }
        let p = &mut self.px[(y as usize) * (self.w as usize) + x as usize];
        let sa = cov.min(1.0);
        let da = p[3];
        let oa = sa + da * (1.0 - sa);
        if oa <= 0.0 {
            return;
        }
        for k in 0..3 {
            let sc = c[k] as f32 / 255.0;
            p[k] = (sc * sa + p[k] * da * (1.0 - sa)) / oa;
        }
        p[3] = oa;
    }

    /// 바탕이 투명한 RGBA 바이트
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            for v in p {
                out.push((v * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
            }
        }
        out
    }
}

/// 다각형 모음 (점 목록들). 채우기 전 변환을 끝낸 픽셀 좌표
type Polys = Vec<Vec<(f64, f64)>>;

/// 경로를 점 목록들로 (베지어는 선분으로 펴서). 열린 조각은 `closed = false`
fn flatten(segs: &[Seg], m: &Mat) -> Vec<(Vec<(f64, f64)>, bool)> {
    let mut out: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let mut start = (0.0, 0.0);
    let flush =
        |cur: &mut Vec<(f64, f64)>, out: &mut Vec<(Vec<(f64, f64)>, bool)>, closed: bool| {
            if cur.len() >= 2 || (closed && !cur.is_empty()) {
                out.push((std::mem::take(cur), closed));
            } else {
                cur.clear();
            }
        };
    for s in segs {
        match *s {
            Seg::Move(x, y) => {
                flush(&mut cur, &mut out, false);
                let p = apply(m, x, y);
                start = p;
                cur.push(p);
            }
            Seg::Line(x, y) => {
                if cur.is_empty() {
                    cur.push(start);
                }
                cur.push(apply(m, x, y));
            }
            Seg::Cubic(x1, y1, x2, y2, x3, y3) => {
                if cur.is_empty() {
                    cur.push(start);
                }
                let p0 = *cur.last().unwrap();
                let (p1, p2, p3) = (apply(m, x1, y1), apply(m, x2, y2), apply(m, x3, y3));
                // 제어점 거리로 나눌 수 (0.2px 안쪽이 되게)
                let len = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
                let n = ((len / 2.0).sqrt() * 1.5).ceil().clamp(2.0, 64.0) as usize;
                for i in 1..=n {
                    let t = i as f64 / n as f64;
                    let u = 1.0 - t;
                    cur.push((
                        u * u * u * p0.0
                            + 3.0 * u * u * t * p1.0
                            + 3.0 * u * t * t * p2.0
                            + t * t * t * p3.0,
                        u * u * u * p0.1
                            + 3.0 * u * u * t * p1.1
                            + 3.0 * u * t * t * p2.1
                            + t * t * t * p3.1,
                    ));
                }
            }
            Seg::Close => {
                flush(&mut cur, &mut out, true);
                cur.push(start);
                cur.clear();
            }
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// 사각형 자르기 (픽셀 좌표)
#[derive(Clone, Copy)]
struct ClipRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

/// 다각형들을 채운다. `even_odd` 면 짝홀 규칙, 아니면 비제로 규칙
fn fill_polys(
    bm: &mut Bitmap,
    polys: &Polys,
    color: [u8; 3],
    even_odd: bool,
    clip: Option<ClipRect>,
) {
    let mut ymin = f64::MAX;
    let mut ymax = f64::MIN;
    // (x0, y0, x1, y1, 방향)
    let mut edges: Vec<(f64, f64, f64, f64, i32)> = Vec::new();
    for p in polys {
        let n = p.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let (a, b) = (p[i], p[(i + 1) % n]);
            ymin = ymin.min(a.1);
            ymax = ymax.max(a.1);
            if a.1 == b.1 {
                continue;
            }
            if a.1 < b.1 {
                edges.push((a.0, a.1, b.0, b.1, 1));
            } else {
                edges.push((b.0, b.1, a.0, a.1, -1));
            }
        }
    }
    if edges.is_empty() {
        return;
    }
    let (cx0, cy0, cx1, cy1) = clip.map_or((0.0, 0.0, bm.w as f64, bm.h as f64), |c| {
        (
            c.x0.max(0.0),
            c.y0.max(0.0),
            c.x1.min(bm.w as f64),
            c.y1.min(bm.h as f64),
        )
    });
    let y_start = (ymin.floor().max(cy0.floor())) as i64;
    let y_end = (ymax.ceil().min(cy1.ceil())) as i64;
    let mut cov = vec![0f32; bm.w as usize + 1];
    let mut xs: Vec<(f64, i32)> = Vec::new();
    for y in y_start..y_end {
        if y < 0 || y >= bm.h as i64 {
            continue;
        }
        let mut touched = false;
        for s in 0..SS {
            let sy = y as f64 + (s as f64 + 0.5) / SS as f64;
            if sy < cy0 || sy >= cy1 {
                continue;
            }
            xs.clear();
            for &(x0, y0, x1, y1, d) in &edges {
                if sy >= y0 && sy < y1 {
                    xs.push((x0 + (x1 - x0) * (sy - y0) / (y1 - y0), d));
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut wind = 0;
            for i in 0..xs.len() - 1 {
                wind += xs[i].1;
                let inside = if even_odd {
                    (i + 1) % 2 == 1
                } else {
                    wind != 0
                };
                if !inside {
                    continue;
                }
                let (a, b) = (xs[i].0.max(cx0), xs[i + 1].0.min(cx1));
                if b <= a {
                    continue;
                }
                touched = true;
                let (ia, ib) = (a.floor() as usize, (b.ceil() as usize).min(bm.w as usize));
                for (x, c) in cov.iter_mut().enumerate().take(ib).skip(ia) {
                    let ov = (b.min(x as f64 + 1.0) - a.max(x as f64)).max(0.0);
                    *c += (ov / SS as f64) as f32;
                }
            }
        }
        if touched {
            for (x, c) in cov.iter_mut().enumerate().take(bm.w as usize) {
                if *c > 0.0 {
                    bm.blend(x as u32, y as u32, color, *c);
                    *c = 0.0;
                }
            }
        }
    }
}

/// 점선 무늬로 나눈 조각들 (선·다각선 길이를 따라 켜고 끈다)
fn dash_pieces(pts: &[(f64, f64)], closed: bool, dash: &[f64]) -> Vec<Vec<(f64, f64)>> {
    let mut path: Vec<(f64, f64)> = pts.to_vec();
    if closed && let Some(&f) = pts.first() {
        path.push(f);
    }
    if dash.is_empty() || dash.iter().all(|d| *d <= 0.0) {
        return vec![path];
    }
    let mut out = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let (mut idx, mut left, mut on) = (0usize, dash[0], true);
    if path.is_empty() {
        return out;
    }
    cur.push(path[0]);
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = dist(a, b);
        if len == 0.0 {
            continue;
        }
        let mut pos = 0.0;
        while pos < len {
            let step = left.min(len - pos);
            pos += step;
            left -= step;
            let p = (a.0 + (b.0 - a.0) * pos / len, a.1 + (b.1 - a.1) * pos / len);
            if on {
                cur.push(p);
            }
            if left <= 1e-9 {
                if on && cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                idx = (idx + 1) % dash.len();
                left = dash[idx].max(1e-6);
                on = !on;
                if on {
                    cur.push(p);
                }
            }
        }
    }
    if on && cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// 선 하나를 다각형들로: 선분마다 직사각형 + 꺾인 곳의 바깥 접합 (마이터, 한계 4, 넘으면 빗면). 모두 같은 방향
fn stroke_polys(pts: &[(f64, f64)], closed: bool, hw: f64, polys: &mut Polys) {
    let mut p: Vec<(f64, f64)> = Vec::new();
    for &q in pts {
        if p.last().is_none_or(|l| dist(*l, q) > 1e-9) {
            p.push(q);
        }
    }
    if closed && p.len() > 1 && dist(p[0], *p.last().unwrap()) < 1e-9 {
        p.pop();
    }
    let n = p.len();
    if n < 2 {
        return;
    }
    let segs = if closed { n } else { n - 1 };
    let norm = |a: (f64, f64), b: (f64, f64)| {
        let l = dist(a, b);
        let (dx, dy) = ((b.0 - a.0) / l, (b.1 - a.1) / l);
        (-dy * hw, dx * hw)
    };
    let mut push = |mut poly: Vec<(f64, f64)>| {
        // 방향을 하나로 (면적 부호)
        let mut area = 0.0;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            area += a.0 * b.1 - b.0 * a.1;
        }
        if area < 0.0 {
            poly.reverse();
        }
        polys.push(poly);
    };
    for i in 0..segs {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let nn = norm(a, b);
        push(vec![
            (a.0 + nn.0, a.1 + nn.1),
            (b.0 + nn.0, b.1 + nn.1),
            (b.0 - nn.0, b.1 - nn.1),
            (a.0 - nn.0, a.1 - nn.1),
        ]);
        // 다음 선분과의 접합
        let has_next = closed || i + 1 < segs;
        if !has_next {
            continue;
        }
        let c = (p[(i + 2) % n], b);
        let n1 = nn;
        let n2 = norm(b, c.0);
        let cross = n1.0 * n2.1 - n1.1 * n2.0;
        if cross.abs() < 1e-9 {
            continue;
        }
        // 꺾인 바깥쪽: cross > 0 이면 (왼쪽으로 돈다) 바깥은 −법선쪽
        let s = if cross > 0.0 { -1.0 } else { 1.0 };
        let (o1, o2) = (
            (b.0 + s * n1.0, b.1 + s * n1.1),
            (b.0 + s * n2.0, b.1 + s * n2.1),
        );
        let dot = (n1.0 * n2.0 + n1.1 * n2.1) / (hw * hw);
        let k = 1.0 + dot;
        if k > 1.0 / 16.0 {
            // 마이터 길이 / 선 굵기 = 1 / sin(θ/2) ≤ 4
            let mx = b.0 + s * (n1.0 + n2.0) / k;
            let my = b.1 + s * (n1.1 + n2.1) / k;
            push(vec![b, o1, (mx, my), o2]);
        } else {
            push(vec![b, o1, o2]);
        }
    }
}

/// 글자 윤곽을 `Seg` 로 모으는 도우미 (글리프 좌표 → 변환)
struct Outline {
    segs: Vec<Seg>,
    m: Mat,
    last: (f64, f64),
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = apply(&self.m, x as f64, y as f64);
        self.last = p;
        self.segs.push(Seg::Move(p.0, p.1));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = apply(&self.m, x as f64, y as f64);
        self.last = p;
        self.segs.push(Seg::Line(p.0, p.1));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let c = apply(&self.m, x1 as f64, y1 as f64);
        let p = apply(&self.m, x as f64, y as f64);
        let a = self.last;
        self.segs.push(Seg::Cubic(
            a.0 + 2.0 / 3.0 * (c.0 - a.0),
            a.1 + 2.0 / 3.0 * (c.1 - a.1),
            p.0 + 2.0 / 3.0 * (c.0 - p.0),
            p.1 + 2.0 / 3.0 * (c.1 - p.1),
            p.0,
            p.1,
        ));
        self.last = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = (
            apply(&self.m, x1 as f64, y1 as f64),
            apply(&self.m, x2 as f64, y2 as f64),
            apply(&self.m, x as f64, y as f64),
        );
        self.segs.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, p.0, p.1));
        self.last = p;
    }
    fn close(&mut self) {
        self.segs.push(Seg::Close);
    }
}

/// 장치 단위 → 픽셀 (1px = 6.25 장치 단위)
const K: f64 = 1.0 / 6.25;

/// 그리기 목록을 `w × h` 비트맵에 그린다. 글꼴이 없으면 글자는 그리지 않는다
pub fn draw(ops: &[Op], w: u32, h: u32, fonts: &FontSet) -> Bitmap {
    // 그림 묶음(차트)은 풀어서 그린다
    let flat: Vec<Op>;
    let ops = if ops.iter().any(|o| matches!(o, Op::Raster { .. })) {
        flat = flatten_raster(ops);
        &flat[..]
    } else {
        ops
    };
    let mut bm = Bitmap::new(w, h);
    let id: Mat = [K, 0.0, 0.0, K, 0.0, 0.0];
    let mut clips: Vec<ClipRect> = Vec::new();
    for op in ops {
        let clip = clips.last().copied();
        match op {
            Op::Clip { x0, y0, x1, y1 } => {
                let r = ClipRect {
                    x0: x0 * K,
                    y0: y0 * K,
                    x1: x1 * K,
                    y1: y1 * K,
                };
                let r = match clip {
                    Some(c) => ClipRect {
                        x0: r.x0.max(c.x0),
                        y0: r.y0.max(c.y0),
                        x1: r.x1.min(c.x1),
                        y1: r.y1.min(c.y1),
                    },
                    None => r,
                };
                clips.push(r);
            }
            Op::Unclip => {
                clips.pop();
            }
            Op::Rect {
                x0,
                y0,
                x1,
                y1,
                color,
            } => {
                let poly = vec![vec![
                    (x0 * K, y0 * K),
                    (x1 * K, y0 * K),
                    (x1 * K, y1 * K),
                    (x0 * K, y1 * K),
                ]];
                fill_polys(&mut bm, &poly, *color, false, clip);
            }
            Op::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
                dash,
            } => {
                let pts = [(x0 * K, y0 * K), (x1 * K, y1 * K)];
                let d: Vec<f64> = dash.iter().map(|v| v * K).collect();
                let mut polys = Polys::new();
                for piece in dash_pieces(&pts, false, &d) {
                    stroke_polys(&piece, false, width * K / 2.0, &mut polys);
                }
                fill_polys(&mut bm, &polys, *color, false, clip);
            }
            Op::Path {
                segs,
                fill,
                stroke,
                even_odd,
                ..
            } => {
                let parts = flatten(segs, &id);
                if let Some(c) = fill {
                    let polys: Polys = parts
                        .iter()
                        .filter(|p| p.0.len() >= 3)
                        .map(|p| p.0.clone())
                        .collect();
                    fill_polys(&mut bm, &polys, *c, *even_odd, clip);
                }
                if let Some((sw, c, dash)) = stroke {
                    let d: Vec<f64> = dash.iter().map(|v| v * K).collect();
                    let mut polys = Polys::new();
                    for (pts, closed) in &parts {
                        for piece in dash_pieces(pts, *closed, &d) {
                            let closed_piece = *closed && d.is_empty();
                            stroke_polys(&piece, closed_piece, sw * K / 2.0, &mut polys);
                        }
                    }
                    fill_polys(&mut bm, &polys, *c, false, clip);
                }
            }
            Op::Text {
                x,
                y,
                size,
                font,
                color,
                text,
                underline,
                strike,
                stretch,
                angle,
            } => {
                draw_text(
                    &mut bm,
                    fonts,
                    font,
                    text,
                    (*x * K, *y * K),
                    *size * K,
                    *color,
                    (*underline, *strike),
                    *stretch,
                    *angle,
                    clip,
                );
            }
            Op::Image {
                x0,
                y0,
                x1,
                y1,
                data,
            } => {
                draw_image(&mut bm, (x0 * K, y0 * K, x1 * K, y1 * K), data, clip);
            }
            // 위에서 풀었다
            Op::Raster { .. } => {}
        }
    }
    bm
}

/// [`Op::Raster`] 묶음을 안의 목록으로 펴 (겹쳐 든 것도)
pub fn flatten_raster(ops: &[Op]) -> Vec<Op> {
    let mut out = Vec::with_capacity(ops.len());
    for op in ops {
        match op {
            Op::Raster { ops: sub, .. } => out.extend(flatten_raster(sub)),
            o => out.push(o.clone()),
        }
    }
    out
}

/// 글자 한 줄: (x, y) 는 기준선 왼쪽 끝. 글리프는 윤곽을 채운다 (합성 굵게·기울임은 덧선·기울이기)
#[allow(clippy::too_many_arguments)]
fn draw_text(
    bm: &mut Bitmap,
    fonts: &FontSet,
    key: &FontKey,
    text: &str,
    (x, y): (f64, f64),
    size: f64,
    color: [u8; 3],
    (underline, strike): (bool, bool),
    stretch: f64,
    angle: f64,
    clip: Option<ClipRect>,
) {
    let (sa, ca) = angle.to_radians().sin_cos();
    let mut pen = 0.0;
    for run in fonts.runs(key, text, false) {
        let Some(fi) = run.font else {
            pen += run.advance * size / 1000.0 * stretch;
            continue;
        };
        let Some((data, index)) = fonts.face_data(fi) else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(&data, index) else {
            continue;
        };
        let upem = face.units_per_em() as f64;
        let k = size / upem;
        let shear = if run.synth_italic { 0.2 } else { 0.0 };
        for gid in &run.glyphs {
            let g = ttf_parser::GlyphId(*gid);
            // 글리프 좌표 (u, v: 위가 +) → 장치: 크기·장평(k·s), 기울임(v 의 일부를 u 에), 위 뒤집기, 글 방향 회전, 기준선 자리
            let ks = k * stretch;
            let m: Mat = [
                ca * ks,
                sa * ks,
                ca * shear * ks + sa * k,
                sa * shear * ks - ca * k,
                x + ca * pen,
                y + sa * pen,
            ];
            let mut ob = Outline {
                segs: Vec::new(),
                m,
                last: (0.0, 0.0),
            };
            if face.outline_glyph(g, &mut ob).is_some() {
                let parts = flatten(&ob.segs, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                let polys: Polys = parts
                    .iter()
                    .filter(|p| p.0.len() >= 3)
                    .map(|p| p.0.clone())
                    .collect();
                fill_polys(bm, &polys, color, false, clip);
                if run.synth_bold {
                    let mut sp = Polys::new();
                    for (pts, closed) in &parts {
                        stroke_polys(pts, *closed, size * 0.02, &mut sp);
                    }
                    fill_polys(bm, &sp, color, false, clip);
                }
            }
            let adv = face.glyph_hor_advance(g).unwrap_or(0) as f64 * k * stretch;
            pen += adv;
        }
    }
    // 밑줄·취소선: 글 너비만큼 가는 선 (굵기 size/14)
    let wline = size / 14.0;
    for (on, off) in [(underline, size * 0.12), (strike, -size * 0.3)] {
        if !on {
            continue;
        }
        let (x0, y0) = (x - sa * off, y + ca * off);
        let (x1, y1) = (x0 + ca * pen, y0 + sa * pen);
        let mut polys = Polys::new();
        stroke_polys(&[(x0, y0), (x1, y1)], false, wline / 2.0, &mut polys);
        fill_polys(bm, &polys, color, false, clip);
    }
}

/// 그림: 상자에 이중선형으로 (PNG·BMP 만 — JPEG 는 그리지 않는다)
fn draw_image(
    bm: &mut Bitmap,
    (x0, y0, x1, y1): (f64, f64, f64, f64),
    data: &[u8],
    clip: Option<ClipRect>,
) {
    let Some(img) = super::pdfimage::decode(data) else {
        return;
    };
    if img.dct || img.width == 0 || img.height == 0 {
        return;
    }
    let (iw, ih, comps) = (
        img.width as usize,
        img.height as usize,
        img.components as usize,
    );
    if img.data.len() < iw * ih * comps {
        return;
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    if bw <= 0.0 || bh <= 0.0 {
        return;
    }
    let (cx0, cy0, cx1, cy1) = clip.map_or((0.0, 0.0, bm.w as f64, bm.h as f64), |c| {
        (
            c.x0.max(0.0),
            c.y0.max(0.0),
            c.x1.min(bm.w as f64),
            c.y1.min(bm.h as f64),
        )
    });
    let sample = |fx: f64, fy: f64| -> ([f32; 3], f32) {
        let fx = (fx - 0.5).clamp(0.0, iw as f64 - 1.0);
        let fy = (fy - 0.5).clamp(0.0, ih as f64 - 1.0);
        let (xi, yi) = (fx.floor() as usize, fy.floor() as usize);
        let (xj, yj) = ((xi + 1).min(iw - 1), (yi + 1).min(ih - 1));
        let (tx, ty) = ((fx - xi as f64) as f32, (fy - yi as f64) as f32);
        let get = |x: usize, y: usize| -> ([f32; 3], f32) {
            let o = (y * iw + x) * comps;
            let c = if comps >= 3 {
                [
                    img.data[o] as f32,
                    img.data[o + 1] as f32,
                    img.data[o + 2] as f32,
                ]
            } else {
                let v = img.data[o] as f32;
                [v, v, v]
            };
            let a = img
                .alpha
                .as_ref()
                .and_then(|a| a.get(y * iw + x))
                .map_or(1.0, |&a| a as f32 / 255.0);
            (c, a)
        };
        let (a, b, c, d) = (get(xi, yi), get(xj, yi), get(xi, yj), get(xj, yj));
        let mut col = [0.0; 3];
        for (k, v) in col.iter_mut().enumerate() {
            *v = (a.0[k] * (1.0 - tx) + b.0[k] * tx) * (1.0 - ty)
                + (c.0[k] * (1.0 - tx) + d.0[k] * tx) * ty;
        }
        let al = (a.1 * (1.0 - tx) + b.1 * tx) * (1.0 - ty) + (c.1 * (1.0 - tx) + d.1 * tx) * ty;
        (col, al)
    };
    let (px0, px1) = (x0.max(cx0).floor() as i64, x1.min(cx1).ceil() as i64);
    let (py0, py1) = (y0.max(cy0).floor() as i64, y1.min(cy1).ceil() as i64);
    for py in py0..py1 {
        for px in px0..px1 {
            let (sx, sy) = (
                (px as f64 + 0.5 - x0) / bw * iw as f64,
                (py as f64 + 0.5 - y0) / bh * ih as f64,
            );
            if sx < 0.0 || sy < 0.0 || sx > iw as f64 || sy > ih as f64 {
                continue;
            }
            let (c, a) = sample(sx, sy);
            bm.blend(
                px as u32,
                py as u32,
                [c[0] as u8, c[1] as u8, c[2] as u8],
                a,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

fn crc32(data: &[u8], table: &[u32; 256]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// RGBA(8비트) PNG 바이트. 줄마다 필터 하나를 골라 (없음·왼쪽·위) 압축한다
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut table = [0u32; 256];
    for (n, t) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    let stride = w as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for y in 0..h as usize {
        let line = &rgba[y * stride..(y + 1) * stride];
        // 필터 후보: 0 없음, 1 왼쪽 — 절댓값 합이 작은 쪽
        let sub: Vec<u8> = (0..stride)
            .map(|i| line[i].wrapping_sub(if i >= 4 { line[i - 4] } else { 0 }))
            .collect();
        let cost = |v: &[u8]| {
            v.iter()
                .map(|&b| (b as i8).unsigned_abs() as u64)
                .sum::<u64>()
        };
        if cost(&sub) < cost(line) {
            raw.push(1);
            raw.extend_from_slice(&sub);
        } else {
            raw.push(0);
            raw.extend_from_slice(line);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c, &table).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &idat);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::print::font::FontDb;

    #[test]
    fn fills_rect_and_encodes_png() {
        let db = FontDb::new();
        let fonts = FontSet::new(&db);
        // 6.25 장치 단위 = 1px: 10×10 상자 안 (2,2)–(6,6) 채우기
        let ops = vec![Op::Rect {
            x0: 15.625,
            y0: 12.5,
            x1: 37.5,
            y1: 37.5,
            color: [255, 0, 0],
        }];
        let bm = draw(&ops, 10, 10, &fonts);
        let rgba = bm.rgba();
        let at = |x: usize, y: usize| &rgba[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4];
        assert_eq!(at(3, 3), &[255, 0, 0, 255]);
        assert_eq!(at(0, 0), &[0, 0, 0, 0]);
        // 반 픽셀 덮임: x = 2.5 에서 시작하므로 2번 열은 반
        assert!((at(2, 3)[3] as i32 - 128).abs() <= 1);
        let png = encode_png(10, 10, &rgba);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 10);
    }

    #[test]
    fn strokes_close_with_miter_corners() {
        let db = FontDb::new();
        let fonts = FontSet::new(&db);
        // 가운데 선 굵기 2px 짜리 닫힌 사각형: 바깥 모서리 픽셀이 꽉 찬다
        let ops = vec![Op::Path {
            segs: vec![
                Seg::Move(25.0, 25.0),
                Seg::Line(100.0, 25.0),
                Seg::Line(100.0, 100.0),
                Seg::Line(25.0, 100.0),
                Seg::Close,
            ],
            fill: None,
            stroke: Some((12.5, [0, 0, 0], Vec::new())),
            even_odd: false,
            caps: None,
            hatch: None,
        }];
        let rgba = draw(&ops, 20, 20, &fonts).rgba();
        let a = |x: usize, y: usize| rgba[(y * 20 + x) * 4 + 3];
        assert_eq!(a(3, 3), 255);
        assert_eq!(a(8, 3), 255);
        assert!(a(8, 8) < 255);
    }
}
