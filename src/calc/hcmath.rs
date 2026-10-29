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

/// 66비트로 자른 π/2 (x87 의 π) 를 26비트씩: 32비트 C 런타임의 sin·cos 은 몫 k = round(x·2/π) 가 2^16 이상,
/// tan 은 2^13 이상이면 이것으로 줄인다 — 그 위에서 C 런타임 값과 모두 같다 (참 π/2 보다 2.0223E-21 작다,
/// 검은 상자 실험: 그 아래는 참 π/2)
const PIO2_66: [f64; 3] = [
    f64::from_bits(0x3ff921fb50000000),
    f64::from_bits(0x3e5110b460000000),
    f64::from_bits(0x3c91a60000000000),
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
fn rem_pio2(x: f64, k_x87: f64) -> (i64, Dd) {
    if x.abs() <= PIO4.0 {
        return (0, Dd::from(x));
    }
    let k = (x * f64::from_bits(0x3fe45f306dc9c883)).round();
    let mut r = Dd::from(x);
    // 큰 인수는 C 런타임처럼 66비트로 자른 π/2 로 줄인다 (x87 fsin 의 π)
    let parts: &[f64] = if k.abs() >= k_x87 {
        &PIO2_66
    } else {
        &PIO2_PARTS
    };
    for &p in parts {
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

fn sin_cos_x(x: f64, k_x87: f64) -> (Dd, Dd) {
    let (q, r) = rem_pio2(x, k_x87);
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
    sin_cos_x(x, 65536.0).0.round()
}

/// cos x
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() >= TRIG_MAX {
        return libm::cos(x);
    }
    sin_cos_x(x, 65536.0).1.round()
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
    let (s, c) = sin_cos_x(x, 8192.0);
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

// ---------------------------------------------------------------- ln Γ

/// ln Γ(k/2) (k = 1…400) — 32비트 C 런타임 lgamma(= 한셀 GAMMALN)의 값, 한셀에서 잰 것 (대조 실험 g6)
const LGAMMA_HALF: [u64; 400] = [
    0x3fe250d048e7a1bd,
    0x0000000000000000,
    0xbfbeeb95b094c190,
    0x0000000000000000,
    0x3fd2383e809a67e8,
    0x3fe62e42fefa39ef,
    0x3ff3373018970a36,
    0x3ffcab0bfa2a2002,
    0x4003a140a3a623cb,
    0x40096ca77c922cf9,
    0x400fa99a5e94985a,
    0x401326643c4479c9,
    0x4016a676ab2aa094,
    0x401a51273acf01ca,
    0x401e233060e41f80,
    0x40210ce1f32dcc30,
    0x402319398ed5be28,
    0x4025358e82fcb70d,
    0x402760f04f64ba68,
    0x40299a8921a7f7cf,
    0x402be199a0f64394,
    0x402e357590954d15,
    0x40304ac08b1145d1,
    0x403180973f3a8d74,
    0x4032bbfe32d3aa80,
    0x4033fcba16d50143,
    0x4035429459d98a57,
    0x40368d5a9c3b32cd,
    0x4037dcde38e22919,
    0x403930f3df162a42,
    0x403a89733a64170d,
    0x403be636a63fd346,
    0x403d471aeb7e6aa9,
    0x403eabff061f1a84,
    0x40400a61f910a7fa,
    0x4040c0a63f2f353a,
    0x404178be9245be4f,
    0x4042329df2d5ee52,
    0x4042ee38152d81e3,
    0x4043ab8153363985,
    0x40446a6e9fba19d8,
    0x40452af57aed77be,
    0x4045ed0be8171bb5,
    0x4046b0a8643472a9,
    0x404775c1dd7c818f,
    0x40483c4faba84f05,
    0x4049044988ead541,
    0x4049cda78b856a45,
    0x404a98621fe7fb65,
    0x404b6472034e8d14,
    0x404c31d03ecf3823,
    0x404d007622cd65e7,
    0x404dd05d42c85f4b,
    0x404ea17f717c6794,
    0x404f73d6bd4e9a2e,
    0x405023aeb67e4fef,
    0x40508e06fe450892,
    0x4050f8f18d330240,
    0x4051646bd261edc4,
    0x4051d07353917231,
    0x40523d05ac006be8,
    0x4052aa208b59d0e5,
    0x405317c1b4b39e34,
    0x405385e6fd9e5a40,
    0x4053f48e4d43db42,
    0x405463b59b942084,
    0x4054d35af07f2966,
    0x4055437c633ace4a,
    0x4055b4181993b605,
    0x4056252c474896ba,
    0x405696b72d6f033d,
    0x405708b719e11658,
    0x40577b2a66b35c35,
    0x4057ee0f79b26758,
    0x40586164c3e78ac0,
    0x4058d528c1243d96,
    0x40594959f793b6c2,
    0x4059bdf6f75257a3,
    0x405a32fe5a0a8572,
    0x405aa86ec2969812,
    0x405b1e46dca78c15,
    0x405b94855c702ba2,
    0x405c0b28fe5467d2,
    0x405c8230869ca105,
    0x405cf99ac12ca1a6,
    0x405d7166813e12ee,
    0x405de992a11e3763,
    0x405e621e01eeba4f,
    0x405edb078b6966ec,
    0x405f544e2ba69cf1,
    0x405fcdf0d6e65b27,
    0x406023f743addd9f,
    0x406061231e7d5e40,
    0x40609e7b7ea41ea9,
    0x4060dbffe99602b9,
    0x406119afe762626b,
    0x4061578b029ef666,
    0x40619590c853a559,
    0x4061d3c0c7e7268c,
    0x4062121a930c6ec3,
    0x4062509dbdb0dd3c,
    0x40628f49ddeb1f31,
    0x4062ce1e8beac0d3,
    0x40630d1b61e86335,
    0x40634c3ffc168f39,
    0x40638b8bf8931ddb,
    0x4063cafef7592ed4,
    0x40640a989a33a6cd,
    0x40644a5884b02ed5,
    0x40648a3e5c12af1a,
    0x4064ca49c7493f42,
    0x40650a7a6ee08712,
    0x40654acffcf88a2e,
    0x40658b4a1d39da74,
    0x4065cbe87ccb2c0f,
    0x40660caaca474745,
    0x40664d90b5b353c0,
    0x40668e99f0757979,
    0x4066cfc62d4bd3c3,
    0x406711152043b2c3,
    0x406752867eb1283c,
    0x40679419ff26dc59,
    0x4067d5cf596e27bd,
    0x406817a6467f6fba,
    0x4068599e807ac239,
    0x40689bb7c2a0aea1,
    0x4068ddf1c94b594e,
    0x4069204c51e7c760,
    0x406962c71aef6086,
    0x4069a561e3e1a4bc,
    0x4069e81c6d3e1404,
    0x406a2af6787e4608,
    0x406a6defc810300c,
    0x406ab1081f509726,
    0x406af43f4285ad54,
    0x406b3794f6d9d7b0,
    0x406b7b0902569c47,
    0x406bbe9b2bdfb621,
    0x406c024b3b2e4e10,
    0x406c4618f8cc56f8,
    0x406c8a042e100c30,
    0x406cce0ca5179100,
    0x406d123228c4afbe,
    0x406d567484b8b7b6,
    0x406d9ad385507893,
    0x406ddf4ef7a05a70,
    0x406e23e6a970916c,
    0x406e689a69396bf1,
    0x406ead6a061fbaa2,
    0x406ef2554ff15147,
    0x406f375c17219fa1,
    0x406f7c7e2cc66183,
    0x406fc1bb6294656a,
    0x40700389c56e3462,
    0x407026433c44049d,
    0x40704909ff8b652b,
    0x40706bddf94d9765,
    0x40708ebf13dbf263,
    0x4070b1ad39ce8268,
    0x4070d4a85602b128,
    0x4070f7b05399f6c8,
    0x40711ac51df8932b,
    0x40713de6a0c44f6d,
    0x40716114c7e34737,
    0x4071844f7f7ab9a5,
    0x4071a796b3ede1ad,
    0x4071caea51dcd589,
    0x4071ee4a46236d3e,
    0x407211b67dd82fc8,
    0x4072352ee64b46d5,
    0x407258b36d0578dd,
    0x40727c43ffc72961,
    0x40729fe08c875f2d,
    0x4072c3890172d057,
    0x4072e73d4ceaf3fa,
    0x40730afd5d851956,
    0x40732ec922098451,
    0x407352a089728f1b,
    0x4073768382ebd0dd,
    0x40739a71fdd14947,
    0x4073be6be9ae90e2,
    0x4073e271363e0df7,
    0x40740681d3682e02,
    0x40742a9db142a36b,
    0x40744ec4c00fa798,
    0x407472f6f03d410d,
    0x4074973432648d98,
    0x4074bb7c77491066,
    0x4074dfcfafd803e8,
    0x4075042dcd27af64,
    0x40752896c076c02e,
    0x40754d0a7b2ba658,
    0x40757188eed3f4d4,
    0x407596120d23c4ec,
    0x4075baa5c7f51cf1,
    0x4075df4411475a1c,
    0x407603ecdb3e9d7d,
    0x407628a018233bed,
    0x40764d5dba6130ee,
    0x40767225b4879463,
    0x407696f7f9481308,
    0x4076bbd47b7669b7,
    0x4076e0bb2e07e333,
    0x407705ac0412d89f,
    0x40772aa6f0ce346d,
    0x40774fabe790f7be,
    0x407774badbd1c234,
    0x407799d3c1265c0d,
    0x4077bef68b434286,
    0x4077e4232dfb367c,
    0x407809599d3ecd3e,
    0x40782e99cd1c0367,
    0x407853e3b1bdd1e9,
    0x407879373f6bc4fd,
    0x40789e946a899529,
    0x4078c3fb2796c21c,
    0x4078e96b6b2e2f72,
    0x40790ee52a05c35e,
    0x4079346858ee0707,
    0x407959f4ecd1c8b4,
    0x40797f8adab5bfad,
    0x4079a52a17b831cb,
    0x4079cad299109aa7,
    0x4079f084540f545e,
    0x407a163f3e1d41f3,
    0x407a3c034cbb7b2d,
    0x407a61d07582f9f6,
    0x407a87a6ae24493a,
    0x407aad85ec673521,
    0x407ad36e262a7cc0,
    0x407af95f51638525,
    0x407b1f59641e0db5,
    0x407b455c547be5d6,
    0x407b6b6818b4a3ec,
    0x407b917ca7155d92,
    0x407bb799f600610b,
    0x407bddbffbecefe4,
    0x407c03eeaf66face,
    0x407c2a26070ede90,
    0x407c5065f9992225,
    0x407c76ae7dce35f4,
    0x407c9cff8a8a340e,
    0x407cc35916bca189,
    0x407ce9bb196830eb,
    0x407d102589a2857c,
    0x407d36985e93f7b7,
    0x407d5d138f775aa1,
    0x407d83971399c214,
    0x407daa22e25a4a02,
    0x407dd0b6f329dea4,
    0x407df7533d8b057c,
    0x407e1df7b911a74c,
    0x407e44a45d62dae6,
    0x407e6b592234b0c8,
    0x407e9215ff4dffa0,
    0x407eb8daec863182,
    0x407edfa7e1c51212,
    0x407f067cd7029d4d,
    0x407f2d59c446cf36,
    0x407f543ea1a97427,
    0x407f7b2b6751fa02,
    0x407fa2200d7741ea,
    0x407fc91c8c5f72e7,
    0x407ff020dc5fcd0b,
    0x40800b967aee3eb7,
    0x40801f2068a4395c,
    0x408032ae3392992d,
    0x4080463fd801573c,
    0x408059d5523f8e36,
    0x40806d6e9ea365ee,
    0x4080810bb989ff3f,
    0x408094ac9f576039,
    0x4080a8514c766090,
    0x4080bbf9bd589663,
    0x4080cfa5ee76433d,
    0x4080e355dc4e4164,
    0x4080f7098365f173,
    0x40810ac0e0492828,
    0x40811e7bef8a1c8a,
    0x4081323aadc1563e,
    0x408145fd178d9c2b,
    0x408159c32993e34f,
    0x40816d8ce07f3de8,
    0x4081815a3900cac2,
    0x4081952b2fcfa4cf,
    0x4081a8ffc1a8d2fd,
    0x4081bcd7eb4f383c,
    0x4081d0b3a98b83c1,
    0x4081e492f92c2188,
    0x4081f875d7052afe,
    0x40820c5c3ff057f8,
    0x4082204630ccefc3,
    0x40823433a67fba83,
    0x408248249df2f2b0,
    0x40825c19141636dc,
    0x4082701105de7b8d,
    0x4082840c7045fd68,
    0x4082980b504c3372,
    0x4082ac0da2f5c195,
    0x4082c013654c6b40,
    0x4082d41c945f0645,
    0x4082e8292d416dde,
    0x4082fc392d0c75d1,
    0x4083104c90ddddde,
    0x4083246355d84538,
    0x4083387d79231e3d,
    0x40834c9af7eaa248,
    0x408360bbcf5fc5bf,
    0x408374dffcb82c32,
    0x408389077d2e1cb2,
    0x40839d324e00764c,
    0x4083b1606c72a4a4,
    0x4083c591d5cc94b8,
    0x4083d9c6875aa9ce,
    0x4083edfe7e6db27e,
    0x40840239b85adde0,
    0x40841678327bb0e5,
    0x40842ab9ea2dfbd1,
    0x40843efedcd3cfc6,
    0x4084534707d3748f,
    0x4084679268975e6d,
    0x40847be0fc8e241e,
    0x40849032c12a74ef,
    0x4084a487b3e30eff,
    0x4084b8dfd232b593,
    0x4084cd3b19982794,
    0x4084e1998796161f,
    0x4084f5fb19b31b3f,
    0x40850a5fcd79b0bb,
    0x40851ec7a0782708,
    0x4085333290409c4b,
    0x408547a09a68f388,
    0x40855c11bc8acbdb,
    0x40857085f44377e0,
    0x408584fd3f33f517,
    0x408599779b00e38e,
    0x4085adf505527d71,
    0x4085c2757bd48ee8,
    0x4085d6f8fc366ddc,
    0x4085eb7f842af200,
    0x4086000911686cd6,
    0x40861495a1a8a1d5,
    0x4086292532a8beaa,
    0x40863db7c229538b,
    0x4086524d4dee4ba3,
    0x408666e5d3bee595,
    0x40867b815165ac10,
    0x4086901fc4b06e8a,
    0x4086a4c12b7039f0,
    0x4086b96583795198,
    0x4086ce0ccaa32816,
    0x4086e2b6fec85851,
    0x4086f7641dc69e97,
    0x40870c14257ed1c2,
    0x408720c713d4dc7f,
    0x4087357ce6afb697,
    0x40874a359bf95e63,
    0x40875ef1319ed23c,
    0x408773afa5900a03,
    0x40878870f5bff0cc,
    0x40879d3520245e84,
    0x4087b1fc22b611b4,
    0x4087c6c5fb70a95a,
    0x4087db92a8529ed1,
    0x4087f062275d3fbc,
    0x408805347694a81b,
    0x40881a0993ffbc55,
    0x40882ee17da82373,
    0x408843bc319a414c,
    0x40885899ade530d2,
    0x40886d79f09abe6d,
    0x4088825cf7cf6261,
    0x40889742c19a3b49,
    0x4088ac2b4c15089c,
    0x4088c116955c2548,
    0x4088d6049b8e8253,
    0x4088eaf55ccda18f,
    0x4088ffe8d73d9061,
    0x408914df0904e28b,
    0x408929d7f04cad12,
    0x40893ed38b408125,
    0x408953d1d80e671b,
    0x408968d2d4e6d974,
    0x40897dd67ffcbffc,
    0x408992dcd7856adc,
    0x4089a7e5d9b88dd4,
    0x4089bcf184d03b79,
    0x4089d1ffd708e072,
    0x4089e710cea13ed9,
    0x4089fc2469da6998,
    0x408a113aa6f7bfd4,
    0x408a2653843ee86b,
    0x408a3b6efff7cd79,
    0x408a508d186c97e3,
    0x408a65adcbe9aafd,
    0x408a7ad118bda02b,
    0x408a8ff6fd394298,
    0x408aa51f77af8af4,
    0x408aba4a86759b3c,
    0x408acf7827e2ba8e,
];

/// ½ln(2π)
const HALF_LN_2PI: Dd = Dd(
    f64::from_bits(0x3fed67f1c864beb5),
    f64::from_bits(0xbc865b5a1b7ff5df),
);

/// ln Γ(x) (x > 0) — 한셀 GAMMALN 은 32비트 C 런타임 lgamma 의 값이다 (906 경우 모두 같다, 검은 상자 실험).
///
/// - 반정수·정수 0.5–200: C 런타임 값의 표 (t 분포 상수가 쓰는 곳)
/// - x ≥ 54: C 런타임과 같은 꼴 (x − ½)(ln x − 1) + w(1/x), w 는 fdlibm 의 최소 최대 급수
///   (이 구간에서 C 런타임 값과 모두 같다 — 54 아래는 다른 계산이다)
/// - 그 밖 (x < 54): 2배 정밀로 계산해 한 번 반올림 (C 런타임과 70–76% 같다, 오차 2 ulp 안)
pub fn lgamma(x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return x;
    }
    let k2 = x * 2.0;
    if k2 == k2.trunc() && (1.0..=400.0).contains(&k2) {
        return f64::from_bits(LGAMMA_HALF[k2 as usize - 1]);
    }
    if x >= 54.0 {
        // fdlibm e_lgamma_r.c 의 w0–w6 (Sun Microsystems, 1993: "Permission to use, copy, modify, and
        // distribute this software is freely granted, provided that this notice is preserved.")
        const W: [f64; 7] = [
            4.18938533204672725052e-01,
            8.33333333333329678849e-02,
            -2.77777777728775536470e-03,
            7.93650558643019558500e-04,
            -5.95187557450339963135e-04,
            8.36339918996282139126e-04,
            -1.63092934096575273989e-03,
        ];
        let t = ln(x);
        if x >= 288230376151711744.0 {
            // 2^58 이상
            return x * (t - 1.0);
        }
        let z = 1.0 / x;
        let y = z * z;
        let w = W[0] + z * (W[1] + y * (W[2] + y * (W[3] + y * (W[4] + y * (W[5] + y * W[6])))));
        return (x - 0.5) * (t - 1.0) + w;
    }
    // x < 54: ln Γ(x) = ln Γ(x + n) − ln(x(x+1)…(x+n−1)), x + n ≥ 30 에서 스털링 급수 (2배 정밀)
    let mut p = Dd::from(1.0);
    let mut z = Dd::from(x);
    while z.0 < 30.0 {
        p = p.mul(z);
        z = z.add_f(1.0);
    }
    // (z − ½) ln z − z + ½ln 2π + Σ B_2k / (2k(2k−1) z^(2k−1))
    const B: [(f64, f64); 10] = [
        (1.0, 12.0),
        (-1.0, 360.0),
        (1.0, 1260.0),
        (-1.0, 1680.0),
        (1.0, 1188.0),
        (-691.0, 360360.0),
        (1.0, 156.0),
        (-3617.0, 122400.0),
        (43867.0, 244188.0),
        (-174611.0, 125400.0),
    ];
    let lz = ln_dd(z.0).add_f(z.1 / z.0);
    let mut s = z.add_f(-0.5).mul(lz).sub(z).add(HALF_LN_2PI);
    let iz = Dd::from(1.0).div(z);
    let iz2 = iz.mul(iz);
    let mut zp = iz;
    for (n, d) in B {
        s = s.add(zp.mul_f(n).div_f(d));
        zp = zp.mul(iz2);
    }
    let lp = ln_dd(p.0).add_f(p.1 / p.0);
    s.sub(lp).round()
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
