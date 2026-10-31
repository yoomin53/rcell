// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그리기 목록([`Op`]) → PDF. 한셀 PDF 처럼 페이지마다 `0.12 0 0 -0.12 0 H cm` (600 dpi 장치 단위, 위가 0) 아래에
//! 그리고, 글꼴은 쓴 글자만 남긴 TrueType 을 CIDFontType2 + Identity-H 로 넣는다 (pdf-writer, subsetter).
//! 글꼴 파일이 없으면 (웹 어셈블리) 한국어 표준 CID 글꼴 이름(UniKS-UCS2-H)만 적어 보는 프로그램이 고르게 한다.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use pdf_writer::types::{CidFontType, FontFlags, PaintType, SystemInfo, TilingType, UnicodeCmap};
use pdf_writer::{Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};

use super::font::{FaceRef, FontDb, LoadedFace};
use super::pdfimage::{PdfImage, decode};
use crate::render::metafile::Hatch;
use crate::render::page::{FontKey, Op, Seg, TextMeasure};

/// 페이지들이 쓴 그림 (같은 바이트는 한 번만)
#[derive(Default)]
struct Images {
    list: Vec<PdfImage>,
    by_ptr: HashMap<usize, Option<usize>>,
}

impl Images {
    /// 이미 풀린 그림 (그려 낸 비트맵)
    fn push(&mut self, img: PdfImage) -> usize {
        self.list.push(img);
        self.list.len() - 1
    }

    fn add(&mut self, data: &std::sync::Arc<Vec<u8>>) -> Option<usize> {
        let key = std::sync::Arc::as_ptr(data) as usize;
        if let Some(&k) = self.by_ptr.get(&key) {
            return k;
        }
        let k = decode(data).map(|img| {
            self.list.push(img);
            self.list.len() - 1
        });
        self.by_ptr.insert(key, k);
        k
    }
}

/// 페이지들이 쓴 빗금 무늬 (같은 것은 한 번만)
#[derive(Default)]
struct Hatches {
    list: Vec<Hatch>,
}

impl Hatches {
    fn add(&mut self, h: Hatch) -> usize {
        match self.list.iter().position(|x| *x == h) {
            Some(k) => k,
            None => {
                self.list.push(h);
                self.list.len() - 1
            }
        }
    }
}

/// 빗금 무늬 8×8 칸의 내용 (PDF 의 y 는 위). 한셀 PDF 의 Tiling 무늬와 같은 꼴: 칸으로 자르고, 불투명이면 바탕색을
/// 깔고, 1 굵기 둥근 선. 무늬 번호 0 가로·1 세로·2 \·3 /·4 +·5 x (선이 칸 밖으로 나가는 것은 이웃 칸과 이어지게)
fn hatch_tile(h: &Hatch) -> String {
    let (style, fg, bg) = h;
    let lines = match style {
        0 => "0 4 m 8 4 l",
        1 => "4 0 m 4 8 l",
        2 => "0 0 m 8 -8 l 0 8 m 8 0 l 8 8 m 16 0 l",
        3 => "0 0 m 8 8 l 0 -8 m 8 0 l 0 8 m 8 16 l",
        4 => "0 4 m 8 4 l 4 0 m 4 8 l",
        _ => "0 0 m 8 -8 l 0 8 m 8 0 l 8 8 m 16 0 l 0 0 m 8 8 l 0 -8 m 8 0 l 0 8 m 8 16 l",
    };
    let mut s = String::from("q\n1 i\n0 0 8 8 re\nW n\n");
    if let Some(b) = bg {
        let _ = write!(s, "{} rg\n0 0 8 8 re\nf\n", color(*b));
    }
    let _ = write!(
        s,
        "{} RG\n1 J 1 j 1 w 10 M []0 d\n{lines}\nS\nQ\n",
        color(*fg)
    );
    s
}

/// 넣은 글꼴 하나 (얼굴 + 쓴 글리프)
struct Embedded {
    face: LoadedFace,
    remap: subsetter::GlyphRemapper,
    /// 새 글리프 → (폭 1/1000 em, 글자)
    glyphs: BTreeMap<u16, (f64, char)>,
}

/// 글꼴 고르기·글리프 모으기 (글자 폭 재기도 한다)
pub struct FontSet<'a> {
    db: &'a FontDb,
    state: RefCell<State>,
}

#[derive(Default)]
struct State {
    /// 얼굴 번호 → 넣은 글꼴 번호 (읽지 못한 얼굴은 None)
    by_face: HashMap<usize, Option<usize>>,
    fonts: Vec<Embedded>,
    select: HashMap<FontKey, Option<FaceRef>>,
    fallback: HashMap<(bool, bool), Option<FaceRef>>,
}

/// 글자 한 토막: 같은 글꼴로 이어 쓰는 글리프들
pub struct Run {
    /// 넣은 글꼴 번호 (None = 글꼴 파일 없음: 표준 CID 글꼴로 유니코드 그대로)
    pub font: Option<usize>,
    pub glyphs: Vec<u16>,
    /// 토막 폭 (1/1000 em)
    pub advance: f64,
    pub synth_bold: bool,
    pub synth_italic: bool,
    pub text: String,
}

/// 글꼴 파일이 없을 때 글자 폭 (1/1000 em): 전각 1000, 그 밖 500
fn plain_width(ch: char) -> f64 {
    if crate::render::is_wide(ch) {
        1000.0
    } else {
        550.0
    }
}

impl<'a> FontSet<'a> {
    pub fn new(db: &'a FontDb) -> FontSet<'a> {
        FontSet {
            db,
            state: RefCell::new(State::default()),
        }
    }

    fn embedded(st: &mut State, db: &FontDb, face: usize) -> Option<usize> {
        if let Some(&e) = st.by_face.get(&face) {
            return e;
        }
        let e = db
            .data(face)
            .and_then(|(d, i)| LoadedFace::load(d, i))
            .map(|lf| {
                st.fonts.push(Embedded {
                    face: lf,
                    remap: subsetter::GlyphRemapper::new(),
                    glyphs: BTreeMap::new(),
                });
                st.fonts.len() - 1
            });
        st.by_face.insert(face, e);
        e
    }

    /// 글자들을 글꼴 토막으로 (없는 글자는 대체 글꼴). `keep` 이면 글리프를 부분 집합에 넣는다
    pub fn runs(&self, key: &FontKey, text: &str, keep: bool) -> Vec<Run> {
        let mut st = self.state.borrow_mut();
        let st = &mut *st;
        let primary = *st
            .select
            .entry(key.clone())
            .or_insert_with(|| self.db.select(&key.name, key.bold, key.italic));
        let fallback = *st
            .fallback
            .entry((key.bold, key.italic))
            .or_insert_with(|| self.db.fallback(key.bold, key.italic));
        let mut out: Vec<Run> = Vec::new();
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                continue;
            }
            // (넣은 글꼴, 글리프, 폭, 흉내)
            let mut found: Option<(usize, u16, f64, FaceRef)> = None;
            for fr in [primary, fallback].into_iter().flatten() {
                let Some(e) = Self::embedded(st, self.db, fr.face) else {
                    continue;
                };
                if let Some((g, w)) = st.fonts[e].face.glyph(ch)
                    && (g != 0 || ch == ' ')
                {
                    found = Some((e, g, w, fr));
                    break;
                }
            }
            let (font, gid, w, sb, si) = match found {
                Some((e, g, w, fr)) => {
                    let new = if keep {
                        let f = &mut st.fonts[e];
                        let n = f.remap.remap(g);
                        f.glyphs.entry(n).or_insert((w, ch));
                        n
                    } else {
                        g
                    };
                    (Some(e), new, w, fr.synth_bold, fr.synth_italic)
                }
                None => (
                    None,
                    ch as u32 as u16,
                    plain_width(ch),
                    key.bold,
                    key.italic,
                ),
            };
            match out.last_mut() {
                Some(r) if r.font == font && r.synth_bold == sb && r.synth_italic == si => {
                    r.glyphs.push(gid);
                    r.advance += w;
                    r.text.push(ch);
                }
                _ => out.push(Run {
                    font,
                    glyphs: vec![gid],
                    advance: w,
                    synth_bold: sb,
                    synth_italic: si,
                    text: ch.to_string(),
                }),
            }
        }
        out
    }
}

impl FontSet<'_> {
    /// 넣은 글꼴 번호 → 글꼴 파일 바이트와 얼굴 번호 (글리프 윤곽을 그릴 때)
    pub fn face_data(&self, font: usize) -> Option<(std::sync::Arc<Vec<u8>>, u32)> {
        let st = self.state.borrow();
        st.fonts
            .get(font)
            .map(|e| (e.face.data.clone(), e.face.index))
    }

    /// 글꼴의 오름·내림 (1/1000 em, 내림은 양수) — 고른 얼굴, 없으면 대체 글꼴
    pub fn ascent_descent(&self, key: &FontKey) -> (f64, f64) {
        let mut st = self.state.borrow_mut();
        let st = &mut *st;
        let fr = *st
            .select
            .entry(key.clone())
            .or_insert_with(|| self.db.select(&key.name, key.bold, key.italic));
        fr.and_then(|fr| Self::embedded(st, self.db, fr.face))
            .map(|e| {
                let m = &st.fonts[e].face.metrics;
                (m.ascent, -m.descent)
            })
            .unwrap_or((880.0, 120.0))
    }
}

impl TextMeasure for FontSet<'_> {
    fn ascent_descent(&self, font: &FontKey) -> (f64, f64) {
        let (a, d) = FontSet::ascent_descent(self, font);
        (a / 1000.0, d / 1000.0)
    }

    fn device_ascent_descent(&self, font: &FontKey, ppem: u32) -> (f64, f64) {
        let mut st = self.state.borrow_mut();
        let st = &mut *st;
        let fr = *st
            .select
            .entry(font.clone())
            .or_insert_with(|| self.db.select(&font.name, font.bold, font.italic));
        let face = fr.and_then(|fr| Self::embedded(st, self.db, fr.face));
        if let Some(v) = face.and_then(|e| st.fonts[e].face.vdmx(ppem)) {
            return v;
        }
        let (a, d) = face.map_or((880.0, 120.0), |e| {
            let m = &st.fonts[e].face.metrics;
            (m.ascent, -m.descent)
        });
        (
            (a * ppem as f64 / 1000.0).round(),
            (d * ppem as f64 / 1000.0).round(),
        )
    }

    fn width(&self, font: &FontKey, size: f64, text: &str) -> f64 {
        self.runs(font, text, false)
            .iter()
            .map(|r| r.advance)
            .sum::<f64>()
            * size
            / 1000.0
    }
}

/// 페이지 한 장의 크기
#[derive(Debug, Clone, Copy)]
pub struct PageSize {
    /// PDF 용지 (pt, 정수로 자른 값 — 한셀 PDF 의 MediaBox)
    pub width: f64,
    pub height: f64,
    /// 600 dpi 장치 용지 크기
    pub dev_w: f64,
    pub dev_h: f64,
}

/// 숫자를 PDF 에 (소수 셋째 자리까지, 끝 0 없이)
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// 장치 단위 변환의 배율은 소수 여섯 자리 (세 자리면 0.119935 가 0.12 가 되어 오른쪽 끝에서 장치 1.4 어긋난다)
fn num6(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

fn color(c: [u8; 3]) -> String {
    format!(
        "{} {} {}",
        num(c[0] as f64 / 255.0),
        num(c[1] as f64 / 255.0),
        num(c[2] as f64 / 255.0)
    )
}

/// 비트맵 한 장의 최대 픽셀 수 (넘으면 벡터로)
const MAX_BITMAP_PIXELS: i64 = 40_000_000;

/// [`Op`] 의 좌표를 `(x − ox) · k` 꼴로 옮기고 늘인다 (선 굵기·점선·글자 크기도 `k` 배)
fn rebase(op: &Op, (ox, oy): (f64, f64), k: f64) -> Op {
    let pt = |x: f64, y: f64| ((x - ox) * k, (y - oy) * k);
    match op {
        Op::Path {
            segs,
            fill,
            stroke,
            even_odd,
            caps,
            hatch,
        } => Op::Path {
            segs: segs
                .iter()
                .map(|s| match *s {
                    Seg::Move(x, y) => {
                        let p = pt(x, y);
                        Seg::Move(p.0, p.1)
                    }
                    Seg::Line(x, y) => {
                        let p = pt(x, y);
                        Seg::Line(p.0, p.1)
                    }
                    Seg::Cubic(a, b, c, d, e, f) => {
                        let (p, q, r) = (pt(a, b), pt(c, d), pt(e, f));
                        Seg::Cubic(p.0, p.1, q.0, q.1, r.0, r.1)
                    }
                    Seg::Close => Seg::Close,
                })
                .collect(),
            fill: *fill,
            stroke: stroke
                .as_ref()
                .map(|(w, c, d)| (w * k, *c, d.iter().map(|v| v * k).collect())),
            even_odd: *even_odd,
            caps: *caps,
            hatch: *hatch,
        },
        Op::Rect {
            x0,
            y0,
            x1,
            y1,
            color,
        } => {
            let (a, b) = (pt(*x0, *y0), pt(*x1, *y1));
            Op::Rect {
                x0: a.0,
                y0: a.1,
                x1: b.0,
                y1: b.1,
                color: *color,
            }
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
            let (a, b) = (pt(*x0, *y0), pt(*x1, *y1));
            Op::Line {
                x0: a.0,
                y0: a.1,
                x1: b.0,
                y1: b.1,
                width: width * k,
                color: *color,
                dash: dash.iter().map(|v| v * k).collect(),
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
            let p = pt(*x, *y);
            Op::Text {
                x: p.0,
                y: p.1,
                size: size * k,
                font: font.clone(),
                color: *color,
                text: text.clone(),
                underline: *underline,
                strike: *strike,
                stretch: *stretch,
                angle: *angle,
            }
        }
        Op::Image {
            x0,
            y0,
            x1,
            y1,
            data,
        } => {
            let (a, b) = (pt(*x0, *y0), pt(*x1, *y1));
            Op::Image {
                x0: a.0,
                y0: a.1,
                x1: b.0,
                y1: b.1,
                data: data.clone(),
            }
        }
        Op::Clip { x0, y0, x1, y1 } => {
            let (a, b) = (pt(*x0, *y0), pt(*x1, *y1));
            Op::Clip {
                x0: a.0,
                y0: a.1,
                x1: b.0,
                y1: b.1,
            }
        }
        Op::Unclip => Op::Unclip,
        Op::Raster { .. } => op.clone(),
    }
}

/// 차트 비트맵: 상자 `(x0, y0, x1, y1)` (장치 단위, 정수 픽셀로 맞춘다) 안의 그리기 목록을 장치 해상도(한 장치 단위 = 한
/// 픽셀)의 RGB + 알파 그림으로. 웹 페이지 그림이 쓰는 래스터라이저 ([`super::raster`]) 를 같이 쓴다 — 그 래스터라이저는 1px =
/// 6.25 장치 단위로 그리므로 좌표를 6.25 배 늘려 넘긴다. 글꼴 파일이 없는 글자가 있거나 너무 크면 None (벡터로 그린다)
fn chart_bitmap(
    ops: &[Op],
    (bx0, by0, bx1, by1): (f64, f64, f64, f64),
    fonts: &FontSet,
) -> Option<PdfImage> {
    let (ox, oy) = (bx0.round(), by0.round());
    let (w, h) = ((bx1 - bx0).round() as i64, (by1 - by0).round() as i64);
    if w <= 0 || h <= 0 || w * h > MAX_BITMAP_PIXELS {
        return None;
    }
    let flat = super::raster::flatten_raster(ops);
    for op in &flat {
        match op {
            Op::Text { font, text, .. } => {
                if fonts
                    .runs(font, text, false)
                    .iter()
                    .any(|r| r.font.is_none())
                {
                    return None;
                }
            }
            Op::Path { hatch: Some(_), .. } => return None,
            _ => {}
        }
    }
    let scaled: Vec<Op> = flat.iter().map(|o| rebase(o, (ox, oy), 6.25)).collect();
    let bm = super::raster::draw(&scaled, w as u32, h as u32, fonts);
    let rgba = bm.rgba();
    let n = (w * h) as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    let mut alpha = Vec::with_capacity(n);
    let mut opaque = true;
    for p in rgba.as_chunks::<4>().0 {
        if p[3] == 0 {
            rgb.extend_from_slice(&[255, 255, 255]);
        } else {
            rgb.extend_from_slice(&p[..3]);
        }
        opaque &= p[3] == 255;
        alpha.push(p[3]);
    }
    Some(PdfImage {
        width: w as u32,
        height: h as u32,
        components: 3,
        dct: false,
        data: rgb,
        alpha: (!opaque).then_some(alpha),
    })
}

/// 그리기 목록 → 내용 스트림 (장치 단위). 글꼴은 `/F{n}` (넣은 글꼴 번호), 표준 CID 글꼴은 `/FK`
fn content(
    ops: &[Op],
    size: PageSize,
    fonts: &FontSet,
    images: &mut Images,
    hatches: &mut Hatches,
) -> (Vec<u8>, bool) {
    let mut s = String::new();
    let sx = size.width / (size.dev_w * 0.12);
    let sy = size.height / (size.dev_h * 0.12);
    // 글자 변환의 배율: 채우기가 있는 페이지는 0.12 그대로, 없으면 용지 비율을 곱한다 (한셀 PDF 1105 페이지:
    // 채우기 있는 192 페이지 중 168 이 `0.120 0 0 -0.120`, 없는 913 페이지 중 904 가 `0.119935 0 0 -0.119869` 꼴)
    let filled = ops.iter().any(|o| matches!(o, Op::Rect { .. }));
    let (tsx, tsy) = if filled { (1.0, 1.0) } else { (sx, sy) };
    // 선·채우기는 pt 좌표로 (한셀 PDF 처럼 — 같은 렌더러에서 선 굵기·자리가 같게 그려진다), 글자만 장치 단위 cm 아래
    let px = |x: f64| x * 0.12 * sx;
    let py = |y: f64| size.height - y * 0.12 * sy;
    let mut plain = false;
    for op in ops {
        match op {
            Op::Rect {
                x0,
                y0,
                x1,
                y1,
                color: c,
            } => {
                let _ = writeln!(
                    s,
                    "{} rg {} {} m {} {} l {} {} l {} {} l h f",
                    color(*c),
                    num(px(*x0)),
                    num(py(*y0)),
                    num(px(*x1)),
                    num(py(*y0)),
                    num(px(*x1)),
                    num(py(*y1)),
                    num(px(*x0)),
                    num(py(*y1))
                );
            }
            Op::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color: c,
                dash,
            } => {
                let d = dash
                    .iter()
                    .map(|v| num(v * 0.12))
                    .collect::<Vec<_>>()
                    .join(" ");
                let _ = writeln!(
                    s,
                    "{} RG {} w [{d}] 0 d {} {} m {} {} l S",
                    color(*c),
                    num(width * 0.12),
                    num(px(*x0)),
                    num(py(*y0)),
                    num(px(*x1)),
                    num(py(*y1))
                );
            }
            Op::Clip { x0, y0, x1, y1 } => {
                let _ = writeln!(
                    s,
                    "q {} {} m {} {} l {} {} l {} {} l h W n",
                    num(px(*x0)),
                    num(py(*y0)),
                    num(px(*x1)),
                    num(py(*y0)),
                    num(px(*x1)),
                    num(py(*y1)),
                    num(px(*x0)),
                    num(py(*y1))
                );
            }
            Op::Unclip => s.push_str("Q\n"),
            Op::Path {
                segs,
                fill,
                stroke,
                even_odd,
                caps,
                hatch,
            } => {
                let mut d = String::new();
                for sg in segs {
                    let _ = match *sg {
                        Seg::Move(x, y) => write!(d, "{} {} m ", num(px(x)), num(py(y))),
                        Seg::Line(x, y) => write!(d, "{} {} l ", num(px(x)), num(py(y))),
                        Seg::Cubic(x1, y1, x2, y2, x, y) => write!(
                            d,
                            "{} {} {} {} {} {} c ",
                            num(px(x1)),
                            num(py(y1)),
                            num(px(x2)),
                            num(py(y2)),
                            num(px(x)),
                            num(py(y))
                        ),
                        Seg::Close => write!(d, "h "),
                    };
                }
                let mut pre = String::new();
                if let Some(c) = fill {
                    let _ = write!(pre, "{} rg ", color(*c));
                }
                if let Some((w, c, dash)) = stroke {
                    let dd = dash
                        .iter()
                        .map(|v| num(v * 0.12))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = write!(pre, "{} RG {} w [{dd}] 0 d ", color(*c), num(w * 0.12));
                    if let Some((cap, join)) = caps {
                        let _ = write!(pre, "{cap} J {join} j 10 M ");
                    }
                }
                if let Some(h) = hatch {
                    let _ = write!(pre, "/Pattern cs /Pa{} scn ", hatches.add(*h));
                }
                let op = match (
                    fill.is_some() || hatch.is_some(),
                    stroke.is_some(),
                    even_odd,
                ) {
                    (true, true, false) => "B",
                    (true, true, true) => "B*",
                    (true, false, false) => "f",
                    (true, false, true) => "f*",
                    _ => "S",
                };
                let _ = writeln!(s, "{pre}{d}{op}");
            }
            Op::Raster {
                x0,
                y0,
                x1,
                y1,
                ops: sub,
            } => match chart_bitmap(sub, (*x0, *y0, *x1, *y1), fonts) {
                Some(img) => {
                    let (ox, oy) = (x0.round(), y0.round());
                    let (w, h) = (img.width as f64, img.height as f64);
                    let k = images.push(img);
                    let _ = writeln!(
                        s,
                        "q {} 0 0 {} {} {} cm /Im{k} Do Q",
                        num(w * 0.12 * sx),
                        num(h * 0.12 * sy),
                        num(px(ox)),
                        num(py(oy + h))
                    );
                }
                None => {
                    let (c, p) = content(sub, size, fonts, images, hatches);
                    s.push_str(&String::from_utf8_lossy(&c));
                    plain |= p;
                }
            },
            Op::Image {
                x0,
                y0,
                x1,
                y1,
                data,
            } => {
                if let Some(k) = images.add(data) {
                    let _ = writeln!(
                        s,
                        "q {} 0 0 {} {} {} cm /Im{k} Do Q",
                        num((x1 - x0) * 0.12 * sx),
                        num((y1 - y0) * 0.12 * sy),
                        num(px(*x0)),
                        num(py(*y1))
                    );
                }
            }
            Op::Text {
                x,
                y,
                size: sz,
                font,
                color: c,
                text,
                underline,
                strike,
                stretch,
                angle,
            } => {
                let runs = fonts.runs(font, text, true);
                let mut cx = *x;
                let mut cy = *y;
                let (ca, sa) = (angle.to_radians().cos(), angle.to_radians().sin());
                let _ = writeln!(
                    s,
                    "q {} 0 0 {} 0 {} cm {} rg {} RG",
                    num6(0.12 * tsx),
                    num6(-0.12 * tsy),
                    num(size.height),
                    color(*c),
                    color(*c)
                );
                for r in &runs {
                    let name = match r.font {
                        Some(i) => format!("F{i}"),
                        None => {
                            plain = true;
                            "FK".into()
                        }
                    };
                    let shear = if r.synth_italic { -0.21 } else { 0.0 };
                    let mut hex = String::with_capacity(r.glyphs.len() * 4);
                    for g in &r.glyphs {
                        let _ = write!(hex, "{g:04X}");
                    }
                    let (mode, lw) = if r.synth_bold {
                        (2, sz / 30.0)
                    } else {
                        (0, 0.0)
                    };
                    let tz = if (*stretch - 1.0).abs() > 1e-6 {
                        format!("{} Tz ", num(stretch * 100.0))
                    } else {
                        String::new()
                    };
                    // 글리프 공간(위가 +) → 장치(아래가 +): 기준선 방향 (cos, sin), 위 방향 (sin, −cos)
                    let _ = writeln!(
                        s,
                        "BT /{name} {} Tf {tz}{mode} Tr {} w {} {} {} {} {} {} Tm <{hex}> Tj ET",
                        num(*sz),
                        num(lw),
                        num(ca),
                        num(sa),
                        num(sa + shear * ca),
                        num(-ca + shear * sa),
                        num(cx),
                        num(cy)
                    );
                    let adv = r.advance * sz / 1000.0 * stretch;
                    cx += adv * ca;
                    cy += adv * sa;
                }
                let lw = (sz / 18.0).max(3.0);
                if *underline {
                    let uy = y + sz * 0.12;
                    let _ = writeln!(
                        s,
                        "{} w [] 0 d {} {} m {} {} l S",
                        num(lw),
                        num(*x),
                        num(uy),
                        num(cx),
                        num(uy)
                    );
                }
                if *strike {
                    let sy2 = y - sz * 0.3;
                    let _ = writeln!(
                        s,
                        "{} w [] 0 d {} {} m {} {} l S",
                        num(lw),
                        num(*x),
                        num(sy2),
                        num(cx),
                        num(sy2)
                    );
                }
                s.push_str("Q\n");
            }
        }
    }
    (s.into_bytes(), plain)
}

/// 압축 (zlib)
fn deflate(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// 부분 집합 이름 앞 6 글자 (글리프 목록에서)
fn subset_tag(glyphs: &BTreeMap<u16, (f64, char)>, i: usize) -> String {
    let mut h: u64 = 1469598103934665603 ^ i as u64;
    for (g, (_, c)) in glyphs {
        h = (h ^ *g as u64 ^ ((*c as u64) << 16)).wrapping_mul(1099511628211);
    }
    (0..6)
        .map(|k| (b'A' + ((h >> (k * 5)) % 26) as u8) as char)
        .collect()
}

/// 페이지들을 PDF 로
pub fn write_pdf(pages: &[(PageSize, Vec<Op>)], fonts: &FontSet, title: &str) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let mut next = 1;
    let mut alloc = || {
        let r = Ref::new(next);
        next += 1;
        r
    };
    let catalog = alloc();
    let tree = alloc();
    let info = alloc();
    // 내용 먼저 (글리프를 모은다)
    let mut contents = Vec::new();
    let mut any_plain = false;
    let mut images = Images::default();
    let mut hatches = Hatches::default();
    for (size, ops) in pages {
        let (c, plain) = content(ops, *size, fonts, &mut images, &mut hatches);
        any_plain |= plain;
        contents.push(c);
    }
    let st = fonts.state.borrow();
    let font_refs: Vec<Ref> = st.fonts.iter().map(|_| alloc()).collect();
    let plain_ref = alloc();
    let image_refs: Vec<(Ref, Option<Ref>)> = images
        .list
        .iter()
        .map(|im| (alloc(), im.alpha.as_ref().map(|_| alloc())))
        .collect();
    let hatch_refs: Vec<Ref> = hatches.list.iter().map(|_| alloc()).collect();
    let page_refs: Vec<Ref> = pages.iter().map(|_| alloc()).collect();
    let content_refs: Vec<Ref> = pages.iter().map(|_| alloc()).collect();

    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree)
        .kids(page_refs.iter().copied())
        .count(page_refs.len() as i32);
    pdf.document_info(info)
        .title(TextStr(title))
        .producer(TextStr("rcell"));
    for (i, (size, _)) in pages.iter().enumerate() {
        let mut page = pdf.page(page_refs[i]);
        page.parent(tree)
            .media_box(Rect::new(0.0, 0.0, size.width as f32, size.height as f32))
            .contents(content_refs[i]);
        let mut res = page.resources();
        let mut f = res.fonts();
        for (k, r) in font_refs.iter().enumerate() {
            f.pair(Name(format!("F{k}").as_bytes()), *r);
        }
        if any_plain {
            f.pair(Name(b"FK"), plain_ref);
        }
        f.finish();
        if !image_refs.is_empty() {
            let mut x = res.x_objects();
            for (k, (r, _)) in image_refs.iter().enumerate() {
                x.pair(Name(format!("Im{k}").as_bytes()), *r);
            }
            x.finish();
        }
        if !hatch_refs.is_empty() {
            let mut pd = res.patterns();
            for (k, r) in hatch_refs.iter().enumerate() {
                pd.pair(Name(format!("Pa{k}").as_bytes()), *r);
            }
            pd.finish();
        }
        res.finish();
        page.finish();
        let z = deflate(&contents[i]);
        pdf.stream(content_refs[i], &z).filter(Filter::FlateDecode);
    }
    // 넣은 글꼴
    for (k, e) in st.fonts.iter().enumerate() {
        let cid = alloc();
        let desc = alloc();
        let file = alloc();
        let tu = alloc();
        let m = &e.face.metrics;
        let base = format!("{}+{}", subset_tag(&e.glyphs, k), m.ps_name);
        pdf.type0_font(font_refs[k])
            .base_font(Name(base.as_bytes()))
            .encoding_predefined(Name(b"Identity-H"))
            .descendant_font(cid)
            .to_unicode(tu);
        let sysinfo = SystemInfo {
            registry: Str(b"Adobe"),
            ordering: Str(b"Identity"),
            supplement: 0,
        };
        {
            let mut c = pdf.cid_font(cid);
            c.subtype(CidFontType::Type2)
                .base_font(Name(base.as_bytes()))
                .system_info(sysinfo)
                .font_descriptor(desc)
                .default_width(1000.0)
                .cid_to_gid_map_predefined(Name(b"Identity"));
            let mut w = c.widths();
            for (g, (adv, _)) in &e.glyphs {
                w.consecutive(*g, [*adv as f32]);
            }
            w.finish();
        }
        let mut flags = FontFlags::SYMBOLIC;
        if m.fixed {
            flags |= FontFlags::FIXED_PITCH;
        }
        pdf.font_descriptor(desc)
            .name(Name(base.as_bytes()))
            .flags(flags)
            .bbox(Rect::new(
                m.bbox[0] as f32,
                m.bbox[1] as f32,
                m.bbox[2] as f32,
                m.bbox[3] as f32,
            ))
            .italic_angle(m.italic_angle as f32)
            .ascent(m.ascent as f32)
            .descent(m.descent as f32)
            .cap_height(m.cap_height as f32)
            .stem_v(80.0)
            .font_file2(file);
        let sub = subsetter::subset(&e.face.data, e.face.index, &e.remap).unwrap_or_default();
        let z = deflate(&sub);
        pdf.stream(file, &z)
            .filter(Filter::FlateDecode)
            .pair(Name(b"Length1"), sub.len() as i32);
        let mut cmap = UnicodeCmap::new(
            Name(b"Custom"),
            SystemInfo {
                registry: Str(b"Adobe"),
                ordering: Str(b"UCS"),
                supplement: 0,
            },
        );
        for (g, (_, ch)) in &e.glyphs {
            cmap.pair(*g, *ch);
        }
        let buf = cmap.finish();
        pdf.cmap(tu, buf.as_slice());
    }
    // 빗금 무늬
    for (h, r) in hatches.list.iter().zip(&hatch_refs) {
        let z = deflate(hatch_tile(h).as_bytes());
        let mut t = pdf.tiling_pattern(*r, &z);
        t.paint_type(PaintType::Colored)
            .tiling_type(TilingType::NoDistortion)
            .bbox(Rect::new(0.0, 0.0, 8.0, 8.0))
            .x_step(8.0)
            .y_step(8.0)
            .matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        t.filter(Filter::FlateDecode);
        t.resources().finish();
        t.finish();
    }
    // 그림
    for (im, (r, mask)) in images.list.iter().zip(&image_refs) {
        let z;
        let data: &[u8] = if im.dct {
            &im.data
        } else {
            z = deflate(&im.data);
            &z
        };
        let mut x = pdf.image_xobject(*r, data);
        x.filter(if im.dct {
            Filter::DctDecode
        } else {
            Filter::FlateDecode
        });
        x.width(im.width as i32);
        x.height(im.height as i32);
        x.bits_per_component(8);
        if im.components == 1 {
            x.color_space().device_gray();
        } else {
            x.color_space().device_rgb();
        }
        if let Some(m) = mask {
            x.s_mask(*m);
        }
        x.finish();
        if let (Some(m), Some(a)) = (mask, &im.alpha) {
            let z = deflate(a);
            let mut x = pdf.image_xobject(*m, &z);
            x.filter(Filter::FlateDecode);
            x.width(im.width as i32);
            x.height(im.height as i32);
            x.bits_per_component(8);
            x.color_space().device_gray();
            x.finish();
        }
    }
    // 글꼴 파일이 없는 글자: 한국어 표준 CID 글꼴 (보는 프로그램의 대체 글꼴)
    if any_plain {
        let cid = alloc();
        let desc = alloc();
        pdf.type0_font(plain_ref)
            .base_font(Name(b"HYGoThic-Medium"))
            .encoding_predefined(Name(b"UniKS-UCS2-H"))
            .descendant_font(cid);
        pdf.cid_font(cid)
            .subtype(CidFontType::Type0)
            .base_font(Name(b"HYGoThic-Medium"))
            .system_info(SystemInfo {
                registry: Str(b"Adobe"),
                ordering: Str(b"Korea1"),
                supplement: 1,
            })
            .font_descriptor(desc)
            .default_width(1000.0);
        pdf.font_descriptor(desc)
            .name(Name(b"HYGoThic-Medium"))
            .flags(FontFlags::SYMBOLIC)
            .bbox(Rect::new(-6.0, -145.0, 1003.0, 880.0))
            .italic_angle(0.0)
            .ascent(880.0)
            .descent(-120.0)
            .cap_height(880.0)
            .stem_v(93.0);
    }
    drop(st);
    pdf.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 빗금 무늬 칸: 한셀 PDF 의 Tiling 무늬와 같은 내용 (불투명이면 바탕색을 깐다)
    #[test]
    fn hatch_tile_content() {
        let h: Hatch = (0, [0, 0, 192], Some([255, 255, 255]));
        let s = hatch_tile(&h);
        assert!(s.contains(
            "0 0 8 8 re
W n
1 1 1 rg
0 0 8 8 re
f
"
        ));
        assert!(s.contains(
            "1 J 1 j 1 w 10 M []0 d
0 4 m 8 4 l
S"
        ));
        assert!(!hatch_tile(&(1, [0, 0, 0], None)).contains(" rg"));
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.12), "0.12");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(2.5), "2.5");
    }
}
/// 차트 비트맵: 장치 해상도 RGB + 알파 (칠한 곳 불투명, 빈 곳 투명). 글꼴 파일이 없는 글자가 있으면 벡터로 (None)
#[test]
fn chart_bitmap_is_device_resolution_with_alpha() {
    let db = FontDb::new();
    let fonts = FontSet::new(&db);
    let rect = Op::Rect {
        x0: 102.0,
        y0: 102.0,
        x1: 106.0,
        y1: 106.0,
        color: [255, 0, 0],
    };
    let img = chart_bitmap(&[rect], (100.0, 100.0, 110.0, 110.0), &fonts).unwrap();
    assert_eq!((img.width, img.height), (10, 10));
    let alpha = img.alpha.as_ref().unwrap();
    let at = |x: usize, y: usize| y * 10 + x;
    assert_eq!(alpha[at(3, 3)], 255);
    assert_eq!(&img.data[at(3, 3) * 3..at(3, 3) * 3 + 3], &[255, 0, 0]);
    assert_eq!(alpha[at(8, 3)], 0);
    let text = Op::Text {
        x: 100.0,
        y: 105.0,
        size: 10.0,
        font: FontKey {
            name: "Arial".into(),
            bold: false,
            italic: false,
        },
        color: [0, 0, 0],
        text: "A".into(),
        underline: false,
        strike: false,
        stretch: 1.0,
        angle: 0.0,
    };
    assert!(chart_bitmap(&[text], (100.0, 100.0, 110.0, 110.0), &fonts).is_none());
}
