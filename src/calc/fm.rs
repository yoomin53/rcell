// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 부동소수 초월 함수 (순수 Rust: [`hcmath`] — 한셀이 부르는 32비트 윈도 C 런타임과 같은 값, 나머지는 `libm`).
//!
//! 표준 라이브러리의 `f64::sin` 같은 메서드는 플랫폼 C 라이브러리를 부른다. windows-gnu 의 것은 x87
//! 명령(fsin …)이라 π 근처에서 틀린다: SIN(PI()) = 1.2246063538e-16, 한셀(MSVC 런타임)은
//! 1.2246467991e-16 (함수 대조 실험). 계산 엔진은 이 트레이트의 메서드만 쓴다.

use super::hcmath;

pub trait Fm: Copy {
    fn msin(self) -> f64;
    fn mcos(self) -> f64;
    fn mtan(self) -> f64;
    fn masin(self) -> f64;
    fn macos(self) -> f64;
    fn matan(self) -> f64;
    fn matan2(self, x: f64) -> f64;
    fn mexp(self) -> f64;
    fn mln(self) -> f64;
    fn mlog10(self) -> f64;
    fn mpow(self, y: f64) -> f64;
    fn mpowi(self, n: i32) -> f64;
    fn msinh(self) -> f64;
    fn mcosh(self) -> f64;
    fn mtanh(self) -> f64;
    fn masinh(self) -> f64;
    fn macosh(self) -> f64;
    fn matanh(self) -> f64;
    fn mhypot(self, y: f64) -> f64;
}

impl Fm for f64 {
    fn msin(self) -> f64 {
        hcmath::sin(self)
    }
    fn mcos(self) -> f64 {
        hcmath::cos(self)
    }
    fn mtan(self) -> f64 {
        hcmath::tan(self)
    }
    fn masin(self) -> f64 {
        hcmath::asin(self)
    }
    fn macos(self) -> f64 {
        hcmath::acos(self)
    }
    fn matan(self) -> f64 {
        hcmath::atan(self)
    }
    fn matan2(self, x: f64) -> f64 {
        hcmath::atan2(self, x)
    }
    fn mexp(self) -> f64 {
        hcmath::exp(self)
    }
    fn mln(self) -> f64 {
        hcmath::ln(self)
    }
    fn mlog10(self) -> f64 {
        hcmath::log10(self)
    }
    fn mpow(self, y: f64) -> f64 {
        hcmath::pow(self, y)
    }
    fn mpowi(self, n: i32) -> f64 {
        hcmath::pow(self, n as f64)
    }
    fn msinh(self) -> f64 {
        hcmath::sinh(self)
    }
    fn mcosh(self) -> f64 {
        hcmath::cosh(self)
    }
    fn mtanh(self) -> f64 {
        hcmath::tanh(self)
    }
    fn masinh(self) -> f64 {
        libm::asinh(self)
    }
    fn macosh(self) -> f64 {
        libm::acosh(self)
    }
    fn matanh(self) -> f64 {
        libm::atanh(self)
    }
    fn mhypot(self, y: f64) -> f64 {
        libm::hypot(self, y)
    }
}
