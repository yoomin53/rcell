// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 테스트용 .cell 패키지 생성기.
//!
//! 한컴 샘플 파일은 저작물이라 저장소에 넣지 않는다. 대신 한셀이 쓰는 구조
//! (`x:` 접두사, `mc:AlternateContent` 로 감싼 `hs:` 확장)를 그대로 따라
//! 직접 작성한 XML 로 패키지를 만든다.

#![allow(dead_code)]

use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

pub fn build_zip(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, body) in parts {
        w.start_file(*name, opts).unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>"#;

pub fn hancom_style_cell() -> Vec<u8> {
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>"#
    );
    let root_rels = format!(
        r#"{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#
    );
    let app = format!(
        r#"{DECL}<ep:Properties xmlns:ep="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><ep:Application>Cell</ep:Application><ep:AppVersion>12.0300</ep:AppVersion></ep:Properties>"#
    );
    let workbook = format!(
        r#"{DECL}<x:workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:fileVersion appName="HCell" lastEdited="10.0" lowestEdited="10.0" rupBuild="0.4317"/><x:workbookPr date1904="0"/><x:sheets><x:sheet name="데이터" sheetId="1" r:id="rId1"/><x:sheet name="숨김" sheetId="2" state="hidden" r:id="rId2"/></x:sheets><x:definedNames><x:definedName name="합계범위" localSheetId="0">'데이터'!$B$1:$B$3</x:definedName></x:definedNames><x:calcPr xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0" mc:Ignorable="hs" hs:hclCalcId="904"/></x:workbook>"#
    );
    let wb_rels = format!(
        r#"{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
    );
    let sst = format!(
        r#"{DECL}<x:sst xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><x:si><x:t>안녕 &amp; 반가워</x:t></x:si><x:si><x:r><x:rPr><x:b/><x:rFont val="맑은 고딕"/></x:rPr><x:t>굵게</x:t></x:r><x:r><x:t xml:space="preserve"> 보통</x:t></x:r><x:rPh sb="0" eb="1"><x:t>무시</x:t></x:rPh></x:si></x:sst>"#
    );
    // 글꼴 1과 셀 서식 1은 한셀처럼 mc:AlternateContent 로 감싼다.
    // 이 처리를 하지 않는 파서는 글꼴 2/셀 서식 2 를 1로 잘못 읽는다.
    let styles = format!(
        r#"{DECL}<x:styleSheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:fonts count="3"><x:font><x:name val="맑은 고딕"/><x:sz val="10"/></x:font><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0" Requires="hs"><x:font hs:extension="1"><x:name val="함초롬바탕"/><x:sz val="11"/><x:u/><hs:underlineShape val="solid"/><hs:underlineType val="1"/><hs:underlineColor rgb="ff0000ff"/><hs:size val="100"/><hs:ratio val="90"/><hs:spacing val="-5"/><hs:offset val="0"/><hs:useFontSpace val="0"/><hs:futureThing val="42"/></x:font></mc:Choice><mc:Fallback><x:font><x:name val="함초롬바탕"/><x:sz val="11"/><x:u/></x:font></mc:Fallback></mc:AlternateContent><x:font><x:name val="맑은 고딕"/><x:sz val="12"/><x:b val="1"/><x:color rgb="ff289b6e"/></x:font></x:fonts><x:fills count="2"><x:fill><x:patternFill patternType="none"/></x:fill><x:fill><x:patternFill patternType="solid"><x:fgColor theme="4" tint="0.4"/></x:patternFill></x:fill></x:fills><x:borders count="1"><x:border><x:left style="thin"><x:color auto="1"/></x:left><x:right/><x:top/><x:bottom/><x:diagonal/></x:border></x:borders><x:cellXfs count="3"><x:xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0" Requires="hs"><x:xf numFmtId="0" fontId="1" fillId="1" borderId="0" xfId="0" applyAlignment="1" hs:applyExtension="1"><x:alignment horizontal="center" vertical="center" wrapText="1"/><hs:parashape hs:lineSpacingType="betweenLines" hs:lineSpacing="75" hs:breakNonLatinWord="breakWord" hs:breakLatinWord="keepWord" hs:condense="0" hs:textAlignment="baseLine"/></x:xf></mc:Choice><mc:Fallback><x:xf numFmtId="0" fontId="1" fillId="1" borderId="0" xfId="0" applyAlignment="1"><x:alignment horizontal="center" vertical="center" wrapText="1"/></x:xf></mc:Fallback></mc:AlternateContent><x:xf numFmtId="0" fontId="2" fillId="0" borderId="0" xfId="0" applyFont="1"/></x:cellXfs></x:styleSheet>"#
    );
    let sheet1 = format!(
        r#"{DECL}<x:worksheet xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:dimension ref="A1:C4"/><x:sheetViews><x:sheetView showGridLines="0" workbookViewId="0"/></x:sheetViews><x:sheetFormatPr defaultColWidth="9.140625" defaultRowHeight="13.2"/><x:cols><x:col min="1" max="1" width="12.5" customWidth="1"/></x:cols><x:sheetData><x:row r="1" ht="20" customHeight="1"><x:c r="A1" s="1" t="s"><x:v>0</x:v></x:c><x:c r="B1" s="2"><x:v>3.5</x:v></x:c><x:c r="C1"><x:f>B1*2</x:f><x:v>7</x:v></x:c></x:row><x:row r="2"><x:c r="A2" t="b"><x:v>1</x:v></x:c><x:c r="B2" t="e"><x:v>#DIV/0!</x:v></x:c><x:c r="C2" t="inlineStr"><x:is><x:t>인라인</x:t></x:is></x:c></x:row><x:row r="3"><x:c r="A3" t="s"><x:v>1</x:v></x:c><x:c><x:f t="shared" ref="B3:C3" si="0">A2+1</x:f><x:v>2</x:v></x:c><x:c><x:f t="shared" si="0"/><x:v>2</x:v></x:c></x:row><x:row r="4"><x:c r="A4" s="1"/><x:c r="B4" s="1"/><x:c r="C4" s="1"/></x:row></x:sheetData><x:mergeCells count="1"><x:mergeCell ref="A4:C4"/></x:mergeCells><x:hyperlinks><x:hyperlink ref="A1" location="'숨김'!A1" tooltip="이동"/></x:hyperlinks></x:worksheet>"#
    );
    let sheet2 = format!(
        r#"{DECL}<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetData/></x:worksheet>"#
    );
    build_zip(&[
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &root_rels),
        ("docProps/app.xml", &app),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &wb_rels),
        ("xl/sharedStrings.xml", &sst),
        ("xl/styles.xml", &styles),
        ("xl/worksheets/sheet1.xml", &sheet1),
        ("xl/worksheets/sheet2.xml", &sheet2),
    ])
}

/// Excel 처럼 기본 네임스페이스를 쓰는 최소 패키지.
pub fn excel_style_xlsx() -> Vec<u8> {
    build_zip(&[
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
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="2"><c r="B2"><v>42</v></c></row></sheetData></worksheet>"#,
        ),
    ])
}
