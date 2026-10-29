// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀이 정확한 식 대신 쓰는 근사 계산 (함수 대조 실험 dist4·norm4·norm5·tdist5·bessel4 에서 찾음).
//!
//! 엑셀 2010 에 새로 생긴 분포 함수 다수와 베셀 함수 일부는 오래된 교과서식 근사를 그대로 쓴다.
//! 결과가 참값과 1E-10–1E-7 다르므로 한셀과 같은 값을 내려면 같은 근사를 따라야 한다.
//!
//! - 불완전 감마·베타 (CHISQ.DIST·T.DIST·F.DIST 누적): 급수와 연분수를 상대 변화 3E-7 에서 멈추고
//!   (최대 100번), ln Γ 는 6항 란초시 근사. 오래된 CHIDIST·TDIST·FDIST 는 정확하다.
//! - 정규 누적 (NORM.S.DIST·NORM.DIST·NORMDIST·LOGNORM.DIST 누적, Z.TEST, CONFIDENCE.NORM):
//!   e^{-t²/2} 를 0..|z| 에서 8점 닫힌 뉴턴-코츠(무게는 소수 9자리로 반올림한 값)로 적분한다.
//!   칸 수를 1부터 하나씩 늘려 앞 칸 수의 적분과 차이가 3E-7 보다 작아지면 멈추고, 1/2.506628275 를
//!   곱한다. 그래서 큰 |z| 의 꼬리가 0 이 아니다 (NORM.S.DIST(−40,TRUE) = 8.87E-8). NORMSDIST 는 정확하다.
//! - t 분포 양쪽·오른쪽 꼬리 (T.DIST.2T·T.DIST.RT): 같은 적분을 (1+u²/ν)^{-(ν+1)/2} 에 하고 상수
//!   Γ((ν+1)/2)/(√(ν·3.141593)·Γ(ν/2)) 를 곱한다 (π 를 3.141593 으로).
//! - GAUSS: 상대 오차 1.2E-7 의 체비쇼프 근사 erfc (GAUSS(1E-9) = −1.5E-8).
//! - BESSELI: I_n(x) = (1/π')∫_0^{π'} 꼴의 적분 표현을 π' = 3.141592654 로 계산한 값.
//! - BESSELK: A&S 9.8.1–9.8.8 다항식 근사와 올림 점화식 (BESSELK(2,0) = 0.11389388).

// π·γ 를 일부러 짧게 쓴 상수가 한셀의 값이다
#![allow(clippy::approx_constant)]

use crate::calc::fm::Fm;

/// 급수·연분수를 멈추는 상대 변화, 적분 칸 늘리기를 멈추는 차이
const EPS: f64 = 3e-7;
const ITMAX: usize = 100;
const FPMIN: f64 = 1e-30;

/// 6항 란초시 근사 ln Γ(x), x > 0
pub fn gammln(x: f64) -> f64 {
    const COF: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.120_865_097_386_617_9e-2,
        -0.539_523_938_495_3e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5;
    let tmp = tmp - (x + 0.5) * tmp.mln();
    let mut ser = 1.000_000_000_190_015;
    for c in COF {
        y += 1.0;
        ser += c / y;
    }
    -tmp + (2.506_628_274_631_000_5 * ser / x).mln()
}

/// 정규화한 하부 불완전 감마 P(a, x)
pub fn gammp(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let gln = gammln(a);
    if x < a + 1.0 {
        let mut ap = a;
        let mut del = 1.0 / a;
        let mut sum = del;
        for _ in 0..ITMAX {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * EPS {
                break;
            }
        }
        sum * (-x + a * x.mln() - gln).mexp()
    } else {
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / FPMIN;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..=ITMAX {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < FPMIN {
                d = FPMIN;
            }
            c = b + an / c;
            if c.abs() < FPMIN {
                c = FPMIN;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < EPS {
                break;
            }
        }
        1.0 - (-x + a * x.mln() - gln).mexp() * h
    }
}

/// 불완전 베타의 연분수 (수정 Lentz)
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=ITMAX {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
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

/// 정규화한 불완전 베타 I_x(a, b), 0 ≤ x ≤ 1
pub fn betai(a: f64, b: f64, x: f64) -> f64 {
    let bt = if x == 0.0 || x == 1.0 {
        0.0
    } else {
        (gammln(a + b) - gammln(a) - gammln(b) + a * x.mln() + b * (1.0 - x).mln()).mexp()
    };
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
    }
}

/// t 분포 누적 (T.DIST 누적, 소수 9자리 반올림 전)
pub fn t_cdf(x: f64, df: f64) -> f64 {
    let b = betai(df / 2.0, 0.5, df / (df + x * x));
    if x > 0.0 { 1.0 - 0.5 * b } else { 0.5 * b }
}

/// F 분포 누적 (F.DIST 누적): 오른쪽 꼬리의 불완전 베타를 1 에서 뺀다
/// (F.DIST(1,1,1,TRUE) = 0.499999982, 대조 실험 dist4)
pub fn f_cdf(x: f64, d1: f64, d2: f64) -> f64 {
    1.0 - betai(d2 / 2.0, d1 / 2.0, d2 / (d2 + d1 * x))
}

/// 체비쇼프 근사 erfc (상대 오차 1.2E-7)
pub fn erfcc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let ans = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .mexp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// 8점 닫힌 뉴턴-코츠 무게 (751, 3577, 1323, 2989)/17280 을 소수 9자리로 반올림한 값 — 이 반올림
/// 때문에 작은 x 에서 정확한 무게보다 1E-12 쯤 다르다 (대조 실험 norm4·norm5 의 작은 x)
const NC8: [f64; 4] = [0.043_460_648, 0.207_002_315, 0.076_562_5, 0.172_974_537];

/// [a, b] 를 p 칸으로 나눈 8점 뉴턴-코츠 적분. 칸의 첫 점은 a + k·hh, 칸 안의 점은 거기서 h 씩
/// (a 가 음수면 점을 음수 쪽 끝에서부터 잰다). 무게에 칸 폭 hh 를 먼저 곱해 두고, 칸마다 무게 × 값을 점 차례로
/// 더한 합을 전체 합에 더한다 — 정규 왼쪽 꼬리(0.5 − 적분/c)의 끝자리가 이 순서와 C 런타임 exp 로 모두 같다
/// (대조 실험 norm4·norm5 의 z < −3 381 경우)
fn nc8(f: &impl Fn(f64) -> f64, a: f64, b: f64, p: usize) -> f64 {
    let hh = (b - a) / p as f64;
    let h = hh / 7.0;
    let w = NC8.map(|c| c * hh);
    let wf = [w[0], w[1], w[2], w[3], w[3], w[2], w[1], w[0]];
    let mut s = 0.0;
    for k in 0..p {
        let x0 = a + k as f64 * hh;
        let mut t = 0.0;
        for (i, wi) in wf.iter().enumerate() {
            t += wi * f(x0 + i as f64 * h);
        }
        s += t;
    }
    s
}

/// 칸 수를 1부터 하나씩 늘려 앞 칸 수의 값과 차이가 3E-7 보다 작아지면 멈춘다
/// (정규 누적의 칸 수가 바뀌는 x: 1.407, 3.035, 4.519, 6.029 …, 대조 실험 norm5)
pub fn integrate(f: impl Fn(f64) -> f64, x: f64) -> f64 {
    integrate_ab(f, 0.0, x)
}

/// [a, b] 적분 (칸 늘리기는 [`integrate`] 와 같다)
pub fn integrate_ab(f: impl Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    integrate_eps(f, a, b, EPS)
}

/// [a, b] 적분, 칸 늘리기를 멈추는 차이 eps
pub fn integrate_eps(f: impl Fn(f64) -> f64, a: f64, b: f64, eps: f64) -> f64 {
    // 99 칸까지 멈추지 않으면 #NUM! (NaN): NORM.S.DIST(160) 은 95 칸으로 되고 (170) 은 100 칸이 필요해 #NUM!,
    // NORMDIST(300,0,2) 94 칸 되고 (500,0,2) 150 칸 #NUM! (대조 실험 cmisc·c3)
    const LIMIT: usize = 99;
    let mut prev = nc8(&f, a, b, 1);
    for p in 2..=LIMIT {
        let q = nc8(&f, a, b, p);
        if (q - prev).abs() < eps {
            return q;
        }
        prev = q;
    }
    f64::NAN
}

/// 정규 밀도 e^{-z²/2}/2.506628275 (√(2π) 를 10자리로, NORM.S.DIST(2,FALSE) = 0.053990966505240066, 대조 실험 dist·norm4)
pub fn norm_pdf(z: f64) -> f64 {
    (-z * z / 2.0).mexp() / 2.506_628_275
}

/// 정규 누적 Φ(z) (한셀 근사)
pub fn norm_cdf(z: f64) -> f64 {
    norm_cdf_ms(z, 0.0, 1.0)
}

/// 정규 누적 (평균 m, 표준편차 s): z 로 바꾸지 않고 x 공간에서 [x, m] (또는 [m, x]) 를 x 쪽 끝에서부터 잰 점으로
/// e^{−d·d/2/s/s} (d = t − m) 를 적분하고 칸 늘리기도 x 공간의 적분으로 멈춘 뒤 0.5 ∓ 적분/s/2.506628275.
/// 그래서 s ≠ 1 이면 같은 z 라도 NORM.S.DIST 와 다르다 (NORMDIST(−18,0,2,TRUE) = 1.94E-10, NORM.S.DIST(−9) = 4.73E-9,
/// 대조 실험 n6)
pub fn norm_cdf_ms(x: f64, m: f64, s: f64) -> f64 {
    if x == m {
        return 0.5;
    }
    if x.is_nan() || m.is_nan() || s.is_nan() {
        return f64::NAN;
    }
    let f = |t: f64| {
        let d = t - m;
        (-d * d / 2.0 / s / s).mexp()
    };
    if x > m {
        0.5 + integrate_eps(f, m, x, EPS) / s / 2.506_628_275
    } else {
        0.5 - integrate_eps(f, x, m, EPS) / s / 2.506_628_275
    }
}

/// t 분포 상수 Γ((ν+1)/2)/(√(ν·3.141593)·Γ(ν/2)) 와 0..|x| 의 밀도 적분의 곱 (ν 는 정수). 밀도는
/// e^{−(ν+1)/2·ln(1+u²/ν)} 꼴이다 (pow 가 아니다 — 대조 실험 t6)
pub fn t_central(x: f64, df: f64) -> f64 {
    // 상수 = exp(ln Γ((ν+1)/2) − ½ln(ν·3.141593) − ln Γ(ν/2)), ln Γ 는 한셀 GAMMALN (C 런타임 lgamma) — 이 차례로
    // ν = 1–120·150·338 의 상수가 모두 같다 (대조 실험 t6·g6)
    use crate::calc::hcmath::lgamma;
    let k = (lgamma((df + 1.0) / 2.0) - 0.5 * (df * 3.141_593).mln() - lgamma(df / 2.0)).mexp();
    if x == 0.0 {
        return 0.0;
    }
    let e = -(df + 1.0) / 2.0;
    k * integrate(|u| (e * (1.0 + u * u / df).mln()).mexp(), x.abs())
}

/// CONFIDENCE.NORM 의 z: [0, 8] 이분법, |Φ(z) − p| < 3E-7 에서 멈춘다 (결과가 k/2ⁿ, 대조 실험 norm4·tdist5)
pub fn norm_inv_bisect(p: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 8.0f64);
    let mut mid = 4.0;
    for _ in 0..200 {
        mid = (lo + hi) / 2.0;
        let fv = norm_cdf(mid) - p;
        if fv.abs() < EPS {
            break;
        }
        if fv < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    mid
}

/// BESSELI: (π·I_n(x) + c·δ)/π', π' = 3.141592654, δ = π' − π, c = 짝수 n 이면 (−1)^{n/2} (홀수는 0).
/// 적분 표현 (1/π')∫_0^{π'} 을 [0, π] 의 참값과 [π, π'] 의 끝 조각으로 나눈 것
/// (BESSELI(0,2) = −1.3057E-10, BESSELI(1,1) = 0.5651591039186906, 대조 실험 bessel4)
pub fn bessel_i(n: i32, x: f64) -> f64 {
    const P2: f64 = 3.141_592_654;
    let pi = std::f64::consts::PI;
    let c = if n % 2 == 0 {
        if (n / 2) % 2 == 0 { 1.0 } else { -1.0 }
    } else {
        0.0
    };
    (pi * super::engineering::bessel_i(n, x) + c * (P2 - pi)) / P2
}

/// A&S 9.8.1·9.8.2 I0
fn as_i0(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 3.75 {
        let y = (x / 3.75) * (x / 3.75);
        1.0 + y
            * (3.515_622_9
                + y * (3.089_942_4
                    + y * (1.206_749_2 + y * (0.265_973_2 + y * (0.036_076_8 + y * 0.004_581_3)))))
    } else {
        let y = 3.75 / ax;
        (ax.mexp() / ax.sqrt())
            * (0.398_942_28
                + y * (0.013_285_92
                    + y * (0.002_253_19
                        + y * (-0.001_575_65
                            + y * (0.009_162_81
                                + y * (-0.020_577_06
                                    + y * (0.026_355_37
                                        + y * (-0.016_476_33 + y * 0.003_923_77))))))))
    }
}

/// A&S 9.8.3·9.8.4 I1
fn as_i1(x: f64) -> f64 {
    let ax = x.abs();
    let ans = if ax < 3.75 {
        let y = (x / 3.75) * (x / 3.75);
        ax * (0.5
            + y * (0.878_905_94
                + y * (0.514_988_69
                    + y * (0.150_849_34
                        + y * (0.026_587_33 + y * (0.003_015_32 + y * 0.000_324_11))))))
    } else {
        let y = 3.75 / ax;
        let a = 0.022_829_67 + y * (-0.028_953_12 + y * (0.017_876_54 - y * 0.004_200_59));
        let a = 0.398_942_28
            + y * (-0.039_880_24
                + y * (-0.003_620_18 + y * (0.001_638_01 + y * (-0.010_315_55 + y * a))));
        a * (ax.mexp() / ax.sqrt())
    };
    if x < 0.0 { -ans } else { ans }
}

/// A&S 9.8.5·9.8.6 K0 (x > 0)
fn as_k0(x: f64) -> f64 {
    if x <= 2.0 {
        let y = x * x / 4.0;
        (-(x / 2.0).mln() * as_i0(x))
            + (-0.577_215_66
                + y * (0.422_784_20
                    + y * (0.230_697_56
                        + y * (0.034_885_90
                            + y * (0.002_626_98 + y * (0.000_107_50 + y * 0.000_007_4))))))
    } else {
        let y = 2.0 / x;
        ((-x).mexp() / x.sqrt())
            * (1.253_314_14
                + y * (-0.078_323_58
                    + y * (0.021_895_68
                        + y * (-0.010_624_46
                            + y * (0.005_878_72 + y * (-0.002_515_40 + y * 0.000_532_08))))))
    }
}

/// A&S 9.8.7·9.8.8 K1 (x > 0)
fn as_k1(x: f64) -> f64 {
    if x <= 2.0 {
        let y = x * x / 4.0;
        ((x / 2.0).mln() * as_i1(x))
            + (1.0 / x)
                * (1.0
                    + y * (0.154_431_44
                        + y * (-0.672_785_79
                            + y * (-0.181_568_97
                                + y * (-0.019_194_02
                                    + y * (-0.001_104_04 + y * (-0.000_046_86)))))))
    } else {
        let y = 2.0 / x;
        ((-x).mexp() / x.sqrt())
            * (1.253_314_14
                + y * (0.234_986_19
                    + y * (-0.036_556_20
                        + y * (0.015_042_68
                            + y * (-0.007_803_53 + y * (0.003_256_14 + y * (-0.000_682_45)))))))
    }
}

/// BESSELK: K0·K1 근사에서 올림 점화식 K_{j+1} = K_{j−1} + (2j/x)K_j
pub fn bessel_k(n: i32, x: f64) -> f64 {
    let k0 = as_k0(x);
    if n == 0 {
        return k0;
    }
    let tox = 2.0 / x;
    let (mut bkm, mut bk) = (k0, as_k1(x));
    for j in 1..n {
        let bkp = bkm + j as f64 * tox * bk;
        bkm = bk;
        bk = bkp;
    }
    bk
}

/// 오데-에번스(AS 70) 정규 분위수 근사 (오차 1.5E-8). 한셀의 NORM.S.INV 는 정확하지만 T.INV·CHISQ.INV·
/// F.INV 의 시작 값은 이 근사로 구한다 (가운데에서 +1.45E-8, 대조 실험 x1·x3 의 반복 없는 구간)
pub fn norm_inv_oe(p: f64) -> f64 {
    let ps = if p > 0.5 { 1.0 - p } else { p };
    if ps == 0.5 {
        return 0.0;
    }
    let y = (1.0 / (ps * ps)).mln().sqrt();
    let num = (((y * -0.453_642_210_148e-4 - 0.204_231_210_245e-1) * y - 0.342_242_088_547) * y
        - 1.0)
        * y
        - 0.322_232_431_088;
    let den = (((y * 0.385_607_006_34e-2 + 0.103_537_752_850) * y + 0.531_103_462_366) * y
        + 0.588_581_570_495)
        * y
        + 0.993_484_626_060e-1;
    let x = y + num / den;
    if p < 0.5 { -x } else { x }
}

/// 한셀 역함수의 반복 (T.INV·CHISQ.INV·F.INV): 누적 확률을 소수 9자리로 반올림한 값이 p 와 1E-6 안이면
/// 그 x 를 내고, 아니면 핼리 한 걸음 x ← x − s/(1 − s·c), s = (F − p)/f, c = f′/(2f).
/// `dens(x)` 는 (밀도 f, c). 허용 오차가 확률의 절댓값이라 꼬리에서는 참값과 꽤 다를 수 있다
/// (T.INV(0.0001,2) = −70.6948, 참값 −70.7001).
fn hc_solve(
    p: f64,
    mut x: f64,
    cdf: impl Fn(f64) -> f64,
    dens: impl Fn(f64) -> (f64, f64),
    positive: bool,
) -> f64 {
    for _ in 0..100 {
        if !x.is_finite() {
            return x;
        }
        let r = super::stat::round9(cdf(x)) - p;
        if r.abs() < 1e-6 {
            return x;
        }
        let (f, c) = dens(x);
        if !(f > 0.0 && f.is_finite()) {
            return x;
        }
        let s = r / f;
        x -= s / (1.0 - s * c);
        if positive && x <= 0.0 {
            x = 1e-10;
        }
    }
    x
}

/// T.INV (소수 9자리 반올림 전). 자유도 1 은 tan(π′(p − ½)), π′ = 3.141592654. 그 밖에는 코니시-피셔 전개
/// z + (z³+z)/(4ν) + (5w + 16z³ + 3z)/(96ν²) + (19w + 17z³ − 15z)/(384ν³) 에서 [`hc_solve`]. z 는 오데-에번스이고
/// z⁵ 자리의 w 는 z⁷/(128ν³), z⁷ 항은 없다 — 한셀의 시작 값이 이 꼴이다 (대조 실험 x1·x3·x4·x5·tfine: 자유도 1E6 에서
/// z = 오데-에번스, 촘촘한 p 에서 누적 반올림이 바뀌는 자리와 결과의 어긋남이 같은 x₀ 이동을 가리키고, 그 이동이
/// 정확히 z⁷(5/96 + 19/(384ν))/(128ν⁵) — 자유도 2–10⁶ 의 5570 경우가 모두 같다)
pub fn t_inv(p: f64, df: f64) -> f64 {
    if p == 0.5 {
        return 0.0;
    }
    if df == 1.0 {
        return (3.141_592_654 * (p - 0.5)).mtan();
    }
    let z = norm_inv_oe(p);
    let z2 = z * z;
    let w = z2 * z2 * z2 * z / (128.0 * df * df * df);
    let g1 = (z2 * z + z) / 4.0;
    let g2 = (16.0 * z2 + 3.0) * z / 96.0;
    let g3 = (17.0 * z2 - 15.0) * z / 384.0;
    let x0 = z
        + g1 / df
        + g2 / (df * df)
        + g3 / (df * df * df)
        + 5.0 * w / (96.0 * df * df)
        + 19.0 * w / (384.0 * df * df * df);
    hc_solve(
        p,
        x0,
        |x| t_cdf(x, df),
        |x| {
            (
                super::special::t_pdf(x, df),
                -0.5 * (df + 1.0) * x / (df + x * x),
            )
        },
        false,
    )
}

/// 이보다 큰 자유도의 CHISQ.INV 는 윌슨-힐퍼티 근사 그대로 (반복 없음, 대조 실험 x1: 자유도 100·200)
const CHISQ_WH_MIN: f64 = 100.0;

/// CHISQ.INV (소수 9자리 반올림 전). 자유도 1 은 (오데-에번스 z((1+p)/2))², 2 는 −2 ln(1 − p),
/// 큰 자유도는 윌슨-힐퍼티 ν(1 − 2/(9ν) + z√(2/(9ν)))³, 그 사이는 이 근사에서 [`hc_solve`] (대조 실험 x1)
pub fn chisq_inv(p: f64, df: f64) -> f64 {
    if p == 0.0 {
        return 0.0;
    }
    if df == 1.0 {
        let z = norm_inv_oe((1.0 + p) / 2.0);
        return z * z;
    }
    if df == 2.0 {
        return -2.0 * (1.0 - p).mln();
    }
    let z = norm_inv_oe(p);
    let c = 2.0 / (9.0 * df);
    let x0 = df * (1.0 - c + z * c.sqrt()).powi(3);
    if df >= CHISQ_WH_MIN {
        return x0;
    }
    let a = df / 2.0;
    let lg = gammln(a);
    hc_solve(
        p,
        x0,
        |x| gammp(a, x / 2.0),
        |x| {
            (
                ((a - 1.0) * x.mln() - x / 2.0 - a * 2f64.mln() - lg).mexp(),
                0.5 * ((a - 1.0) / x - 0.5),
            )
        },
        true,
    )
}

/// F.INV (소수 9자리 반올림 전). 한 자유도가 1 이면 t 분포로: d1 = 1 은 T.INV((1+p)/2, d2) 를 소수 9자리로
/// 반올림해 제곱, d2 = 1 은 T.INV((2−p)/2, d1) 를 반올림해 제곱한 것의 역수 (반복 없음). 그 밖에는 A&S 26.6.16 의
/// e^{2w} 에서 두 자유도 차이 항을 뺀 w = z√(h+λ)/h 로 시작해 [`hc_solve`] (대조 실험 x1·x6: d1 ≠ d2 459 경우와
/// 자유도 1 인 541 경우가 모두 같다. 둘 다 1 은 99 가운데 88 — 나머지는 1E-9 쯤 작다)
pub fn f_inv(p: f64, d1: f64, d2: f64) -> f64 {
    let r9 = super::stat::round9;
    if d1 == 1.0 {
        let t = r9(t_inv((1.0 + p) / 2.0, d2));
        return t * t;
    }
    if d2 == 1.0 {
        let t = r9(t_inv((2.0 - p) / 2.0, d1));
        return 1.0 / (t * t);
    }
    let z = norm_inv_oe(p);
    let lam = (z * z - 3.0) / 6.0;
    let h = 2.0 / (1.0 / (d1 - 1.0) + 1.0 / (d2 - 1.0));
    let w = z * (h + lam).sqrt() / h;
    let mut x0 = (2.0 * w).mexp();
    if !(x0 > 0.0 && x0.is_finite()) {
        x0 = 1.0;
    }
    let lb = gammln(d1 / 2.0) + gammln(d2 / 2.0) - gammln((d1 + d2) / 2.0);
    hc_solve(
        p,
        x0,
        |x| f_cdf(x, d1, d2),
        |x| {
            (
                ((d1 / 2.0) * (d1 / d2).mln() + (d1 / 2.0 - 1.0) * x.mln()
                    - ((d1 + d2) / 2.0) * (1.0 + d1 * x / d2).mln()
                    - lb)
                    .mexp(),
                0.5 * ((d1 / 2.0 - 1.0) / x - ((d1 + d2) / 2.0) * (d1 / d2) / (1.0 + d1 * x / d2)),
            )
        },
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 대조 실험에서 한셀이 낸 값 (dist4·norm4·norm5·bessel4)
    #[test]
    fn hancell_values() {
        let r9 = |x: f64| (x * 1e9).round() * 1e-9;
        assert_eq!(r9(gammp(0.5, 0.5)), 0.682_689_491_000_000_1);
        assert_eq!(r9(f_cdf(1.0, 1.0, 1.0)), 0.499_999_982);
        assert_eq!(bessel_k(0, 2.0), 0.113_893_88);
        assert!((bessel_k(1, 1.0) - 0.601_907_231_666_905_7).abs() < 1e-15);
        assert!((bessel_i(1, 1.0) - 0.565_159_103_918_690_6).abs() < 1e-15);
        assert!((bessel_i(2, 0.0) + 1.305_728_397_542_944e-10).abs() < 1e-15);
        assert_eq!(
            r9(0.5 - 0.5 * erfcc(1e-9 / std::f64::consts::SQRT_2)),
            -1.500_000_000_000_000_2e-8
        );
        assert!((norm_cdf(-40.0) - 8.873_505_064_066_478e-8).abs() < 1e-14);
        assert!((norm_cdf(1.4) - 0.919_243_340_544_404_9).abs() < 1e-15);
        assert!((norm_cdf(20.0) - 0.999_999_989_173_261_7).abs() < 1e-15);
        assert_eq!(norm_inv_bisect(0.975), 1.959_960_937_5);
        // T.INV: 시작 값의 z⁷/(128ν³) 항이 있어야 끝자리까지 같다 (대조 실험 dist4·tfine)
        assert_eq!(r9(t_inv(0.6, 2.0)), r9(0.288_675_133));
        assert_eq!(r9(t_inv(0.75, 2.0)), r9(0.816_496_578));
        assert_eq!(r9(t_inv(0.0001, 2.0)), r9(-70.694_822_143));
        assert_eq!(r9(t_inv(0.999, 3.0)), r9(10.213_437_504));
        // F.INV: 두 자유도 차이 항이 없는 시작 값, 자유도 1 은 반올림한 T.INV 의 제곱 (대조 실험 dist4·x1)
        assert_eq!(r9(f_inv(0.5, 5.0, 10.0)), r9(0.931_933_162));
        assert_eq!(r9(f_inv(0.99, 5.0, 10.0)), r9(5.636_479_661));
        assert_eq!(r9(f_inv(0.01, 1.0, 1.0)), r9(0.000_246_781));
        assert_eq!(r9(f_inv(0.9, 1.0, 5.0)), r9(4.060_397_224));
        assert_eq!(r9(f_inv(0.1, 5.0, 1.0)), r9(0.246_281_323));
    }
}
