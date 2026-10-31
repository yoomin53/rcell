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
        ..Default::default()
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
    // 4행 칸의 서식은 행 서식과 같으므로 새 행에는 셀 없이 행 서식으로 보인다
    assert_eq!(style_at(s, CellRef::parse("A5").unwrap()), 1);
    assert_eq!(style_at(s, CellRef::parse("D6").unwrap()), 1);
    assert!(s.cell(CellRef::parse("D6").unwrap()).is_none());
    // C 열 삽입: 새 열은 B 열의 열 정보를 너비를 명시해 따로 받고, 칸마다 B 열 칸의 서식을 받는다 (B2 의 서식 3,
    // 4행은 행 서식과 같으므로 셀 없이 행 서식으로 보인다)
    let mut wb = base.clone();
    insert_cols(&mut wb, 0, 2, 1).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(
        s.cols
            .iter()
            .map(|c| (c.min, c.max, c.style, c.custom_width))
            .collect::<Vec<_>>(),
        [(2, 2, Some(2), true), (3, 3, Some(2), true)]
    );
    assert_eq!(style_at(s, CellRef::parse("C4").unwrap()), 1);
    assert!(s.cell(CellRef::parse("C4").unwrap()).is_none());
    assert_eq!(style_at(s, CellRef::parse("C2").unwrap()), 3);
    assert_eq!(style_at(s, CellRef::parse("C5").unwrap()), 2);
    // E 열 삽입: D 열에는 열 정보가 없으므로 새 열 정보는 기본 너비
    let mut wb = base;
    insert_cols(&mut wb, 0, 4, 1).unwrap();
    let s = &wb.sheets[0];
    assert_eq!(
        s.cell(CellRef::parse("E1").unwrap()).map(|c| c.style),
        Some(4)
    );
    assert_eq!(style_at(s, CellRef::parse("E4").unwrap()), 1);
    assert_eq!(
        s.cols
            .iter()
            .map(|c| (c.min, c.width, c.custom_width))
            .collect::<Vec<_>>(),
        [(2, Some(20.77734375), true), (5, Some(9.00390625), true)]
    );
}

/// 열 삽입·삭제 대조 실험의 문서: B 열 서식 1, D:F 서식 2, H:K 서식 3, 3행 행 서식 4, 7행 행 서식 3, A20 에 값
fn row_style_book() -> Workbook {
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
    for (min, max, style) in [(2, 2, 1), (4, 6, 2), (8, 11, 3)] {
        s.cols.push(ColInfo {
            min,
            max,
            width: Some(9.00390625),
            style: Some(style),
            ..Default::default()
        });
    }
    for (index, style) in [(3, 4), (7, 3)] {
        s.rows.push(RowInfo {
            index,
            style: Some(style),
            ..Default::default()
        });
    }
    put(s, "A20", CellValue::Number(1.0), None);
    wb
}

/// 행의 셀 기록 `열=서식` (없는 칸은 빼고)
fn row_cells(wb: &Workbook, row: u32) -> String {
    wb.sheets[0]
        .cells
        .iter()
        .filter(|c| c.r.row == row - 1)
        .map(|c| {
            format!(
                "{}={}",
                CellRef {
                    row: 0,
                    col: c.r.col
                }
                .to_string()
                .trim_end_matches('1'),
                c.style
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// 열 삽입·삭제 대조 실험: 행 서식이 있는 행에는 열 정보가 옮겨지는 칸에 열 서식 셀이 생긴다
#[test]
fn column_shift_writes_cells_in_row_styled_rows() {
    let run = |f: &dyn Fn(&mut Workbook)| {
        let mut wb = row_style_book();
        f(&mut wb);
        (row_cells(&wb, 3), row_cells(&wb, 7))
    };
    let r = run(&|wb| insert_cols(wb, 0, 2, 1).unwrap());
    assert_eq!(r.0, "D=0 E=2 F=2 G=2 H=0 I=3 J=3 K=3 L=3");
    assert_eq!(r.1, "D=0 E=2 F=2 G=2 H=0");
    // 4칸 삽입: 옛 C 열(D:F 의 3칸 앞 중 C)은 만들지 않는다 — 열 정보 칸마다 n 칸 앞 열
    let r = run(&|wb| insert_cols(wb, 0, 2, 4).unwrap());
    assert_eq!(
        r.0,
        "A=0 B=1 C=1 D=1 E=1 F=1 H=2 I=2 J=2 K=0 L=3 M=3 N=3 O=3"
    );
    let r = run(&|wb| insert_cols(wb, 0, 4, 3).unwrap());
    assert_eq!(r.0, "B=1 C=0 H=2 I=2 J=0 K=3 L=3 M=3 N=3");
    let r = run(&|wb| delete_cols(wb, 0, 1, 3).unwrap());
    assert_eq!(r.0, "B=2 C=2 E=3 F=3 G=3 H=3 I=0 J=0 K=0");
    let r = run(&|wb| delete_cols(wb, 0, 2, 2).unwrap());
    assert_eq!(r.0, "C=2 D=2 E=0 F=3 G=3 H=3 I=3 J=0 K=0");
    // 마지막 셀이 있는 행보다 아래 행, 셀이 없는 문서는 그대로
    let r = run(&|wb| {
        wb.sheets[0].rows.push(RowInfo {
            index: 25,
            style: Some(4),
            ..Default::default()
        });
        insert_cols(wb, 0, 2, 1).unwrap();
    });
    assert_eq!(r.1, "D=0 E=2 F=2 G=2 H=0");
    let mut wb = row_style_book();
    wb.sheets[0].rows.push(RowInfo {
        index: 25,
        style: Some(4),
        ..Default::default()
    });
    insert_cols(&mut wb, 0, 2, 1).unwrap();
    assert_eq!(row_cells(&wb, 25), "");
    let r = run(&|wb| {
        wb.sheets[0].cells.clear();
        insert_cols(wb, 0, 2, 1).unwrap();
    });
    assert_eq!(r.0, "");
}

// 서식 물려받기 대조 실험: 맞은편 칸에 같은 선이 없는 변은 물려받지 않는다
#[test]
fn inherited_borders_need_the_same_line_beyond() {
    let mut wb = hcell_book();
    put_edge(&mut wb, "B2", "B", Some("000000"));
    put_edge(&mut wb, "B3", "B", Some("000000"));
    put_edge(&mut wb, "C2", "B", Some("000000"));
    put_edge(&mut wb, "C3", "B", Some("FF0000"));
    put_edge(&mut wb, "D2", "L", Some("000000"));
    put(&mut wb.sheets[0], "D3", CellValue::Number(1.0), None);
    insert_rows(&mut wb, 0, 2, 1).unwrap();
    assert_eq!(edges(&wb, "B3"), "- - - 000000");
    assert_eq!(edges(&wb, "C3"), "- - - -");
    assert_eq!(edges(&wb, "D3"), "- - - -");
    assert_eq!(edges(&wb, "C4"), "- - - ff0000");
    // 열도 같다 (왼쪽 칸의 선은 오른쪽 칸에 같은 선이 있을 때만)
    let mut wb = hcell_book();
    put_edge(&mut wb, "B5", "R", Some("000000"));
    put_edge(&mut wb, "C5", "R", Some("000000"));
    put_edge(&mut wb, "B6", "T", Some("000000"));
    insert_cols(&mut wb, 0, 2, 1).unwrap();
    assert_eq!(edges(&wb, "C5"), "- 000000 - -");
    assert_eq!(edges(&wb, "C6"), "- - - -");
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
    // 자동 높이가 적힌 행: 병합한 첫 행은 고정 높이, 나머지 행은 높이를 지운다
    let mut wb = hcell_book();
    for index in [24, 25] {
        wb.sheets[0].rows.push(RowInfo {
            index,
            height: Some(16.75),
            ..Default::default()
        });
    }
    merge(&mut wb, 0, parse_range("B24:D25").unwrap(), true).unwrap();
    assert_eq!(
        wb.sheets[0]
            .rows
            .iter()
            .map(|r| (r.index, r.height, r.custom_height))
            .collect::<Vec<_>>(),
        [(24, Some(16.75), true), (25, None, false)]
    );
}

/// 보호·필터 대조 실험의 문서 모양: A1:E12 에 값, 1행이 머리글인 자동 필터 (A 열 값 7·9, C 열 20 초과 100 미만)
fn filter_book() -> Workbook {
    let mut wb = Workbook {
        sheets: vec![Sheet {
            name: "Sheet1".into(),
            ..Default::default()
        }],
        styles: styles(),
        ..Default::default()
    };
    let a = [7, 1, 8, 2, 9, 3, 10, 4, 11, 5, 12, 6];
    for (i, v) in a.iter().enumerate() {
        let r = i + 1;
        put(
            &mut wb.sheets[0],
            &format!("A{r}"),
            CellValue::Number(*v as f64),
            None,
        );
        put(
            &mut wb.sheets[0],
            &format!("C{r}"),
            CellValue::Number(r as f64 * 10.0),
            None,
        );
    }
    wb.sheets[0].auto_filter = Some(crate::model::AutoFilter {
        range: "A1:E12".into(),
        sort_state: None,
        columns: vec![
            crate::model::FilterColumn {
                col_id: 0,
                values: vec!["7".into(), "9".into()],
                ..Default::default()
            },
            crate::model::FilterColumn {
                col_id: 2,
                custom: vec![
                    ("greaterThan".into(), "20".into()),
                    ("lessThan".into(), "100".into()),
                ],
                custom_and: true,
                ..Default::default()
            },
        ],
    });
    wb.sheets[0].filter_mode = true;
    for r in [6, 7] {
        wb.sheets[0].rows.push(RowInfo {
            index: r,
            hidden: true,
            ..Default::default()
        });
    }
    wb
}

fn hidden_rows(wb: &Workbook) -> Vec<u32> {
    wb.sheets[0]
        .rows
        .iter()
        .filter(|r| r.hidden)
        .map(|r| r.index)
        .collect()
}

// 필터 대조 실험: 머리글이 옮겨지면 필터를 다시 적용하고, 범위 안에 넣거나 지우면 그대로 둔다
#[test]
fn filters_reapply_when_the_header_moves() {
    let mut wb = filter_book();
    insert_rows(&mut wb, 0, 2, 1).unwrap();
    assert_eq!(hidden_rows(&wb), [7, 8]);
    let mut wb = filter_book();
    insert_rows(&mut wb, 0, 0, 1).unwrap();
    // 새 머리글은 2행, 조건에 맞는 것은 옛 5행(A=9, C=50)뿐
    assert_eq!(hidden_rows(&wb), [3, 4, 5, 7, 8, 9, 10, 11, 12, 13]);
    // 조건이 있는 필터에서는 숨긴 행의 서식을 바꾸지 않는다 (숨긴 열은 바꾼다)
    let mut wb = filter_book();
    wb.sheets[0].cols.push(ColInfo {
        min: 4,
        max: 4,
        hidden: true,
        ..Default::default()
    });
    let p = FormatPatch {
        bold: Some(true),
        ..Default::default()
    };
    format(&mut wb, 0, parse_range("B5:D8").unwrap(), &p).unwrap();
    let bold = |wb: &Workbook, a: &str| {
        let st = style_at(&wb.sheets[0], CellRef::parse(a).unwrap());
        wb.styles.font_of(st).is_some_and(|f| f.bold)
    };
    assert!(bold(&wb, "D5") && bold(&wb, "B8") && !bold(&wb, "B6") && !bold(&wb, "C7"));
}

// 보호 대조 실험: 한셀 자동화는 열 너비·시트 추가·이름 바꾸기에 보호를 보지 않는다 (화면은 막는다).
// 행 삭제가 허용된 시트도 잠긴 칸이 있는 행은 지울 수 없다
#[test]
fn protection_like_hcell_automation_and_screen() {
    let mut wb = filter_book();
    wb.sheets[0].protection = Some(crate::model::SheetProtection {
        password: None,
        flags: vec![("sheet".into(), true), ("deleteRows".into(), false)],
    });
    wb.protection = Some(crate::model::WorkbookProtection {
        lock_structure: true,
        ..Default::default()
    });
    let width: Edit = serde_json::from_str(r#"{"op":"colWidth","col":"B","width":18}"#).unwrap();
    let add: Edit = serde_json::from_str(r#"{"op":"addSheet","name":"새 시트"}"#).unwrap();
    assert!(check_ui(&wb, &width).is_err());
    assert!(check_ui(&wb, &add).is_err());
    apply(&mut wb, &width).unwrap();
    apply(&mut wb, &add).unwrap();
    assert_eq!(wb.sheets.len(), 2);
    // 구조가 보호되면 더한 시트를 선택하지 않고, 지우기도 자동화로는 된다 (옮기기는 막힌다)
    assert!(!wb.sheets[1].view.tab_selected);
    let del: Edit = serde_json::from_str(r#"{"op":"deleteSheet","sheet":1}"#).unwrap();
    let mv: Edit = serde_json::from_str(r#"{"op":"moveSheet","sheet":1,"to":0}"#).unwrap();
    assert!(check_ui(&wb, &del).is_err());
    assert!(apply(&mut wb, &mv).is_err());
    apply(&mut wb, &del).unwrap();
    assert_eq!(wb.sheets.len(), 1);
    assert!(delete_rows(&mut wb, 0, 7, 1).is_err());
    assert!(insert_rows(&mut wb, 0, 7, 1).is_err());
    // 행 서식으로 잠금을 푼 행은 지울 수 있다
    wb.styles.cell_xfs.push(Xf {
        protection: Some(crate::model::Protection {
            locked: Some(false),
            hidden: None,
        }),
        ..Default::default()
    });
    let unlocked = wb.styles.cell_xfs.len() as u32 - 1;
    let s = &mut wb.sheets[0];
    s.cells.retain(|c| c.r.row != 7);
    s.rows.push(RowInfo {
        index: 8,
        style: Some(unlocked),
        ..Default::default()
    });
    s.rows.sort_by_key(|r| r.index);
    delete_rows(&mut wb, 0, 7, 1).unwrap();
}

// 조건부 서식 대조 실험: 붙인 규칙은 맨 위 우선순위, 데이터 막대 묶음은 두 번, 채우기·옮기기는 원래 규칙에 합친다
#[test]
fn pasted_conditional_formats_like_hcell() {
    let book = || {
        let mut wb = hcell_book();
        let s = &mut wb.sheets[0];
        for r in 1..=5 {
            put(s, &format!("A{r}"), CellValue::Number(r as f64), None);
            put(s, &format!("B{r}"), CellValue::Number(r as f64), None);
        }
        s.conditional_formats = vec![
            ConditionalFormat {
                sqref: "A1:A5".into(),
                rules: vec![CfRule {
                    kind: "cellIs".into(),
                    priority: 1,
                    dxf_id: Some(0),
                    operator: Some("greaterThan".into()),
                    formulas: vec!["3".into()],
                    ..Default::default()
                }],
            },
            ConditionalFormat {
                sqref: "B1:B5".into(),
                rules: vec![CfRule {
                    kind: "dataBar".into(),
                    priority: 2,
                    ..Default::default()
                }],
            },
        ];
        wb
    };
    let list = |wb: &Workbook| {
        let mut v: Vec<(String, u32)> = wb.sheets[0]
            .conditional_formats
            .iter()
            .flat_map(|c| c.rules.iter().map(|r| (c.sqref.clone(), r.priority)))
            .collect();
        v.sort_by_key(|x| x.1);
        v
    };
    let s = |a: &str, p: u32| (a.to_string(), p);
    let mut wb = book();
    let c = clip::copy(&wb, 0, parse_range("A1:A2").unwrap()).unwrap();
    clip::paste(&mut wb, 0, CellRef::parse("A20").unwrap(), &c, None).unwrap();
    assert_eq!(list(&wb), [s("A20:A21", 1), s("A1:A5", 2), s("B1:B5", 3)]);
    let mut wb = book();
    let c = clip::copy(&wb, 0, parse_range("B1:B2").unwrap()).unwrap();
    clip::paste(&mut wb, 0, CellRef::parse("B20").unwrap(), &c, None).unwrap();
    assert_eq!(
        list(&wb),
        [
            s("B20:B21", 1),
            s("B20:B21", 2),
            s("A1:A5", 3),
            s("B1:B5", 4)
        ]
    );
    let mut wb = book();
    clip::fill(&mut wb, 0, parse_range("A5:A7").unwrap(), true).unwrap();
    assert_eq!(list(&wb), [s("A1:A7", 1), s("B1:B5", 4)]);
    let mut wb = book();
    clip::move_cells(
        &mut wb,
        0,
        parse_range("A1").unwrap(),
        0,
        CellRef::parse("A30").unwrap(),
    )
    .unwrap();
    assert_eq!(list(&wb), [s("A2:A5 A30", 1), s("B1:B5", 3)]);
}

// 메모 상자 대조 실험: 칸이 옮겨진 메모의 상자는 메모 칸의 열 + 14px, 두 행 위 + 10px 로 가고 크기는 그대로
// (A 열 메모는 옮길 때마다 상자가 A 열 너비 − 1px 만큼 좁아진다)
#[test]
fn comment_boxes_follow_moved_cells_like_hcell() {
    let mut wb = hcell_book();
    wb.sheets[0].default_col_width = Some(9.00390625);
    wb.sheets[0].default_row_height = Some(16.4);
    let note = |a1: &str, anchor: [u32; 8]| Comment {
        cell: CellRef::parse(a1).unwrap(),
        shape: Some(CommentShape::hcell_default(anchor)),
        ..Default::default()
    };
    wb.sheets[0].comments = vec![
        note("A10", [0, 14, 7, 10, 1, 69, 11, 1]),
        note("B10", [2, 14, 8, 10, 3, 69, 12, 1]),
    ];
    put(&mut wb.sheets[0], "Z1", CellValue::Number(1.0), None);
    let boxes = |wb: &Workbook| {
        wb.sheets[0]
            .comments
            .iter()
            .map(|c| (c.cell.to_string(), c.shape.as_ref().unwrap().anchor))
            .collect::<Vec<_>>()
    };
    // 메모 아래에 넣으면 그대로
    insert_rows(&mut wb, 0, 29, 1).unwrap();
    assert_eq!(boxes(&wb)[1].1, [2, 14, 8, 10, 3, 69, 12, 1]);
    insert_rows(&mut wb, 0, 0, 1).unwrap();
    assert_eq!(
        boxes(&wb),
        [
            ("A11".to_string(), [0, 14, 8, 10, 0, 70, 12, 1]),
            ("B11".to_string(), [1, 14, 8, 10, 2, 69, 12, 1])
        ]
    );
    insert_rows(&mut wb, 0, 0, 1).unwrap();
    assert_eq!(
        boxes(&wb),
        [
            ("A12".to_string(), [0, 14, 9, 10, 0, 0, 13, 1]),
            ("B12".to_string(), [1, 14, 9, 10, 2, 69, 13, 1])
        ]
    );
}

// 페이지 나누기 대조 실험: 여러 행·열을 넣거나 지우면 범위의 첫·끝 자리 나누기는 그대로 둔다
#[test]
fn page_breaks_like_hcell() {
    let run = |f: &dyn Fn(&mut Workbook)| {
        let mut wb = refbase();
        wb.sheets[0].page.col_breaks = vec![2, 3, 4, 5, 6, 8];
        f(&mut wb);
        wb.sheets[0].page.col_breaks.clone()
    };
    assert_eq!(
        run(&|wb| delete_cols(wb, 0, 2, 2).unwrap()),
        [2, 3, 2, 3, 4, 6]
    );
    assert_eq!(
        run(&|wb| delete_cols(wb, 0, 4, 3).unwrap()),
        [2, 3, 4, 6, 5]
    );
    assert_eq!(
        run(&|wb| delete_cols(wb, 0, 3, 1).unwrap()),
        [2, 3, 4, 5, 7]
    );
    assert_eq!(
        run(&|wb| insert_cols(wb, 0, 2, 2).unwrap()),
        [2, 3, 6, 7, 8, 10]
    );
    assert_eq!(
        run(&|wb| insert_cols(wb, 0, 3, 1).unwrap()),
        [2, 4, 5, 6, 7, 9]
    );
}

// 병합 대조 실험: 열을 지워 병합이 한 칸이 되거나 왼쪽 위 칸이 지워질 때
#[test]
fn merges_shrunk_by_deleted_columns_like_hcell() {
    let book = || {
        let mut wb = hcell_book();
        put_edge(&mut wb, "A2", "R", Some("000000"));
        put_edge(&mut wb, "B1", "B", Some("000000"));
        put_edge(&mut wb, "C1", "B", Some("000000"));
        put_edge(&mut wb, "B3", "T", Some("000000"));
        put_edge(&mut wb, "C3", "T", Some("000000"));
        put_edge(&mut wb, "D2", "L", Some("000000"));
        put(&mut wb.sheets[0], "B2", CellValue::Text("m".into()), None);
        merge(&mut wb, 0, parse_range("B2:C2").unwrap(), false).unwrap();
        put_edge(&mut wb, "E5", "L", Some("000000"));
        put(&mut wb.sheets[0], "B5", CellValue::Text("n".into()), None);
        merge(&mut wb, 0, parse_range("B5:D5").unwrap(), false).unwrap();
        wb
    };
    // 왼쪽 위 칸이 남아 한 칸이 되면 왼쪽·위·아래 변에 보이던 선을 적는다 (오른쪽은 그대로)
    let mut wb = book();
    delete_cols(&mut wb, 0, 2, 1).unwrap();
    assert_eq!(edges(&wb, "B2"), "000000 - 000000 000000");
    assert_eq!(edges(&wb, "A2"), "- 000000 - -");
    // 왼쪽 위 칸을 지우면 값이 남는 칸으로 가고, 지운 쪽 변은 남는 칸 자리의 선(병합 안쪽이라 없음) — 앞 이웃의
    // 맞닿은 선도 지운다
    let mut wb = book();
    delete_cols(&mut wb, 0, 1, 1).unwrap();
    let b2 = wb.sheets[0].cell(CellRef::parse("B2").unwrap()).unwrap();
    assert_eq!(b2.value, CellValue::Text("m".into()));
    assert_eq!(edges(&wb, "B2"), "- - 000000 000000");
    assert_eq!(edges(&wb, "A2"), "- - - -");
    // 여러 칸이 남으면 값은 새 첫 칸으로, 오른쪽 둘레에 보이던 선을 적는다
    let b5 = wb.sheets[0].cell(CellRef::parse("B5").unwrap()).unwrap();
    assert_eq!(b5.value, CellValue::Text("n".into()));
    assert_eq!(edges(&wb, "C5"), "- 000000 - -");
    assert_eq!(merges(&wb), ["B5:C5"]);
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

// 데이터 도구 대조 실험 (한셀 `Range.RemoveDuplicates`·`TextToColumns`·`Subtotal`)
fn fill_rows(wb: &mut Workbook, rows: &[&[&str]]) {
    for (i, r) in rows.iter().enumerate() {
        for (j, v) in r.iter().enumerate() {
            let at = CellRef {
                row: i as u32,
                col: j as u32,
            };
            set_input(wb, 0, at, v).unwrap();
        }
    }
}

fn text_at(wb: &Workbook, a1: &str) -> String {
    let c = a1.parse::<CellRef>().unwrap();
    match wb.sheets[0].cell(c).map(|x| x.value.clone()) {
        Some(CellValue::Text(t)) => t,
        Some(CellValue::Number(n)) => crate::model::format_general(n),
        _ => String::new(),
    }
}

#[test]
fn remove_duplicates_like_hcell() {
    let mut wb = hcell_book();
    fill_rows(
        &mut wb,
        &[
            &["k", "v"],
            &["a", "1"],
            &["A", "1"],
            &["a", "2"],
            &["'1", "x"],
            &["1", "x"],
        ],
    );
    // 머리글이 있고 두 열로: 대소문자는 무시, 글 `1` 과 수 1 은 다르다
    let n =
        data::remove_duplicates(&mut wb, 0, parse_range("A1:B6").unwrap(), &[1, 2], true).unwrap();
    assert_eq!(n, 1);
    let col: Vec<String> = (1..=6).map(|r| text_at(&wb, &format!("B{r}"))).collect();
    assert_eq!(col, ["v", "1", "2", "x", "x", ""]);
}

#[test]
fn text_to_columns_like_hcell() {
    let mut wb = hcell_book();
    fill_rows(&mut wb, &[&["a,\"b,c\",007"], &["1,,x"]]);
    let o = data::SplitOptions {
        comma: true,
        qualifier: Some('"'),
        ..Default::default()
    };
    data::text_to_columns(
        &mut wb,
        0,
        parse_range("A1:A2").unwrap(),
        "C1".parse().ok(),
        &o,
    )
    .unwrap();
    assert_eq!(text_at(&wb, "C1"), "a");
    assert_eq!(text_at(&wb, "D1"), "b,c");
    assert_eq!(text_at(&wb, "E1"), "7");
    assert_eq!(text_at(&wb, "C2"), "1");
    assert_eq!(text_at(&wb, "D2"), "");
    assert_eq!(text_at(&wb, "E2"), "x");
}

#[test]
fn subtotal_like_hcell() {
    let mut wb = hcell_book();
    fill_rows(
        &mut wb,
        &[&["부서", "금액"], &["A", "10"], &["A", "20"], &["B", "5"]],
    );
    let o = data::SubtotalOptions {
        group_by: 1,
        func: "sum".into(),
        totals: vec![2],
        replace: true,
        page_breaks: false,
        summary_below: true,
    };
    data::subtotal(&mut wb, 0, parse_range("A1").unwrap(), &o).unwrap();
    // 머리글, A·A, A 합계, B, B 합계, 총 합계
    let col: Vec<String> = (1..=7).map(|r| text_at(&wb, &format!("A{r}"))).collect();
    assert_eq!(col, ["부서", "A", "A", "A 합계", "B", "B 합계", "총 합계"]);
    let f = |a1: &str| {
        wb.sheets[0]
            .cell(a1.parse().unwrap())
            .and_then(|c| c.formula.as_ref())
            .map(|f| f.text.clone())
            .unwrap_or_default()
    };
    assert_eq!(f("B4"), "SUBTOTAL(9,B2:B3)");
    assert_eq!(f("B6"), "SUBTOTAL(9,B5:B5)");
    assert_eq!(f("B7"), "SUBTOTAL(9,B2:B5)");
    let levels: Vec<(u32, u8, bool)> = wb.sheets[0]
        .rows
        .iter()
        .map(|r| (r.index, r.outline_level, r.collapsed))
        .collect();
    assert!(levels.contains(&(2, 2, false)) && levels.contains(&(4, 1, true)));
    assert!(levels.contains(&(7, 0, true)) && levels.contains(&(8, 0, true)));
}

// 한셀 자동화의 중복 제거·통합·텍스트 나누기는 시트 보호를 보지 않는다 (잠긴 칸도 덮어쓴다 — 실험). 부분합은 한셀이 오류로 멈춘다
#[test]
fn data_tools_ignore_sheet_protection_like_hcell() {
    let mut wb = hcell_book();
    fill_rows(&mut wb, &[&["a 1"], &["a 1"], &["b 2"]]);
    wb.sheets[0].protection = Some(crate::model::SheetProtection {
        password: None,
        flags: vec![("sheet".into(), true)],
    });
    let n = data::remove_duplicates(&mut wb, 0, parse_range("A1:A3").unwrap(), &[], false).unwrap();
    assert_eq!(n, 1);
    let o = data::SplitOptions {
        space: true,
        ..Default::default()
    };
    data::text_to_columns(&mut wb, 0, parse_range("A1:A2").unwrap(), None, &o).unwrap();
    assert_eq!(text_at(&wb, "B1"), "1");
    let so = data::SubtotalOptions {
        group_by: 1,
        func: "sum".into(),
        totals: vec![2],
        replace: true,
        page_breaks: false,
        summary_below: true,
    };
    assert!(data::subtotal(&mut wb, 0, parse_range("A1:B2").unwrap(), &so).is_err());
}

// 통합 이름표: 숫자 이름표는 숫자로 쓰고, 이름표 칸에 이미 내용이 있으면 그대로 둔다 (한셀 대조 실험 cons3)
#[test]
fn consolidate_labels_like_hcell() {
    let mut wb = hcell_book();
    fill_rows(
        &mut wb,
        &[&["k", "10", "20"], &["a", "1", "2"], &["a", "3", "4"]],
    );
    set_input(&mut wb, 0, "E1".parse().unwrap(), "x").unwrap();
    let o = data::ConsolidateOptions {
        func: "sum".into(),
        top_row: true,
        left_column: true,
    };
    let src = vec![(0, parse_range("A1:C3").unwrap())];
    data::consolidate(&mut wb, 0, "D1".parse().unwrap(), &src, &o).unwrap();
    // E1 은 이미 내용이 있어 이름표(10)를 쓰지 않고, F1 은 숫자 20
    assert_eq!(text_at(&wb, "E1"), "x");
    assert!(matches!(
        wb.sheets[0].cell("F1".parse().unwrap()).map(|c| &c.value),
        Some(CellValue::Number(n)) if *n == 20.0
    ));
    assert_eq!(text_at(&wb, "D2"), "a");
    assert_eq!(text_at(&wb, "E2"), "4");
}

// 줄 바꿈이 든 글을 넣으면 한셀은 자동 줄 바꿈을 켠다 (수식 결과는 그렇지 않다)
#[test]
fn newline_input_turns_on_wrap_like_hcell() {
    let mut wb = hcell_book();
    let wrap = |wb: &Workbook, a: &str| {
        let st = wb.sheets[0].cell(a.parse().unwrap()).map_or(0, |c| c.style);
        wb.styles
            .xf(st)
            .and_then(|x| x.alignment.as_ref())
            .is_some_and(|a| a.wrap_text)
    };
    set_input(&mut wb, 0, "A1".parse().unwrap(), "a\nb").unwrap();
    set_input(&mut wb, 0, "A2".parse().unwrap(), "ab").unwrap();
    set_input(&mut wb, 0, "A3".parse().unwrap(), "=\"a\"&CHAR(10)&\"b\"").unwrap();
    assert!(wrap(&wb, "A1"));
    assert!(!wrap(&wb, "A2"));
    assert!(!wrap(&wb, "A3"));
}

#[test]
fn formula_local_names_like_hcell() {
    assert_eq!(
        formula_local("DBCS(A1)&DOLLAR(1)&USDOLLAR(2)"),
        "JUNJA(A1)&WON(1)&DOLLAR(2)"
    );
    assert_eq!(
        formula_local("SUM(\"DOLLAR(\")+DOLLARDE(1,2)"),
        "SUM(\"DOLLAR(\")+DOLLARDE(1,2)"
    );
}
