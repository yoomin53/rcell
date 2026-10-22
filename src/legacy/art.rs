// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 그리기 쓰기: OfficeArt([MS-ODRAW]) 레코드를 MSODRAWINGGROUP / MSODRAWING 에, 도형마다 OBJ (+ TXO),
//! 메모는 NOTE 로 — [`super::escher`] 와 읽기의 `legacy_shape` 의 반대 (6.2절).
//!
//! 한셀의 배치 (관측): 시트 하나의 OfficeArt 는 `DgContainer → FDG + SpgrContainer → patriarch, 도형…` 이고
//! 도형마다 SpContainer 가 MSODRAWING 하나, 그 뒤에 OBJ, 글이 있으면 ClientTextbox(F00D) 만 든 MSODRAWING 과 TXO 가
//! 이어진다. 순서는 자동 필터 단추(콤보 상자), 그리기 개체, 메모. 도형 번호(spid)는 `그리기 번호 × 0x400 + n`.
//! 그림은 전역 MSODRAWINGGROUP 의 BStore 에 FBSE + 그림 레코드로 들어가고 rgbUid 는 그림 데이터의 MD4 다.

use std::collections::HashMap;

use super::writer::{Buf, Out, SheetCtx, units};
use crate::error::Result;
use crate::model::{Anchor, Comment, DrawColor, Paint, Shape, ShapeKind, Workbook};

// ---------------------------------------------------------------------------
// MD4 (RFC 1320) — FBSE 의 rgbUid
// ---------------------------------------------------------------------------

fn md4(data: &[u8]) -> [u8; 16] {
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    let (mut a, mut b, mut c, mut d) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let f = |x: u32, y: u32, z: u32| (x & y) | (!x & z);
    let g = |x: u32, y: u32, z: u32| (x & y) | (x & z) | (y & z);
    let h = |x: u32, y: u32, z: u32| x ^ y ^ z;
    for chunk in msg.chunks(64) {
        let x: Vec<u32> = (0..16)
            .map(|i| u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap()))
            .collect();
        let (aa, bb, cc, dd) = (a, b, c, d);
        for &i in &[0usize, 4, 8, 12] {
            a = a.wrapping_add(f(b, c, d)).wrapping_add(x[i]).rotate_left(3);
            d = d
                .wrapping_add(f(a, b, c))
                .wrapping_add(x[i + 1])
                .rotate_left(7);
            c = c
                .wrapping_add(f(d, a, b))
                .wrapping_add(x[i + 2])
                .rotate_left(11);
            b = b
                .wrapping_add(f(c, d, a))
                .wrapping_add(x[i + 3])
                .rotate_left(19);
        }
        for &i in &[0usize, 1, 2, 3] {
            let k = 0x5A82_7999u32;
            a = a
                .wrapping_add(g(b, c, d))
                .wrapping_add(x[i])
                .wrapping_add(k)
                .rotate_left(3);
            d = d
                .wrapping_add(g(a, b, c))
                .wrapping_add(x[i + 4])
                .wrapping_add(k)
                .rotate_left(5);
            c = c
                .wrapping_add(g(d, a, b))
                .wrapping_add(x[i + 8])
                .wrapping_add(k)
                .rotate_left(9);
            b = b
                .wrapping_add(g(c, d, a))
                .wrapping_add(x[i + 12])
                .wrapping_add(k)
                .rotate_left(13);
        }
        for &i in &[0usize, 2, 1, 3] {
            let k = 0x6ED9_EBA1u32;
            a = a
                .wrapping_add(h(b, c, d))
                .wrapping_add(x[i])
                .wrapping_add(k)
                .rotate_left(3);
            d = d
                .wrapping_add(h(a, b, c))
                .wrapping_add(x[i + 8])
                .wrapping_add(k)
                .rotate_left(9);
            c = c
                .wrapping_add(h(d, a, b))
                .wrapping_add(x[i + 4])
                .wrapping_add(k)
                .rotate_left(11);
            b = b
                .wrapping_add(h(c, d, a))
                .wrapping_add(x[i + 12])
                .wrapping_add(k)
                .rotate_left(15);
        }
        a = a.wrapping_add(aa);
        b = b.wrapping_add(bb);
        c = c.wrapping_add(cc);
        d = d.wrapping_add(dd);
    }
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a.to_le_bytes());
    out[4..8].copy_from_slice(&b.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out[12..16].copy_from_slice(&d.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------
// OfficeArt 레코드
// ---------------------------------------------------------------------------

/// OfficeArt 레코드 머리 + 내용
fn rec(ver: u16, inst: u16, ty: u16, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + body.len());
    v.extend_from_slice(&((ver & 0xF) | (inst << 4)).to_le_bytes());
    v.extend_from_slice(&ty.to_le_bytes());
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    v
}

/// 도형 속성 (OPT): (pid, 값, 복합 데이터)
struct Props(Vec<(u16, u32, Vec<u8>)>);

impl Props {
    fn set(&mut self, pid: u16, v: u32) {
        self.0.push((pid, v, Vec::new()));
    }
    /// 복합 속성 (내용 바이트 그대로)
    fn complex(&mut self, pid: u16, d: Vec<u8>) {
        self.0.push((pid | 0xC000, d.len() as u32, d));
    }
    fn string(&mut self, pid: u16, s: &str) {
        let mut d = Buf::new();
        d.utf16(s).u16(0);
        let d = d.take();
        // 복합 속성: fComplex 0x8000 + fBid 0x4000 (관측: 한셀의 이름·설명 0xC380·0xC381)
        self.0.push((pid | 0xC000, d.len() as u32, d));
    }
    fn record(mut self) -> Vec<u8> {
        self.0.sort_by_key(|p| p.0 & 0x3FFF);
        let mut b = Buf::new();
        for (pid, v, _) in &self.0 {
            b.u16(*pid).u32(*v);
        }
        for (_, _, d) in &self.0 {
            b.bytes(d);
        }
        rec(3, self.0.len() as u16, 0xF00B, &b.0)
    }
}

/// 한컴 ClientAnchor 22바이트
fn client_anchor(flag: u16, a: [u32; 8]) -> Vec<u8> {
    let mut b = Buf::new();
    b.u16(flag)
        .u16(a[0] as u16)
        .u16(a[1] as u16)
        .u32(a[2])
        .u16(a[3] as u16)
        .u16(a[4] as u16)
        .u16(a[5] as u16)
        .u32(a[6])
        .u16(a[7] as u16);
    rec(0, 0, 0xF010, &b.0)
}

fn draw_rgb(c: &DrawColor) -> Option<u32> {
    let v = c.value.strip_prefix("srgb:")?;
    u32::from_str_radix(v, 16).ok()
}

/// 앵커 오프셋(EMU)을 한셀처럼 정수 픽셀로 올린다 (관측: craft/odd_anchor 의 1.30→2, 2.92→3, 3.50→4, 5.70→6 px)
pub(crate) fn anchor_px(off: i64) -> i64 {
    (off.max(0) + crate::units::EMU_PER_PX - 1) / crate::units::EMU_PER_PX
}

/// 모델 앵커 → 레거시 (col1, dx1, rw1, dy1, col2, dx2, rw2, dy2). dx 는 (열 픽셀 − 1)의 1/32768, dy 는 (행 픽셀 − 1)의
/// 1/8192 (6.2절, 읽기의 반대). 오프셋은 정수 픽셀로 올리고, 칸 끝까지면 dx 32767·dy 8191 로 자른다 (관측)
fn legacy_anchor(a: &Anchor, ctx: &SheetCtx) -> [u32; 8] {
    legacy_anchor_ws(a, ctx.wb, ctx.sheet)
}

/// [`legacy_anchor`] — 통합 문서와 시트로 (차트 레코드 0x7EF0 의 앵커도 같은 단위다)
pub(crate) fn legacy_anchor_ws(
    a: &Anchor,
    wb: &crate::model::Workbook,
    sheet: &crate::model::Sheet,
) -> [u32; 8] {
    let mdw = crate::units::max_digit_width(wb);
    let col_d = |c: u32, off: i64| -> u32 {
        let eff = crate::units::sheet_col_px(sheet, c, mdw).saturating_sub(1) as f64;
        let px = anchor_px(off) as f64;
        if eff <= 0.0 {
            0
        } else {
            ((px * 32768.0 / eff).round() as i64).clamp(0, 0x7FFF) as u32
        }
    };
    let row_d = |r: u32, off: i64| -> u32 {
        let eff = crate::units::sheet_row_px(sheet, r).saturating_sub(1) as f64;
        let px = anchor_px(off) as f64;
        if eff <= 0.0 {
            0
        } else {
            ((px * 8192.0 / eff).round() as i64).clamp(0, 0x1FFF) as u32
        }
    };
    let from = a.from;
    let to = a.to.unwrap_or(from);
    [
        from.col,
        col_d(from.col, from.col_off),
        from.row,
        row_d(from.row, from.row_off),
        to.col,
        col_d(to.col, to.col_off),
        to.row,
        row_d(to.row, to.row_off),
    ]
}

/// 그림 저장소의 그림 하나
struct Blip {
    /// FBSE 의 btWin32 (2 EMF, 3 WMF, 5 JPEG, 6 PNG, 7 DIB)
    bt: u8,
    data: Vec<u8>,
    uid: [u8; 16],
}

/// 시트 하나의 그리기 개체
enum Obj<'a> {
    Button { col: u32, row: u32 },
    Shape(&'a Shape),
    Note(&'a Comment),
}

pub(crate) struct Art {
    blips: Vec<Blip>,
    /// 그림 파트 이름 → pib (1부터)
    pib: HashMap<String, u32>,
    /// 시트마다 그리기 번호 (그리기 개체가 없으면 None)
    dgid: Vec<Option<u32>>,
    /// 시트마다 OfficeArt 도형 수 (patriarch 포함)
    csp: Vec<u32>,
    /// 글 상자 문단 모양의 0xA001 번호 ([`has_text_shapes`] 일 때 표 끝에 하나 더 쓴다). TXO 의 문단이 가리킨다
    pub text_para: Option<u32>,
    /// 메모 글의 가로 맞춤(한글 문단 정렬 번호, [`note_hangul_align`]) → 그 맞춤의 문단 모양 번호
    pub note_para: HashMap<u8, u32>,
}

/// 메모의 가로 맞춤 → 한글 문단 정렬 번호 (2 오른쪽, 3 가운데, 0 양쪽, 4 배분). 왼쪽(기본)은 None — 메모 대조
/// 실험: 가운데 맞춤 메모의 TXO 문단이 정렬 3 인 문단 모양을 가리킨다
pub(crate) fn note_hangul_align(h: Option<&str>) -> Option<u8> {
    match h {
        Some("Center") => Some(3),
        Some("Right") => Some(2),
        Some("Justify") => Some(0),
        Some("Distributed") => Some(4),
        _ => None,
    }
}

/// 글이 있는 도형 (글 상자 TXO 를 쓰는 도형). XML 의 글 상자(`xdr:cNvSpPr txBox="1"`)는 글이 비어도 쓴다
pub(crate) fn shape_has_text(s: &Shape) -> bool {
    (s.text.as_ref().is_some_and(|t| !t.paragraphs.is_empty()) && !s.plain_text().is_empty())
        || s.raw.as_deref().is_some_and(|r| r.contains("txBox=\"1\""))
}

/// 통합 문서에 글이 있는 도형이 있나 (그룹 안까지)
pub(crate) fn has_text_shapes(wb: &Workbook) -> bool {
    fn any(v: &[Shape]) -> bool {
        v.iter()
            .any(|s| (s.kind != ShapeKind::Chart && shape_has_text(s)) || any(&s.children))
    }
    wb.sheets.iter().any(|s| any(&s.shapes))
}

fn sheet_objects(sheet: &crate::model::Sheet) -> Vec<Obj<'_>> {
    let mut v = Vec::new();
    if let Some(af) = &sheet.auto_filter
        && let Some(r) = crate::model::CellRange::parse(&af.range)
    {
        for c in r.start.col..=r.end.col {
            v.push(Obj::Button {
                col: c,
                row: r.start.row,
            });
        }
    }
    for s in &sheet.shapes {
        if s.kind != ShapeKind::Chart {
            v.push(Obj::Shape(s));
        }
    }
    // 메모: 그리기 차례(VML 도형 차례)대로, 차례가 없으면 모델 차례
    let mut notes: Vec<(usize, &Comment)> = sheet.comments.iter().enumerate().collect();
    notes.sort_by_key(|(i, c)| (c.z_order.is_none(), c.z_order, *i));
    for (_, c) in notes {
        v.push(Obj::Note(c));
    }
    v
}

/// 메모 상자 자리 (VML `x:Anchor` 의 픽셀 오프셋) → 레거시 앵커 (dx 는 (열 픽셀 − 1)의 1/32768, dy 는 (행 픽셀 − 1)의
/// 1/8192 — [`legacy_anchor`] 와 같은 식)
fn note_anchor(a: [u32; 8], ctx: &SheetCtx) -> [u32; 8] {
    let mdw = crate::units::max_digit_width(ctx.wb);
    let sheet = ctx.sheet;
    let dx = |c: u32, px: u32| {
        let eff = crate::units::sheet_col_px(sheet, c, mdw).saturating_sub(1) as f64;
        if eff <= 0.0 {
            0
        } else {
            ((px as f64 * 32768.0 / eff).round() as i64).clamp(0, 0x7FFF) as u32
        }
    };
    let dy = |r: u32, px: u32| {
        let eff = crate::units::sheet_row_px(sheet, r).saturating_sub(1) as f64;
        if eff <= 0.0 {
            0
        } else {
            ((px as f64 * 8192.0 / eff).round() as i64).clamp(0, 0x1FFF) as u32
        }
    };
    [
        a[0],
        dx(a[0], a[1]),
        a[2],
        dy(a[2], a[3]),
        a[4],
        dx(a[4], a[5]),
        a[6],
        dy(a[6], a[7]),
    ]
}

fn count_shapes(s: &Shape) -> u32 {
    1 + s.children.iter().map(count_shapes).sum::<u32>()
}

impl Art {
    pub fn new(wb: &Workbook) -> Art {
        let mut art = Art {
            blips: Vec::new(),
            pib: HashMap::new(),
            dgid: Vec::new(),
            csp: Vec::new(),
            text_para: None,
            note_para: HashMap::new(),
        };
        fn pictures<'a>(s: &'a Shape, out: &mut Vec<&'a str>) {
            if s.kind == ShapeKind::Picture
                && let Some(t) = &s.target
            {
                out.push(t);
            }
            for c in &s.children {
                pictures(c, out);
            }
        }
        let mut next = 1;
        for sheet in &wb.sheets {
            let mut targets = Vec::new();
            for s in &sheet.shapes {
                pictures(s, &mut targets);
            }
            for t in targets {
                if art.pib.contains_key(t) {
                    continue;
                }
                let Some(part) = wb.raw_parts.iter().find(|p| p.name == t) else {
                    continue;
                };
                let fmt = crate::image::info(&part.data).map(|i| i.format);
                let bt = match fmt {
                    Some("png") => 6,
                    Some("jpeg") => 5,
                    Some("bmp") => 7,
                    Some("emf") => 2,
                    Some("wmf") => 3,
                    _ => continue,
                };
                let data = if bt == 7 {
                    // BMP → DIB (BITMAPFILEHEADER 14바이트를 뗀다)
                    part.data.get(14..).unwrap_or_default().to_vec()
                } else {
                    part.data.clone()
                };
                art.blips.push(Blip {
                    bt,
                    uid: md4(&data),
                    data,
                });
                art.pib.insert(t.to_string(), art.blips.len() as u32);
            }
            let objs = sheet_objects(sheet);
            if objs.is_empty() {
                art.dgid.push(None);
                art.csp.push(0);
            } else {
                let n: u32 = objs
                    .iter()
                    .map(|o| match o {
                        Obj::Shape(s) => count_shapes(s),
                        _ => 1,
                    })
                    .sum();
                art.dgid.push(Some(next));
                art.csp.push(n + 1);
                next += 1;
            }
        }
        art
    }

    /// 차트 레코드(0x7EF0)의 차트 번호 바탕: 문서에 그리기(MSODRAWINGGROUP)가 있으면 그 시트의 OBJ 수 — 시트의 k 번째
    /// 차트(1부터)는 바탕 + k. 그리기가 없으면 None (번호 0). 차트 번호 대조 실험: 차트 둘만 → 0, 0; 같은 시트에
    /// 사각형 하나 → 2, 3, 둘 → 3, 4; 다른 시트에만 사각형 (그 시트가 앞이든 뒤든) → 1, 2, 3
    pub fn chart_base(&self, si: usize) -> Option<u16> {
        let any = self.dgid.iter().any(Option::is_some) || !self.blips.is_empty();
        any.then(|| self.csp.get(si).map_or(0, |&c| c.saturating_sub(1)) as u16)
    }

    /// MSODRAWINGGROUP 내용 (그리기가 하나도 없으면 None)
    pub fn drawing_group(&self) -> Option<Vec<u8>> {
        let drawings: Vec<(u32, u32)> = self
            .dgid
            .iter()
            .zip(&self.csp)
            .filter_map(|(d, &c)| d.map(|d| (d, c)))
            .collect();
        if drawings.is_empty() && self.blips.is_empty() {
            return None;
        }
        let spid_max = drawings
            .iter()
            .map(|&(d, c)| d * 0x400 + c)
            .max()
            .unwrap_or(0x400);
        let mut fdgg = Buf::new();
        fdgg.u32(spid_max)
            .u32(drawings.len() as u32 + 1)
            .u32(drawings.iter().map(|d| d.1).sum())
            .u32(drawings.len() as u32);
        for &(d, c) in &drawings {
            fdgg.u32(d).u32(c);
        }
        let mut body = rec(0, 0, 0xF006, &fdgg.0);
        if !self.blips.is_empty() {
            let mut store = Vec::new();
            for b in &self.blips {
                let (inst, ty, meta) = match b.bt {
                    6 => (0x6E0u16, 0xF01Eu16, false),
                    5 => (0x46A, 0xF01D, false),
                    7 => (0x7A8, 0xF01F, false),
                    2 => (0x3D4, 0xF01A, true),
                    _ => (0x216, 0xF01B, true),
                };
                let mut blip = Buf::new();
                blip.bytes(&b.uid);
                if meta {
                    // OfficeArtMetafileHeader: 압축하지 않음 (0xFE)
                    blip.u32(b.data.len() as u32)
                        .bytes(&[0; 16])
                        .u32(0)
                        .u32(0)
                        .u32(b.data.len() as u32)
                        .u8(0xFE)
                        .u8(0xFE);
                } else {
                    blip.u8(0xFF);
                }
                blip.bytes(&b.data);
                let blip = rec(0, inst, ty, &blip.0);
                let mut bse = Buf::new();
                bse.u8(b.bt)
                    .u8(b.bt)
                    .bytes(&b.uid)
                    .u16(0x00FF)
                    .u32(blip.len() as u32)
                    .u32(1)
                    .u32(0)
                    .u8(0)
                    .u8(0)
                    .u8(0)
                    .u8(0)
                    .bytes(&blip);
                store.extend(rec(2, b.bt as u16, 0xF007, &bse.0));
            }
            body.extend(rec(0xF, self.blips.len() as u16, 0xF001, &store));
        }
        let mut opt = Props(Vec::new());
        opt.set(0x01BF, 0x0010_0000);
        opt.set(0x01FF, 0x0008_0000);
        body.extend(opt.record());
        let mut split = Buf::new();
        split
            .u32(0x0800_000D)
            .u32(0x0800_000C)
            .u32(0x0800_0017)
            .u32(0x1000_00F7);
        body.extend(rec(0, 4, 0xF11E, &split.0));
        Some(rec(0xF, 0, 0xF000, &body))
    }

    /// 시트의 MSODRAWING / OBJ / TXO
    pub fn sheet_drawing(&self, out: &mut Out, ctx: &SheetCtx) -> Result<()> {
        let Some(dgid) = self.dgid.get(ctx.si).copied().flatten() else {
            return Ok(());
        };
        let objs = sheet_objects(ctx.sheet);
        // 도형마다 (SpContainer 의 앞부분, OBJ, 글 상자 여부, TXO)
        struct Part {
            sp: Vec<u8>,
            obj: Vec<u8>,
            txo: Option<Vec<u8>>,
        }
        let mut parts: Vec<Part> = Vec::new();
        let mut spid = dgid * 0x400;
        let mut obj_id = 0u16;
        // 그리기 차례의 메모 번호 (1부터) — VML 의 자동 id(`_x0000_s…`)는 한셀이 "메모 N" 으로 바꾼다
        let mut note_no = 0usize;
        for o in &objs {
            match o {
                Obj::Button { col, row } => {
                    spid += 1;
                    obj_id += 1;
                    let mut fsp = Buf::new();
                    fsp.u32(spid).u32(0x0A00);
                    let mut sp = rec(2, 0xC9, 0xF00A, &fsp.0);
                    let mut p = Props(Vec::new());
                    p.set(0x007F, 0x031E_031E);
                    p.set(0x033F, 0x0008_0008);
                    // 숨긴 열의 단추는 숨긴다 (관측: filter2 의 D 열)
                    let hidden = ctx
                        .sheet
                        .cols
                        .iter()
                        .any(|c| c.hidden && (c.min..=c.max).contains(&(col + 1)));
                    if hidden {
                        p.set(0x03BF, 0x0002_0002);
                    }
                    sp.extend(p.record());
                    sp.extend(client_anchor(0, [*col, 0, *row, 0, col + 1, 0, row + 1, 0]));
                    sp.extend(rec(0, 0, 0xF011, &[]));
                    let mut obj = Buf::new();
                    obj.u16(0x15)
                        .u16(0x12)
                        .u16(0x14)
                        .u16(obj_id)
                        .u16(0x0011)
                        .bytes(&[0; 12]);
                    obj.u16(0x0C)
                        .u16(0x14)
                        .bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
                    obj.u16(0x13)
                        .u16(0x10)
                        .bytes(&[0, 0, 0, 0, 0, 0, 1, 3, 0, 0, 0x0A, 0, 0, 0, 0, 0]);
                    obj.u32(0);
                    parts.push(Part {
                        sp,
                        obj: obj.take(),
                        txo: None,
                    });
                }
                Obj::Shape(s) => {
                    self.shape_parts(
                        s,
                        ctx,
                        &mut spid,
                        &mut obj_id,
                        false,
                        &mut |sp, obj, txo| parts.push(Part { sp, obj, txo }),
                    );
                }
                Obj::Note(c) => {
                    spid += 1;
                    obj_id += 1;
                    let mut fsp = Buf::new();
                    fsp.u32(spid).u32(0x0A00);
                    let mut sp = rec(2, 202, 0xF00A, &fsp.0);
                    // 메모 상자 (메모 대조 실험): 채우기 0x0181 (늘), 선 색 0x01C0 (검정이면 없음), 두께 0x01CB (EMU,
                    // 0.75 pt 면 없음), 그림자는 검정·0.70544 mm (25399 EMU) 오프셋, 숨김 0x03BF. 자리는 VML 의
                    // x:Anchor, 모양이 없으면 셀 오른쪽 위 (XML 쓰기와 같은 자리)
                    let b = c
                        .shape
                        .clone()
                        .unwrap_or_else(|| crate::model::CommentShape::implied(c.cell));
                    let hex = |s: &str| u32::from_str_radix(s.trim_start_matches('#'), 16).ok();
                    let mut p = Props(Vec::new());
                    p.set(0x007F, 0x0200_0200);
                    p.set(
                        0x0181,
                        super::writer::colorref_of(hex(&b.fill_color).unwrap_or(0xFF_FFE1)),
                    );
                    p.set(0x01BF, 0x0010_0010);
                    if let Some(rgb) = hex(&b.stroke_color).filter(|&v| v != 0) {
                        p.set(0x01C0, super::writer::colorref_of(rgb));
                    }
                    let weight = (b.stroke_weight * 12700.0).round() as u32;
                    if weight != 9525 {
                        p.set(0x01CB, weight);
                    }
                    p.set(0x01CC, 0x0001_0000);
                    p.set(0x01FF, 0x0008_0008);
                    if b.shadow {
                        p.set(0x0201, 0);
                        p.set(0x0205, 25399);
                        p.set(0x0206, 25399);
                        p.set(0x023F, 0x0002_0002);
                    }
                    // 이름: 한셀이 붙이는 메모 이름 (`CommentShape::name` — 메모 이름 실험: 자동 id `_x0000_sN` 은
                    // `메모 N%1024`, id 없음은 `메모 0`), 모양이 없으면 VML id, 자동 id 나 이름이 없으면 그리기 차례의
                    // "메모 N" (메모 대조 실험: Excel 꼴 id 다섯 개 → 메모 1–5)
                    note_no += 1;
                    let name = c
                        .shape
                        .as_ref()
                        .and_then(|s| s.name.clone())
                        .or_else(|| c.shape_name.clone().filter(|n| !n.starts_with("_x0000_")))
                        .unwrap_or_else(|| format!("메모 {note_no}"));
                    p.string(0x0380, &name);
                    if !c.visible {
                        p.set(0x03BF, 0x0002_0002);
                    }
                    sp.extend(p.record());
                    // 앵커 플래그: 0 셀과 함께 옮기고 크기 바꿈, 2 옮김만, 3 고정
                    let flag = match (b.move_with_cells, b.size_with_cells) {
                        (true, true) => 0,
                        (true, false) => 2,
                        _ => 3,
                    };
                    sp.extend(client_anchor(flag, note_anchor(b.anchor, ctx)));
                    sp.extend(rec(0, 0, 0xF011, &[]));
                    let mut obj = Buf::new();
                    obj.u16(0x15)
                        .u16(0x12)
                        .u16(0x19)
                        .u16(obj_id)
                        .u16(0x4011)
                        .bytes(&[0; 12]);
                    obj.u16(0x0D).u16(0x16).bytes(&[0; 22]);
                    obj.u32(0);
                    // 서식 런: 글꼴이 바뀌는 곳마다 (런이 없는 글은 런 0개 — 메모 대조 실험)
                    let mut runs: Vec<(u16, u16)> = Vec::new();
                    let mut ich = 0usize;
                    for r in &c.text.runs {
                        if r.text.is_empty() {
                            continue;
                        }
                        let f = r.font.as_ref().map_or(0, |f| ctx.font_like(f));
                        if runs.last().is_none_or(|&(_, last)| last != f) {
                            runs.push((ich as u16, f));
                        }
                        ich += units(&r.text);
                    }
                    // 맞춤: 가로 1 왼쪽, 2 가운데, 3 오른쪽, 5 양쪽, 7 균등 / 세로 0 위, 1 가운데, 2 아래, 3 양쪽, 4 균등.
                    // TXO 문단은 그 가로 맞춤의 문단 모양을 가리킨다 (왼쪽은 기본 0 번, 글 상자 문단 모양이 있어도)
                    let h = match b.text_halign.as_deref() {
                        Some("Center") => 2,
                        Some("Right") => 3,
                        Some("Justify") => 5,
                        Some("Distributed") => 7,
                        _ => 1,
                    };
                    let v = match b.text_valign.as_deref() {
                        Some("Center") => 1,
                        Some("Bottom") => 2,
                        Some("Justify") => 3,
                        Some("Distributed") => 4,
                        _ => 0,
                    };
                    let para = note_hangul_align(b.text_halign.as_deref())
                        .and_then(|a| self.note_para.get(&a).copied())
                        .unwrap_or(0);
                    let txo = txo(&c.text.text, (h, v), &runs, para);
                    parts.push(Part {
                        sp,
                        obj: obj.take(),
                        txo: Some(txo),
                    });
                }
            }
        }
        // patriarch
        let mut patriarch = rec(1, 0, 0xF009, &[0; 16]);
        let mut fsp = Buf::new();
        fsp.u32(dgid * 0x400).u32(0x0005);
        patriarch.extend(rec(2, 0, 0xF00A, &fsp.0));
        let patriarch = rec(0xF, 0, 0xF004, &patriarch);
        let textbox = rec(0, 0, 0xF00D, &[]);
        // SpContainer 길이에는 글 상자(다음 MSODRAWING)도 든다; 머리 8바이트씩
        let sp_total: usize = parts
            .iter()
            .map(|p| 8 + p.sp.len() + if p.txo.is_some() { textbox.len() } else { 0 })
            .sum();
        let spgr_len = patriarch.len() + sp_total;
        let mut fdg = Buf::new();
        fdg.u32(self.csp[ctx.si]).u32(spid);
        let fdg = rec(0, 0, 0xF008, &fdg.0);
        let dg_len = fdg.len() + 8 + spgr_len;
        for (k, p) in parts.iter().enumerate() {
            let mut first = Vec::new();
            if k == 0 {
                first.extend_from_slice(&0x000Fu16.to_le_bytes());
                first.extend_from_slice(&0xF002u16.to_le_bytes());
                first.extend_from_slice(&(dg_len as u32).to_le_bytes());
                first.extend(&fdg);
                first.extend_from_slice(&0x000Fu16.to_le_bytes());
                first.extend_from_slice(&0xF003u16.to_le_bytes());
                first.extend_from_slice(&(spgr_len as u32).to_le_bytes());
                first.extend(&patriarch);
            }
            // SpContainer 머리 (글 상자 포함 길이) + 앞부분
            let len = p.sp.len() + if p.txo.is_some() { textbox.len() } else { 0 };
            first.extend_from_slice(&0x000Fu16.to_le_bytes());
            first.extend_from_slice(&0xF004u16.to_le_bytes());
            first.extend_from_slice(&(len as u32).to_le_bytes());
            first.extend(&p.sp);
            out.rec(0x00EC, &first);
            out.rec(0x005D, &p.obj);
            if let Some(t) = &p.txo {
                out.rec(0x00EC, &textbox);
                out.rec(0x01B6, t);
            }
        }
        Ok(())
    }

    /// 도형 하나(그룹이면 자식까지)의 SpContainer 앞부분·OBJ·TXO
    #[allow(clippy::too_many_arguments)]
    fn shape_parts(
        &self,
        s: &Shape,
        ctx: &SheetCtx,
        spid: &mut u32,
        obj_id: &mut u16,
        child: bool,
        emit: &mut dyn FnMut(Vec<u8>, Vec<u8>, Option<Vec<u8>>),
    ) {
        *spid += 1;
        *obj_id += 1;
        let (shape_type, ot) = match s.kind {
            ShapeKind::Picture => (75u16, 0x08u16),
            ShapeKind::Group => (0, 0x00),
            // 연결선: straightConnector1 은 32 (도형 대조 실험), 꺾인·굽은 연결선은 [MS-ODRAW] MSOSPT 번호
            ShapeKind::Connector => (
                match s.geometry.as_deref() {
                    Some("straightConnector1") => 32,
                    Some("bentConnector2") => 33,
                    Some("bentConnector3") => 34,
                    Some("bentConnector4") => 35,
                    Some("bentConnector5") => 36,
                    Some("curvedConnector2") => 37,
                    Some("curvedConnector3") => 38,
                    Some("curvedConnector4") => 39,
                    Some("curvedConnector5") => 40,
                    _ => 20,
                },
                0x01,
            ),
            _ => (
                match s.geometry.as_deref() {
                    Some("rect") => 1,
                    Some("roundRect") => 2,
                    Some("ellipse") => 3,
                    Some("diamond") => 4,
                    Some("triangle") => 5,
                    Some("rtTriangle") => 6,
                    Some("parallelogram") => 7,
                    Some("trapezoid") => 8,
                    Some("hexagon") => 9,
                    Some("octagon") => 10,
                    Some("plus") => 11,
                    Some("star5") => 12,
                    Some("rightArrow") => 13,
                    Some("line") => 20,
                    // 사용자 정의 모양: 꼭짓점을 쓰지 않으므로 모양 없음 (8절)
                    Some("custom") | None => 0,
                    Some(_) => 1,
                },
                0x1E,
            ),
        };
        // FSP 플래그: 0x0A00 (앵커·모양 종류 있음), 자식 0x0002, 그룹 0x0001, 연결선 0x0100 (도형 대조 실험 0xB00),
        // 뒤집기 0x0040·0x0080
        let mut flags = 0x0A00u32;
        if child {
            flags |= 0x0002;
        }
        if s.kind == ShapeKind::Group {
            flags |= 0x0001;
        }
        if s.kind == ShapeKind::Connector {
            flags |= 0x0100;
        }
        if let Some(x) = &s.xfrm {
            if x.flip_h {
                flags |= 0x0040;
            }
            if x.flip_v {
                flags |= 0x0080;
            }
        }
        let mut fsp = Buf::new();
        fsp.u32(*spid).u32(flags);
        let mut sp = Vec::new();
        if s.kind == ShapeKind::Group {
            sp.extend(rec(1, 0, 0xF009, &[0; 16]));
        }
        sp.extend(rec(2, shape_type, 0xF00A, &fsp.0));
        let mut p = Props(Vec::new());
        let has_text = shape_has_text(s);
        let raw = s.raw.as_deref().unwrap_or("");
        // 보호 속성 ([`lock_bits`]). 글 상자 여백·줄 바꿈·세로 맞춤(0x0081–0x0087)은 한셀이 쓰지 않는다
        // (관측: 말뭉치 한셀 레거시·XML→레거시의 도형 207개 모두, 도형 대조 실험의 글 넘침·줄임표 도형)
        p.set(0x007F, lock_bits(s));
        if s.kind == ShapeKind::Picture
            && let Some(pib) = s.target.as_ref().and_then(|t| self.pib.get(t))
        {
            p.set(0x4104, *pib);
        }
        // 빈 사용자 정의 경로(`a:custGeom` 의 `a:pathLst/`): 좌표계 오른쪽·아래 = 도형 크기(EMU), 경로 종류 4,
        // 빈 0x0155, 글 상자 사각형 0x0157 (IMsoArray 1/4/16 + l·t·r·b) — 관측: 서식 파일의 AutoShape 33개
        if s.geometry.is_none() && raw.contains("<a:custGeom>") && raw.contains("<a:pathLst/>") {
            let (cx, cy) = s.xfrm.as_ref().map_or((0, 0), |x| (x.cx, x.cy));
            p.set(0x0142, cx as u32);
            p.set(0x0143, cy as u32);
            p.set(0x0144, 4);
            p.complex(0x0155, Vec::new());
            let side = |n: &str| -> i32 {
                let k = format!(" {n}=\"");
                raw.find("<a:rect ")
                    .and_then(|i| {
                        let r = &raw[i..];
                        let j = r.find(&k)? + k.len();
                        r[j..].split('"').next()?.parse().ok()
                    })
                    .unwrap_or(0)
            };
            let mut b = Buf::new();
            b.u16(1).u16(4).u16(16);
            for n in ["l", "t", "r", "b"] {
                b.i32(side(n));
            }
            p.complex(0x0157, b.take());
        }
        if s.kind == ShapeKind::Picture {
            // 그림: 채우기·선 없음 (한셀: 0x01BF = 0x00100000, 0x01FF = 0x00080000, 0x033F = 0x00100010)
            if !matches!(s.fill, Some(Paint::Solid(_))) {
                p.set(0x01BF, 0x0010_0000);
            }
            if s.line
                .as_ref()
                .is_none_or(|l| !matches!(l.fill, Some(Paint::Solid(_))))
            {
                p.set(0x01FF, 0x0008_0000);
            }
        }
        // 모양 잠금 0x033F: 그림은 fPreferRelativeResize(0x10), 모양 종류 잠금(noChangeShapeType)은 0x08
        let mut sbool = if s.kind == ShapeKind::Picture {
            0x10
        } else {
            0
        };
        if s.locks.iter().any(|l| l == "noChangeShapeType") {
            sbool |= 0x08;
        }
        if sbool != 0 {
            p.set(0x033F, sbool | (sbool << 16));
        }
        if s.kind != ShapeKind::Group && s.kind != ShapeKind::Picture {
            match &s.fill {
                Some(Paint::Solid(c)) => {
                    // 흰색은 기본 채우기 색이라 쓰지 않는다 (관측)
                    if let Some(rgb) = draw_rgb(c).filter(|&v| v != 0xFF_FFFF) {
                        p.set(0x0181, super::writer::colorref_of(rgb));
                    }
                    p.set(0x01BF, 0x0010_0010);
                }
                Some(Paint::None) => p.set(0x01BF, 0x0010_0000),
                // 연결선은 채우기가 없다 (도형 대조 실험)
                None if s.kind == ShapeKind::Connector => p.set(0x01BF, 0x0010_0000),
                _ => {}
            }
            line_props(&mut p, s.line.as_ref());
            if let Some(sh) = &s.shadow {
                shadow_props(&mut p, sh);
            }
        }
        if !s.name.is_empty() {
            p.string(0x0380, &s.name);
        }
        if let Some(d) = &s.descr {
            p.string(0x0381, d);
        }
        if s.hidden {
            p.set(0x03BF, 0x0002_0002);
        }
        sp.extend(p.record());
        let flag = match s.anchor.edit_as.as_deref() {
            Some("oneCell") => 2,
            Some("absolute") => 3,
            _ => 0,
        };
        let anchor = legacy_anchor(&s.anchor, ctx);
        sp.extend(client_anchor(flag, anchor));
        sp.extend(rec(0, 0, 0xF011, &[]));
        // ftCmo 플래그: 0x0001 잠금, 0x0010 인쇄 — `xdr:clientData fPrintsWithSheet="0"` 이면 인쇄 끔 (관측: 서식
        // 파일의 배경 도형). 잠금은 늘 켠다: 한셀은 `fLocksWithSheet="0"` 을 저장하지 않는다 (도형 대조 실험)
        let cmo = 0x0001u16
            | if s.prints_with_sheet.unwrap_or(true) {
                0x0010
            } else {
                0
            };
        let mut obj = Buf::new();
        obj.u16(0x15)
            .u16(0x12)
            .u16(ot)
            .u16(*obj_id)
            .u16(cmo)
            .bytes(&[0; 12]);
        obj.u32(0);
        let txo = has_text.then(|| {
            // 세로 맞춤은 `a:bodyPr anchor` (도형 대조 실험: anchor="ctr" → 1), 가로는 늘 왼쪽 1
            let v = match s.text.as_ref().and_then(|t| t.anchor.as_deref()) {
                Some("ctr") => 1,
                Some("b") => 2,
                Some("just") => 3,
                Some("dist") => 4,
                _ => 0,
            };
            txo(
                &s.plain_text(),
                (1, v),
                &shape_text_runs(s, ctx),
                self.text_para.unwrap_or(0),
            )
        });
        emit(sp, obj.take(), txo);
        if s.kind == ShapeKind::Group {
            for c in &s.children {
                self.shape_parts(c, ctx, spid, obj_id, true, emit);
            }
        }
    }

    /// NOTE 레코드 (그리기 뒤, 메모마다)
    pub fn notes(&self, out: &mut Out, ctx: &SheetCtx) {
        let objs = sheet_objects(ctx.sheet);
        let mut id = 0u16;
        for o in &objs {
            id += match o {
                Obj::Shape(s) => count_shapes(s) as u16,
                _ => 1,
            };
            if let Obj::Note(c) = o {
                let mut b = Buf::new();
                b.u32(c.cell.row)
                    .u16(c.cell.col as u16)
                    .u16(if c.visible { 0x0002 } else { 0 })
                    .u16(id)
                    .str16(&c.author);
                out.rec_buf(0x001C, &mut b);
            }
        }
    }
}

/// 잠금 이름 → OPT 0x007F 의 비트 (읽기의 `legacy_shape` 와 같은 차례). `noResize`·`noChangeArrowheads` 는
/// 저장되지 않는다
const LOCK_BITS: [&str; 9] = [
    "noGrp",
    "noAdjustHandles",
    "noTextEdit",
    "noEditPoints",
    "noCrop",
    "noSelect",
    "noMove",
    "noChangeAspect",
    "noRot",
];

/// OPT 0x007F (보호): 한셀은 늘 0x0200 을 켜고, 연결선·그림은 글 잠금(0x0004)도 켠다. 위 16비트는 사용 비트
/// (도형 대조 실험: 잠금 없는 도형 0x02000200, 잠금 11개 0x03EF03EF, 연결선 noGrp 0x02050205; 말뭉치의 그림
/// noChangeAspect 0x02840284)
fn lock_bits(s: &Shape) -> u32 {
    let mut v = 0x0200u32;
    for (i, n) in LOCK_BITS.iter().enumerate() {
        if s.locks.iter().any(|l| l == n) {
            v |= 1 << i;
        }
    }
    if matches!(s.kind, ShapeKind::Connector | ShapeKind::Picture) {
        v |= 0x0004;
    }
    v | (v << 16)
}

/// 점선 → MSOLINEDASHING (읽기의 반대)
fn dash_code(d: &str) -> u32 {
    match d {
        "sysDash" => 1,
        "sysDot" => 2,
        "sysDashDot" => 3,
        "sysDashDotDot" => 4,
        "dot" => 5,
        "dash" => 6,
        "lgDash" => 7,
        "dashDot" => 8,
        "lgDashDot" => 9,
        "lgDashDotDot" => 10,
        _ => 0,
    }
}

/// 선 속성 (도형 대조 실험): 색 0x01C0 (검정이면 없음), 두께 0x01CB (9525 면 없음), 이음 — 둥글게(지정 없음 포함)·
/// 비스듬히는 끝 제한 0x01CC = 1.0 과 0x01D6 (2·0), 뾰족하게(miter)는 둘 다 없음 — 겹선 0x01CD, 점선 0x01CE,
/// 끝 0x01D7 (rnd 0, sq 1, flat 은 없음), 화살표 종류 0x01D0·0x01D1 과 너비·길이 0x01D2–0x01D5 (med 는 없음)
fn line_props(p: &mut Props, l: Option<&crate::model::Outline>) {
    match l.and_then(|l| l.join.as_deref()) {
        Some("miter") => {}
        Some("bevel") => {
            p.set(0x01CC, 0x0001_0000);
            p.set(0x01D6, 0);
        }
        _ => {
            p.set(0x01CC, 0x0001_0000);
            p.set(0x01D6, 2);
        }
    }
    let Some(l) = l else {
        return;
    };
    if let Some(w) = l.width.filter(|&w| w != 9525) {
        p.set(0x01CB, w as u32);
    }
    match &l.fill {
        Some(Paint::Solid(c)) => {
            if let Some(rgb) = draw_rgb(c).filter(|&v| v != 0) {
                p.set(0x01C0, super::writer::colorref_of(rgb));
            }
            p.set(0x01FF, 0x0008_0008);
        }
        _ => p.set(0x01FF, 0x0008_0000),
    }
    let compound = match l.compound.as_deref() {
        Some("dbl") => 1,
        Some("thickThin") => 2,
        Some("thinThick") => 3,
        Some("tri") => 4,
        _ => 0,
    };
    if compound != 0 {
        p.set(0x01CD, compound);
    }
    if let Some(d) = l.dash.as_deref().map(dash_code).filter(|&d| d != 0) {
        p.set(0x01CE, d);
    }
    match l.cap.as_deref() {
        Some("rnd") => p.set(0x01D7, 0),
        Some("sq") => p.set(0x01D7, 1),
        _ => {}
    }
    let size = |v: Option<&str>| match v {
        Some("sm") => Some(0),
        Some("lg") => Some(2),
        _ => None,
    };
    for (e, kind, w, len) in [
        (&l.head, 0x01D0, 0x01D2, 0x01D3),
        (&l.tail, 0x01D1, 0x01D4, 0x01D5),
    ] {
        let Some(e) = e else {
            continue;
        };
        let k = match e.kind.as_deref() {
            Some("triangle") => 1,
            Some("stealth") => 2,
            Some("diamond") => 3,
            Some("oval") => 4,
            Some("arrow") => 5,
            _ => continue,
        };
        p.set(kind, k);
        if let Some(v) = size(e.w.as_deref()) {
            p.set(w, v);
        }
        if let Some(v) = size(e.len.as_deref()) {
            p.set(len, v);
        }
    }
}

/// 그림자의 x·y 오프셋 (EMU): 거리 × cos·sin(방향), 0 쪽으로 자른다 (도형 대조 실험: 25400·225° → −17960)
pub(crate) fn shadow_offsets(dist: i64, dir: i64) -> (i32, i32) {
    let rad = (dir as f64 / 60000.0).to_radians();
    (
        (dist as f64 * rad.cos()).trunc() as i32,
        (dist as f64 * rad.sin()).trunc() as i32,
    )
}

/// 바깥 그림자 (도형 대조 실험): 색 0x0201 (기본 회색 808080 이면 없음), 불투명도 0x0204 (16.16, alpha 가
/// 있을 때), x·y 오프셋 0x0205·0x0206 ([`shadow_offsets`]), 기준점 0x0210·0x0211 (`algn` 의 l·r / t·b → ∓0.5,
/// 지정 없음은 b), 켬 0x023F = 0x00020002. 흐림은 저장되지 않는다
fn shadow_props(p: &mut Props, sh: &crate::model::ShapeShadow) {
    if let Some(rgb) = draw_rgb(&sh.color).filter(|&v| v != 0x80_8080) {
        p.set(0x0201, super::writer::colorref_of(rgb));
    }
    if let Some(a) = sh
        .color
        .mods
        .iter()
        .find(|(k, _)| k == "alpha")
        .map(|(_, v)| *v)
        .filter(|&v| v < 100_000)
    {
        p.set(0x0204, (a as f64 / 100_000.0 * 65536.0).round() as u32);
    }
    let (ox, oy) = shadow_offsets(sh.dist, sh.dir);
    p.set(0x0205, ox as u32);
    p.set(0x0206, oy as u32);
    let align = sh.align.as_deref().unwrap_or("b");
    if align != "ctr" {
        let half = |neg: bool| if neg { 0xFFFF_8000u32 } else { 0x8000 };
        if align.ends_with('l') {
            p.set(0x0210, half(true));
        } else if align.ends_with('r') {
            p.set(0x0210, half(false));
        }
        if align.starts_with('t') {
            p.set(0x0211, half(true));
        } else if align.starts_with('b') {
            p.set(0x0211, half(false));
        }
    }
    p.set(0x023F, 0x0002_0002);
}

/// 도형 글의 서식 런 (ich, 레거시 글꼴 번호): 글꼴이 바뀌는 곳마다 (관측: Closing_Costs 의 `0:7 85:20 89:7`).
/// 글꼴을 못 찾은 조각은 0 번
fn shape_text_runs(s: &Shape, ctx: &SheetCtx) -> Vec<(u16, u16)> {
    let mut runs: Vec<(u16, u16)> = Vec::new();
    let Some(t) = &s.text else {
        return runs;
    };
    let mut ich = 0usize;
    for (k, p) in t.paragraphs.iter().enumerate() {
        if k > 0 {
            ich += 1;
        }
        for r in &p.runs {
            if r.text.is_empty() {
                continue;
            }
            let f = super::writer::drawing_run_font(ctx.wb, r).map_or(0, |i| ctx.font(i as u32));
            if runs.last().is_none_or(|&(_, last)| last != f) {
                runs.push((ich as u16, f));
            }
            ich += units(&r.text);
        }
    }
    runs
}

/// TXO (한컴): grbit u16 (0 — 0x0200 은 한셀이 같은 문서를 저장할 때마다 달라진다, 10.8절), u16 1, 가로 맞춤 u8,
/// 세로 맞춤 u8, 서식 런 수 u16, 문단 수 u16, `cch u16, flag u8, UTF-16`, 런마다 `(ich u16, ifnt u16)`, 문단마다
/// 문단 모양 번호 u32 (도형: 표 끝의 글 상자 문단 모양, 메모: 맞춤의 문단 모양 — 관측). 글이 없으면 런 없이 번호만
fn txo(text: &str, align: (u8, u8), runs: &[(u16, u16)], para: u32) -> Vec<u8> {
    let paragraphs = text.split('\n').count().max(1) as u16;
    let mut b = Buf::new();
    b.u16(0)
        .u16(1)
        .u8(align.0)
        .u8(align.1)
        .u16(runs.len() as u16)
        .u16(paragraphs);
    b.u16(units(text) as u16).u8(0).utf16(text);
    for &(ich, f) in runs {
        b.u16(ich).u16(f);
    }
    for _ in 0..paragraphs {
        b.u32(para);
    }
    b.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md4_test_vectors() {
        // RFC 1320
        let hex = |b: [u8; 16]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(md4(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hex(md4(b"abc")), "a448017aaf21d8525fc10ae87aa6729d");
        assert_eq!(
            hex(md4(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "e33b4ddc9c38f2199c3e7b164fcc0536"
        );
    }
}
