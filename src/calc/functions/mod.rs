// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 함수 등록부.
//!
//! 분류마다 파일 하나(`math`, `stat`, `text`, `logical`, `lookup`, `date`, `info`, `financial`,
//! `engineering`, `database`)가 `FNS` 표를 내놓고, 여기서 이름으로 찾는다. 새 함수는 해당 분류 파일의
//! 표에 한 줄을 더하면 된다 (다른 파일을 건드릴 필요가 없다).
//!
//! 인수 분류 ([`Arg`]):
//! - `V` 값: 참조는 수식 셀의 행·열로 교차(배열 수식이면 배열로 바꿈)하고, 배열이 오면 원소마다 함수를 부른다.
//! - `R` 참조 우선: 참조는 그대로 넘기고, 그 밖의 식은 현재 문맥대로 계산한다 (SUM 같은 집계).
//! - `A` 배열: 식을 배열 문맥에서 계산하고 참조는 배열로 바꾼다 (SUMPRODUCT, MMULT …).
//! - `RA` 참조 우선, 참조가 아닌 식은 배열 문맥 (INDEX 의 첫 인수 …).

pub mod database;
pub mod date;
pub mod engineering;
pub mod financial;
pub mod hcnum;
pub mod info;
pub mod logical;
pub mod lookup;
pub mod math;
pub mod special;
pub mod stat;
pub mod text;
pub mod units;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use super::ast::Expr;
use super::eval::Ctx;
use super::value::{Area, Array, ErrorKind, Value};

/// 인수 분류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    V,
    R,
    A,
    RA,
}

/// 구현
#[derive(Clone, Copy)]
pub enum Imp {
    /// 인수를 분류대로 계산해서 받는다
    Eager(fn(&mut Ctx, &[Value]) -> Value),
    /// 계산하지 않은 식을 받는다 (IF, CHOOSE, IFERROR … 처럼 필요한 인수만 계산)
    Lazy(fn(&mut Ctx, &[Expr]) -> Value),
}

pub struct FnSpec {
    pub name: &'static str,
    pub min: u8,
    /// 255 = 제한 없음
    pub max: u8,
    /// 앞쪽 인수의 분류. 남는 인수는 마지막 `repeat` 개의 분류를 차례로 되풀이한다.
    pub args: &'static [Arg],
    /// 되풀이하는 분류 수 (SUMIFS 의 (범위, 조건) 쌍은 2)
    pub repeat: u8,
    pub imp: Imp,
    /// 계산할 때마다 값이 바뀌는 함수 (NOW, TODAY, RAND, RANDBETWEEN)
    pub volatile: bool,
}

impl std::fmt::Debug for FnSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FnSpec({})", self.name)
    }
}

impl FnSpec {
    pub fn class(&self, i: usize) -> Arg {
        let n = self.args.len();
        if n == 0 {
            return Arg::V;
        }
        if i < n {
            return self.args[i];
        }
        let rep = (self.repeat as usize).clamp(1, n);
        let base = n - rep;
        self.args[base + (i - base) % rep]
    }
}

/// 표 한 줄을 짧게 쓰기 위한 생성자
pub const fn f(
    name: &'static str,
    min: u8,
    max: u8,
    args: &'static [Arg],
    imp: fn(&mut Ctx, &[Value]) -> Value,
) -> FnSpec {
    FnSpec {
        name,
        min,
        max,
        args,
        repeat: 1,
        imp: Imp::Eager(imp),
        volatile: false,
    }
}

pub const fn lazy(
    name: &'static str,
    min: u8,
    max: u8,
    imp: fn(&mut Ctx, &[Expr]) -> Value,
) -> FnSpec {
    FnSpec {
        name,
        min,
        max,
        args: &[],
        repeat: 1,
        imp: Imp::Lazy(imp),
        volatile: false,
    }
}

pub const fn volatile(mut s: FnSpec) -> FnSpec {
    s.volatile = true;
    s
}

/// 남는 인수가 마지막 n 개 분류를 되풀이한다 (SUMIFS 의 (범위, 조건) 쌍)
pub const fn pairs(mut s: FnSpec, n: u8) -> FnSpec {
    s.repeat = n;
    s
}

pub const V: &[Arg] = &[Arg::V];
pub const R: &[Arg] = &[Arg::R];
pub const A: &[Arg] = &[Arg::A];
pub const MANY: u8 = 255;

fn registry() -> &'static HashMap<&'static str, &'static FnSpec> {
    static REG: OnceLock<HashMap<&'static str, &'static FnSpec>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut m = HashMap::new();
        for table in [
            math::FNS,
            stat::FNS,
            text::FNS,
            logical::FNS,
            lookup::FNS,
            date::FNS,
            info::FNS,
            financial::FNS,
            engineering::FNS,
            database::FNS,
        ] {
            for s in table {
                m.insert(s.name, s);
            }
        }
        m
    })
}

/// 정규화한 이름(대문자, `_xlfn.` 없음)으로 함수를 찾는다.
pub fn lookup(name: &str) -> Option<&'static FnSpec> {
    registry().get(name).copied()
}

/// 구현한 함수 이름 전부
pub fn implemented() -> Vec<&'static str> {
    let mut v: Vec<&str> = registry().keys().copied().collect();
    v.sort_unstable();
    v
}

/// 스프레드시트 함수로 알려진 이름인가 (구현 여부와 무관). 모르는 이름은 #NAME? 이다.
pub fn is_known(name: &str) -> bool {
    static KNOWN: OnceLock<HashSet<String>> = OnceLock::new();
    KNOWN
        .get_or_init(|| {
            let mut s: HashSet<String> = HashSet::new();
            for i in 0..2000u16 {
                if let Some((n, _)) = crate::legacy::ftab::lookup(i) {
                    s.insert(n.to_string());
                }
            }
            for n in NEWER_FUNCTIONS {
                s.insert(n.to_string());
            }
            for n in HANCOM_FUNCTIONS {
                s.insert(n.to_string());
            }
            s
        })
        .contains(name)
}

/// Excel 2010 이후 함수 (`_xlfn.` 접두사로 저장되는 것)
pub const NEWER_FUNCTIONS: &[&str] = &[
    "ACOT", "ACOTH", "AGGREGATE", "ARABIC", "BASE", "BETA.DIST", "BETA.INV", "BINOM.DIST",
    "BINOM.DIST.RANGE", "BINOM.INV", "BITAND", "BITLSHIFT", "BITOR", "BITRSHIFT", "BITXOR",
    "CEILING.MATH", "CEILING.PRECISE", "CHISQ.DIST", "CHISQ.DIST.RT", "CHISQ.INV", "CHISQ.INV.RT",
    "CHISQ.TEST", "COMBINA", "CONCAT", "CONFIDENCE.NORM", "CONFIDENCE.T", "COT", "COTH",
    "COVARIANCE.P", "COVARIANCE.S", "CSC", "CSCH", "DAYS", "DECIMAL", "ERF.PRECISE",
    "ERFC.PRECISE", "EXPON.DIST", "F.DIST", "F.DIST.RT", "F.INV", "F.INV.RT", "F.TEST",
    "FILTER", "FLOOR.MATH", "FLOOR.PRECISE", "FORECAST.LINEAR", "FORMULATEXT", "GAMMA",
    "GAMMA.DIST", "GAMMA.INV", "GAMMALN.PRECISE", "GAUSS", "HYPGEOM.DIST", "IFNA", "IFS",
    "IMCOSH", "IMCOT", "IMCSC", "IMCSCH", "IMSEC", "IMSECH", "IMSINH", "IMTAN", "ISFORMULA",
    "ISO.CEILING", "ISOWEEKNUM", "LET", "LOGNORM.DIST", "LOGNORM.INV", "MAXIFS", "MINIFS",
    "MODE.MULT", "MODE.SNGL", "MUNIT", "NEGBINOM.DIST", "NETWORKDAYS.INTL", "NORM.DIST",
    "NORM.INV", "NORM.S.DIST", "NORM.S.INV", "NUMBERVALUE", "PDURATION", "PERCENTILE.EXC",
    "PERCENTILE.INC", "PERCENTRANK.EXC", "PERCENTRANK.INC", "PERMUTATIONA", "PHI",
    "POISSON.DIST", "QUARTILE.EXC", "QUARTILE.INC", "RANDARRAY", "RANK.AVG", "RANK.EQ", "RRI",
    "SEC", "SECH", "SEQUENCE", "SHEET", "SHEETS", "SKEW.P", "SORT", "SORTBY", "STDEV.P",
    "STDEV.S", "SWITCH", "T.DIST", "T.DIST.2T", "T.DIST.RT", "T.INV", "T.INV.2T", "T.TEST",
    "TEXTJOIN", "UNICHAR", "UNICODE", "UNIQUE", "VAR.P", "VAR.S", "WEIBULL.DIST",
    "WORKDAY.INTL", "XLOOKUP", "XMATCH", "XOR", "Z.TEST", "ENCODEURL", "FILTERXML",
    "WEBSERVICE", "COUNTIFS", "SUMIFS", "AVERAGEIF", "AVERAGEIFS", "IFERROR",
];

/// 한셀 전용 함수 (관측: 레거시·바이너리 함수 대조 실험, `docs/format/calc.md`)
pub const HANCOM_FUNCTIONS: &[&str] = &["SUBTOTALA", "CONVERTK"];

/// 한셀 2024 가 모르는 함수: 함수 대조 실험에서 모든 경우가 #NAME? 이었다. 도움말에 있는 WON·JUNJA·RANKEQ·
/// RANKAVG·ERRORTYPE·DATESTRING 도 수식으로 쓰면 #NAME? 이다 (COM 으로 넣어도 같다). 기본 설정은 한셀처럼
/// #NAME? 을 내고, [`crate::calc::CalcOptions::excel_functions`] 면 구현한 것은 계산한다.
pub const HCELL_UNKNOWN: &[&str] = &[
    "AGGREGATE", "ARRAYTOTEXT", "BAHTTEXT", "BETA.DIST", "CHOOSEROWS", "CUBEMEMBER", "CUBESETCOUNT", "CUBEVALUE",
    "DATESTRING", "DROP", "ENCODEURL", "ERRORTYPE", "FILTERXML", "FORECAST.ETS", "FORECAST.LINEAR",
    "HSTACK", "IMAGE", "ISOMITTED", "ISTHAIDIGIT", "JUNJA", "PDURATION", "RANKAVG", "RANKEQ",
    "ROUNDBAHTDOWN", "ROUNDBAHTUP", "RRI", "RTD", "SINGLE", "SORTBY", "STOCKHISTORY", "TAKE",
    "TEXTAFTER", "TEXTBEFORE", "TEXTSPLIT", "THAIDAYOFWEEK", "THAIDIGIT", "THAIMONTHOFYEAR",
    "THAINUMSOUND", "THAINUMSTRING", "THAISTRINGLENGTH", "THAIYEAR", "TOCOL", "VALUETOTEXT", "VSTACK",
    "WON", "XMATCH",
];

/// 한셀이 모르는 함수인가
pub fn hcell_unknown(name: &str) -> bool {
    HCELL_UNKNOWN.binary_search(&name).is_ok()
}

// ---------------------------------------------------------------------------
// 구현에서 함께 쓰는 도우미
// ---------------------------------------------------------------------------

/// 인수 i 가 없으면 None. 쉼표만 있고 비운 인수(`ERF(0.5,)`)는 빈 값으로 있다 — 한셀은 이것을 기본값이
/// 아니라 0·""·FALSE 로 계산한다 (`ERF(0.5,)` = ERF(0.5,0), `CEILING.MATH(-5.5,,1)` = 0, 함수 대조 실험).
pub fn opt(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i)
}

/// 인수 i 가 없거나 비웠으면 None (비운 인수를 생략으로 보는 함수용)
pub fn given(args: &[Value], i: usize) -> Option<&Value> {
    match args.get(i) {
        None | Some(Value::Empty) => None,
        Some(v) => Some(v),
    }
}

/// 집계 함수가 범위·배열·직접 인수를 다루는 방식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agg {
    /// SUM·AVERAGE·MIN·MAX·COUNT: 범위·배열의 숫자만, 직접 인수는 숫자로 바꾼다 (글은 해석, 논리 0/1)
    Numbers,
    /// AVERAGEA·MAXA·…: 범위·배열의 글은 0, 논리는 0/1
    NumbersA,
}

/// 집계 인수들에서 숫자를 모은다. 오류는 만나는 대로 돌려준다.
pub fn collect_numbers(ctx: &mut Ctx, args: &[Value], mode: Agg) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for a in args {
        collect_one(ctx, a, mode, &mut out)?;
    }
    Ok(out)
}

pub fn collect_one(
    ctx: &mut Ctx,
    a: &Value,
    mode: Agg,
    out: &mut Vec<f64>,
) -> Result<(), ErrorKind> {
    match a {
        Value::Ref(r) => {
            for area in r.iter() {
                let mut err = None;
                ctx.for_each_in_area(area, |_, _, v| {
                    if err.is_some() {
                        return;
                    }
                    match v {
                        Value::Number(n) => out.push(*n),
                        Value::Error(e) => err = Some(*e),
                        Value::Bool(b) if mode == Agg::NumbersA => out.push(*b as u8 as f64),
                        Value::Text(_) if mode == Agg::NumbersA => out.push(0.0),
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
                    Value::Number(n) => out.push(*n),
                    Value::Error(e) => return Err(*e),
                    Value::Bool(b) if mode == Agg::NumbersA => out.push(*b as u8 as f64),
                    Value::Text(_) if mode == Agg::NumbersA => out.push(0.0),
                    _ => {}
                }
            }
        }
        Value::Empty => out.push(0.0),
        Value::Number(n) => out.push(*n),
        Value::Bool(b) => out.push(*b as u8 as f64),
        // AVERAGEA·MAXA·… 는 직접 준 글도 0 (AVERAGEA(1,2,3,"4",TRUE) = 1.4, 대조 실험 stat)
        Value::Text(_) if mode == Agg::NumbersA => out.push(0.0),
        Value::Text(_) => out.push(ctx.num(a)?),
        Value::Error(e) => return Err(*e),
    }
    Ok(())
}

/// 범위·배열 인수의 값들을 평평하게 (빈 칸 포함, 행 우선). 직접 값은 그대로 하나.
pub fn flatten(ctx: &mut Ctx, v: &Value) -> Vec<Value> {
    match v {
        Value::Ref(r) => {
            let mut out = Vec::new();
            for area in r.iter() {
                match ctx.area_to_array(area) {
                    Ok(a) => out.extend(a.data.iter().cloned()),
                    Err(e) => out.push(Value::Error(e)),
                }
            }
            out
        }
        Value::Array(a) => a.data.clone(),
        other => vec![other.clone()],
    }
}

/// 범위·배열 인수를 배열 하나로 (여러 영역 참조는 #VALUE!)
pub fn to_array(ctx: &mut Ctx, v: &Value) -> Result<Rc<Array>, ErrorKind> {
    match v {
        Value::Ref(r) => {
            if r.len() != 1 {
                return Err(ErrorKind::Value);
            }
            ctx.area_to_array(&r[0]).map(Rc::new)
        }
        Value::Array(a) => Ok(a.clone()),
        Value::Error(e) => Err(*e),
        other => Ok(Rc::new(Array::new(1, 1, vec![other.clone()]))),
    }
}

/// 참조 인수의 첫 영역
pub fn first_area(v: &Value) -> Option<Area> {
    match v {
        Value::Ref(r) => r.first().copied(),
        _ => None,
    }
}

/// 스칼라 인수를 숫자로
pub fn num_arg(ctx: &Ctx, args: &[Value], i: usize) -> Result<f64, ErrorKind> {
    ctx.num(args.get(i).unwrap_or(&Value::Empty))
}

/// 생략 가능한 숫자 인수
pub fn num_or(ctx: &Ctx, args: &[Value], i: usize, default: f64) -> Result<f64, ErrorKind> {
    match opt(args, i) {
        None => Ok(default),
        Some(v) => ctx.num(v),
    }
}

/// 생략 가능한 논리 인수
pub fn bool_or(ctx: &Ctx, args: &[Value], i: usize, default: bool) -> Result<bool, ErrorKind> {
    match opt(args, i) {
        None => Ok(default),
        Some(v) => ctx.boolean(v),
    }
}

pub fn text_arg(ctx: &Ctx, args: &[Value], i: usize) -> Result<Rc<str>, ErrorKind> {
    ctx.text(args.get(i).unwrap_or(&Value::Empty))
}

/// 정수로 자르기 (함수 인수의 정수 변환은 버림)
pub fn trunc_arg(ctx: &Ctx, args: &[Value], i: usize) -> Result<f64, ErrorKind> {
    Ok(num_arg(ctx, args, i)?.trunc())
}

/// 같은 크기의 숫자 배열 두 개 (SUMXMY2, CORREL …): 둘 다 숫자인 쌍만. 한셀은 오류 값이 든 쌍도 건너뛴다
/// (CORREL(Data!H1:H10,…), SUMX2MY2(Data!H1:H10,…) 이 숫자, 대조 실험 stat·math).
pub fn paired_numbers(
    ctx: &mut Ctx,
    a: &Value,
    b: &Value,
) -> Result<(Vec<f64>, Vec<f64>), ErrorKind> {
    if let Value::Error(e) = a {
        return Err(*e);
    }
    if let Value::Error(e) = b {
        return Err(*e);
    }
    let xa = flatten(ctx, a);
    let xb = flatten(ctx, b);
    if xa.len() != xb.len() {
        return Err(ErrorKind::NA);
    }
    let mut va = Vec::new();
    let mut vb = Vec::new();
    for (x, y) in xa.iter().zip(&xb) {
        if let (Value::Number(x), Value::Number(y)) = (x, y) {
            va.push(*x);
            vb.push(*y);
        }
    }
    Ok((va, vb))
}
