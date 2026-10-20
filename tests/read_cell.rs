// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

mod common;

use rcell::model::{CellRef, CellValue, FormulaKind, SheetState};
use rcell::{Error, FileKind};

/// 한셀 13.0.0.711 COM 차분 실험에서 관측한 글꼴 확장 형태 (docs/format/cell-2024.md 5.1).
#[test]
fn reads_observed_shadow_outline_strike_extensions() {
    let xml = r#"<x:font xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0" hs:extension="1">
        <x:name val="맑은 고딕"/><x:strike/><x:vertAlign val="superscript"/>
        <hs:shadowType val="drop"/><hs:shadowColor rgb="ffc0c0c0"/>
        <hs:shadowOffsetX val="10"/><hs:shadowOffsetY val="10"/>
        <hs:outlineType val="solid"/>
        <hs:strikeoutType val="1"/><hs:strikeoutShape val="solid"/><hs:strikeoutColor rgb="ff000000"/>
        <hs:superscript val="1"/><hs:underlineShape val="doubleslim"/>
      </x:font>"#;
    let font = rcell::parser::styles::parse_font(&rcell::xml::parse(xml, "t").unwrap());
    assert!(font.strike);
    assert_eq!(font.vert_align.as_deref(), Some("superscript"));
    let h = font.hancom.unwrap();
    assert_eq!(h.shadow_type.as_deref(), Some("drop"));
    assert_eq!(h.shadow_color.unwrap().rgb.as_deref(), Some("ffc0c0c0"));
    assert_eq!((h.shadow_offset_x, h.shadow_offset_y), (Some(10), Some(10)));
    assert_eq!(h.outline_type.as_deref(), Some("solid"));
    assert_eq!(h.strikeout_type.as_deref(), Some("1"));
    assert_eq!(h.strikeout_shape.as_deref(), Some("solid"));
    assert_eq!(h.superscript, Some(true));
    assert_eq!(h.underline_shape.as_deref(), Some("doubleslim"));
    assert!(h.other.is_empty());
}

/// 한셀 COM 차분 실험: 들여쓰기 n단계 → `hs:leftMargin = 1134 × n` (오른쪽 정렬이면 rightMargin).
#[test]
fn reads_observed_parashape_margins() {
    let xml = r#"<x:styleSheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
        xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0"><x:cellXfs count="2">
        <x:xf numFmtId="0" fontId="0" fillId="0" borderId="0" hs:applyExtension="1">
          <x:alignment horizontal="left" indent="3"/>
          <hs:parashape hs:lineSpacingType="betweenLines" hs:lineSpacing="75" hs:leftMargin="3402"/>
        </x:xf>
        <x:xf numFmtId="0" fontId="0" fillId="0" borderId="0" hs:applyExtension="1">
          <x:alignment horizontal="right" indent="2"/>
          <hs:parashape hs:rightMargin="2268" hs:indentation="-500"/>
          <hs:accountUnderline hs:shape="double"/>
        </x:xf></x:cellXfs></x:styleSheet>"#;
    let st = rcell::parser::styles::parse_styles(&rcell::xml::parse(xml, "t").unwrap());
    let p0 = st.cell_xfs[0]
        .hancom
        .as_ref()
        .unwrap()
        .parashape
        .as_ref()
        .unwrap();
    assert_eq!(p0.left_margin, Some(3 * 1134));
    assert!(p0.other.is_empty());
    let ext1 = st.cell_xfs[1].hancom.as_ref().unwrap();
    let p1 = ext1.parashape.as_ref().unwrap();
    assert_eq!((p1.right_margin, p1.indentation), (Some(2268), Some(-500)));
    assert_eq!(ext1.account_underline.as_deref(), Some("double"));
    assert!(ext1.other.is_empty());
}

fn cell<'a>(s: &'a rcell::model::Sheet, a1: &str) -> &'a rcell::model::Cell {
    s.cell(CellRef::parse(a1).unwrap())
        .unwrap_or_else(|| panic!("셀 {a1} 없음"))
}

#[test]
fn reads_workbook_metadata() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    assert_eq!(wb.application.as_deref(), Some("Cell"));
    assert!(wb.is_written_by_hancom());
    assert_eq!(wb.hancom.calc_id, Some(904));
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.sheets[0].name, "데이터");
    assert_eq!(wb.sheets[1].state, SheetState::Hidden);
    assert_eq!(wb.defined_names[0].name, "합계범위");
    assert_eq!(wb.defined_names[0].formula, "'데이터'!$B$1:$B$3");
}

#[test]
fn reads_cell_values_of_every_type() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(cell(s, "A1").value, CellValue::Text("안녕 & 반가워".into()));
    assert_eq!(cell(s, "B1").value, CellValue::Number(3.5));
    assert_eq!(cell(s, "C1").formula.as_ref().unwrap().text, "B1*2");
    assert_eq!(cell(s, "A2").value, CellValue::Bool(true));
    assert_eq!(cell(s, "B2").value, CellValue::Error("#DIV/0!".into()));
    assert_eq!(cell(s, "C2").value, CellValue::Text("인라인".into()));
    // 런은 이어 붙이고 윗주(rPh)는 제외한다.
    assert_eq!(cell(s, "A3").value, CellValue::Text("굵게 보통".into()));
}

#[test]
fn cells_without_r_attribute_follow_previous_cell() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &wb.sheets[0];
    let b3 = cell(s, "B3").formula.as_ref().unwrap();
    assert_eq!(b3.kind, FormulaKind::Shared);
    assert_eq!(b3.range.as_deref(), Some("B3:C3"));
    let c3 = cell(s, "C3").formula.as_ref().unwrap();
    assert_eq!((c3.kind, c3.shared_index), (FormulaKind::Shared, Some(0)));
}

#[test]
fn shared_formula_followers_get_shifted_text() {
    // B3 = A2+1 (공유 B3:C3) → C3 는 한 열 옮긴 B2+1
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(cell(s, "B3").formula.as_ref().unwrap().text, "A2+1");
    assert_eq!(cell(s, "C3").formula.as_ref().unwrap().text, "B2+1");
}

#[test]
fn alternate_content_keeps_style_indices_aligned() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let st = &wb.styles;
    assert_eq!(st.fonts.len(), 3);
    assert_eq!(st.cell_xfs.len(), 3);
    // 셀 서식 2 → 글꼴 2 (굵게). 인덱스가 밀리면 실패한다.
    let f2 = st.font_of(2).unwrap();
    assert!(f2.bold);
    assert_eq!(f2.size, Some(12.0));
}

#[test]
fn reads_hancom_font_extension() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let f1 = wb.styles.font_of(1).unwrap();
    assert_eq!(f1.name.as_deref(), Some("함초롬바탕"));
    let h = f1.hancom.as_ref().expect("한컴 글꼴 확장");
    assert_eq!(h.ratio, Some(90));
    assert_eq!(h.spacing, Some(-5));
    assert_eq!(h.size, Some(100));
    assert_eq!(h.underline_shape.as_deref(), Some("solid"));
    assert_eq!(
        h.underline_color.as_ref().unwrap().rgb.as_deref(),
        Some("ff0000ff")
    );
    assert_eq!(h.use_font_space, Some(false));
    // 모르는 확장은 버리지 않고 보존한다.
    assert_eq!(h.other.get("futureThing").map(String::as_str), Some("42"));
}

#[test]
fn reads_hancom_parashape() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let xf = wb.styles.xf(1).unwrap();
    let ext = xf.hancom.as_ref().expect("한컴 셀 서식 확장");
    assert!(ext.apply_extension);
    let p = ext.parashape.as_ref().unwrap();
    assert_eq!(p.line_spacing_type.as_deref(), Some("betweenLines"));
    assert_eq!(p.line_spacing, Some(75));
    assert_eq!(p.break_latin_word.as_deref(), Some("keepWord"));
    assert_eq!(p.break_non_latin_word.as_deref(), Some("breakWord"));
    assert_eq!(p.text_alignment.as_deref(), Some("baseLine"));
    assert!(xf.alignment.as_ref().unwrap().wrap_text);
}

#[test]
fn reads_sheet_layout() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &wb.sheets[0];
    assert!(!s.show_grid_lines);
    assert_eq!(s.cols[0].width, Some(12.5));
    assert_eq!(s.rows[0].height, Some(20.0));
    assert_eq!(s.merges[0].to_string(), "A4:C4");
    assert_eq!(s.hyperlinks[0].location.as_deref(), Some("'숨김'!A1"));
    assert_eq!(s.used_range().unwrap().to_string(), "A1:C3");
}

#[test]
fn reads_excel_default_namespace_documents() {
    let wb = rcell::from_bytes(common::excel_style_xlsx()).unwrap();
    assert!(!wb.is_written_by_hancom());
    assert_eq!(cell(&wb.sheets[0], "B2").value, CellValue::Number(42.0));
}

#[test]
fn scan_reports_unmodeled_hancom_vocabulary() {
    let mut pkg = rcell::open_package(common::hancom_style_cell()).unwrap();
    let rep = rcell::scan::scan_package(&mut pkg).unwrap();
    let unknown: Vec<_> = rep
        .hancom
        .iter()
        .filter(|e| !e.modeled)
        .map(|e| e.path.as_str())
        .collect();
    assert_eq!(unknown, ["font/futureThing"]);
    assert!(rep.coverage() < 1.0);
    // 픽스처의 열거값은 모두 HncOOXML.dll 변환표의 어휘 안에 있다.
    assert!(rep.hancom.iter().all(|e| e.unknown_values.is_empty()));
}

#[test]
fn rejects_broken_ole2_and_unknown_bytes() {
    let mut cfb = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    cfb.resize(512, 0);
    assert_eq!(rcell::detect(&cfb), FileKind::Cfb);
    // 레거시 .cell 은 읽지만, 깨진 OLE2 바이트는 오류여야 한다.
    assert!(rcell::from_bytes(cfb.clone()).is_err());
    assert!(matches!(
        rcell::open_package(cfb),
        Err(Error::LegacyOrEncrypted)
    ));
    assert!(matches!(
        rcell::from_bytes(b"hello".to_vec()),
        Err(Error::UnknownFormat)
    ));
}

#[test]
fn reads_comments_with_vml_visibility() {
    // 한셀처럼 ref 를 "K8:K8" 범위로 쓰고, 표시 여부는 VML 도형에 있다
    let bytes = common::build_zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="/xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><x:sheets><x:sheet name="메모" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><x:sheetData/><x:legacyDrawing r:id="rId2"/></x:worksheet>"#,
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="../comments1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/></Relationships>"#,
        ),
        (
            "xl/comments1.xml",
            r#"<x:comments xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:authors><x:author>KMT</x:author><x:author>yoomin53</x:author></x:authors><x:commentList><x:comment ref="K8:K8" authorId="0"><x:text><x:r><x:rPr><x:rFont val="맑은 고딕"/><x:sz val="10"/></x:rPr><x:t>숨은 메모</x:t></x:r></x:text></x:comment><x:comment ref="B2" authorId="1"><x:text><x:t>보이는 메모</x:t></x:text></x:comment></x:commentList></x:comments>"#,
        ),
        (
            "xl/drawings/vmlDrawing1.vml",
            r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="메모 1" style="position:absolute;visibility:hidden"><x:ClientData ObjectType="Note"><x:Row>7</x:Row><x:Column>10</x:Column></x:ClientData></v:shape><v:shape id="메모 2" style="position:absolute;visibility:visible"><x:ClientData ObjectType="Note"><x:Row>1</x:Row><x:Column>1</x:Column><x:Visible/></x:ClientData></v:shape></xml>"##,
        ),
    ]);
    let wb = rcell::from_bytes(bytes).unwrap();
    let c = &wb.sheets[0].comments;
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].cell.to_string(), "K8");
    assert_eq!((c[0].author.as_str(), c[0].visible), ("KMT", false));
    assert_eq!(c[0].text.text, "숨은 메모");
    assert_eq!(
        c[0].text.runs[0].font.as_ref().unwrap().name.as_deref(),
        Some("맑은 고딕")
    );
    assert_eq!((c[1].author.as_str(), c[1].visible), ("yoomin53", true));
}
