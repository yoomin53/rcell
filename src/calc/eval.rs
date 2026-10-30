// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 식 계산.
//!
//! 값 문맥에서 참조는 수식 셀의 행·열과 교차해 셀 하나가 된다 (암묵 교차). 배열 문맥(배열 수식,
//! `A` 분류 인수 안)에서는 참조가 배열이 되고, 연산자와 값 인수 함수는 원소마다 계산한다.
//! 배열 상수와 배열을 돌려주는 함수의 결과는 어느 문맥에서나 원소마다 계산한다.

use crate::calc::fm::Fm;
use std::rc::Rc;

use super::ast::{BinaryOp, Call, Expr, NameExpr, RefExpr, SheetSpec, UnaryOp};
use super::engine::Engine;
use super::functions::{Arg, FnSpec, Imp};
use super::value::{self, Area, Array, ErrorKind, MAX_COLS, MAX_ROWS, Value};

/// 계산 문맥: 수식이 있는 셀과 엔진
pub struct Ctx<'a> {
    pub(crate) eng: &'a mut Engine,
    pub sheet: u32,
    pub row: u32,
    pub col: u32,
    /// 배열 문맥 (배열 수식이거나 배열 인수 안)
    pub array_mode: bool,
    /// 수식 자체가 배열 수식인가 (배열 인수 안이라도 바뀌지 않는다)
    pub array_formula: bool,
    name_depth: u32,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(
        eng: &'a mut Engine,
        sheet: u32,
        row: u32,
        col: u32,
        array_mode: bool,
    ) -> Ctx<'a> {
        Ctx {
            eng,
            sheet,
            row,
            col,
            array_mode,
            array_formula: array_mode,
            name_depth: 0,
        }
    }

    pub fn date1904(&self) -> bool {
        self.eng.date1904
    }

    /// 계산 시각 (NOW)
    pub fn now(&mut self) -> f64 {
        self.eng.mark_volatile();
        self.eng.now
    }

    pub fn num(&self, v: &Value) -> Result<f64, ErrorKind> {
        value::to_number(v, self.eng.date1904)
    }

    pub fn text(&self, v: &Value) -> Result<Rc<str>, ErrorKind> {
        value::to_text(v)
    }

    pub fn boolean(&self, v: &Value) -> Result<bool, ErrorKind> {
        value::to_bool(v)
    }

    pub fn engine(&mut self) -> &mut Engine {
        self.eng
    }

    // -----------------------------------------------------------------------
    // 식
    // -----------------------------------------------------------------------

    /// 식을 계산한다. 참조·배열은 줄이지 않고 돌려준다.
    pub fn eval(&mut self, e: &Expr) -> Value {
        match e {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(s) => Value::Text(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Error(k) => Value::Error(*k),
            Expr::Missing => Value::Empty,
            Expr::Array(a) => Value::Array(a.clone()),
            Expr::Ref(r) => self.resolve_ref(r),
            Expr::Name(n) => self.eval_name(n),
            Expr::Table(t) => match self.eng.resolve_table(t, self.sheet, self.row, self.col) {
                Ok(a) => Value::Ref(Rc::new(vec![a])),
                Err(e) => Value::Error(e),
            },
            Expr::Paren(x) => self.eval(x),
            Expr::Unary(op, x) => self.unary(*op, x),
            Expr::Binary(op, a, b) => match op {
                BinaryOp::Range | BinaryOp::Union | BinaryOp::Intersect => self.ref_op(*op, a, b),
                _ => self.binary(*op, a, b),
            },
            Expr::Call(c) => self.call(c),
        }
    }

    /// 값 문맥: 참조는 암묵 교차(배열 문맥이면 배열), 배열은 그대로
    pub fn eval_value(&mut self, e: &Expr) -> Value {
        let v = self.eval(e);
        self.deref(v)
    }

    /// 참조를 값으로: 값 문맥이면 암묵 교차, 배열 문맥이면 배열
    pub fn deref(&mut self, v: Value) -> Value {
        match v {
            Value::Ref(r) => {
                if self.array_mode {
                    self.ref_to_array(&r)
                } else {
                    self.intersect_ref(&r)
                }
            }
            other => other,
        }
    }

    /// 배열 문맥에서 계산하고 참조는 배열로
    pub fn eval_array(&mut self, e: &Expr) -> Value {
        let save = self.array_mode;
        self.array_mode = true;
        let v = self.eval(e);
        let v = match v {
            Value::Ref(r) => self.ref_to_array(&r),
            other => other,
        };
        self.array_mode = save;
        v
    }

    /// 인수 분류대로 계산
    pub fn eval_class(&mut self, e: &Expr, cls: Arg) -> Value {
        match cls {
            Arg::V => self.eval_value(e),
            Arg::R => self.eval(e),
            Arg::A => self.eval_array(e),
            Arg::RA => {
                if is_reference_expr(e) {
                    let v = self.eval(e);
                    if matches!(v, Value::Ref(_)) {
                        return v;
                    }
                    return self.deref(v);
                }
                self.eval_array(e)
            }
        }
    }

    /// 스칼라 하나로 (값 문맥: 참조는 교차, 배열은 왼쪽 위)
    pub fn scalar(&mut self, v: Value) -> Value {
        match v {
            Value::Ref(r) => self.intersect_ref(&r),
            Value::Array(a) => a
                .data
                .first()
                .cloned()
                .unwrap_or(Value::Error(ErrorKind::Value)),
            other => other,
        }
    }

    /// 수식 하나의 최종 결과: 빈 값은 0, 참조는 교차, 배열은 왼쪽 위 (배열 수식은 배열 그대로)
    pub fn eval_top(&mut self, e: &Expr) -> Value {
        let v = self.eval(e);
        let v = if self.array_mode {
            match v {
                Value::Ref(r) => self.ref_to_array(&r),
                other => other,
            }
        } else {
            self.scalar(v)
        };
        match v {
            Value::Empty => Value::Number(0.0),
            Value::Number(0.0) => Value::Number(0.0),
            Value::Number(n) if !n.is_finite() => Value::Error(ErrorKind::Num),
            other => other,
        }
    }

    fn resolve_ref(&mut self, r: &RefExpr) -> Value {
        match self.eng.resolve_sheets(r.book, &r.sheet, self.sheet) {
            Ok((book, s0, s1)) => Value::Ref(Rc::new(vec![Area {
                book,
                sheet: s0,
                sheet_last: s1,
                r0: r.r0,
                c0: r.c0,
                r1: r.r1,
                c1: r.c1,
            }])),
            Err(e) => Value::Error(e),
        }
    }

    fn eval_name(&mut self, n: &NameExpr) -> Value {
        if n.book.is_some() {
            return Value::Error(ErrorKind::Ref);
        }
        let scope = match &n.sheet {
            Some(s) => match self
                .eng
                .resolve_sheets(None, &SheetSpec::One(s.clone()), self.sheet)
            {
                Ok((_, i, _)) => Some(i),
                Err(e) => return Value::Error(e),
            },
            None => None,
        };
        let Some(expr) = self.eng.name_expr(&n.name, scope, self.sheet) else {
            return Value::Error(ErrorKind::Name);
        };
        if self.name_depth > 32 {
            return Value::Error(ErrorKind::Ref);
        }
        self.name_depth += 1;
        let v = self.eval(&expr);
        self.name_depth -= 1;
        v
    }

    // -----------------------------------------------------------------------
    // 참조
    // -----------------------------------------------------------------------

    /// 암묵 교차: 한 열짜리는 수식의 행, 한 행짜리는 수식의 열을 고른다
    pub fn intersect_ref(&mut self, areas: &[Area]) -> Value {
        if areas.len() != 1 {
            return Value::Error(ErrorKind::Value);
        }
        let a = areas[0];
        if a.is_3d() {
            return Value::Error(ErrorKind::Value);
        }
        let row = if a.r0 == a.r1 {
            a.r0
        } else if (a.r0..=a.r1).contains(&self.row) {
            self.row
        } else {
            return Value::Error(ErrorKind::Value);
        };
        let col = if a.c0 == a.c1 {
            a.c0
        } else if (a.c0..=a.c1).contains(&self.col) {
            self.col
        } else {
            return Value::Error(ErrorKind::Value);
        };
        self.area_cell(&a, a.sheet, row, col)
    }

    /// 참조 → 배열 (영역 하나만)
    pub fn ref_to_array(&mut self, areas: &[Area]) -> Value {
        if areas.len() != 1 {
            return Value::Error(ErrorKind::Value);
        }
        // 여러 시트에 걸친 영역을 배열로 읽으면 #VALUE! ('S1:S3'!A1&"x", SUMPRODUCT('S1:S3'!A1), TEXT 대조 실험).
        // 참조를 받는 함수(SUM·LARGE·TRIMMEAN 등)는 `flatten` 이 시트마다 읽는다.
        // 한 칸은 배열이 아니라 낱값이다 (XOR(255<=Data!C3,Data!B1&"") 의 글은 낱값이라 #VALUE!, 대조 실험 rand)
        if areas[0].is_cell() {
            let a = areas[0];
            return self.area_cell(&a, a.sheet, a.r0, a.c0);
        }
        match self.area_to_array(&areas[0]) {
            Ok(a) => Value::Array(Rc::new(a)),
            Err(e) => Value::Error(e),
        }
    }

    /// 열·행 전체 참조를 문서가 쓰는 범위까지 줄인 영역 (배열로 바꿀 때 쓴다)
    pub fn effective_area(&self, a: &Area) -> Area {
        let mut e = *a;
        if a.r0 == 0 && a.r1 == MAX_ROWS - 1 {
            e.r1 = self.eng.used_rows.saturating_sub(1).max(a.r0);
        }
        if a.c0 == 0 && a.c1 == MAX_COLS - 1 {
            e.c1 = self.eng.used_cols.saturating_sub(1).max(a.c0);
        }
        e
    }

    pub fn area_to_array(&mut self, a: &Area) -> Result<Array, ErrorKind> {
        if a.is_3d() {
            return Err(ErrorKind::Value);
        }
        let e = self.effective_area(a);
        let rows = e.rows() as usize;
        let cols = e.cols() as usize;
        if rows.saturating_mul(cols) > 8_000_000 {
            return Err(ErrorKind::Num);
        }
        let mut data = vec![Value::Empty; rows * cols];
        self.for_each_in_area(&e, |r, c, v| {
            data[(r - e.r0) as usize * cols + (c - e.c0) as usize] = v.clone();
        });
        Ok(Array::new(rows, cols, data))
    }

    /// 영역 안의 값이 있는 셀마다 (빈 칸은 건너뛴다). 수식 셀은 먼저 계산한다.
    pub fn for_each_in_area(&mut self, a: &Area, f: impl FnMut(u32, u32, &Value)) {
        self.eng.for_each_in_area(a, f);
    }

    /// 영역 안 시트 `sheet` 의 셀 값
    pub fn area_cell(&mut self, a: &Area, sheet: u32, row: u32, col: u32) -> Value {
        self.eng.cell_value(a.book, sheet, row, col)
    }

    /// 이 통합 문서의 셀 값
    pub fn cell(&mut self, sheet: u32, row: u32, col: u32) -> Value {
        self.eng.cell_value(0, sheet, row, col)
    }

    fn ref_op(&mut self, op: BinaryOp, a: &Expr, b: &Expr) -> Value {
        let va = self.eval(a);
        let vb = self.eval(b);
        let (ra, rb) = match (va, vb) {
            (Value::Error(e), _) | (_, Value::Error(e)) => return Value::Error(e),
            (Value::Ref(x), Value::Ref(y)) => (x, y),
            _ => return Value::Error(ErrorKind::Value),
        };
        match op {
            BinaryOp::Range => {
                let mut acc: Option<Area> = None;
                for area in ra.iter().chain(rb.iter()) {
                    acc = match acc {
                        None => Some(*area),
                        Some(x) => match x.bounding(area) {
                            Some(b) => Some(b),
                            // 다른 시트끼리의 범위 연산 (SUM(Data!A1:A2:B3)) — 한셀은 #NULL!
                            None => return Value::Error(ErrorKind::Null),
                        },
                    };
                }
                match acc {
                    Some(x) => Value::Ref(Rc::new(vec![x])),
                    None => Value::Error(ErrorKind::Ref),
                }
            }
            BinaryOp::Union => {
                let mut v: Vec<Area> = ra.to_vec();
                v.extend(rb.iter().copied());
                Value::Ref(Rc::new(v))
            }
            _ => {
                let mut out = Vec::new();
                for x in ra.iter() {
                    for y in rb.iter() {
                        if let Some(i) = x.intersect(y) {
                            out.push(i);
                        }
                    }
                }
                if out.is_empty() {
                    Value::Error(ErrorKind::Null)
                } else {
                    Value::Ref(Rc::new(out))
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 연산자
    // -----------------------------------------------------------------------

    fn unary(&mut self, op: UnaryOp, x: &Expr) -> Value {
        let v = self.eval_value(x);
        match op {
            UnaryOp::Plus => v,
            UnaryOp::Neg => self.map1(v, |ctx, s| match ctx.num(&s) {
                Ok(n) => Value::Number(-n),
                Err(e) => Value::Error(e),
            }),
            UnaryOp::Percent => self.map1(v, |ctx, s| match ctx.num(&s) {
                Ok(n) => Value::Number(n / 100.0),
                Err(e) => Value::Error(e),
            }),
        }
    }

    /// 스칼라 함수를 배열 원소마다
    pub fn map1(&mut self, v: Value, mut f: impl FnMut(&mut Ctx, Value) -> Value) -> Value {
        match v {
            Value::Array(a) => {
                let data: Vec<Value> = a.data.iter().map(|x| f(self, x.clone())).collect();
                Value::Array(Rc::new(Array::new(a.rows, a.cols, data)))
            }
            other => f(self, other),
        }
    }

    fn binary(&mut self, op: BinaryOp, a: &Expr, b: &Expr) -> Value {
        let va = self.eval_value(a);
        let vb = self.eval_value(b);
        self.binary_values(op, va, vb)
    }

    /// 두 값의 연산 (배열이면 원소마다, 크기가 다르면 넘치는 칸은 #N/A)
    pub fn binary_values(&mut self, op: BinaryOp, va: Value, vb: Value) -> Value {
        match (&va, &vb) {
            (Value::Array(x), Value::Array(y)) => {
                let rows = broadcast_dim(x.rows, y.rows);
                let cols = broadcast_dim(x.cols, y.cols);
                let mut data = Vec::with_capacity(rows * cols);
                for r in 0..rows {
                    for c in 0..cols {
                        let p = x.get_broadcast(r, c);
                        let q = y.get_broadcast(r, c);
                        data.push(self.scalar_op(op, &p, &q));
                    }
                }
                Value::Array(Rc::new(Array::new(rows, cols, data)))
            }
            (Value::Array(x), _) => {
                let data = x.data.iter().map(|p| self.scalar_op(op, p, &vb)).collect();
                Value::Array(Rc::new(Array::new(x.rows, x.cols, data)))
            }
            (_, Value::Array(y)) => {
                let data = y.data.iter().map(|q| self.scalar_op(op, &va, q)).collect();
                Value::Array(Rc::new(Array::new(y.rows, y.cols, data)))
            }
            _ => self.scalar_op(op, &va, &vb),
        }
    }

    pub fn scalar_op(&mut self, op: BinaryOp, a: &Value, b: &Value) -> Value {
        use BinaryOp::*;
        match op {
            Add | Sub | Mul | Div | Pow => {
                let x = match self.num(a) {
                    Ok(x) => x,
                    Err(e) => return Value::Error(e),
                };
                let y = match self.num(b) {
                    Ok(y) => y,
                    Err(e) => return Value::Error(e),
                };
                arith(op, x, y)
            }
            Concat => {
                let x = match self.text(a) {
                    Ok(x) => x,
                    Err(e) => return Value::Error(e),
                };
                let y = match self.text(b) {
                    Ok(y) => y,
                    Err(e) => return Value::Error(e),
                };
                let mut s = String::with_capacity(x.len() + y.len());
                s.push_str(&x);
                s.push_str(&y);
                Value::from(s)
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                if let Value::Error(e) = a {
                    return Value::Error(*e);
                }
                if let Value::Error(e) = b {
                    return Value::Error(*e);
                }
                let o = value::compare(a, b);
                use std::cmp::Ordering::*;
                Value::Bool(match op {
                    Eq => o == Equal,
                    Ne => o != Equal,
                    Lt => o == Less,
                    Le => o != Greater,
                    Gt => o == Greater,
                    _ => o != Less,
                })
            }
            Range | Union | Intersect => Value::Error(ErrorKind::Value),
        }
    }

    // -----------------------------------------------------------------------
    // 함수
    // -----------------------------------------------------------------------

    fn call(&mut self, c: &Call) -> Value {
        if !self.eng.excel_functions && super::functions::hcell_unknown(&c.name) {
            return Value::Error(ErrorKind::Name);
        }
        let Some(spec) = c.func else {
            if super::functions::is_known(&c.name) {
                self.eng.mark_unsupported(&c.name);
            }
            return Value::Error(ErrorKind::Name);
        };
        if spec.volatile {
            self.eng.mark_volatile();
        }
        let n = c.args.len();
        if n < spec.min as usize || (spec.max != super::functions::MANY && n > spec.max as usize) {
            // 한셀은 FILTER·SORT 의 인수 개수가 틀리면 #NAME? (FILTER(1), SORT(1,2,3,4,5), 대조 실험 arity)
            if matches!(spec.name, "FILTER" | "SORT") {
                return Value::Error(ErrorKind::Name);
            }
            return Value::Error(ErrorKind::Value);
        }
        match spec.imp {
            Imp::Lazy(f) => f(self, &c.args),
            Imp::Eager(f) => {
                let mut vals = Vec::with_capacity(n);
                let mut lift = false;
                for (i, a) in c.args.iter().enumerate() {
                    let cls = spec.class(i);
                    let v = self.eval_class(a, cls);
                    if cls == Arg::V && matches!(v, Value::Array(_)) {
                        lift = true;
                    }
                    vals.push(v);
                }
                if lift {
                    return self.lift(spec, f, vals);
                }
                f(self, &vals)
            }
        }
    }

    /// 값 인수에 배열이 오면 원소마다 함수를 부른다
    fn lift(
        &mut self,
        spec: &FnSpec,
        f: fn(&mut Ctx, &[Value]) -> Value,
        vals: Vec<Value>,
    ) -> Value {
        let mut rows = 1;
        let mut cols = 1;
        let mut first = true;
        for (i, v) in vals.iter().enumerate() {
            if spec.class(i) != Arg::V {
                continue;
            }
            if let Value::Array(a) = v {
                if first {
                    rows = a.rows;
                    cols = a.cols;
                    first = false;
                } else {
                    rows = broadcast_dim(rows, a.rows);
                    cols = broadcast_dim(cols, a.cols);
                }
            }
        }
        let mut data: Vec<Value> = Vec::with_capacity(rows * cols);
        let mut args = vals.clone();
        for r in 0..rows {
            for c in 0..cols {
                for (i, v) in vals.iter().enumerate() {
                    if spec.class(i) == Arg::V
                        && let Value::Array(a) = v
                    {
                        args[i] = a.get_broadcast(r, c);
                    }
                }
                let out = f(self, &args);
                data.push(match out {
                    // 배열을 돌려주는 함수를 여러 칸에 펼치면 #VALUE! (MUNIT(범위) — 한셀, 대조 실험 rand)
                    Value::Array(_) if rows * cols > 1 => Value::Error(ErrorKind::Value),
                    Value::Array(a) => a
                        .data
                        .first()
                        .cloned()
                        .unwrap_or(Value::Error(ErrorKind::Value)),
                    Value::Ref(rf) => self.intersect_ref(&rf),
                    o => o,
                });
            }
        }
        // 한 칸짜리 결과는 낱값이다 (배열 수식의 SUM(범위, IMCSC(칸)) 에서 글이 인수로 보인다: 한셀은 #VALUE!,
        // 대조 실험 prec)
        if rows * cols == 1 {
            return data.pop().unwrap_or(Value::Empty);
        }
        Value::Array(Rc::new(Array::new(rows, cols, data)))
    }
}

/// 원소마다 계산할 때의 크기: 한쪽이 1 이면 늘리고, 아니면 큰 쪽 (작은 쪽 밖은 #N/A)
pub fn broadcast_dim(a: usize, b: usize) -> usize {
    if a == 1 {
        b
    } else if b == 1 {
        a
    } else {
        a.max(b)
    }
}

/// 참조를 돌려줄 수 있는 식인가 (참조, 이름, 참조 연산, 참조 함수)
pub fn is_reference_expr(e: &Expr) -> bool {
    match e {
        Expr::Ref(_) | Expr::Name(_) | Expr::Table(_) => true,
        Expr::Paren(x) => is_reference_expr(x),
        Expr::Binary(BinaryOp::Range | BinaryOp::Union | BinaryOp::Intersect, _, _) => true,
        Expr::Call(c) => matches!(
            c.name.as_str(),
            "INDEX"
                | "OFFSET"
                | "INDIRECT"
                | "IF"
                | "CHOOSE"
                | "IFERROR"
                | "IFNA"
                | "IFS"
                | "SWITCH"
                | "XLOOKUP"
        ),
        _ => false,
    }
}

/// 사칙연산과 거듭제곱 연산자
pub fn arith(op: BinaryOp, x: f64, y: f64) -> Value {
    let r = match op {
        BinaryOp::Add => x + y,
        BinaryOp::Sub => x - y,
        BinaryOp::Mul => x * y,
        BinaryOp::Div => {
            if y == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            x / y
        }
        // 한셀의 `^` 는 0^0 = 1 (POWER(0,0) 은 #NUM!)
        BinaryOp::Pow if x == 0.0 && y == 0.0 => return Value::Number(1.0),
        BinaryOp::Pow => return power(x, y),
        _ => return Value::Error(ErrorKind::Value),
    };
    value::num(r)
}

/// POWER: 0^0·0 의 음수 제곱·음수의 분수 제곱은 #NUM! (한셀은 세제곱근 같은 홀수 제곱근도 #NUM!)
pub fn power(x: f64, y: f64) -> Value {
    if x == 0.0 {
        if y <= 0.0 {
            return Value::Error(ErrorKind::Num);
        }
        return Value::Number(0.0);
    }
    if x < 0.0 && y.fract() != 0.0 {
        return Value::Error(ErrorKind::Num);
    }
    value::num(x.mpow(y))
}
