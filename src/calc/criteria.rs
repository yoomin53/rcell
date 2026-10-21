// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 조건 (COUNTIF·SUMIF·…IFS·D 함수): `">=10"`, `"<>"`, `"사과*"`, 숫자, 논리, 오류.
//!
//! 글 조건 앞의 비교 연산자를 떼고, 나머지가 숫자로 읽히면 숫자 비교를 한다. `=`·`<>` 의
//! 글 비교는 대소문자를 무시하고 와일드카드(`*`, `?`, `~` 이스케이프)를 쓴다.

use super::value::{self, ErrorKind, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CritOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CritVal {
    Num(f64),
    /// 대소문자를 접은 글
    Text(String),
    Bool(bool),
    Err(ErrorKind),
    /// 빈 피연산자 (`""`, `"="`, `"<>"`)
    Blank,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    pub op: CritOp,
    pub val: CritVal,
    /// 연산자를 글로 적었는가 (`"=5"`)
    pub explicit_op: bool,
    /// 데이터베이스 조건: 연산자 없는 글은 "그 글로 시작"
    pub prefix: bool,
    wildcard: bool,
    /// 숫자 조건의 원래 글 ("5") — 숫자처럼 보이는 글 셀과 비교
    num_text: Option<String>,
    /// D 함수 조건 표인가
    database: bool,
    date1904: bool,
}

impl Criterion {
    /// 조건 값(스칼라)을 해석한다
    pub fn parse(v: &Value, date1904: bool) -> Criterion {
        Self::parse_mode(v, date1904, false)
    }

    /// D 함수 조건 표의 칸
    pub fn parse_database(v: &Value, date1904: bool) -> Criterion {
        Self::parse_mode(v, date1904, true)
    }

    fn parse_mode(v: &Value, date1904: bool, database: bool) -> Criterion {
        let mk = |op, val, explicit_op| Criterion {
            op,
            val,
            explicit_op,
            prefix: false,
            wildcard: false,
            num_text: None,
            database,
            date1904,
        };
        match v {
            Value::Number(n) => mk(CritOp::Eq, CritVal::Num(*n), false),
            Value::Bool(b) => mk(CritOp::Eq, CritVal::Bool(*b), false),
            Value::Error(e) => mk(CritOp::Eq, CritVal::Err(*e), false),
            Value::Empty => mk(CritOp::Eq, CritVal::Num(0.0), false),
            Value::Text(s) => {
                let s: &str = s;
                let (op, rest, explicit) = if let Some(r) = s.strip_prefix(">=") {
                    (CritOp::Ge, r, true)
                } else if let Some(r) = s.strip_prefix("<=") {
                    (CritOp::Le, r, true)
                } else if let Some(r) = s.strip_prefix("<>") {
                    (CritOp::Ne, r, true)
                } else if let Some(r) = s.strip_prefix('>') {
                    (CritOp::Gt, r, true)
                } else if let Some(r) = s.strip_prefix('<') {
                    (CritOp::Lt, r, true)
                } else if let Some(r) = s.strip_prefix('=') {
                    (CritOp::Eq, r, true)
                } else {
                    (CritOp::Eq, s, false)
                };
                if rest.is_empty() {
                    return mk(op, CritVal::Blank, explicit);
                }
                if let Some(n) = value::parse_number(rest, date1904) {
                    let mut c = mk(op, CritVal::Num(n), explicit);
                    c.num_text = Some(value::fold(rest));
                    return c;
                }
                if rest.eq_ignore_ascii_case("TRUE") {
                    return mk(op, CritVal::Bool(true), explicit);
                }
                if rest.eq_ignore_ascii_case("FALSE") {
                    return mk(op, CritVal::Bool(false), explicit);
                }
                // 연산자 없는 "#N/A" 는 글 조건이라 오류 칸에 맞지 않는다 ("<>#N/A" 는 오류 조건,
                // COUNTIF(Data!H1:H10,"#N/A") = 0·"<>#N/A" = 9, 대조 실험 stat)
                if let Some(e) = ErrorKind::parse(rest)
                    && (explicit || database)
                {
                    return mk(op, CritVal::Err(e), explicit);
                }
                let mut c = mk(op, CritVal::Text(value::fold(rest)), explicit);
                c.wildcard = rest.contains(['*', '?', '~']);
                c.prefix = database && !explicit;
                c
            }
            Value::Array(a) => Self::parse_mode(a.data.first().unwrap_or(&Value::Empty), date1904, database),
            Value::Ref(_) => mk(CritOp::Eq, CritVal::Err(ErrorKind::Value), false),
        }
    }

    /// 셀 값이 조건에 맞는가
    pub fn matches(&self, cell: &Value) -> bool {
        use std::cmp::Ordering;
        let cmp_ok = |o: Ordering| match self.op {
            CritOp::Eq => o == Ordering::Equal,
            CritOp::Ne => o != Ordering::Equal,
            CritOp::Lt => o == Ordering::Less,
            CritOp::Le => o != Ordering::Greater,
            CritOp::Gt => o == Ordering::Greater,
            CritOp::Ge => o != Ordering::Less,
        };
        match &self.val {
            CritVal::Blank => {
                let blank = matches!(cell, Value::Empty)
                    || matches!(cell, Value::Text(t) if t.is_empty() && !self.explicit_op);
                match self.op {
                    CritOp::Eq => blank,
                    CritOp::Ne => !matches!(cell, Value::Empty),
                    _ => false,
                }
            }
            CritVal::Num(n) => match cell {
                Value::Number(x) => cmp_ok(value::cmp_num(*x, *n)),
                Value::Text(t) if !self.database && !matches!(self.op, CritOp::Eq | CritOp::Ne) => {
                    // 한셀은 크기 비교에서 숫자로 읽히는 글 칸을 숫자로 본다 (">0" 이 글 "5" 를 센다,
                    // 대조 실험 stat·misc4)
                    match value::parse_number(t, self.date1904) {
                        Some(x) => cmp_ok(value::cmp_num(x, *n)),
                        None => false,
                    }
                }
                Value::Text(t) if !self.database => {
                    // = · <> 는 글끼리: 조건의 숫자 글과 칸의 글이 같은가 (COUNTIF(…,5) 는 글 "5" 를 세고
                    // 글 "1E3" 은 1000 으로 세지 않는다, 대조 실험 misc4)
                    let want = self.num_text.clone().unwrap_or_else(|| value::number_to_text(*n));
                    let same = value::fold(t) == value::fold(&want);
                    match self.op {
                        CritOp::Eq => same,
                        _ => !same,
                    }
                }
                Value::Text(t) => {
                    // 숫자처럼 보이는 글: = 는 글이 같을 때만, <> 는 그 반대
                    let same = self
                        .num_text
                        .as_deref()
                        .is_some_and(|nt| value::fold(t) == nt);
                    match self.op {
                        CritOp::Eq => same,
                        CritOp::Ne => !same,
                        _ => false,
                    }
                }
                _ => self.op == CritOp::Ne,
            },
            CritVal::Bool(b) => match cell {
                Value::Bool(x) => cmp_ok(x.cmp(b)),
                _ => self.op == CritOp::Ne,
            },
            CritVal::Err(e) => match cell {
                Value::Error(x) => cmp_ok(if x == e {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }),
                _ => self.op == CritOp::Ne,
            },
            CritVal::Text(pat) => match cell {
                Value::Text(t) => {
                    let ft = value::fold(t);
                    match self.op {
                        CritOp::Eq | CritOp::Ne => {
                            let hit = if self.prefix {
                                if self.wildcard {
                                    wildcard_match(&format!("{pat}*"), &ft)
                                } else {
                                    ft.starts_with(pat.as_str())
                                }
                            } else if self.wildcard {
                                wildcard_match(pat, &ft)
                            } else {
                                ft == *pat
                            };
                            if self.op == CritOp::Eq { hit } else { !hit }
                        }
                        _ => cmp_ok(ft.as_str().cmp(pat.as_str())),
                    }
                }
                _ => self.op == CritOp::Ne,
            },
        }
    }
}

/// 와일드카드 일치 (`*` 아무 글자열, `?` 한 글자, `~` 다음의 `*`·`?`·`~` 를 그대로). 둘 다 접은 글.
/// 한셀은 다른 글자 앞의 `~` 를 글자 그대로 둔다 (COUNTIF(…,"~a*") 는 "~a" 로 시작하는 글, 대조 실험 stat).
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum P {
        Any,
        One,
        Ch(char),
    }
    let mut pat = Vec::new();
    let mut it = pattern.chars();
    while let Some(c) = it.next() {
        match c {
            '~' => match it.clone().next() {
                Some(n @ ('*' | '?' | '~')) => {
                    it.next();
                    pat.push(P::Ch(n))
                }
                _ => pat.push(P::Ch('~')),
            },
            '*' => {
                if pat.last() != Some(&P::Any) {
                    pat.push(P::Any)
                }
            }
            '?' => pat.push(P::One),
            c => pat.push(P::Ch(c)),
        }
    }
    let t: Vec<char> = text.chars().collect();
    // 되돌아가기 한 번짜리 탐욕 일치
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < pat.len() {
            match pat[pi] {
                P::Any => {
                    star = Some((pi, ti));
                    pi += 1;
                    continue;
                }
                P::One => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                P::Ch(c) if c == t[ti] => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, st)) => {
                pi = sp + 1;
                ti = st + 1;
                star = Some((sp, st + 1));
            }
            None => return false,
        }
    }
    while pi < pat.len() && pat[pi] == P::Any {
        pi += 1;
    }
    pi == pat.len()
}

/// 와일드카드가 있는 찾기 (MATCH·VLOOKUP 의 정확히 일치, SEARCH)
pub fn has_wildcards(s: &str) -> bool {
    s.contains(['*', '?', '~'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Criterion {
        Criterion::parse(&Value::from(s), false)
    }

    #[test]
    fn operators() {
        assert!(c(">10").matches(&Value::Number(11.0)));
        assert!(!c(">10").matches(&Value::Number(10.0)));
        assert!(c(">=10").matches(&Value::Number(10.0)));
        assert!(c("<>5").matches(&Value::Empty));
        assert!(c("<>5").matches(&Value::from("x")));
        // 한셀은 숫자로 읽히는 글 칸을 숫자로 비교한다 (대조 실험 stat)
        assert!(c(">10").matches(&Value::from("20")));
        assert!(c("5").matches(&Value::from("5")));
    }

    #[test]
    fn text_and_wildcards() {
        assert!(c("사과*").matches(&Value::from("사과나무")));
        assert!(c("a?c").matches(&Value::from("ABC")));
        assert!(!c("a?c").matches(&Value::from("abbc")));
        assert!(c("~*").matches(&Value::from("*")));
        assert!(c("").matches(&Value::Empty));
        assert!(c("<>").matches(&Value::Number(1.0)));
        assert!(!c("<>").matches(&Value::Empty));
        assert!(wildcard_match("*b*", "abc"));
        assert!(wildcard_match("a*", "a"));
        assert!(!wildcard_match("a*c", "ab"));
    }
}
