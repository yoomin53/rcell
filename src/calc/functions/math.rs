// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수학·삼각 함수.

use crate::calc::fm::Fm;
use std::rc::Rc;

use super::{
    A, Agg, Arg, FnSpec, MANY, R, V, collect_numbers, f, first_area, num_arg, num_or, opt, pairs,
    to_array, volatile,
};
use crate::calc::criteria::Criterion;
use crate::calc::eval::Ctx;
use crate::calc::value::{self, Area, Array, ErrorKind, MAX_COLS, MAX_ROWS, Value, num, round15};

const VR: &[Arg] = &[Arg::V, Arg::R];
const RVR: &[Arg] = &[Arg::R, Arg::V, Arg::R];
const RRV: &[Arg] = &[Arg::R, Arg::R, Arg::V];
const VVA: &[Arg] = &[Arg::V, Arg::V, Arg::V, Arg::A];

pub const FNS: &[FnSpec] = &[
    f("ABS", 1, 1, V, |c, a| un(c, a, |x| Ok(x.abs()))),
    f("SIGN", 1, 1, V, |c, a| {
        un(c, a, |x| {
            Ok(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            })
        })
    }),
    f("PI", 0, 0, V, |_, _| Value::Number(std::f64::consts::PI)),
    f("SQRT", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x < 0.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(x.sqrt())
            }
        })
    }),
    f("SQRTPI", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x < 0.0 {
                Err(ErrorKind::Num)
            } else {
                Ok((x * std::f64::consts::PI).sqrt())
            }
        })
    }),
    f("EXP", 1, 1, V, |c, a| un(c, a, |x| Ok(x.mexp()))),
    f("LN", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x <= 0.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(x.mln())
            }
        })
    }),
    f("LOG10", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x <= 0.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(x.mlog10())
            }
        })
    }),
    f("LOG", 1, 2, V, log),
    f("POWER", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let y = try_v!(num_arg(c, a, 1));
        crate::calc::eval::power(x, y)
    }),
    f("SIN", 1, 1, V, |c, a| {
        un(c, a, |x| trig_arg(x).map(Fm::msin))
    }),
    f("COS", 1, 1, V, |c, a| {
        un(c, a, |x| trig_arg(x).map(Fm::mcos))
    }),
    f("TAN", 1, 1, V, |c, a| {
        un(c, a, |x| trig_arg(x).map(Fm::mtan))
    }),
    f("ASIN", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x.abs() > 1.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(x.masin())
            }
        })
    }),
    f("ACOS", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x.abs() > 1.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(x.macos())
            }
        })
    }),
    f("ATAN", 1, 1, V, |c, a| un(c, a, |x| Ok(x.matan()))),
    f("ATAN2", 2, 2, V, |c, a| {
        let x = try_v!(num_arg(c, a, 0));
        let y = try_v!(num_arg(c, a, 1));
        if x == 0.0 && y == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num(y.matan2(x))
    }),
    // 쌍곡 함수: 한셀은 e^x 를 pow(e, x) 로 구해 정의식 그대로 계산한다 (SINH(10) = 11013.232874703386,
    // SINH(1E-10) = 1.000000082740371E-10, ASINH(-1) ≠ −ASINH(1), 대조 실험 math). 넘치면 #NUM!.
    f("SINH", 1, 1, V, |c, a| {
        un(c, a, |x| Ok((ex(x) - ex(-x)) / 2.0))
    }),
    f("COSH", 1, 1, V, |c, a| {
        un(c, a, |x| Ok((ex(x) + ex(-x)) / 2.0))
    }),
    f("TANH", 1, 1, V, |c, a| {
        un(c, a, |x| {
            let (p, q) = (ex(x), ex(-x));
            let r = (p - q) / (p + q);
            Ok(if r.is_nan() { x.signum() } else { r })
        })
    }),
    f("ASINH", 1, 1, V, |c, a| {
        un(c, a, |x| Ok((x + (x * x + 1.0).sqrt()).mln()))
    }),
    f("ACOSH", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x < 1.0 {
                Err(ErrorKind::Num)
            } else {
                Ok((x + (x * x - 1.0).sqrt()).mln())
            }
        })
    }),
    f("ATANH", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x.abs() >= 1.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(0.5 * ((1.0 + x) / (1.0 - x)).mln())
            }
        })
    }),
    f("COT", 1, 1, V, |c, a| {
        un(c, a, |x| {
            let x = trig_arg(x)?;
            let t = x.mtan();
            if t == 0.0 {
                Err(ErrorKind::Div0)
            } else {
                Ok(1.0 / t)
            }
        })
    }),
    // CSC(0)·CSCH(0) 은 #NUM! (1/0, 대조 실험 math)
    f("CSC", 1, 1, V, |c, a| {
        un(c, a, |x| Ok(1.0 / trig_arg(x)?.msin()))
    }),
    f("SEC", 1, 1, V, |c, a| {
        un(c, a, |x| Ok(1.0 / trig_arg(x)?.mcos()))
    }),
    // COTH·CSCH·SECH 는 표준 tanh·sinh·cosh 의 역수 (COTH(1E+15) = 1, CSCH(1E+15) = 0)
    f("COTH", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x == 0.0 {
                Err(ErrorKind::Div0)
            } else {
                Ok(1.0 / x.mtanh())
            }
        })
    }),
    f("CSCH", 1, 1, V, |c, a| un(c, a, |x| Ok(1.0 / x.msinh()))),
    f("SECH", 1, 1, V, |c, a| un(c, a, |x| Ok(1.0 / x.mcosh()))),
    f("ACOT", 1, 1, V, |c, a| {
        un(c, a, |x| Ok(std::f64::consts::FRAC_PI_2 - x.matan()))
    }),
    f("ACOTH", 1, 1, V, |c, a| {
        un(c, a, |x| {
            if x.abs() <= 1.0 {
                Err(ErrorKind::Num)
            } else {
                Ok(0.5 * ((x + 1.0) / (x - 1.0)).mln())
            }
        })
    }),
    f("DEGREES", 1, 1, V, |c, a| un(c, a, |x| Ok(x.to_degrees()))),
    f("RADIANS", 1, 1, V, |c, a| un(c, a, |x| Ok(x.to_radians()))),
    f("INT", 1, 1, V, |c, a| un(c, a, |x| Ok(x.floor()))),
    f("TRUNC", 1, 2, V, trunc),
    f("ROUND", 2, 2, V, |c, a| round_fn(c, a, Rounding::HalfAway)),
    f("ROUNDUP", 2, 2, V, |c, a| round_fn(c, a, Rounding::Up)),
    f("ROUNDDOWN", 2, 2, V, |c, a| round_fn(c, a, Rounding::Down)),
    f("MROUND", 2, 2, V, mround),
    f("CEILING", 2, 2, V, ceiling),
    f("FLOOR", 2, 2, V, floor),
    f("CEILING.MATH", 1, 3, V, |c, a| math_round(c, a, true)),
    f("FLOOR.MATH", 1, 3, V, |c, a| math_round(c, a, false)),
    f("CEILING.PRECISE", 1, 2, V, |c, a| precise(c, a, true)),
    f("ISO.CEILING", 1, 2, V, |c, a| precise(c, a, true)),
    f("FLOOR.PRECISE", 1, 2, V, |c, a| precise(c, a, false)),
    f("EVEN", 1, 1, V, |c, a| un(c, a, |x| Ok(even_odd(x, true)))),
    f("ODD", 1, 1, V, |c, a| un(c, a, |x| Ok(even_odd(x, false)))),
    f("MOD", 2, 2, V, modulo),
    f("QUOTIENT", 2, 2, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0));
        let d = try_v!(num_arg(c, a, 1));
        if d == 0.0 {
            return Value::Error(ErrorKind::Div0);
        }
        num((n / d).trunc())
    }),
    f("FACT", 1, 1, V, |c, a| un(c, a, fact)),
    f("FACTDOUBLE", 1, 1, V, |c, a| {
        un(c, a, |x| {
            // FACTDOUBLE(-1) 은 #NUM!, FACTDOUBLE(-0.5) 는 1 (버린 값이 음수인가, 대조 실험 math)
            let n = x.trunc();
            if n < 0.0 {
                return Err(ErrorKind::Num);
            }
            let mut r = 1.0f64;
            let mut k = n;
            // 300!! 이 f64 의 끝 근처 — 넘치면 #NUM!
            while k > 1.0 {
                r *= k;
                if !r.is_finite() {
                    return Err(ErrorKind::Num);
                }
                k -= 2.0;
            }
            Ok(r)
        })
    }),
    f("COMBIN", 2, 2, V, |c, a| {
        let (nr, kr) = (try_v!(num_arg(c, a, 0)), try_v!(num_arg(c, a, 1)));
        let (n, k) = (nr.trunc(), kr.trunc());
        // n·k 는 2^31 − 2 까지 (COMBIN(2147483647,1) = #NUM!, 대조 실험 conv4)
        if nr < 0.0 || kr < 0.0 || k > n || n >= 2_147_483_647.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(super::special::binom_coef(n, k))
    }),
    f("COMBINA", 2, 2, V, |c, a| {
        let (nr, kr) = (try_v!(num_arg(c, a, 0)), try_v!(num_arg(c, a, 1)));
        let (n, k) = (nr.trunc(), kr.trunc());
        // COMBINA(0,0) 은 #NUM! (대조 실험 math)
        if nr < 0.0 || kr < 0.0 || n == 0.0 || n >= 2_147_483_647.0 || k >= 2_147_483_647.0 {
            return Value::Error(ErrorKind::Num);
        }
        num(super::special::binom_coef(n + k - 1.0, k))
    }),
    f("MULTINOMIAL", 1, MANY, R, multinomial),
    f("GCD", 1, MANY, R, |c, a| gcd_lcm(c, a, true)),
    f("LCM", 1, MANY, R, |c, a| gcd_lcm(c, a, false)),
    f("SUM", 1, MANY, R, sum),
    f("PRODUCT", 1, MANY, R, product),
    f("SUMSQ", 1, MANY, R, |c, a| {
        let v = try_v!(collect_numbers(c, a, Agg::Numbers));
        num(v.iter().map(|x| x * x).sum())
    }),
    f("SUMIF", 2, 3, RVR, sumif),
    pairs(f("SUMIFS", 3, MANY, RRV, sumifs), 2),
    f("SUMPRODUCT", 1, MANY, A, sumproduct),
    f("SUMX2MY2", 2, 2, A, |c, a| {
        sum_xy(c, a, |x, y| x * x - y * y)
    }),
    f("SUMX2PY2", 2, 2, A, |c, a| {
        sum_xy(c, a, |x, y| x * x + y * y)
    }),
    f("SUMXMY2", 2, 2, A, |c, a| {
        sum_xy(c, a, |x, y| (x - y) * (x - y))
    }),
    f("SERIESSUM", 4, 4, VVA, seriessum),
    f("SUBTOTAL", 1, MANY, VR, |c, a| subtotal(c, a, false)),
    f("SUBTOTALA", 1, MANY, VR, |c, a| subtotal(c, a, true)),
    f("AGGREGATE", 3, MANY, &[Arg::V, Arg::V, Arg::R], aggregate),
    f("MMULT", 2, 2, A, mmult),
    f("MDETERM", 1, 1, A, mdeterm),
    f("MINVERSE", 1, 1, A, minverse),
    f("MUNIT", 1, 1, V, |c, a| {
        let n = try_v!(num_arg(c, a, 0)).trunc();
        if n < 1.0 || n > 16_384.0 {
            return Value::Error(ErrorKind::Value);
        }
        let n = n as usize;
        let mut d = vec![Value::Number(0.0); n * n];
        for i in 0..n {
            d[i * n + i] = Value::Number(1.0);
        }
        Value::from(Array::new(n, n, d))
    }),
    volatile(f("RAND", 0, 0, V, |c, _| {
        Value::Number(c.engine().random())
    })),
    volatile(f("RANDBETWEEN", 2, 2, V, |c, a| {
        let lo = try_v!(num_arg(c, a, 0)).ceil();
        let hi = try_v!(num_arg(c, a, 1)).floor();
        if lo > hi {
            return Value::Error(ErrorKind::Num);
        }
        let r = c.engine().random();
        num(lo + (r * (hi - lo + 1.0)).floor())
    })),
    f("ROMAN", 1, 2, V, roman),
    f("ARABIC", 1, 1, V, arabic),
    f("BASE", 2, 3, V, base),
    f("DECIMAL", 2, 2, V, decimal),
    f("SEQUENCE", 1, 4, V, sequence),
    volatile(f("RANDARRAY", 0, 5, V, randarray)),
];

/// 숫자 하나 → 숫자 하나
/// 쌍곡 함수의 e^x (pow(e, x))
fn ex(x: f64) -> f64 {
    std::f64::consts::E.mpow(x)
}

fn un(ctx: &mut Ctx, a: &[Value], f: impl Fn(f64) -> Result<f64, ErrorKind>) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    value::num_r(f(x))
}

/// 삼각 함수 인수 한도 (|x| < 2^27)
fn trig_arg(x: f64) -> Result<f64, ErrorKind> {
    if x.abs() >= 134_217_728.0 {
        Err(ErrorKind::Num)
    } else {
        Ok(x)
    }
}

fn log(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let b = try_v!(num_or(ctx, a, 1, 10.0));
    if x <= 0.0 || b <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    if b == 1.0 {
        return Value::Error(ErrorKind::Div0);
    }
    num(x.mln() / b.mln())
}

#[derive(Clone, Copy)]
pub enum Rounding {
    HalfAway,
    Up,
    Down,
}

/// 10진 자릿수 반올림 (유효숫자 15자리로 맞춘 뒤 자른다: ROUND(2.675,2) = 2.68)
pub fn round_digits(x: f64, digits: f64, mode: Rounding) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let d = digits.trunc().clamp(-400.0, 400.0) as i32;
    let apply = |y: f64| match mode {
        Rounding::HalfAway => y.abs().round().copysign(y),
        Rounding::Up => y.abs().ceil().copysign(y),
        Rounding::Down => y.abs().floor().copysign(y),
    };
    if d >= 0 {
        if d > 308 {
            return x;
        }
        let f = 10f64.mpowi(d);
        let y = x * f;
        if !y.is_finite() || y.abs() >= 9.007_199_254_740_992e15 {
            return x;
        }
        let r = apply(round15(y)) / f;
        if r.is_finite() { r } else { x }
    } else {
        let f = 10f64.mpowi(-d);
        if !f.is_finite() {
            // ROUND·ROUNDDOWN(1.5,-400) = 0, ROUNDUP(1.5,-400) = #NUM! (대조 실험 math)
            return if matches!(mode, Rounding::Up) {
                f64::NAN
            } else {
                0.0
            };
        }
        let y = round15(x / f);
        apply(y) * f
    }
}

/// 자릿수는 32비트 정수로 받는다: 반올림한 값이 범위 밖이면 #NUM!, 2147483647 은 0 (대조 실험 misc4·misc6)
fn digits_limit(d: f64) -> Option<Value> {
    let n = d.round();
    if !(-2_147_483_648.0..=2_147_483_647.0).contains(&n) {
        Some(Value::Error(ErrorKind::Num))
    } else if n == 2_147_483_647.0 {
        Some(Value::Number(0.0))
    } else {
        None
    }
}

fn round_fn(ctx: &mut Ctx, a: &[Value], mode: Rounding) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d = try_v!(num_arg(ctx, a, 1));
    if let Some(v) = digits_limit(d) {
        return v;
    }
    // ROUND·ROUNDUP·ROUNDDOWN(x,−2147483648) = 0
    if d.round() == -2_147_483_648.0 {
        return Value::Number(0.0);
    }
    num(round_digits(x, d, mode))
}

fn trunc(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let d = try_v!(num_or(ctx, a, 1, 0.0));
    if let Some(v) = digits_limit(d) {
        return v;
    }
    // TRUNC 는 x·10^d 가 넘치면 0 (TRUNC(1234.5678,306) = 0; ROUNDDOWN 은 x 그대로, 대조 실험 misc6)
    if d.trunc() >= 0.0 {
        let f = 10f64.mpowi(d.trunc().min(400.0) as i32);
        if !f.is_finite() || !(x * f).is_finite() {
            return Value::Number(0.0);
        }
    } else if d.trunc() <= -324.0 {
        // 10^d 가 0 이 되는 자리 (TRUNC(1023,-323) = 0, TRUNC(1023,-324) = #NUM!, 대조 실험 conv4)
        return Value::Error(ErrorKind::Num);
    }
    num(round_digits(x, d, Rounding::Down))
}

fn mround(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0));
    let m = try_v!(num_arg(ctx, a, 1));
    if m == 0.0 || n == 0.0 {
        return Value::Number(0.0);
    }
    if (n > 0.0) != (m > 0.0) {
        return Value::Error(ErrorKind::Num);
    }
    let q = round15(n / m);
    num(q.abs().round().copysign(q) * m)
}

fn ceiling(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let s = try_v!(num_arg(ctx, a, 1));
    if x == 0.0 || s == 0.0 {
        return Value::Number(0.0);
    }
    if x > 0.0 && s < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let q = round15(x / s);
    if x < 0.0 && s > 0.0 {
        // 음수를 양수 단위로: +∞ 쪽 (0 쪽)
        return num(q.ceil() * s);
    }
    num(q.ceil() * s)
}

fn floor(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let s = try_v!(num_arg(ctx, a, 1));
    if x == 0.0 {
        return Value::Number(0.0);
    }
    if s == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    if x > 0.0 && s < 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    let q = round15(x / s);
    num(q.floor() * s)
}

/// CEILING.MATH / FLOOR.MATH (숫자, [단위=1], [음수 방향])
fn math_round(ctx: &mut Ctx, a: &[Value], up: bool) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let s = try_v!(num_or(ctx, a, 1, 1.0)).abs();
    let mode = try_v!(num_or(ctx, a, 2, 0.0));
    if s == 0.0 || x == 0.0 {
        return Value::Number(0.0);
    }
    // 몫은 반올림 보정 없이 (FLOOR.MATH(0.3,0.1) = 0.2, CEILING.MATH(0.3,0.1) = 0.30000000000000004)
    let q = x / s;
    let r = if x < 0.0 && mode != 0.0 {
        // 음수: 0 에서 멀어지는 쪽(올림) / 0 쪽(내림)
        if up { q.floor() } else { q.ceil() }
    } else if up {
        q.ceil()
    } else {
        q.floor()
    };
    num(r * s)
}

fn precise(ctx: &mut Ctx, a: &[Value], up: bool) -> Value {
    // FLOOR.PRECISE 는 배수가 0 이면 첫 인수의 오류·글을 보지 않고 0 (FLOOR.PRECISE(NA(),0) = 0, CEILING.PRECISE 는
    // #N/A — 대조 실험 conv3)
    if !up && matches!(num_or(ctx, a, 1, 1.0), Ok(s) if s == 0.0) {
        return Value::Number(0.0);
    }
    let x = try_v!(num_arg(ctx, a, 0));
    let s = try_v!(num_or(ctx, a, 1, 1.0)).abs();
    if s == 0.0 || x == 0.0 {
        return Value::Number(0.0);
    }
    let q = round15(x / s);
    num(if up { q.ceil() } else { q.floor() } * s)
}

fn even_odd(x: f64, even: bool) -> f64 {
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    let a = round15(x.abs()).ceil();
    let r = if even {
        if a % 2.0 == 0.0 { a } else { a + 1.0 }
    } else if a % 2.0 == 1.0 {
        a
    } else {
        a + 1.0
    };
    s * r
}

/// MOD: 정확한 나머지 fmod 에 부호가 다르면 나누는 수를 더한다 (MOD(10000000000,0.3) = 0.10000037007434154,
/// MOD(1E+20,3) = 1, MOD(-1,1E-10) = 3.6E-17, 대조 실험 math)
fn modulo(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0));
    let d = try_v!(num_arg(ctx, a, 1));
    if d == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let r = n % d;
    if r != 0.0 && (r < 0.0) != (d < 0.0) {
        return num(r + d);
    }
    // 나머지 0 은 늘 +0 (MOD(-4,2) = 0)
    num(if r == 0.0 { 0.0 } else { r })
}

pub fn fact(x: f64) -> Result<f64, ErrorKind> {
    // FACT(-0.5) 도 #NUM! (버리기 전 값으로, 대조 실험 math)
    let n = x.trunc();
    if x < 0.0 {
        return Err(ErrorKind::Num);
    }
    if n > 170.0 {
        return Err(ErrorKind::Num);
    }
    let mut r = 1.0;
    let mut k = 2.0;
    while k <= n {
        r *= k;
        k += 1.0;
    }
    Ok(r)
}

fn multinomial(ctx: &mut Ctx, a: &[Value]) -> Value {
    // 글·논리 인수는 (범위 안의 것도) #VALUE! — GCD 와 같다 (MULTINOMIAL(3,"2"), 대조 실험 conv2)
    let v = try_v!(strict_numbers(ctx, a));
    let mut total = 0.0;
    let mut denom = 1.0;
    for x in v {
        let n = x.trunc();
        if x < 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        total += n;
        denom *= try_v!(fact(n));
    }
    num(try_v!(fact(total)) / denom)
}

/// GCD·LCM: 글·논리 인수(범위 안의 것도)는 #VALUE! (GCD("12",18), 대조 실험 math). 소수를 버리기 전에 0 보다 작으면
/// #NUM! (GCD(-0.5,4)). 1 보다 작거나 2^63 이상인 값은 0 으로 보고 (GCD(0.99) = 0, GCD(1E20) = 0, GCD(1E20,2) = 2),
/// 두 값 가운데 하나가 0 이면 다른 값을 소수째로 돌려주고 (GCD(2.9) = 2.9, GCD(2.9,0) = 2.9), 둘 다 있으면 버린
/// 정수의 최대공약수 (GCD(2.9,2.9) = 2). 최소공배수는 원래 값의 곱 / 버린 값들의 최대공약수 (LCM(2.5,5) = 12.5,
/// LCM(2.9,0) = 0, 대조 실험 misc4·conv3·conv4).
fn gcd_lcm(ctx: &mut Ctx, a: &[Value], gcd: bool) -> Value {
    let v = try_v!(strict_numbers(ctx, a));
    let mut acc: Option<f64> = None;
    for x in v {
        if x < 0.0 || !x.is_finite() {
            return Value::Error(ErrorKind::Num);
        }
        let x = if !(1.0..9_223_372_036_854_775_808.0).contains(&x) {
            0.0
        } else {
            x
        };
        acc = Some(match acc {
            None => x,
            Some(m) if gcd && m == 0.0 => x,
            Some(m) if gcd && x == 0.0 => m,
            Some(m) if !gcd && (m == 0.0 || x == 0.0) => 0.0,
            Some(m) => {
                let g = gcd2(m.trunc(), x.trunc());
                if gcd { g } else { m * x / g }
            }
        });
    }
    num(acc.unwrap_or(0.0))
}

/// 숫자만 받는 인수 모으기: 글·논리는 #VALUE!, 참조 안의 빈 칸은 건너뛰고 직접 준 빈 값은 0
fn strict_numbers(ctx: &mut Ctx, a: &[Value]) -> Result<Vec<f64>, ErrorKind> {
    let mut v = Vec::new();
    for x in a {
        for y in super::flatten(ctx, x) {
            match y {
                // 인수 순서대로 하나씩: 앞의 음수가 뒤의 글·오류보다 먼저 #NUM! (GCD(-1,"3") = #NUM!,
                // GCD("3",-1) = #VALUE!, 대조 실험 conv8)
                Value::Number(n) if n < 0.0 => return Err(ErrorKind::Num),
                Value::Number(n) => v.push(n),
                Value::Empty if matches!(x, Value::Ref(_)) => {}
                Value::Empty => v.push(0.0),
                Value::Error(e) => return Err(e),
                _ => return Err(ErrorKind::Value),
            }
        }
    }
    Ok(v)
}

fn gcd2(mut a: f64, mut b: f64) -> f64 {
    while b != 0.0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

fn sum(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    num(kahan_sum(&v))
}

/// 합 (보정 합산 — 스프레드시트처럼 0.1 을 열 번 더하면 1)
pub fn kahan_sum(v: &[f64]) -> f64 {
    let mut s = 0.0;
    for x in v {
        s += x;
    }
    s
}

fn product(ctx: &mut Ctx, a: &[Value]) -> Value {
    let v = try_v!(collect_numbers(ctx, a, Agg::Numbers));
    if v.is_empty() {
        return Value::Number(0.0);
    }
    num(v.iter().product())
}

// ---------------------------------------------------------------------------
// 조건 집계
// ---------------------------------------------------------------------------

/// 범위 인수 → 칸 배열 (열·행 전체는 쓰는 범위까지)
pub fn grid(ctx: &mut Ctx, v: &Value) -> Result<Rc<Array>, ErrorKind> {
    match v {
        Value::Ref(r) if r.len() == 1 => ctx.area_to_array(&r[0]).map(Rc::new),
        Value::Ref(_) => Err(ErrorKind::Value),
        Value::Error(e) => Err(*e),
        other => to_array(ctx, other),
    }
}

/// 조건 범위와 같은 크기로 늘린 합 범위 (왼쪽 위 칸에서 시작)
pub fn resized(range: &Value, target: &Value) -> Value {
    match (first_area(range), first_area(target)) {
        (Some(r), Some(t)) => {
            let rows = r.r1 - r.r0;
            let cols = r.c1 - r.c0;
            let area = Area {
                r1: (t.r0 + rows).min(MAX_ROWS - 1),
                c1: (t.c0 + cols).min(MAX_COLS - 1),
                ..t
            };
            Value::Ref(Rc::new(vec![area]))
        }
        _ => target.clone(),
    }
}

/// (범위, 조건) 쌍들이 모두 맞는 칸의 표시. 범위 크기가 다르면 #VALUE!
pub fn ifs_mask(
    ctx: &mut Ctx,
    pairs: &[(&Value, &Value)],
) -> Result<(usize, usize, Vec<bool>), ErrorKind> {
    let mut dims: Option<(usize, usize)> = None;
    let mut mask: Vec<bool> = Vec::new();
    for (range, crit) in pairs {
        // 범위 인수는 참조여야 한다 (배열 상수·값은 #VALUE!, 대조 실험 refargs)
        if !matches!(range, Value::Ref(_)) {
            return Err(ErrorKind::Value);
        }
        let g = grid(ctx, range)?;
        match dims {
            None => {
                dims = Some((g.rows, g.cols));
                mask = vec![true; g.rows * g.cols];
            }
            Some(d) => {
                if d != (g.rows, g.cols) {
                    return Err(ErrorKind::Value);
                }
            }
        }
        let crit = match crit {
            Value::Ref(r) => ctx.intersect_ref(r),
            Value::Array(a) => a.data.first().cloned().unwrap_or(Value::Empty),
            o => (*o).clone(),
        };
        let c = Criterion::parse(&crit, ctx.date1904());
        for (m, v) in mask.iter_mut().zip(&g.data) {
            if *m && !c.matches(v) {
                *m = false;
            }
        }
    }
    let (r, c) = dims.unwrap_or((0, 0));
    Ok((r, c, mask))
}

fn sumif(ctx: &mut Ctx, a: &[Value]) -> Value {
    // 합 범위도 참조여야 한다 (SUMIF(…,{4,5,6}) = #VALUE!, 대조 실험 refargs)
    if a.get(2)
        .is_some_and(|t| !matches!(t, Value::Ref(_) | Value::Empty))
    {
        return Value::Error(ErrorKind::Value);
    }
    let target = match opt(a, 2) {
        Some(t) => resized(&a[0], t),
        None => a[0].clone(),
    };
    let (_, _, mask) = try_v!(ifs_mask(ctx, &[(&a[0], &a[1])]));
    let vals = try_v!(grid(ctx, &target));
    // SUMIF 는 합 범위의 오류를 돌려주고, SUMIFS 는 건너뛴다 (대조 실험 math)
    let mut s = 0.0;
    for (m, v) in mask.iter().zip(&vals.data) {
        if *m {
            match v {
                Value::Number(n) => s += n,
                Value::Error(e) => return Value::Error(*e),
                _ => {}
            }
        }
    }
    num(s)
}

fn sumifs(ctx: &mut Ctx, a: &[Value]) -> Value {
    if a.len() % 2 != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let pairs: Vec<(&Value, &Value)> = a[1..].chunks(2).map(|p| (&p[0], &p[1])).collect();
    if !matches!(a[0], Value::Ref(_)) {
        return Value::Error(ErrorKind::Value);
    }
    let (r, c, mask) = try_v!(ifs_mask(ctx, &pairs));
    let vals = try_v!(grid(ctx, &a[0]));
    if (vals.rows, vals.cols) != (r, c) {
        return Value::Error(ErrorKind::Value);
    }
    let mut s = 0.0;
    for (m, v) in mask.iter().zip(&vals.data) {
        if *m && let Value::Number(n) = v {
            s += n;
        }
    }
    num(s)
}

fn sumproduct(ctx: &mut Ctx, a: &[Value]) -> Value {
    let mut arrays = Vec::with_capacity(a.len());
    for v in a {
        // 글·논리 스칼라 인수는 #VALUE! (SUMPRODUCT("3"), 대조 실험 math)
        if matches!(v, Value::Text(_) | Value::Bool(_)) {
            return Value::Error(ErrorKind::Value);
        }
        // 오류 낱값도 한 칸 배열 — 크기가 안 맞으면 오류보다 #VALUE! 가 먼저
        // (SUMPRODUCT(범위, IMPOWER("","-i")) = #VALUE!, 대조 실험 rand)
        if let Value::Error(_) = v {
            arrays.push(Rc::new(Array::new(1, 1, vec![v.clone()])));
            continue;
        }
        arrays.push(try_v!(to_array(ctx, v)));
    }
    let (rows, cols) = (arrays[0].rows, arrays[0].cols);
    if arrays.iter().any(|x| x.rows != rows || x.cols != cols) {
        return Value::Error(ErrorKind::Value);
    }
    let mut total = 0.0;
    for i in 0..rows * cols {
        let mut p = 1.0;
        for arr in &arrays {
            match &arr.data[i] {
                Value::Number(n) => p *= n,
                Value::Error(e) => return Value::Error(*e),
                _ => p = 0.0,
            }
        }
        total += p;
    }
    num(total)
}

fn sum_xy(ctx: &mut Ctx, a: &[Value], f: fn(f64, f64) -> f64) -> Value {
    let x = try_v!(to_array(ctx, &a[0]));
    let y = try_v!(to_array(ctx, &a[1]));
    if x.data.len() != y.data.len() {
        return Value::Error(ErrorKind::NA);
    }
    let mut s = 0.0;
    let mut any = false;
    // 오류 값이 든 쌍은 건너뛴다 (SUMX2MY2(Data!H1:H10,Data!A1:A10) = 86, 대조 실험 math)
    for (p, q) in x.data.iter().zip(&y.data) {
        if let (Value::Number(p), Value::Number(q)) = (p, q) {
            s += f(*p, *q);
            any = true;
        }
    }
    if !any {
        return Value::Error(ErrorKind::Div0);
    }
    num(s)
}

fn seriessum(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(num_arg(ctx, a, 0));
    let n = try_v!(num_arg(ctx, a, 1));
    let m = try_v!(num_arg(ctx, a, 2));
    let coef = try_v!(to_array(ctx, &a[3]));
    let mut s = 0.0;
    for (i, c) in coef.data.iter().enumerate() {
        let c = match c {
            Value::Number(c) => *c,
            Value::Error(e) => return Value::Error(*e),
            _ => return Value::Error(ErrorKind::Value),
        };
        s += c * x.mpow(n + i as f64 * m);
    }
    num(s)
}

/// SUBTOTAL / SUBTOTALA: 숨긴 행(101–111)과 다른 부분합 셀을 뺀 집계
/// SUBTOTAL 과 한셀의 SUBTOTALA. SUBTOTALA 는 계산은 SUBTOTAL 과 같고(숫자만 센다) 숨긴 행 처리가 거꾸로다:
/// 1–11 이 숨긴 행을 빼고 101–111 이 넣는다 (SUBTOTALA(9,Hid!A1:A10) = 46, (109,…) = 55, 대조 실험 newer).
/// 참조가 아닌 인수·여러 시트·여러 영역은 #N/A (SUBTOTAL(9,1,2), 대조 실험 math).
fn subtotal(ctx: &mut Ctx, a: &[Value], hancom_a: bool) -> Value {
    let code = try_v!(num_arg(ctx, a, 0)).trunc() as i64;
    let (kind, skip_hidden) = match code {
        1..=11 => (code, hancom_a),
        101..=111 => (code - 100, !hancom_a),
        _ => return Value::Error(ErrorKind::Value),
    };
    let with_text = false;
    let mut vals: Vec<Value> = Vec::new();
    for arg in &a[1..] {
        match arg {
            Value::Ref(r) if r.len() > 1 || r.iter().any(|x| x.sheet != x.sheet_last) => {
                return Value::Error(ErrorKind::NA);
            }
            Value::Ref(r) => {
                for area in r.iter() {
                    let mut cells: Vec<(u32, u32, u32, Value)> = Vec::new();
                    for s in area.sheet..=area.sheet_last {
                        let one = Area {
                            sheet: s,
                            sheet_last: s,
                            ..*area
                        };
                        ctx.for_each_in_area(&one, |r, c, v| cells.push((s, r, c, v.clone())));
                    }
                    for (s, r, c, v) in cells {
                        let eng = ctx.engine();
                        if area.book == 0 {
                            if eng.cell_has_subtotal(s, r, c) {
                                continue;
                            }
                            if eng.row_hidden(s, r) && (skip_hidden || eng.sheet_filtered(s)) {
                                continue;
                            }
                        }
                        vals.push(v);
                    }
                }
            }
            Value::Error(e) => return Value::Error(*e),
            _ => return Value::Error(ErrorKind::NA),
        }
    }
    let mode = if with_text {
        Agg::NumbersA
    } else {
        Agg::Numbers
    };
    if kind == 3 {
        return Value::Number(vals.iter().filter(|v| !v.is_empty()).count() as f64);
    }
    if kind == 2 {
        let n = vals
            .iter()
            .filter(|v| match v {
                Value::Number(_) => true,
                Value::Bool(_) | Value::Text(_) => with_text,
                _ => false,
            })
            .count();
        return Value::Number(n as f64);
    }
    let mut nums = Vec::new();
    for v in &vals {
        match v {
            Value::Number(n) => nums.push(*n),
            Value::Error(e) => return Value::Error(*e),
            Value::Bool(b) if mode == Agg::NumbersA => nums.push(*b as u8 as f64),
            Value::Text(_) if mode == Agg::NumbersA => nums.push(0.0),
            _ => {}
        }
    }
    super::stat::basic_aggregate(kind as u8, &nums)
}

/// AGGREGATE(함수 1–19, 옵션 0–7, 참조…)
fn aggregate(ctx: &mut Ctx, a: &[Value]) -> Value {
    let func = try_v!(num_arg(ctx, a, 0)).trunc() as i64;
    let opt_ = try_v!(num_arg(ctx, a, 1)).trunc() as i64;
    if !(1..=19).contains(&func) || !(0..=7).contains(&opt_) {
        return Value::Error(ErrorKind::Value);
    }
    let ignore_hidden = matches!(opt_, 1 | 3 | 5 | 7);
    let ignore_errors = matches!(opt_, 2 | 3 | 6 | 7);
    let ignore_nested = matches!(opt_, 0..=3);
    let refs_end = if func >= 14 { a.len() - 1 } else { a.len() };
    let mut nums = Vec::new();
    for arg in &a[2..refs_end.max(3)] {
        match arg {
            Value::Ref(r) => {
                for area in r.iter() {
                    let mut cells = Vec::new();
                    ctx.for_each_in_area(area, |r, c, v| cells.push((r, c, v.clone())));
                    for (r, c, v) in cells {
                        let eng = ctx.engine();
                        if area.book == 0 {
                            if ignore_nested && eng.cell_has_subtotal(area.sheet, r, c) {
                                continue;
                            }
                            if ignore_hidden && eng.row_hidden(area.sheet, r) {
                                continue;
                            }
                        }
                        match v {
                            Value::Number(n) => nums.push(n),
                            Value::Error(e) if !ignore_errors => return Value::Error(e),
                            _ => {}
                        }
                    }
                }
            }
            Value::Array(arr) => {
                for v in &arr.data {
                    match v {
                        Value::Number(n) => nums.push(*n),
                        Value::Error(e) if !ignore_errors => return Value::Error(*e),
                        _ => {}
                    }
                }
            }
            Value::Number(n) => nums.push(*n),
            Value::Error(e) if !ignore_errors => return Value::Error(*e),
            _ => {}
        }
    }
    if func <= 13 {
        let kind = match func {
            12 => return super::stat::median(&nums),
            13 => return super::stat::mode_sngl(&nums),
            k => k as u8,
        };
        return super::stat::basic_aggregate(kind, &nums);
    }
    let k = try_v!(ctx.num(a.last().unwrap()));
    match func {
        14 => super::stat::large_small(&nums, k, true),
        15 => super::stat::large_small(&nums, k, false),
        16 => super::stat::percentile_inc(&nums, k),
        17 => super::stat::quartile(&nums, k, true),
        18 => super::stat::percentile_exc(&nums, k),
        _ => super::stat::quartile(&nums, k, false),
    }
}

// ---------------------------------------------------------------------------
// 행렬
// ---------------------------------------------------------------------------

fn numeric_matrix(ctx: &mut Ctx, v: &Value) -> Result<(usize, usize, Vec<f64>), ErrorKind> {
    let a = to_array(ctx, v)?;
    let mut m = Vec::with_capacity(a.data.len());
    for x in &a.data {
        match x {
            Value::Number(n) => m.push(*n),
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        }
    }
    Ok((a.rows, a.cols, m))
}

fn mmult(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (ar, ac, x) = try_v!(numeric_matrix(ctx, &a[0]));
    let (br, bc, y) = try_v!(numeric_matrix(ctx, &a[1]));
    if ac != br {
        return Value::Error(ErrorKind::Value);
    }
    let mut out = Vec::with_capacity(ar * bc);
    for i in 0..ar {
        for j in 0..bc {
            let mut s = 0.0;
            for k in 0..ac {
                s += x[i * ac + k] * y[k * bc + j];
            }
            out.push(num(s));
        }
    }
    Value::from(Array::new(ar, bc, out))
}

/// 한셀의 피벗 고르기 (MDETERM·MINVERSE 공통): 아직 쓰지 않은 행·열의 칸 가운데 절댓값이 가장 큰 칸
/// (같으면 행 우선으로 먼저 만난 칸 — 완전 피벗). 그 절댓값이 1E-16 보다 작으면 특이 행렬로 본다
/// (마지막 피벗 5.55E-17 은 특이, 1.11E-16 은 정칙, 대조 실험 x9·x10)
fn full_pivot(m: &[f64], n: usize, used_r: &[bool], used_c: &[bool]) -> Option<(usize, usize)> {
    let mut big = 0.0;
    let mut at = None;
    for j in (0..n).filter(|&j| !used_r[j]) {
        for k in (0..n).filter(|&k| !used_c[k]) {
            let v = m[j * n + k].abs();
            if v > big {
                big = v;
                at = Some((j, k));
            }
        }
    }
    if big < 1e-16 { None } else { at }
}

/// 자리 바꿈 수의 홀짝
fn odd_permutation(p: &[usize]) -> bool {
    let mut c = 0;
    for i in 0..p.len() {
        for j in i + 1..p.len() {
            if p[i] > p[j] {
                c += 1;
            }
        }
    }
    c % 2 == 1
}

/// 행렬식: 완전 피벗 소거. 피벗 행을 피벗으로 나눈 뒤(곱하기 1/p 가 아니라 나누기) 남은 행에서
/// 피벗 열 값 × 피벗 행을 뺀다 — 끝자리가 한셀과 같다 (MDETERM({1,2,3;4,5,6;7,8,9}) = −3.996802888650564E-15,
/// {4,7;2,6} = 9.999999999999998, 대조 실험 x7). 피벗 곱에 행·열 자리 바꿈 부호
fn determinant(n: usize, mut m: Vec<f64>) -> f64 {
    let (mut ur, mut uc) = (vec![false; n], vec![false; n]);
    let (mut rows, mut cols) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut det = 1.0;
    for _ in 0..n {
        let Some((pr, pc)) = full_pivot(&m, n, &ur, &uc) else {
            return 0.0;
        };
        ur[pr] = true;
        uc[pc] = true;
        rows.push(pr);
        cols.push(pc);
        let p = m[pr * n + pc];
        det *= p;
        for l in (0..n).filter(|&l| !uc[l]) {
            m[pr * n + l] /= p;
        }
        for ll in (0..n).filter(|&ll| !ur[ll]) {
            let d = m[ll * n + pc];
            for l in (0..n).filter(|&l| !uc[l]) {
                m[ll * n + l] -= m[pr * n + l] * d;
            }
        }
    }
    if odd_permutation(&rows) != odd_permutation(&cols) {
        -det
    } else {
        det
    }
}

fn mdeterm(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (r, c, m) = try_v!(numeric_matrix(ctx, &a[0]));
    if r != c {
        return Value::Error(ErrorKind::Value);
    }
    num(determinant(r, m))
}

/// 역행렬: 행을 옮기지 않는 제자리 가우스-요르단 (완전 피벗). 피벗 (r, c) 에서 피벗 칸은 1/p, 피벗 행의
/// 다른 칸은 ÷p, 다른 행의 피벗 열 칸은 −d/p, 나머지는 −(피벗 행 칸 × d). 끝나면
/// 역[c_i][r_j] = 제자리[r_i][c_j] 로 자리를 맞춘다. 끝자리까지 한셀과 같다 (대조 실험 x7·x10).
/// 피벗이 없으면 #NUM! — 한셀은 그때까지의 제자리 행렬을 그대로 내는데 (MINVERSE({1,2;2,4}) =
/// {0,−0.5;0.5,0.25}) 이는 `CalcOptions::hcell_bugs` 일 때만 흉내 낸다
fn minverse(ctx: &mut Ctx, a: &[Value]) -> Value {
    let (n, c, mut m) = try_v!(numeric_matrix(ctx, &a[0]));
    if n != c {
        return Value::Error(ErrorKind::Value);
    }
    let (mut ur, mut uc) = (vec![false; n], vec![false; n]);
    let mut piv = Vec::with_capacity(n);
    for _ in 0..n {
        let Some((pr, pc)) = full_pivot(&m, n, &ur, &uc) else {
            if ctx.engine().hcell_bugs {
                return Value::from(Array::new(n, n, m.into_iter().map(num).collect()));
            }
            return Value::Error(ErrorKind::Num);
        };
        ur[pr] = true;
        uc[pc] = true;
        piv.push((pr, pc));
        let p = m[pr * n + pc];
        for l in 0..n {
            m[pr * n + l] = if l == pc { 1.0 / p } else { m[pr * n + l] / p };
        }
        for ll in (0..n).filter(|&ll| ll != pr) {
            let d = m[ll * n + pc];
            for l in 0..n {
                if l == pc {
                    m[ll * n + l] = -d / p;
                } else {
                    m[ll * n + l] -= m[pr * n + l] * d;
                }
            }
        }
    }
    let mut inv = vec![0.0; n * n];
    for &(ri, ci) in &piv {
        for &(rj, cj) in &piv {
            inv[ci * n + rj] = m[ri * n + cj];
        }
    }
    Value::from(Array::new(n, n, inv.into_iter().map(num).collect()))
}

// ---------------------------------------------------------------------------
// 로마 숫자·진법
// ---------------------------------------------------------------------------

fn roman(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0)).trunc();
    let mode = match opt(a, 1) {
        None => 0.0,
        Some(Value::Bool(true)) => 0.0,
        Some(Value::Bool(false)) => 4.0,
        Some(v) => try_v!(ctx.num(v)).trunc(),
    };
    if !(0.0..=3999.0).contains(&n) || !(0.0..=4.0).contains(&mode) {
        return Value::Error(ErrorKind::Value);
    }
    Value::from(to_roman(n as u32, mode as u32))
}

/// 로마 숫자. 형식 0 은 고전 표기, 1–4 는 더 큰 빼기 쌍을 허용해 점점 짧게 쓴다.
pub fn to_roman(n: u32, mode: u32) -> String {
    const VALS: [u32; 7] = [1000, 500, 100, 50, 10, 5, 1];
    const CH: [char; 7] = ['M', 'D', 'C', 'L', 'X', 'V', 'I'];
    let mut rest = n;
    let mut out = String::new();
    for pos in 0..4 {
        let mut idx = pos * 2;
        let digit = rest / VALS[idx];
        if digit % 5 == 4 {
            // 4 → 다섯 기호, 9 → 열 기호를 앞의 작은 기호로 뺀다
            let big = if digit == 4 { idx - 1 } else { idx - 2 };
            let mut steps = 0;
            while steps < mode && idx < 6 {
                steps += 1;
                if VALS[big] - VALS[idx + 1] <= rest {
                    idx += 1;
                } else {
                    steps = mode;
                }
            }
            out.push(CH[idx]);
            out.push(CH[big]);
            rest = rest + VALS[idx] - VALS[big];
        } else {
            if digit > 4 {
                out.push(CH[idx - 1]);
            }
            for _ in 0..digit % 5 {
                out.push(CH[idx]);
            }
            rest %= VALS[idx];
        }
    }
    out
}

fn arabic(ctx: &mut Ctx, a: &[Value]) -> Value {
    // 앞뒤 공백도 #VALUE! (ARABIC(" XIV "), 대조 실험 math)
    let t = try_v!(ctx.text(&a[0]));
    let s: &str = &t;
    if s.chars().count() > 255 {
        return Value::Error(ErrorKind::Value);
    }
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let val = |c: char| match c.to_ascii_uppercase() {
        'I' => Some(1i64),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let mut digits = Vec::new();
    for ch in s.chars() {
        match val(ch) {
            Some(v) => digits.push(v),
            None => return Value::Error(ErrorKind::Value),
        }
    }
    let mut total = 0i64;
    for i in 0..digits.len() {
        if i + 1 < digits.len() && digits[i] < digits[i + 1] {
            total -= digits[i];
        } else {
            total += digits[i];
        }
    }
    Value::Number(if neg { -total } else { total } as f64)
}

fn base(ctx: &mut Ctx, a: &[Value]) -> Value {
    let n = try_v!(num_arg(ctx, a, 0)).trunc();
    let radix = try_v!(num_arg(ctx, a, 1)).trunc();
    // 앞 인수가 틀리면 자릿수 인수의 오류는 보지 않는다 (BASE(1,1,NA()) = #NUM!, 대조 실험 prec)
    if !(0.0..9.007_199_254_740_992e15).contains(&n) || !(2.0..=36.0).contains(&radix) {
        return Value::Error(ErrorKind::Num);
    }
    let min_len = try_v!(num_or(ctx, a, 2, 0.0)).trunc();
    if !(0.0..=255.0).contains(&min_len) {
        return Value::Error(ErrorKind::Num);
    }
    let mut s = to_radix(n as u64, radix as u32);
    // 0 은 자릿수를 채우지 않는다 (BASE(0,2,5) = "0", 대조 실험 rand)
    while n != 0.0 && s.len() < min_len as usize {
        s.insert(0, '0');
    }
    Value::from(s)
}

pub fn to_radix(mut n: u64, radix: u32) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut d = Vec::new();
    while n > 0 {
        d.push(
            std::char::from_digit((n % radix as u64) as u32, radix)
                .unwrap()
                .to_ascii_uppercase(),
        );
        n /= radix as u64;
    }
    d.iter().rev().collect()
}

fn decimal(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(ctx.text(&a[0]));
    let radix = try_v!(num_arg(ctx, a, 1)).trunc();
    if !(2.0..=36.0).contains(&radix) || t.chars().count() > 255 {
        return Value::Error(ErrorKind::Num);
    }
    // 10진은 앞 공백을 건너뛴다 (DECIMAL(" 12",10) = 12; DECIMAL(" FF ",16) = #NUM!, 대조 실험 textjoin)
    let t = if radix == 10.0 {
        t.trim_start().into()
    } else {
        t
    };
    let mut v = 0.0f64;
    // 공백은 숫자가 아니다 (DECIMAL(" FF ",16) = #NUM!, 대조 실험 math)
    for ch in t.chars() {
        match ch.to_digit(radix as u32) {
            Some(d) => v = v * radix + d as f64,
            None => return Value::Error(ErrorKind::Num),
        }
    }
    num(v)
}

/// RANDARRAY([행], [열], [최소], [최대], [정수]): 행·열 1 미만이나 최소 > 최대는 #VALUE!
fn randarray(ctx: &mut Ctx, a: &[Value]) -> Value {
    let rows = try_v!(num_or(ctx, a, 0, 1.0)).trunc();
    let cols = try_v!(num_or(ctx, a, 1, 1.0)).trunc();
    let lo = try_v!(num_or(ctx, a, 2, 0.0));
    let hi = try_v!(num_or(ctx, a, 3, 1.0));
    let whole = match opt(a, 4) {
        Some(v) if !v.is_empty() => try_v!(ctx.boolean(v)),
        _ => false,
    };
    if rows < 1.0 || cols < 1.0 || lo > hi || (whole && (lo.fract() != 0.0 || hi.fract() != 0.0)) {
        return Value::Error(ErrorKind::Value);
    }
    if rows * cols > 4_000_000.0 {
        return Value::Error(ErrorKind::Num);
    }
    let (r, c) = (rows as usize, cols as usize);
    let data = (0..r * c)
        .map(|_| {
            let u = ctx.engine().random();
            num(if whole {
                lo + (u * (hi - lo + 1.0)).floor()
            } else {
                lo + u * (hi - lo)
            })
        })
        .collect();
    Value::from(Array::new(r, c, data))
}

fn sequence(ctx: &mut Ctx, a: &[Value]) -> Value {
    let rows = try_v!(num_arg(ctx, a, 0)).trunc();
    let cols = try_v!(num_or(ctx, a, 1, 1.0)).trunc();
    let start = try_v!(num_or(ctx, a, 2, 1.0));
    let step = try_v!(num_or(ctx, a, 3, 1.0));
    if rows < 1.0 || cols < 1.0 {
        return Value::Error(ErrorKind::Value);
    }
    if rows * cols > 4_000_000.0 {
        return Value::Error(ErrorKind::Num);
    }
    let (r, c) = (rows as usize, cols as usize);
    let data = (0..r * c).map(|i| num(start + step * i as f64)).collect();
    Value::from(Array::new(r, c, data))
}
