// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 통계 함수.

use crate::calc::fm::Fm;
use std::rc::Rc;

use super::math::{grid, ifs_mask, resized};
use super::hcnum as hc;
use super::special as sp;
use super::{
    A, Agg, Arg, FnSpec, MANY, R, V, bool_or, collect_numbers, f, flatten, num_arg, num_or, opt, pairs,
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
        // 곱의 n제곱근 (곱이 넘치면 #NUM!, 대조 실험 stat·variance)
        let p: f64 = v.iter().product();
        if !p.is_finite() || p <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(p.mpow(1.0 / v.len() as f64))
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
    f("LARGE", 2, 2, RV, |c, a| {
        let v = try_v!(array_numbers(c, &a[0]));
        let k = try_v!(num_arg(c, a, 1));
        large_small(&v, k, true)
    }),
    f("SMALL", 2, 2, RV, |c, a| {
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
    pairs(f("COUNTIFS", 2, MANY, RV, countifs), 2),
    f("AVERAGEIF", 2, 3, RVR, averageif),
    pairs(f("AVERAGEIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 1)), 2),
    pairs(f("MAXIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 4)), 2),
    pairs(f("MINIFS", 3, MANY, RRV, |c, a| target_ifs(c, a, 5)), 2),
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
        num(0.5 * ((1.0 + x) / (1.0 - x)).mln())
    }),
    f("FISHERINV", 1, 1, V, |c, a| num(try_v!(num_arg(c, a, 0)).mtanh())),
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
        // PERMUTATIONA(0,0) = 0 (대조 실험 stat)
        if n == 0.0 {
            return Value::Number(0.0);
        }
        num(n.mpow(k))
    }),
    // GAMMA: 171 이상은 #NUM!, 결과는 소수 9자리 반올림 (GAMMA(2.5) = 1.329340388, 대조 실험 stat)
    f("GAMMA", 1, 1, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        if (x <= 0.0 && x == x.floor()) || x >= 171.0 {
            return Value::Error(ErrorKind::Num);
        }
        // 양수는 6항 란초시 근사의 exp (GAMMA(30.5) = 4.822696933622876E+31, 대조 실험 stat)
        num(round9(if x > 0.0 { hc::gammln(x).mexp() } else { sp::gamma(x) }))
    }),
    f("GAMMALN", 1, 1, V, gammaln),
    f("GAMMALN.PRECISE", 1, 1, V, gammaln),
    // GAUSS·PHI 는 소수 9자리로 반올림 (PHI(0) = 0.39894228000000004, 대조 실험 stat·normal).
    // GAUSS 는 체비쇼프 근사 erfc 로 (GAUSS(1E-9) = −1.5E-8), 0 만 따로 0 (대조 실험 norm4)
    f("GAUSS", 1, 1, V, |c, a| {
        let z = try_v!(num_arg(c, a, 0));
        if z == 0.0 {
            return Value::Number(0.0);
        }
        num(round9(0.5 - 0.5 * hc::erfcc(z / std::f64::consts::SQRT_2)))
    }),
    f("PHI", 1, 1, V, |c, a| num(round9(sp::norm_s_pdf(try_v!(num_arg(c, a, 0)))))),
    // 정규 분포
    f("NORM.DIST", 4, 4, V, norm_dist),
    f("NORMDIST", 4, 4, V, norm_dist),
    f("NORM.S.DIST", 2, 2, V, |c, a| {
        let z = try_v!(num_arg(c, a, 0));
        let cum = try_v!(bool_or(c, a, 1, true));
        num(if cum { hc::norm_cdf(z) } else { hc::norm_pdf(z) })
    }),
    // NORMSDIST 만은 정확하다 (대조 실험 norm4)
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
    f("BINOM.INV", 3, 3, V, |c, a| binom_inv(c, a, false)),
    f("CRITBINOM", 3, 3, V, |c, a| binom_inv(c, a, true)),
    f("BINOM.DIST.RANGE", 3, 4, V, binom_range),
    f("NEGBINOM.DIST", 4, 4, V, |c, a| negbinom(c, a, true)),
    f("NEGBINOMDIST", 3, 3, V, |c, a| negbinom(c, a, false)),
    f("HYPGEOM.DIST", 5, 5, V, |c, a| hypgeom(c, a, true)),
    f("HYPGEOMDIST", 4, 4, V, |c, a| hypgeom(c, a, false)),
    f("POISSON.DIST", 3, 3, V, poisson),
    f("POISSON", 3, 3, V, poisson),
    // 카이제곱·F·t
    // 엑셀 2010 분포 함수는 소수 9자리 반올림. 확률 밀도의 x = 0 은 #NUM! (대조 실험 dist)
    f("CHISQ.DIST", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        let cum = try_v!(bool_or(c, a, 2, true));
        if x < 0.0 || !(1.0..=1e10).contains(&df) || (x == 0.0 && !cum) {
            return Value::Error(ErrorKind::Num);
        }
        num(round9(if cum { hc::gammp(df / 2.0, x / 2.0) } else { sp::chisq_pdf(x, df) }))
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
    f("CHISQ.TEST", 2, 2, R, |c, a| chitest(c, a, true)),
    f("CHITEST", 2, 2, A, |c, a| chitest(c, a, false)),
    // F.DIST: 밀도의 x = 0 은 0, 자유도는 1E+10 미만, 소수 9자리 반올림 (대조 실험 dist)
    f("F.DIST", 4, 4, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let d1 = try_v!(num_arg(c, a, 1)).trunc();
        let d2 = try_v!(num_arg(c, a, 2)).trunc();
        let cum = try_v!(bool_or(c, a, 3, true));
        if x < 0.0 || d1 < 1.0 || d2 < 1.0 || d1 >= 1e10 || d2 >= 1e10 {
            return Value::Error(ErrorKind::Num);
        }
        if x == 0.0 && !cum {
            return Value::Number(0.0);
        }
        num(round9(if cum { hc::f_cdf(x, d1, d2) } else { sp::f_pdf(x, d1, d2) }))
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
        if !(1.0..1e10).contains(&df) {
            return Value::Error(ErrorKind::Num);
        }
        num(round9(if cum { hc::t_cdf(x, df) } else { sp::t_pdf(x, df) }))
    }),
    // T.DIST.2T·T.DIST.RT 는 밀도를 수치 적분한다 (hcnum, 대조 실험 dist4·tdist5)
    f("T.DIST.2T", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if x < 0.0 || df < 1.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(1.0 - 2.0 * hc::t_central(x, df))
    }),
    // T.DIST.RT 는 x < 0 이면 #NUM! (대조 실험 dist)
    f("T.DIST.RT", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1)).trunc();
        if df < 1.0 || x < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(0.5 - hc::t_central(x, df))
    }),
    // TDIST 는 자유도의 소수 부분을 버리지 않는다 (TDIST(10,2.9,2), 대조 실험 dist)
    f("TDIST", 3, 3, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let df = try_v!(num_arg(c, a, 1));
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
        7 | 8 | 10 | 11 => variance(kind, v, true),
        9 => num(v.iter().sum()),
        _ => Value::Error(ErrorKind::Value),
    }
}

fn agg(ctx: &mut Ctx, a: &[Value], mode: Agg, kind: u8) -> Value {
    let v = try_v!(collect_numbers(ctx, a, mode));
    if mode == Agg::NumbersA && matches!(kind, 7 | 8 | 10 | 11) {
        return variance(kind, &v, false);
    }
    basic_aggregate(kind, &v)
}

/// 한셀의 분산: 표본 (nΣx² − (Σx)²) / (n(n−1)), 모집단 (nΣx² − (Σx)²) / n². 두 번 훑는 식이 아니라
/// 큰 수에서 자릿수를 잃는다: VAR(100000001,100000002) = 0, VAR(100000001,100000002,100000004) = 2.666…
/// (대조 실험 variance). VAR·VARP·STDEV·STDEVP(`clamp`)는 음수를 0 으로, VARA·VARPA 는 음수 그대로
/// (VARA(1000000000.1,1000000000.2,1000000000.3) = −341.33…), 그 제곱근 STDEVA 는 #NUM!.
/// `kind`: 7 표본 표준편차, 8 모집단 표준편차, 10 표본 분산, 11 모집단 분산.
pub fn variance(kind: u8, v: &[f64], clamp: bool) -> Value {
    let sample = matches!(kind, 7 | 10);
    if v.len() < if sample { 2 } else { 1 } {
        return Value::Error(ErrorKind::Div0);
    }
    let var = naive_var(v, sample, clamp);
    if matches!(kind, 7 | 8) {
        if var < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(var.sqrt())
    } else {
        num(var)
    }
}

/// [`variance`] 의 식 (n ≥ 2 표본, n ≥ 1 모집단)
pub fn naive_var(v: &[f64], sample: bool, clamp: bool) -> f64 {
    let n = v.len() as f64;
    let (mut s, mut q) = (0.0, 0.0);
    for x in v {
        s += x;
        q += x * x;
    }
    let d = n * q - s * s;
    let var = if sample { d / (n * (n - 1.0)) } else { d / (n * n) };
    // 넘쳐서 NaN 이 되면 0 (VAR(1E+300,1E+300) = 0, 대조 실험 stat)
    if var.is_nan() || (clamp && var < 0.0) { 0.0 } else { var }
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

/// 한셀의 MODE.MULT 는 배열이 아니라 값 하나: 가장 많이 나온 값들 가운데 차례상 마지막 칸의 값
/// (MODE.MULT(1,2,2,3,3) = 3, 배열 수식이어도 {3;3;3}, 대조 실험 arrays·newer).
fn mode_mult(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let mut best: Option<(f64, usize)> = None;
    for x in &v {
        let c = v.iter().filter(|y| *y == x).count();
        if c > 1 && best.is_none_or(|(_, bc)| c >= bc) {
            best = Some((*x, c));
        }
    }
    match best {
        Some((x, _)) => Value::Number(x),
        None => Value::Error(ErrorKind::NA),
    }
}

fn kurt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let n = v.len() as f64;
    if v.len() < 4 {
        return Value::Error(ErrorKind::Div0);
    }
    // 표준편차는 [`naive_var`] (대조 실험 variance)
    let m = mean(&v);
    let s = naive_var(&v, true, true).sqrt();
    if s == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let mut sum4 = 0.0;
    for x in &v {
        let z = (x - m) / s;
        sum4 += z * z * z * z;
    }
    num((n * (n + 1.0) * sum4) / ((n - 1.0) * (n - 2.0) * (n - 3.0))
        - (3.0 * (n - 1.0) * (n - 1.0)) / ((n - 2.0) * (n - 3.0)))
}

/// SKEW 는 [`naive_var`] 의 표준편차로 나눈다. SKEW.P 는 두 번 훑는 모멘트 m3 / m2^1.5 를 소수 9자리로
/// 반올림한다 (엑셀 2010 함수의 한셀 구현이 대개 그렇다). n < 3 이면 #DIV/0!, m2 = 0 이면 #NUM!
/// (대조 실험 variance·stat).
fn skew(ctx: &mut Ctx, a: &[Value], population: bool) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    let n = v.len() as f64;
    if v.len() < 3 {
        return Value::Error(ErrorKind::Div0);
    }
    let m = mean(&v);
    if population {
        let (mut m2, mut m3) = (0.0, 0.0);
        for x in &v {
            let d = x - m;
            m2 += d * d;
            m3 += d * d * d;
        }
        m2 /= n;
        m3 /= n;
        if m2 == 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        return num(round9(m3 / m2.mpow(1.5)));
    }
    let s = naive_var(&v, true, true).sqrt();
    if s == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let sum3: f64 = v.iter().map(|x| ((x - m) / s).mpowi(3)).sum();
    num(n / ((n - 1.0) * (n - 2.0)) * sum3)
}

/// 소수 9자리 반올림: round(x × 10⁹) × 10⁻⁹. 한셀이 엑셀 2010 새 통계 함수(SKEW.P, COVARIANCE.S,
/// GAMMA, PHI …)의 결과에 쓴다 (0.381801786 → 0.38180178600000003, 대조 실험).
pub fn round9(x: f64) -> f64 {
    (x * 1e9).round() * 1e-9
}

/// 배열 인수(참조 포함)의 숫자
fn array_numbers(ctx: &mut Ctx, v: &Value) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for x in flatten(ctx, v) {
        match x {
            Value::Number(n) => out.push(n),
            Value::Error(e) => return Err(e),
            Value::Text(_) | Value::Bool(_) | Value::Empty
                // 직접 준 스칼라는 숫자로 바꾼다
                if !matches!(v, Value::Ref(_) | Value::Array(_)) => {
                    out.push(ctx.num(&x)?);
                }
            _ => {}
        }
    }
    Ok(out)
}

pub fn large_small(v: &[f64], k: f64, large: bool) -> Value {
    // 한셀은 k 의 소수점 아래를 버린다 (LARGE(…,2.5) = 두 번째, 대조 실험 stat)
    let k = k.trunc();
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

/// 한셀의 PERCENTILE.EXC: 순위 k(n+1) 이 1 보다 작거나 n 보다 커도 오류가 아니다. 범위 밖 칸을 0 으로 보고
/// 보간한다 (PERCENTILE.EXC({1,1,2,…,9},0.05) = 0.55, 0.95 → 9 + 0.45 × (0 − 9) = 4.95, 대조 실험 stat).
pub fn percentile_exc(v: &[f64], k: f64) -> Value {
    if v.is_empty() || k <= 0.0 || k >= 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len() as f64;
    let r = (n + 1.0) * k;
    let i = r.floor();
    let frac = r - i;
    // 1부터 센 i 번째와 i+1 번째 (범위 밖은 0)
    let at = |j: f64| if j >= 1.0 && j <= n { s[j as usize - 1] } else { 0.0 };
    let (a, b) = (at(i), at(i + 1.0));
    if frac == 0.0 {
        return num(a);
    }
    num(a + frac * (b - a))
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
        // 한셀: 1·3 사분위는 보간하지 않고 순위 q(n+1)/4 의 정수 부분 칸 (대조 실험 stat·misc4)
        if q != 2.0 {
            let mut s = v.to_vec();
            s.sort_by(f64::total_cmp);
            let r = (q * (s.len() as f64 + 1.0) / 4.0).floor();
            if r < 1.0 || r > s.len() as f64 {
                return Value::Error(ErrorKind::Num);
            }
            return num(s[r as usize - 1]);
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
    let exact = s.contains(&x);
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
    // PERCENTRANK·PERCENTRANK.INC 는 반올림, .EXC 는 버림. 결과는 정수 × 10^-자릿수
    // (PERCENTRANK(…,4) = 5/9 → 0.556, PERCENTRANK.EXC(…,2.7,4) = 0.33630000000000004, 대조 실험 stat)
    let p = 10f64.mpowi(sig as i32);
    let k = if inclusive { (r * p).round() } else { (r * p).floor() };
    num(k * 10f64.mpowi(-(sig as i32)))
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
        // 여러 영역 참조는 #VALUE! (대조 실험 stat)
        Value::Ref(r) if r.len() > 1 => Value::Error(ErrorKind::Value),
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
    if !a.len().is_multiple_of(2) {
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
    // 범위가 참조가 아니면 #VALUE! (AVERAGEIF(1,1), 말뭉치 probe3)
    if !matches!(a[0], Value::Ref(_)) {
        return Value::Error(ErrorKind::Value);
    }
    let target = match opt(a, 2) {
        Some(t) => resized(&a[0], t),
        None => a[0].clone(),
    };
    let (_, _, mask) = try_v!(ifs_mask(ctx, &[(&a[0], &a[1])]));
    let vals = try_v!(grid(ctx, &target));
    // 평균 범위의 오류 칸은 건너뛴다 (AVERAGEIF(Data!A1:A10,">3",Data!H1:H10) = 5.67, 대조 실험 stat)
    let mut nums = Vec::new();
    for (m, v) in mask.iter().zip(&vals.data) {
        if *m && let Value::Number(n) = v {
            nums.push(*n);
        }
    }
    basic_aggregate(1, &nums)
}

/// AVERAGEIFS·MAXIFS·MINIFS (대상 범위, (조건 범위, 조건)…)
fn target_ifs(ctx: &mut Ctx, a: &[Value], kind: u8) -> Value {
    if a.len() % 2 != 1 || std::iter::once(&a[0]).chain(a[1..].iter().step_by(2)).any(|v| !matches!(v, Value::Ref(_))) {
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
        if *m && let Value::Number(n) = v {
            nums.push(*n);
        }
    }
    basic_aggregate(kind, &nums)
}

// ---------------------------------------------------------------------------
// 상관·회귀
// ---------------------------------------------------------------------------

/// 합 n, Σx, Σy, Σxy, Σx², Σy²
fn sums2(x: &[f64], y: &[f64]) -> [f64; 6] {
    let mut s = [x.len() as f64, 0.0, 0.0, 0.0, 0.0, 0.0];
    for (p, q) in x.iter().zip(y) {
        s[1] += p;
        s[2] += q;
        s[3] += p * q;
        s[4] += p * p;
        s[5] += q * q;
    }
    s
}

/// CORREL·PEARSON·RSQ: (nΣxy − ΣxΣy) / √((nΣx² − (Σx)²)(nΣy² − (Σy)²)) — 분산처럼 한 번에 모은 합으로
/// (대조 실험 stat·variance)
fn correl(ctx: &mut Ctx, a: &[Value], square: bool) -> Value {
    let (x, y) = try_v!(paired_numbers(ctx, &a[0], &a[1]));
    if x.is_empty() {
        return Value::Error(ErrorKind::Div0);
    }
    let [n, sx, sy, sxy, sxx, syy] = sums2(&x, &y);
    let d = ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
    if d == 0.0 || d.is_nan() {
        return Value::Error(ErrorKind::Div0);
    }
    let r = (n * sxy - sx * sy) / d;
    num(if square { r * r } else { r })
}

/// COVAR·COVARIANCE.P: (Σxy − ΣxΣy/n) / n. COVARIANCE.S 는 두 번 훑어 (n−1) 로 나누고 소수 9자리로
/// 반올림하며, 오류 값이 있으면 #VALUE!, 두 범위의 숫자 개수가 다르면 #N/A (대조 실험 stat).
fn covar(ctx: &mut Ctx, a: &[Value], sample: bool) -> Value {
    if sample {
        let mut cols = [Vec::new(), Vec::new()];
        for (k, v) in a.iter().take(2).enumerate() {
            for x in flatten(ctx, v) {
                match x {
                    Value::Number(n) => cols[k].push(n),
                    Value::Error(_) => return Value::Error(ErrorKind::Value),
                    _ => {}
                }
            }
        }
        let [x, y] = cols;
        if flatten(ctx, &a[0]).len() != flatten(ctx, &a[1]).len() || x.len() != y.len() {
            return Value::Error(ErrorKind::NA);
        }
        if x.len() < 2 {
            return Value::Error(ErrorKind::Div0);
        }
        let (mx, my) = (mean(&x), mean(&y));
        let s: f64 = x.iter().zip(&y).map(|(p, q)| (p - mx) * (q - my)).sum();
        return num(round9(s / (x.len() as f64 - 1.0)));
    }
    let (x, y) = try_v!(paired_numbers(ctx, &a[0], &a[1]));
    if x.is_empty() {
        return Value::Error(ErrorKind::Div0);
    }
    let [n, sx, sy, sxy, ..] = sums2(&x, &y);
    num((sxy - sx * sy / n) / n)
}

/// 단순 회귀: 0 기울기, 1 절편, 2 STEYX, 3 예측값 (y, x 순서). 기울기 (nΣxy − ΣxΣy)/(nΣx² − (Σx)²),
/// 분모가 0 이하면 #DIV/0!, 절편 ȳ − b x̄, STEYX √((nΣy² − (Σy)² − B²/C) / (n(n−2))) (대조 실험 stat).
fn regression(ctx: &mut Ctx, ys: &Value, xs: &Value, what: u8, at: f64) -> Value {
    let (y, x) = try_v!(paired_numbers(ctx, ys, xs));
    if x.is_empty() || (what == 2 && x.len() < 3) {
        return Value::Error(ErrorKind::Div0);
    }
    let [n, sx, sy, sxy, sxx, syy] = sums2(&x, &y);
    let b = n * sxy - sx * sy;
    let c = n * sxx - sx * sx;
    if c <= 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let slope = b / c;
    match what {
        0 => num(slope),
        1 => num(mean(&y) - slope * mean(&x)),
        2 => {
            let v = (n * syy - sy * sy - b * b / c) / (n * (n - 2.0));
            num(v.max(0.0).sqrt())
        }
        _ => num(mean(&y) - slope * mean(&x) + slope * at),
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

/// (y, x 열들, y 의 모양)
type RegressionInputs = (Vec<f64>, Vec<Vec<f64>>, (usize, usize));

fn regression_inputs(ctx: &mut Ctx, a: &[Value], log: bool) -> Result<RegressionInputs, ErrorKind> {
    let ya = to_array(ctx, &a[0])?;
    let mut y = Vec::new();
    for v in &ya.data {
        match v {
            Value::Number(n) => y.push(if log {
                if *n <= 0.0 {
                    return Err(ErrorKind::Num);
                }
                n.mln()
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
    let tr = |v: f64| if log { v.mexp() } else { v };
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
    // 한셀은 상수항이 없을 때도 평균을 뺀 전체 제곱합을 쓴다 (R² 가 음수가 될 수 있다:
    // LINEST(K,A,FALSE,TRUE) 의 R² = −0.704, 대조 실험 arrays)
    let sstot: f64 = y.iter().map(|v| (v - my) * (v - my)).sum();
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
    // 표준 오차 행: sey × √((XᵀX)⁻¹ 의 대각), 계수와 같은 차례 (m_k … m_1, b)
    let inv = xtx_inverse(&xs, with_const);
    for j in (0..k).rev() {
        data.push(match &inv {
            Some(m) => num(sey * m[j * (k + with_const as usize) + j].sqrt()),
            None => Value::Error(ErrorKind::NA),
        });
    }
    if with_const {
        data.push(match &inv {
            Some(m) => num(sey * m[k * (k + 1) + k].sqrt()),
            None => Value::Error(ErrorKind::NA),
        });
    } else {
        data.push(Value::Error(ErrorKind::NA));
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

/// (XᵀX)⁻¹ (X 의 열: x_1 … x_k, 상수항이면 1). 가우스–요르단, 특이하면 None
fn xtx_inverse(xs: &[Vec<f64>], with_const: bool) -> Option<Vec<f64>> {
    let k = xs.len() + with_const as usize;
    let n = xs.first().map_or(0, |x| x.len());
    let col = |j: usize, i: usize| if j < xs.len() { xs[j][i] } else { 1.0 };
    let mut m = vec![0.0; k * k];
    for p in 0..k {
        for q in 0..k {
            m[p * k + q] = (0..n).map(|i| col(p, i) * col(q, i)).sum();
        }
    }
    let mut inv = vec![0.0; k * k];
    for i in 0..k {
        inv[i * k + i] = 1.0;
    }
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
                inv.swap(piv * k + q, c * k + q);
            }
        }
        let d = m[c * k + c];
        for q in 0..k {
            m[c * k + q] /= d;
            inv[c * k + q] /= d;
        }
        for r in 0..k {
            if r != c {
                let f = m[r * k + c];
                if f != 0.0 {
                    for q in 0..k {
                        m[r * k + q] -= f * m[c * k + q];
                        inv[r * k + q] -= f * inv[c * k + q];
                    }
                }
            }
        }
    }
    Some(inv)
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
            num(if log { s.mexp() } else { s })
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
    num(if cum { hc::norm_cdf(z) } else { hc::norm_pdf(z) / s })
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
    let z = (x.mln() - m) / s;
    // LOGNORM.DIST 는 한셀 근사 정규 누적, 오래된 LOGNORMDIST 는 정확하다 (대조 실험 dist)
    num(if cum && has_cum {
        hc::norm_cdf(z)
    } else if cum {
        sp::norm_s_cdf(z)
    } else {
        hc::norm_pdf(z) / (s * x)
    })
}

fn lognorm_inv(ctx: &mut Ctx, a: &[Value]) -> Value {
    let p = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    let s = try_v!(num_arg(ctx, a, 2));
    if p <= 0.0 || p >= 1.0 || s <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num((m + s * sp::norm_s_inv(p)).mexp())
}

fn confidence(ctx: &mut Ctx, a: &[Value], t: bool) -> Value {
    let alpha = try_v!(num_arg(ctx, a, 0));
    let sd = try_v!(num_arg(ctx, a, 1));
    let n = try_v!(num_arg(ctx, a, 2)).trunc();
    // CONFIDENCE.T 는 n = 1 을 먼저 본다 (CONFIDENCE.T(1,1,1) = #DIV/0!)
    if t && n == 1.0 {
        return Value::Error(ErrorKind::Div0);
    }
    if alpha <= 0.0 || alpha >= 1.0 || sd <= 0.0 || n < 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    if t {
        return num(sp::t_inv(1.0 - alpha / 2.0, n - 1.0) * sd / n.sqrt());
    }
    // CONFIDENCE·CONFIDENCE.NORM 은 한셀 근사 정규 누적을 이분법으로 푼다 (결과가 k/2ⁿ, 대조 실험 norm4)
    num(hc::norm_inv_bisect(1.0 - alpha / 2.0) * sd / n.sqrt())
}

/// ZTEST: 표준편차 0 은 #NUM! (대조 실험 dist)
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
        return Value::Error(ErrorKind::Num);
    }
    num(1.0 - hc::norm_cdf((m - x) / (sigma / n.sqrt())))
}

fn expon_dist(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let l = try_v!(num_arg(ctx, a, 1));
    let cum = try_v!(bool_or(ctx, a, 2, true));
    if x < 0.0 || l <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    num(if cum { 1.0 - (-l * x).mexp() } else { l * (-l * x).mexp() })
}

fn weibull(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let al = try_v!(num_arg(ctx, a, 1));
    let be = try_v!(num_arg(ctx, a, 2));
    let cum = try_v!(bool_or(ctx, a, 3, true));
    if x < 0.0 || al <= 0.0 || be <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    // 밀도의 x = 0 은 #NUM! (WEIBULL(0,1,1,FALSE), 대조 실험 dist)
    if x == 0.0 && !cum {
        return Value::Error(ErrorKind::Num);
    }
    let t = (x / be).mpow(al);
    num(if cum {
        1.0 - (-t).mexp()
    } else {
        al / be.mpow(al) * x.mpow(al - 1.0) * (-t).mexp()
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
    // 밀도의 x = 0 은 0 (GAMMADIST(0,1,1,FALSE) = 0, 대조 실험 dist)
    if x == 0.0 && !cum {
        return Value::Number(0.0);
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
    // 확률 0·1 은 #NUM! (BETAINV(0,2,3), 대조 실험 dist)
    if p <= 0.0 || p >= 1.0 || al <= 0.0 || be <= 0.0 || lo >= hi {
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

/// BINOM.INV·CRITBINOM: 확률과 기준은 0 과 1 을 뺀 사이. 누적 확률이 기준 이상(BINOM.INV)·초과
/// (CRITBINOM)인 가장 작은 수 (CRITBINOM(10,0.5,2^-10) = 1, BINOM.INV 는 0, 대조 실험 dist)
fn binom_inv(ctx: &mut Ctx, a: &[Value], strict: bool) -> Value {
    let n = try_v!(num_arg(ctx, a, 0)).trunc();
    let p = try_v!(num_arg(ctx, a, 1));
    let alpha = try_v!(num_arg(ctx, a, 2));
    if n < 0.0 || p <= 0.0 || p >= 1.0 || alpha <= 0.0 || alpha >= 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = 0.0;
    let mut k = 0.0;
    while k <= n {
        s += sp::binom_pmf(k, n, p);
        if if strict { s > alpha } else { s >= alpha * (1.0 - 1e-15) } {
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
    // 누적 확률의 차, 누적은 불완전 베타 근사 (BINOM.DIST.RANGE(60,0.75,48) = 0.083974969, 대조 실험 dist)
    let cdf = |k: f64| {
        if k < 0.0 {
            0.0
        } else if k >= n {
            1.0
        } else {
            hc::betai(n - k, k + 1.0, 1.0 - p)
        }
    };
    num(round9(cdf(s2) - cdf(s1 - 1.0)))
}

fn negbinom(ctx: &mut Ctx, a: &[Value], new: bool) -> Value {
    let fl = try_v!(num_arg(ctx, a, 0)).trunc();
    let s = try_v!(num_arg(ctx, a, 1)).trunc();
    let p = try_v!(num_arg(ctx, a, 2));
    let cum = if new { try_v!(bool_or(ctx, a, 3, true)) } else { false };
    // 확률 1 도 받는다 (NEGBINOM.DIST(10,5,1,TRUE) = 1, 대조 실험 dist)
    if fl < 0.0 || s < 1.0 || p <= 0.0 || p > 1.0 {
        return Value::Error(ErrorKind::Num);
    }
    let pmf = |f: f64| sp::binom_coef(f + s - 1.0, s - 1.0) * p.mpow(s) * (1.0 - p).mpow(f);
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
    // 평균이 e^-20 보다 작으면 거듭제곱 쪽만 e^-20 으로 (POISSON(1,0,FALSE) = e^-20, 누적은 1 을
    // 조금 넘는다, 대조 실험 misc4)
    let floor_l = (-20.0f64).mexp();
    if m < floor_l {
        let mut s = 0.0;
        let mut term = (-m).mexp();
        let mut k = 0.0;
        let pmf_at = |k: f64| (k * floor_l.mln() - m - sp::ln_gamma(k + 1.0)).mexp();
        if !cum {
            return num(if x == 0.0 { term } else { pmf_at(x) });
        }
        while k <= x {
            s += if k == 0.0 { term } else { pmf_at(k) };
            k += 1.0;
        }
        term = s;
        return num(term);
    }
    if cum {
        num(sp::gamma_q(x + 1.0, m))
    } else {
        num((x * m.mln() - m - sp::ln_gamma(x + 1.0)).mexp())
    }
}

/// CHIDIST·CHISQ.DIST.RT: 자유도의 소수 부분을 버리지 않는다 (CHIDIST(0.5,1.5), 대조 실험 dist)
fn chi_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let df = try_v!(num_arg(ctx, a, 1));
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

/// CHITEST·CHISQ.TEST: 크기만 같으면 모양은 달라도 된다 ({1;2} 와 {1,2}). CHISQ.TEST 는 자유도를 늘
/// 칸 수 − 1 로 하고 소수 9자리로 반올림한다 (2차원 표에서도, 대조 실험 dist).
fn chitest(ctx: &mut Ctx, a: &[Value], new: bool) -> Value {
    let act = try_v!(to_array(ctx, &a[0]));
    let exp = try_v!(to_array(ctx, &a[1]));
    if act.data.len() != exp.data.len() {
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
    let df = if new || act.rows == 1 || act.cols == 1 {
        (act.rows * act.cols) as f64 - 1.0
    } else {
        (act.rows as f64 - 1.0) * (act.cols as f64 - 1.0)
    };
    if df < 1.0 {
        return Value::Error(ErrorKind::NA);
    }
    let p = sp::gamma_q(df / 2.0, chi / 2.0);
    num(if new { round9(p) } else { p })
}

/// FDIST·F.DIST.RT: 자유도의 소수 부분을 버리지 않는다 (FDIST(3.5,2.5,4.9), 대조 실험 dist)
fn f_rt(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d1 = try_v!(num_arg(ctx, a, 1));
    let d2 = try_v!(num_arg(ctx, a, 2));
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
    // 오른쪽 꼬리 q (FDIST) 와 1 − q 가운데 작은 것의 두 배 — 왼쪽 꼬리가 아주 작으면 1 − q 에서 자릿수를
    // 잃는다 (F.TEST = 1.6671777736121385E-8, 대조 실험 dist)
    let q = 1.0 - sp::f_cdf(fv, x.len() as f64 - 1.0, y.len() as f64 - 1.0);
    num(2.0 * q.min(1.0 - q))
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
            let df = se2 * se2 / ((v1 / n1).mpowi(2) / (n1 - 1.0) + (v2 / n2).mpowi(2) / (n2 - 1.0));
            (diff / se2.sqrt(), df)
        }
    };
    // 한셀의 형식 3(이분산)은 꼬리 수와 관계없이 한쪽 값 (TTEST(…,2,3) 이 (…,1,3) 과 같다, 대조 실험 dist)
    let tails = if kind == 3.0 { 1.0 } else { tails };
    num(tails * sp::t_tail(t.abs(), df))
}

#[allow(dead_code)]
fn unused(_: Rc<Array>) {}
