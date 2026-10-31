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
    assert_eq!(
        one("DATEDIF(DATE(2020,1,15),DATE(2026,10,19),\"Y\")"),
        n(6.0)
    );
    assert_eq!(one("TODAY()"), n(46314.0));
    assert_eq!(one("YEAR(NOW())"), n(2026.0));
}

#[test]
fn names_sheets_and_chains() {
    let mut w = wb(vec![
        sheet(
            "첫 시트",
            &[("A1", "7"), ("A2", "=A1*2"), ("A3", "=A2+'둘'!A1")],
        ),
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
    let refs: Vec<(&str, &str)> = cells
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let w = wb(vec![sheet("S", &refs)]);
    assert_eq!(calc(&w, "A5000"), n(5000.0));
    // 역순 의존 (위 셀이 아래 셀을 참조)
    let mut cells: Vec<(String, String)> = vec![("A5000".into(), "1".into())];
    for i in 1..5000 {
        cells.push((format!("A{i}"), format!("=A{}+1", i + 1)));
    }
    let refs: Vec<(&str, &str)> = cells
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let w = wb(vec![sheet("S", &refs)]);
    assert_eq!(calc(&w, "A1"), n(5000.0));
}

#[test]
fn cycles_are_reported() {
    let w = wb(vec![sheet(
        "S",
        &[("A1", "=B1+1"), ("B1", "=A1+1"), ("C1", "=5")],
    )]);
    let (_, report) = evaluate(&w, &CalcOptions::default());
    assert_eq!(report.cycles, 2);
    assert_eq!(calc(&w, "C1"), n(5.0));
}

/// BINOM.DIST 확률 질량: 조합 수를 e^{lnΓ 차이} 로 구한다 (대조 실험 dist4, 한셀 값과 끝자리까지)
#[test]
fn binom_pmf_uses_gammaln() {
    assert_eq!(
        one("BINOM.DIST(10,20,0.5,FALSE)"),
        n(0.176_197_052_001_953_24)
    );
    assert_eq!(
        one("BINOM.DIST(5,10,0.1,FALSE)"),
        n(0.001_488_034_800_000_000_8)
    );
    assert_eq!(
        one("BINOM.DIST(50,100,0.3,TRUE)"),
        n(0.999_990_965_313_804_3)
    );
    assert_eq!(
        one("BINOM.DIST(170,171,0.9,TRUE)"),
        n(0.999_999_985_021_472_7)
    );
}

/// 무작위 수식 차분 시험(rand)에서 찾은 한셀의 규칙들 (대조 실험 rand1xx·typing·refargs·misc3–6)
#[test]
fn rules_from_random_differential_tests() {
    // 참조가 필요한 인수에 값·배열을 주면
    assert_eq!(one("COUNTIF({1,2,3},\">1\")"), e("#VALUE!"));
    assert_eq!(one("SUMIF({1,2,3},\">1\")"), e("#VALUE!"));
    assert_eq!(one("COUNTBLANK({1,2;3,4})"), e("#NAME?"));
    assert_eq!(one("ROW({1,2})"), e("#REF!"));
    assert_eq!(one("SHEETS(5)"), e("#N/A"));
    assert_eq!(one("MATCH(5,5,0)"), e("#N/A"));
    assert_eq!(one("RANK(3.14159,-255)"), e("#N/A"));
    // 숫자로 읽히는 글은 논리로도 읽힌다
    assert_eq!(one("IF(\"0\",1,2)"), n(2.0));
    assert_eq!(one("NOT(\"5\")"), CellValue::Bool(false));
    // SWITCH 는 같은 오류끼리 맞는다
    assert_eq!(one("SWITCH(#N/A,#N/A,1,2)"), n(1.0));
    // 비트 함수·DEC2BIN 은 소수를 버리기 전에 범위를 잰다
    assert_eq!(one("BITXOR(1023,-0.69)"), e("#NUM!"));
    assert_eq!(one("BITAND(-0.5,1)"), n(0.0));
    assert_eq!(one("DEC2BIN(511.5)"), e("#NUM!"));
    // TEXTJOIN: 구분자가 오류면 빈 글, 배열 상수 안의 오류는 그 오류
    assert_eq!(one("TEXTJOIN(#N/A,1,\"a\",\"b\")"), t(""));
    assert_eq!(one("TEXTJOIN(\"/\",1,{\"a\",#N/A})"), e("#N/A"));
    // 달 수는 16비트로 잘린다, 1899-12-31 이하는 #NUM!
    assert_eq!(one("EDATE(36923,65535)"), n(36892.0));
    assert_eq!(one("EOMONTH(31,-1)"), e("#NUM!"));
    // 자릿수는 32비트
    assert_eq!(one("ROUND(1234.5678,2147483648)"), e("#NUM!"));
    assert_eq!(one("TRUNC(1234.5678,306)"), n(0.0));
    // 와이블 밀도는 넘치면 0
    assert_eq!(one("WEIBULL(65535,36526.5,10,FALSE)"), n(0.0));
    // CHAR 의 두 바이트 코드
    assert_eq!(one("CHAR(45217)"), t("가"));
    assert_eq!(one("CHAR(1000)"), t("�"));
    assert_eq!(one("CHAR(256)"), e("#VALUE!"));
    // TEXT: 논리에 색 서식은 #VALUE!, 빈 서식은 글 그대로·음수는 "-"
    assert_eq!(one("TEXT(TRUE,\"[Red]0.0\")"), e("#VALUE!"));
    assert_eq!(one("TEXT(-5,\"\")"), t("-"));
    assert_eq!(one("TEXT(\"abc\",\"\")"), t("abc"));
    // 조합 수는 e^{GAMMALN 차이}
    assert_eq!(one("COMBIN(1030,2)"), n(529935.0));
    assert_eq!(one("COMBIN(1030,515)"), e("#NUM!"));
    match one("PERMUT(45598,1)") {
        CellValue::Number(x) => assert!((x - 45597.99999943975).abs() < 1e-9),
        other => panic!("{other:?}"),
    }
    // 오류 값 이름의 글은 그 오류 (대소문자·뒤 공백 무시)
    assert_eq!(one("NOT(\"#N/A\")"), e("#N/A"));
    assert_eq!(one("\"#div/0!\"+1"), e("#DIV/0!"));
    assert_eq!(one("VALUE(\"#NUM! \")"), e("#NUM!"));
    // MULTINOMIAL·GCD: 글·논리는 #VALUE!, 1 보다 작은 값은 0, 짝이 0 이면 소수째
    assert_eq!(one("MULTINOMIAL(3,\"2\")"), e("#VALUE!"));
    assert_eq!(one("GCD(2.9)"), n(2.9));
    assert_eq!(one("GCD(0.99)"), n(0.0));
    assert_eq!(one("GCD(2.9,4)"), n(2.0));
    assert_eq!(one("GCD(1E20)"), n(0.0));
    assert_eq!(one("LCM(2.9,0)"), n(0.0));
    // UNICHAR 의 U+10FFFE·U+10FFFF 는 #N/A
    assert_eq!(one("UNICHAR(1114111)"), e("#N/A"));
    assert_eq!(one("LEN(UNICHAR(65535))"), n(1.0));
    // DB: 첫 해가 열두 달이면 기간은 수명까지, 아니면 수명 + 1 까지, 수명을 넘는 해는 남은 달수만큼
    assert_eq!(one("DB(1000,100,3,4)"), e("#NUM!"));
    match one("DB(1000,100,2.5,3,6)") {
        CellValue::Number(x) => assert!((x - 83.738802).abs() < 1e-6),
        other => panic!("{other:?}"),
    }
    // TRUNC 은 10^자리가 0 이 되면 #NUM!, FLOOR.PRECISE 는 배수 0 이면 첫 인수를 보지 않는다
    assert_eq!(one("TRUNC(1023,-323)"), n(0.0));
    assert_eq!(one("TRUNC(1023,-324)"), e("#NUM!"));
    assert_eq!(one("FLOOR.PRECISE(NA(),0)"), n(0.0));
    assert_eq!(one("CEILING.PRECISE(NA(),0)"), e("#N/A"));
    // COMBIN·PERMUT 은 n 이 2^31 − 2 까지
    assert_eq!(one("COMBIN(2147483647,1)"), e("#NUM!"));
    // RANK: 값의 오류가 먼저, 다음이 범위의 오류, 숫자로 읽히는 글 범위는 #N/A
    assert_eq!(one("RANK(1/0,NA())"), e("#DIV/0!"));
    assert_eq!(one("RANK(\"x\",NA())"), e("#N/A"));
    assert_eq!(one("RANK(1,\"50%\")"), e("#N/A"));
    // QUARTILE.EXC 의 순위가 범위 밖이면 0
    assert_eq!(one("QUARTILE.EXC({3},1)"), n(0.0));
    // 음수 큰 수의 글: 15자리 아래는 올림 (양수는 버림)
    assert_eq!(one("(-(2^50))&\"\""), t("-1125899906842630"));
    assert_eq!(one("(2^50)&\"\""), t("1125899906842620"));
    // PERMUTATIONA 는 n!/(n−k)! + n
    assert_eq!(one("PERMUTATIONA(10,3)"), n(730.0));
    assert_eq!(one("PERMUTATIONA(5,5)"), n(125.0));
    assert_eq!(one("PERMUTATIONA(2,0)"), n(1.0));
    assert_eq!(one("PERMUTATIONA(171,1)"), e("#NUM!"));
    assert_eq!(one("PERMUTATIONA(1,2)"), e("#NUM!"));
    // 보통 칸에서 낱값 인수의 배열은 첫 원소, MUNIT(배열) 은 낱값 #VALUE!
    assert_eq!(one("TYPE(TRIM({\"a\",\"b\"}))"), n(2.0));
    assert_eq!(one("TYPE({1,2}*2)"), n(64.0));
    // 오류 낱값은 한 칸 배열: 크기가 다르면 #VALUE!, 같으면 그 오류
    assert_eq!(one("SUMPRODUCT({1,2},NA())"), e("#VALUE!"));
    assert_eq!(one("SUMPRODUCT(NA())"), e("#N/A"));
    // BIT 함수는 인수의 오류를 범위보다 먼저
    assert_eq!(one("BITXOR(-0.5,#VALUE!)"), e("#VALUE!"));
    assert_eq!(one("BITXOR(-0.5,1)"), e("#NUM!"));
    // 수식 안 XOR: 식을 배열로 계산한다
    assert_eq!(one("XOR({1,2,3}>2)"), CellValue::Bool(true));
    assert_eq!(one("XOR({1,1,0})"), CellValue::Bool(false));
}

/// 반복 계산 (calcPr iterate): 파일 값에서 시작해 2×iterateCount 번, 행 차례로 (대조 실험 circ)
#[test]
fn iterative_calculation() {
    let mut w = wb(vec![sheet(
        "S",
        &[
            ("B1", "=B2+1"),
            ("B2", "=B1*0.5"),
            ("B3", "=B3+1"),
            ("B4", "=B6*10"),
            ("B5", "=B6+1"),
            ("B6", "=B5+1"),
        ],
    )]);
    // B3 의 파일 값 100
    w.sheets[0].cells[2].value = CellValue::Number(100.0);
    w.calc_attrs = vec![
        ("iterate".into(), "1".into()),
        ("iterateCount".into(), "2".into()),
    ];
    assert_eq!(calc(&w, "B1"), n(1.875));
    assert_eq!(calc(&w, "B2"), n(0.9375));
    assert_eq!(calc(&w, "B3"), n(104.0));
    // 뒤 행의 순환 칸은 한 바퀴 앞 값
    assert_eq!(calc(&w, "B4"), n(60.0));
    assert_eq!(calc(&w, "B6"), n(8.0));
}

/// 반복 계산을 켜지 않으면 같은 고리에 든 칸은 모두 계산하지 않고 지금 값을 둔다 (새 수식은 0). 고리 밖에서 읽는 칸은 그 값을
/// 읽는다 (바꾸기 대조 실험)
#[test]
fn cycle_cells_keep_their_value_without_iteration() {
    let mut w = wb(vec![sheet(
        "S",
        &[
            ("A1", "=IF(SUM(A1:A3),SUM(A1:A3),\"\")"),
            ("B1", "=SUM(B2:B4)"),
            ("B2", "=B1+1"),
            ("B3", "=IF(SUM(B1,B2),B1-B2,\"\")"),
            ("C1", "=B2+10"),
        ],
    )]);
    // 파일 값: A1 은 새 수식(값 없음), B1·B2 는 5·6
    for c in &mut w.sheets[0].cells {
        match (c.r.row, c.r.col) {
            (0, 1) => c.value = CellValue::Number(5.0),
            (1, 1) => c.value = CellValue::Number(6.0),
            _ => {}
        }
    }
    assert_eq!(calc(&w, "A1"), n(0.0));
    assert_eq!(calc(&w, "B1"), n(5.0));
    assert_eq!(calc(&w, "B2"), n(6.0));
    // B3 도 B1 → B3 → B1 고리에 든다 (B1 의 범위 B2:B4 에 들어 있다)
    assert_eq!(calc(&w, "B3"), n(0.0));
    assert_eq!(calc(&w, "C1"), n(16.0));
}

#[test]
fn array_formula_and_lifting() {
    assert_eq!(one("SUM(1/{1,2,4})"), n(1.75));
    // 보통 칸에서는 낱값 인수의 배열 상수가 첫 원소 (한셀 SUM(LEN({"a","bb"})) = 1), 배열 안에서는 원소마다
    assert_eq!(one("SUM(LEN({\"a\",\"bb\"}))"), n(1.0));
    assert_eq!(one("SUMPRODUCT(LEN({\"a\",\"bb\"}))"), n(3.0));
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

/// 표(A1:C5, 머리글 1행·요약 1행: 이름 수량 단가)와 구조적 참조
#[test]
fn structured_references() {
    use crate::model::{Table, TableColumn};
    let mut s = sheet(
        "S",
        &[
            ("A1", "이름"),
            ("B1", "수량"),
            ("C1", "단가"),
            ("A2", "가"),
            ("B2", "2"),
            ("C2", "10"),
            ("A3", "나"),
            ("B3", "3"),
            ("C3", "20"),
            ("A4", "다"),
            ("B4", "5"),
            ("C4", "30"),
            ("A5", "합계"),
            ("B5", "=SUBTOTAL(109,Table1[수량])"),
            (
                "D2",
                "=Table1[[#This Row],[수량]]*Table1[[#This Row],[단가]]",
            ),
            ("D3", "=[@수량]*[@단가]"),
            ("E1", "=SUM(Table1[[수량]:[단가]])"),
            ("E2", "=ROWS(Table1[#All])"),
            ("E3", "=ROWS(Table1)"),
            ("E4", "=Table1[[#Headers],[단가]]"),
            ("E5", "=Table1[[#Totals],[수량]]"),
            ("E6", "=COUNTA(Table1[[#Headers],[#Data],[이름]])"),
            ("E7", "=SUM(table1[수량])"),
            ("E8", "=Table1[없는열]"),
            ("E9", "=Table1[[#This Row],[수량]]"),
        ],
    );
    s.tables.push(Table {
        name: "Table1".into(),
        display_name: "Table1".into(),
        range: "A1:C5".into(),
        header_rows: 1,
        totals_rows: 1,
        columns: ["이름", "수량", "단가"]
            .iter()
            .map(|n| TableColumn {
                name: n.to_string(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    });
    let w = wb(vec![s]);
    assert_eq!(calc(&w, "B5"), n(10.0));
    assert_eq!(calc(&w, "D2"), n(20.0));
    assert_eq!(
        calc(&w, "D3"),
        e("#REF!"),
        "D 열은 표 밖이라 [@…] 의 표가 없다"
    );
    assert_eq!(calc(&w, "E1"), n(70.0));
    assert_eq!(calc(&w, "E2"), n(5.0));
    assert_eq!(calc(&w, "E3"), n(3.0));
    assert_eq!(calc(&w, "E4"), t("단가"));
    assert_eq!(calc(&w, "E5"), n(10.0));
    assert_eq!(calc(&w, "E6"), n(4.0));
    assert_eq!(calc(&w, "E7"), n(10.0));
    assert_eq!(calc(&w, "E8"), e("#REF!"));
    assert_eq!(
        calc(&w, "E9"),
        e("#VALUE!"),
        "9행은 표의 데이터 행이 아니다"
    );
}

/// 모델의 배열 수식 (FormulaKind::Array + 범위): 범위 칸마다 결과
#[test]
fn array_formula_from_model() {
    use crate::model::FormulaKind;
    let mut s = sheet("S", &[("A1", "1"), ("A2", "2"), ("A3", "3")]);
    s.cells.push(Cell {
        r: CellRef::parse("C1").unwrap(),
        style: 0,
        value: CellValue::Number(0.0),
        formula: Some(Formula {
            text: "A1:A3*10".into(),
            kind: FormulaKind::Array,
            range: Some("C1:C3".into()),
            ..Default::default()
        }),
    });
    s.cells.push(Cell {
        r: CellRef::parse("D1").unwrap(),
        style: 0,
        value: CellValue::Number(0.0),
        formula: Some(Formula {
            text: "SUM(A1:A3*A1:A3)".into(),
            kind: FormulaKind::Array,
            range: Some("D1".into()),
            ..Default::default()
        }),
    });
    s.cells.sort_by_key(|c| (c.r.row, c.r.col));
    let w = wb(vec![s]);
    assert_eq!(calc(&w, "C1"), n(10.0));
    assert_eq!(calc(&w, "C2"), n(20.0));
    assert_eq!(calc(&w, "C3"), n(30.0));
    assert_eq!(calc(&w, "D1"), n(14.0));
}

/// 데이터 표 (What-If): 1변수 열 입력과 2변수
#[test]
fn data_tables() {
    use crate::model::{DataTableInput, FormulaKind};
    let mut s = sheet(
        "S",
        &[
            ("A1", "0.05"),
            ("B1", "=A1*100"),
            ("C1", "3"),
            ("E1", "=B1"),
            ("D2", "0.1"),
            ("D3", "0.2"),
            ("G1", "=A1*C1"),
            ("H1", "1"),
            ("I1", "2"),
            ("G2", "10"),
            ("G3", "20"),
            ("K1", "=B1+1"),
        ],
    );
    let dt = |at: &str, rg: &str, inp: DataTableInput| Cell {
        r: CellRef::parse(at).unwrap(),
        style: 0,
        value: CellValue::Number(0.0),
        formula: Some(Formula {
            text: String::new(),
            kind: FormulaKind::DataTable,
            range: Some(rg.into()),
            data_table: Some(inp),
            ..Default::default()
        }),
    };
    s.cells.push(dt(
        "E2",
        "E2:E3",
        DataTableInput {
            r1: Some("A1".into()),
            ..Default::default()
        },
    ));
    s.cells.push(dt(
        "H2",
        "H2:I3",
        DataTableInput {
            two_d: true,
            r1: Some("A1".into()),
            r2: Some("C1".into()),
            ..Default::default()
        },
    ));
    s.cells.sort_by_key(|c| (c.r.row, c.r.col));
    let w = wb(vec![s]);
    assert_eq!(calc(&w, "E2"), n(10.0));
    assert_eq!(calc(&w, "E3"), n(20.0));
    assert_eq!(calc(&w, "H2"), n(10.0));
    assert_eq!(calc(&w, "I2"), n(20.0));
    assert_eq!(calc(&w, "H3"), n(20.0));
    assert_eq!(calc(&w, "I3"), n(40.0));
    // 입력 셀과 그에 기대는 수식은 원래 값으로 돌아온다
    assert_eq!(calc(&w, "B1"), n(5.0));
    assert_eq!(calc(&w, "K1"), n(6.0));
}
