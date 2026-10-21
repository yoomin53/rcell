// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 계산 엔진 단위 테스트 (직접 만든 통합 문서).

use super::*;
use crate::model::{Cell, CellRef, CellValue, DefinedName, Formula, Sheet, Workbook};

/// ("A1", "=수식" 또는 값) 목록으로 시트를 만든다
fn sheet(name: &str, cells: &[(&str, &str)]) -> Sheet {
    let mut out: Vec<Cell> = cells
        .iter()
        .map(|(r, v)| {
            let (value, formula) = if let Some(f) = v.strip_prefix('=') {
                (
                    CellValue::Empty,
                    Some(Formula {
                        text: f.to_string(),
                        ..Default::default()
                    }),
                )
            } else if let Ok(n) = v.parse::<f64>() {
                (CellValue::Number(n), None)
            } else if *v == "TRUE" || *v == "FALSE" {
                (CellValue::Bool(*v == "TRUE"), None)
            } else {
                (CellValue::Text(v.to_string()), None)
            };
            Cell {
                r: CellRef::parse(r).unwrap(),
                style: 0,
                value,
                formula,
            }
        })
        .collect();
    out.sort_by_key(|c| (c.r.row, c.r.col));
    Sheet {
        name: name.into(),
        cells: out,
        ..Default::default()
    }
}

fn wb(sheets: Vec<Sheet>) -> Workbook {
    Workbook {
        sheets,
        ..Default::default()
    }
}

fn calc(w: &Workbook, at: &str) -> CellValue {
    let opts = CalcOptions {
        now: Some(46314.5),
        ..Default::default()
    };
    let (res, _) = evaluate(w, &opts);
    let r = CellRef::parse(at).unwrap();
    res.into_iter()
        .find(|x| x.sheet == 0 && x.cell == r)
        .map(|x| x.value)
        .unwrap()
}

fn one(formula: &str) -> CellValue {
    let w = wb(vec![sheet(
        "S",
        &[
            ("A1", "10"),
            ("A2", "20"),
            ("A3", "30"),
            ("A4", "40"),
            ("A5", "50"),
            ("B1", "항목1"),
            ("B2", "항목2"),
            ("C1", "TRUE"),
            ("Z99", &format!("={formula}")),
        ],
    )]);
    calc(&w, "Z99")
}

fn n(v: f64) -> CellValue {
    CellValue::Number(v)
}

fn t(s: &str) -> CellValue {
    CellValue::Text(s.into())
}

fn e(s: &str) -> CellValue {
    CellValue::Error(s.into())
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(one("1+2*3"), n(7.0));
    assert_eq!(one("-2^2"), n(4.0));
    assert_eq!(one("2^3^2"), n(64.0));
    assert_eq!(one("A1%"), n(0.1));
    assert_eq!(one("1/0"), e("#DIV/0!"));
    assert_eq!(one("\"a\"+1"), e("#VALUE!"));
    assert_eq!(one("\"3\"*2"), n(6.0));
    assert_eq!(one("1&2"), t("12"));
    assert_eq!(one("B1&B2"), t("항목1항목2"));
    assert_eq!(one("A1<>A2"), CellValue::Bool(true));
    assert_eq!(one("\"abc\"=\"ABC\""), CellValue::Bool(true));
    assert_eq!(one("1<\"a\""), CellValue::Bool(true));
}

#[test]
fn aggregates() {
    assert_eq!(one("SUM(A1:A5)"), n(150.0));
    assert_eq!(one("SUM(A:A)"), n(150.0));
    assert_eq!(one("AVERAGE(A1:A5)"), n(30.0));
    assert_eq!(one("MAX(A1:A5)-MIN(A1:A5)"), n(40.0));
    assert_eq!(one("COUNT(A1:B5)"), n(5.0));
    assert_eq!(one("COUNTA(A1:B5)"), n(7.0));
    assert_eq!(one("SUMIF(A1:A5,\">20\")"), n(120.0));
    assert_eq!(one("COUNTIF(A1:A5,\">=30\")"), n(3.0));
    assert_eq!(one("SUMPRODUCT(A1:A2,A3:A4)"), n(1100.0));
    assert_eq!(one("SUMPRODUCT((A1:A5>20)*1)"), n(3.0));
    assert_eq!(one("SUM(1,\"2\",TRUE)"), n(4.0));
}

#[test]
fn logic_and_lookup() {
    assert_eq!(one("IF(A1>5,\"크다\",\"작다\")"), t("크다"));
    assert_eq!(one("IF(FALSE,1)"), CellValue::Bool(false));
    assert_eq!(one("AND(A1>0,A2>0)"), CellValue::Bool(true));
    assert_eq!(one("IFERROR(1/0,0)"), n(0.0));
    assert_eq!(one("VLOOKUP(20,A1:B5,2,FALSE)"), t("항목2"));
    assert_eq!(one("INDEX(A1:A5,3)"), n(30.0));
    assert_eq!(one("MATCH(40,A1:A5,0)"), n(4.0));
    assert_eq!(one("MATCH(35,A1:A5)"), n(3.0));
    assert_eq!(one("SUM(OFFSET(A1,1,0,2,1))"), n(50.0));
    assert_eq!(one("SUM(INDIRECT(\"A1:A3\"))"), n(60.0));
    assert_eq!(one("CHOOSE(2,A1,A2,A3)"), n(20.0));
    assert_eq!(one("ROWS(A1:B5)*COLUMNS(A1:B5)"), n(10.0));
}

#[test]
fn text_functions() {
    assert_eq!(one("LEFT(B1,1)"), t("항"));
    assert_eq!(one("LEN(B1)"), n(3.0));
    assert_eq!(one("CONCATENATE(B1,\"-\",B2)"), t("항목1-항목2"));
    assert_eq!(one("TEXT(A1,\"0.00\")"), t("10.00"));
    assert_eq!(one("PROPER(\"hello WORLD\")"), t("Hello World"));
    assert_eq!(one("TRIM(\"  a   b \")"), t("a b"));
    assert_eq!(one("SUBSTITUTE(\"aaa\",\"a\",\"b\",2)"), t("aba"));
    assert_eq!(one("FIND(\"b\",\"abc\")"), n(2.0));
    assert_eq!(one("SEARCH(\"B?\",\"abc\")"), n(2.0));
    assert_eq!(one("FIXED(1234.567,1)"), t("1,234.6"));
    assert_eq!(one("1/3&\"\""), t("0.333333333333333"));
}

#[test]
fn math_functions() {
    assert_eq!(one("ROUND(2.675,2)"), n(2.68));
    assert_eq!(one("ROUND(-2.5,0)"), n(-3.0));
    assert_eq!(one("ROUNDUP(0.1+0.2,1)"), n(0.3));
    assert_eq!(one("MOD(-3,2)"), n(1.0));
    assert_eq!(one("INT(-7.8)"), n(-8.0));
    assert_eq!(one("CEILING(2.5,1)"), n(3.0));
    assert_eq!(one("FLOOR(-2.5,1)"), n(-3.0));
    assert_eq!(one("ROMAN(499,0)"), t("CDXCIX"));
    assert_eq!(one("ROMAN(499,1)"), t("LDVLIV"));
    assert_eq!(one("ROMAN(499,4)"), t("ID"));
    assert_eq!(one("FACT(5)"), n(120.0));
    assert_eq!(one("COMBIN(5,2)"), n(10.0));
    assert_eq!(one("GCD(12,18)"), n(6.0));
}

#[test]
fn dates() {
    assert_eq!(one("DATE(2026,10,19)"), n(46314.0));
    assert_eq!(one("YEAR(46314)"), n(2026.0));
    assert_eq!(one("DATE(1900,2,29)"), n(60.0));
    assert_eq!(one("WEEKDAY(46314)"), n(2.0));
    assert_eq!(one("EOMONTH(46314,1)"), n(46356.0));
    assert_eq!(one("DATEDIF(DATE(2020,1,15),DATE(2026,10,19),\"Y\")"), n(6.0));
    assert_eq!(one("TODAY()"), n(46314.0));
    assert_eq!(one("YEAR(NOW())"), n(2026.0));
}

#[test]
fn names_sheets_and_chains() {
    let mut w = wb(vec![
        sheet("첫 시트", &[("A1", "7"), ("A2", "=A1*2"), ("A3", "=A2+'둘'!A1")]),
        sheet("둘", &[("A1", "=SUM('첫 시트'!A1:A2)")]),
    ]);
    w.defined_names.push(DefinedName {
        name: "가격".into(),
        formula: "'첫 시트'!$A$1".into(),
        ..Default::default()
    });
    w.sheets[0].cells.push(Cell {
        r: CellRef::parse("B1").unwrap(),
        style: 0,
        value: CellValue::Empty,
        formula: Some(Formula {
            text: "가격*3".into(),
            ..Default::default()
        }),
    });
    w.sheets[0].cells.sort_by_key(|c| (c.r.row, c.r.col));
    assert_eq!(calc(&w, "A3"), n(35.0));
    assert_eq!(calc(&w, "B1"), n(21.0));
    let report = recalculate(&mut w);
    assert_eq!(report.formula_cells, 4);
    assert_eq!(report.cycles, 0);
}

#[test]
fn long_chain_does_not_overflow() {
    let mut cells: Vec<(String, String)> = vec![("A1".into(), "1".into())];
    for i in 2..=5000 {
        cells.push((format!("A{i}"), format!("=A{}+1", i - 1)));
    }
    let refs: Vec<(&str, &str)> = cells.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let w = wb(vec![sheet("S", &refs)]);
    assert_eq!(calc(&w, "A5000"), n(5000.0));
    // 역순 의존 (위 셀이 아래 셀을 참조)
    let mut cells: Vec<(String, String)> = vec![("A5000".into(), "1".into())];
    for i in 1..5000 {
        cells.push((format!("A{i}"), format!("=A{}+1", i + 1)));
    }
    let refs: Vec<(&str, &str)> = cells.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let w = wb(vec![sheet("S", &refs)]);
    assert_eq!(calc(&w, "A1"), n(5000.0));
}

#[test]
fn cycles_are_reported() {
    let w = wb(vec![sheet("S", &[("A1", "=B1+1"), ("B1", "=A1+1"), ("C1", "=5")])]);
    let (_, report) = evaluate(&w, &CalcOptions::default());
    assert_eq!(report.cycles, 2);
    assert_eq!(calc(&w, "C1"), n(5.0));
}

#[test]
fn array_formula_and_lifting() {
    assert_eq!(one("SUM(1/{1,2,4})"), n(1.75));
    assert_eq!(one("SUM(LEN({\"a\",\"bb\"}))"), n(3.0));
    assert_eq!(one("INDEX({1,2;3,4},2,1)"), n(3.0));
    assert_eq!(one("MMULT({1,2},{3;4})"), n(11.0));
    assert_eq!(one("SUMPRODUCT(LEN(B1:B2))"), n(6.0));
}

#[test]
fn same_value_rules() {
    assert!(same_value(&n(0.1 + 0.2), &n(0.3), None, false));
    assert!(!same_value(&n(1.0), &n(1.001), None, false));
    assert!(same_value(&n(1.0004), &n(1.0), Some("0.00"), false));
    assert!(same_value(&e("#DIV/0!"), &e("#DIV/0!"), None, false));
    assert!(!same_value(&t("1"), &n(1.0), None, false));
}
