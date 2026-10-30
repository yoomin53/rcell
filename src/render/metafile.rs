// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! Windows 메타파일(EMF·WMF) → 그리기 목록. 공개된 형식 명세([MS-EMF]·[MS-WMF])의 레코드를 읽는다.
//!
//! 지원: 선·다각형·다중 다각형·곡선(16·32비트), 사각형·둥근 사각형·타원, 경로(BEGINPATH…FILLPATH),
//! 펜(실선·점선·굵기)·붓(채우기), 글꼴·글자 정렬·색·각도, 비트맵(STRETCHDIBITS·STRETCHDIB·DIBSTRETCHBLT),
//! 세계 변환·매핑 방식·창/뷰포트, 장치 상태 저장·복원. 호·부채꼴·활꼴·무늬 붓·자르기는 그리지 않는다.
//!
//! 좌표는 메타파일 틀(EMF `rclFrame` 을 기준 장치의 px 로, WMF 는 창 크기)의 왼쪽 위가 (0, 0) 인 px.
//! 화면([`to_svg`])과 PDF(`render::page`)가 같은 목록을 쓴다.
//!
//! 한셀 PDF 실험 (GDI+ 가 만든 EMF 를 그림으로 넣어 인쇄): 벡터 그대로 — 틀 전체가 그림 상자에 늘어난다
//! (가로·세로 따로), 펜 굵기는 가로 배율, 글자는 글꼴 오름(winAscent)만큼 내린 기준선에 놓인다.

use super::page::Seg;

/// 호의 (시작점, 조각들 — 각 조각은 제어점 둘과 끝점)
type ArcPieces = ((f64, f64), Vec<[(f64, f64); 3]>);

/// 빗금 붓: (무늬, 빗금 색, 바탕색)
pub type Hatch = (u8, [u8; 3], Option<[u8; 3]>);

/// 그리기 하나 (틀의 px)
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Path {
        segs: Vec<Seg>,
        fill: Option<[u8; 3]>,
        /// 굵기(px), 색, 점선 무늬(px)
        stroke: Option<(f64, [u8; 3], Vec<f64>)>,
        even_odd: bool,
        /// 선 끝·꺾임 (PDF 의 J·j 값: 0 평평/뾰족, 1 둥글게, 2 네모/비스듬) — GDI 펜 기본은 둘 다 둥글다 (한셀 PDF 는 `1 J 1 j`)
        caps: (u8, u8),
        /// 빗금 붓 채우기: (무늬 0~5 = 가로·세로·\·/·+·x, 빗금 색, 바탕색 — 불투명 모드일 때) — 한셀 PDF 는 8pt 바둑 무늬(Tiling)로,
        /// 페이지 원점에 맞춘다. 이때 `fill` 은 None
        hatch: Option<Hatch>,
    },
    Text {
        /// 기준점 (글자 정렬 기준)
        x: f64,
        y: f64,
        /// 0 왼쪽, 1 가운데, 2 오른쪽
        halign: u8,
        /// 0 위, 1 기준선, 2 아래
        valign: u8,
        /// 글자 크기 (em, px)
        em: f64,
        name: String,
        bold: bool,
        italic: bool,
        underline: bool,
        strike: bool,
        color: [u8; 3],
        text: String,
        /// 시계 방향 각도 (도)
        angle: f64,
    },
    /// 사각형으로 자르기 시작 (틀의 px) — 짝이 되는 `Unclip` 까지
    Clip {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    Unclip,
    /// 비트맵 (BMP 파일 바이트)
    Image {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        bmp: Vec<u8>,
    },
}

/// 읽은 메타파일
#[derive(Debug, Clone, Default)]
pub struct Metafile {
    /// 틀 크기 (px)
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

/// EMF·WMF 바이트인가
pub fn is_metafile(d: &[u8]) -> bool {
    is_emf(d) || is_wmf(d)
}

fn is_emf(d: &[u8]) -> bool {
    d.len() >= 88 && d[..4] == [1, 0, 0, 0] && d[40..44] == *b" EMF"
}

fn is_wmf(d: &[u8]) -> bool {
    d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A])
        || (d.len() >= 18 && matches!(d[0], 1 | 2) && d[1] == 0 && d[2] == 9 && d[3] == 0)
}

/// 메타파일을 읽는다. 알아볼 수 없으면 None
pub fn parse(d: &[u8]) -> Option<Metafile> {
    if is_emf(d) {
        emf(d)
    } else if is_wmf(d) {
        wmf(d)
    } else {
        None
    }
}

// ---------------------------------------------------------------- 바이트 읽기

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u8(&self, o: usize) -> Option<u8> {
        self.0.get(o).copied()
    }
    fn u16(&self, o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(self.0.get(o..o + 2)?.try_into().ok()?))
    }
    fn i16(&self, o: usize) -> Option<i16> {
        Some(self.u16(o)? as i16)
    }
    fn u32(&self, o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(o..o + 4)?.try_into().ok()?))
    }
    fn i32(&self, o: usize) -> Option<i32> {
        Some(self.u32(o)? as i32)
    }
    fn f32(&self, o: usize) -> Option<f64> {
        Some(f32::from_bits(self.u32(o)?) as f64)
    }
}

fn colorref(v: u32) -> [u8; 3] {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

// ---------------------------------------------------------------- 장치 상태

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` 를 먼저, 그 다음 `b` 를 적용하는 변환 (행 벡터 꼴 — GDI XFORM)
fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

#[derive(Debug, Clone)]
enum Obj {
    Pen {
        style: u32,
        width: f64,
        color: [u8; 3],
    },
    Brush {
        style: u32,
        color: [u8; 3],
        /// 빗금 무늬 (style 2)
        hatch: u32,
    },
    Font(FontSpec),
    Other,
}

#[derive(Debug, Clone)]
struct FontSpec {
    /// 논리 높이 (음수 = em, 양수 = 칸 높이)
    height: f64,
    /// 십분의 일 도 (반시계)
    escapement: f64,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    name: String,
}

impl Default for FontSpec {
    fn default() -> FontSpec {
        FontSpec {
            height: -16.0,
            escapement: 0.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            name: "Arial".into(),
        }
    }
}

#[derive(Debug, Clone)]
struct Dc {
    world: Matrix,
    map_mode: u32,
    win_org: (f64, f64),
    win_ext: (f64, f64),
    vp_org: (f64, f64),
    vp_ext: (f64, f64),
    pen: Obj,
    brush: Obj,
    font: FontSpec,
    text_color: [u8; 3],
    text_align: u32,
    winding: bool,
    cur: (f64, f64),
    /// 바탕색과 바탕 모드 (빗금 붓 — 기본 흰색·불투명)
    bk_color: [u8; 3],
    bk_opaque: bool,
    /// 호의 방향 (SETARCDIRECTION — 기본 반시계)
    arc_ccw: bool,
    /// 열려 있는 자르기 수
    clips: u32,
}

impl Dc {
    fn new() -> Dc {
        Dc {
            world: IDENTITY,
            map_mode: 1,
            win_org: (0.0, 0.0),
            win_ext: (1.0, 1.0),
            vp_org: (0.0, 0.0),
            vp_ext: (1.0, 1.0),
            pen: stock(7),
            brush: stock(0),
            font: FontSpec::default(),
            text_color: [0, 0, 0],
            text_align: 0,
            winding: false,
            cur: (0.0, 0.0),
            bk_color: [255; 3],
            bk_opaque: true,
            arc_ccw: true,
            clips: 0,
        }
    }
}

/// 기본(stock) 개체
fn stock(n: u32) -> Obj {
    let gray = |v: u8| Obj::Brush {
        style: 0,
        color: [v, v, v],
        hatch: 0,
    };
    match n {
        0 => gray(255),
        1 => gray(0xC0),
        2 => gray(0x80),
        3 => gray(0x40),
        4 => gray(0),
        5 => Obj::Brush {
            style: 1,
            color: [0; 3],
            hatch: 0,
        },
        6 => Obj::Pen {
            style: 0,
            width: 1.0,
            color: [255; 3],
        },
        7 => Obj::Pen {
            style: 0,
            width: 1.0,
            color: [0; 3],
        },
        8 => Obj::Pen {
            style: 5,
            width: 1.0,
            color: [0; 3],
        },
        18 => gray(255),
        19 => Obj::Pen {
            style: 0,
            width: 1.0,
            color: [0; 3],
        },
        _ => Obj::Other,
    }
}

struct Player {
    dc: Dc,
    stack: Vec<Dc>,
    objects: Vec<Option<Obj>>,
    /// 경로를 모으는 중이면 그 조각
    path: Option<Vec<Seg>>,
    items: Vec<Item>,
    /// 틀의 왼쪽 위 (장치 px)
    origin: (f64, f64),
    /// 고정 매핑 방식의 1mm 당 px (x, y)
    px_per_mm: (f64, f64),
    /// WMF: 창 크기가 부호를 정한다 (EMF 는 쓰지 않는다)
    wmf: bool,
}

impl Player {
    fn new() -> Player {
        Player {
            dc: Dc::new(),
            stack: Vec::new(),
            objects: Vec::new(),
            path: None,
            items: Vec::new(),
            origin: (0.0, 0.0),
            px_per_mm: (3.78, 3.78),
            wmf: false,
        }
    }

    /// 논리 → 틀 px
    fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let d = &self.dc;
        let wx = d.world[0] * x + d.world[2] * y + d.world[4];
        let wy = d.world[1] * x + d.world[3] * y + d.world[5];
        let (sx, sy) = self.scale();
        (
            (wx - d.win_org.0) * sx + d.vp_org.0 - self.origin.0,
            (wy - d.win_org.1) * sy + d.vp_org.1 - self.origin.1,
        )
    }

    /// 매핑 방식의 논리 단위당 장치 px (x, y)
    fn scale(&self) -> (f64, f64) {
        let d = &self.dc;
        let unit_mm = |mm: f64| (self.px_per_mm.0 * mm, -self.px_per_mm.1 * mm);
        match d.map_mode {
            2 => unit_mm(0.1),
            3 => unit_mm(0.01),
            4 => unit_mm(0.254),
            5 => unit_mm(0.0254),
            6 => unit_mm(25.4 / 1440.0),
            7 | 8 => {
                let f = |v: f64, w: f64| if w == 0.0 { 1.0 } else { v / w };
                if self.wmf {
                    // 창이 곧 그림 틀 — 크기의 부호만 방향을 뒤집는다
                    (d.win_ext.0.signum(), d.win_ext.1.signum())
                } else {
                    (f(d.vp_ext.0, d.win_ext.0), f(d.vp_ext.1, d.win_ext.1))
                }
            }
            _ => (1.0, 1.0),
        }
    }

    /// 논리 길이 → px (세계 변환·매핑의 평균 배율)
    fn len(&self, v: f64) -> f64 {
        let d = &self.dc;
        let (sx, sy) = self.scale();
        let det = (d.world[0] * d.world[3] - d.world[1] * d.world[2]).abs();
        v.abs() * (det * (sx * sy).abs()).sqrt()
    }

    fn pt(&self, x: f64, y: f64) -> (f64, f64) {
        self.map(x, y)
    }

    fn stroke(&self) -> Option<(f64, [u8; 3], Vec<f64>)> {
        let Obj::Pen {
            style,
            width,
            color,
        } = &self.dc.pen
        else {
            return None;
        };
        let st = style & 0xF;
        if st == 5 {
            return None;
        }
        let w = self.len(*width).max(1.0);
        let dash = match st {
            1 => vec![3.0 * w, w],
            2 => vec![w, w],
            3 => vec![3.0 * w, w, w, w],
            4 => vec![3.0 * w, w, w, w, w, w],
            _ => Vec::new(),
        };
        Some((w, *color, dash))
    }

    /// 펜의 (끝 모양, 꺾임) — 확장 펜의 스타일 비트, 기본은 둥글게
    fn caps(&self) -> (u8, u8) {
        let Obj::Pen { style, .. } = &self.dc.pen else {
            return (1, 1);
        };
        let cap = match style & 0x0F00 {
            0x100 => 2,
            0x200 => 0,
            _ => 1,
        };
        let join = match style & 0xF000 {
            0x1000 => 2,
            0x2000 => 0,
            _ => 1,
        };
        (cap, join)
    }

    /// 빗금 붓이면 (무늬, 색, 바탕색)
    fn hatch(&self) -> Option<Hatch> {
        match &self.dc.brush {
            Obj::Brush {
                style: 2,
                color,
                hatch,
            } if *hatch < 6 => Some((
                *hatch as u8,
                *color,
                self.dc.bk_opaque.then_some(self.dc.bk_color),
            )),
            _ => None,
        }
    }

    fn fill(&self) -> Option<[u8; 3]> {
        match &self.dc.brush {
            Obj::Brush {
                style: 0, color, ..
            } => Some(*color),
            _ => None,
        }
    }

    /// 경로 모으는 중이면 조각을 더하고, 아니면 곧 그린다
    fn emit(&mut self, segs: Vec<Seg>, fill: bool, stroke: bool) {
        if let Some(p) = &mut self.path {
            p.extend(segs);
            return;
        }
        let hatch = if fill { self.hatch() } else { None };
        let (fill, stroke) = (
            fill.then(|| self.fill()).flatten(),
            stroke.then(|| self.stroke()).flatten(),
        );
        if fill.is_none() && stroke.is_none() && hatch.is_none() {
            return;
        }
        self.items.push(Item::Path {
            segs,
            fill,
            stroke,
            even_odd: !self.dc.winding,
            caps: self.caps(),
            hatch,
        });
    }

    fn poly(&mut self, pts: &[(f64, f64)], close: bool, fill: bool) {
        if pts.is_empty() {
            return;
        }
        let mut segs = Vec::with_capacity(pts.len() + 1);
        for (i, &(x, y)) in pts.iter().enumerate() {
            let (x, y) = self.pt(x, y);
            segs.push(if i == 0 {
                Seg::Move(x, y)
            } else {
                Seg::Line(x, y)
            });
        }
        if close {
            segs.push(Seg::Close);
        }
        self.emit(segs, fill && close, true);
        if let Some(&(x, y)) = pts.last() {
            self.dc.cur = (x, y);
        }
    }

    /// 3차 베지어 열 (시작점 포함 3n+1 점)
    fn bezier(&mut self, pts: &[(f64, f64)], with_start: bool) {
        let mut pts = pts.to_vec();
        if !with_start {
            pts.insert(0, self.dc.cur);
        }
        if pts.len() < 4 {
            return;
        }
        let m: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        let mut segs = vec![Seg::Move(m[0].0, m[0].1)];
        for c in m[1..].as_chunks::<3>().0 {
            segs.push(Seg::Cubic(c[0].0, c[0].1, c[1].0, c[1].1, c[2].0, c[2].1));
        }
        self.emit(segs, false, true);
        if let Some(&p) = pts.last() {
            self.dc.cur = p;
        }
    }

    fn rect(&mut self, l: f64, t: f64, r: f64, b: f64) {
        self.poly(&[(l, t), (r, t), (r, b), (l, b)], true, true);
    }

    fn ellipse(&mut self, l: f64, t: f64, r: f64, b: f64) {
        const K: f64 = 0.552_284_749_8;
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let (rx, ry) = ((r - l) / 2.0, (b - t) / 2.0);
        let p = |x: f64, y: f64| self.pt(x, y);
        let (a, b1) = (p(cx + rx, cy), p(cx + rx, cy + K * ry));
        let (c, d) = (p(cx + K * rx, cy + ry), p(cx, cy + ry));
        let (e, f) = (p(cx - K * rx, cy + ry), p(cx - rx, cy + K * ry));
        let (g, h) = (p(cx - rx, cy), p(cx - rx, cy - K * ry));
        let (i, j) = (p(cx - K * rx, cy - ry), p(cx, cy - ry));
        let (k, m) = (p(cx + K * rx, cy - ry), p(cx + rx, cy - K * ry));
        let segs = vec![
            Seg::Move(a.0, a.1),
            Seg::Cubic(b1.0, b1.1, c.0, c.1, d.0, d.1),
            Seg::Cubic(e.0, e.1, f.0, f.1, g.0, g.1),
            Seg::Cubic(h.0, h.1, i.0, i.1, j.0, j.1),
            Seg::Cubic(k.0, k.1, m.0, m.1, a.0, a.1),
            Seg::Close,
        ];
        self.emit(segs, true, true);
    }

    fn round_rect(&mut self, l: f64, t: f64, r: f64, b: f64, cw: f64, ch: f64) {
        const K: f64 = 0.552_284_749_8;
        let (rx, ry) = (
            (cw / 2.0).min((r - l).abs() / 2.0),
            (ch / 2.0).min((b - t).abs() / 2.0),
        );
        if rx <= 0.0 || ry <= 0.0 {
            self.rect(l, t, r, b);
            return;
        }
        let p = |x: f64, y: f64| self.pt(x, y);
        let mut segs = Vec::new();
        let m = |s: (f64, f64)| Seg::Move(s.0, s.1);
        let ln = |s: (f64, f64)| Seg::Line(s.0, s.1);
        let cu =
            |a: (f64, f64), b: (f64, f64), c: (f64, f64)| Seg::Cubic(a.0, a.1, b.0, b.1, c.0, c.1);
        segs.push(m(p(l + rx, t)));
        segs.push(ln(p(r - rx, t)));
        segs.push(cu(
            p(r - rx + K * rx, t),
            p(r, t + ry - K * ry),
            p(r, t + ry),
        ));
        segs.push(ln(p(r, b - ry)));
        segs.push(cu(
            p(r, b - ry + K * ry),
            p(r - rx + K * rx, b),
            p(r - rx, b),
        ));
        segs.push(ln(p(l + rx, b)));
        segs.push(cu(
            p(l + rx - K * rx, b),
            p(l, b - ry + K * ry),
            p(l, b - ry),
        ));
        segs.push(ln(p(l, t + ry)));
        segs.push(cu(
            p(l, t + ry - K * ry),
            p(l + rx - K * rx, t),
            p(l + rx, t),
        ));
        segs.push(Seg::Close);
        self.emit(segs, true, true);
    }

    fn text(&mut self, x: f64, y: f64, s: String) {
        if s.trim().is_empty() {
            return;
        }
        let f = self.dc.font.clone();
        let (px, py) = self.pt(x, y);
        let d = &self.dc;
        let rot = d.world[1].atan2(d.world[0]).to_degrees();
        let em_logical = if f.height == 0.0 {
            16.0
        } else {
            f.height.abs()
        };
        // 양수 높이는 칸 높이 (오름 + 내림 ≈ 1.15 em)
        let em_logical = if f.height > 0.0 {
            em_logical / 1.15
        } else {
            em_logical
        };
        let em = self.len(em_logical).max(1.0);
        let al = self.dc.text_align;
        let halign = match al & 6 {
            6 => 1,
            2 => 2,
            _ => 0,
        };
        let valign = match al & 24 {
            24 => 1,
            8 => 2,
            _ => 0,
        };
        self.items.push(Item::Text {
            x: px,
            y: py,
            halign,
            valign,
            em,
            name: f.name,
            bold: f.bold,
            italic: f.italic,
            underline: f.underline,
            strike: f.strike,
            color: self.dc.text_color,
            text: s,
            angle: -f.escapement / 10.0 + rot,
        });
    }

    /// 사각형으로 자르기 시작 (논리 좌표)
    fn clip_rect(&mut self, l: f64, t: f64, r: f64, b: f64) {
        let (x0, y0) = self.pt(l, t);
        let (x1, y1) = self.pt(r, b);
        self.items.push(Item::Clip {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        });
        self.dc.clips += 1;
    }

    /// 저장한 장치 상태로 돌아간다 — 그 뒤에 연 자르기를 닫는다
    fn restore(&mut self, dc: Dc) {
        for _ in dc.clips..self.dc.clips {
            self.items.push(Item::Unclip);
        }
        self.dc = dc;
    }

    /// 열린 자르기를 모두 닫는다
    fn finish(&mut self) {
        for _ in 0..self.dc.clips {
            self.items.push(Item::Unclip);
        }
        self.dc.clips = 0;
    }

    /// 타원 `box` 위의 호 (논리 좌표): 시작·끝 반지름 점의 각도로 방향에 따라 조각 (베지어 ≤ 90°).
    /// 돌려주는 것은 (시작점, 조각들 — 각 조각은 제어점 둘과 끝점)
    fn arc_pieces(
        &self,
        (l, t, r, b): (f64, f64, f64, f64),
        start: (f64, f64),
        end: (f64, f64),
    ) -> Option<ArcPieces> {
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let (rx, ry) = ((r - l).abs() / 2.0, (b - t).abs() / 2.0);
        if rx == 0.0 || ry == 0.0 {
            return None;
        }
        let ang = |p: (f64, f64)| ((p.1 - cy) / ry).atan2((p.0 - cx) / rx);
        let (a0, a1) = (ang(start), ang(end));
        let tau = std::f64::consts::TAU;
        // 화면(y 아래) 에서 반시계 = 각도 감소
        let mut sweep = a1 - a0;
        if self.dc.arc_ccw {
            while sweep >= 0.0 {
                sweep -= tau;
            }
        } else {
            while sweep <= 0.0 {
                sweep += tau;
            }
        }
        let n = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
        let d = sweep / n as f64;
        let k = 4.0 / 3.0 * (d / 4.0).tan();
        let pt_at = |a: f64| (cx + rx * a.cos(), cy + ry * a.sin());
        let tan_at = |a: f64| (-rx * a.sin(), ry * a.cos());
        let mut out = Vec::new();
        for i in 0..n {
            let (s0, s1) = (a0 + d * i as f64, a0 + d * (i + 1) as f64);
            let (p0, p3) = (pt_at(s0), pt_at(s1));
            let (t0, t3) = (tan_at(s0), tan_at(s1));
            out.push([
                (p0.0 + k * t0.0, p0.1 + k * t0.1),
                (p3.0 - k * t3.0, p3.1 - k * t3.1),
                p3,
            ]);
        }
        Some((pt_at(a0), out))
    }

    /// 호·활꼴·부채꼴 (kind 0 호, 1 활꼴, 2 부채꼴)
    fn arc_shape(
        &mut self,
        kind: u8,
        bx: (f64, f64, f64, f64),
        start: (f64, f64),
        end: (f64, f64),
    ) {
        let Some((p0, pieces)) = self.arc_pieces(bx, start, end) else {
            return;
        };
        let mut segs = Vec::new();
        let m = |p: (f64, f64)| self.pt(p.0, p.1);
        if kind == 2 {
            let c = m(((bx.0 + bx.2) / 2.0, (bx.1 + bx.3) / 2.0));
            segs.push(Seg::Move(c.0, c.1));
            let q = m(p0);
            segs.push(Seg::Line(q.0, q.1));
        } else {
            let q = m(p0);
            segs.push(Seg::Move(q.0, q.1));
        }
        for pc in &pieces {
            let (a, b, c) = (m(pc[0]), m(pc[1]), m(pc[2]));
            segs.push(Seg::Cubic(a.0, a.1, b.0, b.1, c.0, c.1));
        }
        if kind != 0 {
            segs.push(Seg::Close);
        }
        self.emit(segs, kind != 0, true);
    }

    fn image(&mut self, x: f64, y: f64, w: f64, h: f64, bmp: Vec<u8>) {
        let (x0, y0) = self.pt(x, y);
        let (x1, y1) = self.pt(x + w, y + h);
        self.items.push(Item::Image {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x1 - x0).abs(),
            h: (y1 - y0).abs(),
            bmp,
        });
    }

    fn select(&mut self, o: Obj) {
        match o {
            Obj::Pen { .. } => self.dc.pen = o,
            Obj::Brush { .. } => self.dc.brush = o,
            Obj::Font(f) => self.dc.font = f,
            Obj::Other => {}
        }
    }

    fn put(&mut self, idx: usize, o: Obj) {
        if self.objects.len() <= idx {
            self.objects.resize(idx + 1, None);
        }
        self.objects[idx] = Some(o);
    }

    fn finish_path(&mut self, fill: bool, stroke: bool) {
        let Some(segs) = self.path.take() else {
            return;
        };
        let hatch = if fill { self.hatch() } else { None };
        let (fill, stroke) = (
            fill.then(|| self.fill()).flatten(),
            stroke.then(|| self.stroke()).flatten(),
        );
        if segs.is_empty() || (fill.is_none() && stroke.is_none() && hatch.is_none()) {
            return;
        }
        self.items.push(Item::Path {
            segs,
            fill,
            stroke,
            even_odd: !self.dc.winding,
            caps: self.caps(),
            hatch,
        });
    }
}

/// DIB (BITMAPINFO + 픽셀) → BMP 파일. 지원하지 않는 압축이면 None
fn dib_to_bmp(info: &[u8], bits: &[u8]) -> Option<Vec<u8>> {
    let b = Bytes(info);
    let hs = b.u32(0)? as usize;
    if hs < 12 || info.len() < hs {
        return None;
    }
    let bmi_len = info.len();
    let mut f = Vec::with_capacity(14 + bmi_len + bits.len());
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&((14 + bmi_len + bits.len()) as u32).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&((14 + bmi_len) as u32).to_le_bytes());
    f.extend_from_slice(info);
    f.extend_from_slice(bits);
    Some(f)
}

// ---------------------------------------------------------------- EMF

fn emf(d: &[u8]) -> Option<Metafile> {
    let b = Bytes(d);
    let (devx, devy) = (b.i32(72)? as f64, b.i32(76)? as f64);
    let (mmx, mmy) = (b.i32(80)? as f64, b.i32(84)? as f64);
    let ppm = if mmx > 0.0 && mmy > 0.0 && devx > 0.0 && devy > 0.0 {
        (devx / mmx, devy / mmy)
    } else {
        (3.78, 3.78)
    };
    // rclFrame (0.01mm) → 기준 장치 px. 비어 있으면 rclBounds
    let fr = [b.i32(24)?, b.i32(28)?, b.i32(32)?, b.i32(36)?];
    let (fx0, fy0, fw, fh) = if fr[2] > fr[0] && fr[3] > fr[1] {
        (
            fr[0] as f64 / 100.0 * ppm.0,
            fr[1] as f64 / 100.0 * ppm.1,
            (fr[2] - fr[0]) as f64 / 100.0 * ppm.0,
            (fr[3] - fr[1]) as f64 / 100.0 * ppm.1,
        )
    } else {
        let bd = [b.i32(8)?, b.i32(12)?, b.i32(16)?, b.i32(20)?];
        (
            bd[0] as f64,
            bd[1] as f64,
            (bd[2] - bd[0] + 1) as f64,
            (bd[3] - bd[1] + 1) as f64,
        )
    };
    if fw <= 0.0 || fh <= 0.0 {
        return None;
    }
    let mut p = Player::new();
    p.origin = (fx0, fy0);
    p.px_per_mm = ppm;
    let mut o = 0usize;
    let end = d.len();
    while o + 8 <= end {
        let (ty, n) = (b.u32(o)?, b.u32(o + 4)? as usize);
        if n < 8 || o + n > end {
            break;
        }
        let r = Bytes(&d[o..o + n]);
        emf_record(&mut p, ty, &r);
        if ty == 14 {
            break;
        }
        o += n;
    }
    p.finish();
    Some(Metafile {
        width: fw,
        height: fh,
        items: p.items,
    })
}

fn emf_record(p: &mut Player, ty: u32, r: &Bytes) {
    let pts32 = |o: usize, n: usize| -> Option<Vec<(f64, f64)>> {
        (0..n)
            .map(|i| Some((r.i32(o + 8 * i)? as f64, r.i32(o + 8 * i + 4)? as f64)))
            .collect()
    };
    let pts16 = |o: usize, n: usize| -> Option<Vec<(f64, f64)>> {
        (0..n)
            .map(|i| Some((r.i16(o + 4 * i)? as f64, r.i16(o + 4 * i + 2)? as f64)))
            .collect()
    };
    let _ = (|| -> Option<()> {
        match ty {
            // 곡선·다각형·꺾은선 (32비트: 2–6, 16비트: 85–89)
            2 | 3 | 4 | 5 | 6 | 85 | 86 | 87 | 88 | 89 => {
                let wide = ty < 80;
                let n = r.u32(24)? as usize;
                let pts = if wide { pts32(28, n)? } else { pts16(28, n)? };
                let kind = if wide { ty } else { ty - 83 };
                match kind {
                    2 => p.bezier(&pts, true),
                    5 => p.bezier(&pts, false),
                    3 => p.poly(&pts, true, true),
                    4 => p.poly(&pts, false, false),
                    6 => {
                        let mut all = vec![p.dc.cur];
                        all.extend(pts);
                        p.poly(&all, false, false);
                    }
                    _ => {}
                }
            }
            // 다중 꺾은선·다중 다각형 (7·8: 32비트, 90·91: 16비트)
            7 | 8 | 90 | 91 => {
                let wide = ty < 80;
                let polys = r.u32(24)? as usize;
                let total = r.u32(28)? as usize;
                let counts: Option<Vec<usize>> = (0..polys)
                    .map(|i| r.u32(32 + 4 * i).map(|v| v as usize))
                    .collect();
                let counts = counts?;
                let base = 32 + 4 * polys;
                let all = if wide {
                    pts32(base, total)?
                } else {
                    pts16(base, total)?
                };
                let closed = ty == 8 || ty == 91;
                let mut segs = Vec::new();
                let mut at = 0;
                for c in counts {
                    for (i, &(x, y)) in all.get(at..at + c)?.iter().enumerate() {
                        let (x, y) = p.pt(x, y);
                        segs.push(if i == 0 {
                            Seg::Move(x, y)
                        } else {
                            Seg::Line(x, y)
                        });
                    }
                    if closed {
                        segs.push(Seg::Close);
                    }
                    at += c;
                }
                p.emit(segs, closed, true);
            }
            9 => p.dc.win_ext = (r.i32(8)? as f64, r.i32(12)? as f64),
            10 => p.dc.win_org = (r.i32(8)? as f64, r.i32(12)? as f64),
            11 => p.dc.vp_ext = (r.i32(8)? as f64, r.i32(12)? as f64),
            12 => p.dc.vp_org = (r.i32(8)? as f64, r.i32(12)? as f64),
            17 => p.dc.map_mode = r.u32(8)?,
            18 => p.dc.bk_opaque = r.u32(8)? != 1,
            25 => p.dc.bk_color = colorref(r.u32(8)?),
            19 => p.dc.winding = r.u32(8)? == 2,
            22 => p.dc.text_align = r.u32(8)?,
            24 => p.dc.text_color = colorref(r.u32(8)?),
            27 => p.dc.cur = (r.i32(8)? as f64, r.i32(12)? as f64),
            33 => p.stack.push(p.dc.clone()),
            34 => {
                let n = r.i32(8)?;
                let mut dc = None;
                // 음수: 상대, 양수: 절대 (대부분 −1)
                let k = if n < 0 {
                    (-n) as usize
                } else {
                    p.stack.len().saturating_sub(n as usize) + 1
                };
                for _ in 0..k.max(1) {
                    dc = p.stack.pop();
                }
                if let Some(mut dc) = dc {
                    // 현재 위치는 복원하지 않는다 (GDI 도 위치는 장치 상태에서 따로)
                    dc.cur = p.dc.cur;
                    p.restore(dc);
                }
            }
            35 => {
                p.dc.world = [
                    r.f32(8)?,
                    r.f32(12)?,
                    r.f32(16)?,
                    r.f32(20)?,
                    r.f32(24)?,
                    r.f32(28)?,
                ];
            }
            36 => {
                let x = [
                    r.f32(8)?,
                    r.f32(12)?,
                    r.f32(16)?,
                    r.f32(20)?,
                    r.f32(24)?,
                    r.f32(28)?,
                ];
                p.dc.world = match r.u32(32)? {
                    1 => IDENTITY,
                    2 => mul(&x, &p.dc.world),
                    3 => mul(&p.dc.world, &x),
                    _ => x,
                };
            }
            37 => {
                let i = r.u32(8)?;
                let o = if i & 0x8000_0000 != 0 {
                    let n = i & 0x7FFF_FFFF;
                    if matches!(n, 10..=17) {
                        Obj::Other
                    } else {
                        stock(n)
                    }
                } else {
                    p.objects.get(i as usize).cloned().flatten()?
                };
                p.select(o);
            }
            38 => {
                // CREATEPEN: ihPen, LOGPEN (style, width POINT, color)
                let idx = r.u32(8)? as usize;
                p.put(
                    idx,
                    Obj::Pen {
                        style: r.u32(12)?,
                        width: r.i32(16)? as f64,
                        color: colorref(r.u32(24)?),
                    },
                );
            }
            95 => {
                // EXTCREATEPEN: ihPen, offBmi, cbBmi, offBits, cbBits, EXTLOGPEN
                let idx = r.u32(8)? as usize;
                p.put(
                    idx,
                    Obj::Pen {
                        style: r.u32(28)?,
                        width: r.u32(32)? as f64,
                        color: colorref(r.u32(40)?),
                    },
                );
            }
            39 => {
                // CREATEBRUSHINDIRECT: ihBrush, LOGBRUSH (style, color, hatch)
                let idx = r.u32(8)? as usize;
                p.put(
                    idx,
                    Obj::Brush {
                        style: r.u32(12)?,
                        color: colorref(r.u32(16)?),
                        hatch: r.u32(20)?,
                    },
                );
            }
            82 => {
                // EXTCREATEFONTINDIRECTW: ihFont, LOGFONTW
                let idx = r.u32(8)? as usize;
                let name: Vec<u16> = (0..32)
                    .map_while(|i| r.u16(12 + 28 + 2 * i))
                    .take_while(|&c| c != 0)
                    .collect();
                p.put(
                    idx,
                    Obj::Font(FontSpec {
                        height: r.i32(12)? as f64,
                        escapement: r.i32(20)? as f64,
                        bold: r.i32(28)? >= 600,
                        italic: r.u8(32)? != 0,
                        underline: r.u8(33)? != 0,
                        strike: r.u8(34)? != 0,
                        name: String::from_utf16_lossy(&name),
                    }),
                );
            }
            40 => {
                let i = r.u32(8)? as usize;
                if let Some(s) = p.objects.get_mut(i) {
                    *s = None;
                }
            }
            42 => p.ellipse(
                r.i32(8)? as f64,
                r.i32(12)? as f64,
                r.i32(16)? as f64,
                r.i32(20)? as f64,
            ),
            43 => p.rect(
                r.i32(8)? as f64,
                r.i32(12)? as f64,
                r.i32(16)? as f64,
                r.i32(20)? as f64,
            ),
            44 => p.round_rect(
                r.i32(8)? as f64,
                r.i32(12)? as f64,
                r.i32(16)? as f64,
                r.i32(20)? as f64,
                r.i32(24)? as f64,
                r.i32(28)? as f64,
            ),
            54 => {
                let to = (r.i32(8)? as f64, r.i32(12)? as f64);
                let from = p.dc.cur;
                p.poly(&[from, to], false, false);
            }
            59 => p.path = Some(Vec::new()),
            60 => {}
            61 => {
                if let Some(path) = &mut p.path {
                    path.push(Seg::Close);
                }
            }
            62 => p.finish_path(true, false),
            63 => p.finish_path(true, true),
            64 => p.finish_path(false, true),
            30 => p.clip_rect(
                r.i32(8)? as f64,
                r.i32(12)? as f64,
                r.i32(16)? as f64,
                r.i32(20)? as f64,
            ),
            // 경로·영역 자르기는 그리지 않는다 (경로는 버린다)
            67 | 75 => {
                if ty == 67 {
                    p.path = None;
                }
            }
            57 => p.dc.arc_ccw = r.u32(8)? != 2,
            // ARC·CHORD·PIE: 상자(16), 시작점(8), 끝점(8). ARCTO(55) 는 한셀 PDF 에 경로가 없다 (그리지 않는다)
            45..=47 => {
                let bx = (
                    r.i32(8)? as f64,
                    r.i32(12)? as f64,
                    r.i32(16)? as f64,
                    r.i32(20)? as f64,
                );
                let st = (r.i32(24)? as f64, r.i32(28)? as f64);
                let en = (r.i32(32)? as f64, r.i32(36)? as f64);
                p.arc_shape(ty as u8 - 45, bx, st, en);
            }
            // ANGLEARC: 중심(8, 12), 반지름(16), 시작·쓸기 각도 (도, 반시계 — 화면에서 위가 +)
            41 => {
                let (cx, cy, rad) = (r.i32(8)? as f64, r.i32(12)? as f64, r.u32(16)? as f64);
                let (a0, sw) = (r.f32(20)?.to_radians(), r.f32(24)?.to_radians());
                let pt_at = |a: f64| (cx + rad * a.cos(), cy - rad * a.sin());
                let from = p.dc.cur;
                let n = (sw.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
                let d = sw / n as f64;
                let k = 4.0 / 3.0 * (d / 4.0).tan();
                let mut segs = Vec::new();
                let m = |q: (f64, f64)| p.pt(q.0, q.1);
                let (f0, s0) = (m(from), m(pt_at(a0)));
                segs.push(Seg::Move(f0.0, f0.1));
                segs.push(Seg::Line(s0.0, s0.1));
                for i in 0..n {
                    let (u0, u1) = (a0 + d * i as f64, a0 + d * (i + 1) as f64);
                    let (q0, q3) = (pt_at(u0), pt_at(u1));
                    let (t0, t3) = (
                        (-rad * u0.sin(), -rad * u0.cos()),
                        (-rad * u1.sin(), -rad * u1.cos()),
                    );
                    let (c1, c2, c3) = (
                        m((q0.0 + k * t0.0, q0.1 + k * t0.1)),
                        m((q3.0 - k * t3.0, q3.1 - k * t3.1)),
                        m(q3),
                    );
                    segs.push(Seg::Cubic(c1.0, c1.1, c2.0, c2.1, c3.0, c3.1));
                }
                p.emit(segs, false, true);
                p.dc.cur = pt_at(a0 + sw);
            }
            81 => {
                // STRETCHDIBITS: bounds, xDest yDest xSrc ySrc cxSrc cySrc, offBmi cbBmi offBits cbBits usage rop cxDest cyDest
                let (xd, yd) = (r.i32(24)? as f64, r.i32(28)? as f64);
                let (off_bmi, cb_bmi) = (r.u32(48)? as usize, r.u32(52)? as usize);
                let (off_bits, cb_bits) = (r.u32(56)? as usize, r.u32(60)? as usize);
                let (cx, cy) = (r.i32(72)? as f64, r.i32(76)? as f64);
                let bmp = dib_to_bmp(
                    r.0.get(off_bmi..off_bmi + cb_bmi)?,
                    r.0.get(off_bits..off_bits + cb_bits)?,
                )?;
                p.image(xd, yd, cx, cy, bmp);
            }
            84 => {
                // EXTTEXTOUTW: bounds(16), mode, exScale, eyScale, EMRTEXT (ref, nChars, offString, options, rcl, offDx)
                let (x, y) = (r.i32(36)? as f64, r.i32(40)? as f64);
                let n = r.u32(44)? as usize;
                let off = r.u32(48)? as usize;
                let u: Vec<u16> = (0..n).map(|i| r.u16(off + 2 * i)).collect::<Option<_>>()?;
                p.text(x, y, String::from_utf16_lossy(&u));
            }
            _ => {}
        }
        Some(())
    })();
}

// ---------------------------------------------------------------- WMF

fn wmf(d: &[u8]) -> Option<Metafile> {
    let b = Bytes(d);
    let (mut at, bbox, inch) = if d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        (
            22usize,
            Some([b.i16(6)?, b.i16(8)?, b.i16(10)?, b.i16(12)?]),
            (b.u16(14)? as f64).max(1.0),
        )
    } else {
        (0usize, None, 96.0)
    };
    // 표준 머리 18 바이트
    at += b.u16(at + 2)? as usize * 2;
    let _ = inch;
    // 틀: 처음 나오는 SETWINDOWORG·SETWINDOWEXT (없으면 bbox)
    let (mut org, mut ext) = (None, None);
    let mut o = at;
    while o + 6 <= d.len() {
        let n = b.u32(o)? as usize * 2;
        let f = b.u16(o + 4)?;
        if n < 6 {
            break;
        }
        match f {
            0x020B if org.is_none() => org = Some((b.i16(o + 8)? as f64, b.i16(o + 6)? as f64)),
            0x020C if ext.is_none() => ext = Some((b.i16(o + 8)? as f64, b.i16(o + 6)? as f64)),
            0 => break,
            _ => {}
        }
        o += n;
    }
    let (org, ext) = match (org, ext, bbox) {
        (Some(o), Some(e), _) => (o, e),
        (_, _, Some(bb)) => (
            (bb[0] as f64, bb[1] as f64),
            ((bb[2] - bb[0]) as f64, (bb[3] - bb[1]) as f64),
        ),
        (Some(o), None, None) => (o, (1000.0, 1000.0)),
        _ => return None,
    };
    if ext.0 == 0.0 || ext.1 == 0.0 {
        return None;
    }
    let mut p = Player::new();
    p.wmf = true;
    p.dc.map_mode = 8;
    p.dc.win_org = org;
    p.dc.win_ext = ext;
    let mut o = at;
    while o + 6 <= d.len() {
        let n = b.u32(o)? as usize * 2;
        if n < 6 || o + n > d.len() {
            break;
        }
        let f = b.u16(o + 4)?;
        if f == 0 {
            break;
        }
        wmf_record(&mut p, f, &Bytes(&d[o + 6..o + n]));
        o += n;
    }
    p.finish();
    Some(Metafile {
        width: ext.0.abs(),
        height: ext.1.abs(),
        items: p.items,
    })
}

fn wmf_record(p: &mut Player, f: u16, r: &Bytes) {
    // 인수: 6 바이트 머리 뒤. 점은 (x, y) 쌍이지만 단일 점 인수는 y, x 차례
    let _ = (|| -> Option<()> {
        match f {
            0x0102 => p.dc.bk_opaque = r.u16(0)? != 1,
            0x0103 => {}
            0x0201 => p.dc.bk_color = colorref(r.u32(0)?),
            0x0106 => p.dc.winding = r.u16(0)? == 2,
            0x012E => p.dc.text_align = r.u16(0)? as u32,
            0x0209 => p.dc.text_color = colorref(r.u32(0)?),
            0x020B => p.dc.win_org = (r.i16(2)? as f64, r.i16(0)? as f64),
            0x020C => p.dc.win_ext = (r.i16(2)? as f64, r.i16(0)? as f64),
            0x001E => p.stack.push(p.dc.clone()),
            0x0127 => {
                let n = r.i16(0)?;
                let k = if n < 0 { (-n) as usize } else { 1 };
                let mut dc = None;
                for _ in 0..k {
                    dc = p.stack.pop();
                }
                if let Some(dc) = dc {
                    p.restore(dc);
                }
            }
            0x0214 => p.dc.cur = (r.i16(2)? as f64, r.i16(0)? as f64),
            0x0213 => {
                let to = (r.i16(2)? as f64, r.i16(0)? as f64);
                let from = p.dc.cur;
                p.poly(&[from, to], false, false);
            }
            0x02FA => {
                // CREATEPENINDIRECT: style, width x, y, color
                let o = Obj::Pen {
                    style: r.u16(0)? as u32,
                    width: r.i16(2)? as f64,
                    color: colorref(r.u32(6)?),
                };
                wmf_put(p, o);
            }
            0x02FC => {
                let o = Obj::Brush {
                    style: r.u16(0)? as u32,
                    color: colorref(r.u32(2)?),
                    hatch: r.u16(6).unwrap_or(0) as u32,
                };
                wmf_put(p, o);
            }
            0x02FB => {
                // CREATEFONTINDIRECT
                let name: Vec<u8> = (0..32)
                    .map_while(|i| r.u8(18 + i))
                    .take_while(|&c| c != 0)
                    .collect();
                let o = Obj::Font(FontSpec {
                    height: r.i16(0)? as f64,
                    escapement: r.i16(4)? as f64,
                    bold: r.i16(8)? >= 600,
                    italic: r.u8(10)? != 0,
                    underline: r.u8(11)? != 0,
                    strike: r.u8(12)? != 0,
                    name: name.iter().map(|&c| c as char).collect(),
                });
                wmf_put(p, o);
            }
            0x012D => {
                let i = r.u16(0)? as usize;
                let o = p.objects.get(i).cloned().flatten()?;
                p.select(o);
            }
            0x01F0 => {
                let i = r.u16(0)? as usize;
                if let Some(s) = p.objects.get_mut(i) {
                    *s = None;
                }
            }
            0x0324 | 0x0325 => {
                let n = r.u16(0)? as usize;
                let pts: Vec<(f64, f64)> = (0..n)
                    .map(|i| Some((r.i16(2 + 4 * i)? as f64, r.i16(4 + 4 * i)? as f64)))
                    .collect::<Option<_>>()?;
                p.poly(&pts, f == 0x0324, true);
            }
            0x0538 => {
                let polys = r.u16(0)? as usize;
                let counts: Vec<usize> = (0..polys)
                    .map(|i| r.u16(2 + 2 * i).map(|v| v as usize))
                    .collect::<Option<_>>()?;
                let base = 2 + 2 * polys;
                let mut segs = Vec::new();
                let mut at = 0;
                for c in counts {
                    for i in 0..c {
                        let (x, y) = (
                            r.i16(base + 4 * (at + i))? as f64,
                            r.i16(base + 4 * (at + i) + 2)? as f64,
                        );
                        let (x, y) = p.pt(x, y);
                        segs.push(if i == 0 {
                            Seg::Move(x, y)
                        } else {
                            Seg::Line(x, y)
                        });
                    }
                    segs.push(Seg::Close);
                    at += c;
                }
                p.emit(segs, true, true);
            }
            0x041B => p.rect(
                r.i16(6)? as f64,
                r.i16(4)? as f64,
                r.i16(2)? as f64,
                r.i16(0)? as f64,
            ),
            0x0418 => p.ellipse(
                r.i16(6)? as f64,
                r.i16(4)? as f64,
                r.i16(2)? as f64,
                r.i16(0)? as f64,
            ),
            0x061C => p.round_rect(
                r.i16(10)? as f64,
                r.i16(8)? as f64,
                r.i16(6)? as f64,
                r.i16(4)? as f64,
                r.i16(2)? as f64,
                r.i16(0)? as f64,
            ),
            0x0521 => {
                // TEXTOUT: 길이, 글(짝수로 채움), y, x
                let n = r.u16(0)? as usize;
                let s: String = (0..n)
                    .map(|i| r.u8(2 + i).map(|c| c as char))
                    .collect::<Option<_>>()?;
                let at = 2 + n + (n & 1);
                p.text(r.i16(at + 2)? as f64, r.i16(at)? as f64, s);
            }
            0x0A32 => {
                // EXTTEXTOUT: y, x, 길이, 옵션, [사각형], 글
                let (y, x) = (r.i16(0)? as f64, r.i16(2)? as f64);
                let n = r.u16(4)? as usize;
                let opts = r.u16(6)?;
                let at = 8 + if opts & 6 != 0 { 8 } else { 0 };
                let s: String = (0..n)
                    .map(|i| r.u8(at + i).map(|c| c as char))
                    .collect::<Option<_>>()?;
                p.text(x, y, s);
            }
            0x0F43 => {
                // STRETCHDIB: rop(4), usage, srcH, srcW, ySrc, xSrc, destH, destW, yDest, xDest, DIB
                let (dh, dw, yd, xd) = (
                    r.i16(14)? as f64,
                    r.i16(16)? as f64,
                    r.i16(18)? as f64,
                    r.i16(20)? as f64,
                );
                let dib = r.0.get(22..)?;
                let bmp = wmf_dib(dib)?;
                p.image(xd, yd, dw, dh, bmp);
            }
            0x0416 => p.clip_rect(
                r.i16(6)? as f64,
                r.i16(4)? as f64,
                r.i16(2)? as f64,
                r.i16(0)? as f64,
            ),
            // ARC·PIE·CHORD
            0x0817 | 0x081A | 0x0830 => {
                let bx = (
                    r.i16(14)? as f64,
                    r.i16(12)? as f64,
                    r.i16(10)? as f64,
                    r.i16(8)? as f64,
                );
                let st = (r.i16(6)? as f64, r.i16(4)? as f64);
                let en = (r.i16(2)? as f64, r.i16(0)? as f64);
                let kind = match f {
                    0x0817 => 0,
                    0x0830 => 1,
                    _ => 2,
                };
                p.arc_shape(kind, bx, st, en);
            }
            0x0B41 => {
                // DIBSTRETCHBLT: rop(4), srcH, srcW, ySrc, xSrc, destH, destW, yDest, xDest, DIB
                let (dh, dw, yd, xd) = (
                    r.i16(12)? as f64,
                    r.i16(14)? as f64,
                    r.i16(16)? as f64,
                    r.i16(18)? as f64,
                );
                let dib = r.0.get(20..)?;
                let bmp = wmf_dib(dib)?;
                p.image(xd, yd, dw, dh, bmp);
            }
            _ => {}
        }
        Some(())
    })();
}

/// WMF 개체는 가장 낮은 빈 자리에 들어간다
fn wmf_put(p: &mut Player, o: Obj) {
    match p.objects.iter().position(Option::is_none) {
        Some(i) => p.objects[i] = Some(o),
        None => p.objects.push(Some(o)),
    }
}

/// 묶음 DIB (머리 + 색표 + 픽셀) → BMP
fn wmf_dib(dib: &[u8]) -> Option<Vec<u8>> {
    let b = Bytes(dib);
    let hs = b.u32(0)? as usize;
    let bpp = b.u16(14)? as usize;
    let used = b.u32(32)? as usize;
    let colors = if used != 0 {
        used
    } else if bpp <= 8 {
        1 << bpp
    } else {
        0
    };
    let bitfields = if b.u32(16)? == 3 && hs == 40 { 12 } else { 0 };
    let head = hs + colors * 4 + bitfields;
    dib_to_bmp(dib.get(..head)?, dib.get(head..)?)
}

// ---------------------------------------------------------------- SVG (화면)

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 빗금 무늬 8×8 칸의 선 (SVG 는 y 가 아래)
fn hatch_svg(style: u8) -> &'static str {
    match style {
        0 => "M0 4H8",
        1 => "M4 0V8",
        2 => "M0 0L8 8M0 8L8 16M0 -8L8 0",
        3 => "M0 8L8 0M0 16L8 8M8 8L16 0",
        4 => "M0 4H8M4 0V8",
        _ => "M0 0L8 8M0 8L8 16M0 -8L8 0M0 8L8 0M0 16L8 8M8 8L16 0",
    }
}

fn path_d(segs: &[Seg]) -> String {
    use std::fmt::Write as _;
    let mut d = String::new();
    for s in segs {
        let _ = match s {
            Seg::Move(x, y) => write!(d, "M{x:.2} {y:.2}"),
            Seg::Line(x, y) => write!(d, "L{x:.2} {y:.2}"),
            Seg::Cubic(a, b, c, e, f, g) => write!(d, "C{a:.2} {b:.2} {c:.2} {e:.2} {f:.2} {g:.2}"),
            Seg::Close => write!(d, "Z"),
        };
    }
    d
}

/// 메타파일 → SVG (틀 전체를 viewBox 로, `preserveAspectRatio="none"` — 그림 상자에 늘어난다)
pub fn to_svg(m: &Metafile) -> String {
    use std::fmt::Write as _;
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 {:.2} {:.2}\" preserveAspectRatio=\"none\">",
        m.width, m.height
    );
    let mut clips = 0;
    let mut hatches = 0;
    for it in &m.items {
        match it {
            Item::Path {
                segs,
                fill,
                stroke,
                even_odd,
                caps,
                hatch,
            } => {
                let mut fill = fill.map(hex);
                if let Some((style, fg, bg)) = hatch {
                    hatches += 1;
                    let id = format!("mh{hatches}");
                    let _ = write!(
                        s,
                        "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" width=\"8\" height=\"8\">"
                    );
                    if let Some(b) = bg {
                        let _ = write!(s, "<rect width=\"8\" height=\"8\" fill=\"{}\"/>", hex(*b));
                    }
                    let _ = write!(
                        s,
                        "<path d=\"{}\" stroke=\"{}\" stroke-width=\"1\" fill=\"none\"/></pattern>",
                        hatch_svg(*style),
                        hex(*fg)
                    );
                    fill = Some(format!("url(#{id})"));
                }
                let _ = write!(s, "<path d=\"{}\"", path_d(segs));
                match &fill {
                    Some(c) => {
                        let _ = write!(s, " fill=\"{c}\"");
                        if *even_odd {
                            s.push_str(" fill-rule=\"evenodd\"");
                        }
                    }
                    None => s.push_str(" fill=\"none\""),
                }
                if let Some((w, c, dash)) = stroke {
                    let _ = write!(s, " stroke=\"{}\" stroke-width=\"{w:.2}\"", hex(*c));
                    if !dash.is_empty() {
                        let d: Vec<String> = dash.iter().map(|v| format!("{v:.2}")).collect();
                        let _ = write!(s, " stroke-dasharray=\"{}\"", d.join(" "));
                    }
                    let _ = write!(
                        s,
                        " stroke-linecap=\"{}\" stroke-linejoin=\"{}\"",
                        ["butt", "round", "square"][caps.0 as usize % 3],
                        ["miter", "round", "bevel"][caps.1 as usize % 3]
                    );
                }
                s.push_str("/>");
            }
            Item::Text {
                x,
                y,
                halign,
                valign,
                em,
                name,
                bold,
                italic,
                underline,
                strike,
                color,
                text,
                angle,
                ..
            } => {
                let anchor = ["start", "middle", "end"][*halign as usize % 3];
                let base =
                    ["text-before-edge", "alphabetic", "text-after-edge"][*valign as usize % 3];
                let mut deco = Vec::new();
                if *underline {
                    deco.push("underline");
                }
                if *strike {
                    deco.push("line-through");
                }
                let rot = if *angle != 0.0 {
                    format!(" transform=\"rotate({angle:.2} {x:.2} {y:.2})\"")
                } else {
                    String::new()
                };
                let _ = write!(
                    s,
                    "<text x=\"{x:.2}\" y=\"{y:.2}\" font-family=\"{}\" font-size=\"{em:.2}\" fill=\"{}\" text-anchor=\"{anchor}\" dominant-baseline=\"{base}\" xml:space=\"preserve\"{}{}{}{rot}>{}</text>",
                    esc(name),
                    hex(*color),
                    if *bold { " font-weight=\"bold\"" } else { "" },
                    if *italic {
                        " font-style=\"italic\""
                    } else {
                        ""
                    },
                    if deco.is_empty() {
                        String::new()
                    } else {
                        format!(" text-decoration=\"{}\"", deco.join(" "))
                    },
                    esc(text)
                );
            }
            Item::Clip { x0, y0, x1, y1 } => {
                clips += 1;
                let _ = write!(
                    s,
                    "<clipPath id=\"mc{clips}\"><rect x=\"{x0:.2}\" y=\"{y0:.2}\" width=\"{:.2}\" height=\"{:.2}\"/></clipPath><g clip-path=\"url(#mc{clips})\">",
                    x1 - x0,
                    y1 - y0
                );
            }
            Item::Unclip => s.push_str("</g>"),
            Item::Image { x, y, w, h, bmp } => {
                let _ = write!(
                    s,
                    "<image x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" preserveAspectRatio=\"none\" href=\"data:image/bmp;base64,{}\"/>",
                    super::draw::base64(bmp)
                );
            }
        }
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시험용 EMF 만들기: 머리(틀 400×300 px, 기준 장치 2560×1440 px = 597×336 mm 에서 틀 mm) + 레코드
    fn emf_with(records: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let rec = |ty: u32, body: &[u8]| {
            let mut r = Vec::new();
            r.extend((ty).to_le_bytes());
            r.extend(((8 + body.len()) as u32).to_le_bytes());
            r.extend(body);
            r
        };
        let mut h = Vec::new();
        for v in [0i32, 0, 399, 299] {
            h.extend(v.to_le_bytes());
        }
        // rclFrame: 400×300 px 를 0.01mm 로 (597mm / 2560px)
        for v in [
            0i32,
            0,
            (400.0 * 597.0 / 2560.0 * 100.0) as i32,
            (300.0 * 336.0 / 1440.0 * 100.0) as i32,
        ] {
            h.extend(v.to_le_bytes());
        }
        h.extend(0x464D_4520u32.to_le_bytes());
        h.extend(0x10000u32.to_le_bytes());
        h.extend([0u8; 4 + 4 + 2 + 2 + 4 + 4 + 4]);
        for v in [2560i32, 1440, 597, 336] {
            h.extend(v.to_le_bytes());
        }
        out.extend(rec(1, &h));
        for (ty, body) in records {
            out.extend(rec(*ty, body));
        }
        out.extend(rec(14, &[0; 12]));
        out
    }

    fn i32s(v: &[i32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn emf_rectangle_and_polygon() {
        // 붓 (파랑) 선택 + 사각형, 16비트 다각형
        let mut brush = i32s(&[1, 0]);
        brush.extend(0x00FF_0000u32.to_le_bytes()); // COLORREF 0x00BBGGRR → 파랑
        brush.extend([0; 4]);
        let mut poly = i32s(&[0, 0, 0, 0, 3]);
        for v in [10i16, 10, 50, 10, 30, 40] {
            poly.extend(v.to_le_bytes());
        }
        let d = emf_with(&[
            (39, brush),
            (37, 1u32.to_le_bytes().to_vec()),
            (43, i32s(&[20, 30, 120, 80])),
            (86, poly),
        ]);
        assert!(is_metafile(&d));
        let m = parse(&d).unwrap();
        assert!((m.width - 400.0).abs() < 0.6 && (m.height - 300.0).abs() < 0.6);
        assert_eq!(m.items.len(), 2);
        let Item::Path {
            segs, fill, stroke, ..
        } = &m.items[0]
        else {
            panic!()
        };
        assert_eq!(*fill, Some([0, 0, 255]));
        assert_eq!(stroke.as_ref().map(|s| s.1), Some([0, 0, 0]));
        assert_eq!(segs[0], Seg::Move(20.0, 30.0));
        assert_eq!(segs.last(), Some(&Seg::Close));
        let svg = to_svg(&m);
        assert!(svg.contains("viewBox=\"0 0 ") && svg.contains("fill=\"#0000FF\""));
    }

    #[test]
    fn emf_world_transform_and_text() {
        // 세계 변환 2 배 (왼쪽 곱) 뒤 글자 (위 맞춤): 기준점 (10, 5) → (20, 10), 크기 12 → 24
        let mut xf = Vec::new();
        for v in [2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0] {
            xf.extend(v.to_le_bytes());
        }
        xf.extend(2u32.to_le_bytes());
        // ihFont, LOGFONTW (높이 −12 … 무게 400, 문자 속성 8 바이트, 글꼴 이름 64 바이트)
        let mut font = i32s(&[3, -12, 0, 0, 0, 400]);
        font.extend([0u8; 8]);
        let name: Vec<u8> = "Arial"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        font.extend(&name);
        font.resize(32 + 64, 0);
        let mut text = i32s(&[0, 0, 0, 0, 1, 1, 1]);
        text.extend(i32s(&[10, 5, 2, 76, 0, 0, 0, 0, 0, 0]));
        text.extend([b'H', 0, b'i', 0]);
        let d = emf_with(&[
            (36, xf),
            (82, font),
            (37, 3u32.to_le_bytes().to_vec()),
            (84, text),
        ]);
        let m = parse(&d).unwrap();
        let Some(Item::Text {
            x,
            y,
            em,
            text,
            name,
            ..
        }) = m.items.first()
        else {
            panic!("{:?}", m.items)
        };
        assert_eq!((text.as_str(), name.as_str()), ("Hi", "Arial"));
        assert!((x - 20.0).abs() < 1e-9 && (y - 10.0).abs() < 1e-9 && (em - 24.0).abs() < 1e-9);
    }

    #[test]
    fn wmf_placeable_polygon() {
        // 놓을 수 있는 WMF: bbox 0,0,100,50 — 창 크기 그대로, 다각형 하나
        let rec = |f: u16, params: &[i16]| {
            let mut r = Vec::new();
            r.extend((3 + params.len() as u32).to_le_bytes());
            r.extend(f.to_le_bytes());
            for p in params {
                r.extend(p.to_le_bytes());
            }
            r
        };
        let mut d = vec![0xD7, 0xCD, 0xC6, 0x9A, 0, 0];
        for v in [0i16, 0, 100, 50] {
            d.extend(v.to_le_bytes());
        }
        d.extend(96u16.to_le_bytes());
        d.extend([0; 6]);
        d.extend([1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        d.extend(rec(0x020B, &[0, 0]));
        d.extend(rec(0x020C, &[50, 100]));
        d.extend(rec(0x0324, &[3, 10, 10, 90, 10, 50, 40]));
        d.extend(rec(0, &[]));
        let m = parse(&d).unwrap();
        assert_eq!((m.width, m.height), (100.0, 50.0));
        let Item::Path { segs, .. } = &m.items[0] else {
            panic!()
        };
        assert_eq!(segs[1], Seg::Line(90.0, 10.0));
    }

    /// ARC·PIE·CHORD 의 본문: 타원 상자 4, 시작점 2, 끝점 2
    fn arc_body(bx: [i32; 4], st: [i32; 2], en: [i32; 2]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(bx);
        v.extend(st);
        v.extend(en);
        i32s(&v)
    }

    fn cubic_count(segs: &[Seg]) -> usize {
        segs.iter().filter(|s| matches!(s, Seg::Cubic(..))).count()
    }

    #[test]
    fn emf_arc_pie_chord() {
        // 중심 (150, 150), 반지름 50: 동쪽(200,150)에서 북쪽(150,100) 까지. 기본 방향은 반시계 → 90° 한 조각
        let body = arc_body([100, 100, 200, 200], [200, 150], [150, 100]);
        let m = parse(&emf_with(&[(45, body.clone())])).unwrap();
        let Item::Path {
            segs, fill, stroke, ..
        } = &m.items[0]
        else {
            panic!()
        };
        assert!(fill.is_none() && stroke.is_some());
        assert_eq!(segs[0], Seg::Move(200.0, 150.0));
        assert_eq!(cubic_count(segs), 1);
        let Some(Seg::Cubic(_, _, _, _, x, y)) = segs.last() else {
            panic!()
        };
        assert!((x - 150.0).abs() < 1e-6 && (y - 100.0).abs() < 1e-6);
        // 시계 방향으로 바꾸면 같은 점들 사이 270° → 세 조각
        let m = parse(&emf_with(&[
            (57, 2u32.to_le_bytes().to_vec()),
            (45, body.clone()),
        ]))
        .unwrap();
        let Item::Path { segs, .. } = &m.items[0] else {
            panic!()
        };
        assert_eq!(cubic_count(segs), 3);
        // 부채꼴: 중심에서 시작, 닫고 채운다 (기본 붓은 흰색)
        let m = parse(&emf_with(&[(47, body.clone())])).unwrap();
        let Item::Path { segs, fill, .. } = &m.items[0] else {
            panic!()
        };
        assert!(fill.is_some());
        assert_eq!(segs[0], Seg::Move(150.0, 150.0));
        assert_eq!(segs[1], Seg::Line(200.0, 150.0));
        assert_eq!(segs.last(), Some(&Seg::Close));
        // 활꼴: 호의 시작에서 시작해 닫는다
        let m = parse(&emf_with(&[(46, body)])).unwrap();
        let Item::Path { segs, .. } = &m.items[0] else {
            panic!()
        };
        assert_eq!(segs[0], Seg::Move(200.0, 150.0));
        assert_eq!(segs.last(), Some(&Seg::Close));
    }

    #[test]
    fn emf_clip_rect_is_balanced() {
        // 자르기 → 사각형 → (저장·복원 없이) 끝에서 닫힌다
        let d = emf_with(&[(30, i32s(&[10, 10, 100, 100])), (43, i32s(&[0, 0, 50, 50]))]);
        let m = parse(&d).unwrap();
        assert!(
            matches!(m.items[0], Item::Clip { x0, y0, x1, y1 } if (x0, y0, x1, y1) == (10.0, 10.0, 100.0, 100.0))
        );
        assert!(matches!(m.items[1], Item::Path { .. }));
        assert!(matches!(m.items[2], Item::Unclip));
        // 저장한 뒤 연 자르기는 복원에서 닫힌다
        let d = emf_with(&[
            (33, vec![]),
            (30, i32s(&[10, 10, 100, 100])),
            (43, i32s(&[0, 0, 50, 50])),
            (34, (-1i32).to_le_bytes().to_vec()),
            (43, i32s(&[0, 0, 5, 5])),
        ]);
        let m = parse(&d).unwrap();
        let kinds: Vec<&str> = m
            .items
            .iter()
            .map(|i| match i {
                Item::Clip { .. } => "clip",
                Item::Unclip => "unclip",
                Item::Path { .. } => "path",
                _ => "?",
            })
            .collect();
        assert_eq!(kinds, ["clip", "path", "unclip", "path"]);
        // SVG 는 clipPath 와 짝이 맞는 g 로
        let svg = to_svg(&m);
        assert_eq!(
            svg.matches("<g clip-path").count(),
            svg.matches("</g>").count()
        );
    }

    #[test]
    fn wmf_text_and_arc() {
        let rec = |f: u16, params: &[i16]| {
            let mut r = Vec::new();
            r.extend((3 + params.len() as u32).to_le_bytes());
            r.extend(f.to_le_bytes());
            for p in params {
                r.extend(p.to_le_bytes());
            }
            r
        };
        let mut d = vec![0xD7, 0xCD, 0xC6, 0x9A, 0, 0];
        for v in [0i16, 0, 100, 50] {
            d.extend(v.to_le_bytes());
        }
        d.extend(96u16.to_le_bytes());
        d.extend([0; 6]);
        d.extend([1, 0, 9, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        d.extend(rec(0x020B, &[0, 0]));
        d.extend(rec(0x020C, &[50, 100]));
        // TEXTOUT "Hi": 길이 2, 글, y 20, x 10
        d.extend(rec(0x0521, &[2, i16::from_le_bytes(*b"Hi"), 20, 10]));
        // ARC: 상자 (0,0)-(100,50), 시작 (100,25), 끝 (50,0)
        d.extend(rec(0x0817, &[0, 50, 25, 100, 50, 100, 0, 0]));
        d.extend(rec(0, &[]));
        let m = parse(&d).unwrap();
        assert!(matches!(m.items[0], Item::Text { .. }));
        let Item::Path { segs, .. } = &m.items[1] else {
            panic!("{:?}", m.items)
        };
        assert_eq!(segs[0], Seg::Move(100.0, 25.0));
        assert_eq!(cubic_count(segs), 1);
    }

    #[test]
    fn emf_hatch_brush_and_pen_caps() {
        // 빗금 붓 (BS_HATCHED, 무늬 1 = 세로, 파랑) + 불투명 바탕 흰색 → 빗금 채우기; 바탕 모드 투명이면 바탕색 없음
        let mut brush = i32s(&[1, 2]);
        brush.extend(0x00FF_0000u32.to_le_bytes());
        brush.extend(1u32.to_le_bytes());
        let d = emf_with(&[
            (39, brush),
            (37, 1u32.to_le_bytes().to_vec()),
            (43, i32s(&[0, 0, 50, 50])),
            (18, 1u32.to_le_bytes().to_vec()),
            (43, i32s(&[60, 0, 110, 50])),
        ]);
        let m = parse(&d).unwrap();
        let Item::Path {
            fill, hatch, caps, ..
        } = &m.items[0]
        else {
            panic!()
        };
        assert!(fill.is_none());
        assert_eq!(*hatch, Some((1, [0, 0, 255], Some([255, 255, 255]))));
        // CREATEPEN 기본 펜: 끝·꺾임 둥글게
        assert_eq!(*caps, (1, 1));
        let Item::Path { hatch, .. } = &m.items[1] else {
            panic!()
        };
        assert_eq!(*hatch, Some((1, [0, 0, 255], None)));
        let svg = to_svg(&m);
        assert!(svg.contains("<pattern id=\"mh1\"") && svg.contains("fill=\"url(#mh2)\""));
    }

    #[test]
    fn garbage_is_not_a_metafile() {
        assert!(parse(b"not a picture").is_none());
        assert!(parse(&[1, 0, 0, 0, 8, 0, 0, 0]).is_none());
    }
}
