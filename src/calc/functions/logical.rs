// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 논리 함수.

use std::rc::Rc;

use super::{Arg, FnSpec, MANY, R, V, f, lazy};
use crate::calc::ast::Expr;
use crate::calc::eval::{Ctx, broadcast_dim};
use crate::calc::value::{self, Array, ErrorKind, Value};

pub const FNS: &[FnSpec] = &[
    f("TRUE", 0, 0, V, |_, _| Value::Bool(true)),
    f("FALSE", 0, 0, V, |_, _| Value::Bool(false)),
    f("NOT", 1, 1, V, not),
    f("AND", 1, MANY, R, and),
    f("OR", 1, MANY, R, or),
    f("XOR", 1, MANY, R, xor),
    lazy("IF", 2, 3, if_),
    lazy("IFERROR", 2, 2, iferror),
    lazy("IFNA", 2, 2, ifna),
    lazy("IFS", 2, MANY, ifs),
    lazy("SWITCH", 3, MANY, switch),
];

const _: &[Arg] = &[];

fn not(ctx: &mut Ctx, a: &[Value]) -> Value {
    Value::Bool(!try_v!(ctx.boolean(&a[0])))
}

/// AND·OR·XOR 의 인수에서 논리 값을 모은다. 빈 칸과 배열의 글은 건너뛰고, 참조한 칸의 글은 숫자로 읽히면
/// 그 숫자로 (Data!C5 의 "5" 는 TRUE), 아니면 건너뛴다. 직접 준 글은 AND·OR 에서 "TRUE"·"FALSE" 만 되고,
/// XOR 은 어떤 글이든 #VALUE! (대조 실험 logical·misc3).
fn logicals(ctx: &mut Ctx, args: &[Value], xor: bool) -> Result<Vec<bool>, ErrorKind> {
    let mut out = Vec::new();
    let date1904 = ctx.date1904();
    for a in args {
        match a {
            Value::Ref(r) => {
                for area in r.iter() {
                    let mut err = None;
                    ctx.for_each_in_area(area, |_, _, v| {
                        if err.is_some() {
                            return;
                        }
                        match v {
                            Value::Bool(b) => out.push(*b),
                            Value::Number(n) => out.push(*n != 0.0),
                            Value::Error(e) => err = Some(*e),
                            Value::Text(t) => {
                                if let Some(b) = text_logical(t, date1904) {
                                    out.push(b);
                                }
                            }
                            _ => {}
                        }
                    });
                    if let Some(e) = err {
                        return Err(e);
                    }
                }
            }
            Value::Array(arr) => {
                for v in &arr.data {
                    match v {
                        Value::Bool(b) => out.push(*b),
                        Value::Number(n) => out.push(*n != 0.0),
                        Value::Error(e) => return Err(*e),
                        Value::Text(t) => {
                            if let Some(b) = text_logical(t, date1904) {
                                out.push(b);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Value::Empty => out.push(false),
            Value::Text(_) if xor => return Err(ErrorKind::Value),
            Value::Text(t) => match text_logical(t, date1904) {
                Some(b) => out.push(b),
                None => return Err(ErrorKind::Value),
            },
            other => out.push(ctx.boolean(other)?),
        }
    }
    if out.is_empty() {
        return Err(ErrorKind::Value);
    }
    Ok(out)
}

/// 글의 논리 값: "TRUE"·"FALSE", 숫자로 읽히면 0 이 아닌가 (AND("5") = TRUE, 대조 실험 misc4)
fn text_logical(t: &str, date1904: bool) -> Option<bool> {
    value::parse_bool_text(t).or_else(|| value::parse_number(t, date1904).map(|n| n != 0.0))
}

fn and(ctx: &mut Ctx, a: &[Value]) -> Value {
    Value::Bool(try_v!(logicals(ctx, a, false)).iter().all(|&b| b))
}

fn or(ctx: &mut Ctx, a: &[Value]) -> Value {
    Value::Bool(try_v!(logicals(ctx, a, false)).iter().any(|&b| b))
}

fn xor(ctx: &mut Ctx, a: &[Value]) -> Value {
    Value::Bool(
        try_v!(logicals(ctx, a, true))
            .iter()
            .filter(|&&b| b)
            .count()
            % 2
            == 1,
    )
}

/// 생략한 인수는 0
fn branch(ctx: &mut Ctx, e: &Expr) -> Value {
    match e {
        Expr::Missing => Value::Number(0.0),
        e => ctx.eval(e),
    }
}

fn branch_value(ctx: &mut Ctx, e: &Expr) -> Value {
    match e {
        Expr::Missing => Value::Number(0.0),
        e => ctx.eval_value(e),
    }
}

fn if_(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let cond = ctx.eval_value(&args[0]);
    if let Value::Array(c) = cond {
        let t = match args.get(1) {
            Some(e) => branch_value(ctx, e),
            None => Value::Bool(true),
        };
        let fv = match args.get(2) {
            Some(e) => branch_value(ctx, e),
            None => Value::Bool(false),
        };
        return elementwise3(&c, &t, &fv, |c, t, f| match value::to_bool(c) {
            Ok(true) => t.clone(),
            Ok(false) => f.clone(),
            Err(e) => Value::Error(e),
        });
    }
    match ctx.boolean(&cond) {
        Ok(true) => branch(ctx, &args[1]),
        Ok(false) => match args.get(2) {
            Some(e) => branch(ctx, e),
            None => Value::Bool(false),
        },
        Err(e) => Value::Error(e),
    }
}

/// 배열 조건과 두 값(배열이거나 스칼라)을 원소마다
fn elementwise3(
    c: &Array,
    t: &Value,
    f: &Value,
    g: impl Fn(&Value, &Value, &Value) -> Value,
) -> Value {
    let dims = |v: &Value| match v {
        Value::Array(a) => (a.rows, a.cols),
        _ => (1, 1),
    };
    let (tr, tc) = dims(t);
    let (fr, fc) = dims(f);
    let rows = broadcast_dim(broadcast_dim(c.rows, tr), fr);
    let cols = broadcast_dim(broadcast_dim(c.cols, tc), fc);
    let pick = |v: &Value, r: usize, k: usize| match v {
        Value::Array(a) => a.get_broadcast(r, k),
        o => o.clone(),
    };
    let mut data = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for k in 0..cols {
            data.push(g(&c.get_broadcast(r, k), &pick(t, r, k), &pick(f, r, k)));
        }
    }
    Value::Array(Rc::new(Array::new(rows, cols, data)))
}

fn iferror(ctx: &mut Ctx, args: &[Expr]) -> Value {
    catch(ctx, args, |_| true)
}

fn ifna(ctx: &mut Ctx, args: &[Expr]) -> Value {
    catch(ctx, args, |e| e == ErrorKind::NA)
}

fn catch(ctx: &mut Ctx, args: &[Expr], hit: fn(ErrorKind) -> bool) -> Value {
    let v = ctx.eval_value(&args[0]);
    match v {
        Value::Array(a) => {
            if !a
                .data
                .iter()
                .any(|x| matches!(x, Value::Error(e) if hit(*e)))
            {
                return Value::Array(a);
            }
            let alt = branch_value(ctx, &args[1]);
            let cond = Array::new(a.rows, a.cols, a.data.clone());
            elementwise3(&cond, &alt, &Value::Empty, |x, alt, _| match x {
                Value::Error(e) if hit(*e) => alt.clone(),
                o => o.clone(),
            })
        }
        Value::Error(e) if hit(e) => branch(ctx, &args[1]),
        Value::Empty => Value::Number(0.0),
        o => o,
    }
}

/// IFS: 인수 개수가 홀수면 #VALUE!, 조건이 오류이거나 논리로 바꿀 수 없으면 #N/A (IFS("x",1),
/// IFS(FALSE,1,1/0,2), 대조 실험 logical)
fn ifs(ctx: &mut Ctx, args: &[Expr]) -> Value {
    if !args.len().is_multiple_of(2) {
        return Value::Error(ErrorKind::Value);
    }
    for pair in args.chunks(2) {
        let c = ctx.eval_value(&pair[0]);
        let c = ctx.scalar(c);
        match ctx.boolean(&c) {
            Ok(true) => return branch(ctx, &pair[1]),
            Ok(false) => {}
            Err(_) => return Value::Error(ErrorKind::NA),
        }
    }
    Value::Error(ErrorKind::NA)
}

fn switch(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let x = ctx.eval_value(&args[0]);
    let x = ctx.scalar(x);
    // 식이 오류여도 멈추지 않는다: 같은 오류인 값과만 같고 (SWITCH(#N/A,#N/A,1,2) = 1), 맞는 값이 없으면 기본값
    // (SWITCH(범위,10,"x","d") 의 암묵 교차 #VALUE! → "d", 대조 실험 misc5)
    let x_err = if let Value::Error(e) = x {
        Some(e)
    } else {
        None
    };
    let rest = &args[1..];
    let mut i = 0;
    while i + 1 < rest.len() {
        let v = ctx.eval_value(&rest[i]);
        let v = ctx.scalar(v);
        // 값이 오류면 식이 같은 오류일 때만 맞는다 (SWITCH(1,#N/A,2,1,3) = 3, 대조 실험 logical)
        if let Value::Error(ve) = v {
            if x_err == Some(ve) {
                return branch(ctx, &rest[i + 1]);
            }
            i += 2;
            continue;
        }
        if x_err.is_some() {
            i += 2;
            continue;
        }
        if value::compare(&x, &v) == std::cmp::Ordering::Equal && same_kind(&x, &v) {
            return branch(ctx, &rest[i + 1]);
        }
        i += 2;
    }
    if i < rest.len() {
        return branch(ctx, &rest[i]);
    }
    Value::Error(ErrorKind::NA)
}

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!(
        (a, b),
        (
            Value::Number(_) | Value::Empty,
            Value::Number(_) | Value::Empty
        ) | (Value::Text(_), Value::Text(_))
            | (Value::Bool(_), Value::Bool(_))
    )
}
