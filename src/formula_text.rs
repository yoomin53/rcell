// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 원문(A1 표기) 다루기.
//!
//! XML `.cell` 의 공유 수식(`<f t="shared" si=".."/>`)은 기준 셀에만 원문이 있다.
//! 종속 셀의 수식은 기준 셀 원문의 상대 참조를 셀 위치 차이만큼 옮겨 얻는다
//! (ECMA-376 18.3.1.40). 한셀도 이렇게 저장한다 (관측).

/// 시트 크기 (한셀 2024)
const MAX_ROW: i64 = 1_048_576;
const MAX_COL: i64 = 16_384;

/// 수식 원문의 상대 참조(`$` 없는 행·열)를 `drow` 행, `dcol` 열만큼 옮긴다.
/// 문자열·따옴표 시트 이름·구조적 참조·함수 이름은 그대로 둔다.
/// 시트 밖으로 나간 참조는 `#REF!` 가 된다.
pub fn shift_references(formula: &str, drow: i64, dcol: i64) -> String {
    if drow == 0 && dcol == 0 {
        return formula.to_string();
    }
    let chars: Vec<char> = formula.chars().collect();
    let mut out = String::with_capacity(formula.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '"' | '\'' => {
                // 문자열 / 따옴표 시트 이름: 같은 따옴표 두 개는 이스케이프
                let q = ch;
                out.push(q);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == q {
                        if chars.get(i + 1) == Some(&q) {
                            out.push(q);
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '[' => {
                // 구조적 참조·외부 통합 문서 번호: 짝이 맞는 ']' 까지 그대로
                let mut depth = 0;
                while i < chars.len() {
                    out.push(chars[i]);
                    match chars[i] {
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
            '#' => {
                // 오류 값 (#REF!, #N/A, #DIV/0! …)
                out.push(ch);
                i += 1;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || matches!(chars[i], '/' | '!' | '?' | '_'))
                {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            c if is_word(c) => {
                let (w1, end1) = word(&chars, i);
                i = end1;
                let next = chars.get(i).copied();
                if next == Some('(') || next == Some('!') || next == Some('[') {
                    // 함수 이름, 시트 이름, 표 이름
                    out.push_str(&w1);
                    continue;
                }
                if next == Some(':') && chars.get(i + 1).is_some_and(|&c| is_word(c)) {
                    let (w2, end2) = word(&chars, i + 1);
                    if let Some(t) = shift_range(&w1, &w2, drow, dcol) {
                        out.push_str(&t);
                        i = end2;
                        continue;
                    }
                }
                match parse_cell(&w1) {
                    Some(p) => out.push_str(&p.shifted(drow, dcol)),
                    None => out.push_str(&w1),
                }
            }
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// 수식 원문의 외부 문서 번호(`[n]시트!`, `'[n]시트'!`)를 `map` 으로 바꾼다.
/// `map` 이 None 을 돌려주면 그 번호는 그대로 둔다. 문자열과 구조적 참조(`표[열]`)는 건드리지 않는다.
pub fn map_external_books(formula: &str, mut map: impl FnMut(usize) -> Option<usize>) -> String {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = String::with_capacity(formula.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '"' => {
                out.push(ch);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                    if chars[i - 1] == '"' {
                        if chars.get(i) == Some(&'"') {
                            out.push('"');
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
            }
            '[' => {
                let prev = i.checked_sub(1).map(|p| chars[p]);
                let digits: String = chars[i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                let close = i + 1 + digits.len();
                let is_book = !prev.is_some_and(|p| is_word(p) || p == ']')
                    && !digits.is_empty()
                    && chars.get(close) == Some(&']');
                if is_book {
                    let n: usize = digits.parse().unwrap_or(0);
                    let m = map(n).unwrap_or(n);
                    out.push_str(&format!("[{m}]"));
                    i = close + 1;
                } else {
                    // 구조적 참조: 짝이 맞는 ']' 까지 그대로
                    let mut depth = 0;
                    while i < chars.len() {
                        out.push(chars[i]);
                        match chars[i] {
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
            _ => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\')
}

fn word(chars: &[char], start: usize) -> (String, usize) {
    let mut end = start;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (chars[start..end].iter().collect(), end)
}

/// 참조의 한 축 (열 또는 행), 1부터 시작.
#[derive(Clone, Copy)]
struct Axis {
    abs: bool,
    n: i64,
}

impl Axis {
    fn shift(self, d: i64, max: i64) -> Option<i64> {
        let n = if self.abs { self.n } else { self.n + d };
        (1..=max).contains(&n).then_some(n)
    }
}

struct CellPart {
    col: Axis,
    row: Axis,
}

impl CellPart {
    fn shifted(&self, drow: i64, dcol: i64) -> String {
        match (self.col.shift(dcol, MAX_COL), self.row.shift(drow, MAX_ROW)) {
            (Some(c), Some(r)) => format!(
                "{}{}{}{}",
                if self.col.abs { "$" } else { "" },
                col_name(c),
                if self.row.abs { "$" } else { "" },
                r
            ),
            _ => "#REF!".into(),
        }
    }
}

/// `$A$1`, `B12` 같은 셀 참조
fn parse_cell(w: &str) -> Option<CellPart> {
    let (col, rest) = parse_col(w)?;
    let row = parse_row(rest)?;
    Some(CellPart { col, row })
}

/// 앞부분의 열 (`$` + 글자 1–3개). 나머지 문자열을 함께 돌려준다.
fn parse_col(w: &str) -> Option<(Axis, &str)> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    let n_letters = s.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if !(1..=3).contains(&n_letters) {
        return None;
    }
    let mut n: i64 = 0;
    for c in s[..n_letters].chars() {
        n = n * 26 + (c.to_ascii_uppercase() as i64 - 'A' as i64 + 1);
    }
    (n <= MAX_COL).then_some((Axis { abs, n }, &s[n_letters..]))
}

/// 행 (`$` + 숫자). 문자열 전체가 행이어야 한다.
fn parse_row(w: &str) -> Option<Axis> {
    let (abs, s) = match w.strip_prefix('$') {
        Some(s) => (true, s),
        None => (false, w),
    };
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) || s.len() > 7 {
        return None;
    }
    let n: i64 = s.parse().ok()?;
    (1..=MAX_ROW).contains(&n).then_some(Axis { abs, n })
}

/// `A1:B2`, `A:C`, `1:3` 범위. 범위 모양이 아니면 None.
fn shift_range(w1: &str, w2: &str, drow: i64, dcol: i64) -> Option<String> {
    if let (Some(a), Some(b)) = (parse_cell(w1), parse_cell(w2)) {
        let (x, y) = (a.shifted(drow, dcol), b.shifted(drow, dcol));
        if x == "#REF!" || y == "#REF!" {
            return Some("#REF!".into());
        }
        return Some(format!("{x}:{y}"));
    }
    let whole_col = |w: &str| {
        parse_col(w)
            .filter(|(_, rest)| rest.is_empty())
            .map(|(a, _)| a)
    };
    if let (Some(a), Some(b)) = (whole_col(w1), whole_col(w2)) {
        return Some(match (a.shift(dcol, MAX_COL), b.shift(dcol, MAX_COL)) {
            (Some(x), Some(y)) => format!(
                "{}{}:{}{}",
                if a.abs { "$" } else { "" },
                col_name(x),
                if b.abs { "$" } else { "" },
                col_name(y)
            ),
            _ => "#REF!".into(),
        });
    }
    if let (Some(a), Some(b)) = (parse_row(w1), parse_row(w2)) {
        return Some(match (a.shift(drow, MAX_ROW), b.shift(drow, MAX_ROW)) {
            (Some(x), Some(y)) => format!(
                "{}{x}:{}{y}",
                if a.abs { "$" } else { "" },
                if b.abs { "$" } else { "" }
            ),
            _ => "#REF!".into(),
        });
    }
    None
}

/// 1부터 시작하는 열 번호 → 열 이름 (1 → A, 27 → AA)
fn col_name(mut n: i64) -> String {
    let mut s = Vec::new();
    while n > 0 {
        let m = (n - 1) % 26;
        s.push((b'A' + m as u8) as char);
        n = (n - 1) / 26;
    }
    s.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::shift_references as sh;

    #[test]
    fn relative_and_absolute() {
        assert_eq!(sh("IF(H28,H28*B28,\"\")", 1, 0), "IF(H29,H29*B29,\"\")");
        assert_eq!(sh("$A$1+A$1+$A1+A1", 2, 3), "$A$1+D$1+$A3+D3");
        assert_eq!(sh("SUM(B2:B10)", 0, 1), "SUM(C2:C10)");
        assert_eq!(sh("SUM(A:A)+SUM($1:2)", 1, 1), "SUM(B:B)+SUM($1:3)");
    }

    #[test]
    fn leaves_non_references_alone() {
        assert_eq!(sh("LOG10(A1)&\"A1\"", 1, 0), "LOG10(A2)&\"A1\"");
        assert_eq!(sh("'Q1 A1'!B2+Sheet2!C3", 1, 0), "'Q1 A1'!B3+Sheet2!C4");
        assert_eq!(sh("1E+10+A1*1.5", 1, 0), "1E+10+A2*1.5");
        assert_eq!(
            sh("Table1[[#This Row],[A1]]+이율", 1, 0),
            "Table1[[#This Row],[A1]]+이율"
        );
        assert_eq!(sh("IFERROR(A1,#N/A)", 1, 0), "IFERROR(A2,#N/A)");
        assert_eq!(sh("TRUE+R2D2", 1, 1), "TRUE+R2D2");
    }

    #[test]
    fn external_book_numbers() {
        use super::map_external_books as mx;
        let f = "'[3]Q 1'!A1+[1]Intro!B2+T[[#This Row],[1]]&\"[3]x\"";
        assert_eq!(
            mx(f, |n| (n == 3).then_some(2)),
            "'[2]Q 1'!A1+[1]Intro!B2+T[[#This Row],[1]]&\"[3]x\""
        );
        let mut seen = Vec::new();
        mx(f, |n| {
            seen.push(n);
            None
        });
        assert_eq!(seen, [3, 1]);
    }

    #[test]
    fn off_sheet_is_ref_error() {
        assert_eq!(sh("A1048576", 1, 0), "#REF!");
        assert_eq!(sh("XFD1", 0, 1), "#REF!");
    }
}
