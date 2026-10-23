// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 피벗 쓰기: 모델의 XML 피벗 파트(`pivotCacheDefinition`·`pivotCacheRecords`·`pivotTableDefinition`)를 한셀의
//! 레거시 저장 꼴로 — 전역의 캐시 원본(SXSTREAMID·SXVS·DCONREF/DCONNAME), 캐시 스트림 `/_SX_DB_CUR/000N`, 시트의
//! 피벗 표 레코드(SXVIEW …).
//!
//! 읽기(`super::pivot`)의 반대이고, 레거시가 담지 않는 값과 한셀이 바꿔 쓰는 값은 한셀을 따른다. 근거는 피벗 실험이다:
//! 한셀이 같은 문서를 XML 과 레거시로 저장한 피벗 문서 54개 (pa–pi, 속성을 하나씩 바꾼 v_*·w_*), XTI 실험 px1·px2,
//! 배치 실험 place1 (docs/format/cell-legacy.md 11절).

use super::writer::{Buf, units};
use crate::binary::pivot::{self as bp, DateTime, Item, ab, ai, au};
use crate::model::Workbook;
use crate::xml::Element;

/// 레코드 하나 (종류, 내용)
pub(crate) type Rec = (u16, Vec<u8>);

/// 캐시 하나
pub(crate) struct Cache {
    /// 스트림 번호 (1부터) — 스트림 `/_SX_DB_CUR/000N` (16진수 네 자리)
    pub id: u16,
    /// 전역 레코드: SXSTREAMID, SXVS, DCONREF 또는 DCONNAME
    pub globals: Vec<Rec>,
    /// 캐시 스트림
    pub stream: Vec<u8>,
    /// 원본 시트 — 한셀은 모든 시트의 수식 다음에 이 시트의 XTI 를 만든다 (XTI 실험 px1, 이름이 먼저 쓰면 그 번호 — px2)
    pub source_sheet: Option<String>,
    /// 모델의 캐시 정의 파트 (피벗 표가 관계로 찾는다)
    part: String,
}

/// 문서의 피벗 전체
#[derive(Default)]
pub(crate) struct Pivots {
    pub caches: Vec<Cache>,
    /// 시트(모델 차례)마다 피벗 표마다 레코드 (SXVIEW …)
    pub tables: Vec<Vec<Vec<Rec>>>,
}

/// 레코드 스트림 (캐시 스트림은 확장 길이가 필요 없을 만큼 짧은 레코드만 쓴다)
fn push(out: &mut Vec<u8>, rt: u16, data: &[u8]) {
    let mut o = super::writer::Out {
        buf: std::mem::take(out),
    };
    o.rec(rt, data);
    *out = o.buf;
}

/// 글자 수 u16 + 플래그 0 + UTF-16 (없으면 글자 수 0xFFFF 만)
fn opt_str(b: &mut Buf, s: Option<&str>) {
    match s {
        Some(s) => {
            b.str16(s);
        }
        None => {
            b.u16(0xFFFF);
        }
    }
}

/// 글자 수만 (글은 뒤에 따로)
fn cch(s: Option<&str>) -> u16 {
    s.map_or(0xFFFF, |s| units(s) as u16)
}

/// 플래그 0 + UTF-16 (글자 수는 앞에 따로)
fn body(b: &mut Buf, s: Option<&str>) {
    if let Some(s) = s {
        b.u8(0).utf16(s);
    }
}

// ---------------------------------------------------------------------------
// 캐시
// ---------------------------------------------------------------------------

/// 캐시 항목 레코드: SXSTRING 0x00CD (글), SXNUM 0x00C9 (f64), SXBOOL 0x00CA (u16), SXERR 0x00CB (u16), SXDTR 0x00CE
/// (해 u16, 달 u16, 날·시·분·초 u8), SXNIL 0x00CF (빈 칸)
fn item_rec(out: &mut Vec<u8>, it: &Item) {
    let mut b = Buf::new();
    let rt = match it {
        Item::Missing => 0x00CF,
        Item::Num(v) => {
            b.f64(*v);
            0x00C9
        }
        Item::Bool(v) => {
            b.u16(u16::from(*v));
            0x00CA
        }
        Item::Err(e) => {
            let code = bp::ERRORS.iter().find(|x| x.0 == e).map_or(0x2A, |x| x.1);
            b.u16(u16::from(code));
            0x00CB
        }
        Item::Str(s) => {
            b.str16(s);
            0x00CD
        }
        Item::Date(d) => {
            let mut x = bp::B::default();
            d.bytes(&mut x);
            b.bytes(&x.0);
            0x00CE
        }
    };
    push(out, rt, &b.0);
}

/// 캐시 필드 하나
struct CField<'a> {
    e: &'a Element,
    name: String,
    /// 원본 필드 (레코드에 값이 있다)
    db: bool,
    formula: Option<String>,
    items: Vec<Item>,
}

/// SXFDB 플래그 (공유 항목 속성에서, 피벗 실험 pa·pg·v_group·v_calc·w_big): 0x01 늘, 0x08 부모 있음, 0x10 범위 묶음,
/// 0x20 수, 0x40 정수, 0x80 글(`containsString`·`containsSemiMixedTypes`), 0x100 최소·최대, 0x200 항목 255개 넘음
/// (레코드의 번호가 u32), 0x400 날짜 아닌 값, 0x800 날짜. 계산 필드는 0x8405
fn field_flags(f: &CField) -> u16 {
    if f.formula.is_some() {
        return 0x8405;
    }
    let si = f.e.sml_child("sharedItems");
    let a = |n: &str, d: bool| si.map_or(d, |s| ab(s, n, d));
    let group = f.e.sml_child("fieldGroup");
    let mut x = 0x01u16;
    if group.is_some_and(|g| g.attr("par").is_some()) {
        x |= 0x08;
    }
    if group.is_some_and(|g| g.sml_child("rangePr").is_some()) {
        x |= 0x10;
    }
    if a("containsNumber", false) {
        x |= 0x20;
    }
    if a("containsInteger", false) {
        x |= 0x40;
    }
    if a("containsString", true) {
        x |= 0x80;
    }
    if si.is_some_and(|s| s.attr("minValue").is_some() || s.attr("minDate").is_some()) {
        x |= 0x100;
    }
    if f.items.len() > 255 {
        x |= 0x200;
    }
    if a("containsNonDate", true) {
        x |= 0x400;
    }
    if a("containsDate", false) {
        x |= 0x800;
    }
    x
}

/// 캐시 하나의 쓰기 결과: (캐시 스트림, 전역 레코드, 원본 시트)
type CacheParts = (Vec<u8>, Vec<Rec>, Option<String>);

/// 캐시 정의 XML (+ 레코드 XML) → (캐시 스트림, 전역 레코드 SXSTREAMID·SXVS·DCONREF/DCONNAME, 원본 시트)
fn cache_stream(def_xml: &str, records_xml: Option<&str>, id: u16) -> Result<CacheParts, String> {
    let root = crate::xml::parse(def_xml, "pivotCacheDefinition").map_err(|e| e.to_string())?;
    let rec_root = match records_xml {
        Some(x) => Some(crate::xml::parse(x, "pivotCacheRecords").map_err(|e| e.to_string())?),
        None => None,
    };
    let mut fields: Vec<CField> = root
        .sml_child("cacheFields")
        .map(|c| c.sml_children("cacheField").collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let formula = e.attr("formula").map(str::to_string);
            CField {
                e,
                name: e.attr("name").unwrap_or("").to_string(),
                db: ab(e, "databaseField", true) && formula.is_none(),
                formula,
                items: e
                    .sml_child("sharedItems")
                    .map(|s| s.elements().filter_map(bp::item_of).collect())
                    .unwrap_or_default(),
            }
        })
        .collect();
    // 레코드: 원본 필드마다 공유 항목 번호. 공유 항목이 없는 필드의 값은 항목으로 모아 번호로 바꾼다 (레거시의 SXDBB
    // 는 번호만 담는다)
    let db: Vec<usize> = (0..fields.len()).filter(|&i| fields[i].db).collect();
    let mut rows: Vec<Vec<u32>> = Vec::new();
    if let Some(r) = &rec_root {
        for row in r.sml_children("r") {
            let mut v = Vec::with_capacity(db.len());
            for (k, cell) in row.elements().enumerate() {
                let Some(&fi) = db.get(k) else {
                    break;
                };
                let idx = if cell.name == "x" {
                    au(cell, "v", 0)
                } else {
                    let it = bp::item_of(cell).unwrap_or(Item::Missing);
                    let f = &mut fields[fi];
                    match f.items.iter().position(|x| *x == it) {
                        Some(i) => i as u32,
                        None => {
                            f.items.push(it);
                            (f.items.len() - 1) as u32
                        }
                    }
                };
                v.push(idx);
            }
            v.resize(db.len(), 0);
            rows.push(v);
        }
    }
    let record_count = if rec_root.is_some() {
        rows.len() as u32
    } else {
        au(&root, "recordCount", 0)
    };
    // 원본
    let src = root.sml_child("cacheSource");
    let vs: u16 = match src.and_then(|s| s.attr("type")) {
        Some("external") => 2,
        Some("consolidation") => 4,
        Some("scenario") => 16,
        _ => 1,
    };
    let mut out = Vec::new();
    // SXDB: 레코드 수 u32, 스트림 번호 u16, 플래그 u16 (0x01 saveData, 0x02 invalid, 0x04 refreshOnLoad,
    // 0x08 optimizeMemory, 0x10 backgroundQuery, 0x20 enableRefresh), 블록당 레코드 수 u16 (= 0x1FFF / 원본 필드 수 —
    // 원본 필드 5개 0x0666, 4개 0x07FF, 3개 0x0AAA), 원본 필드 수 u16, 전체 필드 수 u16, 0 u16, 원본 종류 u16, 고친 사람
    // (한셀은 쓰지 않는다: 글자 수 0 + 플래그)
    let flags = u16::from(ab(&root, "saveData", true))
        | (u16::from(ab(&root, "invalid", false)) << 1)
        | (u16::from(ab(&root, "refreshOnLoad", false)) << 2)
        | (u16::from(ab(&root, "optimizeMemory", false)) << 3)
        | (u16::from(ab(&root, "backgroundQuery", false)) << 4)
        | (u16::from(ab(&root, "enableRefresh", true)) << 5);
    let n_db = db.len() as u16;
    let mut b = Buf::new();
    b.u32(record_count)
        .u16(id)
        .u16(flags)
        .u16(0x1FFFu16.checked_div(n_db).unwrap_or(0))
        .u16(n_db)
        .u16(fields.len() as u16)
        .u16(0)
        .u16(vs)
        .u16(0)
        .u8(0);
    push(&mut out, 0x00C6, &b.0);
    // SXDBEX: 새로 고친 날짜 f64 (한셀은 0 — v_cache 의 refreshedDate 45000.5 도), 수식 수 u32 (한셀은 0 — v_calc)
    push(&mut out, 0x0122, &[0; 12]);
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for (fi, f) in fields.iter().enumerate() {
        let group = f.e.sml_child("fieldGroup");
        let group_items: Vec<Item> = group
            .and_then(|g| g.sml_child("groupItems"))
            .map(|gi| gi.elements().filter_map(bp::item_of).collect())
            .unwrap_or_default();
        // 묶음 필드: 기준 필드의 공유 항목 수 (v_group: 수량 7 — 범위 묶음이 자기 자신)
        let base = group.map(|g| ai(g, "base", fi as i32));
        let base_items = base
            .and_then(|b| usize::try_from(b).ok())
            .and_then(|b| fields.get(b))
            .map_or(0, |b| b.items.len());
        // SXFDB: 플래그, 부모, 기준, 고유 항목 수 u16 (묶음이면 묶음 항목 수), 묶음 항목 수 u32, 기준 항목 수 u32 (묶음만),
        // 공유 항목 수 u32, 이름
        let flags = field_flags(f);
        let unique = if group_items.is_empty() {
            f.items.len()
        } else {
            group_items.len()
        };
        let mut b = Buf::new();
        b.u16(flags)
            .u16(group.map_or(0, |g| ai(g, "par", 0).max(0) as u16))
            .u16(base.map_or(0, |v| v.max(0) as u16))
            .u16(unique.min(0xFFFF) as u16)
            .u32(group_items.len() as u32)
            .u32(if group_items.is_empty() {
                0
            } else {
                base_items as u32
            })
            .u32(f.items.len() as u32)
            .str16(&f.name);
        push(&mut out, 0x00C7, &b.0);
        // SXFDBTYPE: SQL 형식
        push(
            &mut out,
            0x01BB,
            &(au(f.e, "sqlType", 0) as u16).to_le_bytes(),
        );
        if let Some(text) = &f.formula {
            // 계산 필드: SXFMLA (cce u16, 이름 수 u16, 토큰열 — 필드 이름은 PtgSxName, XLSB 와 같은 토큰), 이름마다
            // SXNAME (플래그 u16, 캐시 필드 번호 u16, 함수 u16 0x00FF 없음, 쌍 수 u16) — v_calc
            let mut r = bp::PivotNames {
                fields: &names,
                used: Vec::new(),
                unknown: None,
            };
            let enc =
                crate::binary::formula::encode(text, crate::binary::formula::Place::Cell, &mut r)
                    .map_err(|e| format!("계산 필드 {:?} 의 수식 {text:?}: {e}", f.name))?;
            if let Some(u) = r.unknown {
                return Err(format!(
                    "계산 필드 {:?} 의 수식 {text:?}: 알 수 없는 이름 {u:?}",
                    f.name
                ));
            }
            let mut b = Buf::new();
            b.u16(enc.rgce.len() as u16)
                .u16(r.used.len() as u16)
                .bytes(&enc.rgce);
            push(&mut out, 0x00F9, &b.0);
            for ifdb in r.used {
                let mut b = Buf::new();
                b.u16(0).u16(ifdb as u16).u16(0x00FF).u16(0);
                push(&mut out, 0x00F6, &b.0);
            }
            continue;
        }
        // 묶음 항목, [SXRNG + 시작·끝·간격], 공유 항목 차례 (v_group)
        for it in &group_items {
            item_rec(&mut out, it);
        }
        if let Some(rp) = group.and_then(|g| g.sml_child("rangePr")) {
            const BY: [&str; 8] = [
                "range", "seconds", "minutes", "hours", "days", "months", "quarters", "years",
            ];
            let by = bp::index_of(&BY, rp.attr("groupBy"), 0) as u16;
            // SXRNG: 0x01 autoStart, 0x02 autoEnd, 비트 2–4 단위, 0x20 (v_group 의 범위 묶음 0x23 — 뜻 모름)
            let rf = u16::from(ab(rp, "autoStart", true))
                | (u16::from(ab(rp, "autoEnd", true)) << 1)
                | (by << 2)
                | 0x20;
            push(&mut out, 0x00D8, &rf.to_le_bytes());
            let dated = by > 0;
            for (n, d) in [("startNum", "startDate"), ("endNum", "endDate")] {
                let it = if dated {
                    rp.attr(d)
                        .and_then(DateTime::parse)
                        .map_or(Item::Num(0.0), Item::Date)
                } else {
                    Item::Num(rp.attr_f64(n).unwrap_or(0.0))
                };
                item_rec(&mut out, &it);
            }
            item_rec(
                &mut out,
                &Item::Num(rp.attr_f64("groupInterval").unwrap_or(1.0)),
            );
        }
        for it in &f.items {
            item_rec(&mut out, it);
        }
    }
    // SXDBB: 레코드마다 원본 필드의 공유 항목 번호 (u8, 항목이 255개를 넘는 필드는 u32 — w_big)
    let wide: Vec<bool> = db.iter().map(|&i| fields[i].items.len() > 255).collect();
    for row in &rows {
        let mut b = Buf::new();
        for (k, &v) in row.iter().enumerate() {
            if wide[k] {
                b.u32(v);
            } else {
                b.u8(v.min(255) as u8);
            }
        }
        push(&mut out, 0x00C8, &b.0);
    }
    push(&mut out, 0x000A, &[]);
    // 전역: SXSTREAMID, SXVS, DCONREF (첫·끝 행 u32, 첫·끝 열 u16, 파일 글자 수 u16, 플래그, U+0002 + 시트 이름) 또는
    // DCONNAME (이름, 파일 글자 수 u16 0)
    let mut globals: Vec<Rec> = vec![
        (0x00D5, id.to_le_bytes().to_vec()),
        (0x00E3, vs.to_le_bytes().to_vec()),
    ];
    let ws = src.and_then(|s| s.sml_child("worksheetSource"));
    let sheet = ws
        .and_then(|w| w.attr("sheet"))
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let name = ws.and_then(|w| w.attr("name")).filter(|s| !s.is_empty());
    match (name, ws.and_then(|w| w.attr("ref"))) {
        (Some(n), _) => {
            let mut b = Buf::new();
            b.str16(n).u16(0);
            globals.push((0x0052, b.0));
        }
        (None, Some(r)) => {
            let r = crate::model::CellRange::parse(r).ok_or("피벗 캐시 원본 범위를 읽지 못함")?;
            let mut b = Buf::new();
            b.u32(r.start.row)
                .u32(r.end.row)
                .u16(r.start.col as u16)
                .u16(r.end.col as u16);
            match &sheet {
                Some(s) => {
                    b.str16(&format!("\u{2}{s}"));
                }
                None => {
                    b.u16(0);
                }
            }
            globals.push((0x0051, b.0));
        }
        (None, None) => return Err("피벗 캐시 원본이 없음".into()),
    }
    Ok((out, globals, if name.is_some() { None } else { sheet }))
}

// ---------------------------------------------------------------------------
// 피벗 표
// ---------------------------------------------------------------------------

/// 피벗 항목 종류 (XML `t`) → 레거시 itmType ([MS-XLS] 순서: 0 data, 1 default, 2 sum, 3 countA, 4 count, 5 average,
/// 6 max, 7 min, 8 product, 9 stdDev, 10 stdDevP, 11 var, 12 varP, 13 grand, 14 blank) — 읽기의 반대
fn item_type(t: Option<&str>) -> u16 {
    match t {
        Some("count") => 4,
        Some("avg") => 5,
        Some("max") => 6,
        Some("min") => 7,
        Some("product") => 8,
        t => bp::index_of(&bp::ITEM_TYPES, t, 0) as u16,
    }
}

/// 부분합 함수 (XML 속성, SXVD 비트)
const SUBTOTAL_BITS: [(&str, u16); 12] = [
    ("defaultSubtotal", 0x0001),
    ("sumSubtotal", 0x0002),
    ("countASubtotal", 0x0004),
    ("avgSubtotal", 0x0008),
    ("maxSubtotal", 0x0010),
    ("minSubtotal", 0x0020),
    ("productSubtotal", 0x0040),
    ("countSubtotal", 0x0080),
    ("stdDevSubtotal", 0x0100),
    ("stdDevPSubtotal", 0x0200),
    ("varSubtotal", 0x0400),
    ("varPSubtotal", 0x0800),
];

/// SXLI 의 쓰이지 않는 자리 값 (한셀: 줄의 항목 수 뒤 자리는 데이터 줄이면 이 값, 아니면 앞 줄 값 — pd·pg)
const NO_ITEM: u32 = 0xFFFF_7FFF;

/// SXLI: 줄마다 앞 줄과 같은 수 u16 (`r`), 종류 u16, 항목 수 u16, 플래그 u16 (비트 1–8 데이터 필드 `i`, 0x200 부분합·
/// 총합계, 0x800 총합계), 항목 번호 u32 × 자리 수. 앞 줄과 같은 자리는 앞 줄 값을 다시 쓴다
fn lines(items: Option<&Element>, slots: usize) -> Vec<u8> {
    let mut b = Buf::new();
    let mut cur = vec![NO_ITEM; slots];
    for i in items.into_iter().flat_map(|e| e.sml_children("i")) {
        let r = au(i, "r", 0) as usize;
        let t = item_type(i.attr("t"));
        let xs: Vec<u32> = i.sml_children("x").map(|x| au(x, "v", 0)).collect();
        let mac = r + xs.len();
        for (k, &x) in xs.iter().enumerate() {
            if let Some(c) = cur.get_mut(r + k) {
                *c = x;
            }
        }
        if t == 0 {
            for c in cur.iter_mut().skip(mac) {
                *c = NO_ITEM;
            }
        }
        let mut g = (au(i, "i", 0) as u16 & 0xFF) << 1;
        if t != 0 && t != 14 {
            g |= 0x0200;
        }
        if t == 13 {
            g |= 0x0800;
        }
        b.u16(r as u16).u16(t).u16(mac as u16).u16(g);
        for &c in &cur {
            b.u32(c);
        }
    }
    b.0
}

/// 피벗 표 XML → 레코드 (SXVIEW, 0x00B3, 필드마다 SXVD·SXVI…·SXVDEX, SXIVD, SXPI, SXDI…, SXLI ×2, SXEX).
/// `icache` 는 캐시 차례 (0부터), `calc` 는 캐시 필드마다 계산 필드인지
fn table_records(xml: &str, icache: u16, calc: &[bool]) -> Result<Vec<Rec>, String> {
    let root = crate::xml::parse(xml, "pivotTable").map_err(|e| e.to_string())?;
    let mut recs: Vec<Rec> = Vec::new();
    let fields: Vec<&Element> = root
        .sml_child("pivotFields")
        .map(|p| p.sml_children("pivotField").collect())
        .unwrap_or_default();
    let list = |name: &str| -> Vec<i32> {
        root.sml_child(name)
            .map(|e| e.sml_children("field").map(|f| ai(f, "x", 0)).collect())
            .unwrap_or_default()
    };
    let (rows, cols) = (list("rowFields"), list("colFields"));
    let pages: Vec<&Element> = root
        .sml_child("pageFields")
        .map(|e| e.sml_children("pageField").collect())
        .unwrap_or_default();
    let data: Vec<&Element> = root
        .sml_child("dataFields")
        .map(|e| e.sml_children("dataField").collect())
        .unwrap_or_default();
    let count_i = |name: &str| {
        root.sml_child(name)
            .map_or(0, |e| e.sml_children("i").count())
    };
    // SXVIEW: 범위 (행 u32 ×2, 열 u16 ×2), 머리글 행 u32, 데이터 행 u32, 데이터 열 u16 (절대 좌표), 캐시 u16, 0 u16,
    // 값 축 u16 (1 행, 2 열), 값 자리 i16 (없으면 −1), 필드 수 u16 ×5, 행 줄 수 u32, 열 줄 수 u16, 플래그 u16 (0x01
    // rowGrandTotals, 0x02 colGrandTotals, 0x08 useAutoFormatting, 0x10–0x100 apply*Formats, 0x200
    // applyWidthHeightFormats), 자동 서식 u16, 이름·값 머리글
    let loc = root.sml_child("location");
    let r = loc
        .and_then(|l| l.attr("ref"))
        .and_then(crate::model::CellRange::parse)
        .ok_or("피벗 표 위치가 없음")?;
    let lu = |n: &str| loc.map_or(0, |l| au(l, n, 0));
    let mut grbit = u16::from(ab(&root, "rowGrandTotals", true))
        | (u16::from(ab(&root, "colGrandTotals", true)) << 1)
        | (u16::from(ab(&root, "useAutoFormatting", false)) << 3)
        | (u16::from(ab(&root, "applyWidthHeightFormats", false)) << 9);
    for (k, n) in [
        "applyNumberFormats",
        "applyFontFormats",
        "applyAlignmentFormats",
        "applyBorderFormats",
        "applyPatternFormats",
    ]
    .iter()
    .enumerate()
    {
        if ab(&root, n, false) {
            grbit |= 1 << (4 + k);
        }
    }
    let name = root.attr("name").unwrap_or("");
    let caption = root.attr("dataCaption").unwrap_or("Values");
    let mut b = Buf::new();
    b.u32(r.start.row)
        .u32(r.end.row)
        .u16(r.start.col as u16)
        .u16(r.end.col as u16)
        .u32(r.start.row + lu("firstHeaderRow"))
        .u32(r.start.row + lu("firstDataRow"))
        .u16((r.start.col + lu("firstDataCol")) as u16)
        .u16(icache)
        .u16(0)
        .u16(if ab(&root, "dataOnRows", false) { 1 } else { 2 })
        .u16(
            root.attr("dataPosition")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map_or(0xFFFF, |v| v as u16),
        )
        .u16(fields.len() as u16)
        .u16(rows.len() as u16)
        .u16(cols.len() as u16)
        .u16(pages.len() as u16)
        .u16(data.len() as u16)
        .u32(count_i("rowItems") as u32)
        .u16(count_i("colItems") as u16)
        .u16(grbit)
        .u16(au(&root, "autoFormatId", 0) as u16)
        .u16(units(name) as u16)
        .u16(units(caption) as u16)
        .u8(0)
        .utf16(name)
        .u8(0)
        .utf16(caption);
    recs.push((0x00B0, b.0));
    // 0x00B3: 0x01 compact, 0x02 outline, 0x04 outlineData, 0x10 gridDropZones. 0x08 은 compactData 자리지만 한셀은
    // outlineData 를 쓴다 (w_tcd0: compactData 0 → 0x0F, w_tod0: outlineData 0 → 0x03)
    let od = u16::from(ab(&root, "outlineData", false));
    let b3 = u16::from(ab(&root, "compact", true))
        | (u16::from(ab(&root, "outline", false)) << 1)
        | (od << 2)
        | (od << 3)
        | (u16::from(ab(&root, "gridDropZones", false)) << 4);
    recs.push((0x00B3, b3.to_le_bytes().to_vec()));
    for (fi, f) in fields.iter().enumerate() {
        let items: Vec<&Element> = f
            .sml_child("items")
            .map(|i| i.sml_children("item").collect())
            .unwrap_or_default();
        let is_calc = calc.get(fi).copied().unwrap_or(false);
        // SXVD: 축 u16 (1 행, 2 열, 4 페이지, 8 데이터), 부분합 수 u16, 부분합 플래그 u16, 항목 수 u32, 이름.
        // 항목 없는 계산 필드는 부분합이 없다 (v_calc: 한셀 XML 은 defaultSubtotal="0" 을 쓰지 않는다)
        let mut axis = match f.attr("axis") {
            Some("axisRow") => 1u16,
            Some("axisCol") => 2,
            Some("axisPage") => 4,
            _ => 0,
        };
        if ab(f, "dataField", false) {
            axis |= 8;
        }
        let mut sub = SUBTOTAL_BITS
            .iter()
            .filter(|(n, bit)| ab(f, n, *bit == 1))
            .fold(0u16, |a, (_, bit)| a | bit);
        if is_calc && items.is_empty() && f.attr("defaultSubtotal").is_none() {
            sub &= !1;
        }
        let mut b = Buf::new();
        b.u16(axis)
            .u16(sub.count_ones() as u16)
            .u16(sub)
            .u32(items.len() as u32);
        opt_str(&mut b, f.attr("name"));
        recs.push((0x00B1, b.0));
        // SXVI: 종류 u16, 플래그 u16 (0x01 h, 0x02 sd="0", 0x04 f, 0x08 m), 캐시 항목 i32 (데이터 항목만, 아니면 −1),
        // 이름
        for it in &items {
            let t = item_type(it.attr("t"));
            let g = u16::from(ab(it, "h", false))
                | (u16::from(!ab(it, "sd", true)) << 1)
                | (u16::from(ab(it, "f", false)) << 2)
                | (u16::from(ab(it, "m", false)) << 3);
            let mut b = Buf::new();
            b.u16(t)
                .u16(g)
                .i32(if t == 0 { ai(it, "x", 0) } else { -1 });
            opt_str(&mut b, it.attr("n"));
            recs.push((0x00B2, b.0));
        }
        // SXVDEX: 플래그 24비트 (비트 1–4 dragToRow·Col·Page·dragOff, 5 dragToData 금지, 7 serverField, 9 자동 정렬,
        // 10 오름 — 수동도 켠다, 11 autoShow, 12 topAutoShow, 13 계산 필드, 14 insertPageBreak, 15 hideNewItems) +
        // 자동 표시 수 u8 (itemPageCount), 자동 정렬 데이터 필드 u16 (0xFFFF), rankBy i16, 표시 형식 u16, 한셀 플래그 u16
        // (0x01 showAll, 0x02 outline, 0x04 compact, 0x08 subtotalTop, 0x10 insertBlankRow, 0x20
        // includeNewItemsInFilter), 부분합 글자 수 u16, 예약 8, 부분합 글 (pa·v_drag·v_blank·v_sorta·v_sortd·v_calc)
        let sort = f.attr("sortType");
        let mut g = (u32::from(ab(f, "dragToRow", true)) << 1)
            | (u32::from(ab(f, "dragToCol", true)) << 2)
            | (u32::from(ab(f, "dragToPage", true)) << 3)
            | (u32::from(ab(f, "dragOff", true)) << 4)
            | (u32::from(!ab(f, "dragToData", true)) << 5)
            | (u32::from(ab(f, "serverField", false)) << 7)
            | (u32::from(matches!(sort, Some("ascending" | "descending"))) << 9)
            | (u32::from(sort != Some("descending")) << 10)
            | (u32::from(ab(f, "autoShow", false)) << 11)
            | (u32::from(ab(f, "topAutoShow", true)) << 12)
            | (u32::from(is_calc) << 13)
            | (u32::from(ab(f, "insertPageBreak", false)) << 14)
            | (u32::from(ab(f, "hideNewItems", false)) << 15);
        g |= (ai(f, "itemPageCount", 10).clamp(0, 255) as u32) << 24;
        let h = u16::from(ab(f, "showAll", true))
            | (u16::from(ab(f, "outline", true)) << 1)
            | (u16::from(ab(f, "compact", true)) << 2)
            | (u16::from(ab(f, "subtotalTop", true)) << 3)
            | (u16::from(ab(f, "insertBlankRow", false)) << 4)
            | (u16::from(ab(f, "includeNewItemsInFilter", false)) << 5);
        let sub_caption = f.attr("subtotalCaption");
        let mut b = Buf::new();
        b.u32(g)
            .u16(0xFFFF)
            .u16(ai(f, "rankBy", -1) as u16)
            .u16(au(f, "numFmtId", 0) as u16)
            .u16(h)
            .u16(cch(sub_caption))
            .bytes(&[0; 8]);
        body(&mut b, sub_caption);
        recs.push((0x0100, b.0));
    }
    // SXIVD: 행 필드, 열 필드 (−2 값) — 있을 때만
    for v in [&rows, &cols] {
        if !v.is_empty() {
            let mut b = Buf::new();
            for &x in v.iter() {
                b.u16(x as i16 as u16);
            }
            recs.push((0x00B4, b.0));
        }
    }
    // SXPI: 페이지 필드마다 필드 u16, 항목 i32 (−1 모두), 개체 번호 u16 (한셀 1 — 페이지 필드 하나짜리 표본뿐이라
    // 차례 + 1 로 쓴다)
    if !pages.is_empty() {
        let mut b = Buf::new();
        for (k, p) in pages.iter().enumerate() {
            b.u16(au(p, "fld", 0) as u16)
                .i32(p.attr("item").map_or(-1, |_| ai(p, "item", -1)))
                .u16(k as u16 + 1);
        }
        recs.push((0x00B6, b.0));
    }
    // SXDI: 필드 u16, 함수 u16, 값 표시 u16, 기준 필드 i16, 기준 항목 u16, 표시 형식 u16, 이름
    for d in &data {
        let mut b = Buf::new();
        b.u16(au(d, "fld", 0) as u16)
            .u16(bp::index_of(&bp::SUBTOTALS, d.attr("subtotal"), 0) as u16)
            .u16(bp::index_of(&bp::SHOW_DATA_AS, d.attr("showDataAs"), 0) as u16)
            .u16(ai(d, "baseField", 0) as i16 as u16)
            .u16(au(d, "baseItem", 0) as u16)
            .u16(au(d, "numFmtId", 0) as u16);
        opt_str(&mut b, d.attr("name"));
        recs.push((0x00C5, b.0));
    }
    // SXLI: 행, 열
    recs.push((0x00B5, lines(root.sml_child("rowItems"), rows.len())));
    recs.push((0x00B5, lines(root.sml_child("colItems"), cols.len())));
    // SXEX: 서식 수 u16 (서식은 쓰지 않는다), 오류·빈 칸·태그 글자 수 u16, 선택 수 u16 (0), 페이지 필드 행·열 수 u16,
    // 플래그 u32 (0x01 pageOverThenDown, 비트 1–8 pageWrap, 0x200 늘, 0x10000 enableWizard, 0x20000 enableDrill,
    // 0x40000 enableFieldProperties, 0x80000 preserveFormatting, 0x100000 mergeItem, 0x200000 showError, 0x400000
    // showMissing, 0x800000 subtotalHiddenItems), 페이지·표·빈 자리 스타일 글자 수 u16, 글
    let error = root.attr("errorCaption");
    let missing = root.attr("missingCaption");
    let tag = root.attr("tag");
    let page = root.attr("pageStyle");
    let table_style = root.attr("pivotTableStyle").or_else(|| {
        root.sml_child("pivotTableStyleInfo")
            .and_then(|s| s.attr("name"))
    });
    let vacated = root.attr("vacatedStyle");
    let flags = u32::from(ab(&root, "pageOverThenDown", false))
        | ((au(&root, "pageWrap", 0) & 0xFF) << 1)
        | 0x200
        | (u32::from(ab(&root, "enableWizard", true)) << 16)
        | (u32::from(ab(&root, "enableDrill", true)) << 17)
        | (u32::from(ab(&root, "enableFieldProperties", true)) << 18)
        | (u32::from(ab(&root, "preserveFormatting", true)) << 19)
        | (u32::from(ab(&root, "mergeItem", false)) << 20)
        | (u32::from(ab(&root, "showError", false)) << 21)
        | (u32::from(ab(&root, "showMissing", true)) << 22)
        | (u32::from(ab(&root, "subtotalHiddenItems", false)) << 23);
    let mut b = Buf::new();
    b.u16(0)
        .u16(cch(error))
        .u16(cch(missing))
        .u16(cch(tag))
        .u16(0)
        .u16(lu("rowPageCount") as u16)
        .u16(lu("colPageCount") as u16)
        .u32(flags)
        .u16(cch(page))
        .u16(cch(table_style))
        .u16(cch(vacated));
    for s in [error, missing, tag, page, table_style, vacated] {
        body(&mut b, s);
    }
    recs.push((0x00F1, b.0));
    Ok(recs)
}

// ---------------------------------------------------------------------------
// 문서
// ---------------------------------------------------------------------------

/// 모델의 피벗 파트 (통합 문서의 `pivotCacheDefinition` 관계, 시트의 `pivotTable` 관계)를 레거시 레코드로 바꾼다. 캐시는
/// 통합 문서 관계 차례로 스트림 번호 1부터. 바꾸지 못한 캐시는 경고와 함께 빼고, 그 캐시를 쓰는 피벗 표도 뺀다
pub(crate) fn build(wb: &Workbook, warnings: &mut Vec<String>) -> Pivots {
    let mut out = Pivots::default();
    let mut calc: Vec<Vec<bool>> = Vec::new();
    for l in wb
        .part_links
        .iter()
        .filter(|l| l.rel_type.ends_with("/pivotCacheDefinition"))
    {
        let Some(def_xml) = bp::raw_text(&wb.raw_parts, &l.part) else {
            continue;
        };
        let rec_xml = bp::raw_rels(&wb.raw_parts, &l.part)
            .into_iter()
            .find(|r| !r.external && r.rel_type.ends_with("/pivotCacheRecords"))
            .and_then(|r| bp::raw_text(&wb.raw_parts, &r.target));
        let id = out.caches.len() as u16 + 1;
        match cache_stream(&def_xml, rec_xml.as_deref(), id) {
            Ok((stream, globals, source_sheet)) => {
                calc.push(
                    crate::xml::parse(&def_xml, "pivotCacheDefinition")
                        .ok()
                        .and_then(|r| {
                            r.sml_child("cacheFields").map(|c| {
                                c.sml_children("cacheField")
                                    .map(|f| f.attr("formula").is_some())
                                    .collect()
                            })
                        })
                        .unwrap_or_default(),
                );
                out.caches.push(Cache {
                    id,
                    globals,
                    stream,
                    source_sheet,
                    part: l.part.clone(),
                });
            }
            Err(e) => warnings.push(format!("피벗 캐시 {}: {e} — 쓰지 않음", l.part)),
        }
    }
    for sh in &wb.sheets {
        let mut list = Vec::new();
        for l in sh
            .part_links
            .iter()
            .filter(|l| l.rel_type.ends_with("/pivotTable"))
        {
            let Some(xml) = bp::raw_text(&wb.raw_parts, &l.part) else {
                continue;
            };
            let cache = bp::raw_rels(&wb.raw_parts, &l.part)
                .into_iter()
                .find(|r| !r.external && r.rel_type.ends_with("/pivotCacheDefinition"))
                .and_then(|r| out.caches.iter().position(|c| c.part == r.target));
            let Some(ci) = cache else {
                warnings.push(format!("피벗 표 {}: 캐시를 쓰지 못해 뺌", l.part));
                continue;
            };
            match table_records(&xml, ci as u16, &calc[ci]) {
                Ok(recs) => list.push(recs),
                Err(e) => warnings.push(format!("피벗 표 {}: {e} — 쓰지 않음", l.part)),
            }
        }
        out.tables.push(list);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 피벗 실험 pa 의 SXLI (행 필드 하나, 총합계) 와 pd 의 두 자리 줄 (데이터 줄 뒤 자리 0xFFFF7FFF, 총합계는 앞 줄 값)
    #[test]
    fn line_slots_follow_hancell() {
        let x = crate::xml::parse(
            r#"<rowItems xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><i><x/></i><i r="1"><x/></i><i r="1"><x v="1"/></i><i><x v="1"/></i><i t="grand"><x/></i></rowItems>"#,
            "t",
        )
        .unwrap();
        let b = lines(Some(&x), 2);
        let words: Vec<String> = b
            .chunks(16)
            .map(|c| c.iter().map(|v| format!("{v:02x}")).collect::<String>())
            .collect();
        assert_eq!(
            words,
            [
                "000000000100000000000000ff7fffff",
                "01000000020000000000000000000000",
                "01000000020000000000000001000000",
                "000000000100000001000000ff7fffff",
                "00000d000100000a00000000ff7fffff",
            ]
        );
    }
}
