// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! JPEG 풀기 (ITU-T T.81 에서 직접 쓴 순수 Rust). 기본(순차, SOF0·SOF1)과 점진(SOF2) 허프만 JPEG, 8비트,
//! 성분 1개(회색)·3개(YCbCr, 한셀이 쓰는 JFIF)·4개(Adobe CMYK·YCCK), 재시작 구간, 성분마다 다른 표본 추출.
//! 웹 페이지 저장이 그림을 PNG 로 다시 쓸 때 쓴다 (한셀은 JPEG 그림도 RGBA PNG 로 넣는다).
//! 위로 표본 늘리기는 2배일 때 삼각 필터(libjpeg 의 "fancy"), 다른 배율은 되풀이한다.

/// 푼 그림: 성분 1 이면 회색, 3 이면 RGB (4성분은 RGB 로 바꿔 3 으로 돌려준다)
#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub data: Vec<u8>,
}

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

struct Huff {
    /// 길이 1..=16 의 첫 부호·마지막 부호·기호 시작 자리 (-1 = 그 길이 없음)
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    symbols: Vec<u8>,
}

impl Huff {
    fn new(counts: &[u8; 16], symbols: Vec<u8>) -> Huff {
        let mut maxcode = [-1i32; 18];
        let mut valptr = [0i32; 17];
        let mut mincode = [0i32; 17];
        let (mut code, mut k) = (0i32, 0i32);
        for l in 1..=16 {
            let n = counts[l - 1] as i32;
            valptr[l] = k;
            mincode[l] = code;
            if n > 0 {
                code += n;
                k += n;
                maxcode[l] = code - 1;
            }
            code <<= 1;
        }
        maxcode[17] = i32::MAX;
        Huff {
            maxcode,
            valptr,
            mincode,
            symbols,
        }
    }
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    /// 성분 그림의 블록 수 (MCU 덧붙임 포함)
    bw: usize,
    bh: usize,
    /// 성분 그림의 실제 크기 (올림)
    cw: usize,
    ch: usize,
    coef: Vec<i16>,
    dc_pred: i32,
    dc_tbl: usize,
    ac_tbl: usize,
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
    marker: Option<u8>,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.n <= 24 {
            let b = if self.marker.is_some() || self.pos >= self.d.len() {
                0
            } else {
                let b = self.d[self.pos];
                if b == 0xFF {
                    let nx = *self.d.get(self.pos + 1).unwrap_or(&0xD9);
                    if nx == 0 {
                        self.pos += 2;
                        0xFF
                    } else {
                        self.marker = Some(nx);
                        0
                    }
                } else {
                    self.pos += 1;
                    b
                }
            };
            self.acc |= (b as u32) << (24 - self.n);
            self.n += 8;
        }
    }

    fn bit(&mut self) -> u32 {
        self.bits(1)
    }

    fn bits(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        if self.n < k {
            self.fill();
        }
        let v = self.acc >> (32 - k);
        self.acc <<= k;
        self.n -= k;
        v
    }

    fn huff(&mut self, h: &Huff) -> Option<u8> {
        let mut code = 0i32;
        for l in 1..=16usize {
            code = (code << 1) | self.bit() as i32;
            if h.maxcode[l] >= 0 && code <= h.maxcode[l] && code >= h.mincode[l] {
                return h
                    .symbols
                    .get((h.valptr[l] + code - h.mincode[l]) as usize)
                    .copied();
            }
        }
        None
    }

    /// 비트 버퍼를 비우고 재시작 표지를 건너뛴다
    fn restart(&mut self) {
        self.acc = 0;
        self.n = 0;
        // 표지를 만난 자리에서 이어서: FFD0..D7
        if self.marker.is_none() {
            // 아직 표지까지 안 읽었다면 찾는다
            while self.pos + 1 < self.d.len() {
                if self.d[self.pos] == 0xFF
                    && self.d[self.pos + 1] != 0
                    && self.d[self.pos + 1] != 0xFF
                {
                    self.marker = Some(self.d[self.pos + 1]);
                    break;
                }
                self.pos += 1;
            }
        }
        if let Some(m) = self.marker
            && (0xD0..=0xD7).contains(&m)
        {
            self.pos += 2;
            self.marker = None;
        }
    }
}

fn extend(v: i32, t: u32) -> i32 {
    if t == 0 {
        0
    } else if v < (1 << (t - 1)) {
        v - (1 << t) + 1
    } else {
        v
    }
}

fn u16be(d: &[u8], p: usize) -> Option<usize> {
    Some(((*d.get(p)? as usize) << 8) | *d.get(p + 1)? as usize)
}

struct Scan {
    comps: Vec<usize>,
    ss: usize,
    se: usize,
    ah: u32,
    al: u32,
}

/// JPEG 바이트 → 그림. 모르는 꼴이면 None
pub fn decode(d: &[u8]) -> Option<Decoded> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return None;
    }
    let mut p = 2;
    let mut qt: [[u16; 64]; 4] = [[1; 64]; 4];
    let mut dc: [Option<Huff>; 4] = [None, None, None, None];
    let mut ac: [Option<Huff>; 4] = [None, None, None, None];
    let mut comps: Vec<Component> = Vec::new();
    let (mut w, mut h) = (0usize, 0usize);
    let mut progressive = false;
    let mut restart = 0usize;
    let mut adobe: Option<u8> = None;
    let (mut hmax, mut vmax) = (1usize, 1usize);
    let (mut mcux, mut mcuy) = (0usize, 0usize);
    let mut eobrun = 0u32;
    let mut got_scan = false;
    while p + 4 <= d.len() {
        if d[p] != 0xFF {
            p += 1;
            continue;
        }
        let m = d[p + 1];
        if m == 0xFF {
            p += 1;
            continue;
        }
        if m == 0 || (0xD0..=0xD8).contains(&m) || m == 0x01 {
            p += 2;
            continue;
        }
        if m == 0xD9 {
            break;
        }
        let len = u16be(d, p + 2)?;
        let seg = d.get(p + 4..p + 2 + len)?;
        match m {
            0xDB => {
                let mut q = 0;
                while q < seg.len() {
                    let (pq, tq) = (seg[q] >> 4, (seg[q] & 15) as usize);
                    q += 1;
                    if tq > 3 {
                        return None;
                    }
                    for z in 0..64 {
                        let v = if pq == 0 {
                            *seg.get(q)? as u16
                        } else {
                            u16be(seg, q)? as u16
                        };
                        q += if pq == 0 { 1 } else { 2 };
                        qt[tq][ZIGZAG[z]] = v;
                    }
                }
            }
            0xC4 => {
                let mut q = 0;
                while q + 17 <= seg.len() {
                    let (tc, th) = (seg[q] >> 4, (seg[q] & 15) as usize);
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&seg[q + 1..q + 17]);
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let syms = seg.get(q + 17..q + 17 + n)?.to_vec();
                    q += 17 + n;
                    if th > 3 {
                        return None;
                    }
                    let t = Huff::new(&counts, syms);
                    if tc == 0 {
                        dc[th] = Some(t);
                    } else {
                        ac[th] = Some(t);
                    }
                }
            }
            0xC0..=0xC2 => {
                if !comps.is_empty() || seg.len() < 6 || seg[0] != 8 {
                    return None;
                }
                progressive = m == 0xC2;
                h = u16be(seg, 1)?;
                w = u16be(seg, 3)?;
                let n = seg[5] as usize;
                if w == 0 || h == 0 || !(n == 1 || n == 3 || n == 4) || seg.len() < 6 + 3 * n {
                    return None;
                }
                for i in 0..n {
                    let (id, hv, tq) = (
                        seg[6 + 3 * i],
                        seg[7 + 3 * i],
                        (seg[8 + 3 * i] & 3) as usize,
                    );
                    let (ch, cv) = ((hv >> 4) as usize, (hv & 15) as usize);
                    if ch == 0 || cv == 0 || ch > 4 || cv > 4 {
                        return None;
                    }
                    comps.push(Component {
                        id,
                        h: ch,
                        v: cv,
                        tq,
                        bw: 0,
                        bh: 0,
                        cw: 0,
                        ch: 0,
                        coef: Vec::new(),
                        dc_pred: 0,
                        dc_tbl: 0,
                        ac_tbl: 0,
                    });
                }
                hmax = comps.iter().map(|c| c.h).max()?;
                vmax = comps.iter().map(|c| c.v).max()?;
                mcux = w.div_ceil(8 * hmax);
                mcuy = h.div_ceil(8 * vmax);
                if (mcux * mcuy) as u64 * 64 * (hmax * vmax * n) as u64 > 400_000_000 {
                    return None;
                }
                for c in comps.iter_mut() {
                    c.bw = mcux * c.h;
                    c.bh = mcuy * c.v;
                    c.cw = (w * c.h).div_ceil(hmax);
                    c.ch = (h * c.v).div_ceil(vmax);
                    c.coef = vec![0; c.bw * c.bh * 64];
                }
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCF => return None,
            0xDD => restart = u16be(seg, 0)?,
            0xEE if seg.starts_with(b"Adobe") && seg.len() >= 12 => adobe = Some(seg[11]),
            0xDA => {
                if comps.is_empty() {
                    return None;
                }
                let ns = *seg.first()? as usize;
                let mut sc = Scan {
                    comps: Vec::new(),
                    ss: 0,
                    se: 63,
                    ah: 0,
                    al: 0,
                };
                for i in 0..ns {
                    let cid = *seg.get(1 + 2 * i)?;
                    let tt = *seg.get(2 + 2 * i)?;
                    let ci = comps.iter().position(|c| c.id == cid)?;
                    comps[ci].dc_tbl = (tt >> 4) as usize & 3;
                    comps[ci].ac_tbl = (tt & 15) as usize & 3;
                    sc.comps.push(ci);
                }
                sc.ss = *seg.get(1 + 2 * ns)? as usize;
                sc.se = *seg.get(2 + 2 * ns)? as usize;
                let a = *seg.get(3 + 2 * ns)?;
                sc.ah = (a >> 4) as u32;
                sc.al = (a & 15) as u32;
                if !progressive {
                    sc.ss = 0;
                    sc.se = 63;
                    sc.ah = 0;
                    sc.al = 0;
                }
                if sc.se > 63 || sc.ss > sc.se {
                    return None;
                }
                let start = p + 2 + len;
                let mut bits = Bits {
                    d,
                    pos: start,
                    acc: 0,
                    n: 0,
                    marker: None,
                };
                decode_scan(
                    &mut bits,
                    &mut comps,
                    &sc,
                    &dc,
                    &ac,
                    restart,
                    (mcux, mcuy),
                    progressive,
                    &mut eobrun,
                )?;
                got_scan = true;
                // 표지까지 건너뛴다
                p = bits.pos;
                while p + 1 < d.len()
                    && !(d[p] == 0xFF
                        && d[p + 1] != 0
                        && !(0xD0..=0xD7).contains(&d[p + 1])
                        && d[p + 1] != 0xFF)
                {
                    p += 1;
                }
                continue;
            }
            _ => {}
        }
        p += 2 + len;
    }
    if !got_scan || comps.is_empty() {
        return None;
    }
    finish(&comps, &qt, w, h, (hmax, vmax), adobe)
}

#[allow(clippy::too_many_arguments)]
fn decode_scan(
    bits: &mut Bits,
    comps: &mut [Component],
    sc: &Scan,
    dc: &[Option<Huff>; 4],
    ac: &[Option<Huff>; 4],
    restart: usize,
    (mcux, mcuy): (usize, usize),
    progressive: bool,
    eobrun: &mut u32,
) -> Option<()> {
    for c in comps.iter_mut() {
        c.dc_pred = 0;
    }
    *eobrun = 0;
    // 블록 순서: 성분 하나면 성분 그림의 블록을 행 우선으로 (올림 크기), 둘 이상이면 MCU 안 h×v 블록
    let single = sc.comps.len() == 1;
    let total = if single {
        let c = &comps[sc.comps[0]];
        c.cw.div_ceil(8) * c.ch.div_ceil(8)
    } else {
        mcux * mcuy
    };
    let mut count = 0usize;
    for unit in 0..total {
        if restart > 0 && count == restart {
            bits.restart();
            for c in comps.iter_mut() {
                c.dc_pred = 0;
            }
            *eobrun = 0;
            count = 0;
        }
        count += 1;
        if single {
            let ci = sc.comps[0];
            let per_row = comps[ci].cw.div_ceil(8);
            let (bx, by) = (unit % per_row, unit / per_row);
            decode_block(
                bits,
                &mut comps[ci],
                bx,
                by,
                sc,
                dc,
                ac,
                progressive,
                eobrun,
            )?;
        } else {
            let (mx, my) = (unit % mcux, unit / mcux);
            for &ci in &sc.comps {
                let (ch, cv) = (comps[ci].h, comps[ci].v);
                for v in 0..cv {
                    for hh in 0..ch {
                        decode_block(
                            bits,
                            &mut comps[ci],
                            mx * ch + hh,
                            my * cv + v,
                            sc,
                            dc,
                            ac,
                            progressive,
                            eobrun,
                        )?;
                    }
                }
            }
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn decode_block(
    bits: &mut Bits,
    c: &mut Component,
    bx: usize,
    by: usize,
    sc: &Scan,
    dc: &[Option<Huff>; 4],
    ac: &[Option<Huff>; 4],
    progressive: bool,
    eobrun: &mut u32,
) -> Option<()> {
    if bx >= c.bw || by >= c.bh {
        return Some(());
    }
    let off = (by * c.bw + bx) * 64;
    if !progressive {
        let dct = dc[c.dc_tbl].as_ref()?;
        let act = ac[c.ac_tbl].as_ref()?;
        let t = bits.huff(dct)? as u32;
        let diff = if t == 0 {
            0
        } else {
            extend(bits.bits(t) as i32, t)
        };
        c.dc_pred += diff;
        c.coef[off] = c.dc_pred as i16;
        let mut k = 1;
        while k < 64 {
            let rs = bits.huff(act)?;
            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
            if s == 0 {
                if r == 15 {
                    k += 16;
                    continue;
                }
                break;
            }
            k += r;
            if k > 63 {
                break;
            }
            c.coef[off + ZIGZAG[k]] = extend(bits.bits(s) as i32, s) as i16;
            k += 1;
        }
        return Some(());
    }
    // 점진
    if sc.ss == 0 {
        if sc.ah == 0 {
            let dct = dc[c.dc_tbl].as_ref()?;
            let t = bits.huff(dct)? as u32;
            let diff = if t == 0 {
                0
            } else {
                extend(bits.bits(t) as i32, t)
            };
            c.dc_pred += diff;
            c.coef[off] = (c.dc_pred << sc.al) as i16;
        } else if bits.bit() != 0 {
            c.coef[off] |= (1 << sc.al) as i16;
        }
        return Some(());
    }
    let act = ac[c.ac_tbl].as_ref()?;
    let (ss, se, al) = (sc.ss, sc.se, sc.al);
    if sc.ah == 0 {
        if *eobrun > 0 {
            *eobrun -= 1;
            return Some(());
        }
        let mut k = ss;
        while k <= se {
            let rs = bits.huff(act)?;
            let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
            if s == 0 {
                if r < 15 {
                    *eobrun = (1 << r) - 1;
                    if r > 0 {
                        *eobrun += bits.bits(r as u32);
                    }
                    break;
                }
                k += 16;
                continue;
            }
            k += r;
            if k > 63 {
                break;
            }
            c.coef[off + ZIGZAG[k]] = (extend(bits.bits(s) as i32, s) << al) as i16;
            k += 1;
        }
        return Some(());
    }
    // AC 다듬기
    let p1 = 1i16 << al;
    let m1 = -1i16 << al;
    let mut k = ss;
    if *eobrun == 0 {
        while k <= se {
            let rs = bits.huff(act)?;
            let (mut r, s) = ((rs >> 4) as i32, (rs & 15) as i32);
            let mut newval = 0i16;
            if s != 0 {
                newval = if bits.bit() != 0 { p1 } else { m1 };
            } else if r != 15 {
                *eobrun = 1 << r;
                if r > 0 {
                    *eobrun += bits.bits(r as u32);
                }
                break;
            }
            while k <= se {
                let z = off + ZIGZAG[k];
                if c.coef[z] != 0 {
                    if bits.bit() != 0 && (c.coef[z] & p1) == 0 {
                        c.coef[z] += if c.coef[z] >= 0 { p1 } else { m1 };
                    }
                } else {
                    r -= 1;
                    if r < 0 {
                        break;
                    }
                }
                k += 1;
            }
            if newval != 0 && k <= 63 {
                c.coef[off + ZIGZAG[k]] = newval;
            }
            k += 1;
        }
    }
    if *eobrun > 0 {
        while k <= se {
            let z = off + ZIGZAG[k];
            if c.coef[z] != 0 && bits.bit() != 0 && (c.coef[z] & p1) == 0 {
                c.coef[z] += if c.coef[z] >= 0 { p1 } else { m1 };
            }
            k += 1;
        }
        *eobrun -= 1;
    }
    Some(())
}

/// 역 이산 코사인 변환 (분리형, 실수) → 0..255
fn idct(block: &[i16], q: &[u16; 64], cos: &[[f32; 8]; 8], out: &mut [u8; 64]) {
    let mut tmp = [0f32; 64];
    let mut deq = [0f32; 64];
    for i in 0..64 {
        deq[i] = block[i] as f32 * q[i] as f32;
    }
    // 행 (u → x)
    for v in 0..8 {
        for x in 0..8 {
            let mut s = 0.0;
            for u in 0..8 {
                s += deq[v * 8 + u] * cos[u][x];
            }
            tmp[v * 8 + x] = s;
        }
    }
    // 열 (v → y)
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0.0;
            for v in 0..8 {
                s += tmp[v * 8 + x] * cos[v][y];
            }
            out[y * 8 + x] = (s / 4.0 + 128.5).clamp(0.0, 255.0).floor() as u8;
        }
    }
}

fn finish(
    comps: &[Component],
    qt: &[[u16; 64]; 4],
    w: usize,
    h: usize,
    (hmax, vmax): (usize, usize),
    adobe: Option<u8>,
) -> Option<Decoded> {
    // 코사인 표
    let mut cos = [[0f32; 8]; 8];
    for (u, row) in cos.iter_mut().enumerate() {
        for (x, v) in row.iter_mut().enumerate() {
            let cu = if u == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            *v = cu * (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
        }
    }
    // 성분 그림 (블록 크기)
    let mut planes: Vec<(Vec<u8>, usize, usize)> = Vec::new();
    for c in comps {
        let (pw, ph) = (c.bw * 8, c.bh * 8);
        let mut plane = vec![0u8; pw * ph];
        let mut out = [0u8; 64];
        for by in 0..c.bh {
            for bx in 0..c.bw {
                let off = (by * c.bw + bx) * 64;
                idct(&c.coef[off..off + 64], &qt[c.tq], &cos, &mut out);
                for y in 0..8 {
                    let row = (by * 8 + y) * pw + bx * 8;
                    plane[row..row + 8].copy_from_slice(&out[y * 8..y * 8 + 8]);
                }
            }
        }
        planes.push((plane, pw, ph));
    }
    // 전체 크기로 표본 늘리기
    let mut full: Vec<Vec<u8>> = Vec::new();
    for (i, c) in comps.iter().enumerate() {
        let (plane, pw, _) = &planes[i];
        let (fx, fy) = (hmax / c.h.max(1), vmax / c.v.max(1));
        let (cw, ch) = (c.cw, c.ch);
        let mut out = vec![0u8; w * h];
        if fx == 1 && fy == 1 && hmax == c.h && vmax == c.v {
            for y in 0..h {
                out[y * w..y * w + w].copy_from_slice(&plane[y * pw..y * pw + w]);
            }
        } else if fx == 2 && fy == 1 && hmax % c.h == 0 {
            for y in 0..h {
                for x in 0..w {
                    let i = x / 2;
                    let cur = plane[y * pw + i] as i32;
                    let nb = if x % 2 == 0 {
                        if i > 0 {
                            plane[y * pw + i - 1] as i32
                        } else {
                            cur
                        }
                    } else if i + 1 < cw {
                        plane[y * pw + i + 1] as i32
                    } else {
                        cur
                    };
                    out[y * w + x] = if x % 2 == 0 {
                        ((3 * cur + nb + 1) >> 2) as u8
                    } else {
                        ((3 * cur + nb + 2) >> 2) as u8
                    };
                }
            }
        } else if fx == 2 && fy == 2 {
            for y in 0..h {
                let iy = y / 2;
                let ny = if y % 2 == 0 {
                    iy.saturating_sub(1)
                } else {
                    (iy + 1).min(ch - 1)
                };
                for x in 0..w {
                    let i = x / 2;
                    let col =
                        |ix: usize| 3 * plane[iy * pw + ix] as i32 + plane[ny * pw + ix] as i32;
                    let cur = col(i);
                    let nb = if x % 2 == 0 {
                        if i > 0 { col(i - 1) } else { cur }
                    } else if i + 1 < cw {
                        col(i + 1)
                    } else {
                        cur
                    };
                    out[y * w + x] = if x % 2 == 0 {
                        ((3 * cur + nb + 8) >> 4) as u8
                    } else {
                        ((3 * cur + nb + 7) >> 4) as u8
                    };
                }
            }
        } else {
            for y in 0..h {
                let sy = (y * c.v / vmax).min(ch.saturating_sub(1));
                for x in 0..w {
                    let sx = (x * c.h / hmax).min(cw.saturating_sub(1));
                    out[y * w + x] = plane[sy * pw + sx];
                }
            }
        }
        full.push(out);
    }
    let clamp = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    match comps.len() {
        1 => Some(Decoded {
            width: w as u32,
            height: h as u32,
            components: 1,
            data: full.remove(0),
        }),
        3 => {
            // Adobe 변환 0 이면 이미 RGB, 아니면 YCbCr
            let ycc = adobe != Some(0);
            let mut data = Vec::with_capacity(w * h * 3);
            for ((&a, &b), &c) in full[0].iter().zip(&full[1]).zip(&full[2]) {
                let (a, b, c) = (a as f32, b as f32, c as f32);
                if ycc {
                    data.push(clamp(a + 1.402 * (c - 128.0)));
                    data.push(clamp(a - 0.344_136 * (b - 128.0) - 0.714_136 * (c - 128.0)));
                    data.push(clamp(a + 1.772 * (b - 128.0)));
                } else {
                    data.extend_from_slice(&[a as u8, b as u8, c as u8]);
                }
            }
            Some(Decoded {
                width: w as u32,
                height: h as u32,
                components: 3,
                data,
            })
        }
        4 => {
            // Adobe CMYK(역값)·YCCK → RGB
            let ycck = adobe == Some(2);
            let mut data = Vec::with_capacity(w * h * 3);
            for (((&a, &b), &c), &kk) in full[0].iter().zip(&full[1]).zip(&full[2]).zip(&full[3]) {
                let (mut c0, mut c1, mut c2) = (a as f32, b as f32, c as f32);
                let k = kk as f32;
                if ycck {
                    let (y, cb, cr) = (c0, c1, c2);
                    c0 = (y + 1.402 * (cr - 128.0)).clamp(0.0, 255.0);
                    c1 =
                        (y - 0.344_136 * (cb - 128.0) - 0.714_136 * (cr - 128.0)).clamp(0.0, 255.0);
                    c2 = (y + 1.772 * (cb - 128.0)).clamp(0.0, 255.0);
                }
                // Adobe 는 값을 뒤집어 저장한다 (255 = 잉크 없음)
                data.push(clamp(c0 * k / 255.0));
                data.push(clamp(c1 * k / 255.0));
                data.push(clamp(c2 * k / 255.0));
            }
            Some(Decoded {
                width: w as u32,
                height: h as u32,
                components: 3,
                data,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8×8 회색 한 블록 JPEG (DC 만, 밝기 128 + 16): 손으로 만든 최소 파일
    fn tiny_gray() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        // DQT: 모두 1
        v.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x43, 0x00]);
        v.extend(std::iter::repeat_n(1u8, 64));
        // SOF0: 8비트 8×8, 성분 1 (1x1, tq 0)
        v.extend_from_slice(&[
            0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00,
        ]);
        // DHT DC: 길이 2 부호 하나(범주 0), 길이 ... 간단히 범주 5 하나만: 길이 1 에 기호 5
        v.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, 0x00]);
        let mut counts = [0u8; 16];
        counts[0] = 1;
        v.extend_from_slice(&counts);
        v.push(5);
        // DHT AC: 길이 1 에 기호 0x00 (EOB)
        v.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x14, 0x10]);
        v.extend_from_slice(&counts);
        v.push(0x00);
        // SOS
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
        // 데이터: DC 부호 '0' (범주 5) + 5비트 값 10000 (=16) + AC 부호 '0' (EOB) → 비트 0 10000 0 → 0100 0000 = 0x40
        v.push(0x40);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    #[test]
    fn decodes_flat_gray_block() {
        let img = decode(&tiny_gray()).unwrap();
        assert_eq!((img.width, img.height, img.components), (8, 8, 1));
        // DC 16 → 평균 16/8 = 2 → 128 + 2
        assert!(img.data.iter().all(|&b| b == 130), "{:?}", &img.data[..8]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(&[1, 2, 3, 4]).is_none());
        assert!(decode(&[0xFF, 0xD8, 0xFF, 0xD9]).is_none());
    }
}
