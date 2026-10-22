// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 원문(A1 표기, `=` 없음) → 바이너리 `.cell` 수식 토큰열 (Ptg, [MS-XLSB] 2.5.97).
//!
//! [`crate::legacy::formula::decode`] 의 반대다. 토큰 배치는 한셀이 바이너리에 쓰는 꼴을 따른다
//! (함수 대조 실험 — `X.cell` 의 수식 원문과 `X.bin.cell` 의 토큰을 셀마다 비교):
//!
//! - 피연산자 토큰의 클래스(참조 0x20·값 0x40·배열 0x60)는 그 자리가 기대하는 클래스다. 연산자의 피연산자는 값,
//!   `:`·`,`·공백(교집합)의 피연산자는 참조, 함수 인자는 [`ftab::signature`]. 참조를 돌려주는 함수(`IF`,
//!   `INDEX`, `OFFSET` …)의 토큰은 자리의 클래스를, 값을 돌려주는 함수는 값 클래스를 쓴다
//! - 인자 하나인 `SUM` 은 `tAttrSum`, `IF` 는 `tAttrIf` + `tAttrGoto`, `IFERROR` 는 `tAttr(0x80)` + `tAttrGoto`,
//!   `CHOOSE` 는 `tAttrChoose` + `tAttrGoto`. 휘발성 함수가 있으면 맨 앞에 `tAttrVolatile`
//! - 0–65535 정수는 `PtgInt`, 나머지 수는 `PtgNum`. 수 앞의 단항 `-` 는 음수 `PtgNum` 하나로 접는다
//! - 고정 인자 함수를 그 개수로 부르면 `PtgFunc`, 아니면 `PtgFuncVar`
//! - 이름·이름 정의의 합집합(`A,B`)은 `PtgMemFunc` 로 감싼다
//! - 공유 수식·조건부 서식·유효성은 기준 셀에 대한 상대 토큰(`PtgRefN`/`PtgAreaN`, 행 오프셋은 20비트)

use crate::legacy::ftab::{self, ArgClass};
use crate::model::CellRef;

const MAX_ROW: u32 = 1_048_575;
const MAX_COL: u32 = 16_383;
/// PtgArray 뒤의 빈 바이트 수 (레거시 디코더와 같게 7)
const ARRAY_UNUSED: usize = 7;

/// 수식 하나를 쓰는 데 필요한 문서 정보
pub trait Resolver {
    /// `[book]first[:last]!` 의 XTI 번호. `book` 은 외부 연결 번호(1부터), 없으면 이 통합 문서.
    /// 없는 시트면 None
    fn xti(&mut self, book: Option<usize>, first: &str, last: Option<&str>) -> Option<u16>;
    /// 정의된 이름의 번호(1부터). `sheet` 는 이름 앞의 시트(`Sheet1!이름`), 없으면 수식이 있는 시트의 지역 이름
    /// → 전역 이름 순으로 찾는다. 없으면 새 이름을 만든다 (한셀처럼 수식만 없는 이름)
    fn name(&mut self, sheet: Option<&str>, name: &str) -> u32;
    /// 외부 문서의 이름 `[book]!이름` → (XTI, 이름 번호)
    fn extern_name(&mut self, book: usize, name: &str) -> Option<(u16, u32)>;
    /// 구조적 참조의 표 (표시 이름) → 표 번호, 열 이름, 표가 있는 시트의 XTI. 없는 표면 None
    fn table(&mut self, _name: &str) -> Option<ListRef> {
        None
    }
    /// 피벗 계산 필드 수식의 캐시 필드 이름 → 이름 목록(BrtBeginPName) 번호 (0부터, PtgSxName). 피벗 수식이 아니면 None
    fn pivot_field(&mut self, _name: &str) -> Option<u32> {
        None
    }
}

/// 구조적 참조(PtgList)가 가리키는 표
#[derive(Debug, Clone, PartialEq)]
pub struct ListRef {
    /// 표 번호 (`idList`)
    pub id: u32,
    /// 열 이름 (순서가 열 번호)
    pub columns: Vec<String>,
    /// 표가 있는 시트의 XTI
    pub ixti: u16,
}

/// 수식이 놓이는 곳
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// 셀 수식 (값 클래스, 절대 토큰)
    Cell,
    /// 배열 수식 (배열 클래스)
    Array,
    /// 이름 정의 (참조 클래스)
    Name,
    /// 공유 수식·조건부 서식·유효성: 이 셀에 대한 상대 토큰 (값 클래스)
    Relative(CellRef),
}

/// 토큰열 (`rgce`) 과 덧붙은 데이터 (`rgcb`, 배열 상수)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Encoded {
    pub rgce: Vec<u8>,
    pub rgcb: Vec<u8>,
    /// 휘발성 함수가 있다 (셀의 fAlwaysCalc)
    pub volatile: bool,
}

/// 원문을 토큰열로. 읽을 수 없는 원문이면 Err(이유)
pub fn encode(text: &str, place: Place, r: &mut dyn Resolver) -> Result<Encoded, String> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    let toks = lex(text)?;
    let mut p = Parser { t: &toks, i: 0 };
    let node = p.expr(true)?;
    if p.i < toks.len() {
        return Err(format!("남은 토큰 {:?}", &toks[p.i..]));
    }
    let root = match place {
        Place::Name => ArgClass::Ref,
        Place::Array => ArgClass::Array,
        _ => ArgClass::Value,
    };
    let mut g = Gen {
        out: Vec::new(),
        extra: Vec::new(),
        r,
        base: match place {
            Place::Relative(b) => Some(b),
            _ => None,
        },
        volatile: false,
    };
    g.emit(&node, root)?;
    let mut rgce = g.out;
    if g.volatile {
        let mut v = vec![0x19, 0x01, 0x00, 0x00];
        v.extend_from_slice(&rgce);
        rgce = v;
    }
    Ok(Encoded {
        rgce,
        rgcb: g.extra,
        volatile: g.volatile,
    })
}

// ---------------------------------------------------------------------------
// 낱말
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Err(u8),
    /// 연산자·괄호·구분자
    Op(&'static str),
    /// 이름·참조·함수 이름·논리값 (시트 접두는 따로)
    Word(String),
    /// `[book]first[:last]!` (book 만 있으면 외부 이름 접두)
    Sheet(SheetRef),
    /// 구조적 참조 `표[…]`: (표 이름, 바깥 대괄호 안의 글 — 작은따옴표 가림은 그대로)
    List(String, String),
    Space,
}

#[derive(Debug, Clone, PartialEq)]
struct SheetRef {
    book: Option<usize>,
    first: String,
    last: Option<String>,
}

fn error_code(s: &str) -> Option<u8> {
    Some(match s {
        "#NULL!" => 0x00,
        "#DIV/0!" => 0x07,
        "#VALUE!" => 0x0F,
        "#REF!" => 0x17,
        "#NAME?" => 0x1D,
        "#NUM!" => 0x24,
        "#N/A" => 0x2A,
        "#GETTING_DATA" => 0x2B,
        _ => return None,
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\' | '?')
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            ' ' | '\t' | '\r' | '\n' => {
                while i < c.len() && matches!(c[i], ' ' | '\t' | '\r' | '\n') {
                    i += 1;
                }
                out.push(Tok::Space);
            }
            '"' => {
                let mut v = String::new();
                i += 1;
                loop {
                    let Some(&x) = c.get(i) else {
                        return Err("닫히지 않은 문자열".into());
                    };
                    i += 1;
                    if x == '"' {
                        if c.get(i) == Some(&'"') {
                            v.push('"');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    v.push(x);
                }
                out.push(Tok::Str(v));
            }
            '\'' => {
                // 따옴표 시트 이름 ('[1]시트 1'!, '첫:끝'!)
                let mut v = String::new();
                i += 1;
                loop {
                    let Some(&x) = c.get(i) else {
                        return Err("닫히지 않은 시트 이름".into());
                    };
                    i += 1;
                    if x == '\'' {
                        if c.get(i) == Some(&'\'') {
                            v.push('\'');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                    v.push(x);
                }
                if c.get(i) != Some(&'!') {
                    return Err(format!("시트 이름 뒤에 ! 가 없음: '{v}'"));
                }
                i += 1;
                out.push(Tok::Sheet(split_sheet(&v)?));
            }
            '[' => {
                // 외부 문서 번호 [n]시트! 또는 [n]!이름
                let start = i + 1;
                let mut j = start;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                if j == start || c.get(j) != Some(&']') {
                    return Err("구조적 참조는 아직 쓰지 못한다".into());
                }
                let n: usize = c[start..j].iter().collect::<String>().parse().unwrap_or(0);
                i = j + 1;
                if c.get(i) == Some(&'!') {
                    i += 1;
                    out.push(Tok::Sheet(SheetRef {
                        book: Some(n),
                        first: String::new(),
                        last: None,
                    }));
                    continue;
                }
                let (w, end) = word(&c, i);
                if w.is_empty() {
                    return Err("외부 문서 번호 뒤에 시트 이름이 없음".into());
                }
                i = end;
                let mut last = None;
                if c.get(i) == Some(&':') {
                    let (w2, e2) = word(&c, i + 1);
                    if !w2.is_empty() && c.get(e2) == Some(&'!') {
                        last = Some(w2);
                        i = e2;
                    }
                }
                if c.get(i) != Some(&'!') {
                    return Err("외부 참조 뒤에 ! 가 없음".into());
                }
                i += 1;
                out.push(Tok::Sheet(SheetRef {
                    book: Some(n),
                    first: w,
                    last,
                }));
            }
            '#' => {
                let mut j = i + 1;
                while j < c.len() && (c[j].is_ascii_alphanumeric() || matches!(c[j], '/' | '_')) {
                    j += 1;
                }
                if j < c.len() && matches!(c[j], '!' | '?') {
                    j += 1;
                }
                let s: String = c[i..j].iter().collect();
                let code = error_code(&s.to_ascii_uppercase())
                    .ok_or_else(|| format!("알 수 없는 오류 값 {s}"))?;
                out.push(Tok::Err(code));
                i = j;
            }
            '0'..='9' | '.' if !starts_row_word(&c, i) => {
                let start = i;
                while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                    i += 1;
                }
                if i < c.len()
                    && matches!(c[i], 'e' | 'E')
                    && c.get(i + 1)
                        .is_some_and(|x| x.is_ascii_digit() || matches!(x, '+' | '-'))
                {
                    i += 2;
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let s: String = c[start..i].iter().collect();
                let v: f64 = s.parse().map_err(|_| format!("수 {s}"))?;
                out.push(Tok::Num(v));
            }
            '<' if c.get(i + 1) == Some(&'>') => {
                out.push(Tok::Op("<>"));
                i += 2;
            }
            '<' if c.get(i + 1) == Some(&'=') => {
                out.push(Tok::Op("<="));
                i += 2;
            }
            '>' if c.get(i + 1) == Some(&'=') => {
                out.push(Tok::Op(">="));
                i += 2;
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '=' | '<' | '>' | '%' | ':' | ',' | '(' | ')'
            | '{' | '}' | ';' => {
                out.push(Tok::Op(match ch {
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '^' => "^",
                    '&' => "&",
                    '=' => "=",
                    '<' => "<",
                    '>' => ">",
                    '%' => "%",
                    ':' => ":",
                    ',' => ",",
                    '(' => "(",
                    ')' => ")",
                    '{' => "{",
                    '}' => "}",
                    _ => ";",
                }));
                i += 1;
            }
            c0 if is_word_char(c0) => {
                let (w, end) = word(&c, i);
                i = end;
                // 구조적 참조: 표이름[…]
                if c.get(i) == Some(&'[') {
                    let (spec, end) = bracket(&c, i)?;
                    i = end;
                    out.push(Tok::List(w, spec));
                    continue;
                }
                // 시트 접두: 이름! 또는 첫:끝!
                if c.get(i) == Some(&'!') {
                    i += 1;
                    out.push(Tok::Sheet(SheetRef {
                        book: None,
                        first: w,
                        last: None,
                    }));
                    continue;
                }
                if c.get(i) == Some(&':') {
                    let (w2, e2) = word(&c, i + 1);
                    if !w2.is_empty() && c.get(e2) == Some(&'!') {
                        out.push(Tok::Sheet(SheetRef {
                            book: None,
                            first: w,
                            last: Some(w2),
                        }));
                        i = e2 + 1;
                        continue;
                    }
                }
                out.push(Tok::Word(w));
            }
            _ => return Err(format!("알 수 없는 글자 {ch:?}")),
        }
    }
    Ok(out)
}

/// 숫자로 시작하지만 행 참조(`1:3`, `2:$5`)의 앞쪽인지
fn starts_row_word(c: &[char], i: usize) -> bool {
    let mut j = i;
    while j < c.len() && c[j].is_ascii_digit() {
        j += 1;
    }
    j > i && c.get(j) == Some(&':') && {
        let mut k = j + 1;
        if c.get(k) == Some(&'$') {
            k += 1;
        }
        let s = k;
        while k < c.len() && c[k].is_ascii_digit() {
            k += 1;
        }
        k > s && !c.get(k).is_some_and(|x| is_word_char(*x) || *x == '(')
    }
}

/// `c[start]` 의 `[` 와 짝인 `]` 까지: (안쪽 글, `]` 다음 위치). 작은따옴표는 다음 글자를 가린다 (`'[`, `''`)
fn bracket(c: &[char], start: usize) -> Result<(String, usize), String> {
    let mut depth = 0;
    let mut j = start;
    while j < c.len() {
        match c[j] {
            '\'' => {
                j += 2;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((c[start + 1..j].iter().collect(), j + 1));
                }
            }
            _ => {}
        }
        j += 1;
    }
    Err("구조적 참조가 닫히지 않음".into())
}

/// 대괄호·작은따옴표 가림 밖의 `sep` 로 나눈다
fn split_top(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    let mut it = s.chars();
    while let Some(ch) = it.next() {
        match ch {
            '\'' => {
                cur.push(ch);
                if let Some(n) = it.next() {
                    cur.push(n);
                }
                continue;
            }
            '[' => depth += 1,
            ']' => depth -= 1,
            _ if ch == sep && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    out.push(cur);
    out
}

/// 열 이름의 작은따옴표 가림을 푼다 (`a'[1']` → `a[1]`)
fn unescape_list_name(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(ch) = it.next() {
        if ch == '\'' {
            if let Some(n) = it.next() {
                out.push(n);
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// 구조적 참조의 안쪽 글 → (행 종류 비트, 열 이름 0–2개). 행 종류: 0x01 `#All`, 0x02 `#Headers`, 0x04 `#Data`,
/// 0x08 `#Totals`, 0x10 `#This Row` (`@열` 도). 꼴: 빈 글(`표[]`), 열 하나(`표[열]`), 행 종류 하나(`표[#All]`),
/// `[…],[…]` 목록 — 항목마다 `[행 종류]`, `[열]`, `[열]:[열]`
fn parse_list_spec(spec: &str) -> Result<(u16, Vec<String>), String> {
    let s = spec.trim();
    if s.is_empty() {
        return Ok((0, Vec::new()));
    }
    let items: Vec<String> = if s.starts_with('[') {
        split_top(s, ',')
    } else {
        vec![format!("[{s}]")]
    };
    let mut rows = 0u16;
    let mut cols = Vec::new();
    for it in items {
        for part in split_top(it.trim(), ':') {
            let p = part.trim();
            let inner = p
                .strip_prefix('[')
                .and_then(|x| x.strip_suffix(']'))
                .ok_or_else(|| format!("구조적 참조 항목 {p}"))?;
            if let Some(spec) = inner.strip_prefix('#') {
                rows |= match spec.trim().to_ascii_lowercase().as_str() {
                    "all" => 0x01,
                    "headers" => 0x02,
                    "data" => 0x04,
                    "totals" => 0x08,
                    "this row" => 0x10,
                    other => return Err(format!("구조적 참조 행 종류 #{other}")),
                };
            } else if let Some(col) = inner.strip_prefix('@') {
                rows |= 0x10;
                let col = col.trim();
                let col = col
                    .strip_prefix('[')
                    .and_then(|x| x.strip_suffix(']'))
                    .unwrap_or(col);
                if !col.is_empty() {
                    cols.push(unescape_list_name(col));
                }
            } else {
                cols.push(unescape_list_name(inner));
            }
        }
    }
    if cols.len() > 2 {
        return Err(format!("구조적 참조의 열이 너무 많음: {spec}"));
    }
    Ok((rows, cols))
}

fn word(c: &[char], start: usize) -> (String, usize) {
    let mut end = start;
    while end < c.len() && is_word_char(c[end]) {
        end += 1;
    }
    (c[start..end].iter().collect(), end)
}

/// 따옴표 안의 `[n]첫[:끝]`
fn split_sheet(v: &str) -> Result<SheetRef, String> {
    let (book, rest) = match v.strip_prefix('[') {
        Some(r) => {
            let (n, rest) = r.split_once(']').ok_or("외부 문서 번호가 닫히지 않음")?;
            (
                Some(
                    n.parse::<usize>()
                        .map_err(|_| format!("외부 문서 번호 {n}"))?,
                ),
                rest,
            )
        }
        None => (None, v),
    };
    // 시트 이름에 ':' 가 들어갈 수 없으므로 ':' 는 시트 범위다
    let (first, last) = match rest.split_once(':') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (rest.to_string(), None),
    };
    Ok(SheetRef { book, first, last })
}

// ---------------------------------------------------------------------------
// 구문 나무
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct Axis {
    n: u32,
    abs: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Area {
    Cell(Axis, Axis),
    /// (첫 열, 첫 행), (끝 열, 끝 행)
    Range(Axis, Axis, Axis, Axis),
    Cols(Axis, Axis),
    Rows(Axis, Axis),
    /// `#REF!` 참조 (시트 접두 뒤)
    Err,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(u8),
    Missing,
    Ref(Option<SheetRef>, Area),
    Name(Option<SheetRef>, String),
    /// 구조적 참조 (표 이름, 안쪽 글)
    List(String, String),
    Func(String, Vec<Node>),
    Neg(Box<Node>),
    Plus(Box<Node>),
    Percent(Box<Node>),
    Bin(&'static str, Box<Node>, Box<Node>),
    Paren(Box<Node>),
    Array(Vec<Vec<Node>>),
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.t.get(self.i) == Some(&Tok::Space) {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<&Tok> {
        self.skip_space();
        self.t.get(self.i)
    }
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if *o == op) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// 식. `union` 이면 최상위 `,` 를 합집합으로 읽는다 (괄호 안·이름 정의)
    fn expr(&mut self, union: bool) -> Result<Node, String> {
        let first = self.compare()?;
        if !union {
            return Ok(first);
        }
        let mut n = first;
        while self.eat(",") {
            let r = self.compare()?;
            n = Node::Bin(",", Box::new(n), Box::new(r));
        }
        Ok(n)
    }

    fn compare(&mut self) -> Result<Node, String> {
        let mut n = self.concat()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o @ ("=" | "<>" | "<" | ">" | "<=" | ">="))) => *o,
                _ => return Ok(n),
            };
            self.i += 1;
            let r = self.concat()?;
            n = Node::Bin(op, Box::new(n), Box::new(r));
        }
    }

    fn concat(&mut self) -> Result<Node, String> {
        let mut n = self.additive()?;
        while self.eat("&") {
            let r = self.additive()?;
            n = Node::Bin("&", Box::new(n), Box::new(r));
        }
        Ok(n)
    }

    fn additive(&mut self) -> Result<Node, String> {
        let mut n = self.mult()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o @ ("+" | "-"))) => *o,
                _ => return Ok(n),
            };
            self.i += 1;
            let r = self.mult()?;
            n = Node::Bin(op, Box::new(n), Box::new(r));
        }
    }

    fn mult(&mut self) -> Result<Node, String> {
        let mut n = self.power()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o @ ("*" | "/"))) => *o,
                _ => return Ok(n),
            };
            self.i += 1;
            let r = self.power()?;
            n = Node::Bin(op, Box::new(n), Box::new(r));
        }
    }

    fn power(&mut self) -> Result<Node, String> {
        let mut n = self.percent()?;
        while self.eat("^") {
            let r = self.percent()?;
            n = Node::Bin("^", Box::new(n), Box::new(r));
        }
        Ok(n)
    }

    fn percent(&mut self) -> Result<Node, String> {
        let mut n = self.unary()?;
        while self.eat("%") {
            n = Node::Percent(Box::new(n));
        }
        Ok(n)
    }

    fn unary(&mut self) -> Result<Node, String> {
        if self.eat("-") {
            return Ok(Node::Neg(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return Ok(Node::Plus(Box::new(self.unary()?)));
        }
        self.isect()
    }

    /// 교집합 (참조 사이의 공백)
    fn isect(&mut self) -> Result<Node, String> {
        let mut n = self.range()?;
        loop {
            // 공백 뒤에 피연산자가 바로 오면 교집합
            if self.t.get(self.i) != Some(&Tok::Space) {
                return Ok(n);
            }
            let save = self.i;
            self.skip_space();
            let starts_operand = matches!(
                self.t.get(self.i),
                Some(Tok::Word(_)) | Some(Tok::Sheet(_)) | Some(Tok::List(..)) | Some(Tok::Op("("))
            );
            if !starts_operand || !is_reference(&n) {
                self.i = save;
                return Ok(n);
            }
            let r = self.range()?;
            n = Node::Bin(" ", Box::new(n), Box::new(r));
        }
    }

    /// 범위 연산자 `:` (같은 시트의 셀 둘, 열 둘, 행 둘은 하나의 참조로 합친다)
    fn range(&mut self) -> Result<Node, String> {
        let mut n = self.primary()?;
        while matches!(self.t.get(self.i), Some(Tok::Op(":"))) {
            self.i += 1;
            let r = self.primary_after_colon(&n)?;
            n = match (n, r) {
                (Node::Ref(s, Area::Cell(c1, r1)), Node::Ref(None, Area::Cell(c2, r2))) => {
                    Node::Ref(s, Area::Range(c1, r1, c2, r2))
                }
                (Node::Ref(s, Area::Cols(c1, _)), Node::Ref(None, Area::Cols(c2, _))) => {
                    Node::Ref(s, Area::Cols(c1, c2))
                }
                (Node::Ref(s, Area::Rows(r1, _)), Node::Ref(None, Area::Rows(r2, _))) => {
                    Node::Ref(s, Area::Rows(r1, r2))
                }
                (a, b) => Node::Bin(":", Box::new(a), Box::new(b)),
            };
        }
        Ok(n)
    }

    /// `:` 뒤: 앞이 열/행만이면 열/행 범위를 만든다
    fn primary_after_colon(&mut self, left: &Node) -> Result<Node, String> {
        if let (Node::Ref(_, Area::Cols(..)), Some(Tok::Word(w))) = (left, self.t.get(self.i))
            && let Some(c) = parse_col(w)
        {
            self.i += 1;
            return Ok(Node::Ref(None, Area::Cols(c, c)));
        }
        if let Node::Ref(_, Area::Rows(..)) = left {
            match self.t.get(self.i) {
                Some(Tok::Word(w)) if parse_row(w).is_some() => {
                    let r = parse_row(w).unwrap();
                    self.i += 1;
                    return Ok(Node::Ref(None, Area::Rows(r, r)));
                }
                Some(Tok::Num(v)) if v.fract() == 0.0 && *v >= 1.0 => {
                    let r = Axis {
                        n: *v as u32 - 1,
                        abs: false,
                    };
                    self.i += 1;
                    return Ok(Node::Ref(None, Area::Rows(r, r)));
                }
                _ => {}
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Node, String> {
        self.skip_space();
        let Some(t) = self.t.get(self.i).cloned() else {
            return Err("식이 끝남".into());
        };
        self.i += 1;
        match t {
            Tok::Num(v) => {
                // 행 범위 1:3
                if matches!(self.t.get(self.i), Some(Tok::Op(":"))) && v.fract() == 0.0 && v >= 1.0
                {
                    return Ok(Node::Ref(
                        None,
                        Area::Rows(
                            Axis {
                                n: v as u32 - 1,
                                abs: false,
                            },
                            Axis {
                                n: v as u32 - 1,
                                abs: false,
                            },
                        ),
                    ));
                }
                Ok(Node::Num(v))
            }
            Tok::Str(s) => Ok(Node::Str(s)),
            Tok::Err(e) => Ok(Node::Err(e)),
            Tok::List(t, s) => Ok(Node::List(t, s)),
            Tok::Op("(") => {
                let n = self.expr(true)?;
                if !self.eat(")") {
                    return Err("괄호가 닫히지 않음".into());
                }
                Ok(Node::Paren(Box::new(n)))
            }
            Tok::Op("{") => self.array(),
            Tok::Sheet(s) => {
                self.skip_space();
                match self.t.get(self.i).cloned() {
                    Some(Tok::Err(0x17)) => {
                        self.i += 1;
                        Ok(Node::Ref(Some(s), Area::Err))
                    }
                    Some(Tok::Word(w)) => {
                        self.i += 1;
                        Ok(self.word_node(Some(s), w))
                    }
                    Some(Tok::Num(v))
                        if v.fract() == 0.0
                            && v >= 1.0
                            && matches!(self.t.get(self.i + 1), Some(Tok::Op(":"))) =>
                    {
                        self.i += 1;
                        let r = Axis {
                            n: v as u32 - 1,
                            abs: false,
                        };
                        Ok(Node::Ref(Some(s), Area::Rows(r, r)))
                    }
                    other => Err(format!("시트 접두 뒤에 참조가 없음: {other:?}")),
                }
            }
            Tok::Word(w) => {
                if matches!(self.t.get(self.i), Some(Tok::Op("("))) {
                    self.i += 1;
                    let args = self.args()?;
                    return Ok(Node::Func(w, args));
                }
                Ok(self.word_node(None, w))
            }
            Tok::Op("-") => Ok(Node::Neg(Box::new(self.unary()?))),
            Tok::Op("+") => Ok(Node::Plus(Box::new(self.unary()?))),
            other => Err(format!("뜻밖의 토큰 {other:?}")),
        }
    }

    fn word_node(&mut self, sheet: Option<SheetRef>, w: String) -> Node {
        let up = w.to_ascii_uppercase();
        if sheet.is_none() && (up == "TRUE" || up == "FALSE") {
            return Node::Bool(up == "TRUE");
        }
        if let Some((c, r)) = parse_cell(&w) {
            return Node::Ref(sheet, Area::Cell(c, r));
        }
        let next_colon = matches!(self.t.get(self.i), Some(Tok::Op(":")));
        if next_colon {
            if let Some(c) = parse_col(&w) {
                return Node::Ref(sheet, Area::Cols(c, c));
            }
            if let Some(r) = parse_row(&w) {
                return Node::Ref(sheet, Area::Rows(r, r));
            }
        }
        Node::Name(sheet, w)
    }

    /// 함수 인자: `(` 뒤부터 `)` 까지. 빈 인자는 Missing
    fn args(&mut self) -> Result<Vec<Node>, String> {
        let mut v = Vec::new();
        if self.eat(")") {
            return Ok(v);
        }
        loop {
            if matches!(self.peek(), Some(Tok::Op("," | ")"))) {
                v.push(Node::Missing);
            } else {
                v.push(self.expr(false)?);
            }
            if self.eat(",") {
                continue;
            }
            if self.eat(")") {
                return Ok(v);
            }
            return Err("함수 인자 뒤에 , 나 ) 가 없음".into());
        }
    }

    /// 배열 상수 `{1,2;3,4}`
    fn array(&mut self) -> Result<Node, String> {
        let mut rows = vec![Vec::new()];
        loop {
            let neg = self.eat("-");
            let v = match self.peek().cloned() {
                Some(Tok::Num(n)) => Node::Num(if neg { -n } else { n }),
                Some(Tok::Str(s)) if !neg => Node::Str(s),
                Some(Tok::Err(e)) if !neg => Node::Err(e),
                Some(Tok::Word(w))
                    if !neg && matches!(w.to_ascii_uppercase().as_str(), "TRUE" | "FALSE") =>
                {
                    Node::Bool(w.eq_ignore_ascii_case("TRUE"))
                }
                other => return Err(format!("배열 상수 {other:?}")),
            };
            self.i += 1;
            rows.last_mut().unwrap().push(v);
            if self.eat(",") {
                continue;
            }
            if self.eat(";") {
                rows.push(Vec::new());
                continue;
            }
            if self.eat("}") {
                break;
            }
            return Err("배열 상수가 닫히지 않음".into());
        }
        Ok(Node::Array(rows))
    }
}

fn is_reference(n: &Node) -> bool {
    match n {
        Node::Ref(..) | Node::Name(..) | Node::List(..) => true,
        Node::Bin(op, ..) => matches!(*op, ":" | " " | ","),
        Node::Paren(x) => is_reference(x),
        Node::Func(name, _) => {
            ftab::by_name(name).is_some_and(|(i, _)| ftab::signature(i).ret == ArgClass::Ref)
        }
        _ => false,
    }
}

fn parse_col(w: &str) -> Option<Axis> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    if s.is_empty() || s.len() > 3 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for c in s.chars() {
        n = n * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    (n >= 1 && n - 1 <= MAX_COL).then_some(Axis { n: n - 1, abs })
}

fn parse_row(w: &str) -> Option<Axis> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    if s.is_empty() || s.len() > 7 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = s.parse().ok()?;
    (n >= 1 && n - 1 <= MAX_ROW).then_some(Axis { n: n - 1, abs })
}

/// `$A$1` 꼴 셀 참조 → (열, 행)
fn parse_cell(w: &str) -> Option<(Axis, Axis)> {
    let b = w.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'$') {
        i = 1;
    }
    let ls = i;
    while i < b.len() && b[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == ls {
        return None;
    }
    let col = parse_col(&w[..i])?;
    let row = parse_row(&w[i..])?;
    Some((col, row))
}

// ---------------------------------------------------------------------------
// 토큰 만들기
// ---------------------------------------------------------------------------

fn class_bits(c: ArgClass) -> u8 {
    match c {
        ArgClass::Ref => 0x20,
        ArgClass::Value => 0x40,
        ArgClass::Array => 0x60,
    }
}

/// 연산자의 피연산자: 값 (배열 자리면 배열)
fn operand(want: ArgClass) -> ArgClass {
    if want == ArgClass::Array {
        ArgClass::Array
    } else {
        ArgClass::Value
    }
}

struct Gen<'a> {
    out: Vec<u8>,
    extra: Vec<u8>,
    r: &'a mut dyn Resolver,
    /// 상대 토큰의 기준 셀
    base: Option<CellRef>,
    volatile: bool,
}

impl Gen<'_> {
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn patch16(&mut self, at: usize, v: usize) {
        let v = u16::try_from(v).unwrap_or(u16::MAX);
        self.out[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn emit(&mut self, n: &Node, want: ArgClass) -> Result<(), String> {
        match n {
            Node::Num(v) => self.number(*v),
            Node::Str(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                let len = u16::try_from(units.len()).map_err(|_| "문자열이 너무 김")?;
                self.out.push(0x17);
                self.u16(len);
                for u in units {
                    self.u16(u);
                }
            }
            Node::Bool(b) => self.out.extend_from_slice(&[0x1D, *b as u8]),
            Node::Err(e) => self.out.extend_from_slice(&[0x1C, *e]),
            Node::Missing => self.out.push(0x16),
            Node::Ref(sheet, area) => self.reference(sheet.as_ref(), area, want)?,
            Node::Name(sheet, name) => self.name(sheet.as_ref(), name, want)?,
            Node::List(table, spec) => self.list(table, spec)?,
            Node::Neg(x) => {
                if let Node::Num(v) = x.as_ref() {
                    // 수 앞의 '-' 는 음수 하나로 (한셀: ABS(-5) → PtgNum -5)
                    self.number(-v);
                } else {
                    self.emit(x, operand(want))?;
                    self.out.push(0x13);
                }
            }
            Node::Plus(x) => {
                self.emit(x, operand(want))?;
                self.out.push(0x12);
            }
            Node::Percent(x) => {
                self.emit(x, operand(want))?;
                self.out.push(0x14);
            }
            Node::Paren(x) => {
                self.emit(x, want)?;
                self.out.push(0x15);
            }
            Node::Bin(op, a, b) => {
                let ptg = match *op {
                    "+" => 0x03,
                    "-" => 0x04,
                    "*" => 0x05,
                    "/" => 0x06,
                    "^" => 0x07,
                    "&" => 0x08,
                    "<" => 0x09,
                    "<=" => 0x0A,
                    "=" => 0x0B,
                    ">=" => 0x0C,
                    ">" => 0x0D,
                    "<>" => 0x0E,
                    " " => 0x0F,
                    "," => 0x10,
                    _ => 0x11,
                };
                if ptg >= 0x0F {
                    // 참조 연산자: 피연산자는 참조. 합집합은 PtgMemFunc 로 감싼다
                    let mem = ptg == 0x10 && !matches!(want, ArgClass::Array);
                    let start = self.out.len();
                    if mem {
                        self.out.extend_from_slice(&[0x09 | class_bits(want), 0, 0]);
                    }
                    self.emit(a, ArgClass::Ref)?;
                    self.emit(b, ArgClass::Ref)?;
                    self.out.push(ptg);
                    if mem {
                        let len = self.out.len() - start - 3;
                        self.patch16(start + 1, len);
                    }
                } else {
                    self.emit(a, operand(want))?;
                    self.emit(b, operand(want))?;
                    self.out.push(ptg);
                }
            }
            Node::Func(name, args) => self.func(name, args, want)?,
            Node::Array(rows) => self.array(rows, want)?,
        }
        Ok(())
    }

    fn number(&mut self, v: f64) {
        if v.fract() == 0.0 && (0.0..=65535.0).contains(&v) && !(v == 0.0 && v.is_sign_negative()) {
            self.out.push(0x1E);
            self.u16(v as u16);
        } else {
            self.out.push(0x1F);
            self.out.extend_from_slice(&v.to_le_bytes());
        }
    }

    /// 열 필드: 14비트 열 + 0x4000 열 상대 + 0x8000 행 상대
    fn col_field(col: u32, col_rel: bool, row_rel: bool) -> u16 {
        (col as u16 & 0x3FFF) | if col_rel { 0x4000 } else { 0 } | if row_rel { 0x8000 } else { 0 }
    }

    fn reference(
        &mut self,
        sheet: Option<&SheetRef>,
        area: &Area,
        want: ArgClass,
    ) -> Result<(), String> {
        let cls = class_bits(want);
        let ixti = match sheet {
            Some(s) => match self.r.xti(s.book, &s.first, s.last.as_deref()) {
                Some(x) => Some(x),
                None => {
                    // 없는 시트: 오류 참조
                    self.out.extend_from_slice(&[0x1C, 0x17]);
                    return Ok(());
                }
            },
            None => None,
        };
        // (첫 행, 끝 행, 첫 열 필드, 끝 열 필드), 셀 하나면 None
        let (r1, r2, c1, c2, single) = match *area {
            Area::Err => {
                match ixti {
                    Some(x) => {
                        self.out.push(0x1C | cls);
                        self.u16(x);
                        self.out.extend_from_slice(&[0; 6]);
                    }
                    None => {
                        self.out.push(0x0A | cls);
                        self.out.extend_from_slice(&[0; 6]);
                    }
                }
                return Ok(());
            }
            Area::Cell(c, r) => (r, r, c, c, true),
            Area::Range(c1, r1, c2, r2) => {
                // 왼쪽 위 → 오른쪽 아래로
                let (r1, r2) = if r1.n <= r2.n { (r1, r2) } else { (r2, r1) };
                let (c1, c2) = if c1.n <= c2.n { (c1, c2) } else { (c2, c1) };
                (r1, r2, c1, c2, false)
            }
            Area::Cols(c1, c2) => {
                let (c1, c2) = if c1.n <= c2.n { (c1, c2) } else { (c2, c1) };
                let top = Axis { n: 0, abs: true };
                let bottom = Axis {
                    n: MAX_ROW,
                    abs: true,
                };
                (top, bottom, c1, c2, false)
            }
            Area::Rows(r1, r2) => {
                let (r1, r2) = if r1.n <= r2.n { (r1, r2) } else { (r2, r1) };
                let left = Axis { n: 0, abs: true };
                let right = Axis {
                    n: MAX_COL,
                    abs: true,
                };
                (r1, r2, left, right, false)
            }
        };
        let any_rel = !(r1.abs && r2.abs && c1.abs && c2.abs);
        // 상대 위치 수식이면 상대 축은 기준 셀에서의 오프셋. 3차원 참조도 PtgRef3d/PtgArea3d 그대로
        // 오프셋을 쓴다 (RgceLocRel — 한셀 바이너리 대조 실험, legacy::formula::decode_relative)
        let rel = self.base.filter(|_| any_rel);
        let row_v = |r: Axis| -> u32 {
            match rel {
                Some(b) if !r.abs => (r.n as i64 - b.row as i64).rem_euclid(1 << 20) as u32,
                _ => r.n,
            }
        };
        let col_v = |c: Axis, row: Axis| -> u16 {
            let n = match rel {
                Some(b) if !c.abs => (c.n as i64 - b.col as i64).rem_euclid(1 << 14) as u32,
                _ => c.n,
            };
            Self::col_field(n, !c.abs, !row.abs)
        };
        let base_ptg = match (single, ixti.is_some(), rel.is_some()) {
            (true, true, _) => 0x1A,
            (false, true, _) => 0x1B,
            (true, false, true) => 0x0C,
            (false, false, true) => 0x0D,
            (true, false, false) => 0x04,
            (false, false, false) => 0x05,
        };
        self.out.push(base_ptg | cls);
        if let Some(x) = ixti {
            self.u16(x);
        }
        if single {
            self.u32(row_v(r1));
            let f = col_v(c1, r1);
            self.u16(f);
        } else {
            self.u32(row_v(r1));
            self.u32(row_v(r2));
            let f1 = col_v(c1, r1);
            let f2 = col_v(c2, r2);
            self.u16(f1);
            self.u16(f2);
        }
        Ok(())
    }

    /// PtgList (0x18 0x19): `ixti u16, 플래그 u16, 표 번호 u32, 첫 열 u16, 끝 열 u16` — 한셀이 쓰는 꼴 (표 대조 실험:
    /// tbl2 의 `Table1[…]` 수식 16개). 플래그 비트 0–1 열 (0 없음, 1 하나 — 끝 열 0, 2 범위), 비트 2–6 행 종류, 비트
    /// 10–11 형식은 `#This Row` 에만 1 (나머지는 함수 인자·연산자 자리와 상관없이 0). `ixti` 는 표가 있는 시트의 XTI
    fn list(&mut self, table: &str, spec: &str) -> Result<(), String> {
        let l = self
            .r
            .table(table)
            .ok_or_else(|| format!("없는 표 {table}"))?;
        let (rows, cols) = parse_list_spec(spec)?;
        let index = |name: &str| -> Result<u16, String> {
            l.columns
                .iter()
                .position(|c| c == name)
                .or_else(|| {
                    l.columns
                        .iter()
                        .position(|c| c.to_lowercase() == name.to_lowercase())
                })
                .map(|i| i as u16)
                .ok_or_else(|| format!("표 {table} 에 없는 열 {name}"))
        };
        let (kind, c1, c2) = match cols.as_slice() {
            [] => (0u16, 0u16, 0u16),
            [a] => (1, index(a)?, 0),
            [a, b] => (2, index(a)?, index(b)?),
            _ => unreachable!(),
        };
        let mut flags = kind | (rows << 2);
        if rows == 0x10 {
            flags |= 0x0400;
        }
        self.out.extend_from_slice(&[0x18, 0x19]);
        self.u16(l.ixti);
        self.u16(flags);
        self.u32(l.id);
        self.u16(c1);
        self.u16(c2);
        Ok(())
    }

    fn name(&mut self, sheet: Option<&SheetRef>, name: &str, want: ArgClass) -> Result<(), String> {
        // 피벗 계산 필드 수식의 필드 이름: PtgSxName (0x18 0x1D, 이름 목록 번호 u32 — 피벗 실험 v_calc)
        if sheet.is_none()
            && let Some(i) = self.r.pivot_field(name)
        {
            self.out.extend_from_slice(&[0x18, 0x1D]);
            self.u32(i);
            return Ok(());
        }
        // 이름만 쓴 표 (`COUNTA(Table1)`)는 열 없는 구조적 참조다 — 한셀은 XML 에 `Table1[]` 로 쓴다 (표 대조 실험)
        if sheet.is_none() && self.r.table(name).is_some() {
            return self.list(name, "");
        }
        let cls = class_bits(want);
        match sheet {
            Some(SheetRef {
                book: Some(b),
                first,
                ..
            }) if first.is_empty() => {
                let (x, i) = self
                    .r
                    .extern_name(*b, name)
                    .ok_or_else(|| format!("외부 이름 [{b}]!{name}"))?;
                self.out.push(0x19 | cls);
                self.u16(x);
                self.u32(i);
            }
            Some(s) if s.book.is_some() => {
                return Err(format!("외부 문서의 시트 이름 {name}"));
            }
            other => {
                let i = self.r.name(other.map(|s| s.first.as_str()), name);
                self.out.push(0x03 | cls);
                self.u32(i);
            }
        }
        Ok(())
    }

    fn func(&mut self, name: &str, args: &[Node], want: ArgClass) -> Result<(), String> {
        let up = name.to_ascii_uppercase();
        let plain = up.strip_prefix("_XLFN.").unwrap_or(&up);
        let Some((idx, fixed)) = ftab::by_name(plain) else {
            return self.udf(name, args, want);
        };
        let sig = ftab::signature(idx);
        if ftab::is_volatile(idx) {
            self.volatile = true;
        }
        let ret = match sig.ret {
            ArgClass::Ref => want,
            _ => operand(want),
        };
        let cls = class_bits(ret);
        let argc = args.len();
        // 인자 하나인 SUM: tAttrSum
        if idx == 4 && argc == 1 {
            self.emit(&args[0], sig.param(0))?;
            self.out.extend_from_slice(&[0x19, 0x10, 0x00, 0x00]);
            return Ok(());
        }
        // IF: tAttrIf, tAttrGoto
        if idx == 1 && (2..=3).contains(&argc) {
            self.emit(&args[0], sig.param(0))?;
            let at_if = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x02, 0, 0]);
            self.emit(&args[1], sig.param(1))?;
            let mut gotos = vec![self.out.len()];
            self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            let skip = self.out.len() - (at_if + 4);
            self.patch16(at_if + 2, skip);
            if argc == 3 {
                self.emit(&args[2], sig.param(2))?;
                gotos.push(self.out.len());
                self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            }
            self.out
                .extend_from_slice(&[0x02 | cls, argc as u8, 0x01, 0x00]);
            let end = self.out.len();
            for g in gotos {
                self.patch16(g + 2, end - (g + 4) - 1);
            }
            return Ok(());
        }
        // IFERROR: tAttr 0x80 (오류 쪽 길이), tAttrGoto
        if idx == 480 && argc == 2 {
            self.emit(&args[0], sig.param(0))?;
            let at = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x80, 0, 0]);
            let start = self.out.len();
            self.emit(&args[1], sig.param(1))?;
            let len = self.out.len() - start;
            self.patch16(at + 2, len);
            let g = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            self.out.push(0x01 | cls);
            self.u16(idx);
            let end = self.out.len();
            self.patch16(g + 2, end - (g + 4) - 1);
            return Ok(());
        }
        // CHOOSE: tAttrChoose (선택지 수 + 오프셋 표), 선택지마다 tAttrGoto
        if idx == 100 && argc >= 2 {
            self.emit(&args[0], sig.param(0))?;
            let n = argc - 1;
            let at = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x04]);
            self.u16(n as u16);
            let table = self.out.len();
            self.out.resize(table + 2 * (n + 1), 0);
            let mut gotos = Vec::new();
            for (k, a) in args[1..].iter().enumerate() {
                let off = self.out.len() - table;
                self.patch16(table + 2 * k, off);
                self.emit(a, sig.param(k + 1))?;
                gotos.push(self.out.len());
                self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            }
            let off = self.out.len() - table;
            self.patch16(table + 2 * n, off);
            let _ = at;
            self.out.extend_from_slice(&[0x02 | cls, argc as u8]);
            self.u16(idx);
            let end = self.out.len();
            for g in gotos {
                self.patch16(g + 2, end - (g + 4) - 1);
            }
            return Ok(());
        }
        for (k, a) in args.iter().enumerate() {
            self.emit(a, sig.param(k))?;
        }
        if fixed == Some(argc as u8) {
            self.out.push(0x01 | cls);
            self.u16(idx);
        } else {
            let argc = u8::try_from(argc).map_err(|_| "인자가 너무 많음")?;
            self.out.extend_from_slice(&[0x02 | cls, argc]);
            self.u16(idx);
        }
        Ok(())
    }

    /// 표에 없는 함수: 이름(PtgName) + PtgFuncVar(255)
    fn udf(&mut self, name: &str, args: &[Node], want: ArgClass) -> Result<(), String> {
        let i = self.r.name(None, name);
        self.out.push(0x23);
        self.u32(i);
        for a in args {
            self.emit(a, ArgClass::Ref)?;
        }
        let argc = u8::try_from(args.len() + 1).map_err(|_| "인자가 너무 많음")?;
        self.out
            .extend_from_slice(&[0x02 | class_bits(operand(want)), argc, 0xFF, 0x00]);
        Ok(())
    }

    /// 배열 상수: PtgArray + rgcb 의 PtgExtraArray (열 수 u32, 행 수 u32, 값마다 형식 u8 + 값)
    fn array(&mut self, rows: &[Vec<Node>], want: ArgClass) -> Result<(), String> {
        let cols = rows.first().map_or(0, Vec::len);
        if cols == 0 || rows.iter().any(|r| r.len() != cols) {
            return Err("배열 상수의 행 길이가 다름".into());
        }
        // PtgArray 는 토큰 번호가 곧 클래스 비트다 (0x20/0x40/0x60)
        let cls = if want == ArgClass::Ref {
            ArgClass::Array
        } else {
            want
        };
        self.out.push(class_bits(cls));
        self.out.extend_from_slice(&[0; ARRAY_UNUSED]);
        self.extra
            .extend_from_slice(&(rows.len() as u32).to_le_bytes());
        self.extra.extend_from_slice(&(cols as u32).to_le_bytes());
        for r in rows {
            for v in r {
                match v {
                    Node::Num(n) => {
                        self.extra.push(0x00);
                        self.extra.extend_from_slice(&n.to_le_bytes());
                    }
                    Node::Str(s) => {
                        let units: Vec<u16> = s.encode_utf16().collect();
                        self.extra.push(0x01);
                        self.extra
                            .extend_from_slice(&(units.len() as u16).to_le_bytes());
                        for u in units {
                            self.extra.extend_from_slice(&u.to_le_bytes());
                        }
                    }
                    Node::Bool(b) => {
                        self.extra.push(0x02);
                        self.extra.push(*b as u8);
                    }
                    Node::Err(e) => {
                        self.extra.push(0x04);
                        self.extra.push(*e);
                    }
                    _ => return Err("배열 상수 값".into()),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 시트 Sheet1, Sheet2 / 이름은 차례로 번호를 준다 / 외부 연결 없음
    struct R {
        names: Vec<String>,
        xtis: Vec<(Option<usize>, String)>,
    }

    impl Resolver for R {
        fn xti(&mut self, book: Option<usize>, first: &str, _last: Option<&str>) -> Option<u16> {
            if book.is_none() && !["Sheet1", "Sheet2", "첫 시트"].contains(&first) {
                return None;
            }
            let k = (book, first.to_string());
            Some(match self.xtis.iter().position(|x| *x == k) {
                Some(i) => i as u16,
                None => {
                    self.xtis.push(k);
                    self.xtis.len() as u16 - 1
                }
            })
        }
        fn name(&mut self, _sheet: Option<&str>, name: &str) -> u32 {
            match self.names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                Some(i) => i as u32 + 1,
                None => {
                    self.names.push(name.to_string());
                    self.names.len() as u32
                }
            }
        }
        fn extern_name(&mut self, _book: usize, _name: &str) -> Option<(u16, u32)> {
            None
        }
        /// 표 대조 실험의 Table1 (Sheet1, 열 이름·수량·단가·금액)
        fn table(&mut self, name: &str) -> Option<ListRef> {
            if name != "Table1" {
                return None;
            }
            Some(ListRef {
                id: 1,
                columns: ["이름", "수량", "단가", "금액"].map(String::from).to_vec(),
                ixti: self.xti(None, "Sheet1", None)?,
            })
        }
    }

    // 표 대조 실험 (tbl2): 한셀이 바이너리에 쓴 토큰열
    #[test]
    fn structured_references_match_hancell_tokens() {
        let cases = [
            (
                "SUM(Table1[수량])",
                "18 19 00 00 01 00 01 00 00 00 01 00 00 00 19 10 00 00",
            ),
            (
                "Table1[[#This Row],[수량]]*Table1[[#This Row],[단가]]",
                "18 19 00 00 41 04 01 00 00 00 01 00 00 00 18 19 00 00 41 04 01 00 00 00 02 00 00 00 05",
            ),
            (
                "ROWS(Table1[#All])",
                "18 19 00 00 04 00 01 00 00 00 00 00 00 00 41 4c 00",
            ),
            (
                "COUNTA(Table1[#Headers])",
                "18 19 00 00 08 00 01 00 00 00 00 00 00 00 42 01 a9 00",
            ),
            (
                "SUM(Table1[#Totals])",
                "18 19 00 00 20 00 01 00 00 00 00 00 00 00 19 10 00 00",
            ),
            (
                "SUM(Table1[[#Data],[수량]:[단가]])",
                "18 19 00 00 12 00 01 00 00 00 01 00 02 00 19 10 00 00",
            ),
            (
                "SUBTOTAL(109,Table1[수량])",
                "1e 6d 00 18 19 00 00 01 00 01 00 00 00 01 00 00 00 42 02 58 01",
            ),
            (
                "SUM(Table1[금액])/2",
                "18 19 00 00 01 00 01 00 00 00 03 00 00 00 19 10 00 00 1e 02 00 06",
            ),
            (
                "SUM(Table1[[#Data],[#Headers],[수량]])",
                "18 19 00 00 19 00 01 00 00 00 01 00 00 00 19 10 00 00",
            ),
            (
                "COUNTA(Table1[])",
                "18 19 00 00 00 00 01 00 00 00 00 00 00 00 42 01 a9 00",
            ),
            (
                "COUNTA(Table1)",
                "18 19 00 00 00 00 01 00 00 00 00 00 00 00 42 01 a9 00",
            ),
            (
                "SUM(Table1[[#Data],[#Totals],[금액]])",
                "18 19 00 00 31 00 01 00 00 00 03 00 00 00 19 10 00 00",
            ),
            (
                "INDEX(Table1[수량],1)",
                "18 19 00 00 01 00 01 00 00 00 01 00 00 00 1e 01 00 42 02 1d 00",
            ),
        ];
        for (f, want) in cases {
            assert_eq!(enc(f), hex(want), "{f}");
        }
        assert_eq!(
            parse_list_spec("[#This Row],[a'[1']]").unwrap(),
            (0x10, vec!["a[1]".to_string()])
        );
        assert_eq!(
            parse_list_spec("@수량").unwrap(),
            (0x10, vec!["수량".to_string()])
        );
    }

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    fn enc(f: &str) -> Vec<u8> {
        let mut r = R {
            names: vec!["MILEAGE_RATE".into()],
            xtis: Vec::new(),
        };
        encode(f, Place::Cell, &mut r).unwrap().rgce
    }

    // 아래 토큰열은 한셀 2024 가 저장한 바이너리 .cell 에서 관측한 그대로다 (formulas/values/서식 파일).
    #[test]
    fn matches_hancell_tokens() {
        let cases = [
            (
                "A1+A2*A3-A4/A5",
                "44 00 00 00 00 00 c0 44 01 00 00 00 00 c0 44 02 00 00 00 00 c0 05 03 44 03 00 00 00 00 c0 44 04 00 00 00 00 c0 06 04",
            ),
            (
                "SUM(A1:A5)",
                "25 00 00 00 00 04 00 00 00 00 c0 00 c0 19 10 00 00",
            ),
            (
                "AVERAGE(A1:A5)",
                "25 00 00 00 00 04 00 00 00 00 c0 00 c0 42 01 05 00",
            ),
            (
                "IF(A1>5,\"크다\",\"작다\")",
                "44 00 00 00 00 00 c0 1e 05 00 0d 19 02 0b 00 17 02 00 6c d0 e4 b2 19 08 0e 00 17 02 00 91 c7 e4 b2 19 08 03 00 42 03 01 00",
            ),
            (
                "ROUND(A3/7,2)",
                "44 02 00 00 00 00 c0 1e 07 00 06 1e 02 00 41 1b 00",
            ),
            (
                "$A$1+A$2+$A3",
                "44 00 00 00 00 00 00 44 01 00 00 00 00 40 03 44 02 00 00 00 00 80 03",
            ),
            (
                "VLOOKUP(30,A1:B5,2,FALSE)",
                "1e 1e 00 25 00 00 00 00 04 00 00 00 00 c0 01 c0 1e 02 00 1d 00 42 04 66 00",
            ),
            ("-A1", "44 00 00 00 00 00 c0 13"),
            ("A1%", "44 00 00 00 00 00 c0 14"),
            (
                "(A1+A2)*A3",
                "44 00 00 00 00 00 c0 44 01 00 00 00 00 c0 03 15 44 02 00 00 00 00 c0 05",
            ),
            (
                "COUNTIF(A1:A5,\">=30\")",
                "25 00 00 00 00 04 00 00 00 00 c0 00 c0 17 04 00 3e 00 3d 00 33 00 30 00 41 5a 01",
            ),
            ("TODAY()-TODAY()", "19 01 00 00 41 dd 00 41 dd 00 04"),
            (
                "INDEX(A1:A5,3)",
                "25 00 00 00 00 04 00 00 00 00 c0 00 c0 1e 03 00 42 02 1d 00",
            ),
            ("ABS(-5)", "1f 00 00 00 00 00 00 14 c0 41 18 00"),
            (
                "IFERROR(1/0,0)",
                "1e 01 00 1e 00 00 06 19 80 03 00 1e 00 00 19 08 02 00 41 e0 01",
            ),
            ("Sheet2!A1*2", "5a 00 00 00 00 00 00 00 c0 1e 02 00 05"),
            (
                "SUM(A:A)",
                "25 00 00 00 00 ff ff 0f 00 00 40 00 40 19 10 00 00",
            ),
            (
                "SUM(1:1)",
                "25 00 00 00 00 00 00 00 00 00 80 ff bf 19 10 00 00",
            ),
            ("1.5E+20", "1f 30 29 88 1a 56 43 20 44"),
            (
                "SUMPRODUCT(A1:A2,A3:A4)",
                "65 00 00 00 00 01 00 00 00 00 c0 00 c0 65 02 00 00 00 03 00 00 00 00 c0 00 c0 42 02 e4 00",
            ),
            (
                "IF(SUM(D18),D18,IF(C18,C18,\"\"))",
                "24 11 00 00 00 03 c0 19 10 00 00 19 02 0b 00 24 11 00 00 00 03 c0 19 08 28 00 44 11 00 00 00 02 c0 19 02 0b 00 24 11 00 00 00 02 c0 19 08 0a 00 17 00 00 19 08 03 00 22 03 01 00 19 08 03 00 42 03 01 00",
            ),
            (
                "IF(MILEAGE_RATE,FIXED(MILEAGE_RATE*100,0,TRUE),\"___\")&\"원으로\"",
                "43 01 00 00 00 19 02 16 00 43 01 00 00 00 1e 64 00 05 1e 00 00 1d 01 42 03 0e 00 19 08 10 00 17 03 00 5f 00 5f 00 5f 00 19 08 03 00 42 03 01 00 17 03 00 d0 c6 3c c7 5c b8 08",
            ),
        ];
        for (f, h) in cases {
            assert_eq!(enc(f), hex(h), "{f}");
        }
    }

    #[test]
    fn relative_tokens_for_shared_formulas() {
        let mut r = R {
            names: vec!["MILEAGE_RATE".into()],
            xtis: Vec::new(),
        };
        // 기준 F18 의 공유 수식 (관측: Auto_Expense_Report 서식)
        let e = encode(
            "IF(SUM(E18),ROUND(E18*MILEAGE_RATE,2),\"\")",
            Place::Relative(CellRef { row: 17, col: 5 }),
            &mut r,
        )
        .unwrap();
        assert_eq!(
            e.rgce,
            hex(
                "2c 00 00 00 00 ff ff 19 10 00 00 19 02 17 00 4c 00 00 00 00 ff ff 43 01 00 00 00 05 1e 02 00 41 1b 00 19 08 0a 00 17 00 00 19 08 03 00 42 03 01 00"
            )
        );
        // 기준 E25, 위로 7행 (행 오프셋은 20비트)
        let e = encode(
            "IF(SUM(E18:E24),SUM(E18:E24),\"\")",
            Place::Relative(CellRef { row: 24, col: 4 }),
            &mut r,
        )
        .unwrap();
        assert_eq!(
            e.rgce,
            hex(
                "2d f9 ff 0f 00 ff ff 0f 00 00 c0 00 c0 19 10 00 00 19 02 15 00 2d f9 ff 0f 00 ff ff 0f 00 00 c0 00 c0 19 10 00 00 19 08 0a 00 17 00 00 19 08 03 00 42 03 01 00"
            )
        );
    }

    // 3차원 참조도 상대 위치 수식에서는 오프셋 (한셀 관측: 공유 수식을 넣어 만든 XML 을 한셀이 바이너리로 저장)
    #[test]
    fn relative_3d_references_use_offsets_like_hancell() {
        use crate::legacy::formula::{Context, Decoded, decode_relative};
        let cases = [
            // (기준 셀, 원문, 한셀 토큰열)
            (
                CellRef { row: 0, col: 3 },
                "Sheet2!C1+1",
                "5a 00 00 00 00 00 00 ff ff 1e 01 00 03",
            ),
            (
                CellRef { row: 1, col: 4 },
                "SUM(Sheet2!B1:C2)",
                "3b 00 00 ff ff 0f 00 00 00 00 00 fd ff fe ff 19 10 00 00",
            ),
            (
                CellRef { row: 1, col: 5 },
                "Sheet2!$B1+Sheet2!B$1+B1",
                "5a 00 00 ff ff 0f 00 01 80 5a 00 00 00 00 00 00 fc 7f 03 4c ff ff 0f 00 fc ff 03",
            ),
            (
                CellRef { row: 2, col: 7 },
                "Sheet2!B3",
                "5a 00 00 00 00 00 00 fa ff",
            ),
            (
                CellRef { row: 0, col: 0 },
                "Sheet2!B1*2",
                "5a 00 00 00 00 00 00 01 c0 1e 02 00 05",
            ),
        ];
        let sheets = [Some("Sheet2".to_string())];
        for (base, f, h) in cases {
            let mut r = R {
                names: Vec::new(),
                xtis: Vec::new(),
            };
            let e = encode(f, Place::Relative(base), &mut r).unwrap();
            assert_eq!(e.rgce, hex(h), "{f}");
            let ctx = Context {
                xti_sheets: &sheets,
                names: &[],
                xti_names: &[],
                base,
                xlsb: true,
                row_shift: 0,
                tables: &[],
            };
            assert_eq!(
                decode_relative(&e.rgce, &ctx),
                Ok(Decoded::Text(f.to_string())),
                "{f}"
            );
        }
    }

    #[test]
    fn names_use_reference_class_and_memfunc_union() {
        let mut r = R {
            names: Vec::new(),
            xtis: Vec::new(),
        };
        let e = encode("Sheet1!$B$3:$S$43,Sheet1!$B$47:$S$102", Place::Name, &mut r).unwrap();
        assert_eq!(
            e.rgce,
            hex(
                "29 1f 00 3b 00 00 02 00 00 00 2a 00 00 00 01 00 12 00 3b 00 00 2e 00 00 00 65 00 00 00 01 00 12 00 10"
            )
        );
        let e = encode("'첫 시트'!$A$5", Place::Name, &mut r).unwrap();
        assert_eq!(e.rgce, hex("3a 01 00 04 00 00 00 00 00"));
        let e = encode("#REF!", Place::Name, &mut r).unwrap();
        assert_eq!(e.rgce, hex("1c 17"));
    }

    #[test]
    fn decodes_back_to_the_same_text() {
        use crate::legacy::formula::{Context, Decoded, decode};
        let sheets = [Some("Sheet2".to_string())];
        let names = ["MILEAGE_RATE".to_string()];
        let ctx = Context {
            xti_sheets: &sheets,
            names: &names,
            xti_names: &[],
            base: CellRef::default(),
            xlsb: true,
            row_shift: 0,
            tables: &[],
        };
        for f in [
            "SUM(A1:B2,C3)*-A1",
            "IF(A1=\"\",0,CHOOSE(2,A1,B1,C1))",
            "A1^2&\"x\"",
            "SUM($A:$B)+SUM(2:$3)",
            "Sheet2!A1:B2",
            "MILEAGE_RATE*2%",
            "NOT(TRUE)",
            "-(A1+1)",
        ] {
            let mut r = R {
                names: names.to_vec(),
                xtis: vec![(None, "Sheet2".into())],
            };
            let e = encode(f, Place::Cell, &mut r).unwrap();
            assert_eq!(
                decode(&e.rgce, &ctx),
                Ok(Decoded::Text(f.to_string())),
                "{f}"
            );
        }
    }
}
