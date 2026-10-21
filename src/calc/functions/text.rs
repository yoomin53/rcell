// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 글 함수.
//!
//! 글자 위치는 유니코드 글자 단위로 센다. 바이트 함수(LENB …)는 코드 페이지 949 로 센다
//! ([`crate::calc::cp949`]): 한글·한자·전각은 2바이트, 949 에 없는 글자와 U+0080–U+00FF 는 1바이트,
//! 바이트 경계에 걸린 글자 반쪽은 공백 하나.

use crate::calc::fm::Fm;
use super::math::{Rounding, round_digits};
use super::{Arg, FnSpec, MANY, R, V, bool_or, f, flatten, num_arg, num_or, opt, text_arg};
use crate::calc::criteria::wildcard_match;
use crate::calc::eval::Ctx;
use crate::calc::value::{self, ErrorKind, Value};
use crate::model::CellValue;

const VR: &[Arg] = &[Arg::R, Arg::V, Arg::R];

pub const FNS: &[FnSpec] = &[
    f("LEN", 1, 1, V, |c, a| Value::Number(try_v!(text_arg(c, a, 0)).chars().count() as f64)),
    f("LENB", 1, 1, V, |c, a| Value::Number(try_v!(text_arg(c, a, 0)).chars().map(byte_len).sum::<usize>() as f64)),
    f("LEFT", 1, 2, V, |c, a| left_right(c, a, true, false)),
    f("RIGHT", 1, 2, V, |c, a| left_right(c, a, false, false)),
    f("LEFTB", 1, 2, V, |c, a| left_right(c, a, true, true)),
    f("RIGHTB", 1, 2, V, |c, a| left_right(c, a, false, true)),
    f("MID", 3, 3, V, |c, a| mid(c, a, false)),
    f("MIDB", 3, 3, V, |c, a| mid(c, a, true)),
    // 글자마다 한 글자로 바뀔 때만 (UPPER("ß") = "ß", 대조 실험 text)
    f("LOWER", 1, 1, V, |c, a| Value::from(map_case(&try_v!(text_arg(c, a, 0)), false))),
    f("UPPER", 1, 1, V, |c, a| Value::from(map_case(&try_v!(text_arg(c, a, 0)), true))),
    f("PROPER", 1, 1, V, proper),
    f("TRIM", 1, 1, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        Value::from(t.split(' ').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "))
    }),
    f("CLEAN", 1, 1, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        Value::from(t.chars().filter(|&ch| ch as u32 >= 32).collect::<String>())
    }),
    f("EXACT", 2, 2, V, |c, a| {
        let x = try_v!(text_arg(c, a, 0));
        let y = try_v!(text_arg(c, a, 1));
        Value::Bool(x == y)
    }),
    f("REPT", 2, 2, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        let n = try_v!(num_arg(c, a, 1)).trunc();
        if n < 0.0 || t.chars().count() as f64 * n > 32_767.0 {
            return Value::Error(ErrorKind::Value);
        }
        Value::from(t.repeat(n as usize))
    }),
    f("CONCATENATE", 1, MANY, V, |c, a| {
        let mut s = String::new();
        for v in a {
            s.push_str(&try_v!(c.text(v)));
        }
        Value::from(s)
    }),
    f("CONCAT", 1, MANY, R, |c, a| {
        let mut s = String::new();
        for v in a {
            for x in flatten(c, v) {
                s.push_str(&try_v!(c.text(&x)));
            }
        }
        if s.chars().count() > 32_767 {
            return Value::Error(ErrorKind::Value);
        }
        Value::from(s)
    }),
    f("TEXTJOIN", 3, MANY, VR, textjoin),
    f("SUBSTITUTE", 3, 4, V, substitute),
    f("REPLACE", 4, 4, V, |c, a| replace(c, a, false)),
    f("REPLACEB", 4, 4, V, |c, a| replace(c, a, true)),
    f("FIND", 2, 3, V, |c, a| find(c, a, false, false)),
    f("FINDB", 2, 3, V, |c, a| find(c, a, false, true)),
    f("SEARCH", 2, 3, V, |c, a| find(c, a, true, false)),
    f("SEARCHB", 2, 3, V, |c, a| find(c, a, true, true)),
    f("T", 1, 1, V, |_, a| match &a[0] {
        Value::Text(s) => Value::Text(s.clone()),
        Value::Error(e) => Value::Error(*e),
        _ => Value::from(""),
    }),
    f("VALUE", 1, 1, V, |c, a| match &a[0] {
        // "TRUE" 는 1 (한셀). 논리 값 인수는 #VALUE!
        Value::Text(s) => match value::parse_bool_text(s).map(|b| b as u8 as f64).or_else(|| value::parse_number(s, c.date1904())) {
            Some(n) => Value::Number(n),
            None => Value::Error(ErrorKind::Value),
        },
        Value::Bool(_) => Value::Error(ErrorKind::Value),
        v => value::num_r(c.num(v)),
    }),
    f("NUMBERVALUE", 1, 3, V, numbervalue),
    f("TEXT", 2, 2, V, text),
    f("FIXED", 1, 3, V, fixed),
    f("DOLLAR", 1, 2, V, |c, a| currency(c, a, "₩")),
    f("USDOLLAR", 1, 2, V, |c, a| currency(c, a, "$")),
    f("WON", 1, 2, V, |c, a| currency(c, a, "₩")),
    // CHAR 는 949 의 한 바이트: 1–159 는 그 코드의 글자, 160–255 (두 바이트 글자의 첫 바이트 등)는 U+FFFD
    // (대조 실험 text·misc4)
    f("CHAR", 1, 1, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        if !(1.0..=255.0).contains(&n) {
            return Value::Error(ErrorKind::Value);
        }
        let ch = match n as u32 {
            k @ 1..=159 => char::from_u32(k).unwrap_or('\u{FFFD}'),
            _ => '\u{FFFD}',
        };
        Value::from(ch.to_string())
    }),
    // CODE 는 첫 글자의 949 코드 (CODE("한") = 51153, 949 에 없으면 63, 대조 실험 text)
    f("CODE", 1, 1, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        match t.chars().next() {
            Some(ch) => Value::Number(crate::calc::cp949::code(ch) as f64),
            None => Value::Error(ErrorKind::Value),
        }
    }),
    f("UNICHAR", 1, 1, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        if n < 1.0 || n > 0x10FFFF as f64 {
            return Value::Error(ErrorKind::Value);
        }
        // 대리 쌍 반쪽은 U+FFFD (UNICHAR(55296), 대조 실험 text)
        match char::from_u32(n as u32) {
            Some(ch) => Value::from(ch.to_string()),
            None => Value::from("\u{FFFD}"),
        }
    }),
    f("UNICODE", 1, 1, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        match t.chars().next() {
            Some(ch) => Value::Number(ch as u32 as f64),
            None => Value::Error(ErrorKind::Value),
        }
    }),
    f("ASC", 1, 1, V, |c, a| Value::from(to_halfwidth(&try_v!(text_arg(c, a, 0))))),
    f("DBCS", 1, 1, V, |c, a| Value::from(to_fullwidth(&try_v!(text_arg(c, a, 0))))),
    f("JUNJA", 1, 1, V, |c, a| Value::from(to_fullwidth(&try_v!(text_arg(c, a, 0))))),
    // PHONETIC: 범위 안 글 칸을 이어 붙인다 (숫자 칸은 빼고), 참조가 아니면 #NAME? (대조 실험 text)
    f("PHONETIC", 1, 1, R, |c, a| match &a[0] {
        Value::Ref(_) => {
            let mut s = String::new();
            for v in flatten(c, &a[0]) {
                if let Value::Text(t) = v {
                    s.push_str(&t);
                }
            }
            Value::from(s)
        }
        Value::Error(e) => Value::Error(*e),
        _ => Value::Error(ErrorKind::Name),
    }),
    f("NUMBERSTRING", 2, 2, V, numberstring),
    f("DATESTRING", 1, 1, V, datestring),
    f("BAHTTEXT", 1, 1, V, bahttext),
];

/// 코드 페이지 949 의 바이트 수
pub fn byte_len(ch: char) -> usize {
    crate::calc::cp949::byte_len(ch)
}

/// 바이트 자리 [from, to) (0부터)의 글. 걸친 글자 반쪽은 공백 하나 (LEFTB("한글나라",3) = "한 ")
fn byte_slice(chars: &[char], from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut pos = 0usize;
    for &ch in chars {
        let (s, e) = (pos, pos + byte_len(ch));
        pos = e;
        if e <= from || s >= to {
            continue;
        }
        out.push(if s >= from && e <= to { ch } else { ' ' });
    }
    out
}

/// 글자마다 대·소문자 (한 글자로 바뀔 때만)
fn map_case(s: &str, upper: bool) -> String {
    s.chars()
        .map(|c| {
            let mut it: Box<dyn Iterator<Item = char>> =
                if upper { Box::new(c.to_uppercase()) } else { Box::new(c.to_lowercase()) };
            match (it.next(), it.next()) {
                (Some(x), None) => x,
                _ => c,
            }
        })
        .collect()
}

fn left_right(ctx: &mut Ctx, a: &[Value], left: bool, bytes: bool) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let n = try_v!(num_or(ctx, a, 1, 1.0)).trunc();
    if n < 0.0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = t.chars().collect();
    if bytes {
        let total: usize = chars.iter().map(|c| byte_len(*c)).sum();
        let n = (n as usize).min(total);
        return Value::from(if left { byte_slice(&chars, 0, n) } else { byte_slice(&chars, total - n, total) });
    }
    let take = (n as usize).min(chars.len());
    let s: String = if left {
        chars[..take].iter().collect()
    } else {
        chars[chars.len() - take..].iter().collect()
    };
    Value::from(s)
}

fn mid(ctx: &mut Ctx, a: &[Value], bytes: bool) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let start = try_v!(num_arg(ctx, a, 1)).trunc();
    let n = try_v!(num_arg(ctx, a, 2)).trunc();
    if start < 1.0 || n < 0.0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = t.chars().collect();
    if bytes {
        let from = start as usize - 1;
        return Value::from(byte_slice(&chars, from, from.saturating_add(n as usize)));
    }
    let s = start as usize - 1;
    if s >= chars.len() {
        return Value::from("");
    }
    let e = (s + n as usize).min(chars.len());
    Value::from(chars[s..e].iter().collect::<String>())
}

/// PROPER: U+0080–U+00FF 의 글자(À, ñ, é …)는 글자로 세지만 대·소문자는 그대로 둔다
/// (PROPER("ÀÉÎ ñ ß") 그대로, PROPER("élan vital") = "élan Vital"), 전각·그리스·키릴 글자는 바뀐다
/// (대조 실험 text·misc4)
fn proper(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let mut out = String::with_capacity(t.len());
    let mut prev_letter = false;
    for ch in t.chars() {
        if ch.is_alphabetic() && (0x80..0x100).contains(&(ch as u32)) {
            out.push(ch);
            prev_letter = true;
            continue;
        }
        if ch.is_alphabetic() {
            if prev_letter {
                out.extend(ch.to_lowercase());
            } else {
                out.extend(ch.to_uppercase());
            }
            prev_letter = true;
        } else {
            out.push(ch);
            prev_letter = false;
        }
    }
    Value::from(out)
}

/// TEXTJOIN: 구분자는 범위면 차례로 돌려 쓰고, 빈 칸은 늘 빼며 (빈 글 "" 은 무시 인수가 거짓이면 넣는다),
/// 오류 칸을 만나면 거기까지 이은 글을 돌려준다 (TEXTJOIN(",",TRUE,{1,#N/A,3}) = "1", 대조 실험 text)
fn textjoin(ctx: &mut Ctx, a: &[Value]) -> Value {
    let delims: Vec<String> = match &a[0] {
        Value::Ref(_) | Value::Array(_) => {
            let mut d = Vec::new();
            for x in flatten(ctx, &a[0]) {
                d.push(try_v!(ctx.text(&x)).to_string());
            }
            d
        }
        v => vec![try_v!(ctx.text(v)).to_string()],
    };
    let ignore = ctx.boolean(&a[1]).unwrap_or(false);
    let mut parts: Vec<String> = Vec::new();
    'outer: for v in &a[2..] {
        let from_ref = matches!(v, Value::Ref(_));
        for x in flatten(ctx, v) {
            if from_ref && matches!(x, Value::Empty) {
                continue;
            }
            if matches!(x, Value::Error(_)) {
                break 'outer;
            }
            let s = try_v!(ctx.text(&x));
            if ignore && s.is_empty() {
                continue;
            }
            parts.push(s.to_string());
        }
    }
    let mut s = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 && !delims.is_empty() {
            s.push_str(&delims[(i - 1) % delims.len()]);
        }
        s.push_str(p);
    }
    if s.chars().count() > 32_767 {
        return Value::Error(ErrorKind::Value);
    }
    Value::from(s)
}

fn substitute(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let old = try_v!(text_arg(ctx, a, 1));
    let new = try_v!(text_arg(ctx, a, 2));
    if old.is_empty() {
        return Value::Text(t);
    }
    match opt(a, 3) {
        None => Value::from(t.replace(&*old, &new)),
        Some(v) => {
            let n = try_v!(ctx.num(v)).trunc();
            if n < 1.0 {
                return Value::Error(ErrorKind::Value);
            }
            let mut count = 0;
            let mut idx = 0;
            while let Some(p) = t[idx..].find(&*old) {
                count += 1;
                let at = idx + p;
                if count as f64 == n {
                    let mut s = String::with_capacity(t.len());
                    s.push_str(&t[..at]);
                    s.push_str(&new);
                    s.push_str(&t[at + old.len()..]);
                    return Value::from(s);
                }
                idx = at + old.len();
            }
            Value::Text(t)
        }
    }
}

fn replace(ctx: &mut Ctx, a: &[Value], bytes: bool) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let start = try_v!(num_arg(ctx, a, 1)).trunc();
    let n = try_v!(num_arg(ctx, a, 2)).trunc();
    let new = try_v!(text_arg(ctx, a, 3));
    if start < 1.0 || n < 0.0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = t.chars().collect();
    if bytes {
        // 앞쪽·뒤쪽 바이트 조각 사이에 새 글 (걸친 반쪽은 공백: REPLACEB("한글나라",2,2,"X") = " X 나라")
        let total: usize = chars.iter().map(|c| byte_len(*c)).sum();
        let from = start as usize - 1;
        let mut out = byte_slice(&chars, 0, from.min(total));
        out.push_str(&new);
        out.push_str(&byte_slice(&chars, from.saturating_add(n as usize).min(total), total));
        return Value::from(out);
    }
    let (s, e) = {
        let s = (start as usize - 1).min(chars.len());
        (s, (s + n as usize).min(chars.len()))
    };
    let mut out: String = chars[..s].iter().collect();
    out.push_str(&new);
    out.extend(chars[e..].iter());
    Value::from(out)
}

fn find(ctx: &mut Ctx, a: &[Value], search: bool, bytes: bool) -> Value {
    let needle = try_v!(text_arg(ctx, a, 0));
    let hay = try_v!(text_arg(ctx, a, 1));
    let start = try_v!(num_or(ctx, a, 2, 1.0)).trunc();
    let hc: Vec<char> = hay.chars().collect();
    // 바이트 위치를 글자 위치로
    let to_char_pos = |b: usize| -> Option<usize> {
        let mut pos = 1usize;
        for (i, ch) in hc.iter().enumerate() {
            if pos >= b {
                return Some(i);
            }
            pos += byte_len(*ch);
        }
        (pos >= b).then_some(hc.len())
    };
    let s = if bytes {
        match to_char_pos(start.max(0.0) as usize) {
            Some(i) if start >= 1.0 => i,
            _ => return Value::Error(ErrorKind::Value),
        }
    } else {
        // SEARCH 는 시작이 글 길이를 넘으면 빈 글을 찾아도 #VALUE! (FIND 는 길이 + 1 까지, 대조 실험 text)
        let limit = if search { hc.len() } else { hc.len() + 1 };
        if start < 1.0 || start as usize > limit {
            return Value::Error(ErrorKind::Value);
        }
        start as usize - 1
    };
    if bytes && search && start as usize > hc.iter().map(|c| byte_len(*c)).sum::<usize>() {
        return Value::Error(ErrorKind::Value);
    }
    if s > hc.len() {
        return Value::Error(ErrorKind::Value);
    }
    let found = if search {
        let pat = value::fold(&needle);
        let wild = crate::calc::criteria::has_wildcards(&pat);
        let folded: Vec<String> = hc.iter().map(|c| value::fold(&c.to_string())).collect();
        let mut hit = None;
        for i in s..=hc.len() {
            if wild {
                // 여기서 시작해 끝까지 가는 가장 짧은 일치
                for j in i..=hc.len() {
                    let cand: String = folded[i..j].concat();
                    if wildcard_match(&pat, &cand) {
                        hit = Some(i);
                        break;
                    }
                }
                if hit.is_some() {
                    break;
                }
            } else {
                let cand: String = folded[i..].concat();
                if cand.starts_with(&pat) {
                    hit = Some(i);
                    break;
                }
            }
        }
        hit
    } else {
        let nc: Vec<char> = needle.chars().collect();
        (s..=hc.len().saturating_sub(nc.len()).max(s)).find(|&i| {
            i + nc.len() <= hc.len() && hc[i..i + nc.len()] == nc[..]
        })
    };
    match found {
        Some(i) => {
            if bytes {
                Value::Number((hc[..i].iter().map(|c| byte_len(*c)).sum::<usize>() + 1) as f64)
            } else {
                Value::Number(i as f64 + 1.0)
            }
        }
        None => Value::Error(ErrorKind::Value),
    }
}

/// NUMBERVALUE: 자리 구분자(소수점과 같으면 무시)를 지우고 소수점을 "." 로 바꾼 뒤 공백을 빼고, 끝의 %
/// 마다 100 으로 나누며 나머지는 VALUE 문법으로 읽는다 ("(5)" = −5, "$5" = 5). 논리 값은 그대로
/// (NUMBERVALUE(TRUE) = TRUE, 대조 실험 text).
fn numbervalue(ctx: &mut Ctx, a: &[Value]) -> Value {
    if let Value::Bool(b) = a[0] {
        return Value::Bool(b);
    }
    let t = try_v!(text_arg(ctx, a, 0));
    let dec = match opt(a, 1) {
        Some(v) => try_v!(ctx.text(v)).chars().next().unwrap_or('.'),
        None => '.',
    };
    let grp = match opt(a, 2) {
        Some(v) => try_v!(ctx.text(v)).chars().next().unwrap_or(','),
        None => ',',
    };
    let mut s: String = if dec == grp { t.to_string() } else { t.chars().filter(|&c| c != grp).collect() };
    if dec != '.' {
        s = s.chars().map(|c| if c == dec { '.' } else { c }).collect();
    }
    let mut s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Value::Number(0.0);
    }
    let mut pct = 0;
    while s.ends_with('%') {
        s.pop();
        pct += 1;
    }
    match value::parse_plain_number(&s) {
        Some(v) => value::num_r(Ok(v / 100f64.mpowi(pct))),
        None => Value::Error(ErrorKind::Value),
    }
}

/// TEXT. 한셀의 TEXT 는 셀 표시와 몇 가지가 다르다 (대조 실험 text·misc3):
/// - 빈 칸은 어떤 서식이든 "", 색 지정(`[Blue]`)은 #VALUE!
/// - `\\` 는 이스케이프가 아니라 글자 그대로 (`TEXT(5,"\\$0")` = "\\$5"), 그래서 숫자 자리와 날짜 기호가
///   섞이면 #VALUE! (`"#,##0,,\\M"`)
/// - `_x` 는 공백 하나가 되어 글 앞으로 간다 (`TEXT(1234.567,"0.00_)")` = " 1234.57")
/// - `오전/오후` 는 AM/PM 으로, `@` 는 처음 하나만 값으로 바뀐다
fn text(ctx: &mut Ctx, a: &[Value]) -> Value {
    let code = try_v!(text_arg(ctx, a, 1));
    let v = match &a[0] {
        Value::Error(e) => return Value::Error(*e),
        Value::Bool(b) => return Value::from(if *b { "TRUE" } else { "FALSE" }),
        Value::Text(s) => match value::parse_number(s, ctx.date1904()) {
            Some(n) => CellValue::Number(n),
            None => CellValue::Text(s.to_string()),
        },
        Value::Empty => return Value::from(""),
        Value::Number(n) => CellValue::Number(*n),
        _ => return Value::Error(ErrorKind::Value),
    };
    if code.is_empty() {
        return Value::from("");
    }
    let Some((code, pads)) = text_code(&code) else {
        return Value::Error(ErrorKind::Value);
    };
    // 공백 맞춤은 쓰인 구역의 것만 (음수 구역이 따로 있으면 그 구역)
    let sec = match &v {
        CellValue::Number(n) if *n < 0.0 && pads.len() > 1 => 1,
        CellValue::Number(n) if *n == 0.0 && pads.len() > 2 => 2,
        CellValue::Text(_) if pads.len() > 3 => 3,
        _ => 0,
    };
    let out = crate::numfmt::format_padded(&v, &code, ctx.date1904());
    let marks = out.text.chars().filter(|&c| c == crate::numfmt::PAD_CHAR).count();
    let body: String = out.text.chars().filter(|&c| c != crate::numfmt::PAD_CHAR).collect();
    Value::from(format!("{}{}", " ".repeat(pads.get(sec).copied().unwrap_or(0) + marks), body))
}

/// 따옴표 밖의 "General"(대소문자 무시)을 날짜 기호로: G·g 는 지우고, e·E 는 연도 e, 다른 글자는 따옴표 글자
fn replace_general(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    let mut quoted = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            quoted = !quoted;
        }
        let word: String = chars[i..(i + 7).min(chars.len())].iter().collect();
        if !quoted && word.eq_ignore_ascii_case("general") {
            for &w in &chars[i + 1..i + 7] {
                if w == 'e' || w == 'E' {
                    out.push('e');
                } else {
                    out.push('"');
                    out.push(w);
                    out.push('"');
                }
            }
            i += 7;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// TEXT 의 서식 코드를 표시 엔진의 코드로 (위 규칙). (코드, 구역마다 앞에 붙일 공백 수) 또는 None (#VALUE!)
fn text_code(code: &str) -> Option<(String, Vec<usize>)> {
    const COLORS: [&str; 9] = ["black", "blue", "cyan", "green", "magenta", "red", "white", "yellow", "color"];
    let mut out = String::with_capacity(code.len() + 8);
    let mut pad = 0usize;
    let mut pads = vec![0usize];
    let mut at_seen = false;
    let (mut digits, mut dates) = (false, false);
    // "General" 은 TEXT 에서 일반 서식이 아니다: G 는 연호(빈 글), e 는 연도, 나머지 글자는 그대로
    // (TEXT(5,"General") = "1900n1900ral", 대조 실험 misc3·misc4)
    // 셀 표시와 달리 TEXT 에서는 "G/표준" 이 일반 서식이다 (TEXT(5,"G/표준") = "5", 대조 실험 misc3)
    let code = replace_general(&code.replace("오전/오후", "AM/PM")).replace("G/표준", "General");
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let j = chars[i + 1..].iter().position(|&x| x == '"').map_or(chars.len(), |k| i + 1 + k);
                out.extend(&chars[i..(j + 1).min(chars.len())]);
                i = j + 1;
                continue;
            }
            '[' => {
                let j = chars[i..].iter().position(|&x| x == ']').map_or(chars.len(), |k| i + k);
                let inner: String = chars[i + 1..j.min(chars.len())].iter().collect::<String>().to_ascii_lowercase();
                if COLORS.iter().any(|k| inner.starts_with(k)) {
                    return None;
                }
                if matches!(inner.chars().next(), Some('h' | 'm' | 's')) {
                    dates = true;
                }
                out.extend(&chars[i..(j + 1).min(chars.len())]);
                i = j + 1;
                continue;
            }
            '\\' => out.push_str("\"\\\""),
            '_' => {
                *pads.last_mut().unwrap_or(&mut pad) += 1;
                i += 2;
                continue;
            }
            '@' => {
                if !at_seen {
                    out.push('@');
                }
                at_seen = true;
            }
            ';' => {
                (digits, dates) = (false, false);
                pads.push(0);
                out.push(c);
            }
            '0' | '#' | '?' => {
                digits = true;
                out.push(c);
            }
            // 지수 E+ E- 가 아닌 e 는 연도, G 는 연호
            'e' | 'E' if !matches!(chars.get(i + 1), Some('+' | '-')) => {
                dates = true;
                out.push(c);
            }
            'y' | 'Y' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' | 'g' | 'G' => {
                dates = true;
                out.push(c);
            }
            'm' | 'M' => {
                // AM/PM 의 M 은 날짜 기호가 아니다
                let ampm = i >= 1 && matches!(chars[i - 1], 'A' | 'a' | 'P' | 'p');
                if !ampm {
                    dates = true;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
        if digits && dates {
            return None;
        }
        i += 1;
    }
    let _ = pad;
    Some((out, pads))
}

/// 천 단위 쉼표
pub fn group(int: &str) -> String {
    let b: Vec<char> = int.chars().collect();
    let mut out = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

/// 반올림한 수를 소수 d 자리(음수면 0) 글로. 유효숫자 15자리 뒤는 0 (FIXED(1234.567,127), 대조 실험 text)
pub fn format_fixed(x: f64, d: i32, commas: bool) -> String {
    let r = round_digits(x, d as f64, Rounding::HalfAway);
    let dd = d.max(0) as usize;
    let s = digits15_fixed(r.abs(), dd);
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (s, None),
    };
    let mut out = String::new();
    if r < 0.0 {
        out.push('-');
    }
    out.push_str(&if commas { group(&int) } else { int });
    if let Some(f) = frac {
        out.push('.');
        out.push_str(&f);
    }
    out
}

/// 소수 dd 자리 글, 15자리 넘는 유효숫자는 0
fn digits15_fixed(a: f64, dd: usize) -> String {
    let s = format!("{:.*}", dd, a);
    let mut sig = 0;
    s.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                if sig > 0 || c != '0' {
                    sig += 1;
                }
                if sig > 15 {
                    return '0';
                }
            }
            c
        })
        .collect()
}

fn fixed(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d = try_v!(num_or(ctx, a, 1, 2.0)).trunc();
    let no_commas = try_v!(bool_or(ctx, a, 2, false));
    if d > 127.0 {
        return Value::Error(ErrorKind::Value);
    }
    Value::from(format_fixed(x, d as i32, !no_commas))
}

fn currency(ctx: &mut Ctx, a: &[Value], sym: &str) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d = try_v!(num_or(ctx, a, 1, if sym == "$" { 2.0 } else { 0.0 })).trunc();
    if d > 127.0 {
        return Value::Error(ErrorKind::Value);
    }
    let body = format_fixed(x.abs(), d as i32, true);
    let neg = round_digits(x, d, Rounding::HalfAway) < 0.0;
    if sym == "$" {
        Value::from(if neg { format!("(${body})") } else { format!("${body}") })
    } else {
        Value::from(if neg { format!("-{sym}{body}") } else { format!("{sym}{body}") })
    }
}

/// 전각 → 반각 (ASC)
pub fn to_halfwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            0x3000 => ' ',
            _ => c,
        })
        .collect()
}

/// 반각 → 전각 (DBCS, 한셀 JUNJA)
pub fn to_fullwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c as u32 {
            0x21..=0x7E => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
            0x20 => '\u{3000}',
            _ => c,
        })
        .collect()
}

/// 숫자 → 한글·한자 읽기 (NUMBERSTRING 형식 1 한글, 2 한자 갖은자, 3 한자)
pub fn number_string(n: u64, kind: u8) -> String {
    let (digits, small, big): (&[&str], &[&str], &[&str]) = match kind {
        1 => (
            &["", "일", "이", "삼", "사", "오", "육", "칠", "팔", "구"],
            &["", "십", "백", "천"],
            // 한셀은 경 이상이 없다 (1E+16 → "일조", 대조 실험 misc4)
            &["", "만", "억", "조"],
        ),
        // 한셀은 參·拾 을 호환 한자(U+F96B, U+F973)로 쓴다 (KS X 1001 의 그 칸, 대조 실험 text)
        2 => (
            &["", "壹", "貳", "\u{F96B}", "四", "伍", "\u{F9D1}", "七", "八", "九"],
            &["", "\u{F973}", "百", "阡"],
            &["", "萬", "億", "兆"],
        ),
        _ => (
            &["", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
            &["", "十", "百", "千"],
            &["", "萬", "億", "兆", "京"],
        ),
    };
    // 한셀은 零·六 도 호환 한자 (U+F9B2, U+F9D1, 대조 실험 misc4)
    if n == 0 {
        return match kind {
            1 => "영".into(),
            2 => "\u{F9B2}".into(),
            _ => "零".into(),
        };
    }
    let s = n.to_string();
    let ds: Vec<u32> = s.chars().map(|c| c.to_digit(10).unwrap()).collect();
    let len = ds.len();
    let mut out = String::new();
    let mut group_has = false;
    for (i, &d) in ds.iter().enumerate() {
        let pos = len - 1 - i;
        let (g, p) = (pos / 4, pos % 4);
        if d != 0 {
            out.push_str(digits[d as usize]);
            out.push_str(small[p]);
            group_has = true;
        }
        if p == 0 {
            if group_has && g > 0 {
                out.push_str(big[g.min(big.len() - 1)]);
            }
            group_has = false;
        }
    }
    out
}

/// NUMBERSTRING: 형식 1 한글 읽기, 2 갖은자, 3 은 숫자 하나씩 한글로 ("일이삼사"). 음수·없는 형식은 #NUM!
/// (대조 실험 text)
fn numberstring(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let kind = try_v!(num_arg(ctx, a, 1)).trunc();
    if !(1.0..=3.0).contains(&kind) || x < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let n = x.round();
    if n >= 1e19 {
        return Value::Error(ErrorKind::Num);
    }
    if kind == 3.0 {
        const D: [&str; 10] = ["영", "일", "이", "삼", "사", "오", "육", "칠", "팔", "구"];
        let s: String = (n as u64).to_string().chars().map(|c| D[c.to_digit(10).unwrap_or(0) as usize]).collect();
        return Value::from(s);
    }
    Value::from(number_string(n as u64, kind as u8))
}

fn datestring(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    match crate::calc::date::ymd_from_serial(x, ctx.date1904()) {
        Some((y, m, d)) => Value::from(format!("{:02}년 {:02}월 {:02}일", y % 100, m, d)),
        None => Value::Error(ErrorKind::Value),
    }
}

/// 태국어 바트 읽기
fn bahttext(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let r = round_digits(x, 2.0, Rounding::HalfAway);
    let neg = r < 0.0;
    let r = r.abs();
    let baht = r.trunc() as u64;
    let satang = ((r - r.trunc()) * 100.0).round() as u64;
    const D: [&str; 10] = ["", "หนึ่ง", "สอง", "สาม", "สี่", "ห้า", "หก", "เจ็ด", "แปด", "เก้า"];
    const P: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];
    fn say(n: u64) -> String {
        if n == 0 {
            return String::new();
        }
        if n >= 1_000_000 {
            return format!("{}ล้าน{}", say(n / 1_000_000), say(n % 1_000_000));
        }
        let s = n.to_string();
        let len = s.len();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            let d = c.to_digit(10).unwrap() as usize;
            let pos = len - 1 - i;
            if d == 0 {
                continue;
            }
            if pos == 0 && d == 1 && len > 1 {
                out.push_str("เอ็ด");
            } else if pos == 1 && d == 2 {
                out.push_str("ยี่");
            } else if !(pos == 1 && d == 1) {
                out.push_str(D[d]);
            }
            out.push_str(P[pos]);
        }
        out
    }
    let mut out = String::new();
    if neg {
        out.push_str("ลบ");
    }
    if baht == 0 && satang == 0 {
        out.push_str("ศูนย์บาทถ้วน");
        return Value::from(out);
    }
    if baht > 0 {
        out.push_str(&say(baht));
        out.push_str("บาท");
    }
    if satang == 0 {
        out.push_str("ถ้วน");
    } else {
        out.push_str(&say(satang));
        out.push_str("สตางค์");
    }
    Value::from(out)
}
