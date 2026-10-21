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
            Value::Text(s) => CellValue::Text(s.to_string()),
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

/// 숫자를 글로 (연결 연산자·글 함수): 유효숫자 15자리 일반 표기, 아주 크거나 작으면 지수.
pub fn number_to_text(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    crate::numfmt::general(v)
}

/// 유효숫자 15자리로 반올림 (스프레드시트가 저장·비교에 쓰는 정밀도)
pub fn round15(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    format!("{v:.14e}").parse().unwrap_or(v)
}

/// 글을 숫자로 (VALUE 와 암묵 변환): 공백, 부호, 천 단위 쉼표, 소수, 지수, 백분율, 통화 기호,
/// 괄호 음수, 분수(`1 1/2`), 날짜·시각.
pub fn parse_number(s: &str, date1904: bool) -> Option<f64> {
    let t = s.trim_matches(|c: char| c == ' ' || c == '\u{3000}');
    if t.is_empty() {
        return None;
    }
    if let Some(v) = parse_plain_number(t) {
        return Some(v);
    }
    if let Some(v) = parse_fraction(t) {
        return Some(v);
    }
    crate::calc::date::parse_datetime(t, date1904)
}

/// 숫자 꼴만: [-+]? [₩$]? 숫자(쉼표) [.숫자] [E±숫자] [%]  또는 (숫자)
fn parse_plain_number(t: &str) -> Option<f64> {
    let mut s = t;
    let mut neg = false;
    // 괄호 음수
    if let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        neg = true;
        s = inner.trim();
    }
    let mut sign_seen = false;
    let strip_sign = |s: &mut &str, neg: &mut bool, seen: &mut bool| {
        if !*seen {
            if let Some(r) = s.strip_prefix('-') {
                *neg = !*neg;
                *s = r.trim_start();
                *seen = true;
            } else if let Some(r) = s.strip_prefix('+') {
                *s = r.trim_start();
                *seen = true;
            }
        }
    };
    strip_sign(&mut s, &mut neg, &mut sign_seen);
    let mut currency = false;
    for sym in ["₩", "\\", "$", "￦"] {
        if let Some(r) = s.strip_prefix(sym) {
            s = r.trim_start();
            currency = true;
            break;
        }
    }
    if currency {
        strip_sign(&mut s, &mut neg, &mut sign_seen);
    }
    let mut percent = 0;
    while let Some(r) = s.strip_suffix('%') {
        s = r.trim_end();
        percent += 1;
    }
    if !currency {
        for sym in ["₩", "원"] {
            if let Some(r) = s.strip_suffix(sym) {
                s = r.trim_end();
                break;
            }
        }
    }
    if s.is_empty() {
        return None;
    }
    // 가수부
    let b = s.as_bytes();
    let mut i = 0;
    let mut digits = String::new();
    let mut int_digits = 0;
    let mut saw_comma = false;
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => {
                digits.push(b[i] as char);
                int_digits += 1;
            }
            b',' if int_digits > 0 => {
                // 쉼표 뒤에는 숫자 세 개
                let next3 = b.get(i + 1..i + 4);
                if !next3.is_some_and(|n| n.iter().all(u8::is_ascii_digit)) {
                    return None;
                }
                saw_comma = true;
            }
            _ => break,
        }
        i += 1;
    }
    let _ = saw_comma;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        digits.push('.');
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            digits.push(b[i] as char);
            frac_digits += 1;
            i += 1;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        let mut exp = String::from("e");
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            exp.push(b[j] as char);
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            exp.push(b[j] as char);
            j += 1;
        }
        if j == start {
            return None;
        }
        digits.push_str(&exp);
        i = j;
    }
    if i != b.len() {
        return None;
    }
    let mut v: f64 = digits.parse().ok()?;
    for _ in 0..percent {
        v /= 100.0;
    }
    if neg {
        v = -v;
    }
    v.is_finite().then_some(v)
}

/// `1 1/2`, `3/4` 꼴 분수 (분모 1–… , 분자 < 분모가 아니어도 받는다)
fn parse_fraction(t: &str) -> Option<f64> {
    let (neg, s) = match t.strip_prefix('-') {
        Some(r) => (true, r.trim_start()),
        None => (false, t.strip_prefix('+').unwrap_or(t).trim_start()),
    };
    let (whole, frac) = match s.split_once(' ') {
        Some((w, f)) => (Some(w.trim()), f.trim()),
        None => (None, s),
    };
    let (n, d) = frac.split_once('/')?;
    if n.is_empty() || d.is_empty() || !n.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !d.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: f64 = n.parse().ok()?;
    let d: f64 = d.parse().ok()?;
    if d == 0.0 {
        return None;
    }
    let w: f64 = match whole {
        Some(w) if w.bytes().all(|c| c.is_ascii_digit()) && !w.is_empty() => w.parse().ok()?,
        Some(_) => return None,
        None => {
            // `3/4` 단독은 날짜(3월 4일)로 읽는다 — 분수로 보지 않는다
            return None;
        }
    };
    if n >= d {
        return None;
    }
    let v = w + n / d;
    Some(if neg { -v } else { v })
}

// ---------------------------------------------------------------------------
// 스칼라 변환
// ---------------------------------------------------------------------------

/// 숫자로 (연산자·숫자 인수). 빈 값 0, 논리 0/1, 글은 해석, 오류는 그대로.
pub fn to_number(v: &Value, date1904: bool) -> Result<f64, ErrorKind> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(*b as u8 as f64),
        Value::Text(s) => parse_number(s, date1904).ok_or(ErrorKind::Value),
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

/// 논리로 (IF 조건, AND·OR 인수). 글은 "TRUE"/"FALSE" 만.
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
            } else {
                Err(ErrorKind::Value)
            }
        }
        Value::Error(e) => Err(*e),
        Value::Array(a) => to_bool(a.data.first().unwrap_or(&Value::Empty)),
        Value::Ref(_) => Err(ErrorKind::Value),
    }
}

/// 글 비교: 대소문자 무시
pub fn cmp_text(a: &str, b: &str) -> Ordering {
    let la = fold(a);
    let lb = fold(b);
    la.cmp(&lb)
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
        assert_eq!(compare(&Value::Number(0.1 + 0.2), &Value::Number(0.3)), Equal);
    }
}
