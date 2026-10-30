// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그림 → PDF 이미지: JPEG 는 그대로 (DCTDecode), PNG·BMP 는 풀어서 RGB + 알파(SMask) 로.
//! EMF·WMF 같은 벡터 그림은 아직 그리지 않는다.

use std::io::Read;

/// PDF 에 넣을 그림
pub struct PdfImage {
    pub width: u32,
    pub height: u32,
    /// 색 성분 수 (1 회색, 3 RGB)
    pub components: u8,
    /// JPEG 원본이면 DCT
    pub dct: bool,
    /// 픽셀 (DCT 면 JPEG 바이트)
    pub data: Vec<u8>,
    /// 알파 (8비트, 1 성분)
    pub alpha: Option<Vec<u8>>,
}

/// 그림 바이트를 PDF 이미지로. 모르는 형식이면 None
pub fn decode(d: &[u8]) -> Option<PdfImage> {
    if d.starts_with(&[0xFF, 0xD8]) {
        return jpeg(d);
    }
    if d.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(d);
    }
    if d.starts_with(b"BM") {
        return bmp(d);
    }
    None
}

fn jpeg(d: &[u8]) -> Option<PdfImage> {
    // SOFn 표지에서 크기·성분 수
    let mut i = 2;
    while i + 9 < d.len() {
        if d[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([d[i + 5], d[i + 6]]) as u32;
            let w = u16::from_be_bytes([d[i + 7], d[i + 8]]) as u32;
            let c = d[i + 9];
            if c != 1 && c != 3 {
                return None;
            }
            return Some(PdfImage {
                width: w,
                height: h,
                components: c,
                dct: true,
                data: d.to_vec(),
                alpha: None,
            });
        }
        i += 2 + len;
    }
    None
}

fn png(d: &[u8]) -> Option<PdfImage> {
    let mut i = 8;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0u32, 0u32, 8u8, 0u8, 0u8);
    let mut idat = Vec::new();
    let mut plte: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    while i + 8 <= d.len() {
        let len = u32::from_be_bytes(d[i..i + 4].try_into().ok()?) as usize;
        let kind = &d[i + 4..i + 8];
        let body = d.get(i + 8..i + 8 + len)?;
        match kind {
            b"IHDR" => {
                w = u32::from_be_bytes(body[0..4].try_into().ok()?);
                h = u32::from_be_bytes(body[4..8].try_into().ok()?);
                depth = body[8];
                ctype = body[9];
                interlace = body[12];
            }
            b"PLTE" => plte = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        i += 12 + len;
    }
    if w == 0 || h == 0 || interlace != 0 || (w as u64 * h as u64) > 64_000_000 {
        return None;
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(idat.as_slice())
        .read_to_end(&mut raw)
        .ok()?;
    let chans: usize = match ctype {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    let bits = depth as usize * chans;
    let stride = (w as usize * bits).div_ceil(8);
    let bpp = bits.div_ceil(8).max(1);
    // 줄마다 거르개 풀기
    let mut pix = vec![0u8; stride * h as usize];
    let mut prev = vec![0u8; stride];
    for y in 0..h as usize {
        let row = raw.get(y * (stride + 1)..(y + 1) * (stride + 1))?;
        let f = row[0];
        let src = &row[1..];
        let mut cur = vec![0u8; stride];
        for x in 0..stride {
            let a = if x >= bpp { cur[x - bpp] as i32 } else { 0 };
            let b = prev[x] as i32;
            let c = if x >= bpp { prev[x - bpp] as i32 } else { 0 };
            let v = src[x] as i32;
            cur[x] = match f {
                1 => v + a,
                2 => v + b,
                3 => v + (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    v + if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => v,
            } as u8;
        }
        pix[y * stride..(y + 1) * stride].copy_from_slice(&cur);
        prev = cur;
    }
    // 표본 하나 (8비트로)
    let sample = |row: &[u8], idx: usize| -> u8 {
        match depth {
            8 => row[idx],
            16 => row[idx * 2],
            d => {
                let per = 8 / d as usize;
                let byte = row[idx / per];
                let shift = 8 - d as usize * (idx % per + 1);
                let v = (byte >> shift) & ((1u8 << d) - 1);
                if ctype == 3 {
                    v
                } else {
                    (v as u32 * 255 / ((1u32 << d) - 1)) as u8
                }
            }
        }
    };
    let n = (w * h) as usize;
    let gray = matches!(ctype, 0 | 4);
    let comps = if gray { 1 } else { 3 };
    let mut out = Vec::with_capacity(n * comps);
    let mut alpha = Vec::with_capacity(n);
    let mut has_alpha = false;
    for y in 0..h as usize {
        let row = &pix[y * stride..(y + 1) * stride];
        for x in 0..w as usize {
            match ctype {
                0 => {
                    out.push(sample(row, x));
                    alpha.push(255);
                }
                4 => {
                    out.push(sample(row, x * 2));
                    let a = sample(row, x * 2 + 1);
                    has_alpha |= a != 255;
                    alpha.push(a);
                }
                2 => {
                    out.extend([
                        sample(row, x * 3),
                        sample(row, x * 3 + 1),
                        sample(row, x * 3 + 2),
                    ]);
                    alpha.push(255);
                }
                6 => {
                    out.extend([
                        sample(row, x * 4),
                        sample(row, x * 4 + 1),
                        sample(row, x * 4 + 2),
                    ]);
                    let a = sample(row, x * 4 + 3);
                    has_alpha |= a != 255;
                    alpha.push(a);
                }
                _ => {
                    let k = sample(row, x) as usize;
                    out.extend([
                        plte.get(k * 3).copied().unwrap_or(0),
                        plte.get(k * 3 + 1).copied().unwrap_or(0),
                        plte.get(k * 3 + 2).copied().unwrap_or(0),
                    ]);
                    let a = trns.get(k).copied().unwrap_or(255);
                    has_alpha |= a != 255;
                    alpha.push(a);
                }
            }
        }
    }
    Some(PdfImage {
        width: w,
        height: h,
        components: comps as u8,
        dct: false,
        data: out,
        alpha: has_alpha.then_some(alpha),
    })
}

/// BMP (BI_RGB·BI_BITFIELDS): 1·4·8 비트 색표, 16 비트 (5-5-5 또는 마스크), 24·32 비트. 압축(RLE)은 풀지 않는다
fn bmp(d: &[u8]) -> Option<PdfImage> {
    let le32 = |i: usize| {
        d.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let le16 = |i: usize| d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let off = le32(10)? as usize;
    let hs = le32(14)? as usize;
    let w = le32(18)? as i32;
    let hh = le32(22)? as i32;
    let bpp = le16(28)? as usize;
    let compression = le32(30)?;
    if w <= 0
        || hh == 0
        || !matches!(compression, 0 | 3)
        || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32)
    {
        return None;
    }
    let (w, h, flip) = (w as usize, hh.unsigned_abs() as usize, hh > 0);
    if w.checked_mul(h)? > 1 << 28 {
        return None;
    }
    // 색표 (14 + 머리 크기 뒤)
    let used = le32(46).unwrap_or(0) as usize;
    let colors = if bpp <= 8 {
        if used != 0 { used } else { 1 << bpp }
    } else {
        0
    };
    let pal_at = 14 + hs;
    let mut pal = Vec::with_capacity(colors);
    for i in 0..colors {
        let e = d.get(pal_at + 4 * i..pal_at + 4 * i + 4)?;
        pal.push([e[2], e[1], e[0]]);
    }
    // 마스크 (16·32 비트)
    let (mut masks, mut mask_at) = ([0u32; 3], 0);
    if bpp == 16 {
        masks = [0x7C00, 0x03E0, 0x001F];
    } else if bpp == 32 {
        masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
    }
    if compression == 3 && matches!(bpp, 16 | 32) {
        mask_at = pal_at;
        masks = [le32(mask_at)?, le32(mask_at + 4)?, le32(mask_at + 8)?];
    }
    // 마스크의 비트를 8 비트로
    let chan = |v: u32, m: u32| -> u8 {
        if m == 0 {
            return 0;
        }
        let sh = m.trailing_zeros();
        let bits = (m >> sh).count_ones();
        let x = (v & m) >> sh;
        if bits >= 8 {
            (x >> (bits - 8)) as u8
        } else {
            ((x * 255) / ((1u32 << bits) - 1)) as u8
        }
    };
    let stride = (w * bpp).div_ceil(32) * 4;
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let sy = if flip { h - 1 - y } else { y };
        let row = d.get(off + sy * stride..off + sy * stride + (w * bpp).div_ceil(8))?;
        for x in 0..w {
            let rgb = match bpp {
                1 | 4 | 8 => {
                    let bit = x * bpp;
                    let v = (row[bit / 8] >> (8 - bpp - bit % 8)) & ((1u16 << bpp) - 1) as u8;
                    *pal.get(v as usize)?
                }
                16 => {
                    let v = u16::from_le_bytes([row[2 * x], row[2 * x + 1]]) as u32;
                    [chan(v, masks[0]), chan(v, masks[1]), chan(v, masks[2])]
                }
                24 => [row[3 * x + 2], row[3 * x + 1], row[3 * x]],
                _ => {
                    let v = u32::from_le_bytes([
                        row[4 * x],
                        row[4 * x + 1],
                        row[4 * x + 2],
                        row[4 * x + 3],
                    ]);
                    [chan(v, masks[0]), chan(v, masks[1]), chan(v, masks[2])]
                }
            };
            out.extend(rgb);
        }
    }
    let _ = mask_at;
    Some(PdfImage {
        width: w as u32,
        height: h as u32,
        components: 3,
        dct: false,
        data: out,
        alpha: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 머리 54 바이트 (파일 14 + 정보 40) 의 BMP
    fn bmp_with(w: i32, h: i32, bpp: u16, palette: &[[u8; 4]], bits: &[u8]) -> Vec<u8> {
        let off = 54 + 4 * palette.len();
        let mut f = b"BM".to_vec();
        f.extend(((off + bits.len()) as u32).to_le_bytes());
        f.extend([0u8; 4]);
        f.extend((off as u32).to_le_bytes());
        f.extend(40u32.to_le_bytes());
        f.extend(w.to_le_bytes());
        f.extend(h.to_le_bytes());
        f.extend(1u16.to_le_bytes());
        f.extend(bpp.to_le_bytes());
        f.extend([0u8; 4]); // BI_RGB
        f.extend([0u8; 12]);
        f.extend((palette.len() as u32).to_le_bytes());
        f.extend([0u8; 4]);
        for p in palette {
            f.extend(p);
        }
        f.extend(bits);
        f
    }

    #[test]
    fn bmp_16_bit_555_and_palette() {
        // 16 비트 (5-5-5): 위로 가는 행 차례 — 아래 행(빨강, 초록)이 먼저. 한 행 4 바이트(2 픽셀 + 채움)
        let red = (0x1Fu16 << 10).to_le_bytes();
        let green = (0x1Fu16 << 5).to_le_bytes();
        let blue = 0x1Fu16.to_le_bytes();
        let d = bmp_with(
            2,
            2,
            16,
            &[],
            &[
                red[0], red[1], green[0], green[1], blue[0], blue[1], 0, 0, 0, 0, 0, 0,
            ],
        );
        let im = decode(&d).unwrap();
        assert_eq!((im.width, im.height, im.components), (2, 2, 3));
        // 위 행 = 파일의 둘째 행 (파랑, 검정), 아래 행 = 첫째 (빨강, 초록)
        assert_eq!(&im.data[..6], &[0, 0, 255, 0, 0, 0]);
        assert_eq!(&im.data[6..], &[255, 0, 0, 0, 255, 0]);
        // 8 비트 색표 (BGRA): 0 = 파랑, 1 = 노랑
        let d = bmp_with(2, 1, 8, &[[255, 0, 0, 0], [0, 255, 255, 0]], &[0, 1, 0, 0]);
        let im = decode(&d).unwrap();
        assert_eq!(im.data, vec![0, 0, 255, 255, 255, 0]);
        // 1 비트: 첫 픽셀 1, 둘째 0
        let d = bmp_with(
            2,
            1,
            1,
            &[[0, 0, 0, 0], [255, 255, 255, 0]],
            &[0b1000_0000, 0, 0, 0],
        );
        let im = decode(&d).unwrap();
        assert_eq!(im.data, vec![255, 255, 255, 0, 0, 0]);
    }
}
