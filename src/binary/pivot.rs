// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 피벗 표·캐시: XML 파트(`pivotTableDefinition`, `pivotCacheDefinition`, `pivotCacheRecords`) ↔ 바이너리 레코드 파트.
//!
//! rcell 의 피벗 모델은 XML 파트 그대로다 (`Workbook::raw_parts` + `PartLink`). 바이너리 쓰기는 XML 파트를 레코드로 바꾸고
//! ([`table_to_bin`], [`cache_to_bin`]), 바이너리 읽기는 레코드를 한셀이 쓰는 꼴의 XML 로 바꾼다 ([`table_to_xml`],
//! [`cache_to_xml`]). 대응은 피벗 실험에서 얻었다: COM 으로 만든 피벗 문서 11개(행·열·페이지·값 필드, 함수 11가지, 값
//! 종류, 캐시 둘·피벗 셋)와 그 피벗 XML 의 속성을 하나씩 바꾼 문서 33개를 한셀이 XML·바이너리로 저장한 것을 레코드마다
//! 맞대었다 ([MS-XLSB] 2.4 의 레코드·필드 이름). docs/format/cell-binary.md 5절.

use std::fmt::Write as _;

use crate::xml::Element;

/// 레코드 쓰기 ([MS-XLSB] 2.1.4 의 가변 길이 종류·크기)
#[derive(Default)]
pub(crate) struct W(pub(crate) Vec<u8>);

impl W {
    pub(crate) fn rec(&mut self, t: u16, data: &[u8]) {
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
    pub(crate) fn e(&mut self, t: u16) {
        self.rec(t, &[]);
    }
}

/// 레코드 내용 만들기
#[derive(Default)]
pub(crate) struct B(pub(crate) Vec<u8>);

impl B {
    pub(crate) fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub(crate) fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// XLWideString
    pub(crate) fn wstr(&mut self, s: &str) -> &mut Self {
        let u: Vec<u16> = s.encode_utf16().collect();
        self.u32(u.len() as u32);
        for x in u {
            self.u16(x);
        }
        self
    }
    /// XLNullableWideString
    pub(crate) fn nwstr(&mut self, s: Option<&str>) -> &mut Self {
        match s {
            Some(s) => self.wstr(s),
            None => self.u32(0xFFFF_FFFF),
        }
    }
}

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
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
    }
    fn wstr(&mut self) -> Option<String> {
        let n = self.u32()?;
        if n == 0xFFFF_FFFF {
            return Some(String::new());
        }
        let b = self.take(n as usize * 2)?;
        Some(crate::text::utf16le(b))
    }
    fn nwstr(&mut self) -> Option<Option<String>> {
        let n = self.u32()?;
        if n == 0xFFFF_FFFF {
            return Some(None);
        }
        let b = self.take(n as usize * 2)?;
        Some(Some(crate::text::utf16le(b)))
    }
}

fn esc(s: &str) -> String {
    crate::writer::esc(s)
}

/// XML 속성 → 참/거짓 (없으면 기본값)
pub(crate) fn ab(e: &Element, name: &str, default: bool) -> bool {
    e.attr_bool(name).unwrap_or(default)
}

pub(crate) fn au(e: &Element, name: &str, default: u32) -> u32 {
    e.attr(name)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map_or(default, |v| v as u32)
}

pub(crate) fn ai(e: &Element, name: &str, default: i32) -> i32 {
    e.attr(name)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .map_or(default, |v| v as i32)
}

/// 숫자 → XML (한셀처럼 가장 짧은 꼴: 50, 20.5)
fn num(v: f64) -> String {
    format!("{v}")
}

/// 수 값 꼴의 XML 글 (-0 은 0)
fn fmt_num(v: f64) -> String {
    if v == 0.0 { "0".into() } else { num(v) }
}

// ---------------------------------------------------------------------------
// 날짜 (PCDIDateTime: 해 u16, 달 u16, 날 u8, 시 u8, 분 u8, 초 u8 — XML 은 `2025-12-09T00:00:00.000`)
// ---------------------------------------------------------------------------

/// 그레고리력 날짜 → 1970-01-01 부터의 날 수
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1900 날짜 체계의 일련번호 (1900-03-01 이후) ↔ 날짜. 1899-12-30 이 0 이다
const SERIAL_EPOCH: i64 = -25_569; // 1899-12-30 의 days_from_civil

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DateTime {
    y: u16,
    mo: u16,
    d: u8,
    h: u8,
    mi: u8,
    s: u8,
}

impl DateTime {
    pub(crate) fn parse(v: &str) -> Option<DateTime> {
        // 2025-12-09T00:00:00.000 (시각은 없어도 된다)
        let (date, time) = v.split_once('T').unwrap_or((v, "00:00:00"));
        let mut it = date.split('-');
        let y: u16 = it.next()?.trim().parse().ok()?;
        let mo: u16 = it.next()?.trim().parse().ok()?;
        let d: u8 = it.next()?.trim().parse().ok()?;
        let mut t = time.split(':');
        let h: u8 = t.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let mi: u8 = t.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let s: u8 = t
            .next()
            .and_then(|x| x.split('.').next())
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        Some(DateTime { y, mo, d, h, mi, s })
    }
    fn xml(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000",
            self.y, self.mo, self.d, self.h, self.mi, self.s
        )
    }
    pub(crate) fn bytes(&self, b: &mut B) {
        b.u16(self.y)
            .u16(self.mo)
            .u8(self.d)
            .u8(self.h)
            .u8(self.mi)
            .u8(self.s);
    }
    fn read(r: &mut Rd) -> Option<DateTime> {
        Some(DateTime {
            y: r.u16()?,
            mo: r.u16()?,
            d: r.u8()?,
            h: r.u8()?,
            mi: r.u8()?,
            s: r.u8()?,
        })
    }
    /// 일련번호 (DateAsXnum)
    pub(crate) fn serial(&self) -> f64 {
        let days = days_from_civil(i64::from(self.y), i64::from(self.mo), i64::from(self.d))
            - SERIAL_EPOCH;
        days as f64
            + (f64::from(self.h) * 3600.0 + f64::from(self.mi) * 60.0 + f64::from(self.s))
                / 86_400.0
    }
    fn from_serial(v: f64) -> DateTime {
        let days = v.floor() as i64;
        let secs = ((v - v.floor()) * 86_400.0).round() as i64;
        let (y, mo, d) = civil_from_days(days + SERIAL_EPOCH);
        DateTime {
            y: y as u16,
            mo: mo as u16,
            d: d as u8,
            h: (secs / 3600) as u8,
            mi: (secs / 60 % 60) as u8,
            s: (secs % 60) as u8,
        }
    }
}

/// 8바이트 날짜(해 u16, 달 u16, 날·시·분·초 u8 — PCDIDateTime, 레거시 SXDTR 도 같은 꼴) → 1900 날짜 체계 일련번호
pub(crate) fn date_serial(b: &[u8]) -> Option<f64> {
    DateTime::read(&mut Rd::new(b)).map(|d| d.serial())
}

// ---------------------------------------------------------------------------
// 피벗 표: 속성 ↔ 비트
// ---------------------------------------------------------------------------

/// (XML 속성, 기본값, 비트, 뒤집음) — BrtBeginSXView 의 플래그 묶음 셋 (피벗 실험 v_misc·v_tab·v_cap 등으로 비트마다 확인)
type Flag = (&'static str, bool, u32, bool);

/// 바이트 1–2 (u16)
const VIEW_FLAGS1: &[Flag] = &[
    ("showItems", true, 0, false),
    ("editData", false, 1, false),
    ("disableFieldList", false, 2, false),
    ("showCalcMbrs", true, 4, true),
    ("visualTotals", true, 5, true),
    ("showMultipleLabel", true, 6, false),
    ("showDataDropDown", true, 8, true),
    ("showDrill", true, 12, true),
    ("printDrill", false, 13, false),
    ("showMemberPropertyTips", true, 14, false),
    ("showDataTips", true, 15, true),
];

/// 바이트 4–7 (u32). 0x80000 (fDisplayData) 는 늘 켠다, 0x100000·0x200000·0x400000·0x800000·0x40000000 은 글이 있음
const VIEW_FLAGS2: &[Flag] = &[
    ("showDropZones", true, 0, true),
    ("asteriskTotals", false, 1, true),
    ("showEmptyRow", false, 2, false),
    ("showEmptyCol", false, 3, false),
    ("enableWizard", true, 4, false),
    ("enableDrill", true, 5, false),
    ("enableFieldProperties", true, 6, false),
    ("preserveFormatting", true, 7, false),
    ("useAutoFormatting", false, 8, false),
    ("showError", false, 9, false),
    ("showMissing", true, 10, false),
    ("pageOverThenDown", false, 11, false),
    ("subtotalHiddenItems", false, 12, false),
    ("rowGrandTotals", true, 13, false),
    ("colGrandTotals", true, 14, false),
    ("fieldPrintTitles", false, 15, false),
    ("itemPrintTitles", false, 17, false),
    ("mergeItem", false, 18, false),
    ("applyNumberFormats", false, 24, false),
    ("applyFontFormats", false, 25, false),
    ("applyAlignmentFormats", false, 26, false),
    ("applyBorderFormats", false, 27, false),
    ("applyPatternFormats", false, 28, false),
    ("applyWidthHeightFormats", false, 29, false),
];

/// 바이트 8–11 (u32). 0x40·0x80 은 오류·빈 칸 글이 없음, 0x400·0x800 은 행·열 머리글 글이 있음
const VIEW_FLAGS3: &[Flag] = &[
    ("compact", true, 0, false),
    ("outline", false, 1, false),
    ("outlineData", false, 2, false),
    ("compactData", true, 3, false),
    ("gridDropZones", false, 4, true),
    ("published", false, 5, false),
    ("immersive", true, 8, true),
    ("multipleFieldFilters", true, 9, true),
    ("fieldListSortAscending", false, 12, false),
    ("customListSort", true, 14, true),
];

fn flags_from(e: &Element, table: &[Flag]) -> u32 {
    table.iter().fold(0, |f, (name, def, bit, inv)| {
        if ab(e, name, *def) ^ inv {
            f | (1 << bit)
        } else {
            f
        }
    })
}

fn flag_value(flags: u32, table: &[Flag], name: &str) -> bool {
    table
        .iter()
        .find(|t| t.0 == name)
        .is_some_and(|(_, _, bit, inv)| ((flags >> bit) & 1 != 0) ^ inv)
}

/// 피벗 필드 (BrtBeginSXVD) 첫 묶음 (24비트): 부분합 함수와 표시
const VD_FLAGS1: &[Flag] = &[
    ("defaultSubtotal", true, 0, false),
    ("sumSubtotal", false, 1, false),
    ("countASubtotal", false, 2, false),
    ("avgSubtotal", false, 3, false),
    ("maxSubtotal", false, 4, false),
    ("minSubtotal", false, 5, false),
    ("productSubtotal", false, 6, false),
    ("countSubtotal", false, 7, false),
    ("stdDevSubtotal", false, 8, false),
    ("stdDevPSubtotal", false, 9, false),
    ("varSubtotal", false, 10, false),
    ("varPSubtotal", false, 11, false),
    ("allDrilled", false, 16, false),
    ("showDropDowns", true, 17, true),
    ("hiddenLevel", false, 18, false),
    ("compact", true, 20, false),
    ("dataSourceSort", false, 23, false),
];

/// 피벗 필드 둘째 묶음 (u32). 비트 12·13 은 정렬 (sortType), 18 은 한셀이 늘 켠다
const VD_FLAGS2: &[Flag] = &[
    ("dragToRow", true, 0, false),
    ("dragToCol", true, 1, false),
    ("dragToPage", true, 2, false),
    ("dragOff", true, 3, false),
    ("dragToData", true, 4, false),
    ("showAll", true, 5, false),
    ("outline", true, 6, false),
    ("insertBlankRow", false, 7, false),
    ("subtotalTop", true, 8, false),
    ("serverField", false, 9, false),
    ("insertPageBreak", false, 11, false),
    ("autoShow", false, 14, false),
    ("topAutoShow", true, 15, false),
    ("hideNewItems", false, 16, false),
    ("measureFilter", false, 17, false),
    ("multipleItemSelectionAllowed", false, 19, false),
    ("nonAutoSortDefault", false, 20, false),
    ("showPropCell", false, 21, false),
    ("showPropTip", false, 22, false),
    ("showPropAsCaption", false, 23, false),
    ("defaultAttributeDrillState", false, 24, false),
];

/// 피벗 항목 종류 (PivotItemType 0–14) = XML `item/@t`·`i/@t`
pub(crate) const ITEM_TYPES: [&str; 15] = [
    "data", "default", "sum", "countA", "avg", "max", "min", "product", "count", "stdDev",
    "stdDevP", "var", "varP", "grand", "blank",
];

/// 데이터 필드 함수 (DataConsolidationFunction) = XML `dataField/@subtotal`
pub(crate) const SUBTOTALS: [&str; 11] = [
    "sum",
    "count",
    "average",
    "max",
    "min",
    "product",
    "countNums",
    "stdDev",
    "stdDevp",
    "var",
    "varp",
];

/// 값 표시 (ShowDataAs) = XML `dataField/@showDataAs`
pub(crate) const SHOW_DATA_AS: [&str; 9] = [
    "normal",
    "difference",
    "percent",
    "percentDiff",
    "runTotal",
    "percentOfRow",
    "percentOfCol",
    "percentOfTotal",
    "index",
];

/// `exact` 면 버린다 (한셀이 바이너리에 쓰지 않는 값)
fn unless(exact: bool, v: Option<&str>) -> Option<&str> {
    if exact { None } else { v }
}

pub(crate) fn index_of(list: &[&str], v: Option<&str>, default: usize) -> usize {
    v.and_then(|v| list.iter().position(|x| *x == v))
        .unwrap_or(default)
}

fn axis_code(axis: Option<&str>) -> u8 {
    match axis {
        Some("axisRow") => 0x01,
        Some("axisCol") => 0x02,
        Some("axisPage") => 0x04,
        Some("axisValues") => 0x08,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// 피벗 표: XML → 레코드
// ---------------------------------------------------------------------------

/// `pivotTableDefinition` XML → 피벗 표 레코드 파트. 한셀의 바이너리 저장처럼 서식(`formats`)·조건부 서식·차트 서식과
/// x14 확장은 쓰지 않는다 (피벗 실험 v_fmt: 한셀 바이너리에 없다). `exact` 면 한셀이 바이너리에 쓰지 않는 값도 버린다
/// (피벗 실험 v_cap·v_apply·v_autoshow·v_drag·v_fname·v_mpage·v_blank·v_sda: 총합계·빈 자리 스타일·태그·행/열 머리글 글,
/// 서식 적용 다섯 가지와 자동 서식 번호, 필드의 부분합 글·자동 표시·끌기 금지·드롭다운 숨김·새 항목 숨김·측정값
/// 필터·여러 항목 선택·자동 정렬 아님·페이지 나누기, 데이터 필드의 값 표시 방식·기준 필드·기준 항목·표시 형식)
pub(crate) fn table_to_bin(xml: &str, exact: bool) -> Result<Vec<u8>, String> {
    let root = crate::xml::parse(xml, "pivotTable").map_err(|e| e.to_string())?;
    let mut w = W::default();
    let style = root.sml_child("pivotTableStyleInfo");
    // 한셀이 바이너리에 쓰지 않는 값 (exact)
    let keep = |v| unless(exact, v);
    // BrtBeginSXView
    let mut f2 = flags_from(&root, VIEW_FLAGS2) | 0x0008_0000;
    if exact {
        f2 &= !(0x1F << 24);
    }
    let mut f3 = flags_from(&root, VIEW_FLAGS3);
    let grand = keep(root.attr("grandTotalCaption"));
    let error = root.attr("errorCaption");
    let missing = root.attr("missingCaption");
    let page_style = root.attr("pageStyle");
    // 한셀은 표 스타일 이름(pivotTableStyleInfo)을 irstTableStyle 에도 쓴다
    let table_style = root
        .attr("pivotTableStyle")
        .or_else(|| style.and_then(|s| s.attr("name")));
    let vacated = keep(root.attr("vacatedStyle"));
    let tag = keep(root.attr("tag"));
    let col_hdr = keep(root.attr("colHeaderCaption"));
    let row_hdr = keep(root.attr("rowHeaderCaption"));
    for (present, bit) in [
        (grand.is_some(), 20),
        (page_style.is_some(), 21),
        (table_style.is_some(), 22),
        (vacated.is_some(), 23),
        (tag.is_some(), 30),
    ] {
        if present {
            f2 |= 1 << bit;
        }
    }
    if error.is_none() {
        f3 |= 1 << 6;
    }
    if missing.is_none() {
        f3 |= 1 << 7;
    }
    if row_hdr.is_some() {
        f3 |= 1 << 10;
    }
    if col_hdr.is_some() {
        f3 |= 1 << 11;
    }
    let indent = au(&root, "indent", 1).min(126) as u8 + 1;
    let mut v = B::default();
    // 데이터 기능 수준: 한셀은 XML 의 판과 상관없이 늘 3 (피벗 실험 v_ver)
    v.u8(3)
        .u16(flags_from(&root, VIEW_FLAGS1) as u16)
        .u8(indent
            | if ab(&root, "showHeaders", true) {
                0
            } else {
                0x80
            })
        .u32(f2)
        .u32(f3)
        .u8(if ab(&root, "dataOnRows", false) { 1 } else { 2 })
        .u8(au(&root, "pageWrap", 0) as u8)
        .u8(3)
        .u8(3)
        // dataPosition 이 없으면 0xFFFF (한셀), 있으면 그 값 (4294967295 = -1)
        .u32(
            root.attr("dataPosition")
                .map_or(0xFFFF, |_| au(&root, "dataPosition", 0)),
        )
        .u16(if exact {
            0
        } else {
            au(&root, "autoFormatId", 0) as u16
        })
        .u16(0)
        .u32(au(&root, "chartFormat", 0))
        .u32(au(&root, "cacheId", 0))
        .wstr(root.attr("name").unwrap_or(""))
        .wstr(root.attr("dataCaption").unwrap_or("Values"));
    for s in [
        grand,
        error,
        missing,
        page_style,
        table_style,
        vacated,
        tag,
        col_hdr,
        row_hdr,
    ]
    .into_iter()
    .flatten()
    {
        v.wstr(s);
    }
    w.rec(280, &v.0);
    // BrtBeginSXLocation: 범위, 머리글 행·데이터 행·데이터 열 (절대), 페이지 필드 행·열 수
    if let Some(l) = root.sml_child("location") {
        let r = l
            .attr("ref")
            .and_then(crate::model::CellRange::parse)
            .unwrap_or(crate::model::CellRange {
                start: Default::default(),
                end: Default::default(),
            });
        let mut b = B::default();
        b.u32(r.start.row)
            .u32(r.end.row)
            .u32(r.start.col)
            .u32(r.end.col)
            .u32(r.start.row + au(l, "firstHeaderRow", 0))
            .u32(r.start.row + au(l, "firstDataRow", 0))
            .u32(r.start.col + au(l, "firstDataCol", 0))
            .u32(au(l, "rowPageCount", 0))
            .u32(au(l, "colPageCount", 0));
        w.rec(314, &b.0);
        w.e(313);
    }
    // 피벗 필드
    let fields: Vec<&Element> = root
        .sml_child("pivotFields")
        .map(|p| p.sml_children("pivotField").collect())
        .unwrap_or_default();
    w.rec(287, &(fields.len() as u32).to_le_bytes());
    for f in &fields {
        let name = f.attr("name");
        let sub_caption = keep(f.attr("subtotalCaption"));
        let mut f1 = flags_from(f, VD_FLAGS1);
        if exact {
            f1 &= !(1 << 17);
        }
        if f.attr("uniqueMemberProperty").is_some() {
            f1 |= 1 << 19;
        }
        if name.is_some() {
            f1 |= 1 << 21;
        }
        if sub_caption.is_some() {
            f1 |= 1 << 22;
        }
        let mut f2 = flags_from(f, VD_FLAGS2);
        // 정렬: 수동은 오름 비트만, 오름차순은 자동 + 오름, 내림차순은 자동 (피벗 실험 v_sorta·v_sortd)
        match f.attr("sortType") {
            Some("ascending") => f2 |= (1 << 12) | (1 << 13),
            Some("descending") => f2 |= 1 << 12,
            _ => f2 |= 1 << 13,
        }
        f2 |= 1 << 18;
        if exact {
            f2 = (f2 | 0x1F | (1 << 15))
                & !((1 << 11) | (1 << 14) | (1 << 16) | (1 << 17) | (1 << 19) | (1 << 20));
        }
        let mut axis = axis_code(f.attr("axis"));
        if ab(f, "dataField", false) {
            axis |= 0x08;
        }
        let mut b = B::default();
        b.u8(axis)
            .u8(f1 as u8)
            .u8((f1 >> 8) as u8)
            .u8((f1 >> 16) as u8)
            .u32(au(f, "numFmtId", 0))
            .u32(f2)
            .i32(if exact {
                10
            } else {
                ai(f, "itemPageCount", 10)
            })
            .i32(if exact { -1 } else { ai(f, "rankBy", -1) });
        if let Some(n) = name {
            b.wstr(n);
        }
        if let Some(s) = sub_caption {
            b.wstr(s);
        }
        if let Some(u) = f.attr("uniqueMemberProperty") {
            b.wstr(u);
        }
        w.rec(285, &b.0);
        if let Some(items) = f.sml_child("items") {
            let list: Vec<&Element> = items.sml_children("item").collect();
            w.rec(283, &(list.len() as u32).to_le_bytes());
            for it in list {
                let t = index_of(&ITEM_TYPES, it.attr("t"), 0);
                let n = it.attr("n");
                let flags = u16::from(ab(it, "h", false))
                    | (u16::from(!ab(it, "sd", true)) << 1)
                    | (u16::from(ab(it, "f", false)) << 2)
                    | (u16::from(ab(it, "m", false)) << 3)
                    | (u16::from(n.is_some()) << 4)
                    | (u16::from(ab(it, "d", false)) << 5)
                    | (u16::from(ab(it, "c", false)) << 6);
                let mut b = B::default();
                b.u8(t as u8)
                    .u16(flags)
                    .i32(if t == 0 { ai(it, "x", 0) } else { -1 });
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
    let field_list = |name: &str| -> Option<Vec<i32>> {
        root.sml_child(name)
            .map(|e| e.sml_children("field").map(|f| ai(f, "x", 0)).collect())
    };
    let lines = |w: &mut W, name: &str, begin: u16, end: u16| {
        let Some(e) = root.sml_child(name) else {
            return;
        };
        let items: Vec<&Element> = e.sml_children("i").collect();
        w.rec(begin, &(items.len() as u32).to_le_bytes());
        for i in items {
            let xs: Vec<u32> = i.sml_children("x").map(|x| au(x, "v", 0)).collect();
            let mut b = B::default();
            b.u16(au(i, "r", 0) as u16)
                .u8(index_of(&ITEM_TYPES, i.attr("t"), 0) as u8)
                .u8(0)
                .u32(xs.len() as u32)
                .u32(au(i, "i", 0));
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
    };
    if let Some(rows) = field_list("rowFields") {
        let mut b = B::default();
        b.u32(rows.len() as u32);
        for x in rows {
            b.i32(x);
        }
        w.rec(309, &b.0);
        w.e(310);
    }
    lines(&mut w, "rowItems", 299, 300);
    if let Some(cols) = field_list("colFields") {
        let mut b = B::default();
        b.u32(cols.len() as u32);
        for x in cols {
            b.i32(x);
        }
        w.rec(311, &b.0);
        w.e(312);
    }
    lines(&mut w, "colItems", 301, 302);
    if let Some(pages) = root.sml_child("pageFields") {
        let list: Vec<&Element> = pages.sml_children("pageField").collect();
        w.rec(291, &(list.len() as u32).to_le_bytes());
        for p in list {
            let (name, cap) = (p.attr("name"), p.attr("cap"));
            let mut b = B::default();
            b.u32(au(p, "fld", 0))
                // 고른 항목이 없으면 0x001000FE (모두)
                .u32(au(p, "item", 0x0010_00FE))
                .i32(ai(p, "hier", -1))
                .u8(u8::from(name.is_some()) | (u8::from(cap.is_some()) << 1));
            if let Some(n) = name {
                b.wstr(n);
            }
            if let Some(c) = cap {
                b.wstr(c);
            }
            w.rec(289, &b.0);
            w.e(290);
        }
        w.e(292);
    }
    if let Some(data) = root.sml_child("dataFields") {
        let list: Vec<&Element> = data.sml_children("dataField").collect();
        w.rec(295, &(list.len() as u32).to_le_bytes());
        for d in list {
            let name = d.attr("name");
            let mut b = B::default();
            b.u32(au(d, "fld", 0))
                .u32(index_of(&SUBTOTALS, d.attr("subtotal"), 0) as u32);
            if exact {
                b.u32(0).i32(0).u32(0).u32(0);
            } else {
                b.u32(index_of(&SHOW_DATA_AS, d.attr("showDataAs"), 0) as u32)
                    .i32(ai(d, "baseField", -1))
                    .u32(au(d, "baseItem", 0x0010_0100))
                    .u32(au(d, "numFmtId", 0));
            }
            b
                // 이름이 있음: 한셀은 0x07 을 쓴다 (Boolean 칸)
                .u8(if name.is_some() { 0x07 } else { 0 });
            if let Some(n) = name {
                b.wstr(n);
            }
            w.rec(293, &b.0);
            w.e(294);
        }
        w.e(296);
    }
    if let Some(s) = style {
        // BrtTableStyleClient: 0x02 끝 열, 0x04 행 줄무늬, 0x08 열 줄무늬, 0x10 행 머리글, 0x20 열 머리글. 한셀은 나머지
        // 비트(0x40, 0xD400 …)에 초기화하지 않은 값을 남긴다 — rcell 은 0
        let flags = (u16::from(ab(s, "showLastColumn", false)) << 1)
            | (u16::from(ab(s, "showRowStripes", false)) << 2)
            | (u16::from(ab(s, "showColStripes", false)) << 3)
            | (u16::from(ab(s, "showRowHeaders", false)) << 4)
            | (u16::from(ab(s, "showColHeaders", false)) << 5);
        let mut b = B::default();
        b.u16(flags).nwstr(s.attr("name"));
        w.rec(513, &b.0);
    }
    w.e(315);
    Ok(w.0)
}

// ---------------------------------------------------------------------------
// 피벗 캐시: XML → 레코드
// ---------------------------------------------------------------------------

/// 캐시 필드의 값 꼴 — 레코드(BrtPCRRecord)에서 값을 어떻게 읽고 쓰는지
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct FieldKind {
    /// 원본 필드 (databaseField) — 레코드에 값이 있다. 계산 필드는 없다
    db: bool,
    /// 계산 필드 (수식)
    calc: bool,
    /// 공유 항목이 있다 → 레코드는 항목 번호 (u32)
    items: bool,
    num: bool,
    date: bool,
    text: bool,
}

/// 공유 항목의 값
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    Missing,
    Num(f64),
    Bool(bool),
    Err(String),
    Str(String),
    Date(DateTime),
}

pub(crate) const ERRORS: [(&str, u8); 7] = [
    ("#NULL!", 0x00),
    ("#DIV/0!", 0x07),
    ("#VALUE!", 0x0F),
    ("#REF!", 0x17),
    ("#NAME?", 0x1D),
    ("#NUM!", 0x24),
    ("#N/A", 0x2A),
];

pub(crate) fn item_of(e: &Element) -> Option<Item> {
    let v = e.attr("v");
    Some(match e.name.as_str() {
        "m" => Item::Missing,
        "n" => Item::Num(v?.trim().parse().ok()?),
        "b" => Item::Bool(crate::xml::parse_bool(v?)),
        "e" => Item::Err(v?.to_string()),
        "s" => Item::Str(v.unwrap_or("").to_string()),
        "d" => Item::Date(DateTime::parse(v?)?),
        _ => return None,
    })
}

fn item_record(w: &mut W, it: &Item) {
    match it {
        Item::Missing => w.e(20),
        Item::Num(n) => w.rec(21, &n.to_le_bytes()),
        Item::Bool(b) => w.rec(22, &[u8::from(*b)]),
        Item::Err(e) => {
            let code = ERRORS.iter().find(|x| x.0 == e).map_or(0x2A, |x| x.1);
            w.rec(23, &[code]);
        }
        Item::Str(s) => {
            let mut b = B::default();
            b.wstr(s);
            w.rec(24, &b.0);
        }
        Item::Date(d) => {
            let mut b = B::default();
            d.bytes(&mut b);
            w.rec(25, &b.0);
        }
    }
}

/// 항목 목록을 레코드로: 한셀은 수만 있는 공유 항목은 수 묶음(BrtBeginPCDIRun 0x0001), 날짜만이면 날짜 묶음(0x0020),
/// 묶음 항목(groupItems)의 글만 있거나 공유 항목의 글이 255개를 넘으면 글 묶음(0x0002), 나머지는 항목마다 레코드 하나를 쓴다
/// (피벗 실험 pa·pg·v_group·w_big)
fn items_records(w: &mut W, items: &[Item], string_runs: bool) {
    let all = |f: fn(&Item) -> bool| !items.is_empty() && items.iter().all(f);
    if all(|i| matches!(i, Item::Num(_))) {
        let mut b = B::default();
        b.u16(0x0001).u32(items.len() as u32);
        for i in items {
            if let Item::Num(n) = i {
                b.f64(*n);
            }
        }
        w.rec(191, &b.0);
        w.e(192);
    } else if all(|i| matches!(i, Item::Date(_))) {
        let mut b = B::default();
        b.u16(0x0020).u32(items.len() as u32);
        for i in items {
            if let Item::Date(d) = i {
                d.bytes(&mut b);
            }
        }
        w.rec(191, &b.0);
        w.e(192);
    } else if string_runs && all(|i| matches!(i, Item::Str(_))) {
        let mut b = B::default();
        b.u16(0x0002).u32(items.len() as u32);
        for i in items {
            if let Item::Str(s) = i {
                b.wstr(s);
            }
        }
        w.rec(191, &b.0);
        w.e(192);
    } else {
        for i in items {
            item_record(w, i);
        }
    }
}

/// 공유 항목 속성의 플래그 (BrtBeginPCDFAtbl): 0x01 containsSemiMixedTypes, 0x02 containsNonDate, 0x04 containsDate,
/// 0x08 containsString, 0x10 containsBlank, 0x20 containsMixedTypes, 0x40 containsNumber, 0x80 containsInteger,
/// 0x100 최소·최대 있음, 0x200 longText
const ATBL_FLAGS: &[Flag] = &[
    ("containsSemiMixedTypes", true, 0, false),
    ("containsNonDate", true, 1, false),
    ("containsDate", false, 2, false),
    ("containsString", true, 3, false),
    ("containsBlank", false, 4, false),
    ("containsMixedTypes", false, 5, false),
    ("containsNumber", false, 6, false),
    ("containsInteger", false, 7, false),
    ("longText", false, 9, false),
];

/// 계산 필드 수식의 이름 풀이: 캐시 필드 이름 → 이름 목록 번호 (처음 쓰인 순서)
pub(crate) struct PivotNames<'a> {
    pub(crate) fields: &'a [&'a str],
    /// 목록: 캐시 필드 번호 (ifdb)
    pub(crate) used: Vec<u32>,
    /// 필드가 아닌 이름 (계산 필드 수식에 쓸 수 없다)
    pub(crate) unknown: Option<String>,
}

impl super::formula::Resolver for PivotNames<'_> {
    fn xti(&mut self, _book: Option<usize>, _first: &str, _last: Option<&str>) -> Option<u16> {
        None
    }
    fn name(&mut self, _sheet: Option<&str>, name: &str) -> u32 {
        self.unknown.get_or_insert_with(|| name.to_string());
        0
    }
    fn extern_name(&mut self, _book: usize, _name: &str) -> Option<(u16, u32)> {
        None
    }
    fn pivot_field(&mut self, name: &str) -> Option<u32> {
        let ifdb = self
            .fields
            .iter()
            .position(|f| f.to_lowercase() == name.to_lowercase())? as u32;
        Some(match self.used.iter().position(|&x| x == ifdb) {
            Some(i) => i as u32,
            None => {
                self.used.push(ifdb);
                (self.used.len() - 1) as u32
            }
        })
    }
}

/// 캐시 정의 XML (+ 레코드 XML) → (정의 레코드 파트, 레코드 레코드 파트). `records_rel` 은 정의 파트에서 레코드 파트로
/// 가는 관계 ID
pub(crate) fn cache_to_bin(
    def_xml: &str,
    records_xml: Option<&str>,
    records_rel: Option<&str>,
    exact: bool,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let root = crate::xml::parse(def_xml, "pivotCacheDefinition").map_err(|e| e.to_string())?;
    let rec_root = match records_xml {
        Some(x) => Some(crate::xml::parse(x, "pivotCacheRecords").map_err(|e| e.to_string())?),
        None => None,
    };
    let record_count = rec_root
        .as_ref()
        .map(|r| r.sml_children("r").count() as u32)
        .unwrap_or_else(|| au(&root, "recordCount", 0));
    let mut w = W::default();
    // BrtBeginPivotCacheDef: 판 셋은 한셀처럼 3, 플래그, 남길 항목 수, 새로 고친 날짜, 레코드 수, 레코드 파트 관계 ID.
    // 한셀은 refreshedBy(stRefreshedWho)와 끝의 쓰이지 않는 4바이트를 쓰지 않는다 (피벗 실험 v_cache)
    let flags = u8::from(ab(&root, "saveData", true))
        | (u8::from(ab(&root, "invalid", false)) << 1)
        | (u8::from(ab(&root, "refreshOnLoad", false)) << 2)
        | (u8::from(ab(&root, "optimizeMemory", false)) << 3)
        | (u8::from(ab(&root, "enableRefresh", true)) << 4)
        | (u8::from(ab(&root, "backgroundQuery", false)) << 5)
        | (u8::from(ab(&root, "upgradeOnRefresh", false)) << 6)
        | (u8::from(ab(&root, "tupleCache", false)) << 7);
    let rel = records_rel.filter(|_| rec_root.is_some());
    let flags2 = (u8::from(rel.is_some()) << 1)
        | (u8::from(ab(&root, "supportSubquery", false)) << 2)
        | (u8::from(ab(&root, "supportAdvancedDrill", false)) << 3);
    let mut b = B::default();
    b.u8(3)
        .u8(3)
        .u8(3)
        .u8(flags)
        .i32(ai(&root, "missingItemsLimit", -1))
        .f64(root.attr_f64("refreshedDate").unwrap_or(0.0))
        .u8(flags2)
        .u32(record_count);
    if let Some(r) = rel {
        b.wstr(r);
    }
    w.rec(179, &b.0);
    // 원본
    if let Some(src) = root.sml_child("cacheSource") {
        let ty = match src.attr("type") {
            Some("external") => 1,
            Some("consolidation") => 2,
            Some("scenario") => 3,
            _ => 0,
        };
        let mut b = B::default();
        b.u32(ty).u32(au(src, "connectionId", 0));
        w.rec(185, &b.0);
        if let Some(ws) = src.sml_child("worksheetSource") {
            let name = ws.attr("name").filter(|s| !s.is_empty());
            let sheet = ws.attr("sheet").filter(|s| !s.is_empty());
            let rid = ws.rel_id();
            let mut b = B::default();
            b.u8(u8::from(name.is_some()))
                .u8(u8::from(name.is_some_and(|n| n.starts_with("_xlnm."))))
                .u8(u8::from(rid.is_some()) | (u8::from(sheet.is_some()) << 1));
            if let Some(s) = sheet {
                b.wstr(s);
            }
            if let Some(r) = rid {
                b.wstr(r);
            }
            match name {
                Some(n) => {
                    b.wstr(n.strip_prefix("_xlnm.").unwrap_or(n));
                }
                None => {
                    let r = ws
                        .attr("ref")
                        .and_then(crate::model::CellRange::parse)
                        .ok_or("피벗 캐시 원본 범위가 없음")?;
                    b.u32(r.start.row)
                        .u32(r.end.row)
                        .u32(r.start.col)
                        .u32(r.end.col);
                }
            }
            w.rec(187, &b.0);
            w.e(188);
        }
        w.e(186);
    }
    // 필드
    let fields: Vec<&Element> = root
        .sml_child("cacheFields")
        .map(|c| c.sml_children("cacheField").collect())
        .unwrap_or_default();
    let mut kinds = Vec::new();
    w.rec(181, &(fields.len() as u32).to_le_bytes());
    for f in &fields {
        // 한셀은 캐시 필드의 캡션을 바이너리에 쓰지 않는다 (피벗 실험 v_fcap)
        let caption = f.attr("caption").filter(|_| !exact);
        let formula = f.attr("formula");
        let name = f.attr("name").unwrap_or("");
        // BrtBeginPCDField: 0x04 원본 필드, 0x08 캡션 있음, 0x100 계산 필드 수식 있음
        let mut b = B::default();
        b.u16(
            (u16::from(ab(f, "databaseField", true)) << 2)
                | (u16::from(caption.is_some()) << 3)
                | (u16::from(formula.is_some()) << 8),
        )
        .u32(au(f, "numFmtId", 0))
        .u16(au(f, "sqlType", 0) as u16)
        .u32(au(f, "hierarchy", 0))
        .u32(au(f, "level", 0))
        .u32(0)
        .wstr(name);
        if let Some(c) = caption {
            b.wstr(c);
        }
        // 계산 필드: 수식 (cce u32, 토큰열, cb u32, 덧붙은 데이터) 뒤에 수식이 쓰는 캐시 필드 목록(BrtBeginPNames:
        // 필드마다 BrtBeginPName — ifdb u32, 함수 0xFF 없음, 플래그 0). 필드 이름은 PtgSxName 으로 목록 번호를
        // 가리킨다 (피벗 실험 v_calc: `금액*2` → 18 1D 00000000 1E 0200 05, 목록 [3])
        let mut pnames = Vec::new();
        if let Some(text) = formula {
            let names: Vec<&str> = fields
                .iter()
                .map(|x| x.attr("name").unwrap_or(""))
                .collect();
            let mut r = PivotNames {
                fields: &names,
                used: Vec::new(),
                unknown: None,
            };
            let enc = super::formula::encode(text, super::formula::Place::Cell, &mut r)
                .map_err(|e| format!("계산 필드 {name:?} 의 수식 {text:?}: {e}"))?;
            if let Some(u) = r.unknown {
                return Err(format!(
                    "계산 필드 {name:?} 의 수식 {text:?}: 알 수 없는 이름 {u:?}"
                ));
            }
            b.u32(enc.rgce.len() as u32);
            b.0.extend_from_slice(&enc.rgce);
            b.u32(enc.rgcb.len() as u32);
            b.0.extend_from_slice(&enc.rgcb);
            pnames = r.used;
        }
        w.rec(183, &b.0);
        if formula.is_some() {
            w.rec(253, &(pnames.len() as u32).to_le_bytes());
            for ifdb in pnames {
                let mut b = B::default();
                b.u32(ifdb).u8(0xFF).u8(0);
                w.rec(255, &b.0);
                w.e(256);
            }
            w.e(254);
        }
        let mut kind = FieldKind::default();
        if let Some(si) = f.sml_child("sharedItems") {
            let items: Vec<Item> = si.elements().filter_map(item_of).collect();
            let mut fl = flags_from(si, ATBL_FLAGS);
            let date = ab(si, "containsDate", false);
            // 한셀은 날짜만 있는 필드에도 0x02 를 켠다 (피벗 실험 pg)
            if date {
                fl |= 0x02;
            }
            let runs = !items.is_empty()
                && (items.iter().all(|i| matches!(i, Item::Num(_)))
                    || items.iter().all(|i| matches!(i, Item::Date(_))));
            let first = items.first().and_then(|i| match i {
                Item::Num(n) => Some(*n),
                Item::Date(d) => Some(d.serial()),
                _ => None,
            });
            // 최소·최대: 묶음으로 쓰는 필드는 한셀처럼 둘 다 첫 값 (한셀 바이너리의 꼴 — 다시 읽을 때 한셀이 셈한다)
            let (min, max) = if runs {
                (first, first)
            } else if date {
                (
                    si.attr("minDate")
                        .and_then(DateTime::parse)
                        .map(|d| d.serial()),
                    si.attr("maxDate")
                        .and_then(DateTime::parse)
                        .map(|d| d.serial()),
                )
            } else {
                (si.attr_f64("minValue"), si.attr_f64("maxValue"))
            };
            let minmax = si.attr("minValue").is_some() || si.attr("minDate").is_some();
            if minmax {
                fl |= 0x100;
            }
            let mut b = B::default();
            b.u16(fl as u16).u32(items.len() as u32);
            if minmax {
                b.f64(min.unwrap_or(0.0)).f64(max.unwrap_or(0.0));
            }
            w.rec(189, &b.0);
            // 글만 있는 공유 항목은 255개를 넘으면 글 묶음 (피벗 실험 w_big: 300개는 묶음, 3개는 하나씩)
            items_records(&mut w, &items, items.len() > 255);
            w.e(190);
            kind = FieldKind {
                items: !items.is_empty(),
                num: fl & 0x40 != 0,
                date,
                text: fl & 0x08 != 0,
                ..kind
            };
        }
        kinds.push(FieldKind {
            db: ab(f, "databaseField", true),
            calc: formula.is_some(),
            ..kind
        });
        if let Some(g) = f.sml_child("fieldGroup") {
            let mut b = B::default();
            b.i32(ai(g, "par", -1)).i32(ai(g, "base", -1));
            w.rec(219, &b.0);
            if let Some(r) = g.sml_child("rangePr") {
                const BY: [&str; 8] = [
                    "range", "seconds", "minutes", "hours", "days", "months", "quarters", "years",
                ];
                let by = index_of(&BY, r.attr("groupBy"), 0) as u8;
                let dates = by > 0;
                let val = |n: &str, d: &str| -> f64 {
                    if dates {
                        r.attr(d)
                            .and_then(DateTime::parse)
                            .map_or(0.0, |x| x.serial())
                    } else {
                        r.attr_f64(n).unwrap_or(0.0)
                    }
                };
                let mut b = B::default();
                b.u8(by)
                    .u8(u8::from(ab(r, "autoStart", true))
                        | (u8::from(ab(r, "autoEnd", true)) << 1)
                        | (u8::from(dates) << 2))
                    .f64(val("startNum", "startDate"))
                    .f64(val("endNum", "endDate"))
                    .f64(r.attr_f64("groupInterval").unwrap_or(1.0));
                w.rec(223, &b.0);
                w.e(224);
            }
            if let Some(gi) = g.sml_child("groupItems") {
                let items: Vec<Item> = gi.elements().filter_map(item_of).collect();
                w.rec(221, &(items.len() as u32).to_le_bytes());
                items_records(&mut w, &items, true);
                w.e(222);
            }
            w.e(220);
        }
        w.e(184);
    }
    w.e(182);
    w.e(180);
    // 레코드
    let records = rec_root.map(|r| {
        let mut w = W::default();
        w.rec(193, &record_count.to_le_bytes());
        // 레코드에는 원본 필드만 (계산 필드는 값이 없다)
        let db: Vec<FieldKind> = kinds.iter().copied().filter(|k| k.db).collect();
        for row in r.sml_children("r") {
            let mut b = B::default();
            for (k, v) in row.elements().enumerate() {
                let kind = db.get(k).copied().unwrap_or_default();
                match (v.name.as_str(), kind.items) {
                    ("x", _) => {
                        b.u32(au(v, "v", 0));
                    }
                    ("n", false) => {
                        b.f64(v.attr_f64("v").unwrap_or(0.0));
                    }
                    ("d", false) => {
                        if let Some(d) = v.attr("v").and_then(DateTime::parse) {
                            d.bytes(&mut b);
                        }
                    }
                    (_, _) => {
                        b.wstr(v.attr("v").unwrap_or(""));
                    }
                }
            }
            w.rec(33, &b.0);
        }
        w.e(194);
        w.0
    });
    Ok((w.0, records))
}

// ---------------------------------------------------------------------------
// 레코드 → XML (바이너리 읽기)
// ---------------------------------------------------------------------------

const NS_DECL: &str = "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>";

fn b01(v: bool) -> &'static str {
    if v { "1" } else { "0" }
}

/// 피벗 표 레코드 파트 → 한셀이 쓰는 꼴의 `pivotTableDefinition` XML. 한셀의 바이너리 읽기처럼 x14 확장
/// (`hideValuesRow="1" altText=""`)을 늘 붙인다 (피벗 실험: 확장이 없는 원본의 바이너리도 .xlsb 읽기 결과에는 있다)
/// `include_new` 는 레거시 읽기만 쓴다: 필드마다 `includeNewItemsInFilter` (한셀 바이너리에는 이 값의 자리가 없다 — fFilterInclusive
/// 비트는 한셀이 늘 켜고 읽을 때 쓰지 않는다)
pub(crate) fn table_to_xml(data: &[u8], include_new: Option<&[bool]>) -> Option<String> {
    let recs = super::biff12::records(data);
    let view = recs.iter().find(|r| r.rtype == 280)?;
    let mut r = Rd::new(view.data);
    let _ver = r.u8()?;
    let f1 = u32::from(r.u16()?);
    let b3 = r.u8()?;
    let f2 = r.u32()?;
    let f3 = r.u32()?;
    let data_axis = r.u8()?;
    let wrap = r.u8()?;
    let _last = r.u8()?;
    let _min = r.u8()?;
    let ipos = r.u32()?;
    let auto_fmt = r.u16()?;
    let _ = r.u16()?;
    let chart = r.u32()?;
    let cache_id = r.u32()?;
    let name = r.wstr()?;
    let data_caption = if f2 & (1 << 19) != 0 {
        r.wstr()?
    } else {
        String::new()
    };
    let mut opt = |present: bool| -> Option<String> { if present { r.wstr() } else { None } };
    let grand = opt(f2 & (1 << 20) != 0);
    let error = opt(f3 & (1 << 6) == 0);
    let missing = opt(f3 & (1 << 7) == 0);
    let page_style = opt(f2 & (1 << 21) != 0);
    let _table_style = opt(f2 & (1 << 22) != 0);
    let vacated = opt(f2 & (1 << 23) != 0);
    let tag = opt(f2 & (1 << 30) != 0);
    let col_hdr = opt(f3 & (1 << 11) != 0);
    let row_hdr = opt(f3 & (1 << 10) != 0);

    let mut s = format!("{DECL}<x:pivotTableDefinition {NS_DECL}");
    let mut attr = |n: &str, v: &str| {
        let _ = write!(s, " {n}=\"{}\"", esc(v));
    };
    attr("name", &name);
    attr("cacheId", &cache_id.to_string());
    attr("dataOnRows", b01(data_axis == 1));
    if ipos != 0xFFFF {
        attr("dataPosition", &ipos.to_string());
    }
    if auto_fmt != 0 {
        attr("autoFormatId", &auto_fmt.to_string());
    }
    for (n, bit) in [
        ("applyNumberFormats", 24),
        ("applyBorderFormats", 27),
        ("applyFontFormats", 25),
        ("applyPatternFormats", 28),
        ("applyAlignmentFormats", 26),
    ] {
        if f2 & (1 << bit) != 0 {
            attr(n, "1");
        }
    }
    attr("applyWidthHeightFormats", b01(f2 & (1 << 29) != 0));
    attr("dataCaption", &data_caption);
    if let Some(g) = &grand {
        attr("grandTotalCaption", g);
    }
    if let Some(e) = &error {
        attr("errorCaption", e);
    }
    attr("showError", b01(flag_value(f2, VIEW_FLAGS2, "showError")));
    if let Some(m) = &missing {
        attr("missingCaption", m);
    }
    attr(
        "showMissing",
        b01(flag_value(f2, VIEW_FLAGS2, "showMissing")),
    );
    if let Some(p) = &page_style {
        attr("pageStyle", p);
    }
    if let Some(v) = &vacated {
        attr("vacatedStyle", v);
    }
    if let Some(t) = &tag {
        attr("tag", t);
    }
    attr("updatedVersion", "4");
    for n in [
        "asteriskTotals",
        "showItems",
        "editData",
        "disableFieldList",
        "showCalcMbrs",
        "visualTotals",
        "showMultipleLabel",
        "showDataDropDown",
        "showDrill",
        "printDrill",
        "showMemberPropertyTips",
        "showDataTips",
        "enableWizard",
        "enableDrill",
        "enableFieldProperties",
        "preserveFormatting",
        "useAutoFormatting",
    ] {
        let v = flag_value(f1, VIEW_FLAGS1, n) || flag_value(f2, VIEW_FLAGS2, n);
        attr(n, b01(v));
    }
    attr("pageWrap", &wrap.to_string());
    for n in [
        "pageOverThenDown",
        "subtotalHiddenItems",
        "rowGrandTotals",
        "colGrandTotals",
        "fieldPrintTitles",
        "itemPrintTitles",
        "mergeItem",
        "showDropZones",
    ] {
        attr(n, b01(flag_value(f2, VIEW_FLAGS2, n)));
    }
    attr("createdVersion", "4");
    attr("indent", &(b3 & 0x7F).saturating_sub(1).to_string());
    attr(
        "showEmptyRow",
        b01(flag_value(f2, VIEW_FLAGS2, "showEmptyRow")),
    );
    attr(
        "showEmptyCol",
        b01(flag_value(f2, VIEW_FLAGS2, "showEmptyCol")),
    );
    attr("showHeaders", b01(b3 & 0x80 == 0));
    for n in [
        "compact",
        "outline",
        "outlineData",
        "compactData",
        "published",
        "gridDropZones",
        "immersive",
        "multipleFieldFilters",
    ] {
        attr(n, b01(flag_value(f3, VIEW_FLAGS3, n)));
    }
    attr("chartFormat", &chart.to_string());
    if let Some(h) = &row_hdr {
        attr("rowHeaderCaption", h);
    }
    if let Some(h) = &col_hdr {
        attr("colHeaderCaption", h);
    }
    attr(
        "fieldListSortAscending",
        b01(flag_value(f3, VIEW_FLAGS3, "fieldListSortAscending")),
    );
    s.push('>');

    // 나머지 레코드를 차례로
    let mut i = 0;
    let mut out_fields: Vec<String> = Vec::new();
    let lines = |tag: &str, begin: usize, s: &mut String| -> usize {
        // begin: SXLIRws/SXLICols 의 위치 → 끝 레코드 다음 위치
        let count = recs[begin]
            .data
            .get(0..4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let _ = write!(s, "<x:{tag} count=\"{count}\">");
        let mut k = begin + 1;
        while k < recs.len() && matches!(recs[k].rtype, 297 | 298 | 388 | 389) {
            if recs[k].rtype == 297 {
                let mut r = Rd::new(recs[k].data);
                let csic = r.u16().unwrap_or(0);
                let t = r.u8().unwrap_or(0);
                let _ = r.u8();
                let n = r.u32().unwrap_or(0);
                let idata = r.u32().unwrap_or(0);
                let mut xs = Vec::new();
                if n > 0 && recs.get(k + 1).is_some_and(|x| x.rtype == 388) {
                    let mut rr = Rd::new(recs[k + 1].data);
                    for _ in 0..n {
                        xs.push(rr.u32().unwrap_or(0));
                    }
                }
                let _ = write!(s, "<x:i");
                if t != 0 {
                    let _ = write!(
                        s,
                        " t=\"{}\"",
                        ITEM_TYPES.get(t as usize).unwrap_or(&"data")
                    );
                }
                if csic != 0 {
                    let _ = write!(s, " r=\"{csic}\"");
                }
                if idata != 0 {
                    let _ = write!(s, " i=\"{idata}\"");
                }
                if xs.is_empty() {
                    s.push_str("/>");
                } else {
                    s.push('>');
                    for x in xs {
                        let _ = write!(s, "<x:x v=\"{x}\"/>");
                    }
                    s.push_str("</x:i>");
                }
            }
            k += 1;
        }
        let _ = write!(s, "</x:{tag}>");
        k + 1
    };
    let mut location = String::new();
    let mut row_fields = String::new();
    let mut row_items = String::new();
    let mut col_fields = String::new();
    let mut col_items = String::new();
    let mut page_fields = String::new();
    let mut data_fields = String::new();
    let mut style = String::new();
    while i < recs.len() {
        let rec = &recs[i];
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            314 => {
                let (r1, r2, c1, c2) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let (h, d, dc, rp, cp) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let range = crate::model::CellRange {
                    start: crate::model::CellRef { row: r1, col: c1 },
                    end: crate::model::CellRef { row: r2, col: c2 },
                };
                let _ = write!(
                    location,
                    "<x:location ref=\"{range}\" firstHeaderRow=\"{}\" firstDataRow=\"{}\" firstDataCol=\"{}\" rowPageCount=\"{rp}\" colPageCount=\"{cp}\"/>",
                    h.saturating_sub(r1),
                    d.saturating_sub(r1),
                    dc.saturating_sub(c1)
                );
            }
            285 => {
                let axis = r.u8()?;
                let lo = r.u8()?;
                let mid = r.u8()?;
                let hi = r.u8()?;
                let ff1 = u32::from(lo) | (u32::from(mid) << 8) | (u32::from(hi) << 16);
                let ifmt = r.u32()?;
                let ff2 = r.u32()?;
                let citm = r.i32()?;
                let rank = r.i32()?;
                let fname = if ff1 & (1 << 21) != 0 { r.wstr() } else { None };
                let sub = if ff1 & (1 << 22) != 0 { r.wstr() } else { None };
                let mut f = String::from("<x:pivotField");
                if let Some(n) = &fname {
                    let _ = write!(f, " name=\"{}\"", esc(n));
                }
                match axis & 0x07 {
                    0x01 => f.push_str(" axis=\"axisRow\""),
                    0x02 => f.push_str(" axis=\"axisCol\""),
                    0x04 => f.push_str(" axis=\"axisPage\""),
                    _ => {}
                }
                if axis & 0x08 != 0 {
                    f.push_str(" dataField=\"1\"");
                }
                if let Some(c) = &sub {
                    let _ = write!(f, " subtotalCaption=\"{}\"", esc(c));
                }
                if !flag_value(ff1, VD_FLAGS1, "showDropDowns") {
                    f.push_str(" showDropDowns=\"0\"");
                }
                if !flag_value(ff1, VD_FLAGS1, "compact") {
                    f.push_str(" compact=\"0\"");
                }
                if ifmt != 0 {
                    let _ = write!(f, " numFmtId=\"{ifmt}\"");
                }
                if !flag_value(ff2, VD_FLAGS2, "outline") {
                    f.push_str(" outline=\"0\"");
                }
                if !flag_value(ff2, VD_FLAGS2, "subtotalTop") {
                    f.push_str(" subtotalTop=\"0\"");
                }
                for n in ["dragToRow", "dragToCol"] {
                    if !flag_value(ff2, VD_FLAGS2, n) {
                        let _ = write!(f, " {n}=\"0\"");
                    }
                }
                if flag_value(ff2, VD_FLAGS2, "multipleItemSelectionAllowed") {
                    f.push_str(" multipleItemSelectionAllowed=\"1\"");
                }
                for n in ["dragToPage", "dragToData", "dragOff"] {
                    if !flag_value(ff2, VD_FLAGS2, n) {
                        let _ = write!(f, " {n}=\"0\"");
                    }
                }
                if !flag_value(ff2, VD_FLAGS2, "showAll") {
                    f.push_str(" showAll=\"0\"");
                }
                for n in [
                    "insertBlankRow",
                    "serverField",
                    "insertPageBreak",
                    "autoShow",
                ] {
                    if flag_value(ff2, VD_FLAGS2, n) {
                        let _ = write!(f, " {n}=\"1\"");
                    }
                }
                if !flag_value(ff2, VD_FLAGS2, "topAutoShow") {
                    f.push_str(" topAutoShow=\"0\"");
                }
                if flag_value(ff2, VD_FLAGS2, "hideNewItems") {
                    f.push_str(" hideNewItems=\"1\"");
                }
                if flag_value(ff2, VD_FLAGS2, "measureFilter") {
                    f.push_str(" measureFilter=\"1\"");
                }
                if include_new.is_some_and(|v| v.get(out_fields.len()) == Some(&true)) {
                    f.push_str(" includeNewItemsInFilter=\"1\"");
                }
                if citm != 10 {
                    let _ = write!(f, " itemPageCount=\"{citm}\"");
                }
                match (ff2 & (1 << 12) != 0, ff2 & (1 << 13) != 0) {
                    (true, true) => f.push_str(" sortType=\"ascending\""),
                    (true, false) => f.push_str(" sortType=\"descending\""),
                    _ => {}
                }
                if flag_value(ff2, VD_FLAGS2, "nonAutoSortDefault") {
                    f.push_str(" nonAutoSortDefault=\"1\"");
                }
                if rank != -1 {
                    let _ = write!(f, " rankBy=\"{rank}\"");
                }
                if !flag_value(ff1, VD_FLAGS1, "defaultSubtotal") {
                    f.push_str(" defaultSubtotal=\"0\"");
                }
                for n in [
                    "sumSubtotal",
                    "countASubtotal",
                    "avgSubtotal",
                    "maxSubtotal",
                    "minSubtotal",
                    "productSubtotal",
                    "countSubtotal",
                    "stdDevSubtotal",
                    "stdDevPSubtotal",
                    "varSubtotal",
                    "varPSubtotal",
                ] {
                    if flag_value(ff1, VD_FLAGS1, n) {
                        let _ = write!(f, " {n}=\"1\"");
                    }
                }
                // 항목
                let mut k = i + 1;
                if recs.get(k).is_some_and(|x| x.rtype == 283) {
                    let n = recs[k]
                        .data
                        .get(0..4)
                        .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    let _ = write!(f, "><x:items count=\"{n}\">");
                    k += 1;
                    while k < recs.len() && matches!(recs[k].rtype, 281 | 282) {
                        if recs[k].rtype == 282 {
                            let mut r = Rd::new(recs[k].data);
                            let t = r.u8().unwrap_or(0);
                            let fl = r.u16().unwrap_or(0);
                            let x = r.i32().unwrap_or(-1);
                            let n = if fl & 0x10 != 0 { r.wstr() } else { None };
                            f.push_str("<x:item");
                            if let Some(n) = &n {
                                let _ = write!(f, " n=\"{}\"", esc(n));
                            }
                            if t != 0 {
                                let _ = write!(
                                    f,
                                    " t=\"{}\"",
                                    ITEM_TYPES.get(t as usize).unwrap_or(&"data")
                                );
                            }
                            if fl & 0x01 != 0 {
                                f.push_str(" h=\"1\"");
                            }
                            if fl & 0x02 != 0 {
                                f.push_str(" sd=\"0\"");
                            }
                            if fl & 0x04 != 0 {
                                f.push_str(" f=\"1\"");
                            }
                            if fl & 0x08 != 0 {
                                f.push_str(" m=\"1\"");
                            }
                            if fl & 0x40 != 0 {
                                f.push_str(" c=\"1\"");
                            }
                            if t == 0 {
                                let _ = write!(f, " x=\"{x}\"");
                            }
                            if fl & 0x20 != 0 {
                                f.push_str(" d=\"1\"");
                            }
                            f.push_str("/>");
                        }
                        k += 1;
                    }
                    f.push_str("</x:items></x:pivotField>");
                    i = k; // 284 에서 계속
                } else {
                    f.push_str("/>");
                }
                out_fields.push(f);
            }
            309 | 311 => {
                let n = r.u32()?;
                let mut s = String::new();
                let tag = if rec.rtype == 309 {
                    "rowFields"
                } else {
                    "colFields"
                };
                let _ = write!(s, "<x:{tag} count=\"{n}\">");
                for _ in 0..n {
                    let _ = write!(s, "<x:field x=\"{}\"/>", r.i32()?);
                }
                let _ = write!(s, "</x:{tag}>");
                if rec.rtype == 309 {
                    row_fields = s;
                } else {
                    col_fields = s;
                }
            }
            299 => {
                let mut s = String::new();
                i = lines("rowItems", i, &mut s) - 1;
                row_items = s;
            }
            301 => {
                let mut s = String::new();
                i = lines("colItems", i, &mut s) - 1;
                col_items = s;
            }
            291 => {
                let _ = write!(page_fields, "<x:pageFields count=\"{}\">", r.u32()?);
            }
            289 => {
                let fld = r.u32()?;
                let item = r.u32()?;
                let hier = r.i32()?;
                let fl = r.u8()?;
                let name = if fl & 0x01 != 0 { r.wstr() } else { None };
                let cap = if fl & 0x02 != 0 { r.wstr() } else { None };
                let _ = write!(page_fields, "<x:pageField fld=\"{fld}\"");
                if item != 0x0010_00FE {
                    let _ = write!(page_fields, " item=\"{item}\"");
                }
                let _ = write!(page_fields, " hier=\"{hier}\"");
                if let Some(n) = name {
                    let _ = write!(page_fields, " name=\"{}\"", esc(&n));
                }
                if let Some(c) = cap {
                    let _ = write!(page_fields, " cap=\"{}\"", esc(&c));
                }
                page_fields.push_str("/>");
            }
            292 => page_fields.push_str("</x:pageFields>"),
            295 => data_fields.push_str("<x:dataFields>"),
            293 => {
                let fld = r.u32()?;
                let sub = r.u32()?;
                let df = r.u32()?;
                let base_field = r.i32()?;
                let base_item = r.u32()?;
                let fmt = r.u32()?;
                let has_name = r.u8()? != 0;
                let name = if has_name { r.wstr() } else { None };
                data_fields.push_str("<x:dataField");
                if let Some(n) = name {
                    let _ = write!(data_fields, " name=\"{}\"", esc(&n));
                }
                let _ = write!(
                    data_fields,
                    " fld=\"{fld}\" subtotal=\"{}\" showDataAs=\"{}\" baseField=\"{base_field}\" baseItem=\"{base_item}\"",
                    SUBTOTALS.get(sub as usize).unwrap_or(&"sum"),
                    SHOW_DATA_AS.get(df as usize).unwrap_or(&"normal")
                );
                if fmt != 0 {
                    let _ = write!(data_fields, " numFmtId=\"{fmt}\"");
                }
                data_fields.push_str("/>");
            }
            296 => data_fields.push_str("</x:dataFields>"),
            513 => {
                let fl = r.u16()?;
                let n = r.nwstr()?;
                style.push_str("<x:pivotTableStyleInfo");
                if let Some(n) = n {
                    let _ = write!(style, " name=\"{}\"", esc(&n));
                }
                let _ = write!(
                    style,
                    " showRowHeaders=\"{}\" showColHeaders=\"{}\" showRowStripes=\"{}\" showColStripes=\"{}\" showLastColumn=\"{}\"/>",
                    b01(fl & 0x10 != 0),
                    b01(fl & 0x20 != 0),
                    b01(fl & 0x04 != 0),
                    b01(fl & 0x08 != 0),
                    b01(fl & 0x02 != 0)
                );
            }
            _ => {}
        }
        i += 1;
    }
    s.push_str(&location);
    let _ = write!(s, "<x:pivotFields count=\"{}\">", out_fields.len());
    for f in out_fields {
        s.push_str(&f);
    }
    s.push_str("</x:pivotFields>");
    s.push_str(&row_fields);
    s.push_str(&row_items);
    s.push_str(&col_fields);
    s.push_str(&col_items);
    s.push_str(&page_fields);
    s.push_str(&data_fields);
    s.push_str(&style);
    s.push_str("<x:extLst><x:ext uri=\"{962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:pivotTableDefinition xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\" hideValuesRow=\"1\" altText=\"\"/></x:ext></x:extLst>");
    s.push_str("</x:pivotTableDefinition>");
    Some(s)
}

fn item_xml(it: &Item) -> String {
    match it {
        Item::Missing => "<x:m/>".into(),
        Item::Num(n) => format!("<x:n v=\"{}\"/>", fmt_num(*n)),
        Item::Bool(b) => format!("<x:b v=\"{}\"/>", b01(*b)),
        Item::Err(e) => format!("<x:e v=\"{}\"/>", esc(e)),
        Item::Str(s) => format!("<x:s v=\"{}\"/>", esc(s)),
        Item::Date(d) => format!("<x:d v=\"{}\"/>", d.xml()),
    }
}

/// 항목 레코드(20–25)와 묶음(191…192)을 읽는다. `k` 는 첫 레코드 위치, 돌려주는 값은 다음 위치
fn read_items(recs: &[super::biff12::Record], mut k: usize, out: &mut Vec<Item>) -> usize {
    while let Some(rec) = recs.get(k) {
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            20 => out.push(Item::Missing),
            21 => out.push(Item::Num(r.f64().unwrap_or(0.0))),
            22 => out.push(Item::Bool(r.u8().unwrap_or(0) != 0)),
            23 => {
                let c = r.u8().unwrap_or(0x2A);
                out.push(Item::Err(
                    ERRORS
                        .iter()
                        .find(|x| x.1 == c)
                        .map_or("#N/A", |x| x.0)
                        .to_string(),
                ));
            }
            24 => out.push(Item::Str(r.wstr().unwrap_or_default())),
            25 => {
                if let Some(d) = DateTime::read(&mut r) {
                    out.push(Item::Date(d));
                }
            }
            191 => {
                let ty = r.u16().unwrap_or(0);
                let n = r.u32().unwrap_or(0);
                for _ in 0..n {
                    let it = match ty {
                        0x0001 => r.f64().map(Item::Num),
                        0x0002 => r.wstr().map(Item::Str),
                        0x0010 => r.u8().map(|c| {
                            Item::Err(
                                ERRORS
                                    .iter()
                                    .find(|x| x.1 == c)
                                    .map_or("#N/A", |x| x.0)
                                    .to_string(),
                            )
                        }),
                        0x0020 => DateTime::read(&mut r).map(Item::Date),
                        _ => None,
                    };
                    match it {
                        Some(i) => out.push(i),
                        None => break,
                    }
                }
            }
            192 => {}
            _ => return k,
        }
        k += 1;
    }
    k
}

/// 캐시 정의 레코드 파트 → (`pivotCacheDefinition` XML, 레코드 파트로 가는 관계 ID, 필드마다 값 꼴). 한셀의 바이너리 읽기처럼
/// `recordCount` 를 쓰고 최소·최대는 항목에서 다시 셈한다 (한셀 바이너리의 묶음 필드는 최소·최대가 첫 값이다)
fn cache_def_to_xml(data: &[u8]) -> Option<(String, Option<String>, Vec<FieldKind>)> {
    let recs = super::biff12::records(data);
    // 캐시 필드 이름 (계산 필드 수식의 이름 목록이 번호로 가리킨다)
    let all_names: Vec<String> = recs
        .iter()
        .filter(|r| r.rtype == 183)
        .map(|rec| {
            let mut r = Rd::new(rec.data);
            r.take(16);
            let n = r.u32().unwrap_or(0);
            r.take(n as usize * 4);
            r.wstr().unwrap_or_default()
        })
        .collect();
    let def = recs.iter().find(|r| r.rtype == 179)?;
    let mut r = Rd::new(def.data);
    let (_last, _min, _created) = (r.u8()?, r.u8()?, r.u8()?);
    let fl = r.u8()?;
    let ghost = r.i32()?;
    let refreshed = r.f64()?;
    let fl2 = r.u8()?;
    let count = r.u32()?;
    let who = if fl2 & 0x01 != 0 { r.wstr() } else { None };
    let rel = if fl2 & 0x02 != 0 { r.wstr() } else { None };
    let mut s = format!("{DECL}<x:pivotCacheDefinition {NS_DECL}");
    if rel.is_some() {
        s.push_str(" r:id=\"rId1\"");
    }
    if fl & 0x02 != 0 {
        s.push_str(" invalid=\"1\"");
    }
    if fl & 0x01 == 0 {
        s.push_str(" saveData=\"0\"");
    }
    if fl & 0x04 != 0 {
        s.push_str(" refreshOnLoad=\"1\"");
    }
    if fl & 0x08 != 0 {
        s.push_str(" optimizeMemory=\"1\"");
    }
    if fl & 0x10 == 0 {
        s.push_str(" enableRefresh=\"0\"");
    }
    if let Some(w) = &who {
        let _ = write!(s, " refreshedBy=\"{}\"", esc(w));
    }
    if refreshed != 0.0 {
        let _ = write!(s, " refreshedDate=\"{}\"", num(refreshed));
    }
    if fl & 0x20 != 0 {
        s.push_str(" backgroundQuery=\"1\"");
    }
    if ghost != -1 {
        let _ = write!(s, " missingItemsLimit=\"{ghost}\"");
    }
    let _ = write!(s, " createdVersion=\"3\" recordCount=\"{count}\"");
    if fl & 0x40 != 0 {
        s.push_str(" upgradeOnRefresh=\"1\"");
    }
    if fl & 0x80 != 0 {
        s.push_str(" tupleCache=\"1\"");
    }
    if fl2 & 0x04 != 0 {
        s.push_str(" supportSubquery=\"1\"");
    }
    if fl2 & 0x08 != 0 {
        s.push_str(" supportAdvancedDrill=\"1\"");
    }
    s.push('>');
    let mut kinds = Vec::new();
    let mut fields = String::new();
    let mut nfields = 0;
    let mut k = 0;
    while k < recs.len() {
        let rec = &recs[k];
        let mut r = Rd::new(rec.data);
        match rec.rtype {
            185 => {
                let ty = r.u32()?;
                let conn = r.u32()?;
                let t = ["worksheet", "external", "consolidation", "scenario"]
                    .get(ty as usize)
                    .unwrap_or(&"worksheet");
                let _ = write!(s, "<x:cacheSource type=\"{t}\"");
                if conn != 0 {
                    let _ = write!(s, " connectionId=\"{conn}\"");
                }
                s.push('>');
            }
            187 => {
                let f_name = r.u8()? & 0x01 != 0;
                let builtin = r.u8()? & 0x01 != 0;
                let f3 = r.u8()?;
                let sheet = if f3 & 0x02 != 0 { r.wstr() } else { None };
                let rid = if f3 & 0x01 != 0 { r.wstr() } else { None };
                s.push_str("<x:worksheetSource");
                if f_name {
                    let n = r.wstr()?;
                    let n = if builtin { format!("_xlnm.{n}") } else { n };
                    let _ = write!(s, " name=\"{}\"", esc(&n));
                    // 한셀은 이름 원본에 빈 sheet 를 쓴다 (피벗 실험 v_srcname)
                    let _ = write!(s, " sheet=\"{}\"", esc(sheet.as_deref().unwrap_or("")));
                } else {
                    let (r1, r2, c1, c2) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                    let range = crate::model::CellRange {
                        start: crate::model::CellRef { row: r1, col: c1 },
                        end: crate::model::CellRef { row: r2, col: c2 },
                    };
                    let _ = write!(s, " ref=\"{range}\"");
                    if let Some(sh) = &sheet {
                        let _ = write!(s, " sheet=\"{}\"", esc(sh));
                    }
                }
                if let Some(r) = rid {
                    let _ = write!(s, " r:id=\"{}\"", esc(&r));
                }
                s.push_str("/>");
            }
            186 => s.push_str("</x:cacheSource>"),
            183 => {
                nfields += 1;
                let fl = r.u16()?;
                let ifmt = r.u32()?;
                let sql = r.u16()?;
                let hier = r.u32()?;
                let level = r.u32()?;
                let ntmp = r.u32()?;
                for _ in 0..ntmp {
                    r.u32()?;
                }
                let name = r.wstr()?;
                let caption = if fl & 0x08 != 0 { r.wstr() } else { None };
                // 계산 필드 수식 (0x100): cce u32, 토큰열, cb u32, 덧붙은 데이터. 필드 이름(PtgSxName)은 뒤따르는
                // BrtBeginPName 목록의 캐시 필드 번호로 찾는다
                let formula = if fl & 0x100 != 0 {
                    let cce = r.u32()? as usize;
                    let rgce = r.take(cce)?.to_vec();
                    let pnames: Vec<String> = recs[k + 1..]
                        .iter()
                        .take_while(|x| x.rtype != 184)
                        .filter(|x| x.rtype == 255)
                        .map(|x| {
                            let i = x
                                .data
                                .get(0..4)
                                .map_or(u32::MAX, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                            all_names.get(i as usize).cloned().unwrap_or_default()
                        })
                        .collect();
                    let ctx = crate::legacy::formula::Context {
                        xti_sheets: &[],
                        names: &pnames,
                        xti_names: &[],
                        base: crate::model::CellRef::default(),
                        xlsb: true,
                        row_shift: 0,
                        tables: &[],
                    };
                    match crate::legacy::formula::decode(&rgce, &ctx) {
                        Ok(crate::legacy::formula::Decoded::Text(t)) => Some(t),
                        _ => None,
                    }
                } else {
                    None
                };
                let _ = write!(fields, "<x:cacheField name=\"{}\"", esc(&name));
                if let Some(c) = caption {
                    let _ = write!(fields, " caption=\"{}\"", esc(&c));
                }
                let _ = write!(fields, " numFmtId=\"{ifmt}\"");
                if let Some(f) = &formula {
                    let _ = write!(fields, " formula=\"{}\"", esc(f));
                }
                if sql != 0 {
                    let _ = write!(fields, " sqlType=\"{sql}\"");
                }
                if hier != 0 {
                    let _ = write!(fields, " hierarchy=\"{hier}\"");
                }
                if level != 0 {
                    let _ = write!(fields, " level=\"{level}\"");
                }
                if fl & 0x04 == 0 {
                    fields.push_str(" databaseField=\"0\"");
                }
                // 공유 항목·묶음은 다음 레코드들에서
                let mut kind = FieldKind {
                    db: fl & 0x04 != 0,
                    calc: formula.is_some(),
                    ..Default::default()
                };
                let mut inner = String::new();
                let mut j = k + 1;
                while j < recs.len() && recs[j].rtype != 184 {
                    let rec = &recs[j];
                    let mut r = Rd::new(rec.data);
                    match rec.rtype {
                        189 => {
                            let af = u32::from(r.u16()?);
                            let n = r.u32()?;
                            let mut items = Vec::new();
                            j = read_items(&recs, j + 1, &mut items);
                            // 최소·최대는 항목에서 셈한다 (한셀 바이너리의 묶음 필드 값은 첫 값)
                            let nums: Vec<f64> = items
                                .iter()
                                .filter_map(|i| match i {
                                    Item::Num(v) => Some(*v),
                                    _ => None,
                                })
                                .collect();
                            let dates: Vec<DateTime> = items
                                .iter()
                                .filter_map(|i| match i {
                                    Item::Date(d) => Some(*d),
                                    _ => None,
                                })
                                .collect();
                            inner.push_str("<x:sharedItems");
                            let date = af & 0x04 != 0;
                            for (n, bit, def) in [
                                ("containsSemiMixedTypes", 0, true),
                                ("containsNonDate", 1, true),
                                ("containsDate", 2, false),
                                ("containsString", 3, true),
                                ("containsBlank", 4, false),
                                ("containsMixedTypes", 5, false),
                                ("containsNumber", 6, false),
                                ("containsInteger", 7, false),
                            ] {
                                let mut v = af & (1 << bit) != 0;
                                // 한셀은 날짜만 있는 필드에도 0x02 를 켠다 — 날짜 필드는 항목으로 정한다 (한셀의 .xlsb
                                // 읽기: 날짜만 → containsNonDate="0", 피벗 실험 pg)
                                if n == "containsNonDate" && date {
                                    v = items.iter().any(|i| {
                                        matches!(
                                            i,
                                            Item::Num(_)
                                                | Item::Str(_)
                                                | Item::Bool(_)
                                                | Item::Err(_)
                                        )
                                    });
                                }
                                if v != def {
                                    let _ = write!(inner, " {n}=\"{}\"", b01(v));
                                }
                            }
                            if af & 0x100 != 0 {
                                if date {
                                    let min = dates
                                        .iter()
                                        .map(|d| d.serial())
                                        .fold(f64::INFINITY, f64::min);
                                    let max = dates
                                        .iter()
                                        .map(|d| d.serial())
                                        .fold(f64::NEG_INFINITY, f64::max);
                                    if min.is_finite() {
                                        let _ = write!(
                                            inner,
                                            " minDate=\"{}\" maxDate=\"{}\"",
                                            DateTime::from_serial(min).xml(),
                                            DateTime::from_serial(max).xml()
                                        );
                                    }
                                } else if !nums.is_empty() {
                                    let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
                                    let max =
                                        nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                                    let _ = write!(
                                        inner,
                                        " minValue=\"{}\" maxValue=\"{}\"",
                                        fmt_num(min),
                                        fmt_num(max)
                                    );
                                }
                            }
                            if af & 0x200 != 0 {
                                inner.push_str(" longText=\"1\"");
                            }
                            if n == 0 {
                                inner.push_str("/>");
                            } else {
                                let _ = write!(inner, " count=\"{n}\">");
                                for it in &items {
                                    inner.push_str(&item_xml(it));
                                }
                                inner.push_str("</x:sharedItems>");
                            }
                            kind = FieldKind {
                                items: n > 0,
                                num: af & 0x40 != 0,
                                date,
                                text: af & 0x08 != 0,
                                ..kind
                            };
                            continue;
                        }
                        219 => {
                            let par = r.i32()?;
                            let base = r.i32()?;
                            inner.push_str("<x:fieldGroup");
                            if par != -1 {
                                let _ = write!(inner, " par=\"{par}\"");
                            }
                            if base != -1 {
                                let _ = write!(inner, " base=\"{base}\"");
                            }
                            inner.push('>');
                        }
                        223 => {
                            const BY: [&str; 8] = [
                                "range", "seconds", "minutes", "hours", "days", "months",
                                "quarters", "years",
                            ];
                            let by = r.u8()?;
                            let gf = r.u8()?;
                            let (st, en, iv) = (r.f64()?, r.f64()?, r.f64()?);
                            let _ = write!(
                                inner,
                                "<x:rangePr autoStart=\"{}\" autoEnd=\"{}\" groupBy=\"{}\"",
                                b01(gf & 0x01 != 0),
                                b01(gf & 0x02 != 0),
                                BY.get(by as usize).unwrap_or(&"range")
                            );
                            if gf & 0x04 != 0 {
                                let _ = write!(
                                    inner,
                                    " startDate=\"{}\" endDate=\"{}\"",
                                    DateTime::from_serial(st).xml(),
                                    DateTime::from_serial(en).xml()
                                );
                            } else {
                                let _ = write!(
                                    inner,
                                    " startNum=\"{}\" endNum=\"{}\"",
                                    fmt_num(st),
                                    fmt_num(en)
                                );
                            }
                            let _ = write!(inner, " groupInterval=\"{}\"/>", fmt_num(iv));
                        }
                        221 => {
                            let n = r.u32()?;
                            let mut items = Vec::new();
                            j = read_items(&recs, j + 1, &mut items);
                            let _ = write!(inner, "<x:groupItems count=\"{n}\">");
                            for it in &items {
                                inner.push_str(&item_xml(it));
                            }
                            inner.push_str("</x:groupItems>");
                            continue;
                        }
                        220 => inner.push_str("</x:fieldGroup>"),
                        _ => {}
                    }
                    j += 1;
                }
                if inner.is_empty() {
                    fields.push_str("/>");
                } else {
                    fields.push('>');
                    fields.push_str(&inner);
                    fields.push_str("</x:cacheField>");
                }
                kinds.push(kind);
                k = j;
            }
            _ => {}
        }
        k += 1;
    }
    let _ = write!(
        s,
        "<x:cacheFields count=\"{nfields}\">{fields}</x:cacheFields>"
    );
    s.push_str("</x:pivotCacheDefinition>");
    Some((s, rel, kinds))
}

/// 캐시 정의 (+ 레코드) 레코드 파트 → (정의 XML, 레코드 XML)
pub(crate) fn cache_to_xml(def: &[u8], records: Option<&[u8]>) -> Option<(String, Option<String>)> {
    let (xml, _rel, kinds) = cache_def_to_xml(def)?;
    let rec_xml = records.map(|data| {
        let recs = super::biff12::records(data);
        let count = recs
            .iter()
            .find(|r| r.rtype == 193)
            .and_then(|r| r.data.get(0..4))
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let mut s = format!("{DECL}<x:pivotCacheRecords {NS_DECL} count=\"{count}\">");
        for rec in recs.iter().filter(|r| r.rtype == 33) {
            let mut r = Rd::new(rec.data);
            s.push_str("<x:r>");
            for kind in kinds.iter().filter(|k| k.db) {
                if kind.items {
                    match r.u32() {
                        Some(v) => {
                            let _ = write!(s, "<x:x v=\"{v}\"/>");
                        }
                        None => break,
                    }
                } else if kind.num {
                    match r.f64() {
                        Some(v) => {
                            let _ = write!(s, "<x:n v=\"{}\"/>", fmt_num(v));
                        }
                        None => break,
                    }
                } else if kind.date && !kind.text {
                    match DateTime::read(&mut r) {
                        Some(d) => {
                            let _ = write!(s, "<x:d v=\"{}\"/>", d.xml());
                        }
                        None => break,
                    }
                } else {
                    match r.wstr() {
                        Some(v) => {
                            let _ = write!(s, "<x:s v=\"{}\"/>", esc(&v));
                        }
                        None => break,
                    }
                }
            }
            s.push_str("</x:r>");
        }
        s.push_str("</x:pivotCacheRecords>");
        s
    });
    Some((xml, rec_xml))
}

// ---------------------------------------------------------------------------
// 바이너리 쓰기: 모델의 XML 피벗 파트 → 레코드 파트
// ---------------------------------------------------------------------------

/// 바이너리로 쓸 피벗 캐시
pub(crate) struct BinCache {
    /// `pivotCaches/pivotCache/@cacheId` (BrtBeginPivotCacheID 의 idSx, 피벗 표의 idCache)
    pub id: u32,
    /// `xl/pivotCache/pivotCacheDefinitionN.bin`
    pub def_name: String,
    pub def: Vec<u8>,
    /// (`xl/pivotCache/pivotCacheRecordsN.bin`, 레코드)
    pub records: Option<(String, Vec<u8>)>,
    /// 원본 시트 — 한셀은 모든 시트의 수식 다음에 이 시트의 XTI 를 만든다 (피벗 실험 px1: 시트 다섯, 셋째 시트의
    /// 피벗 원본은 첫 시트 → XTI [셋째·넷째·다섯째(수식), 첫 시트])
    pub source_sheet: Option<String>,
}

/// 바이너리로 쓸 피벗 파트 전체
#[derive(Default)]
pub(crate) struct BinPivots {
    pub caches: Vec<BinCache>,
    /// 워크시트마다 피벗 표 (파트 이름 `xl/pivotTables/pivotTableN.bin`, 레코드, 캐시 정의 파트 이름)
    pub tables: Vec<Vec<(String, Vec<u8>, String)>>,
}

/// 원본 파트(`raw_parts`)의 관계 목록
pub(crate) fn raw_rels(
    raw: &[crate::model::RawPart],
    name: &str,
) -> Vec<crate::package::Relationship> {
    let rels = crate::package::rels_part_for(name);
    raw.iter()
        .find(|p| p.name == rels)
        .and_then(|p| crate::xml::parse_raw(&String::from_utf8_lossy(&p.data), &rels).ok())
        .map(|root| crate::package::relationships_of(name, &root))
        .unwrap_or_default()
}

pub(crate) fn raw_text(raw: &[crate::model::RawPart], name: &str) -> Option<String> {
    raw.iter()
        .find(|p| p.name == name)
        .map(|p| String::from_utf8_lossy(&p.data).into_owned())
}

/// 캐시 정의 XML 의 원본 시트 (`cacheSource/worksheetSource/@sheet`)
fn source_sheet(def_xml: &str) -> Option<String> {
    let root = crate::xml::parse(def_xml, "pivotCacheDefinition").ok()?;
    root.sml_child("cacheSource")?
        .sml_child("worksheetSource")?
        .attr("sheet")
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 모델의 피벗 파트 (통합 문서의 `pivotCacheDefinition` 관계, 시트의 `pivotTable` 관계와 그 XML 원본 파트)를 레코드
/// 파트로 바꾼다. 파트 이름은 한셀처럼 문서 전체에서 1부터 (캐시는 통합 문서 관계 순서, 표는 시트 순서). 바꾸지 못한
/// 캐시는 경고와 함께 빼고, 그 캐시를 쓰는 피벗 표도 뺀다
pub(crate) fn binary_parts(
    wb: &crate::model::Workbook,
    sheets: &[&crate::model::Sheet],
    warnings: &mut Vec<String>,
    exact: bool,
) -> BinPivots {
    let mut out = BinPivots::default();
    // 원본 캐시 정의 파트 이름 → 새 캐시
    let mut by_part: Vec<(String, String)> = Vec::new();
    for l in wb
        .part_links
        .iter()
        .filter(|l| l.rel_type.ends_with("/pivotCacheDefinition"))
    {
        let Some(def_xml) = raw_text(&wb.raw_parts, &l.part) else {
            continue;
        };
        let rec_part = raw_rels(&wb.raw_parts, &l.part)
            .into_iter()
            .find(|r| !r.external && r.rel_type.ends_with("/pivotCacheRecords"))
            .map(|r| r.target);
        let rec_xml = rec_part.as_deref().and_then(|p| raw_text(&wb.raw_parts, p));
        let k = out.caches.len() + 1;
        match cache_to_bin(&def_xml, rec_xml.as_deref(), Some("rId1"), exact) {
            Ok((def, records)) => {
                let def_name = format!("xl/pivotCache/pivotCacheDefinition{k}.bin");
                by_part.push((l.part.clone(), def_name.clone()));
                out.caches.push(BinCache {
                    id: l.cache_id.unwrap_or(k as u32),
                    def_name,
                    def,
                    records: records
                        .map(|r| (format!("xl/pivotCache/pivotCacheRecords{k}.bin"), r)),
                    source_sheet: source_sheet(&def_xml),
                });
            }
            Err(e) => warnings.push(format!("피벗 캐시 {}: {e} — 쓰지 않음", l.part)),
        }
    }
    let mut n = 0;
    for sh in sheets {
        let mut list = Vec::new();
        for l in sh
            .part_links
            .iter()
            .filter(|l| l.rel_type.ends_with("/pivotTable"))
        {
            let Some(xml) = raw_text(&wb.raw_parts, &l.part) else {
                continue;
            };
            let cache = raw_rels(&wb.raw_parts, &l.part)
                .into_iter()
                .find(|r| !r.external && r.rel_type.ends_with("/pivotCacheDefinition"))
                .and_then(|r| by_part.iter().find(|(p, _)| *p == r.target))
                .map(|(_, d)| d.clone());
            let Some(cache) = cache else {
                warnings.push(format!("피벗 표 {}: 캐시를 쓰지 못해 뺌", l.part));
                continue;
            };
            match table_to_bin(&xml, exact) {
                Ok(bin) => {
                    n += 1;
                    list.push((format!("xl/pivotTables/pivotTable{n}.bin"), bin, cache));
                }
                Err(e) => warnings.push(format!("피벗 표 {}: {e} — 쓰지 않음", l.part)),
            }
        }
        out.tables.push(list);
    }
    out
}

// ---------------------------------------------------------------------------
// 바이너리 읽기: 레코드 파트 → 모델의 XML 피벗 파트
// ---------------------------------------------------------------------------

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";
const CT_CACHE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
const CT_RECORDS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";

/// 관계 하나짜리 관계 파트
fn one_rel(ty: &str, target: &str) -> Vec<u8> {
    format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/{ty}\" Target=\"{}\"/></Relationships>",
        esc(target)
    )
    .into_bytes()
}

fn push_raw(raw: &mut Vec<crate::model::RawPart>, name: String, ct: Option<&str>, data: Vec<u8>) {
    raw.retain(|p| p.name != name);
    raw.push(crate::model::RawPart {
        name,
        content_type: ct.map(str::to_string),
        data,
    });
}

/// 바이너리 원본의 피벗 캐시 (통합 문서 관계 `pivotCacheDefinition`, 번호는 BrtBeginPivotCacheID 의 idSx) → XML
/// 원본 파트(`xl/pivotCache/pivotCacheDefinitionN.xml` + 레코드)와 통합 문서 관계. 돌려주는 값은 (바이너리 정의 파트,
/// XML 정의 파트) — 피벗 표가 캐시를 찾는다
pub(crate) fn read_binary_caches(
    pkg: &mut crate::package::Package,
    rels: &[crate::package::Relationship],
    ids: &[(u32, String)],
    raw: &mut Vec<crate::model::RawPart>,
    links: &mut Vec<crate::model::PartLink>,
) -> crate::error::Result<Vec<(String, String)>> {
    let mut map = Vec::new();
    for r in rels
        .iter()
        .filter(|r| !r.external && r.rel_type.ends_with("/pivotCacheDefinition"))
    {
        let Some(def) = pkg.read_bytes(&r.target)? else {
            continue;
        };
        let mut records = None;
        if let Some(rr) = pkg
            .relationships(&r.target)?
            .into_iter()
            .find(|x| !x.external && x.rel_type.ends_with("/pivotCacheRecords"))
        {
            records = pkg.read_bytes(&rr.target)?;
        }
        let Some((def_xml, rec_xml)) = cache_to_xml(&def, records.as_deref()) else {
            continue;
        };
        let name = add_cache_parts(
            raw,
            links,
            map.len() + 1,
            def_xml,
            rec_xml,
            ids.iter().find(|(_, id)| *id == r.id).map(|x| x.0),
        );
        map.push((r.target.clone(), name));
    }
    Ok(map)
}

/// 캐시 XML 을 원본 파트(`xl/pivotCache/pivotCacheDefinitionK.xml` + 관계 + 레코드)와 통합 문서 관계로 둔다. 돌려주는
/// 값은 정의 파트 이름 (바이너리·레거시 읽기가 함께 쓴다)
pub(crate) fn add_cache_parts(
    raw: &mut Vec<crate::model::RawPart>,
    links: &mut Vec<crate::model::PartLink>,
    k: usize,
    def_xml: String,
    rec_xml: Option<String>,
    cache_id: Option<u32>,
) -> String {
    let name = format!("xl/pivotCache/pivotCacheDefinition{k}.xml");
    if let Some(x) = rec_xml {
        let rname = format!("pivotCacheRecords{k}.xml");
        push_raw(
            raw,
            crate::package::rels_part_for(&name),
            None,
            one_rel("pivotCacheRecords", &rname),
        );
        push_raw(
            raw,
            format!("xl/pivotCache/{rname}"),
            Some(CT_RECORDS),
            x.into_bytes(),
        );
    }
    push_raw(raw, name.clone(), Some(CT_CACHE), def_xml.into_bytes());
    links.push(crate::model::PartLink {
        rel_type: format!("{REL}/pivotCacheDefinition"),
        part: name.clone(),
        cache_id,
    });
    name
}

/// 피벗 표 XML 을 원본 파트(`xl/pivotTables/pivotTableN.xml` + 캐시 정의로 가는 관계)와 시트 관계로 둔다
pub(crate) fn add_table_part(
    raw: &mut Vec<crate::model::RawPart>,
    links: &mut Vec<crate::model::PartLink>,
    n: usize,
    xml: String,
    cache: &str,
) {
    let name = format!("xl/pivotTables/pivotTable{n}.xml");
    let target = format!(
        "../pivotCache/{}",
        cache.rsplit('/').next().unwrap_or(cache)
    );
    push_raw(
        raw,
        crate::package::rels_part_for(&name),
        None,
        one_rel("pivotCacheDefinition", &target),
    );
    push_raw(raw, name.clone(), Some(CT_TABLE), xml.into_bytes());
    links.push(crate::model::PartLink {
        rel_type: format!("{REL}/pivotTable"),
        part: name,
        cache_id: None,
    });
}

/// 바이너리 원본 시트의 피벗 표 (시트 관계 `pivotTable`) → XML 원본 파트(`xl/pivotTables/pivotTableN.xml`, N 은 문서
/// 전체에서 `next` 부터)와 시트 관계. 캐시를 읽지 못한 표는 뺀다
pub(crate) fn read_binary_tables(
    pkg: &mut crate::package::Package,
    rels: &[crate::package::Relationship],
    caches: &[(String, String)],
    next: &mut usize,
    raw: &mut Vec<crate::model::RawPart>,
    links: &mut Vec<crate::model::PartLink>,
) -> crate::error::Result<()> {
    for r in rels
        .iter()
        .filter(|r| !r.external && r.rel_type.ends_with("/pivotTable"))
    {
        let cache = pkg
            .relationships(&r.target)?
            .into_iter()
            .find(|x| !x.external && x.rel_type.ends_with("/pivotCacheDefinition"))
            .and_then(|x| caches.iter().find(|c| c.0 == x.target))
            .map(|c| c.1.clone());
        let (Some(cache), Some(data)) = (cache, pkg.read_bytes(&r.target)?) else {
            continue;
        };
        let Some(xml) = table_to_xml(&data, None) else {
            continue;
        };
        *next += 1;
        add_table_part(raw, links, *next, xml, &cache);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_serials() {
        let d = DateTime::parse("2025-12-09T00:00:00.000").unwrap();
        assert_eq!(d.serial(), 46000.0);
        assert_eq!(DateTime::from_serial(46000.0), d);
        assert_eq!(d.xml(), "2025-12-09T00:00:00.000");
        let t = DateTime::from_serial(45000.5);
        assert_eq!(t.xml(), "2023-03-15T12:00:00.000");
    }
}
