// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` 쓰기 검증 (손으로 만든 모델). 말뭉치 검증은 `tests/real_samples.rs`.

mod common;

use rcell::compare::compare;
use rcell::model::{PartLink, RawPart};

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const NS: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// 손으로 쓴 작은 피벗 (행 필드, 값 필드 둘, 계산 필드)을 붙인 문서
fn with_pivot() -> rcell::Workbook {
    let rels = |ty: &str, target: &str| {
        format!(
            r#"{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/{ty}" Target="{target}"/></Relationships>"#
        )
    };
    let cache = format!(
        r#"{DECL}<pivotCacheDefinition {NS} r:id="rId1" createdVersion="3" recordCount="3"><cacheSource type="worksheet"><worksheetSource ref="A1:B4" sheet="데이터"/></cacheSource><cacheFields count="3"><cacheField name="품목" numFmtId="0"><sharedItems count="2"><s v="사과"/><s v="배"/></sharedItems></cacheField><cacheField name="금액" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="100" maxValue="300" count="3"><n v="100"/><n v="200"/><n v="300"/></sharedItems></cacheField><cacheField name="두배" numFmtId="0" formula="금액*2" databaseField="0"/></cacheFields></pivotCacheDefinition>"#
    );
    let records = format!(
        r#"{DECL}<pivotCacheRecords {NS} count="3"><r><x v="0"/><x v="0"/></r><r><x v="1"/><x v="1"/></r><r><x v="0"/><x v="2"/></r></pivotCacheRecords>"#
    );
    let table = format!(
        r#"{DECL}<pivotTableDefinition {NS} name="피벗1" cacheId="7" dataOnRows="0" applyWidthHeightFormats="1" dataCaption="값" updatedVersion="4" createdVersion="4" indent="0" outline="1" outlineData="1"><location ref="D1:F4" firstHeaderRow="1" firstDataRow="2" firstDataCol="1"/><pivotFields count="3"><pivotField axis="axisRow" showAll="0" sortType="descending"><items count="3"><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField dataField="1" showAll="0"/><pivotField dataField="1" dragToRow="0" dragToCol="0" dragToPage="0" showAll="0"/></pivotFields><rowFields count="1"><field x="0"/></rowFields><rowItems count="3"><i><x/></i><i><x v="1"/></i><i t="grand"><x/></i></rowItems><colFields count="1"><field x="-2"/></colFields><colItems count="2"><i><x/></i><i i="1"><x v="1"/></i></colItems><dataFields count="2"><dataField name="합계 : 금액" fld="1" subtotal="sum" showDataAs="normal" baseField="0" baseItem="0"/><dataField name="최대 : 두배" fld="2" subtotal="max" showDataAs="normal" baseField="0" baseItem="0"/></dataFields><pivotTableStyleInfo name="PivotStyleLight16" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/></pivotTableDefinition>"#
    );
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let raw = |name: &str, ct: Option<&str>, data: String| RawPart {
        name: name.into(),
        content_type: ct.map(str::to_string),
        data: data.into_bytes(),
    };
    a.raw_parts.extend([
        raw(
            "xl/pivotCache/pivotCacheDefinition1.xml",
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"),
            cache,
        ),
        raw(
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            None,
            rels("pivotCacheRecords", "pivotCacheRecords1.xml"),
        ),
        raw(
            "xl/pivotCache/pivotCacheRecords1.xml",
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"),
            records,
        ),
        raw(
            "xl/pivotTables/pivotTable1.xml",
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"),
            table,
        ),
        raw(
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            None,
            rels("pivotCacheDefinition", "../pivotCache/pivotCacheDefinition1.xml"),
        ),
    ]);
    a.part_links.push(PartLink {
        rel_type: format!("{REL}/pivotCacheDefinition"),
        part: "xl/pivotCache/pivotCacheDefinition1.xml".into(),
        cache_id: Some(7),
    });
    a.sheets[0].part_links.push(PartLink {
        rel_type: format!("{REL}/pivotTable"),
        part: "xl/pivotTables/pivotTable1.xml".into(),
        cache_id: None,
    });
    a
}

/// 피벗: 모델의 XML 파트 → 레거시 (전역 SXSTREAMID·SXVS·DCONREF, 캐시 스트림 `/_SX_DB_CUR/0001`, 시트의 SXVIEW…)
/// → 다시 읽은 XML 파트가 레거시 보기와 같다 (docs/format/cell-legacy.md 11절)
#[test]
fn pivots_roundtrip() {
    let a = with_pivot();
    let (bytes, report) = rcell::legacy::write_with_report(&a).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    // 캐시 스트림: SXDB (레코드 3, 스트림 1, saveData·enableRefresh, 블록당 0x1FFF / 원본 필드 2, 원본 2·전체 3 필드)
    let cache = rcell::container::read_entry(&bytes, "/_SX_DB_CUR/0001").unwrap();
    assert_eq!(
        &cache[..18],
        &[
            0xC6, 0x00, 0x15, 0x00, 3, 0, 0, 0, 1, 0, 0x21, 0, 0xFF, 0x0F, 2, 0, 3, 0
        ]
    );
    // 전역: STYLE 뒤 SXSTREAMID·SXVS·DCONREF, 원본 시트의 XTI (EXTERNSHEET)
    let stream = rcell::container::read_entry(&bytes, "/Workbook").unwrap();
    let recs = rcell::legacy::biff::records(&stream);
    let at = |t: u16| recs.iter().position(|r| r.rtype == t).unwrap();
    assert!(at(0x0293) < at(0x00D5) && at(0x00D5) < at(0x00E3) && at(0x00E3) < at(0x0051));
    let xti = &recs[at(0x0017)];
    assert_eq!(xti.data, [1, 0, 0, 0, 0, 0, 0, 0]);
    // 시트: SXVIEW (범위 D1:F4, 캐시 0), 계산 필드의 SXVD 는 부분합이 없다
    let view = &recs[at(0x00B0)];
    assert_eq!(&view.data[..12], &[0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 5, 0]);
    let vd: Vec<&[u8]> = recs
        .iter()
        .filter(|r| r.rtype == 0x00B1)
        .map(|r| r.data)
        .collect();
    assert_eq!(&vd[2][..6], &[8, 0, 0, 0, 0, 0]);
    // 다시 읽기: 피벗 표가 레거시 보기와 같다
    let b = rcell::from_bytes(bytes).unwrap();
    let expected = rcell::legacy::lossy::legacy_view(&a);
    let diffs: Vec<String> = compare(&expected, &b, Default::default())
        .into_iter()
        .filter(|d| d.contains("피벗"))
        .collect();
    assert!(diffs.is_empty(), "{diffs:#?}");
    let text = |w: &rcell::Workbook, name: &str| {
        String::from_utf8(
            w.raw_parts
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .data
                .clone(),
        )
        .unwrap()
    };
    let cache_b = text(&b, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(
        cache_b.contains(r#"formula="금액*2" databaseField="0""#),
        "{cache_b}"
    );
    let table_b = text(&b, "xl/pivotTables/pivotTable1.xml");
    assert!(table_b.contains(r#"sortType="descending""#), "{table_b}");
    assert!(table_b.contains(r#"name="최대 : 두배""#), "{table_b}");
}

/// 이름 XTI 차례 (docs/format/cell-legacy.md 10.5절): 값 캐시가 시트 하나인 외부 연결의 XTI 가 이름 차례보다 앞
/// (Trade_Show_Budget 실험) — 레거시·바이너리 쓰기 모두. 다시 읽은 레거시는 읽은 XTI 차례를 지키고, 힌트가 없으면 이름 차례
#[test]
fn preset_xti_come_first() {
    use rcell::model::{DefinedName, ExternalLink, ExternalSheetData};
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.external_links = vec![
        ExternalLink {
            target: "/a.xls".into(),
            sheet_names: vec!["S1".into(), "S2".into()],
            ..Default::default()
        },
        ExternalLink {
            target: "/b.xls".into(),
            sheet_names: vec!["T1".into()],
            sheet_data: vec![ExternalSheetData::default()],
            ..Default::default()
        },
    ];
    a.defined_names = vec![
        DefinedName {
            name: "First".into(),
            formula: "'[1]S2'!$A$1".into(),
            ..Default::default()
        },
        DefinedName {
            name: "Second".into(),
            formula: "'[2]T1'!$B$2".into(),
            ..Default::default()
        },
    ];
    let xti_of = |bytes: &[u8]| {
        let stream = rcell::container::read_entry(bytes, "/Workbook").unwrap();
        let recs = rcell::legacy::biff::records(&stream);
        recs.iter()
            .find(|r| r.rtype == 0x0017)
            .unwrap()
            .data
            .to_vec()
    };
    // 외부 2 의 시트 0 (SUPBOOK 1) 이 먼저, 이름 First 의 외부 1 시트 1 (SUPBOOK 0) 이 다음
    let order = [2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0];
    let (bytes, _) = rcell::legacy::write_with_report(&a).unwrap();
    assert_eq!(xti_of(&bytes), order);
    // 다시 읽은 레거시: 수식은 그대로, 값 캐시가 없어도 읽은 XTI 차례를 지킨다
    let mut b = rcell::from_bytes(bytes).unwrap();
    assert_eq!(b.defined_names[0].formula, "'[1]S2'!$A$1");
    assert_eq!(b.defined_names[1].formula, "'[2]T1'!$B$2");
    assert!(b.external_links.iter().all(|l| l.sheet_data.is_empty()));
    let (again, _) = rcell::legacy::write_with_report(&b).unwrap();
    assert_eq!(xti_of(&again), order);
    // 힌트를 지우면 이름 차례
    b.legacy_xti.clear();
    let (plain, _) = rcell::legacy::write_with_report(&b).unwrap();
    assert_eq!(xti_of(&plain), [2, 0, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0]);
    // 바이너리 BrtExternSheet: (BrtSupBookSrc 번호 u32, 첫·끝 시트 i32)
    let out = rcell::binary::writer::write_report(&a, &Default::default()).unwrap();
    let wbin = rcell::container::read_entry(&out.bytes, "xl/workbook.bin").unwrap();
    let recs = rcell::binary::biff12::records(&wbin);
    let ext = recs.iter().find(|r| r.rtype == 362).unwrap();
    let words: Vec<u32> = ext
        .data
        .chunks(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(words, [2, 1, 0, 0, 0, 1, 1]);
}
