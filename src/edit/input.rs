// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 셀에 입력한 글 → 값과 표시 형식 (한셀 입력 규칙).
//!
//! 한셀 COM 대조 실험(`Range.Value2 = 글`, 표시 형식이 다른 칸 14종 × 입력 94종)에서 확인한 규칙이다
//! (docs/editing.md 2절):
//! - `=` 로 시작하면 수식 (`=` 하나만이면 글), 작은따옴표로 시작하면 글 + 글자 접두(quotePrefix),
//!   `+A1`·`-A1`·`@SUM(A1)` 은 글.
//! - `TRUE`/`FALSE` (대소문자 무시, 공백 없이) → 논리값, `#N/A` 같은 오류 글 → 오류 값.
//! - 숫자 꼴(앞뒤 공백 무시, `1,234`, `(5)`, `$1,000`, `10%`, `1.5E3`), 시각(`12:30`, `9:05 AM`, `25:00`),
//!   날짜(`2024-01-15`, `1/15`, `15-Jan`, `2024년 1월 15일`), 날짜+시각, 대분수(`1 1/4`).
//! - 칸이 일반 서식이면 입력 꼴의 표시 형식을 붙인다 (`1,234` → `#,##0`, `10%` → `0%`, 날짜 → 14 …).
//!   다른 서식이면 서식 종류마다 다르다: 백분율 칸은 숫자를 100 으로 나누고, 날짜 칸에 `1,234`·`10%`·`1.5E3`
//!   을 넣으면 그 꼴의 서식으로 바뀌고, 숫자·지수·시간 칸은 서식을 그대로 둔다. 글자(@) 칸은 모두 글.
//! - `1/2` 는 일반·날짜·시간 칸에서는 날짜(올해 1월 2일), 숫자·지수·분수·백분율 칸에서는 분수.
//! - 대분수 `0 1/2` 는 일반 칸에서 글이 되고 서식만 `# ?/?` 로 바뀐다 (한셀의 동작 그대로).

use crate::calc::date;
use crate::calc::value::{parse_fraction, parse_plain_number};
use crate::model::CellValue;

/// 칸의 표시 형식 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cat {
    General,
    Number,
    /// 통화 기호가 있는 숫자 형식 (수식 결과 형식을 정할 때만 — [`category`] 는 숫자로 돌려준다)
    Currency,
    Percent,
    Scientific,
    Date,
    Time,
    Fraction,
    Text,
}

/// 표시 형식 코드의 종류 (첫 구역 기준)
pub fn category(code: &str) -> Cat {
    let c = code.trim();
    if c.is_empty() || c.eq_ignore_ascii_case("general") || c == "G/표준" {
        return Cat::General;
    }
    // 첫 구역에서 따옴표 글·\ 다음 글자·[색/조건] 을 뺀다 ([h] [m] [s] 는 경과 시간)
    let mut plain = String::new();
    let mut elapsed = false;
    let ch: Vec<char> = c.chars().collect();
    let mut i = 0;
    while i < ch.len() {
        match ch[i] {
            ';' => break,
            '"' => {
                i += 1;
                while i < ch.len() && ch[i] != '"' {
                    i += 1;
                }
            }
            '\\' | '_' | '*' => i += 1,
            '[' => {
                let start = i + 1;
                while i < ch.len() && ch[i] != ']' {
                    i += 1;
                }
                let inner: String = ch[start..i.min(ch.len())].iter().collect();
                let l = inner.to_ascii_lowercase();
                if !l.is_empty() && l.chars().all(|x| matches!(x, 'h' | 'm' | 's')) {
                    elapsed = true;
                }
            }
            x => plain.push(x.to_ascii_lowercase()),
        }
        i += 1;
    }
    if plain.trim() == "@" {
        return Cat::Text;
    }
    let has = |x: char| plain.contains(x);
    if plain.contains("e+") || plain.contains("e-") {
        return Cat::Scientific;
    }
    let ampm = plain.contains("am/pm") || plain.contains("a/p");
    let dated = has('y') || has('d') || has('e') || has('g');
    let timed = elapsed || has('h') || has('s') || ampm;
    if dated || (has('m') && !timed) {
        return Cat::Date;
    }
    if timed {
        return Cat::Time;
    }
    if has('%') {
        return Cat::Percent;
    }
    if has('/') {
        return Cat::Fraction;
    }
    Cat::Number
}

/// 입력을 읽은 결과
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// 빈 입력 — 내용을 지운다
    Empty,
    /// 수식 (`=` 뒤의 원문, 고르기 전)
    Formula(String),
    Value {
        value: CellValue,
        /// 바꿀 표시 형식 번호 (None = 그대로)
        num_fmt: Option<u32>,
        /// 글자 접두 (작은따옴표로 입력한 글)
        quote_prefix: bool,
    },
}

fn value(v: CellValue, num_fmt: Option<u32>) -> Input {
    Input::Value {
        value: v,
        num_fmt,
        quote_prefix: false,
    }
}

fn num(v: f64) -> CellValue {
    // -0 은 0
    CellValue::Number(if v == 0.0 { 0.0 } else { v })
}

/// 숫자 꼴 입력의 특징 (표시 형식을 정하는 데 쓴다)
struct NumLook {
    percent: bool,
    exp: bool,
    grouped: bool,
    decimals: bool,
    /// 통화 기호: Some(true) 달러, Some(false) 원(`\` 또는 ₩)
    currency: Option<bool>,
    /// 통화 기호가 숫자 앞에 있다
    currency_lead: bool,
}

fn look(t: &str) -> NumLook {
    let t = t.trim();
    let mantissa = t.split(['e', 'E']).next().unwrap_or("");
    let currency = if t.contains('$') {
        Some(true)
    } else if t.contains(['₩', '\\']) {
        Some(false)
    } else {
        None
    };
    let body = t.trim_start_matches(['-', '+', '(']);
    NumLook {
        percent: t.trim_end_matches(')').ends_with('%'),
        exp: t.bytes().any(|b| b == b'e' || b == b'E'),
        grouped: t.contains(','),
        decimals: mantissa.contains('.')
            && mantissa
                .split('.')
                .nth(1)
                .is_some_and(|d| d.bytes().any(|b| b.is_ascii_digit())),
        currency,
        currency_lead: body.starts_with(['$', '₩', '\\']),
    }
}

/// 숫자 꼴 입력에 붙는 표시 형식 (일반 칸 — 입력·CSV 대조 실험): 백분율 9/10, 지수 11, 통화 기호가 앞에
/// 있고 자릿점이 있으면 3/4, 그 밖의 통화(자릿점 없이, 또는 기호가 뒤)는 달러 24/26·원 6/8 — 소수점이 있으면
/// (숫자가 뒤에 없어도, `$5.`) 소수 형식. 통화 없이 자릿점만 있으면 3/4
fn implied_format(text: &str, l: &NumLook) -> Option<u32> {
    let dot = text.split(['e', 'E']).next().unwrap_or("").contains('.');
    if l.percent {
        Some(if l.decimals { 10 } else { 9 })
    } else if l.exp {
        Some(11)
    } else if let Some(dollar) = l.currency
        && !(l.currency_lead && l.grouped)
    {
        Some(match (dollar, dot) {
            (true, false) => 24,
            (true, true) => 26,
            (false, false) => 6,
            (false, true) => 8,
        })
    } else if l.grouped || l.currency.is_some() {
        Some(if l.decimals { 4 } else { 3 })
    } else {
        None
    }
}

/// 지수가 -309 보다 작게 쓰인 수 (한셀: `1E-310`·`100E-310`·`4.9E-324`·`1E-400` 은 글). 지수가 그 안이고
/// f64 로 비정규 수가 되면 0 (`1E-309`·`1.5E-308` → 0, `9.9E-308` 은 그대로)
fn exponent_too_small(text: &str) -> bool {
    let t = text.trim();
    let Some(i) = t.find(['e', 'E']) else {
        return false;
    };
    let e = t[i + 1..].trim_end_matches(['%', ')']);
    e.parse::<i32>().is_ok_and(|e| e < -309)
}

/// `분:초.소수` (분 두 자리까지, 초 61 미만, 소수 한 자리 이상) → 일련 시각 (한셀은 `mm:ss.0` 47 을 붙이고
/// 초의 소수는 셋째 자리까지만 — `1:2.123456` = 62.123 초)
fn minute_seconds(text: &str) -> Option<f64> {
    let (m, s) = text.trim().split_once(':')?;
    let (si, sf) = s.split_once('.')?;
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    if !digits(m) || !digits(si) || !digits(sf) || m.len() > 2 || si.len() > 2 {
        return None;
    }
    let ms: f64 = format!("{:0<3}", &sf[..sf.len().min(3)]).parse().ok()?;
    let sec: f64 = si.parse::<f64>().ok()? + ms / 1000.0;
    let min: f64 = m.parse().ok()?;
    (sec < 61.0).then_some((min * 60.0 + sec) / 86_400.0)
}

/// `a/b` (부호·공백 없이) → 값, 분모 자릿수가 한 자리인가
fn simple_fraction(t: &str) -> Option<(f64, bool)> {
    let (a, b) = t.split_once('/')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if !digits(a) || !digits(b) {
        return None;
    }
    let (x, y): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (y != 0.0).then_some((x / y, y < 10.0))
}

/// 날짜 글의 표시 형식 번호 (한셀이 붙이는 것)
fn date_fmt(t: &str) -> u32 {
    if t.contains('년') {
        return 31;
    }
    if t.bytes().any(|b| b.is_ascii_alphabetic()) {
        // D-Mon, Mon-D, Mon D, D Mon → 16. 연도가 있으면 15 (d-mmm-yy), Mon-YYYY 는 17 (mmm-yy)
        let parts: Vec<&str> = t.split(['-', ' ', ',']).filter(|p| !p.is_empty()).collect();
        if parts.len() >= 3 {
            return 15;
        }
        if parts
            .iter()
            .any(|p| p.len() >= 3 && p.bytes().all(|b| b.is_ascii_digit()))
        {
            return 17;
        }
        return 16;
    }
    let sep = if t.contains('/') { '/' } else { '-' };
    let parts: Vec<&str> = t.split(sep).collect();
    match parts.len() {
        3 => 14,
        _ if parts[0].trim().len() == 4 => 17,
        _ => 58,
    }
}

/// 시각 글의 표시 형식 번호: 18 `h:mm AM/PM`, 19 `h:mm:ss AM/PM`, 20 `h:mm`, 21 `h:mm:ss`, 46 `[h]:mm:ss`
fn time_fmt(t: &str, v: f64) -> u32 {
    let upper = t.to_uppercase();
    let ampm = upper.ends_with("AM")
        || upper.ends_with("PM")
        || t.ends_with("오전")
        || t.ends_with("오후");
    let secs = t.matches(':').count() >= 2;
    if !ampm && v >= 1.0 {
        return 46;
    }
    match (ampm, secs) {
        (true, false) => 18,
        (true, true) => 19,
        (false, false) => 20,
        (false, true) => 21,
    }
}

/// 입력 글을 읽는다. `code` 는 칸의 지금 표시 형식
pub fn parse(text: &str, code: &str, date1904: bool) -> Input {
    if text.is_empty() {
        return Input::Empty;
    }
    if let Some(f) = text.strip_prefix('=')
        && !f.is_empty()
    {
        return Input::Formula(f.to_string());
    }
    if let Some(t) = text.strip_prefix('\'') {
        return Input::Value {
            value: CellValue::Text(t.to_string()),
            num_fmt: None,
            quote_prefix: true,
        };
    }
    let cat = category(code);
    let text_value = || value(CellValue::Text(text.to_string()), None);
    if cat == Cat::Text {
        return text_value();
    }
    if text.eq_ignore_ascii_case("TRUE") || text.eq_ignore_ascii_case("FALSE") {
        return value(CellValue::Bool(text.eq_ignore_ascii_case("TRUE")), None);
    }
    const ERRORS: [&str; 7] = [
        "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A",
    ];
    if let Some(e) = ERRORS.iter().find(|e| e.eq_ignore_ascii_case(text)) {
        return value(CellValue::Error(e.to_string()), None);
    }
    // 숫자 꼴 (전각 원 기호 ￦ 는 통화가 아니라 글)
    if text.contains('￦') {
        return text_value();
    }
    // 부호 뒤의 원 기호(`-\123`)는 수가 아니다 (`-$123` 은 수 — CSV 열기 대조 실험)
    let t = text.trim();
    let won_after_sign = t
        .strip_prefix(['-', '+'])
        .is_some_and(|r| r.starts_with(['\\', '₩']));
    // 통화 기호 뒤의 괄호 음수 `$(123)` 는 `($123)` 처럼
    let swapped = t
        .strip_prefix(['$', '\\', '₩'])
        .filter(|r| r.len() > 2 && r.starts_with('(') && r.ends_with(')'))
        .map(|r| format!("({}{})", &t[..t.len() - r.len()], &r[1..r.len() - 1]));
    let num_text = swapped.as_deref().unwrap_or(text);
    if let Some(v) = parse_plain_number(num_text).filter(|_| !won_after_sign) {
        if exponent_too_small(num_text) {
            return text_value();
        }
        let v = if v != 0.0 && v.abs() < f64::MIN_POSITIVE {
            0.0
        } else {
            v
        };
        let l = look(num_text);
        let implied = implied_format(num_text, &l);
        return match cat {
            Cat::General => value(num(v), implied),
            Cat::Number | Cat::Currency | Cat::Scientific | Cat::Time => value(num(v), None),
            Cat::Date => value(num(v), implied),
            Cat::Fraction => value(num(v), implied.filter(|&f| f == 9 || f == 10 || f == 11)),
            Cat::Percent => {
                let v = if l.percent { v } else { v / 100.0 };
                value(num(v), implied.filter(|&f| f == 11))
            }
            Cat::Text => unreachable!(),
        };
    }
    // 대분수
    if let Some(v) = parse_fraction(text) {
        let one_digit = text.rsplit('/').next().is_some_and(|d| d.trim().len() == 1);
        let frac = if one_digit { 12 } else { 13 };
        return match cat {
            Cat::General => value(CellValue::Text(text.to_string()), Some(frac)),
            Cat::Date | Cat::Time => text_value(),
            Cat::Percent => value(num(v), Some(frac)),
            _ => value(num(v), None),
        };
    }
    // 분수 a/b: 숫자 계열 칸에서는 분수, 일반·날짜·시간 칸에서는 날짜
    if let Some((v, one_digit)) = simple_fraction(text) {
        match cat {
            Cat::Number | Cat::Scientific | Cat::Fraction => return value(num(v), None),
            Cat::Percent => return value(num(v), Some(if one_digit { 12 } else { 13 })),
            _ => {}
        }
    }
    let keep_or = |implied: u32| match cat {
        Cat::General | Cat::Fraction | Cat::Percent => Some(implied),
        _ => None,
    };
    if let Some(t) = minute_seconds(text) {
        return value(num(t), keep_or(47));
    }
    if let Some(t) = date::parse_time(text, true) {
        return value(num(t), keep_or(time_fmt(text, t)));
    }
    if let Some(d) = date::parse_date(text, date1904) {
        return value(num(d), keep_or(date_fmt(text)));
    }
    if let Some(date::Parsed::DateTime(d, t)) = date::classify_text(text, date1904) {
        return value(num(d + t), keep_or(22));
    }
    text_value()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str, code: &str) -> (CellValue, Option<u32>) {
        match parse(t, code, false) {
            Input::Value { value, num_fmt, .. } => (value, num_fmt),
            other => panic!("{t}: {other:?}"),
        }
    }

    fn n(v: f64) -> CellValue {
        CellValue::Number(v)
    }

    fn s(v: &str) -> CellValue {
        CellValue::Text(v.into())
    }

    // 기대값은 한셀 COM 대조 실험 (일반 서식 칸에 Value2 = 글) 의 결과다
    #[test]
    fn general_cells() {
        crate::calc::date::set_this_year(Some(2026));
        let g = |t| p(t, "General");
        assert_eq!(g("123"), (n(123.0), None));
        assert_eq!(g("1,234"), (n(1234.0), Some(3)));
        assert_eq!(g("1,234.5"), (n(1234.5), Some(4)));
        assert_eq!(g("(5)"), (n(-5.0), None));
        assert_eq!(g("+5"), (n(5.0), None));
        assert_eq!(g("10%"), (n(0.1), Some(9)));
        assert_eq!(g("12.5%"), (n(0.125), Some(10)));
        assert_eq!(g("1.5E3"), (n(1500.0), Some(11)));
        assert_eq!(g("2024-01-15"), (n(45306.0), Some(14)));
        assert_eq!(g("2024/1/15"), (n(45306.0), Some(14)));
        assert_eq!(g("24-1-15"), (n(45306.0), Some(14)));
        assert_eq!(g("1/15"), (n(46037.0), Some(58)));
        assert_eq!(g("1-2"), (n(46024.0), Some(58)));
        assert_eq!(g("15-Jan"), (n(46037.0), Some(16)));
        assert_eq!(g("2024년 1월 15일"), (n(45306.0), Some(31)));
        assert_eq!(g("12:30").1, Some(20));
        assert_eq!(g("12:30:45").1, Some(21));
        assert_eq!(g("9:05 AM").1, Some(18));
        assert_eq!(g("10:00:00 PM").1, Some(19));
        assert_eq!(g("25:00").1, Some(46));
        assert_eq!(g("2024-01-15 12:30").1, Some(22));
        assert_eq!(g("$1,000"), (n(1000.0), Some(3)));
        assert_eq!(g("₩-1,000"), (n(-1000.0), Some(3)));
        assert_eq!(g("TRUE"), (CellValue::Bool(true), None));
        assert_eq!(g("false"), (CellValue::Bool(false), None));
        assert_eq!(g("TRUE "), (s("TRUE "), None));
        assert_eq!(g("#N/A"), (CellValue::Error("#N/A".into()), None));
        assert_eq!(g("#GETTING_DATA"), (s("#GETTING_DATA"), None));
        assert_eq!(g(" 123 "), (n(123.0), None));
        assert_eq!(g("00123"), (n(123.0), None));
        assert_eq!(g("0 1/2"), (s("0 1/2"), Some(12)));
        assert_eq!(g("+A1"), (s("+A1"), None));
        assert_eq!(g("1.2.3"), (s("1.2.3"), None));
        assert_eq!(g("12/31/2024"), (s("12/31/2024"), None));
        assert_eq!(g("2024.1.15"), (s("2024.1.15"), None));
        assert_eq!(g("1,23"), (s("1,23"), None));
        assert_eq!(g("50,000원"), (s("50,000원"), None));
        assert_eq!(g("-0"), (n(0.0), None));
        assert_eq!(g("="), (s("="), None));
        // 입력·CSV 대조 실험 (formats v2·v3·v4): 자릿점 없는 통화는 통화 형식, 전각 ￦ 는 글, 분:초.소수,
        // 지수가 -308 보다 작으면 글
        assert_eq!(g("$123"), (n(123.0), Some(24)));
        assert_eq!(g("$1234.5"), (n(1234.5), Some(26)));
        assert_eq!(g("123$"), (n(123.0), Some(24)));
        assert_eq!(g("1,234$"), (n(1234.0), Some(24)));
        assert_eq!(g("\\123"), (n(123.0), Some(6)));
        assert_eq!(g("₩123"), (n(123.0), Some(6)));
        assert_eq!(g("\\0.5"), (n(0.5), Some(8)));
        assert_eq!(g("$1,234.56"), (n(1234.56), Some(4)));
        assert_eq!(g("$1e3"), (n(1000.0), Some(11)));
        assert_eq!(g("￦123"), (s("￦123"), None));
        assert_eq!(g("12:34.5").1, Some(47));
        assert_eq!(g("100:00.0"), (s("100:00.0"), None));
        assert_eq!(g("1E-400"), (s("1E-400"), None));
        assert_eq!(g("4.9E-324"), (s("4.9E-324"), None));
        assert_eq!(g("1.5E-308"), (n(0.0), Some(11)));
        assert_eq!(g("9.9E-308"), (n(9.9e-308), Some(11)));
        assert_eq!(
            parse("=sum(a1)", "General", false),
            Input::Formula("sum(a1)".into())
        );
        assert_eq!(
            parse("'123", "General", false),
            Input::Value {
                value: s("123"),
                num_fmt: None,
                quote_prefix: true
            }
        );
    }

    #[test]
    fn formatted_cells() {
        crate::calc::date::set_this_year(Some(2026));
        // 백분율 칸: 숫자는 100 으로 나누고, 분수는 # ?/?, 날짜·시각은 그 꼴의 서식
        assert_eq!(p("123", "0%"), (n(1.23), None));
        assert_eq!(p("1,234", "0%"), (n(12.34), None));
        assert_eq!(p("10%", "0%"), (n(0.1), None));
        assert_eq!(p("1.5E3", "0%"), (n(15.0), Some(11)));
        assert_eq!(p("1/2", "0%"), (n(0.5), Some(12)));
        assert_eq!(p("12:30", "0%").1, Some(20));
        assert_eq!(p("2024-01-15", "0%").1, Some(14));
        // 날짜 칸: 1,234·10%·1.5E3 은 그 꼴로 바뀌고 날짜·시각·숫자는 그대로
        assert_eq!(p("1,234", "yyyy-mm-dd"), (n(1234.0), Some(3)));
        assert_eq!(p("10%", "yyyy-mm-dd"), (n(0.1), Some(9)));
        assert_eq!(p("123", "yyyy-mm-dd"), (n(123.0), None));
        assert_eq!(p("1/2", "yyyy-mm-dd"), (n(46024.0), None));
        assert_eq!(p("12:30", "yyyy-mm-dd").1, None);
        assert_eq!(p("0 1/2", "yyyy-mm-dd"), (s("0 1/2"), None));
        // 숫자 칸: 모두 그대로, 1/2 는 분수
        assert_eq!(p("1/2", "0.00"), (n(0.5), None));
        assert_eq!(p("10%", "#,##0"), (n(0.1), None));
        assert_eq!(p("2024-01-15", "0.00"), (n(45306.0), None));
        // 시간 칸
        assert_eq!(p("1,234", "h:mm"), (n(1234.0), None));
        assert_eq!(p("0 1/2", "h:mm"), (s("0 1/2"), None));
        // 분수 칸
        assert_eq!(p("10%", "# ?/?"), (n(0.1), Some(9)));
        assert_eq!(p("12:30", "# ?/?").1, Some(20));
        assert_eq!(p("1,234", "# ?/?"), (n(1234.0), None));
        // 글자 칸
        assert_eq!(p("123", "@"), (s("123"), None));
        assert_eq!(p("TRUE", "@"), (s("TRUE"), None));
        assert_eq!(parse("=1+1", "@", false), Input::Formula("1+1".into()));
    }

    #[test]
    fn categories() {
        assert_eq!(category("General"), Cat::General);
        assert_eq!(category("yyyy\"년\" mm\"월\" dd\"일\""), Cat::Date);
        assert_eq!(category("mm-dd"), Cat::Date);
        assert_eq!(category("[h]:mm:ss"), Cat::Time);
        assert_eq!(category("h:mm AM/PM"), Cat::Time);
        assert_eq!(category("0.00%"), Cat::Percent);
        assert_eq!(category("0.00E+00"), Cat::Scientific);
        assert_eq!(category("# ??/??"), Cat::Fraction);
        assert_eq!(category("₩#,##0;[Red]-₩#,##0"), Cat::Number);
        assert_eq!(category("@"), Cat::Text);
        assert_eq!(category("[Red]0.00"), Cat::Number);
    }
}
