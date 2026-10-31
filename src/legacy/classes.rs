// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 토큰의 클래스(참조 0x20·값 0x40·배열 0x60) 정하기 — 바이너리·레거시 부호화가 함께 쓴다.
//!
//! 한셀의 규칙 (함수·칸·연산자 조합 수천 가지를 저장해 읽은 대조 실험, `docs/format/cell-binary.md` "토큰 클래스"):
//!
//! 식의 맥락은 두 클래스로 정한다 — `bare` 는 맨 참조·이름 토큰이 쓰는 클래스, `inner` 는 연산자의 피연산자(와
//! 그 안의 함수 인자)가 쓰는 클래스. 최상위 셀 수식은 (값, 값), 배열 수식은 (배열, 배열), 이름 정의는 (참조, 값).
//!
//! - 연산자의 피연산자: (`inner`, `inner`). 괄호는 맥락을 그대로 둔다
//! - 함수 인자 칸은 [`Kind`] 로 정해진다 (V 값·값, R 참조·값, B 참조·배열, A 배열·배열 …). 맥락의 `inner` 가
//!   배열이면 (배열 연산 중) 값 칸의 맨 참조도 배열이 되고 연산자 안은 배열이다. 사용자 함수의 인자는 (참조,
//!   값) 이고, 배열 연산이 A 칸에서 시작한 것(`strong`)일 때만 안쪽 연산자가 배열이다
//! - 함수 토큰의 클래스는 반환 종류와 맥락으로: 값 맥락이면 값, 참조 맥락 (참조, 값) 이면 반환 종류 그대로, 배열
//!   연산 중이면 참조 반환은 맨 클래스, 나머지는 배열
//! - 배열 상수는 (값, 값) 맥락에서만 값, 그 밖에는 배열

use super::ftab::ArgClass;

/// 함수 칸 종류 (맨 참조, 연산자 안의 참조)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// (값, 값)
    V,
    /// (참조, 값)
    R,
    /// (참조, 배열)
    B,
    /// (배열, 배열)
    A,
    /// (참조, 참조)
    D,
    /// TRANSPOSE: (값, 값), 다만 안의 배열 반환 함수·배열 상수는 배열
    T,
    /// (값, 값), 다만 배열 상수는 배열 (IF 의 조건 …)
    W,
    /// (참조, 값), 다만 배열 상수는 값 (QUOTIENT)
    X,
}

/// 함수 반환 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ret {
    V,
    R,
    A,
}

/// 식 맥락
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    pub bare: ArgClass,
    pub inner: ArgClass,
    /// 배열 연산이 A 칸·배열 수식에서 시작했다 (사용자 함수 안까지 이어진다)
    pub strong: bool,
    /// 이 칸의 안쪽 배열 반환 함수는 배열 클래스 (TRANSPOSE 의 칸)
    pub arr_call: bool,
    /// 배열 상수의 클래스 — 칸이 정한다 (연산자 안에서도 그대로)
    pub arr: ArgClass,
    /// 배열 수식의 맨 위: 맨 위 식은 셀 수식과 같고 함수 인자 안은 배열 연산이다
    pub arrf: bool,
    /// 조건부 서식·유효성 수식: 참조 칸(R)의 연산자 안은 배열, IF·CHOOSE 의 첫 인자는 배열
    pub cf: bool,
}

impl Ctx {
    /// 최상위 셀 수식·조건부 서식·유효성
    pub const fn value() -> Ctx {
        Ctx {
            bare: ArgClass::Value,
            inner: ArgClass::Value,
            strong: false,
            arr_call: false,
            arr: ArgClass::Value,
            arrf: false,
            cf: false,
        }
    }

    /// 이름 정의, 참조 연산자(`:`·공백·`,`)의 피연산자
    pub const fn reference() -> Ctx {
        Ctx {
            bare: ArgClass::Ref,
            inner: ArgClass::Value,
            strong: false,
            arr_call: false,
            arr: ArgClass::Array,
            arrf: false,
            cf: false,
        }
    }

    /// 배열 수식 (맨 위는 셀 수식과 같은 값, 함수 인자 안은 배열 연산 — 한셀 수식 대조 실험: `{B1:B3*2}` 의
    /// 참조는 값, `{SUM(B1:B3*2)}` 의 참조는 배열)
    pub const fn array_formula() -> Ctx {
        Ctx {
            bare: ArgClass::Value,
            inner: ArgClass::Value,
            strong: false,
            arr_call: false,
            arr: ArgClass::Array,
            arrf: true,
            cf: false,
        }
    }

    /// 조건부 서식·데이터 유효성 수식
    pub const fn cf() -> Ctx {
        Ctx {
            cf: true,
            ..Ctx::value()
        }
    }

    /// 연산자(`+`·`-`·`%`·`&`·비교 …)의 피연산자
    pub fn operand(self) -> Ctx {
        Ctx {
            bare: self.inner,
            inner: self.inner,
            strong: self.strong,
            arr_call: false,
            arr: self.arr,
            arrf: self.arrf,
            cf: self.cf,
        }
    }

    /// 함수 인자 칸의 맥락. `udf` 는 사용자 함수(이름으로 부르는 함수)
    pub fn arg(self, kind: Kind, udf: bool) -> Ctx {
        // 조건부 서식·유효성: 참조 칸의 연산자 안은 배열 (맨 참조는 참조 그대로)
        let kind = if self.cf && !udf && matches!(kind, Kind::R | Kind::X) {
            Kind::B
        } else {
            kind
        };
        let (bare, inner, strong, arr_call) = match kind {
            Kind::V | Kind::W => (ArgClass::Value, ArgClass::Value, false, false),
            Kind::R | Kind::X => (ArgClass::Ref, ArgClass::Value, false, false),
            Kind::B => (ArgClass::Ref, ArgClass::Array, false, false),
            Kind::A => (ArgClass::Array, ArgClass::Array, true, false),
            Kind::D => (ArgClass::Ref, ArgClass::Ref, false, false),
            Kind::T => (ArgClass::Value, ArgClass::Value, false, true),
        };
        // 배열 상수: 값 칸(과 TRANSPOSE 칸)에서만 값
        let arr = if matches!(kind, Kind::V | Kind::X) {
            ArgClass::Value
        } else {
            ArgClass::Array
        };
        // 배열 수식의 맨 위 식을 지나면 함수 인자 안은 배열 연산이다
        let array_mode = self.inner == ArgClass::Array || self.arrf;
        let outer_strong = self.strong || self.arrf;
        if !array_mode {
            return Ctx {
                bare,
                inner,
                strong,
                arr_call,
                arr: if udf { ArgClass::Array } else { arr },
                arrf: false,
                cf: self.cf,
            };
        }
        if udf {
            return Ctx {
                bare: ArgClass::Ref,
                inner: if outer_strong {
                    ArgClass::Array
                } else {
                    ArgClass::Value
                },
                strong: outer_strong,
                arr_call: false,
                arr: ArgClass::Array,
                arrf: false,
                cf: self.cf,
            };
        }
        Ctx {
            bare: if bare == ArgClass::Value {
                ArgClass::Array
            } else {
                bare
            },
            inner: ArgClass::Array,
            strong: outer_strong || strong,
            arr_call,
            arr: ArgClass::Array,
            arrf: false,
            cf: self.cf,
        }
    }

    /// 함수 토큰의 클래스
    pub fn func_class(self, ret: Ret) -> ArgClass {
        if self.bare == ArgClass::Value && self.inner == ArgClass::Value {
            return if self.arr_call && ret == Ret::A {
                ArgClass::Array
            } else {
                ArgClass::Value
            };
        }
        if self.inner == ArgClass::Array {
            return if ret == Ret::R {
                self.bare
            } else {
                ArgClass::Array
            };
        }
        match ret {
            Ret::V => ArgClass::Value,
            Ret::R => ArgClass::Ref,
            Ret::A => ArgClass::Array,
        }
    }

    /// 배열 상수의 클래스
    pub fn array_const(self) -> ArgClass {
        self.arr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ArgClass::{Array as A, Ref as R, Value as V};

    #[test]
    fn contexts_follow_hancell() {
        let top = Ctx::value();
        // ABS(A1:B2) 칸은 (값, 값), SUM 칸은 (참조, 값), ROWS 칸은 (참조, 배열), SUMPRODUCT 칸은 (배열, 배열)
        assert_eq!(top.arg(Kind::V, false).bare, V);
        let sum = top.arg(Kind::R, false);
        assert_eq!((sum.bare, sum.operand().bare), (R, V));
        let rows = top.arg(Kind::B, false);
        assert_eq!((rows.bare, rows.operand().bare), (R, A));
        // INDEX(ABS(A1:B2),1,1): 안쪽 ABS 의 칸은 배열, ABS 토큰도 배열
        assert_eq!(rows.func_class(Ret::V), A);
        assert_eq!(rows.arg(Kind::V, false).bare, A);
        // INDEX(IF(…)): IF 토큰은 참조
        assert_eq!(rows.func_class(Ret::R), R);
        // SUM(INDEX(…)): INDEX 토큰은 참조, ABS(INDEX(…)): 값
        assert_eq!(sum.func_class(Ret::R), R);
        assert_eq!(top.arg(Kind::V, false).func_class(Ret::R), V);
        // 사용자 함수: ROWS 칸 안에서는 연산자가 값, SUMPRODUCT 칸 안에서는 배열
        assert_eq!(rows.arg(Kind::R, true).inner, V);
        assert_eq!(top.arg(Kind::A, false).arg(Kind::R, true).inner, A);
        // 배열 수식: 맨 위는 값, 함수 인자 안은 배열 연산 ({B1:B3*2} 의 참조는 값, {SUM(B1:B3*2)} 의 참조는 배열)
        let af = Ctx::array_formula();
        assert_eq!(af.operand().bare, V);
        assert_eq!(af.arg(Kind::R, false).operand().bare, A);
        // 조건부 서식: AND(A1>1) 의 참조는 배열, MAX(A1) 의 맨 참조는 참조
        let cf = Ctx::cf().arg(Kind::R, false);
        assert_eq!((cf.bare, cf.operand().bare), (R, A));
        // 배열 상수
        assert_eq!(top.array_const(), V);
        assert_eq!(sum.array_const(), A);
    }
}
