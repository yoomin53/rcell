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

/// DXFN (서식이 없으면 '모두 지정 안 함') — 표시 형식(DXFNumUsr), 맞춤(DXFALC, 번호 +1), 보호 블록
fn dxfn(d: Option<&Dxf>) -> Vec<u8> {
    let mut b = Buf::new();
    let Some(d) = d else {
        b.u32(0x801F_FFFF).u16(0x0300);
        return b.take();
    };
    let mut flags: u32 = 0x801F_FFFF;
    let mut flags2: u16 = 0x0300;
    let mut body = Buf::new();
    if let Some(n) = &d.num_fmt {
        flags |= 0x0200_0000;
        flags &= !(1 << 19);
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
    if let Some(a) = &d.alignment {
        flags |= 0x0800_0000;
        let mut alc: u8 = 0;
        let mut b0: u8 = 0;
        if let Some(h) = a.horizontal.as_deref()
            && let Some(i) = super::reader::H_ALIGN.iter().position(|x| *x == h)
        {
            flags &= !0x01;
            alc = ((i + 1) % 8) as u8;
        }
        b0 |= alc & 7;
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
        if a.shrink_to_fit {
            flags &= !0x40;
            b2 |= 0x10;
        }
        if let Some(i) = a.indent {
            flags &= !0x20;
            b2 |= (i.min(15)) as u8;
        }
        body.u8(b0).u8(trot).u8(b2).u8(0).i32(0);
    }
    if let Some(p) = &d.protection {
        flags |= 0x4000_0000;
        let mut v = 0u16;
        if let Some(l) = p.locked {
            flags &= !(1 << 8);
            v |= l as u16;
        }
        if let Some(h) = p.hidden {
            flags &= !(1 << 9);
            v |= (h as u16) << 1;
        }
        body.u16(v);
    }
    b.u32(flags).u16(flags2).bytes(&body.0);
    b.take()
}

/// XFExtNoFRT 의 그라데이션 (CFEX 의 DXFN12 끝, 0x0006 ExtProp)
fn gradient_ext(d: &Dxf, ctx: &SheetCtx) -> Option<Vec<u8>> {
    let g = d.fill.as_ref()?.gradient.as_ref()?;
    let mut v = Buf::new();
    v.u32(g.path as u32)
        .f64(g.degree)
        .f64(g.left)
        .f64(g.right)
        .f64(g.top)
        .f64(g.bottom)
        .u32(g.stops.len() as u32);
    for s in &g.stops {
        let rgb = super::writer::rgb_of(&s.color, ctx.wb).unwrap_or(0);
        v.u16(2)
            .u32(super::writer::colorref_of(rgb))
            .f64(s.position)
            .f64(s.color.tint.unwrap_or(0.0));
    }
    let v = v.take();
    let mut b = Buf::new();
    b.u16(0)
        .u16(0xFFFF)
        .u16(0)
        .u16(1)
        .u16(6)
        .u16((v.len() + 4) as u16)
        .bytes(&v);
    Some(b.take())
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
    // CONDFMT + CF
    for g in &groups {
        let Some(gi) = g.model else { continue };
        let normal: Vec<&(usize, &CfRule)> = g.rules.iter().filter(|(_, r)| !is_cf12(r)).collect();
        if normal.is_empty() {
            continue;
        }
        let mut b = Buf::new();
        condfmt_body(&mut b, normal.len(), g.nid, &g.ranges);
        out.rec_buf(0x01B0, &mut b);
        for (ri, r) in normal {
            let (ct, cp, _, _) = template(r);
            let (f1, f2) = ctx.enc_cf(gi, *ri);
            let dxf = r.dxf_id.and_then(|i| dxfs.get(i as usize));
            let mut b = Buf::new();
            b.u8(ct).u8(cp).u16(f1.len() as u16).u16(f2.len() as u16);
            b.bytes(&dxfn(dxf));
            b.u32(r.dxf_id.unwrap_or(u32::MAX));
            b.bytes(f1).bytes(f2);
            out.rec_buf(0x01B1, &mut b);
        }
    }
    // CF12 만 있는 범위: CONDFMT12 + CF12, 규칙 없는 범위: CONDFMT12 만 (ccf 0)
    for g in &groups {
        if g.rules.iter().any(|(_, r)| !is_cf12(r)) {
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
            let dxf = r.dxf_id.and_then(|i| dxfs.get(i as usize));
            match dxf.and_then(|d| gradient_ext(d, ctx)) {
                Some(ext) => {
                    let d = dxfn(None);
                    b.u8(1).u32((d.len() + 4 + ext.len()) as u32).bytes(&d);
                    b.u32(r.dxf_id.unwrap_or(u32::MAX)).bytes(&ext);
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
