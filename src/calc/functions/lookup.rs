// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 찾기·참조 함수.

use std::cmp::Ordering;
use std::rc::Rc;

use super::{Arg, FnSpec, MANY, R, V, bool_or, f, lazy, num_arg, num_or, opt, to_array};
use crate::calc::ast::{Expr, SheetSpec};
use crate::calc::criteria::{has_wildcards, wildcard_match};
use crate::calc::eval::Ctx;
use crate::calc::value::{self, Area, Array, ErrorKind, MAX_COLS, MAX_ROWS, Value};

const RAV: &[Arg] = &[Arg::RA, Arg::V, Arg::V, Arg::V];
const VRAV: &[Arg] = &[Arg::V, Arg::RA, Arg::V, Arg::V];
const VRARA: &[Arg] = &[Arg::V, Arg::RA, Arg::RA];
const VRARAV: &[Arg] = &[Arg::V, Arg::RA, Arg::RA, Arg::V, Arg::V, Arg::V];
const RVVVV: &[Arg] = &[Arg::R, Arg::V, Arg::V, Arg::V, Arg::V];

pub const FNS: &[FnSpec] = &[
    f("INDEX", 2, 4, RAV, index),
    f("MATCH", 2, 3, VRAV, match_fn),
    f("XMATCH", 2, 4, VRAV, xmatch),
    f("VLOOKUP", 3, 4, VRAV, |c, a| vh_lookup(c, a, true)),
    f("HLOOKUP", 3, 4, VRAV, |c, a| vh_lookup(c, a, false)),
    f("LOOKUP", 2, 3, VRARA, lookup),
    f("XLOOKUP", 3, 6, VRARAV, xlookup),
    lazy("CHOOSE", 2, MANY, choose),
    super::volatile(f("OFFSET", 3, 5, RVVVV, offset)),
    super::volatile(f("INDIRECT", 1, 2, V, indirect)),
    f("ROW", 0, 1, R, |c, a| row_col(c, a, true)),
    f("COLUMN", 0, 1, R, |c, a| row_col(c, a, false)),
    f("ROWS", 1, 1, R, |c, a| dims(c, a, true)),
    f("COLUMNS", 1, 1, R, |c, a| dims(c, a, false)),
    f("AREAS", 1, 1, R, |_, a| match &a[0] {
        Value::Ref(r) => Value::Number(r.len() as f64),
        Value::Error(e) => Value::Error(*e),
        _ => Value::Error(ErrorKind::Value),
    }),
    f("ADDRESS", 2, 5, V, address),
    // 배열 수식이 아니면 인수를 암묵 교차한 값 하나 (INDEX(TRANSPOSE(A1:A3),1,2) = #VALUE!, 대조 실험 lookup)
    f("TRANSPOSE", 1, 1, R, |c, a| {
        let v = match &a[0] {
            Value::Ref(r) if !c.array_formula => c.intersect_ref(r),
            v => v.clone(),
        };
        match to_array(c, &v) {
            Ok(arr) => Value::from(arr.transpose()),
            Err(e) => Value::Error(e),
        }
    }),
    f("HYPERLINK", 1, 2, V, |c, a| match opt(a, 1) {
        Some(v) => v.clone(),
        None => match c.text(&a[0]) {
            Ok(t) => Value::Text(t),
            Err(e) => Value::Error(e),
        },
    }),
    f("FORMULATEXT", 1, 1, R, formulatext),
    f("GETPIVOTDATA", 2, MANY, R, |_, _| {
        Value::Error(ErrorKind::Value)
    }),
    f("RTD", 3, MANY, V, |_, _| Value::Error(ErrorKind::NA)),
    f("UNIQUE", 1, 3, &[Arg::A, Arg::V, Arg::V], unique),
    f("SORT", 1, 4, &[Arg::A, Arg::V, Arg::V, Arg::V], sort),
    f("FILTER", 2, 3, &[Arg::A, Arg::A, Arg::V], filter),
];

/// 찾기 대상(참조·배열)을 배열로
fn lookup_array(ctx: &mut Ctx, v: &Value) -> Result<Rc<Array>, ErrorKind> {
    match v {
        Value::Ref(r) => {
            if r.len() != 1 {
                return Err(ErrorKind::NA);
            }
            ctx.area_to_array(&r[0]).map(Rc::new)
        }
        Value::Array(a) => Ok(a.clone()),
        Value::Error(e) => Err(*e),
        other => Ok(Rc::new(Array::new(1, 1, vec![other.clone()]))),
    }
}

fn same_type(a: &Value, b: &Value) -> bool {
    matches!(
        (a, b),
        (Value::Number(_), Value::Number(_))
            | (Value::Text(_), Value::Text(_))
            | (Value::Bool(_), Value::Bool(_))
    )
}

/// 정확히 일치하는 첫 칸 (글은 대소문자 무시, 와일드카드)
pub fn exact_index(vals: &[Value], x: &Value, wildcards: bool) -> Option<usize> {
    if let (Value::Text(t), true) = (x, wildcards)
        && has_wildcards(t)
    {
        let pat = value::fold(t);
        return vals.iter().position(|v| match v {
            Value::Text(s) => wildcard_match(&pat, &value::fold(s)),
            _ => false,
        });
    }
    vals.iter().position(|v| value::lookup_eq(v, x))
}

/// 한셀의 근사 찾기 (오름차순 가정: MATCH 1·VLOOKUP·HLOOKUP·LOOKUP·XLOOKUP 찾기 방향 ±2). 이분 찾기로,
/// 가운데 칸이 찾는 값과 같으면 바로 그 칸 (같은 값이 여럿이어도 처음 닿은 칸). 가운데 칸의 형이 다르면
/// (빈 칸 포함) 위쪽 끝까지 같은 형 칸을 찾아 비교하고, 없으면 그 아래 반을 버린다. 끝나면 마지막 hi 칸
/// (정렬되지 않은 자료에서도 한셀과 같은 칸, 대조 실험 search).
pub fn approx_index(vals: &[Value], x: &Value) -> Option<usize> {
    let (mut lo, mut hi) = (0isize, vals.len() as isize - 1);
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let Some(j) = (mid..=hi).find(|&j| same_type(&vals[j as usize], x)) else {
            hi = mid - 1;
            continue;
        };
        match value::compare(&vals[j as usize], x) {
            Ordering::Equal => return Some(j as usize),
            Ordering::Less => lo = j + 1,
            Ordering::Greater => hi = mid - 1,
        }
    }
    (hi >= 0).then_some(hi as usize)
}

/// MATCH -1 (내림차순 가정): 한셀은 처음부터 차례로 본다. 같은 값이면 그 칸, 찾는 값보다 작은 칸을
/// 만나면 바로 앞 칸 (첫 칸이면 #N/A), 끝까지 크면 마지막 칸. 형이 다른 칸은 건너뛴다 (대조 실험 search).
pub fn approx_desc_index(vals: &[Value], x: &Value) -> Option<usize> {
    let mut last = None;
    for (i, v) in vals.iter().enumerate() {
        if !same_type(v, x) {
            continue;
        }
        match value::compare(v, x) {
            Ordering::Equal => return Some(i),
            Ordering::Greater => last = Some(i),
            Ordering::Less => return last,
        }
    }
    last
}

fn scalar_key(ctx: &mut Ctx, v: &Value) -> Result<Value, ErrorKind> {
    let v = ctx.scalar(v.clone());
    match v {
        Value::Error(e) => Err(e),
        Value::Empty => Ok(Value::Number(0.0)),
        o => Ok(o),
    }
}

fn index(ctx: &mut Ctx, a: &[Value]) -> Value {
    let row = match opt(a, 1) {
        Some(v) => try_v!(ctx.num(v)).trunc(),
        None => 0.0,
    };
    let col = match opt(a, 2) {
        Some(v) => Some(try_v!(ctx.num(v)).trunc()),
        None => None,
    };
    if row < 0.0 || col.is_some_and(|c| c < 0.0) {
        return Value::Error(ErrorKind::Value);
    }
    match &a[0] {
        Value::Ref(r) => {
            let n = try_v!(num_or(ctx, a, 3, 1.0)).trunc();
            if n < 1.0 || n as usize > r.len() {
                return Value::Error(ErrorKind::Ref);
            }
            let area = r[n as usize - 1];
            let (h, w) = (area.rows() as f64, area.cols() as f64);
            // 한 행짜리에 인수 하나면 열 번호로 본다
            let (row, col) = match col {
                None if h == 1.0 && a.len() == 2 => (1.0, row),
                None => (row, if w == 1.0 { 1.0 } else { 0.0 }),
                Some(c) => (row, c),
            };
            if row > h || col > w {
                return Value::Error(ErrorKind::Ref);
            }
            let (r0, r1) = if row == 0.0 {
                (area.r0, area.r1)
            } else {
                (area.r0 + row as u32 - 1, area.r0 + row as u32 - 1)
            };
            let (c0, c1) = if col == 0.0 {
                (area.c0, area.c1)
            } else {
                (area.c0 + col as u32 - 1, area.c0 + col as u32 - 1)
            };
            Value::Ref(Rc::new(vec![Area {
                r0,
                r1,
                c0,
                c1,
                ..area
            }]))
        }
        Value::Error(e) => Value::Error(*e),
        other => {
            let arr = try_v!(to_array(ctx, other));
            let (h, w) = (arr.rows as f64, arr.cols as f64);
            let (row, col) = match col {
                None if h == 1.0 => (1.0, row),
                None if w == 1.0 => (row, 1.0),
                None => (row, 0.0),
                Some(c) => (row, c),
            };
            if row > h || col > w {
                return Value::Error(ErrorKind::Ref);
            }
            match (row == 0.0, col == 0.0) {
                (false, false) => arr.get(row as usize - 1, col as usize - 1).clone(),
                (true, false) => {
                    let c = col as usize - 1;
                    let d = (0..arr.rows).map(|r| arr.get(r, c).clone()).collect();
                    Value::from(Array::new(arr.rows, 1, d))
                }
                (false, true) => {
                    let r = row as usize - 1;
                    let d = (0..arr.cols).map(|c| arr.get(r, c).clone()).collect();
                    Value::from(Array::new(1, arr.cols, d))
                }
                (true, true) => Value::Array(arr),
            }
        }
    }
}

fn vector(arr: &Array) -> Result<Vec<Value>, ErrorKind> {
    if arr.rows == 1 || arr.cols == 1 {
        Ok(arr.data.clone())
    } else {
        Err(ErrorKind::NA)
    }
}

fn match_fn(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(scalar_key(ctx, &a[0]));
    let arr = try_v!(lookup_array(ctx, &a[1]));
    let vals = try_v!(vector(&arr));
    let mt = try_v!(num_or(ctx, a, 2, 1.0));
    let found = if mt == 0.0 {
        exact_index(&vals, &x, true)
    } else if mt < 0.0 {
        approx_desc_index(&vals, &x)
    } else {
        approx_index(&vals, &x)
    };
    match found {
        Some(i) => Value::Number(i as f64 + 1.0),
        None => Value::Error(ErrorKind::NA),
    }
}

fn xmatch(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(scalar_key(ctx, &a[0]));
    let arr = try_v!(lookup_array(ctx, &a[1]));
    let vals = try_v!(vector(&arr));
    let mm = try_v!(num_or(ctx, a, 2, 0.0));
    let sm = try_v!(num_or(ctx, a, 3, 1.0));
    match x_find(&vals, &x, mm, sm) {
        Some(i) => Value::Number(i as f64 + 1.0),
        None => Value::Error(ErrorKind::NA),
    }
}

/// XLOOKUP 찾기. 한셀은 일치 방식(0/−1/1/2)과 찾기 방향 ±1 을 무시하고 처음부터 정확히 같은 칸을 찾으며,
/// 찾기 방향 ±2 는 근사 이분 찾기([`approx_index`], x 이하)다 (대조 실험 search).
fn xlookup_find(vals: &[Value], x: &Value, sm: f64) -> Option<usize> {
    if sm.abs() == 2.0 {
        return approx_index(vals, x);
    }
    vals.iter().position(|v| value::lookup_eq(v, x))
}

/// XMATCH 찾기 (일치 방식 0/−1/1/2, 찾기 방향 1/−1/2/−2)
fn x_find(vals: &[Value], x: &Value, mm: f64, sm: f64) -> Option<usize> {
    let order: Vec<usize> = if sm < 0.0 {
        (0..vals.len()).rev().collect()
    } else {
        (0..vals.len()).collect()
    };
    if sm.abs() == 2.0 && mm == 0.0 {
        return approx_index(vals, x).filter(|&i| value::lookup_eq(&vals[i], x));
    }
    let wild = mm == 2.0;
    let mut best: Option<usize> = None;
    for &i in &order {
        let v = &vals[i];
        if wild {
            if let Value::Text(t) = x
                && let Value::Text(s) = v
                && wildcard_match(&value::fold(t), &value::fold(s))
            {
                return Some(i);
            }
            if value::lookup_eq(v, x) {
                return Some(i);
            }
            continue;
        }
        if value::lookup_eq(v, x) {
            return Some(i);
        }
        if mm != 0.0 && same_type(v, x) {
            let o = value::compare(v, x);
            let better = |b: usize| {
                let ob = value::compare(v, &vals[b]);
                if mm < 0.0 {
                    ob == Ordering::Greater
                } else {
                    ob == Ordering::Less
                }
            };
            if ((mm < 0.0 && o == Ordering::Less) || (mm > 0.0 && o == Ordering::Greater))
                && best.is_none_or(better)
            {
                best = Some(i);
            }
        }
    }
    best
}

fn vh_lookup(ctx: &mut Ctx, a: &[Value], vertical: bool) -> Value {
    let x = try_v!(scalar_key(ctx, &a[0]));
    let arr = try_v!(lookup_array(ctx, &a[1]));
    let n = try_v!(num_arg(ctx, a, 2)).trunc();
    let approx = try_v!(bool_or(ctx, a, 3, true));
    if n < 1.0 {
        return Value::Error(ErrorKind::Value);
    }
    let (len, width) = if vertical {
        (arr.rows, arr.cols)
    } else {
        (arr.cols, arr.rows)
    };
    if n as usize > width {
        return Value::Error(ErrorKind::Ref);
    }
    let keys: Vec<Value> = (0..len)
        .map(|i| {
            if vertical {
                arr.get(i, 0)
            } else {
                arr.get(0, i)
            }
            .clone()
        })
        .collect();
    let found = if approx {
        approx_index(&keys, &x)
    } else {
        exact_index(&keys, &x, true)
    };
    match found {
        Some(i) => {
            let k = n as usize - 1;
            if vertical {
                arr.get(i, k)
            } else {
                arr.get(k, i)
            }
            .clone()
        }
        None => Value::Error(ErrorKind::NA),
    }
}

fn lookup(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(scalar_key(ctx, &a[0]));
    let arr = try_v!(lookup_array(ctx, &a[1]));
    let (keys, results): (Vec<Value>, Vec<Value>) = match opt(a, 2) {
        None => {
            if arr.cols > arr.rows {
                (
                    (0..arr.cols).map(|c| arr.get(0, c).clone()).collect(),
                    (0..arr.cols)
                        .map(|c| arr.get(arr.rows - 1, c).clone())
                        .collect(),
                )
            } else {
                (
                    (0..arr.rows).map(|r| arr.get(r, 0).clone()).collect(),
                    (0..arr.rows)
                        .map(|r| arr.get(r, arr.cols - 1).clone())
                        .collect(),
                )
            }
        }
        Some(rv) => {
            let keys = try_v!(vector(&arr));
            let res = try_v!(lookup_array(ctx, rv));
            // 결과 범위는 찾는 범위와 같은 방향으로 읽는다: 세로 찾기에 가로 한 줄 결과면 둘째 칸부터 #REF!
            // (LOOKUP(35,Data!F1:F10,Data!A12:E12), 대조 실험 lookup)
            let vertical = arr.cols == 1 && arr.rows > 1;
            let horizontal = arr.rows == 1 && arr.cols > 1;
            let rv: Vec<Value> = if (vertical && res.rows == 1 && res.cols > 1)
                || (horizontal && res.cols == 1 && res.rows > 1)
            {
                vec![res.get(0, 0).clone()]
            } else {
                try_v!(vector(&res))
            };
            (keys, rv)
        }
    };
    match approx_index(&keys, &x) {
        Some(i) => results
            .get(i)
            .cloned()
            .unwrap_or(Value::Error(ErrorKind::Ref)),
        None => Value::Error(ErrorKind::NA),
    }
}

fn xlookup(ctx: &mut Ctx, a: &[Value]) -> Value {
    let x = try_v!(scalar_key(ctx, &a[0]));
    let la = try_v!(lookup_array(ctx, &a[1]));
    let vals = try_v!(vector(&la));
    // 넷째 인수를 비워 두면 언제나 #VALUE!, 아예 없으면 못 찾았을 때 0 (대조 실험 search·lookup)
    if matches!(a.get(3), Some(Value::Empty)) {
        return Value::Error(ErrorKind::Value);
    }
    let _ = try_v!(num_or(ctx, a, 4, 0.0));
    let sm = try_v!(num_or(ctx, a, 5, 1.0));
    let Some(i) = xlookup_find(&vals, &x, sm) else {
        return match a.get(3) {
            Some(v) => v.clone(),
            None => Value::Number(0.0),
        };
    };
    let vertical = la.cols == 1;
    match &a[2] {
        Value::Ref(r) if r.len() == 1 => {
            let ar = r[0];
            let sub = if vertical {
                if i as u32 > ar.r1 - ar.r0 {
                    return Value::Error(ErrorKind::Value);
                }
                Area {
                    r0: ar.r0 + i as u32,
                    r1: ar.r0 + i as u32,
                    ..ar
                }
            } else {
                if i as u32 > ar.c1 - ar.c0 {
                    return Value::Error(ErrorKind::Value);
                }
                Area {
                    c0: ar.c0 + i as u32,
                    c1: ar.c0 + i as u32,
                    ..ar
                }
            };
            Value::Ref(Rc::new(vec![sub]))
        }
        other => {
            let ra = try_v!(lookup_array(ctx, other));
            if vertical {
                if i >= ra.rows {
                    return Value::Error(ErrorKind::Value);
                }
                let d = (0..ra.cols).map(|c| ra.get(i, c).clone()).collect();
                Value::from(Array::new(1, ra.cols, d))
            } else {
                if i >= ra.cols {
                    return Value::Error(ErrorKind::Value);
                }
                let d = (0..ra.rows).map(|r| ra.get(r, i).clone()).collect();
                Value::from(Array::new(ra.rows, 1, d))
            }
        }
    }
}

fn choose(ctx: &mut Ctx, args: &[Expr]) -> Value {
    let idx = ctx.eval_value(&args[0]);
    if let Value::Array(ia) = idx {
        // 배열 번호: 번호 배열과 고른 값(배열)을 서로 늘려 맞춘다 (CHOOSE({1,2},A1:A2,B1:B2) 은 2×2)
        let opts: Vec<Value> = args[1..].iter().map(|e| ctx.eval_value(e)).collect();
        let dims = |v: &Value| match v {
            Value::Array(a) => (a.rows, a.cols),
            _ => (1, 1),
        };
        let (mut rows, mut cols) = (ia.rows, ia.cols);
        for o in &opts {
            let (r, c) = dims(o);
            rows = crate::calc::eval::broadcast_dim(rows, r);
            cols = crate::calc::eval::broadcast_dim(cols, c);
        }
        let mut data = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let v = ia.get_broadcast(r, c);
                data.push(match value::to_number(&v, ctx.date1904()) {
                    Ok(n) => {
                        let k = n.trunc();
                        if k < 1.0 || k as usize > opts.len() {
                            Value::Error(ErrorKind::Value)
                        } else {
                            match &opts[k as usize - 1] {
                                Value::Array(o) => o.get_broadcast(r, c),
                                o => o.clone(),
                            }
                        }
                    }
                    Err(_) => Value::Error(ErrorKind::Value),
                });
            }
        }
        return Value::from(Array::new(rows, cols, data));
    }
    // 번호가 오류면 #VALUE! (CHOOSE(#N/A,1,2), 대조 실험 lookup)
    let k = match ctx.num(&idx) {
        Ok(n) => n.trunc(),
        Err(_) => return Value::Error(ErrorKind::Value),
    };
    if k < 1.0 || k as usize >= args.len() {
        return Value::Error(ErrorKind::Value);
    }
    match &args[k as usize] {
        Expr::Missing => Value::Number(0.0),
        e => ctx.eval(e),
    }
}

fn offset(ctx: &mut Ctx, a: &[Value]) -> Value {
    // 참조가 아니면 (오류 값 포함) #VALUE! — 한셀: 행 삭제로 OFFSET(#REF!,4,0) 이 된 수식은 #VALUE! (편집 대조 실험)
    let area = match &a[0] {
        Value::Ref(r) if r.len() == 1 => r[0],
        _ => return Value::Error(ErrorKind::Value),
    };
    let dr = try_v!(num_arg(ctx, a, 1)).trunc();
    let dc = try_v!(num_arg(ctx, a, 2)).trunc();
    let h = match opt(a, 3) {
        Some(v) => try_v!(ctx.num(v)).trunc(),
        None => area.rows() as f64,
    };
    let w = match opt(a, 4) {
        Some(v) => try_v!(ctx.num(v)).trunc(),
        None => area.cols() as f64,
    };
    if h == 0.0 || w == 0.0 {
        return Value::Error(ErrorKind::Ref);
    }
    let r0 = area.r0 as f64 + dr;
    let c0 = area.c0 as f64 + dc;
    let (r0, r1) = if h > 0.0 {
        (r0, r0 + h - 1.0)
    } else {
        (r0 + h + 1.0, r0)
    };
    let (c0, c1) = if w > 0.0 {
        (c0, c0 + w - 1.0)
    } else {
        (c0 + w + 1.0, c0)
    };
    if r0 < 0.0 || c0 < 0.0 || r1 >= MAX_ROWS as f64 || c1 >= MAX_COLS as f64 {
        return Value::Error(ErrorKind::Ref);
    }
    Value::Ref(Rc::new(vec![Area {
        r0: r0 as u32,
        r1: r1 as u32,
        c0: c0 as u32,
        c1: c1 as u32,
        ..area
    }]))
}

fn indirect(ctx: &mut Ctx, a: &[Value]) -> Value {
    let t = try_v!(ctx.text(&a[0]));
    let a1 = try_v!(bool_or(ctx, a, 1, true));
    let t = t.trim();
    if t.is_empty() {
        return Value::Error(ErrorKind::Ref);
    }
    if !a1 {
        return match r1c1(ctx, t) {
            Some(v) => v,
            None => Value::Error(ErrorKind::Ref),
        };
    }
    match crate::calc::parser::parse(t) {
        Ok(e) if crate::calc::eval::is_reference_expr(&e) => match ctx.eval(&e) {
            v @ Value::Ref(_) => v,
            _ => Value::Error(ErrorKind::Ref),
        },
        _ => Value::Error(ErrorKind::Ref),
    }
}

/// R1C1 참조 (`R1C1`, `R[-1]C[2]`, `R1C1:R2C3`, `시트!R1C1`)
fn r1c1(ctx: &mut Ctx, s: &str) -> Option<Value> {
    let (sheet, body) = match s.rfind('!') {
        Some(p) => (
            Some(s[..p].trim_matches('\'').replace("''", "'")),
            &s[p + 1..],
        ),
        None => (None, s),
    };
    let cur = ctx.sheet;
    let sheet_idx = match sheet {
        Some(n) => {
            ctx.engine()
                .resolve_sheets(None, &SheetSpec::One(n), cur)
                .ok()?
                .1
        }
        None => cur,
    };
    let one = |p: &str| -> Option<(u32, u32)> {
        let p = p.to_ascii_uppercase();
        let rest = p.strip_prefix('R')?;
        let cpos = rest.find('C')?;
        let (rp, cp) = (&rest[..cpos], &rest[cpos + 1..]);
        let axis = |x: &str, cur: u32, max: u32| -> Option<u32> {
            if x.is_empty() {
                return Some(cur);
            }
            if let Some(inner) = x.strip_prefix('[').and_then(|y| y.strip_suffix(']')) {
                let d: i64 = inner.parse().ok()?;
                let v = cur as i64 + d;
                return (0..max as i64).contains(&v).then_some(v as u32);
            }
            let v: u32 = x.parse().ok()?;
            (1..=max).contains(&v).then(|| v - 1)
        };
        Some((axis(rp, ctx.row, MAX_ROWS)?, axis(cp, ctx.col, MAX_COLS)?))
    };
    let (a, b) = match body.split_once(':') {
        Some((x, y)) => (one(x)?, one(y)?),
        None => {
            let p = one(body)?;
            (p, p)
        }
    };
    Some(Value::Ref(Rc::new(vec![Area {
        book: 0,
        sheet: sheet_idx,
        sheet_last: sheet_idx,
        r0: a.0.min(b.0),
        r1: a.0.max(b.0),
        c0: a.1.min(b.1),
        c1: a.1.max(b.1),
    }])))
}

fn row_col(ctx: &mut Ctx, a: &[Value], row: bool) -> Value {
    let area = match a.first() {
        None | Some(Value::Empty) => {
            return Value::Number(if row { ctx.row } else { ctx.col } as f64 + 1.0);
        }
        Some(Value::Ref(r)) => {
            if r.len() != 1 {
                return Value::Error(ErrorKind::Ref);
            }
            r[0]
        }
        Some(Value::Error(e)) => return Value::Error(*e),
        Some(_) => return Value::Error(ErrorKind::Value),
    };
    let (lo, hi) = if row {
        (area.r0, area.r1)
    } else {
        (area.c0, area.c1)
    };
    if ctx.array_mode && hi > lo {
        let e = ctx.effective_area(&area);
        let hi = if row { e.r1 } else { e.c1 };
        let data: Vec<Value> = (lo..=hi).map(|v| Value::Number(v as f64 + 1.0)).collect();
        let n = data.len();
        return Value::from(if row {
            Array::new(n, 1, data)
        } else {
            Array::new(1, n, data)
        });
    }
    Value::Number(lo as f64 + 1.0)
}

fn dims(_ctx: &mut Ctx, a: &[Value], rows: bool) -> Value {
    match &a[0] {
        // 여러 영역·오류(빈 교차 포함)는 #VALUE! (ROWS((A1:A3,B1:B5)), ROWS(#N/A), 대조 실험 lookup·misc4)
        Value::Error(_) => Value::Error(ErrorKind::Value),
        Value::Ref(r) => {
            if r.len() != 1 {
                return Value::Error(ErrorKind::Value);
            }
            Value::Number(if rows { r[0].rows() } else { r[0].cols() } as f64)
        }
        Value::Array(arr) => Value::Number(if rows { arr.rows } else { arr.cols } as f64),
        _ => Value::Number(1.0),
    }
}

/// 시트 이름에 따옴표가 필요한가
pub fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && crate::calc::parser::parse(name).is_ok_and(|e| matches!(e, Expr::Name(_)));
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn address(ctx: &mut Ctx, a: &[Value]) -> Value {
    let row = try_v!(num_arg(ctx, a, 0)).trunc();
    let col = try_v!(num_arg(ctx, a, 1)).trunc();
    let abs = try_v!(num_or(ctx, a, 2, 1.0)).trunc();
    let a1 = try_v!(bool_or(ctx, a, 3, true));
    if !(1.0..=4.0).contains(&abs) {
        return Value::Error(ErrorKind::Value);
    }
    let (ra, ca) = match abs as u8 {
        1 => (true, true),
        2 => (true, false),
        3 => (false, true),
        _ => (false, false),
    };
    let body = if a1 {
        if row < 1.0 || col < 1.0 || row > MAX_ROWS as f64 || col > MAX_COLS as f64 {
            return Value::Error(ErrorKind::Value);
        }
        format!(
            "{}{}{}{}",
            if ca { "$" } else { "" },
            crate::model::CellRef::col_name(col as u32 - 1),
            if ra { "$" } else { "" },
            row as u32
        )
    } else {
        let r = if ra {
            format!("R{}", row)
        } else {
            format!("R[{}]", row)
        };
        let c = if ca {
            format!("C{}", col)
        } else {
            format!("C[{}]", col)
        };
        format!("{r}{c}")
    };
    match opt(a, 4) {
        Some(s) => {
            let s = try_v!(ctx.text(s));
            if s.is_empty() {
                Value::from(format!("!{body}"))
            } else {
                // 따옴표 안의 작은따옴표를 겹치지 않는다 ('it's'!$C$2, 대조 실험 lookup)
                let q = quote_sheet(&s);
                let q = if q.starts_with('\'') {
                    format!("'{s}'")
                } else {
                    q
                };
                Value::from(format!("{q}!{body}"))
            }
        }
        None => Value::from(body),
    }
}

fn formulatext(ctx: &mut Ctx, a: &[Value]) -> Value {
    let area = match &a[0] {
        Value::Ref(r) => r[0],
        Value::Error(e) => return Value::Error(*e),
        _ => return Value::Error(ErrorKind::NA),
    };
    if area.book != 0 {
        return Value::Error(ErrorKind::NA);
    }
    let eng = ctx.engine();
    match eng.formula_at(area.sheet, area.r0, area.c0) {
        Some(fid) => Value::from(format!("={}", eng.formulas[fid as usize].text)),
        None => Value::Error(ErrorKind::NA),
    }
}

fn unique(ctx: &mut Ctx, a: &[Value]) -> Value {
    let arr = try_v!(to_array(ctx, &a[0]));
    let by_col = try_v!(bool_or(ctx, a, 1, false));
    let once = try_v!(bool_or(ctx, a, 2, false));
    let arr = if by_col {
        Rc::new(arr.transpose())
    } else {
        arr
    };
    let rows: Vec<Vec<Value>> = (0..arr.rows)
        .map(|r| (0..arr.cols).map(|c| arr.get(r, c).clone()).collect())
        .collect();
    let eq = |x: &Vec<Value>, y: &Vec<Value>| {
        x.iter()
            .zip(y)
            .all(|(p, q)| value::lookup_eq(p, q) || (p.is_empty() && q.is_empty()))
    };
    let mut out: Vec<Vec<Value>> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let count = rows.iter().filter(|o| eq(o, r)).count();
        if once && count != 1 {
            continue;
        }
        if rows[..i].iter().any(|o| eq(o, r)) {
            continue;
        }
        out.push(r.clone());
    }
    if out.is_empty() {
        return Value::Error(ErrorKind::Calc);
    }
    let n = out.len();
    let res = Array::new(n, arr.cols, out.into_iter().flatten().collect());
    Value::from(if by_col { res.transpose() } else { res })
}

fn sort(ctx: &mut Ctx, a: &[Value]) -> Value {
    let arr = try_v!(to_array(ctx, &a[0]));
    let idx = try_v!(num_or(ctx, a, 1, 1.0)).trunc();
    let order = try_v!(num_or(ctx, a, 2, 1.0)).trunc();
    let by_col = try_v!(bool_or(ctx, a, 3, false));
    let arr = if by_col {
        Rc::new(arr.transpose())
    } else {
        arr
    };
    if idx < 1.0 || idx as usize > arr.cols || !(order == 1.0 || order == -1.0) {
        return Value::Error(ErrorKind::Value);
    }
    let k = idx as usize - 1;
    let mut rows: Vec<Vec<Value>> = (0..arr.rows)
        .map(|r| (0..arr.cols).map(|c| arr.get(r, c).clone()).collect())
        .collect();
    rows.sort_by(|x, y| {
        let o = value::compare(&x[k], &y[k]);
        if order < 0.0 { o.reverse() } else { o }
    });
    let n = rows.len();
    let res = Array::new(n, arr.cols, rows.into_iter().flatten().collect());
    Value::from(if by_col { res.transpose() } else { res })
}

fn filter(ctx: &mut Ctx, a: &[Value]) -> Value {
    let arr = try_v!(to_array(ctx, &a[0]));
    let inc = try_v!(to_array(ctx, &a[1]));
    let keep = |v: &Value| value::to_bool(v);
    if inc.cols == 1 && inc.rows == arr.rows {
        let mut out = Vec::new();
        let mut n = 0;
        for r in 0..arr.rows {
            if try_v!(keep(inc.get(r, 0))) {
                out.extend((0..arr.cols).map(|c| arr.get(r, c).clone()));
                n += 1;
            }
        }
        if n == 0 {
            return a
                .get(2)
                .cloned()
                .filter(|v| !v.is_empty())
                .unwrap_or(Value::Error(ErrorKind::Calc));
        }
        return Value::from(Array::new(n, arr.cols, out));
    }
    if inc.rows == 1 && inc.cols == arr.cols {
        let cols: Vec<usize> = (0..arr.cols)
            .filter(|&c| keep(inc.get(0, c)).unwrap_or(false))
            .collect();
        if cols.is_empty() {
            return a
                .get(2)
                .cloned()
                .filter(|v| !v.is_empty())
                .unwrap_or(Value::Error(ErrorKind::Calc));
        }
        let mut out = Vec::new();
        for r in 0..arr.rows {
            out.extend(cols.iter().map(|&c| arr.get(r, c).clone()));
        }
        return Value::from(Array::new(arr.rows, cols.len(), out));
    }
    Value::Error(ErrorKind::Value)
}
