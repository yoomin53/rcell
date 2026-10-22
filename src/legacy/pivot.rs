// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 피벗: 캐시 스트림(`/_SX_DB_CUR/000N`)과 `/Workbook` 의 피벗 레코드(SXVIEW …)를 XML 피벗 파트로 읽는다.
//!
//! 레코드는 [MS-XLS] 의 BIFF8 피벗 레코드를 따르되, 한셀은 행 번호·항목 수·캐시 항목 번호를 u32 로 넓히고 글은 플래그
//! 바이트(0) + UTF-16 으로 쓴다. 레코드를 바이너리(XLSB) 피벗 레코드로 옮긴 뒤 `binary::pivot` 으로 XML 을 쓴다 — 한셀이
//! 레거시를 읽어 쓴 XML(`X.legacy.as.cell`)이 `.xlsb` 를 읽어 쓴 XML 과 같은 꼴이라서다. 레거시가 담지 않는 값은 한셀의
//! 레거시 읽기가 쓰는 값으로 채운다. 피벗 실험 (docs/format/cell-legacy.md 11절).

use crate::binary::pivot::{B, W};
use crate::model::{SheetKind, Workbook};

/// 레코드 내용 읽기
struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Rd { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
    }
    /// 글자 수가 앞에 따로 있는 글: 플래그 바이트 + UTF-16 (0xFFFF 는 없음)
    fn st(&mut self, cch: u16) -> Option<Option<String>> {
        if cch == 0xFFFF {
            return Some(None);
        }
        let _flag = self.u8()?;
        Some(Some(crate::text::utf16le(self.take(cch as usize * 2)?)))
    }
    /// 글자 수 u16 + 플래그 바이트 + UTF-16
    fn xst(&mut self) -> Option<String> {
        let cch = self.u16()?;
        self.st(cch).map(Option::unwrap_or_default)
    }
}

/// 전역의 캐시 원본: SXSTREAMID(0x00D5) 스트림 번호, SXVS(0x00E3) 원본 종류, DCONREF(0x0051)/DCONNAME(0x0052)
#[derive(Default)]
struct Source {
    stream: u16,
    /// 범위 (첫 행, 끝 행, 첫 열, 끝 열)와 시트 이름
    range: Option<(u32, u32, u32, u32, Option<String>)>,
    /// 이름 원본
    name: Option<String>,
}

/// DCONREF: 첫 행 u32, 끝 행 u32, 첫 열 u16, 끝 열 u16, 파일 글자 수 u16, 플래그 + 파일 — 이 통합 문서의 시트는 첫 글자
/// 0x0002 + 시트 이름 (가상 경로)
fn dconref(d: &[u8]) -> Option<(u32, u32, u32, u32, Option<String>)> {
    let mut r = Rd::new(d);
    let (r1, r2, c1, c2) = (r.u32()?, r.u32()?, r.u16()?, r.u16()?);
    let cch = r.u16()?;
    let file = if cch == 0 { None } else { r.st(cch)? };
    let sheet = file.and_then(|f| f.strip_prefix('\u{2}').map(str::to_string));
    Some((r1, r2, u32::from(c1), u32::from(c2), sheet))
}

/// 캐시 항목 (SXSTRING 0x00CD, SXNUM 0x00C9, SXBOOL 0x00CA, SXERR 0x00CB, SXINT 0x00CC, SXDTR 0x00CE, SXNIL 0x00CF)
#[derive(Clone)]
enum Item {
    Missing,
    Num(f64),
    Bool(bool),
    Err(u8),
    Str(String),
    Date([u8; 8]),
}

fn item(rtype: u16, d: &[u8]) -> Option<Item> {
    let mut r = Rd::new(d);
    Some(match rtype {
        0x00CD => Item::Str(r.xst()?),
        0x00C9 => Item::Num(r.f64()?),
        0x00CA => Item::Bool(r.u16()? != 0),
        0x00CB => Item::Err(r.u16()? as u8),
        0x00CC => Item::Num(f64::from(r.u16()? as i16)),
        0x00CE => Item::Date(r.take(8)?.try_into().ok()?),
        0x00CF => Item::Missing,
        _ => return None,
    })
}

/// XLSB 항목 레코드 (BrtPCDIMissing … BrtPCDIDatetime)
fn item_bin(w: &mut W, it: &Item) {
    match it {
        Item::Missing => w.e(20),
        Item::Num(v) => w.rec(21, &v.to_le_bytes()),
        Item::Bool(b) => w.rec(22, &[u8::from(*b)]),
        Item::Err(e) => w.rec(23, &[*e]),
        Item::Str(s) => {
            let mut b = B::default();
            b.wstr(s);
            w.rec(24, &b.0);
        }
        Item::Date(d) => w.rec(25, d),
    }
}

/// 캐시 필드 (SXFDB 0x00C7 과 뒤따르는 항목·묶음·수식)
#[derive(Default)]
struct Field {
    /// 0x01 늘, 0x08 부모 있음, 0x10 범위 묶음, 0x20 수, 0x40 정수, 0x80 글(섞임), 0x100 최소·최대, 0x200 레코드의 항목
    /// 번호가 u32 (아니면 u8), 0x400 날짜 아닌 값, 0x800 날짜, 0x8000 계산 필드 — SXFDB 와 XML 공유 항목 대조
    flags: u16,
    parent: u16,
    base: u16,
    /// 묶음 항목 수
    n_group: u16,
    name: String,
    group: Vec<Item>,
    /// SXRNG (0x00D8) 플래그 — 범위 묶음
    range: Option<u16>,
    /// 범위의 시작·끝·간격
    range_vals: Vec<Item>,
    items: Vec<Item>,
    /// 계산 필드 수식 (SXFMLA 0x00F9 의 토큰열)
    formula: Option<Vec<u8>>,
    /// 수식이 쓰는 캐시 필드 (SXNAME 0x00F6 의 ifdb)
    pnames: Vec<u16>,
}

impl Field {
    fn calc(&self) -> bool {
        self.flags & 0x8000 != 0
    }
    fn grouped(&self) -> bool {
        self.flags & 0x18 != 0 || !self.group.is_empty()
    }
    /// XLSB BrtBeginPCDFAtbl 플래그
    fn atbl(&self) -> u16 {
        let f = self.flags;
        let mut x = 0;
        if f & 0x80 != 0 {
            x |= 0x01 | 0x08;
        }
        if f & 0x400 != 0 {
            x |= 0x02;
        }
        if f & 0x800 != 0 {
            x |= 0x04;
        }
        if self.items.iter().any(|i| matches!(i, Item::Missing)) {
            x |= 0x10;
        }
        if f & 0x20 != 0 && f & 0x80 != 0 {
            x |= 0x20;
        }
        if f & 0x20 != 0 {
            x |= 0x40;
        }
        if f & 0x40 != 0 {
            x |= 0x80;
        }
        if f & 0x100 != 0 {
            x |= 0x100;
        }
        x
    }
}

fn num_or_serial(it: Option<&Item>) -> (f64, bool) {
    match it {
        Some(Item::Num(v)) => (*v, false),
        Some(Item::Date(d)) => (crate::binary::pivot::date_serial(d).unwrap_or(0.0), true),
        _ => (0.0, false),
    }
}

/// 캐시 스트림 → XLSB 캐시 정의·레코드 레코드 파트
fn cache_bin(data: &[u8], src: &Source) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut count = 0u32;
    let mut flags = 0u16;
    let mut refreshed = 0.0;
    let mut fields: Vec<Field> = Vec::new();
    let mut rows: Vec<&[u8]> = Vec::new();
    for rec in super::biff::records(data) {
        let d = rec.data;
        let mut r = Rd::new(d);
        match rec.rtype {
            // SXDB: 레코드 수 u32, 스트림 번호 u16, 플래그 u16 (0x01 saveData, 0x02 invalid, 0x04 refreshOnLoad,
            // 0x08 optimizeMemory, 0x10 backgroundQuery, 0x20 enableRefresh), …
            0x00C6 => {
                count = r.u32()?;
                let _stream = r.u16()?;
                flags = r.u16()?;
            }
            // SXDBEX: 새로 고친 날짜 f64, 수식 수 u32
            0x0122 => refreshed = r.f64().unwrap_or(0.0),
            // SXFDB: 플래그 u16, 부모 u16, 기준 u16, 보이는 항목 수 u16, 묶음 항목 수 u16, u16, u32, 공유 항목 수 u32, 이름
            0x00C7 => {
                let flags = r.u16()?;
                let parent = r.u16()?;
                let base = r.u16()?;
                let _unique = r.u16()?;
                let n_group = r.u16()?;
                r.take(6)?;
                let _items = r.u32()?;
                let name = r.xst()?;
                fields.push(Field {
                    flags,
                    parent,
                    base,
                    n_group,
                    name,
                    ..Default::default()
                });
            }
            0x00D8 => {
                if let Some(f) = fields.last_mut() {
                    f.range = r.u16();
                }
            }
            // SXFMLA: cce u16, 이름 수 u16, 토큰열 (PtgSxName 0x18 0x1D + u32 — XLSB 와 같다)
            0x00F9 => {
                let cce = r.u16()? as usize;
                let _names = r.u16()?;
                if let Some(f) = fields.last_mut() {
                    f.formula = Some(r.take(cce)?.to_vec());
                }
            }
            // SXNAME: 플래그 u16, ifdb u16, 함수 u16, 쌍 수 u16
            0x00F6 => {
                let _f = r.u16()?;
                let ifdb = r.u16()?;
                if let Some(f) = fields.last_mut() {
                    f.pnames.push(ifdb);
                }
            }
            0x00C8 => rows.push(d),
            0x000A => break,
            t => {
                if let (Some(it), Some(f)) = (item(t, d), fields.last_mut()) {
                    if f.group.len() < usize::from(f.n_group) {
                        f.group.push(it);
                    } else if f.range.is_some() && f.range_vals.len() < 3 {
                        f.range_vals.push(it);
                    } else {
                        f.items.push(it);
                    }
                }
            }
        }
    }
    let mut w = W::default();
    // BrtBeginPivotCacheDef: 판 3·3·3, 플래그 (XLSB 는 0x10 enableRefresh·0x20 backgroundQuery 로 자리가 바뀐다)
    let xf = (flags & 0x0F) | ((flags & 0x10) << 1) | ((flags & 0x20) >> 1);
    let mut b = B::default();
    b.u8(3)
        .u8(3)
        .u8(3)
        .u8(xf as u8)
        .i32(-1)
        .f64(refreshed)
        .u8(0x02)
        .u32(count)
        .wstr("rId1");
    w.rec(179, &b.0);
    w.rec(185, &[0; 8]);
    let mut b = B::default();
    if let Some(n) = &src.name {
        b.u8(1)
            .u8(u8::from(n.starts_with("_xlnm.")))
            .u8(0)
            .wstr(n.strip_prefix("_xlnm.").unwrap_or(n));
    } else {
        let (r1, r2, c1, c2, sheet) = src.range.clone()?;
        b.u8(0).u8(0).u8(if sheet.is_some() { 0x02 } else { 0 });
        if let Some(s) = &sheet {
            b.wstr(s);
        }
        b.u32(r1).u32(r2).u32(c1).u32(c2);
    }
    w.rec(187, &b.0);
    w.e(188);
    w.e(186);
    w.rec(181, &(fields.len() as u32).to_le_bytes());
    for f in &fields {
        let calc = f.calc();
        let mut b = B::default();
        b.u16(if calc { 0x100 } else { 0x04 })
            .u32(0)
            .u16(0)
            .u32(0)
            .u32(0)
            .u32(0)
            .wstr(&f.name);
        if calc {
            let rgce = f.formula.clone().unwrap_or_default();
            b.u32(rgce.len() as u32);
            b.0.extend_from_slice(&rgce);
            b.u32(0);
        }
        w.rec(183, &b.0);
        if calc {
            w.rec(253, &(f.pnames.len() as u32).to_le_bytes());
            for p in &f.pnames {
                let mut b = B::default();
                b.u32(u32::from(*p)).u8(0xFF).u8(0);
                w.rec(255, &b.0);
                w.e(256);
            }
            w.e(254);
        } else {
            let at = f.atbl();
            let mut b = B::default();
            b.u16(at).u32(f.items.len() as u32);
            if at & 0x100 != 0 {
                b.f64(0.0).f64(0.0);
            }
            w.rec(189, &b.0);
            for it in &f.items {
                item_bin(&mut w, it);
            }
            w.e(190);
        }
        if f.grouped() {
            let mut b = B::default();
            b.i32(if f.flags & 0x08 != 0 {
                i32::from(f.parent)
            } else {
                -1
            })
            .i32(i32::from(f.base));
            w.rec(219, &b.0);
            if let Some(rf) = f.range {
                let (start, dates) = num_or_serial(f.range_vals.first());
                let (end, _) = num_or_serial(f.range_vals.get(1));
                let (by, _) = num_or_serial(f.range_vals.get(2));
                let mut b = B::default();
                b.u8(((rf >> 2) & 0x07) as u8)
                    .u8((rf & 0x03) as u8 | if dates { 0x04 } else { 0 })
                    .f64(start)
                    .f64(end)
                    .f64(by);
                w.rec(223, &b.0);
                w.e(224);
            }
            if !f.group.is_empty() {
                w.rec(221, &(f.group.len() as u32).to_le_bytes());
                for it in &f.group {
                    item_bin(&mut w, it);
                }
                w.e(222);
            }
            w.e(220);
        }
        w.e(184);
    }
    w.e(182);
    w.e(180);
    // 레코드: SXDBB 마다 원본 필드의 항목 번호 (u8, 0x200 이면 u32 — 항목 300개 필드, 피벗 실험 w_big)
    let db: Vec<&Field> = fields.iter().filter(|f| !f.calc()).collect();
    let mut rw = W::default();
    rw.rec(193, &(rows.len() as u32).to_le_bytes());
    for row in &rows {
        let mut r = Rd::new(row);
        let mut b = B::default();
        for f in &db {
            let v = if f.flags & 0x200 != 0 {
                r.u32()
            } else {
                r.u8().map(u32::from)
            };
            b.u32(v.unwrap_or(0));
        }
        rw.rec(33, &b.0);
    }
    rw.e(194);
    Some((w.0, rw.0))
}

/// 레거시 항목 종류 ([MS-XLS] itmType: 4 count, 5 average, 6 max, 7 min, 8 product) → XLSB (4 avg, 5 max, 6 min,
/// 7 product, 8 count)
fn item_type(t: u16) -> u8 {
    match t {
        4 => 8,
        5 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        t => t.min(14) as u8,
    }
}

/// 피벗 필드 (SXVD 0x00B1 + SXVI 0x00B2 + SXVDEX 0x0100)
#[derive(Default)]
struct ViewField {
    axis: u16,
    grbit_sub: u16,
    name: Option<String>,
    items: Vec<(u16, u16, i32, Option<String>)>,
    /// SXVDEX: 플래그 24비트, 자동 표시 수 u8, 자동 표시 데이터 필드, 표시 형식, 한셀 플래그 u16, 부분합 글
    ex_flags: u32,
    auto_show: u8,
    rank: i16,
    ifmt: u16,
    hflags: u16,
    sub: Option<String>,
}

/// 줄 (SXLI 0x00B5 의 항목 하나): 앞 줄과 같은 수, 종류, 항목 수, 플래그, 항목 번호들
type Line = (u16, u16, u16, u16, Vec<u32>);

/// 데이터 필드 (SXDI 0x00C5): 필드, 함수, 값 표시, 기준 필드, 기준 항목, 표시 형식, 이름
type DataField = (u16, u16, u16, i16, u16, u16, Option<String>);

fn lines(d: &[u8], n: u32, slots: u16) -> Vec<Line> {
    let mut r = Rd::new(d);
    let mut out = Vec::new();
    for _ in 0..n {
        let (Some(sic), Some(t), Some(mac), Some(g)) = (r.u16(), r.u16(), r.u16(), r.u16()) else {
            break;
        };
        let idx: Vec<u32> = (0..slots).filter_map(|_| r.u32()).collect();
        out.push((sic, t, mac, g, idx));
    }
    out
}

fn lines_bin(w: &mut W, begin: u16, end: u16, list: &[Line]) {
    w.rec(begin, &(list.len() as u32).to_le_bytes());
    for (sic, t, mac, g, idx) in list {
        let xs: Vec<u32> = idx
            .get(usize::from(*sic)..usize::from(*mac).min(idx.len()))
            .unwrap_or_default()
            .to_vec();
        let mut b = B::default();
        b.u16(*sic)
            .u8(item_type(*t))
            .u8(0)
            .u32(xs.len() as u32)
            .u32(u32::from((*g >> 1) & 0xFF));
        w.rec(297, &b.0);
        if !xs.is_empty() {
            let mut b = B::default();
            for x in xs {
                b.u32(x);
            }
            w.rec(388, &b.0);
            w.e(389);
        }
        w.e(298);
    }
    w.e(end);
}

/// 피벗 표 레코드 (SXVIEW 부터) → (XLSB 피벗 표 레코드, 필드마다 includeNewItemsInFilter, 캐시 번호)
fn view_bin(recs: &[&super::reader::Logical]) -> Option<(Vec<u8>, Vec<bool>, usize)> {
    let view = recs.first()?;
    let mut r = Rd::new(&view.data);
    // SXVIEW: 범위 (행 u32 ×2, 열 u16 ×2), 머리글 행 u32, 데이터 행 u32, 데이터 열 u16, 캐시 u16, 예약 u16, 값 축 u16,
    // 값 자리 i16, 필드 수 u16 ×5 (전체·행·열·페이지·데이터), 줄 수 u32·u16 (행·열), 플래그 u16, 자동 서식 u16,
    // 이름·데이터 글자 수 u16, 글
    let (r1, r2, c1, c2) = (r.u32()?, r.u32()?, r.u16()?, r.u16()?);
    let (head, data_row, data_col) = (r.u32()?, r.u32()?, r.u16()?);
    let icache = r.u16()? as usize;
    let _ = r.u16()?;
    let axis4 = r.u16()?;
    let ipos = r.u16()? as i16;
    let (cdim, cdim_rw, cdim_col, cdim_pg, cdim_data) =
        (r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?);
    // 값 자리 −1: 한셀의 레거시 읽기는 값 필드가 열에 있을 때(데이터 필드 둘 이상, 값 축 2)만 쓰지 않고, 데이터 필드가 하나이거나
    // 값 필드가 행에 있으면 4294967295 를 쓴다 (pa·pi 대 pc·pe)
    let ipos = if ipos < 0 && cdim_data > 1 && axis4 == 2 {
        0xFFFF
    } else {
        i32::from(ipos) as u32
    };
    let (crw, ccol) = (r.u32()?, r.u16()?);
    let grbit = r.u16()?;
    let autofmt = r.u16()?;
    let (cch_name, cch_data) = (r.u16()?, r.u16()?);
    let name = r.st(cch_name)?.unwrap_or_default();
    let data_caption = r.st(cch_data).flatten().unwrap_or_default();

    let mut b3 = 0x0Fu16;
    let mut fields: Vec<ViewField> = Vec::new();
    let mut ivds: Vec<Vec<i32>> = Vec::new();
    let mut pages: Vec<(u16, i32)> = Vec::new();
    let mut data: Vec<DataField> = Vec::new();
    let mut lis: Vec<&[u8]> = Vec::new();
    let mut ex: Option<&[u8]> = None;
    for rec in &recs[1..] {
        let d = &rec.data[..];
        let mut r = Rd::new(d);
        match rec.rtype {
            0x00B3 => b3 = r.u16().unwrap_or(0x0F),
            // SXVD: 축 u16, 부분합 수 u16, 부분합 플래그 u16, 항목 수 u32, 이름
            0x00B1 => {
                let axis = r.u16()?;
                let _csub = r.u16()?;
                let grbit_sub = r.u16()?;
                let _citm = r.u32()?;
                let cch = r.u16()?;
                let name = r.st(cch)?;
                fields.push(ViewField {
                    axis,
                    grbit_sub,
                    name,
                    ..Default::default()
                });
            }
            // SXVI: 종류 u16, 플래그 u16 (0x01 숨김, 0x02 세부 숨김), 캐시 항목 i32, 이름
            0x00B2 => {
                let (t, g, ic, cch) = (r.u16()?, r.u16()?, r.u32()? as i32, r.u16()?);
                let n = r.st(cch)?;
                fields.last_mut()?.items.push((t, g, ic, n));
            }
            // SXVDEX: 플래그 24비트 + 자동 표시 수 u8, 자동 정렬 데이터 필드 u16, 자동 표시 데이터 필드 i16, 표시 형식 u16,
            // 한셀 플래그 u16 (0x01 showAll, 0x02 outline, 0x04 compact, 0x08 subtotalTop, 0x10 insertBlankRow,
            // 0x20 includeNewItemsInFilter), 부분합 글자 수 u16, 예약 8, 부분합 글
            0x0100 => {
                let f = fields.last_mut()?;
                let g = r.u32()?;
                f.ex_flags = g & 0x00FF_FFFF;
                f.auto_show = (g >> 24) as u8;
                let _sort = r.u16()?;
                f.rank = r.u16()? as i16;
                f.ifmt = r.u16()?;
                f.hflags = r.u16()?;
                let cch = r.u16()?;
                r.take(8);
                f.sub = r.st(cch).flatten();
            }
            0x00B4 => ivds.push(
                d.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| i32::from(i16::from_le_bytes(*c)))
                    .collect(),
            ),
            // SXPI: 필드 u16, 항목 i32 (−1 모두), 개체 u16 — 페이지 필드마다
            0x00B6 => {
                while let (Some(f), Some(i), Some(_)) = (r.u16(), r.u32(), r.u16()) {
                    pages.push((f, i as i32));
                }
            }
            // SXDI: 필드 u16, 함수 u16, 값 표시 u16, 기준 필드 i16, 기준 항목 u16, 표시 형식 u16, 이름
            0x00C5 => {
                let (fld, func, df, bf, bi, fmt, cch) = (
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                    r.u16()? as i16,
                    r.u16()?,
                    r.u16()?,
                    r.u16()?,
                );
                data.push((fld, func, df, bf, bi, fmt, r.st(cch)?));
            }
            0x00B5 => lis.push(d),
            0x00F1 => ex = Some(d),
            _ => {}
        }
    }
    // SXEX: 서식 수 u16, 오류·빈 칸·태그 글자 수 u16, 선택 수 u16, 페이지 필드 행·열 수 u16, 플래그 u32 (0x01
    // pageOverThenDown, 비트 1–8 pageWrap, 0x10000 enableWizard, 0x20000 enableDrill, 0x40000 enableFieldProperties,
    // 0x80000 preserveFormatting, 0x100000 mergeItem, 0x200000 showError, 0x400000 showMissing, 0x800000
    // subtotalHiddenItems), 페이지 스타일·표 스타일·빈 자리 스타일 글자 수 u16, 글 (오류, 빈 칸, 태그, 페이지, 표, 빈 자리)
    let mut exf = 0x004F_0200u32;
    let (mut crw_page, mut ccol_page) = (0u32, 0u32);
    let mut strs: [Option<String>; 6] = Default::default();
    if let Some(d) = ex {
        let mut r = Rd::new(d);
        let _fmts = r.u16()?;
        let (ce, cn, ct) = (r.u16()?, r.u16()?, r.u16()?);
        let _sel = r.u16()?;
        crw_page = u32::from(r.u16()?);
        ccol_page = u32::from(r.u16()?);
        exf = r.u32()?;
        let (cp, cs, cv) = (r.u16()?, r.u16()?, r.u16()?);
        for (k, c) in [ce, cn, ct, cp, cs, cv].into_iter().enumerate() {
            strs[k] = r.st(c).flatten();
        }
    }
    let [error, missing, tag, page, table_style, vacated] = strs;
    // 한셀의 레거시 읽기: 페이지 스타일이 없으면 태그를, 빈 자리 스타일이 없으면 표 스타일을 쓴다 (피벗 실험 v_cap, pa)
    let page = page.or_else(|| tag.clone());
    let vacated = vacated.or_else(|| table_style.clone());
    let bit = |v: u32, n: u32| (v >> n) & 1;
    // 레거시가 담지 않는 값은 한셀의 레거시 읽기가 쓰는 기본값: asteriskTotals 0, itemPrintTitles 1, 나머지는 OOXML 기본값
    let mut f2 = (1 << 1)
        | (bit(exf, 16) << 4)
        | (bit(exf, 17) << 5)
        | (bit(exf, 18) << 6)
        | (bit(exf, 19) << 7)
        | (u32::from(grbit >> 3 & 1) << 8)
        | (bit(exf, 21) << 9)
        | (bit(exf, 22) << 10)
        | (bit(exf, 0) << 11)
        | (bit(exf, 23) << 12)
        | (u32::from(grbit & 1) << 13)
        | (u32::from(grbit >> 1 & 1) << 14)
        | (1 << 17)
        | (bit(exf, 20) << 18)
        | (1 << 19)
        | (u32::from(grbit >> 4 & 0x1F) << 24)
        | (u32::from(grbit >> 9 & 1) << 29);
    for (s, n) in [(&page, 21), (&table_style, 22), (&vacated, 23), (&tag, 30)] {
        if s.is_some() {
            f2 |= 1 << n;
        }
    }
    let mut f3 = u32::from(b3 & 0x0F) | (1 << 9);
    if b3 & 0x10 == 0 {
        f3 |= 1 << 4;
    }
    if error.is_none() {
        f3 |= 1 << 6;
    }
    if missing.is_none() {
        f3 |= 1 << 7;
    }
    // 한셀은 pageWrap 을 두 배로 읽는다 (피벗 실험 w_pw1: 1 → 2, v_misc: 2 → 4)
    let wrap = (exf & 0x1FE).min(255) as u8;
    let mut w = W::default();
    let mut b = B::default();
    b.u8(3)
        .u16(0x4041)
        .u8(1)
        .u32(f2)
        .u32(f3)
        .u8(axis4 as u8)
        .u8(wrap)
        .u8(3)
        .u8(3)
        .u32(ipos)
        .u16(autofmt)
        .u16(0)
        .u32(0)
        .u32(icache as u32 + 1)
        .wstr(&name)
        .wstr(&data_caption);
    for s in [&error, &missing, &page, &table_style, &vacated, &tag]
        .into_iter()
        .flatten()
    {
        b.wstr(s);
    }
    w.rec(280, &b.0);
    let mut b = B::default();
    b.u32(r1)
        .u32(r2)
        .u32(u32::from(c1))
        .u32(u32::from(c2))
        .u32(head)
        .u32(data_row)
        .u32(u32::from(data_col))
        .u32(crw_page)
        .u32(ccol_page);
    w.rec(314, &b.0);
    w.e(313);
    w.rec(287, &u32::from(cdim).to_le_bytes());
    let mut include = Vec::new();
    for f in &fields {
        let g = f.ex_flags;
        let h = u32::from(f.hflags);
        let f1 = u32::from(f.grbit_sub & 0x0FFF)
            | (bit(h, 2) << 20)
            | (u32::from(f.name.is_some()) << 21)
            | (u32::from(f.sub.is_some()) << 22);
        let f2 = (g >> 1 & 0x0F)
            | ((1 - bit(g, 5)) << 4)
            | (bit(h, 0) << 5)
            | (bit(h, 1) << 6)
            | (bit(h, 4) << 7)
            | (bit(h, 3) << 8)
            | (bit(g, 7) << 9)
            | (bit(g, 14) << 11)
            | (bit(g, 9) << 12)
            | (bit(g, 10) << 13)
            | (bit(g, 11) << 14)
            | (bit(g, 12) << 15)
            | (bit(g, 15) << 16)
            | (1 << 18);
        include.push(bit(h, 5) != 0);
        let mut b = B::default();
        b.u8(f.axis as u8)
            .u8(f1 as u8)
            .u8((f1 >> 8) as u8)
            .u8((f1 >> 16) as u8)
            .u32(u32::from(f.ifmt))
            .u32(f2)
            .i32(i32::from(f.auto_show))
            .i32(i32::from(f.rank));
        if let Some(n) = &f.name {
            b.wstr(n);
        }
        if let Some(s) = &f.sub {
            b.wstr(s);
        }
        w.rec(285, &b.0);
        if !f.items.is_empty() {
            w.rec(283, &(f.items.len() as u32).to_le_bytes());
            for (t, g, ic, n) in &f.items {
                let mut b = B::default();
                b.u8(item_type(*t))
                    .u16((g & 0x0F) | if n.is_some() { 0x10 } else { 0 })
                    .i32(*ic);
                if let Some(n) = n {
                    b.wstr(n);
                }
                w.rec(282, &b.0);
                w.e(281);
            }
            w.e(284);
        }
        w.e(286);
    }
    w.e(288);
    let mut ivd = ivds.into_iter();
    let rows = if cdim_rw > 0 { ivd.next() } else { None };
    let cols = if cdim_col > 0 { ivd.next() } else { None };
    if let Some(v) = rows {
        let mut b = B::default();
        b.u32(v.len() as u32);
        for x in v {
            b.i32(x);
        }
        w.rec(309, &b.0);
        w.e(310);
    }
    if let Some(d) = lis.first() {
        lines_bin(&mut w, 299, 300, &lines(d, crw, cdim_rw));
    }
    if let Some(v) = cols {
        let mut b = B::default();
        b.u32(v.len() as u32);
        for x in v {
            b.i32(x);
        }
        w.rec(311, &b.0);
        w.e(312);
    }
    if let Some(d) = lis.get(1) {
        lines_bin(&mut w, 301, 302, &lines(d, u32::from(ccol), cdim_col));
    }
    if cdim_pg > 0 {
        w.rec(291, &(pages.len() as u32).to_le_bytes());
        for (fld, it) in &pages {
            let mut b = B::default();
            b.u32(u32::from(*fld))
                .u32(if *it < 0 { 0x0010_00FE } else { *it as u32 })
                .i32(-1)
                .u8(0);
            w.rec(289, &b.0);
            w.e(290);
        }
        w.e(292);
    }
    if !data.is_empty() {
        w.rec(295, &(data.len() as u32).to_le_bytes());
        for (fld, func, df, bf, bi, fmt, n) in &data {
            let mut b = B::default();
            b.u32(u32::from(*fld))
                .u32(u32::from(*func))
                .u32(u32::from(*df))
                .i32(i32::from(*bf))
                .u32(u32::from(*bi))
                .u32(u32::from(*fmt))
                .u8(if n.is_some() { 0x07 } else { 0 });
            if let Some(n) = n {
                b.wstr(n);
            }
            w.rec(293, &b.0);
            w.e(294);
        }
        w.e(296);
    }
    // 표 스타일의 머리글·줄무늬 표시는 레거시에 없다 — 한셀은 행·열 머리글과 끝 열을 켠다 (피벗 실험 v_style)
    let mut b = B::default();
    b.u16(0x32).nwstr(table_style.as_deref());
    w.rec(513, &b.0);
    w.e(315);
    Some((w.0, include, icache))
}

/// 피벗 표 레코드 종류 (SXVIEW 뒤에 오는 것)
fn view_record(t: u16) -> bool {
    matches!(
        t,
        0x00B1 | 0x00B2 | 0x00B3 | 0x00B4 | 0x00B5 | 0x00B6 | 0x00C5 | 0x0100 | 0x00F1
    )
}

/// 레거시 문서의 피벗을 XML 원본 파트와 관계로 읽는다 (`bytes`: 복합 문서, `stream`: `/Workbook`)
pub(crate) fn read(bytes: &[u8], stream: &[u8], wb: &mut Workbook) {
    let recs = super::reader::logical_records(stream);
    let mut sources: Vec<Source> = Vec::new();
    for rec in recs.iter().skip(1) {
        let mut r = Rd::new(&rec.data);
        match rec.rtype {
            0x000A => break,
            0x00D5 => sources.push(Source {
                stream: r.u16().unwrap_or(0),
                ..Default::default()
            }),
            0x0051 => {
                if let Some(s) = sources.last_mut() {
                    s.range = dconref(&rec.data);
                }
            }
            0x0052 => {
                if let Some(s) = sources.last_mut() {
                    s.name = r.xst();
                }
            }
            _ => {}
        }
    }
    if sources.is_empty() {
        return;
    }
    // 캐시: 스트림 `/_SX_DB_CUR/` + 번호 16진수 네 자리
    let mut caches: Vec<Option<String>> = Vec::new();
    for (k, s) in sources.iter().enumerate() {
        let data = crate::container::read_entry(bytes, &format!("/_SX_DB_CUR/{:04X}", s.stream));
        let xml = data
            .ok()
            .and_then(|d| cache_bin(&d, s))
            .and_then(|(def, rec)| crate::binary::pivot::cache_to_xml(&def, Some(&rec)));
        caches.push(xml.map(|(def, rec)| {
            crate::binary::pivot::add_cache_parts(
                &mut wb.raw_parts,
                &mut wb.part_links,
                k + 1,
                def,
                rec,
                Some(k as u32 + 1),
            )
        }));
    }
    // 시트마다 피벗 표: 시트 BOF 부터 짝이 맞는 EOF 까지 (차트 BOF…EOF 는 안쪽)
    let mut n = 0;
    for si in 0..wb.sheets.len() {
        if wb.sheets[si].kind != SheetKind::Worksheet {
            continue;
        }
        let Some(off) = wb.sheets[si]
            .part
            .strip_prefix("/Workbook@0x")
            .and_then(|h| usize::from_str_radix(h, 16).ok())
        else {
            continue;
        };
        let Some(start) = recs.iter().position(|r| r.offset == off) else {
            continue;
        };
        let mut depth = 0;
        let mut views: Vec<Vec<&super::reader::Logical>> = Vec::new();
        for rec in &recs[start..] {
            match rec.rtype {
                0x0809 => depth += 1,
                0x000A => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                0x00B0 if depth == 1 => views.push(vec![rec]),
                t if depth == 1 && view_record(t) => {
                    if let Some(v) = views.last_mut() {
                        v.push(rec);
                    }
                }
                _ => {}
            }
        }
        for v in views {
            let Some((bin, include, icache)) = view_bin(&v) else {
                continue;
            };
            let (Some(Some(cache)), Some(xml)) = (
                caches.get(icache),
                crate::binary::pivot::table_to_xml(&bin, Some(&include)),
            ) else {
                continue;
            };
            n += 1;
            crate::binary::pivot::add_table_part(
                &mut wb.raw_parts,
                &mut wb.sheets[si].part_links,
                n,
                xml,
                &cache.clone(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::reader::Logical;
    use super::*;

    /// 레거시 레코드 하나 (종류 u16, 길이 u16, 내용)
    fn rec(t: u16, d: &[u8]) -> Vec<u8> {
        let mut v = t.to_le_bytes().to_vec();
        v.extend_from_slice(&(d.len() as u16).to_le_bytes());
        v.extend_from_slice(d);
        v
    }

    /// 글자 수 u16 + 플래그 0 + UTF-16
    fn xst(s: &str) -> Vec<u8> {
        let u: Vec<u16> = s.encode_utf16().collect();
        let mut v = (u.len() as u16).to_le_bytes().to_vec();
        v.push(0);
        for c in u {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v
    }

    fn sxfdb(flags: u16, n: u32, name: &str) -> Vec<u8> {
        let mut d = flags.to_le_bytes().to_vec();
        d.extend_from_slice(&[0; 4]);
        d.extend_from_slice(&(n as u16).to_le_bytes());
        d.extend_from_slice(&[0; 10]);
        d.extend_from_slice(&n.to_le_bytes());
        d.extend_from_slice(&xst(name));
        rec(0x00C7, &d)
    }

    /// 손으로 만든 캐시 스트림: 글 필드(항목 둘 + 빈 칸)와 정수 필드, 레코드 셋
    #[test]
    fn cache_stream_to_xml() {
        let mut s = Vec::new();
        let mut db = 3u32.to_le_bytes().to_vec();
        db.extend_from_slice(&[1, 0, 0x21, 0, 0, 0, 2, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0]);
        s.extend(rec(0x00C6, &db));
        s.extend(rec(0x0122, &[0; 12]));
        s.extend(sxfdb(0x0481, 3, "품목"));
        s.extend(rec(0x01BB, &[0, 0]));
        s.extend(rec(0x00CD, &xst("사과")));
        s.extend(rec(0x00CF, &[]));
        s.extend(rec(0x00CD, &xst("배")));
        s.extend(sxfdb(0x0561, 2, "금액"));
        s.extend(rec(0x01BB, &[0, 0]));
        s.extend(rec(0x00C9, &100f64.to_le_bytes()));
        s.extend(rec(0x00C9, &300f64.to_le_bytes()));
        for r in [[0u8, 0], [1, 1], [2, 0]] {
            s.extend(rec(0x00C8, &r));
        }
        s.extend(rec(0x000A, &[]));
        let src = Source {
            stream: 1,
            range: Some((0, 3, 0, 1, Some("데이터".into()))),
            name: None,
        };
        let (def, recs) = cache_bin(&s, &src).unwrap();
        let (xml, rec_xml) = crate::binary::pivot::cache_to_xml(&def, Some(&recs)).unwrap();
        for want in [
            r#"<x:worksheetSource ref="A1:B4" sheet="데이터"/>"#,
            r#"recordCount="3""#,
            r#"<x:sharedItems containsBlank="1" count="3"><x:s v="사과"/><x:m/><x:s v="배"/>"#,
            r#"containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="100" maxValue="300" count="2""#,
        ] {
            assert!(xml.contains(want), "{want}\n{xml}");
        }
        let rec_xml = rec_xml.unwrap();
        assert!(
            rec_xml.contains(r#"<x:r><x:x v="2"/><x:x v="0"/></x:r>"#),
            "{rec_xml}"
        );
    }

    /// 손으로 만든 피벗 표 레코드: 행 필드 하나(항목 둘, 하나 숨김) + 데이터 필드, 스타일 글
    #[test]
    fn view_records_to_xml() {
        let mut v = Vec::new();
        for x in [0u32, 3] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&[0, 0, 1, 0]);
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 0, 0, 0, 0, 2, 0, 0xFF, 0xFF]);
        v.extend_from_slice(&[2, 0, 1, 0, 0, 0, 0, 0, 1, 0]);
        v.extend_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 0x0B, 0x02, 0, 0, 2, 0, 1, 0]);
        v.push(0);
        for c in "피벗".encode_utf16() {
            v.extend_from_slice(&c.to_le_bytes());
        }
        v.push(0);
        for c in "값".encode_utf16() {
            v.extend_from_slice(&c.to_le_bytes());
        }
        let mut ex = vec![0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0];
        ex.extend_from_slice(&0x004F_0200u32.to_le_bytes());
        ex.extend_from_slice(&[0xFF, 0xFF, 4, 0, 0xFF, 0xFF, 0]);
        for c in "표스타일".encode_utf16() {
            ex.extend_from_slice(&c.to_le_bytes());
        }
        let li = |lines: &[(u16, u16, u16, u16, u32)]| {
            let mut d = Vec::new();
            for (sic, t, mac, g, i) in lines {
                for x in [*sic, *t, *mac, *g] {
                    d.extend_from_slice(&x.to_le_bytes());
                }
                d.extend_from_slice(&i.to_le_bytes());
            }
            d
        };
        let mut data_field = vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        data_field.extend_from_slice(&xst("합계"));
        let vdex = |h: u8| {
            vec![
                0x1E, 0x14, 0, 10, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, h, 0, 0xFF, 0xFF, 0, 0, 0, 0, 0,
                0, 0, 0,
            ]
        };
        let raw: Vec<(u16, Vec<u8>)> = vec![
            (0x00B0, v),
            (0x00B3, vec![0x0F, 0]),
            (0x00B1, vec![1, 0, 1, 0, 1, 0, 3, 0, 0, 0, 0xFF, 0xFF]),
            (0x00B2, vec![0, 0, 1, 0, 1, 0, 0, 0, 0xFF, 0xFF]),
            (0x00B2, vec![0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]),
            (0x00B2, vec![1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            (0x0100, vdex(0x0E)),
            (0x00B1, vec![8, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0xFF, 0xFF]),
            (0x0100, vdex(0x2E)),
            (0x00B4, vec![0, 0]),
            (0x00C5, data_field),
            (0x00B5, li(&[(0, 0, 1, 0, 0), (0, 13, 1, 0x0A00, 0)])),
            (0x00B5, vec![0, 0, 0, 0, 0, 0, 0, 0]),
            (0x00F1, ex),
        ];
        let logical: Vec<Logical> = raw
            .into_iter()
            .map(|(rtype, data)| Logical {
                rtype,
                offset: 0,
                data,
            })
            .collect();
        let refs: Vec<&Logical> = logical.iter().collect();
        let (bin, include, icache) = view_bin(&refs).unwrap();
        assert_eq!(icache, 0);
        assert_eq!(include, [false, true]);
        let xml = crate::binary::pivot::table_to_xml(&bin, Some(&include)).unwrap();
        for want in [
            r#"name="피벗" cacheId="1" dataOnRows="0" dataPosition="4294967295""#,
            r#"dataCaption="값""#,
            r#"vacatedStyle="표스타일""#,
            r#"<x:location ref="A1:B4" firstHeaderRow="1" firstDataRow="1" firstDataCol="1" rowPageCount="0" colPageCount="0"/>"#,
            r#"<x:item h="1" x="1"/>"#,
            r#"<x:pivotField dataField="1" showAll="0" includeNewItemsInFilter="1"/>"#,
            r#"<x:i t="grand"><x:x v="0"/></x:i>"#,
            r#"<x:dataField name="합계" fld="1" subtotal="sum""#,
            r#"<x:pivotTableStyleInfo name="표스타일" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/>"#,
        ] {
            assert!(xml.contains(want), "{want}\n{xml}");
        }
    }
}
