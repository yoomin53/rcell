// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 대조 경우 목록.
//!
//! `Data` 시트 배치 (`main.rs` 가 채운다):
//! - A1:A10 정수 3 1 4 1 5 9 2 6 5 3 · B1:B10 실수 2.5 −1.25 0 7 10.5 −3 4.75 100 0.001 8
//! - C1:C10 섞임: 1, "abc", TRUE, 빈 칸, "5", 2.5, FALSE, "", 0, −4
//! - D1:D10 글 apple Banana cherry date Elder fig grape apple kiwi lemon · E1:E10 날짜 일련 번호
//! - F1:F10 10…100 (정렬) · G1:G10 r1…r10 · H1:H10 오류 섞임 1 #N/A 3 #DIV/0! 5 #VALUE! 7 #NUM! 9 #REF!
//! - I1:I10 a…j (정렬) · J1:J10 논리 · K1:K10 회귀용 y · L1:L10 양수
//! - M1:O3 3×3 행렬, M5:N6 2×2, M8:N9 특이 행렬
//! - Q1:T11 목록(Name Dept Age Score), 조건 표 V1:W3 · Y1:Y2 · V5:W6
//! - AA1:AA6 현금 흐름, AB1:AB6 그 날짜 · AC1:AD4 값·확률 · AE1:AE3 구간 · AF1:AF2 휴일
//! - AG1:AG4 복소수 글 · AH1:AH3 진법 글 · A12:E12 가로 숫자, A13:E13 가로 글
//!
//! `Hid` 시트: A1:A10 = 1…10, C1:C10, 3·6행 숨김, B5·B10 은 부분합 수식. `S1`–`S3`: A1:A3 숫자.
//! 이름: Nums, Rate, Two, Greeting, Arr, Local(통합 문서 + T 시트), DataTop, Both.

pub struct Case {
    pub f: String,
    /// 배열 수식이면 (행, 열)
    pub array: Option<(u32, u32)>,
}

fn c(f: impl Into<String>) -> Case {
    Case {
        f: f.into(),
        array: None,
    }
}

fn arr(f: impl Into<String>, h: u32, w: u32) -> Case {
    Case {
        f: f.into(),
        array: Some((h, w)),
    }
}

/// 이름(인수 목록) 경우들
fn calls(out: &mut Vec<Case>, name: &str, argsets: &[&str]) {
    for a in argsets {
        out.push(c(format!("{name}({a})")));
    }
}

/// 숫자 하나를 받는 함수에 넣어 볼 값
const NUM1: &[&str] = &[
    "0", "1", "-1", "0.5", "-0.5", "2.5", "-2.5", "3.7", "-3.7", "10", "123.456", "1E-10", "1E+15",
    "\"2\"", "TRUE", "FALSE", "Data!$C$4", "Data!$C$2", "\"\"", "#N/A", "Data!$H$4", "Data!$A$3",
    "Data!$C$5", "Data!$C$8",
];

fn unary(out: &mut Vec<Case>, name: &str, extra: &[&str]) {
    calls(out, name, NUM1);
    calls(out, name, extra);
}

/// 집계 함수에 넣어 볼 인수 목록
const AGG: &[&str] = &[
    "Data!A1:A10",
    "Data!B1:B10",
    "Data!C1:C10",
    "Data!A1:A10,Data!C1:C10",
    "1,2,3,\"4\",TRUE",
    "Data!H1:H10",
    "Data!D1:D10",
    "Data!C4",
    "5",
    "Data!L1:L10",
    "Data!A1:B10",
    "Data!A:A",
    "S1:S3!A1:A3",
    "(Data!A1:A3,Data!B1:B3)",
    "Data!A1:A3 Data!A2:B2",
    "Data!C2",
    "\"abc\"",
    "FALSE,TRUE",
    "Data!C1:C10,\"3\"",
    "{1,2,3;4,5,6}",
    "Nums",
    "-1,-2,-3",
    "0.1,0.2,0.3",
    "1E+300,1E+300",
];

fn agg(out: &mut Vec<Case>, name: &str) {
    calls(out, name, AGG);
}

// ---------------------------------------------------------------------------
// 연산자·참조·변환
// ---------------------------------------------------------------------------

fn ops() -> Vec<Case> {
    let mut v = Vec::new();
    for f in [
        "1+2*3", "(1+2)*3", "-2^2", "2^3^2", "-(2^2)", "2^-1", "10/4", "10/0", "0/0", "1-0.9",
        "0.1+0.2", "0.1+0.2-0.3", "1E+308*10", "-1E+308*10", "2^1024", "2^-1075", "5%", "5%%",
        "200%*3", "-5%", "--5", "+-+5", "1/3*3", "10^15+1", "2^53+1", "1E+15+0.3",
        "3-2-1", "2*3/4", "8/2/2", "2^0.5", "(-8)^(1/3)", "(-8)^0.5", "0^0", "0^-1", "(-2)^3",
        "(-2)^-2", "4^0.5", "1E+200*1E+200",
        // 비교
        "1=1", "1=1.0000000000000002", "0.1+0.2=0.3", "1<2", "\"a\"<\"b\"", "\"a\"<\"B\"", "\"A\"=\"a\"",
        "\"abc\"<\"abd\"", "\"10\"<\"9\"", "1<\"a\"", "\"a\"<TRUE", "1<TRUE", "FALSE<TRUE", "TRUE>1",
        "\"\"=0", "\"\"<0", "\"1\"=1", "1<>1", "2>=2", "2<=1", "\"가\">\"a\"", "\"가\"<\"나\"", "\"Z\"<\"a\"",
        "\"_\"<\"A\"", "\"-\"<\"A\"", "\" \"<\"!\"", "\"a1\"<\"a10\"", "\"ä\"<\"b\"", "#N/A=1", "1=#DIV/0!",
        "Data!C4=0", "Data!C4=\"\"", "Data!C4=FALSE", "Data!C4<1", "Data!C8=\"\"", "Data!C8=Data!C4",
        "Data!C4=Data!C4", "Data!C2>5", "Data!C3=TRUE", "Data!C5=5", "Data!C5=\"5\"",
        // 연결
        "1&2", "\"a\"&TRUE", "1.5&\"\"", "1/3&\"\"", "2/3&\"\"", "1E+20&\"\"", "1E-20&\"\"", "123456789012345&\"\"",
        "1234567890123456&\"\"", "12345678901234567890&\"\"", "0.000001&\"\"", "0.0000001&\"\"", "-0.5&\"\"",
        "100000&\"\"", "1E+15&\"\"", "1E+16&\"\"", "123.456789012345678&\"\"", "(0.1+0.2)&\"\"", "1E+308&\"\"",
        "Data!C4&\"x\"", "Data!H2&\"x\"", "\"x\"&Data!J1", "44927.5&\"\"", "-1E-300&\"\"", "2^0.5&\"\"",
        "PI()&\"\"", "1/7&\"\"", "10/3&\"\"", "99999999999999.9&\"\"", "0.1&\"\"", "3E-5&\"\"",
        // 글 → 숫자 변환
        "\"1E3\"*1", "\" 12 \"*1", "\"1,234\"*1", "\"1,23\"*1", "\"$5\"*1", "\"₩5\"*1", "\"5원\"*1",
        "\"(3)\"*1", "\"50%\"*1", "\"-50%\"*1", "\"2024-01-15\"*1", "\"2024/1/15\"*1", "\"2024.1.15\"*1",
        "\"10:30\"*1", "\"10:30:15\"*1", "\"10:30 PM\"*1", "\"1/2\"*1", "\"1 1/2\"*1", "\"3/4\"*1",
        "\"Jan 5, 2024\"*1", "\"5-Jan-2024\"*1", "\"2024년 1월 15일\"*1", "\"오후 3:00\"*1", "\"TRUE\"+0",
        "\"\"+0", "\"abc\"+0", "\".5\"*1", "\"5.\"*1", "\"+5\"*1", "\"--5\"*1", "\"1e\"*1", "\"0x10\"*1",
        "\" \"*1", "\"1 000\"*1", "\"12:00:00 AM\"*1", "\"24:00\"*1", "\"25:00\"*1", "\"1:60\"*1",
        "\"2024-02-30\"*1", "\"1900-02-29\"*1", "\"1900-03-01\"*1", "\"99-01-01\"*1", "\"29-01-01\"*1",
        "\"30-01-01\"*1", "\"1/1\"*1", "\"13/1\"*1", "\"2024-01\"*1", "\"Jan-2024\"*1", "\"January 2024\"*1",
        "\"1.5E+3\"*1", "\"1.2.3\"*1", "\"(1,000)\"*1", "\"-$5\"*1", "\"$-5\"*1", "\"5-\"*1",
        "TRUE+TRUE", "TRUE*5", "-TRUE", "Data!C3+1", "Data!C5+1", "Data!C2+1", "Data!C8+1", "Data!C4+1",
        // 참조·이름
        "Data!A3", "Data!$A$3", "'Data'!A3", "SUM(Data!A1:A3)", "SUM(Data!A:A)", "SUM(Data!1:1)",
        "SUM(Data!$A$1:$B$2)", "SUM(S1:S3!A1)", "SUM(S1:S3!A1:A3)", "SUM('S1:S3'!A2)", "SUM(S3:S1!A1)",
        "SUM((Data!A1,Data!A3,Data!A5))", "SUM((Data!A1:A3,Data!B1:B3))", "SUM(Data!A1:B3 Data!B2:C4)",
        "Data!A1:A3 Data!A2:B2", "SUM(Data!A1:A2 Data!C3:C4)", "Data!A1:A10", "Data!A1:A10*2",
        "Data!A1:C1", "Data!A12:E12", "Data!A12:E12*1", "Data!B:B", "Data!5:5", "Nums", "SUM(Nums)",
        "Rate", "Rate*100", "Two", "Greeting", "Arr", "SUM(Arr)", "Local", "DataTop", "SUM(Both)",
        "Both", "NoSuchName", "SUM(NoSuchName)", "Data!A1:A3:Data!B2", "SUM(Data!A1:Data!B2)",
        "SUM(Data!A1:A2:B3)", "ROWS(Data!A1:A3 Data!B1:B3)", "Hid!B2", "Hid!B5",
        // 배열 상수
        "{1,2,3}", "{1,2,3}*2", "SUM({1,2,3}*{4,5,6})", "{1,2;3,4}", "SUM({1,2;3,4})",
        "{\"a\",TRUE,#N/A}", "{-1,-2.5}", "SUM({1,2}*{1;2})", "{1,2}+{10;20}", "INDEX({1,2}+{10;20},2,2)",
        "{1,2,3}+{1,2}", "INDEX({1,2,3}+{1,2},1,3)", "SUM({1,\"2\",TRUE})", "{1;2;3}=2",
        // 오류
        "#N/A", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#NULL!", "#N/A+1", "1+#N/A",
        "#DIV/0!+#N/A", "#N/A+#DIV/0!", "Data!H2+Data!H4", "Data!H4+Data!H2", "SUM(#N/A,1)",
        "\"a\"+#N/A", "#N/A&\"a\"", "-#N/A", "#N/A%", "#N/A=#N/A",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 수학
// ---------------------------------------------------------------------------

fn math() -> Vec<Case> {
    let mut v = Vec::new();
    for f in [
        "ABS", "SIGN", "SQRT", "SQRTPI", "EXP", "LN", "LOG10", "SIN", "COS", "TAN", "ASIN", "ACOS",
        "ATAN", "SINH", "COSH", "TANH", "ASINH", "ACOSH", "ATANH", "_xlfn.COT", "_xlfn.CSC",
        "_xlfn.SEC", "_xlfn.COTH", "_xlfn.CSCH", "_xlfn.SECH", "_xlfn.ACOT", "_xlfn.ACOTH",
        "DEGREES", "RADIANS", "INT", "EVEN", "ODD", "FACT", "FACTDOUBLE",
    ] {
        unary(&mut v, f, &[]);
    }
    for f in [
        "SIN(PI())", "COS(PI()/2)", "TAN(PI()/4)", "TAN(PI()/2)", "ASIN(1)", "ACOS(-1)", "LN(EXP(1))",
        "EXP(709)", "EXP(710)", "SQRT(1E+308)", "FACT(170)", "FACT(171)", "FACT(3.9)", "FACTDOUBLE(-1)",
        "FACTDOUBLE(300)", "EVEN(-0.1)", "ODD(0)", "ODD(-2)", "INT(-0.0001)", "SIN(1E+8)", "SIN(134217727)",
        "SIN(134217728)", "COS(1E+9)", "TAN(1E+9)", "SINH(710)", "COSH(-710)", "ASINH(-1E+200)", "ATANH(0.999999)",
        "_xlfn.COT(0)", "_xlfn.CSC(0)", "_xlfn.SEC(PI()/2)", "_xlfn.COTH(1E-10)", "_xlfn.CSCH(1E+3)",
        "_xlfn.SECH(1E+3)", "_xlfn.ACOT(-1E+10)", "_xlfn.ACOTH(-2)", "PI()", "SQRTPI(PI())",
        "DEGREES(PI())", "RADIANS(180)", "EVEN(1E+15)", "ODD(1.0000001)", "LOG10(1E+300)", "LN(1E-300)",
        "EXP(-745)", "EXP(-746)", "ABS(-1E-320)", "SIGN(-1E-320)", "INT(1E+20+0.5)", "INT(-1E+20)",
        "FACT(0)", "FACT(-0.5)", "FACT(\"3\")", "ACOSH(1)", "ATANH(-0.5)", "TANH(20)",
    ] {
        v.push(c(f));
    }
    calls(&mut v, "LOG", &[
        "100", "8,2", "100,10", "1,1", "0", "-1", "10,0", "10,-2", "2,0.5", "\"8\",\"2\"", "TRUE,2",
        "1000,10", "1E+300,10", "0.001", "10,1", "1", "2,2", "27,3", "125,5", "64,4", "3,Data!C4",
    ]);
    calls(&mut v, "POWER", &[
        "2,3", "2,0.5", "-8,1/3", "-8,0.333333333333333", "0,0", "0,-1", "-2,2", "-2,0.5", "10,308",
        "10,309", "4,-0.5", "\"2\",TRUE", "-1,0.5", "-27,1/3", "-32,0.2", "-32,1/5", "-8,2/3",
        "2,-1074", "2,-1075", "1.0000001,1E+10", "Data!C4,2", "-0.5,3", "-2,1E+10", "-2,1E+10+1",
        "1E-200,2", "9,0.5", "-1,1E+15", "-1,1/3", "-1,2/6",
    ]);
    calls(&mut v, "ATAN2", &[
        "1,1", "-1,1", "1,-1", "0,0", "0,1", "1,0", "-1,0", "0,-1", "-1,-1", "1E-300,1", "\"1\",TRUE",
    ]);
    for name in ["ROUND", "ROUNDUP", "ROUNDDOWN"] {
        calls(&mut v, name, &[
            "2.5,0", "-2.5,0", "2.675,2", "1.005,2", "1234.5678,-2", "1234.5678,1.9", "-1234.5678,-1",
            "0.285,2", "1.45,1", "1E+15+0.5,0", "123456789.123456789,5", "5.55,1", "0.1+0.2,15", "1/3,15",
            "2.5,-1", "999.5,-3", "1.23,400", "\"2.5\",0", "TRUE,0", "1.5,-400", "4.35,1", "4.45,1",
            "0.5,0", "1.5,0", "-0.5,0", "0.15,1", "0.25,1", "0.35,1", "1.255,2", "8.325,2", "2.345,2",
            "-2.345,2", "123.456,-1", "123.456,-5", "0,2", "1E+300,-300", "0.00001234,6", "2.2,0.5",
            "12345.6789,-0.5", "Data!C4,1", "1.1,Data!C4", "3.14159265358979,10", "1E-20,25",
            "0.12345678901234567,16", "9.995,2", "-9.995,2", "1.0049999999999999,2",
        ]);
    }
    calls(&mut v, "TRUNC", &[
        "8.9", "-8.9", "3.14159,3", "1234.5,-2", "0.1+0.2,1", "1E+20,0", "-0.5", "2.999999999999999,0",
        "123.456,-1.5", "\"8.9\"", "TRUE", "1.1,15", "1.1,16",
    ]);
    calls(&mut v, "MROUND", &[
        "10,3", "-10,-3", "1.3,0.2", "5,-2", "0,5", "5,0", "7.5,5", "2.5,1", "-2.5,-1", "6.05,0.1",
        "1.05,0.1", "-10,3", "1E+15,7", "0.3,0.1", "15,10", "25,10", "-25,-10", "\"10\",3",
    ]);
    for name in ["CEILING", "FLOOR"] {
        calls(&mut v, name, &[
            "2.5,1", "-2.5,-2", "-2.5,2", "1.5,0.1", "0.234,0.01", "2.5,0", "0,1", "5,-1", "1.01,0.01",
            "4.42,0.05", "-4.42,-0.05", "10,3", "-10,3", "-10,-3", "1E+15+0.4,1", "0.3,0.1", "0.7,0.1",
            "-0.7,-0.1", "\"2.5\",1", "2.5,\"1\"", "2.5", "3.3,1.1", "6.6,2.2",
        ]);
    }
    for name in ["_xlfn.CEILING.MATH", "_xlfn.FLOOR.MATH"] {
        calls(&mut v, name, &[
            "24.3,5", "6.7", "-8.1,2", "-5.5,2,-1", "-5.5,2,1", "5.5,-2", "0,1", "3,0", "-5.5",
            "-5.5,,1", "5.5,2,1", "-5.5,-2,1", "2.5,0.1", "-2.5,0.1,0", "1E+15+0.5", "0.3,0.1",
        ]);
    }
    for name in ["_xlfn.CEILING.PRECISE", "_xlfn.FLOOR.PRECISE", "_xlfn.ISO.CEILING"] {
        calls(&mut v, name, &[
            "4.3", "-4.3", "4.3,2", "4.3,-2", "-4.3,2", "-4.3,-2", "0,1", "5,0", "2.5,0.1", "0.3,0.1",
        ]);
    }
    calls(&mut v, "MOD", &[
        "3,2", "-3,2", "3,-2", "-3,-2", "5.5,2", "10,0", "0,5", "1E+15,7", "2^53,3", "12.34,0.1",
        "-1,1E-10", "1E+20,3", "7,2.5", "-7,2.5", "0.3,0.1", "1,0.2", "\"7\",TRUE", "6,3", "-6,3",
        "4.7,1", "-4.7,1", "123456789,1000", "1E+10,0.3", "2,1E-300", "5,Data!C4",
    ]);
    calls(&mut v, "QUOTIENT", &["5,2", "-10,3", "4.5,3.1", "1,0", "-5,-2", "1E+20,3", "\"7\",2"]);
    calls(&mut v, "COMBIN", &[
        "8,2", "5,5", "5,0", "5,6", "-1,0", "10.9,2.9", "1000,500", "1030,2", "1030,515", "0,0",
        "49,6", "100,50", "\"5\",2",
    ]);
    calls(&mut v, "_xlfn.COMBINA", &["4,3", "10,3", "0,0", "0,1", "1,0", "3.9,2.1", "-1,2", "100,10"]);
    calls(&mut v, "GCD", &[
        "5,2", "24,36", "7,1", "5,0", "0,0", "-1,2", "2.5,5", "Data!A1:A10", "24,36,48", "1E+15,5",
        "9.007199254740992E+15,3", "\"12\",18", "Data!C1:C10", "12",
    ]);
    calls(&mut v, "LCM", &[
        "5,2", "24,36", "7,1", "5,0", "0,0", "-1,2", "2.5,5", "Data!A1:A10", "24,36,48", "3,4,5,6,7,8,9",
        "\"12\",18", "1E+10,1E+10+1",
    ]);
    calls(&mut v, "MULTINOMIAL", &["2,3,4", "1", "0", "Data!A1:A3", "-1", "2.9,1.1", "100,100"]);
    for name in ["SUM", "PRODUCT", "SUMSQ"] {
        agg(&mut v, name);
    }
    calls(&mut v, "SUM", &["", "1", "Data!A1:A10,100", "Data!F:F", "Hid!A1:A10", "Hid!A:C"]);
    calls(&mut v, "SUMIF", &[
        "Data!A1:A10,\">3\"", "Data!A1:A10,3", "Data!A1:A10,\"3\"", "Data!D1:D10,\"apple\",Data!A1:A10",
        "Data!D1:D10,\"a*\",Data!A1:A10", "Data!D1:D10,\"?????\",Data!B1:B10",
        "Data!D1:D10,\"<>apple\",Data!A1:A10", "Data!C1:C10,\"\"", "Data!C1:C10,\"<>\"", "Data!C1:C10,\"=\"",
        "Data!A1:A10,\">=\"&Data!A3", "Data!E1:E10,\">2023-06-01\"", "Data!J1:J10,TRUE,Data!A1:A10",
        "Data!H1:H10,\"#N/A\"", "Data!A1:A10,\"<5\",Data!B1", "Data!A1:A10,\"<5\",Data!B1:B3",
        "Data!D1:D10,\"*e*\",Data!A1:A10", "Data!D1:D10,\"APPLE\",Data!A1:A10", "Data!C1:C10,5",
        "Data!C1:C10,\"5\"", "Data!C1:C10,TRUE", "Data!C1:C10,\"TRUE\"", "Data!C1:C10,\">0\"",
        "Data!C1:C10,\"<>abc\"", "Data!A1:A10,Data!A1", "Data!A1:A10,\"<>1\"", "Data!A1:A10,\">=3.5\"",
        "Data!B1:B10,\"<0\"", "Data!B1:B10,\">=0\"", "Data!D1:D10,\">c\",Data!A1:A10",
        "Data!D1:D10,\"~*\",Data!A1:A10", "Data!H1:H10,\">2\"", "Data!H1:H10,#N/A,Data!A1:A10",
        "Data!A1:A10,\">3\",Data!H1:H10", "Data!A:A,\">5\"", "Data!A1:A10,\"=3\"", "Data!A1:A10,\"= 3\"",
        "Data!A1:A10,\" 3\"", "Data!A1:A10,\"3 \"", "Data!A1:A10,\"<>\"", "Data!A1:A10,\"\"",
        "Data!A1:A10,\"\",Data!B1:B10", "Data!C1:C10,\"<>\",Data!A1:A10", "Data!A1:A10,{3,5}",
        "Data!A1:A10,\">\"&\"3\"", "Data!E1:E10,\">=\"&DATE(2024,1,1)", "Data!E1:E10,\">2024/1/1\"",
        "Data!A1:A10,\">1E0\"", "Data!A1:A10,\"<4.5\"", "Data!A1:A10,\">50%\"", "Data!A1:A10,\">$2\"",
    ]);
    calls(&mut v, "SUMIFS", &[
        "Data!A1:A10,Data!D1:D10,\"apple\"", "Data!T2:T11,Data!R2:R11,\"Sales\",Data!S2:S11,\">30\"",
        "Data!A1:A10,Data!B1:B5,\">0\"", "Data!A1:A10,Data!A1:A10,\">2\",Data!A1:A10,\"<6\"",
        "Data!A1:A10,Data!C1:C10,\"\"", "Data!A1:A10,Data!H1:H10,\">2\"", "Data!H1:H10,Data!A1:A10,\">2\"",
        "Data!T2:T11,Data!R2:R11,\"<>IT\"", "Data!T2:T11,Data!Q2:Q11,\"*n*\"",
    ]);
    calls(&mut v, "SUMPRODUCT", &[
        "Data!A1:A10,Data!B1:B10", "Data!A1:A10*Data!B1:B10", "(Data!A1:A10>3)*1", "--(Data!D1:D10=\"apple\")",
        "Data!A1:A3,Data!B1:B4", "Data!C1:C10", "{1,2,3},{4,5,6}", "Data!A1:B3", "Data!A1:A10",
        "Data!H1:H10", "(Data!A1:A10>3)*Data!B1:B10", "Data!C1:C10*1", "Data!C1:C10,Data!A1:A10",
        "Data!A1:A3*{1;2;3}", "Data!A1:A3*{1,2,3}", "Data!A1:A3,{1;2;3}", "1", "\"3\"", "TRUE",
        "Data!A1:A10/Data!B1:B10", "(Data!R2:R11=\"Sales\")*(Data!S2:S11>30),Data!T2:T11",
        "Data!M1:O3,Data!M1:O3", "LEN(Data!D1:D10)", "--Data!J1:J10", "Data!J1:J10*1",
    ]);
    for name in ["SUMX2MY2", "SUMX2PY2", "SUMXMY2"] {
        calls(&mut v, name, &[
            "Data!A1:A10,Data!K1:K10", "{1,2},{3,4}", "{1,2},{3,4,5}", "Data!C1:C10,Data!A1:A10",
            "Data!H1:H10,Data!A1:A10", "Data!D1:D10,Data!A1:A10", "Data!A1:B2,{1,2;3,4}",
        ]);
    }
    calls(&mut v, "SERIESSUM", &[
        "0.5,0,2,{1,-0.5,0.0416666666666667}", "2,1,1,Data!A1:A4", "1,0,1,Data!C1:C3", "2,1,1,{1}",
        "\"2\",1,1,{1,2}", "0,0,1,{1,2,3}", "0,-1,1,{1,2,3}",
    ]);
    for k in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111] {
        v.push(c(format!("SUBTOTAL({k},Hid!A1:A10)")));
        v.push(c(format!("SUBTOTAL({k},Hid!A1:C10)")));
        v.push(c(format!("SUBTOTAL({k},Data!C1:C10)")));
    }
    calls(&mut v, "SUBTOTAL", &[
        "9,Data!A1:A10,Data!B1:B10", "0,Data!A1:A10", "12,Data!A1:A10", "112,Data!A1:A10",
        "9,Data!H1:H10", "9,1,2", "9,Hid!B1:B10", "109,Hid!A1:A10,Hid!C1:C10", "9.5,Hid!A1:A10",
        "\"9\",Hid!A1:A10", "9,S1:S3!A1:A3", "1,Data!D1:D10", "4,Data!D1:D10", "2,Data!C1:C10",
        "3,Data!C1:C10", "9,(Data!A1:A2,Data!A5)",
    ]);
    for k in 1..=19 {
        for o in [0, 1, 2, 3, 4, 5, 6, 7] {
            if k <= 13 {
                v.push(c(format!("_xlfn.AGGREGATE({k},{o},Hid!A1:A10)")));
                v.push(c(format!("_xlfn.AGGREGATE({k},{o},Data!H1:H10)")));
            } else if o % 2 == 0 {
                v.push(c(format!("_xlfn.AGGREGATE({k},{o},Data!H1:H10,2)")));
                v.push(c(format!("_xlfn.AGGREGATE({k},{o},Hid!A1:A10,0.3)")));
            }
        }
    }
    calls(&mut v, "_xlfn.AGGREGATE", &[
        "9,0,Hid!A1:B10", "9,0,Hid!A1:A10,Data!A1:A3", "14,6,Data!H1:H10", "20,0,Data!A1:A10",
        "9,8,Data!A1:A10", "14,6,Data!A1:A10*1,2", "15,6,1/Data!A1:A10,1", "9,6,{1,2,3}",
        "4,0,Data!C1:C10", "14,0,Data!C1:C10,1", "2,0,Data!C1:C10", "3,0,Data!C1:C10",
    ]);
    for f in [
        "MDETERM(Data!M1:O3)", "MDETERM(Data!M5:N6)", "MDETERM(Data!M8:N9)", "MDETERM({1,2;3,4})",
        "MDETERM(Data!M1:N3)", "MDETERM(Data!C1:D2)", "MDETERM({3})", "MDETERM(Data!A1:C3)",
        "MDETERM({1,3,8,5;1,3,6,1;1,1,1,0;7,3,10,2})", "MDETERM({1E+200,0;0,1E+200})",
        "INDEX(MINVERSE(Data!M1:O3),1,1)", "INDEX(MINVERSE(Data!M5:N6),2,1)", "INDEX(MINVERSE(Data!M8:N9),1,1)",
        "INDEX(MMULT(Data!M1:O3,Data!M1:O3),2,3)", "INDEX(MMULT({1,2},{3;4}),1,1)", "MMULT({1,2},{3,4})",
        "INDEX(MMULT(Data!A1:B2,Data!C1:C2),1,1)", "SUM(MUNIT(3))", "INDEX(_xlfn.MUNIT(2),1,2)", "_xlfn.MUNIT(0)",
        "ROMAN(499)", "ROMAN(499,0)", "ROMAN(499,1)", "ROMAN(499,2)", "ROMAN(499,3)", "ROMAN(499,4)",
        "ROMAN(3999)", "ROMAN(4000)", "ROMAN(0)", "ROMAN(-1)", "ROMAN(1.5)", "ROMAN(1999,TRUE)",
        "ROMAN(1999,FALSE)", "ROMAN(1999,5)", "ROMAN(1999,-1)", "ROMAN(\"10\")", "ROMAN(1449,1)",
        "ROMAN(1449,2)", "ROMAN(999,1)", "ROMAN(999,2)", "ROMAN(999,3)", "ROMAN(999,4)", "ROMAN(45,1)",
        "ROMAN(95,1)", "ROMAN(490,1)", "ROMAN(490,2)", "ROMAN(3888)", "ROMAN(1994,1)", "ROMAN(4,4)",
        "_xlfn.ARABIC(\"MCMXCIX\")", "_xlfn.ARABIC(\"mcm\")", "_xlfn.ARABIC(\"\")", "_xlfn.ARABIC(\"-XIV\")",
        "_xlfn.ARABIC(\"IIII\")", "_xlfn.ARABIC(\"ABC\")", "_xlfn.ARABIC(\" XIV \")", "_xlfn.ARABIC(\"LDVLIV\")",
        "_xlfn.ARABIC(\"IM\")", "_xlfn.ARABIC(1)", "_xlfn.ARABIC(\"MMMMMMMMMM\")",
        "_xlfn.BASE(7,2)", "_xlfn.BASE(255,16)", "_xlfn.BASE(255,16,5)", "_xlfn.BASE(0,2)", "_xlfn.BASE(-1,2)",
        "_xlfn.BASE(10,37)", "_xlfn.BASE(10,1)", "_xlfn.BASE(1E+15,36)", "_xlfn.BASE(7.9,2)", "_xlfn.BASE(7,2,256)",
        "_xlfn.BASE(2^53,2)", "_xlfn.BASE(\"10\",\"8\")",
        "_xlfn.DECIMAL(\"FF\",16)", "_xlfn.DECIMAL(\"111\",2)", "_xlfn.DECIMAL(\"zap\",36)", "_xlfn.DECIMAL(\"\",10)",
        "_xlfn.DECIMAL(\"-1\",10)", "_xlfn.DECIMAL(\"12\",2)", "_xlfn.DECIMAL(\"7FFFFFFFFFFFFFFF\",16)",
        "_xlfn.DECIMAL(\" FF \",16)", "_xlfn.DECIMAL(255,10)", "_xlfn.DECIMAL(\"1.5\",10)",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 통계
// ---------------------------------------------------------------------------

fn stat() -> Vec<Case> {
    let mut v = Vec::new();
    for name in [
        "AVERAGE", "AVERAGEA", "COUNT", "COUNTA", "MAX", "MIN", "MAXA", "MINA", "MEDIAN", "MODE",
        "_xlfn.MODE.SNGL", "STDEV", "_xlfn.STDEV.S", "STDEVP", "_xlfn.STDEV.P", "STDEVA", "STDEVPA", "VAR",
        "_xlfn.VAR.S", "VARP", "_xlfn.VAR.P", "VARA", "VARPA", "DEVSQ", "AVEDEV", "GEOMEAN", "HARMEAN",
        "KURT", "SKEW", "_xlfn.SKEW.P",
    ] {
        agg(&mut v, name);
    }
    calls(&mut v, "COUNT", &["", "Data!E1:E10", "#N/A", "\"1\"", "\"a\"", "Data!C4", "Data!A1:Z99"]);
    calls(&mut v, "COUNTA", &["Data!C4", "\"\"", "Data!A1:Z99", "#N/A", "Data!C1:C10,Data!C4"]);
    calls(&mut v, "COUNTBLANK", &[
        "Data!C1:C10", "Data!A1:A10", "Data!C4", "Data!C8", "Data!A11:A20", "Hid!A1:C10",
    ]);
    for name in ["LARGE", "SMALL"] {
        calls(&mut v, name, &[
            "Data!A1:A10,1", "Data!A1:A10,3", "Data!A1:A10,10", "Data!A1:A10,0", "Data!A1:A10,11",
            "Data!A1:A10,2.5", "Data!C1:C10,1", "{3,1,2},2", "Data!H1:H10,1", "Data!A1:A10,\"2\"",
            "Data!D1:D10,1", "5,1", "Data!B1:B10,2", "(Data!A1:A3,Data!B1:B3),2", "Data!A1:A10,{1,2}",
        ]);
    }
    for name in ["PERCENTILE", "_xlfn.PERCENTILE.INC", "_xlfn.PERCENTILE.EXC"] {
        calls(&mut v, name, &[
            "Data!A1:A10,0", "Data!A1:A10,0.1", "Data!A1:A10,0.25", "Data!A1:A10,0.5", "Data!A1:A10,0.9",
            "Data!A1:A10,1", "Data!A1:A10,1.1", "Data!A1:A10,-0.1", "Data!B1:B10,0.3", "Data!C1:C10,0.5",
            "{1,2,3,4},0.75", "Data!H1:H10,0.5", "Data!A1:A10,0.05", "Data!A1:A10,0.95", "5,0.5",
        ]);
    }
    for name in ["QUARTILE", "_xlfn.QUARTILE.INC", "_xlfn.QUARTILE.EXC"] {
        calls(&mut v, name, &[
            "Data!A1:A10,0", "Data!A1:A10,1", "Data!A1:A10,2", "Data!A1:A10,3", "Data!A1:A10,4",
            "Data!A1:A10,5", "Data!A1:A10,1.5", "Data!B1:B10,1", "Data!A1:A10,-1",
        ]);
    }
    for name in ["PERCENTRANK", "_xlfn.PERCENTRANK.INC", "_xlfn.PERCENTRANK.EXC"] {
        calls(&mut v, name, &[
            "Data!A1:A10,4", "Data!A1:A10,3.5", "Data!A1:A10,1", "Data!A1:A10,9", "Data!A1:A10,10",
            "Data!A1:A10,0", "Data!A1:A10,4,2", "Data!A1:A10,4,5", "Data!A1:A10,4,0", "Data!B1:B10,2",
            "Data!A1:A10,2.7,4", "{1,2,3,4,5},3.3",
        ]);
    }
    for name in ["RANK", "_xlfn.RANK.EQ", "_xlfn.RANK.AVG", "RANKEQ", "RANKAVG"] {
        calls(&mut v, name, &[
            "5,Data!A1:A10", "5,Data!A1:A10,1", "1,Data!A1:A10", "7,Data!A1:A10", "3,Data!C1:C10",
            "3,Data!A1:A10,0", "9,Data!A1:A10,1", "\"5\",Data!A1:A10", "5,{1,5,5,9}", "2.5,Data!B1:B10",
            "5,Data!A1:A10,-1", "5,Data!A1:B10",
        ]);
    }
    calls(&mut v, "TRIMMEAN", &[
        "Data!A1:A10,0.2", "Data!A1:A10,0", "Data!A1:A10,1", "Data!A1:A10,0.5", "Data!B1:B10,0.25",
        "Data!A1:A10,-0.1", "Data!C1:C10,0.2", "Data!A1:A10,0.19",
    ]);
    calls(&mut v, "STANDARDIZE", &["42,40,1.5", "1,2,0", "1,2,-1", "\"1\",2,3"]);
    calls(&mut v, "COUNTIF", &[
        "Data!A1:A10,1", "Data!A1:A10,\">3\"", "Data!D1:D10,\"apple\"", "Data!D1:D10,\"a*\"",
        "Data!D1:D10,\"*e*\"", "Data!D1:D10,\"?????\"", "Data!C1:C10,\"\"", "Data!C1:C10,\"<>\"",
        "Data!C1:C10,\"=\"", "Data!C1:C10,TRUE", "Data!C1:C10,\"TRUE\"", "Data!C1:C10,5", "Data!C1:C10,\"5\"",
        "Data!H1:H10,\"#N/A\"", "Data!H1:H10,#N/A", "Data!E1:E10,\">=45000\"", "Data!A1:A10,\"<>1\"",
        "Data!A1:A10,\">=3.5\"", "Data!A1:Z99,\"\"", "Data!A1:Z99,\"<>\"", "Data!D1:D10,\"<c\"",
        "Data!D1:D10,\">=B\"", "Data!J1:J10,FALSE", "Data!C1:C10,\">0\"", "Data!C1:C10,\"<>abc\"",
        "Data!A1:A10,\"3\"", "Data!A1:A10,Data!A1", "Data!A1:A10,Data!C4", "Data!C1:C10,Data!C4",
        "Data!D1:D10,\"~a*\"", "Data!D1:D10,\"*\"", "Data!C1:C10,\"*\"", "Data!C1:C10,\"?*\"",
        "Data!A1:A10,{1,3}", "(Data!A1:A3,Data!A5:A6),3", "Data!H1:H10,\">2\"", "Data!H1:H10,\"<>#N/A\"",
        "Data!B1:B10,\"<>0\"", "Data!A:A,1", "Data!D1:D10,\"APPLE\"", "Data!D1:D10,\"<>a*\"",
    ]);
    calls(&mut v, "COUNTIFS", &[
        "Data!A1:A10,\">2\",Data!B1:B10,\">0\"", "Data!R2:R11,\"Sales\",Data!S2:S11,\">30\"",
        "Data!A1:A10,\">2\"", "Data!A1:A10,\">2\",Data!B1:B5,\">0\"", "Data!C1:C10,\"\",Data!A1:A10,\">2\"",
        "Data!A1:A10,\">2\",Data!A1:A10,\"<6\"",
    ]);
    calls(&mut v, "AVERAGEIF", &[
        "Data!A1:A10,\">3\"", "Data!D1:D10,\"apple\",Data!A1:A10", "Data!A1:A10,\">100\"",
        "Data!C1:C10,\"<>\",Data!A1:A10", "Data!A1:A10,\"<5\",Data!B1", "Data!D1:D10,\"*e\",Data!B1:B10",
        "Data!A1:A10,\">3\",Data!H1:H10", "Data!C1:C10,\">0\"",
    ]);
    calls(&mut v, "AVERAGEIFS", &[
        "Data!T2:T11,Data!R2:R11,\"Sales\"", "Data!T2:T11,Data!R2:R11,\"Sales\",Data!S2:S11,\">30\"",
        "Data!A1:A10,Data!A1:A10,\">100\"", "Data!A1:A10,Data!B1:B5,\">0\"", "Data!C1:C10,Data!A1:A10,\">0\"",
    ]);
    for name in ["_xlfn.MAXIFS", "_xlfn.MINIFS"] {
        calls(&mut v, name, &[
            "Data!T2:T11,Data!R2:R11,\"Sales\"", "Data!T2:T11,Data!R2:R11,\"Sales\",Data!S2:S11,\">30\"",
            "Data!A1:A10,Data!A1:A10,\">100\"", "Data!A1:A10,Data!B1:B5,\">0\"", "Data!C1:C10,Data!A1:A10,\">0\"",
        ]);
    }
    for name in [
        "CORREL", "PEARSON", "RSQ", "COVAR", "_xlfn.COVARIANCE.P", "_xlfn.COVARIANCE.S", "SLOPE",
        "INTERCEPT", "STEYX",
    ] {
        calls(&mut v, name, &[
            "Data!K1:K10,Data!A1:A10", "Data!A1:A10,Data!K1:K10", "Data!K1:K10,Data!B1:B10",
            "Data!C1:C10,Data!A1:A10", "{1,2,3},{1,2}", "{1,2,3},{2,4,6}", "{1,1,1},{1,2,3}",
            "Data!H1:H10,Data!A1:A10", "{1,2},{3,4}", "{5},{6}", "Data!A1:A5,Data!K1:K5",
        ]);
    }
    for name in ["FORECAST", "_xlfn.FORECAST.LINEAR"] {
        calls(&mut v, name, &[
            "11,Data!K1:K10,Data!A1:A10", "0,Data!K1:K10,Data!B1:B10", "5,{1,2,3},{1,1,1}", "\"3\",{2,4},{1,2}",
            "5,{1,2,3},{1,2}",
        ]);
    }
    for f in [
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10),1)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10),2)",
        "INDEX(LINEST(Data!K1:K10),1)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,FALSE),1)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,FALSE),2)", "INDEX(LINEST(Data!K1:K10,Data!A1:B10),1)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:B10),2)", "INDEX(LINEST(Data!K1:K10,Data!A1:B10),3)",
        "INDEX(LOGEST(Data!L1:L10,Data!A1:A10),1)", "INDEX(LOGEST(Data!L1:L10,Data!A1:A10),2)",
        "INDEX(TREND(Data!K1:K10,Data!A1:A10,12),1)", "INDEX(TREND(Data!K1:K10),1)",
        "INDEX(GROWTH(Data!L1:L10,Data!A1:A10,12),1)", "INDEX(GROWTH(Data!L1:L10),2)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),3,1)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),2,1)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),2,2)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),4,1)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),4,2)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),5,1)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),5,2)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE),3,2)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:B10,TRUE,TRUE),2,1)", "INDEX(LINEST(Data!K1:K10,Data!A1:B10,TRUE,TRUE),3,3)",
        "INDEX(LINEST(Data!K1:K10,Data!A1:A10,FALSE,TRUE),2,1)", "INDEX(LINEST(Data!K1:K10,Data!A1:A10,FALSE,TRUE),3,1)",
        "INDEX(LINEST({1,2,3},{1,2,3}),1)", "INDEX(LINEST(Data!K1:K10,Data!C1:C10),1)",
        "INDEX(FREQUENCY(Data!A1:A10,Data!AE1:AE3),1)", "INDEX(FREQUENCY(Data!A1:A10,Data!AE1:AE3),4)",
        "INDEX(FREQUENCY(Data!C1:C10,{0,2}),2)", "SUM(FREQUENCY(Data!A1:A10,5))",
        "PROB(Data!AC1:AC4,Data!AD1:AD4,2)", "PROB(Data!AC1:AC4,Data!AD1:AD4,2,3)", "PROB(Data!AC1:AC4,Data!AD1:AD4,0)",
        "PROB({1,2},{0.5,0.6},1)", "PROB(Data!AC1:AC4,Data!AD1:AD4,3,1)", "PROB({1,2},{0.5,0.5,0},1)",
        "FISHER(0.75)", "FISHER(1)", "FISHER(-0.5)", "FISHERINV(0.97295507452765)", "FISHERINV(-1)",
        "PERMUT(100,3)", "PERMUT(3,2)", "PERMUT(5,0)", "PERMUT(0,0)", "PERMUT(2,3)", "PERMUT(10.5,2.5)",
        "_xlfn.PERMUTATIONA(3,2)", "_xlfn.PERMUTATIONA(2,2)", "_xlfn.PERMUTATIONA(0,0)", "_xlfn.PERMUTATIONA(-1,2)",
        "_xlfn.GAMMA(2.5)", "_xlfn.GAMMA(-3.75)", "_xlfn.GAMMA(0)", "_xlfn.GAMMA(-1)", "_xlfn.GAMMA(171)",
        "_xlfn.GAMMA(172)", "_xlfn.GAMMA(0.5)", "_xlfn.GAMMA(1E-10)", "_xlfn.GAMMA(5)", "_xlfn.GAMMA(30.5)",
        "GAMMALN(4)", "GAMMALN(0.5)", "GAMMALN(0)", "GAMMALN(100)", "GAMMALN(1E-5)", "GAMMALN(1E+300)",
        "_xlfn.GAMMALN.PRECISE(4.5)", "_xlfn.GAUSS(2)", "_xlfn.GAUSS(-1)", "_xlfn.GAUSS(0)", "_xlfn.GAUSS(10)",
        "_xlfn.PHI(0.75)", "_xlfn.PHI(0)", "_xlfn.PHI(-3)", "_xlfn.PHI(40)",
        "MEDIAN(1,2,3,4)", "MODE(1,2,2,3,3)", "MODE(1,2,3)", "MODE(Data!C1:C10)", "MAX()", "MAX(Data!D1:D10)",
        "MIN(Data!C4)", "MAXA(Data!C1:C10)", "MINA(Data!C1:C10)", "AVERAGEA(Data!C1:C10)", "AVERAGE(Data!D1:D10)",
        "GEOMEAN(Data!B1:B10)", "HARMEAN(Data!A1:A10)", "KURT(1,2,3)", "SKEW(1,2)", "STDEV(1)", "VAR(1)",
        "STDEVP(1)", "VARP(5)", "AVEDEV(5)", "DEVSQ()", "STDEV(1E+200,1E+200)", "VAR(1E+15,1E+15+2)",
        "VAR(1E+8+1,1E+8+2,1E+8+3)", "STDEV(0.1,0.2,0.3)", "KURT(Data!B1:B10)", "SKEW(Data!B1:B10)",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 분포
// ---------------------------------------------------------------------------

fn dist() -> Vec<Case> {
    let mut v = Vec::new();
    for name in ["NORMDIST", "_xlfn.NORM.DIST"] {
        calls(&mut v, name, &[
            "42,40,1.5,TRUE", "42,40,1.5,FALSE", "0,0,1,TRUE", "-5,0,1,TRUE", "10,0,1,TRUE", "-40,0,1,TRUE",
            "1,0,0,TRUE", "1,0,-1,TRUE", "8,0,1,TRUE", "-8,0,1,TRUE", "0.5,0,1,TRUE", "-0.5,0,1,FALSE",
            "3,2,0.5,1", "\"1\",0,1,TRUE", "38,40,1.5,TRUE", "-1.96,0,1,TRUE", "1E-10,0,1,TRUE", "-20,0,1,TRUE",
        ]);
    }
    calls(&mut v, "_xlfn.NORM.S.DIST", &[
        "1.333333,TRUE", "1.333333,FALSE", "0,TRUE", "-3,TRUE", "6,TRUE", "-6,TRUE", "-10,TRUE", "-38,TRUE",
        "-39,TRUE", "0.1,TRUE", "0.0001,TRUE", "-0.0001,TRUE", "2,FALSE", "-37.5,TRUE", "5.5,TRUE", "-5.5,TRUE",
    ]);
    calls(&mut v, "NORMSDIST", &["1.333333", "0", "-3", "6", "-6", "-10", "-38", "1E-10", "0.7", "-0.7", "-2.5", "2.5"]);
    for name in ["NORMINV", "_xlfn.NORM.INV"] {
        calls(&mut v, name, &[
            "0.908789,40,1.5", "0.5,0,1", "0.001,0,1", "0.999,0,1", "0,0,1", "1,0,1", "0.3,1,0", "1E-10,0,1",
            "0.975,0,1", "0.025,0,1", "0.9999999,0,1", "1E-300,0,1", "0.2,5,2",
        ]);
    }
    for name in ["NORMSINV", "_xlfn.NORM.S.INV"] {
        calls(&mut v, name, &[
            "0.908789", "0.5", "0.001", "0.999", "0", "1", "1E-10", "0.975", "0.025", "0.9999999", "1E-300",
            "0.3", "0.1", "0.425", "0.075", "0.92", "0.08", "1E-20",
        ]);
    }
    calls(&mut v, "_xlfn.LOGNORM.DIST", &["4,3.5,1.2,TRUE", "4,3.5,1.2,FALSE", "0,1,1,TRUE", "1,0,1,TRUE", "2,1,0,TRUE"]);
    calls(&mut v, "LOGNORMDIST", &["4,3.5,1.2", "0,1,1", "1,0,1", "10,2,0.5"]);
    for name in ["LOGINV", "_xlfn.LOGNORM.INV"] {
        calls(&mut v, name, &["0.039084,3.5,1.2", "0.5,0,1", "0,0,1", "1,0,1", "0.9,1,0"]);
    }
    for name in ["CONFIDENCE", "_xlfn.CONFIDENCE.NORM"] {
        calls(&mut v, name, &["0.05,2.5,50", "0.1,1,1", "0,1,10", "1,1,10", "0.05,0,10", "0.05,1,0.5", "0.05,1,1.9"]);
    }
    calls(&mut v, "_xlfn.CONFIDENCE.T", &["0.05,1,50", "0.05,1,1", "0.1,2,2", "0.5,1,10", "0.05,1,2.5"]);
    for name in ["ZTEST", "_xlfn.Z.TEST"] {
        calls(&mut v, name, &[
            "Data!A1:A10,4", "Data!A1:A10,6,1", "Data!A1:A10,4,0", "{3,6,7,8,6,5,4,2,1,9},4", "Data!A1:A10,10",
            "{1},1", "Data!C1:C10,1",
        ]);
    }
    for name in ["EXPONDIST", "_xlfn.EXPON.DIST"] {
        calls(&mut v, name, &["0.2,10,TRUE", "0.2,10,FALSE", "-1,1,TRUE", "1,0,TRUE", "0,1,FALSE", "50,1,TRUE"]);
    }
    for name in ["WEIBULL", "_xlfn.WEIBULL.DIST"] {
        calls(&mut v, name, &["105,20,100,TRUE", "105,20,100,FALSE", "0,1,1,TRUE", "0,1,1,FALSE", "0,0.5,1,FALSE", "-1,1,1,TRUE", "1,0,1,TRUE"]);
    }
    for name in ["GAMMADIST", "_xlfn.GAMMA.DIST"] {
        calls(&mut v, name, &[
            "10.00001131,9,2,FALSE", "10.00001131,9,2,TRUE", "0,1,1,TRUE", "0,1,1,FALSE", "0,0.5,1,FALSE",
            "0,2,1,FALSE", "5,0,1,TRUE", "-1,1,1,TRUE", "100,3,2,TRUE", "0.001,0.1,10,TRUE", "50,50,1,TRUE",
            "1E-5,1E-3,1,TRUE", "200,100,1,TRUE",
        ]);
    }
    for name in ["GAMMAINV", "_xlfn.GAMMA.INV"] {
        // GAMMAINV(1,…) 은 한셀을 죽인다 (관측) — 넣지 않는다
        calls(&mut v, name, &["0.068094,9,2", "0.5,1,1", "0,1,1", "0.99,0.5,3", "0.01,100,1", "0.5,0.01,1", "0.999999,1,1"]);
    }
    calls(&mut v, "_xlfn.BETA.DIST", &[
        "2,8,10,TRUE,1,3", "2,8,10,FALSE,1,3", "0.5,1,1,TRUE", "0.5,2,3,FALSE", "0,2,3,TRUE", "1,2,3,TRUE",
        "0.4,0.5,0.5,TRUE", "0,0.5,2,FALSE", "1.5,2,2,TRUE,1,2", "0.5,0,1,TRUE", "0.3,2,3,TRUE,0.5,1",
    ]);
    calls(&mut v, "BETADIST", &["2,8,10,1,3", "0.5,1,1", "0.2,2,3", "0,2,3", "1,2,3", "0.5,2,3,0.5,0.5"]);
    for name in ["BETAINV", "_xlfn.BETA.INV"] {
        calls(&mut v, name, &["0.685470581,8,10,1,3", "0.5,2,3", "0,2,3", "1,2,3", "0.1,0.5,0.5", "0.99,10,2", "0.5,1,1,2,4"]);
    }
    for name in ["BINOMDIST", "_xlfn.BINOM.DIST"] {
        calls(&mut v, name, &[
            "6,10,0.5,FALSE", "6,10,0.5,TRUE", "0,10,0.5,TRUE", "10,10,0.5,TRUE", "11,10,0.5,TRUE", "-1,10,0.5,TRUE",
            "5,10,0,FALSE", "5,10,1,FALSE", "6.9,10,0.5,FALSE", "50,100,0.3,TRUE", "500,1000,0.5,FALSE",
            "1500,2000,0.75,TRUE", "6,10,1.5,TRUE", "0,0,0.5,FALSE",
        ]);
    }
    for name in ["CRITBINOM", "_xlfn.BINOM.INV"] {
        calls(&mut v, name, &["6,0.5,0.75", "10,0.5,0", "10,0.5,1", "100,0.1,0.5", "10,0,0.5", "10,1,0.5", "10,0.5,0.0009765625"]);
    }
    calls(&mut v, "_xlfn.BINOM.DIST.RANGE", &["60,0.75,48", "60,0.75,45,50", "10,0.5,0,10", "10,0.5,5,4", "10,0.5,11"]);
    calls(&mut v, "_xlfn.NEGBINOM.DIST", &["10,5,0.25,FALSE", "10,5,0.25,TRUE", "0,1,0.5,TRUE", "10,0.5,0.25,TRUE", "10,5,1,TRUE"]);
    calls(&mut v, "NEGBINOMDIST", &["10,5,0.25", "0,1,0.5", "3.7,2.2,0.5"]);
    calls(&mut v, "_xlfn.HYPGEOM.DIST", &["1,4,8,20,TRUE", "1,4,8,20,FALSE", "0,4,8,20,TRUE", "4,4,8,20,TRUE", "5,4,8,20,TRUE"]);
    calls(&mut v, "HYPGEOMDIST", &["1,4,8,20", "0,4,8,20", "4,4,8,20", "2,5,3,6", "3,5,3,6"]);
    for name in ["POISSON", "_xlfn.POISSON.DIST"] {
        calls(&mut v, name, &["2,5,FALSE", "2,5,TRUE", "0,0,TRUE", "0,0,FALSE", "3,0,FALSE", "-1,1,TRUE", "100,100,TRUE", "2.9,5,TRUE", "200,10,FALSE"]);
    }
    calls(&mut v, "_xlfn.CHISQ.DIST", &["0.5,1,TRUE", "2,3,FALSE", "0,1,FALSE", "0,2,FALSE", "0,3,FALSE", "10,4,TRUE", "-1,1,TRUE", "1,0,TRUE", "100,50,TRUE"]);
    for name in ["CHIDIST", "_xlfn.CHISQ.DIST.RT"] {
        calls(&mut v, name, &["18.307,10", "0,1", "1,1", "100,3", "1000,10", "0.5,1.5", "-1,1", "2,1E+10"]);
    }
    calls(&mut v, "_xlfn.CHISQ.INV", &["0.93,1", "0.6,2", "0,1", "1,1", "0.5,100", "0.001,3"]);
    for name in ["CHIINV", "_xlfn.CHISQ.INV.RT"] {
        calls(&mut v, name, &["0.050001,10", "0.5,1", "1,1", "0,1", "0.001,3", "0.999,30"]);
    }
    for name in ["CHITEST", "_xlfn.CHISQ.TEST"] {
        calls(&mut v, name, &[
            "Data!A1:A10,Data!K1:K10", "Data!M1:O3,Data!M1:O3", "{58,35;11,25;10,23},{45.35,47.65;17.56,18.44;16.09,16.91}",
            "{1,2},{1,2}", "{1;2},{1,2}", "Data!M1:N3,Data!M1:N3+1",
        ]);
    }
    calls(&mut v, "_xlfn.F.DIST", &["15.2068649,6,4,TRUE", "15.2068649,6,4,FALSE", "0,1,1,FALSE", "0,2,3,FALSE", "1,0,1,TRUE", "2,1E+10,1E+10,TRUE"]);
    for name in ["FDIST", "_xlfn.F.DIST.RT"] {
        calls(&mut v, name, &["15.2068649,6,4", "0,1,1", "1,1,1", "100,5,5", "-1,1,1", "3.5,2.5,4.9"]);
    }
    calls(&mut v, "_xlfn.F.INV", &["0.01,6,4", "0.5,1,1", "0,1,1", "1,1,1", "0.99,10,10"]);
    for name in ["FINV", "_xlfn.F.INV.RT"] {
        calls(&mut v, name, &["0.01,6,4", "0.5,1,1", "1,1,1", "0,1,1", "0.99,10,10"]);
    }
    for name in ["FTEST", "_xlfn.F.TEST"] {
        calls(&mut v, name, &["Data!A1:A10,Data!K1:K10", "Data!A1:A10,Data!B1:B10", "{1,2,3},{1,1,1}", "{1},{1,2}", "Data!C1:C10,Data!A1:A10"]);
    }
    calls(&mut v, "_xlfn.T.DIST", &["60,1,TRUE", "8,3,FALSE", "0,1,TRUE", "-1,2,TRUE", "1,0,TRUE", "1,1E+10,TRUE", "0.5,2.5,TRUE"]);
    calls(&mut v, "_xlfn.T.DIST.2T", &["1.959999998,60", "0,1", "-1,1", "1,0", "100,5"]);
    calls(&mut v, "_xlfn.T.DIST.RT", &["1.959999998,60", "0,1", "-1,1", "1,0", "100,5"]);
    calls(&mut v, "TDIST", &["1.959999998,60,2", "1.959999998,60,1", "0,1,1", "-1,1,1", "1,1,3", "1,0.5,1", "10,2.9,2"]);
    calls(&mut v, "_xlfn.T.INV", &["0.75,2", "0.5,1", "0.0001,3", "0,1", "1,1", "0.99,1E+6"]);
    for name in ["TINV", "_xlfn.T.INV.2T"] {
        calls(&mut v, name, &["0.546449,60", "1,1", "0,1", "0.05,1", "0.05,30", "1.5,1", "0.0001,2"]);
    }
    for name in ["TTEST", "_xlfn.T.TEST"] {
        calls(&mut v, name, &[
            "Data!A1:A10,Data!K1:K10,2,1", "Data!A1:A10,Data!K1:K10,1,1", "Data!A1:A10,Data!K1:K10,2,2",
            "Data!A1:A10,Data!K1:K10,2,3", "Data!A1:A10,Data!B1:B10,1,3", "Data!A1:A5,Data!K1:K10,2,2",
            "Data!A1:A5,Data!K1:K10,2,1", "Data!A1:A10,Data!K1:K10,3,1", "Data!A1:A10,Data!K1:K10,2,4",
            // 짝지은 표본의 차이가 모두 같으면(분산 0) 한셀이 죽는다 (관측) — {1,2,3},{1,2,3} 은 넣지 않는다
            "{1,2,3},{2,3,5},2,1", "Data!C1:C10,Data!A1:A10,2,1",
        ]);
    }
    v
}

// ---------------------------------------------------------------------------
// 글
// ---------------------------------------------------------------------------

const TXT1: &[&str] = &[
    "\"Hello World\"", "\"  a  b  \"", "\"\"", "123", "1.5", "TRUE", "Data!$C$4", "Data!$D$2", "#N/A",
    "\"한글 테스트\"", "\"ÀÉÎ ñ ß\"", "\"ＡＢＣ　１２３\"", "\"abc\"&CHAR(9)&\"def\"", "1/3", "Data!$E$1",
    "\"mIxEd CaSe-word's\"", "\"o'neil mcDONALD 2nd\"",
];

fn text() -> Vec<Case> {
    let mut v = Vec::new();
    for name in [
        "LEN", "LENB", "LOWER", "UPPER", "PROPER", "TRIM", "CLEAN", "T", "CODE", "_xlfn.UNICODE", "ASC",
        "DBCS", "JUNJA", "VALUE",
    ] {
        calls(&mut v, name, TXT1);
    }
    for name in ["LEFT", "RIGHT", "LEFTB", "RIGHTB"] {
        calls(&mut v, name, &[
            "\"Hello\"", "\"Hello\",2", "\"Hello\",0", "\"Hello\",10", "\"Hello\",-1", "\"한글나라\",3",
            "\"한a글b\",3", "123.45,3", "TRUE,2", "Data!C4,1", "\"Hello\",2.9", "\"Hello\",\"2\"", "#N/A,1",
            "\"Hello\",TRUE", "\"한글\",1", "\"a한\",2",
        ]);
    }
    for name in ["MID", "MIDB"] {
        calls(&mut v, name, &[
            "\"Hello World\",7,5", "\"Hello\",1,0", "\"Hello\",0,1", "\"Hello\",6,1", "\"Hello\",5,10",
            "\"Hello\",2,-1", "\"한글나라만세\",2,3", "\"한a글b나c\",2,3", "\"한a글b나c\",3,3", "12345,2,3",
            "\"Hello\",1.9,2.9", "\"Hello\",100,1",
        ]);
    }
    calls(&mut v, "EXACT", &["\"abc\",\"abc\"", "\"abc\",\"ABC\"", "1,\"1\"", "TRUE,\"TRUE\"", "\"\",Data!C4", "\"a \",\"a\""]);
    calls(&mut v, "REPT", &["\"ab\",3", "\"ab\",0", "\"ab\",-1", "\"\",5", "\"ab\",2.9", "\"x\",32767", "\"x\",32768", "1,3", "\"한\",2"]);
    calls(&mut v, "CONCATENATE", &[
        "\"a\",\"b\",\"c\"", "1,2", "TRUE,FALSE", "Data!C4,\"x\"", "Data!A1:A3", "\"a\",#N/A", "1/3,\"\"", "\"\"",
    ]);
    calls(&mut v, "_xlfn.CONCAT", &[
        "\"a\",\"b\"", "Data!A1:A3", "Data!C1:C10", "Data!D1:D3,\"-\",Data!A1:A2", "Data!H1:H3", "{1,2;3,4}", "1",
    ]);
    calls(&mut v, "_xlfn.TEXTJOIN", &[
        "\",\",TRUE,Data!C1:C10", "\",\",FALSE,Data!C1:C10", "\"-\",TRUE,\"a\",\"\",\"b\"", "\"\",TRUE,Data!A1:A5",
        "\", \",TRUE,Data!D1:D3,Data!A1:A2", "\",\",TRUE,Data!H1:H3", "Data!I1:I2,TRUE,Data!A1:A5", "\",\",\"x\",1,2",
        "\",\",1,1,2",
    ]);
    calls(&mut v, "SUBSTITUTE", &[
        "\"aaa\",\"a\",\"b\"", "\"aaa\",\"a\",\"b\",2", "\"aaa\",\"a\",\"b\",4", "\"aaa\",\"a\",\"b\",0", "\"aaa\",\"\",\"b\"",
        "\"abcabc\",\"bc\",\"X\"", "\"aaaa\",\"aa\",\"b\"", "\"aaaa\",\"aa\",\"b\",2", "\"Hello\",\"L\",\"x\"", "123123,2,9",
        "\"한글한글\",\"글\",\"국\",2", "\"a\",\"a\",\"\"", "\"aaa\",\"a\",\"b\",1.5",
    ]);
    for name in ["REPLACE", "REPLACEB"] {
        calls(&mut v, name, &[
            "\"abcdef\",2,3,\"X\"", "\"abcdef\",1,0,\"X\"", "\"abcdef\",7,2,\"X\"", "\"abcdef\",10,2,\"X\"",
            "\"abcdef\",0,2,\"X\"", "\"abcdef\",2,-1,\"X\"", "\"한글나라\",2,2,\"X\"", "\"한a글b\",2,2,\"XY\"",
            "12345,2,2,\"-\"", "\"abc\",2,100,\"\"",
        ]);
    }
    for name in ["FIND", "FINDB", "SEARCH", "SEARCHB"] {
        calls(&mut v, name, &[
            "\"b\",\"abc\"", "\"B\",\"abc\"", "\"\",\"abc\"", "\"\",\"abc\",3", "\"\",\"abc\",4", "\"\",\"abc\",5",
            "\"c\",\"abcabc\",4", "\"c\",\"abcabc\",0", "\"z\",\"abc\"", "\"글\",\"한글나라\"", "\"나\",\"한a글b나c\"",
            "\"b?\",\"abc\"", "\"*c\",\"abc\"", "\"~*\",\"a*b\"", "\"a*c\",\"xxabbcxx\"", "\"?\",\"abc\",2",
            "1,123", "\"a\",\"ABA\",2", "\"b\",\"abc\",1.9", "\"\",\"\"", "\"x\",\"\"", "\"c\",\"abc\",3",
            "\"c\",\"abc\",4", "\"*\",\"abc\"", "\"a?c\",\"ABC\"",
        ]);
    }
    calls(&mut v, "_xlfn.NUMBERVALUE", &[
        "\"2.500,27\",\",\",\".\"", "\"3.5%\"", "\"1,234.5\"", "\"\"", "\" 1 2 3 \"", "\"1.2.3\"", "\"12%%\"",
        "\"1,5\",\",\"", "\"1,5\",\",\",\",\"", "\"(5)\"", "\"-5\"", "\"1e3\"", "\"$5\"", "5", "TRUE",
        "\"1.234,5\",\",\"", "\"1,234\",\".\",\" \"",
    ]);
    calls(&mut v, "TEXT", &[
        "1234.567,\"0.00\"", "1234.567,\"#,##0\"", "0.25,\"0%\"", "45000,\"yyyy-mm-dd\"", "45000.75,\"h:mm AM/PM\"",
        "-5,\"0;(0)\"", "0,\"0;-0;zero\"", "1234.567,\"$#,##0.00\"", "\"abc\",\"@@\"", "\"12\",\"0.0\"", "TRUE,\"0\"",
        "1234.567,\"\"", "1234.567,\"General\"", "0.000123,\"0.00E+00\"", "1.5,\"# ?/?\"", "45000,\"dddd\"",
        "45000,\"mmm d, yyyy\"", "45000,\"yyyy년 m월 d일\"", "45000,\"aaaa\"", "1234,\"[<1000]0;#,##0\"",
        "-1234.5,\"#,##0.0\"", "1E+20,\"0\"", "0.5,\"h:mm:ss\"", "1.5,\"[h]:mm\"", "Data!C4,\"0.0\"",
        "#N/A,\"0\"", "1234.5,\"0.0,\"", "12345678,\"#,##0,,\\M\"", "45000,\"ggge\"", "3.14159,\"?.??\"",
        "1234.567,\"#,##0.00_);(#,##0.00)\"", "45000.5,\"yyyy-mm-dd hh:mm:ss\"", "0.123,\"0.0%\"",
        "100,\"[Red]0\"", "-0.001,\"0.00\"", "2.5,\"0\"", "3.5,\"0\"", "-2.5,\"0\"",
    ]);
    calls(&mut v, "FIXED", &[
        "1234.567", "1234.567,1", "1234.567,-1", "1234.567,1,TRUE", "-1234.567,0", "0.5,0", "1.5,0", "2.5,0",
        "1E+20,2", "1234.5,\"1\"", "1234.5,2,1", "-0.001,2", "123456789.123,-3", "1.005,2", "0,2", "1234.567,128",
        "1234.567,127",
    ]);
    for name in ["DOLLAR", "USDOLLAR", "WON"] {
        calls(&mut v, name, &["1234.567", "1234.567,1", "-1234.567,2", "0.5", "1234.567,-2", "-0.5,0", "1E+10,0", "\"12\""]);
    }
    calls(&mut v, "CHAR", &["65", "97", "1", "255", "0", "256", "65.9", "\"66\"", "128", "160", "13"]);
    calls(&mut v, "_xlfn.UNICHAR", &["65", "44032", "128512", "0", "1114112", "55296", "1"]);
    calls(&mut v, "PHONETIC", &["Data!D1", "Data!D1:D3", "\"abc\"", "Data!A1"]);
    calls(&mut v, "NUMBERSTRING", &[
        "1234,1", "1234,2", "1234,3", "0,1", "10000,1", "100010001,1", "1234.5,1", "-5,1", "123,4", "123,0",
        "20000000,2", "11,3", "1E+15,1",
    ]);
    calls(&mut v, "DATESTRING", &["45000", "1", "0", "60", "2958465", "-1", "\"2024-01-15\"", "45000.9"]);
    calls(&mut v, "BAHTTEXT", &["1234", "0", "21.25", "-100", "1000000", "0.5", "11", "101"]);
    calls(&mut v, "VALUE", &[
        "\"$1,000\"", "\"16:48:00\"-\"12:00:00\"", "\"1E3\"", "\"12%\"", "\"(5)\"", "\"2024-01-15\"", "\"  7  \"",
        "\"1 1/4\"", "\"abc\"", "\"\"", "\"TRUE\"",
    ]);
    v
}

// ---------------------------------------------------------------------------
// 논리
// ---------------------------------------------------------------------------

fn logical() -> Vec<Case> {
    let mut v = Vec::new();
    calls(&mut v, "IF", &[
        "TRUE,1,2", "FALSE,1,2", "1,\"y\",\"n\"", "0,\"y\",\"n\"", "\"TRUE\",1,2", "\"abc\",1,2", "Data!C4,1,2",
        "Data!C2,1,2", "#N/A,1,2", "TRUE,1", "FALSE,1", "FALSE,1,", "TRUE,,2", "TRUE,Data!C4", "TRUE,Data!C8",
        "Data!A1>2,Data!A1,Data!B1", "2.5,1,2", "-1,1,2", "\"\",1,2", "\"FALSE\",1,2", "TRUE,Data!A1:A3",
        "TRUE,SUM(Data!A1:A3),1/0", "FALSE,1/0,3",
    ]);
    calls(&mut v, "IFERROR", &[
        "1/0,\"e\"", "1,\"e\"", "#N/A,0", "Data!H2,Data!H4", "Data!C4,1", "\"a\"+1,\"x\"", "1/0,", "Data!C8,1",
    ]);
    calls(&mut v, "_xlfn.IFNA", &["#N/A,0", "1/0,0", "1,0", "Data!H2,\"na\"", "VLOOKUP(99,Data!A1:B10,2,FALSE),\"없음\""]);
    calls(&mut v, "_xlfn.IFS", &[
        "FALSE,1,TRUE,2", "FALSE,1,FALSE,2", "1,\"a\"", "\"x\",1", "#N/A,1,TRUE,2", "FALSE,1,1/0,2", "TRUE,Data!C4",
        "Data!A1>5,\"big\",Data!A1>2,\"mid\",TRUE,\"small\"", "FALSE,1,TRUE",
    ]);
    for name in ["AND", "OR", "_xlfn.XOR"] {
        calls(&mut v, name, &[
            "TRUE,TRUE", "TRUE,FALSE", "FALSE,FALSE", "1,0", "Data!J1:J10", "Data!C1:C10", "Data!D1:D10",
            "\"TRUE\"", "\"abc\"", "Data!C4", "Data!H1:H3", "TRUE,#N/A", "Data!A1:A10", "{TRUE,FALSE}", "2,3",
            "Data!C2", "\"TRUE\",FALSE", "Data!C3,Data!C7", "TRUE,TRUE,TRUE", "Data!C4:C4,TRUE", "Data!D1,TRUE",
            "1=1,2=2", "(Data!A1:A3,Data!J1:J3)",
        ]);
    }
    calls(&mut v, "NOT", &["TRUE", "FALSE", "0", "1", "2", "\"TRUE\"", "\"abc\"", "Data!C4", "#N/A", "\"\"", "Data!C3"]);
    calls(&mut v, "_xlfn.SWITCH", &[
        "2,1,\"a\",2,\"b\"", "3,1,\"a\",2,\"b\"", "3,1,\"a\",2,\"b\",\"z\"", "\"b\",\"a\",1,\"B\",2", "TRUE,1,\"n\",TRUE,\"t\"",
        "1,TRUE,\"t\",1,\"one\"", "#N/A,1,2", "1,#N/A,2,1,3", "Data!C4,0,\"zero\",\"\",\"empty\"", "\"1\",1,\"n\",\"1\",\"s\"",
    ]);
    for f in ["TRUE()", "FALSE()", "TRUE", "FALSE", "IF(TRUE,TRUE())"] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 찾기·참조
// ---------------------------------------------------------------------------

fn lookup() -> Vec<Case> {
    let mut v = Vec::new();
    calls(&mut v, "VLOOKUP", &[
        "30,Data!F1:G10,2,FALSE", "35,Data!F1:G10,2,TRUE", "35,Data!F1:G10,2", "5,Data!F1:G10,2,TRUE",
        "100,Data!F1:G10,2", "1000,Data!F1:G10,2", "\"apple\",Data!D1:E10,2,FALSE", "\"APPLE\",Data!D1:E10,2,0",
        "\"a*\",Data!D1:E10,2,FALSE", "\"?ig\",Data!D1:E10,2,FALSE", "\"c\",Data!I1:J10,2,TRUE", "\"cz\",Data!I1:J10,2",
        "30,Data!F1:G10,3,FALSE", "30,Data!F1:G10,0,FALSE", "30,Data!F1:G10,1,FALSE", "\"30\",Data!F1:G10,2,FALSE",
        "30,Data!F1:G10,2.9,FALSE", "Data!C4,Data!C1:D10,2,FALSE", "TRUE,Data!C1:D10,2,FALSE", "5,Data!C1:D10,2,FALSE",
        "\"5\",Data!C1:D10,2,FALSE", "#N/A,Data!F1:G10,2", "30,Data!F1:G10,2,\"FALSE\"", "3,Data!A1:B10,2,TRUE",
        "3,Data!A1:B10,2,FALSE", "4,{1,\"a\";3,\"b\";5,\"c\"},2", "\"Kim\",Data!Q1:T11,4,FALSE", "\"k*\",Data!Q1:T11,3,0",
        "30,Data!F1:G10,2,2", "\"\",Data!C1:D10,2,FALSE", "\"abc\",Data!C1:D10,2,TRUE",
    ]);
    calls(&mut v, "HLOOKUP", &[
        "30,Data!A12:E13,2,FALSE", "35,Data!A12:E13,2", "5,Data!A12:E13,2", "30,Data!A12:E13,3,FALSE",
        "\"x3\",Data!A13:E13,1,FALSE", "60,Data!A12:E13,2,TRUE", "30,{10,20,30;\"a\",\"b\",\"c\"},2",
    ]);
    calls(&mut v, "LOOKUP", &[
        "35,Data!F1:F10,Data!G1:G10", "5,Data!F1:F10,Data!G1:G10", "1000,Data!F1:F10,Data!G1:G10",
        "35,Data!F1:G10", "\"c\",Data!I1:I10,Data!G1:G10", "35,Data!A12:E12,Data!A13:E13", "35,Data!A12:E13",
        "4,{1,2,3,4,5}", "4,{1,3,5},{\"a\",\"b\",\"c\"}", "35,Data!F1:F10,Data!G1:G5", "\"zz\",Data!D1:D10",
        "2,Data!C1:C10", "TRUE,Data!J1:J10", "35,Data!F1:F10,Data!A12:E12",
    ]);
    calls(&mut v, "MATCH", &[
        "40,Data!F1:F10,0", "35,Data!F1:F10", "35,Data!F1:F10,1", "5,Data!F1:F10,1", "35,Data!F1:F10,-1",
        "\"apple\",Data!D1:D10,0", "\"APPLE\",Data!D1:D10,0", "\"b*\",Data!D1:D10,0", "\"?i?\",Data!D1:D10,0",
        "\"c\",Data!I1:I10", "\"cz\",Data!I1:I10", "TRUE,Data!C1:C10,0", "5,Data!C1:C10,0", "\"5\",Data!C1:C10,0",
        "\"\",Data!C1:C10,0", "Data!C4,Data!C1:C10,0", "0,Data!C1:C10,0", "40,Data!A12:E12,0", "40,Data!A1:B10,0",
        "40,{10,20,40},0", "3,{5,4,3,2,1},-1", "6,{5,4,3,2,1},-1", "0,{5,4,3,2,1},-1", "30,Data!F1:F10,2",
        "30,Data!F1:F10,0.5", "#N/A,Data!H1:H10,0", "1,Data!H1:H10,0", "9,Data!H1:H10,1", "\"~*\",{\"a\",\"*\"},0",
        "3,Data!A1:A10,1", "5,Data!A1:A10,0",
    ]);
    calls(&mut v, "INDEX", &[
        "Data!A1:A10,3", "Data!A1:B10,3,2", "Data!A1:B10,3", "Data!A1:B10,0,2", "Data!A12:E12,3", "Data!A12:E12,1,3",
        "Data!A1:A10,11", "Data!A1:A10,0", "Data!A1:A10,-1", "Data!A1:B10,3,3", "(Data!A1:B3,Data!D1:E3),2,1,2",
        "(Data!A1:B3,Data!D1:E3),2,1,3", "{1,2;3,4},2,1", "{1,2;3,4},2", "{1,2,3},2", "{1;2;3},2", "Data!A1:A10,2.9",
        "Data!A1:A10,\"2\"", "SUM(INDEX(Data!A1:B10,0,1))", "SUM(INDEX(Data!A1:B10,2,0))", "SUM(Data!A1:INDEX(Data!A1:A10,3))",
        "Data!A1:A10,1,1", "Data!A1:A10,1,2", "Data!A1,1,1", "Data!A1,2,1", "{1,2;3,4},0,0", "SUM(INDEX({1,2;3,4},0,0))",
        "Data!H1:H10,2",
    ]);
    calls(&mut v, "CHOOSE", &[
        "2,\"a\",\"b\",\"c\"", "1,Data!A1,Data!A2", "4,1,2,3", "0,1,2", "2.9,1,2,3", "\"2\",1,2", "TRUE,1,2",
        "#N/A,1,2", "2,1,", "SUM(CHOOSE(2,Data!A1:A3,Data!B1:B3))", "2,1,1/0", "-1,1,2",
    ]);
    for f in [
        "SUM(OFFSET(Data!A1,1,0,2,1))", "OFFSET(Data!A1,2,1)", "SUM(OFFSET(Data!A1:B2,1,1))", "OFFSET(Data!A1,-1,0)",
        "SUM(OFFSET(Data!A1,0,0,-2,1))", "SUM(OFFSET(Data!C3,0,0,-2,-2))", "OFFSET(Data!A1,0,0,0,1)",
        "SUM(OFFSET(Data!A1,1,0,2))", "ROWS(OFFSET(Data!A1:B3,0,0))", "COLUMNS(OFFSET(Data!A1,0,0,1,3))",
        "OFFSET(Data!A1,1.9,0.9)", "SUM(OFFSET(Data!A1:A3,1,0,,2))", "OFFSET(Data!A1:A3,0,0)",
        "SUM(INDIRECT(\"Data!A1:A3\"))", "INDIRECT(\"Data!B2\")", "INDIRECT(\"'Data'!B2\")", "INDIRECT(\"Data!R2C2\",FALSE)",
        "INDIRECT(\"R2C2\",FALSE)", "INDIRECT(\"B2\")", "INDIRECT(\"NoSheet!A1\")", "INDIRECT(\"\")", "INDIRECT(\"Nums\")",
        "SUM(INDIRECT(\"Nums\"))", "INDIRECT(\"Data!A\"&3)", "INDIRECT(\"1+1\")", "SUM(INDIRECT(\"Data!A:A\"))",
        "INDIRECT(\"Data!R[1]C[1]\",FALSE)", "INDIRECT(\"Data!a1\")", "INDIRECT(\"Data!$A$1\")", "INDIRECT(\"S1:S3!A1\")",
        "ROW()", "COLUMN()", "ROW(Data!C5)", "COLUMN(Data!C5)", "ROW(Data!A3:B7)", "COLUMN(Data!B3:D3)",
        "SUM(ROW(Data!A1:A3))", "ROWS(Data!A1:B7)", "COLUMNS(Data!A1:B7)", "ROWS({1,2;3,4;5,6})", "COLUMNS({1,2,3})",
        "ROWS(5)", "ROWS(Data!A:A)", "COLUMNS(Data!1:1)", "ROW(Data!A:A)", "COLUMN(Data!5:5)", "AREAS(Data!A1:B2)",
        "AREAS((Data!A1,Data!B2,Data!C3))", "AREAS(Data!A1:B2 Data!B1:C2)", "ROWS((Data!A1:A3,Data!B1:B5))",
        "ADDRESS(2,3)", "ADDRESS(2,3,2)", "ADDRESS(2,3,3)", "ADDRESS(2,3,4)", "ADDRESS(2,3,1,FALSE)", "ADDRESS(2,3,4,FALSE)",
        "ADDRESS(2,3,1,TRUE,\"Data\")", "ADDRESS(2,3,1,TRUE,\"My Sheet\")", "ADDRESS(2,3,1,TRUE,\"\")", "ADDRESS(0,1)",
        "ADDRESS(1048576,16384)", "ADDRESS(1048577,1)", "ADDRESS(1,16385)", "ADDRESS(2,3,5)", "ADDRESS(2.9,3.9)",
        "ADDRESS(2,3,2,FALSE)", "ADDRESS(2,3,3,FALSE,\"S1\")", "ADDRESS(1,27)", "ADDRESS(1,702)", "ADDRESS(1,703)",
        "ADDRESS(2,3,1,TRUE,\"한글\")", "ADDRESS(2,3,1,TRUE,\"1abc\")", "ADDRESS(2,3,1,TRUE,\"A1\")",
        "ADDRESS(2,3,1,TRUE,\"a.b\")", "ADDRESS(2,3,1,TRUE,\"it's\")", "ADDRESS(2,3,1,TRUE,\"Data!\")",
        "INDEX(TRANSPOSE(Data!A1:A3),1,2)", "TRANSPOSE(5)", "HYPERLINK(\"http://example.com\",\"link\")",
        "HYPERLINK(\"http://example.com\")", "HYPERLINK(\"#Data!A1\",Data!A1)", "HYPERLINK(\"x\",1/0)",
        "_xlfn.FORMULATEXT(Data!A1)", "_xlfn.FORMULATEXT(Hid!B5)", "_xlfn.FORMULATEXT(B1)", "_xlfn.FORMULATEXT(1)",
        "_xlfn.FORMULATEXT(Hid!B2:B5)",
        "GETPIVOTDATA(\"x\",Data!A1)",
    ] {
        v.push(c(f));
    }
    calls(&mut v, "_xlfn.XLOOKUP", &[
        "30,Data!F1:F10,Data!G1:G10", "35,Data!F1:F10,Data!G1:G10", "35,Data!F1:F10,Data!G1:G10,\"none\"",
        "35,Data!F1:F10,Data!G1:G10,,-1", "35,Data!F1:F10,Data!G1:G10,,1", "\"a*\",Data!D1:D10,Data!A1:A10,,2",
        "\"apple\",Data!D1:D10,Data!A1:A10,,0,-1", "\"apple\",Data!D1:D10,Data!A1:A10", "40,Data!F1:F10,Data!G1:G10,,0,2",
        "45,Data!F1:F10,Data!G1:G10,,-1,2", "30,Data!A12:E12,Data!A13:E13", "\"Kim\",Data!Q2:Q11,Data!S2:S11",
        "1000,Data!F1:F10,Data!G1:G10", "30,Data!F1:F10,Data!G1:G5",
    ]);
    calls(&mut v, "_xlfn.XMATCH", &[
        "40,Data!F1:F10", "35,Data!F1:F10,-1", "35,Data!F1:F10,1", "\"b*\",Data!D1:D10,2", "\"apple\",Data!D1:D10,0,-1",
        "40,Data!F1:F10,0,2", "1000,Data!F1:F10",
    ]);
    v
}

// ---------------------------------------------------------------------------
// 날짜·시간
// ---------------------------------------------------------------------------

fn date() -> Vec<Case> {
    let mut v = Vec::new();
    calls(&mut v, "DATE", &[
        "2024,1,15", "2024,13,1", "2024,0,1", "2024,-1,1", "2024,1,0", "2024,1,-1", "2024,2,30", "1900,1,1",
        "1900,2,29", "1900,3,1", "1900,1,0", "1899,12,31", "0,1,1", "99,1,1", "100,1,1", "1899,1,1",
        "1904,1,1", "9999,12,31", "10000,1,1", "-1,1,1", "2024.9,1.9,1.9", "\"2024\",\"1\",\"1\"", "2024,1,1000",
        "2024,120,1", "1900,1,-1", "1900,0,0", "1,1,1", "2024,1,1E+6", "2024,1,3E+6", "1900,2,28", "1900,2,30",
        "2000,2,29", "2100,2,29", "TRUE,1,1",
    ]);
    calls(&mut v, "TIME", &[
        "12,0,0", "0,0,0", "23,59,59", "24,0,0", "25,0,0", "0,90,0", "0,0,90", "-1,0,0", "0,-1,0", "1,-30,0",
        "12.9,30.9,30.9", "32767,0,0", "32768,0,0", "0,0,-1", "0,0,86400", "\"6\",\"30\",0",
    ]);
    calls(&mut v, "DATEVALUE", &[
        "\"2024-01-15\"", "\"2024/1/15\"", "\"2024.1.15\"", "\"1/15/2024\"", "\"15-Jan-2024\"", "\"Jan 15, 2024\"",
        "\"2024년 1월 15일\"", "\"2024-01-15 10:30\"", "\"1900-01-01\"", "\"1900-02-29\"", "\"1899-12-31\"",
        "\"9999-12-31\"", "\"abc\"", "\"\"", "45000", "\"1/15\"", "\"15-Jan\"", "\"Jan-2024\"", "\"2024-13-01\"",
        "\"10:30\"", "\"24-01-15\"", "\"01-15-24\"", "\"2024. 1. 15.\"", "\"January 15, 2024\"", "\"15 January 2024\"",
        "\"1월 15일\"", "\"2024-1-5\"", "\"2/29/2023\"", "\"12/31/29\"", "\"12/31/30\"", "\"2024/01/15 오후 3:00\"",
        "TRUE", "\"  2024-01-15  \"",
    ]);
    calls(&mut v, "TIMEVALUE", &[
        "\"10:30\"", "\"10:30:15\"", "\"10:30 PM\"", "\"22:30\"", "\"오후 3:00\"", "\"2024-01-15 18:00\"", "\"12:00 AM\"",
        "\"12:00 PM\"", "\"24:00\"", "\"25:00\"", "\"10:60\"", "\"abc\"", "\"10\"", "\"10:30:15.5\"", "\"0:00\"",
        "0.5", "\"1:2:3\"", "\"10:30 pm\"", "\"10:30PM\"", "\"10 PM\"",
    ]);
    for name in ["YEAR", "MONTH", "DAY"] {
        calls(&mut v, name, &[
            "45000", "0", "1", "59", "60", "61", "2958465", "2958466", "-1", "45000.75", "\"2024-01-15\"", "TRUE",
            "Data!C4", "366", "1462", "\"abc\"", "36526", "2958465.9999",
        ]);
    }
    for name in ["HOUR", "MINUTE", "SECOND"] {
        calls(&mut v, name, &[
            "0.5", "0.999999", "1.5", "45000.123456", "-0.5", "0.99999999", "\"10:30:15\"", "\"2024-01-15 18:45:30\"",
            "0.000011574", "0.0000057870", "0.0000057871", "TRUE", "2958466", "45000.99999999", "1/24/60/60*59.5",
        ]);
    }
    calls(&mut v, "WEEKDAY", &[
        "45000", "45000,1", "45000,2", "45000,3", "45000,11", "45000,12", "45000,13", "45000,14", "45000,15",
        "45000,16", "45000,17", "45000,4", "45000,0", "0", "1", "60", "61", "-1", "\"2024-01-15\"", "45000.9,2",
        "2958465", "45001,2", "45002,3",
    ]);
    calls(&mut v, "WEEKNUM", &[
        "45000", "45000,1", "45000,2", "45000,11", "45000,12", "45000,13", "45000,14", "45000,15", "45000,16",
        "45000,17", "45000,21", "45000,3", "44927", "44927,2", "44927,21", "45291", "45291,21", "1", "0", "60",
        "45292,1", "45292,2", "45657,21", "45658,21",
    ]);
    // ISOWEEKNUM(1)(1900-01-01)은 한셀을 죽인다 (관측: 자동화 서버가 RPC 오류로 끝남) — 넣지 않는다
    calls(&mut v, "_xlfn.ISOWEEKNUM", &[
        "45000", "44927", "45291", "45292", "45657", "45658", "0", "-1", "36526", "8", "9", "15", "366", "367",
    ]);
    for name in ["EDATE", "EOMONTH"] {
        calls(&mut v, name, &[
            "45000,1", "45000,-1", "45000,0", "45000,12", "45000,-12", "45000,1.9", "45016,1", "45322,1", "45351,12",
            "1,0", "60,0", "60,1", "0,1", "45000,-1500", "2958465,1", "45000,\"1\"", "\"2024-01-31\",1", "-1,1",
            "45000.9,0", "31,1", "59,1",
        ]);
    }
    calls(&mut v, "_xlfn.DAYS", &[
        "45100,45000", "45000,45100", "\"2024-03-01\",\"2024-02-01\"", "45000.9,45000.1", "1,0", "-1,0", "45000,\"abc\"",
    ]);
    calls(&mut v, "DAYS360", &[
        "44927,45291", "44927,45291,TRUE", "44985,45016", "44985,45016,TRUE", "45016,45046", "45351,45382",
        "45351,45382,TRUE", "45322,45351", "45000,44927", "44957,44985", "44957,44986", "44986,45016", "1,60",
        "60,61", "44985,45351", "45351,45716", "44926,44957", "44957,45016,FALSE",
    ]);
    calls(&mut v, "DATEDIF", &[
        "43831,45000,\"Y\"", "43831,45000,\"M\"", "43831,45000,\"D\"", "43831,45000,\"YM\"", "43831,45000,\"MD\"",
        "43831,45000,\"YD\"", "45000,43831,\"Y\"", "43861,43890,\"MD\"", "43861,43921,\"MD\"", "43890,44255,\"YD\"",
        "43831,45000,\"y\"", "43831,45000,\"X\"", "43861,43951,\"M\"", "43861,43951,\"YM\"", "44255,44620,\"YD\"",
        "44620,44985,\"YD\"", "43524,43890,\"YD\"", "43890,43891,\"MD\"", "45351,45382,\"MD\"", "45000,45000,\"D\"",
        "1,60,\"D\"", "59,61,\"MD\"", "45322,45351,\"MD\"", "45322,45382,\"MD\"",
    ]);
    calls(&mut v, "YEARFRAC", &[
        "44927,45291", "44927,45291,1", "44927,45291,2", "44927,45291,3", "44927,45291,4", "44927,45291,5",
        "45000,44927", "45000,45000", "44985,45351,1", "45351,45716,1", "44927,46022,1", "43890,44256,1",
        "43890,44255,1", "45322,45351,0", "45322,45351,4", "45351,45382,0", "44927.5,45291.5", "1,60,1",
        "36526,45000,1", "45291,45657,1", "45292,45657,1", "45000,45031", "45016,45046,0", "45016,45046,4",
        "44985,45016,0", "44957,44985,0",
    ]);
    calls(&mut v, "NETWORKDAYS", &[
        "45000,45031", "45031,45000", "45000,45031,Data!AF1:AF2", "45292,45310,Data!AF1:AF2", "45000,45000",
        "45001,45001", "45000,45031,45010", "45292,45310,{45292,45306}", "45000.9,45031.1",
    ]);
    calls(&mut v, "_xlfn.NETWORKDAYS.INTL", &[
        "45000,45031", "45000,45031,1", "45000,45031,2", "45000,45031,7", "45000,45031,11", "45000,45031,17",
        "45000,45031,\"0000011\"", "45000,45031,\"1111111\"", "45000,45031,\"0000000\"", "45000,45031,\"011\"",
        "45000,45031,8", "45292,45310,1,Data!AF1:AF2", "45031,45000,11",
    ]);
    calls(&mut v, "WORKDAY", &[
        "45000,10", "45000,-10", "45000,0", "45001,1", "45292,10,Data!AF1:AF2", "45000,10.9", "45000,\"5\"",
        "45000,-1", "45002,1",
    ]);
    // WORKDAY.INTL(…,"1111111")(모든 요일이 주말)은 한셀이 끝나지 않는다 (관측) — 넣지 않는다
    calls(&mut v, "_xlfn.WORKDAY.INTL", &[
        "45000,10", "45000,10,1", "45000,10,7", "45000,10,11", "45000,10,\"0000011\"", "45000,-10,17",
        "45292,10,1,Data!AF1:AF2", "45000,10,99", "45000,0,\"1111111\"",
    ]);
    v
}

fn date1904() -> Vec<Case> {
    let mut v = Vec::new();
    for f in [
        "DATE(1904,1,1)", "DATE(1904,1,0)", "DATE(1903,12,31)", "DATE(2024,1,15)", "DATE(0,1,1)", "DATE(100,1,1)",
        "DATE(1900,1,1)", "DATE(9999,12,31)", "YEAR(0)", "MONTH(0)", "DAY(0)", "YEAR(1)", "DAY(59)", "DAY(60)",
        "YEAR(2957003)", "YEAR(2957004)", "WEEKDAY(0)", "WEEKDAY(1)", "DATEVALUE(\"1904-01-01\")",
        "DATEVALUE(\"1903-12-31\")", "DATEVALUE(\"2024-01-15\")", "\"2024-01-15\"*1", "EDATE(0,1)", "EOMONTH(0,1)",
        "DAYS360(0,365)", "YEARFRAC(0,366,1)", "WEEKNUM(0)", "NETWORKDAYS(0,30)", "WORKDAY(0,10)", "TEXT(0,\"yyyy-mm-dd\")",
        "TEXT(45000,\"yyyy-mm-dd\")", "DATESTRING(0)", "DATEDIF(0,1000,\"Y\")", "_xlfn.ISOWEEKNUM(0)", "_xlfn.DAYS(10,0)",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 정보
// ---------------------------------------------------------------------------

fn info() -> Vec<Case> {
    let mut v = Vec::new();
    const VALS: &[&str] = &[
        "Data!C1", "Data!C2", "Data!C3", "Data!C4", "Data!C5", "Data!C8", "Data!H2", "Data!H4", "1", "\"a\"",
        "TRUE", "#N/A", "1/0", "\"\"", "Data!A1:A3", "{1,2}", "Data!C4:C5", "Hid!B2", "Nums", "Data!A1:A3 Data!B1:B3",
    ];
    for name in [
        "ISBLANK", "ISERR", "ISERROR", "ISNA", "ISLOGICAL", "ISNUMBER", "ISTEXT", "ISNONTEXT", "ISREF",
        "_xlfn.ISFORMULA", "N", "TYPE", "ERROR.TYPE", "ERRORTYPE",
    ] {
        calls(&mut v, name, VALS);
    }
    for name in ["ISEVEN", "ISODD"] {
        calls(&mut v, name, &["2", "3", "-2", "2.5", "3.9", "0", "\"4\"", "TRUE", "Data!C4", "\"abc\"", "#N/A", "-1.5"]);
    }
    for f in [
        "NA()", "_xlfn.SHEET()", "_xlfn.SHEET(Data!A1)", "_xlfn.SHEET(\"S2\")", "_xlfn.SHEET(\"Nope\")", "_xlfn.SHEET(S3!A1)",
        "_xlfn.SHEET(Nums)", "_xlfn.SHEETS()", "_xlfn.SHEETS(S1:S3!A1)", "_xlfn.SHEETS(Data!A1)", "_xlfn.SHEET(1)",
        "CELL(\"address\",Data!B3)", "CELL(\"row\",Data!B3)", "CELL(\"col\",Data!B3)", "CELL(\"contents\",Data!B3)",
        "CELL(\"type\",Data!B3)", "CELL(\"type\",Data!D3)", "CELL(\"type\",Data!C4)", "CELL(\"prefix\",Data!D3)",
        "CELL(\"prefix\",Data!B3)", "CELL(\"format\",Data!B3)", "CELL(\"color\",Data!B3)", "CELL(\"parentheses\",Data!B3)",
        "CELL(\"protect\",Data!B3)", "CELL(\"width\",Data!B3)", "CELL(\"address\",Data!B3:C4)", "CELL(\"row\")",
        "CELL(\"nope\",Data!B3)", "CELL(\"ADDRESS\",Data!B3)", "CELL(\"address\",B5)", "CELL(\"contents\",Data!H2)",
        "INFO(\"numfile\")", "INFO(\"recalc\")", "INFO(\"nope\")", "INFO(\"osversion\")", "INFO(\"system\")",
        "TYPE(Data!A1:A3)", "TYPE(1/0)", "TYPE({1,2})", "N(Data!E1)", "N(\"5\")", "ISNUMBER(\"5\")",
        "ISTEXT(Data!C5)", "ISBLANK(\"\")", "ISBLANK(Data!C8)", "ISREF(Data!A1:B2)", "ISREF(INDIRECT(\"Data!A1\"))",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 재무
// ---------------------------------------------------------------------------

fn financial() -> Vec<Case> {
    let mut v = Vec::new();
    calls(&mut v, "PV", &[
        "0.08/12,240,500", "0.05,10,-100", "0,10,-100", "0.05,10,-100,1000", "0.05,10,-100,0,1", "-0.05,10,100",
        "0.05,0,100", "0.05,10,0,-1000", "\"0.05\",10,100", "0.05,10,-100,,2", "-1,10,100", "1E-10,10,100",
    ]);
    calls(&mut v, "FV", &[
        "0.06/12,10,-200,-500,1", "0.12/12,12,-1000", "0,12,-100,-1000", "0.05,10,-100,0,0", "-0.05,10,100",
        "0.05,10.5,-100", "0.05,-10,-100", "1E-10,10,-100",
    ]);
    calls(&mut v, "PMT", &[
        "0.08/12,10,10000", "0.08/12,10,10000,0,1", "0,10,1000", "0.05,10,-1000,500", "0.05,0,1000", "-0.5,10,1000",
        "0.06/12,18*12,0,50000", "1E-10,10,1000", "-1,10,100",
    ]);
    calls(&mut v, "NPER", &[
        "0.12/12,-100,-1000,10000,1", "0.12/12,-100,-1000,10000", "0.12/12,-100,-1000", "0,-100,1000", "0,0,1000",
        "0.1,100,1000", "0.05,-100,1000,0,1", "-0.05,-100,1000", "0.1,-100,-1000",
    ]);
    calls(&mut v, "RATE", &[
        "4*12,-200,8000", "4,-2000,8000", "10,-100,1000,0,1", "10,-100,1000,0,0,0.5", "10,100,1000", "360,-1000,200000",
        "1,-1,1", "10,0,-1000,2000", "12,-1000,10000,0,0,-0.5", "5,-100,-100,1000", "0.5,-100,50",
    ]);
    for name in ["IPMT", "PPMT"] {
        calls(&mut v, name, &[
            "0.1/12,1,36,8000", "0.1,3,3,8000", "0.1,0,3,8000", "0.1,4,3,8000", "0.1,1,3,8000,0,1", "0.1,2,3,8000,0,1",
            "0,2,3,8000", "0.1,2.5,3,8000", "0.1,1,3,8000,1000",
        ]);
    }
    for name in ["CUMIPMT", "CUMPRINC"] {
        calls(&mut v, name, &[
            "0.09/12,30*12,125000,13,24,0", "0.09/12,30*12,125000,1,1,0", "0.09/12,30*12,125000,1,1,1",
            "0.09/12,30*12,125000,13,24,2", "0.09/12,30*12,125000,0,24,0", "0.09/12,30*12,125000,25,24,0",
            "0,10,1000,1,2,0", "0.09/12,30*12,125000,13.5,24.5,0", "0.01,12,-1000,1,12,0",
        ]);
    }
    calls(&mut v, "ISPMT", &["0.1/12,1,36,8000000", "0.1,1,3,8000000", "0.1,0,0,100", "0,1,3,100"]);
    calls(&mut v, "NPV", &[
        "0.1,-10000,3000,4200,6800", "0.08,Data!AA2:AA6", "0.08,Data!AA2:AA6,-500", "-1,100", "0.1,Data!C1:C10",
        "0.1,\"5\",TRUE", "0.1,Data!H1:H3", "0,1,2,3", "-0.5,1,2", "0.1",
    ]);
    calls(&mut v, "XNPV", &[
        "0.09,Data!AA1:AA6,Data!AB1:AB6", "0,Data!AA1:AA6,Data!AB1:AB6", "0.09,Data!AA1:AA5,Data!AB1:AB6",
        "0.09,{-100,110},{45000,44000}", "-1,Data!AA1:AA6,Data!AB1:AB6", "0.09,{100},{45000}",
    ]);
    calls(&mut v, "IRR", &[
        "Data!AA1:AA6", "Data!AA1:AA3", "Data!AA1:AA6,0.5", "{-70000,12000,15000,18000,21000}",
        "{-70000,12000,15000,18000,21000,26000}", "{-70000,12000,15000},-0.1", "{100,200}", "{-100,-200}",
        "{-100,0,0,0,200}", "Data!C1:C10",
    ]);
    calls(&mut v, "XIRR", &[
        "Data!AA1:AA6,Data!AB1:AB6", "Data!AA1:AA6,Data!AB1:AB6,0.5", "{-10000,2750,4250,3250,2750},{39448,39508,39751,39859,39904}",
        "{100,200},{45000,45100}", "Data!AA1:AA5,Data!AB1:AB6",
    ]);
    calls(&mut v, "MIRR", &[
        "{-120000,39000,30000,21000,37000,46000},0.1,0.12", "{-120000,39000,30000,21000},0.1,0.12", "Data!AA1:AA6,0.1,0.12",
        "{100,200},0.1,0.1", "{-100,-200},0.1,0.1", "{-100,200},-1,0.1",
    ]);
    calls(&mut v, "SLN", &["30000,7500,10", "30000,7500,0", "30000,7500,-10", "\"30000\",0,5"]);
    calls(&mut v, "SYD", &["30000,7500,10,1", "30000,7500,10,10", "30000,7500,10,11", "30000,7500,10,0", "30000,7500,10,2.5", "30000,-1,10,1"]);
    calls(&mut v, "DB", &[
        "1000000,100000,6,1,7", "1000000,100000,6,2,7", "1000000,100000,6,7,7", "1000000,100000,6,8,7", "1000000,100000,6,1",
        "1000000,100000,6,6", "1000000,0,6,1", "1000000,100000,6,1,13", "1000000,100000,6,1,0", "1000000,1000000,6,1",
        "0,0,6,1", "1000000,100000,6.5,2", "1000000,100000,6,2.5", "100,200,5,1",
    ]);
    calls(&mut v, "DDB", &[
        "2400,300,10*365,1", "2400,300,10*12,1,2", "2400,300,10,1,2", "2400,300,10,10", "2400,300,10,11",
        "2400,300,10,1,1.5", "2400,300,10,0.5", "2400,300,10,2.5", "2400,0,10,10", "2400,300,10,1,0", "2400,300,10,8",
        "100,200,5,1", "2400,300,1,1",
    ]);
    calls(&mut v, "VDB", &[
        "2400,300,10*365,0,1", "2400,300,10*12,0,1", "2400,300,10,0,0.875,1.5", "2400,300,10,6,10", "2400,300,10,6,10,2,TRUE",
        "2400,300,10,0,10", "2400,300,10,0,10,2,TRUE", "2400,300,10,1.5,3.5", "2400,300,10,3,2", "2400,300,10,0,11",
        "2400,0,10,0,10", "2400,300,10,0,1,0", "2400,300,10,5,7,3", "2400,300,10,0,0",
    ]);
    calls(&mut v, "EFFECT", &["0.0525,4", "0.1,1", "0.1,0", "-0.1,4", "0.1,4.9", "0.1,365"]);
    calls(&mut v, "NOMINAL", &["0.053543,4", "0.1,1", "0.1,0", "-0.1,4", "0.1,12.5"]);
    calls(&mut v, "FVSCHEDULE", &["1,{0.09,0.11,0.1}", "1000,Data!B1:B3", "1,Data!C1:C4", "1,{\"a\"}", "1,Data!H1:H2", "100,0.1"]);
    calls(&mut v, "DOLLARDE", &["1.02,16", "1.1,32", "1.02,0", "1.02,-1", "-1.02,16", "1.1,1", "1.123,100", "1.1,3.9", "1.1,10"]);
    calls(&mut v, "DOLLARFR", &["1.125,16", "1.125,32", "1.125,0", "-1.125,16", "1.5,10", "1.5,3"]);
    calls(&mut v, "_xlfn.PDURATION", &["0.025,2000,2200", "0.0025,1000,1200", "0,1,2", "0.1,-1,2", "0.1,2,1"]);
    calls(&mut v, "_xlfn.RRI", &["96,10000,11000", "10,100,100", "0,1,2", "10,0,2", "10,-1,2", "10,1,-2", "2,4,1"]);
    calls(&mut v, "TBILLPRICE", &["39538,39600,0.09", "45000,45366,0.05", "45000,45367,0.05", "45000,45000,0.05", "45000,45100,-0.05", "45000,45100,4"]);
    calls(&mut v, "TBILLYIELD", &["39538,39600,98.45", "45000,45366,95", "45000,45100,0", "45000,45100,101"]);
    calls(&mut v, "TBILLEQ", &["39538,39600,0.0914", "45000,45300,0.05", "45000,45366,0.05", "45000,45100,0", "45000,45100,2"]);
    let secs = "39484,39634";
    calls(&mut v, "DISC", &[&format!("{secs},97.975,100,1"), &format!("{secs},97.975,100"), "45000,44000,97,100", "45000,45100,0,100", &format!("{secs},97.975,100,5"), &format!("{secs},97.975,100,0"), &format!("{secs},97.975,100,2"), &format!("{secs},97.975,100,3"), &format!("{secs},97.975,100,4")]);
    calls(&mut v, "INTRATE", &["39478,39568,1000000,1014420,2", "39478,39568,1000000,1014420", "39478,39568,0,1014420", "39478,39568,1000000,1014420,1"]);
    calls(&mut v, "RECEIVED", &["39478,39568,1000000,0.0575,2", "39478,39568,1000000,0.0575", "39478,39568,1000000,20", "39478,39568,1000000,0.0575,1"]);
    calls(&mut v, "PRICEDISC", &["39484,39634,0.0525,100,2", "39484,39634,0.0525,100", "39484,39634,0.0525,100,1", "39484,39634,0,100"]);
    calls(&mut v, "YIELDDISC", &["39494,39555,99.795,100,2", "39494,39555,99.795,100", "39494,39555,99.795,100,1", "39494,39555,0,100"]);
    calls(&mut v, "PRICEMAT", &["39493,39551,39397,0.061,0.061,0", "39493,39551,39397,0.061,0.061", "39493,39551,39397,0.061,0.061,1", "39493,39551,39397,0.061,0.061,2", "39493,39551,39397,0.061,0.061,3", "39493,39551,39397,0.061,0.061,4", "39493,39551,39600,0.061,0.061"]);
    calls(&mut v, "YIELDMAT", &["39522,40258,39381,0.0625,100.0123,0", "39522,40258,39381,0.0625,100.0123", "39522,40258,39381,0.0625,100.0123,1", "39522,40258,39381,0.0625,100.0123,3"]);
    calls(&mut v, "ACCRINTM", &["39539,39614,0.1,1000,3", "39539,39614,0.1,1000", "39539,39614,0.1", "39539,39614,0.1,1000,1", "39614,39539,0.1,1000", "39539,39539,0.1,1000", "1,1,1,1", "39539,39614,0,1000"]);
    calls(&mut v, "ACCRINT", &[
        "39508,39691,39569,0.1,1000,2,0", "39508,39691,39569,0.1,1000,2,0,FALSE", "39508,39691,39569,0.1,1000,2,0,TRUE",
        "39508,39691,39569,0.1,,2", "39508,39691,39569,0.1,1000,4,1", "39508,39691,39569,0.1,1000,1,3", "39508,39691,39569,0.1,1000,3,0",
        "39508,39691,39569,0.1,1000,2,4", "39508,39691,39508,0.1,1000,2,0",
    ]);
    let cp = ["40568,40862,2,1", "40568,40862,2", "40568,40862,4,0", "40568,40862,1,3", "40568,40862,2,4", "40568,40862,2,2",
        "45000,46000,2,0", "45351,45716,4,1", "45350,45716,2,0", "40862,40568,2,1", "40568,40862,3,1", "45016,45382,2,0",
        "45000,45016,4,1"];
    for name in ["COUPDAYBS", "COUPDAYS", "COUPDAYSNC", "COUPNCD", "COUPNUM", "COUPPCD"] {
        calls(&mut v, name, &cp);
    }
    calls(&mut v, "PRICE", &[
        "39493,43054,0.0575,0.065,100,2,0", "39493,43054,0.0575,0.065,100,2", "39493,43054,0.0575,0.065,100,2,1",
        "39493,43054,0.0575,0.065,100,1,3", "39493,43054,0.0575,0.065,100,4,4", "39493,39600,0.0575,0.065,100,2,0",
        "39493,43054,0,0,100,2,0", "39493,43054,0.0575,-0.065,100,2,0", "45000,45100,0.05,0.04,100,2,1",
    ]);
    calls(&mut v, "YIELD", &[
        "39494,42842,0.0575,95.04287,100,2,0", "39494,42842,0.0575,95.04287,100,2", "39494,42842,0.0575,95.04287,100,1,1",
        "39494,39600,0.0575,99,100,2,0", "39494,42842,0.0575,0,100,2,0", "45000,45100,0.05,101,100,2,1",
    ]);
    for name in ["DURATION", "MDURATION"] {
        calls(&mut v, name, &[
            "39448,42370,0.08,0.09,2,1", "39448,42370,0.08,0.09,2", "39448,42370,0.08,0.09,1,0", "39448,42370,0.08,0.09,4,3",
            "39448,39600,0.08,0.09,2,1", "39448,42370,0,0.09,2,1",
        ]);
    }
    for name in ["AMORLINC", "AMORDEGRC"] {
        calls(&mut v, name, &[
            "2400,39679,39813,300,1,0.15,1", "2400,39679,39813,300,0,0.15,1", "2400,39679,39813,300,5,0.15,1",
            "2400,39679,39813,300,7,0.15,1", "2400,39679,39813,300,1,0.15,0", "2400,39679,39813,300,1,0.15,3",
            "2400,39679,39813,300,1,0.15,4", "2400,39679,39813,300,1,0.15,2", "2400,39679,39813,300,1,0.25",
            "2400,39679,39813,300,1,0.4", "2400,39679,39813,300,2,0.1,1", "2400,39679,39813,300,10,0.1,1",
        ]);
    }
    calls(&mut v, "ODDFPRICE", &[
        "39763,44256,39736,39873,0.0785,0.0625,100,2,1", "39763,44256,39736,39873,0.0785,0.0625,100,2,0",
        "39763,44256,39736,39873,0.0785,0.0625,100,1,0", "39763,44256,39736,39873,0.0785,0.0625,100,4,1",
    ]);
    calls(&mut v, "ODDFYIELD", &[
        "39763,44256,39736,39873,0.0575,84.5,100,2,0", "39763,44256,39736,39873,0.0575,84.5,100,2,1",
    ]);
    calls(&mut v, "ODDLPRICE", &[
        "39494,39762,39345,0.0375,0.0405,100,2,0", "39494,39762,39345,0.0375,0.0405,100,2,1", "39494,39762,39345,0.0375,0.0405,100,1,0",
    ]);
    calls(&mut v, "ODDLYIELD", &[
        "39555,39614,39494,0.0375,99.875,100,2,0", "39555,39614,39494,0.0375,99.875,100,2,1",
    ]);
    v
}

// ---------------------------------------------------------------------------
// 공학
// ---------------------------------------------------------------------------

fn engineering() -> Vec<Case> {
    let mut v = Vec::new();
    calls(&mut v, "BIN2DEC", &["\"1100100\"", "\"1111111111\"", "\"1000000000\"", "\"0\"", "\"\"", "\"12\"", "\"11111111111\"", "101", "1.5", "TRUE", "Data!AH1", "\" 101\"", "-1"]);
    for name in ["BIN2OCT", "BIN2HEX"] {
        calls(&mut v, name, &["\"1001\"", "\"1001\",3", "\"1001\",0", "\"1001\",11", "\"1111111111\"", "\"1111111111\",3", "\"1001\",1.9", "\"1001\",-1", "\"\"", "\"2\""]);
    }
    calls(&mut v, "OCT2DEC", &["\"54\"", "\"7777777533\"", "\"4000000000\"", "\"8\"", "\"\"", "777", "\"77777777777\""]);
    for name in ["OCT2BIN", "OCT2HEX"] {
        calls(&mut v, name, &["\"3\"", "\"3\",3", "\"7777777000\"", "\"1000\"", "\"777\"", "\"7777777000\",3", "\"12\",1"]);
    }
    calls(&mut v, "HEX2DEC", &["\"A5\"", "\"FFFFFFFF5B\"", "\"8000000000\"", "\"3DA408B9\"", "\"G\"", "\"\"", "\"ff\"", "\"FFFFFFFFFFF\"", "Data!AH2", "10"]);
    for name in ["HEX2BIN", "HEX2OCT"] {
        calls(&mut v, name, &["\"F\"", "\"F\",8", "\"B7\"", "\"FFFFFFFFFF\"", "\"FFFFFFFE00\"", "\"FFFFFFFDFF\"", "\"200\"", "\"1FF\"", "\"FFE0000000\"", "\"1FFFFFFF\"", "\"20000000\""]);
    }
    for name in ["DEC2BIN", "DEC2OCT", "DEC2HEX"] {
        calls(&mut v, name, &[
            "9", "9,4", "-100", "511", "512", "-512", "-513", "0", "9,0", "9,1", "9,10", "9,11", "9.9", "-0.5",
            "\"9\"", "TRUE", "536870911", "536870912", "-536870912", "549755813887", "549755813888", "-549755813888",
            "255,2.5", "Data!C4",
        ]);
    }
    for name in ["_xlfn.BITAND", "_xlfn.BITOR", "_xlfn.BITXOR"] {
        calls(&mut v, name, &["1,5", "13,25", "0,0", "-1,1", "1.5,1", "2^48,1", "2^48-1,1", "\"5\",3", "TRUE,1"]);
    }
    for name in ["_xlfn.BITLSHIFT", "_xlfn.BITRSHIFT"] {
        calls(&mut v, name, &["4,2", "13,-2", "1,47", "1,48", "1,53", "1,54", "2^48-1,1", "0,100", "5,1.9", "-1,1", "13,0"]);
    }
    calls(&mut v, "DELTA", &["5,4", "5,5", "0.5,0", "5", "0", "\"5\",5", "TRUE,1", "\"a\",1"]);
    calls(&mut v, "GESTEP", &["5,4", "5,5", "-4,-5", "-1", "1", "\"5\",5", "0"]);
    calls(&mut v, "ERF", &["0.745", "1,2", "0", "-1", "3", "6", "-6", "0.1", "1.5", "2.5", "4", "10", "1E-10", "2,1", "\"1\"", "-0.5,0.5", "0.5,"]);
    calls(&mut v, "_xlfn.ERF.PRECISE", &["0.745", "1", "-1", "0", "3", "0.25"]);
    calls(&mut v, "ERFC", &["1", "0", "-1", "3", "6", "10", "26", "27", "28", "-6", "0.5", "1.5", "2.5", "5"]);
    calls(&mut v, "_xlfn.ERFC.PRECISE", &["1", "0", "-1", "3", "10"]);
    for name in ["BESSELJ", "BESSELY", "BESSELI", "BESSELK"] {
        for x in ["0", "0.1", "0.5", "1", "1.5", "1.9", "2.5", "3.7", "5", "8", "10", "25", "30", "50", "100", "-1", "-2.5", "\"1\""] {
            for n in ["0", "1", "2", "3", "5"] {
                v.push(c(format!("{name}({x},{n})")));
            }
        }
        calls(&mut v, name, &["1,-1", "1,1.9", "1,10", "1,30", "1E-5,1", "1,\"1\"", "1,TRUE", "Data!C4,1"]);
    }
    calls(&mut v, "CONVERT", &[
        "1,\"lbm\",\"kg\"", "68,\"F\",\"C\"", "2.5,\"ft\",\"sec\"", "1,\"m\",\"ft\"", "1,\"km\",\"mi\"", "1,\"in\",\"cm\"",
        "1,\"hr\",\"mn\"", "1,\"yr\",\"day\"", "100,\"C\",\"K\"", "0,\"K\",\"F\"", "1,\"atm\",\"Pa\"", "1,\"mmHg\",\"Pa\"",
        "1,\"BTU\",\"J\"", "1,\"HP\",\"W\"", "1,\"T\",\"ga\"", "1,\"gal\",\"l\"", "1,\"tsp\",\"tbs\"", "1,\"ha\",\"m2\"",
        "1,\"byte\",\"bit\"", "1,\"kbyte\",\"byte\"", "1,\"kibyte\",\"byte\"", "1,\"Mibyte\",\"kibyte\"", "1,\"mph\",\"m/s\"",
        "1,\"kn\",\"m/s\"", "1,\"ly\",\"m\"", "1,\"Nmi\",\"m\"", "1,\"ang\",\"m\"", "1,\"pica\",\"pt\"", "1,\"Pica\",\"in\"",
        "1,\"cal\",\"J\"", "1,\"eV\",\"J\"", "1,\"Wh\",\"J\"", "1,\"kWh\",\"J\"", "1,\"g\",\"ozm\"", "1,\"stone\",\"lbm\"",
        "1,\"ton\",\"kg\"", "1,\"uk_ton\",\"kg\"", "1,\"m^2\",\"ft^2\"", "1,\"m3\",\"l\"", "1,\"ft3\",\"l\"", "1,\"barrel\",\"l\"",
        "1,\"cup\",\"oz\"", "1,\"pt\",\"qt\"", "1,\"uk_pt\",\"l\"", "1,\"Rank\",\"K\"", "1,\"Reau\",\"C\"", "1,\"cel\",\"fah\"",
        "1,\"kel\",\"C\"", "1,\"mK\",\"K\"", "1,\"m\",\"m\"", "1,\"xyz\",\"m\"", "1,\"M\",\"m\"", "1,\"cm\",\"mm\"",
        "1,\"dam\",\"m\"", "1,\"hm\",\"m\"", "1,\"um\",\"m\"", "1,\"nm\",\"m\"", "1,\"Gm\",\"km\"", "1,\"mg\",\"g\"",
        "1,\"kg\",\"lbm\"", "1,\"N\",\"dyn\"", "1,\"lbf\",\"N\"", "1,\"psi\",\"Pa\"", "1,\"Torr\",\"Pa\"", "1,\"mi2\",\"m2\"",
        "1,\"us_acre\",\"m2\"", "1,\"uk_acre\",\"m2\"", "1,\"Morgen\",\"m2\"", "1,\"ar\",\"m2\"", "1,\"admkn\",\"kn\"",
        "1,\"km/hr\",\"m/s\"", "1,\"m/hr\",\"m/s\"", "1,\"day\",\"sec\"", "1,\"d\",\"s\"", "1,\"min\",\"s\"", "1,\"Pa\",\"atm\"",
        "1,\"J\",\"e\"", "1,\"c\",\"J\"", "1,\"sg\",\"kg\"", "1,\"u\",\"g\"", "1,\"grain\",\"g\"", "1,\"cwt\",\"lbm\"",
        "1,\"ozm\",\"g\"", "1,\"yd\",\"m\"", "1,\"ell\",\"m\"", "1,\"parsec\",\"ly\"", "1,\"survey_mi\",\"m\"", "1,\"Picapt\",\"pt\"",
        "\"1\",\"m\",\"cm\"", "1,\"m\",\"\"", "1,\"kg\",\"m\"", "1,\"lt\",\"ml\"", "1,\"ft^3\",\"in^3\"", "1,\"GRT\",\"m3\"",
        "1,\"MTON\",\"m3\"", "1,\"HPh\",\"J\"", "1,\"flb\",\"J\"", "1,\"PS\",\"W\"", "1,\"pond\",\"N\"", "1,\"mn\",\"hr\"",
        "1,\"F\",\"K\"", "-40,\"C\",\"F\"", "1,\"ly^3\",\"m^3\"", "1,\"ang^2\",\"m^2\"", "1,\"Nmi^2\",\"m2\"", "1,\"bushel\",\"l\"",
    ]);
    calls(&mut v, "CONVERTK", &[
        "1,\"m\",\"자\",1", "1,\"평\",\"m2\",2", "10,\"근\",\"kg\",3", "1,\"되\",\"l\",4", "1,\"m\",\"자\"", "1,\"자\",\"치\",1",
        "1,\"간\",\"m\",1", "1,\"리\",\"km\",1", "1,\"정보\",\"평\",2", "1,\"관\",\"근\",3", "1,\"섬\",\"말\",4", "1,\"돈\",\"g\",3",
        "1,\"냥\",\"돈\",3", "1,\"홉\",\"l\",4", "1,\"m\",\"m\",1", "1,\"xyz\",\"m\",1", "1,\"m\",\"평\",1", "1,\"m\",\"자\",9",
        "1,\"m\",\"자\",0", "1,\"평\",\"홉\",2", "1,\"작\",\"평\",2", "1,\"단보\",\"평\",2", "1,\"ft\",\"자\",1", "1,\"in\",\"치\",1",
        "1,\"kg\",\"근\",3", "1,\"lb\",\"근\",3", "1,\"gal\",\"되\",4", "1,\"말\",\"되\",4", "1,\"정\",\"간\",1", "1,\"푼\",\"치\",1",
        "1,\"km\",\"리\",1", "1,\"mi\",\"리\",1", "1,\"a\",\"평\",2", "1,\"ha\",\"정보\",2", "1,\"acre\",\"평\",2", "1,\"t\",\"관\",3",
        "1,\"oz\",\"돈\",3", "1,\"ml\",\"홉\",4", "\"2\",\"m\",\"자\",1",
    ]);
    calls(&mut v, "COMPLEX", &["3,4", "3,4,\"j\"", "0,1", "0,-1", "1,0", "0,0", "-1.5,2.25", "3,4,\"I\"", "3,4,\"k\"", "\"3\",\"4\"", "1E-20,1", "1/3,2/3", "1E+20,1"]);
    for name in [
        "IMREAL", "IMAGINARY", "IMABS", "IMARGUMENT", "IMCONJUGATE", "IMSQRT", "IMEXP", "IMLN", "IMLOG10", "IMLOG2",
        "IMSIN", "IMCOS", "_xlfn.IMTAN", "_xlfn.IMCOT", "_xlfn.IMSEC", "_xlfn.IMCSC", "_xlfn.IMSINH", "_xlfn.IMCOSH",
        "_xlfn.IMSECH", "_xlfn.IMCSCH",
    ] {
        calls(&mut v, name, &[
            "\"3+4i\"", "\"1-i\"", "\"-2.5j\"", "\"2\"", "\"i\"", "\"0\"", "\"-1\"", "Data!AG1", "5", "\"3+4\"", "\"abc\"",
            "\"1.5e2+2.5e-1i\"", "\"+i\"", "\"-i\"", "\"4j\"", "\"1+0i\"", "\"0-2i\"", "TRUE", "\"\"",
        ]);
    }
    calls(&mut v, "IMPOWER", &["\"2+3i\",3", "\"2+3i\",0.5", "\"i\",2", "\"0\",2", "\"0\",0", "\"2\",-1", "\"-4\",0.5", "\"1+i\",\"2\""]);
    calls(&mut v, "IMDIV", &["\"-238+240i\",\"10+24i\"", "\"1\",\"i\"", "\"1\",\"0\"", "\"1+i\",\"1-j\"", "\"3\",\"2\""]);
    calls(&mut v, "IMSUB", &["\"13+4i\",\"5+3i\"", "\"1\",\"i\"", "\"1+i\",\"1+i\"", "\"1j\",\"2\"", "\"1i\",\"1j\""]);
    calls(&mut v, "IMSUM", &["\"3+4i\",\"5-3i\"", "Data!AG1:AG2", "Data!AG1:AG4", "\"1\",2,\"i\"", "\"1+i\",\"1+j\"", "Data!C4"]);
    calls(&mut v, "IMPRODUCT", &["\"3+4i\",\"5-3i\"", "\"1+2i\",30", "Data!AG1:AG2", "\"i\",\"i\"", "\"2\""]);
    v
}

// ---------------------------------------------------------------------------
// 데이터베이스
// ---------------------------------------------------------------------------

fn database() -> Vec<Case> {
    let mut v = Vec::new();
    let crits = [
        "Data!V1:W2", "Data!V1:W3", "Data!Y1:Y2", "Data!V5:W6", "Data!V1:V2", "Data!V1:V3", "Data!V1:W1", "Data!AE1:AE2",
    ];
    for name in [
        "DSUM", "DAVERAGE", "DCOUNT", "DCOUNTA", "DMAX", "DMIN", "DPRODUCT", "DSTDEV", "DSTDEVP", "DVAR", "DVARP", "DGET",
    ] {
        for cr in crits {
            v.push(c(format!("{name}(Data!Q1:T11,\"Score\",{cr})")));
        }
        for field in ["4", "\"age\"", "3", "1", "\"Nope\"", "0", "5", "Data!T1", "\"Name\""] {
            v.push(c(format!("{name}(Data!Q1:T11,{field},Data!V1:W2)")));
        }
        v.push(c(format!("{name}(Data!Q1:T11,,Data!V1:W2)")));
    }
    v
}

// ---------------------------------------------------------------------------
// 새 함수(Excel 2019 이후)·한셀 전용·배열 수식
// ---------------------------------------------------------------------------

fn newer() -> Vec<Case> {
    let mut v = Vec::new();
    for f in [
        "_xlfn.SEQUENCE(3)", "SUM(_xlfn.SEQUENCE(3,2))", "SUM(_xlfn.SEQUENCE(3,2,10,5))", "_xlfn.SEQUENCE(0)",
        "SUM(_xlfn.UNIQUE(Data!A1:A10))", "ROWS(_xlfn.UNIQUE(Data!A1:A10))", "ROWS(_xlfn.UNIQUE(Data!A1:A10,FALSE,TRUE))",
        "INDEX(_xlfn._xlws.SORT(Data!A1:A10),1)", "INDEX(_xlfn._xlws.SORT(Data!A1:A10,1,-1),1)", "INDEX(_xlfn._xlws.SORT(Data!D1:D10),2)",
        "INDEX(_xlfn.SORTBY(Data!D1:D10,Data!A1:A10),1)", "SUM(_xlfn._xlws.FILTER(Data!A1:A10,Data!A1:A10>3))",
        "_xlfn._xlws.FILTER(Data!A1:A10,Data!A1:A10>100,\"none\")", "_xlfn._xlws.FILTER(Data!A1:A10,Data!A1:A10>100)",
        "ROWS(_xlfn.RANDARRAY(3,2))", "_xlfn.LET(_xlpm.x,2,_xlpm.x*3)", "_xlfn.LET(_xlpm.a,1,_xlpm.b,_xlpm.a+1,_xlpm.a+_xlpm.b)",
        "_xlfn.TEXTBEFORE(\"a-b-c\",\"-\")", "_xlfn.TEXTAFTER(\"a-b-c\",\"-\",2)", "INDEX(_xlfn.TEXTSPLIT(\"a,b,c\",\",\"),2)",
        "SUM(_xlfn.VSTACK({1,2},{3,4}))", "SUM(_xlfn.HSTACK({1,2},{3,4}))", "SUM(_xlfn.TAKE(Data!A1:A10,3))",
        "SUM(_xlfn.DROP(Data!A1:A10,8))", "SUM(_xlfn.CHOOSEROWS(Data!A1:A10,1,3))", "SUM(_xlfn.TOCOL(Data!A1:B2))",
        "_xlfn.ARRAYTOTEXT({1,2,3})", "_xlfn.VALUETOTEXT(123)", "_xlfn.ENCODEURL(\"a b&c\")", "_xlfn.ISOMITTED(1)",
        "_xlfn.LAMBDA(_xlpm.x,_xlpm.x+1)(2)", "_xlfn.FORECAST.ETS(5,Data!K1:K4,Data!A1:A4)", "_xlfn.CONCAT(\"a\",\"b\")",
        "_xlfn.STOCKHISTORY(\"x\",1)", "_xlfn.IMAGE(\"x\")", "_xlfn.SINGLE(Data!A1:A10)", "_xlfn.NOSUCHFUNCTION(1)",
        "NOSUCHFUNCTION(1)", "_xlfn.FILTERXML(\"<a><b>1</b></a>\",\"//b\")", "CALL(\"x\")", "REGISTER.ID(\"a\",\"b\")",
        "_xlfn.MODE.MULT(1,2,2,3,3)", "INDEX(_xlfn.MODE.MULT(Data!A1:A10),1)", "INDEX(_xlfn.MODE.MULT(Data!A1:A10),2)",
        "_xlfn.TEXTJOIN(\",\",TRUE,_xlfn.SEQUENCE(3))", "SUBTOTALA(9,Hid!A1:A10)", "SUBTOTALA(2,Data!C1:C10)",
        "SUBTOTALA(102,Hid!A1:C10)", "SUBTOTALA(1,Data!C1:C10)", "SUBTOTALA(3,Data!C1:C10)", "SUBTOTALA(4,Data!C1:C10)",
        "SUBTOTALA(109,Hid!A1:A10)", "SUBTOTALA(5,Data!C1:C10)", "SUBTOTALA(7,Data!C1:C10)", "SUBTOTALA(10,Data!C1:C10)",
        "THAIDAYOFWEEK(45000)", "THAIDIGIT(123)", "THAIMONTHOFYEAR(45000)", "THAINUMSOUND(123)", "THAINUMSTRING(123)",
        "THAISTRINGLENGTH(\"abc\")", "ISTHAIDIGIT(\"1\")", "ROUNDBAHTDOWN(12.33)", "ROUNDBAHTUP(12.33)", "THAIYEAR(45000)",
        "RTD(\"a\",\"b\",\"c\")", "CUBEVALUE(\"a\")", "CUBEMEMBER(\"a\",\"b\")", "CUBESETCOUNT(1)", "INFO(\"directory\")",
    ] {
        v.push(c(f));
    }
    v
}

fn arrays() -> Vec<Case> {
    vec![
        arr("SUM(Data!A1:A10*Data!B1:B10)", 1, 1),
        arr("Data!A1:A3*2", 3, 1),
        arr("Data!A1:A3*2", 4, 1),
        arr("Data!A1:A3*2", 3, 2),
        arr("TRANSPOSE(Data!A1:A3)", 1, 3),
        arr("TRANSPOSE(Data!A1:B2)", 2, 2),
        arr("MMULT(Data!M1:O3,Data!M1:O3)", 3, 3),
        arr("MINVERSE(Data!M1:O3)", 3, 3),
        arr("MINVERSE(Data!M5:N6)", 2, 2),
        arr("MINVERSE(Data!M8:N9)", 2, 2),
        arr("FREQUENCY(Data!A1:A10,Data!AE1:AE3)", 4, 1),
        arr("FREQUENCY(Data!B1:B10,{0,5})", 3, 1),
        arr("LINEST(Data!K1:K10,Data!A1:A10,TRUE,TRUE)", 5, 2),
        arr("LINEST(Data!K1:K10,Data!A1:B10,TRUE,TRUE)", 5, 3),
        arr("LINEST(Data!K1:K10,Data!A1:A10,FALSE,TRUE)", 5, 2),
        arr("LOGEST(Data!L1:L10,Data!A1:A10,TRUE,TRUE)", 5, 2),
        arr("TREND(Data!K1:K10,Data!A1:A10,{11;12})", 2, 1),
        arr("GROWTH(Data!L1:L10,Data!A1:A10,{11;12})", 2, 1),
        arr("TREND(Data!K1:K10)", 10, 1),
        arr("IF(Data!A1:A5>3,Data!A1:A5,0)", 5, 1),
        arr("IF(Data!A1:A5>3,\"big\")", 5, 1),
        arr("ROW(Data!A1:A5)", 5, 1),
        arr("COLUMN(Data!A1:C1)", 1, 3),
        arr("Data!A1:A3&\"x\"", 3, 1),
        arr("SUM(IF(Data!A1:A10>3,1,0))", 1, 1),
        arr("MAX(IF(Data!D1:D10=\"apple\",Data!A1:A10))", 1, 1),
        arr("Data!A1:A2*1", 3, 1),
        arr("5", 2, 2),
        arr("{1,2,3}", 2, 3),
        arr("{1;2}", 2, 2),
        arr("{1,2;3,4}", 3, 3),
        arr("_xlfn.MUNIT(3)", 3, 3),
        arr("_xlfn.MODE.MULT(1,2,2,3,3)", 3, 1),
        arr("TEXT(Data!A1:A3,\"0.0\")", 3, 1),
        arr("LEN(Data!D1:D3)", 3, 1),
        arr("Data!A1:A3>2", 3, 1),
        arr("SUM(LEN(Data!D1:D10))", 1, 1),
        arr("INDEX(Data!A1:B10,0,2)", 10, 1),
        arr("Data!A1:B2", 2, 2),
        arr("Data!A1:A10", 1, 1),
        arr("-Data!B1:B3", 3, 1),
        arr("Data!B1:B3%", 3, 1),
        arr("ABS(Data!B1:B3)", 3, 1),
        arr("ROUND(Data!B1:B3,0)", 3, 1),
        arr("Data!A1:A3+Data!A12:C12", 3, 3),
        arr("SUMPRODUCT(Data!A1:A3,Data!B1:B3)", 1, 1),
        arr("ISNUMBER(Data!C1:C5)", 5, 1),
        arr("IFERROR(Data!H1:H4,\"e\")", 4, 1),
        arr("CHOOSE({1,2},Data!A1:A2,Data!B1:B2)", 2, 2),
        arr("VLOOKUP(Data!F1:F3,Data!F1:G10,2,FALSE)", 3, 1),
        arr("MATCH(Data!A1:A3,Data!F1:F10,1)", 3, 1),
        arr("SUM(Data!A1:A3*{1;2;3})", 1, 1),
        arr("COUNTIF(Data!A1:A10,Data!A1:A3)", 3, 1),
        arr("SUMIF(Data!D1:D10,{\"apple\",\"fig\"},Data!A1:A10)", 1, 2),
        arr("OFFSET(Data!A1,{0;1},0)", 2, 1),
        arr("AND(Data!A1:A3>0)", 1, 1),
        arr("ROWS(Data!A1:A3)", 2, 1),
        arr("Data!A1:A3=Data!A1:A3", 3, 1),
        arr("Data!E1:E3-INT(Data!E1:E3)", 3, 1),
        arr("YEAR(Data!E1:E3)", 3, 1),
        arr("MID(\"abcdef\",{1,2,3},2)", 1, 3),
        arr("RANK(Data!A1:A3,Data!A1:A10)", 3, 1),
        arr("LARGE(Data!A1:A10,{1;2;3})", 3, 1),
        arr("N(Data!A1:A3)", 3, 1),
    ]
}

// ---------------------------------------------------------------------------
// 두 번째 묶음: 규칙을 좁히는 촘촘한 실험
// ---------------------------------------------------------------------------

/// 숫자 → 글 (연결 연산자), 글 → 숫자 (연산자·VALUE), 날짜·시각 글
fn conv() -> Vec<Case> {
    let mut v = Vec::new();
    for x in [
        "1E+14", "1E+15", "1.5E+15", "123456789012345", "1234567890123456", "12345678901234567", "1E+16",
        "1.23456789012345E+16", "1E+17", "1E+18", "1E+19", "1.5E+19", "9.99999999999999E+19", "99999999999999990000",
        "1E+20", "1.5E+20", "1E+21", "1E+100", "-1E+15", "-1E+19", "-1E+20", "0.1", "0.01", "0.001", "0.0001",
        "0.00001", "0.000001", "0.0000001", "0.00000001", "1E-9", "1E-10", "1.5E-10", "1.23456789012345E-10",
        "1E-15", "1E-20", "1E-100", "1E-300", "-0.000001", "-0.0000001", "123.456", "1234567.89012345",
        "0.123456789012345", "0.1234567890123456", "1/3", "2/3", "1/7", "1/3*1E+10", "1/3*1E+15", "1/3*1E+16",
        "1/3*1E-5", "1/3*1E-6", "2^53", "2^60", "2^70", "PI()*1E+18", "1E+15+0.5", "0.5", "100", "-100", "1E+308",
        "4.94E-324", "2.2250738585072E-308", "123456789012345678", "0.1+0.7", "1-0.9", "3*1.1",
    ] {
        v.push(c(format!("({x})&\"\"")));
    }
    for s in [
        "TRUE", "true", "FALSE", "True", " TRUE", "TRUE ", "1E3", "1e3", "1E+3", "1E-3", "1.5e2", "e3", "1E", "1E+", ".5",
        "5.", "-.5", "+5", "- 5", "--5", "+-5", "5-", "5+", "(5)", "(-5)", "-(5)", "( 5 )", "$5", "$-5", "-$5", "$ 5",
        "₩5", "\\5", "5$", "5₩", "1,000", "1,000,000", "10,00", "1,0000", ",100", "1,000.5", "1.000,5", "1 000", "50%",
        "5%%", "%5", "50 %", "(50%)", "$50%", "0x10", "1_000", "١٢", "１２", "12３", " 12 ", "\t12", "12\t", "\u{a0}12",
        "\u{3000}12", "", " ", "1 1/2", "1 1/3", "-1 1/2", "3/4", "1/2", "1 /2", "1/ 2", "0 1/2", "1 3/2", "1 1/0",
        "1/0", "10:30", "10:30:15", "10:30:15.25", "10:30 AM", "10:30 PM", "10:30AM", "10:30 am", "10:30 a", "10:30 P",
        "10 AM", "10 PM", "13:00 PM", "0:00 AM", "12:00 AM", "12:30 PM", "24:00", "24:00:01", "25:00", "100:00",
        "1:60", "1:59:60", "1:90", "0:0", ":30", "10:", "10:30:", "10::30", "1.5:00", "-1:00", "오전 10:30",
        "오후 10:30", "10:30 오후", "AM 10:30", "2024-01-15", "2024/01/15", "2024.01.15", "2024-1-5", "2024/1/5",
        "24-1-5", "24/1/5", "1-5-24", "1/5/24", "05-01-2024", "05/01/2024", "13-01-2024", "2024-13-01", "2024-01-32",
        "2024-02-29", "2023-02-29", "1900-02-29", "1899-12-31", "1900-01-00", "9999-12-31", "10000-01-01", "0-1-1",
        "00-01-01", "29-12-31", "30-01-01", "99-12-31", "100-1-1", "999-1-1", "1000-1-1", "1-1", "1-1-1", "12-25",
        "25-12", "2024-01", "2024/01", "01-2024", "1-2024", "13-2024", "Jan", "Jan 1", "1 Jan", "Jan 1 2024",
        "Jan 1, 2024", "1 Jan 2024", "1-Jan-2024", "1-Jan-24", "Jan-24", "January", "January 1", "1-January-2024",
        "Sept 1, 2024", "Sep 1, 2024", "Mon Jan 1 2024", "2024년", "2024년 1월", "2024년 1월 15일", "2024년1월15일",
        "1월 15일", "24년 1월 15일", "2024년 13월 1일", "2024-01-15 10:30", "2024-01-15 10:30:15", "2024-01-15T10:30",
        "2024-01-15 25:00", "10:30 2024-01-15", "2024-01-15  10:30", "2024-01-15 10:30 PM", "1/15", "15/1", "1/32",
        "2/29", "13/13",
    ] {
        let lit = s.replace('"', "\"\"");
        v.push(c(format!("\"{lit}\"*1")));
        v.push(c(format!("DATEVALUE(\"{lit}\")")));
        v.push(c(format!("TIMEVALUE(\"{lit}\")")));
        v.push(c(format!("VALUE(\"{lit}\")")));
    }
    v
}

/// 글 비교 순서 (같음·작음)
fn cmp() -> Vec<Case> {
    let mut v = Vec::new();
    let items = [
        "", " ", "!", "-", "_", "'", "~", "0", "1", "9", "10", "a", "A", "b", "B", "z", "Z", "ä", "Ä", "à", "á", "â", "ã",
        "å", "æ", "ç", "é", "è", "ë", "ñ", "ö", "ø", "ß", "ss", "ü", "ÿ", "œ", "Ω", "α", "я", "가", "나", "힣", "ㄱ", "ㅏ",
        "一", "丁", "あ", "ア", "ａ", "Ａ", "１", "co-op", "coop", "co op", "a-b", "ab", "a'b", "a1", "a10", "a2", "Ⅰ",
        "①", "€", "₩", "\\",
    ];
    for x in items {
        for y in ["a", "b", "ss", "가", "co-op", "A", "z", "1"] {
            let (lx, ly) = (x.replace('"', "\"\""), y.replace('"', "\"\""));
            v.push(c(format!("IF(\"{lx}\"=\"{ly}\",0,IF(\"{lx}\"<\"{ly}\",-1,1))")));
        }
    }
    v
}

/// 정렬되지 않은 자료의 근사 찾기 (이진 탐색인지 처음부터 훑는지)
fn search() -> Vec<Case> {
    let mut v = Vec::new();
    // Data!A1:A10 = 3 1 4 1 5 9 2 6 5 3 (정렬 안 됨)
    for x in ["0", "1", "2", "3", "3.5", "4", "5", "6", "7", "8", "9", "10"] {
        for mt in ["1", "-1"] {
            v.push(c(format!("MATCH({x},Data!A1:A10,{mt})")));
        }
        v.push(c(format!("VLOOKUP({x},Data!A1:B10,2,TRUE)")));
        v.push(c(format!("LOOKUP({x},Data!A1:A10,Data!B1:B10)")));
        v.push(c(format!("MATCH({x},{{9,6,5,5,4,3,3,2,1,1}},-1)")));
        v.push(c(format!("MATCH({x},{{1,3,2,5,4}},1)")));
        v.push(c(format!("MATCH({x},{{5,4,6,2,1}},-1)")));
        v.push(c(format!("HLOOKUP({x},{{3,1,4,1,5;\"a\",\"b\",\"c\",\"d\",\"e\"}},2,TRUE)")));
    }
    for x in ["\"a\"", "\"b\"", "\"c\"", "\"m\"", "\"z\"", "1", "TRUE"] {
        v.push(c(format!("MATCH({x},{{\"b\",1,\"d\",TRUE,\"a\"}},1)")));
        v.push(c(format!("MATCH({x},{{\"b\",1,\"d\",TRUE,\"a\"}},-1)")));
        v.push(c(format!("LOOKUP({x},{{1,\"a\",\"c\",TRUE,\"e\"}})")));
        v.push(c(format!("MATCH({x},Data!C1:C10,1)")));
    }
    for (x, nf) in [("35", "\"none\""), ("40", "\"none\""), ("1000", "\"none\""), ("5", "\"none\"")] {
        for mm in ["0", "-1", "1", "2"] {
            for sm in ["1", "-1", "2", "-2"] {
                v.push(c(format!("_xlfn.XLOOKUP({x},Data!F1:F10,Data!G1:G10,{nf},{mm},{sm})")));
            }
        }
    }
    v.push(c("_xlfn.XLOOKUP(\"a*\",Data!D1:D10,Data!A1:A10,\"none\",2)"));
    v.push(c("_xlfn.XLOOKUP(\"apple\",Data!D1:D10,Data!A1:A10,\"none\",0,-1)"));
    v.push(c("_xlfn.XLOOKUP(3,Data!A1:A10,Data!B1:B10,\"none\",0,-1)"));
    v.push(c("_xlfn.XLOOKUP(3,Data!A1:A10,Data!B1:B10)"));
    v.push(c("_xlfn.XLOOKUP(99,Data!A1:A10,Data!B1:B10)"));
    v.push(c("_xlfn.XLOOKUP(99,Data!A1:A10,Data!D1:D10)"));
    v.push(c("_xlfn.XLOOKUP(99,Data!A1:A10,Data!J1:J10)"));
    v.push(c("_xlfn.XLOOKUP(99,Data!A1:A10,Data!B1:B10,)"));
    v
}

/// 분산 계산의 취소 오차 (한 번 훑는 공식인지 두 번 훑는 공식인지)
fn variance() -> Vec<Case> {
    let mut v = Vec::new();
    let sets = [
        "1E+8+1,1E+8+2", "1E+8+1,1E+8+2,1E+8+3", "1E+8+1,1E+8+2,1E+8+4", "1E+9+0.1,1E+9+0.2,1E+9+0.3",
        "1E+7+0.1,1E+7+0.2,1E+7+0.3", "1E+6+0.1,1E+6+0.2,1E+6+0.3", "0.1,0.2,0.3", "1,2,3,4", "1E+15,1E+15+2",
        "1E+15,1E+15+2,1E+15+4", "123456789.123,123456789.456,123456789.789", "1E+10,1E+10+1,1E+10+2,1E+10+3",
        "3,3,3", "1E+8,1E+8,1E+8+1", "2.5,-1.25,0,7,10.5", "100000.1,100000.2,100000.4,100000.8",
    ];
    for name in [
        "VAR", "VARP", "STDEV", "STDEVP", "DEVSQ", "_xlfn.VAR.S", "_xlfn.VAR.P", "_xlfn.STDEV.S", "_xlfn.STDEV.P",
        "VARA", "VARPA", "STDEVA", "STDEVPA", "AVEDEV", "SKEW", "KURT", "_xlfn.SKEW.P", "AVERAGE", "GEOMEAN",
        "HARMEAN",
    ] {
        calls(&mut v, name, &sets);
    }
    for s in sets {
        v.push(c(format!("CORREL({{{s}}},{{{s}}})")));
        v.push(c(format!("_xlfn.COVARIANCE.S({{{s}}},{{{s}}})")));
        v.push(c(format!("COVAR({{{s}}},{{{s}}})")));
        v.push(c(format!("SLOPE({{{s}}},{{{s}}})")));
        v.push(c(format!("SUMSQ({s})")));
    }
    v
}

/// 정규 분포 누적 함수의 촘촘한 격자 (근사식 찾기)
fn normal() -> Vec<Case> {
    let mut v = Vec::new();
    let mut x = -42.0f64;
    while x <= 42.0001 {
        let xs = format!("{}", (x * 100.0).round() / 100.0);
        v.push(c(format!("NORMSDIST({xs})")));
        x += if x.abs() < 8.0 { 0.25 } else { 1.0 };
    }
    for xs in ["1E-9", "1E-5", "0.001", "0.01", "0.1", "0.2", "0.3", "0.7", "1.1", "1.5", "2.2", "2.9", "3.1", "4.4", "6.6", "7.9", "8.1"] {
        v.push(c(format!("NORMSDIST({xs})")));
        v.push(c(format!("NORMSDIST(-{xs})")));
        v.push(c(format!("_xlfn.NORM.S.DIST({xs},TRUE)")));
        v.push(c(format!("_xlfn.NORM.S.DIST(-{xs},TRUE)")));
        v.push(c(format!("NORMDIST({xs},0,1,TRUE)")));
        v.push(c(format!("ERF({xs})")));
        v.push(c(format!("ERFC({xs})")));
        v.push(c(format!("_xlfn.ERF.PRECISE({xs})")));
        v.push(c(format!("_xlfn.GAUSS({xs})")));
        v.push(c(format!("_xlfn.PHI({xs})")));
    }
    for p in ["0.001", "0.01", "0.025", "0.05", "0.1", "0.2", "0.3", "0.4", "0.5"] {
        v.push(c(format!("CONFIDENCE({p},1,1)")));
        v.push(c(format!("_xlfn.CONFIDENCE.T({p},1,10)")));
        v.push(c(format!("ZTEST({{1,2,3,4}},{p})")));
    }
    v
}

/// 단위 환산표: CONVERT 는 단위마다 기본 단위와, CONVERTK 는 분류마다 단위 번호 쌍
fn units() -> Vec<Case> {
    let mut v = Vec::new();
    let units = [
        ("g", "g"), ("sg", "g"), ("lbm", "g"), ("u", "g"), ("ozm", "g"), ("grain", "g"), ("cwt", "g"), ("shweight", "g"),
        ("uk_cwt", "g"), ("lcwt", "g"), ("stone", "g"), ("ton", "g"), ("uk_ton", "g"), ("LTON", "g"), ("brton", "g"),
        ("m", "m"), ("mi", "m"), ("Nmi", "m"), ("in", "m"), ("ft", "m"), ("yd", "m"), ("ang", "m"), ("Pica", "m"),
        ("pica", "m"), ("Picapt", "m"), ("ell", "m"), ("ly", "m"), ("parsec", "m"), ("pc", "m"), ("survey_mi", "m"),
        ("yr", "sec"), ("day", "sec"), ("d", "sec"), ("hr", "sec"), ("mn", "sec"), ("min", "sec"), ("sec", "sec"),
        ("s", "sec"), ("Pa", "Pa"), ("p", "Pa"), ("atm", "Pa"), ("at", "Pa"), ("mmHg", "Pa"), ("psi", "Pa"),
        ("Torr", "Pa"), ("N", "N"), ("dyn", "N"), ("dy", "N"), ("lbf", "N"), ("pond", "N"), ("J", "J"), ("e", "J"),
        ("c", "J"), ("cal", "J"), ("eV", "J"), ("ev", "J"), ("HPh", "J"), ("hh", "J"), ("Wh", "J"), ("wh", "J"),
        ("flb", "J"), ("BTU", "J"), ("btu", "J"), ("HP", "W"), ("h", "W"), ("W", "W"), ("w", "W"), ("PS", "W"),
        ("T", "T"), ("ga", "T"), ("C", "C"), ("cel", "C"), ("F", "C"), ("fah", "C"), ("K", "C"), ("kel", "C"),
        ("Rank", "C"), ("Reau", "C"), ("tsp", "l"), ("tspm", "l"), ("tbs", "l"), ("oz", "l"), ("cup", "l"),
        ("pt", "l"), ("us_pt", "l"), ("uk_pt", "l"), ("qt", "l"), ("uk_qt", "l"), ("gal", "l"), ("uk_gal", "l"),
        ("l", "l"), ("L", "l"), ("lt", "l"), ("m3", "l"), ("m^3", "l"), ("ft3", "l"), ("ft^3", "l"), ("in3", "l"),
        ("yd3", "l"), ("ang3", "l"), ("barrel", "l"), ("bushel", "l"), ("GRT", "l"), ("MTON", "l"), ("m2", "m2"),
        ("m^2", "m2"), ("ft2", "m2"), ("in2", "m2"), ("yd2", "m2"), ("mi2", "m2"), ("ha", "m2"), ("ar", "m2"),
        ("uk_acre", "m2"), ("us_acre", "m2"), ("Morgen", "m2"), ("bit", "bit"), ("byte", "bit"), ("m/s", "m/s"),
        ("m/h", "m/s"), ("m/hr", "m/s"), ("mph", "m/s"), ("kn", "m/s"), ("admkn", "m/s"), ("km", "m"), ("cm", "m"),
        ("mm", "m"), ("um", "m"), ("nm", "m"), ("km/h", "m/s"), ("kg", "g"), ("mg", "g"), ("kJ", "J"), ("kW", "W"),
        ("mW", "W"), ("kPa", "Pa"), ("hPa", "Pa"), ("ml", "l"), ("dl", "l"), ("cl", "l"), ("km2", "m2"), ("cm2", "m2"),
        ("Gbyte", "bit"), ("kbyte", "bit"), ("kibyte", "bit"), ("Mibyte", "bit"), ("mK", "K"), ("kK", "K"),
        ("dam", "m"), ("hm", "m"), ("Mm", "m"), ("Em", "m"), ("ym", "m"), ("Ym", "m"),
    ];
    for (u, base) in units {
        v.push(c(format!("CONVERT(1,\"{u}\",\"{base}\")")));
        v.push(c(format!("CONVERT(1,\"{base}\",\"{u}\")")));
    }
    for t in ["0", "100", "-40"] {
        for (a, b) in [("C", "F"), ("F", "C"), ("C", "K"), ("K", "C"), ("F", "K"), ("K", "F"), ("cel", "fah"), ("C", "Rank"), ("C", "Reau")] {
            v.push(c(format!("CONVERT({t},\"{a}\",\"{b}\")")));
        }
    }
    for g in 1..=25 {
        for i in 0..=13 {
            v.push(c(format!("CONVERTK(1,{g},{i},1)")));
            v.push(c(format!("CONVERTK(1,{g},1,{i})")));
        }
        v.push(c(format!("CONVERTK(0,{g},2,1)")));
        v.push(c(format!("CONVERTK(100,{g},2,1)")));
        v.push(c(format!("CONVERTK(100,{g},1,2)")));
    }
    for f in [
        "CONVERTK(1.5,1,3,8)", "CONVERTK(-2,1,3,8)", "CONVERTK(\"2\",1,3,8)", "CONVERTK(TRUE,1,3,8)", "CONVERTK(1,1.9,3,8)",
        "CONVERTK(1,1,3.9,8.9)", "CONVERTK(1,\"1\",3,8)", "CONVERTK(1,1,\"3\",8)", "CONVERTK(1,0,1,1)", "CONVERTK(1,-1,1,1)",
        "CONVERTK(Data!C4,1,3,8)", "CONVERTK(1,1,3,8)*1000",
    ] {
        v.push(c(f));
    }
    v
}

// ---------------------------------------------------------------------------
// 세 번째 묶음
// ---------------------------------------------------------------------------

/// 단위 환산표 전체: 분류마다 모든 단위 쌍 (기준 단위와 곱하는 순서를 찾는다)
fn units2() -> Vec<Case> {
    let mut v = Vec::new();
    let cats: &[&[&str]] = &[
        &["g", "sg", "lbm", "u", "ozm"],
        &["m", "mi", "Nmi", "in", "ft", "yd", "ang", "Pica"],
        &["yr", "day", "hr", "mn", "sec"],
        &["Pa", "atm", "mmHg"],
        &["N", "dyn", "lbf"],
        &["J", "e", "c", "cal", "eV", "HPh", "Wh", "flb", "BTU"],
        &["HP", "W"],
        &["T", "ga"],
        &["tsp", "tbs", "oz", "cup", "pt", "uk_pt", "qt", "gal", "l"],
    ];
    for units in cats {
        for a in units.iter() {
            for b in units.iter() {
                v.push(c(format!("CONVERT(1,\"{a}\",\"{b}\")")));
            }
        }
    }
    for (x, a, b) in [
        ("2.5", "m", "ft"), ("7", "lbm", "kg"), ("-3", "in", "cm"), ("1E+10", "yd", "m"), ("0.1", "gal", "l"),
        ("3", "kg", "ozm"), ("2", "kmi", "m"), ("2", "Mm", "km"), ("5", "mm", "cm"), ("5", "dm", "m"), ("5", "em", "m"),
        ("5", "pm", "m"), ("5", "fm", "m"), ("5", "am", "m"), ("5", "Pm", "m"), ("5", "Tm", "m"), ("5", "Gm", "m"),
        ("1", "kft", "m"), ("1", "mft", "m"), ("1", "kl", "tsp"), ("1", "ml", "tsp"), ("1", "kJ", "BTU"),
        ("1", "kcal", "J"), ("1", "keV", "J"), ("1", "kHP", "W"), ("1", "mHP", "W"), ("1", "kN", "lbf"),
        ("1", "katm", "Pa"), ("1", "kyr", "day"), ("1", "ksec", "hr"), ("1", "mT", "ga"), ("1", "kga", "T"),
    ] {
        v.push(c(format!("CONVERT({x},\"{a}\",\"{b}\")")));
    }
    for x in ["1", "0", "100", "-40", "37", "68", "1.5", "-273.15", "1E+10"] {
        for (a, b) in [("C", "F"), ("F", "C"), ("C", "K"), ("K", "C"), ("F", "K"), ("K", "F"), ("C", "C"), ("F", "F"), ("K", "K"), ("mK", "C"), ("kC", "F")] {
            v.push(c(format!("CONVERT({x},\"{a}\",\"{b}\")")));
        }
    }
    for g in 1..=24 {
        for i in 1..=12 {
            for j in 1..=12 {
                v.push(c(format!("CONVERTK(1,{g},{i},{j})")));
            }
        }
    }
    for x in ["0", "37", "100", "-40", "1.5"] {
        for (i, j) in [(1, 2), (2, 1), (1, 1), (2, 2)] {
            v.push(c(format!("CONVERTK({x},5,{i},{j})")));
        }
    }
    v
}

/// 숫자 → 글의 경계, 복소수 글 꼴, 상수 자르기, 그 밖의 규칙 확인
fn misc3() -> Vec<Case> {
    let mut v = Vec::new();
    for x in [
        "0.0123456789012345", "0.00123456789012345", "0.000123456789012345", "0.0012345678901234",
        "0.000123456789012", "0.0000123456789", "1.5E-15", "1.5E-16", "1E-16", "1E-17", "1E-18", "1E-19",
        "1.25E-14", "1.25E-15", "1.25E-16", "3.5E-5", "3.51E-5", "1.234E-12", "1.2345E-12", "1.23456E-12",
        "0.1234567890123", "0.01234567890123", "2E+19", "5E+19", "9E+19", "9.9E+19", "1.8446744073709E+19",
        "1.8446744073710E+19", "1.84467440737096E+19", "-1.84467440737095E+19", "1.2345678901234567E+19",
        "123456789012345.6", "12345678901234.56", "1234567890123456.7", "-123456789012345.6",
    ] {
        v.push(c(format!("({x})&\"\"")));
    }
    for s in [
        "1234567890123456", "12345678901234567890", "0.1234567890123456", "1.234567890123456789", "9007199254740993",
        "123456789012345678", "99999999999999999", "0.99999999999999999",
    ] {
        v.push(c(format!("VALUE(\"{s}\")")));
        v.push(c(format!("\"{s}\"*1")));
        v.push(c(format!("{s}*1")));
        v.push(c(format!("{s}=VALUE(\"{s}\")")));
    }
    for z in [
        "\"1E-5i\"", "\"0.00001+i\"", "\"0.0001+0.001i\"", "\"1E-4\"", "\"123456789012345678\"", "\"1E+20i\"",
        "\"1.5E+15+2i\"", "\"0.1+0.2i\"", "\"1/3\"", "\"3E-7-2E-8i\"",
    ] {
        v.push(c(format!("IMSUM({z},\"0\")")));
    }
    for f in [
        "IMPRODUCT(\"0.001i\",\"0.01\")", "IMDIV(\"1\",\"3\")", "IMDIV(\"1\",\"3i\")", "IMSUM(\"1E-15\",\"1\")",
        "COMPLEX(0.00001,1)", "COMPLEX(1E+16,1)", "COMPLEX(1E+15,1)", "COMPLEX(123456789012345,1)",
        "COMPLEX(1234567890123456,1)", "COMPLEX(1/3,2/3)", "COMPLEX(0.0001,0.00001)",
        "DATE(2024,1,32767)", "DATE(2024,1,32768)", "DATE(2024,1,65535)", "DATE(2024,1,65536)", "DATE(2024,1,-32767)",
        "DATE(2024,1,-32768)", "DATE(2024,32767,1)", "DATE(2024,32768,1)", "DATE(2024,-32767,1)", "DATE(1900,1,40000)",
        "DATE(1900,1,2958465)", "DATE(2024,1,100000)", "DATE(2024,1,50000)",
        "_xlfn.NETWORKDAYS.INTL(45292,45310,1,{45292})", "_xlfn.NETWORKDAYS.INTL(45292,45310,1,45292)",
        "_xlfn.NETWORKDAYS.INTL(45292,45310,1,Data!AF1)", "_xlfn.NETWORKDAYS.INTL(45292,45310,,Data!AF1:AF2)",
        "_xlfn.NETWORKDAYS.INTL(45292,45310,99)", "_xlfn.NETWORKDAYS.INTL(45292,45310,0)",
        "_xlfn.NETWORKDAYS.INTL(45292,45310,-1)", "_xlfn.NETWORKDAYS.INTL(45292,45310,18)",
        "_xlfn.NETWORKDAYS.INTL(45292,45310,8)", "_xlfn.NETWORKDAYS.INTL(45292,45310,\"1\")",
        "_xlfn.WORKDAY.INTL(45292,10,8)", "_xlfn.WORKDAY.INTL(45292,10,18)", "_xlfn.WORKDAY.INTL(45292,10,0)",
        "_xlfn.WORKDAY.INTL(45292,10,1,{45292})", "_xlfn.WORKDAY.INTL(45292,10,1,45293)",
        "_xlfn.WORKDAY.INTL(45292,10,\"011\")", "_xlfn.WORKDAY.INTL(45292,10,,45293)",
        "_xlfn.XOR(Data!C1:C3)", "_xlfn.XOR(Data!C4:C6)", "_xlfn.XOR(Data!C7:C10)", "_xlfn.XOR(Data!C2)",
        "_xlfn.XOR(Data!C5)", "_xlfn.XOR(Data!C1,Data!C2)", "_xlfn.XOR(Data!C5:C6)", "_xlfn.XOR(Data!D1:D2)",
        "_xlfn.XOR(Data!C2:C3)", "_xlfn.XOR(1,\"a\")", "_xlfn.XOR(TRUE,Data!C4)", "_xlfn.XOR({1,0,\"a\"})",
        "AND(Data!C2:C3)", "OR(Data!C2:C3)", "AND(Data!C5)", "OR(Data!C5:C6)",
        "_xlfn.BITAND(2^52,1)", "_xlfn.BITAND(2^53,1)", "_xlfn.BITAND(2^53+2,2)", "_xlfn.BITAND(1E+20,1)",
        "_xlfn.BITAND(-0.5,1)", "_xlfn.BITAND(-1,1)", "_xlfn.BITAND(5.9,7)", "_xlfn.BITOR(2^48,1)",
        "_xlfn.BITOR(2^60,1)", "_xlfn.BITXOR(2^49,2^48)", "_xlfn.BITLSHIFT(1,60)", "_xlfn.BITLSHIFT(2^47,2)",
        "_xlfn.BITRSHIFT(2^52,3)", "_xlfn.BITLSHIFT(1.9,1)", "_xlfn.BITRSHIFT(5,-1)", "_xlfn.BITLSHIFT(5,-1)",
        "_xlfn.BITLSHIFT(1,63)", "_xlfn.BITLSHIFT(1,64)",
        "EOMONTH(60,0)", "EOMONTH(59,0)", "EOMONTH(31,1)", "EOMONTH(1,1)", "EDATE(60,12)", "EDATE(59,1)",
        "EOMONTH(45000,-1500)", "EDATE(31,13)", "EOMONTH(0,0)", "EDATE(0,0)",
        "_xlfn.ISOWEEKNUM(2)", "_xlfn.ISOWEEKNUM(7)", "_xlfn.ISOWEEKNUM(44197)", "_xlfn.ISOWEEKNUM(44562)", "_xlfn.ISOWEEKNUM(43465)",
        "_xlfn.ISOWEEKNUM(43830)", "_xlfn.ISOWEEKNUM(44926)", "_xlfn.ISOWEEKNUM(45291)", "WEEKNUM(43465,21)", "WEEKNUM(44197,21)",
        "TEXT(1234.567,\"#,##0.00_);(#,##0.00)\")", "TEXT(-1234.567,\"#,##0.00_);(#,##0.00)\")",
        "TEXT(1234.567,\"0.00_)\")", "TEXT(1234.567,\"_(0.00\")", "TEXT(5,\"0_x\")", "TEXT(\"abc\",\"@\")",
        "TEXT(\"abc\",\"@@\")", "TEXT(\"abc\",\"x@x\")", "TEXT(\"abc\",\"0\")", "TEXT(5,\"@\")", "TEXT(5,\"G/표준\")",
        "TEXT(1234.5,\"G/표준\")", "TEXT(0.5,\"G/표준\")", "TEXT(5,\"General\")", "TEXT(5,\"Gen\")",
        "TEXT(45000,\"e\")", "TEXT(45000,\"ee\")", "TEXT(45000,\"g\")", "TEXT(5,\"\\$0\")", "TEXT(5,\"\"\"$\"\"0\")",
        "TEXT(5,\"[Blue]0\")", "TEXT(5,\"[>3]0;0\")", "TEXT(Data!C4,\"@\")", "TEXT(Data!C4,\"0\")", "TEXT(\"\",\"0\")",
        "TEXT(5,\"0;-0;\"\"z\"\"\")", "TEXT(0,\"0;-0;\"\"z\"\"\")", "TEXT(1E+20,\"0\")", "TEXT(123456789,\"#,##0\")",
        "TEXT(0.123,\"0.0%\")", "TEXT(45000.5,\"hh:mm\")", "TEXT(1.5,\"[mm]\")", "TEXT(2.5,\"0\")", "TEXT(3.5,\"0\")",
        "TEXT(-0.5,\"0\")", "TEXT(0.125,\"0.00\")", "TEXT(1.005,\"0.00\")", "TEXT(1234,\"0000000\")",
        "TEXT(45000,\"yyyy\\\"년\\\"\")", "TEXT(45000,\"aaa\")", "TEXT(45000,\"ddd\")", "TEXT(45000,\"mmmm\")",
        "TEXT(45000,\"mmmmm\")", "TEXT(0.75,\"AM/PM h\")", "TEXT(0.75,\"오전/오후 h\")", "TEXT(5,\"0.0E+0\")",
        "TEXT(12345,\"0.00E+00\")", "TEXT(5,\"# ?/?\")", "TEXT(5.25,\"# ?/?\")", "TEXT(0.333,\"?/?\")",
    ] {
        v.push(c(f));
    }
    v
}

/// (문서 이름, 경우, 1904 날짜 체계)
pub fn catalog() -> Vec<(&'static str, Vec<Case>, bool)> {
    vec![
        ("ops", ops(), false),
        ("math", math(), false),
        ("stat", stat(), false),
        ("dist", dist(), false),
        ("text", text(), false),
        ("logical", logical(), false),
        ("lookup", lookup(), false),
        ("date", date(), false),
        ("date1904", date1904(), true),
        ("info", info(), false),
        ("financial", financial(), false),
        ("engineering", engineering(), false),
        ("database", database(), false),
        ("newer", newer(), false),
        ("arrays", arrays(), false),
        ("conv", conv(), false),
        ("cmp", cmp(), false),
        ("search", search(), false),
        ("variance", variance(), false),
        ("normal", normal(), false),
        ("units", units(), false),
        ("units2", units2(), false),
        ("misc3", misc3(), false),
    ]
}
