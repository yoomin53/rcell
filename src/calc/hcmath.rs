// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀과 같은 값을 내는 초월 함수 (순수 Rust, 플랫폼과 관계없이 같은 값).
//!
//! 한셀(32비트)의 EXP·LN·LOG10·POWER(^)·SQRT·SIN·COS·TAN·ATAN·ASIN·ACOS 는 32비트 윈도 C 런타임(ucrtbase)
//! 함수의 값과 끝자리까지 같다 (검은 상자 실험: 한셀이 계산한 1만여 경우를 같은 입력의 C 런타임 값과 비교하면
//! 모두 같다 — 한셀이 XML 에 쓸 때 [1E-6, 0.1) 의 수를 1 ulp 틀리게 적는 8 경우뿐, docs/format/calc.md).
//! SINH·COSH·TANH 는 C 런타임이 아니라 pow(e, x) 정의식이다 (functions/math.rs).
//! C 런타임 함수들은 거의 정확히 반올림한 값을 낸다 — 무작위 입력에서 참값을 반올림한 값과 99.5–99.9% 같고
//! 다를 때도 오차가 0.6 ulp 아래다. 그래서 여기서는
//!
//! - `exp`: C 런타임과 같은 꼴의 계산 — 2^(j/64) 표(hi, lo/hi)와 5차 다항식, ln2/64 를 두 조각으로 나눈 인수 줄이기.
//!   다항식 계수는 C 런타임 값 수백만 개에 맞춘 것이다 (C 런타임과 99.83% 같다, 정확한 반올림은 99.46%).
//! - 나머지: 2배 정밀(double-double, 약 104비트)로 계산해 한 번 반올림한 값 = 정확히 반올림한 값
//!   (C 런타임과 99.6–99.9% 같다). `libm` 크레이트(musl)의 값은 90–97% 만 같다.
//!
//! 모든 계산은 f64 의 더하기·곱하기·나누기·제곱근뿐이라 wasm 에서도 같은 값이 나온다.

// 상수는 일부러 비트로 적는다 (정확히 그 값이어야 한다)
#![allow(clippy::excessive_precision, clippy::unreadable_literal)]

/// 2배 정밀 수: hi + lo, |lo| ≤ ulp(hi)/2
#[derive(Clone, Copy, Debug)]
struct Dd(f64, f64);

#[inline]
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd(s, (a - (s - bb)) + (b - bb))
}

#[inline]
fn fast_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd(s, b - (s - a))
}

#[inline]
fn split(a: f64) -> (f64, f64) {
    // 아주 큰 수는 넘치지 않게 줄여서
    if a.abs() > 6.69692879491417e+299 {
        let a2 = a * 3.7252902984619140625e-09;
        let c = 134217729.0 * a2;
        let h = c - (c - a2);
        return (h * 268435456.0, (a2 - h) * 268435456.0);
    }
    let c = 134217729.0 * a;
    let h = c - (c - a);
    (h, a - h)
}

#[inline]
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    Dd(p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

impl Dd {
    #[inline]
    fn from(x: f64) -> Dd {
        Dd(x, 0.0)
    }
    #[inline]
    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.0, o.0);
        let t = two_sum(self.1, o.1);
        let s = fast_two_sum(s.0, s.1 + t.0);
        fast_two_sum(s.0, s.1 + t.1)
    }
    #[inline]
    fn add_f(self, o: f64) -> Dd {
        let s = two_sum(self.0, o);
        fast_two_sum(s.0, s.1 + self.1)
    }
    #[inline]
    fn neg(self) -> Dd {
        Dd(-self.0, -self.1)
    }
    #[inline]
    fn sub(self, o: Dd) -> Dd {
        self.add(o.neg())
    }
    #[inline]
    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.0, o.0);
        fast_two_sum(p.0, p.1 + (self.0 * o.1 + self.1 * o.0))
    }
    #[inline]
    fn mul_f(self, o: f64) -> Dd {
        let p = two_prod(self.0, o);
        fast_two_sum(p.0, p.1 + self.1 * o)
    }
    #[inline]
    fn div(self, o: Dd) -> Dd {
        let q1 = self.0 / o.0;
        let r = self.sub(o.mul_f(q1));
        let q2 = r.0 / o.0;
        let r = r.sub(o.mul_f(q2));
        let q3 = r.0 / o.0;
        let q = fast_two_sum(q1, q2);
        q.add_f(q3)
    }
    #[inline]
    fn div_f(self, o: f64) -> Dd {
        self.div(Dd::from(o))
    }
    fn sqrt(self) -> Dd {
        if self.0 <= 0.0 {
            return Dd(self.0.sqrt(), 0.0);
        }
        let s = self.0.sqrt();
        // 뉴턴 한 걸음: s + (a − s²)/(2s)
        let r = self.sub(two_prod(s, s));
        fast_two_sum(s, r.0 / (2.0 * s))
    }
    /// 한 번 반올림한 f64
    #[inline]
    fn round(self) -> f64 {
        self.0 + self.1
    }
    /// × 2^n (정상 범위)
    fn scale(self, n: i32) -> Dd {
        Dd(libm::scalbn(self.0, n), libm::scalbn(self.1, n))
    }
}

const LN2: Dd = Dd(
    f64::from_bits(0x3fe62e42fefa39ef),
    f64::from_bits(0x3c7abc9e3b39803f),
);
const LN10: Dd = Dd(
    f64::from_bits(0x40026bb1bbb55516),
    f64::from_bits(0xbcaf48ad494ea3e9),
);
const PI: Dd = Dd(
    f64::from_bits(0x400921fb54442d18),
    f64::from_bits(0x3ca1a62633145c07),
);
const PIO2: Dd = Dd(
    f64::from_bits(0x3ff921fb54442d18),
    f64::from_bits(0x3c91a62633145c07),
);
const PIO4: Dd = Dd(
    f64::from_bits(0x3fe921fb54442d18),
    f64::from_bits(0x3c81a62633145c07),
);
/// π/2 를 26비트씩 나눈 조각 (k·조각이 정확하다, |k| < 2^27)
const PIO2_PARTS: [f64; 6] = [
    f64::from_bits(0x3ff921fb50000000),
    f64::from_bits(0x3e5110b460000000),
    f64::from_bits(0x3c91a62630000000),
    f64::from_bits(0x3ae8a2e030000000),
    f64::from_bits(0x394c1cd128000000),
    f64::from_bits(0x378024e088000000),
];

// ---------------------------------------------------------------- exp

/// 2^(j/64): (hi, lo/hi)
const T64: [(u64, u64); 64] = [
    (0x3ff0000000000000, 0x0000000000000000),
    (0x3ff02c9a3e778061, 0xbc7160139cd8dc5c),
    (0x3ff059b0d3158574, 0x3c8cd2523567f613),
    (0x3ff0874518759bc8, 0x3c60f74e61e6c861),
    (0x3ff0b5586cf9890f, 0x3c979aa65d837b6d),
    (0x3ff0e3ec32d3d1a2, 0x3c3ebe3d702f9cd2),
    (0x3ff11301d0125b51, 0xbc9556522a2fbd0e),
    (0x3ff1429aaea92de0, 0xbc91c923b9d5f415),
    (0x3ff172b83c7d517b, 0xbc801b15eaa59348),
    (0x3ff1a35beb6fcb75, 0x3c8b898c3f1353bf),
    (0x3ff1d4873168b9aa, 0x3c9aecf73e3a2f60),
    (0x3ff2063b88628cd6, 0x3c8a6f4144a6c38d),
    (0x3ff2387a6e756238, 0x3c968efde3a8a894),
    (0x3ff26b4565e27cdd, 0x3c80472b981fe7f2),
    (0x3ff29e9df51fdee1, 0x3c82f7e16d09ab31),
    (0x3ff2d285a6e4030b, 0x3c8b3782720c0ab4),
    (0x3ff306fe0a31b715, 0x3c834d754db0abb6),
    (0x3ff33c08b26416ff, 0x3c8fdd395dd3f84b),
    (0x3ff371a7373aa9cb, 0xbc924aedcc4b5069),
    (0x3ff3a7db34e59ff7, 0xbc71d1e83e9436d2),
    (0x3ff3dea64c123422, 0x3c859f48a72a4c6d),
    (0x3ff4160a21f72e2a, 0xbc58a78f4817895b),
    (0x3ff44e086061892d, 0x3c4363ed60c2ac12),
    (0x3ff486a2b5c13cd0, 0x3c6ecce1daa10378),
    (0x3ff4bfdad5362a27, 0x3c7690cebb7aafb0),
    (0x3ff4f9b2769d2ca7, 0xbc8f94340071a38e),
    (0x3ff5342b569d4f82, 0xbc78dec6bd0f3860),
    (0x3ff56f4736b527da, 0x3c93350518fdd78e),
    (0x3ff5ab07dd485429, 0x3c9063e1e21c5409),
    (0x3ff5e76f15ad2148, 0x3c9432e62b64c036),
    (0x3ff6247eb03a5585, 0xbc8c33c53bef4da8),
    (0x3ff6623882552225, 0xbc93cedd78565858),
    (0x3ff6a09e667f3bcd, 0xbc93b3efbf5e2229),
    (0x3ff6dfb23c651a2f, 0xbc6367efb86da9ee),
    (0x3ff71f75e8ec5f74, 0xbc781f647e5a3ece),
    (0x3ff75feb564267c9, 0xbc8619321e55e68a),
    (0x3ff7a11473eb0187, 0xbc7b32dcb94da51d),
    (0x3ff7e2f336cf4e62, 0x3c65ebe1abd66c55),
    (0x3ff82589994cce13, 0xbc9369b6f13b3734),
    (0x3ff868d99b4492ed, 0xbc94d450d872576e),
    (0x3ff8ace5422aa0db, 0x3c8db72fc1f0eab5),
    (0x3ff8f1ae99157736, 0x3c7bf68359f35f44),
    (0x3ff93737b0cdc5e5, 0xbc5da9b88b6c1e29),
    (0x3ff97d829fde4e50, 0xbc92434322f4f9aa),
    (0x3ff9c49182a3f090, 0x3c71affc2b91ce27),
    (0x3ffa0c667b5de565, 0xbc87c50422622263),
    (0x3ffa5503b23e255d, 0xbc91bbd1d3bcbb15),
    (0x3ffa9e6b5579fdbf, 0x3c8469846e735ab3),
    (0x3ffae89f995ad3ad, 0x3c8c1a7792cb3386),
    (0x3ffb33a2b84f15fb, 0xbc55c3d956dcaeba),
    (0x3ffb7f76f2fb5e47, 0xbc68d6f438ad9334),
    (0x3ffbcc1e904bc1d2, 0x3c74ffd70a5fddcd),
    (0x3ffc199bdd85529c, 0x3c736eae30af0cb3),
    (0x3ffc67f12e57d14b, 0x3c84e08fd10959ac),
    (0x3ffcb720dcef9069, 0x3c676b2c6c921967),
    (0x3ffd072d4a07897c, 0xbc8fad5d3ffffa6e),
    (0x3ffd5818dcfba487, 0x3c74a385a63d07a8),
    (0x3ffda9e603db3285, 0x3c8e5a50d5c192ac),
    (0x3ffdfc97337b9b5f, 0xbc82d52107b43e20),
    (0x3ffe502ee78b3ff6, 0x3c74b604603a88d3),
    (0x3ffea4afa2a490da, 0xbc8ff7128fd391f0),
    (0x3ffefa1bee615a27, 0x3c8ec3bc41aa2008),
    (0x3fff50765b6e4540, 0x3c8a64a931d185ee),
    (0x3fffa7c1819e90d8, 0x3c77893b4d91cd9d),
];

/// e^x (한셀 EXP 와 같은 계산)
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782712893384 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    const K: f64 = f64::from_bits(0x40571547652b82fe); // 64/ln2
    const L1: f64 = f64::from_bits(0x3f862e42ff000000); // ln2/64 앞 32비트
    const L2: f64 = f64::from_bits(0xbd6718432a1b0e26); // 나머지
    const SH: f64 = 6755399441055744.0; // 1.5·2^52: 더해서 정수로 반올림
    const C: [f64; 4] = [
        0.4999999999995667,
        0.16666666668731042,
        0.04166672081672555,
        0.008332592618964014,
    ];
    let m = (x * K + SH) - SH;
    let y = (x - m * L1) - m * L2;
    let mi = m as i64;
    let (th, rel) = T64[(mi & 63) as usize];
    let (th, rel) = (f64::from_bits(th), f64::from_bits(rel));
    let p = y + y * y * (C[0] + y * (C[1] + y * (C[2] + y * C[3])));
    libm::scalbn(th + th * (p + rel), (mi >> 6) as i32)
}

// ---------------------------------------------------------------- 로그

/// x = m·2^e, m ∈ [0.5, 1)
fn frexp(x: f64) -> (f64, i32) {
    let b = x.to_bits();
    let ex = ((b >> 52) & 0x7ff) as i32;
    if ex == 0 {
        // 비정규 수
        let (m, e) = frexp(x * 18014398509481984.0); // 2^54
        return (m, e - 54);
    }
    let m = f64::from_bits((b & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, ex - 1022)
}

/// ln x (x > 0, 유한) 를 2배 정밀로: x = m·2^e (m ∈ [√½, √2)), ln m = 2·atanh((m−1)/(m+1)) 급수
fn ln_dd(x: f64) -> Dd {
    let (mut m, mut e) = frexp(x);
    if m < std::f64::consts::FRAC_1_SQRT_2 {
        m *= 2.0;
        e -= 1;
    }
    // m − 1 은 정확하다 (m ∈ [0.707, 1.414])
    let num = Dd::from(m - 1.0);
    let den = two_sum(m, 1.0);
    let s = num.div(den);
    let s2 = s.mul(s);
    // s·(1 + s²/3 + s⁴/5 + …)
    let mut term = s;
    let mut sum = s;
    let mut k = 3.0;
    while k < 80.0 {
        term = term.mul(s2);
        let t = term.div_f(k);
        sum = sum.add(t);
        if t.0.abs() <= 1e-34 * sum.0.abs() {
            break;
        }
        k += 2.0;
    }
    LN2.mul_f(e as f64).add(sum.mul_f(2.0))
}

/// 자연로그 (한셀 LN)
pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() || x == 1.0 {
        return if x == 1.0 { 0.0 } else { x };
    }
    ln_dd(x).round()
}

/// 상용로그 (한셀 LOG10)
pub fn log10(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() || x == 1.0 {
        return if x == 1.0 { 0.0 } else { x };
    }
    ln_dd(x).div(LN10).round()
}

// ---------------------------------------------------------------- 거듭제곱

/// e^a (a 는 2배 정밀, a.0 ∈ [−746, 710]): k·ln2 를 빼고 2^-10 으로 줄인 테일러 급수를 10번 제곱
fn exp_dd(a: Dd) -> Dd {
    let k = (a.0 / LN2.0).round();
    // a − k·ln2 (ln2 를 세 조각으로, k·조각은 two_prod 로 정확히)
    let r = a
        .sub(two_prod(k, LN2.0))
        .sub(two_prod(k, LN2.1))
        .add_f(-k * 5.707708438416212e-34);
    let r = r.scale(-10);
    // e^r − 1 = r + r²/2 + … (|r| < 3.4e-4)
    let mut term = r;
    let mut sum = r;
    let mut n = 2.0;
    while n < 20.0 {
        term = term.mul(r).div_f(n);
        sum = sum.add(term);
        if term.0.abs() <= 1e-36 * sum.0.abs() {
            break;
        }
        n += 1.0;
    }
    // (1 + s)^(2^10): s ← 2s + s² 을 10번 (1 을 따로 두어 자릿수를 지킨다)
    for _ in 0..10 {
        sum = sum.mul_f(2.0).add(sum.mul(sum));
    }
    let v = sum.add_f(1.0);
    v.scale(k as i32)
}

/// x^y (한셀 POWER·^ 가 부르는 C 런타임 pow)
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let yi = y == y.trunc();
    let odd = yi && y.abs() < 9007199254740992.0 && (y % 2.0).abs() == 1.0;
    if x == 0.0 {
        let r = if y < 0.0 { f64::INFINITY } else { 0.0 };
        return if x.is_sign_negative() && odd { -r } else { r };
    }
    if x.is_infinite() || y.is_infinite() {
        return libm::pow(x, y);
    }
    if x < 0.0 && !yi {
        return f64::NAN;
    }
    // C 런타임은 정수 지수도 같은 계산이다 (pow(x, 2) 가 x·x 와 다른 경우가 있다)
    if y == 1.0 {
        return x;
    }
    let t = ln_dd(x.abs()).mul_f(y);
    let r = if t.0 > 709.79 {
        f64::INFINITY
    } else if t.0 < -745.2 {
        0.0
    } else if t.0 < -708.0 {
        // 비정규 결과: 2^54 배로 계산해 반올림한 뒤 줄인다
        let v = exp_dd(t.add(LN2.mul_f(54.0)));
        libm::scalbn(v.round(), -54)
    } else {
        exp_dd(t).round()
    };
    if x < 0.0 && odd { -r } else { r }
}

// ---------------------------------------------------------------- 삼각

/// x = k·π/2 + r (|r| ≤ π/4 쯤): (k mod 4, r)
fn rem_pio2(x: f64) -> (i64, Dd) {
    if x.abs() <= PIO4.0 {
        return (0, Dd::from(x));
    }
    let k = (x * f64::from_bits(0x3fe45f306dc9c883)).round();
    let mut r = Dd::from(x);
    for p in PIO2_PARTS {
        // k·p 는 정확하다 (|k| < 2^27, p 는 26비트)
        r = r.sub(Dd::from(k * p));
    }
    ((k as i64).rem_euclid(4), r)
}

/// sin r, cos r (|r| ≤ π/4 쯤) 의 급수
fn sincos_dd(r: Dd) -> (Dd, Dd) {
    let r2 = r.mul(r);
    let (mut ts, mut s) = (r, r);
    let (mut tc, mut c) = (Dd::from(1.0), Dd::from(1.0));
    let mut n = 1.0;
    while n < 60.0 {
        tc = tc.mul(r2).div_f(-(n * (n + 1.0)));
        ts = ts.mul(r2).div_f(-((n + 1.0) * (n + 2.0)));
        c = c.add(tc);
        s = s.add(ts);
        if tc.0.abs() <= 1e-36 && ts.0.abs() <= 1e-36 * s.0.abs() {
            break;
        }
        n += 2.0;
    }
    (s, c)
}

fn sin_cos_x(x: f64) -> (Dd, Dd) {
    let (q, r) = rem_pio2(x);
    let (s, c) = sincos_dd(r);
    match q {
        0 => (s, c),
        1 => (c, s.neg()),
        2 => (s.neg(), c.neg()),
        _ => (c.neg(), s),
    }
}

/// 한셀 삼각 함수의 인수 한도 (이 밖은 한셀이 #NUM!)
const TRIG_MAX: f64 = 134217728.0;

/// sin x
pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() >= TRIG_MAX {
        return libm::sin(x);
    }
    if x.abs() < 1e-8 {
        return Dd::from(x).sub(two_prod(x, x).mul_f(x).div_f(6.0)).round();
    }
    sin_cos_x(x).0.round()
}

/// cos x
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() >= TRIG_MAX {
        return libm::cos(x);
    }
    sin_cos_x(x).1.round()
}

/// tan x
pub fn tan(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() >= TRIG_MAX {
        return libm::tan(x);
    }
    if x.abs() < 1e-8 {
        return Dd::from(x).add(two_prod(x, x).mul_f(x).div_f(3.0)).round();
    }
    let (s, c) = sin_cos_x(x);
    s.div(c).round()
}

/// atan t (0 ≤ t ≤ 1, 2배 정밀): 반각 공식 t ← t/(1 + √(1 + t²)) 로 줄인 뒤 급수
fn atan_dd(t: Dd) -> Dd {
    let mut t = t;
    let mut mult = 1.0;
    for _ in 0..3 {
        if t.0 < 0.05 {
            break;
        }
        let d = t.mul(t).add_f(1.0).sqrt().add_f(1.0);
        t = t.div(d);
        mult *= 2.0;
    }
    let t2 = t.mul(t);
    let (mut term, mut sum) = (t, t);
    let mut k = 3.0;
    let mut sign = -1.0;
    while k < 200.0 {
        term = term.mul(t2);
        let d = term.div_f(k * sign);
        sum = sum.add(d);
        if d.0.abs() <= 1e-36 * sum.0.abs() {
            break;
        }
        k += 2.0;
        sign = -sign;
    }
    sum.mul_f(mult)
}

/// atan q (q ≥ 0, 2배 정밀)
fn atan_pos(q: Dd) -> Dd {
    if q.0 > 1.0 {
        PIO2.sub(atan_dd(Dd::from(1.0).div(q)))
    } else {
        atan_dd(q)
    }
}

/// atan x
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return x.signum() * PIO2.0;
    }
    if x.abs() < 1e-8 {
        return Dd::from(x).sub(two_prod(x, x).mul_f(x).div_f(3.0)).round();
    }
    let r = atan_pos(Dd::from(x.abs())).round();
    if x < 0.0 { -r } else { r }
}

/// asin x (|x| ≤ 1)
pub fn asin(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1.0 {
        return f64::NAN;
    }
    if x.abs() < 1e-8 {
        return Dd::from(x).add(two_prod(x, x).mul_f(x).div_f(6.0)).round();
    }
    let a = x.abs();
    let r = if a == 1.0 {
        PIO2.0
    } else {
        // 1 − a² 를 자릿수를 잃지 않게: a ≥ ½ 이면 (1 − a)(1 + a), 1 − a 는 정확하다
        let d = if a >= 0.5 {
            Dd::from(1.0 - a).mul(two_sum(1.0, a))
        } else {
            Dd::from(1.0).sub(two_prod(a, a))
        };
        atan_pos(Dd::from(a).div(d.sqrt())).round()
    };
    if x < 0.0 { -r } else { r }
}

/// acos x (|x| ≤ 1) = 2·atan √((1 − x)/(1 + x))
pub fn acos(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1.0 {
        return f64::NAN;
    }
    if x == -1.0 {
        return PI.0;
    }
    let q = two_sum(1.0, -x).div(two_sum(1.0, x)).sqrt();
    atan_pos(q).mul_f(2.0).round()
}

/// atan2(y, x)
pub fn atan2(y: f64, x: f64) -> f64 {
    if !x.is_finite() || !y.is_finite() || x == 0.0 || y == 0.0 {
        return libm::atan2(y, x);
    }
    let a = atan_pos(Dd::from(y.abs()).div(Dd::from(x.abs())));
    let a = if x < 0.0 { PI.sub(a) } else { a };
    let r = a.round();
    if y < 0.0 { -r } else { r }
}

// ---------------------------------------------------------------- 쌍곡

/// sinh·cosh 의 급수 (|a| < 0.5): (sinh a, cosh a)
fn sinhcosh_series(a: f64) -> (Dd, Dd) {
    let x2 = two_prod(a, a);
    let (mut ts, mut s) = (Dd::from(a), Dd::from(a));
    let (mut tc, mut c) = (Dd::from(1.0), Dd::from(1.0));
    let mut n = 1.0;
    while n < 60.0 {
        tc = tc.mul(x2).div_f(n * (n + 1.0));
        ts = ts.mul(x2).div_f((n + 1.0) * (n + 2.0));
        c = c.add(tc);
        s = s.add(ts);
        if tc.0 <= 1e-36 {
            break;
        }
        n += 2.0;
    }
    (s, c)
}

/// sinh x (C 런타임 sinh — 한셀 SINH 함수는 pow(e, x) 정의식이고, 이것은 COTH·CSCH·IM… 이 쓴다)
pub fn sinh(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let a = x.abs();
    let r = if a > 710.5 {
        f64::INFINITY
    } else if a < 0.5 {
        sinhcosh_series(a).0.round()
    } else if a > 709.0 {
        exp_dd(Dd::from(a).sub(LN2)).round()
    } else {
        let e = exp_dd(Dd::from(a));
        e.sub(Dd::from(1.0).div(e)).mul_f(0.5).round()
    };
    if x < 0.0 { -r } else { r }
}

/// cosh x
pub fn cosh(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let a = x.abs();
    if a > 710.5 {
        f64::INFINITY
    } else if a < 0.5 {
        sinhcosh_series(a).1.round()
    } else if a > 709.0 {
        exp_dd(Dd::from(a).sub(LN2)).round()
    } else {
        let e = exp_dd(Dd::from(a));
        e.add(Dd::from(1.0).div(e)).mul_f(0.5).round()
    }
}

/// tanh x
pub fn tanh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    let a = x.abs();
    let r = if a > 22.0 {
        1.0
    } else if a < 0.5 {
        let (s, c) = sinhcosh_series(a);
        s.div(c).round()
    } else {
        // (e^2a − 1)/(e^2a + 1)
        let e = exp_dd(Dd::from(a).mul_f(2.0));
        e.add_f(-1.0).div(e.add_f(1.0)).round()
    };
    if x < 0.0 { -r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32비트 C 런타임(= 한셀)의 값
    #[test]
    fn crt_values() {
        assert_eq!(exp(1.0), std::f64::consts::E);
        assert_eq!(exp(-2.778061224489795), 0.06215890278827597);
        assert_eq!(ln(10.0), std::f64::consts::LN_10);
        assert_eq!(sin(2.5), 0.5984721441039565); // 정확한 반올림 (C 런타임은 …644, 0.5027 ulp)
        assert_eq!(log10(1000.0), 3.0);
        assert_eq!(pow(2.0, 0.5), std::f64::consts::SQRT_2);
        assert_eq!(pow(10.0, -2.0), 0.01);
        assert_eq!(sin(std::f64::consts::PI), 1.2246467991473532e-16);
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(atan(1.0), std::f64::consts::FRAC_PI_4);
        assert_eq!(asin(1.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(acos(-1.0), std::f64::consts::PI);
    }
}
