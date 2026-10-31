// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 재무 함수.
//!
//! 금액 부호는 스프레드시트 관례대로 받는 돈 +, 내는 돈 −. 날짜 기준(basis): 0 미국 30/360,
//! 1 실제/실제, 2 실제/360, 3 실제/365, 4 유럽 30/360.

use super::date::{days_30_360, year_frac};
use super::{Agg, Arg, FnSpec, MANY, R, V, collect_numbers, f, flatten, num_arg, num_or, opt};
use crate::calc::date::{days_in_month, serial_from_ymd, ymd_from_serial};
use crate::calc::eval::Ctx;
use crate::calc::fm::Fm;
use crate::calc::value::{ErrorKind, Value, num, num_r};

const VR: &[Arg] = &[Arg::V, Arg::R];
const RV: &[Arg] = &[Arg::R, Arg::V];
const VRR: &[Arg] = &[Arg::V, Arg::R, Arg::R];
const RRV: &[Arg] = &[Arg::R, Arg::R, Arg::V];

pub const FNS: &[FnSpec] = &[
    f("PV", 3, 5, V, |c, a| {
        let (r, n, p) = try_v!(three(c, a));
        let fv = try_v!(num_or(c, a, 3, 0.0));
        let t = try_v!(pay_type(c, a, 4));
        // (1+r)^n = 0 (이율 −1) 은 #DIV/0! (대조 실험 financial)
        if r != 0.0 && (1.0 + r).mpow(n) == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num(pv(r, n, p, fv, t))
    }),
    f("FV", 3, 5, V, |c, a| {
        let (r, n, p) = try_v!(three(c, a));
        let pv_ = try_v!(num_or(c, a, 3, 0.0));
        let t = try_v!(pay_type(c, a, 4));
        num(fv(r, n, p, pv_, t))
    }),
    f("PMT", 3, 5, V, |c, a| {
        let (r, n, p) = try_v!(three(c, a));
        let fv_ = try_v!(num_or(c, a, 3, 0.0));
        let t = try_v!(pay_type(c, a, 4));
        num_r(pmt(r, n, p, fv_, t))
    }),
    f("NPER", 3, 5, V, |c, a| {
        let (r, p, pv_) = try_v!(three(c, a));
        let fv_ = try_v!(num_or(c, a, 3, 0.0));
        let t = try_v!(pay_type(c, a, 4));
        num_r(nper(r, p, pv_, fv_, t))
    }),
    f("RATE", 3, 6, V, |c, a| {
        let (n, p, pv_) = try_v!(three(c, a));
        let fv_ = try_v!(num_or(c, a, 3, 0.0));
        let t = try_v!(pay_type(c, a, 4));
        let g = try_v!(num_or(c, a, 5, 0.1));
        num_r(rate(n, p, pv_, fv_, t, g))
    }),
    f("IPMT", 4, 6, V, |c, a| ipmt_fn(c, a, true)),
    f("PPMT", 4, 6, V, |c, a| ipmt_fn(c, a, false)),
    f("CUMIPMT", 6, 6, V, |c, a| cum(c, a, true)),
    f("CUMPRINC", 6, 6, V, |c, a| cum(c, a, false)),
    f("ISPMT", 4, 4, V, |c, a| {
        let r = try_v!(num_arg(c, a, 0));
        let per = try_v!(num_arg(c, a, 1));
        let n = try_v!(num_arg(c, a, 2));
        let p = try_v!(num_arg(c, a, 3));
        if n == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num(p * r * (per / n - 1.0))
    }),
    // 현금 흐름이 없으면 0 (NPV(0.1), 대조 실험 financial)
    f("NPV", 1, MANY, VR, |c, a| {
        let r = try_v!(num_arg(c, a, 0));
        if r == -1.0 {
            return Value::Error(ErrorKind::Div0);
        }
        let v = try_v!(collect_numbers(c, &a[1..], Agg::Numbers));
        num(v
            .iter()
            .enumerate()
            .map(|(i, x)| x / (1.0 + r).mpowi(i as i32 + 1))
            .sum())
    }),
    f("XNPV", 3, 3, VRR, xnpv_fn),
    f("IRR", 1, 2, RV, irr),
    f("XIRR", 2, 3, &[Arg::R, Arg::R, Arg::V], xirr),
    f("MIRR", 3, 3, RV, mirr),
    f("SLN", 3, 3, V, |c, a| {
        let cost = try_v!(num_arg(c, a, 0));
        let salv = try_v!(num_arg(c, a, 1));
        let life = try_v!(num_arg(c, a, 2));
        if life == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num((cost - salv) / life)
    }),
    f("SYD", 4, 4, V, |c, a| {
        let cost = try_v!(num_arg(c, a, 0));
        let salv = try_v!(num_arg(c, a, 1));
        let life = try_v!(num_arg(c, a, 2));
        let per = try_v!(num_arg(c, a, 3));
        if life <= 0.0 || per <= 0.0 || per > life || salv < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((cost - salv) * (life - per + 1.0) * 2.0 / (life * (life + 1.0)))
    }),
    f("DB", 4, 5, V, db),
    f("DDB", 4, 5, V, ddb),
    f("VDB", 5, 7, V, vdb),
    f("EFFECT", 2, 2, V, |c, a| {
        let r = try_v!(num_arg(c, a, 0));
        let n = try_v!(num_arg(c, a, 1)).trunc();
        if r <= 0.0 || n < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((1.0 + r / n).mpow(n) - 1.0)
    }),
    f("NOMINAL", 2, 2, V, |c, a| {
        let r = try_v!(num_arg(c, a, 0));
        let n = try_v!(num_arg(c, a, 1)).trunc();
        if r <= 0.0 || n < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(n * ((1.0 + r).mpow(1.0 / n) - 1.0))
    }),
    f("FVSCHEDULE", 2, 2, &[Arg::V, Arg::A], |c, a| {
        let mut p = try_v!(num_arg(c, a, 0));
        for v in flatten(c, &a[1]) {
            match v {
                Value::Number(r) => p *= 1.0 + r,
                Value::Empty => {}
                Value::Error(e) => return Value::Error(e),
                _ => return Value::Error(ErrorKind::Value),
            }
        }
        num(p)
    }),
    f("DOLLARDE", 2, 2, V, |c, a| dollar_frac(c, a, true)),
    f("DOLLARFR", 2, 2, V, |c, a| dollar_frac(c, a, false)),
    f("PDURATION", 3, 3, V, |c, a| {
        let r = try_v!(num_arg(c, a, 0));
        let p = try_v!(num_arg(c, a, 1));
        let fv_ = try_v!(num_arg(c, a, 2));
        if r <= 0.0 || p <= 0.0 || fv_ <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((fv_.mln() - p.mln()) / (1.0 + r).mln())
    }),
    f("RRI", 3, 3, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0));
        let p = try_v!(num_arg(c, a, 1));
        let fv_ = try_v!(num_arg(c, a, 2));
        if n <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        if p == 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((fv_ / p).mpow(1.0 / n) - 1.0)
    }),
    // 증권
    f("TBILLPRICE", 3, 3, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let d = try_v!(num_arg(c, a, 2));
        let dsm = m - s;
        if d <= 0.0 || dsm > 366.0 {
            return Value::Error(ErrorKind::Num);
        }
        // 가격이 음수여도 그대로 (TBILLPRICE(45000,45100,4) = −11.11, 대조 실험 financial)
        num(100.0 * (1.0 - d * dsm / 360.0))
    }),
    f("TBILLYIELD", 3, 3, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let p = try_v!(num_arg(c, a, 2));
        let dsm = m - s;
        if p <= 0.0 || dsm > 366.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((100.0 - p) / p * 360.0 / dsm)
    }),
    f("TBILLEQ", 3, 3, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let d = try_v!(num_arg(c, a, 2));
        let dsm = m - s;
        if d <= 0.0 || dsm > 366.0 {
            return Value::Error(ErrorKind::Num);
        }
        // 만기가 반년을 넘어도 365·d/(360 − d·일수) 하나 (TBILLEQ(45000,45300,0.05), 대조 실험 financial)
        num(365.0 * d / (360.0 - d * dsm))
    }),
    f("DISC", 4, 5, V, |c, a| {
        try_v!(numbers_only(a));
        let (s, m) = try_v!(sm(c, a));
        let pr = try_v!(num_arg(c, a, 2));
        let red = try_v!(num_arg(c, a, 3));
        let b = try_v!(basis(c, a, 4));
        if pr <= 0.0 || red <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let yf = try_v!(year_frac(s, m, b, c.date1904()));
        num((red - pr) / red / yf)
    }),
    f("INTRATE", 4, 5, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let inv = try_v!(num_arg(c, a, 2));
        let red = try_v!(num_arg(c, a, 3));
        let b = try_v!(basis(c, a, 4));
        if inv <= 0.0 || red <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let yf = try_v!(year_frac(s, m, b, c.date1904()));
        num((red - inv) / inv / yf)
    }),
    f("RECEIVED", 4, 5, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let inv = try_v!(num_arg(c, a, 2));
        let d = try_v!(num_arg(c, a, 3));
        let b = try_v!(basis(c, a, 4));
        if inv <= 0.0 || d <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let yf = try_v!(year_frac(s, m, b, c.date1904()));
        // 분모가 음수여도 그대로 (RECEIVED(…,20) = −250000, 대조 실험 financial)
        let den = 1.0 - d * yf;
        if den == 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(inv / den)
    }),
    f("PRICEDISC", 4, 5, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let d = try_v!(num_arg(c, a, 2));
        let red = try_v!(num_arg(c, a, 3));
        let b = try_v!(basis(c, a, 4));
        if d <= 0.0 || red <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let yf = try_v!(year_frac(s, m, b, c.date1904()));
        num(red - d * red * yf)
    }),
    f("YIELDDISC", 4, 5, V, |c, a| {
        let (s, m) = try_v!(sm(c, a));
        let pr = try_v!(num_arg(c, a, 2));
        let red = try_v!(num_arg(c, a, 3));
        let b = try_v!(basis(c, a, 4));
        if pr <= 0.0 || red <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let yf = try_v!(year_frac(s, m, b, c.date1904()));
        num((red - pr) / pr / yf)
    }),
    f("PRICEMAT", 5, 6, V, pricemat),
    f("YIELDMAT", 5, 6, V, yieldmat),
    f("ACCRINTM", 3, 5, V, |c, a| {
        let iss = try_v!(date_arg(c, a, 0));
        let set = try_v!(date_arg(c, a, 1));
        let r = try_v!(num_arg(c, a, 2));
        let par = try_v!(num_or(c, a, 3, 1000.0));
        let b = try_v!(basis(c, a, 4));
        // 발행일과 만기일이 같으면 0 (ACCRINTM(1,1,1,1), 대조 실험 financial)
        if r <= 0.0 || par <= 0.0 || iss > set {
            return Value::Error(ErrorKind::Num);
        }
        if iss == set {
            return Value::Number(0.0);
        }
        let yf = try_v!(year_frac(iss, set, b, c.date1904()));
        num(par * r * yf)
    }),
    f("ACCRINT", 6, 8, V, accrint),
    f("COUPDAYBS", 3, 4, V, |c, a| coup(c, a, 0)),
    f("COUPDAYS", 3, 4, V, |c, a| coup(c, a, 1)),
    f("COUPDAYSNC", 3, 4, V, |c, a| coup(c, a, 2)),
    f("COUPNCD", 3, 4, V, |c, a| coup(c, a, 3)),
    f("COUPNUM", 3, 4, V, |c, a| coup(c, a, 4)),
    f("COUPPCD", 3, 4, V, |c, a| coup(c, a, 5)),
    f("PRICE", 6, 7, V, price_fn),
    f("YIELD", 6, 7, V, yield_fn),
    f("DURATION", 5, 6, V, |c, a| duration_fn(c, a, false)),
    f("MDURATION", 5, 6, V, |c, a| duration_fn(c, a, true)),
    f("AMORLINC", 6, 7, V, |c, a| amor(c, a, false)),
    f("AMORDEGRC", 6, 7, V, |c, a| amor(c, a, true)),
    f("ODDFPRICE", 8, 9, V, |c, a| odd_first(c, a, false)),
    f("ODDFYIELD", 8, 9, V, |c, a| odd_first(c, a, true)),
    f("ODDLPRICE", 7, 8, V, |c, a| odd_last(c, a, false)),
    f("ODDLYIELD", 7, 8, V, |c, a| odd_last(c, a, true)),
];

#[allow(dead_code)]
const _UNUSED: (&[Arg], &[Arg]) = (R, RRV);

fn three(ctx: &mut Ctx, a: &[Value]) -> Result<(f64, f64, f64), ErrorKind> {
    Ok((
        num_arg(ctx, a, 0)?,
        num_arg(ctx, a, 1)?,
        num_arg(ctx, a, 2)?,
    ))
}

fn pay_type(ctx: &mut Ctx, a: &[Value], i: usize) -> Result<f64, ErrorKind> {
    let t = num_or(ctx, a, i, 0.0)?;
    Ok(if t != 0.0 { 1.0 } else { 0.0 })
}

pub fn pv(r: f64, n: f64, p: f64, fv: f64, t: f64) -> f64 {
    if r == 0.0 {
        return -(fv + p * n);
    }
    let g = (1.0 + r).mpow(n);
    -(fv + p * (1.0 + r * t) * (g - 1.0) / r) / g
}

pub fn fv(r: f64, n: f64, p: f64, pv: f64, t: f64) -> f64 {
    if r == 0.0 {
        return -(pv + p * n);
    }
    let g = (1.0 + r).mpow(n);
    // 지급 항의 괄호: ((g−1)/r) 를 먼저 — 이 차례로 FV 8 경우가 끝자리까지 같다 (대조 실험 financial·cfin)
    -(pv * g + p * (1.0 + r * t) * ((g - 1.0) / r))
}

/// 정기 지급액: −(pv + fv/g)·r / ((1 + rt)(1 − 1/g)), g = (1+r)^n — 아주 작은 이율에서 한셀과 같은 끝자리
/// (PMT(1E-10,10,1000) = −99.9999917259636, 이율 −1 은 #NUM!, 대조 실험 financial)
pub fn pmt(r: f64, n: f64, pv: f64, fv: f64, t: f64) -> Result<f64, ErrorKind> {
    if n == 0.0 {
        return Err(ErrorKind::Num);
    }
    if r == 0.0 {
        return Ok(-(pv + fv) / n);
    }
    let g = (1.0 + r).mpow(n);
    if g == 0.0 {
        return Err(ErrorKind::Num);
    }
    let den = (1.0 + r * t) * (1.0 - 1.0 / g);
    if den == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(-(pv + fv / g) * r / den)
}

fn nper(r: f64, p: f64, pv: f64, fv: f64, t: f64) -> Result<f64, ErrorKind> {
    if r == 0.0 {
        if p == 0.0 {
            return Err(ErrorKind::Div0);
        }
        return Ok(-(pv + fv) / p);
    }
    let a = p * (1.0 + r * t) - fv * r;
    let b = p * (1.0 + r * t) + pv * r;
    if b == 0.0 || a / b <= 0.0 || r <= -1.0 {
        return Err(ErrorKind::Num);
    }
    Ok((a / b).mln() / (1.0 + r).mln())
}

/// 미지의 이율 (뉴턴 방법, 수렴할 때까지)
fn rate(n: f64, p: f64, pv: f64, fv: f64, t: f64, guess: f64) -> Result<f64, ErrorKind> {
    let f = |r: f64| -> f64 {
        if r.abs() < 1e-12 {
            return pv + p * n + fv;
        }
        let g = (1.0 + r).mpow(n);
        pv * g + p * (1.0 + r * t) * (g - 1.0) / r + fv
    };
    let df = |r: f64| -> f64 {
        let h = 1e-7 * (1.0 + r.abs());
        (f(r + h) - f(r - h)) / (2.0 * h)
    };
    let mut r = guess;
    for _ in 0..200 {
        let y = f(r);
        let d = df(r);
        if d == 0.0 || !d.is_finite() {
            return Err(ErrorKind::Num);
        }
        let nr = r - y / d;
        if !nr.is_finite() || nr <= -1.0 {
            return Err(ErrorKind::Num);
        }
        if (nr - r).abs() < 1e-15 * (1.0 + nr.abs()) {
            return Ok(nr);
        }
        r = nr;
    }
    if f(r).abs() < 1e-7 {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}

pub fn ipmt(r: f64, per: f64, n: f64, pv: f64, fv_: f64, t: f64) -> Result<f64, ErrorKind> {
    let p = pmt(r, n, pv, fv_, t)?;
    if per == 1.0 && t == 1.0 {
        return Ok(0.0);
    }
    // 미래 가치를 −(pv·g + 지급·(1+rt)·((g−1)/r)) 차례로 (FV 와 다른 괄호 — 기간이 횟수를 넘을 때의 잔차
    // IPMT(0.05,11,10,1000) = −1.1368683772161604E-14 가 이 차례로 같다, 대조 실험 cfin·cmisc)
    let n1 = per - 1.0;
    let base = if r == 0.0 {
        fv(r, n1, p, pv, t)
    } else {
        let g = (1.0 + r).mpow(n1);
        -(pv * g + p * (1.0 + r * t) * ((g - 1.0) / r))
    };
    let mut i = base * r;
    if t == 1.0 {
        i /= 1.0 + r;
    }
    Ok(i)
}

fn ipmt_fn(ctx: &mut Ctx, a: &[Value], interest: bool) -> Value {
    let r = try_v!(num_arg(ctx, a, 0));
    let per = try_v!(num_arg(ctx, a, 1));
    let n = try_v!(num_arg(ctx, a, 2));
    let p = try_v!(num_arg(ctx, a, 3));
    let fv_ = try_v!(num_or(ctx, a, 4, 0.0));
    let t = try_v!(pay_type(ctx, a, 5));
    // IPMT 는 기간이 횟수를 넘어도 계산한다 (IPMT(0.1,4,3,8000) = 0), PPMT 는 #NUM! (대조 실험 financial)
    if per < 1.0 || (per > n && !interest) {
        return Value::Error(ErrorKind::Num);
    }
    let i = try_v!(ipmt(r, per, n, p, fv_, t));
    if interest {
        num(i)
    } else {
        num(try_v!(pmt(r, n, p, fv_, t)) - i)
    }
}

fn cum(ctx: &mut Ctx, a: &[Value], interest: bool) -> Value {
    let r = try_v!(num_arg(ctx, a, 0));
    let n = try_v!(num_arg(ctx, a, 1)).trunc();
    let p = try_v!(num_arg(ctx, a, 2));
    let s = try_v!(num_arg(ctx, a, 3)).trunc();
    let e = try_v!(num_arg(ctx, a, 4)).trunc();
    let t = try_v!(num_arg(ctx, a, 5)).trunc();
    if r <= 0.0 || n <= 0.0 || p <= 0.0 || s < 1.0 || e < s || e > n || !(t == 0.0 || t == 1.0) {
        return Value::Error(ErrorKind::Num);
    }
    let pm = try_v!(pmt(r, n, p, 0.0, t));
    let mut total = 0.0;
    let mut k = s;
    while k <= e {
        let i = try_v!(ipmt(r, k, n, p, 0.0, t));
        total += if interest { i } else { pm - i };
        k += 1.0;
    }
    num(total)
}

fn xnpv(r: f64, v: &[f64], d: &[f64]) -> f64 {
    v.iter()
        .zip(d)
        .map(|(x, di)| x / (1.0 + r).mpow((di - d[0]) / 365.0))
        .sum()
}

fn cash_and_dates(
    ctx: &mut Ctx,
    va: &Value,
    da: &Value,
) -> Result<(Vec<f64>, Vec<f64>), ErrorKind> {
    let vs = flatten(ctx, va);
    let ds = flatten(ctx, da);
    if vs.len() != ds.len() {
        return Err(ErrorKind::Num);
    }
    let mut v = Vec::new();
    let mut d = Vec::new();
    for (x, y) in vs.iter().zip(&ds) {
        let x = match x {
            Value::Number(n) => *n,
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        };
        let y = match y {
            Value::Number(n) => n.floor(),
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        };
        v.push(x);
        d.push(y);
    }
    if v.is_empty() || d.iter().any(|&x| x < d[0]) {
        return Err(ErrorKind::Num);
    }
    Ok((v, d))
}

fn xnpv_fn(ctx: &mut Ctx, a: &[Value]) -> Value {
    let r = try_v!(num_arg(ctx, a, 0));
    let (v, d) = try_v!(cash_and_dates(ctx, &a[1], &a[2]));
    num(xnpv(r, &v, &d))
}

fn irr(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v: Vec<f64> = flatten(ctx, &a[0])
        .into_iter()
        .filter_map(|x| {
            if let Value::Number(n) = x {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    let g = try_v!(num_or(ctx, a, 1, 0.1));
    num_r(irr_bisect(&v, g))
}

/// 한셀의 IRR (대조 실험 x7–x10, 127 경우 중 115 끝자리까지 같음 — 나머지는 겹근·0 근의 잡음):
/// 앞의 0 을 뺀 현금 흐름의 NPV = Σ v_i / d (d ← d·(1+r)). 추정값 g 에서 NPV 가 양수면 위로
/// r ← (1+r)·2 − 1 을 17번까지 (IRR({-1,120001}) 은 찾고 {-1,250001} 은 #NUM!), 아니면 아래로
/// r ← (r − 1)/2 를 −1 에 닿기 전까지 부호가 바뀌는 점을 찾아 구간을 잡고 이분법. 중점의 |NPV| ≤ 1E-16
/// 이거나 중점이 끝점과 같아지면(이웃한 두 수의 짝수 쪽) 그 중점. 그래서 결과가 추정값과 무관하고,
/// 첫 현금 흐름이 양수면 어느 쪽으로도 부호가 바뀌지 않아 #NUM! (IRR({1,2.5,0,-4}) — 한셀 고유).
/// 근이 둘이면 추정값 쪽에서 찾는다 (IRR({-100,230,-132},0.25) = #NUM!, 0.15 → 0.2)
fn irr_bisect(v: &[f64], g: f64) -> Result<f64, ErrorKind> {
    let Some(first) = v.iter().position(|&x| x != 0.0) else {
        return Err(ErrorKind::Num);
    };
    let v = &v[first..];
    let npv = |r: f64| -> f64 {
        let mut s = 0.0;
        let mut d = 1.0;
        for &x in v {
            s += x / d;
            d *= 1.0 + r;
        }
        s
    };
    let (mut lo, mut hi);
    if npv(g) > 0.0 {
        let mut x = g;
        let mut found = false;
        for _ in 0..17 {
            x = (1.0 + x) * 2.0 - 1.0;
            if npv(x) < 0.0 {
                found = true;
                break;
            }
        }
        if !found {
            return Err(ErrorKind::Num);
        }
        (lo, hi) = (g, x);
    } else {
        let mut x = g;
        let mut found = false;
        for _ in 0..100 {
            x = (x - 1.0) / 2.0;
            if x <= -1.0 {
                break;
            }
            if npv(x) > 0.0 {
                found = true;
                break;
            }
        }
        if !found {
            return Err(ErrorKind::Num);
        }
        (lo, hi) = (x, g);
    }
    for _ in 0..3000 {
        let mid = (lo + hi) / 2.0;
        if mid == lo || mid == hi {
            return Ok(mid);
        }
        let y = npv(mid);
        if y.abs() <= 1e-16 {
            return Ok(mid);
        }
        if y > 0.0 {
            lo = mid;
        } else if y < 0.0 {
            hi = mid;
        } else {
            return Err(ErrorKind::Num);
        }
    }
    Err(ErrorKind::Num)
}

fn newton(mut r: f64, f: impl Fn(f64) -> f64, df: impl Fn(f64) -> f64) -> Result<f64, ErrorKind> {
    for _ in 0..200 {
        let y = f(r);
        let d = df(r);
        if d == 0.0 || !d.is_finite() {
            return Err(ErrorKind::Num);
        }
        let nr = r - y / d;
        if !nr.is_finite() || nr <= -1.0 {
            return Err(ErrorKind::Num);
        }
        if (nr - r).abs() < 1e-15 * (1.0 + nr.abs()) {
            return Ok(nr);
        }
        r = nr;
    }
    if f(r).abs() < 1e-6 {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}

fn xirr(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (v, d) = try_v!(cash_and_dates(ctx, &a[0], &a[1]));
    let g = try_v!(num_or(ctx, a, 2, 0.1));
    // 추정값 0 은 #NUM! (XIRR({-100,110},{40000,40365},0), 대조 실험 x8·x9 — 다섯 경우 모두)
    if g == 0.0 || !(v.iter().any(|&x| x > 0.0) && v.iter().any(|&x| x < 0.0)) {
        return Value::Error(ErrorKind::Num);
    }
    let f = |r: f64| xnpv(r, &v, &d);
    let df = |r: f64| -> f64 {
        v.iter()
            .zip(&d)
            .map(|(x, di)| {
                let t = (di - d[0]) / 365.0;
                -t * x / (1.0 + r).mpow(t + 1.0)
            })
            .sum()
    };
    num_r(newton(g, f, df))
}

fn mirr(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v: Vec<f64> = flatten(ctx, &a[0])
        .into_iter()
        .filter_map(|x| {
            if let Value::Number(n) = x {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    let fr = try_v!(num_arg(ctx, a, 1));
    let rr = try_v!(num_arg(ctx, a, 2));
    // 이율 −1 이하는 #NUM! (MIRR({-100,200},-1,0.1), 대조 실험 financial)
    if fr <= -1.0 || rr <= -1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let n = v.len() as f64;
    let mut npv_pos = 0.0;
    let mut npv_neg = 0.0;
    for (i, x) in v.iter().enumerate() {
        if *x > 0.0 {
            npv_pos += x / (1.0 + rr).mpowi(i as i32);
        } else {
            npv_neg += x / (1.0 + fr).mpowi(i as i32);
        }
    }
    if npv_neg == 0.0 || npv_pos == 0.0 || n < 2.0 {
        return Value::Error(ErrorKind::Div0);
    }
    num((-npv_pos * (1.0 + rr).mpow(n - 1.0) / npv_neg).mpow(1.0 / (n - 1.0)) - 1.0)
}

fn db(ctx: &mut Ctx, a: &[Value]) -> Value {
    let cost = try_v!(num_arg(ctx, a, 0));
    let salv = try_v!(num_arg(ctx, a, 1));
    // 수명은 소수 그대로, 기간은 버린다 (DB(1000000,100000,6.5,2) = 209196, 대조 실험 financial)
    let life = try_v!(num_arg(ctx, a, 2));
    let per = try_v!(num_arg(ctx, a, 3)).trunc();
    let month = try_v!(num_or(ctx, a, 4, 12.0)).trunc();
    if cost < 0.0
        || salv < 0.0
        || life <= 0.0
        || per <= 0.0
        || !(1.0..=12.0).contains(&month)
        // 기간은 수명을 넘을 수 없다. 첫 해가 열두 달이 아니면 (month < 12) 수명 + 1 까지
        // (DB(1000,100,3,4) = #NUM!, DB(1000,100,3,4,6) = 42.2, 대조 실험 conv3·conv4)
        || per > life + if month < 12.0 { 1.0 } else { 0.0 }
    {
        return Value::Error(ErrorKind::Num);
    }
    if cost == 0.0 {
        return Value::Number(0.0);
    }
    let rate = super::math::round_digits(
        1.0 - (salv / cost).mpow(1.0 / life),
        3.0,
        super::math::Rounding::HalfAway,
    );
    let mut total = 0.0;
    let mut dep = cost * rate * month / 12.0;
    if per == 1.0 {
        return num(dep);
    }
    total += dep;
    let mut p = 2.0;
    while p <= per {
        // 수명을 넘는 마지막 해는 남은 달수만큼 (DB(1000,100,2.5,3,6) = 83.7, 수명이 소수여도)
        if p > life {
            dep = (cost - total) * rate * (12.0 - month) / 12.0;
        } else {
            dep = (cost - total) * rate;
        }
        total += dep;
        p += 1.0;
    }
    num(dep)
}

/// 정수 기간의 상각액: 장부가 × 율 과 장부가 − 잔존가 가운데 작은 것 (잔존가가 더 크면 음수,
/// DDB(100,200,5,1) = −100, 대조 실험 financial)
fn ddb_period(cost: f64, salv: f64, life: f64, per: f64, factor: f64) -> f64 {
    let rate = (factor / life).min(1.0);
    let mut book = cost;
    let mut dep = 0.0;
    let mut p = 1.0;
    while p <= per {
        dep = (book * rate).min(book - salv);
        book -= dep;
        p += 1.0;
    }
    dep
}

/// t 기간 뒤 장부가: 정수 기간 장부가 사이를 직선으로 잇는다
fn ddb_book(cost: f64, salv: f64, life: f64, t: f64, factor: f64) -> f64 {
    let rate = (factor / life).min(1.0);
    let mut book = cost;
    let mut p = 1.0;
    while p <= t.floor() {
        book -= (book * rate).min(book - salv);
        p += 1.0;
    }
    let f = t - t.floor();
    if f == 0.0 {
        return book;
    }
    let next = book - (book * rate).min(book - salv);
    book + (next - book) * f
}

fn ddb(ctx: &mut Ctx, a: &[Value]) -> Value {
    let cost = try_v!(num_arg(ctx, a, 0));
    let salv = try_v!(num_arg(ctx, a, 1));
    let life = try_v!(num_arg(ctx, a, 2));
    let per = try_v!(num_arg(ctx, a, 3));
    let factor = try_v!(num_or(ctx, a, 4, 2.0));
    // 기간이 1 보다 작으면 #NUM!, 소수 기간은 직선으로 이은 장부가의 차
    // (DDB(2400,300,10,2.5) = 345.5999999999999, DDB(2400,300,10,0.5) = #NUM!, 대조 실험 financial)
    if cost < 0.0 || salv < 0.0 || life <= 0.0 || per < 1.0 || factor <= 0.0 || per > life {
        return Value::Error(ErrorKind::Num);
    }
    if per.fract() != 0.0 {
        return num(
            ddb_book(cost, salv, life, per - 1.0, factor) - ddb_book(cost, salv, life, per, factor)
        );
    }
    num(ddb_period(cost, salv, life, per, factor))
}

/// 가변 체감 상각 (VDB): 정액이 더 크면 정액으로 바꾼다
fn vdb(ctx: &mut Ctx, a: &[Value]) -> Value {
    let cost = try_v!(num_arg(ctx, a, 0));
    let salv = try_v!(num_arg(ctx, a, 1));
    let life = try_v!(num_arg(ctx, a, 2));
    let start = try_v!(num_arg(ctx, a, 3));
    let end = try_v!(num_arg(ctx, a, 4));
    let factor = try_v!(num_or(ctx, a, 5, 2.0));
    let no_switch = match opt(a, 6) {
        Some(v) => try_v!(ctx.boolean(v)),
        None => false,
    };
    if cost < 0.0
        || salv < 0.0
        || life <= 0.0
        || start < 0.0
        || end < start
        || end > life
        || factor <= 0.0
    {
        return Value::Error(ErrorKind::Num);
    }
    // 기간 단위로 나눠 합한다 (정수 경계마다)
    let dep_in = |t0: f64, t1: f64| -> f64 {
        // [t0, t1] 구간의 상각액 (체감 + 전환)
        let rate = factor / life;
        let mut total = 0.0;
        let mut book = cost;
        let mut t = 0.0;
        let mut switched = false;
        while t < t1 - 1e-12 {
            let seg_end = (t.floor() + 1.0).min(t1);
            let frac = seg_end - t;
            let ddb = book * rate;
            let remaining = life - t;
            let sl = if remaining > 0.0 {
                (book - salv) / remaining
            } else {
                0.0
            };
            let per_dep = if !no_switch && (switched || sl > ddb) {
                switched = true;
                sl
            } else {
                ddb
            };
            let mut d = per_dep * frac;
            if book - d < salv {
                d = (book - salv).max(0.0);
            }
            if seg_end > t0 + 1e-12 {
                let overlap = (seg_end - t.max(t0)) / frac;
                total += d * overlap;
            }
            book -= d;
            t = seg_end;
        }
        total
    };
    num(dep_in(start, end))
}

fn dollar_frac(ctx: &mut Ctx, a: &[Value], de: bool) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let f = try_v!(num_arg(ctx, a, 1)).trunc();
    if f < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    if f == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let int = x.trunc();
    let frac = x - int;
    // 10 의 거듭제곱은 적어도 10 (DOLLARDE(1.1,1) = 2.000000000000001, 대조 실험 financial)
    let p = 10f64.mpow(f.mlog10().ceil().max(1.0));
    if de {
        num(int + frac * p / f)
    } else {
        num(int + frac * f / p)
    }
}

// ---------------------------------------------------------------------------
// 증권
// ---------------------------------------------------------------------------

fn date_arg(ctx: &mut Ctx, a: &[Value], i: usize) -> Result<f64, ErrorKind> {
    let v = num_arg(ctx, a, i)?;
    if v < 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(v.floor())
}

/// (결제일, 만기일): 결제일 ≥ 만기일이면 #NUM!
fn sm(ctx: &mut Ctx, a: &[Value]) -> Result<(f64, f64), ErrorKind> {
    // 인수의 오류 값이 날짜 검사(결제일 ≥ 만기일 = #NUM!)보다 먼저 (PRICE(366,1,NA(),…) = #N/A, 대조 실험 conv10)
    if let Some(Value::Error(e)) = a.iter().find(|v| matches!(v, Value::Error(_))) {
        return Err(*e);
    }
    let s = date_arg(ctx, a, 0)?;
    let m = date_arg(ctx, a, 1)?;
    if s >= m {
        return Err(ErrorKind::Num);
    }
    Ok((s, m))
}

fn basis(ctx: &mut Ctx, a: &[Value], i: usize) -> Result<u8, ErrorKind> {
    let b = num_or(ctx, a, i, 0.0)?.trunc();
    if !(0.0..=4.0).contains(&b) {
        return Err(ErrorKind::Num);
    }
    Ok(b as u8)
}

fn freq(ctx: &mut Ctx, a: &[Value], i: usize) -> Result<f64, ErrorKind> {
    let f = num_arg(ctx, a, i)?.trunc();
    if !(f == 1.0 || f == 2.0 || f == 4.0) {
        return Err(ErrorKind::Num);
    }
    Ok(f)
}

/// 기준에 따른 두 날짜 사이 일수
fn day_count(s: f64, e: f64, b: u8, date1904: bool) -> f64 {
    match b {
        0 => days_30_360(s, e, true, date1904).unwrap_or(e - s),
        4 => days_30_360(s, e, false, date1904).unwrap_or(e - s),
        _ => e - s,
    }
}

/// 1년 일수 (기준 1 은 해당 연도)
fn year_basis(b: u8, s: f64, date1904: bool) -> f64 {
    match b {
        1 => {
            let y = ymd_from_serial(s, date1904).map(|x| x.0).unwrap_or(1900);
            if crate::calc::date::is_leap(y) {
                366.0
            } else {
                365.0
            }
        }
        3 => 365.0,
        _ => 360.0,
    }
}

/// 만기일에서 months 개월 앞의 이자 지급일 (말일 규칙)
fn coupon_date(maturity: f64, months_back: i64, date1904: bool) -> f64 {
    let (y, m, d) = ymd_from_serial(maturity, date1904).unwrap_or((1900, 1, 1));
    let eom = d == days_in_month(y, m);
    let total = y * 12 + (m as i64 - 1) - months_back;
    let ny = total.div_euclid(12);
    let nm = (total.rem_euclid(12) + 1) as u32;
    let dim = days_in_month(ny, nm);
    let nd = if eom { dim } else { d.min(dim) };
    serial_from_ymd(ny, nm as i64, nd as i64, date1904).unwrap_or(0.0)
}

/// (직전 이자 지급일, 다음 이자 지급일, 남은 지급 횟수)
fn coupon_dates(s: f64, m: f64, fr: f64, date1904: bool) -> (f64, f64, f64) {
    let step = (12.0 / fr) as i64;
    let mut k = 0i64;
    let mut ncd = m;
    loop {
        let prev = coupon_date(m, (k + 1) * step, date1904);
        if prev <= s {
            return (prev, ncd, (k + 1) as f64);
        }
        ncd = prev;
        k += 1;
        if k > 100_000 {
            return (prev, ncd, (k + 1) as f64);
        }
    }
}

/// 숫자만 받는 인수 확인: 글·논리·오류 값은 (오류 값도) #VALUE! — COUP* 와 DISC 는 날짜 글도 읽지 않는다
/// (COUPNCD("2024-03-15",…) = #VALUE!, COUPNCD(NA(),…) = #VALUE!, 대조 실험 conv8·conv9)
fn numbers_only(a: &[Value]) -> Result<(), ErrorKind> {
    if a.iter()
        .any(|v| matches!(v, Value::Text(_) | Value::Bool(_) | Value::Error(_)))
    {
        return Err(ErrorKind::Value);
    }
    Ok(())
}

fn coup(ctx: &mut Ctx, a: &[Value], what: u8) -> Value {
    try_v!(numbers_only(a));
    // 날짜는 1 이상 2958465 이하: 0 이하·결제일 ≥ 만기일은 #NUM!, 그다음 넘으면 #VALUE!
    let (s, m) = try_v!(sm(ctx, a));
    if s < 1.0 || m < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    if s >= 2_958_466.0 || m >= 2_958_466.0 {
        return Value::Error(ErrorKind::Value);
    }
    let fr = try_v!(freq(ctx, a, 2));
    let b = try_v!(basis(ctx, a, 3));
    let d1904 = ctx.date1904();
    let (pcd, ncd, n) = coupon_dates(s, m, fr, d1904);
    let e = coup_days(pcd, ncd, fr, b, d1904);
    match what {
        0 => num(day_count(pcd, s, b, d1904)),
        1 => num(e),
        2 => num(if b == 0 || b == 4 {
            e - day_count(pcd, s, b, d1904)
        } else {
            ncd - s
        }),
        3 => num(ncd),
        4 => num(n),
        _ => num(pcd),
    }
}

/// PRICE·YIELD 의 이자 기간 일수: 30/360 기준(0·4)은 360/fr, 나머지는 실제 일수 (기준 3 도,
/// PRICE(…,1,3) = 94.6725635950793, 대조 실험 financial). 실제/360(2)도 실제 일수라 기준 1·3 과 값이
/// 같다 (PRICE(39493,43054,0.0575,0.065,100,2,2) = 94.63544920787723, 대조 실험 x8·x9)
fn bond_period_days(pcd: f64, ncd: f64, fr: f64, b: u8) -> f64 {
    match b {
        0 | 4 => 360.0 / fr,
        _ => ncd - pcd,
    }
}

/// ODDF*·ODDL* 의 준이표 기간 정상 일수: 기준 1 은 그 기간의 실제 일수, 3 은 365/fr, 나머지는 360/fr
fn odd_period_days(p: f64, q: f64, fr: f64, b: u8) -> f64 {
    match b {
        1 => q - p,
        3 => 365.0 / fr,
        _ => 360.0 / fr,
    }
}

fn coup_days(pcd: f64, ncd: f64, fr: f64, b: u8, _d1904: bool) -> f64 {
    match b {
        1 => ncd - pcd,
        3 => 365.0 / fr,
        _ => 360.0 / fr,
    }
}

#[allow(clippy::too_many_arguments)]
/// 채권 가격: 남은 지급이 하나여도 일반식 (PRICE(39493,39600,…) = 99.77359320891287, 대조 실험 financial)
fn bond_price(s: f64, m: f64, rate: f64, yld: f64, red: f64, fr: f64, b: u8, d1904: bool) -> f64 {
    let (pcd, ncd, n) = coupon_dates(s, m, fr, d1904);
    let e = bond_period_days(pcd, ncd, fr, b);
    let a = day_count(pcd, s, b, d1904);
    let dsc = e - a;
    let c = 100.0 * rate / fr;
    let v = 1.0 + yld / fr;
    let mut p = red / v.mpow(n - 1.0 + dsc / e);
    let mut k = 1.0;
    while k <= n {
        p += c / v.mpow(k - 1.0 + dsc / e);
        k += 1.0;
    }
    p - c * a / e
}

fn price_fn(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let rate = try_v!(num_arg(ctx, a, 2));
    let yld = try_v!(num_arg(ctx, a, 3));
    let red = try_v!(num_arg(ctx, a, 4));
    let fr = try_v!(freq(ctx, a, 5));
    let b = try_v!(basis(ctx, a, 6));
    if rate < 0.0 || yld < 0.0 || red <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(bond_price(s, m, rate, yld, red, fr, b, ctx.date1904()))
}

fn yield_fn(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let rate = try_v!(num_arg(ctx, a, 2));
    let pr = try_v!(num_arg(ctx, a, 3));
    let red = try_v!(num_arg(ctx, a, 4));
    let fr = try_v!(freq(ctx, a, 5));
    let b = try_v!(basis(ctx, a, 6));
    if rate < 0.0 || pr <= 0.0 || red <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    // 가격 함수(일반식)의 해 — 한셀은 남은 지급이 하나여도 같은 식을 푼다
    let f = |y: f64| bond_price(s, m, rate, y, red, fr, b, d1904) - pr;
    // 남은 지급이 5–40 번이면 한셀은 [0, fr] 에서 거짓 위치법(할선이 구간 안의 한 끝을 늘 바꾸는)을 쓰고
    // |가격 − pr| < 1E-7 인 첫 값을 낸다 — 볼록한 가격 곡선이라 위에서 다가가 참 근보다 1E-10 쯤 크다
    // (YIELD(39494,42842,0.0575,95.04287,100,2,0) = 0.06473806170959318, 대조 실험 x7·x12: n = 5–40 의
    // 60여 경우 끝자리까지 같음). 4 번 이하·41 번 이상은 다른 반복이라 (아직 모름) 정확한 해를 낸다
    let (_, _, n) = coupon_dates(s, m, fr, d1904);
    if (5.0..=40.0).contains(&n) {
        let (mut a0, mut b0) = (0.0, fr);
        let (mut fa, mut fb) = (f(a0), f(b0));
        if fa * fb < 0.0 {
            for _ in 0..10_000 {
                let c = b0 - fb * (b0 - a0) / (fb - fa);
                let fc = f(c);
                if fc.abs() < 1e-7 {
                    return num(c);
                }
                if !fc.is_finite() {
                    break;
                }
                if fc * fa < 0.0 {
                    b0 = c;
                    fb = fc;
                } else {
                    a0 = c;
                    fa = fc;
                }
            }
        }
    }
    let (mut lo, mut hi) = (-0.99 * fr, 10.0);
    if f(lo) * f(hi) > 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    for _ in 0..300 {
        let mid = 0.5 * (lo + hi);
        if f(lo) * f(mid) <= 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
        if hi - lo < 1e-15 {
            break;
        }
    }
    let y0 = 0.5 * (lo + hi);
    // 뉴턴으로 다듬기
    let h = 1e-9;
    let d = (f(y0 + h) - f(y0 - h)) / (2.0 * h);
    let y1 = if d != 0.0 { y0 - f(y0) / d } else { y0 };
    num(if f(y1).abs() <= f(y0).abs() { y1 } else { y0 })
}

fn duration_fn(ctx: &mut Ctx, a: &[Value], modified: bool) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let cpn = try_v!(num_arg(ctx, a, 2));
    let yld = try_v!(num_arg(ctx, a, 3));
    let fr = try_v!(freq(ctx, a, 4));
    let b = try_v!(basis(ctx, a, 5));
    if cpn < 0.0 || yld < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    let (pcd, ncd, n) = coupon_dates(s, m, fr, d1904);
    let e = coup_days(pcd, ncd, fr, b, d1904);
    let a_ = day_count(pcd, s, b, d1904);
    let dsc = if b == 0 || b == 4 { e - a_ } else { ncd - s };
    let v = 1.0 + yld / fr;
    let c = 100.0 * cpn / fr;
    let mut pv_ = 0.0;
    let mut w = 0.0;
    let mut k = 1.0;
    while k <= n {
        let t = k - 1.0 + dsc / e;
        let cf = if k == n { c + 100.0 } else { c };
        let d = cf / v.mpow(t);
        pv_ += d;
        w += t * d;
        k += 1.0;
    }
    let dur = w / pv_ / fr;
    num(if modified { dur / v } else { dur })
}

fn pricemat(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let iss = try_v!(date_arg(ctx, a, 2));
    let rate = try_v!(num_arg(ctx, a, 3));
    let yld = try_v!(num_arg(ctx, a, 4));
    let b = try_v!(basis(ctx, a, 5));
    if rate < 0.0 || yld < 0.0 || iss >= s {
        return Value::Error(ErrorKind::Num);
    }
    let (fdim, fa, fdsm) = try_v!(mat_fracs(iss, s, m, b, ctx.date1904()));
    num((100.0 + fdim * rate * 100.0) / (1.0 + fdsm * yld) - fa * rate * 100.0)
}

fn yieldmat(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let iss = try_v!(date_arg(ctx, a, 2));
    let rate = try_v!(num_arg(ctx, a, 3));
    let pr = try_v!(num_arg(ctx, a, 4));
    let b = try_v!(basis(ctx, a, 5));
    if rate < 0.0 || pr <= 0.0 || iss >= s {
        return Value::Error(ErrorKind::Num);
    }
    let (fdim, fa, fdsm) = try_v!(mat_fracs(iss, s, m, b, ctx.date1904()));
    let t1 = 1.0 + fdim * rate;
    let t2 = pr / 100.0 + fa * rate;
    num((t1 - t2) / t2 / fdsm)
}

/// (발행→만기, 발행→결제, 결제→만기) 의 햇수: 기준 1 은 구간마다 YEARFRAC, 나머지는 일수 ÷ 1년 일수
/// (PRICEMAT(…,1) = 99.98029785136376, 대조 실험 financial)
fn mat_fracs(iss: f64, s: f64, m: f64, b: u8, d1904: bool) -> Result<(f64, f64, f64), ErrorKind> {
    if b == 1 {
        return Ok((
            year_frac(iss, m, 1, d1904)?,
            year_frac(iss, s, 1, d1904)?,
            year_frac(s, m, 1, d1904)?,
        ));
    }
    let bb = year_basis(b, s, d1904);
    Ok((
        day_count(iss, m, b, d1904) / bb,
        day_count(iss, s, b, d1904) / bb,
        day_count(s, m, b, d1904) / bb,
    ))
}

/// 실제/실제 경과 이자: 첫 이자일을 기준으로 센 이자 기간마다 (겹친 일수 / 기간 일수) 를 더한다
/// (ACCRINT(…,1000,4,1) = 16.57608695652174 = 25 × 61/92, 대조 실험 financial)
fn accrint_periods(iss: f64, first: f64, set: f64, fr: f64, d1904: bool) -> f64 {
    let step = (12.0 / fr) as i64;
    let mut total = 0.0;
    // 첫 이자일 앞쪽 기간 (k = 0 은 첫 이자일로 끝나는 기간)
    let mut k = 0i64;
    while k < 100_000 {
        let end = coupon_date(first, k * step, d1904);
        let start = coupon_date(first, (k + 1) * step, d1904);
        if end <= iss {
            break;
        }
        let (lo, hi) = (start.max(iss), end.min(set));
        if hi > lo {
            total += (hi - lo) / (end - start);
        }
        k += 1;
    }
    // 첫 이자일 뒤쪽 기간
    let mut k = 0i64;
    while k < 100_000 {
        let start = coupon_date(first, -k * step, d1904);
        let end = coupon_date(first, -(k + 1) * step, d1904);
        if start >= set {
            break;
        }
        let (lo, hi) = (start.max(iss), end.min(set));
        if hi > lo {
            total += (hi - lo) / (end - start);
        }
        k += 1;
    }
    total
}

fn accrint(ctx: &mut Ctx, a: &[Value]) -> Value {
    let iss = try_v!(date_arg(ctx, a, 0));
    let first = try_v!(date_arg(ctx, a, 1));
    let set = try_v!(date_arg(ctx, a, 2));
    let rate = try_v!(num_arg(ctx, a, 3));
    // 비운 액면 인수는 1000 (ACCRINT(…,0.1,,2), 대조 실험 financial)
    let par = if matches!(a.get(4), Some(Value::Empty)) {
        1000.0
    } else {
        try_v!(num_or(ctx, a, 4, 1000.0))
    };
    let fr = try_v!(freq(ctx, a, 5));
    let b = try_v!(basis(ctx, a, 6));
    if rate <= 0.0 || par <= 0.0 || iss >= set {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    // 계산 방법(여덟째 인수)을 주면 기준과 관계없이 30/360(US) 일수: 참이면 발행일부터, 거짓이면 발행일과
    // (첫 이자일 − 이자 기간) 가운데 늦은 날부터 결산일까지 (결산일이 앞이면 음수). 주지 않으면 기준대로
    // (ACCRINT(39508,39691,39569,0.1,1000,4,1) = 16.576, …,4,1,TRUE) = 16.667, …,4,1,FALSE) = −8.056, 대조 실험 c3)
    if a.len() >= 8 {
        let by_issue = try_v!(super::bool_or(ctx, a, 7, true));
        let start = if by_issue {
            iss
        } else {
            let (y, m, d) = ymd_from_serial(first, d1904).unwrap_or((1900, 1, 1));
            let total = y * 12 + m as i64 - 1 - (12.0 / fr) as i64;
            let (ny, nm) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32);
            let nd = d.min(days_in_month(ny, nm));
            serial_from_ymd(ny, nm as i64, nd as i64, d1904)
                .unwrap_or(iss)
                .max(iss)
        };
        let days = try_v!(days_30_360(start, set, true, d1904).ok_or(ErrorKind::Num));
        return num(par * rate * days / 360.0);
    }
    if b == 1 {
        return num(par * rate / fr * accrint_periods(iss, first, set, fr, d1904));
    }
    let yf = try_v!(year_frac(iss, set, b, d1904));
    num(par * rate * yf)
}

/// 프랑스식 상각 (AMORLINC 정액, AMORDEGRC 체감)
fn amor(ctx: &mut Ctx, a: &[Value], degressive: bool) -> Value {
    let cost = try_v!(num_arg(ctx, a, 0));
    let purchased = try_v!(date_arg(ctx, a, 1));
    let first = try_v!(date_arg(ctx, a, 2));
    let salv = try_v!(num_arg(ctx, a, 3));
    let period = try_v!(num_arg(ctx, a, 4)).trunc();
    let rate = try_v!(num_arg(ctx, a, 5));
    let b = try_v!(basis(ctx, a, 6));
    if b == 2
        || rate <= 0.0
        || cost < 0.0
        || salv < 0.0
        || salv > cost
        || period < 0.0
        || first < purchased
    {
        return Value::Error(ErrorKind::Num);
    }
    let yf = try_v!(year_frac(purchased, first, b, ctx.date1904()));
    if !degressive {
        let one = cost * rate;
        let first_dep = one * yf;
        let mut book = cost - salv;
        if period == 0.0 {
            return num(first_dep.min(book));
        }
        book -= first_dep;
        let mut p = 1.0;
        loop {
            let d = one.min(book.max(0.0));
            if p == period {
                return num(d);
            }
            book -= d;
            p += 1.0;
            if book <= 0.0 && p > period {
                return Value::Number(0.0);
            }
            if p > 10_000.0 {
                return Value::Number(0.0);
            }
        }
    }
    // 수명 3–4 년 1.5, 5–6 년 2, 6 년 초과 2.5, 그 밖(3 년 미만, 4–5 년 사이)은 #NUM!
    // (AMORDEGRC(…,0.4) = #NUM!, 대조 실험 financial)
    let life = 1.0 / rate;
    let coef = if (3.0..=4.0).contains(&life) {
        1.5
    } else if (5.0..=6.0).contains(&life) {
        2.0
    } else if life > 6.0 {
        2.5
    } else {
        return Value::Error(ErrorKind::Num);
    };
    let r = rate * coef;
    let round = |x: f64| super::math::round_digits(x, 0.0, super::math::Rounding::HalfAway);
    let mut dep = round(yf * r * cost);
    let mut book = cost - dep;
    if period == 0.0 {
        return num(dep);
    }
    // 마지막 기간은 floor(수명) − 1: 그 앞 기간에 남은 장부가의 절반, 마지막 기간에 남은 장부가 전부, 그 뒤는 0.
    // 마지막 기간 앞까지는 잔존가 아래로 내려가면 그 기간부터 0 (AMORDEGRC(1000,…,100,2,0.3,1) = 438,
    // (…,3,0.25,1) = 280, (2400,…,300,5,0.15,1) = 0, 대조 실험 financial·cmisc)
    let last = life.floor() - 1.0;
    let mut p = 1.0;
    while p <= period {
        if p > last {
            return Value::Number(0.0);
        }
        if p == last {
            dep = book;
        } else {
            dep = if p == last - 1.0 {
                round(book * 0.5)
            } else {
                round(book * r)
            };
            if book - dep < salv {
                return Value::Number(0.0);
            }
        }
        book -= dep;
        p += 1.0;
    }
    num(dep)
}

/// 첫 이자 기간이 다른 증권 (ODDFPRICE, ODDFYIELD)
fn odd_first(ctx: &mut Ctx, a: &[Value], yield_: bool) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let iss = try_v!(date_arg(ctx, a, 2));
    let fc = try_v!(date_arg(ctx, a, 3));
    let rate = try_v!(num_arg(ctx, a, 4));
    let x = try_v!(num_arg(ctx, a, 5));
    let red = try_v!(num_arg(ctx, a, 6));
    let fr = try_v!(freq(ctx, a, 7));
    let b = try_v!(basis(ctx, a, 8));
    if !(iss < s && s < fc && fc < m) || rate < 0.0 || x <= 0.0 && yield_ || red <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    // 첫 기간을 첫 이자일에서 거꾸로 센 준이표 기간으로 나눠 기간마다 일수/정상 일수를 더한다
    // (기간을 다 채우면 1 — 기준 3 의 분기 이자에서 드러난다). 발행→첫 이자일 dfc, 발행→결제 a,
    // 결제→첫 이자일 dsc (대조 실험 x11)
    let step = (12.0 / fr) as i64;
    let (mut dfc, mut a_, mut dsc) = (0.0, 0.0, 0.0);
    let mut q = fc;
    let mut j = 1;
    while q > iss && j < 100_000 {
        let p = coupon_date(fc, j * step, d1904);
        let nl = odd_period_days(p, q, fr, b);
        let frac = |lo: f64, hi: f64| {
            let (x, y) = (lo.max(p), hi.min(q));
            if x == p && y == q {
                1.0
            } else if x < y {
                day_count(x, y, b, d1904) / nl
            } else {
                0.0
            }
        };
        dfc += frac(iss, fc);
        a_ += frac(iss, s);
        dsc += frac(s, fc);
        q = p;
        j += 1;
    }
    let (_, _, n) = coupon_dates(fc, m, fr, d1904);
    // 경과 이자도 첫 이자일까지 할인한다 (Excel 과 다른 한셀 식: ODDFPRICE(39763,44256,39736,39873,
    // 0.0785,0.0625,100,2,1) = 113.60856511556307, Excel 은 113.597717474079)
    let price = |yld: f64| -> f64 {
        let v = 1.0 + yld / fr;
        let c = 100.0 * rate / fr;
        let mut p = red / v.mpow(n + dsc);
        p += c * dfc / v.mpow(dsc);
        let mut k = 1.0;
        while k <= n {
            p += c / v.mpow(k + dsc);
            k += 1.0;
        }
        p - c * a_ / v.mpow(dsc)
    };
    if !yield_ {
        if x < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        return num(price(x));
    }
    let f = |y: f64| price(y) - x;
    num_r(solve(f))
}

/// 마지막 이자 기간이 다른 증권 (ODDLPRICE, ODDLYIELD)
fn odd_last(ctx: &mut Ctx, a: &[Value], yield_: bool) -> Value {
    let (s, m) = try_v!(sm(ctx, a));
    let lc = try_v!(date_arg(ctx, a, 2));
    let rate = try_v!(num_arg(ctx, a, 3));
    let x = try_v!(num_arg(ctx, a, 4));
    let red = try_v!(num_arg(ctx, a, 5));
    let fr = try_v!(freq(ctx, a, 6));
    let b = try_v!(basis(ctx, a, 7));
    if !(lc < s && s < m) || rate < 0.0 || red <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let d1904 = ctx.date1904();
    // 마지막 기간을 만기에서 거꾸로 센 준이표 기간으로 나눠 기간마다 일수/정상 일수를 더한다
    // (기준 1 은 기간마다 실제 일수, ODDLPRICE(39494,39762,39345,0,0.0405,100,2,1) = 97.12545994508676,
    // 대조 실험 x11). 첫 기간과 달리 다 채운 기간도 일수/정상 일수 그대로
    let step = (12.0 / fr) as i64;
    let (mut dci, mut a_, mut dsc) = (0.0, 0.0, 0.0);
    let mut q = m;
    let mut k = 1;
    while q > lc && k < 100_000 {
        let p = coupon_date(m, k * step, d1904);
        let nl = odd_period_days(p, q, fr, b);
        let part = |lo: f64, hi: f64| {
            let (x, y) = (lo.max(p), hi.min(q));
            if x < y {
                day_count(x, y, b, d1904) / nl
            } else {
                0.0
            }
        };
        dci += part(lc, m);
        a_ += part(lc, s);
        dsc += part(s, m);
        q = p;
        k += 1;
    }
    let c = 100.0 * rate / fr;
    if !yield_ {
        if x < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        let p = (red + dci * c) / (1.0 + dsc * x / fr) - a_ * c;
        return num(p);
    }
    let pr = x;
    let y = ((red + dci * c) - (pr + a_ * c)) / (pr + a_ * c) * fr / dsc;
    num(y)
}

fn solve(f: impl Fn(f64) -> f64) -> Result<f64, ErrorKind> {
    let (mut lo, mut hi) = (-0.99, 10.0);
    if f(lo) * f(hi) > 0.0 {
        return Err(ErrorKind::Num);
    }
    for _ in 0..300 {
        let mid = 0.5 * (lo + hi);
        if f(lo) * f(mid) <= 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
        if hi - lo < 1e-15 {
            break;
        }
    }
    Ok(0.5 * (lo + hi))
}
