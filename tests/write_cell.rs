// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 저장(쓰기) 왕복 테스트: 읽기 → 쓰기 → 다시 읽기 결과가 같아야 한다.

mod common;

use rcell::compare::{Options, compare};
use rcell::writer::{Flavor, write};

fn roundtrip(bytes: Vec<u8>, flavor: Flavor) -> (rcell::Workbook, rcell::Workbook) {
    let a = rcell::from_bytes(bytes).unwrap();
    let out = write(&a, flavor).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    (a, b)
}

#[test]
fn hancom_fixture_roundtrips_without_loss() {
    let (a, b) = roundtrip(common::hancom_style_cell(), Flavor::Cell);
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    // 한컴 확장이 그대로 남는다
    let f1 = b.styles.font_of(1).unwrap().hancom.as_ref().unwrap();
    assert_eq!((f1.ratio, f1.spacing), (Some(90), Some(-5)));
    let p = b.styles.xf(1).unwrap().hancom.as_ref().unwrap();
    assert_eq!(p.parashape.as_ref().unwrap().line_spacing, Some(75));
}

#[test]
fn external_links_roundtrip() {
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.external_links.push(rcell::model::ExternalLink {
        target: "file:///C:/a%20b/Book%201.xls".into(),
        sheet_names: vec!["첫 시트".into(), "Data".into()],
        sheet_data: vec![rcell::model::ExternalSheetData {
            sheet_id: 1,
            refresh_error: true,
            cells: vec![(
                rcell::model::CellRef { row: 3, col: 1 },
                rcell::model::CellValue::Number(5.0),
            )],
        }],
        path_missing: false,
    });
    a.defined_names.push(rcell::model::DefinedName {
        name: "Ext".into(),
        local_sheet_id: None,
        hidden: false,
        formula: "'[1]첫 시트'!$A$1".into(),
    });
    let b = rcell::from_bytes(write(&a, Flavor::Cell).unwrap()).unwrap();
    assert_eq!(b.external_links.len(), 1);
    assert_eq!(b.external_links[0].target, "file:///C:/a%20b/Book%201.xls");
    assert_eq!(b.external_links[0].sheet_names, ["첫 시트", "Data"]);
    assert_eq!(b.external_links[0].file_name(), "Book 1.xls");
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn views_and_print_settings_roundtrip() {
    use rcell::model::{HeaderFooter, PageMargins, PageSetup, Pane, PrintOptions};
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &mut a.sheets[0];
    s.code_name = Some("Sheet1".into());
    s.view.zoom_scale = Some(85);
    s.view.show_zeros = false;
    s.view.pane = Some(Pane {
        x_split: 1.0,
        y_split: 2.0,
        top_left_cell: Some("B3".into()),
        active_pane: Some("bottomRight".into()),
        state: Some("frozen".into()),
    });
    s.view.active_cell = Some("C4".into());
    s.view.sqref = Some("C4:D5 F6".into());
    s.page.fit_to_page = Some(true);
    s.page.print_options = Some(PrintOptions {
        grid_lines: true,
        ..Default::default()
    });
    s.page.margins = Some(PageMargins {
        left: 0.5,
        right: 0.5,
        top: 1.0,
        bottom: 1.0,
        header: 0.3,
        footer: 0.3,
    });
    s.page.setup = Some(PageSetup {
        paper_size: Some(9),
        orientation: Some("landscape".into()),
        use_printer_defaults: true,
        ..Default::default()
    });
    s.page.header_footer = Some(HeaderFooter {
        different_first: true,
        scale_with_doc: true,
        align_with_margins: true,
        odd_header: Some("&C&A".into()),
        first_footer: Some("&C첫 쪽".into()),
        ..Default::default()
    });
    s.page.row_breaks = vec![10, 20];
    s.page.col_breaks = vec![3];
    let b = rcell::from_bytes(write(&a, Flavor::Cell).unwrap()).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(
        b.sheets[0].view.pane.as_ref().unwrap().state.as_deref(),
        Some("frozen")
    );
}

#[test]
fn excel_fixture_roundtrips_as_xlsx() {
    // 원본에 스타일 파트가 없으므로(쓰기에서 기본 스타일을 채운다) 값과 구조만 비교한다.
    let (a, b) = roundtrip(common::excel_style_xlsx(), Flavor::Xlsx);
    let opts = Options {
        skip_styles: true,
        ..Default::default()
    };
    let diffs = compare(&a, &b, opts);
    assert!(
        diffs.is_empty(),
        "{}",
        diffs.join(
            "
"
        )
    );
}

#[test]
fn written_cell_uses_hancom_package_conventions() {
    let wb = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let out = write(&wb, Flavor::Cell).unwrap();
    let mut pkg = rcell::open_package(out).unwrap();
    assert_eq!(
        pkg.content_type("xl/workbook.xml"),
        Some("application/vnd.ms-excel.sheet.macroEnabled.main+xml")
    );
    let styles = pkg.read_string("xl/styles.xml").unwrap().unwrap();
    assert!(styles.contains("<mc:AlternateContent"));
    assert!(styles.contains("Requires=\"hs\""));
    // 개인정보: 저장 경로를 남기지 않는다
    let wbx = pkg.read_string("xl/workbook.xml").unwrap().unwrap();
    assert!(!wbx.contains("absPath"));
}

#[test]
fn comments_roundtrip() {
    use rcell::model::{CellRef, Comment, RichText};
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    for (row, col, author, text, visible) in [
        (0, 0, "yoomin53", "첫 메모", false),
        (2, 1, "KMT", "두 줄\n메모", true),
    ] {
        a.sheets[0].comments.push(Comment {
            cell: CellRef { row, col },
            author: author.into(),
            text: RichText {
                text: text.into(),
                runs: Vec::new(),
            },
            visible,
        });
    }
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.sheets[0].comments.len(), 2);
}

#[test]
fn drawings_and_chart_parts_roundtrip() {
    const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
    const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let drawing = format!(
        r#"<xdr:wsDr xmlns:xdr="{XDR}" xmlns:a="{A}" xmlns:r="{R}" xmlns:c="{C}"><xdr:twoCellAnchor editAs="oneCell"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>9525</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>6</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp><xdr:nvSpPr><xdr:cNvPr id="2" name="글상자 1"/><xdr:cNvSpPr txBox="1"/></xdr:nvSpPr><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="f5fcea"/></a:solidFill><a:ln w="9525"><a:noFill/></a:ln></xdr:spPr><xdr:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1100" b="1"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill><a:latin typeface="맑은 고딕"/></a:rPr><a:t>안녕</a:t></a:r><a:br/><a:r><a:t>yoomin53</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor><xdr:twoCellAnchor><xdr:from><xdr:col>5</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>10</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>15</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="3" name="차트 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="{C}"><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#
    );
    let chart = format!(
        r#"<c:chartSpace xmlns:c="{C}" xmlns:a="{A}"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>매출</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:layout/><c:bar3DChart><c:barDir val="col"/><c:grouping val="clustered"/><c:ser><c:idx val="0"/><c:tx><c:strRef><c:f>그림!$A$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>yoomin53</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:srgbClr val="73B1D1"/></a:solidFill></c:spPr><c:cat><c:strRef><c:f>그림!$B$1:$D$1</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>1월</c:v></c:pt><c:pt idx="1"><c:v>2월</c:v></c:pt><c:pt idx="2"><c:v>3월</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>그림!$B$2:$D$2</c:f><c:numCache><c:ptCount val="3"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="2"><c:v>30.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:bar3DChart></c:plotArea><c:legend><c:legendPos val="b"/></c:legend></c:chart></c:chartSpace>"#
    );
    let bytes = common::build_zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="/xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<x:workbook xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><x:sheets><x:sheet name="그림" sheetId="1" r:id="rId1"/></x:sheets></x:workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><x:sheetData/><x:drawing r:id="rId1"/></x:worksheet>"#,
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/drawing1.xml"/></Relationships>"#,
        ),
        ("xl/drawings/drawing1.xml", &drawing),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>"#,
        ),
        ("xl/charts/chart1.xml", &chart),
    ]);
    let a = rcell::from_bytes(bytes).unwrap();
    let sh = &a.sheets[0].shapes;
    assert_eq!(sh.len(), 2);
    assert_eq!(sh[0].name, "글상자 1");
    assert_eq!(sh[0].anchor.edit_as.as_deref(), Some("oneCell"));
    assert_eq!(sh[0].anchor.from.col_off, 9525);
    assert_eq!(sh[0].plain_text(), "안녕\nyoomin53");
    let t = sh[0].text.as_ref().unwrap();
    assert_eq!(t.anchor.as_deref(), Some("ctr"));
    assert_eq!(t.paragraphs[0].runs[0].bold, Some(true));
    assert_eq!(
        t.paragraphs[0].runs[0].color.as_ref().unwrap().mods,
        [("lumMod".to_string(), 75000)]
    );
    assert_eq!(sh[1].kind, rcell::model::ShapeKind::Chart);
    assert_eq!(sh[1].target.as_deref(), Some("xl/charts/chart1.xml"));
    let ch = sh[1].chart.as_ref().unwrap();
    assert_eq!((ch.kind.as_str(), ch.three_d), ("bar", true));
    assert_eq!(ch.bar_dir.as_deref(), Some("col"));
    assert_eq!(ch.title.as_deref(), Some("매출"));
    assert_eq!(ch.legend.as_deref(), Some("b"));
    let s = &ch.series[0];
    assert_eq!(s.name.as_deref(), Some("yoomin53"));
    assert_eq!(s.cat_ref.as_deref(), Some("그림!$B$1:$D$1"));
    assert_eq!(s.categories, ["1월", "2월", "3월"]);
    // 빠진 점(idx 1)은 None
    assert_eq!(s.values, [Some(10.0), None, Some(30.5)]);
    // 뷰어: SVG 막대 3개 자리 (값 없는 점은 높이 0)
    let html = rcell::render::sheet_table(&a, &a.sheets[0], Default::default());
    assert!(html.contains("<svg") && html.contains("매출"));

    // 저장 → 다시 읽기: 도형은 그대로, 차트 파트도 옮겨진다
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(
        b.sheets[0].shapes[1].target.as_deref(),
        Some("xl/charts/chart1.xml")
    );
    assert!(b.raw_parts.iter().any(|p| p.name == "xl/charts/chart1.xml"));

    // 원본 XML 이 없는 도형(레거시에서 온 것처럼)도 모델로 써서 같게 읽힌다
    let mut c = a.clone();
    for s in &mut c.sheets[0].shapes {
        s.raw = None;
    }
    c.sheets[0].shapes.truncate(1);
    let d = rcell::from_bytes(write(&c, Flavor::Cell).unwrap()).unwrap();
    let diffs = compare(&c, &d, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn conditional_formats_and_validations_roundtrip() {
    use rcell::model::{
        CfRule, Cfvo, Color, ColorScale, ConditionalFormat, DataValidation, Dxf, Fill, Font,
        IconSet,
    };
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.styles.dxfs.push(Dxf {
        font: Some(Font {
            bold: true,
            color: Some(Color {
                rgb: Some("FF9C0006".into()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        fill: Some(Fill {
            bg_color: Some(Color {
                rgb: Some("FFFFC7CE".into()),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    });
    let s = &mut a.sheets[0];
    s.conditional_formats = vec![
        ConditionalFormat {
            sqref: "B1:B3".into(),
            rules: vec![
                CfRule {
                    kind: "cellIs".into(),
                    priority: 1,
                    dxf_id: Some(0),
                    operator: Some("between".into()),
                    formulas: vec!["1".into(), "5".into()],
                    stop_if_true: true,
                    ..Default::default()
                },
                CfRule {
                    kind: "top10".into(),
                    priority: 2,
                    dxf_id: Some(0),
                    rank: Some(20),
                    percent: true,
                    bottom: true,
                    ..Default::default()
                },
            ],
        },
        ConditionalFormat {
            sqref: "C1:C3 E1".into(),
            rules: vec![
                CfRule {
                    kind: "colorScale".into(),
                    priority: 3,
                    color_scale: Some(ColorScale {
                        cfvos: vec![
                            Cfvo {
                                kind: "min".into(),
                                gte: true,
                                ..Default::default()
                            },
                            Cfvo {
                                kind: "percentile".into(),
                                val: Some("50".into()),
                                gte: true,
                            },
                        ],
                        colors: vec![
                            Color {
                                rgb: Some("FFF8696B".into()),
                                ..Default::default()
                            },
                            Color {
                                rgb: Some("FF63BE7B".into()),
                                ..Default::default()
                            },
                        ],
                    }),
                    ..Default::default()
                },
                CfRule {
                    kind: "iconSet".into(),
                    priority: 4,
                    icon_set: Some(IconSet {
                        set: "3Arrows".into(),
                        cfvos: vec![
                            Cfvo {
                                kind: "percent".into(),
                                val: Some("0".into()),
                                gte: true,
                            },
                            Cfvo {
                                kind: "percent".into(),
                                val: Some("33".into()),
                                gte: true,
                            },
                            Cfvo {
                                kind: "percent".into(),
                                val: Some("67".into()),
                                gte: false,
                            },
                        ],
                        show_value: false,
                        reverse: false,
                        percent: true,
                    }),
                    ..Default::default()
                },
                CfRule {
                    kind: "containsText".into(),
                    priority: 5,
                    operator: Some("containsText".into()),
                    text: Some("yoomin53".into()),
                    formulas: vec!["NOT(ISERROR(SEARCH(\"yoomin53\",C1)))".into()],
                    ..Default::default()
                },
            ],
        },
    ];
    s.data_validations = vec![DataValidation {
        sqref: "D1:D5".into(),
        kind: "list".into(),
        formula1: Some("\"가,나,다\"".into()),
        allow_blank: true,
        show_input: true,
        show_error: true,
        error_style: Some("warning".into()),
        prompt_title: Some("고르기".into()),
        prompt: Some("목록에서".into()),
        ..Default::default()
    }];
    let b = rcell::from_bytes(write(&a, Flavor::Cell).unwrap()).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.sheets[0].conditional_formats[1].rules.len(), 3);
    assert_eq!(b.styles.dxfs.len(), 1);
    // 뷰어: B 열(값 3.5 → 1–5 사이)은 굵은 빨강
    let html = rcell::render::sheet_table(&b, &b.sheets[0], Default::default());
    assert!(html.contains("font-weight:bold"));
}

#[test]
fn outline_auto_filter_and_protection_roundtrip() {
    use rcell::model::{
        AutoFilter, ColInfo, FilterColumn, RowInfo, SheetProtection, WorkbookProtection,
    };
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.protection = Some(WorkbookProtection {
        lock_structure: true,
        lock_windows: false,
        password: Some("CC1A".into()),
    });
    let s = &mut a.sheets[0];
    s.summary_above = true;
    s.filter_mode = true;
    s.cols.push(ColInfo {
        min: 6,
        max: 7,
        width: Some(9.140625),
        outline_level: 2,
        collapsed: true,
        ..Default::default()
    });
    s.rows.push(RowInfo {
        index: 9,
        outline_level: 1,
        hidden: true,
        ..Default::default()
    });
    let mut states = [false; 16];
    states[0] = true;
    states[12] = true; // sort 는 막고 나머지는 기본값과 다르게
    s.protection = Some(SheetProtection::from_states(Some("CC1A".into()), states));
    s.auto_filter = Some(AutoFilter {
        range: "A1:C3".into(),
        columns: vec![
            FilterColumn {
                col_id: 0,
                values: vec!["1".into(), "x".into()],
                blank: true,
                ..Default::default()
            },
            FilterColumn {
                col_id: 1,
                custom: vec![
                    ("notEqual".into(), "a*".into()),
                    ("greaterThan".into(), "5".into()),
                ],
                custom_and: true,
                ..Default::default()
            },
            FilterColumn {
                col_id: 2,
                top10: Some((false, true, 25.0)),
                ..Default::default()
            },
        ],
    });
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let sb = &b.sheets[0];
    assert_eq!(sb.auto_filter, a.sheets[0].auto_filter);
    assert_eq!(sb.protection, a.sheets[0].protection);
    assert_eq!(b.protection, a.protection);
    assert!(sb.summary_above && !sb.summary_left && sb.filter_mode);
}

#[test]
fn model_pictures_are_written_with_their_media() {
    use rcell::model::{Anchor, AnchorKind, Marker, RawPart, Shape, ShapeKind};
    // 레거시 그림 저장소에서 꺼낸 것처럼: 모델 도형 + xl/media 파트
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&3u32.to_be_bytes());
    png.extend_from_slice(&2u32.to_be_bytes());
    a.raw_parts.push(RawPart {
        name: "xl/media/image1.png".into(),
        content_type: Some("image/png".into()),
        data: png,
    });
    a.sheets[0].shapes.push(Shape {
        anchor: Anchor {
            kind: AnchorKind::TwoCell,
            edit_as: Some("oneCell".into()),
            from: Marker {
                col: 1,
                row: 1,
                ..Default::default()
            },
            to: Some(Marker {
                col: 3,
                row: 5,
                ..Default::default()
            }),
            ..Default::default()
        },
        kind: ShapeKind::Picture,
        id: 2,
        name: "그림 1".into(),
        descr: Some("yoomin53".into()),
        geometry: Some("rect".into()),
        target: Some("xl/media/image1.png".into()),
        ..Default::default()
    });
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let s = &b.sheets[0].shapes[0];
    assert_eq!(s.kind, ShapeKind::Picture);
    let part = b
        .raw_parts
        .iter()
        .find(|p| Some(&p.name) == s.target.as_ref())
        .unwrap();
    assert_eq!(
        rcell::image::info(&part.data).map(|i| (i.width, i.height)),
        Some((3, 2))
    );
}

#[test]
fn document_properties_roundtrip() {
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.properties = rcell::model::DocProperties {
        title: Some("자동차 경비 보고서".into()),
        subject: Some("경비".into()),
        creator: Some("yoomin53".into()),
        keywords: Some("rcell, 한셀".into()),
        description: Some("설명 <&>".into()),
        category: Some("양식".into()),
        created: Some("1997-03-01T10:49:08Z".into()),
        last_printed: Some("1997-05-05T14:10:37Z".into()),
        ..Default::default()
    };
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.properties, a.properties);
}

#[test]
fn margins_survive_truncation_to_hwpunit() {
    // 레거시에서 읽은 여백 = f32(3683 / 7200): ×7200 이 3682.99… 라 자르는 읽기(한셀)는 한 단위를 잃는다
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let v = (3683.0f64 / 7200.0) as f32 as f64;
    assert!(v * 7200.0 < 3683.0);
    a.sheets[0].page.margins = Some(rcell::model::PageMargins {
        left: 0.75,
        right: 0.75,
        top: 1.0,
        bottom: 1.0,
        header: v,
        footer: v,
    });
    let b = rcell::from_bytes(write(&a, Flavor::Cell).unwrap()).unwrap();
    let m = b.sheets[0].page.margins.as_ref().unwrap();
    assert_eq!((m.header * 7200.0).trunc(), 3683.0);
    assert_eq!((m.header as f32 as f64 * 7200.0).trunc(), 3683.0);
    assert!((m.header - v).abs() < 1e-5);
    assert_eq!(m.left, 0.75);
}
