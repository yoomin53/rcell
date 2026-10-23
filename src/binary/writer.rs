// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! [`Workbook`] → 바이너리 `.cell` (한셀 저장 형식 `xlCellb`, -2007).
//!
//! 한셀이 `SaveAs -2007` 로 쓰는 패키지를 따른다: 파트 이름·콘텐츠 타입·관계, 파트마다 레코드의 종류와
//! 순서, 한셀이 늘 같게 쓰는 고정값 (관측: 코퍼스 114개 문서의 `X.cell` → `X.bin.cell` 대조,
//! docs/format/cell-binary.md 4절). 문서에 드러나지 않는 한셀 내부 상태가 들어가는 칸(보조 확대율 등)은
//! 가장 흔한 값을 쓴다.
//!
//! 한컴 확장(장평·문단 모양 …)은 형식에 자리가 없어 쓰지 않는다 (한셀도 쓰지 않는다).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write};

use zip::write::{SimpleFileOptions, ZipWriter};

use super::formula::{self, Encoded, Place, Resolver};
use crate::error::Result;
use crate::model::{
    Border, Cell, CellRange, CellRef, CellValue, CfRule, Cfvo, Color, Comment, ConditionalFormat,
    DataValidation, Dxf, FILTER_OPERATORS, Fill, Font, FormulaKind, RichText, Sheet, SheetKind,
    SheetState, Styles, Workbook, Xf,
};
use crate::writer::esc;

/// 쓰기 옵션
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `BrtAbsPath15` 에 쓸 저장 폴더 (한셀은 저장한 폴더의 절대 경로를 쓴다, 끝에 `\`).
    /// 없으면 빈 문자열 — 개인정보 보호 (XML 쓰기가 `x15ac:absPath` 를 쓰지 않는 것과 같다)
    pub abs_path: Option<String>,
    /// 한셀의 바이너리 저장이 잃는 값도 똑같이 잃는다 (차등 서식의 맞춤·표시 형식, `draft`·자동 페이지 나누기 끔,
    /// 요약 열 왼쪽, 구조 잠금 없는 통합 문서 보호, 나눈 창의 활성 창 — docs/format/cell-binary.md 2.1).
    /// 기본은 잃지 않는다 (한셀은 rcell 이 더 쓴 값을 그대로 읽는다)
    pub exact: bool,
}

/// 쓴 결과와, 쓰면서 버린 것 (토큰으로 바꾸지 못한 수식 등)
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// 통합 문서를 바이너리 `.cell` 바이트로 쓴다.
pub fn write(wb: &Workbook, opts: &Options) -> Result<Vec<u8>> {
    Ok(write_report(wb, opts)?.bytes)
}

// ---------------------------------------------------------------------------
// 레코드
// ---------------------------------------------------------------------------

/// BIFF12 레코드열 ([MS-XLSB] 2.1.4: 종류·길이를 7비트 가변 길이로)
#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn rec(&mut self, t: u16, data: &[u8]) {
        if t < 0x80 {
            self.0.push(t as u8);
        } else {
            self.0.push((t & 0x7F) as u8 | 0x80);
            self.0.push((t >> 7) as u8);
        }
        let mut n = data.len();
        loop {
            let b = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                self.0.push(b);
                break;
            }
            self.0.push(b | 0x80);
        }
        self.0.extend_from_slice(data);
    }
    fn e(&mut self, t: u16) {
        self.rec(t, &[]);
    }
    fn p(&mut self, t: u16, p: &P) {
        self.rec(t, &p.0);
    }
}

/// 레코드 내용
#[derive(Default)]
struct P(Vec<u8>);

impl P {
    fn new() -> Self {
        P(Vec::new())
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn raw(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    /// XLWideString: cch u32 + UTF-16
    fn wstr(&mut self, s: &str) -> &mut Self {
        let u: Vec<u16> = s.encode_utf16().collect();
        self.u32(u.len() as u32);
        for x in u {
            self.u16(x);
        }
        self
    }
    /// XLNullableWideString: 없으면 cch 0xFFFFFFFF
    fn nwstr(&mut self, s: Option<&str>) -> &mut Self {
        match s {
            Some(s) => self.wstr(s),
            None => self.u32(0xFFFF_FFFF),
        }
    }
    /// RfX: rwFirst, rwLast, colFirst, colLast
    fn rfx(&mut self, r: CellRange) -> &mut Self {
        self.u32(r.start.row)
            .u32(r.end.row)
            .u32(r.start.col)
            .u32(r.end.col)
    }
    /// UncheckedSqRfX: crfx u32 + RfX × crfx
    fn sqref(&mut self, s: &str) -> &mut Self {
        let v = ranges(s);
        self.u32(v.len() as u32);
        for r in v {
            self.rfx(r);
        }
        self
    }
    /// CellParsedFormula / NameParsedFormula / CFParsedFormula: cce u32 + rgce + cb u32 + rgcb
    fn fmla(&mut self, e: &Encoded) -> &mut Self {
        self.u32(e.rgce.len() as u32)
            .raw(&e.rgce)
            .u32(e.rgcb.len() as u32)
            .raw(&e.rgcb)
    }
}

/// 공백으로 구분한 범위 목록 → 범위들 (왼쪽 위 → 오른쪽 아래로 맞춘다)
fn ranges(s: &str) -> Vec<CellRange> {
    s.split_whitespace()
        .filter_map(CellRange::parse)
        .map(|r| CellRange {
            start: CellRef {
                row: r.start.row.min(r.end.row),
                col: r.start.col.min(r.end.col),
            },
            end: CellRef {
                row: r.start.row.max(r.end.row),
                col: r.start.col.max(r.end.col),
            },
        })
        .collect()
}

fn first_cell(sqref: &str) -> CellRef {
    ranges(sqref).first().map(|r| r.start).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 색
// ---------------------------------------------------------------------------

fn rgb_of(c: &Color) -> Option<[u8; 3]> {
    let s = c.rgb.as_deref()?;
    let s = s.trim().trim_start_matches('#');
    let s = if s.len() >= 6 { &s[s.len() - 6..] } else { s };
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn tint16(t: Option<f64>) -> [u8; 2] {
    let v = t.map_or(0, |t| (t * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
    v.to_le_bytes()
}

/// BrtColor (8바이트): `fValidRGB + 종류<<1, icv, 명암 i16, R, G, B, A`.
/// 한셀은 RGB 색의 icv 칸에 쓰임새 번호(글꼴 0, 채우기 무늬 0x40, 배경 0x41)를 넣는다 (관측).
/// 자동은 `01 00 00 00 FF FF FF FF`, 색인 64(시스템 배경)는 `03 40 00 00 FF FF FF DF` (한셀의 자동 색 표지)
fn brt_color(c: &Color, icv: u8) -> [u8; 8] {
    if c.auto {
        return [0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let t = tint16(c.tint);
    if let Some(th) = c.theme {
        let [r, g, b] = crate::theme::resolve(c, &crate::theme::Theme::default())
            .and_then(|h| {
                let v = u32::from_str_radix(h.trim_start_matches('#'), 16).ok()?;
                Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
            })
            .unwrap_or([0, 0, 0]);
        return [0x07, th as u8, t[0], t[1], r, g, b, 0xFF];
    }
    if let Some(i) = c.indexed.filter(|_| c.rgb.is_none()) {
        if i == 64 {
            return [0x03, 0x40, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF];
        }
        let [r, g, b] = crate::theme::resolve(c, &crate::theme::Theme::default())
            .and_then(|h| {
                let v = u32::from_str_radix(h.trim_start_matches('#'), 16).ok()?;
                Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
            })
            .unwrap_or([0, 0, 0]);
        return [0x03, i as u8, t[0], t[1], r, g, b, 0xFF];
    }
    let [r, g, b] = rgb_of(c).unwrap_or([0, 0, 0]);
    [0x05, icv, t[0], t[1], r, g, b, 0xFF]
}

/// 조건부 서식의 BrtColor: 한셀은 icv 칸에 빨강 값을 넣고, 흰색은 자동 종류로 쓴다 (관측: 색 눈금·데이터 막대)
fn cf_color(c: &Color) -> [u8; 8] {
    if c.theme.is_some() || (c.indexed.is_some() && c.rgb.is_none()) {
        return brt_color(c, 0);
    }
    if c.auto {
        return [0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let [r, g, b] = rgb_of(c).unwrap_or([0, 0, 0]);
    if [r, g, b] == [0xFF, 0xFF, 0xFF] {
        return [0x01, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    let t = tint16(c.tint);
    [0x05, r, t[0], t[1], r, g, b, 0xFF]
}

/// 차등 서식의 XFPropColor (BrtColor 와 같은 꼴). 자동은 한셀 표지 `01 00 00 00 FF FF FF DF`
fn xfprop_color(c: &Color) -> [u8; 8] {
    if c.auto {
        return [0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF];
    }
    brt_color(c, 0)
}

fn color_eq(a: Option<&Color>, b: Option<&Color>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.auto == y.auto
                && x.theme == y.theme
                && x.indexed == y.indexed
                && x.tint.unwrap_or(0.0) == y.tint.unwrap_or(0.0)
                && rgb_of(x) == rgb_of(y)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// 스타일 (xl/styles.bin)
// ---------------------------------------------------------------------------

const H_ALIGN: [&str; 8] = [
    "general",
    "left",
    "center",
    "right",
    "fill",
    "justify",
    "centerContinuous",
    "distributed",
];
const V_ALIGN: [&str; 5] = ["top", "center", "bottom", "justify", "distributed"];
const BORDER_STYLES: [&str; 14] = [
    "none",
    "thin",
    "medium",
    "dashed",
    "dotted",
    "thick",
    "double",
    "hair",
    "mediumDashed",
    "dashDot",
    "mediumDashDot",
    "dashDotDot",
    "mediumDashDotDot",
    "slantDashDot",
];
const PATTERNS: [&str; 19] = [
    "none",
    "solid",
    "mediumGray",
    "darkGray",
    "lightGray",
    "darkHorizontal",
    "darkVertical",
    "darkDown",
    "darkUp",
    "darkGrid",
    "darkTrellis",
    "lightHorizontal",
    "lightVertical",
    "lightDown",
    "lightUp",
    "lightGrid",
    "lightTrellis",
    "gray125",
    "gray0625",
];

fn index_of(list: &[&str], v: Option<&str>) -> Option<usize> {
    let v = v?;
    list.iter().position(|x| x.eq_ignore_ascii_case(v))
}

fn underline_code(u: Option<&str>) -> u8 {
    match u {
        None | Some("none") => 0,
        Some("double") => 2,
        Some("singleAccounting") => 0x21,
        Some("doubleAccounting") => 0x22,
        _ => 1,
    }
}

fn font_eq(a: &Font, b: &Font) -> bool {
    a.name == b.name
        && a.size.unwrap_or(11.0) == b.size.unwrap_or(11.0)
        && a.bold == b.bold
        && a.italic == b.italic
        && underline_code(a.underline.as_deref()) == underline_code(b.underline.as_deref())
        && a.strike == b.strike
        && a.outline == b.outline
        && a.shadow == b.shadow
        && a.condense == b.condense
        && a.extend == b.extend
        && a.vert_align == b.vert_align
        && color_eq(a.color.as_ref(), b.color.as_ref())
        && a.family.unwrap_or(2) == b.family.unwrap_or(2)
        && a.charset.unwrap_or(0x81) == b.charset.unwrap_or(0x81)
        && a.scheme == b.scheme
}

/// 쓸 글꼴 표: 문서의 글꼴 + 서식 있는 글·메모의 글꼴 중 표에 없는 것
struct FontTable {
    fonts: Vec<Font>,
}

impl FontTable {
    fn new(st: &Styles) -> Self {
        let fonts = if st.fonts.is_empty() {
            vec![Font {
                name: Some("맑은 고딕".into()),
                size: Some(11.0),
                color: Some(Color {
                    rgb: Some("ff000000".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }]
        } else {
            st.fonts.clone()
        };
        FontTable { fonts }
    }

    /// 글꼴의 번호 (같은 글꼴이 없으면 덧붙인다). 글꼴이 없는 조각은 0번
    fn index(&mut self, f: Option<&Font>) -> u16 {
        let Some(f) = f else { return 0 };
        if let Some(i) = self.fonts.iter().position(|x| font_eq(x, f)) {
            return i as u16;
        }
        self.fonts.push(f.clone());
        (self.fonts.len() - 1) as u16
    }
}

fn font_record(f: &Font) -> P {
    let mut p = P::new();
    let mut grbit = 0u16;
    if f.italic {
        grbit |= 0x02;
    }
    if f.strike {
        grbit |= 0x08;
    }
    if f.outline {
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
    let sss = match f.vert_align.as_deref() {
        Some("superscript") => 1,
        Some("subscript") => 2,
        _ => 0,
    };
    let color = match &f.color {
        Some(c) => brt_color(c, 0),
        None => [0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
    };
    p.u16((f.size.unwrap_or(11.0) * 20.0).round() as u16)
        .u16(grbit)
        .u16(if f.bold { 700 } else { 400 })
        .u16(sss)
        .u8(underline_code(f.underline.as_deref()))
        // 한셀은 글꼴 계열·문자 집합이 없는 글꼴에 2(Swiss)와 0x81(한글)을 쓴다 (관측)
        .u8(f.family.unwrap_or(2) as u8)
        .u8(f.charset.unwrap_or(0x81) as u8)
        .u8(0)
        .raw(&color)
        .u8(match f.scheme.as_deref() {
            Some("major") => 1,
            Some("minor") => 2,
            _ => 0,
        })
        .wstr(f.name.as_deref().unwrap_or("맑은 고딕"));
    p
}

/// BrtFill: fls u32, 무늬 색, 배경색, 그라데이션 칸들 (무늬 채우기는 0)
fn fill_record(f: &Fill) -> P {
    let mut p = P::new();
    if let Some(g) = &f.gradient {
        // 그라데이션의 무늬 색·배경색 칸: 자동 표지 (관측: 그라데이션 대조 실험)
        p.u32(0x28)
            .raw(&[0x01, 0x40, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF])
            .raw(&[0x01, 0x41, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF])
            .u32(u32::from(g.path))
            .f64(g.degree)
            .f64(g.left)
            .f64(g.right)
            .f64(g.top)
            .f64(g.bottom)
            .u32(g.stops.len() as u32);
        for s in &g.stops {
            p.raw(&brt_color(&s.color, 0)).f64(s.position);
        }
        return p;
    }
    let fls = index_of(&PATTERNS, f.pattern_type.as_deref()).unwrap_or(0) as u32;
    // 무늬 색이 없으면 검정 RGB(icv 0x40), 배경색이 없으면 자동(icv 0x41) — 관측
    let fg = f
        .fg_color
        .as_ref()
        .map_or([0x05, 0x40, 0, 0, 0, 0, 0, 0xFF], |c| brt_color(c, 0x40));
    let bg = f
        .bg_color
        .as_ref()
        .map_or([0x01, 0x41, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], |c| {
            match c.indexed {
                Some(64) if c.rgb.is_none() => [0x03, 0x40, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF],
                _ => brt_color(c, 0x41),
            }
        });
    p.u32(fls).raw(&fg).raw(&bg).raw(&[0; 48]);
    p
}

/// BrtBorder: 플래그 u8 (0x01 대각선 아래, 0x02 대각선 위), 위·아래·왼쪽·오른쪽·대각선 (dg u8, 예약 u8, BrtColor).
/// 선의 자동 색은 한셀의 자동 색 표지 `01 00 00 00 FF FF FF DF` (관측: 서식 파일 683개·조작 문서 33개 선,
/// 새 문서 15개 선만 `FF`), 선 없음은 RGB 색 그대로이고 자동·없음이면 검정 RGB (관측: 1101/1239).
/// `exact` 면 색인 65(시스템 전경 — 피벗 서식의 테두리)를 한셀처럼 RGB `02 00 80` 으로 쓴다 (한셀은 레거시 COLORREF
/// 0x80800002 의 아래 세 바이트를 RGB 로 옮긴다 — 피벗 실험 pa 의 테두리 6개)
fn border_record(b: &Border, exact: bool) -> P {
    const AUTO: [u8; 8] = [0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF];
    let mut p = P::new();
    p.u8(u8::from(b.diagonal_down) | (u8::from(b.diagonal_up) << 1));
    for e in [&b.top, &b.bottom, &b.left, &b.right, &b.diagonal] {
        let style = e.as_ref().and_then(|e| {
            index_of(&BORDER_STYLES, e.style.as_deref())
                .filter(|&i| i != 0)
                .map(|i| (i, e))
        });
        match style {
            Some((i, e)) => {
                let c = match &e.color {
                    Some(c) if c.auto => AUTO,
                    Some(c) if exact && c.indexed == Some(65) && c.rgb.is_none() => {
                        [0x05, 0x00, 0, 0, 0x02, 0x00, 0x80, 0xFF]
                    }
                    Some(c) => brt_color(c, 0x00),
                    None => AUTO,
                };
                p.u8(i as u8).u8(0).raw(&c);
            }
            None => {
                let c = match e.as_ref().and_then(|e| e.color.as_ref()) {
                    Some(c) if c.rgb.is_some() && !c.auto => brt_color(c, 0x00),
                    _ => [0x05, 0, 0, 0, 0, 0, 0, 0xFF],
                };
                p.u8(0).u8(0).raw(&c);
            }
        }
    }
    p
}

/// BrtXF: 부모 u16, 표시 형식 u16, 글꼴 u16, 채우기 u16, 테두리 u16, 회전 u8, 들여쓰기 u8, 플래그 u16,
/// 적용 비트 u8, 0
fn xf_record(x: &Xf, style_xf: bool) -> P {
    let mut p = P::new();
    let a = x.alignment.as_ref();
    let alc = a
        .and_then(|a| index_of(&H_ALIGN, a.horizontal.as_deref()))
        .unwrap_or(0) as u16;
    let alcv = a
        .and_then(|a| index_of(&V_ALIGN, a.vertical.as_deref()))
        .unwrap_or(2) as u16;
    let mut flags = alc | (alcv << 3);
    if a.is_some_and(|a| a.wrap_text) {
        flags |= 0x40;
    }
    if a.is_some_and(|a| a.shrink_to_fit) {
        flags |= 0x100;
    }
    if let Some(o) = a.and_then(|a| a.reading_order) {
        flags |= ((o as u16) & 0x03) << 10;
    }
    let prot = x.protection.as_ref();
    if prot.and_then(|p| p.locked).unwrap_or(true) {
        flags |= 0x1000;
    }
    if prot.and_then(|p| p.hidden).unwrap_or(false) {
        flags |= 0x2000;
    }
    if x.quote_prefix {
        flags |= 0x8000;
    }
    let mut atr = 0u8;
    for (bit, v) in [
        (0x01, x.apply_number_format),
        (0x02, x.apply_font),
        (0x04, x.apply_alignment),
        (0x08, x.apply_border),
        (0x10, x.apply_fill),
        (0x20, x.apply_protection),
    ] {
        if v == Some(true) {
            atr |= bit;
        }
    }
    let parent = if style_xf {
        0xFFFF
    } else {
        x.xf_id.unwrap_or(0) as u16
    };
    let trot = a
        .and_then(|a| a.text_rotation)
        .map_or(0, |r| r.clamp(0, 255) as u8);
    let indent = a.and_then(|a| a.indent).map_or(0, |i| i.min(255) as u8);
    p.u16(parent)
        .u16(x.num_fmt_id as u16)
        .u16(x.font_id as u16)
        .u16(x.fill_id as u16)
        .u16(x.border_id as u16)
        .u8(trot)
        .u8(indent)
        .u16(flags)
        .u8(atr)
        .u8(0);
    p
}

/// 차등 서식의 XFProp 하나: 종류 u16, 크기 u16 (머리 4바이트 포함), 값
fn xfprop(out: &mut Vec<u8>, ty: u16, v: &[u8]) {
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&((v.len() + 4) as u16).to_le_bytes());
    out.extend_from_slice(v);
}

/// BrtDXF: 플래그 u16 (0x8000 fNewBorder — 한셀은 위·아래·왼쪽·오른쪽을 모두 지정한 테두리에 켠다), 예약 u16,
/// XFProp 수 u16, XFProp 들 (채우기, 글꼴, 테두리, 보호 순, 묶음 안은 번호 순 — 관측). 표시 형식·맞춤은
/// 한셀도 바이너리에 쓰지 않는다 (docs/format/cell-binary.md 2.1)
fn dxf_record(d: &Dxf, fmt_id: Option<u16>, exact: bool) -> P {
    let mut props = Vec::new();
    let mut n = 0u16;
    let mut add = |props: &mut Vec<u8>, ty: u16, v: &[u8]| {
        xfprop(props, ty, v);
        n += 1;
    };
    if let Some(f) = &d.fill {
        if let Some(g) = &f.gradient {
            add(&mut props, 0x00, &[0x28]);
            let mut v = Vec::new();
            v.extend_from_slice(&u32::from(g.path).to_le_bytes());
            for x in [g.degree, g.left, g.right, g.top, g.bottom] {
                v.extend_from_slice(&x.to_le_bytes());
            }
            add(&mut props, 0x03, &v);
            for s in &g.stops {
                let mut v = vec![0, 0];
                v.extend_from_slice(&s.position.to_le_bytes());
                v.extend_from_slice(&xfprop_color(&s.color));
                add(&mut props, 0x04, &v);
            }
        } else {
            if let Some(i) = index_of(&PATTERNS, f.pattern_type.as_deref()) {
                add(&mut props, 0x00, &[i as u8]);
            }
            if let Some(c) = &f.fg_color {
                add(&mut props, 0x01, &xfprop_color(c));
            }
            if let Some(c) = &f.bg_color {
                add(&mut props, 0x02, &xfprop_color(c));
            }
        }
    }
    if let Some(f) = &d.font {
        if let Some(c) = &f.color {
            add(&mut props, 0x05, &xfprop_color(c));
        }
        // 속성이 하나도 없는 글꼴(`<x:font/>`)은 한셀처럼 글꼴 이름(0x18)으로 자리를 남긴다 (한셀은 깨진 문자열,
        // rcell 은 빈 이름)
        let bare = f.name.is_none()
            && f.color.is_none()
            && f.size.is_none()
            && !(f.bold || f.italic || f.strike || f.outline || f.shadow || f.condense || f.extend)
            && f.underline.is_none()
            && f.vert_align.is_none()
            && f.charset.is_none()
            && f.family.is_none()
            && f.scheme.is_none();
        if let Some(name) = f.name.as_deref().or(bare.then_some("")) {
            let u: Vec<u16> = name.encode_utf16().collect();
            let mut v = (u.len() as u16).to_le_bytes().to_vec();
            for x in u {
                v.extend_from_slice(&x.to_le_bytes());
            }
            add(&mut props, 0x18, &v);
        }
        if f.bold {
            add(&mut props, 0x19, &700u16.to_le_bytes());
        }
        if f.underline.is_some() {
            add(
                &mut props,
                0x1A,
                &u16::from(underline_code(f.underline.as_deref())).to_le_bytes(),
            );
        }
        match f.vert_align.as_deref() {
            Some("superscript") => add(&mut props, 0x1B, &1u16.to_le_bytes()),
            Some("subscript") => add(&mut props, 0x1B, &2u16.to_le_bytes()),
            _ => {}
        }
        for (on, ty) in [
            (f.italic, 0x1C),
            (f.strike, 0x1D),
            (f.outline, 0x1E),
            (f.shadow, 0x1F),
            (f.condense, 0x20),
            (f.extend, 0x21),
        ] {
            if on {
                add(&mut props, ty, &[1]);
            }
        }
        if let Some(c) = f.charset {
            add(&mut props, 0x22, &[c as u8]);
        }
        if let Some(fam) = f.family {
            add(&mut props, 0x23, &[fam as u8]);
        }
        if let Some(sz) = f.size {
            add(
                &mut props,
                0x24,
                &((sz * 20.0).round() as u32).to_le_bytes(),
            );
        }
        match f.scheme.as_deref() {
            Some("major") => add(&mut props, 0x25, &[1]),
            Some("minor") => add(&mut props, 0x25, &[2]),
            _ => {}
        }
    }
    let mut new_border = false;
    if let Some(b) = &d.border {
        new_border = b.left.is_some() && b.right.is_some() && b.top.is_some() && b.bottom.is_some();
        for (ty, e) in [
            (0x06, &b.top),
            (0x07, &b.bottom),
            (0x08, &b.left),
            (0x09, &b.right),
            (0x0A, &b.diagonal),
            (0x0B, &b.vertical),
            (0x0C, &b.horizontal),
        ] {
            if let Some(e) = e {
                let dg = index_of(&BORDER_STYLES, e.style.as_deref()).unwrap_or(0) as u16;
                let c = e
                    .color
                    .as_ref()
                    .map_or([0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xDF], xfprop_color);
                let mut v = c.to_vec();
                v.extend_from_slice(&dg.to_le_bytes());
                add(&mut props, ty, &v);
            }
        }
        if b.diagonal_up {
            add(&mut props, 0x0D, &[1]);
        }
        if b.diagonal_down {
            add(&mut props, 0x0E, &[1]);
        }
    }
    // 맞춤: 한셀은 바이너리 차등 서식에 쓰지 않아 잃는다 (docs/format/cell-binary.md 2.1) — rcell 은 쓴다 (한셀도 읽는다,
    // 관측: dxfprobe2). `exact` 면 한셀처럼 버린다
    if let Some(a) = d.alignment.as_ref().filter(|_| !exact) {
        if let Some(h) = index_of(&H_ALIGN, a.horizontal.as_deref()) {
            add(&mut props, 0x0F, &[h as u8]);
        }
        if let Some(v) = index_of(&V_ALIGN, a.vertical.as_deref()) {
            add(&mut props, 0x10, &[v as u8]);
        }
        if let Some(r) = a.text_rotation {
            add(&mut props, 0x11, &[r.clamp(0, 255) as u8]);
        }
        if let Some(i) = a.indent {
            add(&mut props, 0x12, &(i.min(255) as u16).to_le_bytes());
        }
        if let Some(o) = a.reading_order {
            add(&mut props, 0x13, &[o as u8]);
        }
        if a.wrap_text {
            add(&mut props, 0x14, &[1]);
        }
        if a.shrink_to_fit {
            add(&mut props, 0x16, &[1]);
        }
    }
    // 표시 형식: 번호(0x29)만, 사용자 정의 번호를 BrtFmt 표에 올린다 ([`dxf_formats`]). 한셀의 바이너리 읽기는 형식 문자열
    // (0x26)을 빈 형식으로, 내장 번호를 다른 형식으로 읽고, 표에 올린 사용자 번호는 무시한다 (차등 서식 표시 형식 대조
    // 실험) — 한셀에는 해가 없고 rcell 은 다시 읽는다
    if let Some(id) = fmt_id.filter(|_| !exact) {
        add(&mut props, 0x29, &id.to_le_bytes());
    }
    if let Some(pr) = &d.protection {
        if let Some(l) = pr.locked {
            add(&mut props, 0x2B, &[u8::from(l)]);
        }
        // 한셀은 값 없는 x:protection 을 숨김 0 으로 쓴다 (관측: 차등 서식 대조 실험)
        add(&mut props, 0x2C, &[u8::from(pr.hidden.unwrap_or(false))]);
    }
    let mut p = P::new();
    p.u16(if new_border { 0x8000 } else { 0 })
        .u16(0)
        .u16(n)
        .raw(&props);
    p
}

/// 차등 서식의 표시 형식 → (BrtFmt 표, 차등 서식마다의 번호). 같은 형식 문자열이 표에 있으면 그 번호, 없으면 사용자 정의
/// 번호(164 이상)를 새로 준다. `exact` 면 차등 서식의 표시 형식을 쓰지 않는다 (한셀처럼)
fn dxf_formats(st: &Styles, exact: bool) -> (Vec<crate::model::NumFmt>, Vec<Option<u16>>) {
    let mut fmts = st.num_fmts.clone();
    let ids = st
        .dxfs
        .iter()
        .map(|d| {
            let n = d.num_fmt.as_ref().filter(|_| !exact)?;
            if let Some(f) = fmts.iter().find(|f| f.code == n.code && f.id >= 164) {
                return Some(f.id as u16);
            }
            let id = fmts.iter().map(|f| f.id).max().unwrap_or(163).max(163) + 1;
            fmts.push(crate::model::NumFmt {
                id,
                code: n.code.clone(),
            });
            Some(id as u16)
        })
        .collect();
    (fmts, ids)
}

fn styles_bin(st: &Styles, fonts: &FontTable, exact: bool) -> Vec<u8> {
    let mut o = Out::default();
    o.e(278); // BrtBeginStyleSheet
    let (num_fmts, dxf_fmt) = dxf_formats(st, exact);
    if !num_fmts.is_empty() {
        o.p(615, P::new().u32(num_fmts.len() as u32));
        for n in &num_fmts {
            o.p(44, P::new().u16(n.id as u16).wstr(&n.code));
        }
        o.e(616);
    }
    o.p(611, P::new().u32(fonts.fonts.len() as u32));
    for f in &fonts.fonts {
        o.p(43, &font_record(f));
    }
    o.e(612);
    let fills: Vec<Fill> = if st.fills.is_empty() {
        vec![
            Fill {
                pattern_type: Some("none".into()),
                ..Default::default()
            },
            Fill {
                pattern_type: Some("gray125".into()),
                ..Default::default()
            },
        ]
    } else {
        st.fills.clone()
    };
    o.p(603, P::new().u32(fills.len() as u32));
    for f in &fills {
        o.p(45, &fill_record(f));
    }
    o.e(604);
    let borders: Vec<Border> = if st.borders.is_empty() {
        vec![Border::default()]
    } else {
        st.borders.clone()
    };
    o.p(613, P::new().u32(borders.len() as u32));
    for b in &borders {
        o.p(46, &border_record(b, exact));
    }
    o.e(614);
    let style_xfs: Vec<Xf> = if st.cell_style_xfs.is_empty() {
        vec![Xf::default()]
    } else {
        st.cell_style_xfs.clone()
    };
    o.p(626, P::new().u32(style_xfs.len() as u32));
    for x in &style_xfs {
        o.p(47, &xf_record(x, true));
    }
    o.e(627);
    let cell_xfs: Vec<Xf> = if st.cell_xfs.is_empty() {
        vec![Xf::default()]
    } else {
        st.cell_xfs.clone()
    };
    o.p(617, P::new().u32(cell_xfs.len() as u32));
    for x in &cell_xfs {
        o.p(47, &xf_record(x, false));
    }
    o.e(618);
    // BrtStyle: 한셀은 모든 스타일에 fBuiltIn 을 켜고, 사용자 정의 스타일에는 앞 스타일의 내장 번호를 남긴다 (관측)
    let styles: Vec<(String, u32, Option<u32>, Option<u32>)> = if st.cell_styles.is_empty() {
        vec![("표준".into(), 0, Some(0), None)]
    } else {
        st.cell_styles
            .iter()
            .map(|c| (c.name.clone(), c.xf_id, c.builtin_id, c.i_level))
            .collect()
    };
    o.p(619, P::new().u32(styles.len() as u32));
    let mut last_builtin = 0u8;
    for (name, xf, builtin, level) in &styles {
        if let Some(b) = builtin {
            last_builtin = *b as u8;
        }
        o.p(
            48,
            P::new()
                .u32(*xf)
                .u16(0x0001)
                .u8(last_builtin)
                .u8(level.map_or(0xFF, |l| l as u8))
                .wstr(name),
        );
    }
    o.e(620);
    o.p(505, P::new().u32(st.dxfs.len() as u32));
    for (d, id) in st.dxfs.iter().zip(dxf_fmt) {
        o.p(507, &dxf_record(d, id, exact));
    }
    o.e(506);
    // 표 스타일 묶음: 한셀 문서에는 늘 있다 (기본 이름만이라도). 모델에 없으면 쓰지 않는다
    if let Some(t) = &st.table_styles {
        table_styles(&mut o, t);
    }
    o.e(279);
    o.0
}

fn table_styles(o: &mut Out, t: &crate::model::TableStyles) {
    let list = t.styles.as_slice();
    o.p(
        508,
        P::new()
            .u32(list.len() as u32)
            .wstr(
                t.default_table_style
                    .as_deref()
                    .unwrap_or("TableStyleMedium2"),
            )
            .wstr(
                t.default_pivot_style
                    .as_deref()
                    .unwrap_or("PivotStyleLight16"),
            ),
    );
    for ts in list {
        let flags = (u16::from(ts.pivot) << 1) | (u16::from(ts.table) << 2);
        o.p(
            510,
            P::new()
                .u16(flags)
                .u32(ts.elements.len() as u32)
                .wstr(&ts.name),
        );
        let mut els: Vec<_> = ts
            .elements
            .iter()
            .filter_map(|e| {
                crate::model::TABLE_STYLE_TYPES
                    .iter()
                    .position(|k| *k == e.kind)
                    .map(|t| (t, e))
            })
            .collect();
        els.sort_by_key(|(t, _)| *t);
        for (t, e) in els {
            o.p(
                512,
                P::new()
                    .u32(t as u32)
                    .u32(e.size)
                    .u32(e.dxf_id.unwrap_or(0)),
            );
        }
        o.e(511);
    }
    o.e(509);
}

/// RichStr: 플래그 u8 (0x01 서식 조각), XLWideString, 조각 수 u32 + (시작 글자 u16, 글꼴 u16) × n
fn rich_str(p: &mut P, t: &RichText, fonts: &mut FontTable) {
    if t.runs.is_empty() {
        p.u8(0).wstr(&t.text);
        return;
    }
    let text: String = t.runs.iter().map(|r| r.text.as_str()).collect();
    p.u8(0x01).wstr(&text);
    let mut runs = Vec::new();
    let mut ich = 0usize;
    for r in &t.runs {
        if !r.text.is_empty() {
            runs.push((ich as u16, fonts.index(r.font.as_ref())));
        }
        ich += r.text.encode_utf16().count();
    }
    p.u32(runs.len() as u32);
    for (i, f) in runs {
        p.u16(i).u16(f);
    }
}

// ---------------------------------------------------------------------------
// 공유 문자열
// ---------------------------------------------------------------------------

/// 원본 공유 문자열 순서를 지키고, 없는 글은 뒤에 붙인다 (한셀의 바이너리 저장도 XML 의 순서를 지킨다, 관측)
struct Sst {
    items: Vec<RichText>,
    index: HashMap<String, u32>,
    refs: u32,
    /// 원본의 참조 수 (한셀은 XML 의 count 를 cstTotal 로 옮긴다, 관측)
    count: Option<u32>,
}

impl Sst {
    fn new(wb: &Workbook) -> Self {
        let mut index = HashMap::new();
        for (i, s) in wb.shared_strings.iter().enumerate() {
            index.entry(s.text.clone()).or_insert(i as u32);
        }
        Sst {
            items: wb.shared_strings.clone(),
            index,
            refs: 0,
            count: wb.shared_strings_count,
        }
    }
    fn get(&mut self, text: &str) -> u32 {
        self.refs += 1;
        if let Some(&i) = self.index.get(text) {
            return i;
        }
        let i = self.items.len() as u32;
        self.items.push(RichText {
            text: text.to_string(),
            runs: Vec::new(),
        });
        self.index.insert(text.to_string(), i);
        i
    }
    fn bin(&self, fonts: &mut FontTable) -> Vec<u8> {
        let mut o = Out::default();
        o.p(
            159,
            P::new()
                .u32(self.count.unwrap_or(self.refs))
                .u32(self.items.len() as u32),
        );
        for it in &self.items {
            let mut p = P::new();
            rich_str(&mut p, it, fonts);
            o.p(19, &p);
        }
        o.e(160);
        o.0
    }
}

// ---------------------------------------------------------------------------
// 수식 문맥: 시트·외부 연결(XTI)과 이름 표
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupBook {
    /// 이 통합 문서 (BrtSupSelf)
    This,
    /// 외부 연결 (0부터, BrtSupBookSrc)
    Ext(usize),
}

struct NameEntry {
    /// 모델의 이름 (`_xlnm.` 포함)
    name: String,
    local: Option<u32>,
    hidden: bool,
    formula: Option<String>,
    /// 사용자 정의/미래 함수 이름
    func: bool,
}

struct Book<'a> {
    sheets: Vec<&'a str>,
    links: &'a [crate::model::ExternalLink],
    /// XTI: (연결, 첫 시트, 끝 시트). 연결 번호는 다 모은 뒤에 정한다 ([`Book::supbooks`])
    xtis: Vec<(SupBook, i32, i32)>,
    names: Vec<NameEntry>,
    /// 지금 수식을 쓰는 시트 (지역 이름 찾기)
    cur: Option<u32>,
    /// 표: (번호, 표시 이름, 이름, 열 이름, 시트 이름) — 구조적 참조
    tables: Vec<(u32, String, String, Vec<String>, String)>,
}

impl Book<'_> {
    fn sheet_index(list: &[&str], name: &str) -> Option<i32> {
        list.iter()
            .position(|s| s.eq_ignore_ascii_case(name))
            .map(|i| i as i32)
    }

    /// 연결 목록 (BrtSupBookSrc·BrtSupSelf 순서): 외부 연결은 [n] 번호 순서 그대로 (읽는 쪽이 BrtSupBookSrc
    /// 순서로 번호를 매긴다), 이 통합 문서는 쓰일 때만 첫 외부 연결 바로 뒤에 (관측: 한셀 서식 파일의
    /// [외부 1, 자신], [외부 1, 자신, 외부 2], [외부 1, 외부 2], [자신])
    fn supbooks(&self) -> Vec<SupBook> {
        let mut v: Vec<SupBook> = (0..self.links.len()).map(SupBook::Ext).collect();
        if self.xtis.iter().any(|x| x.0 == SupBook::This) {
            v.insert(v.len().min(1), SupBook::This);
        }
        v
    }
}

impl Resolver for Book<'_> {
    fn xti(&mut self, book: Option<usize>, first: &str, last: Option<&str>) -> Option<u16> {
        let (sb, f, l) = match book {
            None => {
                let f = Self::sheet_index(&self.sheets, first)?;
                let l = match last {
                    Some(x) => Self::sheet_index(&self.sheets, x)?,
                    None => f,
                };
                (SupBook::This, f, l)
            }
            Some(n) => {
                let link = self.links.get(n.checked_sub(1)?)?;
                let names: Vec<&str> = link.sheet_names.iter().map(String::as_str).collect();
                let f = Self::sheet_index(&names, first)?;
                let l = match last {
                    Some(x) => Self::sheet_index(&names, x)?,
                    None => f,
                };
                (SupBook::Ext(n - 1), f, l)
            }
        };
        let key = (sb, f.min(l), f.max(l));
        Some(match self.xtis.iter().position(|x| *x == key) {
            Some(i) => i as u16,
            None => {
                self.xtis.push(key);
                (self.xtis.len() - 1) as u16
            }
        })
    }

    fn name(&mut self, sheet: Option<&str>, name: &str) -> u32 {
        let same = |e: &NameEntry| {
            e.name.eq_ignore_ascii_case(name)
                || e.name
                    .strip_prefix("_xlnm.")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        };
        let scope = match sheet {
            Some(s) => Self::sheet_index(&self.sheets, s).map(|i| i as u32),
            None => self.cur,
        };
        if let Some(l) = scope
            && let Some(i) = self
                .names
                .iter()
                .position(|e| e.local == Some(l) && same(e))
        {
            return i as u32 + 1;
        }
        if sheet.is_none()
            && let Some(i) = self.names.iter().position(|e| e.local.is_none() && same(e))
        {
            return i as u32 + 1;
        }
        // 정의되지 않은 이름: 한셀처럼 수식 없는 이름을 만든다 (관측: values 의 XYZ)
        self.names.push(NameEntry {
            name: name.to_string(),
            local: if sheet.is_some() { scope } else { None },
            hidden: false,
            formula: None,
            func: false,
        });
        self.names.len() as u32
    }

    fn extern_name(&mut self, _book: usize, _name: &str) -> Option<(u16, u32)> {
        None
    }

    fn table(&mut self, name: &str) -> Option<formula::ListRef> {
        let same = |t: &(u32, String, String, Vec<String>, String)| {
            t.1.to_lowercase() == name.to_lowercase() || t.2.to_lowercase() == name.to_lowercase()
        };
        let (id, columns, sheet) = self
            .tables
            .iter()
            .find(|t| same(t))
            .map(|t| (t.0, t.3.clone(), t.4.clone()))?;
        let ixti = self.xti(None, &sheet, None)?;
        Some(formula::ListRef { id, columns, ixti })
    }
}

/// 수식 하나를 토큰으로. 실패하면 경고를 남기고 None
fn encode(
    book: &mut Book,
    warnings: &mut Vec<String>,
    what: &str,
    text: &str,
    place: Place,
) -> Option<Encoded> {
    match formula::encode(text, place, book) {
        Ok(e) => Some(e),
        Err(err) => {
            warnings.push(format!("{what}: 수식 {text:?} 를 쓰지 못함 ({err})"));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// 시트
// ---------------------------------------------------------------------------

/// 시트 관계 하나 (ID, 종류, 대상, 외부)
type Rel = (String, String, String, bool);

const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

struct SheetOut {
    bin: Vec<u8>,
    rels: Vec<Rel>,
    comments: Option<Vec<u8>>,
    vml: Option<String>,
    /// 그리기 파트 (이름, XML, 관계)
    drawing: Option<(String, String, Vec<Rel>)>,
    /// 표 파트 (이름, 레코드)
    tables: Vec<(String, Vec<u8>)>,
}

/// 표 열의 요약 행 함수 (`ilta` 순서, 0 = 없음 — 바이너리 읽기와 같다)
const TOTALS_FUNCTIONS: [&str; 10] = [
    "none",
    "average",
    "count",
    "countNums",
    "max",
    "min",
    "sum",
    "stdDev",
    "var",
    "custom",
];

fn error_code(e: &str) -> u8 {
    match e {
        "#NULL!" => 0x00,
        "#DIV/0!" => 0x07,
        "#VALUE!" => 0x0F,
        "#REF!" => 0x17,
        "#NAME?" => 0x1D,
        "#NUM!" => 0x24,
        "#GETTING_DATA" => 0x2B,
        _ => 0x2A,
    }
}

/// 한셀의 RK: 부동소수 상위 30비트로 정확히 나타낼 수 있을 때만 (정수·×100 꼴은 쓰지 않는다, 관측: 코퍼스의
/// BrtCellRk 8218개가 모두 이 꼴이고, 이 꼴로 쓸 수 있는 BrtCellReal 은 하나도 없다)
fn rk(v: f64) -> Option<u32> {
    let bits = v.to_bits();
    (bits & 0x3_FFFF_FFFF == 0).then_some((bits >> 32) as u32)
}

fn twips(pt: f64) -> u16 {
    (pt * 20.0).round().clamp(0.0, 65535.0) as u16
}

/// 열 너비(글자 수) → 1/256 글자
fn width256(w: f64) -> u32 {
    (w * 256.0).round().max(0.0) as u32
}

fn pane_number(p: Option<&str>) -> u32 {
    match p {
        Some("bottomRight") => 0,
        Some("topRight") => 1,
        Some("bottomLeft") => 2,
        _ => 3,
    }
}

struct SheetCtx<'a, 'b> {
    book: &'a mut Book<'b>,
    sst: &'a mut Sst,
    fonts: &'a mut FontTable,
    warnings: &'a mut Vec<String>,
    /// 데이터 막대마다 x14 확장의 GUID 를 만들 때 쓰는 번호
    guid_seed: u32,
    /// 열 너비 단위 (최대 숫자 너비, 픽셀) — 모델 차트의 앵커 위치를 셈한다
    mdw: u32,
    /// 모델로 새로 쓰는 차트 파트 (레거시에서 읽은 차트): (파트 이름, XML)
    charts: &'a mut Vec<(String, String)>,
    /// [`Options::exact`]
    exact: bool,
    /// 지금까지 쓴 표 파트 수 (파트 이름 `tableN.bin` 은 문서 전체에서 1부터)
    next_table: usize,
    /// 워크시트마다 피벗 표 파트 이름 ([`super::pivot::binary_parts`])
    pivot_tables: &'a [Vec<String>],
}

/// 데이터 막대의 x14 확장 GUID. 한셀은 새로 만든다 (임의 값) — rcell 은 문서 안 순서로 정한다
fn guid(seed: u32) -> [u8; 16] {
    let mut g = [0u8; 16];
    let h = crate::crypto::sha1(format!("rcell-databar-{seed}").as_bytes());
    g.copy_from_slice(&h[..16]);
    // RFC 4122 변형 4
    g[7] = (g[7] & 0x0F) | 0x40;
    g[8] = (g[8] & 0x3F) | 0x80;
    g
}

/// 조건부 서식 규칙 (종류, 틀, 매개 변수, 플래그, strParam)
fn cf_kind(r: &CfRule) -> (u32, u32, u32, u16, Option<&str>) {
    const OPS: [&str; 9] = [
        "",
        "between",
        "notBetween",
        "equal",
        "notEqual",
        "greaterThan",
        "lessThan",
        "greaterThanOrEqual",
        "lessThanOrEqual",
    ];
    let stop = if r.stop_if_true { 0x02 } else { 0 };
    match r.kind.as_str() {
        "cellIs" => {
            let op = OPS
                .iter()
                .position(|o| Some(*o) == r.operator.as_deref())
                .unwrap_or(1) as u32;
            (1, 0, op, stop, None)
        }
        "colorScale" => (3, 2, 0, stop, None),
        "dataBar" => (4, 3, 0, stop, None),
        "iconSet" => (6, 4, 0, stop, None),
        "top10" => {
            let mut f = stop;
            if r.bottom {
                f |= 0x08;
            }
            if r.percent {
                f |= 0x10;
            }
            (5, 5, r.rank.unwrap_or(10), f, None)
        }
        "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
            let p = match r.kind.as_str() {
                "notContainsText" => 1,
                "beginsWith" => 2,
                "endsWith" => 3,
                _ => 0,
            };
            (2, 8, p, stop, r.text.as_deref())
        }
        "containsBlanks" => (2, 9, 0, stop, None),
        "notContainsBlanks" => (2, 0x0A, 0, stop, None),
        "containsErrors" => (2, 0x0B, 0, stop, None),
        "notContainsErrors" => (2, 0x0C, 0, stop, None),
        "timePeriod" => {
            const PERIODS: [&str; 10] = [
                "today",
                "tomorrow",
                "yesterday",
                "last7Days",
                "lastMonth",
                "nextMonth",
                "thisWeek",
                "nextWeek",
                "lastWeek",
                "thisMonth",
            ];
            let i = PERIODS
                .iter()
                .position(|p| Some(*p) == r.time_period.as_deref())
                .unwrap_or(0) as u32;
            (2, 0x0F + i, 0, stop, None)
        }
        "aboveAverage" => {
            let above = r.above_average.unwrap_or(true);
            let t = match (above, r.equal_average) {
                (true, false) => 0x19,
                (false, false) => 0x1A,
                (true, true) => 0x1D,
                (false, true) => 0x1E,
            };
            // 평균 초과 쪽 규칙은 플래그 0x04 (관측: cf_dv)
            let f = if above { stop | 0x04 } else { stop };
            (2, t, r.std_dev.unwrap_or(0), f, None)
        }
        "duplicateValues" => (2, 0x1B, 0, stop, None),
        // 한셀은 고유 값을 틀 7 로 쓴다 (관측: cf_dv)
        "uniqueValues" => (2, 0x07, 0, stop, None),
        _ => (2, 1, 0, stop, None),
    }
}

fn cfvo_record(c: &Cfvo, icon: bool) -> P {
    let ty = match c.kind.as_str() {
        "num" => 1,
        "min" => 2,
        "max" => 3,
        "percent" => 4,
        "percentile" => 5,
        "formula" => 6,
        "autoMin" => 8,
        "autoMax" => 9,
        _ => 1,
    };
    let num = c
        .val
        .as_deref()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut p = P::new();
    p.u32(ty).f64(num);
    if icon {
        p.u32(1).u32(u32::from(c.gte));
    } else {
        p.u32(0).u32(0);
    }
    p.u32(0);
    p
}

const ICON_SETS: [&str; 20] = [
    "3Arrows",
    "3ArrowsGray",
    "3Flags",
    "3TrafficLights1",
    "3TrafficLights2",
    "3Signs",
    "3Symbols",
    "3Symbols2",
    "4Arrows",
    "4ArrowsGray",
    "4RedToBlack",
    "4Rating",
    "4TrafficLights",
    "5Arrows",
    "5ArrowsGray",
    "5Rating",
    "5Quarters",
    "3Stars",
    "3Triangles",
    "5Boxes",
];

const DV_KINDS: [&str; 8] = [
    "none",
    "whole",
    "decimal",
    "list",
    "date",
    "time",
    "textLength",
    "custom",
];
const DV_OPS: [&str; 8] = [
    "between",
    "notBetween",
    "equal",
    "notEqual",
    "greaterThan",
    "lessThan",
    "greaterThanOrEqual",
    "lessThanOrEqual",
];

impl SheetCtx<'_, '_> {
    fn fmla(&mut self, what: &str, text: &str, place: Place) -> Option<Encoded> {
        encode(self.book, self.warnings, what, text, place)
    }

    #[allow(clippy::too_many_arguments)]
    fn sheet(
        &mut self,
        index: usize,
        sheet: &Sheet,
        selected: bool,
        wb: &Workbook,
        drawing_name: Option<String>,
        comments_number: usize,
    ) -> SheetOut {
        self.book.cur = Some(index as u32);
        // 표가 있는 시트는 셀 수식보다 먼저 XTI 를 받는다 — 쓰이지 않아도 (XTI 순서 실험: 시트 셋에 수식·표·이름을 나눠
        // 넣은 COM 문서 셋 → [이름, S1 의 표, S1 수식의 S3, S2 의 표] 순서)
        if !sheet.tables.is_empty() {
            let _ = self.book.xti(None, &sheet.name, None);
        }
        let mut o = Out::default();
        let mut rels: Vec<Rel> = Vec::new();
        o.e(129); // BrtBeginSheet
        self.ws_prop(&mut o, sheet, index);
        self.dimension(&mut o, sheet);
        self.views(&mut o, sheet, selected);
        // BrtWsFmtInfo: 기본 열 너비(1/256), 기본 너비 글자 수 8, 기본 행 높이(twips), 플래그, 개요 수준 (한셀은 0)
        o.p(
            485,
            P::new()
                .u32(width256(sheet.default_col_width.unwrap_or(9.00390625)))
                .u16(8)
                .u16(twips(sheet.default_row_height.unwrap_or(16.4)))
                .u16(0)
                .u8(0)
                .u8(0),
        );
        if !sheet.cols.is_empty() {
            o.e(390);
            for c in &sheet.cols {
                let mut flags = 0u16;
                if c.hidden {
                    flags |= 0x0001;
                }
                if c.custom_width {
                    flags |= 0x0002;
                }
                flags |= u16::from(c.outline_level.min(7)) << 8;
                if c.collapsed {
                    // 한셀은 접힘을 [MS-XLSB] 의 0x1000 이 아니라 0x2000 에 쓴다 (관측)
                    flags |= 0x2000;
                }
                let w = c
                    .width
                    .unwrap_or(sheet.default_col_width.unwrap_or(9.00390625));
                o.p(
                    60,
                    P::new()
                        .u32(c.min.saturating_sub(1))
                        .u32(c.max.saturating_sub(1))
                        .u32(width256(w))
                        .u32(c.style.unwrap_or(0))
                        .u16(flags),
                );
            }
            o.e(391);
        }
        self.sheet_data(&mut o, sheet);
        self.protection(&mut o, sheet);
        auto_filter(&mut o, sheet);
        if !sheet.merges.is_empty() {
            o.p(177, P::new().u32(sheet.merges.len() as u32));
            for m in &sheet.merges {
                o.p(176, P::new().rfx(*m));
            }
            o.e(178);
        }
        let bars = self.conditional_formats(&mut o, sheet);
        self.validations(&mut o, sheet);
        self.chart_refs(&sheet.shapes);
        for h in &sheet.hyperlinks {
            let id = h.target.as_ref().map(|t| {
                let id = format!("rId{}", rels.len() + 1);
                rels.push((id.clone(), format!("{REL_BASE}/hyperlink"), t.clone(), true));
                id
            });
            let range = ranges(&h.range).first().copied().unwrap_or(CellRange {
                start: CellRef::default(),
                end: CellRef::default(),
            });
            o.p(
                494,
                P::new()
                    .rfx(range)
                    .nwstr(id.as_deref())
                    .wstr(h.location.as_deref().unwrap_or(""))
                    .wstr(h.tooltip.as_deref().unwrap_or(""))
                    .wstr(h.display.as_deref().unwrap_or("")),
            );
        }
        let hf_font = self
            .exact
            .then(|| self.fonts.fonts.first().and_then(|f| f.name.clone()))
            .flatten();
        // 프린터 설정 파트(원본 그대로)는 BrtPageSetup 끝의 관계 ID 로 가리킨다 (시트 관계 rIdP<n>, XML 쓰기와 같다)
        let printer = sheet
            .part_links
            .iter()
            .filter(|l| binary_keeps(&l.rel_type))
            .position(|l| l.rel_type.ends_with("/printerSettings"))
            .map(|k| format!("rIdP{}", k + 1));
        page(
            &mut o,
            sheet,
            self.exact,
            hf_font.as_deref(),
            printer.as_deref(),
        );
        let mut drawing = None;
        if let Some(name) = drawing_name {
            let id = format!("rId{}", rels.len() + 1);
            rels.push((
                id.clone(),
                format!("{REL_BASE}/drawing"),
                name.clone(),
                false,
            ));
            o.p(550, P::new().wstr(&id));
            let (xml, drels) =
                crate::writer::drawing_xml(sheet, self.mdw, &wb.raw_parts, self.charts);
            drawing = Some((name, xml, drels));
        }
        let (mut comments, mut vml) = (None, None);
        if !sheet.comments.is_empty() {
            let vml_name = format!("xl/drawings/vmlDrawing{comments_number}.vml");
            let id = format!("rId{}", rels.len() + 1);
            rels.push((
                id.clone(),
                format!("{REL_BASE}/vmlDrawing"),
                vml_name,
                false,
            ));
            o.p(551, P::new().wstr(&id));
            comments = Some(self.comments(&sheet.comments));
            vml = Some(crate::writer::comments_vml(sheet, comments_number, true));
        }
        // 한셀이 늘 쓰는 ActiveX 컨트롤 묶음(643, 645, 비어 있음)과 표 목록(660 개수, 661 관계 ID, 662)
        o.e(643);
        o.e(645);
        o.p(660, P::new().u32(sheet.tables.len() as u32));
        let mut tables = Vec::new();
        for t in &sheet.tables {
            self.next_table += 1;
            let name = format!("xl/tables/table{}.bin", self.next_table);
            let id = format!("rId{}", rels.len() + 1);
            rels.push((id.clone(), format!("{REL_BASE}/table"), name.clone(), false));
            o.p(661, P::new().wstr(&id));
            tables.push((name, self.table_part(t, sheet)));
        }
        o.e(662);
        // 시트 관계 순서: 하이퍼링크, 그리기, VML, 표, 피벗 표, 메모 (표·피벗·메모 실험의 COM 문서). 피벗 표는 시트에
        // 레코드가 없고 관계로만 가리킨다. 원본 그대로의 프린터 설정은 rIdP<n> (한셀은 버리는 파트라 자리를 관측할 수
        // 없다 — XML 쓰기와 같은 이름)
        for name in self.pivot_tables.get(index).into_iter().flatten() {
            rels.push((
                format!("rId{}", rels.len() + 1),
                format!("{REL_BASE}/pivotTable"),
                name.clone(),
                false,
            ));
        }
        for (k, l) in sheet
            .part_links
            .iter()
            .filter(|l| binary_keeps(&l.rel_type))
            .enumerate()
        {
            rels.push((
                format!("rIdP{}", k + 1),
                l.rel_type.clone(),
                l.part.clone(),
                false,
            ));
        }
        if comments.is_some() {
            rels.push((
                format!("rId{}", rels.len() + 1),
                format!("{REL_BASE}/comments"),
                format!("xl/comments{comments_number}.bin"),
                false,
            ));
        }
        if !bars.is_empty() {
            data_bars14(&mut o, &bars);
        }
        o.e(130); // BrtEndSheet
        let _ = wb;
        SheetOut {
            bin: o.0,
            rels,
            comments,
            vml,
            drawing,
            tables,
        }
    }

    /// 표 파트 (`xl/tables/tableN.bin`) — 한셀이 쓰는 꼴 (표 대조 실험: 조작한 XML·COM 문서의 X.bin.cell):
    /// `BrtBeginList` 343, [자동 필터 161…162], `BrtBeginListCols` 345, 열마다 `BrtBeginListCol` 347 [`BrtListCCFmla` 351]
    /// [`BrtListTrFmla` 352] `BrtEndListCol` 348, 346, [`BrtTableStyleClient` 513], `BrtEndList` 344
    fn table_part(&mut self, t: &crate::model::Table, sheet: &Sheet) -> Vec<u8> {
        let mut o = Out::default();
        let range = ranges(&t.range).first().copied().unwrap_or(CellRange {
            start: CellRef::default(),
            end: CellRef::default(),
        });
        let lt = match t.table_type.as_deref() {
            Some("xml") => 1,
            Some("queryTable") => 2,
            _ => 0,
        };
        let mut p = P::new();
        p.rfx(range)
            .u32(lt)
            .u32(t.id)
            .u32(t.header_rows)
            .u32(t.totals_rows)
            // 0x01 요약 행을 보인 적 있음 (fShownTotalRow)
            .u32(u32::from(t.totals_shown));
        for d in t.dxfs {
            p.u32(d.unwrap_or(0xFFFF_FFFF));
        }
        p.u32(t.connection_id.unwrap_or(0))
            .nwstr(Some(&t.name))
            .nwstr(Some(&t.display_name))
            .nwstr(t.comment.as_deref());
        for s in &t.cell_styles {
            p.nwstr(s.as_deref());
        }
        o.p(343, &p);
        if let Some(af) = &t.auto_filter {
            auto_filter_records(&mut o, af, sheet);
        }
        o.p(345, P::new().u32(t.columns.len() as u32));
        for c in &t.columns {
            let ilta = c
                .totals_function
                .as_deref()
                .and_then(|f| TOTALS_FUNCTIONS.iter().position(|x| *x == f))
                .unwrap_or(0) as u32;
            let mut p = P::new();
            p.u32(c.id).u32(ilta);
            for d in c.dxfs {
                p.u32(d.unwrap_or(0xFFFF_FFFF));
            }
            p.u32(c.query_table_field_id.unwrap_or(0))
                .nwstr(c.unique_name.as_deref())
                .nwstr(Some(&c.name))
                .nwstr(c.totals_label.as_deref())
                .nwstr(None)
                .nwstr(None)
                .nwstr(None);
            o.p(347, &p);
            // 계산 열 수식(351)·요약 행 수식(352): fArray u8 + CellParsedFormula
            for (rt, f) in [
                (351u16, c.calculated_formula.as_deref()),
                (352, c.totals_formula.as_deref()),
            ] {
                if let Some(f) = f
                    && let Some(e) =
                        self.fmla(&format!("표 {} 열 {}", t.name, c.name), f, Place::Cell)
                {
                    o.p(rt, P::new().u8(0).fmla(&e));
                }
            }
            o.e(348);
        }
        o.e(346);
        if let Some(st) = &t.style {
            let flags = u16::from(st.first_column)
                | (u16::from(st.last_column) << 1)
                | (u16::from(st.row_stripes) << 2)
                | (u16::from(st.column_stripes) << 3);
            o.p(513, P::new().u16(flags).nwstr(st.name.as_deref()));
        }
        o.e(344);
        o.0
    }

    /// BrtWsProp: 플래그 3바이트, 탭 색, rwSync, colSync, 코드 이름
    fn ws_prop(&mut self, o: &mut Out, sheet: &Sheet, index: usize) {
        let p = &sheet.page;
        // 0x01 자동 페이지 나누기 표시, 0x08 (한셀이 늘 켠다), 0x40 요약 행 아래, 0x80 요약 열 오른쪽. 한셀은
        // 0x80 을 늘 켜서 요약 열 왼쪽을 잃는다 (docs/format/cell-binary.md 2.1) — rcell 은 지킨다
        let mut b0 = 0x08u8;
        if !sheet.summary_left || self.exact {
            b0 |= 0x80;
        }
        // 한셀은 자동 페이지 나누기 끔도 저장하지 않는다
        if p.auto_page_breaks.unwrap_or(true) || self.exact {
            b0 |= 0x01;
        }
        if !sheet.summary_above {
            b0 |= 0x40;
        }
        // 0x01 페이지에 맞추기, 0x04 개요 기호 표시
        let mut b1 = 0x04u8;
        if p.fit_to_page.unwrap_or(false) {
            b1 |= 0x01;
        }
        // 0x01 필터 적용 중, 0x02 조건부 서식 계산
        let b2 = 0x02u8 | u8::from(sheet.filter_mode);
        let tab = sheet
            .tab_color
            .as_ref()
            .map_or([0x01, 0x00, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF], |c| {
                brt_color(c, 0)
            });
        o.p(
            147,
            P::new()
                .u8(b0)
                .u8(b1)
                .u8(b2)
                .raw(&tab)
                .u32(0xFFFF_FFFF)
                .u32(0xFFFF_FFFF)
                // 코드 이름이 없으면 한셀은 SheetN 을 채운다 (관측: filter2)
                .wstr(
                    &sheet
                        .code_name
                        .clone()
                        .unwrap_or_else(|| format!("Sheet{}", index + 1)),
                ),
        );
    }

    /// BrtWsDim: 쓰인 범위 (모델의 dimension, 없으면 셀들의 범위)
    fn dimension(&mut self, o: &mut Out, sheet: &Sheet) {
        let dim = sheet
            .dimension
            .as_deref()
            .and_then(|d| ranges(d).first().copied())
            .or_else(|| {
                let mut it = sheet.cells.iter();
                let f = it.next()?;
                let mut r = CellRange {
                    start: f.r,
                    end: f.r,
                };
                for c in it {
                    r.start.row = r.start.row.min(c.r.row);
                    r.start.col = r.start.col.min(c.r.col);
                    r.end.row = r.end.row.max(c.r.row);
                    r.end.col = r.end.col.max(c.r.col);
                }
                Some(r)
            })
            .unwrap_or(CellRange {
                start: CellRef::default(),
                end: CellRef::default(),
            });
        o.p(148, P::new().rfx(dim));
    }

    fn views(&mut self, o: &mut Out, sheet: &Sheet, selected: bool) {
        let v = &sheet.view;
        o.e(133);
        // 0x02 수식, 0x04 눈금선, 0x08 머리글, 0x10 0 값, 0x40 선택, 0x100 개요 기호, 0x200 기본 머리글 색
        let mut flags = 0x0300u16;
        if v.show_formulas {
            flags |= 0x02;
        }
        if sheet.show_grid_lines {
            flags |= 0x04;
        }
        if v.show_row_col_headers {
            flags |= 0x08;
        }
        if v.show_zeros {
            flags |= 0x10;
        }
        if selected {
            flags |= 0x40;
        }
        let xl_view = match v.view.as_deref() {
            Some("pageBreakPreview") => 1,
            Some("pageLayout") => 2,
            _ => 0,
        };
        let tl = v
            .top_left_cell
            .as_deref()
            .and_then(CellRef::parse)
            .unwrap_or_default();
        // 확대율: (18) 현재, (20)·(22) 보조, (24) 페이지 레이아웃. 한셀의 XML 은 기본 보기에서 zoomScaleNormal 을
        // (22) 에, 페이지 나누기 미리 보기에서는 (20) 에 둔다 (관측, docs/format/cell-binary.md 2.1)
        let z = v.zoom_scale.unwrap_or(100) as u16;
        let normal = v.zoom_scale_normal.unwrap_or(100) as u16;
        let (z20, z22) = if xl_view == 1 {
            (normal, z)
        } else {
            (z, normal)
        };
        o.p(
            137,
            P::new()
                .u16(flags)
                .u32(xl_view)
                .u32(tl.row)
                .u32(tl.col)
                .u8(0x40)
                .u8(0)
                .u16(0)
                .u16(z)
                .u16(z20)
                .u16(z22)
                .u16(100)
                .u32(0),
        );
        let mut active = v.pane.as_ref().and_then(|p| p.active_pane.as_deref());
        // 한셀은 나눈 창(고정 아님)의 활성 창을 잃는다: 창 0, 선택은 창 3 (docs/format/cell-binary.md 2.1)
        let split = v
            .pane
            .as_ref()
            .is_some_and(|p| !matches!(p.state.as_deref(), Some("frozen" | "frozenSplit")));
        let mut sel_pane = None;
        if self.exact && split {
            active = Some("bottomRight");
            sel_pane = Some(3);
        }
        if let Some(p) = &v.pane {
            let t = p
                .top_left_cell
                .as_deref()
                .and_then(CellRef::parse)
                .unwrap_or_default();
            let state = match p.state.as_deref() {
                Some("frozen") => 0x03,
                Some("frozenSplit") => 0x01,
                _ => 0x00,
            };
            o.p(
                151,
                P::new()
                    .f64(p.x_split)
                    .f64(p.y_split)
                    .u32(t.row)
                    .u32(t.col)
                    .u32(match active {
                        Some(a) => pane_number(Some(a)),
                        None => 3,
                    })
                    .u8(state),
            );
        }
        // BrtSel: 한셀은 활성 창의 선택 하나만 쓴다 (관측)
        let ac = v
            .active_cell
            .as_deref()
            .and_then(CellRef::parse)
            .unwrap_or_default();
        let mut sq = ranges(v.sqref.as_deref().unwrap_or(""));
        if sq.is_empty() {
            sq.push(CellRange { start: ac, end: ac });
        }
        let iact = sq.iter().position(|r| r.contains(ac)).unwrap_or(0) as u32;
        let mut p = P::new();
        p.u32(if let Some(n) = sel_pane {
            n
        } else if v.pane.is_some() {
            pane_number(active)
        } else {
            3
        })
        .u32(ac.row)
        .u32(ac.col)
        .u32(iact)
        .u32(sq.len() as u32);
        for r in sq {
            p.rfx(r);
        }
        o.p(152, &p);
        o.e(138);
        o.e(134);
    }

    fn sheet_data(&mut self, o: &mut Out, sheet: &Sheet) {
        o.e(145);
        let row_info: HashMap<u32, &crate::model::RowInfo> =
            sheet.rows.iter().map(|r| (r.index, r)).collect();
        let mut rows: Vec<u32> = sheet.cells.iter().map(|c| c.r.row + 1).collect();
        rows.extend(sheet.rows.iter().map(|r| r.index));
        rows.sort_unstable();
        rows.dedup();
        let default_h = twips(sheet.default_row_height.unwrap_or(16.4));
        // 공유 수식·배열 수식의 기준 셀
        let mut shared: HashMap<u32, (CellRef, CellRange, Option<Encoded>)> = HashMap::new();
        let mut arrays: Vec<(CellRef, CellRange)> = Vec::new();
        for c in &sheet.cells {
            let Some(f) = &c.formula else { continue };
            match (f.kind, f.shared_index, f.range.as_deref()) {
                (FormulaKind::Shared, Some(si), Some(rg)) if !f.text.is_empty() => {
                    if let Some(range) = ranges(rg).first().copied()
                        && range.start == c.r
                    {
                        let enc = self.fmla(
                            &format!("{}!{} 공유 수식", sheet.name, c.r),
                            &f.text,
                            Place::Relative(c.r),
                        );
                        shared.insert(si, (c.r, range, enc));
                    }
                }
                (FormulaKind::Array, _, Some(rg)) if !f.text.is_empty() => {
                    if let Some(range) = ranges(rg).first().copied() {
                        arrays.push((c.r, range));
                    }
                }
                _ => {}
            }
        }
        let mut ci = 0;
        for rn in rows {
            let info = row_info.get(&rn);
            let start = ci;
            while ci < sheet.cells.len() && sheet.cells[ci].r.row + 1 == rn {
                ci += 1;
            }
            let cells = &sheet.cells[start..ci];
            // 열 구간: 1024열 묶음마다 (첫 열, 끝 열)
            let mut spans: Vec<(u32, u32)> = Vec::new();
            for c in cells {
                match spans.last_mut() {
                    Some(s) if s.0 / 1024 == c.r.col / 1024 => s.1 = c.r.col,
                    _ => spans.push((c.r.col, c.r.col)),
                }
            }
            let mut b11 = 0u8;
            let (mut ixfe, mut height) = (0u32, default_h);
            if let Some(i) = info {
                b11 |= i.outline_level.min(7);
                if i.collapsed {
                    b11 |= 0x08;
                }
                if i.hidden {
                    b11 |= 0x10;
                }
                if i.custom_height {
                    b11 |= 0x20;
                }
                if let Some(s) = i.style {
                    b11 |= 0x40;
                    ixfe = s;
                }
                if let Some(h) = i.height {
                    height = twips(h);
                }
            }
            let mut p = P::new();
            p.u32(rn - 1)
                .u32(ixfe)
                .u16(height)
                .u8(0)
                .u8(b11)
                .u8(0)
                .u32(spans.len() as u32);
            for (a, b) in spans {
                p.u32(a).u32(b);
            }
            o.p(0, &p);
            for c in cells {
                self.cell(o, sheet, c, &shared, &arrays);
            }
        }
        o.e(146);
    }

    fn cell(
        &mut self,
        o: &mut Out,
        sheet: &Sheet,
        c: &Cell,
        shared: &HashMap<u32, (CellRef, CellRange, Option<Encoded>)>,
        arrays: &[(CellRef, CellRange)],
    ) {
        let head = |p: &mut P| {
            p.u32(c.r.col).u32(c.style & 0x00FF_FFFF);
        };
        // 수식: (토큰, 공유 수식 기록, 배열 수식 기록)
        let mut extra: Option<(u16, P)> = None;
        let enc: Option<Encoded> = match &c.formula {
            Some(f) if !f.text.is_empty() || f.shared_index.is_some() => {
                let exp = |m: CellRef| Encoded {
                    rgce: {
                        let mut v = vec![0x01];
                        v.extend_from_slice(&m.row.to_le_bytes());
                        v
                    },
                    rgcb: m.col.to_le_bytes().to_vec(),
                    volatile: false,
                };
                let master = f
                    .shared_index
                    .filter(|_| f.kind == FormulaKind::Shared)
                    .and_then(|si| shared.get(&si))
                    .filter(|(_, range, enc)| range.contains(c.r) && enc.is_some());
                match master {
                    Some((m, range, Some(e))) => {
                        if *m == c.r {
                            let mut p = P::new();
                            p.rfx(*range).fmla(e);
                            extra = Some((427, p));
                        }
                        let mut x = exp(*m);
                        x.volatile = e.volatile;
                        Some(x)
                    }
                    _ if f.text.is_empty() => None,
                    _ if f.kind == FormulaKind::Array && arrays.iter().any(|(m, _)| *m == c.r) => {
                        let range = arrays.iter().find(|(m, _)| *m == c.r).unwrap().1;
                        match self.fmla(
                            &format!("{}!{} 배열 수식", sheet.name, c.r),
                            &f.text,
                            Place::Array,
                        ) {
                            Some(e) => {
                                let mut p = P::new();
                                p.rfx(range).u8(0).fmla(&e);
                                extra = Some((426, p));
                                let mut x = exp(c.r);
                                x.volatile = e.volatile;
                                Some(x)
                            }
                            None => None,
                        }
                    }
                    _ => self.fmla(&format!("{}!{}", sheet.name, c.r), &f.text, Place::Cell),
                }
            }
            // 배열 수식 범위 안의 다른 셀: 기준 셀을 가리킨다
            None if !c.value.is_empty() => arrays
                .iter()
                .find(|(m, rg)| *m != c.r && rg.contains(c.r))
                .map(|(m, _)| Encoded {
                    rgce: {
                        let mut v = vec![0x01];
                        v.extend_from_slice(&m.row.to_le_bytes());
                        v
                    },
                    rgcb: m.col.to_le_bytes().to_vec(),
                    volatile: false,
                }),
            _ => None,
        };
        let mut p = P::new();
        head(&mut p);
        match enc {
            Some(e) => {
                let ty = match &c.value {
                    CellValue::Number(n) => {
                        p.f64(*n);
                        9
                    }
                    CellValue::Bool(b) => {
                        p.u8(u8::from(*b));
                        10
                    }
                    CellValue::Error(x) => {
                        p.u8(error_code(x));
                        11
                    }
                    CellValue::Text(s) | CellValue::Date(s) => {
                        p.wstr(s);
                        8
                    }
                    CellValue::Empty => {
                        p.wstr("");
                        8
                    }
                };
                // grbitFlags: 0x02 fAlwaysCalc (휘발성 함수, 한셀 XML 의 ca="1")
                p.u16(if e.volatile { 0x02 } else { 0 }).fmla(&e);
                o.p(ty, &p);
            }
            None => match &c.value {
                CellValue::Empty => o.p(1, &p),
                CellValue::Number(n) => match rk(*n) {
                    Some(v) => {
                        p.u32(v);
                        o.p(2, &p);
                    }
                    None => {
                        p.f64(*n);
                        o.p(5, &p);
                    }
                },
                CellValue::Bool(b) => {
                    p.u8(u8::from(*b));
                    o.p(4, &p);
                }
                CellValue::Error(x) => {
                    p.u8(error_code(x));
                    o.p(3, &p);
                }
                CellValue::Text(s) | CellValue::Date(s) => {
                    let i = self.sst.get(s);
                    p.u32(i);
                    o.p(7, &p);
                }
            },
        }
        if let Some((t, p)) = extra {
            o.p(t, &p);
        }
    }

    /// BrtSheetProtection: 암호 u16, 시트 잠금 u32, 이어서 15개 항목의 '허용' 여부 u32 (OOXML 과 반대)
    fn protection(&mut self, o: &mut Out, sheet: &Sheet) {
        let Some(sp) = &sheet.protection else { return };
        let pw = sp
            .password
            .as_deref()
            .and_then(|h| u16::from_str_radix(h.trim(), 16).ok())
            .unwrap_or(0);
        let mut p = P::new();
        p.u16(pw);
        let unlocked_blocked = sp.flag("selectUnlockedCells");
        for (i, (k, _)) in crate::model::SHEET_PROTECTION_FLAGS.iter().enumerate() {
            // 한셀은 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막는다 (관측)
            let blocked = sp.flag(k) || (*k == "selectLockedCells" && unlocked_blocked);
            p.u32(if i == 0 {
                u32::from(blocked)
            } else {
                u32::from(!blocked)
            });
        }
        o.p(535, &p);
    }

    /// 조건부 서식. 규칙은 우선순위가 큰 것부터 (관측). 데이터 막대 규칙을 돌려준다 (x14 확장용)
    fn conditional_formats(&mut self, o: &mut Out, sheet: &Sheet) -> Vec<DataBar14> {
        let mut bars = Vec::new();
        for cf in &sheet.conditional_formats {
            let base = first_cell(&cf.sqref);
            let mut rules: Vec<&CfRule> = cf.rules.iter().collect();
            rules.sort_by_key(|r| std::cmp::Reverse(r.priority));
            o.p(
                461,
                P::new().u32(rules.len() as u32).u32(0).sqref(&cf.sqref),
            );
            for r in rules {
                self.cf_rule(o, sheet, cf, r, base, &mut bars);
            }
            o.e(462);
        }
        bars
    }

    fn cf_rule(
        &mut self,
        o: &mut Out,
        sheet: &Sheet,
        cf: &ConditionalFormat,
        r: &CfRule,
        base: CellRef,
        bars: &mut Vec<DataBar14>,
    ) {
        let (ty, tmpl, param, flags, text) = cf_kind(r);
        let mut fs: Vec<Vec<u8>> = Vec::new();
        for f in r.formulas.iter().take(3) {
            let e = self
                .fmla(
                    &format!("{} 조건부 서식 {}", sheet.name, cf.sqref),
                    f,
                    Place::Relative(base),
                )
                .unwrap_or_default();
            let mut p = P::new();
            p.fmla(&e);
            fs.push(p.0);
        }
        let mut p = P::new();
        p.u32(ty)
            .u32(tmpl)
            .u32(r.dxf_id.unwrap_or(0xFFFF_FFFF))
            .u32(r.priority)
            .u32(param)
            .u32(0)
            .u32(0)
            .u16(flags);
        for i in 0..3 {
            p.u32(fs.get(i).map_or(0, |f| f.len() as u32));
        }
        p.nwstr(text);
        for f in &fs {
            p.raw(f);
        }
        o.p(463, &p);
        if let Some(c) = &r.color_scale {
            o.e(469);
            for v in &c.cfvos {
                o.p(471, &cfvo_record(v, false));
            }
            for col in &c.colors {
                o.rec(564, &cf_color(col));
            }
            o.e(470);
        }
        if let Some(d) = &r.data_bar {
            // 막대 길이: 기본 레코드에는 기본값 10·90, 실제 값은 x14 확장(1051)에 (관측: cf_dv)
            o.p(467, P::new().u8(10).u8(90).u8(u8::from(d.show_value)));
            for v in &d.cfvos {
                o.p(471, &cfvo_record(v, false));
            }
            if let Some(col) = &d.color {
                o.rec(564, &cf_color(col));
            }
            o.e(468);
            // 한셀은 데이터 막대마다 Excel 2010 확장(x14)을 만들어 GUID 로 잇는다 (관측)
            self.guid_seed += 1;
            let g = guid(self.guid_seed);
            o.p(35, P::new().raw(&[0x02, 0x0E, 0x00, 0x00]));
            o.p(1146, P::new().u32(0).raw(&g));
            o.e(36);
            bars.push(DataBar14 {
                sqref: cf.sqref.clone(),
                rules_in_block: cf.rules.len() as u32,
                guid: g,
                bar: d.clone(),
            });
        }
        if let Some(i) = &r.icon_set {
            let set = ICON_SETS.iter().position(|s| *s == i.set).unwrap_or(3) as u32;
            let mut f = 0u16;
            if !i.show_value {
                f |= 0x02;
            }
            if i.reverse {
                f |= 0x04;
            }
            o.p(465, P::new().u32(set).u16(f));
            for v in &i.cfvos {
                o.p(471, &cfvo_record(v, true));
            }
            o.e(466);
        }
        o.e(464);
    }

    fn validations(&mut self, o: &mut Out, sheet: &Sheet) {
        if sheet.data_validations.is_empty() {
            return;
        }
        o.p(
            573,
            P::new()
                .u16(0)
                .u32(0)
                .u32(0)
                .u32(0)
                .u32(sheet.data_validations.len() as u32),
        );
        for d in &sheet.data_validations {
            o.p(64, &self.validation(sheet, d));
        }
        o.e(574);
    }

    fn validation(&mut self, sheet: &Sheet, d: &DataValidation) -> P {
        let kind = DV_KINDS.iter().position(|k| *k == d.kind).unwrap_or(0) as u32;
        let style = match d.error_style.as_deref() {
            Some("warning") => 1,
            Some("information") => 2,
            _ => 0,
        };
        let op = DV_OPS
            .iter()
            .position(|k| Some(*k) == d.operator.as_deref())
            .unwrap_or(0) as u32;
        let mut flags = kind | (style << 4) | (op << 20);
        if d.allow_blank {
            flags |= 0x0100;
        }
        if d.hide_drop_down {
            flags |= 0x0200;
        }
        if d.show_input {
            flags |= 0x0004_0000;
        }
        if d.show_error {
            flags |= 0x0008_0000;
        }
        let base = first_cell(&d.sqref);
        let mut f = |t: &Option<String>| -> Encoded {
            t.as_deref()
                .and_then(|t| {
                    self.fmla(
                        &format!("{} 유효성 {}", sheet.name, d.sqref),
                        t,
                        Place::Relative(base),
                    )
                })
                .unwrap_or_default()
        };
        let (f1, f2) = (f(&d.formula1), f(&d.formula2));
        let mut p = P::new();
        p.u32(flags)
            .sqref(&d.sqref)
            .nwstr(d.error_title.as_deref())
            .nwstr(d.error.as_deref())
            .nwstr(d.prompt_title.as_deref())
            .nwstr(d.prompt.as_deref())
            .fmla(&f1)
            .fmla(&f2);
        p
    }

    /// 차트 계열의 셀 참조도 한셀은 수식으로 읽어 XTI 를 만든다 (관측: 차트만 있는 문서에도 BrtSupSelf +
    /// BrtExternSheet). 토큰은 버리고 XTI 만 남긴다
    fn chart_refs(&mut self, shapes: &[crate::model::Shape]) {
        for s in shapes {
            if let Some(c) = &s.chart {
                let groups =
                    std::iter::once(&c.series).chain(c.more_groups.iter().map(|g| &g.series));
                for series in groups {
                    for x in series {
                        let tx = match &x.tx {
                            Some(crate::model::ChartTx::Ref(d)) => d.formula.as_deref(),
                            _ => None,
                        };
                        let refs = [
                            tx,
                            x.cat.as_ref().and_then(|d| d.formula.as_deref()),
                            x.val.as_ref().and_then(|d| d.formula.as_deref()),
                            x.bubble_size.as_ref().and_then(|d| d.formula.as_deref()),
                        ];
                        for f in refs.into_iter().flatten() {
                            let _ = formula::encode(f, Place::Name, self.book);
                        }
                    }
                }
            }
            self.chart_refs(&s.children);
        }
    }

    /// 메모 파트 (`xl/commentsN.bin`)
    fn comments(&mut self, list: &[Comment]) -> Vec<u8> {
        let mut authors: Vec<&str> = Vec::new();
        for c in list {
            if !authors.contains(&c.author.as_str()) {
                authors.push(&c.author);
            }
        }
        let mut o = Out::default();
        o.e(628);
        o.e(630);
        for a in &authors {
            o.p(632, P::new().wstr(a));
        }
        o.e(631);
        o.e(633);
        for c in list {
            let a = authors.iter().position(|x| *x == c.author).unwrap_or(0) as u32;
            o.p(
                635,
                P::new()
                    .u32(a)
                    .rfx(CellRange {
                        start: c.cell,
                        end: c.cell,
                    })
                    .raw(&[0; 16]),
            );
            let mut p = P::new();
            rich_str(&mut p, &c.text, self.fonts);
            o.p(637, &p);
            o.e(636);
        }
        o.e(634);
        o.e(629);
        o.0
    }
}

/// x14 확장을 쓸 데이터 막대 하나
struct DataBar14 {
    sqref: String,
    rules_in_block: u32,
    guid: [u8; 16],
    bar: crate::model::DataBar,
}

/// 시트 끝의 Excel 2010 조건부 서식 확장 (FRT 묶음). 한셀이 데이터 막대마다 기본값으로 만드는 꼴 (관측)
fn data_bars14(o: &mut Out, bars: &[DataBar14]) {
    o.p(35, P::new().raw(&[0x02, 0x0E, 0x00, 0x00]));
    o.e(1135);
    for b in bars {
        // FRTHeader(fSqref), FRTSqref 1개(플래그 2) + 범위, 규칙 수, fPivot
        let rs = ranges(&b.sqref);
        let mut p = P::new();
        p.u32(0x02).u32(1).u32(0x02).u32(rs.len() as u32);
        for r in &rs {
            p.rfx(*r);
        }
        p.u32(b.rules_in_block).u32(0);
        o.p(1046, &p);
        // BrtBeginCFRule14: 머리, 종류 4(데이터 막대), 틀 3, 차등 서식 없음, 우선순위 없음, … , GUID, 1
        let mut p = P::new();
        p.u32(0)
            .u32(4)
            .u32(3)
            .u32(0xFFFF_FFFF)
            .u32(0xFFFF_FFFF)
            .raw(&[0; 30])
            .raw(&b.guid)
            .u32(1);
        o.p(1048, &p);
        // BrtBeginDatabar14: 길이, 값 표시, 방향 (0 context, 1 leftToRight, 2 rightToLeft), 축 (0 automatic, 1 middle,
        // 2 none), 플래그 u16 (0x01 테두리, 0x02 그라데이션, 0x04 음수 색 다름, 0x08 음수 테두리 색 다름). 확장이 없는
        // 막대는 한셀이 만드는 기본 확장 (방향 leftToRight, 플래그 0x06)
        let ext = b
            .bar
            .ext
            .clone()
            .unwrap_or_else(|| crate::model::DataBarExt::implied(&b.bar));
        let dir = match ext.direction.as_deref() {
            Some("leftToRight") => 1,
            Some("rightToLeft") => 2,
            _ => 0,
        };
        let axis = match ext.axis_position.as_deref() {
            Some("middle") => 1,
            Some("none") => 2,
            _ => 0,
        };
        let flags = u16::from(ext.border)
            | (u16::from(ext.gradient) << 1)
            | (u16::from(!ext.negative_same_as_positive) << 2)
            | (u16::from(!ext.negative_border_same_as_positive) << 3);
        o.p(
            1051,
            P::new()
                .u32(0)
                .u8(b.bar.min_length.unwrap_or(10) as u8)
                .u8(b.bar.max_length.unwrap_or(90) as u8)
                .u8(u8::from(b.bar.show_value))
                .u8(dir)
                .u8(axis)
                .u16(flags),
        );
        for v in &ext.cfvos {
            let mut p = P::new();
            p.u32(0);
            p.raw(&cfvo_record(v, false).0);
            o.p(1050, &p);
        }
        // BrtColor14: 있는 색만 테두리 → 음수 막대 → 음수 테두리 → 축 순서로 (관측: cf_dv)
        for c in [
            &ext.border_color,
            &ext.negative_fill_color,
            &ext.negative_border_color,
            &ext.axis_color,
        ]
        .into_iter()
        .flatten()
        {
            let mut p = P::new();
            p.u32(0);
            p.raw(&cf_color(c));
            o.p(1055, &p);
        }
        o.e(1156);
        o.e(1049);
        o.e(1047);
    }
    o.e(1136);
    o.e(36);
}

/// 상위/하위 필터의 기준값 — 한셀의 바이너리 저장이 기준값 없는 필터에 쓰는 값 (상위/하위 필터 기준값 실험: 시트 22개의
/// 조작한 XML → 한셀 → 바이너리 BrtTop10Filter). 필터 범위의 머리글 아래 그 열의 숫자 값(글·빈 칸은 빼고, 숨은 행도
/// 넣는다)을 늘어놓고 위(아래)에서 k번째 값이다. k 는 개수, 백분율이면 ⌊숫자 수 × 값 / 100⌋ (적어도 1, 11개의 20% → 2,
/// 50% → 5, 5% → 1), 숫자 수보다 크면 끝 값. 같은 값도 하나씩 센다. 숫자가 없으면 0
fn top10_value(
    sheet: &Sheet,
    range: CellRange,
    col_id: u32,
    top: bool,
    percent: bool,
    val: f64,
) -> f64 {
    let col = range.start.col + col_id;
    let mut nums: Vec<f64> = sheet
        .cells
        .iter()
        .filter(|c| c.r.col == col && c.r.row > range.start.row && c.r.row <= range.end.row)
        .filter_map(|c| match c.value {
            CellValue::Number(n) => Some(n),
            _ => None,
        })
        .collect();
    if nums.is_empty() {
        return 0.0;
    }
    let n = nums.len();
    let k = if percent {
        // 정수 백분율의 곱이 부동소수 오차로 정수 바로 아래가 되지 않게
        ((n as f64 * val / 100.0) + 1e-9).floor().max(1.0) as usize
    } else {
        val.max(1.0) as usize
    }
    .min(n);
    if top {
        nums.sort_by(|a, b| b.total_cmp(a));
    } else {
        nums.sort_by(|a, b| a.total_cmp(b));
    }
    nums[k - 1]
}

fn auto_filter(o: &mut Out, sheet: &Sheet) {
    if let Some(af) = &sheet.auto_filter {
        auto_filter_records(o, af, sheet);
    }
}

/// 자동 필터 레코드 (시트와 표 파트가 같은 꼴): BrtBeginAFilter 161 … BrtEndAFilter 162. 상위/하위 기준값은 `sheet` 의
/// 셀로 셈한다
fn auto_filter_records(o: &mut Out, af: &crate::model::AutoFilter, sheet: &Sheet) {
    let Some(range) = ranges(&af.range).first().copied() else {
        return;
    };
    o.p(161, P::new().rfx(range));
    for c in &af.columns {
        o.p(163, P::new().u32(c.col_id).u16(0));
        if !c.values.is_empty() || c.blank {
            o.p(165, P::new().u32(u32::from(c.blank)).u32(0));
            for v in &c.values {
                o.p(167, P::new().wstr(v));
            }
            o.e(166);
        } else if !c.custom.is_empty() {
            o.p(172, P::new().u32(u32::from(c.custom_and)));
            for (op, v) in &c.custom {
                let sign = FILTER_OPERATORS
                    .iter()
                    .position(|x| *x == op.as_str())
                    .filter(|&i| i != 0)
                    .unwrap_or(2) as u8;
                let mut p = P::new();
                match v.trim().parse::<f64>() {
                    Ok(n) if !v.trim().is_empty() => {
                        p.u8(4).u8(sign).f64(n);
                    }
                    _ => {
                        let cch = v.encode_utf16().count().min(255) as u8;
                        p.u8(6).u8(sign).raw(&[0, 0, cch, 0, 0, 0, 0, 0]).wstr(v);
                    }
                }
                o.p(174, &p);
            }
            o.e(173);
        } else if let Some((top, percent, val)) = c.top10 {
            // 플래그: 0x01 위, 0x02 백분율, 0x04 (한셀이 늘 켠다), 개수, 기준값 (원본에 없으면 한셀처럼 데이터로 셈한다)
            let f = 0x04 | u8::from(top) | (u8::from(percent) << 1);
            let fv = c
                .filter_val
                .unwrap_or_else(|| top10_value(sheet, range, c.col_id, top, percent, val));
            o.p(170, P::new().u8(f).f64(val).f64(fv));
        } else if let Some(t) = &c.dynamic {
            let cft = crate::model::DYNAMIC_FILTER_TYPES
                .iter()
                .position(|x| *x == t.as_str())
                .unwrap_or(0) as u32;
            o.p(
                171,
                P::new()
                    .u32(cft)
                    .u8(0)
                    .f64(c.dynamic_val.unwrap_or(0.0))
                    .f64(c.dynamic_max_val.unwrap_or(0.0)),
            );
        }
        o.e(164);
    }
    o.e(162);
}

/// 머리글·바닥글 구역(`&L`·`&C`·`&R`)마다 글꼴 코드(`&"…"`)가 없으면 기본 글꼴 코드 `&"글꼴,Regular"` 를 앞에 넣는다 —
/// 한셀의 바이너리 저장이 하는 일 (관측: view_print `&L왼쪽` → `&L&"맑은 고딕,Regular"왼쪽`). 구역 표시가 없는 글은 가운데
/// 구역 하나로 본다
fn hf_with_font(s: &str, font: &str) -> String {
    let code = format!("&\"{font},Regular\"");
    // (구역 표시, 내용)
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut cur = (String::new(), String::new());
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '&'
            && c.get(i + 1)
                .is_some_and(|x| matches!(x, 'L' | 'C' | 'R' | 'l' | 'c' | 'r'))
        {
            if !cur.0.is_empty() || !cur.1.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.0 = c[i..i + 2].iter().collect();
            i += 2;
            continue;
        }
        if c[i] == '&' && c.get(i + 1) == Some(&'&') {
            cur.1.push_str("&&");
            i += 2;
            continue;
        }
        cur.1.push(c[i]);
        i += 1;
    }
    parts.push(cur);
    let mut out = String::new();
    for (mark, text) in parts {
        out.push_str(&mark);
        if !text.is_empty() && !text.contains("&\"") {
            out.push_str(&code);
        }
        out.push_str(&text);
    }
    out
}

/// 인쇄: BrtPrintOptions, BrtMargins, BrtPageSetup, 머리글·바닥글, 페이지 나누기
fn page(o: &mut Out, sheet: &Sheet, exact: bool, hf_font: Option<&str>, printer: Option<&str>) {
    let p = &sheet.page;
    // 0x01 가로 가운데, 0x02 세로 가운데, 0x04 머리글, 0x08 눈금선. 0x10 은 한셀 내부 상태 —
    // printOptions 가 없는 시트 104/108, 있는 시트 2/51 에 켜져 있다 (관측)
    let po = match &p.print_options {
        Some(x) => {
            u16::from(x.horizontal_centered)
                | (u16::from(x.vertical_centered) << 1)
                | (u16::from(x.headings) << 2)
                | (u16::from(x.grid_lines) << 3)
        }
        None => 0x10,
    };
    o.p(477, P::new().u16(po));
    let m = p.margins.clone().unwrap_or(crate::model::PageMargins {
        left: 0.7,
        right: 0.7,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    });
    o.p(
        476,
        P::new()
            .f64(m.left)
            .f64(m.right)
            .f64(m.top)
            .f64(m.bottom)
            .f64(m.header)
            .f64(m.footer),
    );
    // BrtPageSetup: 모델에 없으면 쓰지 않는다 (한셀 문서에는 늘 있다)
    if let Some(s) = &p.setup {
        page_setup(o, s, exact, printer);
    }
    // 머리글·바닥글은 글이 있을 때만 (한셀은 빈 headerFooter 를 쓰지 않는다, 관측)
    if let Some(h) = &p.header_footer {
        let texts = [
            &h.odd_header,
            &h.odd_footer,
            &h.even_header,
            &h.even_footer,
            &h.first_header,
            &h.first_footer,
        ];
        let flags = u16::from(h.different_odd_even)
            | (u16::from(h.different_first) << 1)
            | (u16::from(h.scale_with_doc) << 2)
            | (u16::from(h.align_with_margins) << 3);
        if texts.iter().any(|t| t.is_some()) || flags != 0x0C {
            let mut q = P::new();
            q.u16(flags);
            for t in texts {
                match (t, hf_font) {
                    (Some(t), Some(f)) => q.nwstr(Some(&hf_with_font(t, f))),
                    _ => q.nwstr(t.as_deref()),
                };
            }
            o.p(479, &q);
            o.e(480);
        }
    }
    for (begin, end, list, max) in [
        // 끝 칸: 한셀은 행 나누기에 1048575, 열 나누기에 16383 을 쓴다 (관측: view_print — XML 의 max 와 반대)
        (392u16, 393u16, &p.row_breaks, 1_048_575u32),
        (394, 395, &p.col_breaks, 16383),
    ] {
        if list.is_empty() {
            continue;
        }
        o.p(
            begin,
            P::new().u32(list.len() as u32).u32(list.len() as u32),
        );
        for id in list {
            o.p(396, P::new().u32(*id).u32(0).u32(max).u32(1).u32(0));
        }
        o.e(end);
    }
}

fn page_setup(o: &mut Out, s: &crate::model::PageSetup, exact: bool, printer: Option<&str>) {
    let mut flags = 0u16;
    if s.page_order.as_deref() == Some("overThenDown") {
        flags |= 0x0001;
    }
    match s.orientation.as_deref() {
        Some("landscape") => flags |= 0x0002,
        Some("portrait") => {}
        _ => flags |= 0x0040,
    }
    if !s.use_printer_defaults {
        flags |= 0x0004;
    }
    if s.black_and_white {
        flags |= 0x0008;
    }
    // 한셀은 draft 를 저장하지 않는다
    if s.draft && !exact {
        flags |= 0x0010;
    }
    if s.use_first_page_number {
        flags |= 0x0080;
    }
    o.p(
        478,
        P::new()
            .u32(s.paper_size.unwrap_or(9))
            .u32(s.scale.unwrap_or(100))
            .u32(s.horizontal_dpi.unwrap_or(0))
            .u32(s.vertical_dpi.unwrap_or(0))
            .u32(s.copies.unwrap_or(1))
            .u32(s.first_page_number.unwrap_or(1))
            .u32(s.fit_to_width.unwrap_or(1))
            .u32(s.fit_to_height.unwrap_or(1))
            .u16(flags)
            .wstr(printer.unwrap_or("")),
    );
}

/// 원본 그대로 옮기는 파트 가운데 바이너리 패키지에도 같은 꼴로 들어가는 것: 매크로 프로젝트(한컴·VBA), 사용자 지정
/// XML, 프린터 설정 (한셀의 바이너리 저장은 매크로 프로젝트만 지키고 나머지는 버린다 — 패키지 파트 실험). 연결·쿼리 표·
/// XML 대응은 바이너리에서 레코드 파트라 옮기지 않는다 (한셀도 세 형식에서 버린다)
fn binary_keeps(rel_type: &str) -> bool {
    [
        "/customXml",
        "/vbaProject",
        "/vbsProject",
        "/printerSettings",
    ]
    .iter()
    .any(|k| rel_type.ends_with(k))
}

// ---------------------------------------------------------------------------
// 통합 문서와 패키지
// ---------------------------------------------------------------------------

/// 관계 파트 XML
fn rels_xml(rels: &[Rel], base_dir: &str) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, ty, target, external) in rels {
        let t = if *external {
            target.clone()
        } else {
            relative(base_dir, target)
        };
        let _ = write!(
            s,
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
            esc(id),
            esc(ty),
            esc(&t),
            if *external {
                " TargetMode=\"External\""
            } else {
                ""
            }
        );
    }
    s.push_str("</Relationships>");
    s
}

/// 파트 이름 → 기준 폴더에서의 상대 경로 (`xl/worksheets` 에서 `xl/drawings/d.xml` → `../drawings/d.xml`)
fn relative(base_dir: &str, target: &str) -> String {
    let b: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    let t: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let common = b.iter().zip(&t).take_while(|(x, y)| x == y).count();
    let mut out: Vec<String> = vec!["..".to_string(); b.len() - common];
    out.extend(t[common..].iter().map(|s| s.to_string()));
    out.join("/")
}

/// 외부 연결 대상의 확장자 → 콘텐츠 타입 (한셀은 외부 대상의 확장자도 Default 로 등록한다, 관측: xls)
fn external_default(target: &str) -> Option<(&'static str, &'static str)> {
    let ext = target.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "xls" => ("xls", "application/vnd.ms-excel"),
        "xlsx" => (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        _ => return None,
    })
}

/// 통합 문서를 바이너리 `.cell` 로 쓰고 경고를 함께 돌려준다.
pub fn write_report(wb: &Workbook, opts: &Options) -> Result<Output> {
    let mut warnings = Vec::new();
    let worksheets: Vec<&Sheet> = wb
        .sheets
        .iter()
        .filter(|s| s.kind == SheetKind::Worksheet)
        .collect();
    let mut fonts = FontTable::new(&wb.styles);
    let mut sst = Sst::new(wb);
    let mut book = Book {
        sheets: worksheets.iter().map(|s| s.name.as_str()).collect(),
        links: &wb.external_links,
        xtis: Vec::new(),
        names: wb
            .defined_names
            .iter()
            .map(|d| NameEntry {
                name: d.name.clone(),
                local: d.local_sheet_id,
                hidden: d.hidden,
                formula: Some(d.formula.clone()).filter(|f| !f.is_empty()),
                func: false,
            })
            .collect(),
        cur: None,
        tables: worksheets
            .iter()
            .flat_map(|sh| {
                sh.tables.iter().map(|t| {
                    (
                        t.id,
                        t.display_name.clone(),
                        t.name.clone(),
                        t.columns.iter().map(|c| c.name.clone()).collect(),
                        sh.name.clone(),
                    )
                })
            })
            .collect(),
    };
    // 이름 정의를 먼저 토큰으로 (XTI 는 처음 쓰이는 순서로 번호를 받는다)
    let mut name_tokens: Vec<Option<Encoded>> = Vec::new();
    for i in 0..book.names.len() {
        let (text, local, label) = {
            let n = &book.names[i];
            (n.formula.clone(), n.local, n.name.clone())
        };
        book.cur = local;
        name_tokens.push(text.and_then(|t| {
            encode(
                &mut book,
                &mut warnings,
                &format!("이름 {label}"),
                &t,
                Place::Name,
            )
        }));
    }
    // 선택된 시트 (없으면 처음 보이는 시트) — XML 쓰기와 같다
    let selected = worksheets
        .iter()
        .position(|s| s.view.tab_selected)
        .or_else(|| {
            worksheets
                .iter()
                .position(|s| s.state == SheetState::Visible)
        })
        .unwrap_or(0);
    // 그리기 파트 이름 (원본 그대로 옮기는 파트와 겹치지 않게)
    let mut next_drawing = 1;
    let drawing_names: Vec<Option<String>> = worksheets
        .iter()
        .map(|sh| {
            if sh.shapes.is_empty() {
                return None;
            }
            loop {
                let n = format!("xl/drawings/drawing{next_drawing}.xml");
                next_drawing += 1;
                if !wb.raw_parts.iter().any(|p| p.name == n) {
                    return Some(n);
                }
            }
        })
        .collect();
    let mut sheets_out = Vec::new();
    let mut comments_n = 0;
    let mut model_charts: Vec<(String, String)> = Vec::new();
    // 피벗 캐시·표: 모델의 XML 원본 파트를 레코드 파트로
    let pivots = super::pivot::binary_parts(wb, &worksheets, &mut warnings, opts.exact);
    let pivot_tables: Vec<Vec<String>> = pivots
        .tables
        .iter()
        .map(|l| l.iter().map(|t| t.0.clone()).collect())
        .collect();
    {
        let mut ctx = SheetCtx {
            book: &mut book,
            sst: &mut sst,
            fonts: &mut fonts,
            warnings: &mut warnings,
            guid_seed: 0,
            mdw: crate::units::max_digit_width(wb),
            charts: &mut model_charts,
            exact: opts.exact,
            next_table: 0,
            pivot_tables: &pivot_tables,
        };
        for (i, sh) in worksheets.iter().enumerate() {
            if !sh.comments.is_empty() {
                comments_n += 1;
            }
            sheets_out.push(ctx.sheet(
                i,
                sh,
                i == selected,
                wb,
                drawing_names[i].clone(),
                comments_n,
            ));
        }
    }
    // 피벗 캐시의 원본 시트는 모든 시트의 수식 다음에 XTI 를 받는다 (쓰이지 않아도 — 피벗 실험 px1)
    for c in &pivots.caches {
        if let Some(s) = &c.source_sheet {
            let _ = book.xti(None, s, None);
        }
    }
    let sst_bin = sst.bin(&mut fonts);
    let styles = styles_bin(&wb.styles, &fonts, opts.exact);

    // ---- workbook.bin ----
    let mut wbrels: Vec<Rel> = Vec::new();
    let theme = wb.theme.as_ref().and_then(|t| t.source.as_deref());
    if theme.is_some() {
        wbrels.push((
            format!("rId{}", wbrels.len() + 1),
            format!("{REL_BASE}/theme"),
            "xl/theme/theme1.xml".into(),
            false,
        ));
    }
    wbrels.push((
        format!("rId{}", wbrels.len() + 1),
        format!("{REL_BASE}/styles"),
        "xl/styles.bin".into(),
        false,
    ));
    // 문자열이 하나도 없으면 한셀은 공유 문자열 파트를 쓰지 않는다 (관측: 수식·숫자만 있는 문서)
    let has_sst = !sst.items.is_empty();
    if has_sst {
        wbrels.push((
            format!("rId{}", wbrels.len() + 1),
            format!("{REL_BASE}/sharedStrings"),
            "xl/sharedStrings.bin".into(),
            false,
        ));
    }
    let mut sheet_ids = Vec::new();
    for i in 0..worksheets.len() {
        let id = format!("rId{}", wbrels.len() + 1);
        wbrels.push((
            id.clone(),
            format!("{REL_BASE}/worksheet"),
            format!("xl/worksheets/sheet{}.bin", i + 1),
            false,
        ));
        sheet_ids.push(id);
    }
    let mut link_ids = Vec::new();
    for i in 0..wb.external_links.len() {
        let id = format!("rId{}", wbrels.len() + 1);
        wbrels.push((
            id.clone(),
            format!("{REL_BASE}/externalLink"),
            format!("xl/externalLinks/externalLink{}.bin", i + 1),
            false,
        ));
        link_ids.push(id);
    }
    // 피벗 캐시 정의: 시트 다음 (피벗 실험 pf3: 시트 둘 뒤 rId6·rId7)
    let mut cache_ids = Vec::new();
    for c in &pivots.caches {
        let id = format!("rId{}", wbrels.len() + 1);
        wbrels.push((
            id.clone(),
            format!("{REL_BASE}/pivotCacheDefinition"),
            c.def_name.clone(),
            false,
        ));
        cache_ids.push(id);
    }
    // 원본 그대로 옮기는 통합 문서 수준 파트 (매크로 프로젝트·사용자 지정 XML). 한셀의 매크로 프로젝트 관계는 시트
    // 다음 (패키지 파트 실험의 vbs.bin.cell: rId4 = vbsProject, 콘텐츠 타입 application/vnd.hancom-office.vbsProject)
    for l in wb.part_links.iter().filter(|l| binary_keeps(&l.rel_type)) {
        wbrels.push((
            format!("rId{}", wbrels.len() + 1),
            l.rel_type.clone(),
            l.part.clone(),
            false,
        ));
    }
    // 레거시 원본의 매크로 프로젝트: `_VBS_PROJECT_W` 스트림 하나짜리 복합 문서 (XML 쓰기와 같은 파트 — 세 형식에서 스트림
    // 바이트가 같다. 복합 문서의 저장 시각은 한셀이 저장할 때마다 새로 쓴다)
    let raw_has = |n: &str| wb.raw_parts.iter().any(|p| p.name == n);
    let vbs_part = match &wb.hancom.vbs_project {
        Some(st)
            if !wb
                .part_links
                .iter()
                .any(|l| l.rel_type == crate::ns::REL_HANCOM_VBS_PROJECT)
                && !raw_has("xl/vbsProject.bin") =>
        {
            wbrels.push((
                format!("rId{}", wbrels.len() + 1),
                crate::ns::REL_HANCOM_VBS_PROJECT.into(),
                "xl/vbsProject.bin".into(),
                false,
            ));
            Some(crate::cfb_write::write_streams(&[(
                crate::ns::VBS_PROJECT_STREAM.trim_start_matches('/'),
                st,
            )])?)
        }
        _ => None,
    };

    let mut o = Out::default();
    o.e(131); // BrtBeginBook
    // BrtFileVersion: guidCodeName 0, 프로그램 이름·마지막 버전·최저 버전·빌드 — 한셀 2024 의 고정값 (관측)
    o.p(
        128,
        P::new()
            .raw(&[0; 16])
            .wstr("HCell")
            .wstr("13.0")
            .wstr("13.0")
            .wstr("0.711"),
    );
    // BrtWbProp: 플래그 (0x01 1904 날짜 체계), 테마 버전 0, 코드 이름
    o.p(
        153,
        P::new()
            .u32(u32::from(wb.date1904))
            .u32(0)
            .wstr("ThisWorkbook"),
    );
    // BrtBookProtection: 한셀은 구조 잠금이 없으면 통째로 잃는다 (docs/format/cell-binary.md 2.1) — rcell 은 쓴다
    if let Some(p) = wb
        .protection
        .as_ref()
        .filter(|p| p.lock_structure || !opts.exact)
    {
        let pw = p
            .password
            .as_deref()
            .and_then(|h| u16::from_str_radix(h.trim(), 16).ok())
            .unwrap_or(0);
        let f = u16::from(p.lock_structure) | (u16::from(p.lock_windows) << 1);
        o.p(534, P::new().u16(pw).u16(0).u16(f));
    }
    // 대체 내용 묶음: 버전 1개 (0x0F03, 0x8000), BrtAbsPath15 (저장 폴더)
    o.p(37, P::new().raw(&[0x01, 0x00, 0x03, 0x0F, 0x00, 0x80]));
    o.p(2071, P::new().wstr(opts.abs_path.as_deref().unwrap_or("")));
    o.e(38);
    o.e(135);
    // 창이 없는 문서는 OOXML 기본값 (XML 쓰기와 같게: tabRatio 600)
    let bv = wb.view.clone().unwrap_or_default();
    // 플래그: 0x08 가로 스크롤, 0x10 세로 스크롤, 0x20 시트 탭 (통합 문서 창 대조 실험)
    let bv_flags = (u8::from(bv.show_horizontal_scroll) << 3)
        | (u8::from(bv.show_vertical_scroll) << 4)
        | (u8::from(bv.show_sheet_tabs) << 5);
    o.p(
        158,
        P::new()
            .i32(bv.x_window)
            .i32(bv.y_window)
            .u32(bv.window_width)
            .u32(bv.window_height)
            .u32(bv.tab_ratio)
            .u32(bv.first_sheet)
            .u32(selected as u32)
            .u8(bv_flags),
    );
    o.e(136);
    o.e(143);
    for (i, sh) in worksheets.iter().enumerate() {
        let state = match sh.state {
            SheetState::Visible => 0,
            SheetState::Hidden => 1,
            SheetState::VeryHidden => 2,
        };
        o.p(
            156,
            P::new()
                .u32(state)
                .u32(i as u32 + 1)
                .nwstr(Some(&sheet_ids[i]))
                .wstr(&sh.name),
        );
    }
    o.e(144);
    o.e(353);
    let supbooks = book.supbooks();
    for sb in &supbooks {
        match sb {
            SupBook::This => o.e(357),
            SupBook::Ext(k) => o.p(355, P::new().wstr(&link_ids[*k])),
        }
    }
    if !supbooks.is_empty() {
        let mut p = P::new();
        p.u32(book.xtis.len() as u32);
        for (sb, f, l) in &book.xtis {
            let i = supbooks.iter().position(|x| x == sb).unwrap_or(0);
            p.u32(i as u32).i32(*f).i32(*l);
        }
        o.p(362, &p);
    }
    o.e(354);
    for (i, n) in book.names.iter().enumerate() {
        let builtin = n.name.starts_with("_xlnm.");
        let mut flags = 0u32;
        if n.hidden {
            flags |= 0x01;
        }
        if n.func {
            flags |= 0x02;
        }
        if builtin {
            flags |= 0x20;
        }
        if n.name.starts_with("_xlfn.") {
            flags |= 0x02 | 0x2_0000;
        }
        let name = n.name.strip_prefix("_xlnm.").unwrap_or(&n.name);
        let empty = Encoded::default();
        let e = name_tokens
            .get(i)
            .and_then(Option::as_ref)
            .unwrap_or(&empty);
        o.p(
            39,
            P::new()
                .u32(flags)
                .u8(0)
                .u32(n.local.unwrap_or(0xFFFF_FFFF))
                .wstr(name)
                .fmla(e)
                .nwstr(None),
        );
    }
    // BrtCalcProp: 한셀 2024 의 고정값 (자동 계산, 반복 100회, 변화 0.001, 플래그 0x2A — 관측)
    o.p(
        157,
        P::new().raw(&[
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x64, 0x00, 0x00, 0x00, 0xFC, 0xA9,
            0xF1, 0xD2, 0x4D, 0x62, 0x50, 0x3F, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x00,
        ]),
    );
    // 피벗 캐시 목록: 캐시 번호(idSx)와 정의 파트의 관계 ID
    if !pivots.caches.is_empty() {
        o.e(384);
        for (c, id) in pivots.caches.iter().zip(&cache_ids) {
            o.p(386, P::new().u32(c.id).wstr(id));
            o.e(387);
        }
        o.e(385);
    }
    o.e(132);
    let workbook_bin = o.0;

    // ---- 파트 모으기 ----
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut defaults: Vec<(String, String)> = vec![
        (
            "rels".into(),
            "application/vnd.openxmlformats-package.relationships+xml".into(),
        ),
        ("xml".into(), "application/xml".into()),
    ];
    let add_default = |defaults: &mut Vec<(String, String)>, ext: &str, ct: &str| {
        if !defaults.iter().any(|(e, _)| e.eq_ignore_ascii_case(ext)) {
            defaults.push((ext.to_ascii_lowercase(), ct.to_string()));
        }
    };
    let root_rels = vec![
        (
            "rId1".to_string(),
            format!("{REL_BASE}/officeDocument"),
            "xl/workbook.bin".to_string(),
            false,
        ),
        (
            "rId2".to_string(),
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties"
                .to_string(),
            "docProps/core.xml".to_string(),
            false,
        ),
        (
            "rId3".to_string(),
            format!("{REL_BASE}/extended-properties"),
            "docProps/app.xml".to_string(),
            false,
        ),
    ];
    parts.push(("_rels/.rels".into(), rels_xml(&root_rels, "").into_bytes()));
    parts.push((
        "docProps/core.xml".into(),
        core_xml(&wb.properties).into_bytes(),
    ));
    overrides.push((
        "docProps/core.xml".into(),
        "application/vnd.openxmlformats-package.core-properties+xml".into(),
    ));
    parts.push((
        "docProps/app.xml".into(),
        app_xml(&worksheets, wb.total_time).into_bytes(),
    ));
    overrides.push((
        "docProps/app.xml".into(),
        "application/vnd.openxmlformats-officedocument.extended-properties+xml".into(),
    ));
    parts.push(("xl/workbook.bin".into(), workbook_bin));
    overrides.push((
        "xl/workbook.bin".into(),
        "application/vnd.ms-excel.sheet.binary.macroEnabled.main".into(),
    ));
    parts.push((
        "xl/_rels/workbook.bin.rels".into(),
        rels_xml(&wbrels, "xl").into_bytes(),
    ));
    if let Some(t) = theme {
        parts.push(("xl/theme/theme1.xml".into(), t.to_vec()));
        overrides.push((
            "xl/theme/theme1.xml".into(),
            "application/vnd.openxmlformats-officedocument.theme+xml".into(),
        ));
    }
    parts.push(("xl/styles.bin".into(), styles));
    overrides.push((
        "xl/styles.bin".into(),
        "application/vnd.ms-excel.styles".into(),
    ));
    if has_sst {
        parts.push(("xl/sharedStrings.bin".into(), sst_bin));
        overrides.push((
            "xl/sharedStrings.bin".into(),
            "application/vnd.ms-excel.sharedStrings".into(),
        ));
    }
    // 차트 모델이 있는 차트 파트는 모델로 다시 쓴다 (chart_xml), 나머지 원본 파트는 그대로
    let mut charts: HashMap<String, &crate::model::Chart> = HashMap::new();
    fn collect<'a>(
        list: &'a [crate::model::Shape],
        out: &mut HashMap<String, &'a crate::model::Chart>,
    ) {
        for s in list {
            if let (Some(t), Some(c)) = (&s.target, &s.chart) {
                out.insert(t.clone(), c);
            }
            collect(&s.children, out);
        }
    }
    for sh in &worksheets {
        collect(&sh.shapes, &mut charts);
    }
    for (i, so) in sheets_out.into_iter().enumerate() {
        let name = format!("xl/worksheets/sheet{}.bin", i + 1);
        parts.push((name.clone(), so.bin));
        overrides.push((name.clone(), "application/vnd.ms-excel.worksheet".into()));
        for (tname, data) in so.tables {
            overrides.push((tname.clone(), "application/vnd.ms-excel.table".into()));
            parts.push((tname, data));
        }
        if !so.rels.is_empty() {
            parts.push((
                crate::package::rels_part_for(&name),
                rels_xml(&so.rels, "xl/worksheets").into_bytes(),
            ));
        }
        if let Some(c) = so.comments {
            let cname = so
                .rels
                .iter()
                .find(|r| r.1.ends_with("/comments"))
                .map(|r| r.2.clone())
                .unwrap_or_default();
            parts.push((cname.clone(), c));
            overrides.push((cname, "application/vnd.ms-excel.comments".into()));
        }
        if let Some(v) = so.vml {
            let vname = so
                .rels
                .iter()
                .find(|r| r.1.ends_with("/vmlDrawing"))
                .map(|r| r.2.clone())
                .unwrap_or_default();
            parts.push((vname, v.into_bytes()));
            add_default(
                &mut defaults,
                "vml",
                "application/vnd.openxmlformats-officedocument.vmlDrawing",
            );
        }
        if let Some((dname, xml, drels)) = so.drawing {
            parts.push((dname.clone(), xml.into_bytes()));
            overrides.push((
                dname.clone(),
                "application/vnd.openxmlformats-officedocument.drawing+xml".into(),
            ));
            if !drels.is_empty() {
                parts.push((
                    crate::package::rels_part_for(&dname),
                    rels_xml(&drels, "xl/drawings").into_bytes(),
                ));
            }
        }
    }
    // 피벗 파트: 캐시 정의 (→ 레코드 rId1), 레코드, 피벗 표 (→ 캐시 정의 rId1) — 한셀 바이너리와 같은 관계
    for c in pivots.caches {
        if let Some((rname, data)) = c.records {
            parts.push((
                crate::package::rels_part_for(&c.def_name),
                rels_xml(
                    &[(
                        "rId1".into(),
                        format!("{REL_BASE}/pivotCacheRecords"),
                        rname.clone(),
                        false,
                    )],
                    "xl/pivotCache",
                )
                .into_bytes(),
            ));
            overrides.push((
                rname.clone(),
                "application/vnd.ms-excel.pivotCacheRecords".into(),
            ));
            parts.push((rname, data));
        }
        overrides.push((
            c.def_name.clone(),
            "application/vnd.ms-excel.pivotCacheDefinition".into(),
        ));
        parts.push((c.def_name, c.def));
    }
    for (name, data, cache) in pivots.tables.into_iter().flatten() {
        parts.push((
            crate::package::rels_part_for(&name),
            rels_xml(
                &[(
                    "rId1".into(),
                    format!("{REL_BASE}/pivotCacheDefinition"),
                    cache,
                    false,
                )],
                "xl/pivotTables",
            )
            .into_bytes(),
        ));
        overrides.push((name.clone(), "application/vnd.ms-excel.pivotTable".into()));
        parts.push((name, data));
    }
    // 원본 파트가 없는 모델 차트 (레거시에서 읽은 차트)
    for (name, xml) in model_charts {
        overrides.push((
            name.clone(),
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml".into(),
        ));
        parts.push((name, xml.into_bytes()));
    }
    // XML 원본의 표 파트는 표 모델에서 레코드 파트로 새로 썼다 — XML 파트는 옮기지 않는다. 바이너리에 둘 수 없는 원본
    // 파트([`binary_keeps`] 가 아닌 관계)도 관계가 가리키는 파트째 뺀다
    let mut dropped: Vec<String> = worksheets
        .iter()
        .flat_map(|sh| sh.tables.iter().filter_map(|t| t.part.clone()))
        .flat_map(|p| [crate::package::rels_part_for(&p), p])
        .collect();
    for l in wb
        .part_links
        .iter()
        .chain(worksheets.iter().flat_map(|sh| sh.part_links.iter()))
        .filter(|l| !binary_keeps(&l.rel_type))
    {
        let mut tree = Vec::new();
        crate::package::raw_part_tree(&wb.raw_parts, &l.part, &mut tree);
        for n in tree {
            if !dropped.contains(&n) {
                dropped.push(n);
            }
        }
    }
    if let Some(v) = vbs_part {
        parts.push(("xl/vbsProject.bin".into(), v));
        overrides.push((
            "xl/vbsProject.bin".into(),
            crate::ns::CT_HANCOM_VBS_PROJECT.into(),
        ));
    }
    for p in &wb.raw_parts {
        if parts.iter().any(|(n, _)| *n == p.name) || dropped.contains(&p.name) {
            continue;
        }
        let data = match charts.get(&p.name) {
            Some(c) => crate::chart_xml::chart_xml(c).into_bytes(),
            None => p.data.clone(),
        };
        parts.push((p.name.clone(), data));
        if let Some(t) = &p.content_type
            && !p.name.ends_with(".rels")
        {
            if t.starts_with("image/")
                && let Some(ext) = p.name.rsplit('.').next()
            {
                add_default(&mut defaults, ext, t);
            } else {
                overrides.push((p.name.clone(), t.clone()));
            }
        }
    }
    for (i, link) in wb.external_links.iter().enumerate() {
        let name = format!("xl/externalLinks/externalLink{}.bin", i + 1);
        let mut x = Out::default();
        // BrtBeginSupBook: sbt 0 (외부), 관계 ID, 빈 두째 문자열
        x.p(360, P::new().u16(0).nwstr(Some("rId1")).nwstr(None));
        let mut p = P::new();
        p.u32(link.sheet_names.len() as u32);
        for s in &link.sheet_names {
            p.wstr(s);
        }
        x.p(359, &p);
        x.e(588);
        parts.push((name.clone(), x.0));
        overrides.push((name.clone(), "application/vnd.ms-excel.externalLink".into()));
        // 한셀은 경로를 잃은 연결도 보통의 externalLinkPath 로 쓴다 (docs/format/cell-binary.md 2.1)
        parts.push((
            crate::package::rels_part_for(&name),
            rels_xml(
                &[(
                    "rId1".into(),
                    format!("{REL_BASE}/externalLinkPath"),
                    link.target.clone(),
                    true,
                )],
                "xl/externalLinks",
            )
            .into_bytes(),
        ));
        if let Some((e, ct)) = external_default(&link.target) {
            add_default(&mut defaults, e, ct);
        }
    }
    defaults.sort_by(|a, b| a.0.cmp(&b.0));
    overrides.sort_by(|a, b| a.0.cmp(&b.0));
    let mut ct = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    );
    for (e, t) in &defaults {
        let _ = write!(
            ct,
            "<Default Extension=\"{}\" ContentType=\"{}\"/>",
            esc(e),
            esc(t)
        );
    }
    for (n, t) in &overrides {
        let _ = write!(
            ct,
            "<Override PartName=\"/{}\" ContentType=\"{}\"/>",
            esc(n),
            esc(t)
        );
    }
    ct.push_str("</Types>");
    parts.push(("[Content_Types].xml".into(), ct.into_bytes()));
    // 한셀은 파트를 이름 순으로 담는다 (관측)
    parts.sort_by(|a, b| a.0.cmp(&b.0));

    let mut z = ZipWriter::new(Cursor::new(Vec::new()));
    let zopts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in &parts {
        z.start_file(name.as_str(), zopts)?;
        z.write_all(data)?;
    }
    let mut bytes = z.finish()?.into_inner();
    // 구조 잠금이 걸린 문서는 한셀처럼 기본 암호로 감싼다 (super::encrypt)
    if wb.protection.as_ref().is_some_and(|p| p.lock_structure) {
        bytes = super::encrypt::wrap(&bytes);
    }
    Ok(Output { bytes, warnings })
}

/// docProps/app.xml — 한셀이 쓰는 꼴 (프로그램 이름, 시트 이름 목록, 편집 시간, 버전).
/// 편집 시간(분)은 원본 값을 옮긴다 (한셀 관측: 서식 파일의 `TotalTime` 51 이 바이너리에도 51)
fn app_xml(sheets: &[&Sheet], total_time: Option<u32>) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?><ep:Properties xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:ep=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><ep:Application>Cell</ep:Application><ep:TitlesOfParts>",
    );
    let _ = write!(
        s,
        "<vt:vector size=\"{}\" baseType=\"lpstr\">",
        sheets.len()
    );
    for sh in sheets {
        let _ = write!(s, "<vt:lpstr>{}</vt:lpstr>", esc(&sh.name));
    }
    let minutes = total_time.unwrap_or(0);
    let _ = write!(
        s,
        "</vt:vector></ep:TitlesOfParts><ep:TotalTime>{minutes}</ep:TotalTime><ep:AppVersion>12.0300</ep:AppVersion></ep:Properties>"
    );
    s
}

/// docProps/core.xml — 문서 속성 (한셀은 늘 쓴다). 수정 날짜·수정한 사람 같은 값은 모델에 있는 것만
fn core_xml(p: &crate::model::DocProperties) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?><cp:coreProperties xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
    );
    for (tag, v) in [
        ("dc:title", &p.title),
        ("dc:subject", &p.subject),
        ("dc:creator", &p.creator),
        ("cp:keywords", &p.keywords),
        ("dc:description", &p.description),
        ("cp:lastModifiedBy", &p.last_modified_by),
        ("cp:revision", &p.revision),
        ("cp:lastPrinted", &p.last_printed),
        ("cp:category", &p.category),
        ("cp:contentStatus", &p.content_status),
    ] {
        if let Some(v) = v {
            let _ = write!(s, "<{tag}>{}</{tag}>", esc(v));
        }
    }
    for (tag, v) in [
        ("dcterms:created", &p.created),
        ("dcterms:modified", &p.modified),
    ] {
        if let Some(v) = v {
            let _ = write!(s, "<{tag} xsi:type=\"dcterms:W3CDTF\">{}</{tag}>", esc(v));
        }
    }
    // 한셀 2024 는 문서와 상관없이 늘 자기 버전을 끝에 쓴다 (관측: 코퍼스의 바이너리 114개 모두)
    s.push_str("<cp:version>1300.0100.01</cp:version></cp:coreProperties>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_header_varints() {
        let mut o = Out::default();
        o.rec(2071, &[0; 200]);
        // 2071 = 0x817 → 97 10, 길이 200 → C8 01
        assert_eq!(&o.0[..4], &[0x97, 0x10, 0xC8, 0x01]);
        let mut o = Out::default();
        o.e(0);
        assert_eq!(o.0, [0, 0]);
    }

    #[test]
    fn rk_only_for_exact_high_word_doubles() {
        assert_eq!(rk(1.0), Some(0x3FF0_0000));
        assert_eq!(rk(-1.0), Some(0xBFF0_0000));
        assert_eq!(rk(0.0), Some(0));
        assert_eq!(rk(0.1), None);
        assert_eq!(rk(1e15), None);
        assert_eq!(rk(std::f64::consts::PI), None);
    }

    #[test]
    fn header_footer_gets_default_font_codes_like_hancell() {
        // 관측: view_print 의 머리글을 한셀이 바이너리로 저장한 것
        assert_eq!(
            hf_with_font("&L왼쪽&C&\"굴림,Bold\"가운데&R&P / &N", "맑은 고딕"),
            "&L&\"맑은 고딕,Regular\"왼쪽&C&\"굴림,Bold\"가운데&R&\"맑은 고딕,Regular\"&P / &N"
        );
        assert_eq!(
            hf_with_font("&C&D &T", "맑은 고딕"),
            "&C&\"맑은 고딕,Regular\"&D &T"
        );
        assert_eq!(hf_with_font("&L&C&R", "X"), "&L&C&R");
    }

    #[test]
    fn top10_filter_values_like_hancell() {
        // 상위/하위 필터 기준값 실험의 한셀 바이너리 값 (머리글 + 값, A열)
        let sheet_of = |vals: &[Option<f64>]| {
            let mut s = Sheet::default();
            s.cells.push(Cell {
                r: CellRef { row: 0, col: 0 },
                style: 0,
                value: CellValue::Text("값".into()),
                formula: None,
            });
            for (i, v) in vals.iter().enumerate() {
                s.cells.push(Cell {
                    r: CellRef {
                        row: i as u32 + 1,
                        col: 0,
                    },
                    style: 0,
                    value: v.map_or(CellValue::Text("x".into()), CellValue::Number),
                    formula: None,
                });
            }
            let range = CellRange {
                start: CellRef { row: 0, col: 0 },
                end: CellRef {
                    row: vals.len() as u32,
                    col: 0,
                },
            };
            (s, range)
        };
        let d1: Vec<Option<f64>> = (-4..=6).map(|v| Some(f64::from(v))).collect();
        let (s, r) = sheet_of(&d1);
        let v = |top, percent, val| top10_value(&s, r, 0, top, percent, val);
        assert_eq!(v(false, true, 20.0), -3.0);
        assert_eq!(v(true, true, 20.0), 5.0);
        assert_eq!(v(false, true, 10.0), -4.0);
        assert_eq!(v(false, true, 5.0), -4.0);
        assert_eq!(v(true, true, 50.0), 2.0);
        assert_eq!(v(true, true, 30.0), 4.0);
        assert_eq!(v(false, false, 3.0), -2.0);
        assert_eq!(v(true, false, 20.0), -4.0);
        assert_eq!(v(false, false, 20.0), 6.0);
        let ties: Vec<Option<f64>> = [5., 5., 5., 4., 4., 3., 2., 1., 1., 0.]
            .iter()
            .map(|v| Some(*v))
            .collect();
        let (s, r) = sheet_of(&ties);
        assert_eq!(top10_value(&s, r, 0, true, false, 3.0), 5.0);
        assert_eq!(top10_value(&s, r, 0, true, false, 4.0), 4.0);
        assert_eq!(top10_value(&s, r, 0, false, false, 2.0), 1.0);
        let mixed = [
            Some(10.),
            None,
            Some(30.),
            None,
            Some(20.),
            Some(50.),
            None,
            Some(40.),
        ];
        let (s, r) = sheet_of(&mixed);
        assert_eq!(top10_value(&s, r, 0, true, false, 2.0), 40.0);
        let d8: Vec<Option<f64>> = (1..=25).map(|v| Some(f64::from(v))).collect();
        let (s, r) = sheet_of(&d8);
        assert_eq!(top10_value(&s, r, 0, true, true, 10.0), 24.0);
        assert_eq!(top10_value(&s, r, 0, false, true, 10.0), 2.0);
    }

    #[test]
    fn relative_targets() {
        assert_eq!(
            relative("xl/worksheets", "xl/drawings/drawing1.xml"),
            "../drawings/drawing1.xml"
        );
        assert_eq!(relative("xl", "xl/styles.bin"), "styles.bin");
        assert_eq!(relative("", "xl/workbook.bin"), "xl/workbook.bin");
    }
}
