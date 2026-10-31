// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 계산 값: 숫자·글·논리·오류·빈 값·배열·참조와 그 사이의 변환 규칙.
//!
//! 변환 규칙은 스프레드시트 공통 규칙(ECMA-376 Part 1 18.17.2)을 따르고, 한셀과 다른 점은
//! 함수 대조 실험(`tests/calc_oracle.rs`)으로 맞췄다.

use std::cmp::Ordering;
use std::fmt;
use std::rc::Rc;

use super::fm::Fm;
use crate::model::CellValue;

/// 오류 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ErrorKind {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    GettingData,
    Spill,
    Calc,
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Null => "#NULL!",
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
            ErrorKind::GettingData => "#GETTING_DATA",
            ErrorKind::Spill => "#SPILL!",
            ErrorKind::Calc => "#CALC!",
        }
    }

    pub fn parse(s: &str) -> Option<ErrorKind> {
        let u = s.trim().to_ascii_uppercase();
        Some(match u.as_str() {
            "#NULL!" => ErrorKind::Null,
            "#DIV/0!" => ErrorKind::Div0,
            "#VALUE!" => ErrorKind::Value,
            "#REF!" => ErrorKind::Ref,
            "#NAME?" => ErrorKind::Name,
            "#NUM!" => ErrorKind::Num,
            "#N/A" => ErrorKind::NA,
            "#GETTING_DATA" => ErrorKind::GettingData,
            "#SPILL!" => ErrorKind::Spill,
            "#CALC!" => ErrorKind::Calc,
            _ => return None,
        })
    }

    /// ERROR.TYPE 번호
    pub fn code(self) -> u8 {
        match self {
            ErrorKind::Null => 1,
            ErrorKind::Div0 => 2,
            ErrorKind::Value => 3,
            ErrorKind::Ref => 4,
            ErrorKind::Name => 5,
            ErrorKind::Num => 6,
            ErrorKind::NA => 7,
            ErrorKind::GettingData => 8,
            ErrorKind::Spill => 9,
            ErrorKind::Calc => 14,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 시트 수 제한 (한셀 2024)
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// 직사각형 영역 하나. 좌표는 0부터, 양 끝 포함.
/// `book` 0 은 이 통합 문서, n 은 n번째 외부 연결(1부터, 시트 번호는 그 연결의 `sheet_names` 안 번호).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Area {
    pub book: u32,
    pub sheet: u32,
    /// 3차원 참조(`Sheet1:Sheet3!A1`)의 마지막 시트. 한 시트면 `sheet` 와 같다.
    pub sheet_last: u32,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Area {
    pub fn cell(sheet: u32, row: u32, col: u32) -> Area {
        Area {
            book: 0,
            sheet,
            sheet_last: sheet,
            r0: row,
            c0: col,
            r1: row,
            c1: col,
        }
    }
    pub fn rows(&self) -> u32 {
        self.r1 - self.r0 + 1
    }
    pub fn cols(&self) -> u32 {
        self.c1 - self.c0 + 1
    }
    pub fn is_cell(&self) -> bool {
        self.r0 == self.r1 && self.c0 == self.c1 && self.sheet == self.sheet_last
    }
    pub fn is_3d(&self) -> bool {
        self.sheet != self.sheet_last
    }
    pub fn contains(&self, sheet: u32, row: u32, col: u32) -> bool {
        self.book == 0
            && (self.sheet..=self.sheet_last).contains(&sheet)
            && (self.r0..=self.r1).contains(&row)
            && (self.c0..=self.c1).contains(&col)
    }
    pub fn same_sheets(&self, o: &Area) -> bool {
        self.book == o.book && self.sheet == o.sheet && self.sheet_last == o.sheet_last
    }
    /// 겹치는 부분 (같은 시트일 때)
    pub fn intersect(&self, o: &Area) -> Option<Area> {
        if !self.same_sheets(o) {
            return None;
        }
        let (r0, r1) = (self.r0.max(o.r0), self.r1.min(o.r1));
        let (c0, c1) = (self.c0.max(o.c0), self.c1.min(o.c1));
        (r0 <= r1 && c0 <= c1).then_some(Area {
            r0,
            r1,
            c0,
            c1,
            ..*self
        })
    }
    /// 두 영역을 감싸는 영역 (범위 연산자 `:`)
    pub fn bounding(&self, o: &Area) -> Option<Area> {
        if !self.same_sheets(o) {
            return None;
        }
        Some(Area {
            r0: self.r0.min(o.r0),
            r1: self.r1.max(o.r1),
            c0: self.c0.min(o.c0),
            c1: self.c1.max(o.c1),
            ..*self
        })
    }
}

/// 참조 값: 영역 하나 이상 (`(A1:B2,D4)` 같은 합집합은 여럿)
pub type Reference = Rc<Vec<Area>>;

/// 2차원 배열 (행 우선). 원소는 스칼라만 담는다.
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Value>,
}

impl Array {
    pub fn new(rows: usize, cols: usize, data: Vec<Value>) -> Array {
        debug_assert_eq!(rows * cols, data.len());
        Array { rows, cols, data }
    }
    pub fn filled(rows: usize, cols: usize, v: Value) -> Array {
        Array {
            rows,
            cols,
            data: vec![v; rows * cols],
        }
    }
    pub fn get(&self, r: usize, c: usize) -> &Value {
        &self.data[r * self.cols + c]
    }
    /// 브로드캐스트 규칙: 한 행·한 열짜리는 늘려 쓰고, 범위 밖은 #N/A
    pub fn get_broadcast(&self, r: usize, c: usize) -> Value {
        let rr = if self.rows == 1 { 0 } else { r };
        let cc = if self.cols == 1 { 0 } else { c };
        if rr < self.rows && cc < self.cols {
            self.get(rr, cc).clone()
        } else {
            Value::Error(ErrorKind::NA)
        }
    }
    pub fn transpose(&self) -> Array {
        let mut data = Vec::with_capacity(self.data.len());
        for c in 0..self.cols {
            for r in 0..self.rows {
                data.push(self.get(r, c).clone());
            }
        }
        Array::new(self.cols, self.rows, data)
    }
}

/// 계산 값.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// 빈 칸, 생략한 인수
    #[default]
    Empty,
    Number(f64),
    Text(Rc<str>),
    Bool(bool),
    Error(ErrorKind),
    Array(Rc<Array>),
    Ref(Reference),
}

impl From<f64> for Value {
    fn from(v: f64) -> Value {
        Value::Number(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}
impl From<ErrorKind> for Value {
    fn from(e: ErrorKind) -> Value {
        Value::Error(e)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Text(Rc::from(s))
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Text(Rc::from(s))
    }
}
impl From<Array> for Value {
    fn from(a: Array) -> Value {
        Value::Array(Rc::new(a))
    }
}

impl<T: Into<Value>> From<Result<T, ErrorKind>> for Value {
    fn from(r: Result<T, ErrorKind>) -> Value {
        match r {
            Ok(v) => v.into(),
            Err(e) => Value::Error(e),
        }
    }
}

/// 숫자 결과: NaN·무한대는 #NUM!
pub fn num(v: f64) -> Value {
    if v.is_finite() {
        Value::Number(if v == 0.0 { 0.0 } else { v })
    } else {
        Value::Error(ErrorKind::Num)
    }
}

/// 숫자 결과 (Result 판)
pub fn num_r(v: Result<f64, ErrorKind>) -> Value {
    match v {
        Ok(v) => num(v),
        Err(e) => Value::Error(e),
    }
}

impl Value {
    pub fn text(s: impl Into<Rc<str>>) -> Value {
        Value::Text(s.into())
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    pub fn error(&self) -> Option<ErrorKind> {
        match self {
            Value::Error(e) => Some(*e),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    /// 모델의 셀 값 → 계산 값. `t="d"` 날짜는 일련 번호로 바꾼다.
    pub fn from_cell(v: &CellValue, date1904: bool) -> Value {
        match v {
            CellValue::Empty => Value::Empty,
            CellValue::Number(n) => Value::Number(*n),
            CellValue::Text(s) => Value::Text(Rc::from(s.as_str())),
            CellValue::Bool(b) => Value::Bool(*b),
            CellValue::Error(e) => Value::Error(ErrorKind::parse(e).unwrap_or(ErrorKind::Value)),
            CellValue::Date(d) => crate::calc::date::parse_iso(d, date1904)
                .map(Value::Number)
                .unwrap_or_else(|| Value::Text(Rc::from(d.as_str()))),
        }
    }

    /// 계산 값 → 모델의 셀 값 (배열은 왼쪽 위, 참조는 #VALUE!)
    pub fn to_cell(&self) -> CellValue {
        match self {
            Value::Empty => CellValue::Number(0.0),
            Value::Number(n) => CellValue::Number(*n),
            Value::Text(s) => CellValue::Text(lone_to_fffd(s)),
            Value::Bool(b) => CellValue::Bool(*b),
            Value::Error(e) => CellValue::Error(e.as_str().into()),
            Value::Array(a) => a
                .data
                .first()
                .map(|v| v.to_cell())
                .unwrap_or(CellValue::Error("#VALUE!".into())),
            Value::Ref(_) => CellValue::Error("#VALUE!".into()),
        }
    }

    /// 스칼라 값의 형 이름 (TYPE 함수: 1 숫자, 2 글, 4 논리, 16 오류, 64 배열)
    pub fn type_code(&self) -> u32 {
        match self {
            Value::Empty | Value::Number(_) => 1,
            Value::Text(_) => 2,
            Value::Bool(_) => 4,
            Value::Error(_) => 16,
            Value::Array(_) => 64,
            Value::Ref(_) => 1,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Empty => Ok(()),
            Value::Number(n) => f.write_str(&number_to_text(*n)),
            Value::Text(s) => f.write_str(s),
            Value::Bool(b) => f.write_str(if *b { "TRUE" } else { "FALSE" }),
            Value::Error(e) => f.write_str(e.as_str()),
            Value::Array(a) => write!(f, "{{배열 {}×{}}}", a.rows, a.cols),
            Value::Ref(r) => write!(f, "{{참조 {}}}", r.len()),
        }
    }
}

// ---------------------------------------------------------------------------
// 숫자 ↔ 글
// ---------------------------------------------------------------------------

/// 숫자를 글로 (연결 연산자·글 함수·복소수 글). 한셀의 규칙 (함수 대조 실험 conv·misc3 묶음):
/// - 유효숫자 15자리. 1E+15 이상을 그대로 쓸 때는 16자리로 반올림한 뒤 15자리 뒤를 버린다
///   (2^60 = 1152921504606846976 → "1152921504606840000", 12345678901234499584 → "12345678901234500000"),
///   그보다 작으면 15자리로 반올림 (2/3 → "0.666666666666667"). 복소수 글은 늘 15자리 반올림
///   ([`number_to_text_rounded`]).
/// - 부호를 뺀 그대로의 꼴이 20자 이내면 그대로 쓴다: 정수 자리 20자리까지 (`99000000000000000000`),
///   1 보다 작으면 소수 자리 18자리까지 (`0.000000000000000001`, `0.000123456789012345`).
///   넘으면 지수 꼴 (`1E+20`, `1E-19`, `3.33333333333333E-6`) — 지수 자리는 채우지 않는다.
/// - 정수 자릿수는 log10 으로 재는 듯하다: 9.99999999999999E+19 는 log10 이 20.0 으로 반올림되어 지수 꼴.
pub fn number_to_text(v: f64) -> String {
    format_general(v, false)
}

/// [`number_to_text`] 과 같되 큰 수도 15자리로 반올림한다 (복소수 글: IMSUM("123456789012345678","0") →
/// "123456789012345000", 대조 실험 misc3).
pub fn number_to_text_rounded(v: f64) -> String {
    format_general(v, true)
}

fn format_general(v: f64, round_large: bool) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    let a = v.abs();
    let neg = if v < 0.0 { "-" } else { "" };
    // 15자리 반올림 (작은 수·지수 꼴), 그리고 큰 수를 그대로 쓸 때의 16자리 반올림 뒤 15자리 버림
    let split = |s: String, keep: usize| -> (String, i32) {
        let (mant, e) = s.split_once('e').unwrap_or((&s, "0"));
        let d = mant.replace('.', "");
        let d = d[..keep.min(d.len())].trim_end_matches('0').to_string();
        (
            if d.is_empty() { "0".to_string() } else { d },
            e.parse().unwrap_or(0),
        )
    };
    let (r_digits, r_exp) = split(format!("{a:.14e}"), 15);
    let (digits, exp) = if a >= 1e15 && !round_large {
        // 큰 수: 자릿수 n = ⌊log10 a⌋ + 1 (부동소수점 log10 이라 99999999999999904 는 17 로 반올림되어 한 자리 크게 센다),
        // 앞 15자리 = ⌊v / 10^(n−15)⌋ (부호 있는 값의 내림 — 음수는 크기가 올라간다), 나머지는 0. 두 부호와
        // 99999999999999999 → 99999999999999000, (-(2^50)) → -1125899906842630 이 모두 이 식이다 (대조 실험 conv5·fp2)
        let n = a.mlog10().floor() as i32 + 1;
        let shift = 10f64.mpowi(n - 15);
        let q = (v / shift).floor().abs();
        let qs = format!("{q:.0}");
        let exp = qs.len() as i32 - 1 + (n - 15);
        let d = qs.trim_end_matches('0').to_string();
        (if d.is_empty() { "0".to_string() } else { d }, exp)
    } else {
        (r_digits.clone(), r_exp)
    };
    let k = digits.len() as i32;
    let positional = if exp >= 0 {
        exp.max(a.mlog10().floor() as i32) < 20
    } else {
        k - 1 - exp <= 18
    };
    // 지수 꼴은 15자리 반올림 (2^69 → 5.90295810358706E+20)
    let (digits, exp) = if positional {
        (digits, exp)
    } else {
        (r_digits, r_exp)
    };
    let k = digits.len() as i32;
    if positional {
        if exp >= 0 {
            let int_len = exp + 1;
            if k <= int_len {
                format!("{neg}{digits}{}", "0".repeat((int_len - k) as usize))
            } else {
                let (a, b) = digits.split_at(int_len as usize);
                format!("{neg}{a}.{b}")
            }
        } else {
            format!("{neg}0.{}{digits}", "0".repeat((-exp - 1) as usize))
        }
    } else {
        let (a, b) = digits.split_at(1);
        let sign = if exp < 0 { '-' } else { '+' };
        if b.is_empty() {
            format!("{neg}{a}E{sign}{}", exp.abs())
        } else {
            format!("{neg}{a}.{b}E{sign}{}", exp.abs())
        }
    }
}

/// 유효숫자 15자리로 반올림 (스프레드시트가 저장·비교에 쓰는 정밀도)
pub fn round15(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    format!("{v:.14e}").parse().unwrap_or(v)
}

/// 십진 숫자 글(`123.45`, `1.5e-3`)을 유효숫자 15자리까지만 남기고(뒤 자리는 버림) 읽는다.
/// 한셀은 수식 안 숫자 상수와 글→숫자 변환을 이렇게 읽는다 (함수 대조 실험 misc3:
/// `1234567890123456` → 1234567890123450, `"0.99999999999999999"*1` → 0.999999999999999).
pub fn parse_decimal15(s: &str) -> Option<f64> {
    let (mant, exp) = match s.find(['e', 'E']) {
        Some(k) => (&s[..k], &s[k..]),
        None => (s, ""),
    };
    let mut out = String::with_capacity(s.len());
    let mut sig = 0;
    for c in mant.chars() {
        if c.is_ascii_digit() && (sig > 0 || c != '0') {
            sig += 1;
            if sig > 15 {
                out.push('0');
                continue;
            }
        }
        out.push(c);
    }
    out.push_str(exp);
    out.parse().ok()
}

/// 글이 논리 상수인가 (`TRUE`·`true`·`False` …, 앞뒤 공백 없이). 한셀은 연산자·VALUE 에서 1/0 으로 바꾼다.
pub fn parse_bool_text(s: &str) -> Option<bool> {
    if s.eq_ignore_ascii_case("TRUE") {
        Some(true)
    } else if s.eq_ignore_ascii_case("FALSE") {
        Some(false)
    } else {
        None
    }
}

/// 글을 숫자로 (VALUE 와 암묵 변환): 숫자 꼴, 대분수, 시각, 날짜(·시각). 논리 글은 다루지 않는다
/// ([`parse_bool_text`]). 문법은 한셀을 따른다 (함수 대조 실험 conv 묶음, `docs/format/calc.md`).
pub fn parse_number(s: &str, date1904: bool) -> Option<f64> {
    crate::calc::date::classify_text(s, date1904).map(|p| p.value())
}

/// 숫자 꼴 (앞뒤 공백·탭·전각 공백은 무시, 전각 숫자는 숫자):
/// `[+-]? [$₩\]? [+-]? 숫자(,세 자리 이상)* [.숫자] [E[+-]숫자] (%|[$₩\])?` 또는 `(…)` (음수, 안에 부호 없이).
/// 통화 기호와 % 는 함께 쓸 수 없다. 공백이 끼면 숫자가 아니다 (`- 5`, `$ 5`, `50 %`).
/// 짝 없는 UTF-16 대리 반쪽(U+D800–DFFF)을 글 안에 담는 자리 (U+10F000 + 코드 − D800). 한셀의 글은 UTF-16 이라
/// MID("😀a",2,1) 처럼 반쪽만 남을 수 있다 (UNICODE 는 56832, 파일에는 U+FFFD 로 쓴다, 대조 실험 cmisc)
pub const LONE_BASE: u32 = 0x10F000;

/// 대리 반쪽 자리 글자인가
pub fn is_lone(c: char) -> bool {
    (LONE_BASE..LONE_BASE + 0x800).contains(&(c as u32))
}

fn unit_char(u: u16) -> char {
    if (0xD800..0xE000).contains(&u) {
        char::from_u32(LONE_BASE + (u as u32 - 0xD800)).unwrap_or('\u{FFFD}')
    } else {
        char::from_u32(u as u32).unwrap_or('\u{FFFD}')
    }
}

/// 글 → UTF-16 단위마다 한 글자 (BMP 글자는 그대로, 대리 쌍은 반쪽 자리 글자 둘)
pub fn utf16_chars(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        if is_lone(c) {
            out.push(c);
            continue;
        }
        let mut b = [0u16; 2];
        for &u in c.encode_utf16(&mut b).iter() {
            out.push(unit_char(u));
        }
    }
    out
}

/// [`utf16_chars`] 의 거꾸로: 이웃한 대리 반쪽은 다시 한 글자로
pub fn from_utf16_chars(v: &[char]) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(v.len());
    for &c in v {
        if is_lone(c) {
            units.push((0xD800 + c as u32 - LONE_BASE) as u16);
        } else {
            let mut b = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut b));
        }
    }
    char::decode_utf16(units)
        .map(|r| r.unwrap_or_else(|e| unit_char(e.unpaired_surrogate())))
        .collect()
}

/// 반쪽 자리 글자를 U+FFFD 로 (파일에 쓸 값)
pub fn lone_to_fffd(s: &str) -> String {
    if !s.chars().any(is_lone) {
        return s.to_string();
    }
    s.chars()
        .map(|c| if is_lone(c) { '\u{FFFD}' } else { c })
        .collect()
}

pub fn parse_plain_number(t: &str) -> Option<f64> {
    let t = t.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\u{3000}'));
    if t.is_empty() {
        return None;
    }
    let t: String = t
        .chars()
        .map(|c| match c as u32 {
            0xFF10..=0xFF19 => char::from(b'0' + (c as u32 - 0xFF10) as u8),
            _ => c,
        })
        .collect();
    let mut s = t.as_str();
    let mut neg = false;
    let paren = s.starts_with('(') && s.ends_with(')') && s.len() >= 2;
    if paren {
        s = &s[1..s.len() - 1];
        neg = true;
    }
    let mut signed = false;
    let strip_sign = |s: &mut &str, neg: &mut bool, signed: &mut bool| {
        if *signed || paren {
            return;
        }
        if let Some(r) = s.strip_prefix('-') {
            *neg = !*neg;
            *s = r;
            *signed = true;
        } else if let Some(r) = s.strip_prefix('+') {
            *s = r;
            *signed = true;
        }
    };
    strip_sign(&mut s, &mut neg, &mut signed);
    const CUR: [&str; 4] = ["$", "₩", "\\", "￦"];
    let mut currency = false;
    for sym in CUR {
        if let Some(r) = s.strip_prefix(sym) {
            s = r;
            currency = true;
            break;
        }
    }
    if currency {
        strip_sign(&mut s, &mut neg, &mut signed);
    }
    let mut percent = false;
    if let Some(r) = s.strip_suffix('%') {
        if currency {
            return None;
        }
        s = r;
        percent = true;
    } else if !currency {
        for sym in CUR {
            if let Some(r) = s.strip_suffix(sym) {
                s = r;
                break;
            }
        }
    }
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut i = 0;
    let mut num = String::new();
    // 정수 부분 (쉼표 뒤에는 숫자 세 개 이상)
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => num.push(b[i] as char),
            b',' => {
                if num.is_empty() {
                    return None;
                }
                let run = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
                if run < 3 {
                    return None;
                }
            }
            _ => break,
        }
        i += 1;
    }
    let int_digits = num.len();
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        num.push('.');
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            num.push(b[i] as char);
            frac_digits += 1;
            i += 1;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        let mut e = String::from("e");
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            e.push(b[j] as char);
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            e.push(b[j] as char);
            j += 1;
        }
        if j == start {
            return None;
        }
        // 쓴 지수가 −309 보다 작으면 글로 남는다 (VALUE("1E-310")·VALUE("100E-310")·VALUE("0E-400") = #VALUE!,
        // "10E-309" 는 0, "0.1E+309" 는 1E+308 — 큰 쪽은 넘칠 때만, 대조 실험 cmisc·c3)
        if e[1..].parse::<i32>().is_ok_and(|x| x < -309) {
            return None;
        }
        num.push_str(&e);
        i = j;
    }
    if i != b.len() {
        return None;
    }
    let mut v: f64 = parse_decimal15(&num)?;
    // 비정규 수는 0 (VALUE("2.2250738585072E-308") = 0, % 는 그다음이라 VALUE("1E-306%") = 1E-308, 대조 실험 cmisc·c3)
    if v != 0.0 && v.abs() < f64::MIN_POSITIVE {
        v = 0.0;
    }
    if percent {
        v /= 100.0;
    }
    if neg {
        v = -v;
    }
    v.is_finite().then_some(v)
}

/// 대분수 `정수 분자/분모` (부호 없이, 분모 0 은 안 됨, 분자가 분모보다 커도 된다: `1 3/2` = 2.5).
/// `3/4` 처럼 정수 부분이 없으면 날짜로 읽는다.
pub fn parse_fraction(t: &str) -> Option<f64> {
    let t = t.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\u{3000}'));
    let (whole, frac) = t.split_once(' ')?;
    let (n, d) = frac.split_once('/')?;
    let all = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if !all(whole) || !all(n) || !all(d) {
        return None;
    }
    let (w, n, d): (f64, f64, f64) = (whole.parse().ok()?, n.parse().ok()?, d.parse().ok()?);
    if d == 0.0 {
        return None;
    }
    Some(w + n / d)
}

// ---------------------------------------------------------------------------
// 스칼라 변환
// ---------------------------------------------------------------------------

/// 숫자로 (연산자·숫자 인수). 빈 값 0, 논리 0/1, 글은 해석(`"TRUE"` 는 1 — 한셀), 오류는 그대로.
pub fn to_number(v: &Value, date1904: bool) -> Result<f64, ErrorKind> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(*b as u8 as f64),
        // 오류 값 이름의 글은 그 오류 (NOT("#N/A") = #N/A, "#DIV/0!"+1 = #DIV/0! — 대조 실험 conv2)
        Value::Text(s) => match parse_bool_text(s) {
            Some(b) => Ok(b as u8 as f64),
            None => match parse_number(s, date1904) {
                Some(n) => Ok(n),
                None => Err(ErrorKind::parse(s).unwrap_or(ErrorKind::Value)),
            },
        },
        Value::Error(e) => Err(*e),
        Value::Array(a) => to_number(a.data.first().unwrap_or(&Value::Empty), date1904),
        Value::Ref(_) => Err(ErrorKind::Value),
    }
}

/// 글로 (연결 연산자·글 인수)
pub fn to_text(v: &Value) -> Result<Rc<str>, ErrorKind> {
    match v {
        Value::Empty => Ok(Rc::from("")),
        Value::Number(n) => Ok(Rc::from(number_to_text(*n))),
        Value::Text(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(Rc::from(if *b { "TRUE" } else { "FALSE" })),
        Value::Error(e) => Err(*e),
        Value::Array(a) => to_text(a.data.first().unwrap_or(&Value::Empty)),
        Value::Ref(_) => Err(ErrorKind::Value),
    }
}

/// 논리로 (IF 조건, NOT 인수). 글은 "TRUE"/"FALSE", 숫자로 읽히는 글은 0 이 아닌가
/// (IF("0",1,2) = 2, NOT("5") = FALSE, IF("12:00",1,2) = 1 — 대조 실험 misc5·misc6), 그 밖의 글은 #VALUE!.
pub fn to_bool(v: &Value) -> Result<bool, ErrorKind> {
    match v {
        Value::Empty => Ok(false),
        Value::Number(n) => Ok(*n != 0.0),
        Value::Bool(b) => Ok(*b),
        Value::Text(s) => {
            if s.eq_ignore_ascii_case("TRUE") {
                Ok(true)
            } else if s.eq_ignore_ascii_case("FALSE") {
                Ok(false)
            } else if let Some(n) = parse_number(s, false) {
                Ok(n != 0.0)
            } else {
                Err(ErrorKind::parse(s).unwrap_or(ErrorKind::Value))
            }
        }
        Value::Error(e) => Err(*e),
        Value::Array(a) => to_bool(a.data.first().unwrap_or(&Value::Empty)),
        Value::Ref(_) => Err(ErrorKind::Value),
    }
}

/// 같은 종류·같은 값의 숫자·논리 (XOR 이 이웃 칸을 한 칸으로 세는지 가릴 때)
pub fn same_constant(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
    }
}

/// 글 비교: 대소문자 무시, 한셀의 언어별 정렬 순서 ([`crate::calc::collate`])
pub fn cmp_text(a: &str, b: &str) -> Ordering {
    crate::calc::collate::compare(a, b)
}

/// 대소문자 무시 비교용 정규화
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// 비교 연산자의 순서: 숫자 < 글 < 논리, 빈 값은 상대 형의 기본값(0, "", FALSE).
/// 오류는 호출하는 쪽에서 먼저 걸러야 한다.
pub fn compare(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Number(_) | Value::Empty => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            _ => 3,
        }
    }
    match (a, b) {
        (Value::Empty, Value::Empty) => Ordering::Equal,
        (Value::Empty, Value::Text(t)) => {
            if t.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Value::Text(t), Value::Empty) => {
            if t.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Value::Empty, Value::Bool(x)) => false.cmp(x),
        (Value::Bool(x), Value::Empty) => x.cmp(&false),
        (Value::Number(x), Value::Number(y)) => cmp_num(*x, *y),
        (Value::Empty, Value::Number(y)) => cmp_num(0.0, *y),
        (Value::Number(x), Value::Empty) => cmp_num(*x, 0.0),
        (Value::Text(x), Value::Text(y)) => cmp_text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// 숫자 비교: 유효숫자 15자리로 같으면 같다고 본다 (0.1+0.2 = 0.3)
pub fn cmp_num(x: f64, y: f64) -> Ordering {
    if x == y {
        return Ordering::Equal;
    }
    let (rx, ry) = (round15(x), round15(y));
    if rx == ry {
        return Ordering::Equal;
    }
    x.partial_cmp(&y).unwrap_or(Ordering::Equal)
}

/// 찾기 함수(MATCH·VLOOKUP …)의 같음: 같은 형이고, 글은 대소문자 무시
pub fn lookup_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => cmp_num(*x, *y) == Ordering::Equal,
        (Value::Text(x), Value::Text(y)) => cmp_text(x, y) == Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Error(x), Value::Error(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_from_text() {
        let p = |s| parse_number(s, false);
        assert_eq!(p("12"), Some(12.0));
        assert_eq!(p(" 1,234.5 "), Some(1234.5));
        assert_eq!(p("-1.5E+3"), Some(-1500.0));
        assert_eq!(p("50%"), Some(0.5));
        assert_eq!(p("(12)"), Some(-12.0));
        assert_eq!(p("$1,000"), Some(1000.0));
        assert_eq!(p("1 1/2"), Some(1.5));
        assert_eq!(p("1,23"), None);
        assert_eq!(p("abc"), None);
        assert_eq!(p("TRUE"), None);
        assert_eq!(p(".5"), Some(0.5));
    }

    #[test]
    fn number_text() {
        assert_eq!(number_to_text(0.1 + 0.2), "0.3");
        assert_eq!(number_to_text(1.0 / 3.0), "0.333333333333333");
        assert_eq!(number_to_text(1e20), "1E+20");
        assert_eq!(number_to_text(-5.0), "-5");
    }

    #[test]
    fn ordering() {
        use std::cmp::Ordering::*;
        assert_eq!(compare(&Value::Number(5.0), &"a".into()), Less);
        assert_eq!(compare(&"a".into(), &Value::Bool(false)), Less);
        assert_eq!(compare(&"abc".into(), &"ABC".into()), Equal);
        assert_eq!(compare(&Value::Empty, &Value::Number(0.0)), Equal);
        assert_eq!(compare(&Value::Empty, &"".into()), Equal);
        assert_eq!(
            compare(&Value::Number(0.1 + 0.2), &Value::Number(0.3)),
            Equal
        );
    }
}
