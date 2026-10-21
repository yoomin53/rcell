// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 글 함수.
//!
//! 글자 위치는 유니코드 글자 단위로 센다. 바이트 함수(LENB …)는 한국어 환경처럼 ASCII 밖의 글자를
//! 2바이트로 센다.

use super::math::{Rounding, round_digits};
use super::{Arg, FnSpec, MANY, R, V, bool_or, f, flatten, num_arg, num_or, opt, text_arg};
use crate::calc::criteria::wildcard_match;
use crate::calc::eval::Ctx;
use crate::calc::value::{self, ErrorKind, Value};
use crate::model::CellValue;

const VR: &[Arg] = &[Arg::V, Arg::V, Arg::R];

pub const FNS: &[FnSpec] = &[
    f("LEN", 1, 1, V, |c, a| Value::Number(try_v!(text_arg(c, a, 0)).chars().count() as f64)),
    f("LENB", 1, 1, V, |c, a| Value::Number(try_v!(text_arg(c, a, 0)).chars().map(byte_len).sum::<usize>() as f64)),
    f("LEFT", 1, 2, V, |c, a| left_right(c, a, true, false)),
    f("RIGHT", 1, 2, V, |c, a| left_right(c, a, false, false)),
    f("LEFTB", 1, 2, V, |c, a| left_right(c, a, true, true)),
    f("RIGHTB", 1, 2, V, |c, a| left_right(c, a, false, true)),
    f("MID", 3, 3, V, |c, a| mid(c, a, false)),
    f("MIDB", 3, 3, V, |c, a| mid(c, a, true)),
    f("LOWER", 1, 1, V, |c, a| Value::from(try_v!(text_arg(c, a, 0)).to_lowercase())),
    f("UPPER", 1, 1, V, |c, a| Value::from(try_v!(text_arg(c, a, 0)).to_uppercase())),
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
        Value::Text(s) => match value::parse_number(s, c.date1904()) {
            Some(n) => Value::Number(n),
            None => Value::Error(ErrorKind::Value),
        },
        v => value::num_r(c.num(v)),
    }),
    f("NUMBERVALUE", 1, 3, V, numbervalue),
    f("TEXT", 2, 2, V, text),
    f("FIXED", 1, 3, V, fixed),
    f("DOLLAR", 1, 2, V, |c, a| currency(c, a, "₩")),
    f("USDOLLAR", 1, 2, V, |c, a| currency(c, a, "$")),
    f("WON", 1, 2, V, |c, a| currency(c, a, "₩")),
    f("CHAR", 1, 1, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        if !(1.0..=255.0).contains(&n) {
            return Value::Error(ErrorKind::Value);
        }
        match char::from_u32(n as u32) {
            Some(ch) => Value::from(ch.to_string()),
            None => Value::Error(ErrorKind::Value),
        }
    }),
    f("CODE", 1, 1, V, |c, a| {
        let t = try_v!(text_arg(c, a, 0));
        match t.chars().next() {
            Some(ch) => Value::Number(ch as u32 as f64),
            None => Value::Error(ErrorKind::Value),
        }
    }),
    f("UNICHAR", 1, 1, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        if n < 1.0 || n > 0x10FFFF as f64 {
            return Value::Error(ErrorKind::Value);
        }
        match char::from_u32(n as u32) {
            Some(ch) => Value::from(ch.to_string()),
            None => Value::Error(ErrorKind::NA),
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
    f("PHONETIC", 1, 1, R, |c, a| match &a[0] {
        Value::Ref(r) => {
            let v = c.intersect_ref(r);
            value::to_text(&v).map(Value::Text).unwrap_or(Value::from(""))
        }
        v => value::to_text(v).map(Value::Text).unwrap_or_else(Value::Error),
    }),
    f("NUMBERSTRING", 2, 2, V, numberstring),
    f("DATESTRING", 1, 1, V, datestring),
    f("BAHTTEXT", 1, 1, V, bahttext),
];

/// 한국어 환경의 바이트 수 (ASCII 1, 그 밖 2)
pub fn byte_len(ch: char) -> usize {
    if (ch as u32) < 0x80 { 1 } else { 2 }
}

fn left_right(ctx: &mut Ctx, a: &[Value], left: bool, bytes: bool) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let n = try_v!(num_or(ctx, a, 1, 1.0)).trunc();
    if n < 0.0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = t.chars().collect();
    let take = if bytes {
        // 바이트 수 안에 드는 글자 수
        let mut used = 0usize;
        let mut k = 0usize;
        let it: Box<dyn Iterator<Item = &char>> = if left {
            Box::new(chars.iter())
        } else {
            Box::new(chars.iter().rev())
        };
        for ch in it {
            let b = byte_len(*ch);
            if (used + b) as f64 > n {
                break;
            }
            used += b;
            k += 1;
        }
        k
    } else {
        (n as usize).min(chars.len())
    };
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
        // 바이트 위치로
        let mut out = String::new();
        let mut pos = 1usize;
        let end = start as usize + n as usize;
        for ch in &chars {
            let b = byte_len(*ch);
            if pos >= start as usize && pos + b <= end {
                out.push(*ch);
            }
            pos += b;
        }
        return Value::from(out);
    }
    let s = start as usize - 1;
    if s >= chars.len() {
        return Value::from("");
    }
    let e = (s + n as usize).min(chars.len());
    Value::from(chars[s..e].iter().collect::<String>())
}

fn proper(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let mut out = String::with_capacity(t.len());
    let mut prev_letter = false;
    for ch in t.chars() {
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

fn textjoin(ctx: &mut Ctx, a: &[Value]) -> Value {
    let delim = try_v!(text_arg(ctx, a, 0));
    let ignore = try_v!(ctx.boolean(&a[1]));
    let mut parts: Vec<String> = Vec::new();
    for v in &a[2..] {
        for x in flatten(ctx, v) {
            let s = try_v!(ctx.text(&x));
            if ignore && s.is_empty() {
                continue;
            }
            parts.push(s.to_string());
        }
    }
    let s = parts.join(&delim);
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
    let (s, e) = if bytes {
        // 바이트 위치 → 글자 위치
        let mut pos = 1usize;
        let mut si = chars.len();
        let mut ei = chars.len();
        for (i, ch) in chars.iter().enumerate() {
            if pos >= start as usize && si == chars.len() {
                si = i;
            }
            if pos >= start as usize + n as usize && ei == chars.len() {
                ei = i;
                break;
            }
            pos += byte_len(*ch);
        }
        (si, ei.max(si))
    } else {
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
        if start < 1.0 || start as usize > hc.len() + 1 {
            return Value::Error(ErrorKind::Value);
        }
        start as usize - 1
    };
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

fn numbervalue(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(text_arg(ctx, a, 0));
    let dec = match opt(a, 1) {
        Some(v) => try_v!(ctx.text(v)).chars().next().unwrap_or('.'),
        None => '.',
    };
    let grp = match opt(a, 2) {
        Some(v) => try_v!(ctx.text(v)).chars().next().unwrap_or(','),
        None => ',',
    };
    if dec == grp {
        return Value::Error(ErrorKind::Value);
    }
    let s: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Value::Number(0.0);
    }
    let mut out = String::new();
    let mut seen_dec = false;
    for ch in s.chars() {
        if ch == dec {
            if seen_dec {
                return Value::Error(ErrorKind::Value);
            }
            seen_dec = true;
            out.push('.');
        } else if ch == grp {
            if seen_dec {
                return Value::Error(ErrorKind::Value);
            }
        } else {
            out.push(ch);
        }
    }
    let mut pct = 0;
    while out.ends_with('%') {
        out.pop();
        pct += 1;
    }
    match out.parse::<f64>() {
        Ok(v) if out.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')) => {
            Value::Number(v / 100f64.powi(pct))
        }
        _ => Value::Error(ErrorKind::Value),
    }
}

fn text(ctx: &mut Ctx, a: &[Value]) -> Value {
    let code = try_v!(text_arg(ctx, a, 1));
    let v = match &a[0] {
        Value::Error(e) => return Value::Error(*e),
        Value::Bool(b) => return Value::from(if *b { "TRUE" } else { "FALSE" }),
        Value::Text(s) => match value::parse_number(s, ctx.date1904()) {
            Some(n) => CellValue::Number(n),
            None => CellValue::Text(s.to_string()),
        },
        Value::Empty => CellValue::Number(0.0),
        Value::Number(n) => CellValue::Number(*n),
        _ => return Value::Error(ErrorKind::Value),
    };
    if code.is_empty() {
        return Value::from("");
    }
    let out = crate::numfmt::format(&v, &code, ctx.date1904());
    Value::from(out.text)
}

/// 천 단위 쉼표
pub fn group(int: &str) -> String {
    let b: Vec<char> = int.chars().collect();
    let mut out = String::new();
    for (i, c) in b.iter().enumerate() {
        if i > 0 && (b.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

/// 반올림한 수를 소수 d 자리(음수면 0) 글로
pub fn format_fixed(x: f64, d: i32, commas: bool) -> String {
    let r = round_digits(x, d as f64, Rounding::HalfAway);
    let dd = d.max(0) as usize;
    let s = format!("{:.*}", dd, r.abs());
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
            &["", "만", "억", "조", "경"],
        ),
        2 => (
            &["", "壹", "貳", "參", "四", "伍", "六", "七", "八", "九"],
            &["", "拾", "百", "阡"],
            &["", "萬", "億", "兆", "京"],
        ),
        _ => (
            &["", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
            &["", "十", "百", "千"],
            &["", "萬", "億", "兆", "京"],
        ),
    };
    if n == 0 {
        return match kind {
            1 => "영".into(),
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

fn numberstring(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let kind = try_v!(num_arg(ctx, a, 1)).trunc();
    if !(1.0..=3.0).contains(&kind) || x < 0.0 {
        return Value::Error(ErrorKind::Value);
    }
    let n = x.round();
    if n >= 1e19 {
        return Value::Error(ErrorKind::Value);
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
