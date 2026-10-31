// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 원문(OOXML A1 표기, `=` 없이)의 참조를 찾아 고친다.
//!
//! 문자열·구조적 참조·함수 이름은 건드리지 않고 참조만 새 글로 바꾼다. 바뀌지 않은 참조는 원문 그대로 둔다.
//! 행·열 삽입·삭제, 시트 이름 바꾸기·삭제, 잘라 붙이기의 규칙은 한셀 COM 대조 실험에서 확인했다
//! (docs/editing.md 3절):
//! - 삽입: 삽입 위치 이후의 칸은 밀리고, 범위 안에 삽입하면 범위가 넓어진다. 시트 끝을 넘으면 끝에 머문다
//!   (`A1048576` 은 그대로).
//! - 삭제: 지운 칸 참조는 `#REF!` (시트 이름은 남는다: `Sheet1!#REF!`), 범위는 남은 칸으로 줄어든다
//!   (`A6:A7` 에서 5–6행 삭제 → `A5:A5`), 범위 전체를 지우면 `#REF!`.
//! - 3차원 참조(`Sheet1:Sheet2!A5`)와 외부 문서 참조는 행·열 편집으로 바꾸지 않는다.

use crate::model::CellRef;

/// 시트의 행·열 수 (한셀 2024)
pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// 참조 한 축 (0부터) + `$`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ax {
    pub n: u32,
    pub abs: bool,
}

/// 영역 참조
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Cell {
        col: Ax,
        row: Ax,
    },
    Rect {
        c0: Ax,
        r0: Ax,
        c1: Ax,
        r1: Ax,
    },
    /// 열 전체 `A:C`
    Cols {
        c0: Ax,
        c1: Ax,
    },
    /// 행 전체 `1:3`
    Rows {
        r0: Ax,
        r1: Ax,
    },
}

impl Area {
    /// (첫 행, 첫 열, 끝 행, 끝 열) — 열·행 전체는 시트 끝까지
    pub fn bounds(&self) -> (u32, u32, u32, u32) {
        match *self {
            Area::Cell { col, row } => (row.n, col.n, row.n, col.n),
            Area::Rect { c0, r0, c1, r1 } => (r0.n, c0.n, r1.n, c1.n),
            Area::Cols { c0, c1 } => (0, c0.n, MAX_ROWS - 1, c1.n),
            Area::Rows { r0, r1 } => (r0.n, 0, r1.n, MAX_COLS - 1),
        }
    }

    /// 끝이 앞보다 작은 범위를 바로잡는다: 열·행을 따로 작은 쪽이 앞으로, `$` 는 그 열·행을 따라간다
    /// (`$B$2:$A$1` → `$A$1:$B$2`, `B$2:$A1` → `$A1:B$2`, `3:1` → `1:3` — 한셀이 입력에서 한다)
    pub fn ordered(self) -> Area {
        let sw = |a: Ax, b: Ax| if a.n <= b.n { (a, b) } else { (b, a) };
        match self {
            Area::Rect { c0, r0, c1, r1 } => {
                let (c0, c1) = sw(c0, c1);
                let (r0, r1) = sw(r0, r1);
                Area::Rect { c0, r0, c1, r1 }
            }
            Area::Cols { c0, c1 } => {
                let (c0, c1) = sw(c0, c1);
                Area::Cols { c0, c1 }
            }
            Area::Rows { r0, r1 } => {
                let (r0, r1) = sw(r0, r1);
                Area::Rows { r0, r1 }
            }
            cell => cell,
        }
    }

    pub fn text(&self) -> String {
        match *self {
            Area::Cell { col, row } => format!("{}{}", col_text(col), row_text(row)),
            Area::Rect { c0, r0, c1, r1 } => format!(
                "{}{}:{}{}",
                col_text(c0),
                row_text(r0),
                col_text(c1),
                row_text(r1)
            ),
            Area::Cols { c0, c1 } => format!("{}:{}", col_text(c0), col_text(c1)),
            Area::Rows { r0, r1 } => format!("{}:{}", row_text(r0), row_text(r1)),
        }
    }

    /// `$` 없는 직사각형 (조건부 서식·유효성의 sqref 처럼)
    pub fn rect(r0: u32, c0: u32, r1: u32, c1: u32) -> Area {
        let a = |n| Ax { n, abs: false };
        if r0 == r1 && c0 == c1 {
            Area::Cell {
                col: a(c0),
                row: a(r0),
            }
        } else {
            Area::Rect {
                c0: a(c0),
                r0: a(r0),
                c1: a(c1),
                r1: a(r1),
            }
        }
    }

    /// 영역을 (drow, dcol) 만큼 옮긴다 (`$` 와 상관없이). 시트 밖이면 None
    pub fn offset(&self, drow: i64, dcol: i64) -> Option<Area> {
        let mv = |a: Ax, d: i64, max: u32| -> Option<Ax> {
            let n = a.n as i64 + d;
            (0..max as i64).contains(&n).then_some(Ax {
                n: n as u32,
                abs: a.abs,
            })
        };
        Some(match *self {
            Area::Cell { col, row } => Area::Cell {
                col: mv(col, dcol, MAX_COLS)?,
                row: mv(row, drow, MAX_ROWS)?,
            },
            Area::Rect { c0, r0, c1, r1 } => Area::Rect {
                c0: mv(c0, dcol, MAX_COLS)?,
                r0: mv(r0, drow, MAX_ROWS)?,
                c1: mv(c1, dcol, MAX_COLS)?,
                r1: mv(r1, drow, MAX_ROWS)?,
            },
            Area::Cols { c0, c1 } => Area::Cols {
                c0: mv(c0, dcol, MAX_COLS)?,
                c1: mv(c1, dcol, MAX_COLS)?,
            },
            Area::Rows { r0, r1 } => Area::Rows {
                r0: mv(r0, drow, MAX_ROWS)?,
                r1: mv(r1, drow, MAX_ROWS)?,
            },
        })
    }
}

fn col_text(a: Ax) -> String {
    format!("{}{}", if a.abs { "$" } else { "" }, CellRef::col_name(a.n))
}

fn row_text(a: Ax) -> String {
    format!("{}{}", if a.abs { "$" } else { "" }, a.n + 1)
}

/// 시트 접두 (`Sheet1!`, `'My Data'!`, `Sheet1:Sheet3!`, `[1]Sheet1!`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    /// 외부 통합 문서 번호 (`[n]` 의 n)
    pub book: Option<String>,
    pub first: String,
    /// 3차원 참조의 끝 시트
    pub last: Option<String>,
}

/// 접두 뒤의 몸
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Area(Area),
    /// `#REF!`
    RefError,
    /// 시트 범위 이름 등 (`Sheet1!이름`)
    Name(String),
}

/// 수식 안의 참조 하나
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// 원문 전체
    pub raw: String,
    /// 접두 원문 (`!` 포함, 없으면 빈 글)
    pub prefix_raw: String,
    pub prefix: Option<Prefix>,
    pub body: Body,
}

/// 수식 조각: 글 그대로이거나 참조
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Text(String),
    Ref(Ref),
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\')
}

fn word_end(ch: &[char], start: usize) -> usize {
    let mut e = start;
    while e < ch.len() && is_word(ch[e]) {
        e += 1;
    }
    e
}

/// 열 (`$` + 글자 1–3개, XFD 까지)
fn parse_col(w: &str) -> Option<Ax> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let mut n: u32 = 0;
    for b in s.bytes() {
        n = n * 26 + (b.to_ascii_uppercase() - b'A') as u32 + 1;
    }
    (n <= MAX_COLS).then_some(Ax { n: n - 1, abs })
}

/// 행 (`$` + 숫자)
fn parse_row(w: &str) -> Option<Ax> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    if s.is_empty() || s.len() > 7 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = s.parse().ok()?;
    (1..=MAX_ROWS).contains(&n).then_some(Ax { n: n - 1, abs })
}

/// 셀 (`$A$1`)
fn parse_cell(w: &str) -> Option<(Ax, Ax)> {
    let body = w.strip_prefix('$').unwrap_or(w);
    let letters = body.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if letters == 0 {
        return None;
    }
    let split = w.len() - body.len() + letters;
    Some((parse_col(&w[..split])?, parse_row(&w[split..])?))
}

/// `w1:w2` 영역
fn parse_pair(w1: &str, w2: &str) -> Option<Area> {
    if let (Some((c0, r0)), Some((c1, r1))) = (parse_cell(w1), parse_cell(w2)) {
        return Some(Area::Rect { c0, r0, c1, r1 });
    }
    if let (Some(c0), Some(c1)) = (parse_col(w1), parse_col(w2)) {
        return Some(Area::Cols { c0, c1 });
    }
    if let (Some(r0), Some(r1)) = (parse_row(w1), parse_row(w2)) {
        return Some(Area::Rows { r0, r1 });
    }
    None
}

/// 영역 글 하나 (`A1`, `$A$1:B2`, `A:C`, `1:3`)
pub fn parse_area(s: &str) -> Option<Area> {
    let s = s.trim();
    match s.split_once(':') {
        Some((a, b)) => parse_pair(a, b),
        None => parse_cell(s).map(|(col, row)| Area::Cell { col, row }),
    }
}

/// 공백으로 구분한 영역 목록 (`sqref`)
pub fn parse_sqref(s: &str) -> Vec<Area> {
    s.split_whitespace().filter_map(parse_area).collect()
}

pub fn sqref_text(list: &[Area]) -> String {
    list.iter().map(Area::text).collect::<Vec<_>>().join(" ")
}

/// 접두 뒤의 몸을 읽는다: (몸, 끝 위치). 읽을 수 없으면 None
fn parse_body(ch: &[char], k: usize) -> Option<(Body, usize)> {
    let rest: String = ch[k..ch.len().min(k + 5)].iter().collect();
    if rest.eq_ignore_ascii_case("#REF!") {
        return Some((Body::RefError, k + 5));
    }
    let e = word_end(ch, k);
    if e == k {
        return None;
    }
    let w: String = ch[k..e].iter().collect();
    if ch.get(e) == Some(&':') {
        let e2 = word_end(ch, e + 1);
        if e2 > e + 1 {
            let w2: String = ch[e + 1..e2].iter().collect();
            if let Some(a) = parse_pair(&w, &w2) {
                return Some((Body::Area(a), e2));
            }
        }
    }
    if let Some((col, row)) = parse_cell(&w) {
        return Some((Body::Area(Area::Cell { col, row }), e));
    }
    Some((Body::Name(w), e))
}

/// 따옴표 시트 이름 안 (`[1]Sheet 1:Sheet 3`)
fn split_prefix(name: &str) -> Prefix {
    let (book, rest) = match name.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((b, r)) if b.bytes().all(|c| c.is_ascii_digit()) => (Some(b.to_string()), r),
        _ => (None, name),
    };
    let (first, last) = match rest.split_once(':') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (rest.to_string(), None),
    };
    Prefix { book, first, last }
}

/// 수식을 조각으로 나눈다
pub fn segments(f: &str) -> Vec<Seg> {
    let ch: Vec<char> = f.chars().collect();
    let mut out: Vec<Seg> = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Seg>| {
        if !text.is_empty() {
            out.push(Seg::Text(std::mem::take(text)));
        }
    };
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        match c {
            '"' => {
                text.push(c);
                i += 1;
                while i < ch.len() {
                    text.push(ch[i]);
                    if ch[i] == '"' {
                        if ch.get(i + 1) == Some(&'"') {
                            text.push('"');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '\'' => {
                // 따옴표 시트 이름 → '!' → 몸
                let mut j = i + 1;
                let mut name = String::new();
                let mut closed = false;
                while j < ch.len() {
                    if ch[j] == '\'' {
                        if ch.get(j + 1) == Some(&'\'') {
                            name.push('\'');
                            j += 2;
                            continue;
                        }
                        closed = true;
                        j += 1;
                        break;
                    }
                    name.push(ch[j]);
                    j += 1;
                }
                if closed
                    && ch.get(j) == Some(&'!')
                    && let Some((body, end)) = parse_body(&ch, j + 1)
                {
                    flush(&mut text, &mut out);
                    out.push(Seg::Ref(Ref {
                        raw: ch[i..end].iter().collect(),
                        prefix_raw: ch[i..=j].iter().collect(),
                        prefix: Some(split_prefix(&name)),
                        body,
                    }));
                    i = end;
                } else {
                    text.extend(&ch[i..j]);
                    i = j;
                }
            }
            '[' => {
                // 외부 문서 번호 `[1]Sheet1!` (앞이 이름 글자가 아니고 숫자만) 또는 구조적 참조
                let prev = i.checked_sub(1).map(|p| ch[p]);
                let digits = ch[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
                let close = i + 1 + digits;
                let mut done = false;
                if digits > 0
                    && ch.get(close) == Some(&']')
                    && !prev.is_some_and(|p| is_word(p) || p == ']')
                {
                    let e = word_end(&ch, close + 1);
                    let mut k = e;
                    let mut last = None;
                    if ch.get(e) == Some(&':') {
                        let e2 = word_end(&ch, e + 1);
                        if e2 > e + 1 && ch.get(e2) == Some(&'!') {
                            last = Some(ch[e + 1..e2].iter().collect::<String>());
                            k = e2;
                        }
                    }
                    if ch.get(k) == Some(&'!')
                        && let Some((body, end)) = parse_body(&ch, k + 1)
                    {
                        flush(&mut text, &mut out);
                        out.push(Seg::Ref(Ref {
                            raw: ch[i..end].iter().collect(),
                            prefix_raw: ch[i..=k].iter().collect(),
                            prefix: Some(Prefix {
                                book: Some(ch[i + 1..close].iter().collect()),
                                first: ch[close + 1..e].iter().collect(),
                                last,
                            }),
                            body,
                        }));
                        i = end;
                        done = true;
                    }
                }
                if !done {
                    let mut depth = 0;
                    while i < ch.len() {
                        text.push(ch[i]);
                        match ch[i] {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            '#' => {
                text.push(c);
                i += 1;
                while i < ch.len()
                    && (ch[i].is_ascii_alphanumeric() || matches!(ch[i], '/' | '!' | '?' | '_'))
                {
                    text.push(ch[i]);
                    let bang = ch[i] == '!' || ch[i] == '?';
                    i += 1;
                    if bang {
                        break;
                    }
                }
            }
            c if is_word(c) => {
                let e = word_end(&ch, i);
                let w: String = ch[i..e].iter().collect();
                let next = ch.get(e).copied();
                if next == Some('(') || next == Some('[') {
                    text.push_str(&w);
                    i = e;
                    continue;
                }
                if next == Some('!')
                    && let Some((body, end)) = parse_body(&ch, e + 1)
                {
                    flush(&mut text, &mut out);
                    out.push(Seg::Ref(Ref {
                        raw: ch[i..end].iter().collect(),
                        prefix_raw: ch[i..=e].iter().collect(),
                        prefix: Some(Prefix {
                            book: None,
                            first: w,
                            last: None,
                        }),
                        body,
                    }));
                    i = end;
                    continue;
                }
                if next == Some(':') {
                    let e2 = word_end(&ch, e + 1);
                    if e2 > e + 1 {
                        let w2: String = ch[e + 1..e2].iter().collect();
                        if ch.get(e2) == Some(&'!')
                            && let Some((body, end)) = parse_body(&ch, e2 + 1)
                        {
                            flush(&mut text, &mut out);
                            out.push(Seg::Ref(Ref {
                                raw: ch[i..end].iter().collect(),
                                prefix_raw: ch[i..=e2].iter().collect(),
                                prefix: Some(Prefix {
                                    book: None,
                                    first: w,
                                    last: Some(w2),
                                }),
                                body,
                            }));
                            i = end;
                            continue;
                        }
                        if let Some(a) = parse_pair(&w, &w2) {
                            flush(&mut text, &mut out);
                            out.push(Seg::Ref(Ref {
                                raw: ch[i..e2].iter().collect(),
                                prefix_raw: String::new(),
                                prefix: None,
                                body: Body::Area(a),
                            }));
                            i = e2;
                            continue;
                        }
                    }
                }
                match parse_cell(&w) {
                    Some((col, row)) => {
                        flush(&mut text, &mut out);
                        out.push(Seg::Ref(Ref {
                            raw: w,
                            prefix_raw: String::new(),
                            prefix: None,
                            body: Body::Area(Area::Cell { col, row }),
                        }));
                    }
                    None => text.push_str(&w),
                }
                i = e;
            }
            _ => {
                text.push(c);
                i += 1;
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

/// 참조마다 `g` 로 새 글을 정한다 (None 이면 원문). 하나도 바뀌지 않으면 원래 수식 그대로
pub fn map_refs(f: &str, mut g: impl FnMut(&Ref) -> Option<String>) -> String {
    let segs = segments(f);
    let mut changed = false;
    let mut out = String::with_capacity(f.len() + 8);
    for s in &segs {
        match s {
            Seg::Text(t) => out.push_str(t),
            Seg::Ref(r) => match g(r) {
                Some(t) if t != r.raw => {
                    changed = true;
                    out.push_str(&t);
                }
                _ => out.push_str(&r.raw),
            },
        }
    }
    if changed { out } else { f.to_string() }
}

/// 시트 이름 비교 (대소문자 무시 — 한셀·Excel 의 시트 이름은 대소문자를 가리지 않는다)
pub fn same_sheet(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// 참조가 가리키는 시트가 `target` 하나인가 (`here` = 수식이 있는 시트). 3차원·외부 참조는 아니다
fn on_sheet(r: &Ref, here: &str, target: &str) -> bool {
    match &r.prefix {
        None => same_sheet(here, target),
        Some(p) => p.book.is_none() && p.last.is_none() && same_sheet(&p.first, target),
    }
}

/// 시트 접두 글 (`!` 포함). 이름에 따옴표가 필요하면 감싼다
pub fn prefix_text(first: &str, last: Option<&str>) -> String {
    format!(
        "{}!",
        crate::legacy::formula::sheet_ref_prefix(None, first, last)
    )
}

// ---------------------------------------------------------------------------
// 행·열 삽입·삭제
// ---------------------------------------------------------------------------

/// 행·열 구조 편집 (0부터)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    InsRows { at: u32, n: u32 },
    DelRows { at: u32, n: u32 },
    InsCols { at: u32, n: u32 },
    DelCols { at: u32, n: u32 },
}

impl Shift {
    pub fn on_rows(self) -> bool {
        matches!(self, Shift::InsRows { .. } | Shift::DelRows { .. })
    }

    fn parts(self) -> (bool, u32, u32) {
        match self {
            Shift::InsRows { at, n } | Shift::InsCols { at, n } => (true, at, n),
            Shift::DelRows { at, n } | Shift::DelCols { at, n } => (false, at, n),
        }
    }

    fn max(self) -> u32 {
        if self.on_rows() { MAX_ROWS } else { MAX_COLS }
    }

    /// 한 축의 칸 하나 (`rows` = 행 축인가). 지워지면 None
    pub fn point(self, rows: bool, v: u32) -> Option<u32> {
        if rows != self.on_rows() {
            return Some(v);
        }
        let (ins, at, n) = self.parts();
        if ins {
            Some(if v >= at {
                (v + n).min(self.max() - 1)
            } else {
                v
            })
        } else if v < at {
            Some(v)
        } else if v >= at + n {
            Some(v - n)
        } else {
            None
        }
    }

    /// 한 축의 범위 a..=b. `adjacent` 면 바로 앞(위·왼쪽)에서 끝나는 범위도 넓힌다 — 삽입한 행·열은 앞
    /// 행·열의 서식을 물려받으므로 조건부 서식·유효성 범위가 늘어난다 (한셀 대조 실험)
    pub fn span(self, rows: bool, a: u32, b: u32, adjacent: bool) -> Option<(u32, u32)> {
        if rows != self.on_rows() {
            return Some((a, b));
        }
        let (ins, at, n) = self.parts();
        let max = self.max() - 1;
        if ins {
            if a >= at {
                Some(((a + n).min(max), (b + n).min(max)))
            } else if b >= at || (adjacent && at > 0 && b + 1 == at) {
                Some((a, (b + n).min(max)))
            } else {
                Some((a, b))
            }
        } else {
            let end = at + n;
            let na = if a < at {
                a
            } else if a >= end {
                a - n
            } else {
                at
            };
            let nb = if b < at {
                b
            } else if b >= end {
                b - n
            } else if at == 0 {
                return None;
            } else {
                at - 1
            };
            (nb >= na).then_some((na, nb))
        }
    }

    /// 영역 하나 (None = 모두 지워짐)
    pub fn area(self, a: Area, adjacent: bool) -> Option<Area> {
        let ax = |x: Ax, n: u32| Ax { n, abs: x.abs };
        Some(match a {
            // 조건부 서식·유효성의 칸 하나도 바로 앞에 넣은 행·열만큼 넓어진다 (`O2` → `O2:O4`)
            Area::Cell { col, row } if adjacent => {
                let (x0, x1) = self.span(false, col.n, col.n, true)?;
                let (y0, y1) = self.span(true, row.n, row.n, true)?;
                if x0 == x1 && y0 == y1 {
                    Area::Cell {
                        col: ax(col, x0),
                        row: ax(row, y0),
                    }
                } else {
                    Area::Rect {
                        c0: ax(col, x0),
                        r0: ax(row, y0),
                        c1: ax(col, x1),
                        r1: ax(row, y1),
                    }
                }
            }
            Area::Cell { col, row } => Area::Cell {
                col: ax(col, self.point(false, col.n)?),
                row: ax(row, self.point(true, row.n)?),
            },
            Area::Rect { c0, r0, c1, r1 } => {
                let (x0, x1) = self.span(false, c0.n, c1.n, adjacent)?;
                let (y0, y1) = self.span(true, r0.n, r1.n, adjacent)?;
                Area::Rect {
                    c0: ax(c0, x0),
                    r0: ax(r0, y0),
                    c1: ax(c1, x1),
                    r1: ax(r1, y1),
                }
            }
            Area::Cols { c0, c1 } => {
                let (x0, x1) = self.span(false, c0.n, c1.n, adjacent)?;
                Area::Cols {
                    c0: ax(c0, x0),
                    c1: ax(c1, x1),
                }
            }
            Area::Rows { r0, r1 } => {
                let (y0, y1) = self.span(true, r0.n, r1.n, adjacent)?;
                Area::Rows {
                    r0: ax(r0, y0),
                    r1: ax(r1, y1),
                }
            }
        })
    }

    /// 편집 뒤의 칸 → 편집 앞의 칸 (삽입한 칸이면 None)
    pub fn inverse(self, rows: bool, v: u32) -> Option<u32> {
        if rows != self.on_rows() {
            return Some(v);
        }
        let (ins, at, n) = self.parts();
        if ins {
            if v >= at + n {
                Some(v - n)
            } else if v >= at {
                None
            } else {
                Some(v)
            }
        } else if v >= at {
            Some(v + n)
        } else {
            Some(v)
        }
    }
}

/// 수식 안의 `target` 시트 참조를 행·열 편집에 맞춘다. `here` 는 수식이 있는 시트 (이름 정의·차트처럼 시트가
/// 없으면 빈 글 — 접두 없는 참조는 건드리지 않는다)
pub fn shift_formula(f: &str, here: &str, target: &str, op: Shift) -> String {
    map_refs(f, |r| {
        if !on_sheet(r, here, target) {
            return None;
        }
        let Body::Area(a) = r.body else { return None };
        Some(match op.area(a, false) {
            Some(b) if b == a => return None,
            Some(b) => format!("{}{}", r.prefix_raw, b.text()),
            None => format!("{}#REF!", r.prefix_raw),
        })
    })
}

/// 셀 삽입·삭제(한 띠만 미는 것)의 참조: 미는 축과 엇갈린 축의 범위가 띠(`band`) 안에 모두 든 참조만 행·열 편집처럼
/// 바꾼다 (띠에 일부만 걸친 참조는 그대로 — 셀 삽입·삭제 대조 실험)
/// 모든 축이 절대 참조(`$A$1`)인 것만 [`shift_formula_band`] 처럼 옮긴다 (조건부 서식·유효성 수식: 상대 참조는 범위의
/// 왼쪽 위 칸 기준이라 따로 맞춘다)
pub fn shift_formula_band_abs(
    f: &str,
    here: &str,
    target: &str,
    op: Shift,
    band: (u32, u32),
) -> String {
    map_refs(f, |r| {
        if !on_sheet(r, here, target) {
            return None;
        }
        let Body::Area(a) = r.body else { return None };
        let abs = match a {
            Area::Cell { col, row } => col.abs && row.abs,
            Area::Rect { c0, r0, c1, r1 } => c0.abs && r0.abs && c1.abs && r1.abs,
            Area::Cols { c0, c1 } => c0.abs && c1.abs,
            Area::Rows { r0, r1 } => r0.abs && r1.abs,
        };
        if !abs {
            return None;
        }
        let (r0, c0, r1, c1) = a.bounds();
        let cross = if op.on_rows() { (c0, c1) } else { (r0, r1) };
        if cross.0 < band.0 || cross.1 > band.1 {
            return None;
        }
        Some(match op.area(a, false) {
            Some(b) if b == a => return None,
            Some(b) => format!("{}{}", r.prefix_raw, b.text()),
            None => format!("{}#REF!", r.prefix_raw),
        })
    })
}

pub fn shift_formula_band(
    f: &str,
    here: &str,
    target: &str,
    op: Shift,
    band: (u32, u32),
) -> String {
    map_refs(f, |r| {
        if !on_sheet(r, here, target) {
            return None;
        }
        let Body::Area(a) = r.body else { return None };
        let (r0, c0, r1, c1) = a.bounds();
        let cross = if op.on_rows() { (c0, c1) } else { (r0, r1) };
        if cross.0 < band.0 || cross.1 > band.1 {
            return None;
        }
        Some(match op.area(a, false) {
            Some(b) if b == a => return None,
            Some(b) => format!("{}{}", r.prefix_raw, b.text()),
            None => format!("{}#REF!", r.prefix_raw),
        })
    })
}

/// 조건부 서식·유효성처럼 범위의 왼쪽 위 칸에 걸린 수식: 상대 참조를 옛 기준 칸(`old_anchor`)에서 새 범위의
/// 왼쪽 위 칸이 편집 앞에 있던 자리(`from`)로 옮긴 뒤 행·열 편집을 맞춘다 (한셀 대조 실험 — 1–3행 삭제로
/// `B1:B10` 이 `B1:B7` 이 되면 `C1>$A$5` 는 `C1>$A$2`).
pub fn shift_anchored(
    f: &str,
    here: &str,
    target: &str,
    op: Shift,
    old_anchor: (u32, u32),
    from: (u32, u32),
) -> String {
    let drow = from.0 as i64 - old_anchor.0 as i64;
    let dcol = from.1 as i64 - old_anchor.1 as i64;
    let moved = crate::formula_text::shift_references(f, drow, dcol);
    shift_formula(&moved, here, target, op)
}

// ---------------------------------------------------------------------------
// 시트 이름 바꾸기·삭제
// ---------------------------------------------------------------------------

/// 시트 이름이 바뀌었다: 그 시트를 가리키는 접두를 새 이름으로 (따옴표는 필요할 때만)
pub fn rename_sheet(f: &str, old: &str, new: &str) -> String {
    map_refs(f, |r| {
        let p = r.prefix.as_ref()?;
        if p.book.is_some() {
            return None;
        }
        let hit_first = same_sheet(&p.first, old);
        let hit_last = p.last.as_deref().is_some_and(|l| same_sheet(l, old));
        if !hit_first && !hit_last {
            return None;
        }
        let first = if hit_first { new } else { p.first.as_str() };
        let last = p
            .last
            .as_deref()
            .map(|l| if same_sheet(l, old) { new } else { l });
        Some(format!(
            "{}{}",
            prefix_text(first, last),
            &r.raw[r.prefix_raw.len()..]
        ))
    })
}

/// 시트를 지웠다: 그 시트만 가리키는 참조는 `#REF!` (접두째), 3차원 참조의 끝이면 그 자리에 온 시트로
/// (한셀 대조 실험: `Sheet1:Sheet2!A5` 에서 Sheet2 를 지우면 `'Sheet1:Data 3'!A5`). `after` 는 지운 뒤의 시트
/// 이름들, `index` 는 지운 시트가 있던 자리
pub fn delete_sheet(f: &str, deleted: &str, after: &[String], index: usize) -> String {
    let next = after
        .get(index)
        .or_else(|| index.checked_sub(1).and_then(|i| after.get(i)));
    map_refs(f, |r| {
        let p = r.prefix.as_ref()?;
        if p.book.is_some() {
            return None;
        }
        match &p.last {
            None if same_sheet(&p.first, deleted) => Some("#REF!".into()),
            Some(last) if same_sheet(&p.first, deleted) || same_sheet(last, deleted) => {
                let n = next?;
                let first = if same_sheet(&p.first, deleted) {
                    n.as_str()
                } else {
                    p.first.as_str()
                };
                let last = if same_sheet(last, deleted) {
                    n.as_str()
                } else {
                    last.as_str()
                };
                Some(format!(
                    "{}{}",
                    prefix_text(first, Some(last)),
                    &r.raw[r.prefix_raw.len()..]
                ))
            }
            _ => None,
        }
    })
}

// ---------------------------------------------------------------------------
// 잘라 붙이기
// ---------------------------------------------------------------------------

/// 셀 옮기기 (잘라 붙이기): 원본 범위 안에 모두 든 참조는 옮긴 자리로, 덮어쓴 대상 범위만 가리키던 참조는
/// `#REF!`. `here` 는 수식이 있는 시트
pub struct Move<'a> {
    pub src_sheet: &'a str,
    pub dst_sheet: &'a str,
    /// 원본 (첫 행, 첫 열, 끝 행, 끝 열)
    pub src: (u32, u32, u32, u32),
    pub drow: i64,
    pub dcol: i64,
}

impl Move<'_> {
    pub fn dst(&self) -> (u32, u32, u32, u32) {
        let (r0, c0, r1, c1) = self.src;
        (
            (r0 as i64 + self.drow) as u32,
            (c0 as i64 + self.dcol) as u32,
            (r1 as i64 + self.drow) as u32,
            (c1 as i64 + self.dcol) as u32,
        )
    }
}

fn inside(b: (u32, u32, u32, u32), outer: (u32, u32, u32, u32)) -> bool {
    b.0 >= outer.0 && b.1 >= outer.1 && b.2 <= outer.2 && b.3 <= outer.3
}

pub fn move_formula(f: &str, here: &str, m: &Move) -> String {
    let same = same_sheet(m.src_sheet, m.dst_sheet);
    map_refs(f, |r| {
        let Body::Area(a) = r.body else { return None };
        if matches!(a, Area::Cols { .. } | Area::Rows { .. }) {
            return None;
        }
        let b = a.bounds();
        if on_sheet(r, here, m.src_sheet) && inside(b, m.src) {
            let moved = a.offset(m.drow, m.dcol)?;
            let prefix = if same {
                r.prefix_raw.clone()
            } else if r.prefix.is_none() && same_sheet(here, m.dst_sheet) {
                String::new()
            } else {
                prefix_text(m.dst_sheet, None)
            };
            return Some(format!("{prefix}{}", moved.text()));
        }
        if on_sheet(r, here, m.dst_sheet) && inside(b, m.dst()) {
            return Some(format!("{}#REF!", r.prefix_raw));
        }
        None
    })
}

// ---------------------------------------------------------------------------
// 입력한 수식 고르기
// ---------------------------------------------------------------------------

/// 입력한 수식을 한셀이 저장하는 꼴로 (한셀 대조 실험): 함수·참조·논리값·오류는 대문자, 시트·이름은 문서에
/// 있는 이름의 대소문자, 숫자는 다시 쓴 꼴(`1E3` → `1000`, `1.50` → `1.5`), 연산자 둘레의 공백은 지우고
/// (교차 연산자 공백은 하나), 닫지 않은 괄호는 닫는다. `canon` 은 시트·이름의 문서 표기를 돌려준다
pub fn normalize(input: &str, canon: &dyn Fn(&str) -> Option<String>) -> String {
    normalize_checked(input, canon).text
}

/// [`normalize`] 의 결과와 문법 검사
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub text: String,
    /// 닫지 않은 괄호가 있었다 (입력은 닫아 주고, CSV 열기는 글로 둔다)
    pub unclosed: bool,
    /// 수식 문법에 맞지 않는다: 참조가 아닌 피연산자 사이의 공백(`1 2`), 범위 연산자 `:` 옆의 숫자·글
    /// (`1:2.5`), 지수가 -309 보다 작거나 넘치는 수 (`-1E-400`) — 한셀은 이런 CSV 필드를 글로 둔다
    pub invalid: bool,
}

/// 함수 이름 앞의 `_xlfn.`·`_xlws.`·`_xll.` 접두사를 뗀다
fn strip_fn_prefix(w: &str) -> &str {
    let l = w.to_ascii_lowercase();
    for p in ["_xlfn._xlws.", "_xlfn.", "_xlws.", "_xll."] {
        if l.starts_with(p) {
            return &w[p.len()..];
        }
    }
    w
}

/// [`normalize`] + 문법 검사
pub fn normalize_checked(input: &str, canon: &dyn Fn(&str) -> Option<String>) -> Normalized {
    normalize_scoped(input, canon, &|_, _| false)
}

/// [`normalize_checked`] + 시트 범위 이름: `local(시트, 이름)` 이 참이면 `시트!이름` 의 시트를 남기고, 아니면 뗀다
/// (한셀: `Data!nm` → `nm`, 그 시트에 같은 이름이 따로 있을 때만 `S1!nm` 을 남긴다)
pub fn normalize_scoped(
    input: &str,
    canon: &dyn Fn(&str) -> Option<String>,
    local: &dyn Fn(&str, &str) -> bool,
) -> Normalized {
    #[derive(Debug, Clone, PartialEq)]
    enum T {
        /// 그대로 쓸 글 (문자열·구조적 참조·연산자)
        Raw(String),
        Space,
        /// 피연산자 (참조·이름·숫자·괄호 닫기 …): 교차 연산자 판단용 (참조처럼 쓰이는가)
        Operand(String, bool),
        Open(String),
    }
    let mut toks: Vec<T> = Vec::new();
    let mut invalid = false;
    for seg in segments(input) {
        match seg {
            Seg::Ref(r) => {
                // 이름 앞의 시트는 그 시트에 같은 이름이 따로 없으면 뗀다
                let drop_sheet = match (&r.prefix, &r.body) {
                    (Some(p), Body::Name(n)) if p.book.is_none() && p.last.is_none() => {
                        let sh = canon(&p.first).unwrap_or_else(|| p.first.clone());
                        let nm = canon(n).unwrap_or_else(|| n.clone());
                        !local(&sh, &nm)
                    }
                    _ => false,
                };
                let prefix = match &r.prefix {
                    _ if drop_sheet => String::new(),
                    // 이름 앞의 시트는 셀 주소처럼 보여도 따옴표를 붙이지 않는다 (`S1!nm`, 참조는 `'S1'!A1`)
                    Some(p) if matches!(r.body, Body::Name(_)) && p.book.is_none() && p.last.is_none()
                        && canon(&p.first).unwrap_or_else(|| p.first.clone()).chars().all(|c| c.is_alphanumeric() || c == '_') =>
                    {
                        format!("{}!", canon(&p.first).unwrap_or_else(|| p.first.clone()))
                    }
                    Some(p) if p.book.is_none() => {
                        let f = canon(&p.first).unwrap_or_else(|| p.first.clone());
                        let l = p
                            .last
                            .as_ref()
                            .map(|l| canon(l).unwrap_or_else(|| l.clone()));
                        prefix_text(&f, l.as_deref())
                    }
                    Some(_) => r.prefix_raw.clone(),
                    None => String::new(),
                };
                let body = match &r.body {
                    Body::Area(a) => a.ordered().text(),
                    Body::RefError => "#REF!".into(),
                    Body::Name(n) => canon(n).unwrap_or_else(|| n.clone()),
                };
                toks.push(T::Operand(format!("{prefix}{body}"), true));
            }
            Seg::Text(t) => {
                let ch: Vec<char> = t.chars().collect();
                let mut i = 0;
                while i < ch.len() {
                    let c = ch[i];
                    if c == '"' {
                        let mut s = String::from('"');
                        i += 1;
                        while i < ch.len() {
                            s.push(ch[i]);
                            if ch[i] == '"' {
                                if ch.get(i + 1) == Some(&'"') {
                                    s.push('"');
                                    i += 2;
                                    continue;
                                }
                                i += 1;
                                break;
                            }
                            i += 1;
                        }
                        toks.push(T::Operand(s, false));
                    } else if c == '[' {
                        let mut s = String::new();
                        let mut depth = 0;
                        while i < ch.len() {
                            s.push(ch[i]);
                            match ch[i] {
                                '[' => depth += 1,
                                ']' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        i += 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            i += 1;
                        }
                        // 표 이름 바로 뒤의 구조적 참조는 앞 이름에 붙인다
                        match toks.last_mut() {
                            Some(T::Operand(prev, _)) => prev.push_str(&s),
                            _ => toks.push(T::Operand(s, true)),
                        }
                    } else if c == '#' {
                        let mut s = String::from('#');
                        i += 1;
                        while i < ch.len()
                            && (ch[i].is_ascii_alphanumeric()
                                || matches!(ch[i], '/' | '!' | '?' | '_'))
                        {
                            s.push(ch[i]);
                            let bang = ch[i] == '!' || ch[i] == '?';
                            i += 1;
                            if bang {
                                break;
                            }
                        }
                        // 한셀이 받는 오류 글자는 일곱 가지뿐 (#GETTING_DATA·#SPILL!·#CALC! 은 입력 거부)
                        let up = s.to_ascii_uppercase();
                        if !["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A"]
                            .contains(&up.as_str())
                        {
                            invalid = true;
                        }
                        toks.push(T::Operand(up, false));
                    } else if c.is_whitespace() {
                        while i < ch.len() && ch[i].is_whitespace() {
                            i += 1;
                        }
                        toks.push(T::Space);
                    } else if is_word(c) {
                        let mut e = word_end(&ch, i);
                        // 지수 표기 `1E+3`
                        let w: String = ch[i..e].iter().collect();
                        if w.starts_with(|c: char| c.is_ascii_digit())
                            && (w.ends_with('E') || w.ends_with('e'))
                            && matches!(ch.get(e), Some('+') | Some('-'))
                            && ch.get(e + 1).is_some_and(|c| c.is_ascii_digit())
                        {
                            e = word_end(&ch, e + 1);
                        }
                        let w: String = ch[i..e].iter().collect();
                        // 함수 이름: 뒤의 공백을 건너 '('
                        let mut k = e;
                        while k < ch.len() && ch[k].is_whitespace() {
                            k += 1;
                        }
                        if ch.get(k) == Some(&'(') {
                            // `_xlfn.`·`_xlws.`·`_xll.` 접두사는 한셀이 입력에서 뗀다 (`_xlfn.STDEV.S(1)` → `STDEV.S(1)`)
                            let name = strip_fn_prefix(&w);
                            toks.push(T::Open(format!("{}(", name.to_uppercase())));
                            i = k + 1;
                            continue;
                        }
                        let tok = if w.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                            let exp = w
                                .find(['e', 'E'])
                                .and_then(|p| w[p + 1..].parse::<i32>().ok());
                            if exp.is_some_and(|e| e < -309)
                                || w.parse::<f64>().is_ok_and(|v| !v.is_finite())
                            {
                                invalid = true;
                            }
                            match crate::calc::value::parse_decimal15(&w).ok_or(()) {
                                // 한셀은 숫자 글자를 15자리로 읽어 일반 서식 글로 다시 쓴다 (`1E-10` → `0.0000000001`,
                                // `1E+15` → `1000000000000000`, `12345678901234567` → `12345678901234500`,
                                // `1E+20` 은 그대로 — 수식 글 대조 실험)
                                Ok(v) if v.is_finite() => {
                                    // 단항 `-` 바로 뒤의 숫자는 부호를 숫자에 접는다 (한셀: `-0` → `0`, `--0` → `-0`,
                                    // `-123456789012345678` → `-123456789012345000` — 큰 수의 글은 부호 있는 값으로
                                    // 내림해서 부호에 따라 다르다). `-(0)` 은 그대로
                                    let unary_minus = matches!(toks.last(), Some(T::Raw(m)) if m == "-")
                                        && match toks[..toks.len() - 1]
                                            .iter()
                                            .rev()
                                            .find(|x| **x != T::Space)
                                        {
                                            None | Some(T::Open(_)) => true,
                                            Some(T::Raw(p)) => p != "%",
                                            _ => false,
                                        };
                                    if unary_minus {
                                        toks.pop();
                                        // 앞의 `+` 는 접힌 숫자 앞이라 버리지 않는다 (`+-0` → `+0`)
                                        if matches!(toks.last(), Some(T::Raw(p)) if p == "+") {
                                            toks.push(T::Raw(String::new()));
                                        }
                                        T::Operand(crate::calc::value::number_to_text(-v), false)
                                    } else {
                                        T::Operand(crate::calc::value::number_to_text(v), false)
                                    }
                                }
                                _ => T::Operand(w.clone(), false),
                            }
                        } else if w.eq_ignore_ascii_case("TRUE") || w.eq_ignore_ascii_case("FALSE")
                        {
                            T::Operand(w.to_ascii_uppercase(), false)
                        } else {
                            T::Operand(canon(&w).unwrap_or(w.clone()), true)
                        };
                        toks.push(tok);
                        i = e;
                    } else if c == '(' {
                        toks.push(T::Open("(".into()));
                        i += 1;
                    } else if c == ')' {
                        toks.push(T::Operand(")".into(), true));
                        i += 1;
                    } else {
                        toks.push(T::Raw(c.to_string()));
                        i += 1;
                    }
                }
            }
        }
    }
    // 앞이 없거나 여는 괄호·연산자(`%` 제외)이고 뒤가 숫자인 `+`
    let unary_before_number = |before: &[T], after: &[T]| {
        let prev = before.iter().rev().find(|x| **x != T::Space);
        // 숫자가 바로 붙어야 한다 (`+ 0.5` 는 그대로, 한셀 대조 실험)
        let next = after.first();
        let unary = match prev {
            None | Some(T::Open(_)) => true,
            Some(T::Raw(p)) => p != "%",
            _ => false,
        };
        unary
            && matches!(next, Some(T::Operand(n, false))
                if n.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
    };
    // 피연산자가 연산자 없이 이어지면 문법 오류: 공백 사이(교차 연산자)는 둘 다 참조일 때만, 붙어 있으면 늘
    // (`1 2`, `-1 1/2`, `1:2.5` 의 `1:2` 와 `.5`). 범위 연산자 `:` 옆에 참조가 아닌 피연산자도 오류
    for (k, t) in toks.iter().enumerate() {
        let is_operand_start =
            |x: &T| matches!(x, T::Operand(s, _) if s != ")") || matches!(x, T::Open(_));
        if !is_operand_start(t) || k == 0 {
            continue;
        }
        let spaced = toks[k - 1] == T::Space;
        let prev = toks[..k].iter().rev().find(|x| **x != T::Space);
        if let Some(T::Operand(_, prev_ref)) = prev {
            let cur_ref = matches!(t, T::Operand(_, true) | T::Open(_));
            if !spaced || !(*prev_ref && cur_ref) {
                invalid = true;
            }
        }
    }
    for (k, t) in toks.iter().enumerate() {
        if *t == T::Raw(":".into()) {
            let prev = toks[..k].iter().rev().find(|x| **x != T::Space);
            let next = toks[k + 1..].iter().find(|x| **x != T::Space);
            if matches!(prev, Some(T::Operand(_, false)))
                || matches!(next, Some(T::Operand(_, false)))
            {
                invalid = true;
            }
        }
    }
    // 비교 연산자 철자: `!=` → `<>`, `==` → `=`, `=>` → `>=`, `=<` → `<=`, 그 밖의 `<>=!` 이어짐 (`><`, `<<`, `=!`,
    // 세 글자)은 오류 — 한셀 대조 실험
    {
        let is_op = |t: &T| matches!(t, T::Raw(r) if r.len() == 1 && "<>=!".contains(r.as_str()));
        let mut k = 0;
        let mut merged: Vec<T> = Vec::with_capacity(toks.len());
        while k < toks.len() {
            if is_op(&toks[k]) {
                let mut j = k;
                let mut run = String::new();
                while j < toks.len() && is_op(&toks[j]) {
                    if let T::Raw(r) = &toks[j] {
                        run.push_str(r);
                    }
                    j += 1;
                }
                let fixed = match run.as_str() {
                    "<" | ">" | "=" | "<>" | "<=" | ">=" | "!" => Some(run.clone()),
                    "!=" => Some("<>".to_string()),
                    "==" => Some("=".to_string()),
                    "=>" => Some(">=".to_string()),
                    "=<" => Some("<=".to_string()),
                    _ => None,
                };
                match fixed {
                    Some(t) => merged.push(T::Raw(t)),
                    None => {
                        invalid = true;
                        merged.push(T::Raw(run));
                    }
                }
                k = j;
            } else {
                merged.push(toks[k].clone());
                k += 1;
            }
        }
        toks = merged;
    }
    let mut out = String::with_capacity(input.len());
    let mut depth: i32 = 0;
    for (k, t) in toks.iter().enumerate() {
        match t {
            T::Space => {
                // 교차 연산자: 참조처럼 쓰이는 피연산자 사이의 공백
                let prev = toks[..k].iter().rev().find(|x| **x != T::Space);
                let next = toks[k + 1..].iter().find(|x| **x != T::Space);
                let prev_ref = matches!(prev, Some(T::Operand(_, true)));
                let next_ref = matches!(next, Some(T::Operand(s, true)) if s != ")")
                    || matches!(next, Some(T::Open(_)));
                if prev_ref && next_ref && !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            // 숫자 바로 앞의 단항 `+` 는 버린다 (한셀: `=+1+2` → `=1+2`, `=-+1` → `=-1`, `=++1` → `=+1`,
            // `=+A1`·`=+(1)`·`=+-1` 은 그대로 — CSV 열기 대조 실험)
            T::Raw(s) if s == "+" && unary_before_number(&toks[..k], &toks[k + 1..]) => {}
            T::Raw(s) => out.push_str(s),
            T::Operand(s, _) => {
                if s == ")" {
                    depth -= 1;
                }
                out.push_str(s);
            }
            T::Open(s) => {
                depth += 1;
                out.push_str(s);
            }
        }
    }
    for _ in 0..depth.max(0) {
        out.push(')');
    }
    Normalized {
        text: out,
        unclosed: depth > 0,
        invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 한셀에 입력해 돌려받은 글 (tools/probe/formula-probe.ps1, 수식 글 대조 실험)
    #[test]
    fn normalize_like_hcell_typed_input() {
        let none = |_: &str| None;
        let n = |s: &str| normalize_checked(s, &none);
        // 숫자 글자: 15자리, 일반 서식 글, 단항 -·+ 와 함께
        assert_eq!(n("1E-10+1E+15+1E+20").text, "0.0000000001+1000000000000000+1E+20");
        assert_eq!(n("123456789012345678").text, "123456789012344000");
        assert_eq!(n("-123456789012345678").text, "-123456789012345000");
        assert_eq!(n("--0").text, "-0");
        assert_eq!(n("1--0").text, "1-0");
        assert_eq!(n("+-0").text, "+0");
        assert_eq!(n("+ 0.5").text, "+0.5");
        assert_eq!(n("+1").text, "1");
        // 비교 연산자 철자
        assert_eq!(n("A1!=B1").text, "A1<>B1");
        assert_eq!(n("A1==B1").text, "A1=B1");
        assert_eq!(n("A1=>B1").text, "A1>=B1");
        assert_eq!(n("A1=<B1").text, "A1<=B1");
        for bad in ["A1><B1", "A1<<B1", "A1=!B1", "A1<>=B1", "A1===B1", "#CALC!", "#SPILL!"] {
            assert!(n(bad).invalid, "{bad}");
        }
        // 거꾸로 된 범위, 접두사
        assert_eq!(n("$B$2:$A$1").text, "$A$1:$B$2");
        assert_eq!(n("B$2:$A1").text, "$A1:B$2");
        assert_eq!(n("3:1").text, "1:3");
        assert_eq!(n("C:A").text, "A:C");
        assert_eq!(n("_xlfn.STDEV.S(1)").text, "STDEV.S(1)");
    }

    #[test]
    fn normalize_checks() {
        let none = |_: &str| None;
        assert_eq!(normalize("+1+2", &none), "1+2");
        assert_eq!(normalize("-+1", &none), "-1");
        assert_eq!(normalize("++1", &none), "+1");
        assert_eq!(normalize("+A1", &none), "+A1");
        assert_eq!(normalize("+(1)", &none), "+(1)");
        assert_eq!(normalize("+-1", &none), "+-1");
        assert_eq!(normalize("5%+1", &none), "5%+1");
        for bad in ["1 2", "-1 1/2", "-1:2.5", "-1E-400", "1 SUM(1)"] {
            assert!(normalize_checked(bad, &none).invalid, "{bad}");
        }
        for good in [
            "A1 B1",
            "SUM(A1:B2)",
            "(A1) (B1)",
            "1+ 2",
            "SUM( 1 )",
            "-A1:B2",
        ] {
            assert!(!normalize_checked(good, &none).invalid, "{good}");
        }
        assert!(normalize_checked("SUM(", &none).unclosed);
    }

    fn ins_rows(at: u32, n: u32) -> Shift {
        Shift::InsRows { at, n }
    }
    fn del_rows(at: u32, n: u32) -> Shift {
        Shift::DelRows { at, n }
    }

    #[test]
    fn segments_find_refs_only() {
        let segs =
            segments("SUM(Sheet1!A5:B6,'My Data'!$C$1)&\"A1\"+T[[#This Row],[A1]]+LOG10(A1)");
        let refs: Vec<String> = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Ref(r) => Some(r.raw.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(refs, ["Sheet1!A5:B6", "'My Data'!$C$1", "A1"]);
        let segs = segments("SUM(5:5)+SUM($A:$C)+[1]Data!B2+Sheet1:Sheet2!A5+#REF!+Sheet1!#REF!");
        let refs: Vec<(String, Option<Prefix>)> = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Ref(r) => Some((r.raw.clone(), r.prefix.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(refs.len(), 5);
        assert_eq!(refs[0].0, "5:5");
        assert_eq!(refs[1].0, "$A:$C");
        assert_eq!(refs[2].1.as_ref().unwrap().book.as_deref(), Some("1"));
        assert_eq!(refs[3].1.as_ref().unwrap().last.as_deref(), Some("Sheet2"));
        assert_eq!(refs[4].0, "Sheet1!#REF!");
        // 숫자·이름·논리값은 참조가 아니다
        assert!(
            segments("1E+10+TRUE+R2D2+이율+A1B")
                .iter()
                .all(|s| matches!(s, Seg::Text(_)))
        );
    }

    // 기대값은 한셀 COM 대조 실험 (Rows("5:6").Insert / Delete …) 의 결과다
    #[test]
    fn insert_rows_like_hcell() {
        let f = |s: &str| shift_formula(s, "Sheet1", "Sheet1", ins_rows(4, 2));
        assert_eq!(f("A5"), "A7");
        assert_eq!(f("$A$5"), "$A$7");
        assert_eq!(f("SUM(A1:A10)"), "SUM(A1:A12)");
        assert_eq!(f("SUM(A5:A10)"), "SUM(A7:A12)");
        assert_eq!(f("SUM(A1:A4)"), "SUM(A1:A4)");
        assert_eq!(f("SUM(A:A)"), "SUM(A:A)");
        assert_eq!(f("SUM(5:5)"), "SUM(7:7)");
        assert_eq!(f("SUM(4:6)"), "SUM(4:8)");
        assert_eq!(f("Sheet2!A5"), "Sheet2!A5");
        assert_eq!(f("SUM(Sheet1!A5:A6)"), "SUM(Sheet1!A7:A8)");
        assert_eq!(f("A$5+$A5"), "A$7+$A7");
        assert_eq!(f("INDIRECT(\"A5\")"), "INDIRECT(\"A5\")");
        assert_eq!(f("A1048576"), "A1048576");
        assert_eq!(f("SUM(A5:B6 B6:C7)"), "SUM(A7:B8 B8:C9)");
        assert_eq!(f("SUM(Sheet1:Sheet2!A5)"), "SUM(Sheet1:Sheet2!A5)");
        // 다른 시트의 수식
        assert_eq!(
            shift_formula("Sheet1!A5+A5", "Sheet2", "Sheet1", ins_rows(4, 2)),
            "Sheet1!A7+A5"
        );
    }

    #[test]
    fn delete_rows_like_hcell() {
        let f = |s: &str| shift_formula(s, "Sheet1", "Sheet1", del_rows(4, 2));
        assert_eq!(f("A5"), "#REF!");
        assert_eq!(f("SUM(A1:A10)"), "SUM(A1:A8)");
        assert_eq!(f("SUM(A5:A10)"), "SUM(A5:A8)");
        assert_eq!(f("SUM(5:5)"), "SUM(#REF!)");
        assert_eq!(f("SUM(4:6)"), "SUM(4:4)");
        assert_eq!(f("SUM(Sheet1!A5:A6)"), "SUM(Sheet1!#REF!)");
        assert_eq!(f("A$5+$A5"), "#REF!+#REF!");
        assert_eq!(f("A1048576"), "A1048574");
        assert_eq!(f("SUM(A5:B6 B6:C7)"), "SUM(#REF! B5:C5)");
        assert_eq!(f("SUM(A6:A7)"), "SUM(A5:A5)");
        assert_eq!(f("SUM(A5,A6,A7)"), "SUM(#REF!,#REF!,A5)");
        assert_eq!(f("SUM(A1:B2,C5:D6)"), "SUM(A1:B2,#REF!)");
        let g = |s: &str| shift_formula(s, "Sheet1", "Sheet1", del_rows(0, 3));
        assert_eq!(g("SUM(A1:A4)"), "SUM(A1:A1)");
        assert_eq!(g("OFFSET(A1,4,0)"), "OFFSET(#REF!,4,0)");
        assert_eq!(g("XFD1"), "#REF!");
    }

    #[test]
    fn columns_like_hcell() {
        let ins = |s: &str| shift_formula(s, "Sheet1", "Sheet1", Shift::InsCols { at: 2, n: 2 });
        assert_eq!(ins("SUM(C:D)"), "SUM(E:F)");
        assert_eq!(ins("SUM(B:C)"), "SUM(B:E)");
        assert_eq!(ins("SUM(A5:B6 B6:C7)"), "SUM(A5:B6 B6:E7)");
        assert_eq!(ins("XFD1"), "XFD1");
        assert_eq!(ins("SUM(5:5)"), "SUM(5:5)");
        let del = |s: &str| shift_formula(s, "Sheet1", "Sheet1", Shift::DelCols { at: 2, n: 2 });
        assert_eq!(del("SUM(C:D)"), "SUM(#REF!)");
        assert_eq!(del("SUM(B:C)"), "SUM(B:B)");
        assert_eq!(del("SUM(D:F)"), "SUM(C:D)");
        assert_eq!(del("XFD1"), "XFB1");
        assert_eq!(del("SUM(A1:B2,C5:D6)"), "SUM(A1:B2,#REF!)");
        assert_eq!(del("SUM(A5:B6 B6:C7)"), "SUM(A5:B6 B6:B7)");
    }

    #[test]
    fn anchored_formulas_follow_the_range() {
        // CF B1:B10 "C1>$A$5": 1–3행 삭제 → 범위 B1:B7 (옛 B4:B10)
        assert_eq!(
            shift_anchored(
                "C1>$A$5",
                "Sheet1",
                "Sheet1",
                del_rows(0, 3),
                (0, 1),
                (3, 1)
            ),
            "C1>$A$2"
        );
        // 1행 삽입 → B2:B11
        assert_eq!(
            shift_anchored(
                "C1>$A$5",
                "Sheet1",
                "Sheet1",
                ins_rows(0, 1),
                (0, 1),
                (0, 1)
            ),
            "C2>$A$6"
        );
    }

    #[test]
    fn sheet_rename_and_delete() {
        assert_eq!(
            rename_sheet(
                "SUM(Sheet1!A5:A6)+Sheet1:Sheet2!A5+Sheet2!A1",
                "Sheet1",
                "My Data"
            ),
            "SUM('My Data'!A5:A6)+'My Data:Sheet2'!A5+Sheet2!A1"
        );
        assert_eq!(rename_sheet("'Old'!A1", "old", "새 시트"), "'새 시트'!A1");
        let after = vec!["Sheet1".to_string(), "Data 3".to_string()];
        assert_eq!(
            delete_sheet("Sheet2!A5+SUM(Sheet1:Sheet2!A5)", "Sheet2", &after, 1),
            "#REF!+SUM('Sheet1:Data 3'!A5)"
        );
    }

    #[test]
    fn move_updates_refs_into_the_block() {
        let m = Move {
            src_sheet: "Sheet1",
            dst_sheet: "Sheet1",
            src: (4, 0, 5, 1),
            drow: 0,
            dcol: 16,
        };
        let f = |s: &str| move_formula(s, "Sheet1", &m);
        assert_eq!(f("A5"), "Q5");
        assert_eq!(f("$A$5"), "$Q$5");
        assert_eq!(f("SUM(A1:A10)"), "SUM(A1:A10)");
        assert_eq!(f("SUM(Sheet1!A5:A6)"), "SUM(Sheet1!Q5:Q6)");
        assert_eq!(f("A5*2+$B$6"), "Q5*2+$R$6");
        assert_eq!(f("SUM(A5:B6 B6:C7)"), "SUM(Q5:R6 B6:C7)");
        assert_eq!(f("SUM(Sheet1:Sheet2!A5)"), "SUM(Sheet1:Sheet2!A5)");
    }

    #[test]
    fn normalize_like_hcell() {
        let sheets = ["Sheet1", "My Data"];
        let canon = |s: &str| {
            sheets
                .iter()
                .find(|x| x.eq_ignore_ascii_case(s))
                .map(|x| x.to_string())
                .or_else(|| {
                    s.eq_ignore_ascii_case("myrange")
                        .then(|| "MyRange".to_string())
                })
        };
        let n = |s: &str| normalize(s, &canon);
        assert_eq!(n("sum(a1:a3)"), "SUM(A1:A3)");
        assert_eq!(n("a1 + b1"), "A1+B1");
        assert_eq!(n(" a1"), "A1");
        assert_eq!(n("Sum(A1:B2)*2"), "SUM(A1:B2)*2");
        assert_eq!(n("if(a1>0,true,false)"), "IF(A1>0,TRUE,FALSE)");
        assert_eq!(n("sheet1!a1"), "Sheet1!A1");
        assert_eq!(n("SUM(A1:A3"), "SUM(A1:A3)");
        assert_eq!(n("sum(A:a)"), "SUM(A:A)");
        assert_eq!(n("1E3"), "1000");
        assert_eq!(n("1.50"), "1.5");
        assert_eq!(n("\"x\"&a1"), "\"x\"&A1");
        assert_eq!(n("SUM(A1:B2 B1:C2)"), "SUM(A1:B2 B1:C2)");
        assert_eq!(n("sum(myrange)"), "SUM(MyRange)");
        assert_eq!(n("'my data'!b2"), "'My Data'!B2");
        assert_eq!(n("iferror(1/0,#n/a)"), "IFERROR(1/0,#N/A)");
    }
}
