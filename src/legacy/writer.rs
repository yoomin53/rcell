// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! [`Workbook`] → 레거시 `.cell` (`/Workbook` 레코드 스트림) — [`super::reader`] 의 반대.
//!
//! 한셀 2024 가 `SaveAs(…, -2005)` 로 저장하는 꼴을 따른다: 레코드 순서, 고정 값, BIFF 상용구(글꼴 다섯,
//! 스타일 XF 0–14·16–20, 표시 형식 12개), 한컴 확장(글꼴 78바이트, XF COLORREF, 문단 모양 표). 근거는
//! 말뭉치의 `X.cell` 과 한셀이 같은 문서를 레거시로 저장한 `X.legacy.cell` 의 대조다
//! (`docs/format/cell-legacy.md` 10절).

use std::collections::HashMap;

use super::encode::{self, Class, EncodeError, Resolve};
use super::reader::{BORDER_STYLES, DEFAULT_PALETTE, H_ALIGN, PATTERNS, V_ALIGN};
use crate::error::{Error, Result};
use crate::model::{
    Border, BorderEdge, Cell, CellRange, CellRef, CellValue, Color, DefinedName, Font, FormulaKind,
    ParaShape, RichText, Sheet, SheetKind, SheetState, Workbook, Xf,
};
use crate::vocab;

// ---------------------------------------------------------------------------
// 바이트 쓰기
// ---------------------------------------------------------------------------

/// 레코드 내용 만들기
#[derive(Default)]
pub(crate) struct Buf(pub Vec<u8>);

impl Buf {
    pub fn new() -> Self {
        Buf(Vec::new())
    }
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    /// UTF-16LE 글자 (길이 없이)
    pub fn utf16(&mut self, s: &str) -> &mut Self {
        for u in s.encode_utf16() {
            self.u16(u);
        }
        self
    }
    /// `cch u16, flag u8 (0), UTF-16` — 한셀 레거시 문자열 (플래그와 상관없이 UTF-16)
    pub fn str16(&mut self, s: &str) -> &mut Self {
        self.u16(units(s) as u16).u8(0).utf16(s)
    }
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

pub(crate) fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 레코드 스트림. 한셀은 긴 레코드를 CONTINUE 로 나누지 않고 확장 길이를 쓴다 (3절)
pub(crate) struct Out {
    pub buf: Vec<u8>,
}

impl Out {
    pub fn rec(&mut self, rt: u16, data: &[u8]) {
        self.buf.extend_from_slice(&rt.to_le_bytes());
        let n = data.len();
        if n < 0x8000 {
            self.buf.extend_from_slice(&(n as u16).to_le_bytes());
        } else {
            self.buf
                .extend_from_slice(&(((n & 0x7FFF) | 0x8000) as u16).to_le_bytes());
            self.buf
                .extend_from_slice(&((n >> 15) as u16).to_le_bytes());
        }
        self.buf.extend_from_slice(data);
    }
    pub fn rec_buf(&mut self, rt: u16, b: &mut Buf) {
        let d = b.take();
        self.rec(rt, &d);
    }
}

/// FRT 머리 12바이트: `rt u16, grbitFrt u16, 예약 8`. 한셀은 예약 칸에 초기화되지 않은 값을 쓴다 (관측) — rcell 은 0
fn frt(rt: u16) -> Buf {
    let mut b = Buf::new();
    b.u16(rt).u16(0).bytes(&[0; 8]);
    b
}

// ---------------------------------------------------------------------------
// 색
// ---------------------------------------------------------------------------

/// 모델 색 → 0xRRGGBB. 자동·없음은 None
pub(crate) fn rgb_of(c: &Color, wb: &Workbook) -> Option<u32> {
    if c.auto {
        return None;
    }
    if let Some(h) = &c.rgb {
        let h = h.trim_start_matches('#');
        let h = if h.len() == 8 { &h[2..] } else { h };
        return u32::from_str_radix(h, 16).ok();
    }
    if let Some(i) = c.indexed {
        return match i {
            0..=7 => DEFAULT_PALETTE.get(i as usize).copied(),
            8..=63 => DEFAULT_PALETTE.get(i as usize - 8).copied(),
            _ => None,
        };
    }
    if let Some(t) = c.theme {
        let theme = wb.theme.as_ref();
        let base = theme
            .and_then(|th| th.colors.get(t as usize).cloned())
            .and_then(|h| u32::from_str_radix(h.trim_start_matches('#'), 16).ok())?;
        return Some(match c.tint {
            Some(tint) if tint != 0.0 => apply_tint(base, tint),
            _ => base,
        });
    }
    None
}

/// 밝기(tint) 적용 — ECMA-376 의 HLS 규칙
fn apply_tint(rgb: u32, tint: f64) -> u32 {
    let (r, g, b) = (
        ((rgb >> 16) & 0xFF) as f64 / 255.0,
        ((rgb >> 8) & 0xFF) as f64 / 255.0,
        (rgb & 0xFF) as f64 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let mut l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    l = if tint < 0.0 {
        l * (1.0 + tint)
    } else {
        l * (1.0 - tint) + tint
    };
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;
        (
            hue(p, q, h + 1.0 / 3.0),
            hue(p, q, h),
            hue(p, q, h - 1.0 / 3.0),
        )
    };
    let c = |v: f64| ((v * 255.0).round().clamp(0.0, 255.0)) as u32;
    (c(r) << 16) | (c(g) << 8) | c(b)
}

/// 0xRRGGBB → COLORREF 0x00BBGGRR
pub(crate) fn colorref_of(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// 기본 팔레트(8–63)에서 가장 가까운 색 번호 (RGB 유클리드 거리, 같으면 앞 번호 — 관측)
pub(crate) fn nearest_icv(rgb: u32) -> u16 {
    let (r, g, b) = (
        (rgb >> 16) as i32 & 0xFF,
        (rgb >> 8) as i32 & 0xFF,
        rgb as i32 & 0xFF,
    );
    let mut best = (i32::MAX, 8u16);
    for (i, p) in DEFAULT_PALETTE.iter().enumerate() {
        let (pr, pg, pb) = (
            (p >> 16) as i32 & 0xFF,
            (p >> 8) as i32 & 0xFF,
            *p as i32 & 0xFF,
        );
        let d = (r - pr).pow(2) + (g - pg).pow(2) + (b - pb).pow(2);
        if d < best.0 {
            best = (d, i as u16 + 8);
        }
    }
    best.1
}

/// 팔레트 번호: 없으면 `none`, 자동 64
fn icv_of(c: Option<&Color>, wb: &Workbook, none: u16) -> u16 {
    let Some(c) = c else { return none };
    if c.auto || c.indexed == Some(64) {
        return 0x40;
    }
    if let Some(i) = c.indexed.filter(|&i| i < 64) {
        return if i < 8 {
            nearest_icv(DEFAULT_PALETTE[i as usize])
        } else {
            i as u16
        };
    }
    match rgb_of(c, wb) {
        Some(rgb) => nearest_icv(rgb),
        None => none,
    }
}

/// 한컴 확장의 COLORREF: 없으면 `none`, 자동이면 `auto`
fn cref_of(c: Option<&Color>, wb: &Workbook, none: u32, auto: u32) -> u32 {
    let Some(c) = c else { return none };
    if c.auto || c.indexed == Some(64) {
        return auto;
    }
    match rgb_of(c, wb) {
        Some(rgb) => colorref_of(rgb),
        None => none,
    }
}

// ---------------------------------------------------------------------------
// 날짜
// ---------------------------------------------------------------------------

/// W3CDTF (`2026-10-19T07:23:02.367`, `…Z`) → FILETIME
pub(crate) fn filetime(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T').unwrap_or((s, "00:00:00"));
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (
        t.next()??,
        t.next().flatten().unwrap_or(0),
        t.next().flatten().unwrap_or(0),
    );
    // 그레고리력 → 1970-01-01 부터의 날 수 (Howard Hinnant 의 days_from_civil)
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hh * 3600 + mm * 60 + ss + 11_644_473_600;
    let mut frac_100ns: i64 = 0;
    let digits: String = frac
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .take(7)
        .collect();
    if !digits.is_empty() {
        frac_100ns = digits.parse::<i64>().ok()? * 10i64.pow(7 - digits.len() as u32);
    }
    u64::try_from(secs * 10_000_000 + frac_100ns).ok()
}

// ---------------------------------------------------------------------------
// 스타일 배치
// ---------------------------------------------------------------------------

/// 레거시 스타일 XF 배치: 0 표준, 1–14 개요 상용구, 15 셀 서식 0, 16–20 내장 스타일 상용구(백분율·쉼표·쉼표[0]·
/// 통화·통화[0]), 21부터 그 밖의 스타일, 그다음 나머지 셀 서식.
///
/// 레거시에서 읽은 서식 표가 아니면(XML 등) 한셀이 XML 을 열 때처럼 21번에 셀 서식 0 의 부모 스타일 사본(이름 없음)을
/// 두고 셀 서식 0(XF 15)의 부모로 삼는다 (관측: 말뭉치 XML 117개를 한셀로 열어 레거시로 저장하면 모두 이렇다)
struct StyleLayout {
    /// 모델 cell_style_xfs 번호 → 레거시 XF 번호
    style: Vec<Option<u16>>,
    /// 모델 cell_xfs 번호 → 레거시 XF 번호
    cell: Vec<u16>,
    /// 레거시 XF 순서대로: (모델 스타일 XF 인가, 모델 번호) — None = 상용구
    order: Vec<XfSlot>,
    /// 셀 서식 0 의 부모를 바꾼다 (스타일 사본의 XF 번호)
    cell0_parent: Option<u16>,
}

#[derive(Clone, Copy)]
enum XfSlot {
    /// 상용구 스타일 XF: 표준 XF 의 복사 + used 바이트, 표시 형식 번호
    Boiler {
        used: u8,
        fmt: u16,
    },
    Style(usize),
    /// 스타일의 이름 없는 사본 (한셀이 XML 을 열 때 만드는 XF 21)
    StyleCopy(usize),
    Cell(usize),
}

/// 내장 스타일 번호 → 상용구 자리 (16–20)
fn builtin_slot(id: u32) -> Option<u16> {
    Some(match id {
        5 => 16,
        3 => 17,
        6 => 18,
        4 => 19,
        7 => 20,
        _ => return None,
    })
}

fn layout_styles(wb: &Workbook) -> StyleLayout {
    let st = &wb.styles;
    let nstyle = st.cell_style_xfs.len();
    let mut style: Vec<Option<u16>> = vec![None; nstyle];
    let normal = st
        .cell_styles
        .iter()
        .find(|c| c.builtin_id == Some(0))
        .map_or(0, |c| c.xf_id as usize);
    if normal < nstyle {
        style[normal] = Some(0);
    }
    for c in &st.cell_styles {
        let i = c.xf_id as usize;
        if i >= nstyle || style[i].is_some() {
            continue;
        }
        match c.builtin_id {
            Some(b) if builtin_slot(b).is_some() => style[i] = builtin_slot(b),
            Some(1) => style[i] = Some(1 + 2 * c.i_level.unwrap_or(0).min(6) as u16),
            Some(2) => style[i] = Some(2 + 2 * c.i_level.unwrap_or(0).min(6) as u16),
            _ => {}
        }
    }
    // 이름이 있거나 셀 서식이 부모로 쓰는 스타일만 21부터 (이름도 쓰임도 없는 것은 레거시 상용구를 읽은 것)
    let named: std::collections::HashSet<usize> =
        st.cell_styles.iter().map(|c| c.xf_id as usize).collect();
    let used: std::collections::HashSet<usize> = st
        .cell_xfs
        .iter()
        .filter_map(|x| x.xf_id.map(|v| v as usize))
        .collect();
    let mut order: Vec<XfSlot> = Vec::new();
    // 0–14
    order.push(match style.iter().position(|s| *s == Some(0)) {
        Some(i) => XfSlot::Style(i),
        None => XfSlot::Boiler { used: 0, fmt: 0 },
    });
    for slot in 1..15u16 {
        order.push(match style.iter().position(|s| *s == Some(slot)) {
            Some(i) => XfSlot::Style(i),
            None => XfSlot::Boiler { used: 0xF4, fmt: 0 },
        });
    }
    order.push(XfSlot::Cell(0));
    for (slot, fmt) in [(16u16, 9u16), (17, 43), (18, 41), (19, 44), (20, 42)] {
        order.push(match style.iter().position(|s| *s == Some(slot)) {
            Some(i) => XfSlot::Style(i),
            None => XfSlot::Boiler { used: 0xF8, fmt },
        });
    }
    let mut next = 21u16;
    let mut cell0_parent = None;
    // 셀 서식 0 을 쓰는 셀이 없고 사용자 표 스타일이 있으면 한셀은 사본을 쓰지 않고 XF 15 의 부모를 XF 표 끝 다음
    // 번호로 남긴다 (관측: 말뭉치 XML 117개 중 13개, 모두 이 조건 — 117/117 이 규칙과 맞는다)
    let mut dangling = false;
    if !st.from_legacy {
        let p = st.cell_xfs.first().and_then(|x| x.xf_id).unwrap_or(0) as usize;
        let uses0 = wb
            .sheets
            .iter()
            .any(|s| s.cells.iter().any(|c| c.style == 0));
        let custom_ts = st
            .table_styles
            .as_ref()
            .is_some_and(|t| !t.styles.is_empty());
        if !uses0 && custom_ts {
            dangling = true;
        } else if p < nstyle {
            order.push(XfSlot::StyleCopy(p));
            cell0_parent = Some(next);
            next += 1;
        }
    }
    for (i, slot) in style.iter_mut().enumerate().take(nstyle) {
        if slot.is_none() && (named.contains(&i) || used.contains(&i)) {
            *slot = Some(next);
            order.push(XfSlot::Style(i));
            next += 1;
        }
    }
    let mut cell = vec![15u16; st.cell_xfs.len().max(1)];
    for (i, slot) in cell.iter_mut().enumerate().skip(1) {
        *slot = next;
        order.push(XfSlot::Cell(i));
        next += 1;
    }
    if dangling {
        cell0_parent = Some(order.len() as u16);
    }
    StyleLayout {
        style,
        cell,
        order,
        cell0_parent,
    }
}

/// 글꼴 배치: 레거시 글꼴 번호 4 는 비워 둔다 (BIFF 규칙). 모델의 4번이 0번의 사본이고 아무도 쓰지 않으면
/// 한셀이 쓴 빈자리로 보고 건너뛴다, 아니면 4번부터 하나씩 민다.
struct FontLayout {
    /// 모델 번호 → 레거시 번호
    map: Vec<u16>,
    /// 레거시 순서대로 쓸 모델 번호 (4번 자리 제외)
    records: Vec<usize>,
}

fn same_font(a: &Font, b: &Font) -> bool {
    format!("{a:?}") == format!("{b:?}")
}

fn layout_fonts(wb: &Workbook) -> FontLayout {
    let fonts = &wb.styles.fonts;
    let n = fonts.len();
    let uses4 = wb.styles.cell_xfs.iter().any(|x| x.font_id == 4)
        || wb.styles.cell_style_xfs.iter().any(|x| x.font_id == 4);
    let phantom = n > 4 && same_font(&fonts[4], &fonts[0]) && !uses4;
    let mut map = Vec::with_capacity(n);
    let mut records = Vec::new();
    for i in 0..n {
        if phantom {
            map.push(i as u16);
            if i != 4 {
                records.push(i);
            }
        } else {
            map.push(if i < 4 { i as u16 } else { i as u16 + 1 });
            records.push(i);
        }
    }
    FontLayout { map, records }
}

// ---------------------------------------------------------------------------
// 글꼴
// ---------------------------------------------------------------------------

fn font_record(f: &Font, wb: &Workbook, legacy_index: usize) -> Vec<u8> {
    let mut b = Buf::new();
    let size = f.size.unwrap_or(11.0);
    let h = f.hancom.as_ref();
    let mut grbit = 0u16;
    if f.italic {
        grbit |= 0x02;
    }
    if f.strike {
        grbit |= 0x08;
    }
    // 외곽선: ECMA 외곽선은 실선 종류일 때만 grbit 0x10 (글꼴 대조 실험)
    let outline_type = h
        .and_then(|h| h.outline_type.as_deref())
        .and_then(|v| vocab::ordinal(vocab::OUTLINE_TYPE, v))
        .unwrap_or(if f.outline { 1 } else { 0 });
    if f.outline && outline_type <= 1 {
        grbit |= 0x10;
    }
    if f.shadow {
        grbit |= 0x20;
    }
    if f.condense {
        grbit |= 0x40;
    }
    if f.extend {
        grbit |= 0x80;
    }
    // 처음 다섯 레코드(BIFF 기본 글꼴)는 색 번호 0x7FFF 에 한컴 확장이 없다 (관측)
    let boiler = legacy_index < 5;
    let icv = if boiler {
        0x7FFF
    } else {
        icv_of(f.color.as_ref(), wb, 0x7FFF)
    };
    let sss = match f.vert_align.as_deref() {
        Some("superscript") => 1,
        Some("subscript") => 2,
        _ => 0,
    };
    let uls: u8 = match f.underline.as_deref() {
        None => 0,
        Some("double") => 2,
        Some("singleAccounting") => 0x21,
        Some("doubleAccounting") => 0x22,
        Some("none") => 0,
        Some(_) => 1,
    };
    let name = f.name.as_deref().unwrap_or("맑은 고딕");
    b.u16((size * 20.0).round() as u16)
        .u16(grbit)
        .u16(icv)
        .u16(if f.bold { 700 } else { 400 })
        .u16(sss)
        .u8(uls)
        .u8(f.family.unwrap_or(0) as u8)
        .u8(f.charset.unwrap_or(1) as u8)
        .u8(0)
        .u8(units(name) as u8)
        .u8(0)
        .utf16(name);
    if boiler {
        b.u8(0);
        return b.take();
    }
    // 한컴 확장 78바이트 (5.3절)
    b.u8(1);
    let text = cref_of(f.color.as_ref(), wb, 0, 0);
    let pick = |v: &[&str], s: Option<&String>| -> u32 {
        s.and_then(|s| vocab::ordinal(v, s)).unwrap_or(0) as u32
    };
    let mut flags = 0x0008u16;
    if h.and_then(|h| h.emboss) == Some(true) {
        flags |= 0x01;
    }
    if h.and_then(|h| h.engrave) == Some(true) {
        flags |= 0x02;
    }
    if h.and_then(|h| h.use_kerning) == Some(true) {
        flags |= 0x10;
    }
    let ul_type: u16 = match h.and_then(|h| h.underline_type.as_deref()) {
        Some(t) => t.parse().unwrap_or(0),
        None if uls != 0 => 1,
        None => 0,
    };
    let ul_shape = match h.and_then(|h| h.underline_shape.as_ref()) {
        Some(s) => pick(vocab::LINE_SHAPE, Some(s)),
        None if uls == 2 || uls == 0x22 => 7,
        None => 0,
    };
    let so_type: u32 = match h.and_then(|h| h.strikeout_type.as_deref()) {
        Some(t) => t.parse().unwrap_or(0),
        None if f.strike => 1,
        None => 0,
    };
    let shadow = pick(vocab::SHADOW_TYPE, h.and_then(|h| h.shadow_type.as_ref()));
    b.u32((size * 100.0).round() as u32)
        .u16(flags)
        .u16(ul_type)
        .u16(0)
        .u32(ul_shape)
        .u32(outline_type as u32)
        .u32(shadow)
        .i32(h.and_then(|h| h.shadow_offset_x).unwrap_or(10))
        .i32(h.and_then(|h| h.shadow_offset_y).unwrap_or(10))
        .u32(so_type)
        .u32(pick(
            vocab::DIACSYM_MARK,
            h.and_then(|h| h.diacsym_mark.as_ref()),
        ))
        .u32(pick(
            vocab::LINE_SHAPE,
            h.and_then(|h| h.strikeout_shape.as_ref()),
        ))
        .u32(text)
        // 밑줄·취소선 색: 지정이 없으면 밑줄(취소선)이 있을 때 글자 색, 없으면 0 (관측: 서식 파일 글꼴 390개)
        .u32(cref_of(
            h.and_then(|h| h.underline_color.as_ref()),
            wb,
            if uls != 0 || ul_type != 0 { text } else { 0 },
            text,
        ))
        .u32(cref_of(
            h.and_then(|h| h.shade_color.as_ref()),
            wb,
            0xFFFF_FFFF,
            0xFFFF_FFFF,
        ))
        .u32(cref_of(
            h.and_then(|h| h.shadow_color.as_ref()),
            wb,
            0x00C0_C0C0,
            0x00C0_C0C0,
        ))
        .i32(h.and_then(|h| h.size).unwrap_or(100))
        .i32(h.and_then(|h| h.ratio).unwrap_or(100))
        .i32(h.and_then(|h| h.spacing).unwrap_or(0))
        .i32(h.and_then(|h| h.offset).unwrap_or(0))
        .u32(cref_of(
            h.and_then(|h| h.strikeout_color.as_ref()),
            wb,
            if so_type != 0 { text } else { 0 },
            text,
        ));
    b.take()
}

// ---------------------------------------------------------------------------
// 문단 모양 (0xA001 / 0xA011)
// ---------------------------------------------------------------------------

/// 문단 모양 레코드. `align` 은 XF 의 가로 정렬에서 온다 (한글 문단 정렬 번호)
fn parashape_record(p: &ParaShape, align: u8) -> Vec<u8> {
    let mut b = Buf::new();
    let lst = p
        .line_spacing_type
        .as_deref()
        .and_then(|v| vocab::ordinal(vocab::LINE_SPACING_TYPE, v))
        .unwrap_or(2)
        .min(2) as u8;
    let num = p.abstract_num_type.unwrap_or(0) as u8;
    b.i32(p.left_margin.unwrap_or(0))
        .i32(p.right_margin.unwrap_or(0))
        .i32(p.indentation.unwrap_or(0))
        .i32(p.prev_spacing.unwrap_or(0))
        .i32(p.next_spacing.unwrap_or(0))
        .i32(p.line_spacing.unwrap_or(75))
        .u8(lst)
        .u8(align)
        .u8(match p.break_latin_word.as_deref() {
            Some("breakWord") => 2,
            _ => 0,
        })
        .u8(match p.break_non_latin_word.as_deref() {
            Some("keepWord") => 0,
            _ => 1,
        })
        .i32(p.condense.unwrap_or(0))
        .u16(
            p.text_alignment
                .as_deref()
                .and_then(|v| vocab::ordinal(vocab::TEXT_ALIGNMENT, v))
                .unwrap_or(0) as u16,
        )
        .u8(0)
        .u8(num)
        .u8(p.font_line_height.unwrap_or(false) as u8);
    // 문단 번호 ID 는 번호(2)·글머리표(3)에만 있고 0 으로 저장된다 (8절). 종류 1 은 37바이트 그대로 (관측: craft/dxfprobe)
    if num >= 2 {
        b.i32(0);
    }
    b.take()
}

/// 가로 정렬 → 한글 문단 정렬 (1 왼쪽, 2 오른쪽, 3 가운데, 0 양쪽, 4 배분)
fn hangul_align(xf: &Xf) -> u8 {
    match xf.alignment.as_ref().and_then(|a| a.horizontal.as_deref()) {
        Some("center") | Some("centerContinuous") => 3,
        Some("right") => 2,
        Some("justify") => 0,
        Some("distributed") => 4,
        _ => 1,
    }
}

// ---------------------------------------------------------------------------
// XF
// ---------------------------------------------------------------------------

struct XfCtx<'a> {
    wb: &'a Workbook,
    fonts: &'a FontLayout,
}

fn edge_dg(e: Option<&BorderEdge>) -> u32 {
    e.and_then(|e| e.style.as_deref())
        .and_then(|s| BORDER_STYLES.iter().position(|x| *x == s))
        .unwrap_or(0) as u32
}

/// XF 레코드 하나를 쓰는 조건
struct XfOpts {
    /// 스타일 XF 인가
    style: bool,
    /// 셀 서식의 부모 스타일 XF 번호
    parent: u16,
    /// 0xA011 번호 (문단 모양이 있으면)
    para_ref: Option<u32>,
    /// XFEXT 가 따로 있다 (fHasXFExt)
    has_ext: bool,
    /// 적용 비트에 더할 것 (한셀이 XML 을 열 때 켜는 비트, [`implied_used`])
    extra_used: u8,
    /// 선 없는 가장자리의 색을 쓰지 않는다 (한셀이 XML 을 열 때의 표준·내장 스타일 XF)
    plain_edges: bool,
    /// 병합한 셀이 쓰는 서식 (fMergeCell, 바이트 8 의 0x20)
    merge_cell: bool,
}

/// 정렬 비교용: XML 의 생략값을 채운다
fn align_key(xf: &Xf) -> String {
    let a = xf.alignment.clone().unwrap_or_default();
    format!(
        "{}/{}/{}/{}/{}/{}/{}",
        a.horizontal.as_deref().unwrap_or("general"),
        a.vertical.as_deref().unwrap_or("bottom"),
        a.wrap_text,
        a.text_rotation.unwrap_or(0),
        a.indent.unwrap_or(0),
        a.shrink_to_fit,
        a.reading_order.unwrap_or(0)
    )
}

/// 한셀이 XML 을 열 때 켜는 적용 비트: 셀 서식의 정렬이 부모 스타일과 다르면 `applyAlignment` 가 없어도
/// 0x10 을 켠다 (관측: 서식 파일을 XML 로 열어 레거시·XML 로 저장하면 세로 정렬만 다른 셀 서식에 0x10 이 생긴다)
fn implied_used(xf: &Xf, parent: Option<&Xf>) -> u8 {
    match parent {
        Some(p) if align_key(xf) != align_key(p) => 0x10,
        _ => 0,
    }
}

/// XF 레코드
fn xf_record(xf: &Xf, o: &XfOpts, ctx: &XfCtx) -> Vec<u8> {
    let (style, parent, para_ref, has_ext) = (o.style, o.parent, o.para_ref, o.has_ext);
    let wb = ctx.wb;
    let st = &wb.styles;
    let fill = st.fills.get(xf.fill_id as usize);
    let border = st.borders.get(xf.border_id as usize);
    let font = ctx.fonts.map.get(xf.font_id as usize).copied().unwrap_or(0);
    let mut b = Buf::new();
    let prot = xf.protection.as_ref();
    let mut p: u16 = 0;
    if prot.and_then(|p| p.locked).unwrap_or(true) {
        p |= 0x0001;
    }
    if prot.and_then(|p| p.hidden).unwrap_or(false) {
        p |= 0x0002;
    }
    if style {
        p |= 0x0004 | 0xFFF0;
    } else {
        p |= (parent & 0x0FFF) << 4;
    }
    if xf.quote_prefix {
        p |= 0x0008;
    }
    let a = xf.alignment.as_ref();
    let alc = a
        .and_then(|a| a.horizontal.as_deref())
        .and_then(|h| H_ALIGN.iter().position(|x| *x == h))
        .unwrap_or(0) as u8;
    let alcv = a
        .and_then(|a| a.vertical.as_deref())
        .and_then(|v| V_ALIGN.iter().position(|x| *x == v))
        .unwrap_or(2) as u8;
    let wrap = a.is_some_and(|a| a.wrap_text) as u8;
    // 글자 회전(trot)은 늘 0 이다: 한셀의 레거시 저장은 회전을 쓰지 않는다 (회전 대조 실험 45·90·135·180·30·255
    // 모두 0, 레거시를 읽을 때는 trot 를 읽는다 — 10절)
    let shrink = a.is_some_and(|a| a.shrink_to_fit) as u8;
    // 한셀은 들여쓰기 칸을 쓰지 않는다 — 문단 모양의 여백으로 들여쓴다 (5.6절)
    let indent = if xf
        .hancom
        .as_ref()
        .and_then(|h| h.parashape.as_ref())
        .is_some()
    {
        0
    } else {
        a.and_then(|a| a.indent).unwrap_or(0).min(15) as u8
    };
    let ro = a.and_then(|a| a.reading_order).unwrap_or(0).min(3) as u8;
    let mut used = o.extra_used;
    for (on, bit) in [
        (xf.apply_number_format, 0x04),
        (xf.apply_font, 0x08),
        (xf.apply_alignment, 0x10),
        (xf.apply_border, 0x20),
        (xf.apply_fill, 0x40),
        (xf.apply_protection, 0x80),
    ] {
        if on == Some(true) {
            used |= bit;
        }
    }
    // 테두리
    let e = |f: fn(&Border) -> &Option<BorderEdge>| border.and_then(|b| f(b).as_ref());
    let (left, right, top, bottom, diag) = (
        e(|b| &b.left),
        e(|b| &b.right),
        e(|b| &b.top),
        e(|b| &b.bottom),
        e(|b| &b.diagonal),
    );
    let edge_icv = |x: Option<&BorderEdge>| -> u32 {
        if edge_dg(x) == 0 {
            0
        } else {
            icv_of(x.and_then(|e| e.color.as_ref()), wb, 0x40) as u32
        }
    };
    let grbit_diag = border.map_or(0, |b| {
        (b.diagonal_down as u32) | ((b.diagonal_up as u32) << 1)
    });
    let border1 = edge_dg(left)
        | (edge_dg(right) << 4)
        | (edge_dg(top) << 8)
        | (edge_dg(bottom) << 12)
        | (edge_icv(left) << 16)
        | (edge_icv(right) << 23)
        | (grbit_diag << 30);
    // 채우기
    let gradient = fill.and_then(|f| f.gradient.as_ref());
    let fls = match fill.and_then(|f| f.pattern_type.as_deref()) {
        _ if gradient.is_some() => 0,
        None | Some("none") => 0,
        Some(p) => PATTERNS.iter().position(|x| *x == p).unwrap_or(0) as u32,
    };
    let diag_dg = edge_dg(diag);
    let border2 = edge_icv(top)
        | (edge_icv(bottom) << 7)
        | (edge_icv(diag) << 14)
        | (diag_dg << 21)
        | ((has_ext as u32) << 25)
        | (fls << 26);
    let (fg, bg) = if fls == 0 {
        (0x40u16, 0x41u16)
    } else {
        let f = fill.unwrap();
        (
            icv_of(f.fg_color.as_ref(), wb, 0x40),
            icv_of(f.bg_color.as_ref(), wb, 0x41),
        )
    };
    b.u16(font)
        .u16(xf.num_fmt_id as u16)
        .u16(p)
        .u8(alc | (wrap << 3) | (alcv << 4))
        .u8(0)
        .u8(indent | (shrink << 4) | ((o.merge_cell as u8) << 5) | (ro << 6))
        .u8(used)
        .u32(border1)
        .u32(border2)
        .u16(fg | (bg << 7));
    // 한컴 확장: 개수, [문단 모양 번호], COLORREF ×8 (무늬 배경, 채우기, 위, 왼쪽, 아래, 오른쪽, 대각선, 미상)
    match para_ref {
        Some(r) => {
            b.u16(3).u32(r);
        }
        None => {
            b.u16(2);
        }
    }
    // 선이 있는 테두리의 자동 색은 0xDFFFFFFF (관측: 서식 파일 — COM 으로 만든 문서는 0xFFFFFFFF + XFEXT 자동).
    // 선이 없어도 색이 있으면 쓴다: XML 을 연 한셀은 `<x:left><x:color rgb="ff000000"/></x:left>` 를 0 으로,
    // `<x:top><x:color auto="1"/></x:top>` 를 0xDFFFFFFF 로 쓴다 (10절)
    let edge_cref = |x: Option<&BorderEdge>| -> u32 {
        let color = x.and_then(|e| e.color.as_ref());
        if edge_dg(x) != 0 {
            cref_of(color, wb, 0xDFFF_FFFF, 0xDFFF_FFFF)
        } else if o.plain_edges {
            0xFFFF_FFFF
        } else {
            cref_of(color, wb, 0xFFFF_FFFF, 0xDFFF_FFFF)
        }
    };
    let (bgc, fgc) = if fls == 0 {
        (0xFFFF_FFFF, 0xDFFF_FFFF)
    } else {
        let f = fill.unwrap();
        (
            cref_of(f.bg_color.as_ref(), wb, 0xFFFF_FFFF, 0xDFFF_FFFF),
            cref_of(f.fg_color.as_ref(), wb, 0xDFFF_FFFF, 0xDFFF_FFFF),
        )
    };
    b.u32(bgc)
        .u32(fgc)
        .u32(edge_cref(top))
        .u32(edge_cref(left))
        .u32(edge_cref(bottom))
        .u32(edge_cref(right))
        .u32(edge_cref(diag))
        .u32(0xFFFF_FFFF);
    b.take()
}

/// RGB 가 기본 팔레트에 똑같이 있나
fn in_palette(rgb: u32) -> bool {
    DEFAULT_PALETTE.contains(&rgb)
}

/// XFEXT 의 ExtProp 하나: (종류, 내용)
type ExtProp = (u16, Vec<u8>);

/// XFEXT (0x087D) 의 ExtProp 목록. 팔레트로 나타낼 수 없는 채우기·테두리 색은 FullColorExt (종류 4 채우기, 5 무늬 배경,
/// 9 왼쪽, 0x0A 오른쪽, 7 위, 8 아래, 0x0B 대각선 — 이 순서, 관측) 로, 그라데이션은 종류 6 으로. 한셀은 테마에서 온 색에도
/// 쓰지만 XML 에는 그 출처가 남지 않는다
fn xf_exts(xf: &Xf, wb: &Workbook) -> Vec<ExtProp> {
    let st = &wb.styles;
    let mut v = Vec::new();
    // `always`: 팔레트에 있는 색이어도 쓴다 (오른쪽 테두리 — 관측: 말뭉치 XML 을 한셀이 레거시로 저장하면 선이 있는
    // 오른쪽 테두리는 검정·회색·흰색도 0x000A 로 쓴다. 왼쪽·위·아래는 팔레트 밖의 색만)
    let full_with = |c: Option<&Color>, always: bool| -> Option<Vec<u8>> {
        let c = c?;
        if c.auto || c.indexed.is_some() && c.rgb.is_none() {
            return None;
        }
        let rgb = rgb_of(c, wb)?;
        if in_palette(rgb) && !always {
            return None;
        }
        let mut b = Buf::new();
        b.u16(2)
            .u16(0)
            .u8((rgb >> 16) as u8)
            .u8((rgb >> 8) as u8)
            .u8(rgb as u8)
            .u8(0)
            .bytes(&[0; 8]);
        Some(b.take())
    };
    let full = |c: Option<&Color>| full_with(c, false);
    // 글자 색 (0x000D): 팔레트 밖의 색이면 맨 앞에 (관측 차례 0x0D, 4, 5, 9, 0x0A, 7, 8)
    if let Some(x) = full(
        st.fonts
            .get(xf.font_id as usize)
            .and_then(|f| f.color.as_ref()),
    ) {
        v.push((0x0D, x));
    }
    if let Some(f) = st.fills.get(xf.fill_id as usize) {
        if let Some(g) = &f.gradient {
            v.push((6, gradient_ext(g, wb)));
        } else if f.pattern_type.as_deref().is_some_and(|p| p != "none") {
            if let Some(x) = full(f.fg_color.as_ref()) {
                v.push((4, x));
            }
            if let Some(x) = full(f.bg_color.as_ref()) {
                v.push((5, x));
            }
        }
    }
    if let Some(b) = st.borders.get(xf.border_id as usize) {
        for (ty, e) in [
            (9u16, &b.left),
            (0x0A, &b.right),
            (7, &b.top),
            (8, &b.bottom),
            (0x0B, &b.diagonal),
        ] {
            if let Some(e) = e
                && edge_dg(Some(e)) != 0
                && let Some(x) = full_with(e.color.as_ref(), ty == 0x0A)
            {
                v.push((ty, x));
            }
        }
    }
    v
}

/// XFEXT (0x087D) 의 그라데이션 채우기 ExtProp (종류 6)
fn gradient_ext(g: &crate::model::GradientFill, wb: &Workbook) -> Vec<u8> {
    let mut v = Buf::new();
    v.u32(g.path as u32)
        .f64(g.degree)
        .f64(g.left)
        .f64(g.right)
        .f64(g.top)
        .f64(g.bottom)
        .u32(g.stops.len() as u32);
    for s in &g.stops {
        let c = &s.color;
        let (kind, val) = if c.auto {
            (0u16, 0u32)
        } else if let Some(t) = c.theme.filter(|_| c.rgb.is_none()) {
            (3, t)
        } else if let Some(i) = c.indexed.filter(|_| c.rgb.is_none()) {
            (1, i)
        } else {
            (2, rgb_of(c, wb).map_or(0, colorref_of))
        };
        v.u16(kind)
            .u32(val)
            .f64(s.position)
            .f64(c.tint.unwrap_or(0.0));
    }
    v.take()
}

// ---------------------------------------------------------------------------
// 공유 문자열
// ---------------------------------------------------------------------------

pub(crate) struct Sst {
    strings: Vec<RichText>,
    index: HashMap<String, u32>,
    total: u32,
}

pub(crate) type SstRef = Sst;

impl Sst {
    fn new(wb: &Workbook) -> Sst {
        let mut s = Sst {
            strings: Vec::new(),
            index: HashMap::new(),
            total: 0,
        };
        for r in &wb.shared_strings {
            s.index
                .entry(r.text.clone())
                .or_insert(s.strings.len() as u32);
            s.strings.push(r.clone());
        }
        s
    }
    fn get(&mut self, text: &str) -> u32 {
        self.total += 1;
        if let Some(&i) = self.index.get(text) {
            return i;
        }
        let i = self.strings.len() as u32;
        self.strings.push(RichText {
            text: text.to_string(),
            runs: Vec::new(),
        });
        self.index.insert(text.to_string(), i);
        i
    }
}

/// SST 문자열: `cch u16, flags u8 (0x01 = 서식 런), UTF-16, [cRun u16, (ich u16, ifnt u16) × cRun]` (5.7절)
fn sst_string(b: &mut Buf, r: &RichText, font_of: &dyn Fn(&Font) -> u16) {
    let runs: Vec<(u16, u16)> = if r.runs.len() > 1 || r.runs.iter().any(|x| x.font.is_some()) {
        let mut out = Vec::new();
        let mut pos = 0u16;
        for run in &r.runs {
            if let Some(f) = &run.font {
                out.push((pos, font_of(f)));
            }
            pos += units(&run.text) as u16;
        }
        out
    } else {
        Vec::new()
    };
    b.u16(units(&r.text) as u16)
        .u8(if runs.is_empty() { 0 } else { 1 })
        .utf16(&r.text);
    if !runs.is_empty() {
        b.u16(runs.len() as u16);
        for (ich, ifnt) in runs {
            b.u16(ich).u16(ifnt);
        }
    }
}

// ---------------------------------------------------------------------------
// 수식 번호 표 (SUPBOOK / EXTERNSHEET / NAME)
// ---------------------------------------------------------------------------

/// NAME 레코드 하나
struct NameRec {
    /// 내장 이름이면 번호 (0x06 Print_Area …)
    builtin: Option<u8>,
    name: String,
    hidden: bool,
    /// 함수 이름 (`_xlfn.…`, grbit 0x000B)
    func: bool,
    /// 시트 범위 (0부터)
    local: Option<u32>,
    formula: String,
    rgce: Vec<u8>,
}

fn builtin_code(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("_xlnm.")?;
    (0u32..=0x0D)
        .find(|&c| super::reader::builtin_name(c).strip_prefix("_xlnm.") == Some(rest))
        .map(|c| c as u8)
}

struct Tables<'a> {
    wb: &'a Workbook,
    sheet_names: Vec<String>,
    /// SUPBOOK: None = 이 문서, Some(n) = 외부 연결 n (1부터)
    supbooks: Vec<Option<usize>>,
    /// (SUPBOOK 번호, 첫 시트, 끝 시트)
    xti: Vec<(u16, u16, u16)>,
    names: Vec<NameRec>,
}

impl Tables<'_> {
    fn supbook(&mut self, book: Option<usize>) -> u16 {
        if let Some(i) = self.supbooks.iter().position(|b| *b == book) {
            return i as u16;
        }
        self.supbooks.push(book);
        (self.supbooks.len() - 1) as u16
    }
}

impl Resolve for Tables<'_> {
    fn sheet(&self, name: &str) -> Option<usize> {
        self.sheet_names
            .iter()
            .position(|s| s.eq_ignore_ascii_case(name) || s == name)
    }
    fn ext_sheet(&self, book: usize, name: &str) -> Option<usize> {
        self.wb
            .external_links
            .get(book.checked_sub(1)?)?
            .sheet_names
            .iter()
            .position(|s| s == name || s.eq_ignore_ascii_case(name))
    }
    fn xti(&mut self, book: Option<usize>, first: usize, last: usize) -> u16 {
        let sb = self.supbook(book);
        let key = (sb, first as u16, last as u16);
        if let Some(i) = self.xti.iter().position(|x| *x == key) {
            return i as u16;
        }
        self.xti.push(key);
        (self.xti.len() - 1) as u16
    }
    fn name(&mut self, name: &str, sheet: Option<usize>) -> Option<u32> {
        let find = |local: Option<u32>| {
            self.names.iter().position(|n| {
                !n.func
                    && n.local == local
                    && (n.name.eq_ignore_ascii_case(name)
                        || n.name.to_lowercase() == name.to_lowercase())
            })
        };
        let found = sheet
            .and_then(|s| find(Some(s as u32)))
            .or_else(|| find(None))
            .or_else(|| {
                self.names
                    .iter()
                    .position(|n| !n.func && n.name.to_lowercase() == name.to_lowercase())
            });
        // 정의되지 않은 이름(`=XYZ`)은 수식 없는 NAME 으로 둔다 (관측: values 의 XYZ)
        let i = found.unwrap_or_else(|| {
            self.names.push(NameRec {
                builtin: None,
                name: name.to_string(),
                hidden: false,
                func: false,
                local: None,
                formula: String::new(),
                rgce: Vec::new(),
            });
            self.names.len() - 1
        });
        Some(i as u32 + 1)
    }
    fn ext_name(&mut self, _book: usize, _name: &str) -> Option<(u16, u32)> {
        None
    }
    fn func_name(&mut self, name: &str) -> u32 {
        if let Some(i) = self
            .names
            .iter()
            .position(|n| n.func && n.name.eq_ignore_ascii_case(name))
        {
            return i as u32 + 1;
        }
        self.names.push(NameRec {
            builtin: None,
            name: name.to_string(),
            hidden: name.starts_with("_xlfn."),
            func: true,
            local: None,
            formula: String::new(),
            rgce: Vec::new(),
        });
        self.names.len() as u32
    }
    fn note_future(&mut self, name: &str) {
        let full = format!("_xlfn.{name}");
        self.func_name(&full);
    }
}

/// SUPBOOK 가상 경로 ([MS-XLS] 2.5.277) — 관계 Target 에서 거꾸로 (읽기의 `virtual_path`)
fn encode_virtual_path(target: &str, path_missing: bool) -> String {
    let decode = |s: &str| -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && let Some(v) = s
                    .get(i + 1..i + 3)
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    if path_missing {
        return decode(target);
    }
    let mut out = String::from('\u{1}');
    let rest = if let Some(r) = target.strip_prefix("file:///") {
        // file:///C:/a/b.xls → U+0001 C a U+0003 b.xls
        let mut it = r.splitn(2, ":/");
        match (it.next(), it.next()) {
            (Some(d), Some(rest)) if d.len() == 1 => {
                out.push('\u{1}');
                out.push_str(d);
                rest.to_string()
            }
            _ => r.to_string(),
        }
    } else if let Some(r) = target.strip_prefix("file://") {
        out.push('\u{1}');
        out.push('@');
        r.to_string()
    } else if let Some(r) = target.strip_prefix('/') {
        out.push('\u{2}');
        r.to_string()
    } else {
        target.to_string()
    };
    let parts: Vec<String> = rest.split('/').map(decode).collect();
    for (i, p) in parts.iter().enumerate() {
        if p == ".." {
            out.push('\u{4}');
            continue;
        }
        out.push_str(p);
        if i + 1 < parts.len() {
            out.push('\u{3}');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 셀 값
// ---------------------------------------------------------------------------

/// RK: 한셀은 f64 의 아래 34비트가 0 인 값만 RK(실수 꼴, 낮은 2비트 0)로 쓴다 (관측: 70 → 0x40518000)
fn rk_of(v: f64) -> Option<u32> {
    let bits = v.to_bits();
    (bits & 0x3_FFFF_FFFF == 0).then_some((bits >> 32) as u32)
}

fn error_code(s: &str) -> u8 {
    match s {
        "#NULL!" => 0x00,
        "#DIV/0!" => 0x07,
        "#VALUE!" => 0x0F,
        "#REF!" => 0x17,
        "#NAME?" => 0x1D,
        "#NUM!" => 0x24,
        "#N/A" => 0x2A,
        "#GETTING_DATA" => 0x2B,
        _ => 0x2A,
    }
}

/// 날짜 값(`t="d"`)을 일련 번호로
fn date_serial(s: &str, date1904: bool) -> Option<f64> {
    let ft = filetime(s)? as f64 / 10_000_000.0 - 11_644_473_600.0;
    let days = ft / 86_400.0;
    // 1900 체계: 1899-12-30 = 0 (유닉스 기준 −25569)
    let base = if date1904 { 24_107.0 } else { 25_569.0 };
    Some(days + base)
}

enum CellRec {
    Blank(u16),
    Rk(u16, u32),
    Number(u16, f64),
    Label(u16, u32),
    BoolErr(u16, u8, bool),
    Formula(u16, usize),
}

// ---------------------------------------------------------------------------
// 쓰기
// ---------------------------------------------------------------------------

/// 미리 부호화한 수식 하나
struct EncFormula {
    rgce: Vec<u8>,
    /// 공유 수식의 기준 셀이면 (범위, 상대 토큰)
    shared: Option<(CellRange, Vec<u8>)>,
    /// 공유 수식을 가리키면 기준 셀
    exp: Option<CellRef>,
    /// 상대 토큰(PtgRefN) 꼴 — 행 반복(0xB002)에서 두 행의 수식이 같은지 본다
    rel: Vec<u8>,
}

/// 시트마다 미리 부호화한 것
#[derive(Default)]
struct SheetEnc {
    /// (행, 열) → 수식
    formulas: HashMap<(u32, u32), EncFormula>,
    /// 조건부 서식 (서식 번호, 규칙 번호) → (수식 1, 수식 2)
    cf: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
    /// 데이터 유효성 → (수식 1, 수식 2)
    dv: Vec<(Vec<u8>, Vec<u8>)>,
}

/// 쓰기 결과의 경고 (부호화하지 못한 수식 등)
#[derive(Debug, Default)]
pub struct Report {
    pub warnings: Vec<String>,
}

/// 통합 문서 → 레거시 OLE2 파일
pub fn write(wb: &Workbook) -> Result<Vec<u8>> {
    write_with_report(wb).map(|(b, _)| b)
}

/// [`write`] 와 같고, 경고 목록을 함께 돌려준다
pub fn write_with_report(wb: &Workbook) -> Result<(Vec<u8>, Report)> {
    let mut report = Report::default();
    let stream = workbook_stream(wb, &mut report)?;
    let summary = super::props::write_summary(&wb.properties);
    let modified = wb
        .properties
        .modified
        .as_deref()
        .and_then(filetime)
        .unwrap_or(0);
    let bytes = crate::cfb_writer::write(
        &[
            crate::cfb_writer::Stream {
                name: "Workbook",
                data: &stream,
            },
            crate::cfb_writer::Stream {
                name: "\u{5}SummaryInformation",
                data: &summary,
            },
        ],
        modified,
    );
    Ok((bytes, report))
}

fn enc_err(at: &str, e: EncodeError) -> String {
    format!("{at}: 수식을 부호화하지 못했습니다 — {e}")
}

/// `/Workbook` 스트림
pub fn workbook_stream(wb: &Workbook, report: &mut Report) -> Result<Vec<u8>> {
    if wb.sheets.is_empty() {
        return Err(Error::InvalidValue {
            context: "레거시 쓰기".into(),
            value: "시트가 없습니다".into(),
        });
    }
    let fonts = layout_fonts(wb);
    let styles = layout_styles(wb);
    let mut sst = Sst::new(wb);

    // 이름 표: 모델 순서 그대로 (한셀 XML 의 definedName 순서 = 레거시 NAME 순서, 관측)
    let mut tables = Tables {
        wb,
        sheet_names: wb.sheets.iter().map(|s| s.name.clone()).collect(),
        supbooks: Vec::new(),
        xti: Vec::new(),
        names: wb
            .defined_names
            .iter()
            .map(|d: &DefinedName| NameRec {
                builtin: builtin_code(&d.name),
                name: d.name.clone(),
                hidden: d.hidden,
                func: false,
                local: d.local_sheet_id,
                formula: d.formula.clone(),
                rgce: Vec::new(),
            })
            .collect(),
    };
    // 외부 연결 SUPBOOK 은 모델 순서대로 먼저 (번호 [n] 이 SUPBOOK 순서와 같게)
    for n in 1..=wb.external_links.len() {
        tables.supbook(Some(n));
    }

    // 이름 수식 (참조 클래스, 3차원)
    for i in 0..tables.names.len() {
        let f = tables.names[i].formula.clone();
        if f.is_empty() {
            continue;
        }
        let opts = encode::Options {
            sheet: tables.names[i].local.map(|v| v as usize),
            force_3d: true,
            root: Class::Ref,
            ..Default::default()
        };
        match encode::encode(&f, opts, &mut tables) {
            Ok(t) => tables.names[i].rgce = t,
            Err(e) => report
                .warnings
                .push(enc_err(&format!("이름 {}", tables.names[i].name), e)),
        }
    }

    // 시트 수식
    let mut encs: Vec<SheetEnc> = Vec::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        encs.push(encode_sheet(sheet, si, &mut tables, report));
    }

    // 전역
    let mut out = Out { buf: Vec::new() };
    out.rec(
        0x0809,
        &[
            0x01, 0x10, 0x05, 0x00, 0x02, 0x01, 0xD8, 0x07, 0, 0, 0, 0, 0x06, 0, 0, 0,
        ],
    );
    out.rec(0xA030, &[1, 0, 0, 0]);
    {
        // WRITEACCESS: 사용자 이름, 112바이트를 0x20 으로 채운다
        let user = wb
            .properties
            .last_modified_by
            .as_deref()
            .or(wb.properties.creator.as_deref())
            .unwrap_or("");
        let user: String = user.chars().take(54).collect();
        let mut b = Buf::new();
        b.str16(&user);
        b.0.resize(112, 0x20);
        out.rec(0x005C, &b.0);
    }
    out.rec(0x0042, &1200u16.to_le_bytes());
    out.rec_buf(0x01BA, Buf::new().str16("ThisWorkbook"));
    let prot = wb.protection.as_ref();
    // 한셀은 창 잠금을 늘 0 으로 쓴다 (6.4절)
    out.rec(0x0019, &[0, 0]);
    out.rec(
        0x0012,
        &(prot.is_some_and(|p| p.lock_structure) as u16).to_le_bytes(),
    );
    out.rec(
        0x0013,
        &prot
            .and_then(|p| p.password.as_deref())
            .and_then(|h| u16::from_str_radix(h, 16).ok())
            .unwrap_or(0)
            .to_le_bytes(),
    );
    {
        // WINDOW1: 창 위치·크기는 모델에 없다 — 한셀의 빈 문서 값 (관측)
        let active = wb
            .sheets
            .iter()
            .position(|s| s.view.tab_selected)
            .unwrap_or(0) as u16;
        let mut b = Buf::new();
        b.u16(0)
            .u16(0)
            .u16(0)
            .u16(0)
            .u16(0x0038)
            .u16(active)
            .u16(0)
            .u16(1)
            .u16(500);
        out.rec_buf(0x003D, &mut b);
    }
    out.rec(0x0022, &(wb.date1904 as u16).to_le_bytes());
    out.rec(0x0900, &[1, 0]);
    out.rec(0x000E, &[1, 0]);

    // 글꼴
    for (k, &i) in fonts.records.iter().enumerate() {
        let legacy = if k >= 4 { k + 1 } else { k };
        // 레거시 번호 0–3 과 5 (= 레코드 0–4) 가 상용구
        let boiler_index = if k < 5 { k } else { legacy };
        out.rec(0x0031, &font_record(&wb.styles.fonts[i], wb, boiler_index));
    }
    // 표시 형식: 한국어 내장 12개, 그다음 사용자 형식
    for id in [5u32, 6, 7, 8, 23, 24, 25, 26, 41, 42, 43, 44] {
        let code = wb
            .styles
            .num_fmts
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.code.as_str())
            .or_else(|| crate::numfmt::builtin(id))
            .unwrap_or("General");
        let mut b = Buf::new();
        b.u16(id as u16).str16(code);
        out.rec_buf(0x041E, &mut b);
    }
    let builtin_ids = [5u32, 6, 7, 8, 23, 24, 25, 26, 41, 42, 43, 44];
    for n in &wb.styles.num_fmts {
        if builtin_ids.contains(&n.id) {
            continue;
        }
        if n.id < 164 && crate::numfmt::builtin(n.id) == Some(n.code.as_str()) {
            continue;
        }
        let mut b = Buf::new();
        b.u16(n.id as u16).str16(&n.code);
        out.rec_buf(0x041E, &mut b);
    }
    // 문단 번호 정의 (0xA020)
    for a in &wb.styles.abstract_nums {
        let text = a.lvl_text.replace("%5e", "^").replace("%25", "%");
        let mut b = Buf::new();
        b.u8(a.num_type.saturating_sub(1) as u8)
            .u16(a.start_num as u16)
            .u16(units(&text) as u16)
            .u8(0)
            .utf16(&text);
        out.rec_buf(0xA020, &mut b);
    }
    // 문단 모양 표: 0번 기본, 그다음 XF 의 문단 모양 (정렬 포함) 처음 나온 순서. 0xA011 은 XF 마다 하나
    let xf_ctx = XfCtx { wb, fonts: &fonts };
    let mut para_records: Vec<Vec<u8>> = vec![parashape_record(&ParaShape::default(), 1)];
    let mut para_links: Vec<u32> = Vec::new();
    let mut xf_para: Vec<Option<u32>> = Vec::new();
    let slot_xf = |s: XfSlot| -> Option<&Xf> {
        match s {
            XfSlot::Style(i) | XfSlot::StyleCopy(i) => wb.styles.cell_style_xfs.get(i),
            XfSlot::Cell(i) => wb.styles.cell_xfs.get(i),
            XfSlot::Boiler { .. } => None,
        }
    };
    for &slot in &styles.order {
        let ps = slot_xf(slot).and_then(|x| {
            x.hancom
                .as_ref()
                .and_then(|h| h.parashape.as_ref())
                .map(|p| (x, p))
        });
        match ps {
            Some((x, p)) => {
                let rec = parashape_record(p, hangul_align(x));
                let idx = match para_records.iter().skip(1).position(|r| *r == rec) {
                    Some(i) => i + 1,
                    None => {
                        para_records.push(rec);
                        para_records.len() - 1
                    }
                };
                xf_para.push(Some(para_links.len() as u32));
                para_links.push(idx as u32);
            }
            None => xf_para.push(None),
        }
    }
    for r in &para_records {
        out.rec(0xA001, r);
    }
    for l in &para_links {
        let mut b = Buf::new();
        b.u32(1).u32(*l);
        out.rec_buf(0xA011, &mut b);
    }
    // XF
    let normal_xf = slot_xf(styles.order[0]).cloned().unwrap_or_default();
    // 병합 범위 안의 셀이 쓰는 셀 서식: 한셀은 이 XF 에 fMergeCell 을 켠다 (관측: 말뭉치의 병합 문서 19개에서
    // 한셀 레거시·XML→레거시 모두 켜진 XF 수가 같다)
    let merged_xfs: std::collections::HashSet<u32> = wb
        .sheets
        .iter()
        .flat_map(|s| {
            s.cells
                .iter()
                .filter(|c| s.merges.iter().any(|m| m.contains(c.r)))
                .map(|c| c.style)
        })
        .collect();
    let mut xfexts: Vec<(u16, Vec<ExtProp>)> = Vec::new();
    for (k, &slot) in styles.order.iter().enumerate() {
        let (xf, style) = match slot {
            XfSlot::Boiler { used, fmt } => {
                let mut x = normal_xf.clone();
                x.num_fmt_id = fmt as u32;
                x.apply_number_format = Some(used & 0x04 != 0);
                x.apply_font = Some(used & 0x08 != 0);
                x.apply_alignment = Some(used & 0x10 != 0);
                x.apply_border = Some(used & 0x20 != 0);
                x.apply_fill = Some(used & 0x40 != 0);
                x.apply_protection = Some(used & 0x80 != 0);
                x.hancom = None;
                (x, true)
            }
            XfSlot::Style(i) | XfSlot::StyleCopy(i) => (wb.styles.cell_style_xfs[i].clone(), true),
            XfSlot::Cell(i) => (wb.styles.cell_xfs[i].clone(), false),
        };
        let parent = match (slot, styles.cell0_parent) {
            (XfSlot::Cell(0), Some(p)) => p,
            _ => xf
                .xf_id
                .and_then(|p| styles.style.get(p as usize).copied().flatten())
                .unwrap_or(0),
        };
        let exts = match slot {
            XfSlot::Boiler { .. } => Vec::new(),
            _ => xf_exts(&xf, wb),
        };
        let has_ext = !exts.is_empty();
        if has_ext {
            xfexts.push((k as u16, exts));
        }
        let from_xml = !wb.styles.from_legacy;
        let extra_used = match slot {
            XfSlot::Cell(_) if from_xml => implied_used(
                &xf,
                wb.styles.cell_style_xfs.get(xf.xf_id.unwrap_or(0) as usize),
            ),
            _ => 0,
        };
        // 표준·내장 스타일 자리(0–20)는 한셀 자신의 것이라 선 없는 가장자리 색이 없다
        let plain_edges = from_xml
            && match slot {
                XfSlot::Boiler { .. } => true,
                XfSlot::Style(i) => styles.style[i].is_some_and(|x| x <= 20),
                _ => false,
            };
        let opts = XfOpts {
            style,
            parent,
            para_ref: xf_para[k],
            has_ext,
            extra_used,
            plain_edges,
            merge_cell: matches!(slot, XfSlot::Cell(i) if merged_xfs.contains(&(i as u32))),
        };
        if std::env::var_os("RCELL_XF_DEBUG").is_some() {
            let b = wb.styles.borders.get(xf.border_id as usize);
            let e = |x: Option<&BorderEdge>| {
                x.map_or("-".to_string(), |e| {
                    format!(
                        "{}:{}",
                        e.style.as_deref().unwrap_or("none"),
                        e.color.as_ref().map_or("nocolor".into(), |c| format!(
                            "{:?}/{:?}/{}",
                            c.rgb, c.indexed, c.auto
                        ))
                    )
                })
            };
            let fc = wb
                .styles
                .fonts
                .get(xf.font_id as usize)
                .and_then(|f| f.color.as_ref())
                .map_or("-".into(), |c| {
                    format!("{:?}/{:?}/{:?}", c.rgb, c.theme, c.indexed)
                });
            eprintln!(
                "XF {k} font={} fc={fc} L={} R={} T={} B={} D={}",
                xf.font_id,
                e(b.and_then(|b| b.left.as_ref())),
                e(b.and_then(|b| b.right.as_ref())),
                e(b.and_then(|b| b.top.as_ref())),
                e(b.and_then(|b| b.bottom.as_ref())),
                e(b.and_then(|b| b.diagonal.as_ref())),
            );
        }
        out.rec(0x00E0, &xf_record(&xf, &opts, &xf_ctx));
    }
    for (ixfe, exts) in xfexts {
        let mut b = frt(0x087D);
        b.u16(0).u16(ixfe).u16(0).u16(exts.len() as u16);
        for (ty, v) in exts {
            b.u16(ty).u16((v.len() + 4) as u16).bytes(&v);
        }
        out.rec_buf(0x087D, &mut b);
    }
    // STYLE: 상용구 다섯, 표준, 그다음 나머지 스타일 (XF 순서)
    for (xf, id) in [(16u16, 5u8), (17, 3), (18, 6), (19, 4), (20, 7)] {
        let mut b = Buf::new();
        b.u16(xf | 0x8000).u8(id).u8(0xFF);
        out.rec_buf(0x0293, &mut b);
    }
    out.rec(0x0293, &[0x00, 0x80, 0x00, 0xFF]);
    let mut rest: Vec<(u16, Vec<u8>)> = Vec::new();
    for c in &wb.styles.cell_styles {
        let Some(Some(ixfe)) = styles.style.get(c.xf_id as usize).copied() else {
            continue;
        };
        if ixfe == 0 && c.builtin_id == Some(0) || (16..=20).contains(&ixfe) {
            continue;
        }
        let mut b = Buf::new();
        match c.builtin_id {
            Some(id) => {
                b.u16(ixfe | 0x8000)
                    .u8(id as u8)
                    .u8(c.i_level.map_or(0xFF, |l| l as u8));
            }
            None => {
                b.u16(ixfe).str16(&c.name);
            }
        }
        rest.push((ixfe, b.take()));
    }
    rest.sort_by_key(|r| r.0);
    for (_, d) in rest {
        out.rec(0x0293, &d);
    }
    // 차등 서식 (0x088D)
    for d in &wb.styles.dxfs {
        let mut b = frt(0x088D);
        b.bytes(&super::dxf::dxf_props(d, wb));
        out.rec_buf(0x088D, &mut b);
    }
    // 표 스타일
    {
        let ts = wb.styles.table_styles.as_ref();
        let dt = ts
            .and_then(|t| t.default_table_style.clone())
            .unwrap_or_else(|| "TableStyleMedium2".into());
        let dp = ts
            .and_then(|t| t.default_pivot_style.clone())
            .unwrap_or_else(|| "PivotStyleLight16".into());
        let n = ts.map_or(0, |t| t.styles.len()) as u32;
        let mut b = frt(0x088E);
        b.u32(144 + n)
            .u16(units(&dt) as u16)
            .u16(units(&dp) as u16)
            .utf16(&dt)
            .utf16(&dp);
        out.rec_buf(0x088E, &mut b);
        for s in ts.map(|t| t.styles.as_slice()).unwrap_or_default() {
            let mut b = frt(0x088F);
            let flags = ((s.pivot as u16) << 1) | ((s.table as u16) << 2);
            b.u16(flags)
                .u32(s.elements.len() as u32)
                .u16(units(&s.name) as u16)
                .utf16(&s.name);
            out.rec_buf(0x088F, &mut b);
            for e in &s.elements {
                let t = crate::model::TABLE_STYLE_TYPES
                    .iter()
                    .position(|x| *x == e.kind)
                    .unwrap_or(0) as u32;
                let mut b = frt(0x0890);
                b.u32(t).u32(e.size).u32(e.dxf_id.unwrap_or(0));
                out.rec_buf(0x0890, &mut b);
            }
        }
    }
    out.rec(0x0160, &[0, 0]);
    // BOUNDSHEET (위치는 나중에)
    let mut bound_at = Vec::new();
    for s in &wb.sheets {
        let mut b = Buf::new();
        let hs = match s.state {
            SheetState::Visible => 0u8,
            SheetState::Hidden => 1,
            SheetState::VeryHidden => 2,
        };
        let dt = match s.kind {
            SheetKind::Chartsheet => 2u8,
            _ => 0,
        };
        b.u32(0)
            .u8(hs)
            .u8(dt)
            .u8(units(&s.name) as u8)
            .u8(0)
            .utf16(&s.name);
        bound_at.push(out.buf.len() + 4);
        out.rec_buf(0x0085, &mut b);
    }
    out.rec(0xA101, &wb.hancom.calc_id.unwrap_or(1).to_le_bytes());
    // 그림 저장소
    let art = super::art::Art::new(wb);
    if let Some(dgg) = art.drawing_group() {
        out.rec(0x00EB, &dgg);
    }
    // SUPBOOK / EXTERNSHEET (참조가 있을 때만)
    if !tables.xti.is_empty() {
        for (i, sb) in tables.supbooks.iter().enumerate() {
            let mut b = Buf::new();
            match sb {
                None => {
                    b.u16(wb.sheets.len() as u16).u16(0x0401);
                }
                Some(n) => {
                    let link = &wb.external_links[n - 1];
                    let path = encode_virtual_path(&link.target, link.path_missing);
                    b.u16(link.sheet_names.len() as u16)
                        .u16(units(&path) as u16)
                        .u8(0)
                        .utf16(&path);
                    for s in &link.sheet_names {
                        b.str16(s);
                    }
                }
            }
            let _ = i;
            out.rec_buf(0x01AE, &mut b);
        }
        let mut b = Buf::new();
        b.u16(tables.xti.len() as u16);
        for &(sb, a, c) in &tables.xti {
            b.u16(sb).u16(a).u16(c);
        }
        out.rec_buf(0x0017, &mut b);
    }
    for n in &tables.names {
        let mut b = Buf::new();
        let mut grbit = 0u16;
        if n.hidden {
            grbit |= 0x0001;
        }
        if n.func {
            grbit |= 0x000A;
        }
        if n.builtin.is_some() {
            grbit |= 0x0020;
        }
        let name_units: Vec<u16> = match n.builtin {
            Some(c) => vec![c as u16],
            None => n.name.encode_utf16().collect(),
        };
        b.u16(grbit)
            .u8(0)
            .u8(name_units.len() as u8)
            .u16(n.rgce.len() as u16)
            .u16(0)
            .u16(n.local.map_or(0, |l| l as u16 + 1))
            .bytes(&[0; 4])
            .u8(0);
        for u in name_units {
            b.u16(u);
        }
        b.bytes(&n.rgce);
        out.rec_buf(0x0018, &mut b);
    }

    // 시트를 먼저 써서 SST 사용 수를 센다
    let mut sheet_streams: Vec<Vec<u8>> = Vec::new();
    for (si, sheet) in wb.sheets.iter().enumerate() {
        let mut so = Out { buf: Vec::new() };
        let ctx = SheetCtx {
            wb,
            si,
            sheet,
            styles: &styles,
            fonts: &fonts,
            enc: &encs[si],
            art: &art,
        };
        write_sheet(&mut so, &ctx, &mut sst)?;
        sheet_streams.push(so.buf);
    }
    let font_of = |f: &Font| -> u16 {
        wb.styles
            .fonts
            .iter()
            .position(|x| same_font(x, f))
            .map_or(0, |i| fonts.map[i])
    };
    {
        let mut b = Buf::new();
        b.u32(sst.total).u32(sst.strings.len() as u32);
        for s in &sst.strings {
            sst_string(&mut b, s, &font_of);
        }
        out.rec_buf(0x00FC, &mut b);
    }
    out.rec(0x000A, &[]);
    for (i, s) in sheet_streams.iter().enumerate() {
        let at = out.buf.len() as u32;
        out.buf[bound_at[i]..bound_at[i] + 4].copy_from_slice(&at.to_le_bytes());
        out.buf.extend_from_slice(s);
    }
    Ok(out.buf)
}

/// 시트의 수식을 미리 부호화한다 (셀, 조건부 서식, 데이터 유효성)
fn encode_sheet(sheet: &Sheet, si: usize, tables: &mut Tables, report: &mut Report) -> SheetEnc {
    let mut e = SheetEnc::default();
    // 공유 수식: 기준 셀(원문이 있고 범위가 있는 셀)
    let mut masters: HashMap<u32, (CellRef, CellRange)> = HashMap::new();
    for c in &sheet.cells {
        let Some(f) = &c.formula else { continue };
        if f.kind == FormulaKind::Shared
            && let (Some(si), Some(range)) = (
                f.shared_index,
                f.range.as_deref().and_then(CellRange::parse),
            )
        {
            masters.insert(si, (c.r, range));
        }
    }
    for c in &sheet.cells {
        let Some(f) = &c.formula else { continue };
        if f.text.is_empty() && f.kind != FormulaKind::Shared {
            continue;
        }
        let at = format!("{}!{}", sheet.name, c.r);
        let opts = encode::Options {
            base: c.r,
            sheet: Some(si),
            ..Default::default()
        };
        let master = (f.kind == FormulaKind::Shared)
            .then(|| f.shared_index.and_then(|i| masters.get(&i).copied()))
            .flatten();
        let rel_opts = encode::Options {
            relative: true,
            ..opts
        };
        match master {
            Some((m, range)) if m == c.r => match encode::encode(&f.text, rel_opts, tables) {
                Ok(t) => {
                    e.formulas.insert(
                        (c.r.row, c.r.col),
                        EncFormula {
                            rgce: exp_token(m),
                            rel: t.clone(),
                            shared: Some((range, t)),
                            exp: Some(m),
                        },
                    );
                }
                Err(err) => report.warnings.push(enc_err(&at, err)),
            },
            Some((m, _)) => {
                let rel = encode::encode(&f.text, rel_opts, tables).unwrap_or_default();
                e.formulas.insert(
                    (c.r.row, c.r.col),
                    EncFormula {
                        rgce: exp_token(m),
                        shared: None,
                        exp: Some(m),
                        rel,
                    },
                );
            }
            None => match encode::encode(&f.text, opts, tables) {
                Ok(t) => {
                    let rel = encode::encode(&f.text, rel_opts, tables).unwrap_or_default();
                    e.formulas.insert(
                        (c.r.row, c.r.col),
                        EncFormula {
                            rgce: t,
                            shared: None,
                            exp: None,
                            rel,
                        },
                    );
                }
                Err(err) => report.warnings.push(enc_err(&at, err)),
            },
        }
    }
    for cf in &sheet.conditional_formats {
        let base = cf
            .sqref
            .split_whitespace()
            .next()
            .and_then(CellRange::parse)
            .map(|r| r.start)
            .unwrap_or_default();
        let mut rules = Vec::new();
        for r in &cf.rules {
            let mut enc = |t: Option<&String>| -> Vec<u8> {
                let Some(t) = t.filter(|t| !t.is_empty()) else {
                    return Vec::new();
                };
                let opts = encode::Options {
                    base,
                    sheet: Some(si),
                    ..Default::default()
                };
                match encode::encode(t, opts, tables) {
                    Ok(v) => v,
                    Err(err) => {
                        report.warnings.push(enc_err(
                            &format!("{} 조건부 서식 {}", sheet.name, cf.sqref),
                            err,
                        ));
                        Vec::new()
                    }
                }
            };
            let f1 = enc(r.formulas.first());
            let f2 = enc(r.formulas.get(1));
            rules.push((f1, f2));
        }
        e.cf.push(rules);
    }
    for dv in &sheet.data_validations {
        let base = dv
            .sqref
            .split_whitespace()
            .next()
            .and_then(CellRange::parse)
            .map(|r| r.start)
            .unwrap_or_default();
        let mut enc = |t: Option<&String>| -> Vec<u8> {
            let Some(t) = t.filter(|t| !t.is_empty()) else {
                return Vec::new();
            };
            let root = if dv.kind == "list" {
                Class::Ref
            } else {
                Class::Val
            };
            let opts = encode::Options {
                base,
                sheet: Some(si),
                root,
                ..Default::default()
            };
            match encode::encode(t, opts, tables) {
                Ok(v) => v,
                Err(err) => {
                    report
                        .warnings
                        .push(enc_err(&format!("{} 유효성 {}", sheet.name, dv.sqref), err));
                    Vec::new()
                }
            }
        };
        let f1 = enc(dv.formula1.as_ref());
        let f2 = enc(dv.formula2.as_ref());
        e.dv.push((f1, f2));
    }
    e
}

fn exp_token(m: CellRef) -> Vec<u8> {
    let mut b = Buf::new();
    b.u8(0x01).u32(m.row).u16(m.col as u16);
    b.take()
}

pub(crate) struct SheetCtx<'a> {
    pub wb: &'a Workbook,
    pub si: usize,
    pub sheet: &'a Sheet,
    styles: &'a StyleLayout,
    fonts: &'a FontLayout,
    enc: &'a SheetEnc,
    pub art: &'a super::art::Art,
}

impl SheetCtx<'_> {
    pub fn xf(&self, style: u32) -> u16 {
        self.styles.cell.get(style as usize).copied().unwrap_or(15)
    }
    pub fn font(&self, i: u32) -> u16 {
        self.fonts.map.get(i as usize).copied().unwrap_or(0)
    }
    /// 모델 글꼴과 같은 글꼴의 레거시 번호
    pub fn font_like(&self, f: &Font) -> u16 {
        self.wb
            .styles
            .fonts
            .iter()
            .position(|x| same_font(x, f))
            .map_or(0, |i| self.fonts.map[i])
    }
    /// 데이터 유효성 i 의 부호화된 수식 1·2
    pub fn enc_dv(&self, i: usize) -> (&[u8], &[u8]) {
        self.enc
            .dv
            .get(i)
            .map_or((&[][..], &[][..]), |(a, b)| (a.as_slice(), b.as_slice()))
    }
    /// 조건부 서식 g 의 규칙 r 의 부호화된 수식 1·2
    pub fn enc_cf(&self, g: usize, r: usize) -> (&[u8], &[u8]) {
        self.enc
            .cf
            .get(g)
            .and_then(|v| v.get(r))
            .map_or((&[][..], &[][..]), |(a, b)| (a.as_slice(), b.as_slice()))
    }
}

/// 여백: 한셀은 문서를 열 때 인치 값을 HWPUNIT(1/7200 인치) 정수로 잘라 들고 있다가 f32 로 바꿔 f64 로 넓혀 쓴다
/// (관측: 머리글 0.51180553 → 3684 → 00 00 00 c0 92 5f e0 3f, 왼쪽 0.74805557 → 5386 → 00 00 00 40 12 f0 e7 3f)
fn margin(v: f64) -> f64 {
    (((v * 7200.0).trunc() / 7200.0) as f32) as f64
}

pub(crate) fn range_ref8(b: &mut Buf, r: &CellRange) {
    b.u32(r.start.row)
        .u32(r.end.row)
        .u16(r.start.col as u16)
        .u16(r.end.col as u16);
}

pub(crate) fn parse_ranges(s: &str) -> Vec<CellRange> {
    s.split_whitespace().filter_map(CellRange::parse).collect()
}

/// 머리글·바닥글의 글꼴 코드를 레거시 꼴로: `&"이름,Regular"` → `&"이름, REGULAR "` (6.1절의 반대)
fn legacy_hf_fonts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("&\"") {
        out.push_str(&rest[..i + 2]);
        rest = &rest[i + 2..];
        let Some(end) = rest.find('"') else { break };
        let spec = &rest[..end];
        match spec.split_once(',') {
            Some((name, style)) => {
                out.push_str(name);
                out.push_str(", ");
                out.push_str(&style.trim().to_uppercase());
                out.push(' ');
            }
            None => out.push_str(spec),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn write_sheet(out: &mut Out, ctx: &SheetCtx, sst: &mut Sst) -> Result<()> {
    let sheet = ctx.sheet;
    let wb = ctx.wb;
    out.rec(
        0x0809,
        &[
            0x01, 0x10, 0x10, 0x00, 0x02, 0x01, 0xD8, 0x07, 0, 0, 0, 0, 0x06, 0, 0, 0,
        ],
    );
    out.rec(0x000D, &[1, 0]);
    out.rec(0x000C, &100u16.to_le_bytes());
    out.rec(0x000F, &[1, 0]);
    out.rec(0x0011, &[0, 0]);
    out.rec(0x0010, &0.001f64.to_le_bytes());
    out.rec(0x005F, &[1, 0]);
    let po = sheet.page.print_options.clone().unwrap_or_default();
    out.rec(0x002A, &(po.headings as u16).to_le_bytes());
    out.rec(0x002B, &(po.grid_lines as u16).to_le_bytes());
    // GUTS: 한셀은 개요가 있어도 늘 0 을 쓴다 (관측: outline, filter2)
    out.rec(0x0080, &[0; 8]);
    let default_h = sheet.default_row_height.unwrap_or(15.0);
    {
        let mut b = Buf::new();
        b.u16(0).u16((default_h * 20.0).round() as u16);
        out.rec_buf(0x0225, &mut b);
    }
    // WSBOOL: 한셀은 요약 위치·자동 나누기를 쓰지 않고 0x04C1 을 쓴다 (6.4절)
    let wsbool = 0x04C1u16
        | if sheet.page.fit_to_page == Some(true) {
            0x0100
        } else {
            0
        };
    out.rec(0x0081, &wsbool.to_le_bytes());
    if !sheet.page.row_breaks.is_empty() {
        let mut b = Buf::new();
        b.u16(sheet.page.row_breaks.len() as u16);
        for &r in &sheet.page.row_breaks {
            b.u32(r).u16(0).u16(16383);
        }
        out.rec_buf(0x001B, &mut b);
    }
    if !sheet.page.col_breaks.is_empty() {
        let mut b = Buf::new();
        b.u16(sheet.page.col_breaks.len() as u16);
        for &c in &sheet.page.col_breaks {
            b.u16(c as u16).u16(0).u32(1_048_575);
        }
        out.rec_buf(0x001A, &mut b);
    }
    let hf = sheet.page.header_footer.as_ref();
    // 한셀은 글꼴 코드가 없는 구역에 기본 글꼴 코드를 넣고, HEADER·FOOTER 에는 모양을 대문자 + 공백으로 쓴다 (6.1절)
    let font0 = wb
        .styles
        .fonts
        .first()
        .and_then(|f| f.name.clone())
        .unwrap_or_else(|| "맑은 고딕".into());
    for (rt, text) in [
        (0x0014u16, hf.and_then(|h| h.odd_header.as_deref())),
        (0x0015, hf.and_then(|h| h.odd_footer.as_deref())),
    ] {
        match text.filter(|t| !t.is_empty()) {
            Some(t) => out.rec_buf(
                rt,
                Buf::new().str16(&legacy_hf_fonts(&super::lossy::with_default_fonts(
                    t, &font0,
                ))),
            ),
            None => out.rec(rt, &[]),
        }
    }
    out.rec(0x0083, &(po.horizontal_centered as u16).to_le_bytes());
    out.rec(0x0084, &(po.vertical_centered as u16).to_le_bytes());
    let m = sheet
        .page
        .margins
        .clone()
        .unwrap_or(crate::model::PageMargins {
            left: 0.7,
            right: 0.7,
            top: 0.75,
            bottom: 0.75,
            header: 0.3,
            footer: 0.3,
        });
    for (rt, v) in [
        (0x0026u16, m.left),
        (0x0027, m.right),
        (0x0028, m.top),
        (0x0029, m.bottom),
    ] {
        out.rec(rt, &margin(v).to_le_bytes());
    }
    {
        let s = sheet.page.setup.clone().unwrap_or_default();
        let mut g = 0u16;
        if s.page_order.as_deref() == Some("overThenDown") {
            g |= 0x01;
        }
        match s.orientation.as_deref() {
            Some("portrait") => g |= 0x02,
            Some("landscape") => {}
            _ => g |= 0x40 | 0x02,
        }
        if !s.use_printer_defaults {
            g |= 0x04;
        }
        if s.black_and_white {
            g |= 0x08;
        }
        // 간단히 인쇄(0x10)는 한셀이 저장하지 않는다 (6.1절)
        if s.use_first_page_number {
            g |= 0x80;
        }
        let mut b = Buf::new();
        b.u16(s.paper_size.unwrap_or(9) as u16)
            .u16(s.scale.unwrap_or(100) as u16)
            .u16(s.first_page_number.unwrap_or(1) as u16)
            .u16(s.fit_to_width.unwrap_or(1) as u16)
            .u16(s.fit_to_height.unwrap_or(1) as u16)
            .u16(g)
            .u16(s.horizontal_dpi.unwrap_or(0) as u16)
            .u16(s.vertical_dpi.unwrap_or(0) as u16)
            .f64(margin(m.header))
            .f64(margin(m.footer))
            .u16(s.copies.unwrap_or(1) as u16);
        out.rec_buf(0x00A1, &mut b);
    }
    {
        // HeaderFooter FRT (0x089C): FRT 머리는 0, GUID 0, 플래그, 짝수·첫 페이지 머리글·바닥글
        let h = hf.cloned().unwrap_or(crate::model::HeaderFooter {
            scale_with_doc: true,
            align_with_margins: true,
            ..Default::default()
        });
        let mut flags = 0u16;
        if h.different_odd_even {
            flags |= 0x01;
        }
        if h.different_first {
            flags |= 0x02;
        }
        if h.scale_with_doc {
            flags |= 0x04;
        }
        if h.align_with_margins {
            flags |= 0x08;
        }
        let texts: Vec<String> = [
            &h.even_header,
            &h.even_footer,
            &h.first_header,
            &h.first_footer,
        ]
        .iter()
        .map(|t| {
            t.as_deref()
                .filter(|t| !t.is_empty())
                .map(|t| super::lossy::with_default_fonts(t, &font0))
                .unwrap_or_default()
        })
        .collect();
        let mut b = Buf::new();
        b.bytes(&[0; 12]).bytes(&[0; 16]).u16(flags);
        for t in &texts {
            b.u16(units(t) as u16);
        }
        for t in &texts {
            b.str16(t);
        }
        out.rec_buf(0x089C, &mut b);
    }
    super::sheetx::protection(out, sheet);
    out.rec(0x0055, &8u16.to_le_bytes());
    super::sheetx::col_infos(out, ctx);
    super::sheetx::auto_filter(out, ctx);
    write_cells(out, ctx, sst)?;
    super::sheetx::after_cells(out, ctx, sst)?;
    Ok(())
}

fn write_cells(out: &mut Out, ctx: &SheetCtx, sst: &mut Sst) -> Result<()> {
    let sheet = ctx.sheet;
    let wb = ctx.wb;
    // DIMENSIONS: 셀 레코드가 차지하는 범위 (빈 서식 셀 포함)
    {
        let mut b = Buf::new();
        if sheet.cells.is_empty() {
            b.u32(0).u32(0).u16(0).u16(0).u16(0);
        } else {
            let r0 = sheet.cells.iter().map(|c| c.r.row).min().unwrap_or(0);
            let r1 = sheet.cells.iter().map(|c| c.r.row).max().unwrap_or(0);
            let c0 = sheet.cells.iter().map(|c| c.r.col).min().unwrap_or(0);
            let c1 = sheet.cells.iter().map(|c| c.r.col).max().unwrap_or(0);
            b.u32(r0)
                .u32(r1 + 1)
                .u16(c0 as u16)
                .u16(c1 as u16 + 1)
                .u16(0);
        }
        out.rec_buf(0x0200, &mut b);
    }
    // 행: 속성이 있거나 셀이 있는 행
    let mut rows: std::collections::BTreeMap<u32, (Option<&crate::model::RowInfo>, Vec<&Cell>)> =
        std::collections::BTreeMap::new();
    for r in &sheet.rows {
        rows.entry(r.index - 1).or_default().0 = Some(r);
    }
    for c in &sheet.cells {
        rows.entry(c.r.row).or_default().1.push(c);
    }
    let default_twips = (sheet.default_row_height.unwrap_or(15.0) * 20.0).round() as u16;
    let rows: Vec<(u32, Option<&crate::model::RowInfo>, Vec<&Cell>)> =
        rows.into_iter().map(|(r, (i, c))| (r, i, c)).collect();
    // 행 반복(0xB002): 바로 다음 행들이 행 속성·셀 서식까지 같으면 첫 행만 쓰고 되풀이한다. 한셀은 빈 칸(BLANK·
    // MULBLANK)만 있거나 셀이 없는 행만 되풀이한다 (관측: 서식 파일 76곳 모두, 두 행부터)
    let same_row = |a: &(u32, Option<&crate::model::RowInfo>, Vec<&Cell>),
                    b: &(u32, Option<&crate::model::RowInfo>, Vec<&Cell>)|
     -> bool {
        let key = |i: Option<&crate::model::RowInfo>| {
            i.map(|i| {
                (
                    i.height.map(|h| (h * 20.0).round() as i64),
                    i.custom_height,
                    i.hidden,
                    i.style,
                    i.outline_level,
                    i.collapsed,
                )
            })
        };
        if key(a.1) != key(b.1) || a.2.len() != b.2.len() {
            return false;
        }
        let blank = |c: &&Cell| c.value.is_empty() && c.formula.is_none();
        if !a.2.iter().all(blank) || !b.2.iter().all(blank) {
            return false;
        }
        a.2.iter().zip(&b.2).all(|(x, y)| {
            x.r.col == y.r.col
                && x.style == y.style
                && x.value == y.value
                && match (&x.formula, &y.formula) {
                    (None, None) => true,
                    (Some(_), Some(_)) => {
                        let fx = ctx.enc.formulas.get(&(x.r.row, x.r.col));
                        let fy = ctx.enc.formulas.get(&(y.r.row, y.r.col));
                        match (fx, fy) {
                            (Some(fx), Some(fy)) => {
                                !fx.rel.is_empty()
                                    && fx.rel == fy.rel
                                    && fx.shared.is_none()
                                    && fy.shared.is_none()
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                }
        })
    };
    let mut ri = 0;
    while ri < rows.len() {
        let (row, info, ref cells) = rows[ri];
        let mut last = ri;
        while last + 1 < rows.len()
            && rows[last + 1].0 == rows[last].0 + 1
            && same_row(&rows[ri], &rows[last + 1])
        {
            last += 1;
        }
        let repeat_to = (last > ri).then(|| rows[last].0);
        ri = last + 1;
        let mut b = Buf::new();
        let (c0, c1) = match (cells.first(), cells.last()) {
            (Some(a), Some(z)) => (a.r.col as u16, z.r.col as u16 + 1),
            _ => (0, 0),
        };
        let height = info
            .and_then(|i| i.height)
            .map_or(default_twips, |h| (h * 20.0).round() as u16);
        let mut flags: u32 = 0x0100;
        if let Some(i) = info {
            flags |= (i.outline_level & 0x07) as u32;
            if i.collapsed {
                flags |= 0x10;
            }
            if i.hidden {
                flags |= 0x20;
            }
            if i.custom_height {
                flags |= 0x40;
            }
        }
        let xf = match info.and_then(|i| i.style) {
            Some(s) => {
                flags |= 0x80;
                ctx.xf(s)
            }
            None => 15,
        };
        flags |= (xf as u32 & 0x0FFF) << 16;
        b.u32(row)
            .u16(c0)
            .u16(c1)
            .u16(height)
            .u16(0)
            .u16(0)
            .u32(flags);
        out.rec_buf(0x0208, &mut b);
        // 셀 레코드: 이웃한 RK 는 MULRK, 빈 칸은 MULBLANK
        let recs: Vec<(u32, CellRec)> = cells
            .iter()
            .enumerate()
            .map(|(ci, c)| (c.r.col, cell_rec(c, ci, ctx, sst)))
            .collect();
        let mut i = 0;
        while i < recs.len() {
            let (col, ref r) = recs[i];
            let run = |pred: &dyn Fn(&CellRec) -> bool| -> usize {
                let mut j = i;
                while j + 1 < recs.len() && recs[j + 1].0 == recs[j].0 + 1 && pred(&recs[j + 1].1) {
                    j += 1;
                }
                j - i + 1
            };
            match r {
                CellRec::Rk(..) => {
                    let n = run(&|x| matches!(x, CellRec::Rk(..)));
                    if n >= 2 {
                        let mut b = Buf::new();
                        b.u32(row).u16(col as u16);
                        for k in 0..n {
                            if let CellRec::Rk(xf, rk) = recs[i + k].1 {
                                b.u16(xf).u32(rk);
                            }
                        }
                        b.u16((col + n as u32 - 1) as u16);
                        out.rec_buf(0x00BD, &mut b);
                        i += n;
                        continue;
                    }
                    if let CellRec::Rk(xf, rk) = r {
                        let mut b = Buf::new();
                        b.u32(row).u16(col as u16).u16(*xf).u32(*rk);
                        out.rec_buf(0x027E, &mut b);
                    }
                }
                CellRec::Blank(xf) => {
                    let n = run(&|x| matches!(x, CellRec::Blank(..)));
                    if n >= 2 {
                        let mut b = Buf::new();
                        b.u32(row).u16(col as u16);
                        for k in 0..n {
                            if let CellRec::Blank(xf) = recs[i + k].1 {
                                b.u16(xf);
                            }
                        }
                        b.u16((col + n as u32 - 1) as u16);
                        out.rec_buf(0x00BE, &mut b);
                        i += n;
                        continue;
                    }
                    let mut b = Buf::new();
                    b.u32(row).u16(col as u16).u16(*xf);
                    out.rec_buf(0x0201, &mut b);
                }
                CellRec::Number(xf, v) => {
                    let mut b = Buf::new();
                    b.u32(row).u16(col as u16).u16(*xf).f64(*v);
                    out.rec_buf(0x0203, &mut b);
                }
                CellRec::Label(xf, isst) => {
                    let mut b = Buf::new();
                    b.u32(row).u16(col as u16).u16(*xf).u32(*isst);
                    out.rec_buf(0x00FD, &mut b);
                }
                CellRec::BoolErr(xf, v, is_err) => {
                    let mut b = Buf::new();
                    b.u32(row).u16(col as u16).u16(*xf).u8(*v).u8(*is_err as u8);
                    out.rec_buf(0x0205, &mut b);
                }
                CellRec::Formula(xf, ci) => {
                    write_formula(out, ctx, cells[*ci], *xf)?;
                }
            }
            i += 1;
        }
        if let Some(to) = repeat_to {
            let mut b = Buf::new();
            b.u32(row).u32(to);
            out.rec_buf(0xB002, &mut b);
        }
    }
    let _ = wb;
    Ok(())
}

fn cell_rec(c: &Cell, ci: usize, ctx: &SheetCtx, sst: &mut Sst) -> CellRec {
    let xf = ctx.xf(c.style);
    if c.formula.is_some() && ctx.enc.formulas.contains_key(&(c.r.row, c.r.col)) {
        return CellRec::Formula(xf, ci);
    }
    match &c.value {
        CellValue::Empty => CellRec::Blank(xf),
        CellValue::Number(v) => match rk_of(*v) {
            Some(rk) => CellRec::Rk(xf, rk),
            None => CellRec::Number(xf, *v),
        },
        CellValue::Date(d) => match date_serial(d, ctx.wb.date1904) {
            Some(v) => match rk_of(v) {
                Some(rk) => CellRec::Rk(xf, rk),
                None => CellRec::Number(xf, v),
            },
            None => CellRec::Label(xf, sst.get(d)),
        },
        CellValue::Text(s) => CellRec::Label(xf, sst.get(s)),
        CellValue::Bool(b) => CellRec::BoolErr(xf, *b as u8, false),
        CellValue::Error(e) => CellRec::BoolErr(xf, error_code(e), true),
    }
}

fn write_formula(out: &mut Out, ctx: &SheetCtx, c: &Cell, xf: u16) -> Result<()> {
    let Some(enc) = ctx.enc.formulas.get(&(c.r.row, c.r.col)) else {
        return Ok(());
    };
    let mut b = Buf::new();
    b.u32(c.r.row).u16(c.r.col as u16).u16(xf);
    let mut string: Option<&str> = None;
    match &c.value {
        CellValue::Number(v) => {
            b.f64(*v);
        }
        CellValue::Date(d) => {
            b.f64(date_serial(d, ctx.wb.date1904).unwrap_or(0.0));
        }
        CellValue::Text(s) => {
            b.bytes(&[0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
            string = Some(s);
        }
        CellValue::Bool(v) => {
            b.bytes(&[1, 0, *v as u8, 0, 0, 0, 0xFF, 0xFF]);
        }
        CellValue::Error(e) => {
            b.bytes(&[2, 0, error_code(e), 0, 0, 0, 0xFF, 0xFF]);
        }
        CellValue::Empty => {
            b.f64(0.0);
        }
    }
    // grbit: 0x0008 공유 수식, 0x0001 휘발 함수가 있음 (관측: TODAY() 수식)
    let mut grbit: u16 = if enc.exp.is_some() { 0x0008 } else { 0 };
    if enc.rgce.starts_with(&[0x19, 0x01]) || enc.rel.starts_with(&[0x19, 0x01]) {
        grbit |= 0x0001;
    }
    b.u16(grbit)
        .u32(0)
        .u16(enc.rgce.len() as u16)
        .bytes(&enc.rgce);
    out.rec_buf(0x0006, &mut b);
    if let Some((range, tokens)) = &enc.shared {
        let mut s = Buf::new();
        range_ref8(&mut s, range);
        s.u16(0).u16(tokens.len() as u16).bytes(tokens);
        out.rec_buf(0x04BC, &mut s);
    }
    if let Some(s) = string {
        out.rec_buf(0x0207, Buf::new().str16(s));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_from_w3cdtf() {
        assert_eq!(
            filetime("1997-03-01T10:49:08.000"),
            Some(0x01BC_262E_3049_6A00)
        );
        assert_eq!(
            filetime("1997-05-05T14:10:37Z"),
            Some(0x01BC_595E_1ABE_2C80)
        );
        assert_eq!(
            super::super::props::filetime_text(filetime("2026-10-19T07:23:02.367").unwrap()),
            "2026-10-19T07:23:02Z"
        );
    }

    #[test]
    fn rk_only_for_short_mantissas() {
        assert_eq!(rk_of(1.0), Some(0x3FF0_0000));
        assert_eq!(rk_of(70.0), Some(0x4051_8000));
        assert_eq!(rk_of(0.0), Some(0));
        assert_eq!(rk_of(1e15), None);
        assert_eq!(rk_of(0.1), None);
    }

    #[test]
    fn nearest_palette_color() {
        // 관측: 6182d6 → 54 (666699), 123456 → 56 (003366), ff0000 → 10
        assert_eq!(nearest_icv(0x6182D6), 54);
        assert_eq!(nearest_icv(0x123456), 56);
        assert_eq!(nearest_icv(0xFF0000), 10);
        assert_eq!(nearest_icv(0x000000), 8);
    }

    #[test]
    fn virtual_paths_round_trip() {
        let p = encode_virtual_path("file:///C:/a%20b/c.xls", false);
        assert_eq!(p, "\u{1}\u{1}Ca b\u{3}c.xls");
        assert_eq!(
            encode_virtual_path("RecoveredExternalLink1", true),
            "RecoveredExternalLink1"
        );
    }
}
