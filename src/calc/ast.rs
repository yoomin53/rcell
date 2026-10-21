// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 구문 트리.

use std::rc::Rc;

use super::functions::FnSpec;
use super::value::{Array, ErrorKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(Rc<str>),
    Bool(bool),
    Error(ErrorKind),
    /// 생략한 인수 (`IF(A1,,1)`)
    Missing,
    /// 배열 상수 `{1,2;3,4}`
    Array(Rc<Array>),
    Ref(RefExpr),
    Name(NameExpr),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Call(Box<Call>),
    /// 괄호 — 함수 인수 안의 합집합 `SUM((A1,B1))` 을 구별하려고 남긴다
    Paren(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Neg,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// `:`
    Range,
    /// `,` (괄호 안·최상위)
    Union,
    /// 공백
    Intersect,
}

/// 시트 지정
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetSpec {
    /// 수식이 있는 시트
    Current,
    One(String),
    /// 3차원 `Sheet1:Sheet3`
    Span(String, String),
}

/// 셀·영역 참조. 좌표는 0부터, 양 끝 포함. 열 전체(`A:A`)·행 전체(`1:1`)는 시트 끝까지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefExpr {
    /// 외부 통합 문서 번호 (`[n]`, 1부터)
    pub book: Option<u32>,
    pub sheet: SheetSpec,
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

/// 정의된 이름
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameExpr {
    pub book: Option<u32>,
    /// 시트 이름으로 한정한 이름 (`Sheet1!이름`)
    pub sheet: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Call {
    /// 대문자, `_xlfn.` 등 접두사를 뗀 이름
    pub name: String,
    pub func: Option<&'static FnSpec>,
    pub args: Vec<Expr>,
}

impl PartialEq for Call {
    fn eq(&self, o: &Call) -> bool {
        self.name == o.name && self.args == o.args
    }
}

impl Expr {
    /// 식 안의 모든 함수 이름
    pub fn visit_calls(&self, f: &mut impl FnMut(&Call)) {
        match self {
            Expr::Unary(_, a) | Expr::Paren(a) => a.visit_calls(f),
            Expr::Binary(_, a, b) => {
                a.visit_calls(f);
                b.visit_calls(f);
            }
            Expr::Call(c) => {
                f(c);
                for a in &c.args {
                    a.visit_calls(f);
                }
            }
            _ => {}
        }
    }

    /// 식 안의 참조와 이름
    pub fn visit_refs(&self, f: &mut impl FnMut(RefOrName<'_>)) {
        match self {
            Expr::Ref(r) => f(RefOrName::Ref(r)),
            Expr::Name(n) => f(RefOrName::Name(n)),
            Expr::Unary(_, a) | Expr::Paren(a) => a.visit_refs(f),
            Expr::Binary(_, a, b) => {
                a.visit_refs(f);
                b.visit_refs(f);
            }
            Expr::Call(c) => {
                for a in &c.args {
                    a.visit_refs(f);
                }
            }
            _ => {}
        }
    }
}

pub enum RefOrName<'a> {
    Ref(&'a RefExpr),
    Name(&'a NameExpr),
}
