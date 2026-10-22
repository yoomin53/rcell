// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 데이터베이스 함수 (DSUM …): 첫 행이 머리글인 목록, 필드(머리글 글이나 1부터 번호), 조건 표.
//!
//! 조건 표: 첫 행은 머리글, 그 아래 행마다 조건. 한 행 안의 조건은 모두 맞아야 하고(AND),
//! 행 중 하나만 맞으면 된다(OR). 연산자 없는 글 조건은 "그 글로 시작".

use super::stat::basic_aggregate;
use super::{Arg, FnSpec, f, to_array};
use crate::calc::criteria::Criterion;
use crate::calc::eval::Ctx;
use crate::calc::value::{self, Array, ErrorKind, Value, num};

const RVR: &[Arg] = &[Arg::R, Arg::V, Arg::R];

pub const FNS: &[FnSpec] = &[
    f("DSUM", 3, 3, RVR, |c, a| dfn(c, a, 9)),
    f("DAVERAGE", 3, 3, RVR, |c, a| dfn(c, a, 1)),
    f("DCOUNT", 3, 3, RVR, |c, a| dfn(c, a, 2)),
    f("DCOUNTA", 3, 3, RVR, |c, a| dfn(c, a, 3)),
    f("DMAX", 3, 3, RVR, |c, a| dfn(c, a, 4)),
    f("DMIN", 3, 3, RVR, |c, a| dfn(c, a, 5)),
    f("DPRODUCT", 3, 3, RVR, |c, a| dfn(c, a, 6)),
    f("DSTDEV", 3, 3, RVR, |c, a| dfn(c, a, 7)),
    f("DSTDEVP", 3, 3, RVR, |c, a| dfn(c, a, 8)),
    f("DVAR", 3, 3, RVR, |c, a| dfn(c, a, 10)),
    f("DVARP", 3, 3, RVR, |c, a| dfn(c, a, 11)),
    f("DGET", 3, 3, RVR, |c, a| dfn(c, a, 0)),
];

/// 필드 → 열 번호 (0부터)
fn field_index(ctx: &mut Ctx, db: &Array, field: &Value) -> Result<Option<usize>, ErrorKind> {
    match field {
        Value::Empty => Ok(None),
        Value::Text(t) => {
            let f = value::fold(t);
            (0..db.cols)
                .find(|&c| matches!(db.get(0, c), Value::Text(h) if value::fold(h) == f))
                .map(Some)
                .ok_or(ErrorKind::Value)
        }
        Value::Error(e) => Err(*e),
        other => {
            let n = ctx.num(other)?.trunc();
            if n < 1.0 || n as usize > db.cols {
                return Err(ErrorKind::Value);
            }
            Ok(Some(n as usize - 1))
        }
    }
}

/// 조건 표에 맞는 데이터 행 (1부터, 머리글 제외)
fn matching_rows(ctx: &mut Ctx, db: &Array, crit: &Array) -> Result<Vec<usize>, ErrorKind> {
    if crit.rows < 1 {
        return Err(ErrorKind::Value);
    }
    // 조건 머리글 → 데이터 열
    let mut cols: Vec<Option<usize>> = Vec::new();
    for c in 0..crit.cols {
        match crit.get(0, c) {
            Value::Text(h) => {
                let f = value::fold(h);
                cols.push(
                    (0..db.cols)
                        .find(|&k| matches!(db.get(0, k), Value::Text(x) if value::fold(x) == f)),
                );
            }
            Value::Empty => cols.push(None),
            _ => cols.push(None),
        }
    }
    let d1904 = ctx.date1904();
    let conds: Vec<Vec<(usize, Criterion)>> = (1..crit.rows.max(2))
        .map(|r| {
            (0..crit.cols)
                .filter_map(|c| {
                    if r >= crit.rows {
                        return None;
                    }
                    let v = crit.get(r, c);
                    if v.is_empty() {
                        return None;
                    }
                    cols[c].map(|k| (k, Criterion::parse_database(v, d1904)))
                })
                .collect()
        })
        .collect();
    let mut out = Vec::new();
    for r in 1..db.rows {
        let hit = conds.is_empty()
            || conds
                .iter()
                .any(|row| row.iter().all(|(k, c)| c.matches(db.get(r, *k))));
        if hit {
            out.push(r);
        }
    }
    Ok(out)
}

fn dfn(ctx: &mut Ctx, a: &[Value], kind: u8) -> Value {
    let db = try_v!(to_array(ctx, &a[0]));
    let crit = try_v!(to_array(ctx, &a[2]));
    let field = match &a[1] {
        Value::Ref(r) => ctx.intersect_ref(r),
        o => o.clone(),
    };
    let col = try_v!(field_index(ctx, &db, &field));
    let rows = try_v!(matching_rows(ctx, &db, &crit));
    let Some(col) = col else {
        // 필드 생략: DCOUNT 는 맞는 행 수
        return match kind {
            2 | 3 => Value::Number(rows.len() as f64),
            _ => Value::Error(ErrorKind::Value),
        };
    };
    let vals: Vec<Value> = rows.iter().map(|&r| db.get(r, col).clone()).collect();
    match kind {
        0 => match vals.len() {
            0 => Value::Error(ErrorKind::Value),
            1 => vals[0].clone(),
            _ => Value::Error(ErrorKind::Num),
        },
        2 => Value::Number(
            vals.iter()
                .filter(|v| matches!(v, Value::Number(_)))
                .count() as f64,
        ),
        3 => Value::Number(vals.iter().filter(|v| !v.is_empty()).count() as f64),
        _ => {
            let nums: Vec<f64> = vals
                .iter()
                .filter_map(|v| {
                    if let Value::Number(n) = v {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .collect();
            match basic_aggregate(kind, &nums) {
                Value::Number(n) => num(n),
                o => o,
            }
        }
    }
}
