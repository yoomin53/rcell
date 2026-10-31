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

use std::collections::HashMap;

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
    /// 수식의 참조가 BIFF8 시트 밖이라 #REF! 가 되었다
    ref_err: bool,
}

impl<'a> Cv<'a> {
    fn new(s: &'a [u8], to8: bool) -> Cv<'a> {
        Cv {
            s,
            p: 0,
            o: Vec::with_capacity(s.len()),
            to8,
            over: false,
            ref_err: false,
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
        let (n, err) = ptgs_ex(t, self.to8, rel)?;
        self.ref_err |= err;
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
    ptgs_ex(src, to8, rel).map(|(v, _)| v)
}

/// [`ptgs`] 와 같고, 참조가 BIFF8 시트 밖이라 오류 토큰으로 바꿨는지 함께 돌려준다
fn ptgs_ex(src: &[u8], to8: bool, rel: bool) -> Option<(Vec<u8>, bool)> {
    let h_in = to8; // 원본이 레거시인가
    let mut invalid = false;
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
                    invalid = true;
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
                    invalid = true;
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
                    invalid = true;
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
                    invalid = true;
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
    Some((out, invalid))
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
            // 시트 밖 참조가 있으면 값도 #REF! (한셀의 .xls 저장)
            cell_head(&mut cv)?;
            let at = cv.o.len();
            cv.copy(14)?;
            formula_tail(&mut cv, false)?;
            if cv.ref_err {
                // 한셀의 .xls 저장: 값은 #REF!, 수식은 #REF! 상수 하나 (PtgErr)
                cv.o[at..at + 8].copy_from_slice(&[0x02, 0, 0x17, 0, 0, 0, 0xFF, 0xFF]);
                cv.o.truncate(at + 14);
                cv.o.extend_from_slice(&[0x02, 0x00, 0x1C, 0x17]);
            }
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
            // SHEETEXT: FRT 12 + cb + icvPlain = 20바이트. 한컴은 뒤에 탭 색 COLORREF 를 붙이고 (넥셀 .nxl 도),
            // BIFF8 은 선택 부분(cb 0x28: 비트 u32 + CFColor)을 붙일 수 있다. .xls 로는 둘 다 뗀다 (한셀과 같다),
            // 레거시로는 COLORREF 를 두고 BIFF8 의 RGB CFColor (종류 2) 는 COLORREF 로
            cv.copy(20.min(d.len()))?;
            if !to8 {
                let cb = d
                    .get(12..16)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                if cb == Some(0x14) && d.len() == 24 {
                    cv.copy(4)?;
                } else if cb == Some(0x28) && d.len() >= 32 && d[24..28] == [2, 0, 0, 0] {
                    if let Some(c) = cv.o.get_mut(12..16) {
                        c.copy_from_slice(&0x14u32.to_le_bytes());
                    }
                    cv.o.extend_from_slice(&[d[28], d[29], d[30], 0]);
                }
            }
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
    let dxfn = dxfn_std_to8(d.get(6..)?)?;
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

/// 레거시 차등 서식 (0x088D) 의 글꼴 색 속성 (종류 0x05) 값의 끝 4바이트 (R, G, B, 0 — 자동은 `FF FF FF DF`)
fn dxf_font_color(rec: &[u8]) -> Option<[u8; 4]> {
    let n = u16::from_le_bytes([*rec.get(16)?, *rec.get(17)?]) as usize;
    let mut p = 18;
    for _ in 0..n {
        let ty = u16::from_le_bytes([*rec.get(p)?, *rec.get(p + 1)?]);
        let cb = u16::from_le_bytes([*rec.get(p + 2)?, *rec.get(p + 3)?]) as usize;
        if cb < 4 {
            return None;
        }
        if ty == 0x05 {
            let v = rec.get(p + 4..p + cb)?;
            return v.get(4..8)?.try_into().ok();
        }
        p += cb;
    }
    None
}

/// 넥셀 조건부 서식 (CF 0x01B1): 머리 6바이트 + 플래그 6바이트(0) + 차등 서식에 있는 블록만 (글꼴 118, 테두리 8,
/// 무늬 4 — 표시 형식·맞춤·보호는 담지 않는다) + 수식. 한셀 -2003 저장을 BIFF8 쪽과 바이트 대조해 얻은 규칙
/// (docs/format/other-formats.md 6절). `d` 는 레거시 CF, `dxfs` 는 레거시 차등 서식 표
fn cf_to_nexcel(d: &[u8], dxfs: &[Vec<u8>]) -> Option<Vec<u8>> {
    let b8 = cf_to8(d)?;
    // 레거시 CF 의 차등 서식 번호: 수식 앞 u32 (테두리 블록이 있으면 안쪽 선 확장 2바이트가 그 뒤)
    let c1 = u16::from_le_bytes([*d.get(2)?, *d.get(3)?]) as usize;
    let c2 = u16::from_le_bytes([*d.get(4)?, *d.get(5)?]) as usize;
    let lflags = u32::from_le_bytes([*d.get(6)?, *d.get(7)?, *d.get(8)?, *d.get(9)?]);
    let fstart = d.len().checked_sub(c1 + c2)?;
    let id_at = fstart.checked_sub(4 + if lflags & 0x1000_0000 != 0 { 2 } else { 0 })?;
    let id = u32::from_le_bytes(d.get(id_at..id_at + 4)?.try_into().ok()?) as usize;
    let color = dxfs.get(id).and_then(|r| dxf_font_color(r));
    // BIFF8 CF 의 DXFN 블록
    let e1 = u16::from_le_bytes([b8[2], b8[3]]) as usize;
    let e2 = u16::from_le_bytes([b8[4], b8[5]]) as usize;
    let f8 = b8.len().checked_sub(e1 + e2)?;
    let dxfn = b8.get(6..f8)?;
    let flags = u32::from_le_bytes([*dxfn.first()?, *dxfn.get(1)?, *dxfn.get(2)?, *dxfn.get(3)?]);
    let mut at = 6 + numfmt_block_len(dxfn, flags)?;
    let mut out = b8[..6].to_vec();
    // 블록이 하나도 없으면 플래그 6바이트도 없다
    if flags & 0x3400_0000 != 0 {
        out.extend_from_slice(&[0; 6]);
    }
    if flags & 0x0400_0000 != 0 {
        let f = dxfn.get(at..at + 118)?;
        out.extend_from_slice(&nexcel_cf_font(f, color));
        at += 118;
    }
    if flags & 0x0800_0000 != 0 {
        at += 8;
    }
    if flags & 0x1000_0000 != 0 {
        out.extend_from_slice(&nexcel_cf_border(dxfn.get(at..at + 8)?));
        at += 8;
    }
    if flags & 0x2000_0000 != 0 {
        let mut p = dxfn.get(at..at + 4)?.to_vec();
        // 무늬 없이 배경색만 있는 채우기는 첫 바이트에 0x13 이 선다 (한셀 -2003 저장 대조)
        if p[1] >> 2 == 0 {
            p[0] = 0x13;
        }
        out.extend_from_slice(&p);
    }
    out.extend_from_slice(&b8[f8..]);
    Some(out)
}

/// 넥셀 CF 의 글꼴 블록 118바이트: 이름·크기는 담지 않고, 굵기·기울임·밑줄·취소선·색만 (BIFF8 의 "바꾸지 않음" 표시는 0)
fn nexcel_cf_font(f: &[u8], color: Option<[u8; 4]>) -> [u8; 118] {
    let mut a = [0u8; 118];
    a[71] = 0x08;
    a[72..74].copy_from_slice(&f[72..74]);
    a[68] = f[68] & 0x02;
    a[88] = if f[68] & 0x80 != 0 { 0x02 } else { 0x82 };
    let uls = f[76];
    if uls != 0 {
        a[74] |= 1;
        a[76] = uls;
        a[77] = u8::from(matches!(uls, 1 | 2));
        a[78] = match uls {
            1 => 1,
            2 => 8,
            _ => 0,
        };
    }
    a[96] = u8::from(matches!(uls, 0 | 2));
    let icv = u32::from_le_bytes([f[80], f[81], f[82], f[83]]);
    if icv == u32::MAX {
        a[80] = 0x40;
    } else {
        a[80..84].copy_from_slice(&f[80..84]);
    }
    match color {
        Some(c) => {
            a[74] |= 4;
            a[84..88].copy_from_slice(&c);
        }
        None => a[84..88].copy_from_slice(&[0xFF; 4]),
    }
    a
}

/// 넥셀 CF 의 테두리 블록 8바이트: 선 종류는 그대로, 자동 색(0x40)은 한셀 기본 테두리 색으로 — 왼쪽·오른쪽·아래는 검정(8),
/// 위는 자동 그대로, 대각선 색은 0
fn nexcel_cf_border(b: &[u8]) -> [u8; 8] {
    let a = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let c = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let black = |v: u32| if v == 0x40 { 8 } else { v };
    let (left, right) = (black((a >> 16) & 0x7F), black((a >> 23) & 0x7F));
    let (top, bottom) = (c & 0x7F, black((c >> 7) & 0x7F));
    let a2 = (a & 0x0000_FFFF) | (left << 16) | (right << 23);
    let c2 = top | (bottom << 7);
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&a2.to_le_bytes());
    o[4..].copy_from_slice(&c2.to_le_bytes());
    o
}

/// 레거시 DXFN 의 표준 부분 → BIFF8: flags2 의 높은 바이트는 0, 글꼴 블록의 이름 플래그(cchFont 다음 바이트) 1
fn dxfn_std_to8(d: &[u8]) -> Option<Vec<u8>> {
    let std = dxfn_std_len(d)?;
    let mut dxfn = d.get(..std)?.to_vec();
    if dxfn.len() >= 6 {
        dxfn[5] = 0;
        let flags = u32::from_le_bytes([dxfn[0], dxfn[1], dxfn[2], dxfn[3]]);
        if flags & 0x0400_0000 != 0 {
            let at = 6 + numfmt_block_len(&dxfn, flags)?;
            if let Some(b) = dxfn.get_mut(at + 1) {
                *b = 0x01;
            }
        }
    }
    Some(dxfn)
}

/// XFExtNoFRT (예약 0, 0xFFFF, 0, cexts u16, ExtProp × cexts) 가 `d[p..]` 에서 시작해 끝까지 맞으면 그 길이.
/// 들여쓰기(0x0F) 는 한셀이 크기 칸을 4 로 쓰고 값 2바이트를 붙인다
fn xfext_len(d: &[u8]) -> Option<usize> {
    if d.get(..6)? != [0, 0, 0xFF, 0xFF, 0, 0] {
        return None;
    }
    let n = u16::from_le_bytes([*d.get(6)?, *d.get(7)?]) as usize;
    let mut p = 8;
    for _ in 0..n {
        let ty = u16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]);
        let cb = u16::from_le_bytes([*d.get(p + 2)?, *d.get(p + 3)?]) as usize;
        let cb = if ty == 0x0F && cb == 4 { 6 } else { cb };
        if cb < 4 {
            return None;
        }
        p += cb;
    }
    (p <= d.len()).then_some(p)
}

/// XFExtNoFRT 의 ExtProp (종류, 값) 목록
fn xfext_props(d: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    let Some(n) = d.get(6..8).map(|b| u16::from_le_bytes([b[0], b[1]])) else {
        return out;
    };
    let mut p = 8;
    for _ in 0..n {
        let (Some(ty), Some(cb)) = (
            d.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]])),
            d.get(p + 2..p + 4)
                .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize),
        ) else {
            break;
        };
        let cb = if ty == 0x0F && cb == 4 { 6 } else { cb };
        let Some(v) = d.get(p + 4..p + cb.max(4)) else {
            break;
        };
        out.push((ty, v.to_vec()));
        p += cb.max(4);
    }
    out
}

/// 레거시 DXFN12 (DXFN 표준 + 한컴 확장 + 차등 서식 번호 + … + XFExtNoFRT) → BIFF8 DXFN12 (표준 + XFExtNoFRT).
/// 한컴 확장 길이는 블록마다 달라 XFExtNoFRT 를 머리 모양과 끝까지 맞는 길이로 찾는다
fn dxfn12_to8(d: &[u8]) -> Option<Vec<u8>> {
    let std = dxfn_std_len(d)?;
    let mut o = dxfn_std_to8(d)?;
    if let Some(p) = (std..d.len()).find(|&p| xfext_len(&d[p..]) == Some(d.len() - p)) {
        o.extend_from_slice(&d[p..]);
    }
    Some(o)
}

/// CFEX (0x087B): FRT 머리 12 (행 16비트), fIsCF12 u32, nID u16, [CF12 가 아니면 icf u16, cp u8, 틀 u8, 우선순위 u16,
/// 플래그 u8, fHasDXF u8, (DXFN12: cbdxf u32 + 내용), 틀 매개 변수 (cb u8 + 16)]. BIFF8 로는 DXFN12 만 바꾼다
/// (레거시로는 그대로 — 리더는 머리와 끝 16바이트, XFExtNoFRT 의 그라데이션만 본다)
fn cfex_to8(d: &[u8]) -> Option<Vec<u8>> {
    let is12 = u32::from_le_bytes(d.get(12..16)?.try_into().ok()?);
    if is12 != 0 || d.len() < 26 || d[25] == 0 {
        return Some(d.to_vec());
    }
    let cb = u32::from_le_bytes(d.get(26..30)?.try_into().ok()?) as usize;
    let nd = if cb == 0 {
        Vec::new()
    } else {
        dxfn12_to8(d.get(30..30 + cb)?)?
    };
    let mut o = d[..26].to_vec();
    o.extend_from_slice(&(nd.len() as u32).to_le_bytes());
    o.extend_from_slice(&nd);
    o.extend_from_slice(d.get(30 + cb..)?);
    Some(o)
}

/// CFVO: 종류 u8, cce u16, 토큰, (cce 0 이고 최소·최대가 아니면 값 f64)
fn cfvo(src: &[u8], p: &mut usize, o: &mut Vec<u8>, to8: bool) -> Option<()> {
    let ty = *src.get(*p)?;
    let cce = u16::from_le_bytes([*src.get(*p + 1)?, *src.get(*p + 2)?]) as usize;
    let t = ptgs(src.get(*p + 3..*p + 3 + cce)?, to8, true)?;
    o.push(ty);
    o.extend_from_slice(&(t.len() as u16).to_le_bytes());
    o.extend_from_slice(&t);
    *p += 3 + cce;
    if cce == 0 && !matches!(ty, 2 | 3) {
        o.extend_from_slice(src.get(*p..*p + 8)?);
        *p += 8;
    }
    Some(())
}

/// CF12 (0x087A): FRT 머리 12, ct u8, cp u8, cce1·cce2 u16, DXFN12 (cbdxf u32 + 내용, 0 이면 예약 u16), 수식 1·2,
/// 활성 수식 (cce u16 + 토큰), 플래그 u8, 우선순위 u16, 틀 u16, 매개 변수 (cb u8 + cb), 종류별 내용 (앞 u16 = 뒤 길이).
/// 종류별 내용의 CFVO 수식까지 바꾼다 (색조 3, 데이터 막대 4, 위·아래 5, 아이콘 6)
fn cf12(d: &[u8], to8: bool) -> Option<Vec<u8>> {
    let ct = *d.get(12)?;
    let c1 = u16::from_le_bytes([*d.get(14)?, *d.get(15)?]) as usize;
    let c2 = u16::from_le_bytes([*d.get(16)?, *d.get(17)?]) as usize;
    let cb = u32::from_le_bytes(d.get(18..22)?.try_into().ok()?) as usize;
    let mut p = 22;
    let dx = if cb == 0 {
        p += 2;
        None
    } else {
        p += cb;
        let x = d.get(22..22 + cb)?;
        Some(if to8 { dxfn12_to8(x)? } else { x.to_vec() })
    };
    let f1 = ptgs(d.get(p..p + c1)?, to8, true)?;
    let f2 = ptgs(d.get(p + c1..p + c1 + c2)?, to8, true)?;
    p += c1 + c2;
    let ca = u16::from_le_bytes([*d.get(p)?, *d.get(p + 1)?]) as usize;
    let fa = ptgs(d.get(p + 2..p + 2 + ca)?, to8, true)?;
    p += 2 + ca;
    // 플래그 1, 우선순위 2, 틀 2, 매개 변수
    let cbp = *d.get(p + 5)? as usize;
    let mid = d.get(p..p + 6 + cbp)?;
    p += 6 + cbp;
    let ct_old = d.get(p..)?;
    let mut ctn: Vec<u8> = Vec::with_capacity(ct_old.len());
    match ct {
        3 | 4 | 6 if ct_old.len() >= 6 => {
            ctn.extend_from_slice(&ct_old[..6]);
            let mut q = 6;
            match ct {
                3 => {
                    // 색조: 보간 점 수 [3], 색 수 [4] — 점마다 CFVO + 위치 f64, 색마다 위치 f64 + 색 16
                    let (ni, ng) = (ct_old[3] as usize, ct_old[4] as usize);
                    for _ in 0..ni {
                        cfvo(ct_old, &mut q, &mut ctn, to8)?;
                        ctn.extend_from_slice(ct_old.get(q..q + 8)?);
                        q += 8;
                    }
                    ctn.extend_from_slice(ct_old.get(q..q + ng * 24)?);
                    q += ng * 24;
                }
                4 => {
                    // 데이터 막대: 색 16, CFVO 둘
                    ctn.extend_from_slice(ct_old.get(q..q + 16)?);
                    q += 16;
                    cfvo(ct_old, &mut q, &mut ctn, to8)?;
                    cfvo(ct_old, &mut q, &mut ctn, to8)?;
                }
                _ => {
                    // 아이콘: 상태 수 [3] — 상태마다 CFVO + fEqual u8 + 4
                    for _ in 0..ct_old[3] as usize {
                        cfvo(ct_old, &mut q, &mut ctn, to8)?;
                        ctn.extend_from_slice(ct_old.get(q..q + 5)?);
                        q += 5;
                    }
                }
            }
            ctn.extend_from_slice(ct_old.get(q..)?);
            // 한셀은 앞 u16 (명세의 unused) 에 뒤 길이를 쓴다
            let old_len = u16::from_le_bytes([ct_old[0], ct_old[1]]) as usize;
            if old_len == ct_old.len() - 2 {
                let n = (ctn.len() - 2) as u16;
                ctn[0..2].copy_from_slice(&n.to_le_bytes());
            }
        }
        _ => ctn.extend_from_slice(ct_old),
    }
    let mut o = d[..14].to_vec();
    o.extend_from_slice(&(f1.len() as u16).to_le_bytes());
    o.extend_from_slice(&(f2.len() as u16).to_le_bytes());
    match dx {
        Some(x) => {
            o.extend_from_slice(&(x.len() as u32).to_le_bytes());
            o.extend_from_slice(&x);
        }
        None => o.extend_from_slice(&[0; 6]),
    }
    o.extend_from_slice(&f1);
    o.extend_from_slice(&f2);
    o.extend_from_slice(&(fa.len() as u16).to_le_bytes());
    o.extend_from_slice(&fa);
    o.extend_from_slice(mid);
    o.extend_from_slice(&ctn);
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

/// HLINK (BIFF8): Ref8U 8 + CLSID 16 + streamVersion u32 + 플래그 u32 (0x10 표시 이름) + 표시 이름 (u32 글자 수 +
/// UTF-16, NUL 포함) … 표시 이름을 도구 설명(NUL 로 끝나는 UTF-16)으로 바꾸고 0x02 를 켠다
fn hlink_display(b: &[u8], tip: Option<&[u8]>) -> Vec<u8> {
    let Some(tip) = tip else {
        return b.to_vec();
    };
    if b.len() < 36 {
        return b.to_vec();
    }
    let flags = u32::from_le_bytes([b[28], b[29], b[30], b[31]]);
    if flags & 0x10 == 0 {
        return b.to_vec();
    }
    let n = u32::from_le_bytes([b[32], b[33], b[34], b[35]]) as usize;
    let end = 36 + n * 2;
    if end > b.len() {
        return b.to_vec();
    }
    // 도구 설명 글 (NUL 앞까지) + NUL
    let mut units: Vec<u8> = Vec::new();
    for c in tip.chunks(2) {
        if c.len() < 2 || c == [0, 0] {
            break;
        }
        units.extend_from_slice(c);
    }
    units.extend_from_slice(&[0, 0]);
    let mut o = b[..28].to_vec();
    o.extend_from_slice(&(flags | 0x02).to_le_bytes());
    o.extend_from_slice(&((units.len() / 2) as u32).to_le_bytes());
    o.extend_from_slice(&units);
    o.extend_from_slice(&b[end..]);
    o
}

/// 넥셀(.nxl) 의 한컴 차트 0x7EF0 → 레거시 꼴: 앵커의 행 u16 → u32 (18 → 22바이트), 예약 11바이트는 0 (넥셀은 끝
/// 바이트 1), 원본 범위·항목·계열 수식의 행 u16 → u32. Vt 아카이브는 그대로 (docs/format/cell-legacy-chart.md 1·5절)
fn nexcel_chart(d: &[u8]) -> Option<Vec<u8>> {
    let u16_at =
        |k: usize| -> Option<u16> { Some(u16::from_le_bytes([*d.get(k)?, *d.get(k + 1)?])) };
    let mut o = d.get(..8)?.to_vec();
    // 앵커: flags, col1, dx1, row1, dy1, col2, dx2, row2, dy2 (dx·dy 는 32 배 — 레거시 단위)
    for (k, wide) in [
        (8, false),
        (10, false),
        (12, false),
        (14, true),
        (16, false),
        (18, false),
        (20, false),
        (22, true),
        (24, false),
    ] {
        let v = u16_at(k)?;
        if wide {
            o.extend_from_slice(&(v as u32).to_le_bytes());
        } else if matches!(k, 12 | 16 | 20 | 24) {
            o.extend_from_slice(&(v.saturating_mul(32)).to_le_bytes());
        } else {
            o.extend_from_slice(&v.to_le_bytes());
        }
    }
    o.extend_from_slice(&[0; 11]);
    o.extend_from_slice(d.get(37..39)?);
    let mut p = 39;
    let formula = |p: &mut usize, o: &mut Vec<u8>| -> Option<()> {
        let cce = u16_at(*p)? as usize;
        let t = ptgs(d.get(*p + 2..*p + 2 + cce)?, false, false)?;
        o.extend_from_slice(&(t.len() as u16).to_le_bytes());
        o.extend_from_slice(&t);
        *p += 2 + cce;
        Some(())
    };
    formula(&mut p, &mut o)?;
    let alen = u32::from_le_bytes(d.get(p..p + 4)?.try_into().ok()?) as usize;
    o.extend_from_slice(d.get(p..p + 4 + alen)?);
    p += 4 + alen;
    formula(&mut p, &mut o)?;
    let n = u16_at(p)?;
    o.extend_from_slice(&n.to_le_bytes());
    p += 2;
    for _ in 0..n {
        formula(&mut p, &mut o)?;
        formula(&mut p, &mut o)?;
    }
    Some(o)
}

/// 레거시 한컴 차트 0x7EF0 → 넥셀 꼴 ([`nexcel_chart`] 의 거꾸로: 앵커·수식의 행 u32 → u16, 예약 끝 바이트 1)
fn chart_to_nexcel(d: &[u8]) -> Option<Vec<u8>> {
    let patched = nexcel_markers(d);
    let d = patched.as_deref().unwrap_or(d);
    let u16_at =
        |k: usize| -> Option<u16> { Some(u16::from_le_bytes([*d.get(k)?, *d.get(k + 1)?])) };
    let u32_at =
        |k: usize| -> Option<u32> { Some(u32::from_le_bytes(d.get(k..k + 4)?.try_into().ok()?)) };
    let mut o = d.get(..8)?.to_vec();
    // 앵커 22바이트: flags, col1, dx1, row1 u32, dy1, col2, dx2, row2 u32, dy2
    // dx·dy 는 1/32 로 (그리기 앵커처럼 — 한셀 -2003)
    for v in [
        u16_at(8)?,
        u16_at(10)?,
        u16_at(12)? / 32,
        u32_at(14)?.min(0xFFFF) as u16,
        u16_at(18)? / 32,
        u16_at(20)?,
        u16_at(22)? / 32,
        u32_at(24)?.min(0xFFFF) as u16,
        u16_at(28)? / 32,
    ] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    o.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    o.extend_from_slice(d.get(41..43)?);
    let mut p = 43;
    let formula = |p: &mut usize, o: &mut Vec<u8>| -> Option<()> {
        let cce = u16_at(*p)? as usize;
        let t = ptgs(d.get(*p + 2..*p + 2 + cce)?, true, false)?;
        o.extend_from_slice(&(t.len() as u16).to_le_bytes());
        o.extend_from_slice(&t);
        *p += 2 + cce;
        Some(())
    };
    formula(&mut p, &mut o)?;
    let alen = u32_at(p)? as usize;
    o.extend_from_slice(d.get(p..p + 4 + alen)?);
    p += 4 + alen;
    formula(&mut p, &mut o)?;
    let n = u16_at(p)?;
    o.extend_from_slice(&n.to_le_bytes());
    p += 2;
    for _ in 0..n {
        formula(&mut p, &mut o)?;
        formula(&mut p, &mut o)?;
    }
    Some(o)
}

/// 한셀의 넥셀 저장은 자동 표식 계열(VtSeries markerStyle 2, u16_d 1 — 꺾은선)을 원 표식 (markerStyle 4, 크기 140,
/// u16_d 0, 표식·pen5 색의 플래그 0x20) 으로 쓴다 (레거시와 넥셀을 맞댄 chart_line). 바꿀 것이 없으면 None
fn nexcel_markers(d: &[u8]) -> Option<Vec<u8>> {
    use super::vtchart::Value;
    let rec = super::vtchart::parse(d, &[]).ok()?;
    // 꺾은선만: chartType 3 이고 X 축이 값 축(xy 1 — 분산형)이 아니다
    let has = |name: &str, v: Value| {
        rec.archive.objects().any(|o| {
            o.own_fields()
                .iter()
                .any(|f| f.name == name && f.value == v)
        })
    };
    if !has("chartType", Value::U16(3)) || has("xy", Value::U16(1)) {
        return None;
    }
    let mut patches: Vec<(usize, Vec<u8>)> = Vec::new();
    // 실제 계열만 (계열 목록 뒤쪽의 기본 자리 계열은 그대로)
    for o in rec
        .archive
        .objects()
        .filter(|o| o.is_a("VtSeries"))
        .take(rec.series.len())
    {
        let mut fields = Vec::new();
        let mut cur = Some(o);
        while let Some(c) = cur {
            fields.extend(c.own_fields().iter());
            cur = c.base();
        }
        let get = |n: &str| fields.iter().find(|f| f.name == n);
        let (Some(style), Some(flag)) = (get("markerStyle"), get("u16_d")) else {
            continue;
        };
        if style.value != Value::U16(2) || flag.value != Value::U16(1) {
            continue;
        }
        patches.push((style.at, 4u16.to_le_bytes().to_vec()));
        patches.push((flag.at, 0u16.to_le_bytes().to_vec()));
        if let Some(size) = get("markerSize") {
            patches.push((size.at, 140f32.to_le_bytes().to_vec()));
        }
        if let Some(c) = get("markerColor") {
            patches.push((c.at + 5, vec![0x20]));
        }
        if let Some(pen) = get("pen5")
            && let Value::Group(g) = &pen.value
            && let Some(c) = g.iter().find(|f| f.name == "color")
        {
            patches.push((c.at + 5, vec![0x20]));
        }
    }
    if patches.is_empty() {
        return None;
    }
    let mut out = d.to_vec();
    for (at, bytes) in patches {
        if let Some(dst) = out.get_mut(at..at + bytes.len()) {
            dst.copy_from_slice(&bytes);
        }
    }
    Some(out)
}

/// BIFF8 HLINK → 넥셀 꼴 ([`nexcel_hlink`] 의 거꾸로): URL 모니커는 종류 1 (`mailto:` 는 3), 파일 모니커는 5 (경로), 문서 안
/// 위치만 있으면 2 (주소 없음). 표시 이름 자리의 도구 설명은 둘째 글
fn hlink_to_nexcel(b: &[u8]) -> Option<Vec<u8>> {
    let u32_at =
        |k: usize| -> Option<u32> { Some(u32::from_le_bytes(b.get(k..k + 4)?.try_into().ok()?)) };
    let utf16 = |p: usize, n: usize| -> Option<Vec<u16>> {
        Some(
            b.get(p..p + n * 2)?
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect(),
        )
    };
    let flags = u32_at(28)?;
    let mut p = 32;
    let mut display: Vec<u16> = Vec::new();
    if flags & 0x14 != 0 {
        let n = u32_at(p)? as usize;
        display = utf16(p + 4, n)?;
        p += 4 + n * 2;
    }
    if flags & 0x80 != 0 {
        let n = u32_at(p)? as usize;
        p += 4 + n * 2;
    }
    let (kind, addr) = if flags & 0x01 != 0 {
        let clsid = b.get(p..p + 16)?;
        if clsid == URL_MONIKER {
            let len = u32_at(p + 16)? as usize;
            let url = utf16(p + 20, len / 2)?;
            let mail = String::from_utf16_lossy(&url).starts_with("mailto:");
            (if mail { 3 } else { 1 }, url)
        } else if clsid == FILE_MONIKER {
            // FileMoniker: cAnti u16, ansiLength u32, ansiPath, endServer u16, 0xDEAD, 예약 20, cbUnicodePathSize u32,
            // [cbUnicodePathBytes u32, usKeyValue u16, unicodePath]
            let q = p + 16;
            let alen = u32_at(q + 2)? as usize;
            let ansi = b.get(q + 6..q + 6 + alen)?;
            let r = q + 6 + alen + 24;
            let path = match u32_at(r) {
                Some(sz) if sz > 0 => {
                    let n = u32_at(r + 4)? as usize;
                    utf16(r + 10, n / 2)?
                }
                _ => ansi
                    .iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| c as u16)
                    .collect(),
            };
            (5, path)
        } else {
            return None;
        }
    } else if flags & 0x08 != 0 {
        (2, Vec::new())
    } else {
        return None;
    };
    let mut o = b[..28].to_vec();
    o.extend_from_slice(&(kind as u32).to_le_bytes());
    o.extend_from_slice(&3u32.to_le_bytes());
    for t in [&addr, &display] {
        o.extend_from_slice(&(t.len() as u32).to_le_bytes());
        o.push(1);
        for u in t.iter() {
            o.extend_from_slice(&u.to_le_bytes());
        }
    }
    Some(o)
}

/// 넥셀(.nxl) 의 HLINK: Ref8U + CLSID + 2 뒤에 종류 u32 (1 URL, 2 문서 안 위치, 3 메일, 5 파일), u32 3, 주소, 도구 설명
/// (둘 다 u32 글자 수 + 플래그 u8 + 글자) — 한셀 -2003 저장에서 본 꼴 (문서 안 위치는 주소가 비어 있다). BIFF8 HLINK
/// (URL 모니커, 도구 설명은 표시 이름 자리 — 읽을 때 도구 설명이 된다) 로 바꾼다
fn nexcel_hlink(d: &[u8]) -> Option<Vec<u8>> {
    let u32_at =
        |k: usize| -> Option<u32> { Some(u32::from_le_bytes(d.get(k..k + 4)?.try_into().ok()?)) };
    let kind = u32_at(28)?;
    if !matches!(kind, 1 | 2 | 3 | 5) || u32_at(32)? != 3 {
        return None;
    }
    let text = |at: usize| -> Option<(Vec<u16>, usize)> {
        let n = u32_at(at)? as usize;
        let wide = *d.get(at + 4)? & 1 != 0;
        let p = at + 5;
        if wide {
            let b = d.get(p..p + n * 2)?;
            Some((
                b.chunks(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect(),
                p + n * 2,
            ))
        } else {
            Some((d.get(p..p + n)?.iter().map(|&c| c as u16).collect(), p + n))
        }
    };
    let (addr, end) = text(36)?;
    let display = text(end).map(|(t, _)| t).unwrap_or_default();
    let utf16 = |o: &mut Vec<u8>, t: &[u16]| {
        o.extend_from_slice(&(t.len() as u32 + 1).to_le_bytes());
        for u in t {
            o.extend_from_slice(&u.to_le_bytes());
        }
        o.extend_from_slice(&[0, 0]);
    };
    let mut o = d[..28].to_vec();
    let mut flags = if kind == 2 { 0x08u32 } else { 0x03 };
    if !display.is_empty() {
        flags |= 0x14;
    }
    o.extend_from_slice(&flags.to_le_bytes());
    if !display.is_empty() {
        utf16(&mut o, &display);
    }
    if kind == 2 {
        // 문서 안 위치: 넥셀은 위치 글을 담지 않는다 (빈 위치)
        utf16(&mut o, &addr);
        return Some(o);
    }
    // URL 모니커 CLSID 79EAC9E0-BAF9-11CE-8C82-00AA004BA90B, 길이 (바이트, NUL 포함), UTF-16
    o.extend_from_slice(&URL_MONIKER);
    o.extend_from_slice(&((addr.len() as u32 + 1) * 2).to_le_bytes());
    for u in &addr {
        o.extend_from_slice(&u.to_le_bytes());
    }
    o.extend_from_slice(&[0, 0]);
    Some(o)
}

/// HLINK 의 hlstmfIsAbsolute (0x02): 한셀의 .xls 저장은 파일 모니커와 `mailto:` 가 아닌 URL 에 켠다 (레거시는 끈다 —
/// 하이퍼링크 종류 실험: URL·조각이 있는 URL·파일 0x02, 메일·문서 안 위치 없음)
fn hlink_absolute(mut b: Vec<u8>) -> Vec<u8> {
    let Some(flags) = b
        .get(28..32)
        .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    else {
        return b;
    };
    if flags & 0x01 == 0 || flags & 0x02 != 0 {
        return b;
    }
    let mut p = 32;
    for bit in [0x14u32, 0x80] {
        if flags & bit != 0 {
            let Some(n) = b
                .get(p..p + 4)
                .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize)
            else {
                return b;
            };
            p += 4 + n * 2;
        }
    }
    let absolute = match b.get(p..p + 16) {
        Some(c) if c == FILE_MONIKER => true,
        Some(c) if c == URL_MONIKER => {
            let head: Vec<u8> = "mailto:"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            b.get(p + 20..p + 20 + head.len()) != Some(head.as_slice())
        }
        _ => false,
    };
    if absolute {
        b[28..32].copy_from_slice(&(flags | 0x02).to_le_bytes());
    }
    b
}

/// URL 모니커 CLSID
const URL_MONIKER: [u8; 16] = [
    0xE0, 0xC9, 0xEA, 0x79, 0xF9, 0xBA, 0xCE, 0x11, 0x8C, 0x82, 0x00, 0xAA, 0x00, 0x4B, 0xA9, 0x0B,
];
/// 파일 모니커 CLSID 00000303-0000-0000-C000-000000000046
const FILE_MONIKER: [u8; 16] = [
    0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

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

/// 레거시에 없는 시트 정보 (모델에서): 요약 행·열 위치 — 한셀의 레거시 저장은 WSBOOL 에 쓰지 않고 .xls 저장은 쓴다.
/// 차트 부분 스트림 (차트 도형 차례, OBJ ot 5 뒤에 넣는다 — [`super::xls_chart::write`])
#[derive(Debug, Clone, Default)]
pub struct SheetHints {
    pub summary_above: bool,
    pub summary_left: bool,
    pub charts: Vec<Vec<(u16, Vec<u8>)>>,
}

/// 레거시 `/Workbook` 의 시트 하나짜리 내부 XTI: 시트 번호 → ixti (내부 SUPBOOK 은 `시트 수, 01 04`)
pub(crate) fn sheet_xti(stream: &[u8]) -> HashMap<u16, u16> {
    let recs = biff::records(stream);
    let internal: Vec<bool> = recs
        .iter()
        .filter(|r| r.rtype == 0x01AE)
        .map(|r| r.data.get(2..4) == Some(&[0x01, 0x04][..]))
        .collect();
    let mut out = HashMap::new();
    if let Some(r) = recs.iter().find(|r| r.rtype == 0x0017) {
        let (xtis, _) = r.data.get(2..).unwrap_or(&[]).as_chunks::<6>();
        for (i, x) in xtis.iter().enumerate() {
            let v = |k: usize| u16::from_le_bytes([x[k], x[k + 1]]);
            if internal.get(v(0) as usize) == Some(&true) && v(2) == v(4) {
                out.entry(v(2)).or_insert(i as u16);
            }
        }
    }
    out
}

/// 차트 부분 스트림 하나의 레코드 (종류, 내용)
pub(crate) type ChartRecs = Vec<(u16, Vec<u8>)>;

/// .xls 의 차트: 시트마다 차트 부분 스트림 (BOF dt 0x20 … EOF 의 레코드, CONTINUE 는 이어 붙인다) 과 읽기 맥락
pub(crate) struct XlsCharts {
    pub sheets: Vec<Vec<ChartRecs>>,
    /// ixti → 시트 이름 (내부 SUPBOOK 의 시트 하나짜리 XTI)
    pub xti_names: Vec<Option<String>>,
    /// 문서 기본 글꼴 (FONT 0 의 이름, 1/100 pt)
    pub font: (String, u32),
    pub palette: Vec<u32>,
}

/// BIFF8 `/Workbook` → [`XlsCharts`]
pub(crate) fn charts(stream: &[u8]) -> XlsCharts {
    let recs = split_biff8(stream);
    let mut starts = Vec::with_capacity(recs.len());
    let mut pos = 0usize;
    for r in &recs {
        starts.push(pos);
        pos += 4 + r.data.len() + r.conts.iter().map(|c| 4 + c.len()).sum::<usize>();
    }
    let gend = recs.iter().position(|r| r.rt == 0x000A).unwrap_or(0);
    let globals = &recs[..=gend.min(recs.len().saturating_sub(1))];
    let mut names = Vec::new();
    let mut offs = Vec::new();
    let mut internal = Vec::new();
    let mut xti: Vec<(u16, u16, u16)> = Vec::new();
    let mut palette: Vec<u32> = super::reader::DEFAULT_PALETTE.to_vec();
    let mut font: Option<(String, u32)> = None;
    for r in globals {
        let d = r.joined();
        match r.rt {
            0x0085 if d.len() >= 8 => {
                offs.push(u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize);
                let n = d[6] as usize;
                names.push(string_at(&d[7..], n));
            }
            0x01AE => internal.push(d.get(2..4) == Some(&[0x01, 0x04][..])),
            0x0017 if d.len() >= 2 => {
                let (x, _) = d[2..].as_chunks::<6>();
                xti = x
                    .iter()
                    .map(|e| {
                        let v = |k: usize| u16::from_le_bytes([e[k], e[k + 1]]);
                        (v(0), v(2), v(4))
                    })
                    .collect();
            }
            0x0092 if d.len() >= 2 => {
                let n = u16::from_le_bytes([d[0], d[1]]) as usize;
                for (i, c) in d[2..].chunks(4).take(n).enumerate() {
                    if c.len() == 4 && i < palette.len() {
                        palette[i] = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
                    }
                }
            }
            0x0031 if font.is_none() && d.len() >= 16 => {
                let twips = u16::from_le_bytes([d[0], d[1]]) as u32;
                font = Some((string_at(&d[15..], d[14] as usize), twips * 5));
            }
            _ => {}
        }
    }
    let xti_names = xti
        .iter()
        .map(|&(sb, a, b)| {
            (internal.get(sb as usize) == Some(&true) && a == b)
                .then(|| names.get(a as usize).cloned())
                .flatten()
        })
        .collect();
    let mut sheets = Vec::new();
    for off in offs {
        let mut charts = Vec::new();
        if let Some(start) = starts.iter().position(|&s| s == off) {
            let mut depth = 0usize;
            let mut cur: Option<Vec<(u16, Vec<u8>)>> = None;
            for r in recs.iter().skip(start) {
                if r.rt == 0x0809 {
                    depth += 1;
                    if depth == 2 {
                        cur = Some(Vec::new());
                    }
                }
                if let Some(c) = cur.as_mut() {
                    c.push((r.rt, r.joined()));
                }
                if r.rt == 0x000A {
                    if depth == 2
                        && let Some(c) = cur.take()
                    {
                        charts.push(c);
                    }
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
            }
        }
        sheets.push(charts);
    }
    XlsCharts {
        sheets,
        xti_names,
        font: font.unwrap_or_else(|| ("맑은 고딕".into(), 1100)),
        palette,
    }
}

/// 플래그 바이트 + 글자 (플래그 1 이면 UTF-16)
fn string_at(d: &[u8], cch: usize) -> String {
    match d.first() {
        Some(f) if f & 1 != 0 => {
            let u: Vec<u16> = d
                .get(1..1 + cch * 2)
                .unwrap_or_default()
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&u)
        }
        _ => d
            .get(1..1 + cch)
            .unwrap_or_default()
            .iter()
            .map(|&b| b as char)
            .collect(),
    }
}

/// 하이퍼링크의 도구 설명 (범위의 왼쪽 위 칸 (행, 열) → 글) — 한셀의 .xls 저장은 HLINK 의 표시 이름 자리에 쓴다
pub type LinkTips = Vec<((u32, u32), String)>;

/// 넥셀 조건부 서식 묶음 하나: (규칙 수 — CF 로 쓰지 않는 색 막대·데이터 막대·아이콘 집합도 센다,
/// CF 로 쓰는 규칙마다 우선순위 차례로 "상위·하위 규칙인가")
pub type CfGroup = (u16, Vec<bool>);

fn sheet_to8(
    recs: &[Rec],
    hints: Option<&SheetHints>,
    tips: Option<&LinkTips>,
    nxl: bool,
    dxfs: &[Vec<u8>],
    cf_groups: &[CfGroup],
) -> Vec<OutRec> {
    // 넥셀 CONDFMT 묶음 차례 (CF 로 쓰는 규칙이 있는 묶음마다)
    let mut cf_groups = cf_groups.iter();
    let mut top10: Vec<bool> = Vec::new();
    let mut cf_nth = 0usize;
    let mut out: Vec<OutRec> = Vec::new();
    // 셀 표: ROW 와 셀 묶음을 모았다가 32 행 묶음마다 ROW, 셀 차례로 쓴다
    let mut rows: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut units: Vec<Unit> = Vec::new();
    let mut table_at: Option<usize> = None;
    // MSODRAWING 은 시트 끝에서 앵커를 한꺼번에 바꾼다 (자리만 남긴다)
    let mut art_slots: Vec<usize> = Vec::new();
    let mut art_parts: Vec<Vec<u8>> = Vec::new();
    let row_of = |d: &[u8]| -> u32 { u32::from_le_bytes([d[0], d[1], d[2], d[3]]) };
    let mut chart_k = 0usize;
    for rec in recs {
        let rt = rec.rt;
        let d = &rec.data;
        // 넥셀: 한컴 차트(0x7EF0, 행 16비트 + CONTINUE)와 끝 표시는 남기고, 표준과 다른 몇 레코드는 레거시 그대로
        if nxl {
            match rt {
                0x7EF0 => {
                    if let Some(b) = chart_to_nexcel(d) {
                        let (head, rest) = b.split_at(b.len().min(MAX_DATA));
                        let conts = rest.chunks(MAX_DATA).map(<[u8]>::to_vec).collect();
                        out.push((rt, head.to_vec(), conts));
                    }
                    continue;
                }
                0x7EF8 | 0x00A1 => {
                    out.push((rt, d.clone(), Vec::new()));
                    continue;
                }
                0x0809 => {
                    let mut b = d.clone();
                    b[0..2].copy_from_slice(&BIFF8_VERSION.to_le_bytes());
                    b[4..12].copy_from_slice(&[0x01, 0x01, 0xD8, 0x07, 0, 0, 0, 0]);
                    out.push((rt, b, Vec::new()));
                    continue;
                }
                0x0862 => {
                    let mut b = d.clone();
                    b.truncate(24);
                    out.push((rt, b, Vec::new()));
                    continue;
                }
                0x087A | 0x087B => continue,
                0x023E => {
                    // WINDOW2: .xls 꼴에서 머리글 색은 0, 끝 4바이트는 레거시 그대로 (0x80000000)
                    if let Some(mut b) = conv(rt, d, true) {
                        if b.len() >= 18 && d.len() >= 4 {
                            b[6] = 0;
                            b[7] = 0;
                            b[14..18].copy_from_slice(&d[d.len() - 4..]);
                        }
                        out.push((rt, b, Vec::new()));
                    }
                    continue;
                }
                _ => {}
            }
        }
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
            0x005D => {
                if let Some(b) = conv(rt, d, true) {
                    out.push((rt, b, Vec::new()));
                }
                // 차트 도형의 OBJ (ftCmo ot 5) 뒤에 차트 부분 스트림
                if d.get(4..6) == Some(&[5, 0][..])
                    && let Some(ch) = hints.and_then(|h| h.charts.get(chart_k))
                {
                    out.extend(ch.iter().map(|(rt, d)| (*rt, d.clone(), Vec::new())));
                    chart_k += 1;
                }
            }
            0x01B6 => {
                if let Some((h, c)) = txo_to8(d) {
                    out.push((rt, h, c));
                }
            }
            0x01B0 => {
                if let Some(mut b) = condfmt(d, true, false) {
                    // 넥셀은 CONDFMT 의 nID 칸이 늘 1 이고, 규칙 수(ccf)는 CF 로 쓰지 않는 규칙 (색 막대·데이터 막대·
                    // 아이콘 집합 — 레거시의 CONDFMT12 쪽)까지 센다 (한셀 -2003 저장 대조)
                    if nxl && b.len() >= 4 {
                        let ccf = u16::from_le_bytes([b[0], b[1]]);
                        let g = cf_groups.next();
                        let total = g.map_or(ccf, |g| g.0).max(ccf);
                        top10 = g.map(|g| g.1.clone()).unwrap_or_default();
                        cf_nth = 0;
                        b[0..2].copy_from_slice(&total.to_le_bytes());
                        b[2..4].copy_from_slice(&1u16.to_le_bytes());
                    }
                    out.push((rt, b, Vec::new()));
                }
            }
            // 넥셀은 CONDFMT12 를 쓰지 않는다
            0x0879 if nxl => {}
            0x0879 => {
                if let Some(b) = condfmt(d, true, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x01B1 => {
                let b = if nxl {
                    cf_to_nexcel(d, dxfs).map(|mut b| {
                        // 상위·하위 규칙은 넥셀 CF 의 종류 5 (한셀 -2003 저장 대조 — 한셀도 다시 열면 잃는다)
                        if top10.get(cf_nth) == Some(&true) {
                            b[0] = 5;
                        }
                        b
                    })
                } else {
                    cf_to8(d)
                };
                cf_nth += 1;
                if let Some(b) = b {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x087A => {
                if let Some(b) = cf12(d, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x087B => {
                if let Some(b) = cfex_to8(d) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x01BE => {
                if let Some(b) = dv(d, true) {
                    out.push((rt, b, Vec::new()));
                }
            }
            0x0081 if d.len() >= 2 => {
                // WSBOOL: 요약 행 아래(0x40)·요약 열 오른쪽(0x80)
                let mut v = u16::from_le_bytes([d[0], d[1]]);
                if let Some(h) = hints {
                    v &= !0x00C0;
                    if !h.summary_above {
                        v |= 0x0040;
                    }
                    if !h.summary_left {
                        v |= 0x0080;
                    }
                }
                out.push((rt, v.to_le_bytes().to_vec(), Vec::new()));
            }
            0x01B8 => {
                // HLINK: 한셀의 .xls 저장은 표시 이름 자리에 도구 설명(뒤따르는 HLINKTOOLTIP)을 쓰고 절대 경로
                // 비트(0x02)를 켠다
                let at = (d.len() >= 12).then(|| {
                    (
                        u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
                        u16::from_le_bytes([d[8], d[9]]) as u32,
                    )
                });
                let tip: Option<Vec<u8>> = tips
                    .and_then(|t| t.iter().find(|(c, _)| Some(*c) == at))
                    .map(|(_, s)| s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect());
                if let Some(b) = conv(rt, d, true) {
                    let b = hlink_display(&b, tip.as_deref());
                    let b = if nxl {
                        hlink_to_nexcel(&b).unwrap_or(b)
                    } else {
                        hlink_absolute(b)
                    };
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
    // 넥셀은 한 CONDFMT 의 규칙(CF) 차례를 거꾸로 쓴다 (한셀 -2003 저장 대조 — 규칙이 둘 이상인 문서)
    if nxl {
        let mut i = 0;
        while i < out.len() {
            if out[i].0 == 0x01B1 {
                let mut j = i;
                while j < out.len() && out[j].0 == 0x01B1 {
                    j += 1;
                }
                out[i..j].reverse();
                i = j;
            } else {
                i += 1;
            }
        }
    }
    out
}

/// 레거시 `/Workbook` 스트림 → BIFF8 `/Workbook` 스트림 (`.xls`)
pub fn to_biff8(stream: &[u8]) -> Result<Vec<u8>> {
    to_biff8_with(stream, &[], &[])
}

/// [`to_biff8`] 에 시트마다 레거시에 없는 정보를 더해서
pub fn to_biff8_with(stream: &[u8], hints: &[SheetHints], tips: &[LinkTips]) -> Result<Vec<u8>> {
    transcode(stream, hints, tips, false, &[])
}

/// 레거시 `/Workbook` → 넥셀(.nxl, 한셀 -2003) 꼴: BIFF8 틀(16비트 행, 플래그가 있는 문자열, CONTINUE)에 한컴
/// 글꼴·셀 서식 확장, 문단 모양, 한컴 차트(0x7EF0)를 그대로 두고 표 스타일·차등 서식 표·XFCRC·PLV·CF12·CFEX 는
/// 쓰지 않는다 (한셀이 문서 61개를 .nxl 로 저장한 것 — docs/format/other-formats.md 6절)
pub fn to_nxl(stream: &[u8], tips: &[LinkTips], cf_groups: &[Vec<CfGroup>]) -> Result<Vec<u8>> {
    transcode(stream, &[], tips, true, cf_groups)
}

fn transcode(
    stream: &[u8],
    hints: &[SheetHints],
    tips: &[LinkTips],
    nxl: bool,
    cf_groups: &[Vec<CfGroup>],
) -> Result<Vec<u8>> {
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
    // 레거시 차등 서식 표 (0x088D) — 넥셀 CF 의 글꼴 색이 여기서 온다
    let dxf_recs: Vec<Vec<u8>> = globals
        .iter()
        .filter(|r| r.rt == 0x088D)
        .map(|r| r.data.clone())
        .collect();
    let sheets: Vec<Vec<OutRec>> = subs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let groups = cf_groups.get(i).map_or(&[][..], |v| v.as_slice());
            sheet_to8(s, hints.get(i), tips.get(i), nxl, &dxf_recs, groups)
        })
        .collect();

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
        if nxl {
            match rt {
                // 한컴 레코드는 그대로 (0xA030 은 없다), 표 스타일·차등 서식 표는 없다
                0x0900 | 0xA001 | 0xA011 | 0xA020 => {
                    g.push((rt, d.clone(), Vec::new()));
                    continue;
                }
                0xA030 | 0x088D..=0x0890 => continue,
                // FONT·XF 는 한컴 확장째 (FONT 의 이름 플래그만 1), 머리글·바닥글은 글 플래그만 1
                0x0031 => {
                    let mut b = d.clone();
                    if b.len() > 15 && b[14] > 0 {
                        b[15] = 1;
                    }
                    g.push((rt, b, Vec::new()));
                    continue;
                }
                0x00E0 => {
                    g.push((rt, d.clone(), Vec::new()));
                    continue;
                }
                0x0014 | 0x0015 => {
                    let mut b = d.clone();
                    if b.len() > 2 {
                        b[2] = 1;
                    }
                    g.push((rt, b, Vec::new()));
                    continue;
                }
                0x0809 => {
                    let mut b = d.clone();
                    b[0..2].copy_from_slice(&BIFF8_VERSION.to_le_bytes());
                    b[4..12].copy_from_slice(&[0x01, 0x01, 0xD8, 0x07, 0, 0, 0, 0]);
                    if b.len() >= 16 {
                        b[12..16].copy_from_slice(&[0x06, 0, 0, 0]);
                    }
                    g.push((rt, b, Vec::new()));
                    continue;
                }
                _ => {}
            }
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

    if nxl && let Some(at) = g.iter().rposition(|r| r.0 == 0x0085) {
        // COUNTRY (한국 82, 82)
        g.insert(at + 1, (0x008C, vec![0x52, 0, 0x52, 0], Vec::new()));
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
                parts.iter().map(|p| 4 + p.len()).sum::<usize>() + 4 + 2 + buckets.len().min(1) * 8
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

// ---------------------------------------------------------------------------
// BIFF8 → 레거시 (.xls 읽기)
// ---------------------------------------------------------------------------

/// CONTINUE 로 나뉜 데이터를 차례로 읽는다. 글자 데이터가 조각 경계를 넘으면 새 조각 첫 바이트가 플래그다
struct Frags<'a> {
    parts: Vec<&'a [u8]>,
    i: usize,
    p: usize,
}

impl<'a> Frags<'a> {
    fn new(rec: &'a Rec) -> Frags<'a> {
        let mut parts = vec![rec.data.as_slice()];
        parts.extend(rec.conts.iter().map(|c| c.as_slice()));
        Frags { parts, i: 0, p: 0 }
    }
    fn at_end(&mut self) -> bool {
        while self.i < self.parts.len() && self.p >= self.parts[self.i].len() {
            self.i += 1;
            self.p = 0;
        }
        self.i >= self.parts.len()
    }
    fn byte(&mut self) -> Option<u8> {
        if self.at_end() {
            return None;
        }
        let b = self.parts[self.i][self.p];
        self.p += 1;
        Some(b)
    }
    fn bytes(&mut self, n: usize) -> Option<Vec<u8>> {
        (0..n).map(|_| self.byte()).collect()
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes([self.byte()?, self.byte()?]))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes([
            self.byte()?,
            self.byte()?,
            self.byte()?,
            self.byte()?,
        ]))
    }
    /// 글자 cch 개 → UTF-16LE 바이트. 조각이 바뀌면 그 첫 바이트가 새 플래그 (비트 0 = 2바이트 글자)
    fn chars(&mut self, cch: usize, mut wide: bool) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(cch * 2);
        for _ in 0..cch {
            if self.p >= self.parts.get(self.i)?.len() {
                self.i += 1;
                self.p = 0;
                wide = self.byte()? & 0x01 != 0;
            }
            if wide {
                out.push(self.byte()?);
                out.push(self.byte()?);
            } else {
                out.push(self.byte()?);
                out.push(0);
            }
        }
        Some(out)
    }
}

/// BIFF8 SST → 레거시 SST (서식 런은 글자 뒤, 플래그 0x01, 음성 정보는 버린다)
fn sst_to_hancom(rec: &Rec) -> Option<Vec<u8>> {
    let mut f = Frags::new(rec);
    let total = f.u32()?;
    let unique = f.u32()?;
    let mut o = Vec::new();
    o.extend_from_slice(&total.to_le_bytes());
    let count_at = o.len();
    o.extend_from_slice(&unique.to_le_bytes());
    let mut n = 0u32;
    for _ in 0..unique {
        let Some(cch) = f.u16() else { break };
        let Some(flags) = f.byte() else { break };
        let runs = if flags & 0x08 != 0 { f.u16()? } else { 0 };
        let ext = if flags & 0x04 != 0 { f.u32()? } else { 0 };
        let chars = f.chars(cch as usize, flags & 0x01 != 0)?;
        let run_bytes = f.bytes(runs as usize * 4)?;
        f.bytes(ext as usize)?;
        o.extend_from_slice(&cch.to_le_bytes());
        o.push(if runs > 0 { 0x01 } else { 0x00 });
        o.extend_from_slice(&chars);
        if runs > 0 {
            o.extend_from_slice(&runs.to_le_bytes());
            o.extend_from_slice(&run_bytes);
        }
        n += 1;
    }
    o[count_at..count_at + 4].copy_from_slice(&n.to_le_bytes());
    Some(o)
}

/// BIFF8 STRING (수식의 글 결과, 긴 글은 CONTINUE) → 레거시 (cch u16, 플래그, UTF-16)
fn string_to_hancom(rec: &Rec) -> Option<Vec<u8>> {
    let mut f = Frags::new(rec);
    let cch = f.u16()?;
    let flag = f.byte()?;
    let chars = f.chars(cch as usize, flag & 0x01 != 0)?;
    let mut o = Vec::with_capacity(3 + chars.len());
    o.extend_from_slice(&cch.to_le_bytes());
    o.push(0x01);
    o.extend_from_slice(&chars);
    Some(o)
}

/// BIFF8 TXO (18바이트 + CONTINUE 글 + CONTINUE 런) → 레거시 TXO
fn txo_to_hancom(rec: &Rec) -> Option<Vec<u8>> {
    let d = &rec.data;
    let grbit = u16::from_le_bytes([*d.first()?, *d.get(1)?]);
    let cch = u16::from_le_bytes([*d.get(10)?, *d.get(11)?]) as usize;
    let h = match (grbit >> 1) & 0x07 {
        2 => 2,
        3 => 3,
        4 => 5,
        7 => 7,
        _ => 1,
    };
    let v = match (grbit >> 4) & 0x07 {
        2 => 1,
        3 => 2,
        4 => 3,
        7 => 4,
        _ => 0,
    };
    let mut text = Vec::new();
    let mut runs: Vec<(u16, u16)> = Vec::new();
    if cch > 0 && !rec.conts.is_empty() {
        // 글: 첫 CONTINUE 부터 (조각마다 첫 바이트가 플래그)
        let mut tf = Frags {
            parts: rec.conts.iter().map(|c| c.as_slice()).collect(),
            i: 0,
            p: 0,
        };
        let flag = tf.byte()?;
        text = tf.chars(cch, flag & 0x01 != 0)?;
        // 런: 글 다음 조각부터 8바이트씩, 마지막 (cch, …) 은 끝 표지
        if tf.p > 0 {
            tf.i += 1;
            tf.p = 0;
        }
        while let (Some(ich), Some(ifnt)) = (tf.u16(), tf.u16()) {
            tf.bytes(4);
            if ich as usize >= cch {
                break;
            }
            runs.push((ich, ifnt));
        }
    }
    let paragraphs = {
        let s = crate::text::utf16le(&text);
        s.split('\n').count().max(1) as u16
    };
    let mut o = Vec::new();
    o.extend_from_slice(&(grbit & 0x0200).to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.push(h);
    o.push(v);
    o.extend_from_slice(&(runs.len() as u16).to_le_bytes());
    o.extend_from_slice(&paragraphs.to_le_bytes());
    o.extend_from_slice(&(cch as u16).to_le_bytes());
    o.push(0);
    o.extend_from_slice(&text);
    for (ich, ifnt) in runs {
        o.extend_from_slice(&ich.to_le_bytes());
        o.extend_from_slice(&ifnt.to_le_bytes());
    }
    for _ in 0..paragraphs {
        o.extend_from_slice(&0u32.to_le_bytes());
    }
    Some(o)
}

/// XFEXT 의 색 (FullColorExt: 종류 u16 (1 색인, 2 RGB), 밝기 i16, 값 u32) → COLORREF. 테마 색은 모른다
fn xfext_colorref(v: &[u8], palette: &[u32]) -> Option<u32> {
    let ty = u16::from_le_bytes([*v.first()?, *v.get(1)?]);
    let val = u32::from_le_bytes([*v.get(4)?, *v.get(5)?, *v.get(6)?, *v.get(7)?]);
    match ty {
        // RGBA 바이트 (r, g, b, a) → 0x00BBGGRR
        2 => Some(val & 0x00FF_FFFF),
        1 => {
            let rgb = *palette.get((val as usize).checked_sub(8)?)?;
            Some(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
        }
        _ => None,
    }
}

/// BIFF8 XF (20바이트) + 그 XF 의 XFEXT 색 → 레거시 XF (확장: 개수 2 + COLORREF 8개 — 무늬 배경, 채우기, 위,
/// 왼쪽, 아래, 오른쪽, 대각선, 미상)
fn xf_to_hancom(
    d: &[u8],
    ext: Option<&Vec<(u16, Vec<u8>)>>,
    palette: &[u32],
    para: Option<u32>,
) -> Vec<u8> {
    let mut o = d[..d.len().min(20)].to_vec();
    let mut colors = [0xFFFF_FFFFu32; 8];
    for (ty, v) in ext.map(|e| e.as_slice()).unwrap_or_default() {
        let slot = match ty {
            5 => 0,
            4 => 1,
            7 => 2,
            9 => 3,
            8 => 4,
            10 => 5,
            11 => 6,
            _ => continue,
        };
        if let Some(c) = xfext_colorref(v, palette) {
            colors[slot] = c;
        }
    }
    if para.is_none() && colors.iter().all(|&c| c == 0xFFFF_FFFF) {
        return o;
    }
    match para {
        Some(k) => {
            o.extend_from_slice(&3u16.to_le_bytes());
            o.extend_from_slice(&k.to_le_bytes());
        }
        None => o.extend_from_slice(&2u16.to_le_bytes()),
    }
    for c in colors {
        o.extend_from_slice(&c.to_le_bytes());
    }
    o
}

/// 레거시 FONT 의 한컴 확장 (플래그 1 + 78바이트): 한셀이 .xls 를 열 때 붙이는 값 — 크기 1/100 pt, 밑줄 위치 1 (아래)
/// 과 모양 (한 줄 실선 0, 두 줄 doubleslim 7), 취소선 1 실선, 그림자 위치 10·10, 그림자 색 C0C0C0, 음영 없음,
/// 상대 크기·장평 100, 글자 색, 외곽선 비트면 외곽선 실선
fn font_ext(base: &[u8], color: u32) -> Vec<u8> {
    let mut o = base.to_vec();
    if base.len() < 14 {
        return o;
    }
    let height = u16::from_le_bytes([base[0], base[1]]) as u32;
    let grbit = u16::from_le_bytes([base[2], base[3]]);
    let uls = base[10];
    let mut e = Vec::with_capacity(79);
    e.push(0x01);
    e.extend_from_slice(&(height * 5).to_le_bytes());
    e.extend_from_slice(&0x0008u16.to_le_bytes());
    e.extend_from_slice(&u16::from(uls != 0).to_le_bytes());
    e.extend_from_slice(&0u16.to_le_bytes());
    e.extend_from_slice(&(if uls == 2 || uls == 0x22 { 7u32 } else { 0 }).to_le_bytes());
    // 외곽선: FONT 의 외곽선 비트면 실선 (1)
    e.extend_from_slice(&u32::from(grbit & 0x10 != 0).to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    e.extend_from_slice(&10i32.to_le_bytes());
    e.extend_from_slice(&10i32.to_le_bytes());
    e.extend_from_slice(&u32::from(grbit & 0x08 != 0).to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    e.extend_from_slice(&color.to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    e.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    e.extend_from_slice(&0x00C0_C0C0u32.to_le_bytes());
    e.extend_from_slice(&100i32.to_le_bytes());
    e.extend_from_slice(&100i32.to_le_bytes());
    e.extend_from_slice(&0i32.to_le_bytes());
    e.extend_from_slice(&0i32.to_le_bytes());
    e.extend_from_slice(&0u32.to_le_bytes());
    o.extend_from_slice(&e);
    o
}

/// BIFF8 CF → 레거시 CF: DXFN 표준 블록 뒤에 차등 서식 번호 u32 (테두리 블록이 있으면 안쪽 선 확장 `00 00`),
/// 수식은 레거시 토큰으로
fn cf_to_hancom(d: &[u8], id: u32) -> Option<Vec<u8>> {
    let c1 = u16::from_le_bytes([*d.get(2)?, *d.get(3)?]) as usize;
    let c2 = u16::from_le_bytes([*d.get(4)?, *d.get(5)?]) as usize;
    let std = dxfn_std_len(d.get(6..)?)?;
    let fstart = 6 + std;
    let f1 = ptgs(d.get(fstart..fstart + c1)?, false, true)?;
    let f2 = ptgs(d.get(fstart + c1..fstart + c1 + c2)?, false, true)?;
    let mut o = d[..2].to_vec();
    o.extend_from_slice(&(f1.len() as u16).to_le_bytes());
    o.extend_from_slice(&(f2.len() as u16).to_le_bytes());
    o.extend_from_slice(&d[6..fstart]);
    o.extend_from_slice(&id.to_le_bytes());
    let flags = u32::from_le_bytes([d[6], d[7], d[8], d[9]]);
    if flags & 0x1000_0000 != 0 {
        o.extend_from_slice(&[0, 0]);
    }
    o.extend_from_slice(&f1);
    o.extend_from_slice(&f2);
    Some(o)
}

/// XFExtNoFRT 의 ExtProp (종류, 값) 목록
type ExtProps = Vec<(u16, Vec<u8>)>;

/// .xls 의 CF 서식으로 만드는 레거시 차등 서식 표 (0x088D) — 레거시 CF 는 서식을 이 표의 번호로 가리킨다
struct CfDxfs {
    /// 원래 있던 차등 서식 수 (새 번호의 시작)
    base: u32,
    palette: Vec<u32>,
    /// 새 0x088D 레코드 내용
    out: Vec<Vec<u8>>,
}

/// 색 번호 → COLORREF (0x00BBGGRR). 8–63 은 팔레트, 0–7 은 기본 8색, 그 밖은 자동
fn icv_colorref(icv: u32, palette: &[u32]) -> Option<u32> {
    const BASE: [u32; 8] = [
        0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF,
    ];
    let rgb = match icv {
        0..=7 => BASE[icv as usize],
        8..=63 => *palette.get(icv as usize - 8)?,
        _ => return None,
    };
    Some(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

/// XFPropColor (8바이트): RGB 종류 (0x05) + RGBA, 자동은 `FF FF FF DF` (dxf::prop_color 와 같은 꼴)
fn xfprop_color(c: Option<u32>) -> Vec<u8> {
    match c {
        Some(c) => vec![5, 0, 0, 0, c as u8, (c >> 8) as u8, (c >> 16) as u8, 0],
        None => vec![5, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF],
    }
}

/// BIFF8 DXFN (CF 의 서식) + CFEX 의 XFExtNoFRT 색 → 레거시 0x088D 내용 (FRT 머리 12, u32 3, cprops, XFProp).
/// 글꼴(크기·굵게·기울임·밑줄·첨자·취소선·색), 테두리 네 변, 무늬와 두 색. 표시 형식·맞춤·보호는 CF 의 DXFN 에서
/// 리더가 읽는다
fn cf_dxf(dxfn: &[u8], ext: &[(u16, Vec<u8>)], palette: &[u32]) -> Vec<u8> {
    let mut props: Vec<(u16, Vec<u8>)> = Vec::new();
    let ext_color = |ty: u16| {
        ext.iter()
            .find(|(t, _)| *t == ty)
            .and_then(|(_, v)| xfext_colorref(v, palette))
    };
    let u32_at = |d: &[u8], k: usize| -> u32 {
        d.get(k..k + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(u32::MAX)
    };
    (|| -> Option<()> {
        let flags = u32_at(dxfn, 0);
        let mut at = 6 + numfmt_block_len(dxfn, flags)?;
        if flags & 0x0400_0000 != 0 {
            // DXFFntD: 높이 i32 @64, ts @68 (0x02 기울임, 0x80 취소선), bls u16 @72, sss u16 @74, uls u8 @76,
            // icvFore @80, tsNinch @88, fSssNinch @92, fUlsNinch @96, fBlsNinch @100
            let f = dxfn.get(at..at + 118)?;
            // 글꼴 이름: cchFont u8, 플래그 u8 (1 이면 UTF-16), 글자
            let cch = f[0] as usize;
            if cch > 0 {
                let name: Vec<u16> = if f[1] & 1 != 0 {
                    f.get(2..2 + cch * 2)?
                        .chunks(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect()
                } else {
                    f.get(2..2 + cch)?.iter().map(|&b| b as u16).collect()
                };
                let mut v = (name.len() as u16).to_le_bytes().to_vec();
                for u in name {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                props.push((0x18, v));
            }
            let ts = u32_at(f, 68);
            let ts_ninch = u32_at(f, 88);
            let icv = u32_at(f, 80);
            if icv != u32::MAX || ext_color(0x0D).is_some() {
                let color = ext_color(0x0D).or_else(|| icv_colorref(icv, palette));
                props.push((0x05, xfprop_color(color)));
            }
            let height = u32_at(f, 64) as i32;
            if height > 0 {
                props.push((0x24, ((height as u32) * 5).to_le_bytes().to_vec()));
            }
            if u32_at(f, 100) == 0 && u16::from_le_bytes([f[72], f[73]]) >= 700 {
                props.push((0x19, 700u16.to_le_bytes().to_vec()));
            }
            if u32_at(f, 96) == 0 && f[76] != 0 {
                props.push((0x1A, (f[76] as u16).to_le_bytes().to_vec()));
            }
            let sss = u16::from_le_bytes([f[74], f[75]]);
            if u32_at(f, 92) == 0 && sss != 0 {
                props.push((0x1B, sss.to_le_bytes().to_vec()));
            }
            if ts_ninch & 0x02 == 0 && ts & 0x02 != 0 {
                props.push((0x1C, vec![1]));
            }
            if ts_ninch & 0x80 == 0 && ts & 0x80 != 0 {
                props.push((0x1D, vec![1]));
            }
            at += 118;
        }
        if flags & 0x0800_0000 != 0 {
            at += 8;
        }
        if flags & 0x1000_0000 != 0 {
            // DXFBdr: 선 종류 4비트 × 4 (왼쪽, 오른쪽, 위, 아래), 색 7비트 (왼쪽, 오른쪽) / (위, 아래)
            let a = u32_at(dxfn, at);
            let b = u32_at(dxfn, at + 4);
            for (k, ty, xty, icv) in [
                (0, 0x08u16, 9u16, (a >> 16) & 0x7F),
                (1, 0x09, 10, (a >> 23) & 0x7F),
                (2, 0x06, 7, b & 0x7F),
                (3, 0x07, 8, (b >> 7) & 0x7F),
            ] {
                if flags & (1 << (10 + k)) != 0 {
                    continue;
                }
                let dg = ((a >> (4 * k)) & 0x0F) as u16;
                let mut v = if dg == 0 {
                    vec![5, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]
                } else {
                    let mut c = xfprop_color(ext_color(xty).or_else(|| icv_colorref(icv, palette)));
                    c[1] = 0xFF;
                    c
                };
                v.extend_from_slice(&dg.to_le_bytes());
                props.push((ty, v));
            }
            at += 8;
        }
        if flags & 0x2000_0000 != 0 {
            // DXFPat: 무늬 6비트 (@0 의 10–15), 앞 색 7비트·뒤 색 7비트 (@2)
            let a = u16::from_le_bytes([*dxfn.get(at)?, *dxfn.get(at + 1)?]);
            let b = u16::from_le_bytes([*dxfn.get(at + 2)?, *dxfn.get(at + 3)?]);
            let fls = (a >> 10) as u8;
            if flags & 0x0001_0000 == 0 && fls != 0 {
                props.push((0x00, vec![fls]));
            }
            if flags & 0x0002_0000 == 0 {
                let c = ext_color(4).or_else(|| icv_colorref((b & 0x7F) as u32, palette));
                props.push((0x01, xfprop_color(c)));
            }
            if flags & 0x0004_0000 == 0 {
                let c = ext_color(5).or_else(|| icv_colorref(((b >> 7) & 0x7F) as u32, palette));
                props.push((0x02, xfprop_color(c)));
            }
        }
        Some(())
    })();
    // 한셀의 차례: 채우기, 글꼴 (색, 크기, 그 밖), 테두리 (dxf::dxf_props)
    let key = |t: u16| match t {
        0x00..=0x04 => (0, t),
        0x05 => (1, 0),
        0x18 => (1, 1),
        0x24 => (1, 2),
        0x19..=0x23 => (1, t),
        0x06..=0x0E => (2, t),
        _ => (5, t),
    };
    props.sort_by_key(|p| key(p.0));
    let mut o = vec![0x8D, 0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    o.extend_from_slice(&3u32.to_le_bytes());
    o.extend_from_slice(&(props.len() as u16).to_le_bytes());
    for (ty, v) in props {
        o.extend_from_slice(&ty.to_le_bytes());
        o.extend_from_slice(&((v.len() + 4) as u16).to_le_bytes());
        o.extend_from_slice(&v);
    }
    o
}

/// BIFF8 시트 부분 → 레거시 레코드들
fn sheet_to_hancom(recs: &[Rec], dxfs: &mut CfDxfs) -> Vec<(u16, Vec<u8>)> {
    let mut out: Vec<(u16, Vec<u8>)> = Vec::new();
    // CFEX 의 DXFN12 에 든 XFExtNoFRT 색: (nID, icf) → ExtProp 들 (CFEX 는 시트의 CF 뒤에 온다)
    let mut cf_ext: HashMap<(u16, u16), ExtProps> = HashMap::new();
    for rec in recs.iter().filter(|r| r.rt == 0x087B) {
        let d = rec.joined();
        if d.len() < 30 || d[12..16] != [0, 0, 0, 0] || d[25] == 0 {
            continue;
        }
        let key = (
            u16::from_le_bytes([d[16], d[17]]),
            u16::from_le_bytes([d[18], d[19]]),
        );
        let cb = u32::from_le_bytes([d[26], d[27], d[28], d[29]]) as usize;
        let Some(x) = d.get(30..30 + cb) else {
            continue;
        };
        if let Some(std) = dxfn_std_len(x)
            && std < x.len()
            && xfext_len(&x[std..]) == Some(x.len() - std)
        {
            cf_ext.insert(key, xfext_props(&x[std..]));
        }
    }
    let mut cf_at: (u16, u16) = (0, 0);
    let mut art_slots = Vec::new();
    let mut art_parts = Vec::new();
    let mut depth = 0usize;
    let mut af12_entry: Option<u16> = None;
    for (k, rec) in recs.iter().enumerate() {
        let rt = rec.rt;
        // 시트 안의 차트 부분 스트림 (BOF dt 0x20 … EOF) 은 건너뛴다
        if rt == 0x0809 && k > 0 {
            depth += 1;
            continue;
        }
        if depth > 0 {
            if rt == 0x000A {
                depth -= 1;
            }
            continue;
        }
        let d = rec.joined();
        match rt {
            0x0809 => {
                let mut b = d.clone();
                if b.len() >= 2 {
                    b[0..2].copy_from_slice(&super::reader::HANCELL_BIFF_VERSION.to_le_bytes());
                }
                out.push((rt, b));
            }
            // 색인·페이지 레이아웃 보기
            0x020B | 0x00D7 | 0x088B => {}
            0x087A => {
                if let Some(b) = cf12(&d, false) {
                    out.push((rt, b));
                }
            }
            0x0207 => {
                if let Some(b) = string_to_hancom(rec) {
                    out.push((rt, b));
                }
            }
            0x01B6 => {
                if let Some(b) = txo_to_hancom(rec) {
                    out.push((rt, b));
                }
            }
            0x00EC => {
                art_slots.push(out.len());
                art_parts.push(d);
                out.push((rt, Vec::new()));
            }
            0x01B0 => {
                if d.len() >= 4 {
                    cf_at = (u16::from_le_bytes([d[2], d[3]]) >> 1, 0);
                }
                if let Some(b) = condfmt(&d, false, false) {
                    out.push((rt, b));
                }
            }
            0x0879 => {
                if let Some(b) = condfmt(&d, false, true) {
                    out.push((rt, b));
                }
            }
            0x01B1 => {
                // 규칙마다 차등 서식을 만든다 — 블록이 없는 DXFN 도 빈 서식 (한셀이 .xls 를 열 때와 같다)
                let id = dxfs.base + dxfs.out.len() as u32;
                let ext = cf_ext.get(&cf_at).map(|v| v.as_slice()).unwrap_or_default();
                if let Some(b) = cf_to_hancom(&d, id) {
                    dxfs.out.push(cf_dxf(&d[6..], ext, &dxfs.palette));
                    out.push((rt, b));
                }
                cf_at.1 += 1;
            }
            0x01BE => {
                if let Some(b) = dv(&d, false) {
                    out.push((rt, b));
                }
            }
            0x7EF0 => {
                if let Some(b) = nexcel_chart(&d) {
                    out.push((rt, b));
                }
            }
            0x01B8 => {
                let d = nexcel_hlink(&d).unwrap_or(d);
                if let Some(b) = conv(rt, &d, false) {
                    out.push((rt, b));
                }
                // 표시 이름 (플래그 0x10): 한셀은 .xls 를 열 때 이것을 도구 설명으로 읽는다 (한셀의 .xls 저장은 도구
                // 설명을 이 자리에 쓴다) — 뒤에 HLINKTOOLTIP 이 없으면 레거시 리더가 읽는 0x0800 으로 붙인다
                let flags = d
                    .get(28..32)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                let next = recs.get(k + 1).map(|r| r.rt);
                if let Some(f) = flags
                    && f & 0x10 != 0
                    && next != Some(0x0800)
                    && let Some(n) = d
                        .get(32..36)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    && let Some(text) = d.get(36..36 + n as usize * 2)
                {
                    let mut t = vec![0x00, 0x08];
                    t.extend_from_slice(&d[..8]);
                    for c in text.chunks(2) {
                        if c == [0, 0] {
                            break;
                        }
                        t.extend_from_slice(c);
                    }
                    t.extend_from_slice(&[0, 0]);
                    if t.len() > 12 {
                        out.push((0x0800, t));
                    }
                }
            }
            0x087F => {
                // ContinueFrt12 의 글 조건 (값 목록 필터의 값) → 레거시는 0x009F 로 (iEntry, 종류 0, 글)
                if let Some(b) = conv(rt, &d, false) {
                    if b.len() >= 25 && b[12] == 0x06 && af12_entry.is_some() {
                        let cch = b[14] as usize;
                        let mut v = af12_entry.unwrap_or(0).to_le_bytes().to_vec();
                        v.push(0);
                        v.extend_from_slice(&(cch as u16).to_le_bytes());
                        v.push(0x01);
                        v.extend_from_slice(b.get(23..23 + cch * 2).unwrap_or_default());
                        out.push((0x009F, v));
                    }
                    out.push((rt, b));
                }
            }
            0x087E => {
                // AUTOFILTER12: 동적 필터(cft ≠ 0)가 아니면 뒤따르는 글 조건이 값 목록이다
                af12_entry = (d.len() >= 26 && d[22..26] == [0, 0, 0, 0])
                    .then(|| u16::from_le_bytes([d[12], d[13]]));
                if let Some(b) = conv(rt, &d, false) {
                    out.push((rt, b));
                }
            }
            _ => {
                if let Some(b) = conv(rt, &d, false) {
                    out.push((if rt == 0x00D6 { 0x0204 } else { rt }, b));
                }
            }
        }
    }
    if !art_parts.is_empty() {
        for (slot, part) in art_slots.iter().zip(drawings(&art_parts, false)) {
            out[*slot].1 = part;
        }
    }
    out
}

/// BIFF8 `/Workbook` 스트림 → 레거시 `/Workbook` 스트림 (`.xls` 읽기)
pub fn to_hancom(stream: &[u8]) -> Result<Vec<u8>> {
    if !is_biff8(stream) {
        return Err(bad("BIFF8 BOF 가 아닙니다"));
    }
    let recs = split_biff8(stream);
    // 전역과 시트 부분: BOUNDSHEET 위치로 시트 BOF 를 찾는다
    let mut offsets = Vec::new();
    {
        let mut pos = 0usize;
        let mut starts = Vec::new();
        for r in &recs {
            starts.push(pos);
            pos += 4 + r.data.len() + r.conts.iter().map(|c| 4 + c.len()).sum::<usize>();
        }
        offsets.extend(starts);
    }
    let gend = recs
        .iter()
        .position(|r| r.rt == 0x000A)
        .ok_or_else(|| bad("전역 EOF 가 없습니다"))?;
    let globals = &recs[..=gend];
    let sheet_offs: Vec<u32> = globals
        .iter()
        .filter(|r| r.rt == 0x0085 && r.data.len() >= 4)
        .map(|r| u32::from_le_bytes([r.data[0], r.data[1], r.data[2], r.data[3]]))
        .collect();
    let mut palette: Vec<u32> = super::reader::DEFAULT_PALETTE.to_vec();
    for r in globals
        .iter()
        .filter(|r| r.rt == 0x0092 && r.data.len() >= 2)
    {
        let n = u16::from_le_bytes([r.data[0], r.data[1]]) as usize;
        for (i, c) in r.data[2..].chunks(4).take(n).enumerate() {
            if c.len() == 4 && i < palette.len() {
                palette[i] = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
            }
        }
    }
    let mut cf_dxfs = CfDxfs {
        base: globals.iter().filter(|r| r.rt == 0x088D).count() as u32,
        palette: palette.clone(),
        out: Vec::new(),
    };
    let mut sheets: Vec<Vec<(u16, Vec<u8>)>> = Vec::new();
    for off in &sheet_offs {
        let Some(start) = offsets.iter().position(|&o| o == *off as usize) else {
            sheets.push(vec![
                (0x0809, vec![0x01, 0x10, 0x10, 0x00]),
                (0x000A, Vec::new()),
            ]);
            continue;
        };
        // 그 BOF 의 EOF 까지 (안쪽 차트 BOF·EOF 를 센다)
        let mut depth = 0usize;
        let mut end = start;
        for (k, r) in recs.iter().enumerate().skip(start) {
            if r.rt == 0x0809 {
                depth += 1;
            } else if r.rt == 0x000A {
                depth -= 1;
                if depth == 0 {
                    end = k;
                    break;
                }
            }
            end = k;
        }
        sheets.push(sheet_to_hancom(&recs[start..=end], &mut cf_dxfs));
    }
    // 전역: XFEXT 를 먼저 모은다 (XF 의 색)
    let mut xfext: HashMap<u16, Vec<(u16, Vec<u8>)>> = HashMap::new();
    for r in globals {
        match r.rt {
            0x087D if r.data.len() >= 20 => {
                let d = &r.data;
                let ixfe = u16::from_le_bytes([d[14], d[15]]);
                let n = u16::from_le_bytes([d[18], d[19]]) as usize;
                let mut p = 20;
                let mut props = Vec::new();
                for _ in 0..n {
                    let (Some(ty), Some(cb)) = (
                        d.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]])),
                        d.get(p + 2..p + 4)
                            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize),
                    ) else {
                        break;
                    };
                    if cb < 4 {
                        break;
                    }
                    if let Some(v) = d.get(p + 4..p + cb) {
                        props.push((ty, v.to_vec()));
                    }
                    p += cb;
                }
                xfext.insert(ixfe, props);
            }
            _ => {}
        }
    }
    // 글꼴: BIFF8 FONT 에 한셀이 .xls 를 열 때처럼 한컴 확장을 붙인다 (밑줄·취소선 모양, 색). XFEXT 의 글자 색
    // (종류 0x0D) 이 글꼴의 팔레트 색과 다르면 그 색의 글꼴을 따로 만들어 XF 가 가리키게 한다
    let fonts: Vec<Vec<u8>> = globals
        .iter()
        .filter(|r| r.rt == 0x0031)
        .map(|r| r.joined())
        .collect();
    let font_color = |d: &[u8]| -> u32 {
        let icv = u16::from_le_bytes([d[4], d[5]]);
        match icv {
            8..=63 => {
                let rgb = palette[icv as usize - 8];
                ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
            }
            _ => 0xDFFF_FFFF,
        }
    };
    // 글꼴 번호 ↔ FONT 레코드 차례 (번호 4 는 비어 있다)
    let rec_of = |idx: u16| -> Option<usize> {
        match idx {
            0..=3 => Some(idx as usize),
            4 => None,
            n => Some(n as usize - 1),
        }
    };
    let idx_of = |k: usize| -> u16 { if k < 4 { k as u16 } else { k as u16 + 1 } };
    let mut extra_fonts: Vec<(usize, u32)> = Vec::new();
    let mut xf_font: HashMap<u16, u16> = HashMap::new();
    {
        for (i, r) in globals.iter().filter(|r| r.rt == 0x00E0).enumerate() {
            let i = i as u16;
            let d = &r.data;
            if d.len() >= 2 {
                let ifnt = u16::from_le_bytes([d[0], d[1]]);
                let want = xfext.get(&i).and_then(|props| {
                    props
                        .iter()
                        .find(|(t, _)| *t == 0x0D)
                        .and_then(|(_, v)| xfext_colorref(v, &palette))
                });
                if let (Some(c), Some(k)) = (want, rec_of(ifnt))
                    && let Some(f) = fonts.get(k)
                    && f.len() >= 6
                    && font_color(f) != c
                {
                    let n = match extra_fonts.iter().position(|e| *e == (k, c)) {
                        Some(n) => n,
                        None => {
                            extra_fonts.push((k, c));
                            extra_fonts.len() - 1
                        }
                    };
                    xf_font.insert(i, idx_of(fonts.len() + n));
                }
            }
        }
    }
    // 문단 모양: 한셀은 .xls 를 열 때 가로 맞춤이 일반이 아닌 셀 서식에 기본 문단 모양(줄 간격 75 betweenLines,
    // 영문 keepWord·한글 breakWord, 기준선)을 붙이고 들여쓰기는 왼쪽(오른쪽 맞춤이면 오른쪽) 여백 = 단계 × 1134 로
    // 둔다 (한셀이 .xls 61개를 연 결과). 0번은 기본 문단 모양
    let mut paras: Vec<(i32, i32, u8)> = vec![(0, 0, 1)];
    let mut links: Vec<u32> = Vec::new();
    let mut xf_para: HashMap<u16, u32> = HashMap::new();
    // 넥셀 .nxl (한셀 -2003): BIFF8 틀에 한컴 XF 확장(54바이트 이상)과 문단 모양 레코드를 그대로 담는다 — XF 는 그대로 둔다
    let hancom_xf = globals.iter().any(|r| r.rt == 0x00E0 && r.data.len() >= 54);
    if !hancom_xf {
        for (i, r) in globals.iter().filter(|r| r.rt == 0x00E0).enumerate() {
            let i = i as u16;
            let d = &r.data;
            if d.len() >= 10 && u16::from_le_bytes([d[4], d[5]]) & 0x0004 == 0 {
                let alc = d[6] & 0x07;
                let indent = (d[8] & 0x0F) as i32;
                if alc != 0 && !(alc == 1 && indent == 0) {
                    let margin = indent * 1134;
                    let key = match alc {
                        1 => (margin, 0, 1),
                        3 => (0, margin, 2),
                        2 => (0, 0, 3),
                        _ => (0, 0, 1),
                    };
                    let pi = match paras.iter().position(|x| *x == key) {
                        Some(k) => k,
                        None => {
                            paras.push(key);
                            paras.len() - 1
                        }
                    } as u32;
                    let li = match links.iter().position(|&x| x == pi) {
                        Some(k) => k,
                        None => {
                            links.push(pi);
                            links.len() - 1
                        }
                    } as u32;
                    xf_para.insert(i, li);
                }
            }
        }
    }
    let dxf_after = globals
        .iter()
        .rposition(|r| r.rt == 0x088D)
        .or_else(|| globals.iter().rposition(|r| r.rt == 0x0293));
    let dxf_before = globals.iter().position(|r| r.rt == 0x0085);
    let mut g: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut xf_i = 0u16;
    let mut font_i = 0usize;
    for (gi, r) in globals.iter().enumerate() {
        let rt = r.rt;
        let d = r.joined();
        if dxf_after.is_none() && dxf_before == Some(gi) {
            g.extend(cf_dxfs.out.drain(..).map(|x| (0x088D, x)));
        }
        if dxf_after.is_some_and(|k| k + 1 == gi) {
            g.extend(cf_dxfs.out.drain(..).map(|x| (0x088D, x)));
        }
        match rt {
            0x0809 => {
                let mut b = d.clone();
                b[0..2].copy_from_slice(&super::reader::HANCELL_BIFF_VERSION.to_le_bytes());
                g.push((rt, b));
            }
            0x0031 => {
                if let Some(b) = conv(rt, &d, false) {
                    let c = font_color(&d);
                    g.push((rt, font_ext(&b, c)));
                }
                font_i += 1;
                if font_i == fonts.len() {
                    for (k, c) in &extra_fonts {
                        if let Some(b) = conv(rt, &fonts[*k], false) {
                            g.push((rt, font_ext(&b, *c)));
                        }
                    }
                }
            }
            0x00E0 => {
                if xf_i == 0 && !xf_para.is_empty() {
                    // 0xA001 문단 모양 (37바이트) 과 0xA011 연결 (종류 1, 0xA001 번호)
                    for (left, right, align) in &paras {
                        let mut a = Vec::with_capacity(37);
                        a.extend_from_slice(&left.to_le_bytes());
                        a.extend_from_slice(&right.to_le_bytes());
                        a.extend_from_slice(&[0; 12]);
                        a.extend_from_slice(&75i32.to_le_bytes());
                        a.extend_from_slice(&[2, *align, 0, 1]);
                        a.extend_from_slice(&[0; 9]);
                        g.push((0xA001, a));
                    }
                    for k in &links {
                        let mut a = 1u32.to_le_bytes().to_vec();
                        a.extend_from_slice(&k.to_le_bytes());
                        g.push((0xA011, a));
                    }
                }
                let mut b = if hancom_xf {
                    d.clone()
                } else {
                    xf_to_hancom(&d, xfext.get(&xf_i), &palette, xf_para.get(&xf_i).copied())
                };
                if let Some(f) = xf_font.get(&xf_i) {
                    b[0..2].copy_from_slice(&f.to_le_bytes());
                }
                g.push((rt, b));
                xf_i += 1;
            }
            0x00FC => {
                if let Some(b) = sst_to_hancom(r) {
                    g.push((rt, b));
                }
            }
            0x00FF | 0x087C | 0x020B => {}
            0x0894 => {
                let mut cv = Cv::new(&d, false);
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
                    g.push((rt, cv.done()));
                }
            }
            _ => {
                if let Some(b) = conv(rt, &d, false) {
                    g.push((rt, b));
                }
            }
        }
    }
    // 배치 (레거시 틀: 길이 0x8000 이상은 확장 길이) — 시트 위치를 BOUNDSHEET 에
    let len_h = |d: &[u8]| 4 + d.len() + if d.len() >= 0x8000 { 2 } else { 0 };
    let g_len: usize = g.iter().map(|(_, d)| len_h(d)).sum();
    let mut sheet_pos = Vec::new();
    let mut pos = g_len;
    for s in &sheets {
        sheet_pos.push(pos);
        pos += s.iter().map(|(_, d)| len_h(d)).sum::<usize>();
    }
    let mut out = Vec::with_capacity(pos);
    let mut si = 0;
    for (rt, d) in &g {
        if *rt == 0x0085 && d.len() >= 4 {
            let mut b = d.clone();
            let p = sheet_pos.get(si).copied().unwrap_or(0) as u32;
            b[0..4].copy_from_slice(&p.to_le_bytes());
            si += 1;
            put_h(&mut out, *rt, &b);
        } else {
            put_h(&mut out, *rt, d);
        }
    }
    for s in sheets {
        for (rt, d) in s {
            put_h(&mut out, rt, &d);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheetext_tab_color() {
        // 한컴·넥셀: 20바이트 + COLORREF (00b050)
        let mut h = vec![0x62, 0x08];
        h.extend_from_slice(&[0; 10]);
        h.extend_from_slice(&[0x14, 0, 0, 0, 0x39, 0, 0, 0, 0x00, 0xB0, 0x50, 0x00]);
        assert_eq!(conv(0x0862, &h, true).unwrap(), h[..20].to_vec());
        assert_eq!(conv(0x0862, &h, false).unwrap(), h);
        // BIFF8 선택 부분 (cb 0x28, CFColor 종류 2 RGBA) → COLORREF
        let mut b = h[..12].to_vec();
        b.extend_from_slice(&[0x28, 0, 0, 0, 0x39, 0, 0, 0]);
        b.extend_from_slice(&[0x39, 0, 0, 0, 2, 0, 0, 0, 0x00, 0xB0, 0x50, 0xFF]);
        b.extend_from_slice(&[0; 8]);
        assert_eq!(conv(0x0862, &b, false).unwrap(), h);
    }

    #[test]
    fn xfext_search() {
        // DXFN 표준 6바이트(블록 없음) + 한컴 확장 4바이트 + XFExtNoFRT (글자 색 0x0D)
        let mut d = vec![0xFF, 0xFF, 0x1F, 0x80, 0x00, 0x03];
        d.extend_from_slice(&[1, 2, 3, 4]);
        let x = [
            0, 0, 0xFF, 0xFF, 0, 0, 1, 0, 0x0D, 0, 20, 0, 2, 0, 0, 0, 0x11, 0x22, 0x33, 0xFF, 0, 0,
            0, 0, 0, 0, 0, 0,
        ];
        d.extend_from_slice(&x);
        let o = dxfn12_to8(&d).unwrap();
        assert_eq!(&o[..6], &[0xFF, 0xFF, 0x1F, 0x80, 0x00, 0x00]);
        assert_eq!(&o[6..], &x);
        assert_eq!(xfext_props(&x), vec![(0x0D, x[12..28].to_vec())]);
    }

    #[test]
    fn nexcel_cf_blocks() {
        // 넥셀 CF 글꼴: 아무것도 정하지 않은 BIFF8 블록 → 기본 (굵기 400, 색 번호 0x40, RGB 없음, 취소선 아님 0x82)
        let mut f = [0u8; 118];
        f[64..68].copy_from_slice(&[0xFF; 4]);
        f[72..74].copy_from_slice(&400u16.to_le_bytes());
        f[80..84].copy_from_slice(&[0xFF; 4]);
        let a = nexcel_cf_font(&f, None);
        assert_eq!(
            (a[71], a[72], a[73], a[80], a[88], a[96]),
            (0x08, 0x90, 0x01, 0x40, 0x82, 1)
        );
        assert_eq!(&a[84..88], &[0xFF; 4]);
        assert_eq!(&a[64..68], &[0; 4]);
        // 한 줄 밑줄 + 색: 플래그 |= 1|4, 밑줄 종류 1, 밑줄 모양 1, @96 은 0, RGB 는 그대로
        f[76] = 1;
        let a = nexcel_cf_font(&f, Some([0x2C, 0x2D, 0x63, 0]));
        assert_eq!((a[74], a[76], a[77], a[78], a[96]), (5, 1, 1, 1, 0));
        assert_eq!(&a[84..88], &[0x2C, 0x2D, 0x63, 0]);
        // 두 줄은 모양 8, 취소선은 @88 = 0x02
        f[76] = 2;
        f[68] = 0x80;
        let a = nexcel_cf_font(&f, None);
        assert_eq!((a[78], a[88], a[68]), (8, 0x02, 0));
        // 테두리: 자동(0x40)은 왼쪽·오른쪽·아래가 검정(8), 위는 그대로
        let a = 0x0001u32 | (0x0A << 16) | (0x40 << 23);
        let c = 0x40u32 | (0x40 << 7) | (0x40 << 14);
        let mut b = a.to_le_bytes().to_vec();
        b.extend_from_slice(&c.to_le_bytes());
        assert_eq!(
            nexcel_cf_border(&b),
            [0x01, 0x00, 0x0A, 0x04, 0x40, 0x04, 0x00, 0x00]
        );
    }
}
