// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 조건부 서식 쓰기: CONDFMT/CF, CONDFMT12/CF12, CFEX (6.3절) — 읽기의 반대.
//!
//! 한셀의 배치 (관측, cf_dv 대조): 범위마다 규칙을 우선순위 순서로 정렬해 CF 로 쓸 수 있는 규칙(값 비교·수식·틀)은
//! CONDFMT + CF, 색 눈금·데이터 막대·아이콘 집합은 CF12 로 쓴다. 먼저 CF 가 있는 범위마다 CONDFMT + CF 들, 그다음
//! CF12 만 있는 범위마다 CONDFMT12 + CF12, 마지막으로 CONDFMT 범위마다 규칙을 우선순위 순서로 CFEX (CF 규칙) 또는
//! CFEX(fIsCF12) + CF12 를 쓴다. nID 는 모델의 범위 순서(1부터), `(nID << 1) | 1`.

use super::writer::{Buf, Out, SheetCtx, parse_ranges, range_ref8};
use crate::model::{CellRange, CellRef, CfRule, Cfvo, Color, Dxf};

fn is_cf12(r: &CfRule) -> bool {
    matches!(r.kind.as_str(), "colorScale" | "dataBar" | "iconSet")
}

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

/// 규칙 → (ct, cp, 틀 번호, 매개 변수 16바이트)
fn template(r: &CfRule) -> (u8, u8, u8, [u8; 16]) {
    let mut p = [0u8; 16];
    let op = r
        .operator
        .as_deref()
        .and_then(|o| OPS.iter().position(|x| *x == o))
        .unwrap_or(0) as u8;
    match r.kind.as_str() {
        "cellIs" => (1, op, 0, p),
        "expression" => (2, 0, 1, p),
        "top10" => {
            p[0] = (!r.bottom as u8) | ((r.percent as u8) << 1);
            p[1..3].copy_from_slice(&(r.rank.unwrap_or(10) as u16).to_le_bytes());
            (2, 0, 5, p)
        }
        "uniqueValues" => (2, 0, 7, p),
        "duplicateValues" => (2, 0, 0x1B, p),
        "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
            let t: u16 = match r.operator.as_deref().unwrap_or(&r.kind) {
                "notContains" | "notContainsText" => 1,
                "beginsWith" => 2,
                "endsWith" => 3,
                _ => 0,
            };
            p[0..2].copy_from_slice(&t.to_le_bytes());
            (2, 0, 8, p)
        }
        "containsBlanks" => (2, 0, 9, p),
        "notContainsBlanks" => (2, 0, 0x0A, p),
        "containsErrors" => (2, 0, 0x0B, p),
        "notContainsErrors" => (2, 0, 0x0C, p),
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
            let k = r
                .time_period
                .as_deref()
                .and_then(|t| PERIODS.iter().position(|x| *x == t))
                .unwrap_or(0) as u8;
            (2, 0, 0x0F + k, p)
        }
        "aboveAverage" => {
            let below = r.above_average == Some(false);
            let t = match (below, r.equal_average) {
                (false, false) => 0x19,
                (true, false) => 0x1A,
                (false, true) => 0x1D,
                (true, true) => 0x1E,
            };
            if let Some(sd) = r.std_dev {
                p[0..2].copy_from_slice(&(sd as u16).to_le_bytes());
            }
            (2, 0, t, p)
        }
        "colorScale" => (3, 0, 2, p),
        "dataBar" => (4, 0, 3, p),
        "iconSet" => (6, 0, 4, p),
        _ => (2, 0, 1, p),
    }
}

/// CF·CFEX 의 차등 서식 부분 (6.3절, 10.9절)
struct DxfBytes {
    /// DXFN (플래그 + 표준 블록) + 한컴 확장 + dxfId + [안쪽 선 확장] — CF 레코드에 그대로, CFEX 의 DXFN12 앞부분
    body: Vec<u8>,
    /// CFEX 의 DXFN12 끝에만 붙는 XFExtNoFRT (속성이 없으면 빔)
    xfext: Vec<u8>,
}

/// 한컴 확장의 COLORREF: 자동 0xDFFFFFFF, 모르는 색은 `none`
fn cref(c: &Color, wb: &crate::model::Workbook, none: u32) -> u32 {
    if c.auto || c.indexed == Some(64) {
        return 0xDFFF_FFFF;
    }
    super::writer::rgb_of(c, wb).map_or(none, super::writer::colorref_of)
}

/// DXFALC 뒤 한컴 확장의 가로 맞춤 번호 (관측: 차등 서식 대조 실험 — 왼쪽 3, 가운데 2, 오른쪽 1, 채우기 0,
/// 선택 영역 가운데 7, 분산 4, 일반·양쪽·지정 안 함 1. 뜻은 모른다)
fn align_ext_code(h: Option<&str>) -> u8 {
    match h {
        Some("left") => 3,
        Some("center") => 2,
        Some("fill") => 0,
        Some("centerContinuous") => 7,
        Some("distributed") => 4,
        _ => 1,
    }
}

/// 차등 서식 표시 형식의 내장 번호: 번호가 164 미만이면 그 번호, 아니면 코드가 같은 내장 형식 (관측: 차등 서식
/// 대조 실험 3 — 번호 14 `yyyy-mm-dd` → 14, 번호 200 `#,##0` → 3, 번호 200 `0.0%` → 사용자 형식)
pub(crate) fn dxf_builtin_fmt(n: &crate::model::NumFmt) -> Option<u32> {
    if n.id < 164 {
        return Some(n.id);
    }
    (0..164).find(|&i| crate::numfmt::builtin(i) == Some(n.code.as_str()))
}

fn is_auto(c: &Color) -> bool {
    c.auto || c.indexed == Some(64)
}

fn edge_style(e: Option<&crate::model::BorderEdge>) -> Option<u8> {
    let s = e?.style.as_deref()?;
    let i = super::reader::BORDER_STYLES.iter().position(|x| *x == s)?;
    (i != 0).then_some(i as u8)
}

/// 차등 서식 → DXFN 과 한컴 확장 (관측: craft/dxfprobe·dxfprobe2·cfdv_x, 차등 서식 대조 실험 1·2).
///
/// - 블록 차례 표시 형식·글꼴·맞춤·테두리·무늬·보호, 확장 차례 글꼴·맞춤·테두리·무늬, 그다음 dxfId 와 안쪽 선.
/// - 한셀이 버리는 것: 위·아래 첨자(`x:vertAlign` — 한컴 `hs:superscript` 만 남는다), ECMA 외곽선·그림자·condense·
///   extend, 한컴 상대 크기·장평·자간·위치, 대각선만 있는 테두리, 읽기 순서, 왼쪽·분산 맞춤의 들여쓰기, `locked="0"`.
/// - 글꼴 크기는 정수 pt 로 내린다 (10.5 → 10).
fn dxf_bytes(d: Option<&Dxf>, dxf_id: Option<u32>, wb: &crate::model::Workbook) -> DxfBytes {
    let id = dxf_id.unwrap_or(u32::MAX);
    let mut b = Buf::new();
    let Some(d) = d else {
        b.u32(0x801F_FFFF).u16(0x0300).u32(id);
        return DxfBytes {
            body: b.take(),
            xfext: Vec::new(),
        };
    };
    let mut flags: u32 = 0x801F_FFFF;
    let mut flags2: u16 = 0x0300;
    let mut body = Buf::new();
    let mut ext = Buf::new();
    // XFExtNoFRT 속성 (종류, 값) — 종류 차례로 쓴다
    let mut props: Vec<(u16, Vec<u8>)> = Vec::new();
    let full_color = |c: &Color| -> Option<Vec<u8>> {
        let mut v = Buf::new();
        if c.auto || c.indexed == Some(64) {
            v.u16(0).u16(0).u32(0);
        } else {
            let rgb = super::writer::rgb_of(c, wb)?;
            v.u16(2).u16(0).u32(super::writer::colorref_of(rgb));
        }
        v.bytes(&[0; 8]);
        Some(v.take())
    };
    // 표시 형식: 내장 형식(번호가 164 미만이거나 코드가 내장 코드와 같은 것 — `#,##0` 번호 200 → 3)은
    // DXFNumIFmt (`0, 번호 u8`), 나머지는 DXFNumUsr
    if let Some(n) = &d.num_fmt {
        flags |= 0x0200_0000;
        flags &= !(1 << 19);
        if let Some(id) = dxf_builtin_fmt(n) {
            body.u8(0).u8(id as u8);
        } else {
            flags2 |= 0x0001;
            let code: Vec<u8> = n
                .code
                .chars()
                .map(|c| c as u32)
                .map(|c| c.min(0xFF) as u8)
                .collect();
            let wide = n.code.chars().any(|c| c as u32 > 0xFF);
            let mut s = Buf::new();
            s.u16(n.code.encode_utf16().count() as u16);
            if wide {
                s.u8(1).utf16(&n.code);
            } else {
                s.u8(0).bytes(&code);
            }
            let s = s.take();
            body.u16((s.len() + 2) as u16).bytes(&s);
        }
    }
    // 글꼴: DXFFntD 118바이트 + 한컴 확장 (비트마스크 u32 + 비트 차례의 값 + 꼬리)
    if let Some(f) = &d.font {
        let h = f.hancom.as_ref();
        let color = f.color.as_ref();
        // `x:u` (val 없음 = 한 줄, `none` 도 지정한 것)
        let uls: Option<u8> = match f.underline.as_deref() {
            None => None,
            Some("none") => Some(0),
            Some("double") => Some(2),
            Some("singleAccounting") => Some(0x21),
            Some("doubleAccounting") => Some(0x22),
            Some(_) => Some(1),
        };
        // 밑줄 없음·회계 밑줄은 확장 꼬리에 따로 남는다
        let ul_tail = matches!(uls, Some(0 | 0x21 | 0x22));
        // 스타일 낱말 (비트 0 기울임, 1 굵게, 2 위 첨자, 3 아래 첨자, 4 양각, 5 음각, 7 글꼴 자간, 8 커닝)
        let mut mask = 0u32;
        let mut style = 0u32;
        if f.italic {
            mask |= 0x01;
            style |= 0x01;
        }
        if f.bold {
            mask |= 0x02;
            style |= 0x02;
        }
        if let Some(h) = h {
            for (bit, v) in [
                (2, h.superscript),
                (3, h.subscript),
                (4, h.emboss),
                (5, h.engrave),
                (7, h.use_font_space),
                (8, h.use_kerning),
            ] {
                if let Some(v) = v {
                    mask |= 1 << bit;
                    if v {
                        style |= 1 << bit;
                    }
                }
            }
        }
        let mut vals = Buf::new();
        if mask & 0x1FF != 0 {
            vals.u32(style);
        }
        let size = f.size.map(|s| s.max(0.0).floor() as u32);
        if let Some(s) = size {
            mask |= 1 << 9;
            vals.u32(s * 100);
        }
        if let Some(c) = color {
            mask |= 1 << 10;
            vals.u32(cref(c, wb, 0));
        }
        if let Some(c) = h.and_then(|h| h.shade_color.as_ref()) {
            mask |= 1 << 11;
            vals.u32(cref(c, wb, 0xFFFF_FFFF));
        }
        // 밑줄: `x:u` 만 있으면 (종류, 모양) = 한 줄 (1, 1), 두 줄 (1, 8), 없음 (0, 1), 회계 (0, 0), 색은 글자 색.
        // 한컴 확장이 있으면 그 값 (모양은 `ST_UnderlineShape` 번호 + 1)
        let pick = |v: &[&str], s: Option<&String>| s.and_then(|s| crate::vocab::ordinal(v, s));
        let hs_ul_type = h.and_then(|h| h.underline_type.as_deref());
        let ul_type = hs_ul_type
            .and_then(|t| t.parse::<u8>().ok())
            .or(uls.map(|u| if matches!(u, 1 | 2) { 1 } else { 0 }));
        let ul_shape = pick(
            crate::vocab::LINE_SHAPE,
            h.and_then(|h| h.underline_shape.as_ref()),
        )
        .map(|i| i as u8 + 1)
        .or(uls.map(|u| match u {
            2 => 8,
            0x21 | 0x22 => 0,
            _ => 1,
        }));
        // DXFFntD 의 밑줄: `x:u`, 없으면 한컴 밑줄 종류 1(아래)이 한 줄 밑줄 (종류 3 은 지정 안 함)
        let fnt_uls = uls.or((hs_ul_type == Some("1")).then_some(1));
        let ul_color = h
            .and_then(|h| h.underline_color.as_ref())
            .or(color.filter(|_| uls.is_some()));
        if let Some(t) = ul_type {
            mask |= 1 << 12;
            vals.u8(t);
        }
        if let Some(s) = ul_shape {
            mask |= 1 << 13;
            vals.u8(s);
        }
        if let Some(c) = ul_color {
            mask |= 1 << 14;
            vals.u32(cref(c, wb, 0));
        }
        let outline = pick(
            crate::vocab::OUTLINE_TYPE,
            h.and_then(|h| h.outline_type.as_ref()),
        );
        if let Some(o) = outline {
            mask |= 1 << 15;
            vals.u8(o as u8);
        }
        if let Some(s) = pick(
            crate::vocab::SHADOW_TYPE,
            h.and_then(|h| h.shadow_type.as_ref()),
        ) {
            mask |= 1 << 16;
            vals.u8(s as u8);
        }
        if let Some(c) = h.and_then(|h| h.shadow_color.as_ref()) {
            mask |= 1 << 17;
            vals.u32(cref(c, wb, 0));
        }
        if let Some(x) = h.and_then(|h| h.shadow_offset_x) {
            mask |= 1 << 18;
            vals.u16(x as i16 as u16);
        }
        if let Some(y) = h.and_then(|h| h.shadow_offset_y) {
            mask |= 1 << 19;
            vals.u16(y as i16 as u16);
        }
        // 취소선: `x:strike` 만 있으면 종류 1·모양 1·색 (글자 색, 없으면 0)
        let so_type = h
            .and_then(|h| h.strikeout_type.as_deref())
            .and_then(|t| t.parse::<u8>().ok())
            .or(f.strike.then_some(1));
        let strike = so_type.is_some_and(|t| t != 0);
        if let Some(t) = so_type {
            mask |= 1 << 20;
            vals.u8(t);
        }
        if let Some(m) = pick(
            crate::vocab::DIACSYM_MARK,
            h.and_then(|h| h.diacsym_mark.as_ref()),
        ) {
            mask |= 1 << 21;
            vals.u8(m as u8);
        }
        let so_shape = pick(
            crate::vocab::LINE_SHAPE,
            h.and_then(|h| h.strikeout_shape.as_ref()),
        )
        .map(|i| i as u8 + 1)
        .or(f.strike.then_some(1));
        if let Some(s) = so_shape {
            mask |= 1 << 22;
            vals.u8(s);
        }
        let so_color = match h.and_then(|h| h.strikeout_color.as_ref()) {
            Some(c) => Some(cref(c, wb, 0)),
            None if f.strike => Some(color.map_or(0, |c| cref(c, wb, 0))),
            None => None,
        };
        if let Some(c) = so_color {
            mask |= 1 << 23;
            vals.u32(c);
        }
        let name = f.name.as_deref().filter(|n| !n.is_empty());
        if let Some(n) = name {
            mask |= 1 << 24;
            // 언어 일곱 개마다 같은 이름: `00 00 00 30 00`, cch u16, 0, UTF-16 (관측: Arial·굴림·맑은 고딕·함초롬바탕)
            for _ in 0..7 {
                vals.bytes(&[0, 0, 0, 0x30, 0])
                    .u16(n.encode_utf16().count() as u16)
                    .u8(0)
                    .utf16(n);
            }
        }
        let sss = match h {
            Some(h) if h.superscript == Some(true) => Some(1u16),
            Some(h) if h.subscript == Some(true) => Some(2),
            Some(h) if h.superscript.is_some() || h.subscript.is_some() => Some(0),
            _ => None,
        };
        if mask != 0 || uls.is_some() {
            flags |= 0x0400_0000;
            if name.is_some() {
                flags &= !(1 << 20);
            }
            let mut fnt = Buf::new();
            let mut name_bytes = [0u8; 64];
            if let Some(n) = name {
                let units: Vec<u16> = n.encode_utf16().take(31).collect();
                name_bytes[0] = units.len() as u8;
                for (i, u) in units.iter().enumerate() {
                    name_bytes[2 + 2 * i..4 + 2 * i].copy_from_slice(&u.to_le_bytes());
                }
            }
            let style_set = f.bold || f.italic;
            let mut ts_ninch = 0xBAu32;
            if style_set {
                ts_ninch &= !0x02;
            }
            if strike {
                ts_ninch &= !0x80;
            }
            if outline.is_some() {
                ts_ninch &= !0x08;
            }
            // 외곽선은 실선 종류(번호 1)일 때만 켠다 (차등 서식 대조 실험 3)
            let ts = if f.italic { 0x02 } else { 0 }
                | if outline == Some(1) { 0x08 } else { 0 }
                | if strike { 0x80 } else { 0 };
            fnt.bytes(&name_bytes)
                .u32(size.map_or(u32::MAX, |s| s * 20))
                .u32(ts)
                .u16(if f.bold { 700 } else { 400 })
                .u16(sss.unwrap_or(0))
                .u8(fnt_uls.unwrap_or(0))
                .u8(0)
                .u8(1)
                .u8(0)
                .u32(color.map_or(u32::MAX, |c| {
                    super::writer::icv_of(Some(c), wb, 0x40) as u32
                }))
                .u32(0)
                .u32(ts_ninch)
                .u32(sss.is_none() as u32)
                .u32(fnt_uls.is_none() as u32)
                .u32(!style_set as u32)
                .u32(1)
                .u32(0)
                .u32(0x7FFF_FFFF)
                .u16(1);
            body.bytes(&fnt.0);
            ext.u32(mask).bytes(&vals.0);
            // 꼬리: `x:u` 가 없음·회계 밑줄이면 `1, 종류(0 없음, 1 한 줄 회계, 2 두 줄 회계), 0, 0, 0` + 글자 색
            // 있음 u8 (+ COLORREF), 아니면 `0, 0`
            if ul_tail {
                let kind = match uls {
                    Some(0x21) => 1,
                    Some(0x22) => 2,
                    _ => 0,
                };
                ext.u8(1).u8(kind).bytes(&[0; 3]);
                match color {
                    Some(c) => {
                        ext.u8(1).u32(cref(c, wb, 0));
                    }
                    None => {
                        ext.u8(0);
                    }
                }
            } else {
                ext.u16(0);
            }
            if let Some(c) = color
                && let Some(v) = full_color(c)
            {
                props.push((0x0D, v));
            }
        }
    }
    // 맞춤: DXFALC (번호 +1, iIndent 는 늘 255) + 한컴 확장 (`0x80 | 들여쓰기 0x01` u32, [들여쓰기 × 1134], 번호 u8)
    if let Some(a) = &d.alignment {
        flags |= 0x0800_0000;
        let mut b0: u8 = 0;
        let h = a.horizontal.as_deref();
        if let Some(i) = h.and_then(|h| super::reader::H_ALIGN.iter().position(|x| *x == h)) {
            flags &= !0x01;
            b0 |= ((i + 1) % 8) as u8 & 7;
        }
        if let Some(v) = a.vertical.as_deref()
            && let Some(i) = super::reader::V_ALIGN.iter().position(|x| *x == v)
        {
            flags &= !0x02;
            b0 |= (((i + 1) as u8) & 7) << 4;
        }
        if a.wrap_text {
            flags &= !0x04;
            b0 |= 0x08;
        }
        let mut trot = 0u8;
        if let Some(r) = a.text_rotation {
            flags &= !0x08;
            trot = r.clamp(0, 255) as u8;
        }
        let mut b2 = 0u8;
        // 들여쓰기는 왼쪽·분산·채우기·선택 영역 가운데 맞춤에서 버린다 (차등 서식 대조 실험 1–3)
        let indent = a.indent.filter(|_| {
            !matches!(
                h,
                Some("left" | "distributed" | "fill" | "centerContinuous")
            )
        });
        if let Some(i) = indent {
            flags &= !0x20;
            b2 |= i.min(15) as u8;
        }
        if a.shrink_to_fit {
            flags &= !0x40;
            b2 |= 0x10;
        }
        body.u8(b0).u8(trot).u8(b2).u8(0).u32(255);
        match indent {
            Some(i) => {
                // 가운데 맞춤이면 비트 1, 아니면 비트 0
                let bit = if h == Some("center") { 0x02 } else { 0x01 };
                ext.u32(0x80 | bit).u32(i * 1134);
                props.push((0x0F, (i as u16).to_le_bytes().to_vec()));
            }
            None => {
                ext.u32(0x80);
            }
        }
        ext.u8(align_ext_code(h));
    }
    // 테두리: DXFBdr + 한컴 확장 (왼쪽·오른쪽·위·아래 COLORREF, 0 u16) + dxfId 뒤 안쪽 선 확장
    let mut inner: Option<Vec<u8>> = None;
    if let Some(bd) = &d.border {
        let edges = [&bd.left, &bd.right, &bd.top, &bd.bottom];
        let present = edges.iter().any(|e| edge_style(e.as_ref()).is_some())
            || edge_style(bd.horizontal.as_ref()).is_some()
            || edge_style(bd.vertical.as_ref()).is_some();
        if present {
            flags |= 0x1000_0000;
            let mut dg = [0u32; 4];
            let mut icv = [0x40u32; 4];
            for (k, e) in edges.iter().enumerate() {
                if let Some(s) = edge_style(e.as_ref()) {
                    flags &= !(1 << (10 + k));
                    dg[k] = s as u32;
                    if let Some(c) = e.as_ref().and_then(|e| e.color.as_ref()) {
                        icv[k] = super::writer::icv_of(Some(c), wb, 0x40) as u32;
                    }
                }
            }
            body.u32(dg[0] | dg[1] << 4 | dg[2] << 8 | dg[3] << 12 | icv[0] << 16 | icv[1] << 23)
                .u32(icv[2] | icv[3] << 7 | 0x40 << 14);
            for e in edges {
                let c = e
                    .as_ref()
                    .filter(|e| edge_style(Some(e)).is_some())
                    .and_then(|e| e.color.as_ref());
                ext.u32(c.map_or(0xFFFF_FFFF, |c| cref(c, wb, 0xFFFF_FFFF)));
            }
            ext.u16(0);
            // XFExtNoFRT: 위 7, 아래 8, 왼쪽 9, 오른쪽 10 — 색이 없는 선은 자동(종류 0)으로 쓰고, 자동으로 지정한
            // 선은 쓰지 않는다 (차등 서식 대조 실험 1·3)
            for (ty, e) in [
                (7u16, &bd.top),
                (8, &bd.bottom),
                (9, &bd.left),
                (10, &bd.right),
            ] {
                if edge_style(e.as_ref()).is_none() {
                    continue;
                }
                let v = match e.as_ref().and_then(|e| e.color.as_ref()) {
                    None => full_color(&Color {
                        auto: true,
                        ..Default::default()
                    }),
                    Some(c) if is_auto(c) => None,
                    Some(c) => full_color(c),
                };
                if let Some(v) = v {
                    props.push((ty, v));
                }
            }
            let mut ib = Buf::new();
            for e in [&bd.horizontal, &bd.vertical] {
                match edge_style(e.as_ref()) {
                    Some(s) => {
                        let c = e.as_ref().and_then(|e| e.color.as_ref());
                        ib.u8(1)
                            .u8(s)
                            .u32(c.map_or(0xFFFF_FFFF, |c| cref(c, wb, 0xFFFF_FFFF)));
                    }
                    None => {
                        ib.u8(0);
                    }
                }
            }
            inner = Some(ib.take());
        }
    }
    // 무늬: DXFPat (`fls << 10`, `icvF | icvB << 7` — 지정 안 한 색 0x40 / 0x41) + 한컴 확장 (앞·뒤 COLORREF)
    if let Some(fl) = &d.fill {
        flags |= 0x2000_0000;
        let (fls, fg, bg) = match &fl.gradient {
            Some(g) => {
                flags &= !(1 << 16);
                let mut v = Buf::new();
                v.u32(g.path as u32)
                    .f64(g.degree)
                    .f64(g.left)
                    .f64(g.right)
                    .f64(g.top)
                    .f64(g.bottom)
                    .u32(g.stops.len() as u32);
                for s in &g.stops {
                    let rgb = super::writer::rgb_of(&s.color, wb).unwrap_or(0);
                    v.u16(2)
                        .u32(super::writer::colorref_of(rgb))
                        .f64(s.position)
                        .f64(s.color.tint.unwrap_or(0.0));
                }
                props.push((0x06, v.take()));
                (0x28u32, None, None)
            }
            None => {
                let mut fls = 0u32;
                if let Some(p) = fl.pattern_type.as_deref() {
                    flags &= !(1 << 16);
                    fls = super::reader::PATTERNS
                        .iter()
                        .position(|x| *x == p)
                        .unwrap_or(0) as u32;
                    if p == "solid" {
                        flags2 |= 0x0002;
                    }
                }
                // 자동 색은 지정하지 않은 것으로 쓴다 (차등 서식 대조 실험 3)
                let fg = fl.fg_color.as_ref().filter(|c| !is_auto(c));
                let bg = fl.bg_color.as_ref().filter(|c| !is_auto(c));
                if fg.is_some() {
                    flags &= !(1 << 17);
                }
                if bg.is_some() {
                    flags &= !(1 << 18);
                }
                (fls, fg, bg)
            }
        };
        let icv_f = fg.map_or(0x40, |c| super::writer::icv_of(Some(c), wb, 0x40) as u32);
        let icv_b = bg.map_or(0x41, |c| super::writer::icv_of(Some(c), wb, 0x41) as u32);
        body.u16((fls << 10) as u16)
            .u16((icv_f | icv_b << 7) as u16);
        ext.u32(fg.map_or(0xDFFF_FFFF, |c| cref(c, wb, 0xDFFF_FFFF)))
            .u32(bg.map_or(0xFFFF_FFFF, |c| cref(c, wb, 0xFFFF_FFFF)));
        // XFExtNoFRT 의 앞·뒤 색: 자동도 쓴다 (종류 0)
        if fl.gradient.is_none() {
            for (ty, c) in [(4u16, fl.fg_color.as_ref()), (5, fl.bg_color.as_ref())] {
                if let Some(v) = c.and_then(full_color) {
                    props.push((ty, v));
                }
            }
        }
    }
    // 보호: 잠금은 지정이 없으면 켠 것 (`x:protection` 만 있어도 잠금), 꺼진 잠금·숨김은 '지정 안 함' — 둘 다
    // 꺼지면 블록이 없다 (`locked="0"`, 차등 서식 대조 실험 3)
    if let Some(p) = &d.protection {
        let locked = p.locked.unwrap_or(true);
        let hidden = p.hidden.unwrap_or(false);
        if locked || hidden {
            flags |= 0x4000_0000;
            if locked {
                flags &= !(1 << 8);
            }
            if hidden {
                flags &= !(1 << 9);
            }
            body.u16(locked as u16 | (hidden as u16) << 1);
        }
    }
    b.u32(flags)
        .u16(flags2)
        .bytes(&body.0)
        .bytes(&ext.0)
        .u32(id);
    if let Some(i) = inner {
        b.bytes(&i);
    }
    let mut xfext = Buf::new();
    if !props.is_empty() {
        props.sort_by_key(|(t, _)| *t);
        xfext.u16(0).u16(0xFFFF).u16(0).u16(props.len() as u16);
        for (t, v) in &props {
            // 들여쓰기(0x0F)의 길이 칸은 4 (값 2바이트가 뒤에 붙는다 — 관측)
            let cb = if *t == 0x0F { 4 } else { v.len() + 4 };
            xfext.u16(*t).u16(cb as u16).bytes(v);
        }
    }
    DxfBytes {
        body: b.take(),
        xfext: xfext.take(),
    }
}

fn bounding(ranges: &[CellRange]) -> CellRange {
    let mut it = ranges.iter();
    let first = it.next().copied().unwrap_or(CellRange {
        start: CellRef::default(),
        end: CellRef::default(),
    });
    it.fold(first, |a, r| CellRange {
        start: CellRef {
            row: a.start.row.min(r.start.row),
            col: a.start.col.min(r.start.col),
        },
        end: CellRef {
            row: a.end.row.max(r.end.row),
            col: a.end.col.max(r.end.col),
        },
    })
}

fn condfmt_body(b: &mut Buf, n: usize, nid: u16, ranges: &[CellRange]) {
    b.u16(n as u16).u16((nid << 1) | 1);
    range_ref8(b, &bounding(ranges));
    b.u16(ranges.len() as u16);
    for r in ranges {
        range_ref8(b, r);
    }
}

fn frt_ref(rt: u16, r: &CellRange) -> Buf {
    let mut b = Buf::new();
    b.u16(rt)
        .u16(1)
        .u16(r.start.row.min(0xFFFF) as u16)
        .u16(r.end.row.min(0xFFFF) as u16)
        .u16(r.start.col as u16)
        .u16(r.end.col as u16);
    b
}

/// CFVO: 종류 u8 (1 num, 2 min, 3 max, 4 percent, 5 percentile, 7 formula), 수식 cce u16 + 토큰, 수식이 없고
/// 최소·최대가 아니면 숫자 f64 (x14 조건부 서식 실험: `$H$12` → `07 07 00 44 0b 00 00 00 07 00`)
fn cfvo(b: &mut Buf, c: &Cfvo, tokens: Option<&[u8]>) {
    let ty: u8 = match c.kind.as_str() {
        "num" => 1,
        "min" => 2,
        "max" => 3,
        "percent" => 4,
        "percentile" => 5,
        "formula" => 7,
        _ => 1,
    };
    if let Some(t) = tokens {
        b.u8(ty).u16(t.len() as u16).bytes(t);
        return;
    }
    b.u8(ty).u16(0);
    if !matches!(ty, 2 | 3) {
        b.f64(c.val.as_deref().and_then(|v| v.parse().ok()).unwrap_or(0.0));
    }
}

fn cf_color(b: &mut Buf, c: Option<&Color>, ctx: &SheetCtx) {
    match c {
        Some(c) if c.theme.is_some() && c.rgb.is_none() => {
            // 테마 번호, 0, 0, 0xFF (x14 조건부 서식 실험: 테마 5 → `05 00 00 ff`)
            b.u32(3).u8(c.theme.unwrap_or(0) as u8).bytes(&[0, 0, 0xFF]);
            b.f64(c.tint.unwrap_or(0.0));
        }
        Some(c) => {
            let rgb = super::writer::rgb_of(c, ctx.wb).unwrap_or(0);
            b.u32(2)
                .u8((rgb >> 16) as u8)
                .u8((rgb >> 8) as u8)
                .u8(rgb as u8)
                .u8(0xFF)
                .f64(c.tint.unwrap_or(0.0));
        }
        None => {
            b.u32(0).u32(0).f64(0.0);
        }
    }
}

/// CF12 의 종류별 내용 (CFGradient / CFDatabar / CFMultistate). `key` = (서식 번호, 규칙 번호) — 기준 값 수식을 찾는다
fn cf12_content(r: &CfRule, ctx: &SheetCtx, key: (usize, usize)) -> Vec<u8> {
    let mut b = Buf::new();
    let tok = |k: usize| ctx.enc_cfvo(key.0, key.1, k);
    if let Some(cs) = &r.color_scale {
        b.u8(0)
            .u8(cs.cfvos.len() as u8)
            .u8(cs.colors.len() as u8)
            .u8(0x03);
        let n = cs.cfvos.len().max(1);
        for (i, c) in cs.cfvos.iter().enumerate() {
            cfvo(&mut b, c, tok(i));
            b.f64(i as f64 / (n - 1).max(1) as f64);
        }
        let m = cs.colors.len().max(1);
        for (i, c) in cs.colors.iter().enumerate() {
            b.f64(i as f64 / (m - 1).max(1) as f64);
            cf_color(&mut b, Some(c), ctx);
        }
    } else if let Some(d) = &r.data_bar {
        // 플래그: 0x01 오른쪽에서 왼쪽 (x14 확장의 방향), 0x02 값 숨김
        let mut flags = if d.show_value { 0 } else { 0x02 };
        if d.ext
            .as_ref()
            .is_some_and(|e| e.direction.as_deref() == Some("rightToLeft"))
        {
            flags |= 0x01;
        }
        b.u8(0)
            .u8(flags)
            .u8(d.min_length.unwrap_or(10) as u8)
            .u8(d.max_length.unwrap_or(90) as u8);
        cf_color(&mut b, d.color.as_ref(), ctx);
        for (k, (c, _)) in super::writer::rule_cfvos(r).iter().enumerate() {
            cfvo(&mut b, c, tok(k));
        }
    } else if let Some(i) = &r.icon_set {
        const SETS: [&str; 20] = [
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
        let set = SETS.iter().position(|s| *s == i.set).unwrap_or(3) as u8;
        let mut flags = 0u8;
        if !i.show_value {
            flags |= 0x01;
        }
        if i.reverse {
            flags |= 0x04;
        }
        b.u8(0).u8(i.cfvos.len() as u8).u8(set).u8(flags);
        for (k, c) in i.cfvos.iter().enumerate() {
            cfvo(&mut b, c, tok(k));
            b.u8(c.gte as u8).u32(0);
        }
    }
    b.take()
}

fn cf12_record(out: &mut Out, r: &CfRule, ctx: &SheetCtx, key: (usize, usize)) {
    let (ct, _cp, tmpl, params) = template(r);
    let mut b = Buf::new();
    b.u16(0x087A).u16(0).bytes(&[0; 8]);
    b.u8(ct).u8(0).u16(0).u16(0).u32(0);
    b.u16(0).u16(0);
    let mut flags = 0x01u8;
    if r.stop_if_true {
        flags |= 0x02;
    }
    b.u8(flags)
        .u16(r.priority as u16)
        .u16(tmpl as u16)
        .u8(16)
        .bytes(&params);
    let content = cf12_content(r, ctx, key);
    b.u16(content.len() as u16).bytes(&content);
    out.rec_buf(0x087A, &mut b);
}

/// 시트의 조건부 서식 전부
pub(crate) fn conditional_formats(out: &mut Out, ctx: &SheetCtx) {
    let cfs = &ctx.sheet.conditional_formats;
    if cfs.is_empty() {
        return;
    }
    let dxfs = &ctx.wb.styles.dxfs;
    // 서식 범위 하나 (nID 차례). `model` 은 모델의 조건부 서식 번호 (수식 부호화 표의 번호), 없으면 규칙 없는
    // CONDFMT12 만 쓴다
    struct Group<'a> {
        nid: u16,
        model: Option<usize>,
        ranges: Vec<CellRange>,
        rules: Vec<(usize, &'a CfRule)>,
    }
    let mut groups: Vec<Group> = Vec::new();
    // 한셀은 XML 의 x14 구역을 본 구역 뒤의 서식 범위로 저장한다: x14 확장이 붙은 데이터 막대마다 규칙 없는
    // CONDFMT12 를 하나씩 (본 규칙에 합쳐졌으므로), 그다음 x14 구역에만 있는 규칙 (x14 조건부 서식 실험의 nID
    // 1–4 본 구역, 5–7 데이터 막대 확장, 8– 아이콘 집합). 레거시에서 읽은 모델은 그 빈 범위가 모델에 그대로 있다
    let first_x14 = cfs
        .iter()
        .position(|cf| !cf.rules.is_empty() && cf.rules.iter().all(|r| r.x14))
        .unwrap_or(cfs.len());
    let mut order: Vec<(Option<usize>, &str)> = Vec::new();
    for (k, cf) in cfs.iter().enumerate().take(first_x14) {
        order.push((Some(k), cf.sqref.as_str()));
    }
    if !ctx.wb.styles.from_legacy {
        for cf in cfs.iter().take(first_x14) {
            if cf
                .rules
                .iter()
                .any(|r| r.data_bar.as_ref().is_some_and(|d| d.ext.is_some()))
            {
                order.push((None, cf.sqref.as_str()));
            }
        }
    }
    for (k, cf) in cfs.iter().enumerate().skip(first_x14) {
        order.push((Some(k), cf.sqref.as_str()));
    }
    for (slot, (model, sqref)) in order.into_iter().enumerate() {
        let mut rules: Vec<(usize, &CfRule)> = model
            .map(|k| cfs[k].rules.iter().enumerate().collect())
            .unwrap_or_default();
        rules.sort_by_key(|(_, r)| r.priority);
        groups.push(Group {
            nid: slot as u16 + 1,
            model,
            ranges: parse_ranges(sqref),
            rules,
        });
    }
    // 범위 차례(nID)로: CF 로 쓸 규칙이 있으면 CONDFMT + CF, CF12 만 있으면 CONDFMT12 + CF12, 규칙 없는 범위는
    // CONDFMT12 만 (ccf 0) — cfdv_x: 아이콘 집합만 있는 nID 5 의 CONDFMT12 가 nID 6 의 CONDFMT 보다 먼저
    for g in &groups {
        let normal: Vec<&(usize, &CfRule)> = g.rules.iter().filter(|(_, r)| !is_cf12(r)).collect();
        if let (Some(gi), false) = (g.model, normal.is_empty()) {
            let mut b = Buf::new();
            condfmt_body(&mut b, normal.len(), g.nid, &g.ranges);
            out.rec_buf(0x01B0, &mut b);
            for (ri, r) in normal {
                let (ct, cp, _, _) = template(r);
                let (f1, f2) = ctx.enc_cf(gi, *ri);
                let dxf = r.dxf_id.and_then(|i| dxfs.get(i as usize));
                let mut b = Buf::new();
                b.u8(ct).u8(cp).u16(f1.len() as u16).u16(f2.len() as u16);
                b.bytes(&dxf_bytes(dxf, r.dxf_id, ctx.wb).body);
                b.bytes(f1).bytes(f2);
                out.rec_buf(0x01B1, &mut b);
            }
            continue;
        }
        let bb = bounding(&g.ranges);
        let mut b = frt_ref(0x0879, &bb);
        condfmt_body(&mut b, g.rules.len(), g.nid, &g.ranges);
        out.rec_buf(0x0879, &mut b);
        let Some(gi) = g.model else { continue };
        for (ri, r) in &g.rules {
            cf12_record(out, r, ctx, (gi, *ri));
        }
    }
    // CFEX
    for g in &groups {
        let Some(gi) = g.model else { continue };
        if !g.rules.iter().any(|(_, r)| !is_cf12(r)) {
            continue;
        }
        let bb = bounding(&g.ranges);
        let mut icf = 0u16;
        for (ri, r) in &g.rules {
            let mut b = frt_ref(0x087B, &bb);
            if is_cf12(r) {
                b.u32(1).u16(g.nid);
                out.rec_buf(0x087B, &mut b);
                cf12_record(out, r, ctx, (gi, *ri));
                continue;
            }
            let (_, cp, tmpl, params) = template(r);
            b.u32(0)
                .u16(g.nid)
                .u16(icf)
                .u8(cp)
                .u8(tmpl)
                .u16(r.priority as u16);
            let mut flags = 0x01u8;
            if r.stop_if_true {
                flags |= 0x02;
            }
            b.u8(flags);
            // 차등 서식이 있는 규칙은 DXFN12 (cbdxf u32 + CF 와 같은 DXFN·확장·dxfId·안쪽 선 + XFExtNoFRT)
            match r.dxf_id {
                Some(i) => {
                    let x = dxf_bytes(dxfs.get(i as usize), Some(i), ctx.wb);
                    b.u8(1)
                        .u32((x.body.len() + x.xfext.len()) as u32)
                        .bytes(&x.body)
                        .bytes(&x.xfext);
                }
                None => {
                    b.u8(0);
                }
            }
            b.u8(16).bytes(&params);
            out.rec_buf(0x087B, &mut b);
            icf += 1;
        }
    }
}
