// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 정보 함수.

use super::{Arg, FnSpec, R, V, f, first_area};
use crate::calc::ast::SheetSpec;
use crate::calc::eval::Ctx;
use crate::calc::value::{ErrorKind, Value};

const VR: &[Arg] = &[Arg::V, Arg::R];

pub const FNS: &[FnSpec] = &[
    f("ISBLANK", 1, 1, V, |_, a| Value::Bool(a[0].is_empty())),
    f("ISERR", 1, 1, V, |_, a| Value::Bool(matches!(a[0], Value::Error(e) if e != ErrorKind::NA))),
    f("ISERROR", 1, 1, V, |_, a| Value::Bool(a[0].is_error())),
    f("ISNA", 1, 1, V, |_, a| Value::Bool(matches!(a[0], Value::Error(ErrorKind::NA)))),
    f("ISLOGICAL", 1, 1, V, |_, a| Value::Bool(matches!(a[0], Value::Bool(_)))),
    f("ISNUMBER", 1, 1, V, |_, a| Value::Bool(matches!(a[0], Value::Number(_)))),
    f("ISTEXT", 1, 1, V, |_, a| Value::Bool(matches!(a[0], Value::Text(_)))),
    f("ISNONTEXT", 1, 1, V, |_, a| Value::Bool(!matches!(a[0], Value::Text(_)))),
    f("ISREF", 1, 1, R, |_, a| Value::Bool(matches!(a[0], Value::Ref(_)))),
    f("ISFORMULA", 1, 1, R, |c, a| match first_area(&a[0]) {
        Some(ar) if ar.book == 0 => Value::Bool(c.engine().formula_at(ar.sheet, ar.r0, ar.c0).is_some()),
        Some(_) => Value::Bool(false),
        None => match &a[0] {
            Value::Error(e) => Value::Error(*e),
            _ => Value::Error(ErrorKind::Value),
        },
    }),
    f("ISEVEN", 1, 1, V, |c, a| even_odd(c, a, true)),
    f("ISODD", 1, 1, V, |c, a| even_odd(c, a, false)),
    f("N", 1, 1, V, |_, a| match &a[0] {
        Value::Number(n) => Value::Number(*n),
        Value::Bool(b) => Value::Number(*b as u8 as f64),
        Value::Error(e) => Value::Error(*e),
        _ => Value::Number(0.0),
    }),
    f("NA", 0, 0, V, |_, _| Value::Error(ErrorKind::NA)),
    f("ERROR.TYPE", 1, 1, V, error_type),
    f("ERRORTYPE", 1, 1, V, error_type),
    f("TYPE", 1, 1, R, |c, a| match &a[0] {
        Value::Ref(r) => {
            if r.len() == 1 && r[0].is_cell() {
                let v = c.intersect_ref(r);
                Value::Number(v.type_code() as f64)
            } else {
                Value::Number(64.0)
            }
        }
        v => Value::Number(v.type_code() as f64),
    }),
    f("SHEET", 0, 1, R, sheet),
    f("SHEETS", 0, 1, R, |c, a| match a.first() {
        None | Some(Value::Empty) => Value::Number(c.engine().sheet_count() as f64),
        Some(Value::Ref(r)) => Value::Number(r.iter().map(|x| (x.sheet_last - x.sheet + 1) as f64).sum()),
        Some(Value::Error(e)) => Value::Error(*e),
        Some(_) => Value::Error(ErrorKind::Value),
    }),
    f("CELL", 1, 2, VR, cell),
    f("INFO", 1, 1, V, |c, a| {
        let t = try_v!(c.text(&a[0])).to_ascii_lowercase();
        match t.as_str() {
            "numfile" => Value::Number(1.0),
            "recalc" => Value::from("Automatic"),
            "system" => Value::from("pcdos"),
            "origin" => Value::from("$A:$A$1"),
            _ => Value::Error(ErrorKind::NA),
        }
    }),
];

fn even_odd(ctx: &mut Ctx, a: &[Value], even: bool) -> Value {
    if let Value::Bool(_) = a[0] {
        return Value::Error(ErrorKind::Value);
    }
    let n = try_v!(ctx.num(&a[0])).trunc();
    let is_even = n % 2.0 == 0.0;
    Value::Bool(if even { is_even } else { !is_even })
}

fn error_type(_: &mut Ctx, a: &[Value]) -> Value {
    match &a[0] {
        Value::Error(e) => Value::Number(e.code() as f64),
        _ => Value::Error(ErrorKind::NA),
    }
}

fn sheet(ctx: &mut Ctx, a: &[Value]) -> Value {
    match a.first() {
        None | Some(Value::Empty) => Value::Number(ctx.sheet as f64 + 1.0),
        Some(Value::Ref(r)) => match r.first() {
            Some(x) if x.book == 0 => Value::Number(x.sheet as f64 + 1.0),
            _ => Value::Error(ErrorKind::NA),
        },
        Some(Value::Text(t)) => {
            match ctx
                .engine()
                .resolve_sheets(None, &SheetSpec::One(t.to_string()), 0)
            {
                Ok((_, i, _)) => Value::Number(i as f64 + 1.0),
                Err(_) => Value::Error(ErrorKind::NA),
            }
        }
        Some(Value::Error(e)) => Value::Error(*e),
        Some(_) => Value::Error(ErrorKind::NA),
    }
}

/// 표시 형식 코드 → CELL("format") 기호
fn format_class(code: &str) -> String {
    let c = code.split(';').next().unwrap_or("");
    let lower = c.to_ascii_lowercase();
    if c.eq_ignore_ascii_case("general") || c.is_empty() {
        return "G".into();
    }
    let decimals = c
        .split_once('.')
        .map(|(_, f)| f.chars().take_while(|&ch| ch == '0' || ch == '#').count())
        .unwrap_or(0);
    if lower.contains('y') || lower.contains('d') && !lower.contains('"') {
        return if lower.contains('h') { "D4".into() } else { "D1".into() };
    }
    if lower.contains('h') || lower.contains('s') {
        return "D9".into();
    }
    if c.contains('%') {
        return format!("P{decimals}");
    }
    if c.contains('E') || c.contains('e') {
        return format!("S{decimals}");
    }
    if c.contains('$') || c.contains('₩') {
        return format!("C{decimals}");
    }
    if c.contains(',') {
        return format!(",{decimals}");
    }
    format!("F{decimals}")
}

fn cell(ctx: &mut Ctx, a: &[Value]) -> Value {
    let what = try_v!(ctx.text(&a[0])).to_ascii_lowercase();
    let area = match a.get(1) {
        Some(Value::Ref(r)) => r[0],
        Some(Value::Error(e)) => return Value::Error(*e),
        Some(Value::Empty) | None => crate::calc::value::Area::cell(ctx.sheet, ctx.row, ctx.col),
        Some(_) => return Value::Error(ErrorKind::Value),
    };
    let (sheet, row, col) = (area.sheet, area.r0, area.c0);
    match what.as_str() {
        "address" => {
            let addr = format!("${}${}", crate::model::CellRef::col_name(col), row + 1);
            if sheet != ctx.sheet && area.book == 0 {
                let name = ctx.engine().sheet_names[sheet as usize].clone();
                Value::from(format!("{}!{addr}", super::lookup::quote_sheet(&name)))
            } else {
                Value::from(addr)
            }
        }
        "row" => Value::Number(row as f64 + 1.0),
        "col" => Value::Number(col as f64 + 1.0),
        "contents" => ctx.area_cell(&area, sheet, row, col),
        "type" => {
            let v = ctx.area_cell(&area, sheet, row, col);
            Value::from(match v {
                Value::Empty => "b",
                Value::Text(_) => "l",
                _ => "v",
            })
        }
        "format" | "color" | "parentheses" => {
            let eng = ctx.engine();
            let xf = eng.cell_style(sheet, row, col) as usize;
            let code = eng.xf_codes.get(xf).cloned().unwrap_or_else(|| "General".into());
            match what.as_str() {
                "format" => {
                    let mut s = format_class(&code);
                    if code.split(';').nth(1).is_some_and(|n| n.contains("[Red]") || n.contains("[red]")) {
                        s.push('-');
                    }
                    if code.contains('(') {
                        s.push_str("()");
                    }
                    Value::from(s)
                }
                "color" => Value::Number(
                    code.split(';').nth(1).is_some_and(|n| n.contains('[')) as u8 as f64,
                ),
                _ => Value::Number(code.split(';').next().is_some_and(|p| p.contains('(')) as u8 as f64),
            }
        }
        "prefix" => {
            let v = ctx.area_cell(&area, sheet, row, col);
            Value::from(if matches!(v, Value::Text(_)) { "'" } else { "" })
        }
        "protect" => Value::Number(1.0),
        "width" => Value::Number(9.0),
        "filename" => Value::from(""),
        _ => Value::Error(ErrorKind::Value),
    }
}
