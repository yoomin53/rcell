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
        comment: None,
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
    use rcell::model::{CellRef, Comment, CommentShape, RichText};
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
            ..Default::default()
        });
    }
    // 둘째 메모: 자리·색·선·그림자·셀 고정·맞춤 (메모 대조 실험의 값)
    let shape = CommentShape {
        anchor: [3, 5, 1, 7, 6, 20, 8, 3],
        fill_color: "#c0ffc0".into(),
        stroke_color: "#ff0000".into(),
        stroke_weight: 2.0,
        shadow: true,
        move_with_cells: true,
        size_with_cells: false,
        text_halign: Some("Right".into()),
        text_valign: Some("Bottom".into()),
        name: Some("메모 9".into()),
    };
    a.sheets[0].comments[1].shape = Some(shape.clone());
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.sheets[0].comments.len(), 2);
    assert_eq!(b.sheets[0].comments[1].shape, Some(shape));
    // 모양이 없는 메모는 셀 오른쪽 위에, 이름은 시트 안 순서로 (`메모 1`) 둔다
    assert_eq!(
        b.sheets[0].comments[0].shape,
        Some(CommentShape {
            name: Some("메모 1".into()),
            ..CommentShape::implied(CellRef { row: 0, col: 0 })
        })
    );
    // 한셀의 VML 꼴: 이름이 v:shape id, 시트 안 순서의 o:spid, 모든 시트 idmap 0 (메모 이름 실험)
    let vml = String::from_utf8(
        rcell::container::read_entry(
            &write(&a, Flavor::Cell).unwrap(),
            "xl/drawings/vmlDrawing1.vml",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(vml.contains(r#"<v:shape id="메모 9" style="position:absolute;visibility:visible;z-index:1" o:spid="_x0000_s2""#), "{vml}");
    assert!(vml.contains(r#"strokeweight="2.00pt""#), "{vml}");
    assert!(vml.contains(r#"<o:idmap data="0" v:ext="edit"/>"#), "{vml}");
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

    // 원본 XML 이 없는 도형·차트(레거시에서 온 것처럼)도 모델로 써서 같게 읽힌다 — 차트는 새 차트 파트로
    let mut c = a.clone();
    for s in &mut c.sheets[0].shapes {
        s.raw = None;
    }
    c.raw_parts.clear();
    let d = rcell::from_bytes(write(&c, Flavor::Cell).unwrap()).unwrap();
    let diffs = compare(&c, &d, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(d.sheets[0].shapes[1].chart, c.sheets[0].shapes[1].chart);
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
                        custom_icons: Vec::new(),
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
        range: "A1:E3".into(),
        columns: vec![
            FilterColumn {
                col_id: 3,
                dynamic: Some("thisWeek".into()),
                dynamic_val: Some(46311.0),
                dynamic_max_val: Some(46318.0),
                ..Default::default()
            },
            FilterColumn {
                col_id: 4,
                dynamic: Some("aboveAverage".into()),
                dynamic_val: Some(72.5),
                ..Default::default()
            },
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
                filter_val: Some(-3.0),
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

/// 모델의 차트(레거시에서 읽은 것처럼 원본 XML 이 없는 것)는 한셀이 레거시 차트를 읽어 쓰는 꼴의 앵커 +
/// 새 차트 파트로 쓰고, 다시 읽으면 같은 차트다.
#[test]
fn model_charts_are_written_in_hancell_form() {
    use rcell::model::{
        Anchor, AnchorKind, Chart, ChartAxis, ChartCache, ChartColor, ChartData, ChartFill,
        ChartLine, ChartPoint, ChartSeries, ChartShapeProps, Marker, Shape, ShapeKind,
    };
    let data = |kind: &str, f: &str, vals: &[&str]| ChartData {
        kind: kind.into(),
        formula: Some(f.into()),
        cache: Some(ChartCache {
            format_code: (kind == "numRef").then(|| "G/표준".into()),
            pt_count: Some(vals.len() as u32),
            points: vals
                .iter()
                .enumerate()
                .map(|(i, v)| ChartPoint {
                    idx: i as u32,
                    format_code: None,
                    value: v.to_string(),
                })
                .collect(),
        }),
    };
    let mut chart = Chart {
        kind: "bar".into(),
        bar_dir: Some("col".into()),
        grouping: Some("clustered".into()),
        series: vec![ChartSeries {
            idx: Some(0),
            order: Some(0),
            sp_pr: Some(ChartShapeProps {
                fill: Some(ChartFill::Solid(ChartColor::srgb("73B1D1"))),
                line: Some(ChartLine::hcell(9525, ChartFill::None)),
            }),
            cat: Some(data("strRef", "Sheet1!$A$2:$A$3", &["가", "나"])),
            val: Some(data("numRef", "Sheet1!$B$2:$B$3", &["10", "20"])),
            ..Default::default()
        }],
        axes: vec![
            ChartAxis {
                kind: "catAx".into(),
                id: 0,
                cross_ax: Some(1),
                ..Default::default()
            },
            ChartAxis {
                kind: "valAx".into(),
                id: 1,
                cross_ax: Some(0),
                ..Default::default()
            },
        ],
        legend: Some("r".into()),
        c14_style: Some(102),
        style: Some(2),
        ..Default::default()
    };
    chart.group.ax_ids = vec![0, 1];
    chart.refresh_summary();
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let anchor = Anchor {
        kind: AnchorKind::TwoCell,
        from: Marker {
            col: 2,
            row: 1,
            ..Default::default()
        },
        to: Some(Marker {
            col: 6,
            row: 10,
            col_off: 9525,
            ..Default::default()
        }),
        ..Default::default()
    };
    for _ in 0..2 {
        a.sheets[0].shapes.push(Shape {
            anchor: anchor.clone(),
            kind: ShapeKind::Chart,
            chart: Some(chart.clone()),
            ..Default::default()
        });
    }
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out.clone()).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let charts: Vec<_> = b.sheets[0]
        .shapes
        .iter()
        .filter(|s| s.kind == ShapeKind::Chart)
        .collect();
    assert_eq!(charts.len(), 2);
    assert_eq!(charts[0].target.as_deref(), Some("xl/charts/chart1.xml"));
    assert_eq!(charts[1].target.as_deref(), Some("xl/charts/chart2.xml"));
    assert_eq!(charts[0].chart.as_ref(), Some(&chart));

    // 한셀 꼴: graphicFrame fPublished="0", 번호 2·3, 시트 안 절대 위치, clientData 잠금·인쇄, 차트 콘텐츠 형식
    let mut pkg = rcell::Package::from_bytes(out).unwrap();
    let ct = pkg.read_string("[Content_Types].xml").unwrap().unwrap();
    assert!(ct.contains(r#"<Override PartName="/xl/charts/chart2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#), "{ct}");
    let drawing = pkg
        .part_names()
        .iter()
        .find(|n| n.starts_with("xl/drawings/drawing"))
        .cloned()
        .unwrap();
    let x = pkg.read_string(&drawing).unwrap().unwrap();
    assert!(x.contains(r#"<xdr:graphicFrame fPublished="0"><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name=""/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=""#), "{x}");
    assert!(x.contains(r#"<xdr:cNvPr id="3" name=""/>"#), "{x}");
    assert!(x.contains(r#"<c:chart r:id="rIdChart1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData fLocksWithSheet="1" fPrintsWithSheet="1"/></xdr:twoCellAnchor>"#), "{x}");
    // 위치는 앞 열·행의 픽셀 합 (기본 열·행), 크기는 끝 칸 + 오프셋까지
    let sheet = &a.sheets[0];
    let mdw = rcell::units::max_digit_width(&a);
    let col = |c: u32| {
        (0..c)
            .map(|i| i64::from(rcell::units::sheet_col_px(sheet, i, mdw)))
            .sum::<i64>()
            * 9525
    };
    let row = |r: u32| {
        (0..r)
            .map(|i| i64::from(rcell::units::sheet_row_px(sheet, i)))
            .sum::<i64>()
            * 9525
    };
    let xfrm = format!(
        r#"<a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/>"#,
        col(2),
        row(1),
        col(6) + 9525 - col(2),
        row(10) - row(1)
    );
    assert!(x.contains(&xfrm), "{xfrm}\n{x}");
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
    a.total_time = Some(979);
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.properties, a.properties);
    assert_eq!(b.total_time, Some(979));
}

#[test]
fn workbook_view_and_code_name_roundtrip() {
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.code_name = Some("MyBook".into());
    a.view = Some(rcell::model::WorkbookView {
        x_window: -15,
        y_window: 30,
        window_width: 12000,
        window_height: 8000,
        tab_ratio: 250,
        first_sheet: 1,
        active_tab: 0,
        show_horizontal_scroll: false,
        show_vertical_scroll: true,
        show_sheet_tabs: false,
    });
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert_eq!(b.view, a.view);
    assert_eq!(b.code_name.as_deref(), Some("MyBook"));
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

#[test]
fn x14_conditional_formats_roundtrip() {
    use rcell::model::{CfRule, Cfvo, Color, ConditionalFormat, DataBar, DataBarExt, IconSet};
    let rgb = |v: &str| Color {
        rgb: Some(v.into()),
        ..Default::default()
    };
    let cfvo = |kind: &str, val: Option<&str>| Cfvo {
        kind: kind.into(),
        val: val.map(str::to_string),
        gte: true,
    };
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    a.sheets[0].conditional_formats = vec![
        ConditionalFormat {
            sqref: "B1:B3".into(),
            rules: vec![CfRule {
                kind: "dataBar".into(),
                priority: 1,
                data_bar: Some(DataBar {
                    cfvos: vec![cfvo("min", None), cfvo("max", None)],
                    color: Some(rgb("ff638ec6")),
                    show_value: false,
                    min_length: Some(5),
                    max_length: Some(95),
                    ext: Some(DataBarExt {
                        cfvos: vec![cfvo("autoMin", None), cfvo("num", Some("40"))],
                        border: true,
                        gradient: false,
                        direction: Some("rightToLeft".into()),
                        negative_border_same_as_positive: false,
                        axis_position: Some("middle".into()),
                        border_color: Some(rgb("ff1f4e79")),
                        negative_fill_color: Some(rgb("ffff0000")),
                        axis_color: Some(Color {
                            indexed: Some(64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                }),
                ..Default::default()
            }],
        },
        ConditionalFormat {
            sqref: "C1:C3".into(),
            rules: vec![CfRule {
                kind: "iconSet".into(),
                priority: 2,
                icon_set: Some(IconSet {
                    set: "3Arrows".into(),
                    cfvos: vec![
                        cfvo("percent", Some("0")),
                        cfvo("num", Some("2")),
                        cfvo("percentile", Some("70")),
                    ],
                    show_value: true,
                    reverse: true,
                    percent: true,
                    custom_icons: vec![("3Flags".into(), 0), ("NoIcons".into(), 0)],
                }),
                ..Default::default()
            }],
        },
    ];
    let out = write(&a, Flavor::Cell).unwrap();
    let mut pkg = rcell::open_package(out.clone()).unwrap();
    let sheet = pkg
        .read_string("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(sheet.contains("<x14:dataBar minLength=\"5\" maxLength=\"95\" showValue=\"0\" border=\"1\" gradient=\"0\" direction=\"rightToLeft\" negativeBarBorderColorSameAsPositive=\"0\" axisPosition=\"middle\">"));
    assert!(sheet.contains("<x14:cfIcon iconSet=\"NoIcons\" iconId=\"0\"/>"));
    // 사용자 지정 아이콘이 있는 규칙은 x14 구역에만 쓴다
    assert!(!sheet.contains("<x:iconSet"));
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let bar = b.sheets[0].conditional_formats[0].rules[0]
        .data_bar
        .as_ref()
        .unwrap();
    assert_eq!(
        bar.ext,
        a.sheets[0].conditional_formats[0].rules[0]
            .data_bar
            .as_ref()
            .unwrap()
            .ext
    );
    assert!(b.sheets[0].conditional_formats[1].rules[0].x14);
}

#[test]
fn array_formulas_data_tables_and_names_roundtrip() {
    use rcell::model::{
        Cell, CellRef, CellValue, DataTableInput, DefinedName, Formula, FormulaKind,
    };
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let s = &mut a.sheets[0];
    s.cells.retain(|c| c.r.row > 4 || c.r.col > 4);
    let cell = |row, col, formula| Cell {
        r: CellRef { row, col },
        style: 0,
        value: CellValue::Number(1.0),
        formula,
    };
    s.cells.push(cell(
        0,
        0,
        Some(Formula {
            text: "B1:B2*2".into(),
            kind: FormulaKind::Array,
            range: Some("A1:A2".into()),
            ..Default::default()
        }),
    ));
    s.cells.push(cell(1, 0, None));
    s.cells.push(cell(
        2,
        3,
        Some(Formula {
            kind: FormulaKind::DataTable,
            range: Some("D3:E4".into()),
            data_table: Some(DataTableInput {
                two_d: true,
                row_input: true,
                r1: Some("A1".into()),
                r2: Some("B1".into()),
                ..Default::default()
            }),
            ..Default::default()
        }),
    ));
    s.cells.sort_by_key(|c| (c.r.row, c.r.col));
    a.defined_names.push(DefinedName {
        name: "Hidden".into(),
        local_sheet_id: Some(0),
        hidden: true,
        formula: "'데이터'!$A$1".into(),
        comment: Some("설명 글".into()),
    });
    let out = write(&a, Flavor::Cell).unwrap();
    let mut pkg = rcell::open_package(out.clone()).unwrap();
    let sheet = pkg
        .read_string("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(sheet.contains("<x:f t=\"array\" ref=\"A1:A2\">B1:B2*2</x:f>"));
    // 한셀처럼 2변수 표에는 dtr 을 쓰지 않는다
    assert!(sheet.contains("<x:f t=\"dataTable\" ref=\"D3:E4\" dt2D=\"1\" r1=\"A1\" r2=\"B1\"/>"));
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let n = b.defined_names.iter().find(|n| n.name == "Hidden").unwrap();
    assert!(n.hidden);
    assert_eq!(n.comment.as_deref(), Some("설명 글"));
}

#[test]
fn model_shape_details_roundtrip() {
    use rcell::model::{
        Anchor, DrawColor, LineEnd, Marker, Outline, Paint, Shape, ShapeKind, ShapeShadow,
    };
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let rgb = |v: &str| DrawColor {
        value: format!("srgb:{v}"),
        mods: Vec::new(),
    };
    // 레거시에서 읽은 것처럼 원문(raw) 없이 모델만 있는 도형: 그림자·잠금·clientData·선 끝/겹선/이음/화살표
    a.sheets[0].shapes.push(Shape {
        anchor: Anchor {
            from: Marker {
                col: 1,
                row: 1,
                ..Default::default()
            },
            to: Some(Marker {
                col: 4,
                row: 3,
                ..Default::default()
            }),
            ..Default::default()
        },
        kind: ShapeKind::Shape,
        id: 2,
        name: "도형 1".into(),
        geometry: Some("rect".into()),
        fill: Some(Paint::Solid(rgb("FFF2CC"))),
        line: Some(Outline {
            width: Some(38100),
            fill: Some(Paint::Solid(rgb("FF0000"))),
            dash: Some("dash".into()),
            cap: Some("rnd".into()),
            compound: Some("dbl".into()),
            join: Some("bevel".into()),
            head: Some(LineEnd {
                kind: Some("triangle".into()),
                w: Some("lg".into()),
                len: Some("sm".into()),
            }),
            tail: None,
        }),
        shadow: Some(ShapeShadow {
            dist: 25399,
            dir: 13_500_000,
            blur: 0,
            align: Some("t".into()),
            rot_with_shape: false,
            color: DrawColor {
                value: "srgb:808080".into(),
                mods: vec![("alpha".into(), 50000)],
            },
        }),
        locks: vec!["noMove".into(), "noRot".into()],
        locks_with_sheet: Some(true),
        prints_with_sheet: Some(false),
        ..Default::default()
    });
    let out = write(&a, Flavor::Cell).unwrap();
    let mut pkg = rcell::open_package(out.clone()).unwrap();
    let d = pkg
        .read_string("xl/drawings/drawing1.xml")
        .unwrap()
        .unwrap();
    assert!(d.contains("<a:spLocks noMove=\"1\" noRot=\"1\"/>"));
    assert!(d.contains("<a:ln w=\"38100\" cap=\"rnd\" cmpd=\"dbl\">"));
    assert!(d.contains(
        "<a:effectLst><a:outerShdw dist=\"25399\" dir=\"13500000\" algn=\"t\" rotWithShape=\"0\">"
    ));
    assert!(d.contains("<xdr:clientData fLocksWithSheet=\"1\" fPrintsWithSheet=\"0\"/>"));
    let b = rcell::from_bytes(out).unwrap();
    let diffs = compare(&a, &b, Options::default());
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

#[test]
fn unknown_drawing_anchors_are_kept() {
    // mc:AlternateContent 로 감싼 앵커와 모르는 개체(contentPart)를 담은 앵커는 원문 그대로 다시 쓴다
    let anchor = |inner: &str| {
        format!(
            "<xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>{inner}<xdr:clientData/></xdr:twoCellAnchor>"
        )
    };
    let sp = |name: &str| {
        format!(
            "<xdr:sp><xdr:nvSpPr><xdr:cNvPr id=\"2\" name=\"{name}\"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:sp>"
        )
    };
    let drawing = format!(
        "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>{}</xdr:wsDr>",
        anchor(&sp("선택")),
        anchor(&sp("대체")),
        anchor(
            "<xdr:contentPart xmlns:p14=\"http://schemas.microsoft.com/office/powerpoint/2010/main\" r:id=\"rId9\"/>"
        )
    );
    let bytes = common::build_zip(&[
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#,
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
    ]);
    let a = rcell::from_bytes(bytes).unwrap();
    let shapes = &a.sheets[0].shapes;
    assert_eq!(shapes.len(), 2);
    assert_eq!(shapes[0].name, "선택");
    assert_eq!(shapes[1].kind, rcell::model::ShapeKind::Other);
    let out = write(&a, Flavor::Cell).unwrap();
    let mut pkg = rcell::open_package(out.clone()).unwrap();
    let d = pkg
        .read_string("xl/drawings/drawing1.xml")
        .unwrap()
        .unwrap();
    assert!(d.contains("<mc:AlternateContent"));
    assert!(d.contains("xdr:contentPart"));
    let b = rcell::from_bytes(out).unwrap();
    assert_eq!(b.sheets[0].shapes.len(), 2);
}

#[test]
fn package_parts_roundtrip() {
    const SML: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
    const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const PR: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const T: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.";
    let rels = |list: &[(&str, &str, &str)]| {
        let mut s = format!("<Relationships xmlns=\"{PR}\">");
        for (id, ty, target) in list {
            s.push_str(&format!(
                "<Relationship Id=\"{id}\" Type=\"{ty}\" Target=\"{target}\"/>"
            ));
        }
        s + "</Relationships>"
    };
    let ct = format!(
        "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"bin\" ContentType=\"{T}printerSettings\"/><Override PartName=\"/xl/tables/table1.xml\" ContentType=\"{T}table+xml\"/><Override PartName=\"/xl/pivotTables/pivotTable1.xml\" ContentType=\"{T}pivotTable+xml\"/><Override PartName=\"/xl/pivotCache/pivotCacheDefinition1.xml\" ContentType=\"{T}pivotCacheDefinition+xml\"/><Override PartName=\"/xl/pivotCache/pivotCacheRecords1.xml\" ContentType=\"{T}pivotCacheRecords+xml\"/><Override PartName=\"/xl/connections.xml\" ContentType=\"{T}connections+xml\"/><Override PartName=\"/xl/queryTables/queryTable1.xml\" ContentType=\"{T}queryTable+xml\"/><Override PartName=\"/xl/vbsProject.bin\" ContentType=\"application/vnd.hancom-office.vbsProject\"/></Types>"
    );
    let wb_rels = rels(&[
        ("rId1", &format!("{R}/worksheet"), "worksheets/sheet1.xml"),
        ("rId2", &format!("{R}/connections"), "connections.xml"),
        ("rId3", &format!("{R}/customXml"), "../customXml/item1.xml"),
        (
            "rId4",
            &format!("{R}/pivotCacheDefinition"),
            "pivotCache/pivotCacheDefinition1.xml",
        ),
        (
            "rId5",
            "http://schemas.hancom.com/office/relationships/vbsProject",
            "vbsProject.bin",
        ),
    ]);
    let sheet_rels = rels(&[
        ("rId1", &format!("{R}/table"), "../tables/table1.xml"),
        (
            "rId2",
            &format!("{R}/pivotTable"),
            "../pivotTables/pivotTable1.xml",
        ),
        (
            "rId3",
            &format!("{R}/queryTable"),
            "../queryTables/queryTable1.xml",
        ),
        (
            "rId4",
            &format!("{R}/printerSettings"),
            "../printerSettings/printerSettings1.bin",
        ),
    ]);
    let pt_rels = rels(&[(
        "rId1",
        &format!("{R}/pivotCacheDefinition"),
        "../pivotCache/pivotCacheDefinition1.xml",
    )]);
    let pc_rels = rels(&[(
        "rId1",
        &format!("{R}/pivotCacheRecords"),
        "pivotCacheRecords1.xml",
    )]);
    let cx_rels = rels(&[("rId1", &format!("{R}/customXmlProps"), "itemProps1.xml")]);
    let workbook = format!(
        "<x:workbook xmlns:x=\"{SML}\" xmlns:r=\"{R}\"><x:sheets><x:sheet name=\"표\" sheetId=\"1\" r:id=\"rId1\"/></x:sheets><x:calcPr/><x:pivotCaches><x:pivotCache cacheId=\"3\" r:id=\"rId4\"/></x:pivotCaches></x:workbook>"
    );
    let sheet = format!(
        "<x:worksheet xmlns:x=\"{SML}\" xmlns:r=\"{R}\"><x:sheetData><x:row r=\"1\"><x:c r=\"A1\" t=\"inlineStr\"><x:is><x:t>이름</x:t></x:is></x:c><x:c r=\"B1\" t=\"inlineStr\"><x:is><x:t>수량</x:t></x:is></x:c></x:row><x:row r=\"2\"><x:c r=\"B2\"><x:v>5</x:v></x:c></x:row></x:sheetData><x:pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/><x:pageSetup paperSize=\"9\" orientation=\"landscape\" r:id=\"rId4\"/><x:tableParts count=\"1\"><x:tablePart r:id=\"rId1\"/></x:tableParts></x:worksheet>"
    );
    let table = format!(
        "<table xmlns=\"{SML}\" id=\"4\" name=\"Table4\" displayName=\"Table4\" ref=\"A1:B3\" totalsRowCount=\"1\" totalsRowDxfId=\"0\"><autoFilter ref=\"A1:B2\"><filterColumn colId=\"1\"><filters><filter val=\"5\"/></filters></filterColumn></autoFilter><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"이름\" totalsRowLabel=\"합계\"/><tableColumn id=\"2\" name=\"수량\" totalsRowFunction=\"custom\" dataDxfId=\"0\"><calculatedColumnFormula>Table4[[#This Row],[수량]]*2</calculatedColumnFormula><totalsRowFormula>SUM(Table4[수량])</totalsRowFormula></tableColumn></tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>"
    );
    let pivot = format!(
        "<pivotTableDefinition xmlns=\"{SML}\" name=\"피벗1\" cacheId=\"3\" dataCaption=\"값\"><location ref=\"D1:E3\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/><pivotFields count=\"2\"><pivotField axis=\"axisRow\" showAll=\"0\"><items count=\"2\"><item x=\"0\"/><item t=\"default\"/></items></pivotField><pivotField dataField=\"1\" showAll=\"0\"/></pivotFields><rowFields count=\"1\"><field x=\"0\"/></rowFields><dataFields count=\"1\"><dataField name=\"합계 : 수량\" fld=\"1\" baseField=\"0\" baseItem=\"0\"/></dataFields></pivotTableDefinition>"
    );
    let cache = format!(
        "<pivotCacheDefinition xmlns=\"{SML}\" xmlns:r=\"{R}\" r:id=\"rId1\" recordCount=\"1\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"A1:B2\" sheet=\"표\"/></cacheSource><cacheFields count=\"2\"><cacheField name=\"이름\" numFmtId=\"0\"><sharedItems count=\"1\"><m/></sharedItems></cacheField><cacheField name=\"수량\" numFmtId=\"0\"><sharedItems containsNumber=\"1\" minValue=\"5\" maxValue=\"5\"/></cacheField></cacheFields></pivotCacheDefinition>"
    );
    let records = format!(
        "<pivotCacheRecords xmlns=\"{SML}\" count=\"1\"><r><x v=\"0\"/><n v=\"5\"/></r></pivotCacheRecords>"
    );
    let connections = format!(
        "<connections xmlns=\"{SML}\"><connection id=\"1\" name=\"q\" type=\"6\"/></connections>"
    );
    let query = format!("<queryTable xmlns=\"{SML}\" name=\"q\" connectionId=\"1\"/>");
    let root_rels = rels(&[("rId1", &format!("{R}/officeDocument"), "xl/workbook.xml")]);
    let base = common::build_zip(&[
        ("[Content_Types].xml", &ct),
        ("_rels/.rels", &root_rels),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &wb_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/worksheets/_rels/sheet1.xml.rels", &sheet_rels),
        ("xl/tables/table1.xml", &table),
        ("xl/pivotTables/pivotTable1.xml", &pivot),
        ("xl/pivotTables/_rels/pivotTable1.xml.rels", &pt_rels),
        ("xl/pivotCache/pivotCacheDefinition1.xml", &cache),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            &pc_rels,
        ),
        ("xl/pivotCache/pivotCacheRecords1.xml", &records),
        ("xl/connections.xml", &connections),
        ("xl/queryTables/queryTable1.xml", &query),
        (
            "customXml/item1.xml",
            "<rc:data xmlns:rc=\"urn:rcell:test\"/>",
        ),
        ("customXml/_rels/item1.xml.rels", &cx_rels),
        (
            "customXml/itemProps1.xml",
            "<ds:datastoreItem ds:itemID=\"{00000000-0000-0000-0000-000000000001}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"/>",
        ),
    ]);
    // 매크로 프로젝트 스트림: 머리 0x104 바이트 + 글자 수 u32 + UTF-16
    let mut stream = vec![0u8; 0x104];
    stream[..8].copy_from_slice(&[1, 1, 1, 0, 0, 1, 0x20, 2]);
    let text: Vec<u16> = "Sub A()\nEnd Sub\n".encode_utf16().collect();
    stream.extend_from_slice(&(text.len() as u32).to_le_bytes());
    for u in &text {
        stream.extend_from_slice(&u.to_le_bytes());
    }
    let vbs = rcell::cfb_write::write_streams(&[("_VBS_PROJECT_W", &stream)]).unwrap();
    let printer = vec![7u8; 220];
    let bytes = rcell::container::replace_zip_parts(
        &base,
        &[
            ("xl/vbsProject.bin".into(), vbs.clone()),
            (
                "xl/printerSettings/printerSettings1.bin".into(),
                printer.clone(),
            ),
        ],
    )
    .unwrap();

    // 이 패키지에는 styles.xml 이 없다 (rcell 은 기본 서식을 쓴다)
    let no_styles = Options {
        skip_styles: true,
        ..Default::default()
    };
    let a = rcell::from_bytes(bytes).unwrap();
    let sh = &a.sheets[0];
    let t = &sh.tables[0];
    assert_eq!(
        (t.id, t.name.as_str(), t.range.as_str()),
        (4, "Table4", "A1:B3")
    );
    assert_eq!((t.header_rows, t.totals_rows, t.dxfs[2]), (1, 1, Some(0)));
    assert_eq!(t.auto_filter.as_ref().unwrap().columns[0].values, ["5"]);
    assert_eq!(t.columns[0].totals_label.as_deref(), Some("합계"));
    assert_eq!(t.columns[1].totals_function.as_deref(), Some("custom"));
    assert_eq!(
        t.columns[1].totals_formula.as_deref(),
        Some("SUM(Table4[수량])")
    );
    assert_eq!(t.columns[1].dxfs[1], Some(0));
    assert_eq!(
        t.style.as_ref().unwrap().name.as_deref(),
        Some("TableStyleMedium2")
    );
    let kinds = |l: &[rcell::model::PartLink]| {
        l.iter()
            .map(|l| l.rel_type.rsplit("/").next().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(&sh.part_links),
        ["pivotTable", "queryTable", "printerSettings"]
    );
    assert_eq!(
        kinds(&a.part_links),
        [
            "connections",
            "customXml",
            "pivotCacheDefinition",
            "vbsProject"
        ]
    );
    assert_eq!(a.part_links[2].cache_id, Some(3));
    assert_eq!(a.hancom.vbs_project.as_deref(), Some(&stream[..]));

    // XML 로 쓰면 파트·관계·참조 요소가 그대로 남는다
    let out = write(&a, Flavor::Cell).unwrap();
    let b = rcell::from_bytes(out.clone()).unwrap();
    let diffs = compare(&a, &b, no_styles);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    let part = |w: &rcell::Workbook, n: &str| {
        w.raw_parts
            .iter()
            .find(|p| p.name == n)
            .map(|p| p.data.clone())
    };
    assert_eq!(part(&b, "xl/vbsProject.bin"), Some(vbs.clone()));
    assert_eq!(
        part(&b, "xl/printerSettings/printerSettings1.bin"),
        Some(printer)
    );
    assert_eq!(
        part(&b, "xl/tables/table1.xml"),
        Some(table.as_bytes().to_vec())
    );
    assert!(part(&b, "customXml/itemProps1.xml").is_some());
    assert!(part(&b, "xl/pivotCache/pivotCacheRecords1.xml").is_some());
    assert_eq!(b.part_links[2].cache_id, Some(3));
    let mut pkg = rcell::open_package(out).unwrap();
    let x = pkg
        .read_string("xl/worksheets/sheet1.xml")
        .unwrap()
        .unwrap();
    assert!(
        x.contains("<x:tableParts count=\"1\"><x:tablePart r:id=\"rIdT1\"/></x:tableParts>"),
        "{x}"
    );
    assert!(x.contains(" r:id=\"rIdP3\"/>"), "{x}");
    let w = pkg.read_string("xl/workbook.xml").unwrap().unwrap();
    assert!(
        w.contains("<x:pivotCaches><x:pivotCache cacheId=\"3\" r:id=\"rIdX3\"/></x:pivotCaches>"),
        "{w}"
    );

    // Excel 형식에는 매크로 프로젝트를 쓰지 않는다
    let c = rcell::from_bytes(write(&a, Flavor::Xlsx).unwrap()).unwrap();
    assert!(part(&c, "xl/vbsProject.bin").is_none());
    assert!(c.hancom.vbs_project.is_none());
    assert_eq!(c.part_links.len(), 3);

    // 레거시처럼 파트 없이 스트림과 모델만 있으면: 매크로는 복합 문서로, 표는 모델로 쓴다
    let mut d = a.clone();
    d.raw_parts.clear();
    d.part_links.clear();
    d.sheets[0].part_links.clear();
    d.sheets[0].tables[0].part = None;
    let e = rcell::from_bytes(write(&d, Flavor::Cell).unwrap()).unwrap();
    assert_eq!(e.hancom.vbs_project.as_deref(), Some(&stream[..]));
    let mut expect = d.sheets[0].tables.clone();
    expect[0].part = Some("xl/tables/table1.xml".into());
    assert_eq!(e.sheets[0].tables, expect);
    let diffs = compare(&d, &e, no_styles);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}
