// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 공학 함수: 진법 변환, 비트 연산, 복소수, 단위 변환, 베셀 함수, 오차 함수.

use crate::calc::fm::Fm;
use super::special as sp;
use super::{FnSpec, MANY, R, V, f, flatten, num_arg, num_or, opt};
use crate::calc::eval::Ctx;
use crate::calc::value::{self, ErrorKind, Value, num};

pub const FNS: &[FnSpec] = &[
    f("BIN2DEC", 1, 1, V, |c, a| from_base(c, a, 2, None)),
    f("BIN2OCT", 1, 2, V, |c, a| from_base(c, a, 2, Some(8))),
    f("BIN2HEX", 1, 2, V, |c, a| from_base(c, a, 2, Some(16))),
    f("OCT2DEC", 1, 1, V, |c, a| from_base(c, a, 8, None)),
    f("OCT2BIN", 1, 2, V, |c, a| from_base(c, a, 8, Some(2))),
    f("OCT2HEX", 1, 2, V, |c, a| from_base(c, a, 8, Some(16))),
    f("HEX2DEC", 1, 1, V, |c, a| from_base(c, a, 16, None)),
    f("HEX2BIN", 1, 2, V, |c, a| from_base(c, a, 16, Some(2))),
    f("HEX2OCT", 1, 2, V, |c, a| from_base(c, a, 16, Some(8))),
    f("DEC2BIN", 1, 2, V, |c, a| dec2(c, a, 2)),
    f("DEC2OCT", 1, 2, V, |c, a| dec2(c, a, 8)),
    f("DEC2HEX", 1, 2, V, |c, a| dec2(c, a, 16)),
    f("BITAND", 2, 2, V, |c, a| bit(c, a, |x, y| x & y, true)),
    f("BITOR", 2, 2, V, |c, a| bit(c, a, |x, y| x | y, false)),
    f("BITXOR", 2, 2, V, |c, a| bit(c, a, |x, y| x ^ y, false)),
    f("BITLSHIFT", 2, 2, V, |c, a| shift(c, a, true)),
    f("BITRSHIFT", 2, 2, V, |c, a| shift(c, a, false)),
    f("DELTA", 1, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let y = try_v!(num_or(c, a, 1, 0.0));
        Value::Number((x == y) as u8 as f64)
    }),
    f("GESTEP", 1, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let y = try_v!(num_or(c, a, 1, 0.0));
        Value::Number((x >= y) as u8 as f64)
    }),
    // ERF·ERFC 는 음수 인수가 #NUM! (ERF(-1)·ERF(-0.5,0.5)·ERFC(-1), 대조 실험 engineering)
    f("ERF", 1, 2, V, |c, a| {
        let lo = try_v!(num_arg(c, a, 0));
        let hi = match opt(a, 1) {
            Some(u) => Some(try_v!(c.num(u))),
            None => None,
        };
        if lo < 0.0 || hi.is_some_and(|h| h < 0.0) {
            return Value::Error(ErrorKind::Num);
        }
        match hi {
            Some(h) => num(sp::erf(h) - sp::erf(lo)),
            None => num(sp::erf(lo)),
        }
    }),
    f("ERF.PRECISE", 1, 1, V, |c, a| erf_nonneg(try_v!(num_arg(c, a, 0)), sp::erf)),
    f("ERFC", 1, 1, V, |c, a| erf_nonneg(try_v!(num_arg(c, a, 0)), sp::erfc)),
    f("ERFC.PRECISE", 1, 1, V, |c, a| erf_nonneg(try_v!(num_arg(c, a, 0)), sp::erfc)),
    f("BESSELJ", 2, 2, V, |c, a| bessel(c, a, 'J')),
    f("BESSELY", 2, 2, V, |c, a| bessel(c, a, 'Y')),
    f("BESSELI", 2, 2, V, |c, a| bessel(c, a, 'I')),
    f("BESSELK", 2, 2, V, |c, a| bessel(c, a, 'K')),
    f("CONVERT", 3, 3, V, super::units::convert),
    f("CONVERTK", 4, 4, V, super::units::convertk),
    // 복소수
    f("COMPLEX", 2, 3, V, |c, a| {
        let re = try_v!(num_arg(c, a, 0));
        let im = try_v!(num_arg(c, a, 1));
        let suf = match opt(a, 2) {
            Some(v) => try_v!(c.text(v)).to_string(),
            None => "i".into(),
        };
        if suf != "i" && suf != "j" {
            return Value::Error(ErrorKind::Value);
        }
        Value::from(fmt_complex(re, im, suf.chars().next().unwrap()))
    }),
    f("IMREAL", 1, 1, V, |c, a| im1n(c, a, |z| Ok(z.0))),
    f("IMAGINARY", 1, 1, V, |c, a| im1n(c, a, |z| Ok(z.1))),
    f("IMABS", 1, 1, V, |c, a| im1n(c, a, |z| Ok(z.0.mhypot(z.1)))),
    f("IMARGUMENT", 1, 1, V, |c, a| im1n(c, a, |z| {
        if z.0 == 0.0 && z.1 == 0.0 { Err(ErrorKind::Div0) } else { Ok(z.1.matan2(z.0)) }
    })),
    f("IMCONJUGATE", 1, 1, V, |c, a| im1(c, a, |z| Ok((z.0, -z.1)))),
    f("IMSQRT", 1, 1, V, |c, a| im1(c, a, |z| {
        let r = z.0.mhypot(z.1).sqrt();
        let t = z.1.matan2(z.0) / 2.0;
        Ok((r * t.mcos(), r * t.msin()))
    })),
    f("IMEXP", 1, 1, V, |c, a| im1(c, a, |z| Ok(cexp(z)))),
    f("IMLN", 1, 1, V, |c, a| im1(c, a, cln)),
    f("IMLOG10", 1, 1, V, |c, a| im1(c, a, |z| {
        let l = cln(z)?;
        let k = std::f64::consts::LN_10;
        Ok((l.0 / k, l.1 / k))
    })),
    f("IMLOG2", 1, 1, V, |c, a| im1(c, a, |z| {
        let l = cln(z)?;
        let k = std::f64::consts::LN_2;
        Ok((l.0 / k, l.1 / k))
    })),
    f("IMSIN", 1, 1, V, |c, a| im1(c, a, |z| Ok(csin(z)))),
    f("IMCOS", 1, 1, V, |c, a| im1(c, a, |z| Ok(ccos(z)))),
    f("IMTAN", 1, 1, V, |c, a| im1(c, a, |z| cdiv(csin(z), ccos(z)))),
    f("IMCOT", 1, 1, V, |c, a| im1(c, a, |z| cdiv(ccos(z), csin(z)))),
    f("IMSEC", 1, 1, V, |c, a| im1(c, a, |z| cdiv((1.0, 0.0), ccos(z)))),
    f("IMCSC", 1, 1, V, |c, a| im1(c, a, |z| cdiv((1.0, 0.0), csin(z)))),
    f("IMSINH", 1, 1, V, |c, a| im1(c, a, |z| Ok(csinh(z)))),
    f("IMCOSH", 1, 1, V, |c, a| im1(c, a, |z| Ok(ccosh(z)))),
    f("IMSECH", 1, 1, V, |c, a| im1(c, a, |z| cdiv((1.0, 0.0), ccosh(z)))),
    f("IMCSCH", 1, 1, V, |c, a| im1(c, a, |z| cdiv((1.0, 0.0), csinh(z)))),
    f("IMPOWER", 2, 2, V, |c, a| {
        let (z, s) = try_v!(complex_arg(c, &a[0]));
        let p = try_v!(num_arg(c, a, 1));
        let r = z.0.mhypot(z.1);
        if r == 0.0 {
            if p <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            return Value::from(fmt_complex(0.0, 0.0, s));
        }
        let t = z.1.matan2(z.0);
        let rp = r.mpow(p);
        Value::from(fmt_complex(rp * (p * t).mcos(), rp * (p * t).msin(), s))
    }),
    f("IMDIV", 2, 2, V, |c, a| im2(c, a, cdiv)),
    f("IMSUB", 2, 2, V, |c, a| im2(c, a, |x, y| Ok((x.0 - y.0, x.1 - y.1)))),
    f("IMSUM", 1, MANY, R, |c, a| imfold(c, a, (0.0, 0.0), |x, y| (x.0 + y.0, x.1 + y.1))),
    f("IMPRODUCT", 1, MANY, R, |c, a| imfold(c, a, (1.0, 0.0), cmul)),
];

// ---------------------------------------------------------------------------
// 진법
// ---------------------------------------------------------------------------

/// 진법 인수 글 (숫자는 그대로 글로)
fn base_text(ctx: &mut Ctx, v: &Value) -> Result<String, ErrorKind> {
    match v {
        Value::Number(n) => {
            if n.fract() != 0.0 || *n < 0.0 {
                return Err(ErrorKind::Num);
            }
            Ok(format!("{}", *n as u64))
        }
        Value::Bool(_) => Err(ErrorKind::Value),
        Value::Empty => Ok("0".into()),
        other => Ok(ctx.text(other)?.trim().to_string()),
    }
}

/// 2·8·16 진 글 → 정수 (10자리 2의 보수)
fn parse_radix(s: &str, radix: u32) -> Result<i64, ErrorKind> {
    if s.is_empty() {
        return Ok(0);
    }
    if s.chars().count() > 10 {
        return Err(ErrorKind::Num);
    }
    let mut v: i64 = 0;
    for ch in s.chars() {
        let d = ch.to_digit(radix).ok_or(ErrorKind::Num)? as i64;
        v = v * radix as i64 + d;
    }
    let bits = match radix {
        2 => 10,
        8 => 30,
        _ => 40,
    };
    if s.chars().count() == 10 && v >= 1i64 << (bits - 1) {
        v -= 1i64 << bits;
    }
    Ok(v)
}

/// 정수 → 2·8·16 진 글 (음수는 10자리 2의 보수)
fn fmt_radix(v: i64, radix: u32, places: Option<f64>) -> Result<String, ErrorKind> {
    let (bits, lo, hi) = match radix {
        2 => (10, -512i64, 511i64),
        8 => (30, -(1i64 << 29), (1i64 << 29) - 1),
        _ => (40, -(1i64 << 39), (1i64 << 39) - 1),
    };
    if v < lo || v > hi {
        return Err(ErrorKind::Num);
    }
    if v < 0 {
        let u = (v + (1i64 << bits)) as u64;
        return Ok(super::math::to_radix(u, radix));
    }
    let s = super::math::to_radix(v as u64, radix);
    match places {
        None => Ok(s),
        Some(p) => {
            let p = p.trunc();
            if p < 0.0 || p > 10.0 || (s.len() as f64) > p {
                return Err(ErrorKind::Num);
            }
            Ok(format!("{:0>width$}", s, width = p as usize))
        }
    }
}

fn from_base(ctx: &mut Ctx, a: &[Value], radix: u32, to: Option<u32>) -> Value {
    let s = try_v!(base_text(ctx, &a[0]));
    let v = try_v!(parse_radix(&s, radix));
    match to {
        None => Value::Number(v as f64),
        Some(r) => {
            let places = match opt(a, 1) {
                Some(p) => Some(try_v!(ctx.num(p))),
                None => None,
            };
            match fmt_radix(v, r, places) {
                Ok(s) => Value::from(s),
                Err(e) => Value::Error(e),
            }
        }
    }
}

fn dec2(ctx: &mut Ctx, a: &[Value], radix: u32) -> Value {
    let x = try_v!(num_arg(ctx, a, 0)).trunc();
    let places = match opt(a, 1) {
        Some(p) => Some(try_v!(ctx.num(p))),
        None => None,
    };
    if x.abs() > 1e15 {
        return Value::Error(ErrorKind::Num);
    }
    match fmt_radix(x as i64, radix, places) {
        Ok(s) => Value::from(s),
        Err(e) => Value::Error(e),
    }
}

/// 비트 함수 인수: 소수점 아래를 버린다. BITAND 만 양의 소수 부분이 있으면 #NUM! (BITAND(0.5,1),
/// BITAND(-0.5,1) = 0, BITOR(1.5,1) = 1, BITXOR(3.5,1) = 2). 버린 값은 0 이상 2^48 이하 (자리 이동
/// 함수는 2^48 미만) — 대조 실험 engineering·misc3·misc4
fn bit_arg(ctx: &mut Ctx, a: &[Value], i: usize, strict: bool, limit: f64) -> Result<u64, ErrorKind> {
    let x = num_arg(ctx, a, i)?;
    let n = x.trunc();
    if (strict && x - n > 0.0) || n < 0.0 || n > limit {
        return Err(ErrorKind::Num);
    }
    Ok(n as u64)
}

const BIT_MAX: f64 = 281_474_976_710_656.0;

/// 결과가 2^48 을 넘으면 #NUM! (BITOR(2^48,1); BITAND(2^48,2^48) = 2^48 은 된다)
fn bit(ctx: &mut Ctx, a: &[Value], f: fn(u64, u64) -> u64, strict: bool) -> Value {
    let x = try_v!(bit_arg(ctx, a, 0, strict, BIT_MAX));
    let y = try_v!(bit_arg(ctx, a, 1, strict, BIT_MAX));
    let r = f(x, y);
    if r > 281_474_976_710_656 {
        return Value::Error(ErrorKind::Num);
    }
    Value::Number(r as f64)
}

fn shift(ctx: &mut Ctx, a: &[Value], left: bool) -> Value {
    let x = try_v!(bit_arg(ctx, a, 0, false, BIT_MAX - 1.0));
    let s = try_v!(num_arg(ctx, a, 1)).trunc();
    if s.abs() > 53.0 {
        return Value::Error(ErrorKind::Num);
    }
    let s = if left { s } else { -s };
    let r = if s >= 0.0 {
        (x as f64) * 2f64.mpowi(s as i32)
    } else {
        (x >> (-s) as u32) as f64
    };
    if r >= 281_474_976_710_656.0 {
        return Value::Error(ErrorKind::Num);
    }
    Value::Number(r)
}

// ---------------------------------------------------------------------------
// 복소수
// ---------------------------------------------------------------------------

type C = (f64, f64);

/// "a+bi" 글 → 복소수와 접미사
pub fn parse_complex(s: &str) -> Option<(C, char)> {
    let s = s.trim();
    if s.is_empty() {
        return Some(((0.0, 0.0), 'i'));
    }
    let suffix = if s.ends_with('i') {
        'i'
    } else if s.ends_with('j') {
        'j'
    } else {
        // 실수만
        return value::parse_number(s, false)
            .filter(|_| s.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
            .map(|v| ((v, 0.0), 'i'));
    };
    let body = &s[..s.len() - 1];
    // 허수부 시작: 마지막 +/- (지수 표기의 부호는 건너뛴다)
    let bytes = body.as_bytes();
    let mut split = None;
    for i in (0..bytes.len()).rev() {
        if (bytes[i] == b'+' || bytes[i] == b'-') && !(i > 0 && (bytes[i - 1] == b'e' || bytes[i - 1] == b'E')) {
            split = Some(i);
            break;
        }
    }
    let (re_s, im_s) = match split {
        Some(0) | None => ("", body),
        Some(i) => (&body[..i], &body[i..]),
    };
    let num_ok = |t: &str| t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    let re = if re_s.is_empty() {
        0.0
    } else {
        if !num_ok(re_s) {
            return None;
        }
        re_s.parse::<f64>().ok()?
    };
    let im = match im_s {
        "" | "+" => 1.0,
        "-" => -1.0,
        t => {
            if !num_ok(t) {
                return None;
            }
            t.parse::<f64>().ok()?
        }
    };
    Some(((re, im), suffix))
}

fn complex_arg(ctx: &mut Ctx, v: &Value) -> Result<(C, char), ErrorKind> {
    match v {
        Value::Number(n) => Ok(((*n, 0.0), 'i')),
        Value::Bool(_) => Err(ErrorKind::Value),
        Value::Empty => Ok(((0.0, 0.0), 'i')),
        other => {
            let t = ctx.text(other)?;
            parse_complex(&t).ok_or(ErrorKind::Num)
        }
    }
}

fn clean(x: f64) -> f64 {
    if x.abs() < 1e-300 { 0.0 } else { x }
}

/// 복소수 → 글 (유효숫자 15자리)
pub fn fmt_complex(re: f64, im: f64, suffix: char) -> String {
    let re = clean(value::round15(re));
    let im = clean(value::round15(im));
    let number_to_text = value::number_to_text_rounded;
    if im == 0.0 {
        return number_to_text(re);
    }
    let im_s = if im == 1.0 {
        String::new()
    } else if im == -1.0 {
        "-".into()
    } else {
        number_to_text(im)
    };
    if re == 0.0 {
        return format!("{im_s}{suffix}");
    }
    let sign = if im > 0.0 { "+" } else { "" };
    format!("{}{sign}{im_s}{suffix}", number_to_text(re))
}

fn im1(ctx: &mut Ctx, a: &[Value], f: impl Fn(C) -> Result<C, ErrorKind>) -> Value {
    let (z, s) = try_v!(complex_arg(ctx, &a[0]));
    match f(z) {
        Ok(r) if r.0.is_finite() && r.1.is_finite() => Value::from(fmt_complex(r.0, r.1, s)),
        Ok(_) => Value::Error(ErrorKind::Num),
        Err(e) => Value::Error(e),
    }
}

fn im1n(ctx: &mut Ctx, a: &[Value], f: impl Fn(C) -> Result<f64, ErrorKind>) -> Value {
    let (z, _) = try_v!(complex_arg(ctx, &a[0]));
    value::num_r(f(z))
}

fn im2(ctx: &mut Ctx, a: &[Value], f: impl Fn(C, C) -> Result<C, ErrorKind>) -> Value {
    let (x, s1) = try_v!(complex_arg(ctx, &a[0]));
    let (y, s2) = try_v!(complex_arg(ctx, &a[1]));
    if s1 != s2 && !(x.1 == 0.0 || y.1 == 0.0) {
        return Value::Error(ErrorKind::Value);
    }
    let s = if x.1 != 0.0 { s1 } else { s2 };
    match f(x, y) {
        Ok(r) if r.0.is_finite() && r.1.is_finite() => Value::from(fmt_complex(r.0, r.1, s)),
        Ok(_) => Value::Error(ErrorKind::Num),
        Err(e) => Value::Error(e),
    }
}

fn imfold(ctx: &mut Ctx, a: &[Value], init: C, f: fn(C, C) -> C) -> Value {
    let mut acc = init;
    let mut suf = 'i';
    let mut seen_suffix: Option<char> = None;
    for v in a {
        for x in flatten(ctx, v) {
            if matches!(v, Value::Ref(_)) && x.is_empty() {
                continue;
            }
            let (z, s) = try_v!(complex_arg(ctx, &x));
            if z.1 != 0.0 {
                if let Some(p) = seen_suffix
                    && p != s
                {
                    return Value::Error(ErrorKind::Value);
                }
                seen_suffix = Some(s);
                suf = s;
            }
            acc = f(acc, z);
        }
    }
    Value::from(fmt_complex(acc.0, acc.1, suf))
}

fn cmul(x: C, y: C) -> C {
    (x.0 * y.0 - x.1 * y.1, x.0 * y.1 + x.1 * y.0)
}

fn cdiv(x: C, y: C) -> Result<C, ErrorKind> {
    let d = y.0 * y.0 + y.1 * y.1;
    if d == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(((x.0 * y.0 + x.1 * y.1) / d, (x.1 * y.0 - x.0 * y.1) / d))
}

fn cexp(z: C) -> C {
    let e = z.0.mexp();
    (e * z.1.mcos(), e * z.1.msin())
}

fn cln(z: C) -> Result<C, ErrorKind> {
    if z.0 == 0.0 && z.1 == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok((z.0.mhypot(z.1).mln(), z.1.matan2(z.0)))
}

fn csin(z: C) -> C {
    (z.0.msin() * z.1.mcosh(), z.0.mcos() * z.1.msinh())
}

fn ccos(z: C) -> C {
    (z.0.mcos() * z.1.mcosh(), -z.0.msin() * z.1.msinh())
}

fn csinh(z: C) -> C {
    (z.0.msinh() * z.1.mcos(), z.0.mcosh() * z.1.msin())
}

fn ccosh(z: C) -> C {
    (z.0.mcosh() * z.1.mcos(), z.0.msinh() * z.1.msin())
}

// ---------------------------------------------------------------------------
// 베셀 함수
// ---------------------------------------------------------------------------

fn erf_nonneg(x: f64, f: fn(f64) -> f64) -> Value {
    if x < 0.0 { Value::Error(ErrorKind::Num) } else { num(f(x)) }
}

fn bessel(ctx: &mut Ctx, a: &[Value], kind: char) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let n = try_v!(num_arg(ctx, a, 1)).trunc();
    if n < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let n = n as i32;
    let r = match kind {
        'J' => bessel_j(n, x),
        // BESSELI·BESSELK 는 한셀의 근사 (hcnum)
        'I' => super::hcnum::bessel_i(n, x),
        'Y' => {
            if x <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            bessel_y(n, x)
        }
        _ => {
            if x <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            super::hcnum::bessel_k(n, x)
        }
    };
    num(r)
}

/// J_n(x): 밀러의 역방향 점화식 (정규화 J0 + 2ΣJ_2k = 1)
pub fn bessel_j(n: i32, x: f64) -> f64 {
    if x == 0.0 {
        return if n == 0 { 1.0 } else { 0.0 };
    }
    let ax = x.abs();
    let start = {
        let m = (n.max(ax as i32) as f64 + 30.0 + (40.0 * n.max(ax as i32) as f64).sqrt()) as i32;
        m + m % 2
    };
    let mut jp1 = 0.0f64;
    let mut j = 1e-300f64;
    let mut sum = 0.0;
    let mut result = 0.0;
    let mut k = start;
    while k > 0 {
        let jm1 = 2.0 * k as f64 / ax * j - jp1;
        jp1 = j;
        j = jm1;
        // j 는 이제 J_{k-1}
        if j.abs() > 1e250 {
            j *= 1e-250;
            jp1 *= 1e-250;
            result *= 1e-250;
            sum *= 1e-250;
        }
        if k - 1 == n {
            result = j;
        }
        if (k - 1) % 2 == 0 && k - 1 > 0 {
            sum += 2.0 * j;
        }
        k -= 1;
    }
    sum += j;
    let mut r = result / sum;
    if x < 0.0 && n % 2 == 1 {
        r = -r;
    }
    r
}

/// I_n(x): 급수 (모든 항이 양수)
pub fn bessel_i(n: i32, x: f64) -> f64 {
    let half = x / 2.0;
    let mut term = 1.0;
    for k in 1..=n {
        term *= half / k as f64;
    }
    let mut sum = term;
    let q = half * half;
    let mut k = 1.0;
    loop {
        term *= q / (k * (k + n as f64));
        sum += term;
        if term.abs() < sum.abs() * 1e-17 || k > 1000.0 {
            break;
        }
        k += 1.0;
    }
    sum
}

const EULER: f64 = std::f64::consts::EULER_GAMMA;

/// Y0, Y1 (노이만 급수: J 값들로)
fn bessel_y01(x: f64) -> (f64, f64) {
    use std::f64::consts::PI;
    let j0 = bessel_j(0, x);
    let j1 = bessel_j(1, x);
    if x > 25.0 {
        // 점근 전개
        let y0 = asymptotic_y(0.0, x);
        let y1 = asymptotic_y(1.0, x);
        return (y0, y1);
    }
    let lnx = (x / 2.0).mln() + EULER;
    // Y0 = 2/π [ (ln(x/2)+γ) J0 − 2 Σ (−1)^k J_{2k}/k ]
    let mut s = 0.0;
    let mut k = 1;
    loop {
        let t = bessel_j(2 * k, x) / k as f64;
        s += if k % 2 == 0 { t } else { -t };
        if t.abs() < 1e-18 && (2 * k) as f64 > x {
            break;
        }
        k += 1;
        if k > 400 {
            break;
        }
    }
    let y0 = 2.0 / PI * (lnx * j0 - 2.0 * s);
    // 론스키안 J1 Y0 − J0 Y1 = 2/(πx)
    let y1 = if j0.abs() > 1e-3 {
        (j1 * y0 - 2.0 / (PI * x)) / j0
    } else {
        // Y1 = 2/π [ (ln(x/2)+γ) J1 − J0/x … ] 대신 수치 미분: Y1 = −Y0'
        let h = 1e-5 * x.max(1.0);
        let f = |t: f64| {
            let lt = (t / 2.0).mln() + EULER;
            let mut s = 0.0;
            let mut k = 1;
            loop {
                let v = bessel_j(2 * k, t) / k as f64;
                s += if k % 2 == 0 { v } else { -v };
                if v.abs() < 1e-18 && (2 * k) as f64 > t {
                    break;
                }
                k += 1;
                if k > 400 {
                    break;
                }
            }
            2.0 / PI * (lt * bessel_j(0, t) - 2.0 * s)
        };
        -(f(x + h) - f(x - h)) / (2.0 * h)
    };
    (y0, y1)
}

/// 큰 x 의 Y_ν (한켈 점근 전개)
fn asymptotic_y(nu: f64, x: f64) -> f64 {
    use std::f64::consts::PI;
    let mu = 4.0 * nu * nu;
    let mut p = 1.0;
    let mut q = 0.0;
    let mut term = 1.0;
    let mut k: f64 = 1.0;
    loop {
        term *= (mu - (2.0 * k - 1.0).mpowi(2)) / (k * 8.0 * x);
        if term.abs() < 1e-17 || k > 60.0 {
            break;
        }
        if (k as i32) % 2 == 1 {
            q += if ((k as i32) / 2) % 2 == 0 { term } else { -term };
        } else {
            p += if ((k as i32) / 2) % 2 == 1 { -term } else { term };
        }
        k += 1.0;
    }
    let chi = x - (nu / 2.0 + 0.25) * PI;
    (2.0 / (PI * x)).sqrt() * (p * chi.msin() + q * chi.mcos())
}

pub fn bessel_y(n: i32, x: f64) -> f64 {
    let (y0, y1) = bessel_y01(x);
    if n == 0 {
        return y0;
    }
    let (mut a, mut b) = (y0, y1);
    for k in 1..n {
        let c = 2.0 * k as f64 / x * b - a;
        a = b;
        b = c;
    }
    b
}

/// K0, K1 (작은 x 는 급수, 큰 x 는 적분 표현의 수치 적분)
fn bessel_k01(x: f64) -> (f64, f64) {
    if x <= 2.0 {
        // K0 = −(ln(x/2)+γ) I0 + Σ (x²/4)^k/(k!)² H_k
        let q = x * x / 4.0;
        let lnx = (x / 2.0).mln() + EULER;
        let mut term = 1.0;
        let mut h = 0.0;
        let mut s0 = 0.0;
        let mut k = 1.0;
        loop {
            term *= q / (k * k);
            h += 1.0 / k;
            s0 += term * h;
            if term * h < 1e-18 || k > 200.0 {
                break;
            }
            k += 1.0;
        }
        let k0 = -lnx * bessel_i(0, x) + s0;
        // K1 = (1/x) + ln(x/2)… : 론스키안 I0 K1 + I1 K0 = 1/x
        let k1 = (1.0 / x - bessel_i(1, x) * k0) / bessel_i(0, x);
        return (k0, k1);
    }
    // K_ν(x) = ∫_0^∞ exp(−x cosh t) cosh(νt) dt (사다리꼴 — 매끈한 함수라 아주 정확하다)
    let integ = |nu: f64| {
        let h = 0.02;
        let mut s = 0.5 * (-x).mexp();
        let mut t: f64 = h;
        loop {
            let v = (-x * t.mcosh()).mexp() * (nu * t).mcosh();
            s += v;
            if v < 1e-300 || t > 50.0 {
                break;
            }
            t += h;
        }
        s * h
    };
    (integ(0.0), integ(1.0))
}

pub fn bessel_k(n: i32, x: f64) -> f64 {
    let (k0, k1) = bessel_k01(x);
    if n == 0 {
        return k0;
    }
    let (mut a, mut b) = (k0, k1);
    for k in 1..n {
        let c = 2.0 * k as f64 / x * b + a;
        a = b;
        b = c;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_text() {
        assert_eq!(parse_complex("3+4i"), Some(((3.0, 4.0), 'i')));
        assert_eq!(parse_complex("-i"), Some(((0.0, -1.0), 'i')));
        assert_eq!(parse_complex("1.5e2-2j"), Some(((150.0, -2.0), 'j')));
        assert_eq!(fmt_complex(1.0, 1.0, 'i'), "1+i");
        assert_eq!(fmt_complex(0.0, -2.0, 'j'), "-2j");
        assert_eq!(fmt_complex(5.0, 0.0, 'i'), "5");
    }

    #[test]
    fn bessel_values() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12 * b.abs().max(1.0);
        assert!(close(bessel_j(0, 1.0), 0.765_197_686_557_966_6));
        assert!(close(bessel_j(1, 2.5), 0.497_094_102_464_274_4));
        assert!(close(bessel_i(1, 1.5), 0.981_666_428_577_908));
        assert!(close(bessel_y(0, 1.0), 0.088_256_964_215_676_96));
        assert!(close(bessel_k(0, 1.0), 0.421_024_438_240_708_3));
    }

    #[test]
    fn radix() {
        assert_eq!(parse_radix("1111111111", 2), Ok(-1));
        assert_eq!(fmt_radix(-1, 16, None).unwrap(), "FFFFFFFFFF");
        assert_eq!(fmt_radix(10, 2, Some(8.0)).unwrap(), "00001010");
    }
}
