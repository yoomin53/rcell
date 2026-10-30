// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 시트 부분의 셀 밖 레코드 쓰기: 보호, 열 정보, 자동 필터, 병합, 하이퍼링크, 메모, 데이터 유효성,
//! 조건부 서식, 보기(WINDOW2·SCL·PANE·SELECTION), 탭 색, 코드 이름.
//!
//! 순서는 한셀의 순서다 (관측, 말뭉치 163개 시트): `… 0x089C → PROTECT·SCENPROTECT·OBJPROTECT·PASSWORD →
//! DEFCOLWIDTH → COLINFO·0xB00E → FILTERMODE·AUTOFILTERINFO·AUTOFILTER… → DIMENSIONS → 셀 → SHEETEXT →
//! MERGECELLS → 차트 → 그리기 → HLINK → NOTE → DVAL·DV → CONDFMT·CF… → WINDOW2 → SCL → PANE → SELECTION →
//! CODENAME → FEATHDR → EOF`.

use super::writer::{Buf, Out, SheetCtx, parse_ranges, range_ref8, units};
use crate::error::Result;
use crate::model::{CellRange, CellRef, Sheet};

/// PROTECT, SCENPROTECT, OBJPROTECT, PASSWORD (시트 보호가 있을 때만)
pub(crate) fn protection(out: &mut Out, sheet: &Sheet) {
    let Some(p) = &sheet.protection else { return };
    if !p.flag("sheet") {
        return;
    }
    out.rec(0x0012, &[1, 0]);
    if p.flag("scenarios") {
        out.rec(0x00DD, &[1, 0]);
    }
    if p.flag("objects") {
        out.rec(0x0063, &[1, 0]);
    }
    if let Some(h) = p
        .password
        .as_deref()
        .and_then(|h| u16::from_str_radix(h, 16).ok())
    {
        out.rec(0x0013, &h.to_le_bytes());
    }
}

/// FEATHDR (isf 2): 보호 항목의 '허용' 비트 (OOXML 과 반대), objects 부터 selectUnlockedCells 까지
pub(crate) fn feat_header(out: &mut Out, sheet: &Sheet) {
    let Some(p) = &sheet.protection else { return };
    if !p.flag("sheet") {
        return;
    }
    let mut allow = 0u32;
    for (i, (k, _)) in crate::model::SHEET_PROTECTION_FLAGS
        .iter()
        .enumerate()
        .skip(1)
    {
        // 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막는다 (6.4절, Excel 과 같은 규칙)
        let blocked = p.flag(k) || (*k == "selectLockedCells" && p.flag("selectUnlockedCells"));
        if !blocked {
            allow |= 1 << (i - 1);
        }
    }
    let mut b = Buf::new();
    b.u16(0x0867)
        .u16(0)
        .bytes(&[0; 8])
        .u16(2)
        .u8(1)
        .u32(0xFFFF_FFFF)
        .u32(allow);
    out.rec_buf(0x0867, &mut b);
}

/// COLINFO (257번째 열 0x0100 까지) 와 0xB00E (그 뒤). 마지막 u16 은 한셀이 늘 0x0046 을 쓴다 (관측)
pub(crate) fn col_infos(out: &mut Out, ctx: &SheetCtx) {
    let mut colinfo = Vec::new();
    let mut wide = Vec::new();
    for c in &ctx.sheet.cols {
        if c.min == 0 || c.max < c.min {
            continue;
        }
        let (first, last) = (c.min - 1, (c.max - 1).min(16_383));
        let w = c
            .width
            .or(ctx.sheet.default_col_width)
            .map_or(0x0924, |w| (w * 256.0).round() as u16);
        let xf = ctx.xf(c.style.unwrap_or(0));
        let mut grbit = 0u16;
        if c.hidden {
            grbit |= 0x0001;
        }
        if c.custom_width {
            grbit |= 0x0002;
        }
        grbit |= ((c.outline_level & 0x07) as u16) << 8;
        if c.collapsed {
            grbit |= 0x1000;
        }
        let rec = |a: u32, z: u32, into: &mut Vec<Vec<u8>>| {
            let mut b = Buf::new();
            b.u16(a as u16)
                .u16(z as u16)
                .u16(w)
                .u16(xf)
                .u16(grbit)
                .u16(0x0046);
            into.push(b.take());
        };
        if first <= 0x0100 {
            rec(first, last.min(0x0100), &mut colinfo);
        }
        if last > 0x0100 {
            rec(first.max(0x0101), last, &mut wide);
        }
    }
    for r in colinfo {
        out.rec(0x007D, &r);
    }
    for r in wide {
        out.rec(0xB00E, &r);
    }
}

/// DOPER 10바이트 (자동 필터 조건): 문자열이면 6번째 바이트에 길이를 두고 내용은 레코드 끝에 (한컴: 플래그 + UTF-16)
fn doper(op: &str, val: &str) -> ([u8; 10], Option<String>) {
    let sign = crate::model::FILTER_OPERATORS
        .iter()
        .position(|x| *x == op)
        .filter(|&i| i > 0)
        .unwrap_or(2) as u8;
    let mut d = [0u8; 10];
    d[1] = sign;
    if val.is_empty() {
        d[0] = 0x0C;
        return (d, None);
    }
    if let Ok(v) = val.parse::<f64>() {
        d[0] = 0x04;
        d[2..10].copy_from_slice(&v.to_le_bytes());
        return (d, None);
    }
    d[0] = 0x06;
    d[6] = units(val).min(255) as u8;
    (d, Some(val.to_string()))
}

/// AUTOFILTER12 로 쓸 조건: (cft, DOPER 들, 값 문자열들)
type Filter12 = (u32, Vec<[u8; 10]>, Vec<Option<String>>);

/// 숫자 → RK ([MS-XLS] 2.5.217): 실수 꼴(아래 34비트가 0), 정수 꼴, 그 100 배 꼴 차례로. 어느 꼴도 아니면 0
fn rk_any(v: f64) -> u32 {
    let bits = v.to_bits();
    if bits & 0x3_FFFF_FFFF == 0 {
        return (bits >> 32) as u32;
    }
    let int = |x: f64| -> Option<u32> {
        (x.fract() == 0.0 && (-(1i64 << 29) as f64..(1i64 << 29) as f64).contains(&x))
            .then_some(((x as i32) << 2) as u32 | 0x02)
    };
    if let Some(r) = int(v) {
        return r;
    }
    let x100 = (v * 100.0).round();
    if (x100 / 100.0) == v
        && let Some(r) = int(x100)
    {
        return r | 0x01;
    }
    0
}

/// 자동 필터: FILTERMODE, AUTOFILTERINFO, 열마다 AUTOFILTER (+ 0x009F 값, AUTOFILTER12 + CONTINUEFRT12)
pub(crate) fn auto_filter(out: &mut Out, ctx: &SheetCtx) {
    let Some(af) = &ctx.sheet.auto_filter else {
        return;
    };
    let Some(range) = CellRange::parse(&af.range) else {
        return;
    };
    if ctx.sheet.filter_mode {
        out.rec(0x009B, &[]);
    }
    let ncols = (range.end.col - range.start.col + 1) as u16;
    out.rec(0x009D, &ncols.to_le_bytes());
    let frt_ref = |rt: u16| {
        let mut b = Buf::new();
        b.u16(rt)
            .u16(1)
            .u16(range.start.row.min(0xFFFF) as u16)
            .u16(range.end.row.min(0xFFFF) as u16)
            .u16(range.start.col as u16)
            .u16(range.end.col as u16);
        b
    };
    for fc in &af.columns {
        let entry = fc.col_id as u16;
        let mut b = Buf::new();
        b.u16(entry);
        let mut strings: Vec<String> = Vec::new();
        let mut af12: Option<Filter12> = None;
        if let Some((top, percent, n)) = fc.top10 {
            let mut g = 0x10u16;
            if top {
                g |= 0x20;
            }
            if percent {
                g |= 0x40;
            }
            g |= ((n as u16) & 0x1FF) << 7;
            // 첫 DOPER: 기준값(filterVal)을 RK 로 (필터 대조 실험)
            let mut d1 = [0u8; 10];
            d1[0] = 0x02;
            d1[1] = if top { 6 } else { 3 };
            d1[2..6].copy_from_slice(&rk_any(fc.filter_val.unwrap_or(0.0)).to_le_bytes());
            b.u16(g).bytes(&d1).bytes(&[0; 10]);
        } else if let Some(t) = &fc.dynamic {
            b.u16(0x0001)
                .bytes(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 0])
                .bytes(&[0; 10]);
            let cft = crate::model::DYNAMIC_FILTER_TYPES
                .iter()
                .position(|x| x == t)
                .unwrap_or(0) as u32;
            // 동적 필터의 조건 (관측): 평균 위 >, 평균 아래 <, 날짜 범위 >= 와 <, 분기·달은 없음. 숫자는 차례로
            // val·maxVal (필터 대조 실험)
            let crit: Vec<u8> = match cft {
                1 => vec![4],
                2 => vec![1],
                8..=23 => vec![6, 1],
                _ => Vec::new(),
            };
            let vals = [fc.dynamic_val, fc.dynamic_max_val];
            let dopers = crit
                .iter()
                .enumerate()
                .map(|(i, &s)| {
                    let mut d = [0u8; 10];
                    d[0] = 4;
                    d[1] = s;
                    d[2..10].copy_from_slice(&vals[i].unwrap_or(0.0).to_le_bytes());
                    d
                })
                .collect::<Vec<_>>();
            let n = dopers.len();
            af12 = Some((cft, dopers, vec![None; n]));
        } else if !fc.values.is_empty() || fc.blank {
            if fc.values.len() <= 2 && !fc.blank {
                // 값이 둘 이하면 = 조건 두 개를 or 로도 남긴다 (6.4절)
                let mut g = 0x0001u16;
                let mut ds = Vec::new();
                for (i, v) in fc.values.iter().enumerate() {
                    let (d, s) = doper("equal", v);
                    if d[0] == 0x06 || d[0] == 0x04 {
                        g |= if i == 0 { 0x04 } else { 0x08 };
                    }
                    ds.push(d);
                    strings.extend(s);
                }
                // 한셀은 숫자 값도 문자열 DOPER 로 쓴다
                let mut ds2 = Vec::new();
                strings.clear();
                for v in &fc.values {
                    let mut d = [0u8; 10];
                    d[0] = 0x06;
                    d[1] = 2;
                    d[6] = units(v).min(255) as u8;
                    ds2.push(d);
                    strings.push(v.clone());
                }
                let _ = ds;
                while ds2.len() < 2 {
                    ds2.push([0; 10]);
                }
                b.u16(g).bytes(&ds2[0]).bytes(&ds2[1]);
            } else {
                b.u16(0x0001)
                    .bytes(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 0])
                    .bytes(&[0; 10]);
                let dopers: Vec<[u8; 10]> = fc
                    .values
                    .iter()
                    .map(|v| {
                        let mut d = [0u8; 10];
                        d[0] = 0x06;
                        d[1] = 2;
                        d[2] = units(v).min(255) as u8;
                        d[3] = 1;
                        d
                    })
                    .collect();
                let texts = fc.values.iter().map(|v| Some(v.clone())).collect();
                af12 = Some((0, dopers, texts));
            }
        } else {
            let mut g = if fc.custom_and { 0u16 } else { 1 };
            let mut ds = Vec::new();
            for (i, (op, v)) in fc.custom.iter().enumerate().take(2) {
                let (d, s) = doper(op, v);
                if fc.custom.len() == 2 && d[0] == 0x06 && op == "equal" && !v.contains(['*', '?'])
                {
                    g |= if i == 0 { 0x04 } else { 0x08 };
                }
                ds.push(d);
                strings.extend(s);
            }
            while ds.len() < 2 {
                ds.push([0; 10]);
            }
            b.u16(g).bytes(&ds[0]).bytes(&ds[1]);
        }
        for s in &strings {
            b.u8(0).utf16(s);
        }
        out.rec_buf(0x009E, &mut b);
        // 값 목록: 0x009F 값마다 (빈 칸은 종류 0x0C)
        if fc.top10.is_none() && fc.dynamic.is_none() {
            for v in &fc.values {
                let mut b = Buf::new();
                b.u16(entry).u8(0).str16(v);
                out.rec_buf(0x009F, &mut b);
            }
            if fc.blank {
                let mut b = Buf::new();
                b.u16(entry).u8(0x0C);
                out.rec_buf(0x009F, &mut b);
            }
        }
        if let Some((cft, dopers, texts)) = af12 {
            let mut b = frt_ref(0x087E);
            b.u16(entry)
                .u32(0)
                .u32(0)
                .u32(cft)
                .u32(dopers.len() as u32)
                .u32(0)
                .u16(8)
                .u32(0)
                .u32(0xFFFF_FFFF)
                .bytes(&[0; 16]);
            out.rec_buf(0x087E, &mut b);
            for (d, t) in dopers.iter().zip(texts) {
                let mut b = frt_ref(0x087F);
                b.bytes(d);
                if let Some(t) = t {
                    b.u8(0).utf16(&t);
                }
                out.rec_buf(0x087F, &mut b);
            }
        }
    }
}

/// 셀 뒤의 레코드들
pub(crate) fn after_cells(
    out: &mut Out,
    ctx: &SheetCtx,
    sst: &mut super::writer::SstRef,
) -> Result<()> {
    let sheet = ctx.sheet;
    let wb = ctx.wb;
    let _ = sst;
    // 탭 색 (SHEETEXT): FRT 머리, cb 20, 팔레트 번호, COLORREF
    if let Some(c) = &sheet.tab_color
        && let Some(rgb) = super::writer::rgb_of(c, wb)
    {
        let mut b = Buf::new();
        b.u16(0x0862)
            .u16(0)
            .bytes(&[0; 8])
            .u32(20)
            .u32(super::writer::nearest_icv(rgb) as u32)
            .u32(super::writer::colorref_of(rgb));
        out.rec_buf(0x0862, &mut b);
    }
    if !sheet.merges.is_empty() {
        // 한 레코드에 1026개까지
        for chunk in sheet.merges.chunks(1026) {
            let mut b = Buf::new();
            b.u16(chunk.len() as u16);
            for m in chunk {
                range_ref8(&mut b, m);
            }
            out.rec_buf(0x00E5, &mut b);
        }
    }
    // 한컴 차트: 레코드마다 끝 표시 0x7EF8 (ff ff ff ff), 다른 그리기 개체보다 먼저 (관측: 서식 파일 차트 5개)
    for rec in &ctx.enc.charts {
        out.rec(0x7EF0, rec);
        out.rec(0x7EF8, &[0xFF; 4]);
    }
    ctx.art.sheet_drawing(out, ctx)?;
    // 피벗 표, 유효성, 조건부 서식, 하이퍼링크, 메모 차례 (배치 실험 place1: 그리기 → SXVIEW… → DVAL → CONDFMT →
    // HLINK → NOTE → WINDOW2)
    for t in &ctx.enc.pivots {
        for (rt, d) in t {
            out.rec(*rt, d);
        }
    }
    data_validations(out, ctx);
    super::cf::conditional_formats(out, ctx);
    for h in &sheet.hyperlinks {
        hlink(out, h, ctx.sheet, ctx.wb.styles.from_legacy);
    }
    ctx.art.notes(out, ctx);
    window(out, ctx);
    let code = sheet
        .code_name
        .clone()
        .unwrap_or_else(|| sheet.name.clone());
    out.rec_buf(0x01BA, Buf::new().str16(&code));
    feat_header(out, sheet);
    // 이 시트의 표: 0xB00A (표 번호), 자동 필터가 있으면 0xB006 + 0xB007 (열 수) — 필터 조건은 쓰지 않는다
    // (표 대조 실험의 한셀 레거시)
    for t in &sheet.tables {
        out.rec(0xB00A, &t.id.to_le_bytes());
        if t.auto_filter.is_some() {
            out.rec(0xB006, &[]);
            out.rec(0xB007, &(t.columns.len() as u16).to_le_bytes());
        }
    }
    out.rec(0x000A, &[]);
    Ok(())
}

/// HLINK: 범위 (행 u32) + CLSID_StdHlink + 하이퍼링크 객체 ([MS-OSHARED] 2.3.7.1).
///
/// 한셀의 꼴 (관측: structure·view_print, 하이퍼링크 실험 문서의 다섯 링크): XML 에서 온 링크는 **화면 설명이 있을 때만**
/// 표시 이름을 쓰고 그 글은 범위 첫 셀의 글이다 (`display` 속성은 쓰지 않는다 — `hlstmfSiteGaveDisplayName |
/// hlstmfHasDisplayName`), 레거시에서 읽은 링크는 읽은 표시 이름 그대로. URL 에 hlstmfIsAbsolute 를 켜지 않으며
/// (플래그 0x15), 화면 설명(HLINKTOOLTIP 0x0800)은 쓰지 않는다
fn hlink(
    out: &mut Out,
    h: &crate::model::Hyperlink,
    sheet: &crate::model::Sheet,
    from_legacy: bool,
) {
    let Some(r) = CellRange::parse(&h.range) else {
        return;
    };
    let mut b = Buf::new();
    range_ref8(&mut b, &r);
    b.bytes(&[
        0xD0, 0xC9, 0xEA, 0x79, 0xF9, 0xBA, 0xCE, 0x11, 0x8C, 0x82, 0x00, 0xAA, 0x00, 0x4B, 0xA9,
        0x0B,
    ]);
    b.u32(2);
    let cell_text = sheet
        .cells
        .iter()
        .find(|c| c.r == r.start)
        .and_then(|c| match &c.value {
            crate::model::CellValue::Text(t) => Some(t.clone()),
            _ => None,
        });
    let display = if from_legacy {
        h.display.clone()
    } else {
        h.tooltip
            .as_ref()
            .and(cell_text.or_else(|| h.display.clone()))
    };
    let mut flags = 0u32;
    if h.target.is_some() {
        flags |= 0x01;
    }
    if display.is_some() {
        flags |= 0x04 | 0x10;
    }
    if h.location.is_some() {
        flags |= 0x08;
    }
    b.u32(flags);
    let hstr = |b: &mut Buf, s: &str| {
        b.u32(units(s) as u32 + 1).utf16(s).u16(0);
    };
    if let Some(d) = &display {
        hstr(&mut b, d);
    }
    if let Some(t) = &h.target {
        match file_target(t) {
            Some((anti, path)) => {
                // 파일 모니커 {00000303-0000-0000-C000-000000000046} (하이퍼링크 대조 실험: `file:///C:/docs/book.xlsx`
                // 를 그 글 그대로 경로로): cAnti u16, ANSI 경로 (NUL 끝), endServer 0xFFFF, 판 0xDEAD, 예약 16 + 4,
                // 유니코드 경로 (cb u32 = 6 + 바이트, 바이트 u32, usKeyValue 3, UTF-16)
                b.bytes(&[
                    0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x46,
                ]);
                let ansi: Vec<u8> = path
                    .chars()
                    .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
                    .collect();
                b.u16(anti).u32(ansi.len() as u32 + 1).bytes(&ansi).u8(0);
                b.u16(0xFFFF).u16(0xDEAD).bytes(&[0; 20]);
                let bytes = units(&path) as u32 * 2;
                b.u32(bytes + 6).u32(bytes).u16(3).utf16(&path);
            }
            None => {
                // URL 모니커 {79EAC9E0-BAF9-11CE-8C82-00AA004BA90B}
                b.bytes(&[
                    0xE0, 0xC9, 0xEA, 0x79, 0xF9, 0xBA, 0xCE, 0x11, 0x8C, 0x82, 0x00, 0xAA, 0x00,
                    0x4B, 0xA9, 0x0B,
                ]);
                b.u32(((units(t) + 1) * 2) as u32).utf16(t).u16(0);
            }
        }
    }
    if let Some(l) = &h.location {
        hstr(&mut b, l);
    }
    out.rec_buf(0x01B8, &mut b);
}

/// 파일 모니커로 쓰는 대상: `file:` 주소, 레거시 읽기의 상대 경로(`.\…`, `..\…`), 드라이브·UNC 경로 → (cAnti, 경로).
/// 한셀은 `file:///…` 주소를 그 글 그대로 파일 모니커의 경로로 쓴다 (하이퍼링크 대조 실험)
pub(crate) fn file_target(t: &str) -> Option<(u16, String)> {
    let mut rest = t;
    let mut anti = 0u16;
    if let Some(r) = rest.strip_prefix(".\\") {
        rest = r;
    } else {
        while let Some(r) = rest.strip_prefix("..\\") {
            rest = r;
            anti += 1;
        }
    }
    let file = rest.starts_with("file:")
        || rest.get(1..3) == Some(":\\")
        || rest.starts_with("\\\\")
        || rest.len() < t.len();
    file.then(|| (anti, rest.to_string()))
}

/// DVAL + DV (6.3절)
fn data_validations(out: &mut Out, ctx: &SheetCtx) {
    let dvs = &ctx.sheet.data_validations;
    if dvs.is_empty() {
        return;
    }
    let mut b = Buf::new();
    b.u16(0)
        .u32(0)
        .u32(0)
        .u32(0xFFFF_FFFF)
        .u32(dvs.len() as u32);
    out.rec_buf(0x01B2, &mut b);
    const KINDS: [&str; 8] = [
        "none",
        "whole",
        "decimal",
        "list",
        "date",
        "time",
        "textLength",
        "custom",
    ];
    const OPS: [&str; 8] = [
        "between",
        "notBetween",
        "equal",
        "notEqual",
        "greaterThan",
        "lessThan",
        "greaterThanOrEqual",
        "lessThanOrEqual",
    ];
    for (i, dv) in dvs.iter().enumerate() {
        let mut flags = KINDS.iter().position(|k| *k == dv.kind).unwrap_or(0) as u32;
        flags |= match dv.error_style.as_deref() {
            Some("warning") => 1,
            Some("information") => 2,
            _ => 0,
        } << 4;
        if dv.allow_blank {
            flags |= 0x0100;
        }
        if dv.hide_drop_down {
            flags |= 0x0200;
        }
        if dv.show_input {
            flags |= 0x0004_0000;
        }
        if dv.show_error {
            flags |= 0x0008_0000;
        }
        let op = dv
            .operator
            .as_deref()
            .and_then(|o| OPS.iter().position(|x| *x == o))
            .unwrap_or(0) as u32;
        flags |= op << 20;
        let mut b = Buf::new();
        b.u32(flags);
        for s in [&dv.prompt_title, &dv.error_title, &dv.prompt, &dv.error] {
            b.str16(s.as_deref().unwrap_or(""));
        }
        let (f1, f2) = ctx.enc_dv(i);
        b.u16(f1.len() as u16).u16(0).bytes(f1);
        b.u16(f2.len() as u16).u16(0).bytes(f2);
        let ranges = parse_ranges(&dv.sqref);
        b.u16(ranges.len() as u16);
        for r in &ranges {
            range_ref8(&mut b, r);
        }
        out.rec_buf(0x01BE, &mut b);
    }
}

fn pane_number(name: Option<&str>) -> u8 {
    match name {
        Some("bottomRight") => 0,
        Some("topRight") => 1,
        Some("bottomLeft") => 2,
        _ => 3,
    }
}

/// WINDOW2, SCL, PANE, SELECTION
fn window(out: &mut Out, ctx: &SheetCtx) {
    let sheet = ctx.sheet;
    let v = &sheet.view;
    let pane = v.pane.as_ref();
    let frozen =
        pane.is_some_and(|p| matches!(p.state.as_deref(), Some("frozen") | Some("frozenSplit")));
    let no_split = pane.is_some_and(|p| p.state.as_deref() == Some("frozen"));
    let mut g = 0x0020u16 | 0x0080;
    if v.show_formulas {
        g |= 0x0001;
    }
    if sheet.show_grid_lines {
        g |= 0x0002;
    }
    if v.show_row_col_headers {
        g |= 0x0004;
    }
    if frozen {
        g |= 0x0008;
    }
    if v.show_zeros {
        g |= 0x0010;
    }
    if v.right_to_left {
        g |= 0x0040;
    }
    if no_split {
        g |= 0x0100;
    }
    if v.tab_selected {
        g |= 0x0200 | 0x0400;
    }
    if v.view.as_deref() == Some("pageBreakPreview") {
        g |= 0x0800;
    }
    if v.grid_color.is_some() {
        g &= !0x0020;
    }
    let tl = v
        .top_left_cell
        .as_deref()
        .and_then(CellRef::parse)
        .unwrap_or_default();
    // 확대율: 레거시에서 읽은 보기는 그대로. XML 에서 온 보기는 한셀이 XML 을 열 때처럼 wScaleNormal =
    // zoomScaleSheetLayoutView (없으면 75), wScaleSLV = 페이지 나누기 미리 보기면 zoomScaleNormal, 아니면 zoomScale
    // (없으면 100) — 관측: 말뭉치 XML 시트 163개, 7가지 꼴 모두 맞는다
    let (normal, slv) = if ctx.wb.styles.from_legacy {
        (
            v.zoom_scale_normal.unwrap_or(0),
            v.zoom_scale_sheet_layout_view.unwrap_or(0),
        )
    } else if v.view.as_deref() == Some("pageBreakPreview") {
        (
            v.zoom_scale_sheet_layout_view.unwrap_or(75),
            v.zoom_scale_normal.unwrap_or(100),
        )
    } else {
        (
            v.zoom_scale_sheet_layout_view.unwrap_or(75),
            v.zoom_scale.unwrap_or(100),
        )
    };
    let mut b = Buf::new();
    b.u16(g)
        .u32(tl.row)
        .u16(tl.col as u16)
        .u32(v.grid_color.unwrap_or(0))
        .u16(0)
        .u16(normal as u16)
        .u16(slv as u16)
        .bytes(&[0, 0, 0, 0x80]);
    out.rec_buf(0x023E, &mut b);
    if let Some(z) = v.zoom_scale.filter(|&z| z != 100) {
        let mut b = Buf::new();
        b.u16(z as u16).u16(100);
        out.rec_buf(0x00A0, &mut b);
    }
    let act = pane.map_or(3, |p| pane_number(p.active_pane.as_deref()));
    if let Some(p) = pane {
        let ptl = p
            .top_left_cell
            .as_deref()
            .and_then(CellRef::parse)
            .unwrap_or_default();
        let mut b = Buf::new();
        b.u16(p.x_split.round() as u16)
            .u32(p.y_split.round() as u32)
            .u32(ptl.row)
            .u16(ptl.col as u16)
            .u16(act as u16);
        out.rec_buf(0x0041, &mut b);
    }
    let ac = v
        .active_cell
        .as_deref()
        .and_then(CellRef::parse)
        .unwrap_or(tl);
    let refs: Vec<CellRange> = v
        .sqref
        .as_deref()
        .map(parse_ranges)
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| vec![CellRange { start: ac, end: ac }]);
    let sel = |pnn: u8| {
        let mut b = Buf::new();
        b.u8(pnn).u32(ac.row).u16(ac.col as u16);
        let iref = refs.iter().position(|r| r.contains(ac)).unwrap_or(0) as u16;
        b.u16(iref).u16(refs.len() as u16);
        for r in &refs {
            range_ref8(&mut b, r);
        }
        b.take()
    };
    // 틀이 나뉘면 활성 창의 것, 그다음 왼쪽 위(3)에 같은 내용 (관측)
    if act != 3 {
        out.rec(0x001D, &sel(act));
    }
    out.rec(0x001D, &sel(3));
}
