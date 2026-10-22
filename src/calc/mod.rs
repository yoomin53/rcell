// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 계산 엔진.
//!
//! 수식 원문(모델의 `Formula::text`, OOXML 표기) → 구문 트리([`parser`]) → 계산([`eval`]).
//! 엔진([`engine`])이 의존 순서대로 모든 수식을 계산한다. 함수는 분류별 파일([`functions`])에 있다.
//! 정답은 한셀이 저장한 파일의 캐시 값(`<v>`)이다 — `tests/calc_corpus.rs`, `tests/calc_oracle.rs`.
//!
//! ```no_run
//! let mut wb = rcell::open("문서.cell")?;
//! let report = rcell::calc::recalculate(&mut wb);
//! println!("수식 {}개 계산", report.computed);
//! # Ok::<(), rcell::Error>(())
//! ```

/// `Result<_, ErrorKind>` 가 오류면 그 오류 값을 돌려준다 (함수 구현용)
macro_rules! try_v {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return $crate::calc::value::Value::Error(e),
        }
    };
}

pub mod ast;
pub mod collate;
pub mod cp949;
pub mod criteria;
pub mod date;
pub mod engine;
pub mod eval;
pub mod fm;
pub mod functions;
pub mod parser;
pub mod value;

use std::collections::BTreeMap;

use serde::Serialize;

pub use engine::Engine;
pub use parser::{ParseError, parse};
pub use value::{ErrorKind, Value};

use crate::model::{CellRef, CellValue, Workbook};

/// 계산 설정
#[derive(Debug, Clone)]
pub struct CalcOptions {
    /// NOW()·TODAY() 의 시각 (일련 번호). None 이면 현재 시각.
    pub now: Option<f64>,
    /// 현재 시각을 쓸 때의 시간대 (분). 기본 +9:00 (한국 표준시) — 표준 라이브러리로는 지역 시간대를 알 수 없다.
    pub utc_offset_minutes: i32,
    /// 한셀이 모르는 Excel 함수(AGGREGATE, XMATCH …)도 계산한다. 기본은 한셀처럼 #NAME?.
    pub excel_functions: bool,
    /// 문서 파일 이름 (CELL("address") 가 다른 시트를 가리키면 한셀은 `[이름]시트!$A$1` 로 쓴다)
    pub file_name: Option<String>,
}

impl Default for CalcOptions {
    fn default() -> CalcOptions {
        CalcOptions {
            now: None,
            utc_offset_minutes: 9 * 60,
            excel_functions: false,
            file_name: None,
        }
    }
}

/// 수식 셀 하나의 계산 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CellStatus {
    /// 계산함
    Computed,
    /// 계산했지만 계산할 때마다 바뀐다 (NOW, TODAY, RAND …에 기대는 셀)
    Volatile,
    /// 구현하지 않은 함수를 써서 파일의 값을 그대로 둠
    Unsupported,
    /// 수식을 해석하지 못함
    ParseError,
    /// 순환 참조
    Cycle,
}

/// 수식 셀 하나의 결과
#[derive(Debug, Clone)]
pub struct FormulaResult {
    pub sheet: usize,
    pub cell: CellRef,
    pub formula: String,
    /// 계산한 값
    pub value: CellValue,
    /// 파일에 있던 값 (한셀이 계산한 값)
    pub cached: CellValue,
    pub status: CellStatus,
}

/// 계산 요약
#[derive(Debug, Clone, Default, Serialize)]
pub struct CalcReport {
    pub formula_cells: usize,
    pub computed: usize,
    pub volatile: usize,
    pub unsupported: usize,
    pub parse_errors: usize,
    pub cycles: usize,
    /// 구현하지 않은 함수 → 그 함수를 쓴 수식 셀 수
    pub unsupported_functions: BTreeMap<String, usize>,
}

/// 지금 시각의 일련 번호
pub fn now_serial(utc_offset_minutes: i32, date1904: bool) -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
        + utc_offset_minutes as f64 * 60.0;
    // 1970-01-01 = 25569 (1900 체계)
    let serial = secs / 86_400.0 + 25_569.0;
    if date1904 { serial - 1462.0 } else { serial }
}

fn engine_for(wb: &Workbook, opts: &CalcOptions) -> Engine {
    let now = opts
        .now
        .unwrap_or_else(|| now_serial(opts.utc_offset_minutes, wb.date1904));
    let mut eng = Engine::new(wb, now);
    eng.excel_functions = opts.excel_functions;
    eng.file_name = opts.file_name.clone();
    eng
}

/// 계산을 마친 엔진에서 수식 셀마다 결과와 요약을 모은다
fn collect(eng: &Engine) -> (Vec<FormulaResult>, CalcReport) {
    let mut report = CalcReport {
        formula_cells: eng.formulas.len(),
        unsupported_functions: eng.unsupported.clone(),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(eng.formulas.len());
    for f in &eng.formulas {
        let sd = &eng.sheets[f.sheet as usize];
        let value = sd
            .find(f.row, f.col)
            .map(|i| sd.values[i].to_cell())
            .unwrap_or_default();
        let status = if f.parse_error.is_some() {
            report.parse_errors += 1;
            CellStatus::ParseError
        } else if f.unsupported {
            report.unsupported += 1;
            CellStatus::Unsupported
        } else if f.cycle {
            report.cycles += 1;
            CellStatus::Cycle
        } else if f.volatile {
            report.volatile += 1;
            CellStatus::Volatile
        } else {
            CellStatus::Computed
        };
        if matches!(
            status,
            CellStatus::Computed | CellStatus::Volatile | CellStatus::Cycle
        ) {
            report.computed += 1;
        }
        match f.array {
            // 배열 수식: 범위 칸마다 하나씩 (행 우선)
            Some((h, w)) => {
                for r in 0..h {
                    for c in 0..w {
                        let (row, col) = (f.row + r, f.col + c);
                        let k = (r * w + c) as usize;
                        out.push(FormulaResult {
                            sheet: f.sheet as usize,
                            cell: CellRef { row, col },
                            formula: f.text.clone(),
                            value: sd
                                .find(row, col)
                                .map(|i| sd.values[i].to_cell())
                                .unwrap_or_default(),
                            cached: f
                                .array_cached
                                .get(k)
                                .map(Value::to_cell_or_empty)
                                .unwrap_or_default(),
                            status,
                        });
                    }
                }
            }
            None => out.push(FormulaResult {
                sheet: f.sheet as usize,
                cell: CellRef {
                    row: f.row,
                    col: f.col,
                },
                formula: f.text.clone(),
                value,
                cached: f.cached.to_cell_or_empty(),
                status,
            }),
        }
    }
    (out, report)
}

/// 모든 수식을 계산해 결과를 돌려준다 (문서는 바꾸지 않는다).
pub fn evaluate(wb: &Workbook, opts: &CalcOptions) -> (Vec<FormulaResult>, CalcReport) {
    let mut eng = engine_for(wb, opts);
    eng.recalc_all();
    collect(&eng)
}

/// 모든 수식을 다시 계산해 셀 값을 바꾼다 (기본 설정).
pub fn recalculate(wb: &mut Workbook) -> CalcReport {
    recalculate_with(wb, &CalcOptions::default())
}

/// 모든 수식을 다시 계산해 셀 값을 바꾼다. 구현하지 않은 함수를 쓴 셀은 파일의 값을 그대로 둔다.
pub fn recalculate_with(wb: &mut Workbook, opts: &CalcOptions) -> CalcReport {
    let mut eng = engine_for(wb, opts);
    eng.recalc_all();
    let (_, report) = collect(&eng);
    for (si, sheet) in wb.sheets.iter_mut().enumerate() {
        let sd = &eng.sheets[si];
        for cell in &mut sheet.cells {
            if let Some(i) = sd.find(cell.r.row, cell.r.col)
                && sd.fid[i] != u32::MAX
            {
                cell.value = sd.values[i].to_cell();
            }
        }
    }
    report
}

/// 셀 하나의 값을 계산한다 (수식이 아니면 그 값).
pub fn evaluate_cell(wb: &Workbook, sheet: usize, cell: CellRef) -> CellValue {
    let mut eng = engine_for(wb, &CalcOptions::default());
    eng.evaluate_one(sheet as u32, cell.row, cell.col).to_cell()
}

/// 수식 원문을 셀 위치에서 계산한다 (문서는 바꾸지 않는다).
pub fn evaluate_formula(
    wb: &Workbook,
    sheet: usize,
    at: CellRef,
    formula: &str,
) -> Result<CellValue, ParseError> {
    let e = parser::parse(formula)?;
    let mut eng = engine_for(wb, &CalcOptions::default());
    Ok(eng
        .evaluate_expr(&e, sheet as u32, at.row, at.col)
        .to_cell())
}

/// 계산한 값과 한셀의 캐시 값이 같은가: 숫자는 상대 오차 1e-12 이내이거나 셀 서식으로 같은 모양,
/// 글은 그대로, 논리, 오류는 종류.
pub fn same_value(
    computed: &CellValue,
    cached: &CellValue,
    num_fmt: Option<&str>,
    date1904: bool,
) -> bool {
    match (computed, cached) {
        (CellValue::Number(a), CellValue::Number(b)) => {
            if a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs()) {
                return true;
            }
            match num_fmt {
                Some(code) if !code.eq_ignore_ascii_case("General") => {
                    let fa = crate::numfmt::format(computed, code, date1904);
                    let fb = crate::numfmt::format(cached, code, date1904);
                    fa.text == fb.text
                }
                _ => false,
            }
        }
        (CellValue::Text(a), CellValue::Text(b)) => a == b,
        (CellValue::Bool(a), CellValue::Bool(b)) => a == b,
        (CellValue::Error(a), CellValue::Error(b)) => {
            ErrorKind::parse(a).is_some() && ErrorKind::parse(a) == ErrorKind::parse(b)
        }
        _ => false,
    }
}

/// 셀의 표시 형식 코드
pub fn cell_format_code(wb: &Workbook, sheet: usize, cell: CellRef) -> String {
    let style = wb
        .sheets
        .get(sheet)
        .and_then(|s| s.cell(cell))
        .map(|c| c.style)
        .unwrap_or(0);
    wb.styles
        .xf(style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into())
}

/// 계산 결과가 파일의 캐시 값(한셀이 계산한 값)과 같은가. 캐시 값이 없으면 None.
pub fn matches_cached(wb: &Workbook, r: &FormulaResult) -> Option<bool> {
    if r.cached.is_empty() {
        return None;
    }
    let code = cell_format_code(wb, r.sheet, r.cell);
    Some(same_value(&r.value, &r.cached, Some(&code), wb.date1904))
}

impl Value {
    /// 파일 캐시 값 → 모델 값 (빈 값은 빈 값 그대로)
    fn to_cell_or_empty(&self) -> CellValue {
        match self {
            Value::Empty => CellValue::Empty,
            v => v.to_cell(),
        }
    }
}

#[cfg(test)]
mod tests;
