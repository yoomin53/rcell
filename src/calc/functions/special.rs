// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수치 핵심 함수: 감마·베타·오차 함수, 정규 분포, 불완전 감마·베타와 그 역함수.
//!
//! 급수와 연분수(Lentz 방법), 정규 분포 역함수는 Wichura 의 AS241 (유효숫자 약 16자리),
//! 나머지 역함수는 이분법으로 좁힌 뒤 뉴턴 방법으로 다듬는다.

use crate::calc::fm::Fm;
use std::f64::consts::PI;

const EPS: f64 = 1e-16;
const TINY: f64 = 1e-300;

/// ln Γ(x), x > 0
pub fn ln_gamma(x: f64) -> f64 {
    if x <= 0.0 {
        return f64::NAN;
    }
    if x < 15.0 {
        // 올림: ln Γ(x) = ln Γ(x+n) - ln(x(x+1)…(x+n-1))
        let mut prod = 1.0;
        let mut y = x;
        while y < 15.0 {
            prod *= y;
            y += 1.0;
        }
        return stirling(y) - prod.mln();
    }
    stirling(x)
}

/// 스털링 급수 (x ≥ 15 에서 오차 < 1e-17)
fn stirling(x: f64) -> f64 {
    const C: [f64; 8] = [
        1.0 / 12.0,
        -1.0 / 360.0,
        1.0 / 1260.0,
        -1.0 / 1680.0,
        1.0 / 1188.0,
        -691.0 / 360_360.0,
        1.0 / 156.0,
        -3617.0 / 122_400.0,
    ];
    let x2 = x * x;
    let mut s = 0.0;
    let mut p = x;
    for c in C {
        s += c / p;
        p *= x2;
    }
    (x - 0.5) * x.mln() - x + 0.5 * (2.0 * PI).mln() + s
}

/// Γ(x)
pub fn gamma(x: f64) -> f64 {
    if x == x.floor() {
        if x <= 0.0 {
            return f64::NAN;
        }
        if x <= 171.0 {
            let mut r = 1.0;
            let mut i = 2.0;
            while i < x {
                r *= i;
                i += 1.0;
            }
            return r;
        }
        return f64::INFINITY;
    }
    if x < 0.5 {
        // 반사 공식
        return PI / ((PI * x).msin() * gamma(1.0 - x));
    }
    if x > 171.7 {
        return f64::INFINITY;
    }
    // 작은 x 는 올려서 정밀도를 지킨다
    let mut y = x;
    let mut prod = 1.0;
    while y < 15.0 {
        prod *= y;
        y += 1.0;
    }
    stirling(y).mexp() / prod
}

/// ln B(a, b)
pub fn ln_beta(a: f64, b: f64) -> f64 {
    ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b)
}

/// 오차 함수
pub fn erf(x: f64) -> f64 {
    if x < 0.0 {
        return -erf(-x);
    }
    if x > 6.0 {
        return 1.0;
    }
    if x <= 2.5 {
        erf_series(x)
    } else {
        1.0 - erfc_cf(x)
    }
}

/// 여오차 함수
pub fn erfc(x: f64) -> f64 {
    if x < 0.0 {
        return 2.0 - erfc(-x);
    }
    // 1.5 넘으면 1 − erf 가 자릿수를 잃는다 — 연분수 (ERFC(2.5), 대조 실험 engineering)
    if x <= 1.5 {
        1.0 - erf_series(x)
    } else if x > 27.0 {
        0.0
    } else {
        erfc_cf(x)
    }
}

/// erf(x) = 2/√π · e^{-x²} · Σ 2ⁿ x^{2n+1} / (1·3·…·(2n+1)) — 모든 항이 양수
fn erf_series(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    let mut n = 0.0;
    loop {
        n += 1.0;
        term *= 2.0 * x2 / (2.0 * n + 1.0);
        sum += term;
        if term < sum * EPS {
            break;
        }
        if n > 500.0 {
            break;
        }
    }
    2.0 / PI.sqrt() * (-x2).mexp() * sum
}

/// erfc 연분수: e^{-x²}/√π · 1/(x + (1/2)/(x + 1/(x + (3/2)/(x + …))))
fn erfc_cf(x: f64) -> f64 {
    // 수정 Lentz
    let mut f = x;
    if f == 0.0 {
        f = TINY;
    }
    let mut c = f;
    let mut d = 0.0;
    for n in 1..500 {
        let an = n as f64 / 2.0;
        d = x + an * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = x + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() < EPS {
            break;
        }
    }
    (-x * x).mexp() / PI.sqrt() / f
}

/// 표준 정규 누적 분포
pub fn norm_s_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// 표준 정규 밀도
pub fn norm_s_pdf(z: f64) -> f64 {
    (-0.5 * z * z).mexp() / (2.0 * PI).sqrt()
}

/// 표준 정규 분포의 역함수 (Wichura AS241 PPND16)
pub fn norm_s_inv(p: f64) -> f64 {
    if p <= 0.0 || p >= 1.0 {
        return f64::NAN;
    }
    let q = p - 0.5;
    if q.abs() <= 0.425 {
        let r = 0.180625 - q * q;
        return q
            * (((((((r * 2_509.080_928_730_122_7 + 33_430.575_583_588_13) * r
                + 67_265.770_927_008_7)
                * r
                + 45_921.953_931_549_87)
                * r
                + 13_731.693_765_509_46)
                * r
                + 1_971.590_950_306_551_3)
                * r
                + 133.141_667_891_784_38)
                * r
                + 3.387_132_872_796_366_5)
            / (((((((r * 5_226.495_278_852_854 + 28_729.085_735_721_943) * r
                + 39_307.895_800_092_71)
                * r
                + 21_213.794_301_586_597)
                * r
                + 5_394.196_021_424_751)
                * r
                + 687.187_007_492_057_9)
                * r
                + 42.313_330_701_600_91)
                * r
                + 1.0);
    }
    let mut r = if q < 0.0 { p } else { 1.0 - p };
    r = (-r.mln()).sqrt();
    let val = if r <= 5.0 {
        r -= 1.6;
        (((((((r * 7.745_450_142_783_414e-4 + 0.022_723_844_989_269_184) * r
            + 0.241_780_725_177_450_6)
            * r
            + 1.270_458_252_452_368_4)
            * r
            + 3.647_848_324_763_204_5)
            * r
            + 5.769_497_221_460_691)
            * r
            + 4.630_337_846_156_546)
            * r
            + 1.423_437_110_749_683_5)
            / (((((((r * 1.050_750_071_644_416_9e-9 + 5.475_938_084_995_345e-4) * r
                + 0.015_198_666_563_616_457)
                * r
                + 0.148_103_976_427_480_08)
                * r
                + 0.689_767_334_985_1)
                * r
                + 1.676_384_830_183_803_8)
                * r
                + 2.053_191_626_637_759)
                * r
                + 1.0)
    } else {
        r -= 5.0;
        (((((((r * 2.010_334_399_292_288_1e-7 + 2.711_555_568_743_487_6e-5) * r
            + 0.001_242_660_947_388_078_4)
            * r
            + 0.026_532_189_526_576_124)
            * r
            + 0.296_560_571_828_504_87)
            * r
            + 1.784_826_539_917_291_3)
            * r
            + 5.463_784_911_164_114)
            * r
            + 6.657_904_643_501_103)
            / (((((((r * 2.044_263_103_389_939_7e-15 + 1.421_511_758_316_446e-7) * r
                + 1.846_318_317_510_054_8e-5)
                * r
                + 7.868_691_311_456_133e-4)
                * r
                + 0.014_875_361_290_850_615)
                * r
                + 0.136_929_880_922_735_8)
                * r
                + 0.599_832_206_555_888)
                * r
                + 1.0)
    };
    if q < 0.0 { -val } else { val }
}

/// 정규화한 하부 불완전 감마 P(a, x)
pub fn gamma_p(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x < a + 1.0 {
        gamma_series(a, x)
    } else {
        1.0 - gamma_cf(a, x)
    }
}

/// GAMMA.DIST 의 누적: P(a, x) 를 끝까지 수렴시키되 앞에 곱하는 지수의 ln Γ 는 한셀 GAMMALN (C 런타임 lgamma) 이다 —
/// a = 1E4 근처에서 끝자리가 정확한 값과 1E-11 다르다 (GAMMA.DIST(10000,10000,1,TRUE) = 0.5013298083352052).
/// 아래쪽 급수의 지수가 넘쳐 0 이 되면 NaN (GAMMA.DIST(10,1000,1,TRUE) = #NUM!, 대조 실험 misc6)
pub fn gamma_p_hc(a: f64, x: f64) -> f64 {
    use crate::calc::hcmath::lgamma;
    if x <= 0.0 {
        return 0.0;
    }
    let lg = lgamma(a);
    if x < a + 1.0 {
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..10_000 {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * EPS {
                break;
            }
        }
        let e = (-x + a * x.mln() - lg).mexp();
        if e == 0.0 {
            return f64::NAN;
        }
        sum * e
    } else {
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..10_000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < EPS {
                break;
            }
        }
        1.0 - (-x + a * x.mln() - lg).mexp() * h
    }
}

/// CHIDIST·CHISQ.DIST.RT 의 오른쪽 꼬리 Q(a, x): GAMMA.DIST 와 같은 한셀 GAMMALN 지수, 자유도가 아주 커도
/// (CHIDIST(1E8,1E8) = 0.4999811936804336) 끝까지 수렴시킨다
pub fn gamma_q_hc(a: f64, x: f64) -> f64 {
    use crate::calc::hcmath::lgamma;
    if x <= 0.0 {
        return 1.0;
    }
    let max_iter = 1_000_000usize.max((50.0 * a.sqrt()) as usize);
    // a 가 크면 −x + a·ln x − lnΓ(a) 의 항들이 서로 지워져 자리를 잃는다 — 스털링 꼴로 차이를 바로 계산한다
    // (CHIDIST(1E8,1E8) = 0.4999811936804336, 대조 실험 conv9)
    let e = if a >= 50.0 {
        let t = (x - a) / a;
        let corr = 1.0 / (12.0 * a) - 1.0 / (360.0 * a * a * a) + 1.0 / (1260.0 * a.powi(5));
        (a * (t.ln_1p() - t) + 0.5 * a.mln() - 0.5 * (2.0 * std::f64::consts::PI).mln() - corr)
            .mexp()
    } else {
        (-x + a * x.mln() - lgamma(a)).mexp()
    };
    if x < a + 1.0 {
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..max_iter {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * EPS {
                break;
            }
        }
        1.0 - sum * e
    } else {
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..max_iter {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < EPS {
                break;
            }
        }
        e * h
    }
}

/// 정규화한 상부 불완전 감마 Q(a, x)
pub fn gamma_q(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    if x < a + 1.0 {
        1.0 - gamma_series(a, x)
    } else {
        gamma_cf(a, x)
    }
}

fn gamma_series(a: f64, x: f64) -> f64 {
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut del = sum;
    for _ in 0..10_000 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * EPS {
            break;
        }
    }
    sum * (-x + a * x.mln() - ln_gamma(a)).mexp()
}

fn gamma_cf(a: f64, x: f64) -> f64 {
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..10_000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    (-x + a * x.mln() - ln_gamma(a)).mexp() * h
}

/// 정규화한 불완전 베타 I_x(a, b)
pub fn beta_i(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (a * x.mln() + b * (1.0 - x).mln() - ln_beta(a, b)).mexp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_cf(x, a, b) / a
    } else {
        1.0 - bt * beta_cf(1.0 - x, b, a) / b
    }
}

fn beta_cf(x: f64, a: f64, b: f64) -> f64 {
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..10_000 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

/// 단조 증가 함수 f 의 f(x) = p 해 (lo..hi 안). 이분법 + 할선.
pub fn solve_increasing(f: impl Fn(f64) -> f64, p: f64, mut lo: f64, mut hi: f64) -> f64 {
    let mut flo = f(lo) - p;
    let mut fhi = f(hi) - p;
    if flo > 0.0 || fhi < 0.0 {
        // 범위 밖이면 가까운 쪽
        if flo > 0.0 {
            return lo;
        }
        return hi;
    }
    for _ in 0..400 {
        let mid = if (fhi - flo).abs() > 0.0 && (hi - lo) > 1e-300 {
            // 할선 후보가 구간 안이면 쓰고, 아니면 가운데
            let s = lo - flo * (hi - lo) / (fhi - flo);
            let m = 0.5 * (lo + hi);
            if s > lo && s < hi && (s - m).abs() < 0.45 * (hi - lo) {
                s
            } else {
                m
            }
        } else {
            0.5 * (lo + hi)
        };
        let fm = f(mid) - p;
        if fm == 0.0 {
            return mid;
        }
        if fm < 0.0 {
            lo = mid;
            flo = fm;
        } else {
            hi = mid;
            fhi = fm;
        }
        if (hi - lo) <= 1e-15 * hi.abs().max(1e-300) {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// P(a, x) = p 인 x
pub fn gamma_p_inv(p: f64, a: f64) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    // 위쪽 끝 찾기
    let mut hi = a.max(1.0);
    while gamma_p(a, hi) < p {
        hi *= 2.0;
        if hi > 1e300 {
            return f64::INFINITY;
        }
    }
    let x = solve_increasing(|x| gamma_p(a, x), p, 0.0, hi);
    newton_refine(
        x,
        p,
        |x| gamma_p(a, x),
        |x| (-x + (a - 1.0) * x.mln() - ln_gamma(a)).mexp(),
    )
}

/// Q(a, x) = q 인 x (위쪽 꼬리로 풀어 p 가 1 에 가까울 때 자릿수를 잃지 않는다)
pub fn gamma_q_inv(q: f64, a: f64) -> f64 {
    if q >= 1.0 {
        return 0.0;
    }
    if q <= 0.0 {
        return f64::INFINITY;
    }
    let mut hi = a.max(1.0);
    while gamma_q(a, hi) > q {
        hi *= 2.0;
        if hi > 1e300 {
            return f64::INFINITY;
        }
    }
    let x = solve_increasing(|x| -gamma_q(a, x), -q, 0.0, hi);
    newton_refine(
        x,
        -q,
        |x| -gamma_q(a, x),
        |x| (-x + (a - 1.0) * x.mln() - ln_gamma(a)).mexp(),
    )
}

/// I_x(a, b) = p 인 x
pub fn beta_i_inv(p: f64, a: f64, b: f64) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let x = solve_increasing(|x| beta_i(x, a, b), p, 0.0, 1.0);
    let lb = ln_beta(a, b);
    let r = newton_refine(
        x,
        p,
        |x| beta_i(x, a, b),
        |x| ((a - 1.0) * x.mln() + (b - 1.0) * (1.0 - x).mln() - lb).mexp(),
    );
    r.clamp(0.0, 1.0)
}

/// 뉴턴 방법으로 마지막 자릿수를 다듬는다 (나빠지면 원래 값)
pub fn newton_refine(x0: f64, p: f64, f: impl Fn(f64) -> f64, df: impl Fn(f64) -> f64) -> f64 {
    let mut x = x0;
    let mut err = (f(x) - p).abs();
    for _ in 0..8 {
        let d = df(x);
        if !(d.is_finite()) || d == 0.0 {
            break;
        }
        let nx = x - (f(x) - p) / d;
        if !nx.is_finite() {
            break;
        }
        let ne = (f(nx) - p).abs();
        if ne < err {
            x = nx;
            err = ne;
        } else {
            break;
        }
        if err == 0.0 {
            break;
        }
    }
    x
}

/// 이항 계수 C(n, k) (n, k 는 0 이상 정수): e^{GAMMALN 차이}, 2^53 보다 작으면 정수로 반올림한다
/// (COMBIN(1030,2) = 529935, COMBIN(1029,514) = 1.4298206865000652E+308, COMBIN(200,100) = 9.054851465609245E+58,
/// 대조 실험 misc4)
pub fn binom_coef(n: f64, k: f64) -> f64 {
    use crate::calc::hcmath::lgamma;
    if k < 0.0 || k > n {
        return 0.0;
    }
    let r = (lgamma(n + 1.0) - lgamma(k + 1.0) - lgamma(n - k + 1.0)).mexp();
    if r < 9.007_199_254_740_992e15 {
        r.round()
    } else {
        r
    }
}

/// 이항 확률 P(X = k): 조합 수를 e^{GAMMALN 차이}로 구해 p^k·q^(n−k) 를 곱한다 (한셀 BINOM.DIST(…,FALSE) 가 n = 20–1000 의
/// 6 경우 끝자리까지 같다; 정수 곱으로 구한 조합 수는 `BINOM.DIST(10,20,0.5,FALSE)` 의 끝자리가 다르다, 대조 실험 dist4·c3).
/// 조합 수가 넘치는 큰 n 은 로그 꼴 e^{ln C + k ln p + (n−k) ln q} (누적에서만 쓴다)
pub fn binom_pmf(k: f64, n: f64, p: f64) -> f64 {
    use crate::calc::hcmath::lgamma;
    if p == 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if p == 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    let lc = lgamma(n + 1.0) - lgamma(k + 1.0) - lgamma(n - k + 1.0);
    if lc <= 709.782_712_893_384 {
        return lc.mexp() * p.mpow(k) * (1.0 - p).mpow(n - k);
    }
    (lc + k * p.mln() + (n - k) * (1.0 - p).mln()).mexp()
}

/// t 분포 오른쪽 꼬리 P(T > t) (t ≥ 0)
pub fn t_tail(t: f64, df: f64) -> f64 {
    let x = df / (df + t * t);
    0.5 * beta_i(x, df / 2.0, 0.5)
}

/// t 분포 누적 P(T ≤ t)
pub fn t_cdf(t: f64, df: f64) -> f64 {
    if t >= 0.0 {
        1.0 - t_tail(t, df)
    } else {
        t_tail(-t, df)
    }
}

/// t 분포 밀도
pub fn t_pdf(t: f64, df: f64) -> f64 {
    (ln_gamma((df + 1.0) / 2.0)
        - ln_gamma(df / 2.0)
        - 0.5 * (df * PI).mln()
        - (df + 1.0) / 2.0 * (1.0 + t * t / df).mln())
    .mexp()
}

/// P(T ≤ t) = p 인 t
pub fn t_inv(p: f64, df: f64) -> f64 {
    if p == 0.5 {
        return 0.0;
    }
    if p < 0.5 {
        return -t_inv(1.0 - p, df);
    }
    // 오른쪽 꼬리 q = 1 - p = 0.5·I_x(df/2, 1/2), x = df/(df+t²)
    let q = 1.0 - p;
    let x = beta_i_inv(2.0 * q, df / 2.0, 0.5);
    let t = if x <= 0.0 {
        f64::INFINITY
    } else {
        (df * (1.0 - x) / x).sqrt()
    };
    newton_refine(t, p, |t| t_cdf(t, df), |t| t_pdf(t, df))
}

/// 양쪽 꼬리 확률 p2 = P(|T| > t) 인 t. p2 가 1 에 가까우면(작은 t) 가운데 확률 1 − p2 = I_y(1/2, ν/2),
/// y = t²/(ν+t²) 로 풀어 자릿수를 잃지 않는다 (TINV(0.999,1) = 0.00157079761872433, 대조 실험 dist4)
pub fn t_inv_two_tailed(p2: f64, df: f64) -> f64 {
    if p2 > 0.5 {
        let y = beta_i_inv(1.0 - p2, 0.5, df / 2.0);
        if y <= 0.0 {
            return 0.0;
        }
        return (df * y / (1.0 - y)).sqrt();
    }
    t_inv(1.0 - p2 / 2.0, df)
}

/// F 분포 누적
pub fn f_cdf(x: f64, d1: f64, d2: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    beta_i(d1 * x / (d1 * x + d2), d1 / 2.0, d2 / 2.0)
}

/// F 분포 밀도
pub fn f_pdf(x: f64, d1: f64, d2: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if d1 < 2.0 {
            f64::INFINITY
        } else if d1 == 2.0 {
            1.0
        } else {
            0.0
        };
    }
    ((d1 / 2.0) * (d1 / d2).mln() + (d1 / 2.0 - 1.0) * x.mln()
        - ((d1 + d2) / 2.0) * (1.0 + d1 * x / d2).mln()
        - ln_beta(d1 / 2.0, d2 / 2.0))
    .mexp()
}

/// F 분포 역함수 (누적 확률 p)
pub fn f_inv(p: f64, d1: f64, d2: f64) -> f64 {
    let x = beta_i_inv(p, d1 / 2.0, d2 / 2.0);
    if x >= 1.0 {
        return f64::INFINITY;
    }
    let f = d2 * x / (d1 * (1.0 - x));
    newton_refine(f, p, |v| f_cdf(v, d1, d2), |v| f_pdf(v, d1, d2))
}

/// 카이제곱 누적
pub fn chisq_cdf(x: f64, df: f64) -> f64 {
    gamma_p(df / 2.0, x / 2.0)
}

/// 카이제곱 밀도
pub fn chisq_pdf(x: f64, df: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if df < 2.0 {
            f64::INFINITY
        } else if df == 2.0 {
            0.5
        } else {
            0.0
        };
    }
    ((df / 2.0 - 1.0) * x.mln() - x / 2.0 - (df / 2.0) * 2f64.mln() - ln_gamma(df / 2.0)).mexp()
}

/// 감마 분포 밀도 (모양 a, 척도 b)
pub fn gamma_pdf(x: f64, a: f64, b: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    if x == 0.0 {
        return if a < 1.0 {
            f64::INFINITY
        } else if a == 1.0 {
            1.0 / b
        } else {
            0.0
        };
    }
    ((a - 1.0) * x.mln() - x / b - ln_gamma(a) - a * b.mln()).mexp()
}

/// 베타 분포 밀도 (0..1)
pub fn beta_pdf(x: f64, a: f64, b: f64) -> f64 {
    if !(0.0..=1.0).contains(&x) {
        return 0.0;
    }
    if x == 0.0 || x == 1.0 {
        let e = if x == 0.0 { a } else { b };
        return if e < 1.0 {
            f64::INFINITY
        } else if e == 1.0 {
            (-ln_beta(a, b)).mexp()
        } else {
            0.0
        };
    }
    ((a - 1.0) * x.mln() + (b - 1.0) * (1.0 - x).mln() - ln_beta(a, b)).mexp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1e-300)
    }

    #[test]
    fn gamma_values() {
        assert!(close(gamma(5.0), 24.0, 1e-15));
        assert!(close(gamma(0.5), PI.sqrt(), 1e-14));
        assert!(close(ln_gamma(10.0), 362_880f64.mln(), 1e-14));
        assert!(close(gamma(4.5), 11.631_728_396_567_448, 1e-13));
    }

    #[test]
    fn erf_values() {
        assert!(close(erf(1.0), 0.842_700_792_949_714_9, 1e-14));
        assert!(close(erfc(3.0), 2.209_049_699_858_544e-5, 1e-12));
        assert!(close(erf(0.1), 0.112_462_916_018_284_9, 1e-14));
    }

    #[test]
    fn normal() {
        assert!(close(norm_s_cdf(1.0), 0.841_344_746_068_542_9, 1e-14));
        assert!(close(norm_s_inv(0.975), 1.959_963_984_540_054, 1e-14));
        assert!(close(norm_s_inv(0.001), -3.090_232_306_167_813_6, 1e-13));
    }

    #[test]
    fn incomplete() {
        assert!(close(gamma_p(2.0, 3.0), 0.800_851_726_528_544, 1e-13));
        assert!(close(beta_i(0.4, 2.0, 3.0), 0.5248, 1e-13));
        assert!(close(t_inv(0.975, 10.0), 2.228_138_851_986_274, 1e-12));
        assert!(close(gamma_p_inv(0.5, 3.0), 2.674_060_313_723_561, 1e-12));
    }
}
