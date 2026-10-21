// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` 수식 토큰(Ptg) 부호기 — [`super::formula`] 의 반대.
//!
//! 수식 원문(A1 표기, `=` 없음, 한셀 XML 과 같은 꼴)을 RPN 토큰으로 바꾼다. 토큰 체계는 BIFF8 Ptg 이고
//! 한셀 레거시의 폭(행 u32, 열 u16, PtgStr 길이 u16)을 쓴다. 한셀이 쓰는 꼴을 따른다 (관측, 말뭉치 수식 대조):
//!
//! - 피연산자 토큰의 클래스(참조 0x20 / 값 0x40 / 배열 0x60)는 BIFF8 규칙대로 자리가 기대하는 클래스를 따른다:
//!   수식 뿌리는 값(이름은 참조), 연산자의 피연산자는 값, 함수 인자는 함수마다 정해진 클래스.
//! - 함수 하나짜리 `SUM` 은 tAttrSum, `IF` 는 tAttrIf + tAttrGoto, `CHOOSE` 는 tAttrChoose 로 쓴다.
//! - 휘발 함수(`TODAY`, `NOW`, `RAND`, `OFFSET`, `INDIRECT`, `CELL`, `INFO`)가 있으면 맨 앞에 tAttrVolatile.
//! - 0–65535 의 정수는 PtgInt, 그 밖의 수는 PtgNum. 숫자 상수에 바로 붙은 단항 `-` 는 음수 PtgNum 으로 접는다.
//! - PtgStr 은 모든 글자가 U+00FF 이하면 1바이트(플래그 0), 아니면 UTF-16(플래그 1).

use crate::model::CellRef;

const MAX_ROW: u32 = 1_048_575;
const MAX_COL: u32 = 16_383;

/// 피연산자 클래스
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Ref,
    Val,
    Arr,
}

impl Class {
    fn bits(self) -> u8 {
        match self {
            Class::Ref => 0x20,
            Class::Val => 0x40,
            Class::Arr => 0x60,
        }
    }
}

/// 이름·시트·외부 문서를 번호로 바꾸는 쪽 (쓰기 쪽이 준다). 번호 표(XTI, `_xlfn.` 이름)는 필요할 때 만든다.
pub trait Resolve {
    /// 이 문서의 시트 이름 → 번호 (0부터)
    fn sheet(&self, name: &str) -> Option<usize>;
    /// 외부 문서(1부터)의 시트 이름 → 그 문서 안 번호
    fn ext_sheet(&self, book: usize, name: &str) -> Option<usize>;
    /// XTI 번호 (없으면 붙인다). `book` None = 이 문서
    fn xti(&mut self, book: Option<usize>, first: usize, last: usize) -> u16;
    /// 정의된 이름 → NAME 순서 번호 (1부터). `sheet` = 시트 범위 이름을 찾을 시트
    fn name(&mut self, name: &str, sheet: Option<usize>) -> Option<u32>;
    /// 외부 문서의 이름 → (ixti, EXTERNNAME 번호 1부터)
    fn ext_name(&mut self, book: usize, name: &str) -> Option<(u16, u32)>;
    /// BIFF8 함수표에 없는 함수 이름(`_xlfn.…`, 한컴 함수)의 NAME 번호 (없으면 붙인다)
    fn func_name(&mut self, name: &str) -> u32;
    /// 새 함수(Excel 2007 이후)를 썼다 — 한셀은 번호로 쓰는 IFERROR 도 `_xlfn.` 이름을 남긴다 (관측)
    fn note_future(&mut self, _name: &str) {}
}

/// 부호화 설정
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// 수식이 있는 셀 (상대 토큰의 기준, 현재 시트의 셀)
    pub base: CellRef,
    /// 현재 시트 번호 (시트 범위 이름 찾기)
    pub sheet: Option<usize>,
    /// 상대 토큰(PtgRefN/PtgAreaN)으로 쓴다 — 공유 수식 (SHRFMLA)
    pub relative: bool,
    /// 시트 이름 없는 참조도 3차원(PtgRef3d)으로 쓴다 — 정의된 이름
    pub force_3d: bool,
    /// 뿌리 클래스: 셀 수식 값, 이름 참조
    pub root: Class,
    /// 배열 수식 (ARRAY): 함수 인자 안의 값 자리는 배열 클래스. 맨 위는 셀 수식과 같다 (수식 대조 실험:
    /// `{B1:B3*2}` → 0x45, `{SUM(B1:B3*2)}` → 0x65 + tAttrSum, `{TRANSPOSE(B1:B2)}` → 0x65 0x41 0x53 0x00)
    pub array: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            base: CellRef::default(),
            sheet: None,
            relative: false,
            force_3d: false,
            root: Class::Val,
            array: false,
        }
    }
}

/// 부호화 실패 (모르는 함수, 문법 오류 …)
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeError(pub String);

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(m: impl Into<String>) -> Result<T, EncodeError> {
    Err(EncodeError(m.into()))
}

// ---------------------------------------------------------------------------
// 함수 표: 번호, 반환 클래스, 인자 클래스 (마지막 글자가 되풀이된다)
// R 참조, V 값, A 배열, O 함수 토큰 자신의 클래스를 따른다 (IF·CHOOSE 의 결과 인자)
// ---------------------------------------------------------------------------

struct FuncInfo {
    index: u16,
    name: &'static str,
    /// 고정 인자 수 (None = 가변, PtgFuncVar)
    fixed: Option<u8>,
    ret: u8,
    params: &'static [u8],
}

fn class_of(c: u8) -> Class {
    match c {
        b'R' => Class::Ref,
        b'A' => Class::Arr,
        _ => Class::Val,
    }
}

/// BIFF8 함수의 반환·인자 클래스. 표에 없는 함수는 값 → 값.
fn classes(index: u16) -> (u8, &'static [u8]) {
    match index {
        0
        | 4..=7
        | 12
        | 36
        | 37
        | 46
        | 169
        | 183
        | 193
        | 194
        | 227
        | 269
        | 318..=323
        | 361..=367 => (b'V', b"R"),
        // IF·CHOOSE 의 값 자리는 참조 클래스 (관측: 서식 파일 IF(…,A1,…) 의 A1 이 0x24, 안쪽 IF 가 0x22)
        1 => (b'R', b"VR"),
        100 => (b'R', b"VR"),
        8 | 9 | 75..=77 | 105 | 130 | 131 | 347 | 360 => (b'V', b"R"),
        11 => (b'V', b"VR"),
        28 => (b'V', b"VR"),
        29 => (b'R', b"RV"),
        40..=45 | 47 | 189 | 195 | 196 | 199 | 235 => (b'V', b"R"),
        49 | 51 => (b'A', b"RRV"),
        50 | 52 => (b'A', b"RRRV"),
        61 => (b'V', b"RV"),
        62 => (b'V', b"RV"),
        64 => (b'V', b"VRR"),
        78 => (b'R', b"RV"),
        83 | 164 => (b'A', b"A"),
        101 | 102 => (b'V', b"VRRV"),
        125 => (b'V', b"VR"),
        148 => (b'R', b"V"),
        163 | 330 => (b'V', b"A"),
        165 => (b'A', b"A"),
        216 => (b'V', b"VRV"),
        228 | 303..=308 | 310..=315 => (b'V', b"A"),
        252 => (b'A', b"R"),
        309 => (b'V', b"VA"),
        316 | 317 => (b'V', b"AAV"),
        324..=329 | 331 => (b'V', b"RV"),
        344 => (b'V', b"VR"),
        345 => (b'V', b"RVR"),
        346 => (b'V', b"RV"),
        358 => (b'V', b"VRV"),
        481 => (b'V', b"RV"),
        482 | 484 => (b'V', b"RRV"),
        483 => (b'V', b"RVR"),
        _ => (b'V', b"V"),
    }
}

fn lookup_function(name: &str) -> Option<FuncInfo> {
    let upper = name.to_ascii_uppercase();
    // 번호 → 이름 표를 거꾸로 찾는다 (0–484, 한셀 전용 번호 제외)
    for index in 0u16..=484 {
        if let Some((n, fixed)) = super::ftab::lookup(index)
            && n == upper
        {
            let (ret, params) = classes(index);
            return Some(FuncInfo {
                index,
                name: n,
                fixed,
                ret,
                params,
            });
        }
    }
    None
}

/// 휘발 함수 — 수식 맨 앞에 tAttrVolatile 을 둔다
fn is_volatile(index: u16) -> bool {
    matches!(index, 63 | 74 | 78 | 125 | 148 | 221 | 244)
}

// ---------------------------------------------------------------------------
// 낱말
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(u8),
    /// 참조 (시트 앞머리 포함)
    Ref(RefTok),
    /// 이름 (시트·외부 문서 앞머리 포함)
    Name(Prefix, String),
    /// 함수 이름 (뒤에 `(`)
    Func(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

/// 시트 앞머리 `시트!`, `시트1:시트3!`, `[n]시트!`, `[n]!`
#[derive(Debug, Clone, PartialEq, Default)]
struct Prefix {
    book: Option<usize>,
    first: Option<String>,
    last: Option<String>,
}

/// 한 축: 번호 (0부터), 절대 여부
#[derive(Debug, Clone, Copy, PartialEq)]
struct Axis {
    n: u32,
    abs: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum RefKind {
    Cell(Axis, Axis),
    /// (열1, 행1, 열2, 행2)
    Area(Axis, Axis, Axis, Axis),
    /// 열 전체 `A:C`
    Cols(Axis, Axis),
    /// 행 전체 `1:3`
    Rows(Axis, Axis),
    /// `#REF!`
    Err,
}

#[derive(Debug, Clone, PartialEq)]
struct RefTok {
    prefix: Prefix,
    kind: RefKind,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\' | '?')
}

/// `$A`, `AB` → 열 (0부터)
fn parse_col(s: &str) -> Option<Axis> {
    let (abs, t) = match s.strip_prefix('$') {
        Some(t) => (true, t),
        None => (false, s),
    };
    if t.is_empty() || t.len() > 3 || !t.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for c in t.chars() {
        n = n * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    (n >= 1 && n - 1 <= MAX_COL).then_some(Axis { n: n - 1, abs })
}

/// `$12` → 행 (0부터)
fn parse_row(s: &str) -> Option<Axis> {
    let (abs, t) = match s.strip_prefix('$') {
        Some(t) => (true, t),
        None => (false, s),
    };
    if t.is_empty() || t.len() > 7 || !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u32 = t.parse().ok()?;
    (n >= 1 && n - 1 <= MAX_ROW).then_some(Axis { n: n - 1, abs })
}

/// `$A$1` → (열, 행)
fn parse_cell(s: &str) -> Option<(Axis, Axis)> {
    let body = s.strip_prefix('$').unwrap_or(s);
    let split = body.find(|c: char| c == '$' || c.is_ascii_digit())?;
    let col_len = split + (s.len() - body.len());
    let (c, r) = s.split_at(col_len);
    Some((parse_col(c)?, parse_row(r)?))
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

struct Lexer {
    c: Vec<char>,
    i: usize,
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn at(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }
    fn word_from(&self, start: usize) -> usize {
        let mut e = start;
        while e < self.c.len() && is_word_char(self.c[e]) {
            e += 1;
        }
        e
    }
    fn text(&self, a: usize, b: usize) -> String {
        self.c[a..b].iter().collect()
    }

    /// 따옴표 시트 이름 `'…'` (작은따옴표 두 개 = 하나)
    fn quoted(&mut self) -> Result<String, EncodeError> {
        self.i += 1;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return err("닫는 작은따옴표가 없습니다"),
                Some('\'') if self.at(1) == Some('\'') => {
                    s.push('\'');
                    self.i += 2;
                }
                Some('\'') => {
                    self.i += 1;
                    return Ok(s);
                }
                Some(c) => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// 앞머리 뒤의 참조나 이름: `A1`, `A1:B2`, `A:A`, `1:1`, `#REF!`, 이름
    fn after_prefix(&mut self, prefix: Prefix) -> Result<Tok, EncodeError> {
        if self.peek() == Some('#') {
            let start = self.i;
            while let Some(c) = self.peek() {
                self.i += 1;
                if c == '!' || c == '?' || (c == 'A' && self.text(start, self.i) == "#N/A") {
                    break;
                }
            }
            let t = self.text(start, self.i);
            if t == "#REF!" {
                return Ok(Tok::Ref(RefTok {
                    prefix,
                    kind: RefKind::Err,
                }));
            }
            return err(format!("앞머리 뒤의 오류 값 {t}"));
        }
        let s = self.i;
        let e = self.word_from(s);
        if e == s {
            return err("앞머리 뒤에 참조가 없습니다");
        }
        let w = self.text(s, e);
        self.i = e;
        if let Some(k) = self.range_tail(&w)? {
            return Ok(Tok::Ref(RefTok { prefix, kind: k }));
        }
        if let Some((c, r)) = parse_cell(&w) {
            return Ok(Tok::Ref(RefTok {
                prefix,
                kind: RefKind::Cell(c, r),
            }));
        }
        Ok(Tok::Name(prefix, w))
    }

    /// `w` 뒤에 `:둘째` 가 이어져 범위가 되면 범위로 읽는다
    fn range_tail(&mut self, w: &str) -> Result<Option<RefKind>, EncodeError> {
        if self.peek() != Some(':') {
            // 열·행 전체는 범위로만 쓴다 (A:A, 1:1)
            return Ok(None);
        }
        let s2 = self.i + 1;
        let e2 = self.word_from(s2);
        if e2 == s2 {
            return Ok(None);
        }
        let w2 = self.text(s2, e2);
        let kind = if let (Some((c1, r1)), Some((c2, r2))) = (parse_cell(w), parse_cell(&w2)) {
            RefKind::Area(c1, r1, c2, r2)
        } else if let (Some(a), Some(b)) = (parse_col(w), parse_col(&w2)) {
            RefKind::Cols(a, b)
        } else if let (Some(a), Some(b)) = (parse_row(w), parse_row(&w2)) {
            RefKind::Rows(a, b)
        } else {
            return Ok(None);
        };
        self.i = e2;
        Ok(Some(kind))
    }

    fn next(&mut self) -> Result<Option<Tok>, EncodeError> {
        while self
            .peek()
            .is_some_and(|c| c == ' ' || c == '\n' || c == '\r' || c == '\t')
        {
            self.i += 1;
        }
        let Some(c) = self.peek() else {
            return Ok(None);
        };
        let one = |l: &mut Lexer, t: Tok| {
            l.i += 1;
            Ok(Some(t))
        };
        match c {
            '(' => one(self, Tok::LParen),
            ')' => one(self, Tok::RParen),
            ',' => one(self, Tok::Comma),
            '+' => one(self, Tok::Op("+")),
            '-' => one(self, Tok::Op("-")),
            '*' => one(self, Tok::Op("*")),
            '/' => one(self, Tok::Op("/")),
            '^' => one(self, Tok::Op("^")),
            '&' => one(self, Tok::Op("&")),
            '%' => one(self, Tok::Op("%")),
            '=' => one(self, Tok::Op("=")),
            ':' => one(self, Tok::Op(":")),
            '<' => match self.at(1) {
                Some('>') => {
                    self.i += 2;
                    Ok(Some(Tok::Op("<>")))
                }
                Some('=') => {
                    self.i += 2;
                    Ok(Some(Tok::Op("<=")))
                }
                _ => one(self, Tok::Op("<")),
            },
            '>' => match self.at(1) {
                Some('=') => {
                    self.i += 2;
                    Ok(Some(Tok::Op(">=")))
                }
                _ => one(self, Tok::Op(">")),
            },
            '"' => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    match self.peek() {
                        None => return err("닫는 큰따옴표가 없습니다"),
                        Some('"') if self.at(1) == Some('"') => {
                            s.push('"');
                            self.i += 2;
                        }
                        Some('"') => {
                            self.i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(ch);
                            self.i += 1;
                        }
                    }
                }
                Ok(Some(Tok::Str(s)))
            }
            '#' => {
                let start = self.i;
                let mut found = None;
                for e in start + 2..=(start + 14).min(self.c.len()) {
                    if let Some(code) = error_code(&self.text(start, e)) {
                        found = Some((e, code));
                    }
                }
                match found {
                    Some((e, code)) => {
                        let t = self.text(start, e);
                        self.i = e;
                        // 시트 없는 #REF! 는 오류 값 (한셀: 이름 `=#REF!` → PtgErr)
                        let _ = t;
                        Ok(Some(Tok::Err(code)))
                    }
                    None => err(format!("모르는 오류 값 위치 {start}")),
                }
            }
            '\'' => {
                let first = self.quoted()?;
                // 'Sheet1:Sheet3'! / '[1]Sheet'!
                if self.peek() != Some('!') {
                    return err("따옴표 시트 이름 뒤에 ! 가 없습니다");
                }
                self.i += 1;
                let prefix = split_prefix(&first)?;
                self.after_prefix(prefix).map(Some)
            }
            '[' => {
                // 외부 문서 번호 [n]
                let start = self.i + 1;
                let mut e = start;
                while e < self.c.len() && self.c[e].is_ascii_digit() {
                    e += 1;
                }
                if e == start || self.c.get(e) != Some(&']') {
                    return err("구조적 참조는 레거시에 쓸 수 없습니다");
                }
                let book: usize = self.text(start, e).parse().unwrap_or(0);
                self.i = e + 1;
                if self.peek() == Some('!') {
                    self.i += 1;
                    return self
                        .after_prefix(Prefix {
                            book: Some(book),
                            ..Default::default()
                        })
                        .map(Some);
                }
                let s = self.i;
                let w = self.word_from(s);
                let sheet = self.text(s, w);
                self.i = w;
                if self.peek() != Some('!') {
                    return err("외부 참조의 시트 이름 뒤에 ! 가 없습니다");
                }
                self.i += 1;
                self.after_prefix(Prefix {
                    book: Some(book),
                    first: Some(sheet),
                    last: None,
                })
                .map(Some)
            }
            c if c.is_ascii_digit()
                || (c == '.' && self.at(1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                // 행 전체 (1:3) 이거나 숫자
                let s = self.i;
                let e = self.word_from(s);
                let w = self.text(s, e);
                if w.chars().all(|ch| ch.is_ascii_digit()) {
                    let save = self.i;
                    self.i = e;
                    if let Some(k) = self.range_tail(&w)? {
                        return Ok(Some(Tok::Ref(RefTok {
                            prefix: Prefix::default(),
                            kind: k,
                        })));
                    }
                    self.i = save;
                }
                // 숫자: 정수부, 소수부, 지수 (1.5E+20)
                let mut e = self.i;
                while e < self.c.len() && (self.c[e].is_ascii_digit() || self.c[e] == '.') {
                    e += 1;
                }
                if e < self.c.len() && (self.c[e] == 'E' || self.c[e] == 'e') {
                    let mut k = e + 1;
                    if k < self.c.len() && (self.c[k] == '+' || self.c[k] == '-') {
                        k += 1;
                    }
                    if k < self.c.len() && self.c[k].is_ascii_digit() {
                        while k < self.c.len() && self.c[k].is_ascii_digit() {
                            k += 1;
                        }
                        e = k;
                    }
                }
                let t = self.text(self.i, e);
                self.i = e;
                t.parse::<f64>()
                    .map(|v| Some(Tok::Num(v)))
                    .map_err(|_| EncodeError(format!("숫자 {t}")))
            }
            '{' => err("배열 상수는 아직 쓸 수 없습니다"),
            c if is_word_char(c) => {
                let s = self.i;
                let e = self.word_from(s);
                let w = self.text(s, e);
                self.i = e;
                match self.peek() {
                    Some('(') => return Ok(Some(Tok::Func(w))),
                    Some('!') => {
                        self.i += 1;
                        return self
                            .after_prefix(Prefix {
                                book: None,
                                first: Some(w),
                                last: None,
                            })
                            .map(Some);
                    }
                    Some(':') => {
                        // 3차원 시트 범위 `Sheet1:Sheet3!A1` 또는 범위 `A1:B2`
                        let s2 = self.i + 1;
                        let e2 = self.word_from(s2);
                        if e2 > s2 && self.c.get(e2) == Some(&'!') {
                            let w2 = self.text(s2, e2);
                            self.i = e2 + 1;
                            return self
                                .after_prefix(Prefix {
                                    book: None,
                                    first: Some(w),
                                    last: Some(w2),
                                })
                                .map(Some);
                        }
                        if let Some(k) = self.range_tail(&w)? {
                            return Ok(Some(Tok::Ref(RefTok {
                                prefix: Prefix::default(),
                                kind: k,
                            })));
                        }
                    }
                    _ => {}
                }
                if let Some((c, r)) = parse_cell(&w) {
                    return Ok(Some(Tok::Ref(RefTok {
                        prefix: Prefix::default(),
                        kind: RefKind::Cell(c, r),
                    })));
                }
                match w.to_ascii_uppercase().as_str() {
                    "TRUE" => Ok(Some(Tok::Bool(true))),
                    "FALSE" => Ok(Some(Tok::Bool(false))),
                    _ => Ok(Some(Tok::Name(Prefix::default(), w))),
                }
            }
            other => err(format!("모르는 글자 {other:?}")),
        }
    }
}

/// 따옴표 안의 앞머리 `[1]Sheet`, `Sheet1:Sheet3`
fn split_prefix(s: &str) -> Result<Prefix, EncodeError> {
    let (book, rest) = match s.strip_prefix('[') {
        Some(t) => {
            let Some(close) = t.find(']') else {
                return err("외부 문서 번호의 ] 가 없습니다");
            };
            let n: usize = t[..close]
                .parse()
                .map_err(|_| EncodeError(format!("외부 문서 번호 {s}")))?;
            (Some(n), &t[close + 1..])
        }
        None => (None, s),
    };
    let (first, last) = match rest.split_once(':') {
        Some((a, b)) if book.is_none() => (a.to_string(), Some(b.to_string())),
        _ => (rest.to_string(), None),
    };
    Ok(Prefix {
        book,
        first: (!first.is_empty()).then_some(first),
        last,
    })
}

// ---------------------------------------------------------------------------
// 구문 나무
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Num(f64),
    Str(String),
    Bool(bool),
    Err(u8),
    Missing,
    Ref(RefTok),
    Name(Prefix, String),
    Func(String, Vec<Node>),
    Unary(&'static str, Box<Node>),
    Percent(Box<Node>),
    Binary(&'static str, Box<Node>, Box<Node>),
    Paren(Box<Node>),
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn is_op(&self, ops: &[&str]) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(o) => Some(o),
            _ => None,
        }
    }

    fn expr(&mut self) -> Result<Node, EncodeError> {
        self.comparison()
    }

    fn binary_level(
        &mut self,
        ops: &[&str],
        next: fn(&mut Parser) -> Result<Node, EncodeError>,
    ) -> Result<Node, EncodeError> {
        let mut a = next(self)?;
        while let Some(op) = self.is_op(ops) {
            self.i += 1;
            let b = next(self)?;
            a = Node::Binary(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn comparison(&mut self) -> Result<Node, EncodeError> {
        self.binary_level(&["=", "<>", "<", ">", "<=", ">="], Parser::concat)
    }
    fn concat(&mut self) -> Result<Node, EncodeError> {
        self.binary_level(&["&"], Parser::additive)
    }
    fn additive(&mut self) -> Result<Node, EncodeError> {
        self.binary_level(&["+", "-"], Parser::multiplicative)
    }
    fn multiplicative(&mut self) -> Result<Node, EncodeError> {
        self.binary_level(&["*", "/"], Parser::power)
    }
    fn power(&mut self) -> Result<Node, EncodeError> {
        self.binary_level(&["^"], Parser::percent)
    }
    fn percent(&mut self) -> Result<Node, EncodeError> {
        let mut a = self.unary()?;
        while self.is_op(&["%"]).is_some() {
            self.i += 1;
            a = Node::Percent(Box::new(a));
        }
        Ok(a)
    }
    fn unary(&mut self) -> Result<Node, EncodeError> {
        if let Some(op) = self.is_op(&["-", "+"]) {
            self.i += 1;
            let x = self.unary()?;
            // 숫자 상수에 바로 붙은 - 는 음수 상수로 (한셀: ABS(-5) → PtgNum -5)
            if op == "-"
                && let Node::Num(v) = x
            {
                return Ok(Node::Num(-v));
            }
            return Ok(Node::Unary(op, Box::new(x)));
        }
        self.range()
    }
    fn range(&mut self) -> Result<Node, EncodeError> {
        let mut a = self.primary()?;
        while self.is_op(&[":"]).is_some() {
            self.i += 1;
            let b = self.primary()?;
            a = Node::Binary(":", Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn primary(&mut self) -> Result<Node, EncodeError> {
        match self.bump() {
            Some(Tok::Num(v)) => Ok(Node::Num(v)),
            Some(Tok::Str(s)) => Ok(Node::Str(s)),
            Some(Tok::Bool(b)) => Ok(Node::Bool(b)),
            Some(Tok::Err(e)) => Ok(Node::Err(e)),
            Some(Tok::Ref(r)) => Ok(Node::Ref(r)),
            Some(Tok::Name(p, n)) => Ok(Node::Name(p, n)),
            Some(Tok::Func(name)) => {
                if self.bump() != Some(Tok::LParen) {
                    return err("함수 이름 뒤에 ( 가 없습니다");
                }
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::RParen) {
                    self.i += 1;
                    return Ok(Node::Func(name, args));
                }
                loop {
                    let arg = match self.peek() {
                        Some(Tok::Comma) | Some(Tok::RParen) => Node::Missing,
                        _ => self.expr()?,
                    };
                    args.push(arg);
                    match self.bump() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RParen) => break,
                        _ => return err(format!("{name} 의 인자 목록이 닫히지 않았습니다")),
                    }
                }
                Ok(Node::Func(name, args))
            }
            Some(Tok::LParen) => {
                // 괄호 안의 쉼표는 합집합 연산자
                let mut a = self.expr()?;
                while self.peek() == Some(&Tok::Comma) {
                    self.i += 1;
                    let b = self.expr()?;
                    a = Node::Binary(",", Box::new(a), Box::new(b));
                }
                if self.bump() != Some(Tok::RParen) {
                    return err("닫는 괄호가 없습니다");
                }
                Ok(Node::Paren(Box::new(a)))
            }
            t => err(format!("뜻밖의 낱말 {t:?}")),
        }
    }
}

fn parse(text: &str) -> Result<Node, EncodeError> {
    let mut lx = Lexer {
        c: text.chars().collect(),
        i: 0,
    };
    let mut toks = Vec::new();
    while let Some(t) = lx.next()? {
        toks.push(t);
    }
    if toks.is_empty() {
        return err("빈 수식");
    }
    let mut p = Parser { toks, i: 0 };
    let mut n = p.expr()?;
    // 맨 바깥의 쉼표는 합집합 (정의된 이름 `=시트!$A$1:$B$2,시트!$D$1`)
    while p.peek() == Some(&Tok::Comma) {
        p.i += 1;
        let b = p.expr()?;
        n = Node::Binary(",", Box::new(n), Box::new(b));
    }
    if p.i != p.toks.len() {
        return err(format!("수식 끝에 남은 낱말 {:?}", p.toks.get(p.i)));
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// 토큰 쓰기
// ---------------------------------------------------------------------------

struct Emitter<'r> {
    out: Vec<u8>,
    res: &'r mut dyn Resolve,
    opts: Options,
    volatile: bool,
    /// 합집합 안 (PtgMemFunc 는 맨 바깥 합집합에만)
    in_union: bool,
}

/// 클래스 정하기: 기대 클래스 + 배열 강제 여부 → 피연산자 클래스
fn operand_class(expected: Class, forced_array: bool) -> Class {
    match expected {
        Class::Val if forced_array => Class::Arr,
        c => c,
    }
}

impl Emitter<'_> {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn col_field(&self, col: Axis, row: Axis) -> u16 {
        let mut v = (col.n & 0x3FFF) as u16;
        if !col.abs {
            v |= 0x4000;
        }
        if !row.abs {
            v |= 0x8000;
        }
        v
    }

    /// 상대 토큰(PtgRefN)의 행: 상대면 기준 행과의 차이 (20비트 2의 보수 — 한셀: −21 → `eb ff 0f 00`), 절대면 그대로
    fn rel_row(&self, r: Axis) -> u32 {
        if r.abs {
            r.n
        } else {
            (r.n as i64 - self.opts.base.row as i64).rem_euclid(MAX_ROW as i64 + 1) as u32
        }
    }
    /// 상대 토큰의 열 필드: 상대면 14비트 2의 보수 차이
    fn rel_col_field(&self, c: Axis, r: Axis) -> u16 {
        let mut v = if c.abs {
            (c.n & 0x3FFF) as u16
        } else {
            ((c.n as i64 - self.opts.base.col as i64) as i16 as u16) & 0x3FFF
        };
        if !c.abs {
            v |= 0x4000;
        }
        if !r.abs {
            v |= 0x8000;
        }
        v
    }

    fn sheet_xti(&mut self, p: &Prefix) -> Result<Option<u16>, EncodeError> {
        if p.book.is_none() && p.first.is_none() {
            if !self.opts.force_3d {
                return Ok(None);
            }
            let Some(s) = self.opts.sheet else {
                return err("시트 없는 참조");
            };
            return Ok(Some(self.res.xti(None, s, s)));
        }
        match p.book {
            None => {
                let first = p.first.as_deref().unwrap_or("");
                let a = self
                    .res
                    .sheet(first)
                    .ok_or_else(|| EncodeError(format!("시트 {first} 가 없습니다")))?;
                let b = match &p.last {
                    Some(l) => self
                        .res
                        .sheet(l)
                        .ok_or_else(|| EncodeError(format!("시트 {l} 가 없습니다")))?,
                    None => a,
                };
                Ok(Some(self.res.xti(None, a, b)))
            }
            Some(book) => {
                let first = p.first.as_deref().unwrap_or("");
                let a = self.res.ext_sheet(book, first).ok_or_else(|| {
                    EncodeError(format!("외부 문서 [{book}] 에 시트 {first} 가 없습니다"))
                })?;
                Ok(Some(self.res.xti(Some(book), a, a)))
            }
        }
    }

    fn reference(&mut self, r: &RefTok, class: Class) -> Result<(), EncodeError> {
        let cls = class.bits();
        let xti = self.sheet_xti(&r.prefix)?;
        let full_cols = |a: Axis, b: Axis| {
            (
                Axis { n: 0, abs: true },
                a,
                Axis {
                    n: MAX_ROW,
                    abs: true,
                },
                b,
            )
        };
        // 열·행 전체를 넓이로 바꾼다: 열 전체는 행 0–1048575 (행은 절대), 행 전체는 열 0–16383 (열은 절대)
        let area = match &r.kind {
            RefKind::Cell(c, rw) => {
                match xti {
                    Some(x) => {
                        self.u8(0x1A | cls);
                        self.u16(x);
                        self.u32(rw.n);
                        let f = self.col_field(*c, *rw);
                        self.u16(f);
                    }
                    // 공유 수식에서도 상대 부분이 없으면 PtgRef (관측)
                    None if self.opts.relative && !(c.abs && rw.abs) => {
                        self.u8(0x0C | cls);
                        let rr = self.rel_row(*rw);
                        self.u32(rr);
                        let f = self.rel_col_field(*c, *rw);
                        self.u16(f);
                    }
                    None => {
                        self.u8(0x04 | cls);
                        self.u32(rw.n);
                        let f = self.col_field(*c, *rw);
                        self.u16(f);
                    }
                }
                return Ok(());
            }
            RefKind::Err => {
                match xti {
                    Some(x) => {
                        self.u8(0x1C | cls);
                        self.u16(x);
                        self.out.extend_from_slice(&[0; 6]);
                    }
                    None => {
                        self.u8(0x0A | cls);
                        self.out.extend_from_slice(&[0; 6]);
                    }
                }
                return Ok(());
            }
            RefKind::Area(c1, r1, c2, r2) => (*r1, *c1, *r2, *c2),
            RefKind::Cols(a, b) => {
                let (r1, c1, r2, c2) = full_cols(*a, *b);
                (r1, c1, r2, c2)
            }
            RefKind::Rows(a, b) => (
                *a,
                Axis { n: 0, abs: true },
                *b,
                Axis {
                    n: MAX_COL,
                    abs: true,
                },
            ),
        };
        let (r1, c1, r2, c2) = area;
        match xti {
            Some(x) => {
                self.u8(0x1B | cls);
                self.u16(x);
                self.u32(r1.n);
                self.u32(r2.n);
                let f1 = self.col_field(c1, r1);
                let f2 = self.col_field(c2, r2);
                self.u16(f1);
                self.u16(f2);
            }
            None if self.opts.relative && !(r1.abs && r2.abs && c1.abs && c2.abs) => {
                self.u8(0x0D | cls);
                let a = self.rel_row(r1);
                let b = self.rel_row(r2);
                self.u32(a);
                self.u32(b);
                let f1 = self.rel_col_field(c1, r1);
                let f2 = self.rel_col_field(c2, r2);
                self.u16(f1);
                self.u16(f2);
            }
            None => {
                self.u8(0x05 | cls);
                self.u32(r1.n);
                self.u32(r2.n);
                let f1 = self.col_field(c1, r1);
                let f2 = self.col_field(c2, r2);
                self.u16(f1);
                self.u16(f2);
            }
        }
        Ok(())
    }

    fn number(&mut self, v: f64) {
        if (0.0..=65535.0).contains(&v) && v.fract() == 0.0 && !(v == 0.0 && v.is_sign_negative()) {
            self.u8(0x1E);
            self.u16(v as u16);
        } else {
            self.u8(0x1F);
            self.out.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn string(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u8(0x17);
        self.u16(units.len() as u16);
        if units.iter().all(|&u| u <= 0xFF) {
            self.u8(0);
            self.out.extend(units.iter().map(|&u| u as u8));
        } else {
            self.u8(1);
            for u in units {
                self.u16(u);
            }
        }
    }

    fn emit(&mut self, n: &Node, expected: Class, forced: bool) -> Result<(), EncodeError> {
        match n {
            Node::Num(v) => self.number(*v),
            Node::Str(s) => self.string(s),
            Node::Bool(b) => {
                self.u8(0x1D);
                self.u8(*b as u8);
            }
            Node::Err(e) => {
                self.u8(0x1C);
                self.u8(*e);
            }
            Node::Missing => self.u8(0x16),
            Node::Ref(r) => self.reference(r, operand_class(expected, forced))?,
            Node::Name(p, name) => self.name(p, name, operand_class(expected, forced))?,
            Node::Paren(x) => {
                self.emit(x, expected, forced)?;
                self.u8(0x15);
            }
            Node::Unary(op, x) => {
                self.emit(x, Class::Val, forced)?;
                self.u8(if *op == "-" { 0x13 } else { 0x12 });
            }
            Node::Percent(x) => {
                self.emit(x, Class::Val, forced)?;
                self.u8(0x14);
            }
            Node::Binary(op, a, b) => {
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
                    "," => 0x10,
                    _ => 0x11,
                };
                // 참조 연산자(범위·합집합)의 피연산자는 참조, 나머지는 값
                let cls = if matches!(ptg, 0x10 | 0x11) {
                    Class::Ref
                } else {
                    Class::Val
                };
                // 합집합은 PtgMemFunc(뒤따르는 식의 길이 u16)로 감싼다 (관측: 이름 Print_Area 의 두 영역)
                let wrap = ptg == 0x10 && !self.in_union;
                let at = self.out.len();
                if wrap {
                    self.in_union = true;
                    self.out.extend_from_slice(&[0x09 | expected.bits(), 0, 0]);
                }
                self.emit(a, cls, forced)?;
                self.emit(b, cls, forced)?;
                self.u8(ptg);
                if wrap {
                    self.in_union = false;
                    let len = (self.out.len() - at - 3) as u16;
                    self.out[at + 1..at + 3].copy_from_slice(&len.to_le_bytes());
                }
            }
            Node::Func(name, args) => self.func(name, args, expected, forced)?,
        }
        Ok(())
    }

    fn name(&mut self, p: &Prefix, name: &str, class: Class) -> Result<(), EncodeError> {
        if let Some(book) = p.book {
            let (ixti, idx) = self
                .res
                .ext_name(book, name)
                .ok_or_else(|| EncodeError(format!("외부 이름 [{book}]!{name}")))?;
            self.u8(0x19 | class.bits());
            self.u16(ixti);
            self.u32(idx);
            return Ok(());
        }
        let sheet = match &p.first {
            Some(s) => Some(
                self.res
                    .sheet(s)
                    .ok_or_else(|| EncodeError(format!("시트 {s} 가 없습니다")))?,
            ),
            None => self.opts.sheet,
        };
        let idx = self
            .res
            .name(name, sheet)
            .ok_or_else(|| EncodeError(format!("이름 {name} 이 없습니다")))?;
        self.u8(0x03 | class.bits());
        self.u32(idx);
        Ok(())
    }

    fn func(
        &mut self,
        name: &str,
        args: &[Node],
        expected: Class,
        forced: bool,
    ) -> Result<(), EncodeError> {
        let Some(info) = lookup_function(name).filter(|f| f.index <= 367 || f.index == 480) else {
            return self.user_func(name, args, expected, forced);
        };
        if info.index == 480 {
            self.res.note_future("IFERROR");
        }
        if is_volatile(info.index) {
            self.volatile = true;
        }
        // 함수 토큰 자신의 클래스: 참조를 돌려주는 함수는 자리의 클래스, 값·배열을 돌려주는 함수는 값(배열 자리면
        // 배열) — 배열 수식 맨 위의 TRANSPOSE 도 0x41 (수식 대조 실험, 바이너리 BrtArrFmla 도 같다)
        let token_class = match info.ret {
            b'R' => operand_class(expected, forced),
            _ => {
                if expected == Class::Arr || forced {
                    Class::Arr
                } else {
                    Class::Val
                }
            }
        };
        let in_array = self.opts.array;
        let param = |i: usize| -> (Class, bool) {
            let c = info
                .params
                .get(i)
                .or(info.params.last())
                .copied()
                .unwrap_or(b'V');
            match c {
                b'O' => (token_class, forced || in_array),
                b'A' => (Class::Arr, true),
                other => (class_of(other), forced || in_array),
            }
        };
        // SUM 인자 하나 → tAttrSum
        if info.index == 4 && args.len() == 1 {
            let (c, f) = param(0);
            self.emit(&args[0], c, f)?;
            self.out.extend_from_slice(&[0x19, 0x10, 0x00, 0x00]);
            return Ok(());
        }
        // IF → 조건, tAttrIf, 참 값, tAttrGoto, [거짓 값, tAttrGoto], tFuncVar
        if info.index == 1 && (2..=3).contains(&args.len()) {
            let (c0, f0) = param(0);
            self.emit(&args[0], c0, f0)?;
            let if_at = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x02, 0, 0]);
            let (c1, f1) = param(1);
            self.emit(&args[1], c1, f1)?;
            let goto1 = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            let else_start = self.out.len();
            let mut gotos = vec![goto1];
            if args.len() == 3 {
                let (c2, f2) = param(2);
                self.emit(&args[2], c2, f2)?;
                gotos.push(self.out.len());
                self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            }
            let fpos = self.out.len();
            self.out.push(0x02 | token_class.bits());
            self.out.push(args.len() as u8);
            self.out.extend_from_slice(&1u16.to_le_bytes());
            let end = self.out.len();
            // tAttrIf: 거짓 값(또는 함수 토큰)까지 건너뛸 바이트 수
            let skip_if = (else_start - (if_at + 4)) as u16;
            self.out[if_at + 2..if_at + 4].copy_from_slice(&skip_if.to_le_bytes());
            // tAttrGoto: 함수 토큰 끝까지 바이트 수 − 1
            for g in gotos {
                let skip = (end - (g + 4) - 1) as u16;
                self.out[g + 2..g + 4].copy_from_slice(&skip.to_le_bytes());
            }
            let _ = fpos;
            return Ok(());
        }
        // CHOOSE → 번호, tAttrChoose(점프 표), 값마다 tAttrGoto, tFuncVar
        if info.index == 100 && args.len() >= 2 {
            let (c0, f0) = param(0);
            self.emit(&args[0], c0, f0)?;
            let n = args.len() - 1;
            let choose_at = self.out.len();
            self.out.extend_from_slice(&[0x19, 0x04]);
            self.u16(n as u16);
            let table_at = self.out.len();
            self.out.extend(std::iter::repeat_n(0u8, (n + 1) * 2));
            let base = self.out.len();
            let mut starts = Vec::new();
            let mut gotos = Vec::new();
            for a in &args[1..] {
                starts.push(self.out.len() - base);
                let (c, f) = param(1);
                self.emit(a, c, f)?;
                gotos.push(self.out.len());
                self.out.extend_from_slice(&[0x19, 0x08, 0, 0]);
            }
            let after = self.out.len() - base;
            self.out.push(0x02 | token_class.bits());
            self.out.push(args.len() as u8);
            self.out.extend_from_slice(&100u16.to_le_bytes());
            let end = self.out.len();
            // 점프 표: 처음 값부터 (점프 표 시작 기준), 마지막은 범위 밖 번호용
            let table_base = (table_at - choose_at - 4) as u16 + 2;
            let _ = table_base;
            for (k, s) in starts.iter().enumerate() {
                let v = (*s + (n + 1) * 2) as u16;
                self.out[table_at + k * 2..table_at + k * 2 + 2].copy_from_slice(&v.to_le_bytes());
            }
            let v = (after + (n + 1) * 2) as u16;
            self.out[table_at + n * 2..table_at + n * 2 + 2].copy_from_slice(&v.to_le_bytes());
            for g in gotos {
                let skip = (end - (g + 4) - 1) as u16;
                self.out[g + 2..g + 4].copy_from_slice(&skip.to_le_bytes());
            }
            return Ok(());
        }
        for (i, a) in args.iter().enumerate() {
            let (c, f) = param(i);
            self.emit(a, c, f)?;
        }
        match info.fixed {
            Some(n) if n as usize == args.len() => {
                self.u8(0x01 | token_class.bits());
                self.u16(info.index);
            }
            _ => {
                self.u8(0x02 | token_class.bits());
                self.u8(args.len() as u8);
                self.u16(info.index);
            }
        }
        let _ = info.name;
        Ok(())
    }

    /// BIFF8 함수표에 없는 함수: 첫 인자 자리에 이름(PtgName)을 두고 PtgFuncVar 255.
    /// Excel 2007 이후 함수는 `_xlfn.` 을 붙인 이름이다.
    fn user_func(
        &mut self,
        name: &str,
        args: &[Node],
        expected: Class,
        forced: bool,
    ) -> Result<(), EncodeError> {
        let full = if name.contains('.') && name.to_ascii_lowercase().starts_with("_xlfn.") {
            name.to_string()
        } else if lookup_function(name).is_some() {
            format!("_xlfn.{}", name.to_ascii_uppercase())
        } else {
            name.to_string()
        };
        let idx = self.res.func_name(&full);
        self.u8(0x23);
        self.u32(idx);
        for a in args {
            self.emit(a, Class::Ref, forced)?;
        }
        let token_class = if expected == Class::Arr || forced {
            Class::Arr
        } else {
            Class::Val
        };
        self.u8(0x02 | token_class.bits());
        self.u8((args.len() + 1) as u8);
        self.u16(0x00FF);
        Ok(())
    }
}

/// 수식 원문 → 토큰
pub fn encode(text: &str, opts: Options, res: &mut dyn Resolve) -> Result<Vec<u8>, EncodeError> {
    let text = text.strip_prefix('=').unwrap_or(text);
    let node = parse(text)?;
    let mut e = Emitter {
        out: Vec::new(),
        res,
        opts,
        volatile: false,
        in_union: false,
    };
    e.emit(&node, opts.root, opts.root == Class::Arr)?;
    let mut out = e.out;
    if e.volatile {
        let mut v = vec![0x19, 0x01, 0x00, 0x00];
        v.extend_from_slice(&out);
        out = v;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy::formula::{self, Context, Decoded};

    /// 시트 둘(Sheet1, Sheet2), 이름 하나(XYZ)
    struct Fake {
        xti: Vec<(Option<usize>, usize, usize)>,
        names: Vec<String>,
    }

    impl Resolve for Fake {
        fn sheet(&self, name: &str) -> Option<usize> {
            ["Sheet1", "Sheet2", "첫 시트"]
                .iter()
                .position(|s| *s == name)
        }
        fn ext_sheet(&self, _book: usize, name: &str) -> Option<usize> {
            (name == "Intro").then_some(0)
        }
        fn xti(&mut self, book: Option<usize>, first: usize, last: usize) -> u16 {
            if let Some(i) = self.xti.iter().position(|x| *x == (book, first, last)) {
                return i as u16;
            }
            self.xti.push((book, first, last));
            (self.xti.len() - 1) as u16
        }
        fn name(&mut self, name: &str, _sheet: Option<usize>) -> Option<u32> {
            self.names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(name))
                .map(|i| i as u32 + 1)
        }
        fn ext_name(&mut self, _book: usize, _name: &str) -> Option<(u16, u32)> {
            None
        }
        fn func_name(&mut self, name: &str) -> u32 {
            self.names.push(name.to_string());
            self.names.len() as u32
        }
    }

    fn hex(b: &[u8]) -> String {
        b.iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn enc(text: &str) -> String {
        let mut f = Fake {
            xti: Vec::new(),
            names: vec!["XYZ".into()],
        };
        hex(&encode(text, Options::default(), &mut f).unwrap())
    }

    // 아래 토큰열은 한셀 2024 가 같은 수식을 레거시로 저장한 것과 같다 (말뭉치 formulas 대조)
    #[test]
    fn matches_hancell_tokens() {
        assert_eq!(
            enc("A1+A2*A3-A4/A5"),
            "44 00 00 00 00 00 c0 44 01 00 00 00 00 c0 44 02 00 00 00 00 c0 05 03 44 03 00 00 00 00 c0 44 04 00 00 00 00 c0 06 04"
        );
        assert_eq!(
            enc("SUM(A1:A5)"),
            "25 00 00 00 00 04 00 00 00 00 c0 00 c0 19 10 00 00"
        );
        assert_eq!(
            enc("IF(A1>5,\"크다\",\"작다\")"),
            "44 00 00 00 00 00 c0 1e 05 00 0d 19 02 0c 00 17 02 00 01 6c d0 e4 b2 19 08 0f 00 17 02 00 01 91 c7 e4 b2 19 08 03 00 42 03 01 00"
        );
        assert_eq!(enc("ABS(-5)"), "1f 00 00 00 00 00 00 14 c0 41 18 00");
        assert_eq!(
            enc("CONCATENATE(B1,\"-\",B2)"),
            "44 00 00 00 00 01 c0 17 01 00 00 2d 44 01 00 00 00 01 c0 42 03 50 01"
        );
        assert_eq!(enc("TODAY()-TODAY()"), "19 01 00 00 41 dd 00 41 dd 00 04");
        assert_eq!(
            enc("SUM(A1,A2,A3)"),
            "24 00 00 00 00 00 c0 24 01 00 00 00 00 c0 24 02 00 00 00 00 c0 42 03 04 00"
        );
        assert_eq!(
            enc("SUMPRODUCT(A1:A2,A3:A4)"),
            "65 00 00 00 00 01 00 00 00 00 c0 00 c0 65 02 00 00 00 03 00 00 00 00 c0 00 c0 42 02 e4 00"
        );
        assert_eq!(
            enc("SUM(A:A)"),
            "25 00 00 00 00 ff ff 0f 00 00 40 00 40 19 10 00 00"
        );
        assert_eq!(
            enc("SUM(1:1)"),
            "25 00 00 00 00 00 00 00 00 00 80 ff bf 19 10 00 00"
        );
        assert_eq!(enc("Sheet2!A1*2"), "5a 00 00 00 00 00 00 00 c0 1e 02 00 05");
        assert_eq!(enc("XYZ"), "43 01 00 00 00");
        assert_eq!(enc("1.5E+20"), "1f 30 29 88 1a 56 43 20 44");
    }

    #[test]
    fn decodes_back_to_the_same_text() {
        let sheets = [
            Some("Sheet1".to_string()),
            Some("Sheet2".to_string()),
            Some("'첫 시트'".to_string()),
        ];
        for text in [
            "A1+A2*A3-A4/A5",
            "IF(A1>5,\"크다\",\"작다\")",
            "IF(A1,B1)",
            "ROUND(A3/7,2)",
            "$A$1+A$2+$A3",
            "VLOOKUP(30,A1:B5,2,FALSE)",
            "-A1",
            "A1%",
            "(A1+A2)*A3",
            "NOT(TRUE)",
            "A1<>A2",
            "\"문자\"",
            "CHOOSE(2,A1,B1,C1)",
            "'첫 시트'!$A$5",
            "Sheet1:Sheet2!A1",
            "SUM(A:A)+SUM($1:$2)",
            "IF(A1,,B1)",
        ] {
            let mut f = Fake {
                xti: Vec::new(),
                names: vec!["XYZ".into()],
            };
            let t = encode(text, Options::default(), &mut f).unwrap();
            let xti: Vec<Option<String>> = f
                .xti
                .iter()
                .map(|&(_, a, b)| {
                    let n = |i: usize| sheets[i].clone().unwrap().trim_matches('\'').to_string();
                    Some(formula::sheet_ref_prefix(
                        None,
                        &n(a),
                        (a != b).then(|| n(b)).as_deref(),
                    ))
                })
                .collect();
            let ctx = Context {
                xti_sheets: &xti,
                names: &f.names,
                xti_names: &[],
                base: CellRef::default(),
                xlsb: false,
                row_shift: 0,
                tables: &[],
            };
            assert_eq!(
                formula::decode(&t, &ctx).unwrap(),
                Decoded::Text(text.into()),
                "{text}"
            );
        }
    }

    #[test]
    fn shared_formula_offsets() {
        // H29 의 =IF(H28,…) 를 기준 셀 H29 로: 행 −1 → u32 2의 보수, 같은 열
        let mut f = Fake {
            xti: Vec::new(),
            names: Vec::new(),
        };
        let t = encode(
            "G29",
            Options {
                base: CellRef { row: 28, col: 7 },
                relative: true,
                ..Default::default()
            },
            &mut f,
        )
        .unwrap();
        assert_eq!(hex(&t), "4c 00 00 00 00 ff ff");
    }
}
