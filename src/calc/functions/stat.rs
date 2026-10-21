// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 통계 함수.

use std::rc::Rc;

use super::math::{grid, ifs_mask, resized};
use super::special as sp;
use super::{
    A, Agg, Arg, FnSpec, MANY, R, V, bool_or, collect_numbers, f, flatten, num_arg, num_or, opt,
    paired_numbers, to_array,
};
use crate::calc::eval::Ctx;
use crate::calc::value::{self, Array, ErrorKind, Value, num};

const RVR: &[Arg] = &[Arg::R, Arg::V, Arg::R];
const RRV: &[Arg] = &[Arg::R, Arg::R, Arg::V];
const RV: &[Arg] = &[Arg::R, Arg::V];
const AV: &[Arg] = &[Arg::A, Arg::V];
const VRV: &[Arg] = &[Arg::V, Arg::R, Arg::V];
const AAV: &[Arg] = &[Arg::A, Arg::A, Arg::V];
const VAA: &[Arg] = &[Arg::V, Arg::A, Arg::A];

pub const FNS: &[FnSpec] = &[
    f("AVERAGE", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 1)),
    f("AVERAGEA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 1)),
    f("COUNT", 0, MANY, R, count),
    f("COUNTA", 1, MANY, R, counta),
    f("COUNTBLANK", 1, 1, R, countblank),
    f("MAX", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 4)),
    f("MIN", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 5)),
    f("MAXA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 4)),
    f("MINA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 5)),
    f("STDEV", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 7)),
    f("STDEV.S", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 7)),
    f("STDEVP", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 8)),
    f("STDEV.P", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 8)),
    f("STDEVA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 7)),
    f("STDEVPA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 8)),
    f("VAR", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 10)),
    f("VAR.S", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 10)),
    f("VARP", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 11)),
    f("VAR.P", 1, MANY, R, |c, a| agg(c, a, Agg::Numbers, 11)),
    f("VARA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 10)),
    f("VARPA", 1, MANY, R, |c, a| agg(c, a, Agg::NumbersA, 11)),
    f("MEDIAN", 1, MANY, R, |c, a| median(&try_v!(collect_numbers(c, a, Agg::Numbers)))),
    f("MODE", 1, MANY, R, |c, a| mode_sngl(&try_v!(collect_numbers(c, a, Agg::Numbers)))),
    f("MODE.SNGL", 1, MANY, R, |c, a| mode_sngl(&try_v!(collect_numbers(c, a, Agg::Numbers)))),
    f("MODE.MULT", 1, MANY, R, mode_mult),
    f("DEVSQ", 1, MANY, R, |c, a| {
        let v = try_v!(collect_numbers(c, a, Agg::Numbers));
        if v.is_empty() {
            return Value::Error(ErrorKind::Num);
        }
        num(devsq(&v))
    }),
    f("AVEDEV", 1, MANY, R, |c, a| {
        let v = try_v!(collect_numbers(c, a, Agg::Numbers));
        if v.is_empty() {
            return Value::Error(ErrorKind::Num);
        }
        let m = mean(&v);
        num(v.iter().map(|x| (x - m).abs()).sum::<f64>() / v.len() as f64)
    }),
    f("GEOMEAN", 1, MANY, R, |c, a| {
        let v = try_v!(collect_numbers(c, a, Agg::Numbers));
        if v.is_empty() || v.iter().any(|&x| x <= 0.0) {
            return Value::Error(ErrorKind::Num);
        }
        num((v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp())
    }),
    f("HARMEAN", 1, MANY, R, |c, a| {
        let v = try_v!(collect_numbers(c, a, Agg::Numbers));
        if v.is_empty() || v.iter().any(|&x| x <= 0.0) {
            return Value::Error(ErrorKind::Num);
        }
        num(v.len() as f64 / v.iter().map(|x| 1.0 / x).sum::<f64>())
    }),
    f("KURT", 1, MANY, R, kurt),
    f("SKEW", 1, MANY, R, |c, a| skew(c, a, false)),
    f("SKEW.P", 1, MANY, R, |c, a| skew(c, a, true)),
    f("LARGE", 2, 2, AV, |c, a| {
        let v = try_v!(array_numbers(c, &a[0]));
        let k = try_v!(num_arg(c, a, 1));
        large_small(&v, k, true)
    }),
    f("SMALL", 2, 2, AV, |c, a| {
        let v = try_v!(array_numbers(c, &a[0]));
        let k = try_v!(num_arg(c, a, 1));
        large_small(&v, k, false)
    }),
    f("PERCENTILE", 2, 2, AV, |c, a| pct(c, a, percentile_inc)),
    f("PERCENTILE.INC", 2, 2, AV, |c, a| pct(c, a, percentile_inc)),
    f("PERCENTILE.EXC", 2, 2, AV, |c, a| pct(c, a, percentile_exc)),
    f("QUARTILE", 2, 2, AV, |c, a| pct(c, a, |v, q| quartile(v, q, true))),
    f("QUARTILE.INC", 2, 2, AV, |c, a| pct(c, a, |v, q| quartile(v, q, true))),
    f("QUARTILE.EXC", 2, 2, AV, |c, a| pct(c, a, |v, q| quartile(v, q, false))),
    f("PERCENTRANK", 2, 3, AV, |c, a| percentrank(c, a, true)),
    f("PERCENTRANK.INC", 2, 3, AV, |c, a| percentrank(c, a, true)),
    f("PERCENTRANK.EXC", 2, 3, AV, |c, a| percentrank(c, a, false)),
    f("RANK", 2, 3, VRV, |c, a| rank(c, a, false)),
    f("RANK.EQ", 2, 3, VRV, |c, a| rank(c, a, false)),
    f("RANK.AVG", 2, 3, VRV, |c, a| rank(c, a, true)),
    f("RANKEQ", 2, 3, VRV, |c, a| rank(c, a, false)),
    f("RANKAVG", 2, 3, VRV, |c, a| rank(c, a, true)),
    f("TRIMMEAN", 2, 2, AV, trimmean),
    f("STANDARDIZE", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let m = try_v!(num_arg(c, a, 1));
        let s = try_v!(num_arg(c, a, 2));
        if s <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num((x - m) / s)
    }),
    f("COUNTIF", 2, 2, RV, countif),
    f("COUNTIFS", 2, MANY, RV, countifs),
    f("AVERAGEIF", 2, 3, RVR, averageif),
    f("AVERAGEIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 1)),
    f("MAXIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 4)),
    f("MINIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 5)),
    f("CORREL", 2, 2, A, |c, a| correl(c, a, false)),
    f("PEARSON", 2, 2, A, |c, a| correl(c, a, false)),
    f("RSQ", 2, 2, A, |c, a| correl(c, a, true)),
    f("COVAR", 2, 2, A, |c, a| covar(c, a, false)),
    f("COVARIANCE.P", 2, 2, A, |c, a| covar(c, a, false)),
    f("COVARIANCE.S", 2, 2, A, |c, a| covar(c, a, true)),
    f("SLOPE", 2, 2, A, |c, a| regression(c, &a[0], &a[1], 0, 0.0)),
    f("INTERCEPT", 2, 2, A, |c, a| regression(c, &a[0], &a[1], 1, 0.0)),
    f("STEYX", 2, 2, A, |c, a| regression(c, &a[0], &a[1], 2, 0.0)),
    f("FORECAST", 3, 3, VAA, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        regression(c, &a[1], &a[2], 3, x)
    }),
    f("FORECAST.LINEAR", 3, 3, VAA, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        regression(c, &a[1], &a[2], 3, x)
    }),
    f("LINEST", 1, 4, &[Arg::A, Arg::A, Arg::V, Arg::V], |c, a| linest(c, a, false)),
    f("LOGEST", 1, 4, &[Arg::A, Arg::A, Arg::V, Arg::V], |c, a| linest(c, a, true)),
    f("TREND", 1, 4, &[Arg::A, Arg::A, Arg::A, Arg::V], |c, a| trend(c, a, false)),
    f("GROWTH", 1, 4, &[Arg::A, Arg::A, Arg::A, Arg::V], |c, a| trend(c, a, true)),
    f("FREQUENCY", 2, 2, A, frequency),
    f("PROB", 3, 4, AAV, prob),
    f("FISHER", 1, 1, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        if x.abs() >= 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(0.5 * ((1.0 + x) / (1.0 - x)).ln())
    }),
    f("FISHERINV", 1, 1, V, |c, a| num(try_v!(num_arg(c, a, 0)).tanh())),
    f("PERMUT", 2, 2, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        let k = try_v!(num_arg(c, a, 1)).trunc();
        if n <= 0.0 && !(n == 0.0 && k == 0.0) || k < 0.0 || k > n {
            return Value::Error(ErrorKind::Num);
        }
        let mut r = 1.0;
        let mut i = 0.0;
        while i < k {
            r *= n - i;
            i += 1.0;
        }
        num(r)
    }),
    f("PERMUTATIONA", 2, 2, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        let k = try_v!(num_arg(c, a, 1)).trunc();
        if n < 0.0 || k < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(n.powf(k))
    }),
    f("GAMMA", 1, 1, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        if x <= 0.0 && x == x.floor() {
            return Value::Error(ErrorKind::Num);
        }
        num(sp::gamma(x))
    }),
    f("GAMMALN", 1, 1, V, gammaln),
    f("GAMMALN.PRECISE", 1, 1, V, gammaln),
    f("GAUSS", 1, 1, V, |c, a| num(sp::norm_s_cdf(try_v!(num_arg(c, a, 0))) - 0.5)),
    f("PHI", 1, 1, V, |c, a| num(sp::norm_s_pdf(try_v!(num_arg(c, a, 0))))),
    // 정규 분포
    f("NORM.DIST", 4, 4, V, norm_dist),
    f("NORMDIST", 4, 4, V, norm_dist),
    f("NORM.S.DIST", 2, 2, V, |c, a| {
        let z = try_v!(num_arg(c, a, 0));
        let cum = try_v!(bool_or(c, a, 1, true));
        num(if cum { sp::norm_s_cdf(z) } else { sp::norm_s_pdf(z) })
    }),
    f("NORMSDIST", 1, 1, V, |c, a| num(sp::norm_s_cdf(try_v!(num_arg(c, a, 0))))),
    f("NORM.INV", 3, 3, V, norm_inv),
    f("NORMINV", 3, 3, V, norm_inv),
    f("NORM.S.INV", 1, 1, V, norm_s_inv),
    f("NORMSINV", 1, 1, V, norm_s_inv),
    f("LOGNORM.DIST", 4, 4, V, |c, a| lognorm_dist(c, a, true)),
    f("LOGNORMDIST", 3, 3, V, |c, a| lognorm_dist(c, a, false)),
    f("LOGNORM.INV", 3, 3, V, lognorm_inv),
    f("LOGINV", 3, 3, V, lognorm_inv),
    f("CONFIDENCE", 3, 3, V, |c, a| confidence(c, a, false)),
    f("CONFIDENCE.NORM", 3, 3, V, |c, a| confidence(c, a, false)),
    f("CONFIDENCE.T", 3, 3, V, |c, a| confidence(c, a, true)),
    f("ZTEST", 2, 3, &[Arg::A, Arg::V, Arg::V], ztest),
    f("Z.TEST", 2, 3, &[Arg::A, Arg::V, Arg::V], ztest),
    // 지수·와이블·감마·베타
    f("EXPON.DIST", 3, 3, V, expon_dist),
    f("EXPONDIST", 3, 3, V, expon_dist),
    f("WEIBULL.DIST", 4, 4, V, weibull),
    f("WEIBULL", 4, 4, V, weibull),
    f("GAMMA.DIST", 4, 4, V, gamma_dist),
    f("GAMMADIST", 4, 4, V, gamma_dist),
    f("GAMMA.INV", 3, 3, V, gamma_inv),
    f("GAMMAINV", 3, 3, V, gamma_inv),
    f("BETA.DIST", 4, 6, V, |c, a| beta_dist(c, a, true)),
    f("BETADIST", 3, 5, V, |c, a| beta_dist(c, a, false)),
    f("BETA.INV", 3, 5, V, beta_inv),
    f("BETAINV", 3, 5, V, beta_inv),
    // 이산 분포
    f("BINOM.DIST", 4, 4, V, binom_dist),
    f("BINOMDIST", 4, 4, V, binom_dist),
    f("BINOM.INV", 3, 3, V, binom_inv),
    f("CRITBINOM", 3, 3, V, binom_inv),
    f("BINOM.DIST.RANGE", 3, 4, V, binom_range),
    f("NEGBINOM.DIST", 4, 4, V, |c, a| negbinom(c, a, true)),
    f("NEGBINOMDIST", 3, 3, V, |c, a| negbinom(c, a, false)),
    f("HYPGEOM.DIST", 5, 5, V, |c, a| hypgeom(c, a, true)),
    f("HYPGEOMDIST", 4, 4, V, |c, a| hypgeom(c, a, false)),
    f("POISSON.DIST", 3, 3, V, poisson),
    f("POISSON", 3, 3, V, poisson),
    // 카이제곱·F·t
    f("CHISQ.DIST", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        let cum = try_v!(bool_or(c, a, 2, true));
        if x < 0.0 || !(1.0..=1e10).contains(&df) {
            return Value::Error(ErrorKind::Num);
        }
        num(if cum { sp::chisq_cdf(x, df) } else { sp::chisq_pdf(x, df) })
    }),
    f("CHISQ.DIST.RT", 2, 2, V, chi_rt),
    f("CHIDIST", 2, 2, V, chi_rt),
    f("CHISQ.INV", 2, 2, V, |c, a| {
        let p = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if !(0.0..1.0).contains(&p) || !(1.0..=1e10).contains(&df) {
            return Value::Error(ErrorKind::Num);
        }
        num(2.0 * sp::gamma_p_inv(p, df / 2.0))
    }),
    f("CHISQ.INV.RT", 2, 2, V, chi_inv_rt),
    f("CHIINV", 2, 2, V, chi_inv_rt),
    f("CHISQ.TEST", 2, 2, A, chitest),
    f("CHITEST", 2, 2, A, chitest),
    f("F.DIST", 4, 4, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let d1 = try_v!(num_arg(c, a, 1)).trunc();
        let d2 = try_v!(num_arg(c, a, 2)).trunc();
        let cum = try_v!(bool_or(c, a, 3, true));
        if x < 0.0 || d1 < 1.0 || d2 < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(if cum { sp::f_cdf(x, d1, d2) } else { sp::f_pdf(x, d1, d2) })
    }),
    f("F.DIST.RT", 3, 3, V, f_rt),
    f("FDIST", 3, 3, V, f_rt),
    f("F.INV", 3, 3, V, |c, a| {
        let p = try_v!(num_arg(c, a, 0));
        let d1 = try_v!(num_arg(c, a, 1)).trunc();
        let d2 = try_v!(num_arg(c, a, 2)).trunc();
        if !(0.0..=1.0).contains(&p) || d1 < 1.0 || d2 < 1.0 || p == 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(sp::f_inv(p, d1, d2))
    }),
    f("F.INV.RT", 3, 3, V, f_inv_rt),
    f("FINV", 3, 3, V, f_inv_rt),
    f("F.TEST", 2, 2, A, ftest),
    f("FTEST", 2, 2, A, ftest),
    f("T.DIST", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        let cum = try_v!(bool_or(c, a, 2, true));
        if df < 1.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num(if cum { sp::t_cdf(x, df) } else { sp::t_pdf(x, df) })
    }),
    f("T.DIST.2T", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if x < 0.0 || df < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(2.0 * sp::t_tail(x, df))
    }),
    f("T.DIST.RT", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if df < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(1.0 - sp::t_cdf(x, df))
    }),
    f("TDIST", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        let tails = try_v!(num_arg(c, a, 2)).trunc();
        if x < 0.0 || df < 1.0 || !(tails == 1.0 || tails == 2.0) {
            return Value::Error(ErrorKind::Num);
        }
        num(tails * sp::t_tail(x, df))
    }),
    f("T.INV", 2, 2, V, |c, a| {
        let p = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if p <= 0.0 || p >= 1.0 || df < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(sp::t_inv(p, df))
    }),
    f("T.INV.2T", 2, 2, V, t_inv_2t),
    f("TINV", 2, 2, V, t_inv_2t),
    f("T.TEST", 4, 4, &[Arg::A, Arg::A, Arg::V, Arg::V], ttest),
    f("TTEST", 4, 4, &[Arg::A, Arg::A, Arg::V, Arg::V], ttest),
];

// ---------------------------------------------------------------------------
// 기본 집계
// ---------------------------------------------------------------------------

pub fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

pub fn devsq(v: &[f64]) -> f64 {
    let m = mean(v);
    v.iter().map(|x| (x - m) * (x - m)).sum()
}

/// SUBTOTAL 번호의 집계 (1 평균, 2 개수, 3 개수A, 4 최대, 5 최소, 6 곱, 7 표본 표준편차, 8 모집단 표준편차, 9 합, 10 표본 분산, 11 모집단 분산)
pub fn basic_aggregate(kind: u8, v: &[f64]) -> Value {
    let n = v.len() as f64;
    match kind {
        1 => {
            if v.is_empty() {
                Value::Error(ErrorKind::Div0)
            } else {
                num(v.iter().sum::<f64>() / n)
            }
        }
        2 | 3 => Value::Number(n),
        4 => num(v.iter().copied().fold(f64::NEG_INFINITY, f64::max).max(if v.is_empty() { 0.0 } else { f64::NEG_INFINITY })),
        5 => num(if v.is_empty() { 0.0 } else { v.iter().copied().fold(f64::INFINITY, f64::min) }),
        6 => num(if v.is_empty() { 0.0 } else { v.iter().product() }),
        7 | 10 => {
            if v.len() < 2 {
                return Value::Error(ErrorKind::Div0);
            }
            let var = devsq(v) / (n - 1.0);
            num(if kind == 7 { var.sqrt() } else { var })
        }
        8 | 11 => {
            if v.is_empty() {
                return Value::Error(ErrorKind::Div0);
            }
            let var = devsq(v) / n;
            num(if kind == 8 { var.sqrt() } else { var })
        }
        9 => num(v.iter().sum()),
        _ => Value::Error(ErrorKind::Value),
    }
}

fn agg(ctx: &mut Ctx, a: &[Value], mode: Agg, kind: u8) -> Value {
    let v = try_v!(collect_numbers(ctx, a, mode));
    basic_aggregate(kind, &v)
}

fn count(ctx: &mut Ctx, a: &[Value]) -> Value {
    let mut n = 0usize;
    for v in a {
        match v {
            Value::Ref(r) => {
                for area in r.iter() {
                    ctx.for_each_in_area(area, |_, _, v| {
                        if matches!(v, Value::Number(_)) {
                            n += 1;
                        }
                    });
                }
            }
            Value::Array(arr) => n += arr.data.iter().filter(|v| matches!(v, Value::Number(_))).count(),
            Value::Number(_) | Value::Bool(_) | Value::Empty => n += 1,
            Value::Text(t) => {
                if value::parse_number(t, ctx.date1904()).is_some() {
                    n += 1;
                }
            }
            Value::Error(_) => {}
        }
    }
    Value::Number(n as f64)
}

fn counta(ctx: &mut Ctx, a: &[Value]) -> Value {
    let mut n = 0usize;
    for v in a {
        match v {
            Value::Ref(r) => {
                for area in r.iter() {
                    ctx.for_each_in_area(area, |_, _, _| n += 1);
                }
            }
            Value::Array(arr) => n += arr.data.iter().filter(|v| !v.is_empty()).count(),
            _ => n += 1,
        }
    }
    Value::Number(n as f64)
}

fn countblank(ctx: &mut Ctx, a: &[Value]) -> Value {
    match &a[0] {
        Value::Ref(r) => {
            let mut total = 0f64;
            let mut filled = 0f64;
            for area in r.iter() {
                total += area.rows() as f64 * area.cols() as f64 * (area.sheet_last - area.sheet + 1) as f64;
                ctx.for_each_in_area(area, |_, _, v| {
                    if !matches!(v, Value::Text(t) if t.is_empty()) {
                        filled += 1.0;
                    }
                });
            }
            Value::Number(total - filled)
        }
        Value::Array(arr) => Value::Number(
            arr.data
                .iter()
                .filter(|v| v.is_empty() || matches!(v, Value::Text(t) if t.is_empty()))
                .count() as f64,
        ),
        Value::Error(e) => Value::Error(*e),
        Value::Empty => Value::Number(1.0),
        Value::Text(t) if t.is_empty() => Value::Number(1.0),
        _ => Value::Number(0.0),
    }
}

pub fn median(v: &[f64]) -> Value {
    if v.is_empty() {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    num(if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    })
}

/// 가장 많이 나온 값 (같으면 먼저 나온 값). 반복이 없으면 #N/A
pub fn mode_sngl(v: &[f64]) -> Value {
    let mut best: Option<(f64, usize)> = None;
    for (i, x) in v.iter().enumerate() {
        let c = v.iter().filter(|y| *y == x).count();
        if c > 1 && best.is_none_or(|(_, bc)| c > bc) {
            best = Some((*x, c));
        }
        let _ = i;
    }
    match best {
        Some((x, _)) => num(x),
        None => Value::Error(ErrorKind::NA),
    }
}

fn mode_mult(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for x in &v {
        if let Some(e) = counts.iter_mut().find(|(y, _)| y == x) {
            e.1 += 1;
        } else {
            counts.push((*x, 1));
        }
    }
    let max = counts.iter().map(|c| c.1).max().unwrap_or(0);
    if max < 2 {
        return Value::Error(ErrorKind::NA);
    }
    let out: Vec<Value> = counts
        .iter()
        .filter(|c| c.1 == max)
        .map(|c| Value::Number(c.0))
        .collect();
    let n = out.len();
    Value::from(Array::new(n, 1, out))
}

fn kurt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let n = v.len() as f64;
    if v.len() < 4 {
        return Value::Error(ErrorKind::Div0);
    }
    let m = mean(&v);
    let s = (devsq(&v) / (n - 1.0)).sqrt();
    if s == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let sum4: f64 = v.iter().map(|x| ((x - m) / s).powi(4)).sum();
    num(n * (n + 1.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0)) * sum4
        - 3.0 * (n - 1.0) * (n - 1.0) / ((n - 2.0) * (n - 3.0)))
}

fn skew(ctx: &mut Ctx, a: &[Value], population: bool) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let n = v.len() as f64;
    if v.len() < if population { 1 } else { 3 } {
        return Value::Error(ErrorKind::Div0);
    }
    let m = mean(&v);
    let s = if population {
        (devsq(&v) / n).sqrt()
    } else {
        (devsq(&v) / (n - 1.0)).sqrt()
    };
    if s == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let sum3: f64 = v.iter().map(|x| ((x - m) / s).powi(3)).sum();
    if population {
        num(sum3 / n)
    } else {
        num(n / ((n - 1.0) * (n - 2.0)) * sum3)
    }
}

/// 배열 인수(참조 포함)의 숫자
fn array_numbers(ctx: &mut Ctx, v: &Value) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for x in flatten(ctx, v) {
        match x {
            Value::Number(n) => out.push(n),
            Value::Error(e) => return Err(e),
            Value::Text(_) | Value::Bool(_) | Value::Empty => {
                // 직접 준 스칼라는 숫자로 바꾼다
                if !matches!(v, Value::Ref(_) | Value::Array(_)) {
                    out.push(ctx.num(&x)?);
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

pub fn large_small(v: &[f64], k: f64, large: bool) -> Value {
    let k = k.ceil();
    if v.is_empty() || k < 1.0 || k > v.len() as f64 {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let i = k as usize - 1;
    num(if large { s[s.len() - 1 - i] } else { s[i] })
}

fn pct(ctx: &mut Ctx, a: &[Value], f: fn(&[f64], f64) -> Value) -> Value {
    let v = try_v!(array_numbers(ctx, &a[0]));
    let k = try_v!(num_arg(ctx, a, 1));
    f(&v, k)
}

pub fn percentile_inc(v: &[f64], k: f64) -> Value {
    if v.is_empty() || !(0.0..=1.0).contains(&k) {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let h = (s.len() - 1) as f64 * k;
    let lo = h.floor() as usize;
    let frac = h - lo as f64;
    if lo + 1 < s.len() {
        num(s[lo] + frac * (s[lo + 1] - s[lo]))
    } else {
        num(s[lo])
    }
}

pub fn percentile_exc(v: &[f64], k: f64) -> Value {
    if v.is_empty() || k <= 0.0 || k >= 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len() as f64;
    let h = (n + 1.0) * k - 1.0;
    if h < 0.0 || h > n - 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let lo = h.floor() as usize;
    let frac = h - lo as f64;
    if lo + 1 < s.len() {
        num(s[lo] + frac * (s[lo + 1] - s[lo]))
    } else {
        num(s[lo])
    }
}

pub fn quartile(v: &[f64], q: f64, inclusive: bool) -> Value {
    let q = q.trunc();
    if inclusive {
        if !(0.0..=4.0).contains(&q) {
            return Value::Error(ErrorKind::Num);
        }
        percentile_inc(v, q / 4.0)
    } else {
        if !(1.0..=3.0).contains(&q) {
            return Value::Error(ErrorKind::Num);
        }
        percentile_exc(v, q / 4.0)
    }
}

fn percentrank(ctx: &mut Ctx, a: &[Value], inclusive: bool) -> Value {
    let v = try_v!(array_numbers(ctx, &a[0]));
    let x = try_v!(num_arg(ctx, a, 1));
    let sig = try_v!(num_or(ctx, a, 2, 3.0)).trunc();
    if sig < 1.0 || v.is_empty() {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v;
    s.sort_by(f64::total_cmp);
    let n = s.len() as f64;
    if x < s[0] || x > s[s.len() - 1] {
        return Value::Error(ErrorKind::NA);
    }
    let less = s.iter().filter(|&&y| y < x).count();
    let exact = s.iter().any(|&y| y == x);
    let r = if exact {
        if inclusive {
            if s.len() == 1 {
                1.0
            } else {
                less as f64 / (n - 1.0)
            }
        } else {
            (less as f64 + 1.0) / (n + 1.0)
        }
    } else {
        let xl = s[less - 1];
        let xu = s[less];
        let frac = (x - xl) / (xu - xl);
        if inclusive {
            ((less - 1) as f64 + frac) / (n - 1.0)
        } else {
            (less as f64 + frac) / (n + 1.0)
        }
    };
    let p = 10f64.powi(sig as i32);
    num((value::round15(r * p)).floor() / p)
}

fn rank(ctx: &mut Ctx, a: &[Value], average: bool) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let mut v = Vec::new();
    match &a[1] {
        Value::Ref(r) => {
            for area in r.iter() {
                ctx.for_each_in_area(area, |_, _, c| {
                    if let Value::Number(n) = c {
                        v.push(*n);
                    }
                });
            }
        }
        Value::Array(arr) => {
            for c in &arr.data {
                if let Value::Number(n) = c {
                    v.push(*n);
                }
            }
        }
        Value::Error(e) => return Value::Error(*e),
        _ => return Value::Error(ErrorKind::Value),
    }
    let asc = match opt(a, 2) {
        None => false,
        Some(o) => try_v!(ctx.num(o)) != 0.0,
    };
    let ties = v.iter().filter(|&&y| y == x).count();
    if ties == 0 {
        return Value::Error(ErrorKind::NA);
    }
    let better = v
        .iter()
        .filter(|&&y| if asc { y < x } else { y > x })
        .count();
    if average {
        num(better as f64 + (ties as f64 + 1.0) / 2.0)
    } else {
        num(better as f64 + 1.0)
    }
}

fn trimmean(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(array_numbers(ctx, &a[0]));
    let p = try_v!(num_arg(ctx, a, 1));
    if !(0.0..1.0).contains(&p) || v.is_empty() {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v;
    s.sort_by(f64::total_cmp);
    let k = ((s.len() as f64 * p) / 2.0).floor() as usize;
    let inner = &s[k..s.len() - k];
    if inner.is_empty() {
        return Value::Error(ErrorKind::Num);
    }
    num(mean(inner))
}

fn gammaln(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    if x <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(sp::ln_gamma(x))
}

// ---------------------------------------------------------------------------
// 조건 집계
// ---------------------------------------------------------------------------

fn countif(ctx: &mut Ctx, a: &[Value]) -> Value {
    let crit = match &a[1] {
        Value::Ref(r) => ctx.intersect_ref(r),
        Value::Array(arr) => arr.data.first().cloned().unwrap_or(Value::Empty),
        o => o.clone(),
    };
    let c = crate::calc::criteria::Criterion::parse(&crit, ctx.date1904());
    match &a[0] {
        Value::Ref(r) => {
            let mut n = 0f64;
            let mut total = 0f64;
            let mut filled = 0f64;
            for area in r.iter() {
                total += area.rows() as f64 * area.cols() as f64 * (area.sheet_last - area.sheet + 1) as f64;
                ctx.for_each_in_area(area, |_, _, v| {
                    filled += 1.0;
                    if c.matches(v) {
                        n += 1.0;
                    }
                });
            }
            if c.matches(&Value::Empty) {
                n += total - filled;
            }
            Value::Number(n)
        }
        Value::Array(arr) => Value::Number(arr.data.iter().filter(|v| c.matches(v)).count() as f64),
        Value::Error(e) => Value::Error(*e),
        other => Value::Number(c.matches(other) as u8 as f64),
    }
}

fn countifs(ctx: &mut Ctx, a: &[Value]) -> Value {
    if a.len() % 2 != 0 {
        return Value::Error(ErrorKind::Value);
    }
    if a.len() == 2 {
        return countif(ctx, a);
    }
    let pairs: Vec<(&Value, &Value)> = a.chunks(2).map(|p| (&p[0], &p[1])).collect();
    let (_, _, mask) = try_v!(ifs_mask(ctx, &pairs));
    Value::Number(mask.iter().filter(|&&m| m).count() as f64)
}

fn averageif(ctx: &mut Ctx, a: &[Value]) -> Value {
    let target = match opt(a, 2) {
        Some(t) => resized(&a[0], t),
        None => a[0].clone(),
    };
    let (_, _, mask) = try_v!(ifs_mask(ctx, &[(&a[0], &a[1])]));
    let vals = try_v!(grid(ctx, &target));
    let mut nums = Vec::new();
    for (m, v) in mask.iter().zip(&vals.data) {
        if *m {
            match v {
                Value::Number(n) => nums.push(*n),
                Value::Error(e) => return Value::Error(*e),
                _ => {}
            }
        }
    }
    basic_aggregate(1, &nums)
}

/// AVERAGEIFS·MAXIFS·MINIFS (대상 범위, (조건 범위, 조건)…)
fn target_ifs(ctx: &mut Ctx, a: &[Value], kind: u8) -> Value {
    if a.len() % 2 != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let pairs: Vec<(&Value, &Value)> = a[1..].chunks(2).map(|p| (&p[0], &p[1])).collect();
    let (r, c, mask) = try_v!(ifs_mask(ctx, &pairs));
    let vals = try_v!(grid(ctx, &a[0]));
    if (vals.rows, vals.cols) != (r, c) {
        return Value::Error(ErrorKind::Value);
    }
    let mut nums = Vec::new();
    for (m, v) in mask.iter().zip(&vals.data) {
        if *m {
            match v {
                Value::Number(n) => nums.push(*n),
                Value::Error(e) => return Value::Error(*e),
                _ => {}
            }
        }
    }
    basic_aggregate(kind, &nums)
}

// ---------------------------------------------------------------------------
// 상관·회귀
// ---------------------------------------------------------------------------

fn correl(ctx: &mut Ctx, a: &[Value], square: bool) -> Value {
    let (x, y) = try_v!(paired_numbers(ctx, &a[0], &a[1]));
    if x.len() < 2 {
        return Value::Error(ErrorKind::Div0);
    }
    let mx = mean(&x);
    let my = mean(&y);
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (p, q) in x.iter().zip(&y) {
        sxy += (p - mx) * (q - my);
        sxx += (p - mx) * (p - mx);
        syy += (q - my) * (q - my);
    }
    if sxx == 0.0 || syy == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let r = sxy / (sxx * syy).sqrt();
    num(if square { r * r } else { r })
}

fn covar(ctx: &mut Ctx, a: &[Value], sample: bool) -> Value {
    let (x, y) = try_v!(paired_numbers(ctx, &a[0], &a[1]));
    let n = x.len() as f64;
    if x.is_empty() || (sample && x.len() < 2) {
        return Value::Error(ErrorKind::Div0);
    }
    let mx = mean(&x);
    let my = mean(&y);
    let s: f64 = x.iter().zip(&y).map(|(p, q)| (p - mx) * (q - my)).sum();
    num(s / if sample { n - 1.0 } else { n })
}

/// 단순 회귀: 0 기울기, 1 절편, 2 STEYX, 3 예측값 (y, x 순서)
fn regression(ctx: &mut Ctx, ys: &Value, xs: &Value, what: u8, at: f64) -> Value {
    let (y, x) = try_v!(paired_numbers(ctx, ys, xs));
    let n = x.len() as f64;
    if x.is_empty() || (what == 2 && x.len() < 3) {
        return Value::Error(ErrorKind::Div0);
    }
    let mx = mean(&x);
    let my = mean(&y);
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (p, q) in x.iter().zip(&y) {
        sxy += (p - mx) * (q - my);
        sxx += (p - mx) * (p - mx);
        syy += (q - my) * (q - my);
    }
    if sxx == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let slope = sxy / sxx;
    match what {
        0 => num(slope),
        1 => num(my - slope * mx),
        2 => num(((syy - sxy * sxy / sxx) / (n - 2.0)).max(0.0).sqrt()),
        _ => num(my - slope * mx + slope * at),
    }
}

/// 최소 제곱 (QR 분해 없이 정규 방정식). 계수는 [b_k … b_1, a] 순서 (LINEST 출력 순서)
fn least_squares(y: &[f64], xs: &[Vec<f64>], with_const: bool) -> Option<Vec<f64>> {
    let n = y.len();
    let k = xs.len() + with_const as usize;
    let mut ata = vec![0.0; k * k];
    let mut aty = vec![0.0; k];
    for i in 0..n {
        let row: Vec<f64> = xs
            .iter()
            .map(|x| x[i])
            .chain(with_const.then_some(1.0))
            .collect();
        for p in 0..k {
            aty[p] += row[p] * y[i];
            for q in 0..k {
                ata[p * k + q] += row[p] * row[q];
            }
        }
    }
    // 가우스 소거
    let mut m = ata;
    let mut b = aty;
    for c in 0..k {
        let mut piv = c;
        for r in c + 1..k {
            if m[r * k + c].abs() > m[piv * k + c].abs() {
                piv = r;
            }
        }
        if m[piv * k + c].abs() < 1e-300 {
            return None;
        }
        if piv != c {
            for q in 0..k {
                m.swap(piv * k + q, c * k + q);
            }
            b.swap(piv, c);
        }
        for r in 0..k {
            if r != c {
                let f = m[r * k + c] / m[c * k + c];
                for q in c..k {
                    m[r * k + q] -= f * m[c * k + q];
                }
                b[r] -= f * b[c];
            }
        }
    }
    let coef: Vec<f64> = (0..k).map(|i| b[i] / m[i * k + i]).collect();
    Some(coef)
}

fn regression_inputs(
    ctx: &mut Ctx,
    a: &[Value],
    log: bool,
) -> Result<(Vec<f64>, Vec<Vec<f64>>, (usize, usize)), ErrorKind> {
    let ya = to_array(ctx, &a[0])?;
    let mut y = Vec::new();
    for v in &ya.data {
        match v {
            Value::Number(n) => y.push(if log {
                if *n <= 0.0 {
                    return Err(ErrorKind::Num);
                }
                n.ln()
            } else {
                *n
            }),
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        }
    }
    let xs: Vec<Vec<f64>> = match opt(a, 1) {
        None => vec![(1..=y.len()).map(|i| i as f64).collect()],
        Some(xv) => {
            let xa = to_array(ctx, xv)?;
            let mut nums = Vec::new();
            for v in &xa.data {
                match v {
                    Value::Number(n) => nums.push(*n),
                    Value::Error(e) => return Err(*e),
                    _ => return Err(ErrorKind::Value),
                }
            }
            if xa.data.len() == y.len() {
                vec![nums]
            } else if ya.cols == 1 && xa.rows == ya.rows {
                (0..xa.cols)
                    .map(|c| (0..xa.rows).map(|r| nums[r * xa.cols + c]).collect())
                    .collect()
            } else if ya.rows == 1 && xa.cols == ya.cols {
                (0..xa.rows)
                    .map(|r| (0..xa.cols).map(|c| nums[r * xa.cols + c]).collect())
                    .collect()
            } else {
                return Err(ErrorKind::Ref);
            }
        }
    };
    Ok((y, xs, (ya.rows, ya.cols)))
}

fn linest(ctx: &mut Ctx, a: &[Value], log: bool) -> Value {
    let (y, xs, _) = try_v!(regression_inputs(ctx, a, log));
    let with_const = try_v!(bool_or(ctx, a, 2, true));
    let stats = try_v!(bool_or(ctx, a, 3, false));
    let Some(coef) = least_squares(&y, &xs, with_const) else {
        return Value::Error(ErrorKind::Num);
    };
    let k = xs.len();
    // 출력: m_k … m_1, b
    let mut first: Vec<f64> = coef[..k].iter().rev().copied().collect();
    first.push(if with_const { coef[k] } else { 0.0 });
    let tr = |v: f64| if log { v.exp() } else { v };
    let row1: Vec<Value> = first.iter().map(|&v| num(tr(v))).collect();
    if !stats {
        let n = row1.len();
        return Value::from(Array::new(1, n, row1));
    }
    // 통계: 표준 오차, R², F, SS
    let n = y.len() as f64;
    let pred: Vec<f64> = (0..y.len())
        .map(|i| {
            let mut s = if with_const { coef[k] } else { 0.0 };
            for (j, x) in xs.iter().enumerate() {
                s += coef[j] * x[i];
            }
            s
        })
        .collect();
    let ssresid: f64 = y.iter().zip(&pred).map(|(a, b)| (a - b) * (a - b)).sum();
    let my = mean(&y);
    let sstot: f64 = if with_const {
        y.iter().map(|v| (v - my) * (v - my)).sum()
    } else {
        y.iter().map(|v| v * v).sum()
    };
    let ssreg = sstot - ssresid;
    let df = n - k as f64 - with_const as usize as f64;
    let r2 = if sstot == 0.0 { 1.0 } else { ssreg / sstot };
    let sey = if df > 0.0 { (ssresid / df).sqrt() } else { 0.0 };
    let fstat = if df > 0.0 && ssresid > 0.0 {
        (ssreg / k as f64) / (ssresid / df)
    } else {
        f64::INFINITY
    };
    let cols = k + 1;
    let mut data = row1;
    // 표준 오차 행: 단순화를 위해 단순 회귀만 정확히, 다중 회귀는 #N/A
    if k == 1 {
        let x = &xs[0];
        let mx = mean(x);
        let sxx: f64 = if with_const {
            x.iter().map(|v| (v - mx) * (v - mx)).sum()
        } else {
            x.iter().map(|v| v * v).sum()
        };
        data.push(num(sey / sxx.sqrt()));
        if with_const {
            let sumx2: f64 = x.iter().map(|v| v * v).sum();
            data.push(num(sey * (sumx2 / (n * sxx)).sqrt()));
        } else {
            data.push(Value::Error(ErrorKind::NA));
        }
    } else {
        for _ in 0..cols {
            data.push(Value::Error(ErrorKind::NA));
        }
    }
    data.push(num(r2));
    data.push(num(sey));
    for _ in 2..cols {
        data.push(Value::Error(ErrorKind::NA));
    }
    data.push(if fstat.is_finite() { num(fstat) } else { Value::Error(ErrorKind::Num) });
    data.push(num(df));
    for _ in 2..cols {
        data.push(Value::Error(ErrorKind::NA));
    }
    data.push(num(ssreg));
    data.push(num(ssresid));
    for _ in 2..cols {
        data.push(Value::Error(ErrorKind::NA));
    }
    Value::from(Array::new(5, cols, data))
}

fn trend(ctx: &mut Ctx, a: &[Value], log: bool) -> Value {
    let (y, xs, _) = try_v!(regression_inputs(ctx, a, log));
    let with_const = try_v!(bool_or(ctx, a, 3, true));
    let Some(coef) = least_squares(&y, &xs, with_const) else {
        return Value::Error(ErrorKind::Num);
    };
    let k = xs.len();
    let new_x: Vec<Vec<f64>> = match opt(a, 2) {
        None => xs.clone(),
        Some(nv) => {
            let na = try_v!(to_array(ctx, nv));
            let mut nums = Vec::new();
            for v in &na.data {
                match v {
                    Value::Number(n) => nums.push(*n),
                    Value::Error(e) => return Value::Error(*e),
                    _ => return Value::Error(ErrorKind::Value),
                }
            }
            if k == 1 {
                vec![nums]
            } else if na.cols == k {
                (0..k).map(|c| (0..na.rows).map(|r| nums[r * k + c]).collect()).collect()
            } else {
                (0..k).map(|r| (0..na.cols).map(|c| nums[r * na.cols + c]).collect()).collect()
            }
        }
    };
    let m = new_x[0].len();
    let out: Vec<Value> = (0..m)
        .map(|i| {
            let mut s = if with_const { coef[k] } else { 0.0 };
            for (j, x) in new_x.iter().enumerate() {
                s += coef[j] * x[i];
            }
            num(if log { s.exp() } else { s })
        })
        .collect();
    Value::from(Array::new(m, 1, out))
}

fn frequency(ctx: &mut Ctx, a: &[Value]) -> Value {
    let data: Vec<f64> = flatten(ctx, &a[0])
        .into_iter()
        .filter_map(|v| if let Value::Number(n) = v { Some(n) } else { None })
        .collect();
    let bins: Vec<f64> = flatten(ctx, &a[1])
        .into_iter()
        .filter_map(|v| if let Value::Number(n) = v { Some(n) } else { None })
        .collect();
    let mut order: Vec<usize> = (0..bins.len()).collect();
    order.sort_by(|&i, &j| bins[i].total_cmp(&bins[j]));
    let mut counts = vec![0.0; bins.len() + 1];
    for x in data {
        match order.iter().find(|&&i| x <= bins[i]) {
            Some(&i) => counts[i] += 1.0,
            None => counts[bins.len()] += 1.0,
        }
    }
    let n = counts.len();
    Value::from(Array::new(n, 1, counts.into_iter().map(Value::Number).collect()))
}

fn prob(ctx: &mut Ctx, a: &[Value]) -> Value {
    let xs = flatten(ctx, &a[0]);
    let ps = flatten(ctx, &a[1]);
    if xs.len() != ps.len() {
        return Value::Error(ErrorKind::NA);
    }
    let lo = try_v!(num_arg(ctx, a, 2));
    let hi = match opt(a, 3) {
        None => lo,
        Some(v) => try_v!(ctx.num(v)),
    };
    let mut total = 0.0;
    let mut s = 0.0;
    for (x, p) in xs.iter().zip(&ps) {
        let p = match p {
            Value::Number(p) => *p,
            Value::Error(e) => return Value::Error(*e),
            _ => continue,
        };
        if !(0.0..=1.0).contains(&p) {
            return Value::Error(ErrorKind::Num);
        }
        total += p;
        if let Value::Number(x) = x
            && *x >= lo
            && *x <= hi
        {
            s += p;
        }
    }
    if (total - 1.0).abs() > 1e-7 {
        return Value::Error(ErrorKind::Num);
    }
    num(s)
}

// ---------------------------------------------------------------------------
// 분포
// ---------------------------------------------------------------------------

fn norm_dist(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    let s = try_v!(num_arg(ctx, a, 2));
    let cum = try_v!(bool_or(ctx, a, 3, true));
    if s <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let z = (x - m) / s;
    num(if cum { sp::norm_s_cdf(z) } else { sp::norm_s_pdf(z) / s })
}

fn norm_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    let s = try_v!(num_arg(ctx, a, 2));
    if p <= 0.0 || p >= 1.0 || s <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(m + s * sp::norm_s_inv(p))
}

fn norm_s_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    if p <= 0.0 || p >= 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(sp::norm_s_inv(p))
}

fn lognorm_dist(ctx: &mut Ctx, a: &[Value], has_cum: bool) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    let s = try_v!(num_arg(ctx, a, 2));
    let cum = if has_cum { try_v!(bool_or(ctx, a, 3, true)) } else { true };
    if x <= 0.0 || s <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let z = (x.ln() - m) / s;
    num(if cum {
        sp::norm_s_cdf(z)
    } else {
        sp::norm_s_pdf(z) / (s * x)
    })
}

fn lognorm_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    let s = try_v!(num_arg(ctx, a, 2));
    if p <= 0.0 || p >= 1.0 || s <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num((m + s * sp::norm_s_inv(p)).exp())
}

fn confidence(ctx: &mut Ctx, a: &[Value], t: bool) -> Value {
    let alpha = try_v!(num_arg(ctx, a, 0));
    let sd = try_v!(num_arg(ctx, a, 1));
    let n = try_v!(num_arg(ctx, a, 2)).trunc();
    if alpha <= 0.0 || alpha >= 1.0 || sd <= 0.0 || n < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    if t {
        if n == 1.0 {
            return Value::Error(ErrorKind::Div0);
        }
        return num(sp::t_inv(1.0 - alpha / 2.0, n - 1.0) * sd / n.sqrt());
    }
    num(sp::norm_s_inv(1.0 - alpha / 2.0) * sd / n.sqrt())
}

fn ztest(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(array_numbers(ctx, &a[0]));
    let x = try_v!(num_arg(ctx, a, 1));
    if v.is_empty() {
        return Value::Error(ErrorKind::NA);
    }
    let n = v.len() as f64;
    let m = mean(&v);
    let sigma = match opt(a, 2) {
        Some(s) => try_v!(ctx.num(s)),
        None => {
            if v.len() < 2 {
                return Value::Error(ErrorKind::Div0);
            }
            (devsq(&v) / (n - 1.0)).sqrt()
        }
    };
    if sigma == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    num(1.0 - sp::norm_s_cdf((m - x) / (sigma / n.sqrt())))
}

fn expon_dist(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let l = try_v!(num_arg(ctx, a, 1));
    let cum = try_v!(bool_or(ctx, a, 2, true));
    if x < 0.0 || l <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(if cum { 1.0 - (-l * x).exp() } else { l * (-l * x).exp() })
}

fn weibull(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    let cum = try_v!(bool_or(ctx, a, 3, true));
    if x < 0.0 || al <= 0.0 || be <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let t = (x / be).powf(al);
    num(if cum {
        1.0 - (-t).exp()
    } else {
        al / be.powf(al) * x.powf(al - 1.0) * (-t).exp()
    })
}

fn gamma_dist(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    let cum = try_v!(bool_or(ctx, a, 3, true));
    if x < 0.0 || al <= 0.0 || be <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(if cum {
        sp::gamma_p(al, x / be)
    } else {
        sp::gamma_pdf(x, al, be)
    })
}

fn gamma_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    if !(0.0..1.0).contains(&p) || al <= 0.0 || be <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(be * sp::gamma_p_inv(p, al))
}

fn beta_dist(ctx: &mut Ctx, a: &[Value], new: bool) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    let (cum, lo, hi) = if new {
        (
            try_v!(bool_or(ctx, a, 3, true)),
            try_v!(num_or(ctx, a, 4, 0.0)),
            try_v!(num_or(ctx, a, 5, 1.0)),
        )
    } else {
        (true, try_v!(num_or(ctx, a, 3, 0.0)), try_v!(num_or(ctx, a, 4, 1.0)))
    };
    if al <= 0.0 || be <= 0.0 || x < lo || x > hi || lo >= hi {
        return Value::Error(ErrorKind::Num);
    }
    let t = (x - lo) / (hi - lo);
    num(if cum {
        sp::beta_i(t, al, be)
    } else {
        sp::beta_pdf(t, al, be) / (hi - lo)
    })
}

fn beta_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    let lo = try_v!(num_or(ctx, a, 3, 0.0));
    let hi = try_v!(num_or(ctx, a, 4, 1.0));
    if !(0.0..=1.0).contains(&p) || al <= 0.0 || be <= 0.0 || lo >= hi {
        return Value::Error(ErrorKind::Num);
    }
    num(lo + (hi - lo) * sp::beta_i_inv(p, al, be))
}

fn binom_dist(ctx: &mut Ctx, a: &[Value]) -> Value {
    let k = try_v!(num_arg(ctx, a, 0)).trunc();
    let n = try_v!(num_arg(ctx, a, 1)).trunc();
    let p = try_v!(num_arg(ctx, a, 2));
    let cum = try_v!(bool_or(ctx, a, 3, true));
    if k < 0.0 || n < 0.0 || k > n || !(0.0..=1.0).contains(&p) {
        return Value::Error(ErrorKind::Num);
    }
    if cum {
        let mut s = 0.0;
        let mut i = 0.0;
        while i <= k {
            s += sp::binom_pmf(i, n, p);
            i += 1.0;
        }
        num(s.min(1.0))
    } else {
        num(sp::binom_pmf(k, n, p))
    }
}

fn binom_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0)).trunc();
    let p = try_v!(num_arg(ctx, a, 1));
    let alpha = try_v!(num_arg(ctx, a, 2));
    if n < 0.0 || !(0.0..=1.0).contains(&p) || !(0.0..=1.0).contains(&alpha) {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = 0.0;
    let mut k = 0.0;
    while k <= n {
        s += sp::binom_pmf(k, n, p);
        if s >= alpha * (1.0 - 1e-15) {
            return num(k);
        }
        k += 1.0;
    }
    num(n)
}

fn binom_range(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0)).trunc();
    let p = try_v!(num_arg(ctx, a, 1));
    let s1 = try_v!(num_arg(ctx, a, 2)).trunc();
    let s2 = match opt(a, 3) {
        Some(v) => try_v!(ctx.num(v)).trunc(),
        None => s1,
    };
    if n < 0.0 || !(0.0..=1.0).contains(&p) || s1 < 0.0 || s1 > n || s2 < s1 || s2 > n {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = 0.0;
    let mut k = s1;
    while k <= s2 {
        s += sp::binom_pmf(k, n, p);
        k += 1.0;
    }
    num(s)
}

fn negbinom(ctx: &mut Ctx, a: &[Value], new: bool) -> Value {
    let fl = try_v!(num_arg(ctx, a, 0)).trunc();
    let s = try_v!(num_arg(ctx, a, 1)).trunc();
    let p = try_v!(num_arg(ctx, a, 2));
    let cum = if new { try_v!(bool_or(ctx, a, 3, true)) } else { false };
    if fl < 0.0 || s < 1.0 || p <= 0.0 || p >= 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let pmf = |f: f64| sp::binom_coef(f + s - 1.0, s - 1.0) * p.powf(s) * (1.0 - p).powf(f);
    if cum {
        num(sp::beta_i(p, s, fl + 1.0))
    } else {
        num(pmf(fl))
    }
}

fn hypgeom(ctx: &mut Ctx, a: &[Value], new: bool) -> Value {
    let k = try_v!(num_arg(ctx, a, 0)).trunc();
    let n = try_v!(num_arg(ctx, a, 1)).trunc();
    let m = try_v!(num_arg(ctx, a, 2)).trunc();
    let big = try_v!(num_arg(ctx, a, 3)).trunc();
    let cum = if new { try_v!(bool_or(ctx, a, 4, true)) } else { false };
    if k < 0.0 || n < 0.0 || m < 0.0 || big < 0.0 || k > n || k > m || n > big || m > big || n - k > big - m {
        return Value::Error(ErrorKind::Num);
    }
    let pmf = |x: f64| {
        sp::binom_coef(m, x) * sp::binom_coef(big - m, n - x) / sp::binom_coef(big, n)
    };
    if cum {
        let lo = (n - (big - m)).max(0.0);
        let mut s = 0.0;
        let mut x = lo;
        while x <= k {
            s += pmf(x);
            x += 1.0;
        }
        num(s.min(1.0))
    } else {
        num(pmf(k))
    }
}

fn poisson(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0)).trunc();
    let m = try_v!(num_arg(ctx, a, 1));
    let cum = try_v!(bool_or(ctx, a, 2, true));
    if x < 0.0 || m < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    if m == 0.0 {
        return Value::Number(if cum || x == 0.0 { 1.0 } else { 0.0 });
    }
    if cum {
        num(sp::gamma_q(x + 1.0, m))
    } else {
        num((x * m.ln() - m - sp::ln_gamma(x + 1.0)).exp())
    }
}

fn chi_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let df = try_v!(num_arg(ctx, a, 1)).trunc();
    if x < 0.0 || !(1.0..=1e10).contains(&df) {
        return Value::Error(ErrorKind::Num);
    }
    num(sp::gamma_q(df / 2.0, x / 2.0))
}

fn chi_inv_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let df = try_v!(num_arg(ctx, a, 1)).trunc();
    if !(0.0..=1.0).contains(&p) || p == 0.0 || !(1.0..=1e10).contains(&df) {
        return Value::Error(ErrorKind::Num);
    }
    num(2.0 * sp::gamma_p_inv(1.0 - p, df / 2.0))
}

fn chitest(ctx: &mut Ctx, a: &[Value]) -> Value {
    let act = try_v!(to_array(ctx, &a[0]));
    let exp = try_v!(to_array(ctx, &a[1]));
    if act.rows != exp.rows || act.cols != exp.cols {
        return Value::Error(ErrorKind::NA);
    }
    let mut chi = 0.0;
    for (x, e) in act.data.iter().zip(&exp.data) {
        if let (Value::Number(x), Value::Number(e)) = (x, e) {
            if *e == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            chi += (x - e) * (x - e) / e;
        }
    }
    let df = if act.rows == 1 || act.cols == 1 {
        (act.rows * act.cols) as f64 - 1.0
    } else {
        (act.rows as f64 - 1.0) * (act.cols as f64 - 1.0)
    };
    if df < 1.0 {
        return Value::Error(ErrorKind::NA);
    }
    num(sp::gamma_q(df / 2.0, chi / 2.0))
}

fn f_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d1 = try_v!(num_arg(ctx, a, 1)).trunc();
    let d2 = try_v!(num_arg(ctx, a, 2)).trunc();
    if x < 0.0 || d1 < 1.0 || d2 < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(1.0 - sp::f_cdf(x, d1, d2))
}

fn f_inv_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let d1 = try_v!(num_arg(ctx, a, 1)).trunc();
    let d2 = try_v!(num_arg(ctx, a, 2)).trunc();
    if p <= 0.0 || p > 1.0 || d1 < 1.0 || d2 < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(sp::f_inv(1.0 - p, d1, d2))
}

fn ftest(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(array_numbers(ctx, &a[0]));
    let y = try_v!(array_numbers(ctx, &a[1]));
    if x.len() < 2 || y.len() < 2 {
        return Value::Error(ErrorKind::Div0);
    }
    let vx = devsq(&x) / (x.len() as f64 - 1.0);
    let vy = devsq(&y) / (y.len() as f64 - 1.0);
    if vx == 0.0 || vy == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let fv = vx / vy;
    let c = sp::f_cdf(fv, x.len() as f64 - 1.0, y.len() as f64 - 1.0);
    num(2.0 * c.min(1.0 - c))
}

fn t_inv_2t(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let df = try_v!(num_arg(ctx, a, 1)).trunc();
    if p <= 0.0 || p > 1.0 || df < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(sp::t_inv(1.0 - p / 2.0, df))
}

fn ttest(ctx: &mut Ctx, a: &[Value]) -> Value {
    let tails = try_v!(num_arg(ctx, a, 2)).trunc();
    let kind = try_v!(num_arg(ctx, a, 3)).trunc();
    if !(tails == 1.0 || tails == 2.0) || !(1.0..=3.0).contains(&kind) {
        return Value::Error(ErrorKind::Num);
    }
    let (t, df) = if kind == 1.0 {
        let (x, y) = try_v!(paired_numbers(ctx, &a[0], &a[1]));
        let d: Vec<f64> = x.iter().zip(&y).map(|(p, q)| p - q).collect();
        let n = d.len() as f64;
        if d.len() < 2 {
            return Value::Error(ErrorKind::Div0);
        }
        let sd = (devsq(&d) / (n - 1.0)).sqrt();
        if sd == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        (mean(&d) / (sd / n.sqrt()), n - 1.0)
    } else {
        let x = try_v!(array_numbers(ctx, &a[0]));
        let y = try_v!(array_numbers(ctx, &a[1]));
        let (n1, n2) = (x.len() as f64, y.len() as f64);
        if x.len() < 2 || y.len() < 2 {
            return Value::Error(ErrorKind::Div0);
        }
        let (v1, v2) = (devsq(&x) / (n1 - 1.0), devsq(&y) / (n2 - 1.0));
        let diff = mean(&x) - mean(&y);
        if kind == 2.0 {
            let sp2 = ((n1 - 1.0) * v1 + (n2 - 1.0) * v2) / (n1 + n2 - 2.0);
            let se = (sp2 * (1.0 / n1 + 1.0 / n2)).sqrt();
            if se == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            (diff / se, n1 + n2 - 2.0)
        } else {
            let se2 = v1 / n1 + v2 / n2;
            if se2 == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            let df = se2 * se2 / ((v1 / n1).powi(2) / (n1 - 1.0) + (v2 / n2).powi(2) / (n2 - 1.0));
            (diff / se2.sqrt(), df)
        }
    };
    num(tails * sp::t_tail(t.abs(), df))
}

#[allow(dead_code)]
fn unused(_: Rc<Array>) {}
