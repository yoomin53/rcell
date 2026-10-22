// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 편집 시험. 기대값은 한셀 COM 대조 실험(같은 내용의 문서에 `Rows("5:6").Insert` 등을 실행해 저장한 결과)에서
//! 가져온 사실이다 — 한컴 파일은 저장소에 넣지 않고 같은 모양의 문서를 여기서 만든다.

use super::*;
use crate::model::{
    Anchor, CfRule, Chart, ChartData, ChartSeries, Comment, CommentShape, ConditionalFormat,
    DataValidation, DefinedName, Hyperlink, Marker, RichText, Shape, ShapeKind, Styles, Xf,
};

const F1: [&str; 40] = [
    "A5",
    "$A$5",
    "SUM(A1:A10)",
    "SUM(A5:A10)",
    "SUM(A1:A4)",
    "A4",
    "SUM(A:A)",
    "SUM(5:5)",
    "SUM(4:6)",
    "Sheet2!A5",
    "SUM(Sheet1!A5:A6)",
    "A$5+$A5",
    "INDIRECT(\"A5\")",
    "OFFSET(A1,4,0)",
    "A1048576",
    "SUM(A:A)",
    "SUM(MyRange)",
    "A5*2+$B$6",
    "SUM(A5:B6 B6:C7)",
    "SUM(C:D)",
    "SUM(B:C)",
    "SUM(D:F)",
    "C5",
    "SUM(A3:F3)",
    "XFD1",
    "SUM(C1:D1)",
    "SUM(A5:A6)",
    "SUM(A6:A7)",
    "'Data 3'!A5",
    "SUM('Data 3'!A1:A10)",
    "MyCell*2",
    "SUM(Sheet1:Sheet2!A5)",
    "SUM(A5,A6,A7)",
    "ROW(A5)",
    "COLUMN(D1)",
    "SUM($A$5:$A$6)",
    "SUM(A1:B2,C5:D6)",
    "E5",
    "B5",
    "SUM(B5:E5)",
];

const F2: [&str; 9] = [
    "Sheet1!A5",
    "SUM(Sheet1!A1:A10)",
    "Sheet1!$A$5",
    "Sheet1!A4",
    "SUM(Sheet1!A5:A6)",
    "A5",
    "Sheet1!C5",
    "SUM(Sheet1!C:D)",
    "SUM(Sheet1!5:6)",
];

fn styles() -> Styles {
    Styles {
        fonts: vec![crate::model::Font {
            name: Some("맑은 고딕".into()),
            size: Some(11.0),
            ..Default::default()
        }],
        fills: vec![crate::model::Fill::default()],
        borders: vec![crate::model::Border::default()],
        cell_xfs: vec![Xf::default()],
        ..Default::default()
    }
}

fn put(sheet: &mut Sheet, a1: &str, value: CellValue, formula: Option<&str>) {
    let r = CellRef::parse(a1).unwrap();
    let c = cell_mut(sheet, r);
    c.value = value;
    c.formula = formula.map(|t| Formula {
        text: t.into(),
        ..Default::default()
    });
}

/// 한셀 대조 실험의 기본 문서와 같은 모양
fn refbase() -> Workbook {
    let mut s1 = Sheet {
        name: "Sheet1".into(),
        default_col_width: Some(9.00390625),
        ..Default::default()
    };
    for r in 1..=20u32 {
        for c in 0..6u32 {
            let a1 = format!("{}{r}", CellRef::col_name(c));
            put(
                &mut s1,
                &a1,
                CellValue::Number((r * 10 + c + 1) as f64),
                None,
            );
        }
    }
    for (i, f) in F1.iter().enumerate() {
        put(&mut s1, &format!("H{}", i + 1), CellValue::Empty, Some(f));
    }
    s1.merges = ["J4:K6", "A22:B23", "C25:E25"]
        .iter()
        .map(|m| CellRange::parse(m).unwrap())
        .collect();
    s1.conditional_formats = vec![
        ConditionalFormat {
            sqref: "B1:B10".into(),
            rules: vec![CfRule {
                kind: "expression".into(),
                formulas: vec!["C1>$A$5".into()],
                ..Default::default()
            }],
        },
        ConditionalFormat {
            sqref: "C3:D8".into(),
            rules: vec![CfRule {
                kind: "cellIs".into(),
                formulas: vec!["50".into()],
                ..Default::default()
            }],
        },
    ];
    s1.data_validations = vec![
        DataValidation {
            sqref: "L5:L8".into(),
            kind: "list".into(),
            ..Default::default()
        },
        DataValidation {
            sqref: "M3:M12".into(),
            kind: "whole".into(),
            ..Default::default()
        },
    ];
    s1.comments = vec![Comment {
        cell: CellRef::parse("N6").unwrap(),
        author: String::new(),
        text: RichText {
            text: "메모".into(),
            runs: Vec::new(),
        },
        visible: false,
        shape: Some(CommentShape::hcell_default([13, 14, 3, 10, 14, 69, 7, 1])),
    }];
    s1.hyperlinks = vec![Hyperlink {
        range: "O7:O7".into(),
        ..Default::default()
    }];
    let series = |r: &str| ChartSeries {
        val: Some(ChartData {
            kind: "numRef".into(),
            formula: Some(r.into()),
            cache: None,
        }),
        ..Default::default()
    };
    s1.shapes = vec![Shape {
        kind: ShapeKind::Chart,
        anchor: Anchor {
            from: Marker {
                col: 9,
                col_off: 171449,
                row: 18,
                row_off: 38100,
            },
            to: Some(Marker {
                col: 14,
                col_off: 552450,
                row: 30,
                row_off: 57150,
            }),
            ..Default::default()
        },
        raw: Some("<xdr:twoCellAnchor><xdr:from><xdr:col>9</xdr:col><xdr:colOff>171449</xdr:colOff><xdr:row>18</xdr:row><xdr:rowOff>38100</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>552450</xdr:colOff><xdr:row>30</xdr:row><xdr:rowOff>57150</xdr:rowOff></xdr:to><xdr:graphicFrame/><xdr:clientData/></xdr:twoCellAnchor>".into()),
        target: Some("xl/charts/chart1.xml".into()),
        chart: Some(Chart {
            kind: "bar".into(),
            series: vec![series("Sheet1!$A$1:$A$10"), series("Sheet1!$B$1:$B$10")],
            ..Default::default()
        }),
        ..Default::default()
    }];
    let mut s2 = Sheet {
        name: "Sheet2".into(),
        ..Default::default()
    };
    let mut s3 = Sheet {
        name: "Data 3".into(),
        ..Default::default()
    };
    for r in 1..=10u32 {
        put(
            &mut s2,
            &format!("A{r}"),
            CellValue::Number((r * 100) as f64),
            None,
        );
        put(
            &mut s3,
            &format!("A{r}"),
            CellValue::Number((r * 1000) as f64),
            None,
        );
    }
    for (i, f) in F2.iter().enumerate() {
        put(&mut s2, &format!("C{}", i + 1), CellValue::Empty, Some(f));
    }
    let name = |n: &str, f: &str| DefinedName {
        name: n.into(),
        formula: f.into(),
        ..Default::default()
    };
    Workbook {
        sheets: vec![s1, s2, s3],
        styles: styles(),
        defined_names: vec![
            name("MyRange", "Sheet1!$A$3:$A$8"),
            name("MyCell", "Sheet1!$A$5"),
            name("MyCols", "Sheet1!$C:$D"),
        ],
        raw_parts: vec![crate::model::RawPart {
            name: "xl/charts/chart1.xml".into(),
            content_type: None,
            data: b"<c:chartSpace><c:ser><c:val><c:numRef><c:f>Sheet1!$A$1:$A$10</c:f></c:numRef></c:val></c:ser><c:ser><c:val><c:numRef><c:f>Sheet1!$B$1:$B$10</c:f></c:numRef></c:val></c:ser></c:chartSpace>".to_vec(),
        }],
        ..Default::default()
    }
}

/// 시트의 수식들 (주소 → 원문)
fn formulas(wb: &Workbook, si: usize) -> Vec<(String, String)> {
    wb.sheets[si]
        .cells
        .iter()
        .filter_map(|c| {
            c.formula
                .as_ref()
                .map(|f| (c.r.to_string(), f.text.clone()))
        })
        .collect()
}

fn formula_at(wb: &Workbook, si: usize, a1: &str) -> String {
    wb.sheets[si]
        .cell(CellRef::parse(a1).unwrap())
        .and_then(|c| c.formula.as_ref())
        .map(|f| f.text.clone())
        .unwrap_or_default()
}

fn names(wb: &Workbook) -> Vec<String> {
    wb.defined_names
        .iter()
        .map(|d| format!("{}={}", d.name, d.formula))
        .collect()
}

fn merges(wb: &Workbook) -> Vec<String> {
    wb.sheets[0].merges.iter().map(|m| m.to_string()).collect()
}

fn cfs(wb: &Workbook) -> Vec<(String, Vec<String>)> {
    wb.sheets[0]
        .conditional_formats
        .iter()
        .map(|c| (c.sqref.clone(), c.rules[0].formulas.clone()))
        .collect()
}

fn dvs(wb: &Workbook) -> Vec<String> {
    wb.sheets[0]
        .data_validations
        .iter()
        .map(|d| d.sqref.clone())
        .collect()
}

fn chart_refs(wb: &Workbook) -> Vec<String> {
    let c = wb.sheets[0].shapes[0].chart.as_ref().unwrap();
    c.series.iter().filter_map(|s| s.val_ref.clone()).collect()
}

fn anchor(wb: &Workbook) -> (u32, u32, u32, u32) {
    let a = &wb.sheets[0].shapes[0].anchor;
    let to = a.to.unwrap();
    (a.from.col, a.from.row, to.col, to.row)
}

#[test]
fn insert_rows_5_6() {
    let mut wb = refbase();
    insert_rows(&mut wb, 0, 4, 2).unwrap();
    let f = formulas(&wb, 0);
    let h: Vec<&str> = f
        .iter()
        .filter(|(a, _)| a.starts_with('H'))
        .map(|(_, t)| t.as_str())
        .collect();
    assert_eq!(
        h,
        [
            "A7",
            "$A$7",
            "SUM(A1:A12)",
            "SUM(A7:A12)",
            "SUM(A1:A4)",
            "A4",
            "SUM(A:A)",
            "SUM(7:7)",
            "SUM(4:8)",
            "Sheet2!A5",
            "SUM(Sheet1!A7:A8)",
            "A$7+$A7",
            "INDIRECT(\"A5\")",
            "OFFSET(A1,4,0)",
            "A1048576",
            "SUM(A:A)",
            "SUM(MyRange)",
            "A7*2+$B$8",
            "SUM(A7:B8 B8:C9)",
            "SUM(C:D)",
            "SUM(B:C)",
            "SUM(D:F)",
            "C7",
            "SUM(A3:F3)",
            "XFD1",
            "SUM(C1:D1)",
            "SUM(A7:A8)",
            "SUM(A8:A9)",
            "'Data 3'!A5",
            "SUM('Data 3'!A1:A10)",
            "MyCell*2",
            "SUM(Sheet1:Sheet2!A5)",
            "SUM(A7,A8,A9)",
            "ROW(A7)",
            "COLUMN(D1)",
            "SUM($A$7:$A$8)",
            "SUM(A1:B2,C7:D8)",
            "E7",
            "B7",
            "SUM(B7:E7)"
        ]
    );
    // 셀은 아래로 밀린다: H5 → H7
    assert_eq!(formula_at(&wb, 0, "H7"), "SUM(A1:A4)");
    assert!(wb.sheets[0].cell(CellRef::parse("H5").unwrap()).is_none());
    assert_eq!(
        formulas(&wb, 1)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        [
            "Sheet1!A7",
            "SUM(Sheet1!A1:A12)",
            "Sheet1!$A$7",
            "Sheet1!A4",
            "SUM(Sheet1!A7:A8)",
            "A5",
            "Sheet1!C7",
            "SUM(Sheet1!C:D)",
            "SUM(Sheet1!7:8)"
        ]
    );
    assert_eq!(
        names(&wb),
        [
            "MyRange=Sheet1!$A$3:$A$10",
            "MyCell=Sheet1!$A$7",
            "MyCols=Sheet1!$C:$D"
        ]
    );
    assert_eq!(merges(&wb), ["J4:K8", "A24:B25", "C27:E27"]);
    assert_eq!(
        cfs(&wb),
        [
            ("B1:B12".to_string(), vec!["C1>$A$7".to_string()]),
            ("C3:D10".to_string(), vec!["50".to_string()])
        ]
    );
    assert_eq!(dvs(&wb), ["L7:L10", "M3:M14"]);
    let c = &wb.sheets[0].comments[0];
    assert_eq!(c.cell.to_string(), "N8");
    assert_eq!(
        c.shape.as_ref().unwrap().anchor,
        [13, 14, 5, 10, 14, 69, 9, 1]
    );
    assert_eq!(wb.sheets[0].hyperlinks[0].range, "O9:O9");
    assert_eq!(anchor(&wb), (9, 20, 14, 32));
    assert_eq!(chart_refs(&wb), ["Sheet1!$A$1:$A$12", "Sheet1!$B$1:$B$12"]);
    let raw = wb.sheets[0].shapes[0].raw.as_ref().unwrap();
    assert!(raw.contains("<xdr:row>20</xdr:row>") && raw.contains("<xdr:row>32</xdr:row>"));
    let part = String::from_utf8(wb.raw_parts[0].data.clone()).unwrap();
    assert!(part.contains("<c:f>Sheet1!$A$1:$A$12</c:f>"));
}

#[test]
fn delete_rows_5_6() {
    let mut wb = refbase();
    delete_rows(&mut wb, 0, 4, 2).unwrap();
    let h: Vec<String> = formulas(&wb, 0)
        .into_iter()
        .filter(|(a, _)| a.starts_with('H'))
        .map(|(_, t)| t)
        .collect();
    assert_eq!(
        h,
        [
            "#REF!",
            "#REF!",
            "SUM(A1:A8)",
            "SUM(A5:A8)",
            "SUM(A:A)",
            "SUM(#REF!)",
            "SUM(4:4)",
            "Sheet2!A5",
            "SUM(Sheet1!#REF!)",
            "#REF!+#REF!",
            "INDIRECT(\"A5\")",
            "OFFSET(A1,4,0)",
            "A1048574",
            "SUM(A:A)",
            "SUM(MyRange)",
            "#REF!*2+#REF!",
            "SUM(#REF! B5:C5)",
            "SUM(C:D)",
            "SUM(B:C)",
            "SUM(D:F)",
            "#REF!",
            "SUM(A3:F3)",
            "XFD1",
            "SUM(C1:D1)",
            "SUM(#REF!)",
            "SUM(A5:A5)",
            "'Data 3'!A5",
            "SUM('Data 3'!A1:A10)",
            "MyCell*2",
            "SUM(Sheet1:Sheet2!A5)",
            "SUM(#REF!,#REF!,A5)",
            "ROW(#REF!)",
            "COLUMN(D1)",
            "SUM(#REF!)",
            "SUM(A1:B2,#REF!)",
            "#REF!",
            "#REF!",
            "SUM(#REF!)"
        ]
    );
    assert_eq!(
        formulas(&wb, 1)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        [
            "Sheet1!#REF!",
            "SUM(Sheet1!A1:A8)",
            "Sheet1!#REF!",
            "Sheet1!A4",
            "SUM(Sheet1!#REF!)",
            "A5",
            "Sheet1!#REF!",
            "SUM(Sheet1!C:D)",
            "SUM(Sheet1!#REF!)"
        ]
    );
    assert_eq!(
        names(&wb),
        [
            "MyRange=Sheet1!$A$3:$A$6",
            "MyCell=Sheet1!#REF!",
            "MyCols=Sheet1!$C:$D"
        ]
    );
    assert_eq!(merges(&wb), ["J4:K4", "A20:B21", "C23:E23"]);
    assert_eq!(
        cfs(&wb),
        [
            ("B1:B8".to_string(), vec!["C1>#REF!".to_string()]),
            ("C3:D6".to_string(), vec!["50".to_string()])
        ]
    );
    assert_eq!(dvs(&wb), ["L5:L6", "M3:M10"]);
    assert!(wb.sheets[0].comments.is_empty());
    assert_eq!(wb.sheets[0].hyperlinks[0].range, "O5:O5");
    assert_eq!(anchor(&wb), (9, 16, 14, 28));
    assert_eq!(chart_refs(&wb), ["Sheet1!$A$1:$A$8", "Sheet1!$B$1:$B$8"]);
}

#[test]
fn delete_first_rows_reanchors_cf() {
    let mut wb = refbase();
    delete_rows(&mut wb, 0, 0, 3).unwrap();
    assert_eq!(
        cfs(&wb),
        [
            ("B1:B7".to_string(), vec!["C1>$A$2".to_string()]),
            ("C1:D5".to_string(), vec!["50".to_string()])
        ]
    );
    assert_eq!(dvs(&wb), ["L2:L5", "M1:M9"]);
    assert_eq!(merges(&wb), ["J1:K3", "A19:B20", "C22:E22"]);
    assert_eq!(names(&wb)[0], "MyRange=Sheet1!$A$1:$A$5");
    assert_eq!(formula_at(&wb, 0, "H2"), "SUM(A1:A1)");
    assert_eq!(formula_at(&wb, 0, "H11"), "OFFSET(#REF!,4,0)");
    assert_eq!(
        wb.sheets[0].comments[0].shape.as_ref().unwrap().anchor,
        [13, 14, 0, 10, 14, 69, 4, 1]
    );
    let mut wb = refbase();
    insert_rows(&mut wb, 0, 0, 1).unwrap();
    assert_eq!(
        cfs(&wb)[0],
        ("B2:B11".to_string(), vec!["C2>$A$6".to_string()])
    );
    assert_eq!(formula_at(&wb, 0, "H15"), "OFFSET(A2,4,0)");
    assert_eq!(formula_at(&wb, 0, "H26"), "XFD2");
}

#[test]
fn insert_and_delete_columns() {
    let mut wb = refbase();
    insert_cols(&mut wb, 0, 2, 2).unwrap();
    assert_eq!(formula_at(&wb, 0, "J19"), "SUM(A5:B6 B6:E7)");
    assert_eq!(formula_at(&wb, 0, "J20"), "SUM(E:F)");
    assert_eq!(formula_at(&wb, 0, "J21"), "SUM(B:E)");
    assert_eq!(formula_at(&wb, 0, "J22"), "SUM(F:H)");
    assert_eq!(formula_at(&wb, 0, "J25"), "XFD1");
    assert_eq!(formula_at(&wb, 0, "J35"), "COLUMN(F1)");
    assert_eq!(names(&wb)[2], "MyCols=Sheet1!$E:$F");
    assert_eq!(merges(&wb), ["L4:M6", "A22:B23", "E25:G25"]);
    // B1:B10 은 바로 오른쪽에 넣은 열까지 넓어진다 (C 열이 B 열의 서식을 물려받는다)
    assert_eq!(
        cfs(&wb),
        [
            ("B1:D10".to_string(), vec!["E1>$A$5".to_string()]),
            ("E3:F8".to_string(), vec!["50".to_string()])
        ]
    );
    assert_eq!(dvs(&wb), ["N5:N8", "O3:O12"]);
    assert_eq!(
        wb.sheets[0].comments[0].shape.as_ref().unwrap().anchor,
        [15, 14, 3, 10, 16, 69, 7, 1]
    );
    assert_eq!(anchor(&wb), (11, 18, 16, 30));
    // 기본 너비를 명시한 새 열
    let c = &wb.sheets[0].cols;
    assert_eq!(
        (c[0].min, c[0].max, c[0].width, c[0].custom_width),
        (3, 4, Some(9.00390625), true)
    );

    let mut wb = refbase();
    delete_cols(&mut wb, 0, 2, 2).unwrap();
    assert_eq!(formula_at(&wb, 0, "F20"), "SUM(#REF!)");
    assert_eq!(formula_at(&wb, 0, "F21"), "SUM(B:B)");
    assert_eq!(formula_at(&wb, 0, "F25"), "XFB1");
    assert_eq!(formula_at(&wb, 0, "F40"), "SUM(B5:C5)");
    assert_eq!(names(&wb)[2], "MyCols=Sheet1!#REF!");
    assert_eq!(merges(&wb), ["H4:I6", "A22:B23"]);
    assert_eq!(
        cfs(&wb),
        [("B1:B10".to_string(), vec!["#REF!>$A$5".to_string()])]
    );
    assert_eq!(anchor(&wb), (7, 18, 12, 30));
    assert_eq!(formulas(&wb, 1)[6].1, "Sheet1!#REF!");
}

#[test]
fn sheet_rename_delete_add() {
    let mut wb = refbase();
    rename_sheet(&mut wb, 0, "My Data").unwrap();
    assert_eq!(formula_at(&wb, 0, "H11"), "SUM('My Data'!A5:A6)");
    assert_eq!(formula_at(&wb, 0, "H32"), "SUM('My Data:Sheet2'!A5)");
    assert_eq!(formula_at(&wb, 1, "C1"), "'My Data'!A5");
    assert_eq!(names(&wb)[0], "MyRange='My Data'!$A$3:$A$8");
    assert_eq!(chart_refs(&wb)[0], "'My Data'!$A$1:$A$10");
    assert!(rename_sheet(&mut wb, 1, "my data").is_err());
    assert!(rename_sheet(&mut wb, 1, "a/b").is_err());

    let mut wb = refbase();
    delete_sheet(&mut wb, 1).unwrap();
    assert_eq!(formula_at(&wb, 0, "H10"), "#REF!");
    assert_eq!(formula_at(&wb, 0, "H32"), "SUM('Sheet1:Data 3'!A5)");

    let mut wb = refbase();
    let i = add_sheet(&mut wb, None, 1).unwrap();
    assert_eq!((i, wb.sheets[1].name.as_str()), (1, "Sheet3"));
    assert_eq!(
        wb.sheets[1].page.setup.as_ref().unwrap().paper_size,
        Some(9)
    );
}

#[test]
fn copy_move_fill_like_hcell() {
    let mut wb = refbase();
    let c = copy(&wb, 0, parse_range("H1:H40").unwrap()).unwrap();
    paste(&mut wb, 0, CellRef::parse("I1").unwrap(), &c, None).unwrap();
    assert_eq!(formula_at(&wb, 0, "I1"), "B5");
    assert_eq!(formula_at(&wb, 0, "I8"), "SUM(5:5)");
    assert_eq!(formula_at(&wb, 0, "I10"), "Sheet2!B5");
    assert_eq!(formula_at(&wb, 0, "I12"), "B$5+$A5");
    assert_eq!(formula_at(&wb, 0, "I25"), "#REF!");
    assert_eq!(formula_at(&wb, 0, "I32"), "SUM(Sheet1:Sheet2!B5)");

    let mut wb = refbase();
    move_cells(
        &mut wb,
        0,
        parse_range("A5:B6").unwrap(),
        0,
        CellRef::parse("Q5").unwrap(),
    )
    .unwrap();
    for (a, f) in [
        ("H1", "Q5"),
        ("H2", "$Q$5"),
        ("H3", "SUM(A1:A10)"),
        ("H11", "SUM(Sheet1!Q5:Q6)"),
        ("H12", "Q$5+$Q5"),
        ("H18", "Q5*2+$R$6"),
        ("H19", "SUM(Q5:R6 B6:C7)"),
        ("H28", "SUM(A6:A7)"),
        ("H32", "SUM(Sheet1:Sheet2!A5)"),
        ("H33", "SUM(Q5,Q6,A7)"),
        ("H39", "R5"),
    ] {
        assert_eq!(formula_at(&wb, 0, a), f, "{a}");
    }
    assert_eq!(names(&wb)[1], "MyCell=Sheet1!$Q$5");
    assert_eq!(formula_at(&wb, 1, "C1"), "Sheet1!Q5");
    assert_eq!(
        cfs(&wb),
        [
            ("B1:B4 B7:B10".to_string(), vec!["C1>$Q$5".to_string()]),
            ("C3:D8".to_string(), vec!["50".to_string()]),
            ("R5:R6".to_string(), vec!["C5>$Q$5".to_string()])
        ]
    );
    assert_eq!(
        wb.sheets[0]
            .cell(CellRef::parse("Q5").unwrap())
            .map(|c| c.value.clone()),
        Some(CellValue::Number(51.0))
    );
    assert!(wb.sheets[0].cell(CellRef::parse("A5").unwrap()).is_none());

    let mut wb = refbase();
    fill(&mut wb, 0, parse_range("H1:H5").unwrap(), true).unwrap();
    fill(&mut wb, 0, parse_range("H10:J10").unwrap(), false).unwrap();
    assert_eq!(formula_at(&wb, 0, "H5"), "A9");
    assert_eq!(formula_at(&wb, 0, "J10"), "Sheet2!C5");
}

#[test]
fn inserted_rows_and_columns_take_the_previous_format() {
    // 4행: 행 서식 1 + A4:C4 굵게, B 열: 열 서식 2, B2 에 테두리 (서식 3)
    let mut wb = Workbook {
        sheets: vec![Sheet {
            name: "Sheet1".into(),
            default_col_width: Some(9.00390625),
            ..Default::default()
        }],
        styles: styles(),
        ..Default::default()
    };
    for i in 1..=4 {
        wb.styles.cell_xfs.push(Xf {
            num_fmt_id: i,
            ..Default::default()
        });
    }
    let s = &mut wb.sheets[0];
    for r in 1..=8u32 {
        for c in 0..4u32 {
            let style = match (r, c) {
                (4, _) => 1,
                (2, 1) => 3,
                (_, 1) => 2,
                (_, 3) => 4,
                _ => 0,
            };
            s.cells.push(Cell {
                r: CellRef { row: r - 1, col: c },
                style,
                value: CellValue::Number(1.0),
                formula: None,
            });
        }
    }
    s.rows.push(RowInfo {
        index: 4,
        height: Some(30.0),
        custom_height: true,
        style: Some(1),
        ..Default::default()
    });
    s.cols.push(ColInfo {
        min: 2,
        max: 2,
        width: Some(20.77734375),
        style: Some(2),
        custom_width: true,
        ..Default::default()
    });
    let base = wb.clone();
    // 5행 삽입: 4행의 높이·행 서식과 서식 있는 칸을 물려받는다
    insert_rows(&mut wb, 0, 4, 2).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(
        s.rows
            .iter()
            .map(|r| (r.index, r.height, r.style))
            .collect::<Vec<_>>(),
        [
            (4, Some(30.0), Some(1)),
            (5, Some(30.0), Some(1)),
            (6, Some(30.0), Some(1))
        ]
    );
    assert_eq!(
        s.cell(CellRef::parse("A5").unwrap()).map(|c| c.style),
        Some(1)
    );
    assert_eq!(
        s.cell(CellRef::parse("D6").unwrap()).map(|c| c.style),
        Some(1)
    );
    // C 열 삽입: B 열에 열 서식이 있으므로 행 서식 있는 4행의 칸만 복사, 열 정보는 넓어진다
    let mut wb = base.clone();
    insert_cols(&mut wb, 0, 2, 1).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(
        s.cols.iter().map(|c| (c.min, c.max)).collect::<Vec<_>>(),
        [(2, 3)]
    );
    assert_eq!(
        s.cell(CellRef::parse("C4").unwrap()).map(|c| c.style),
        Some(1)
    );
    assert!(s.cell(CellRef::parse("C2").unwrap()).is_none());
    // E 열 삽입: D 열에는 열 서식이 없으므로 모든 칸의 서식을 복사, 새 열 정보는 기본 너비
    let mut wb = base;
    insert_cols(&mut wb, 0, 4, 1).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(
        s.cell(CellRef::parse("E1").unwrap()).map(|c| c.style),
        Some(4)
    );
    assert_eq!(
        s.cell(CellRef::parse("E4").unwrap()).map(|c| c.style),
        Some(1)
    );
    assert_eq!(
        s.cols
            .iter()
            .map(|c| (c.min, c.width, c.custom_width))
            .collect::<Vec<_>>(),
        [(2, Some(20.77734375), true), (5, Some(9.00390625), true)]
    );
}

#[test]
fn input_rules_and_formula_formats() {
    crate::calc::date::set_this_year(Some(2026));
    let mut wb = refbase();
    let fmt = |wb: &Workbook, a1: &str| {
        let st = style_at(&wb.sheets[0], CellRef::parse(a1).unwrap());
        crate::numfmt::code_for(&wb.styles, wb.styles.xf(st).unwrap().num_fmt_id)
    };
    set_input(&mut wb, 0, CellRef::parse("Q1").unwrap(), "1,234").unwrap();
    set_input(&mut wb, 0, CellRef::parse("Q2").unwrap(), "2024-01-15").unwrap();
    set_input(&mut wb, 0, CellRef::parse("Q3").unwrap(), "'0123").unwrap();
    set_input(&mut wb, 0, CellRef::parse("Q4").unwrap(), "=sum(q1, q1)").unwrap();
    set_input(&mut wb, 0, CellRef::parse("Q5").unwrap(), "=q2+1").unwrap();
    set_input(&mut wb, 0, CellRef::parse("Q6").unwrap(), "=today()").unwrap();
    assert_eq!(fmt(&wb, "Q1"), "#,##0");
    assert_eq!(fmt(&wb, "Q2"), "yyyy-mm-dd");
    let q3 = wb.sheets[0].cell(CellRef::parse("Q3").unwrap()).unwrap();
    assert_eq!(q3.value, CellValue::Text("0123".into()));
    assert!(wb.styles.xf(q3.style).unwrap().quote_prefix);
    assert_eq!(formula_at(&wb, 0, "Q4"), "SUM(Q1,Q1)");
    assert_eq!(fmt(&wb, "Q4"), "#,##0");
    assert_eq!(fmt(&wb, "Q5"), "yyyy-mm-dd");
    assert_eq!(fmt(&wb, "Q6"), "yyyy-mm-dd");
    // 문법 오류는 받지 않는다
    assert!(set_input(&mut wb, 0, CellRef::parse("Q7").unwrap(), "=1+").is_err());
    // 빈 입력은 지운다
    set_input(&mut wb, 0, CellRef::parse("Q1").unwrap(), "").unwrap();
    assert!(
        wb.sheets[0]
            .cell(CellRef::parse("Q1").unwrap())
            .is_none_or(|c| c.value.is_empty())
    );
}

#[test]
fn json_edits() {
    let mut wb = refbase();
    let script: Script = serde_json::from_str(
        r#"{"edits":[
            {"op":"set","sheet":"Sheet2","cell":"E1","input":"=SUM(A1:A3)"},
            {"op":"format","range":"A1:B2","bold":true,"fill":"FFFF00","hAlign":"center"},
            {"op":"border","range":"A1:B2","preset":"outline","style":"medium"},
            {"op":"merge","range":"J20:K21","center":true},
            {"op":"colWidth","col":"B","width":20},
            {"op":"rowHeight","row":3,"height":30},
            {"op":"insertRows","row":2,"count":1},
            {"op":"renameSheet","sheet":2,"name":"자료"}
        ]}"#,
    )
    .unwrap();
    for e in &script.edits {
        apply(&mut wb, e).unwrap();
    }
    assert_eq!(formula_at(&wb, 1, "E1"), "SUM(A1:A3)");
    let a1 = wb.sheets[0].cell(CellRef::parse("A1").unwrap()).unwrap();
    let xf = wb.styles.xf(a1.style).unwrap();
    assert!(wb.styles.fonts[xf.font_id as usize].bold);
    let b = &wb.styles.borders[xf.border_id as usize];
    assert_eq!(
        b.top.as_ref().and_then(|e| e.style.as_deref()),
        Some("medium")
    );
    assert!(b.right.as_ref().is_none_or(|e| e.style.is_none()));
    assert!(merges(&wb).contains(&"J21:K22".to_string()));
    let col_b = wb.sheets[0].cols.iter().find(|c| c.min == 2).unwrap();
    assert_eq!(col_b.width, Some(20.77734375));
    assert_eq!(
        wb.sheets[0]
            .rows
            .iter()
            .find(|r| r.index == 4)
            .and_then(|r| r.height),
        Some(30.0)
    );
    assert_eq!(wb.sheets[2].name, "자료");
}

/// 한셀이 만든 문서의 기본 테두리: 선은 없고 색만 — 왼쪽·오른쪽·아래 검정, 위 자동
fn hcell_book() -> Workbook {
    let mut st = styles();
    let none = |rgb: Option<&str>| {
        Some(crate::model::BorderEdge {
            style: None,
            color: Some(match rgb {
                Some(h) => crate::model::Color {
                    rgb: Some(h.into()),
                    ..Default::default()
                },
                None => crate::model::Color {
                    auto: true,
                    ..Default::default()
                },
            }),
        })
    };
    st.borders[0] = crate::model::Border {
        left: none(Some("FF000000")),
        right: none(Some("FF000000")),
        top: none(None),
        bottom: none(Some("FF000000")),
        ..Default::default()
    };
    Workbook {
        sheets: vec![Sheet {
            name: "Sheet1".into(),
            ..Default::default()
        }],
        styles: st,
        ..Default::default()
    }
}

/// 칸 하나의 변에 가는 선을 바로 넣는다 (이웃은 건드리지 않는다 — 실험 문서의 처음 상태: 한셀이 COM 으로 색까지
/// 준 칸은 나머지 변의 색이 자동이다)
fn put_edge(wb: &mut Workbook, a1: &str, side: &str, rgb: Option<&str>) {
    let at = CellRef::parse(a1).unwrap();
    let old = style_at(&wb.sheets[0], at);
    let mut xf = wb.styles.xf(old).cloned().unwrap_or_default();
    if xf.border_id == 0 {
        let auto = || {
            Some(crate::model::BorderEdge {
                style: None,
                color: Some(crate::model::Color {
                    auto: true,
                    ..Default::default()
                }),
            })
        };
        wb.styles.borders.push(crate::model::Border {
            left: auto(),
            right: auto(),
            top: auto(),
            bottom: auto(),
            ..Default::default()
        });
        xf.border_id = wb.styles.borders.len() as u32 - 1;
    }
    let e = Some(Some(crate::model::BorderEdge {
        style: Some("thin".into()),
        color: Some(match rgb {
            Some(h) => style::rgb(h).unwrap(),
            None => crate::model::Color {
                auto: true,
                ..Default::default()
            },
        }),
    }));
    let mut set = style::EdgeSet::default();
    match side {
        "L" => set.left = e,
        "R" => set.right = e,
        "T" => set.top = e,
        _ => set.bottom = e,
    }
    style::patch_border(&mut wb.styles, &mut xf, &set);
    let new = style::intern_xf(&mut wb.styles, xf);
    cell_mut(&mut wb.sheets[0], at).style = new;
}

/// 칸의 네 변 `왼쪽 오른쪽 위 아래` (선 없음 `-`, 색 `000000`·`auto`)
fn edges(wb: &Workbook, a1: &str) -> String {
    let st = style_at(&wb.sheets[0], CellRef::parse(a1).unwrap());
    let b = wb
        .styles
        .xf(st)
        .and_then(|x| wb.styles.borders.get(x.border_id as usize))
        .cloned()
        .unwrap_or_default();
    let one = |e: &Option<crate::model::BorderEdge>| match e {
        Some(e) if e.style.as_deref().is_some_and(|s| s != "none") => match e.color.as_ref() {
            Some(c) if !c.auto => c.rgb.clone().unwrap_or_default()[2..].to_ascii_lowercase(),
            _ => "auto".to_string(),
        },
        _ => "-".to_string(),
    };
    format!(
        "{} {} {} {}",
        one(&b.left),
        one(&b.right),
        one(&b.top),
        one(&b.bottom)
    )
}

// 테두리 대조 실험 (한셀 COM `Borders(묶음).LineStyle`, 색을 주지 않고)의 결과
#[test]
fn borders_like_hcell() {
    let all = |wb: &mut Workbook, r: &str| {
        border(
            wb,
            0,
            parse_range(r).unwrap(),
            BorderPreset::All,
            "thin",
            None,
        )
        .unwrap()
    };
    // 묶음 첫 칸의 색: 기본 테두리면 위만 자동, 나머지 검정
    let mut wb = hcell_book();
    all(&mut wb, "G2:I4");
    assert_eq!(edges(&wb, "G2"), "000000 000000 auto 000000");
    assert_eq!(edges(&wb, "I4"), "000000 000000 000000 000000");
    // 묶음 첫 칸에 선이 있으면 그 색이 묶음 전체에 간다. 안쪽 세로선은 첫 행의 칸 사이 선이 제각각이면 끝 칸
    // 왼쪽 변의 색, 안쪽 가로선은 첫 행 아래 변의 색이 제각각이면 자동
    let mut wb = hcell_book();
    put_edge(&mut wb, "D2", "R", Some("FF0000"));
    put_edge(&mut wb, "B2", "L", Some("0000FF"));
    put_edge(&mut wb, "B2", "T", Some("00FF00"));
    put_edge(&mut wb, "B2", "R", Some("FFA500"));
    put_edge(&mut wb, "B2", "B", Some("FFC0CB"));
    put_edge(&mut wb, "B4", "B", Some("808080"));
    all(&mut wb, "B2:D4");
    assert_eq!(edges(&wb, "B2"), "0000ff auto 00ff00 auto");
    assert_eq!(edges(&wb, "D3"), "auto ff0000 auto auto");
    assert_eq!(edges(&wb, "C4"), "auto auto auto 808080");
    // 오른쪽 이웃의 맞닿은 선: 새 선과 같으면 두고(양쪽에 남는다) 다르면 지운다
    let mut wb = hcell_book();
    put_edge(&mut wb, "D7", "L", Some("000000"));
    put_edge(&mut wb, "D8", "L", Some("0000FF"));
    all(&mut wb, "B7:C8");
    assert_eq!(edges(&wb, "C7"), "000000 000000 auto 000000");
    assert_eq!(edges(&wb, "D7"), "000000 - - -");
    assert_eq!(edges(&wb, "D8"), "- - - -");
    // 위 이웃의 아래 선이 있으면 위 묶음의 색은 그 선의 색
    let mut wb = hcell_book();
    put_edge(&mut wb, "B10", "B", Some("000000"));
    put_edge(&mut wb, "C10", "B", None);
    all(&mut wb, "B11:C12");
    assert_eq!(edges(&wb, "B11"), "000000 000000 000000 000000");
    assert_eq!(edges(&wb, "B10"), "- - - 000000");
    assert_eq!(edges(&wb, "C10"), "- - - -");
    // 묶음의 칸 사이가 모두 이미 같은 선이면 그 묶음은 그대로 둔다
    let mut wb = hcell_book();
    put_edge(&mut wb, "H12", "L", Some("000000"));
    put_edge(&mut wb, "H13", "L", Some("000000"));
    all(&mut wb, "F12:G13");
    assert_eq!(edges(&wb, "G12"), "000000 - auto 000000");
    assert_eq!(edges(&wb, "H13"), "000000 - - -");
}

// 병합 대조 실험: 첫 값 칸(C24)의 값과 서식이 왼쪽 위 칸으로 오되 테두리는 왼쪽 위 칸의 것을 두고, 병합 안쪽의
// 칸 사이 선은 지운다
#[test]
fn merge_like_hcell() {
    let mut wb = hcell_book();
    wb.styles.fonts.push(crate::model::Font {
        bold: true,
        ..wb.styles.fonts[0].clone()
    });
    put_edge(&mut wb, "B24", "L", None);
    put_edge(&mut wb, "C24", "L", None);
    put_edge(&mut wb, "C24", "T", None);
    put_edge(&mut wb, "D25", "B", None);
    let c24 = CellRef::parse("C24").unwrap();
    let st = style_at(&wb.sheets[0], c24);
    let mut xf = wb.styles.xf(st).cloned().unwrap();
    xf.font_id = 1;
    let bold = style::intern_xf(&mut wb.styles, xf);
    let c = cell_mut(&mut wb.sheets[0], c24);
    c.style = bold;
    c.value = CellValue::Text("x".into());
    merge(&mut wb, 0, parse_range("B24:D25").unwrap(), true).unwrap();
    let b24 = wb.sheets[0].cell(CellRef::parse("B24").unwrap()).unwrap();
    assert_eq!(b24.value, CellValue::Text("x".into()));
    let xf = wb.styles.xf(b24.style).unwrap();
    assert_eq!(xf.font_id, 1);
    assert_eq!(
        xf.alignment.as_ref().and_then(|a| a.horizontal.as_deref()),
        Some("center")
    );
    assert_eq!(edges(&wb, "B24"), "auto - - -");
    assert_eq!(edges(&wb, "C24"), "- - auto -");
    assert_eq!(edges(&wb, "D25"), "- - - auto");
    assert!(wb.sheets[0].cell(c24).is_some_and(|c| c.value.is_empty()));
}

// 열 너비·행 높이 대조 실험: 바뀐 칸 안의 표지 오프셋은 (칸 px − 1) 의 비율로 옮기고 px 로 반올림한다
#[test]
fn anchors_follow_resized_cells_like_hcell() {
    let mut wb = hcell_book();
    wb.styles.fonts[0] = crate::model::Font {
        name: Some("Arial".into()),
        size: Some(10.0),
        ..Default::default()
    };
    let s = &mut wb.sheets[0];
    s.cols.push(ColInfo {
        min: 2,
        max: 2,
        width: Some(4.0),
        custom_width: true,
        ..Default::default()
    });
    s.rows.push(RowInfo {
        index: 3,
        height: Some(30.0),
        custom_height: true,
        ..Default::default()
    });
    const PX: i64 = crate::units::EMU_PER_PX;
    for (i, off) in [1i64, 18, 25, 27].into_iter().enumerate() {
        s.shapes.push(Shape {
            kind: ShapeKind::Shape,
            anchor: Anchor {
                from: Marker {
                    col: 1,
                    col_off: off * PX,
                    row: 2,
                    row_off: off * PX,
                },
                to: Some(Marker {
                    col: 3,
                    col_off: 0,
                    row: 5 + i as u32,
                    row_off: 0,
                }),
                ..Default::default()
            },
            ..Default::default()
        });
    }
    // B 열 28px → 131px (Arial 10): 1·18·25·27px → 5·87·120·130px
    apply(
        &mut wb,
        &serde_json::from_str(r#"{"op":"colWidth","col":"B","width":18}"#).unwrap(),
    )
    .unwrap();
    // 3행 40px → 60px: 1·18·25·27px → 2·27·38·41px
    apply(
        &mut wb,
        &serde_json::from_str(r#"{"op":"rowHeight","row":3,"height":45}"#).unwrap(),
    )
    .unwrap();
    let got: Vec<(i64, i64)> = wb.sheets[0]
        .shapes
        .iter()
        .map(|s| (s.anchor.from.col_off / PX, s.anchor.from.row_off / PX))
        .collect();
    assert_eq!(got, [(5, 2), (87, 27), (120, 38), (130, 41)]);
}

// 편집 대조 실험: 코드 이름이 없는 문서에 시트를 더해 앞으로 옮기면 원래 시트가 Sheet1, 새 시트가 Sheet2
#[test]
fn code_names_follow_the_original_order() {
    let mut wb = hcell_book();
    let at = add_sheet(&mut wb, Some("새 시트"), 1).unwrap();
    rename_sheet(&mut wb, 0, "편집됨").unwrap();
    move_sheet(&mut wb, at, 0).unwrap();
    let got: Vec<(String, Option<String>)> = wb
        .sheets
        .iter()
        .map(|s| (s.name.clone(), s.code_name.clone()))
        .collect();
    assert_eq!(
        got,
        [
            ("새 시트".to_string(), Some("Sheet2".to_string())),
            ("편집됨".to_string(), Some("Sheet1".to_string()))
        ]
    );
}
