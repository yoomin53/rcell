// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! Excel 97-2003 `.xls` (BIFF8, [MS-XLS]) ↔ 한셀 레거시 레코드.
//!
//! 한셀 레거시(BOF 0x1001)는 BIFF8 의 변형이다 (docs/format/cell-legacy.md). rcell 은 레거시 읽기·쓰기를 그대로
//! 쓰고 `/Workbook` 의 레코드만 서로 바꾼다: [`to_biff8`] 는 레거시 레코드 → BIFF8 (`.xls` 쓰기), [`to_hancom`] 는
//! BIFF8 → 레거시 레코드 (`.xls` 읽기). 바꾸는 규칙은 한셀이 같은 문서를 레거시(-2005)와 `.xls`(56)로 저장한 것을
//! 레코드마다 맞대 본 결과다 (docs/format/other-formats.md 5절):
//!
//! - 행 번호 u32 → u16 (셀·ROW·범위·수식 토큰·PANE·WINDOW2·SELECTION·NOTE·페이지 나누기·그리기 앵커),
//!   65536 행·256 열 밖의 셀은 버린다
//! - 문자열 플래그: 레거시는 플래그와 상관없이 UTF-16 — BIFF8 은 플래그 1 (읽을 때는 플래그 0 의 1바이트 글자를
//!   UTF-16 으로 늘린다)
//! - FONT·XF 의 한컴 확장 영역을 뗀다 (색은 XFEXT 에 같이 있다), XFCRC 를 더한다
//! - SST 는 서식 런을 글자 앞으로 옮기고 CONTINUE 로 나눈다 (EXTSST 를 더한다), TXO 의 글·런은 CONTINUE 로
//! - 한컴 전용 레코드(0xA030·0x0900·0xA001·0xA011·0xA020·0x009F·0xB00E·0x7EF0)를 빼고, 행 반복 0xB002 는 풀어 쓴다
//! - ROW 는 32 행 묶음마다 셀 앞에, WINDOW2 뒤에 PLV

use super::biff;
use crate::error::{Error, Result};

/// BIFF8 BOF 버전
pub const BIFF8_VERSION: u16 = 0x0600;
/// BIFF8 레코드 데이터 최대 길이 (넘으면 CONTINUE)
const MAX_DATA: usize = 8224;
const MAX_ROW8: u32 = 0xFFFF;
const MAX_COL8: u32 = 0xFF;
const MAX_ROW_H: u32 = 1_048_575;
const MAX_COL_H: u32 = 0x3FFF;

fn bad(what: &str) -> Error {
    Error::InvalidValue {
        context: ".xls (BIFF8)".into(),
        value: what.into(),
    }
}

/// `/Workbook` 스트림이 BIFF8 인가 (BOF 버전 0x0600)
pub fn is_biff8(stream: &[u8]) -> bool {
    stream.len() >= 6
        && stream[0..2] == [0x09, 0x08]
        && u16::from_le_bytes([stream[4], stream[5]]) == BIFF8_VERSION
}

// ---------------------------------------------------------------------------
// 레코드
// ---------------------------------------------------------------------------

/// 논리 레코드: 데이터와 뒤따른 CONTINUE 조각들
#[derive(Debug, Clone, Default)]
struct Rec {
    rt: u16,
    data: Vec<u8>,
    conts: Vec<Vec<u8>>,
}

impl Rec {
    fn new(rt: u16, data: Vec<u8>) -> Rec {
        Rec {
            rt,
            data,
            conts: Vec::new(),
        }
    }
    /// CONTINUE 를 이어 붙인 데이터
    fn joined(&self) -> Vec<u8> {
        let mut d = self.data.clone();
        for c in &self.conts {
            d.extend_from_slice(c);
        }
        d
    }
}

/// BIFF8 스트림 → 논리 레코드 (CONTINUE 는 앞 레코드의 조각으로)
fn split_biff8(stream: &[u8]) -> Vec<Rec> {
    let mut out: Vec<Rec> = Vec::new();
    let mut pos = 0;
    while pos + 4 <= stream.len() {
        let rt = u16::from_le_bytes([stream[pos], stream[pos + 1]]);
        let len = u16::from_le_bytes([stream[pos + 2], stream[pos + 3]]) as usize;
        let end = (pos + 4 + len).min(stream.len());
        let data = stream[pos + 4..end].to_vec();
        pos += 4 + len;
        if rt == 0x003C
            && let Some(last) = out.last_mut()
        {
            last.conts.push(data);
            continue;
        }
        out.push(Rec::new(rt, data));
    }
    out
}

/// BIFF8 레코드 하나를 쓴다 (긴 데이터는 8224 바이트마다 CONTINUE)
fn put8(out: &mut Vec<u8>, rt: u16, data: &[u8]) {
    let mut chunks = data.chunks(MAX_DATA);
    let first = chunks.next().unwrap_or(&[]);
    out.extend_from_slice(&rt.to_le_bytes());
    out.extend_from_slice(&(first.len() as u16).to_le_bytes());
    out.extend_from_slice(first);
    for c in chunks {
        out.extend_from_slice(&0x003Cu16.to_le_bytes());
        out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        out.extend_from_slice(c);
    }
}

/// 레거시 레코드 하나를 쓴다 (길이 0x8000 이상은 확장 길이)
fn put_h(out: &mut Vec<u8>, rt: u16, data: &[u8]) {
    out.extend_from_slice(&rt.to_le_bytes());
    let n = data.len();
    if n < 0x8000 {
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else {
        out.extend_from_slice(&(((n & 0x7FFF) | 0x8000) as u16).to_le_bytes());
        out.extend_from_slice(&((n >> 15) as u16).to_le_bytes());
    }
    out.extend_from_slice(data);
}

// ---------------------------------------------------------------------------
// 바꾸기 도우미 — 같은 코드로 두 방향을 쓴다
// ---------------------------------------------------------------------------

/// 원본 바이트를 읽으며 다른 꼴로 쓴다. `to8` 이면 레거시 → BIFF8, 아니면 BIFF8 → 레거시
struct Cv<'a> {
    s: &'a [u8],
    p: usize,
    o: Vec<u8>,
    to8: bool,
    /// 행·열이 BIFF8 범위를 넘었다 (셀 레코드는 버린다)
    over: bool,
}

impl<'a> Cv<'a> {
    fn new(s: &'a [u8], to8: bool) -> Cv<'a> {
        Cv {
            s,
            p: 0,
            o: Vec::with_capacity(s.len()),
            to8,
            over: false,
        }
    }
    fn left(&self) -> usize {
        self.s.len().saturating_sub(self.p)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.s.get(self.p..self.p + n)?;
        self.p += n;
        Some(b)
    }
    fn copy(&mut self, n: usize) -> Option<()> {
        let b = self.take(n)?;
        self.o.extend_from_slice(b);
        Some(())
    }
    fn rest(&mut self) {
        let b = &self.s[self.p.min(self.s.len())..];
        self.o.extend_from_slice(b);
        self.p = self.s.len();
    }
    fn r8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn r16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn r32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn w8(&mut self, v: u8) {
        self.o.push(v);
    }
    fn w16(&mut self, v: u16) {
        self.o.extend_from_slice(&v.to_le_bytes());
    }
    fn w32(&mut self, v: u32) {
        self.o.extend_from_slice(&v.to_le_bytes());
    }
    /// 행 번호: 레거시 u32 ↔ BIFF8 u16 (넘으면 0xFFFF, `over`)
    fn row(&mut self) -> Option<u32> {
        if self.to8 {
            let v = self.r32()?;
            if v > MAX_ROW8 {
                self.over = true;
            }
            self.w16(v.min(MAX_ROW8) as u16);
            Some(v)
        } else {
            let v = self.r16()? as u32;
            self.w32(v);
            Some(v)
        }
    }
    /// 열 번호 u16 (BIFF8 은 255 까지)
    fn col(&mut self) -> Option<u32> {
        let v = self.r16()? as u32;
        if self.to8 && v > MAX_COL8 {
            self.over = true;
        }
        self.w16(if self.to8 { v.min(MAX_COL8) } else { v } as u16);
        Some(v)
    }
    /// 범위: 레거시 (행 u32 ×2, 열 u16 ×2) ↔ BIFF8 Ref8 (u16 ×4). 끝 행·열은 시트 끝에 맞춘다
    fn ref8(&mut self) -> Option<()> {
        let (r1, r2, c1, c2) = self.read_range(false)?;
        self.write_range(r1, r2, c1, c2, false);
        Some(())
    }
    /// 범위: 레거시 (행 u32 ×2, 열 u16 ×2) ↔ BIFF8 RefU (행 u16 ×2, 열 u8 ×2)
    fn ref_u(&mut self) -> Option<()> {
        let (r1, r2, c1, c2) = self.read_range(true)?;
        self.write_range(r1, r2, c1, c2, true);
        Some(())
    }
    fn read_range(&mut self, narrow: bool) -> Option<(u32, u32, u32, u32)> {
        if self.to8 {
            Some((
                self.r32()?,
                self.r32()?,
                self.r16()? as u32,
                self.r16()? as u32,
            ))
        } else if narrow {
            Some((
                self.r16()? as u32,
                self.r16()? as u32,
                self.r8()? as u32,
                self.r8()? as u32,
            ))
        } else {
            Some((
                self.r16()? as u32,
                self.r16()? as u32,
                self.r16()? as u32,
                self.r16()? as u32,
            ))
        }
    }
    fn write_range(&mut self, r1: u32, r2: u32, c1: u32, c2: u32, narrow: bool) {
        if self.to8 {
            if r1 > MAX_ROW8 || c1 > MAX_COL8 {
                self.over = true;
            }
            let (r1, r2) = (r1.min(MAX_ROW8), r2.min(MAX_ROW8));
            let (c1, c2) = (c1.min(MAX_COL8), c2.min(MAX_COL8));
            if narrow {
                self.w16(r1 as u16);
                self.w16(r2 as u16);
                self.w8(c1 as u8);
                self.w8(c2 as u8);
            } else {
                self.w16(r1 as u16);
                self.w16(r2 as u16);
                self.w16(c1 as u16);
                self.w16(c2 as u16);
            }
        } else {
            let r2 = if r2 == MAX_ROW8 { MAX_ROW_H } else { r2 };
            let c2 = if c2 == MAX_COL8 { MAX_COL_H } else { c2 };
            self.w32(r1);
            self.w32(r2);
            self.w16(c1 as u16);
            self.w16(c2 as u16);
        }
    }
    /// 플래그 + 글자 (길이는 따로): 레거시는 UTF-16 (플래그 무시) — BIFF8 은 플래그 1 로 쓰고, 읽을 때는
    /// 플래그 비트 0 이 0 이면 1바이트 글자다
    fn chars(&mut self, cch: usize) -> Option<()> {
        let flag = self.r8()?;
        if self.to8 {
            self.w8(0x01);
            self.copy(cch * 2)
        } else {
            self.w8(0x01);
            if flag & 0x01 != 0 {
                self.copy(cch * 2)
            } else {
                let b = self.take(cch)?;
                for &c in b {
                    self.w16(c as u16);
                }
                Some(())
            }
        }
    }
    /// XLUnicodeString (cch u16 + 플래그 + 글자)
    fn str16(&mut self) -> Option<()> {
        let cch = self.r16()?;
        self.w16(cch);
        self.chars(cch as usize)
    }
    /// ShortXLUnicodeString (cch u8 + 플래그 + 글자)
    fn str8(&mut self) -> Option<()> {
        let cch = self.r8()?;
        self.w8(cch);
        self.chars(cch as usize)
    }
    /// cce u16 + 수식 토큰
    fn rgce(&mut self, rel: bool) -> Option<()> {
        let cce = self.r16()? as usize;
        let t = self.take(cce)?;
        let n = ptgs(t, self.to8, rel)?;
        self.w16(n.len() as u16);
        self.o.extend_from_slice(&n);
        Some(())
    }
    fn done(self) -> Vec<u8> {
        self.o
    }
}

// ---------------------------------------------------------------------------
// 수식 토큰
// ---------------------------------------------------------------------------

/// 토큰 크기 (토큰 바이트 뒤 데이터 길이). PtgStr·PtgAttr choose 는 따로 센다
fn ptg_len(base: u8, h: bool) -> Option<usize> {
    let rw = if h { 4 } else { 2 };
    Some(match base {
        0x01 | 0x02 => rw + 2,
        0x03..=0x16 => 0,
        0x1C | 0x1D => 1,
        0x1E => 2,
        0x1F => 8,
        0x20 => 7,
        0x21 => 2,
        0x22 => 3,
        0x23 => 4,
        0x24 | 0x2A | 0x2C => rw + 2,
        0x25 | 0x2B | 0x2D => rw * 2 + 4,
        0x26 | 0x27 => 6,
        0x28 | 0x29 | 0x2E | 0x2F => 2,
        0x39 => 6,
        0x3A | 0x3C => 2 + rw + 2,
        0x3B | 0x3D => 2 + rw * 2 + 4,
        _ => return None,
    })
}

/// 열 필드 (14비트 열 + 0x4000 열 상대 + 0x8000 행 상대). 상대 오프셋이면 레거시 14비트 ↔ BIFF8 8비트 부호
fn col_field(v: u16, offset: bool, to8: bool) -> (u16, bool) {
    let flags = v & 0xC000;
    let col_rel = v & 0x4000 != 0;
    if offset && col_rel {
        if to8 {
            let off = ((v << 2) as i16) >> 2; // 14비트 부호 확장
            if !(-128..=127).contains(&off) {
                return (flags, true);
            }
            (flags | (off as i8 as u8 as u16), false)
        } else {
            let off = (v & 0xFF) as u8 as i8 as i16;
            (flags | ((off as u16) & 0x3FFF), false)
        }
    } else {
        let c = (v & 0x3FFF) as u32;
        if to8 && c > MAX_COL8 {
            (flags | MAX_COL8 as u16, true)
        } else {
            (v, false)
        }
    }
}

/// 행 값: 상대 오프셋이면 i32 ↔ i16, 아니면 u32 ↔ u16
fn row_value(v: u32, offset_rel: bool, to8: bool) -> (u32, bool) {
    if to8 {
        if offset_rel {
            // 레거시의 상대 행은 20비트 (0xFFFF9 = -7)
            let off = ((v << 12) as i32) >> 12;
            if !(-32768..=32767).contains(&off) {
                return (0, true);
            }
            (off as i16 as u16 as u32, false)
        } else if v > MAX_ROW8 {
            (MAX_ROW8, true)
        } else {
            (v, false)
        }
    } else if offset_rel {
        ((v as u16 as i16 as i32 as u32) & 0xF_FFFF, false)
    } else {
        (v, false)
    }
}

/// 수식 토큰열을 다른 꼴로. `rel` 이면 3차원 참조의 상대 축도 오프셋이다 (공유 수식·조건부 서식·유효성)
pub(crate) fn ptgs(src: &[u8], to8: bool, rel: bool) -> Option<Vec<u8>> {
    let h_in = to8; // 원본이 레거시인가
    let rw_in = if h_in { 4 } else { 2 };
    let mut out: Vec<u8> = Vec::with_capacity(src.len());
    // 토큰 시작 위치 대응 (원본 → 결과), 그리고 고쳐야 할 점프
    let mut map: Vec<(usize, usize)> = Vec::new();
    // (결과 안 위치, 종류, 원본 기준 위치, 원본 값): 종류 0 if, 1 goto, 2 choose 항목, 3 mem cce
    let mut fixes: Vec<(usize, u8, usize, usize)> = Vec::new();
    let mut p = 0;
    let rd16 = |b: &[u8], i: usize| -> Option<u16> {
        Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
    };
    let rd32 = |b: &[u8], i: usize| -> Option<u32> {
        Some(u32::from_le_bytes([
            *b.get(i)?,
            *b.get(i + 1)?,
            *b.get(i + 2)?,
            *b.get(i + 3)?,
        ]))
    };
    let rd_row = |b: &[u8], i: usize| -> Option<u32> {
        if h_in {
            rd32(b, i)
        } else {
            rd16(b, i).map(u32::from)
        }
    };
    let w_row = |o: &mut Vec<u8>, v: u32| {
        if to8 {
            o.extend_from_slice(&(v as u16).to_le_bytes());
        } else {
            o.extend_from_slice(&v.to_le_bytes());
        }
    };
    while p < src.len() {
        map.push((p, out.len()));
        let ptg = src[p];
        let base = if (0x20..0x80).contains(&ptg) {
            (ptg & 0x1F) | 0x20
        } else {
            ptg
        };
        match base {
            0x17 => {
                // PtgStr: 레거시 cch u16, BIFF8 cch u8
                let (cch, at) = if h_in {
                    (rd16(src, p + 1)? as usize, p + 3)
                } else {
                    (*src.get(p + 1)? as usize, p + 2)
                };
                let flag = *src.get(at)?;
                let n = if flag & 1 != 0 { cch * 2 } else { cch };
                let chars = src.get(at + 1..at + 1 + n)?;
                out.push(ptg);
                if to8 {
                    let keep = cch.min(255);
                    out.push(keep as u8);
                    out.push(flag);
                    out.extend_from_slice(&chars[..if flag & 1 != 0 { keep * 2 } else { keep }]);
                } else {
                    out.extend_from_slice(&(cch as u16).to_le_bytes());
                    out.push(flag);
                    out.extend_from_slice(chars);
                }
                p = at + 1 + n;
            }
            0x18 => {
                // 확장: PtgList (0x19) 12바이트, PtgSxName (0x1D) 4바이트
                let eptg = *src.get(p + 1)?;
                let n = match eptg {
                    0x19 => 12,
                    0x1D => 4,
                    _ => return None,
                };
                out.extend_from_slice(src.get(p..p + 2 + n)?);
                p += 2 + n;
            }
            0x19 => {
                let grbit = *src.get(p + 1)?;
                let data = rd16(src, p + 2)?;
                out.extend_from_slice(src.get(p..p + 2)?);
                let after_src = p + 4;
                if grbit & 0x04 != 0 {
                    // choose: cOffset u16 + (cOffset + 1) × u16, 기준은 rgOffset 의 시작
                    out.extend_from_slice(&data.to_le_bytes());
                    let n = data as usize + 1;
                    for k in 0..n {
                        let v = rd16(src, after_src + k * 2)? as usize;
                        fixes.push((out.len(), 2, after_src, v));
                        out.extend_from_slice(&0u16.to_le_bytes());
                    }
                    p = after_src + n * 2;
                    continue;
                }
                if grbit & 0x02 != 0 {
                    fixes.push((out.len(), 0, after_src, data as usize));
                } else if grbit & 0x08 != 0 {
                    fixes.push((out.len(), 1, after_src, data as usize));
                }
                out.extend_from_slice(&data.to_le_bytes());
                p = after_src;
            }
            0x01 | 0x02 => {
                let row = rd_row(src, p + 1)?;
                let col = rd16(src, p + 1 + rw_in)?;
                out.push(ptg);
                w_row(&mut out, row_value(row, false, to8).0);
                out.extend_from_slice(&col.to_le_bytes());
                p += 1 + rw_in + 2;
            }
            0x24 | 0x2A | 0x2C => {
                let offset = base == 0x2C;
                let row = rd_row(src, p + 1)?;
                let colf = rd16(src, p + 1 + rw_in)?;
                let (r, e1) = row_value(row, offset && colf & 0x8000 != 0, to8);
                let (c, e2) = col_field(colf, offset, to8);
                if to8 && base == 0x24 && (e1 || e2) {
                    // BIFF8 시트 밖 → PtgRefErr
                    out.push(ptg + 6);
                    out.extend_from_slice(&[0; 4]);
                } else {
                    out.push(ptg);
                    w_row(&mut out, r);
                    out.extend_from_slice(&c.to_le_bytes());
                }
                p += 1 + rw_in + 2;
            }
            0x25 | 0x2B | 0x2D => {
                let offset = base == 0x2D;
                let r1 = rd_row(src, p + 1)?;
                let r2 = rd_row(src, p + 1 + rw_in)?;
                let c1f = rd16(src, p + 1 + rw_in * 2)?;
                let c2f = rd16(src, p + 3 + rw_in * 2)?;
                let (a, e1) = row_value(r1, offset && c1f & 0x8000 != 0, to8);
                let (mut b, _) = row_value(r2, offset && c2f & 0x8000 != 0, to8);
                let (c, e3) = col_field(c1f, offset, to8);
                let (mut d, _) = col_field(c2f, offset, to8);
                if !offset {
                    // 열 전체·행 전체: BIFF8 65535·255 ↔ 레거시 1048575·16383
                    if !to8 && b == MAX_ROW8 {
                        b = MAX_ROW_H;
                    }
                    if !to8 && (d & 0x3FFF) as u32 == MAX_COL8 && (c & 0x3FFF) == 0 {
                        d = (d & 0xC000) | MAX_COL_H as u16;
                    }
                }
                if to8 && base == 0x25 && (e1 || e3) {
                    out.push(ptg + 6);
                    out.extend_from_slice(&[0; 8]);
                } else {
                    out.push(ptg);
                    w_row(&mut out, a);
                    w_row(&mut out, b);
                    out.extend_from_slice(&c.to_le_bytes());
                    out.extend_from_slice(&d.to_le_bytes());
                }
                p += 1 + rw_in * 2 + 4;
            }
            0x3A | 0x3C => {
                let ixti = rd16(src, p + 1)?;
                let row = rd_row(src, p + 3)?;
                let colf = rd16(src, p + 3 + rw_in)?;
                let (r, e1) = row_value(row, rel && colf & 0x8000 != 0, to8);
                let (c, e2) = col_field(colf, rel, to8);
                if to8 && base == 0x3A && (e1 || e2) {
                    out.push(ptg + 2);
                    out.extend_from_slice(&ixti.to_le_bytes());
                    out.extend_from_slice(&[0; 4]);
                } else {
                    out.push(ptg);
                    out.extend_from_slice(&ixti.to_le_bytes());
                    w_row(&mut out, r);
                    out.extend_from_slice(&c.to_le_bytes());
                }
                p += 3 + rw_in + 2;
            }
            0x3B | 0x3D => {
                let ixti = rd16(src, p + 1)?;
                let r1 = rd_row(src, p + 3)?;
                let r2 = rd_row(src, p + 3 + rw_in)?;
                let c1f = rd16(src, p + 3 + rw_in * 2)?;
                let c2f = rd16(src, p + 5 + rw_in * 2)?;
                let (a, e1) = row_value(r1, rel && c1f & 0x8000 != 0, to8);
                let (mut b, _) = row_value(r2, rel && c2f & 0x8000 != 0, to8);
                let (c, e3) = col_field(c1f, rel, to8);
                let (mut d, _) = col_field(c2f, rel, to8);
                if !rel && !to8 {
                    if b == MAX_ROW8 {
                        b = MAX_ROW_H;
                    }
                    if (d & 0x3FFF) as u32 == MAX_COL8 && (c & 0x3FFF) == 0 {
                        d = (d & 0xC000) | MAX_COL_H as u16;
                    }
                }
                if to8 && base == 0x3B && (e1 || e3) {
                    out.push(ptg + 2);
                    out.extend_from_slice(&ixti.to_le_bytes());
                    out.extend_from_slice(&[0; 8]);
                } else {
                    out.push(ptg);
                    out.extend_from_slice(&ixti.to_le_bytes());
                    w_row(&mut out, a);
                    w_row(&mut out, b);
                    out.extend_from_slice(&c.to_le_bytes());
                    out.extend_from_slice(&d.to_le_bytes());
                }
                p += 3 + rw_in * 2 + 4;
            }
            0x26..=0x29 | 0x2E | 0x2F => {
                // PtgMem*: 뒤따르는 하위 식의 길이 cce
                let n = ptg_len(base, h_in)?;
                let cce_at = p + 1 + n - 2;
                let cce = rd16(src, cce_at)? as usize;
                out.extend_from_slice(src.get(p..cce_at)?);
                fixes.push((out.len(), 3, p + 1 + n, cce));
                out.extend_from_slice(&[0, 0]);
                p += 1 + n;
            }
            _ => {
                let n = ptg_len(base, h_in)?;
                out.extend_from_slice(src.get(p..p + 1 + n)?);
                p += 1 + n;
            }
        }
    }
    map.push((p, out.len()));
    // 원본 위치 → 결과 위치 (토큰 경계가 아니면 같은 토큰 안의 거리를 그대로)
    let at = |x: usize| -> usize {
        match map.binary_search_by_key(&x, |&(s, _)| s) {
            Ok(i) => map[i].1,
            Err(i) => {
                let (s, d) = map[i.saturating_sub(1)];
                d + (x - s)
            }
        }
    };
    for (pos, kind, anchor_src, v) in fixes {
        let anchor_dst = at(anchor_src);
        let nv = match kind {
            // goto: 건너뛸 바이트 수 − 1
            1 => at(anchor_src + v + 1).saturating_sub(anchor_dst + 1),
            _ => at(anchor_src + v).saturating_sub(anchor_dst),
        };
        out[pos..pos + 2].copy_from_slice(&(nv as u16).to_le_bytes());
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// 레코드 하나
// ---------------------------------------------------------------------------

/// 셀 레코드 머리: 행, 열, XF
fn cell_head(cv: &mut Cv) -> Option<()> {
    cv.row()?;
    cv.col()?;
    cv.copy(2)
}

/// 수식 토큰 + (BIFF8) 뒤따르는 RgbExtra
fn formula_tail(cv: &mut Cv, rel: bool) -> Option<()> {
    cv.rgce(rel)?;
    cv.rest();
    Some(())
}

/// 행·열이 시트 밖이면 버리는 레코드
fn drops_over(rt: u16) -> bool {
    matches!(
        rt,
        0x00FD
            | 0x0203
            | 0x027E
            | 0x0201
            | 0x0205
            | 0x0204
            | 0x00D6
            | 0x0006
            | 0x0208
            | 0x001C
            | 0x00BD
            | 0x00BE
            | 0x04BC
            | 0x0221
            | 0x0236
    )
}

/// 레코드 하나를 바꾼다 (같은 종류로). None = 버린다. 스트림 단위로 다루는 레코드(BOF·SST·XF·TXO·
/// MSODRAWING …)는 여기 오지 않는다
fn conv(rt: u16, d: &[u8], to8: bool) -> Option<Vec<u8>> {
    let mut cv = Cv::new(d, to8);
    match rt {
        // 셀
        0x00FD | 0x027E => {
            cell_head(&mut cv)?;
            cv.copy(4)?;
        }
        0x0203 => {
            cell_head(&mut cv)?;
            cv.copy(8)?;
        }
        0x0201 => cell_head(&mut cv)?,
        0x0205 => {
            cell_head(&mut cv)?;
            cv.copy(2)?;
        }
        0x0204 | 0x00D6 => {
            // LABEL, RSTRING (서식 런은 버린다 — 종류는 conv_sheet 에서 LABEL 로)
            cell_head(&mut cv)?;
            cv.str16()?;
        }
        0x00BD | 0x00BE => {
            // MULRK·MULBLANK: 행, 첫 열, 항목(6·2바이트) …, 끝 열
            cv.row()?;
            let first = cv.r16()? as u32;
            let size = if rt == 0x00BD { 6 } else { 2 };
            let items = cv.left().saturating_sub(2) / size;
            let mut n = items;
            if to8 {
                if first > MAX_COL8 {
                    return None;
                }
                n = n.min((MAX_COL8 - first + 1) as usize);
            }
            if n == 0 {
                return None;
            }
            cv.w16(first as u16);
            cv.copy(n * size)?;
            cv.take((items - n) * size)?;
            cv.r16()?;
            cv.w16((first + n as u32 - 1) as u16);
        }
        0x0006 => {
            cell_head(&mut cv)?;
            cv.copy(14)?;
            formula_tail(&mut cv, false)?;
        }
        0x0207 => cv.str16()?,
        0x0208 => {
            // 셀이 없는 행: 레거시 colMic·colMac 0 ↔ BIFF8 colMac 256 (한셀의 .xls 저장)
            cv.row()?;
            let c1 = cv.r16()?;
            let c2 = cv.r16()?;
            cv.w16(c1);
            cv.w16(match (to8, c1, c2) {
                (true, 0, 0) => 256,
                (false, 0, 256) => 0,
                _ => c2,
            });
            cv.rest();
        }
        0x04BC => {
            cv.ref_u()?;
            cv.copy(2)?;
            formula_tail(&mut cv, true)?;
        }
        0x0221 => {
            cv.ref_u()?;
            cv.copy(6)?;
            formula_tail(&mut cv, false)?;
        }
        0x0236 => {
            // TABLE: 범위, grbit, 입력 두 칸 (없으면 레거시 u32::MAX ↔ BIFF8 0xFFFF)
            cv.ref_u()?;
            cv.copy(2)?;
            for _ in 0..2 {
                if to8 {
                    let r = cv.r32()?;
                    cv.w16(r.min(MAX_ROW8) as u16);
                } else {
                    let r = cv.r16()?;
                    cv.w32(if r == 0xFFFF { u32::MAX } else { r as u32 });
                }
                cv.copy(2)?;
            }
        }
        0x0200 => {
            // DIMENSIONS: rwMic u32, rwMac u32, colMic u16, colMac u16 (두 꼴 같음, BIFF8 은 시트 크기로 자른다)
            let r1 = cv.r32()?;
            let r2 = cv.r32()?;
            let c1 = cv.r16()?;
            let c2 = cv.r16()?;
            if to8 {
                cv.w32(r1.min(MAX_ROW8 + 1));
                cv.w32(r2.min(MAX_ROW8 + 1));
                cv.w16(c1.min(256));
                cv.w16(c2.min(256));
            } else {
                cv.w32(r1);
                cv.w32(r2);
                cv.w16(c1);
                cv.w16(c2);
            }
            cv.rest();
        }
        0x007D => {
            // COLINFO: 열 256 부터는 버리고 끝 열은 255 까지
            let c1 = cv.r16()?;
            let c2 = cv.r16()?;
            if to8 && c1 as u32 > MAX_COL8 {
                return None;
            }
            cv.w16(c1);
            cv.w16(if to8 { c2.min(MAX_COL8 as u16) } else { c2 });
            cv.rest();
        }
        0x00E5 => {
            // MERGECELLS: n u16, 범위 × n
            let n = cv.r16()? as usize;
            let mut kept = Vec::new();
            for _ in 0..n {
                let mut one = Cv::new(cv.take(if to8 { 12 } else { 8 })?, to8);
                one.ref8()?;
                if !one.over {
                    kept.push(one.done());
                }
            }
            if kept.is_empty() {
                return None;
            }
            cv.w16(kept.len() as u16);
            for k in kept {
                cv.o.extend_from_slice(&k);
            }
        }
        0x001D => {
            // SELECTION: pnn u8, rwAct, colAct u16, irefAct u16, cref u16, RefU × cref
            cv.copy(1)?;
            cv.row()?;
            cv.copy(4)?;
            let n = cv.r16()?;
            cv.w16(n);
            for _ in 0..n {
                cv.ref_u()?;
            }
        }
        0x0041 => {
            // PANE: x u16, y (레거시 u32 ↔ u16), rwTop, colLeft u16, pnnAct (u16 ↔ u8 + 예약)
            cv.copy(2)?;
            cv.row()?;
            cv.row()?;
            cv.copy(4)?;
        }
        0x023E => {
            // WINDOW2: 레거시 grbit, rwTop u32, colLeft, icvHdr u32, 예약 u16, 확대율(보통·나누기 미리보기),
            // 미상 u32 ↔ BIFF8 grbit, rwTop u16, colLeft, icvHdr u16, 예약 u16, 확대율(나누기 미리보기·보통), 0 ×4
            cv.copy(2)?;
            cv.row()?;
            cv.copy(2)?;
            if to8 {
                let icv = cv.r32()?;
                cv.take(2)?;
                let normal = cv.r16()?;
                let slv = cv.r16()?;
                // 한셀의 .xls 저장은 icvHdr 0 을 0x16 으로 쓴다
                cv.w16(if icv == 0 { 0x16 } else { icv as u16 });
                cv.w16(0);
                cv.w16(slv);
                cv.w16(normal);
                cv.w32(0);
            } else {
                let icv = cv.r16()?;
                cv.take(2)?;
                let slv = cv.r16().unwrap_or(0);
                let normal = cv.r16().unwrap_or(0);
                cv.w32(if icv == 0x16 || icv == 0x40 {
                    0
                } else {
                    icv as u32
                });
                cv.w16(0);
                cv.w16(normal);
                cv.w16(slv);
                cv.w32(0x8000_0000);
            }
        }
        0x001C => {
            // NOTE: rw, col u16, grbit u16, idObj u16, 작성자
            cell_head(&mut cv)?;
            cv.copy(2)?;
            cv.str16()?;
            cv.rest();
        }
        0x001B => {
            // HORIZONTALPAGEBREAKS: 레거시 (rw u32, ff 3f 00 00) ↔ BIFF8 (rw u16, colStart 0, colEnd 255)
            let n = cv.r16()?;
            cv.w16(n);
            for _ in 0..n {
                if to8 {
                    let r = cv.r32()?;
                    cv.take(4)?;
                    cv.w16(r.min(MAX_ROW8) as u16);
                    cv.w16(0);
                    cv.w16(0xFF);
                } else {
                    let r = cv.r16()?;
                    cv.take(4)?;
                    cv.w32(r as u32);
                    cv.w16(0x3FFF);
                    cv.w16(0);
                }
            }
        }
        0x001A => {
            // VERTICALPAGEBREAKS: 레거시 (col u16, rwStart u16, rwEnd u32) ↔ BIFF8 (col, rwStart, rwEnd u16)
            let n = cv.r16()?;
            cv.w16(n);
            for _ in 0..n {
                cv.copy(4)?;
                if to8 {
                    cv.r32()?;
                    cv.w16(0xFFFF);
                } else {
                    cv.r16()?;
                    cv.w32(MAX_ROW_H);
                }
            }
        }
        0x01B8 => {
            // HLINK: 범위 Ref8U 뒤는 그대로
            cv.ref8()?;
            cv.rest();
        }
        // 문자열이 든 레코드
        0x005C | 0x01BA => {
            cv.str16()?;
            cv.rest();
        }
        0x041E => {
            cv.copy(2)?;
            cv.str16()?;
        }
        0x0085 => {
            // BOUNDSHEET: 위치 u32 (나중에 고친다), 상태 u8, 종류 u8, 이름 (cch u8)
            cv.copy(6)?;
            cv.str8()?;
        }
        0x0293 => {
            let ixfe = cv.r16()?;
            cv.w16(ixfe);
            if ixfe & 0x8000 == 0 {
                cv.str16()?;
            }
            cv.rest();
        }
        0x0031 => {
            // FONT: 14바이트, 이름 (cch u8). 한컴 확장(플래그 + 78바이트)은 뗀다
            cv.copy(14)?;
            cv.str8()?;
            if !to8 {
                cv.rest();
            }
        }
        0x0014 | 0x0015 => {
            // HEADER·FOOTER: 레거시는 글꼴 모양을 대문자 + 공백으로 쓴다 (`"돋움체, REGULAR"`) — BIFF8 은 XML 과
            // 같은 꼴 (`"돋움체,Regular"`)
            if !d.is_empty() {
                if to8 {
                    let cch = cv.r16()? as usize;
                    cv.r8()?;
                    let t = crate::text::utf16le(cv.take(cch * 2)?);
                    let t = super::reader::canonical_hf_fonts(&t);
                    let units: Vec<u16> = t.encode_utf16().collect();
                    cv.w16(units.len() as u16);
                    cv.w8(0x01);
                    for u in units {
                        cv.w16(u);
                    }
                } else {
                    cv.str16()?;
                }
            }
        }
        0x089C => {
            // HEADERFOOTER: FRT 12 + GUID 16 + 플래그 2 + 길이 u16 ×4, 글 4개 (cch u16 + 플래그 + 글자)
            cv.copy(12 + 16 + 2 + 8)?;
            for _ in 0..4 {
                if cv.left() == 0 {
                    break;
                }
                cv.str16()?;
            }
            cv.rest();
        }
        0x01AE => {
            // SUPBOOK: ctab u16, 표지(0x0401 이 문서, 0x3A01 추가 기능) 또는 경로 길이 + 경로 + 시트 이름 ×ctab
            let ctab = cv.r16()?;
            let mark = cv.r16()?;
            cv.w16(ctab);
            cv.w16(mark);
            if mark != 0x0401 && mark != 0x3A01 {
                cv.chars(mark as usize)?;
                for _ in 0..ctab {
                    cv.str16()?;
                }
            }
            cv.rest();
        }
        0x0023 => {
            // EXTERNNAME: 레거시 (옵션 u16, 예약 u32, cch u16, UTF-16) ↔ BIFF8 (옵션, 예약 u32, cch u8, 플래그, 글자,
            // 이름 정의 수식)
            cv.copy(6)?;
            if to8 {
                let cch = cv.r16()?;
                cv.w8(cch.min(255) as u8);
                cv.w8(0x01);
                cv.copy(cch.min(255) as usize * 2)?;
                // 이름 정의: 빈 수식 (cce 2, #REF! 오류 상수)
                cv.o.extend_from_slice(&[0x02, 0x00, 0x1C, 0x17]);
            } else {
                let cch = cv.r8()? as usize;
                let flag = cv.r8()?;
                cv.w16(cch as u16);
                if flag & 1 != 0 {
                    cv.copy(cch * 2)?;
                } else {
                    for &c in cv.take(cch)? {
                        cv.w16(c as u16);
                    }
                }
            }
        }
        0x0018 => {
            // NAME: grbit u16, chKey u8, cch u8, cce u16, ixals u16, itab u16, 길이 u8 ×4, 이름, 수식, 글 ×4
            cv.copy(3)?;
            let cch = cv.r8()?;
            cv.w8(cch);
            let cce = cv.r16()? as usize;
            let at = cv.o.len();
            cv.w16(0);
            cv.copy(4)?;
            let lens: Vec<u8> = cv.take(4)?.to_vec();
            cv.o.extend_from_slice(&lens);
            cv.chars(cch as usize)?;
            let t = cv.take(cce)?;
            let n = ptgs(t, to8, false)?;
            cv.o[at..at + 2].copy_from_slice(&(n.len() as u16).to_le_bytes());
            cv.o.extend_from_slice(&n);
            for l in lens {
                if l > 0 {
                    cv.chars(l as usize)?;
                }
            }
            cv.rest();
        }
        0x009E => {
            // AUTOFILTER: iEntry, grbit, DOPER ×2, 문자열 DOPER 의 글 (cch 는 DOPER 의 6번째 바이트)
            cv.copy(4)?;
            let d1 = cv.take(10)?.to_vec();
            let d2 = cv.take(10)?.to_vec();
            cv.o.extend_from_slice(&d1);
            cv.o.extend_from_slice(&d2);
            for dp in [d1, d2] {
                if dp[0] == 0x06 && cv.left() > 0 {
                    cv.chars(dp[6] as usize)?;
                }
            }
            cv.rest();
        }
        0x087F => {
            // ContinueFrt12: FrtRefHeader 12 + DOPER 10 (문자열 cch 는 DOPER 의 3번째 바이트) + 글
            cv.copy(12)?;
            let dp = cv.take(10)?.to_vec();
            cv.o.extend_from_slice(&dp);
            if dp[0] == 0x06 && cv.left() > 0 {
                cv.chars(dp[2] as usize)?;
            }
            cv.rest();
        }
        0x0862 => {
            // SHEETEXT: FRT 12 + cb + icvPlain = 20바이트. 한컴은 뒤에 COLORREF 를 붙이고, BIFF8 은 선택 부분을
            // 붙일 수 있다 — 둘 다 뗀다
            cv.copy(20.min(d.len()))?;
        }
        0x088D..=0x0890 => {
            // DXF·TABLESTYLES·TABLESTYLE·TABLESTYLEELEMENT: FRT 머리의 예약 칸은 0 (한셀은 쓰레기 값을 쓴다)
            if to8 && d.len() >= 12 {
                cv.copy(2)?;
                cv.take(10)?;
                cv.o.extend_from_slice(&[0; 10]);
            }
            cv.rest();
        }
        _ => cv.rest(),
    }
    if to8 && cv.over && drops_over(rt) {
        return None;
    }
    Some(cv.done())
}

// ---------------------------------------------------------------------------
// 레거시 → BIFF8 (.xls 쓰기)
// ---------------------------------------------------------------------------

/// BIFF8 에 쓰지 않는 한컴 레코드: 0xA030·0x0900 (의미 미상), 0xA001·0xA011·0xA020 (문단 모양·번호), 0x009F
/// (값 목록 필터), 0xB002 (행 반복 — 풀어 쓴다), 0xB005–0xB00D (표), 0xB00E (257번째 열부터의 열 정보), 0x7EF0·
/// 0x7EF8 (한컴 차트 — BIFF8 차트는 아직 쓰지 않는다)
fn hancom_only(rt: u16) -> bool {
    matches!(
        rt,
        0xA030 | 0x0900 | 0xA001 | 0xA011 | 0xA020 | 0x009F | 0xB002 | 0xB005
            ..=0xB00E | 0x7EF0 | 0x7EF8
    )
}

/// 피벗 레코드 (BIFF8 피벗은 아직 쓰지 않는다)
fn pivot_record(rt: u16) -> bool {
    matches!(
        rt,
        0x0051
            | 0x0052
            | 0x00B0..=0x00B6
            | 0x00C5
            | 0x00C6
            | 0x00CD
            | 0x00CE
            | 0x00D5
            | 0x00E3
            | 0x00F1
            | 0x00F2
            | 0x00FB
            | 0x0100
            | 0x0103
            | 0x0122
            | 0x01B5
            | 0x0802..=0x0805
            | 0x080B..=0x080D
            | 0x0810
            | 0x0864
    )
}

/// 조건부 서식 CF (0x01B1): ct, cp, cce1, cce2, DXFN, 수식 1·2. 레거시의 DXFN 은 표준 블록 뒤에 한컴 확장과
/// 차등 서식 번호(u32)를 붙인다 — BIFF8 은 표준 블록만 (flags2 의 높은 바이트는 0, 글꼴 이름 플래그 1)
fn cf_to8(d: &[u8]) -> Option<Vec<u8>> {
    let c1 = u16::from_le_bytes([*d.get(2)?, *d.get(3)?]) as usize;
    let c2 = u16::from_le_bytes([*d.get(4)?, *d.get(5)?]) as usize;
    let fstart = d.len().checked_sub(c1 + c2)?;
    let std = dxfn_std_len(d.get(6..)?)?;
    let mut dxfn = d.get(6..6 + std)?.to_vec();
    if dxfn.len() >= 6 {
        dxfn[5] = 0;
        let flags = u32::from_le_bytes([dxfn[0], dxfn[1], dxfn[2], dxfn[3]]);
        // 글꼴 블록이 표시 형식 블록 뒤에 있으면 이름 플래그 (cchFont u8 다음 바이트)
        if flags & 0x0400_0000 != 0 {
            let at = 6 + numfmt_block_len(&dxfn, flags)?;
            if let Some(b) = dxfn.get_mut(at + 1) {
                *b = 0x01;
            }
        }
    }
    let f1 = ptgs(d.get(fstart..fstart + c1)?, true, true)?;
    let f2 = ptgs(d.get(fstart + c1..)?, true, true)?;
    let mut o = Vec::with_capacity(d.len());
    o.extend_from_slice(&d[..2]);
    o.extend_from_slice(&(f1.len() as u16).to_le_bytes());
    o.extend_from_slice(&(f2.len() as u16).to_le_bytes());
    o.extend_from_slice(&dxfn);
    o.extend_from_slice(&f1);
    o.extend_from_slice(&f2);
    Some(o)
}

/// DXFN 의 표시 형식 블록 길이 (flags 0x02000000)
fn numfmt_block_len(dxfn: &[u8], flags: u32) -> Option<usize> {
    if flags & 0x0200_0000 == 0 {
        return Some(0);
    }
    let flags2 = u16::from_le_bytes([*dxfn.get(4)?, *dxfn.get(5)?]);
    if flags2 & 0x0001 != 0 {
        Some(u16::from_le_bytes([*dxfn.get(6)?, *dxfn.get(7)?]) as usize)
    } else {
        Some(2)
    }
}

/// DXFN 표준 부분의 길이 (플래그 6바이트 + 켠 블록)
fn dxfn_std_len(d: &[u8]) -> Option<usize> {
    let flags = u32::from_le_bytes([*d.first()?, *d.get(1)?, *d.get(2)?, *d.get(3)?]);
    let mut n = 6 + numfmt_block_len(d, flags)?;
    for (bit, len) in [
        (0x0400_0000, 118),
        (0x0800_0000, 8),
        (0x1000_0000, 8),
        (0x2000_0000, 4),
        (0x4000_0000, 2),
    ] {
        if flags & bit != 0 {
            n += len;
        }
    }
    Some(n)
}

/// CONDFMT (0x01B0)·CONDFMT12 (0x0879, FRT 머리 12): ccf, nID, 경계 범위, n, 범위 × n
fn condfmt(d: &[u8], to8: bool, frt: bool) -> Option<Vec<u8>> {
    let mut cv = Cv::new(d, to8);
    if frt {
        cv.copy(12)?;
    }
    cv.copy(4)?;
    cv.ref8()?;
    let n = cv.r16()?;
    cv.w16(n);
    for _ in 0..n {
        cv.ref8()?;
    }
    cv.rest();
    Some(cv.done())
}

/// DV (0x01BE): 플래그 u32, 글 4개 (BIFF8 은 빈 글을 NUL 하나로), 수식 2개 (cce u16 + 예약 u16 + 토큰 — 목록의
/// 글 상수는 BIFF8 에서 항목을 NUL 로 나누고 fStrLookup 0x80 을 켠다), 범위 (n, Ref8U × n)
fn dv(d: &[u8], to8: bool) -> Option<Vec<u8>> {
    let mut cv = Cv::new(d, to8);
    let mut flags = cv.r32()?;
    let flags_at = cv.o.len();
    cv.w32(flags);
    for _ in 0..4 {
        let cch = cv.r16()?;
        if to8 && cch == 0 {
            cv.r8()?;
            cv.o.extend_from_slice(&[1, 0, 1, 0, 0]);
        } else if !to8 && cch == 1 && cv.s.get(cv.p + 1..cv.p + 3) == Some(&[0, 0][..]) {
            // NUL 하나 = 빈 글 (플래그 1 + U+0000)
            cv.take(3)?;
            cv.o.extend_from_slice(&[0, 0, 0]);
        } else if !to8 && cch == 1 && cv.s.get(cv.p..cv.p + 2) == Some(&[0, 0][..]) {
            // 플래그 0 + 1바이트 NUL
            cv.take(2)?;
            cv.o.extend_from_slice(&[0, 0, 0]);
        } else {
            cv.w16(cch);
            cv.chars(cch as usize)?;
        }
    }
    let list = flags & 0x0F == 3;
    for k in 0..2 {
        let cce = cv.r16()? as usize;
        let res = cv.r16()?;
        let t = cv.take(cce)?;
        let mut n = ptgs(t, to8, true)?;
        // 목록의 글 상수: 레거시 쉼표 ↔ BIFF8 NUL
        if k == 0 && list && n.first().is_some_and(|&p| p == 0x17) {
            let (from, to): (u16, u16) = if to8 { (0x2C, 0) } else { (0, 0x2C) };
            let (cch, flag, at) = if to8 {
                (n[1] as usize, n[2], 3)
            } else {
                (u16::from_le_bytes([n[1], n[2]]) as usize, n[3], 4)
            };
            if flag & 1 != 0 {
                for i in 0..cch {
                    let p = at + i * 2;
                    if p + 1 < n.len() && u16::from_le_bytes([n[p], n[p + 1]]) == from {
                        n[p..p + 2].copy_from_slice(&to.to_le_bytes());
                    }
                }
            }
            if to8 {
                flags |= 0x80;
            } else {
                flags &= !0x80;
            }
        }
        cv.w16(n.len() as u16);
        cv.w16(res);
        cv.o.extend_from_slice(&n);
    }
    cv.o[flags_at..flags_at + 4].copy_from_slice(&flags.to_le_bytes());
    let n = cv.r16()?;
    cv.w16(n);
    for _ in 0..n {
        cv.ref8()?;
    }
    cv.rest();
    Some(cv.done())
}

/// TXO: 레거시 (u16, u16 1, 가로 u8, 세로 u8, 런 수 u16, 문단 수 u16, cch u16, 플래그, UTF-16, 런 (ich, ifnt) ×,
/// 문단 모양 u32 ×) ↔ BIFF8 (18바이트 + CONTINUE 글 + CONTINUE 런 (ich, ifnt, 0 ×4)…, (cch, 0, 0))
fn txo_to8(d: &[u8]) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let mut r = Cv::new(d, true);
    let lock = r.r16()? & 0x0200;
    r.r16()?;
    let h = r.r8()?;
    let v = r.r8()?;
    let nruns = r.r16()? as usize;
    r.r16()?;
    let cch = r.r16()? as usize;
    r.r8()?;
    let text = r.take(cch * 2)?.to_vec();
    let mut runs = Vec::new();
    for _ in 0..nruns {
        runs.push((r.r16()?, r.r16()?));
    }
    // 가로: 레거시 1 왼쪽, 2 가운데, 3 오른쪽, 5 양쪽, 7 균등 → BIFF8 1, 2, 3, 4, 7
    let h8: u16 = match h {
        2 => 2,
        3 => 3,
        5 => 4,
        7 => 7,
        _ => 1,
    };
    // 세로: 레거시 0 위, 1 가운데, 2 아래, 3 양쪽, 4 균등 → BIFF8 1, 2, 3, 4, 7
    let v8: u16 = match v {
        1 => 2,
        2 => 3,
        3 => 4,
        4 => 7,
        _ => 1,
    };
    let grbit = (h8 << 1) | (v8 << 4) | lock;
    let mut head = Vec::with_capacity(18);
    head.extend_from_slice(&grbit.to_le_bytes());
    head.extend_from_slice(&[0; 8]);
    head.extend_from_slice(&(cch as u16).to_le_bytes());
    let mut conts = Vec::new();
    if cch == 0 {
        head.extend_from_slice(&[0; 6]);
        return Some((head, conts));
    }
    if runs.is_empty() || runs[0].0 != 0 {
        runs.insert(0, (0, 0));
    }
    let mut rb = Vec::new();
    for (ich, ifnt) in &runs {
        rb.extend_from_slice(&ich.to_le_bytes());
        rb.extend_from_slice(&ifnt.to_le_bytes());
        rb.extend_from_slice(&[0; 4]);
    }
    rb.extend_from_slice(&(cch as u16).to_le_bytes());
    rb.extend_from_slice(&[0; 6]);
    head.extend_from_slice(&(rb.len() as u16).to_le_bytes());
    head.extend_from_slice(&[0; 4]);
    let mut t = vec![0x01];
    t.extend_from_slice(&text);
    conts.push(t);
    conts.push(rb);
    Some((head, conts))
}

/// OfficeArt 레코드 머리 (verInst u16, fbt u16, len u32)
fn art_head(b: &[u8], p: usize) -> Option<(u16, u16, usize)> {
    let vi = u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]);
    let fbt = u16::from_le_bytes([*b.get(p + 2)?, *b.get(p + 3)?]);
    let len = u32::from_le_bytes([
        *b.get(p + 4)?,
        *b.get(p + 5)?,
        *b.get(p + 6)?,
        *b.get(p + 7)?,
    ]);
    Some((vi, fbt, len as usize))
}

/// OfficeArt 레코드들(`b[p..end]`)을 옮겨 쓰며 셀 앵커(0xF010)를 바꾸고 컨테이너 길이를 다시 센다.
/// `map` 에 레코드 시작 위치 대응(원본 → 결과)을 모은다
fn art_rewrite(
    b: &[u8],
    mut p: usize,
    end: usize,
    to8: bool,
    out: &mut Vec<u8>,
    map: &mut Vec<(usize, usize)>,
) {
    while p + 8 <= end {
        let Some((vi, fbt, len)) = art_head(b, p) else {
            break;
        };
        map.push((p, out.len()));
        let body = p + 8;
        let stop = (body + len).min(b.len());
        let at = out.len();
        out.extend_from_slice(&b[p..body]);
        if vi & 0x000F == 0x000F {
            art_rewrite(b, body, stop, to8, out, map);
        } else if fbt == 0xF010 && ((to8 && len == 22) || (!to8 && len == 18)) {
            out.extend_from_slice(&anchor(&b[body..stop], to8));
        } else {
            out.extend_from_slice(&b[body..stop]);
        }
        let n = (out.len() - at - 8) as u32;
        out[at + 4..at + 8].copy_from_slice(&n.to_le_bytes());
        p = stop;
    }
    if p < end {
        map.push((p, out.len()));
        out.extend_from_slice(&b[p..end]);
    }
}

/// 셀 앵커: 레거시 22바이트 (flag, colL, dxL, rwT u32, dyT, colR, dxR, rwB u32, dyB — dx 1/32768, dy 1/8192) ↔
/// BIFF8 18바이트 (행 u16, dx 1/1024, dy 1/256)
fn anchor(a: &[u8], to8: bool) -> Vec<u8> {
    let u16_at = |i: usize| u16::from_le_bytes([a[i], a[i + 1]]) as u32;
    let u32_at = |i: usize| u32::from_le_bytes([a[i], a[i + 1], a[i + 2], a[i + 3]]);
    let mut o = Vec::with_capacity(22);
    if to8 {
        let (flag, c1, dx1, r1, dy1, c2, dx2, r2, dy2) = (
            u16_at(0),
            u16_at(2),
            u16_at(4),
            u32_at(6),
            u16_at(10),
            u16_at(12),
            u16_at(14),
            u32_at(16),
            u16_at(20),
        );
        for v in [
            flag,
            c1.min(MAX_COL8),
            dx1 / 32,
            r1.min(MAX_ROW8),
            dy1 / 32,
            c2.min(MAX_COL8),
            dx2 / 32,
            r2.min(MAX_ROW8),
            dy2 / 32,
        ] {
            o.extend_from_slice(&(v as u16).to_le_bytes());
        }
    } else {
        let v: Vec<u32> = (0..9).map(|k| u16_at(k * 2)).collect();
        o.extend_from_slice(&(v[0] as u16).to_le_bytes());
        o.extend_from_slice(&(v[1] as u16).to_le_bytes());
        o.extend_from_slice(&((v[2] * 32).min(0xFFFF) as u16).to_le_bytes());
        o.extend_from_slice(&v[3].to_le_bytes());
        o.extend_from_slice(&((v[4] * 32).min(0xFFFF) as u16).to_le_bytes());
        o.extend_from_slice(&(v[5] as u16).to_le_bytes());
        o.extend_from_slice(&((v[6] * 32).min(0xFFFF) as u16).to_le_bytes());
        o.extend_from_slice(&v[7].to_le_bytes());
        o.extend_from_slice(&((v[8] * 32).min(0xFFFF) as u16).to_le_bytes());
    }
    o
}

/// 시트의 MSODRAWING 들을 이어 앵커를 바꾸고 같은 경계로 다시 나눈다
fn drawings(parts: &[Vec<u8>], to8: bool) -> Vec<Vec<u8>> {
    let all: Vec<u8> = parts.concat();
    let mut out = Vec::with_capacity(all.len());
    let mut map = Vec::new();
    art_rewrite(&all, 0, all.len(), to8, &mut out, &mut map);
    map.push((all.len(), out.len()));
    let at = |x: usize| -> usize {
        match map.binary_search_by_key(&x, |&(s, _)| s) {
            Ok(i) => map[i].1,
            Err(i) => {
                let (s, d) = map[i.saturating_sub(1)];
                (d + (x - s)).min(out.len())
            }
        }
    };
    let mut res = Vec::with_capacity(parts.len());
    let mut start_src = 0;
    let mut start_dst = 0;
    for p in parts {
        let end_src = start_src + p.len();
        let end_dst = at(end_src).max(start_dst);
        res.push(out[start_dst..end_dst].to_vec());
        start_src = end_src;
        start_dst = end_dst;
    }
    res
}

/// [MS-OSHARED] 2.4.3.2 CRC (XFCRC 의 checksum): 다항식 0xAF
fn oshared_crc(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut v = (i as u32) << 24;
        for _ in 0..8 {
            v = if v & 0x8000_0000 != 0 {
                (v << 1) ^ 0xAF
            } else {
                v << 1
            };
        }
        *t = v;
    }
    let mut c = 0u32;
    for &x in data {
        let idx = ((c >> 24) ^ x as u32) & 0xFF;
        c = (c << 8) ^ table[idx as usize];
    }
    c
}

/// 레거시 SST 문자열 하나 (cch u16, 플래그 u8 (0x01 = 서식 런), UTF-16, [cRun u16, (ich, ifnt) × cRun])
struct SstStr {
    chars: Vec<u8>,
    runs: Vec<u8>,
}

fn sst_strings(d: &[u8]) -> Option<(u32, Vec<SstStr>)> {
    let mut r = Cv::new(d, true);
    let total = r.r32()?;
    let unique = r.r32()? as usize;
    let mut v = Vec::with_capacity(unique);
    for _ in 0..unique {
        let cch = r.r16()? as usize;
        let flags = r.r8()?;
        let chars = r.take(cch * 2)?.to_vec();
        let runs = if flags & 0x01 != 0 {
            let n = r.r16()? as usize;
            r.take(n * 4)?.to_vec()
        } else {
            Vec::new()
        };
        v.push(SstStr { chars, runs });
    }
    Some((total, v))
}

/// BIFF8 SST 조각들 (첫 조각이 SST, 나머지는 CONTINUE) 과 버킷마다 (조각 번호, 조각 안 위치)
fn sst_build(total: u32, strs: &[SstStr], dsst: usize) -> (Vec<Vec<u8>>, Vec<(usize, usize)>) {
    let mut recs: Vec<Vec<u8>> = vec![Vec::new()];
    recs[0].extend_from_slice(&total.to_le_bytes());
    recs[0].extend_from_slice(&(strs.len() as u32).to_le_bytes());
    let mut buckets = Vec::new();
    for (i, s) in strs.iter().enumerate() {
        let cch = s.chars.len() / 2;
        let rich = !s.runs.is_empty();
        let head = 3 + if rich { 2 } else { 0 };
        // 머리와 첫 글자 하나가 들어가야 한다
        let need = head + if cch > 0 { 2 } else { 0 };
        if recs.last().unwrap().len() + need > MAX_DATA {
            recs.push(Vec::new());
        }
        if i % dsst == 0 {
            buckets.push((recs.len() - 1, recs.last().unwrap().len()));
        }
        {
            let cur = recs.last_mut().unwrap();
            cur.extend_from_slice(&(cch as u16).to_le_bytes());
            cur.push(0x01 | if rich { 0x08 } else { 0 });
            if rich {
                cur.extend_from_slice(&((s.runs.len() / 4) as u16).to_le_bytes());
            }
        }
        // 글자: 레코드가 차면 CONTINUE 를 새로 (첫 바이트에 플래그 다시)
        let mut c = &s.chars[..];
        while !c.is_empty() {
            let room = (MAX_DATA - recs.last().unwrap().len()) / 2 * 2;
            if room == 0 {
                recs.push(vec![0x01]);
                continue;
            }
            let n = room.min(c.len());
            recs.last_mut().unwrap().extend_from_slice(&c[..n]);
            c = &c[n..];
        }
        // 서식 런: 4바이트씩, 플래그 없이 이어진다
        let mut r = &s.runs[..];
        while !r.is_empty() {
            let room = (MAX_DATA - recs.last().unwrap().len()) / 4 * 4;
            if room == 0 {
                recs.push(Vec::new());
                continue;
            }
            let n = room.min(r.len());
            recs.last_mut().unwrap().extend_from_slice(&r[..n]);
            r = &r[n..];
        }
    }
    (recs, buckets)
}

/// 바꾼 레코드: 종류, 데이터, 뒤따를 CONTINUE 조각들
type OutRec = (u16, Vec<u8>, Vec<Vec<u8>>);

/// 시트의 셀 묶음: 셀 레코드 하나와 뒤따르는 STRING·SHRFMLA·ARRAY·TABLE
struct Unit {
    row: u32,
    recs: Vec<(u16, Vec<u8>)>,
}

fn is_cell(rt: u16) -> bool {
    matches!(
        rt,
        0x00FD | 0x0203 | 0x027E | 0x0201 | 0x0205 | 0x0204 | 0x00D6 | 0x00BD | 0x00BE | 0x0006
    )
}

/// 레거시 시트 부분의 레코드들 → BIFF8 레코드들
fn sheet_to8(recs: &[Rec]) -> Vec<OutRec> {
    let mut out: Vec<OutRec> = Vec::new();
    // 셀 표: ROW 와 셀 묶음을 모았다가 32 행 묶음마다 ROW, 셀 차례로 쓴다
    let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut table_at: Option<usize> = None;
    // MSODRAWING 은 시트 끝에서 앵커를 한꺼번에 바꾼다 (자리만 남긴다)
    let mut art_slots: Vec<usize> = Vec::new();
    let mut art_parts: Vec<Vec<u8>> = Vec::new();
    let row_of = |d: &[u8]| -> u32 { u32::from_le_bytes([d[0], d[1], d[2], d[3]]) };
    for rec in recs {
        let rt = rec.rt;
        let d = &rec.data;
        if hancom_only(rt) && rt != 0xB002 {
            continue;
        }
        if pivot_record(rt) {
            continue;
        }
        match rt {
            0x0809 => {
                let mut b = d.clone();
                b[0..2].copy_from_slice(&BIFF8_VERSION.to_le_bytes());
                b[4..8].copy_from_slice(&[0xBB, 0x0D, 0xCC, 0x07]);
                out.push((rt, b, Vec::new()));
            }
            0x0208 if d.len() >= 4 => {
                table_at.get_or_insert(out.len());
                rows.push((row_of(d), d.clone()));
            }
            0xB002 if d.len() >= 8 => {
                // 행 반복: first 행의 ROW 와 빈 칸(BLANK·MULBLANK)을 first+1..=last 행에 되풀이한다
                let first = row_of(d);
                let last = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
                let row_rec = rows
                    .iter()
                    .rev()
                    .find(|r| r.0 == first)
                    .map(|r| r.1.clone());
                let cells: Vec<Vec<(u16, Vec<u8>)>> = units
                    .iter()
                    .filter(|u| u.row == first)
                    .map(|u| u.recs.clone())
                    .collect();
                for row in first + 1..=last.min(MAX_ROW8) {
                    let put = |b: &mut Vec<u8>| b[0..4].copy_from_slice(&row.to_le_bytes());
                    if let Some(mut r) = row_rec.clone() {
                        put(&mut r);
                        rows.push((row, r));
                    }
                    for u in &cells {
                        let recs = u
                            .iter()
                            .map(|(t, b)| {
                                let mut b = b.clone();
                                put(&mut b);
                                (*t, b)
                            })
                            .collect();
                        units.push(Unit { row, recs });
                    }
                }
            }
            t if is_cell(t) && d.len() >= 4 => {
                table_at.get_or_insert(out.len());
                units.push(Unit {
                    row: row_of(d),
                    recs: vec![(t, d.clone())],
                });
            }
            0x0207 | 0x04BC | 0x0221 | 0x0236 if !units.is_empty() => {
                units.last_mut().unwrap().recs.push((rt, d.clone()));
            }
            0x00EC => {
                art_slots.push(out.len());
                art_parts.push(d.clone());
                out.push((rt, Vec::new(), Vec::new()));
            }
            0x01B6 => {
                if let Some((h, c)) = txo_to8(d) {
                    out.push((rt, h, c));
                }
            }
            0x01B0 => {
                if let Some(b) = condfmt(d, true, false) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x0879 => {
                if let Some(b) = condfmt(d, true, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x01B1 => {
                if let Some(b) = cf_to8(d) {
                    out.push((rt, b, Vec::new()));
                }
            }
            // CF12·CFEX 는 레거시 DXFN 확장을 아직 풀지 못한다 — 쓰지 않는다
            0x087A | 0x087B => {}
            0x01BE => {
                if let Some(b) = dv(d, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x023E => {
                if let Some(b) = conv(rt, d, true) {
                    out.push((rt, b, Vec::new()));
                }
                // PLV: 한셀의 .xls 저장은 WINDOW2 뒤에 페이지 레이아웃 보기를 쓴다 (확대율 100, 눈금자 표시)
                let mut plv = vec![0x8B, 0x08];
                plv.extend_from_slice(&[0; 10]);
                plv.extend_from_slice(&[0x64, 0x00, 0x02, 0x00]);
                out.push((0x088B, plv, Vec::new()));
            }
            _ => {
                if let Some(b) = conv(rt, d, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
        }
    }
    // 그리기 앵커
    if !art_parts.is_empty() {
        for (slot, part) in art_slots.iter().zip(drawings(&art_parts, true)) {
            out[*slot].1 = part;
        }
    }
    // 셀 표
    let mut table: Vec<OutRec> = Vec::new();
    rows.sort_by_key(|r| r.0);
    rows.dedup_by_key(|r| r.0);
    // 묶음: 남은 첫 행부터 32 행 (한셀의 .xls 저장 — 묶음의 ROW 들 다음에 그 행들의 셀)
    units.sort_by_key(|u| u.row);
    let mut ui = 0;
    let mut ri = 0;
    while ri < rows.len() || ui < units.len() {
        let start = match (rows.get(ri), units.get(ui)) {
            (Some(r), Some(u)) => r.0.min(u.row),
            (Some(r), None) => r.0,
            (None, Some(u)) => u.row,
            (None, None) => break,
        };
        let next_first = start.saturating_add(32);
        let end = ri + rows[ri..].iter().take_while(|r| r.0 < next_first).count();
        for r in &rows[ri..end] {
            if let Some(b) = conv(0x0208, &r.1, true) {
                table.push((0x0208, b, Vec::new()));
            }
        }
        while ui < units.len() && units[ui].row < next_first {
            let u = &units[ui];
            // 셀이 시트 밖이면 묶음 전체를 버린다
            if let Some(first) = conv(u.recs[0].0, &u.recs[0].1, true) {
                let rt0 = if u.recs[0].0 == 0x00D6 {
                    0x0204
                } else {
                    u.recs[0].0
                };
                table.push((rt0, first, Vec::new()));
                for (t, b) in &u.recs[1..] {
                    if let Some(x) = conv(*t, b, true) {
                        table.push((*t, x, Vec::new()));
                    }
                }
            }
            ui += 1;
        }
        ri = end;
    }
    if let Some(at) = table_at {
        out.splice(at..at, table);
    }
    out
}

/// 레거시 `/Workbook` 스트림 → BIFF8 `/Workbook` 스트림 (`.xls`)
pub fn to_biff8(stream: &[u8]) -> Result<Vec<u8>> {
    let recs: Vec<Rec> = biff::records(stream)
        .into_iter()
        .map(|r| Rec::new(r.rtype, r.data.to_vec()))
        .collect();
    if recs.first().map(|r| r.rt) != Some(0x0809) {
        return Err(bad("BOF 레코드가 없습니다"));
    }
    // 부분 스트림으로 나눈다 (BOF … EOF)
    let mut subs: Vec<Vec<Rec>> = Vec::new();
    for r in recs {
        if r.rt == 0x0809 || subs.is_empty() {
            subs.push(Vec::new());
        }
        subs.last_mut().unwrap().push(r);
    }
    let globals = subs.remove(0);
    let sheets: Vec<Vec<OutRec>> =
        subs.iter().map(|s| sheet_to8(s)).collect();

    // 전역: 레코드를 바꾸고, XF 뒤에 XFCRC, SST 는 CONTINUE 로 나누고 EXTSST
    let mut g: Vec<OutRec> = Vec::new();
    let mut xf_data: Vec<u8> = Vec::new();
    let mut xf_count = 0u16;
    let mut sst: Option<(u32, Vec<SstStr>)> = None;
    // 문단 모양 (0xA001 의 왼쪽·오른쪽 여백) 과 XF 의 문단 모양 번호 → 0xA001 번호 (0xA011)
    let mut parashapes: Vec<(i32, i32)> = Vec::new();
    let mut para_links: Vec<u32> = Vec::new();
    let i32_at = |d: &[u8], i: usize| -> Option<i32> {
        Some(i32::from_le_bytes(d.get(i..i + 4)?.try_into().ok()?))
    };
    for (i, r) in globals.iter().enumerate() {
        let rt = r.rt;
        let d = &r.data;
        match rt {
            0xA001 => parashapes.push((i32_at(d, 0).unwrap_or(0), i32_at(d, 4).unwrap_or(0))),
            0xA011 => para_links.push(i32_at(d, 4).unwrap_or(0) as u32),
            _ => {}
        }
        if hancom_only(rt) || pivot_record(rt) {
            continue;
        }
        match rt {
            0x0809 => {
                let mut b = d.clone();
                b[0..2].copy_from_slice(&BIFF8_VERSION.to_le_bytes());
                b[4..12].copy_from_slice(&[0xBB, 0x0D, 0xCC, 0x07, 0x00, 0x80, 0x01, 0x00]);
                if b.len() >= 16 {
                    b[12..16].copy_from_slice(&[0x06, 0x06, 0x00, 0x00]);
                }
                g.push((rt, b, Vec::new()));
            }
            0x00E0 => {
                let mut b = d[..d.len().min(20)].to_vec();
                // 들여쓰기: 레거시는 cIndent 대신 문단 모양의 여백 ÷ 1134 (오른쪽 맞춤이면 오른쪽 여백) — 한셀의
                // .xls 저장은 그 단계를 cIndent 에 쓴다
                if b.len() == 20
                    && d.len() >= 26
                    && u16::from_le_bytes([d[20], d[21]]) >= 3
                    && b[8] & 0x0F == 0
                {
                    let para = i32_at(d, 22).unwrap_or(-1);
                    let shape = usize::try_from(para)
                        .ok()
                        .and_then(|k| para_links.get(k))
                        .and_then(|&k| parashapes.get(k as usize));
                    if let Some(&(left, right)) = shape {
                        let margin = if b[6] & 0x07 == 3 { right } else { left };
                        let level = (margin / 1134).clamp(0, 15) as u8;
                        b[8] |= level;
                    }
                }
                xf_data.extend_from_slice(&b);
                xf_count += 1;
                g.push((rt, b, Vec::new()));
                if globals.get(i + 1).is_none_or(|n| n.rt != 0x00E0) {
                    // XFCRC: FRT 12 + 예약 u16 + cxfs u16 + crc u32
                    let mut c = vec![0x7C, 0x08];
                    c.extend_from_slice(&[0; 12]);
                    c.extend_from_slice(&xf_count.to_le_bytes());
                    c.extend_from_slice(&oshared_crc(&xf_data).to_le_bytes());
                    g.push((0x087C, c, Vec::new()));
                }
            }
            0x00FC => {
                sst = sst_strings(d);
                g.push((rt, Vec::new(), Vec::new()));
            }
            0x0894 => {
                // NAMECMT: FRT 12, cchName u16, cchComment u16, 이름, 설명 (플래그 + 글자)
                let mut cv = Cv::new(d, true);
                let ok = (|| {
                    cv.copy(12)?;
                    let a = cv.r16()?;
                    let b = cv.r16()?;
                    cv.w16(a);
                    cv.w16(b);
                    cv.chars(a as usize)?;
                    cv.chars(b as usize)?;
                    Some(())
                })();
                if ok.is_some() {
                    g.push((rt, cv.done(), Vec::new()));
                }
            }
            _ => {
                if let Some(b) = conv(rt, d, true) {
                    g.push((rt, b, Vec::new()));
                }
            }
        }
    }

    // 한셀의 .xls 차례: XF, XFCRC, XFEXT, DXF, STYLE … (레거시는 XF, XFEXT, STYLE, DXF)
    if let Some(crc_at) = g.iter().position(|r| r.0 == 0x087C) {
        let moved: Vec<_> = g
            .iter()
            .filter(|r| r.0 == 0x087D || r.0 == 0x088D)
            .cloned()
            .collect();
        let mut rest: Vec<_> = g
            .into_iter()
            .filter(|r| r.0 != 0x087D && r.0 != 0x088D)
            .collect();
        let at = crc_at.min(rest.len().saturating_sub(1)) + 1;
        let mut xfext: Vec<_> = moved.iter().filter(|r| r.0 == 0x087D).cloned().collect();
        xfext.extend(moved.into_iter().filter(|r| r.0 == 0x088D));
        rest.splice(at..at, xfext);
        g = rest;
    }
    // 배치: 전역의 길이를 알아야 시트 위치(BOUNDSHEET)를 쓸 수 있다
    let sst_parts = sst.as_ref().map(|(total, strs)| {
        let dsst = (strs.len() / 128 + 1).max(8);
        let (parts, buckets) = sst_build(*total, strs, dsst);
        (parts, buckets, dsst)
    });
    let rec_len = |d: &[u8], conts: &[Vec<u8>]| -> usize {
        let mut n = 0;
        for c in std::iter::once(d).chain(conts.iter().map(|c| c.as_slice())) {
            n += 4 + c.len();
            if c.len() > MAX_DATA {
                n += (c.len() - 1) / MAX_DATA * 4;
            }
        }
        n
    };
    let mut g_len = 0usize;
    for (rt, d, conts) in &g {
        g_len += match rt {
            0x00FC => {
                let (parts, buckets, _) = sst_parts.as_ref().unwrap();
                parts.iter().map(|p| 4 + p.len()).sum::<usize>() + 4 + 2 + buckets.len() * 8
            }
            _ => rec_len(d, conts),
        };
    }
    let mut sheet_pos = Vec::new();
    let mut pos = g_len;
    let mut sheet_bytes = Vec::new();
    for s in &sheets {
        sheet_pos.push(pos);
        let mut b = Vec::new();
        for (rt, d, conts) in s {
            put8(&mut b, *rt, d);
            for c in conts {
                put8(&mut b, 0x003C, c);
            }
        }
        pos += b.len();
        sheet_bytes.push(b);
    }
    let mut out = Vec::with_capacity(pos);
    let mut si = 0;
    for (rt, d, conts) in &g {
        match rt {
            0x0085 => {
                let mut b = d.clone();
                let p = sheet_pos.get(si).copied().unwrap_or(0) as u32;
                b[0..4].copy_from_slice(&p.to_le_bytes());
                si += 1;
                put8(&mut out, *rt, &b);
            }
            0x00FC => {
                let (parts, buckets, dsst) = sst_parts.as_ref().unwrap();
                let mut starts = Vec::new();
                for (k, p) in parts.iter().enumerate() {
                    starts.push(out.len());
                    put8(&mut out, if k == 0 { 0x00FC } else { 0x003C }, p);
                }
                // EXTSST: dsst u16, (ib u32 = 문자열의 스트림 위치, cbOffset u16 = 레코드 머리부터, 예약 u16) × 버킷
                let mut e = Vec::new();
                e.extend_from_slice(&(*dsst as u16).to_le_bytes());
                for &(k, off) in buckets.iter().take(1) {
                    e.extend_from_slice(&((starts[k] + 4 + off) as u32).to_le_bytes());
                    e.extend_from_slice(&((off + 4) as u16).to_le_bytes());
                    e.extend_from_slice(&[0, 0]);
                }
                put8(&mut out, 0x00FF, &e);
            }
            _ => {
                put8(&mut out, *rt, d);
                for c in conts {
                    put8(&mut out, 0x003C, c);
                }
            }
        }
    }
    for b in sheet_bytes {
        out.extend_from_slice(&b);
    }
    Ok(out)
}

/// BIFF8 `/Workbook` 스트림 → 레거시 `/Workbook` 스트림 (`.xls` 읽기)
pub fn to_hancom(stream: &[u8]) -> Result<Vec<u8>> {
    let _ = (split_biff8(stream), put_h as fn(&mut Vec<u8>, u16, &[u8]));
    Err(bad("아직 읽지 못한다"))
}
