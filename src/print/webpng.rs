// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 웹 페이지(`.htm`·`.mht`)의 도형·차트·그림 PNG. 한셀은 도형·차트를 RGBA PNG 로 그려 넣는다. 그리는 일은 인쇄가 차트를
//! 비트맵으로 그릴 때 쓰는 [`super::raster::render`] (zeno) 하나를 같이 쓴다 — 여기는 그 앞뒤만: 장치 단위(1px = 6.25) 그리기
//! 목록을 픽셀 단위로 줄이고, 비트맵이 못 그리는 것(밑줄·취소선은 선으로, 그림은 따로)을 풀어 주고, 결과를 PNG 로 쓴다.

use super::pdf::FontSet;
use super::pdfimage::{PdfImage, decode};
use super::raster;
use crate::render::page::{Op, Seg, TextMeasure};

/// 장치 단위 → 픽셀
const K: f64 = 1.0 / 6.25;

/// 따로 얹는 그림: 상자(왼쪽·위·오른쪽·아래, px)와 원본 바이트
type Pic = (f64, f64, f64, f64, std::sync::Arc<Vec<u8>>);

fn scale_segs(segs: &[Seg], k: f64) -> Vec<Seg> {
    segs.iter()
        .map(|s| match *s {
            Seg::Move(x, y) => Seg::Move(x * k, y * k),
            Seg::Line(x, y) => Seg::Line(x * k, y * k),
            Seg::Cubic(a, b, c, d, e, f) => Seg::Cubic(a * k, b * k, c * k, d * k, e * k, f * k),
            Seg::Close => Seg::Close,
        })
        .collect()
}

/// 그리기 목록을 `k` 배로 줄여 `out` 에 더한다. `Raster` 묶음은 풀어서 안의 목록을, 밑줄·취소선은 가는 사각형으로,
/// 그림은 `images` 로 따로 모은다 (비트맵 그리개가 못 그린다)
fn scaled(ops: &[Op], k: f64, fonts: &FontSet, out: &mut Vec<Op>, images: &mut Vec<Pic>) {
    for op in ops {
        match op {
            Op::Path {
                segs,
                fill,
                stroke,
                even_odd,
                caps,
                hatch,
            } => out.push(Op::Path {
                segs: scale_segs(segs, k),
                fill: *fill,
                stroke: stroke
                    .as_ref()
                    .map(|(w, c, d)| (w * k, *c, d.iter().map(|v| v * k).collect())),
                even_odd: *even_odd,
                caps: *caps,
                hatch: *hatch,
            }),
            Op::Rect {
                x0,
                y0,
                x1,
                y1,
                color,
            } => out.push(Op::Rect {
                x0: x0 * k,
                y0: y0 * k,
                x1: x1 * k,
                y1: y1 * k,
                color: *color,
            }),
            Op::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
                dash,
            } => out.push(Op::Line {
                x0: x0 * k,
                y0: y0 * k,
                x1: x1 * k,
                y1: y1 * k,
                width: width * k,
                color: *color,
                dash: dash.iter().map(|v| v * k).collect(),
            }),
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
                let (x, y, size) = (x * k, y * k, size * k);
                out.push(Op::Text {
                    x,
                    y,
                    size,
                    font: font.clone(),
                    color: *color,
                    text: text.clone(),
                    underline: false,
                    strike: false,
                    stretch: *stretch,
                    angle: *angle,
                });
                // 밑줄·취소선: 글 너비만큼 굵기 size/14 의 가로선 (기울어진 글은 생략)
                if (*underline || *strike) && *angle == 0.0 {
                    let w = fonts.width(font, size, text) * stretch;
                    let t = (size / 14.0).max(0.5);
                    for (on, off) in [(*underline, size * 0.12), (*strike, -size * 0.3)] {
                        if on {
                            out.push(Op::Rect {
                                x0: x,
                                y0: y + off - t / 2.0,
                                x1: x + w,
                                y1: y + off + t / 2.0,
                                color: *color,
                            });
                        }
                    }
                }
            }
            Op::Raster { ops, .. } => scaled(ops, k, fonts, out, images),
            Op::Clip { x0, y0, x1, y1 } => out.push(Op::Clip {
                x0: x0 * k,
                y0: y0 * k,
                x1: x1 * k,
                y1: y1 * k,
            }),
            Op::Unclip => out.push(Op::Unclip),
            Op::Image {
                x0,
                y0,
                x1,
                y1,
                data,
            } => images.push((x0 * k, y0 * k, x1 * k, y1 * k, data.clone())),
        }
    }
}

/// 비트맵 `RGB + 알파` → 곧은 RGBA 바이트 (투명한 곳은 0,0,0,0 — 한셀 PNG 와 같다)
fn rgba(img: &PdfImage) -> Vec<u8> {
    let n = (img.width as usize) * (img.height as usize);
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let a = img.alpha.as_ref().map_or(255, |a| a[i]);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            out.extend_from_slice(&img.data[i * 3..i * 3 + 3]);
            out.push(a);
        }
    }
    out
}

/// 그림(PNG·BMP)을 상자에 이중선형으로 얹는다 (`rgba` 는 `w × h`, 직선 알파). 풀 수 없으면 그대로 둔다
fn draw_image(
    buf: &mut [u8],
    (w, h): (u32, u32),
    (x0, y0, x1, y1): (f64, f64, f64, f64),
    data: &[u8],
) {
    let Some(img) = decode(data) else {
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
    if img.data.len() < iw * ih * comps || x1 <= x0 || y1 <= y0 {
        return;
    }
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
    let sample = |fx: f64, fy: f64| -> ([f32; 3], f32) {
        let fx = (fx - 0.5).clamp(0.0, iw as f64 - 1.0);
        let fy = (fy - 0.5).clamp(0.0, ih as f64 - 1.0);
        let (xi, yi) = (fx.floor() as usize, fy.floor() as usize);
        let (xj, yj) = ((xi + 1).min(iw - 1), (yi + 1).min(ih - 1));
        let (tx, ty) = ((fx - xi as f64) as f32, (fy - yi as f64) as f32);
        let (a, b, c, d) = (get(xi, yi), get(xj, yi), get(xi, yj), get(xj, yj));
        let mut col = [0.0; 3];
        for (k, v) in col.iter_mut().enumerate() {
            *v = (a.0[k] * (1.0 - tx) + b.0[k] * tx) * (1.0 - ty)
                + (c.0[k] * (1.0 - tx) + d.0[k] * tx) * ty;
        }
        let al = (a.1 * (1.0 - tx) + b.1 * tx) * (1.0 - ty) + (c.1 * (1.0 - tx) + d.1 * tx) * ty;
        (col, al)
    };
    let (bw, bh) = (x1 - x0, y1 - y0);
    for py in (y0.max(0.0).floor() as i64)..(y1.min(h as f64).ceil() as i64) {
        for px in (x0.max(0.0).floor() as i64)..(x1.min(w as f64).ceil() as i64) {
            let (sx, sy) = (
                (px as f64 + 0.5 - x0) / bw * iw as f64,
                (py as f64 + 0.5 - y0) / bh * ih as f64,
            );
            if sx < 0.0 || sy < 0.0 || sx > iw as f64 || sy > ih as f64 {
                continue;
            }
            let (c, sa) = sample(sx, sy);
            let o = ((py as u32 * w + px as u32) * 4) as usize;
            let da = buf[o + 3] as f32 / 255.0;
            let oa = sa + da * (1.0 - sa);
            if oa <= 0.0 {
                continue;
            }
            for k in 0..3 {
                let v = (c[k] * sa + buf[o + k] as f32 * da * (1.0 - sa)) / oa;
                buf[o + k] = (v + 0.5).clamp(0.0, 255.0) as u8;
            }
            buf[o + 3] = (oa * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
}

/// 그리기 목록(장치 단위)을 `w × h`(px) 투명 바탕 RGBA 바이트로
pub fn draw(ops: &[Op], w: u32, h: u32, fonts: &FontSet) -> Vec<u8> {
    let mut px = Vec::new();
    let mut images = Vec::new();
    scaled(ops, K, fonts, &mut px, &mut images);
    let mut buf = match raster::render(&px, (0.0, 0.0, w as f64, h as f64), None, fonts) {
        Some(img) if img.width == w && img.height == h => rgba(&img),
        _ => vec![0; (w as usize) * (h as usize) * 4],
    };
    for (x0, y0, x1, y1, data) in &images {
        draw_image(&mut buf, (w, h), (*x0, *y0, *x1, *y1), data);
    }
    buf
}

/// 그림(PNG·BMP)을 `w × h` 로 다시 그린 RGBA 바이트. 풀 수 없는 형식이면 None
pub fn picture(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let img = decode(data)?;
    if img.dct {
        return None;
    }
    let mut buf = vec![0; (w as usize) * (h as usize) * 4];
    draw_image(&mut buf, (w, h), (0.0, 0.0, w as f64, h as f64), data);
    Some(buf)
}

fn crc32(data: &[u8], table: &[u32; 256]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// RGBA(8비트) PNG 바이트. 줄마다 필터 하나를 골라 (없음·왼쪽) 압축한다
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
    use crate::print::FontDb;

    #[test]
    fn draws_rect_into_transparent_png() {
        let db = FontDb::new();
        let fonts = FontSet::new(&db);
        // 6.25 장치 단위 = 1px: 10×10 상자 안 (2.5, 2)–(6, 6) 채우기
        let ops = vec![Op::Rect {
            x0: 15.625,
            y0: 12.5,
            x1: 37.5,
            y1: 37.5,
            color: [255, 0, 0],
        }];
        let buf = draw(&ops, 10, 10, &fonts);
        let at = |x: usize, y: usize| &buf[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4];
        assert_eq!(at(3, 3), &[255, 0, 0, 255]);
        assert_eq!(at(0, 0), &[0, 0, 0, 0]);
        assert!((at(2, 3)[3] as i32 - 128).abs() <= 1);
        let png = encode_png(10, 10, &buf);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 10);
    }
}
