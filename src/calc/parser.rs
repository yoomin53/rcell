// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 원문(OOXML A1 표기, `=` 없음) → 구문 트리.
//!
//! 연산자 우선순위 (높은 것부터): 참조 연산자 `:` → 공백(교집합) → `,`(합집합, 괄호 안·최상위만) →
//! 단항 `-` `+` → `%` → `^` → `*` `/` → `+` `-` → `&` → 비교. `^` 는 왼쪽부터 묶고, 단항 `-` 가
//! `^` 보다 먼저 묶인다 (`-2^2` = 4).

use std::rc::Rc;

use super::ast::{BinaryOp, Call, Expr, NameExpr, RefExpr, SheetSpec, TableRef, UnaryOp};
use super::value::{Array, ErrorKind, MAX_COLS, MAX_ROWS, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub msg: String,
    pub pos: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "수식 해석 오류 ({}번째 글자): {}", self.pos, self.msg)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Err(ErrorKind),
    Bool(bool),
    Ref(RefExpr),
    Name(NameExpr),
    Table(TableRef),
    /// 함수 이름 (여는 괄호까지 읽었다)
    Func(String),
    Op(&'static str),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semi,
    Space,
}

/// 수식 원문을 구문 트리로 바꾼다.
pub fn parse(formula: &str) -> Result<Expr, ParseError> {
    let src = formula.trim_start_matches('=');
    let toks = Lexer::new(src).run()?;
    let mut p = Parser {
        toks,
        pos: 0,
        union_ok: true,
    };
    let e = p.expr()?;
    p.skip_space();
    if p.pos < p.toks.len() {
        return Err(ParseError {
            msg: format!("남은 토큰 {:?}", p.toks[p.pos].0),
            pos: p.toks[p.pos].1,
        });
    }
    Ok(e)
}

// ---------------------------------------------------------------------------
// 토큰
// ---------------------------------------------------------------------------

struct Lexer {
    c: Vec<char>,
    i: usize,
    out: Vec<(Tok, usize)>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '$' | '?')
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '_' | '\\' | '$')
}

/// `$A`, `AB` → 0부터 시작하는 열
fn parse_col(s: &str) -> Option<u32> {
    let s = s.strip_prefix('$').unwrap_or(s);
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for b in s.bytes() {
        n = n * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32;
    }
    (1..=MAX_COLS).contains(&n).then(|| n - 1)
}

/// `$12`, `12` → 0부터 시작하는 행
fn parse_row(s: &str) -> Option<u32> {
    let s = s.strip_prefix('$').unwrap_or(s);
    if s.is_empty() || s.len() > 7 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = s.parse().ok()?;
    (1..=MAX_ROWS).contains(&n).then(|| n - 1)
}

/// `$A$1` → (행, 열)
fn parse_cell(s: &str) -> Option<(u32, u32)> {
    let body = s.strip_prefix('$').unwrap_or(s);
    let split = body.find(|c: char| c.is_ascii_digit() || c == '$')?;
    let (letters, rest) = body.split_at(split);
    let col = parse_col(letters)?;
    let row = parse_row(rest)?;
    Some((row, col))
}

impl Lexer {
    fn new(src: &str) -> Lexer {
        Lexer {
            c: src.chars().collect(),
            i: 0,
            out: Vec::new(),
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            msg: msg.into(),
            pos: self.i,
        })
    }

    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }

    fn word_at(&self, start: usize) -> (String, usize) {
        let mut j = start;
        while j < self.c.len() && is_word_char(self.c[j]) {
            j += 1;
        }
        (self.c[start..j].iter().collect(), j)
    }

    fn run(mut self) -> Result<Vec<(Tok, usize)>, ParseError> {
        while self.i < self.c.len() {
            let start = self.i;
            let ch = self.c[self.i];
            let tok = match ch {
                ' ' | '\t' | '\r' | '\n' | '\u{3000}' | '\u{a0}' => {
                    while self.i < self.c.len()
                        && matches!(self.c[self.i], ' ' | '\t' | '\r' | '\n' | '\u{3000}' | '\u{a0}')
                    {
                        self.i += 1;
                    }
                    Tok::Space
                }
                '"' => self.string()?,
                '#' => {
                    let e = self.error_literal()?;
                    Tok::Err(e)
                }
                '\'' => self.quoted_prefix()?,
                '[' => self.book_prefix()?,
                '(' => {
                    self.i += 1;
                    Tok::LParen
                }
                ')' => {
                    self.i += 1;
                    Tok::RParen
                }
                '{' => {
                    self.i += 1;
                    Tok::LBrace
                }
                '}' => {
                    self.i += 1;
                    Tok::RBrace
                }
                ',' => {
                    self.i += 1;
                    Tok::Comma
                }
                ';' => {
                    self.i += 1;
                    Tok::Semi
                }
                '+' | '-' | '*' | '/' | '^' | '&' | '%' | ':' | '=' => {
                    self.i += 1;
                    Tok::Op(match ch {
                        '+' => "+",
                        '-' => "-",
                        '*' => "*",
                        '/' => "/",
                        '^' => "^",
                        '&' => "&",
                        '%' => "%",
                        ':' => ":",
                        _ => "=",
                    })
                }
                '<' => {
                    self.i += 1;
                    match self.peek(0) {
                        Some('=') => {
                            self.i += 1;
                            Tok::Op("<=")
                        }
                        Some('>') => {
                            self.i += 1;
                            Tok::Op("<>")
                        }
                        _ => Tok::Op("<"),
                    }
                }
                '>' => {
                    self.i += 1;
                    if self.peek(0) == Some('=') {
                        self.i += 1;
                        Tok::Op(">=")
                    } else {
                        Tok::Op(">")
                    }
                }
                c if c.is_ascii_digit() || c == '.' || c == '$' => {
                    if let Some(t) = self.row_range(None, SheetSpec::Current) {
                        t
                    } else if c == '$' {
                        self.word_token(None, SheetSpec::Current, false)?
                    } else {
                        self.number()?
                    }
                }
                c if is_word_start(c) => self.word_token(None, SheetSpec::Current, false)?,
                _ => return self.err(format!("알 수 없는 글자 {ch:?}")),
            };
            self.out.push((tok, start));
        }
        Ok(self.out)
    }

    fn string(&mut self) -> Result<Tok, ParseError> {
        self.i += 1;
        let mut s = String::new();
        loop {
            match self.peek(0) {
                None => return self.err("닫히지 않은 문자열"),
                Some('"') => {
                    if self.peek(1) == Some('"') {
                        s.push('"');
                        self.i += 2;
                    } else {
                        self.i += 1;
                        break;
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(Tok::Str(s))
    }

    fn error_literal(&mut self) -> Result<ErrorKind, ParseError> {
        const ERRS: [&str; 10] = [
            "#NULL!",
            "#DIV/0!",
            "#VALUE!",
            "#REF!",
            "#NAME?",
            "#NUM!",
            "#N/A",
            "#GETTING_DATA",
            "#SPILL!",
            "#CALC!",
        ];
        let rest: String = self.c[self.i..].iter().take(14).collect();
        let upper = rest.to_ascii_uppercase();
        for e in ERRS {
            if upper.starts_with(e) {
                self.i += e.chars().count();
                return Ok(ErrorKind::parse(e).unwrap());
            }
        }
        self.err("알 수 없는 오류 값")
    }

    fn number(&mut self) -> Result<Tok, ParseError> {
        let start = self.i;
        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        if self.peek(0) == Some('.') {
            self.i += 1;
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(0), Some('+' | '-')) {
                self.i += 1;
            }
            if self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1;
                }
            } else {
                self.i = save;
            }
        }
        let s: String = self.c[start..self.i].iter().collect();
        match super::value::parse_decimal15(&s) {
            // 한셀은 수식 안 숫자 상수의 유효숫자 15자리 뒤를 버린다 (1234567890123456 → 1234567890123450)
            Some(v) => Ok(Tok::Num(v)),
            None => self.err(format!("숫자가 아님 {s:?}")),
        }
    }

    /// `1:3`, `$1:$3` (행 전체)
    fn row_range(&mut self, book: Option<u32>, sheet: SheetSpec) -> Option<Tok> {
        let (w1, j) = self.row_word(self.i)?;
        if self.c.get(j) != Some(&':') {
            return None;
        }
        let (w2, k) = self.row_word(j + 1)?;
        if self.c.get(k).is_some_and(|&c| is_word_char(c) || c == '(') {
            return None;
        }
        let (a, b) = (parse_row(&w1)?, parse_row(&w2)?);
        self.i = k;
        Some(Tok::Ref(RefExpr {
            book,
            sheet,
            r0: a.min(b),
            r1: a.max(b),
            c0: 0,
            c1: MAX_COLS - 1,
        }))
    }

    fn row_word(&self, start: usize) -> Option<(String, usize)> {
        let mut j = start;
        if self.c.get(j) == Some(&'$') {
            j += 1;
        }
        let d0 = j;
        while self.c.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        if j == d0 {
            return None;
        }
        Some((self.c[start..j].iter().collect(), j))
    }

    /// `'시트 이름'!` 또는 `'[1]시트'!` 또는 `'시트1:시트3'!`
    fn quoted_prefix(&mut self) -> Result<Tok, ParseError> {
        self.i += 1;
        let mut s = String::new();
        loop {
            match self.peek(0) {
                None => return self.err("닫히지 않은 시트 이름"),
                Some('\'') => {
                    if self.peek(1) == Some('\'') {
                        s.push('\'');
                        self.i += 2;
                    } else {
                        self.i += 1;
                        break;
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
        if self.peek(0) != Some('!') {
            return self.err("시트 이름 뒤에 ! 가 없음");
        }
        self.i += 1;
        // 외부 문서 번호 [n]
        let (book, name) = match s.strip_prefix('[').and_then(|r| r.split_once(']')) {
            Some((n, rest)) => match n.parse::<u32>() {
                Ok(b) => (Some(b), rest.to_string()),
                // 경로로 펼친 외부 참조 — 연결 번호를 알 수 없다
                Err(_) => (Some(0), rest.to_string()),
            },
            None => match s.rfind('[') {
                // 'C:\경로\[파일]시트'
                Some(p) if s[p..].contains(']') => {
                    let close = s[p..].find(']').unwrap() + p;
                    (Some(0), s[close + 1..].to_string())
                }
                _ => (None, s),
            },
        };
        let sheet = match name.split_once(':') {
            Some((a, b)) if !a.is_empty() && !b.is_empty() => {
                SheetSpec::Span(a.to_string(), b.to_string())
            }
            _ => SheetSpec::One(name),
        };
        self.target(book, sheet)
    }

    /// `[1]Sheet1!A1`, `[1]!이름`
    fn book_prefix(&mut self) -> Result<Tok, ParseError> {
        let start = self.i;
        let mut j = self.i + 1;
        while self.c.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        if j == start + 1 || self.c.get(j) != Some(&']') {
            // 표 이름 없는 구조적 참조 ([@열], [#This Row] …)
            return self.structured(String::new());
        }
        let book: u32 = self.c[start + 1..j]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        self.i = j + 1;
        if self.peek(0) == Some('!') {
            self.i += 1;
            let (w, k) = self.word_at(self.i);
            if w.is_empty() {
                return self.err("외부 이름이 없음");
            }
            self.i = k;
            return Ok(Tok::Name(NameExpr {
                book: Some(book),
                sheet: None,
                name: w,
            }));
        }
        let (w, k) = self.word_at(self.i);
        if w.is_empty() {
            return self.err("외부 시트 이름이 없음");
        }
        self.i = k;
        let sheet = if self.peek(0) == Some(':') {
            let (w2, k2) = self.word_at(self.i + 1);
            if self.c.get(k2) == Some(&'!') && !w2.is_empty() {
                self.i = k2;
                SheetSpec::Span(w, w2)
            } else {
                SheetSpec::One(w)
            }
        } else {
            SheetSpec::One(w)
        };
        if self.peek(0) != Some('!') {
            return self.err("외부 시트 이름 뒤에 ! 가 없음");
        }
        self.i += 1;
        self.target(Some(book), sheet)
    }

    /// 구조적 참조 `표[…]` (`self.i` 는 '[' 자리): `[]`, `[열]`, `[#All]`, `[[#Data],[열1]:[열2]]`,
    /// `[@열]`·`[@[열 이름]]`. 열 이름의 `'` 는 다음 글자를 그대로 둔다.
    fn structured(&mut self, table: String) -> Result<Tok, ParseError> {
        let start = self.i;
        let mut depth = 0i32;
        let mut j = self.i;
        while j < self.c.len() {
            match self.c[j] {
                '\'' => {
                    j += 2;
                    continue;
                }
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if j >= self.c.len() {
            return self.err("닫히지 않은 구조적 참조");
        }
        let inner: Vec<char> = self.c[start + 1..j].to_vec();
        self.i = j + 1;
        let unescape = |s: &[char]| -> String {
            let mut out = String::new();
            let mut k = 0;
            while k < s.len() {
                if s[k] == '\'' && k + 1 < s.len() {
                    k += 1;
                }
                out.push(s[k]);
                k += 1;
            }
            out
        };
        // 항목 (괄호 안 글)과 항목 뒤 구분자
        let mut items: Vec<(String, Option<char>)> = Vec::new();
        let mut t = TableRef { table, ..Default::default() };
        let mut body: &[char] = &inner;
        if body.first() == Some(&'@') {
            t.this_row = true;
            body = &body[1..];
        }
        if body.first() == Some(&'[') {
            let mut k = 0;
            while k < body.len() {
                match body[k] {
                    '[' => {
                        let mut e = k + 1;
                        while e < body.len() && body[e] != ']' {
                            if body[e] == '\'' {
                                e += 1;
                            }
                            e += 1;
                        }
                        let text = unescape(&body[k + 1..e.min(body.len())]);
                        items.push((text.trim().to_string(), None));
                        k = e + 1;
                    }
                    ',' | ':' => {
                        if let Some(last) = items.last_mut() {
                            last.1 = Some(body[k]);
                        }
                        k += 1;
                    }
                    _ => k += 1,
                }
            }
        } else if !body.is_empty() {
            items.push((unescape(body).trim().to_string(), None));
        }
        let mut cols: Vec<String> = Vec::new();
        for (text, _) in &items {
            match text.to_ascii_lowercase().as_str() {
                "#all" => t.all = true,
                "#data" => t.data = true,
                "#headers" => t.headers = true,
                "#totals" => t.totals = true,
                "#this row" => t.this_row = true,
                _ => cols.push(text.clone()),
            }
        }
        t.cols = match cols.len() {
            0 => None,
            1 => Some((cols[0].clone(), cols[0].clone())),
            _ => Some((cols[0].clone(), cols[cols.len() - 1].clone())),
        };
        Ok(Tok::Table(t))
    }

    /// 시트 접두사 뒤의 대상: 셀·영역·행·열, 이름, #REF!
    fn target(&mut self, book: Option<u32>, sheet: SheetSpec) -> Result<Tok, ParseError> {
        match self.peek(0) {
            Some('#') => {
                let e = self.error_literal()?;
                Ok(Tok::Err(e))
            }
            Some(c) if c.is_ascii_digit() || c == '$' => {
                if let Some(t) = self.row_range(book, sheet.clone()) {
                    return Ok(t);
                }
                self.word_token(book, sheet, true)
            }
            Some(c) if is_word_start(c) => self.word_token(book, sheet, true),
            _ => self.err("시트 접두사 뒤에 참조가 없음"),
        }
    }

    /// 낱말: 함수 이름, 시트 이름, 셀·영역·열 참조, 논리 상수, 이름
    fn word_token(
        &mut self,
        book: Option<u32>,
        sheet: SheetSpec,
        prefixed: bool,
    ) -> Result<Tok, ParseError> {
        let (w, j) = self.word_at(self.i);
        if w.is_empty() {
            return self.err("낱말이 없음");
        }
        let next = self.c.get(j).copied();
        if !prefixed {
            if next == Some('!') {
                self.i = j + 1;
                return self.target(None, SheetSpec::One(w));
            }
            if next == Some(':') {
                // 3차원 Sheet1:Sheet3!A1
                let (w2, k) = self.word_at(j + 1);
                if !w2.is_empty() && self.c.get(k) == Some(&'!') {
                    self.i = k + 1;
                    return self.target(None, SheetSpec::Span(w, w2));
                }
            }
            if next == Some('(') {
                self.i = j + 1;
                return Ok(Tok::Func(w));
            }
            if next == Some('[') {
                self.i = j;
                return self.structured(w);
            }
        }
        // 영역 A1:B2, A:C
        if next == Some(':') {
            let (w2, k) = self.word_at(j + 1);
            if !w2.is_empty() {
                if let (Some((r0, c0)), Some((r1, c1))) = (parse_cell(&w), parse_cell(&w2)) {
                    self.i = k;
                    return Ok(Tok::Ref(RefExpr {
                        book,
                        sheet,
                        r0: r0.min(r1),
                        r1: r0.max(r1),
                        c0: c0.min(c1),
                        c1: c0.max(c1),
                    }));
                }
                if let (Some(a), Some(b)) = (parse_col(&w), parse_col(&w2)) {
                    self.i = k;
                    return Ok(Tok::Ref(RefExpr {
                        book,
                        sheet,
                        r0: 0,
                        r1: MAX_ROWS - 1,
                        c0: a.min(b),
                        c1: a.max(b),
                    }));
                }
            }
        }
        self.i = j;
        if let Some((r, c)) = parse_cell(&w) {
            return Ok(Tok::Ref(RefExpr {
                book,
                sheet,
                r0: r,
                r1: r,
                c0: c,
                c1: c,
            }));
        }
        if !prefixed {
            if w.eq_ignore_ascii_case("TRUE") {
                return Ok(Tok::Bool(true));
            }
            if w.eq_ignore_ascii_case("FALSE") {
                return Ok(Tok::Bool(false));
            }
        }
        let sheet_name = match sheet {
            SheetSpec::Current => None,
            SheetSpec::One(s) => Some(s),
            SheetSpec::Span(a, _) => Some(a),
        };
        Ok(Tok::Name(NameExpr {
            book,
            sheet: sheet_name,
            name: w,
        }))
    }
}

// ---------------------------------------------------------------------------
// 구문
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    /// `,` 를 합집합으로 읽는가 (함수 인수 목록 안에서는 구분자)
    union_ok: bool,
}

/// 함수 이름 정규화: 대문자, `_xlfn.`·`_xlws.`·`_xll.` 접두사 제거
pub fn canonical_function_name(raw: &str) -> String {
    let mut s = raw.to_ascii_uppercase();
    loop {
        let before = s.len();
        for p in ["_XLFN.", "_XLWS.", "_XLL."] {
            if let Some(r) = s.strip_prefix(p) {
                s = r.to_string();
            }
        }
        if s.len() == before {
            break;
        }
    }
    s
}

impl Parser {
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            msg: msg.into(),
            pos: self.toks.get(self.pos).map(|t| t.1).unwrap_or(usize::MAX),
        })
    }

    fn skip_space(&mut self) {
        while matches!(self.toks.get(self.pos), Some((Tok::Space, _))) {
            self.pos += 1;
        }
    }

    /// 공백을 건너뛴 다음 토큰 (위치는 그대로 — 교집합 공백을 지킨다)
    fn peek(&self) -> Option<&Tok> {
        let mut k = self.pos;
        while matches!(self.toks.get(k), Some((Tok::Space, _))) {
            k += 1;
        }
        self.toks.get(k).map(|t| &t.0)
    }

    /// peek 로 본 토큰을 소비한다
    fn bump(&mut self) {
        self.skip_space();
        self.pos += 1;
    }

    fn next(&mut self) -> Option<Tok> {
        self.skip_space();
        let t = self.toks.get(self.pos).map(|t| t.0.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat_op(&mut self, ops: &[&'static str]) -> Option<&'static str> {
        if let Some(Tok::Op(o)) = self.peek()
            && ops.contains(o)
        {
            let o = *o;
            self.bump();
            return Some(o);
        }
        None
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.concat()?;
        while let Some(op) = self.eat_op(&["=", "<>", "<", "<=", ">", ">="]) {
            let b = self.concat()?;
            let op = match op {
                "=" => BinaryOp::Eq,
                "<>" => BinaryOp::Ne,
                "<" => BinaryOp::Lt,
                "<=" => BinaryOp::Le,
                ">" => BinaryOp::Gt,
                _ => BinaryOp::Ge,
            };
            a = Expr::Binary(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn concat(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.additive()?;
        while self.eat_op(&["&"]).is_some() {
            let b = self.additive()?;
            a = Expr::Binary(BinaryOp::Concat, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn additive(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.multiplicative()?;
        while let Some(op) = self.eat_op(&["+", "-"]) {
            let b = self.multiplicative()?;
            let op = if op == "+" {
                BinaryOp::Add
            } else {
                BinaryOp::Sub
            };
            a = Expr::Binary(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.power()?;
        while let Some(op) = self.eat_op(&["*", "/"]) {
            let b = self.power()?;
            let op = if op == "*" {
                BinaryOp::Mul
            } else {
                BinaryOp::Div
            };
            a = Expr::Binary(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn power(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.unary()?;
        while self.eat_op(&["^"]).is_some() {
            let b = self.unary()?;
            a = Expr::Binary(BinaryOp::Pow, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if let Some(op) = self.eat_op(&["-", "+"]) {
            let e = self.unary()?;
            let op = if op == "-" {
                UnaryOp::Neg
            } else {
                UnaryOp::Plus
            };
            // 숫자 상수의 음수는 상수로 접는다 (-1)
            if let (UnaryOp::Neg, Expr::Number(n)) = (op, &e) {
                return Ok(Expr::Number(-n));
            }
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.percent()
    }

    fn percent(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.union()?;
        while self.eat_op(&["%"]).is_some() {
            e = Expr::Unary(UnaryOp::Percent, Box::new(e));
        }
        Ok(e)
    }

    fn union(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.intersect()?;
        if !self.union_ok {
            return Ok(a);
        }
        while matches!(self.peek(), Some(Tok::Comma)) {
            self.bump();
            let b = self.intersect()?;
            a = Expr::Binary(BinaryOp::Union, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn intersect(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.range()?;
        loop {
            // 공백 다음이 피연산자로 시작하면 교집합
            if !matches!(self.toks.get(self.pos), Some((Tok::Space, _))) {
                break;
            }
            let mut k = self.pos;
            while matches!(self.toks.get(k), Some((Tok::Space, _))) {
                k += 1;
            }
            let starts_operand = matches!(
                self.toks.get(k).map(|t| &t.0),
                Some(Tok::Ref(_) | Tok::Name(_) | Tok::Table(_) | Tok::Func(_) | Tok::LParen)
            );
            if !starts_operand {
                break;
            }
            self.pos = k;
            let b = self.range()?;
            a = Expr::Binary(BinaryOp::Intersect, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn range(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.primary()?;
        while self.eat_op(&[":"]).is_some() {
            let b = self.primary()?;
            a = Expr::Binary(BinaryOp::Range, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        let Some(t) = self.next() else {
            return self.err("식이 끝났음");
        };
        Ok(match t {
            Tok::Num(n) => Expr::Number(n),
            Tok::Str(s) => Expr::Text(Rc::from(s)),
            Tok::Bool(b) => Expr::Bool(b),
            Tok::Err(e) => Expr::Error(e),
            Tok::Ref(r) => Expr::Ref(r),
            Tok::Name(n) => Expr::Name(n),
            Tok::Table(t) => Expr::Table(t),
            Tok::LParen => {
                let save = self.union_ok;
                self.union_ok = true;
                let e = self.expr()?;
                self.union_ok = save;
                match self.next() {
                    Some(Tok::RParen) => Expr::Paren(Box::new(e)),
                    _ => return self.err("닫는 괄호가 없음"),
                }
            }
            Tok::LBrace => self.array()?,
            Tok::Func(name) => self.call(name)?,
            // 단항 연산자가 연달아 오는 경우 (`--A1`) 는 unary 에서 처리한다
            Tok::Op(o) if o == "-" || o == "+" => {
                self.pos -= 1;
                return self.unary();
            }
            other => return self.err(format!("예상하지 못한 토큰 {other:?}")),
        })
    }

    fn call(&mut self, raw: String) -> Result<Expr, ParseError> {
        let name = canonical_function_name(&raw);
        let mut args = Vec::new();
        let save = self.union_ok;
        self.union_ok = false;
        if matches!(self.peek(), Some(Tok::RParen)) {
            self.bump();
        } else {
            loop {
                match self.peek() {
                    Some(Tok::Comma) => {
                        self.bump();
                        args.push(Expr::Missing);
                        continue;
                    }
                    Some(Tok::RParen) => {
                        self.bump();
                        args.push(Expr::Missing);
                        break;
                    }
                    _ => {}
                }
                let e = self.expr()?;
                args.push(e);
                match self.next() {
                    Some(Tok::Comma) => {
                        if matches!(self.peek(), Some(Tok::RParen)) {
                            self.bump();
                            args.push(Expr::Missing);
                            break;
                        }
                    }
                    Some(Tok::RParen) => break,
                    _ => {
                        self.union_ok = save;
                        return self.err(format!("{name} 인수 목록이 닫히지 않음"));
                    }
                }
            }
        }
        self.union_ok = save;
        let func = super::functions::lookup(&name);
        Ok(Expr::Call(Box::new(Call { name, func, args })))
    }

    fn array(&mut self) -> Result<Expr, ParseError> {
        let mut rows: Vec<Vec<Value>> = vec![Vec::new()];
        loop {
            let neg = match self.peek() {
                Some(Tok::Op("-")) => {
                    self.bump();
                    true
                }
                Some(Tok::Op("+")) => {
                    self.bump();
                    false
                }
                _ => false,
            };
            let v = match self.next() {
                Some(Tok::Num(n)) => Value::Number(if neg { -n } else { n }),
                Some(Tok::Str(s)) if !neg => Value::from(s),
                Some(Tok::Bool(b)) if !neg => Value::Bool(b),
                Some(Tok::Err(e)) if !neg => Value::Error(e),
                _ => return self.err("배열 상수의 원소가 아님"),
            };
            rows.last_mut().unwrap().push(v);
            match self.next() {
                Some(Tok::Comma) => {}
                Some(Tok::Semi) => rows.push(Vec::new()),
                Some(Tok::RBrace) => break,
                _ => return self.err("배열 상수가 닫히지 않음"),
            }
        }
        let cols = rows[0].len();
        if rows.iter().any(|r| r.len() != cols) {
            return self.err("배열 상수의 행 길이가 다름");
        }
        let n = rows.len();
        let data: Vec<Value> = rows.into_iter().flatten().collect();
        Ok(Expr::Array(Rc::new(Array::new(n, cols, data))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(r0: u32, c0: u32, r1: u32, c1: u32) -> Expr {
        Expr::Ref(RefExpr {
            book: None,
            sheet: SheetSpec::Current,
            r0,
            c0,
            r1,
            c1,
        })
    }

    #[test]
    fn precedence() {
        let e = parse("-2^2").unwrap();
        assert_eq!(
            e,
            Expr::Binary(
                BinaryOp::Pow,
                Box::new(Expr::Number(-2.0)),
                Box::new(Expr::Number(2.0))
            )
        );
        let e = parse("1+2*3").unwrap();
        assert!(matches!(e, Expr::Binary(BinaryOp::Add, _, _)));
        let e = parse("1&2=\"12\"").unwrap();
        assert!(matches!(e, Expr::Binary(BinaryOp::Eq, _, _)));
        let e = parse("A1%").unwrap();
        assert_eq!(e, Expr::Unary(UnaryOp::Percent, Box::new(r(0, 0, 0, 0))));
    }

    #[test]
    fn references() {
        assert_eq!(parse("$A$1:B2").unwrap(), r(0, 0, 1, 1));
        assert_eq!(parse("A:C").unwrap(), r(0, 0, MAX_ROWS - 1, 2));
        assert_eq!(parse("2:3").unwrap(), r(1, 0, 2, MAX_COLS - 1));
        let e = parse("'My Sheet'!B2").unwrap();
        assert!(
            matches!(e, Expr::Ref(RefExpr { sheet: SheetSpec::One(ref s), r0: 1, c0: 1, .. }) if s == "My Sheet")
        );
        let e = parse("Sheet1:Sheet3!A1").unwrap();
        assert!(matches!(
            e,
            Expr::Ref(RefExpr {
                sheet: SheetSpec::Span(..),
                ..
            })
        ));
        let e = parse("'[1]Sales Analysis'!$B$3").unwrap();
        assert!(matches!(e, Expr::Ref(RefExpr { book: Some(1), .. })));
        let e = parse("서식!$M$7").unwrap();
        assert!(matches!(e, Expr::Ref(RefExpr { sheet: SheetSpec::One(ref s), .. }) if s == "서식"));
        let e = parse("MILEAGE_RATE*2").unwrap();
        assert!(matches!(e, Expr::Binary(BinaryOp::Mul, ref a, _) if matches!(**a, Expr::Name(_))));
        let e = parse("Sheet1!#REF!").unwrap();
        assert_eq!(e, Expr::Error(ErrorKind::Ref));
    }

    #[test]
    fn calls_and_arrays() {
        let e = parse("_xlfn.STDEV.S(A1:A3,,{1,2;3,4})").unwrap();
        let Expr::Call(c) = e else { panic!() };
        assert_eq!(c.name, "STDEV.S");
        assert_eq!(c.args.len(), 3);
        assert_eq!(c.args[1], Expr::Missing);
        let Expr::Array(a) = &c.args[2] else { panic!() };
        assert_eq!((a.rows, a.cols), (2, 2));
        let e = parse("SUM((A1,B1))").unwrap();
        let Expr::Call(c) = e else { panic!() };
        assert_eq!(c.args.len(), 1);
        let e = parse("NOW()").unwrap();
        let Expr::Call(c) = e else { panic!() };
        assert!(c.args.is_empty());
        let e = parse("IF(A1,1,)").unwrap();
        let Expr::Call(c) = e else { panic!() };
        assert_eq!(c.args.len(), 3);
    }

    #[test]
    fn intersection_and_spaces() {
        let e = parse("SUM(A1:B2 B1:C3)").unwrap();
        let Expr::Call(c) = e else { panic!() };
        assert!(matches!(c.args[0], Expr::Binary(BinaryOp::Intersect, _, _)));
        let e = parse(" 1 + 2 ").unwrap();
        assert!(matches!(e, Expr::Binary(BinaryOp::Add, _, _)));
        assert!(parse("IF( A1 , 1 , 2 )").is_ok());
    }

    #[test]
    fn names_that_look_like_refs() {
        assert!(matches!(parse("R2D2").unwrap(), Expr::Name(_)));
        assert!(matches!(parse("TAX2023").unwrap(), Expr::Ref(_)));
        assert!(matches!(parse("XFE1").unwrap(), Expr::Name(_)));
    }
}
