// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 계산 엔진: 셀 저장소, 이름, 외부 값 캐시, 의존 그래프와 다시 계산 순서.
//!
//! 수식마다 구문 트리의 정적 참조(이름을 거친 것 포함)로 의존 그래프를 만들고, 깊이 우선
//! 후위 순서(선행 셀이 먼저)로 계산한다. INDIRECT·OFFSET 처럼 계산해 봐야 아는 참조는 그때
//! 필요한 셀을 먼저 계산한다. 재귀 깊이가 한도를 넘으면 계산을 멈추고 필요한 셀부터 바깥에서
//! 계산한 뒤 다시 시도한다 (호출 스택이 넘치지 않는다). 순환 참조는 그래프와 계산 중 표시로
//! 찾고, 순환 안의 셀은 파일에 있던 값을 읽는다. calcPr iterate 이면 순환과 그것에 기대는 셀을 파일 값에서
//! 시작해 2×iterateCount 번 행 차례로 다시 계산한다 (한셀과 같은 반복 계산).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use super::ast::{Expr, RefOrName, SheetSpec};
use super::eval::Ctx;
use super::parser;
use super::value::{self, Area, ErrorKind, Value};
use crate::model::{FormulaKind, Workbook};

const NO_FORMULA: u32 = u32::MAX;
/// 수식 안에서 다른 수식을 바로 계산하는 최대 깊이
const MAX_DEPTH: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Pending,
    InProgress,
    Done,
}

pub(crate) struct SheetData {
    pub keys: Vec<(u32, u32)>,
    pub values: Vec<Value>,
    pub fid: Vec<u32>,
    pub styles: Vec<u32>,
    /// (행, 열, 수식 번호), 행 우선
    pub formula_pos: Vec<(u32, u32, u32)>,
    pub hidden_rows: HashSet<u32>,
    pub filter_mode: bool,
}

impl SheetData {
    pub fn find(&self, row: u32, col: u32) -> Option<usize> {
        self.keys.binary_search(&(row, col)).ok()
    }
    /// 행이 r0..=r1 인 셀의 번호 범위
    pub fn row_span(&self, r0: u32, r1: u32) -> (usize, usize) {
        let lo = self.keys.partition_point(|k| k.0 < r0);
        let hi = self.keys.partition_point(|k| k.0 <= r1);
        (lo, hi.max(lo))
    }
}

pub(crate) struct FormulaCell {
    pub sheet: u32,
    pub row: u32,
    pub col: u32,
    pub text: String,
    pub expr: Option<Rc<Expr>>,
    pub parse_error: Option<String>,
    /// 배열 수식의 크기 (행, 열)
    pub array: Option<(u32, u32)>,
    pub state: State,
    pub cached: Value,
    /// 배열 수식 범위 칸마다 파일에 있던 값 (행 우선, 기준 셀 포함)
    pub array_cached: Vec<Value>,
    pub volatile: bool,
    pub unsupported: bool,
    pub cycle: bool,
    pub data_table: bool,
    /// 데이터 표 입력: (2변수, 행 입력, r1 셀, r2 셀)
    pub dt: Option<DataTableSpec>,
}

/// (입력 셀, 값을 가져올 셀) — 둘 다 (행, 열)
type InputFrom = ((u32, u32), (u32, u32));

/// 데이터 표(What-If)의 입력. 셀은 (행, 열), 같은 시트
#[derive(Debug, Clone, Copy)]
pub(crate) struct DataTableSpec {
    pub two_d: bool,
    pub row_input: bool,
    pub r1: Option<(u32, u32)>,
    pub r2: Option<(u32, u32)>,
}

/// 표 (구조적 참조용)
struct TableDef {
    sheet: u32,
    /// 접은 표 이름·표시 이름
    names: [String; 2],
    r0: u32,
    c0: u32,
    r1: u32,
    c1: u32,
    header_rows: u32,
    totals_rows: u32,
    /// 접은 열 이름 (차례대로)
    cols: Vec<String>,
}

struct NameDef {
    folded: String,
    scope: Option<u32>,
    expr: Rc<Expr>,
}

struct ExtSheet {
    folded: String,
    cells: HashMap<(u32, u32), Value>,
    cached: bool,
}

#[derive(Default, Clone, Copy)]
struct Flags {
    volatile: bool,
    unsupported: bool,
}

pub struct Engine {
    pub(crate) date1904: bool,
    pub(crate) now: f64,
    /// 문서 전체에서 값이 있는 마지막 행·열 + 1 (열·행 전체 참조를 배열로 바꿀 때)
    pub(crate) used_rows: u32,
    pub(crate) used_cols: u32,
    pub(crate) sheets: Vec<SheetData>,
    sheet_index: HashMap<String, u32>,
    names: Vec<NameDef>,
    tables: Vec<TableDef>,
    externals: Vec<Vec<ExtSheet>>,
    pub(crate) formulas: Vec<FormulaCell>,
    depth: u32,
    abort: Option<u32>,
    flags: Flags,
    last_unsupported: Option<String>,
    pub(crate) unsupported: BTreeMap<String, usize>,
    rng: u64,
    /// 셀 서식 번호 → 표시 형식 코드 (CELL("format"), TEXT 비교)
    pub(crate) xf_codes: Vec<String>,
    pub(crate) sheet_names: Vec<String>,
    /// 한셀이 모르는 Excel 함수도 계산한다 ([`crate::calc::CalcOptions::excel_functions`])
    pub(crate) excel_functions: bool,
    /// 한셀의 잘못된 계산도 흉내 낸다 ([`crate::calc::CalcOptions::hcell_bugs`])
    pub(crate) hcell_bugs: bool,
    /// 문서 파일 이름 (CELL("address") 가 다른 시트를 가리킬 때 `[이름]` 으로 쓴다)
    pub(crate) file_name: Option<String>,
    /// 반복 계산 횟수 (`calcPr iterate="1"` 이면 iterateCount, 없으면 100). None 은 반복 계산 꺼짐
    pub(crate) iterate: Option<u32>,
}

impl Engine {
    pub fn new(wb: &Workbook, now: f64) -> Engine {
        let date1904 = wb.date1904;
        // 연도 없는 날짜 글(`1/15`)은 계산 시각의 해
        crate::calc::date::set_this_year(
            crate::calc::date::ymd_from_serial(now, date1904).map(|d| d.0),
        );
        let mut sheets = Vec::with_capacity(wb.sheets.len());
        let mut formulas = Vec::new();
        let mut sheet_index = HashMap::new();
        let mut used_rows = 0u32;
        let mut used_cols = 0u32;
        for (si, s) in wb.sheets.iter().enumerate() {
            sheet_index.entry(value::fold(&s.name)).or_insert(si as u32);
            // 배열 수식이 차지하는 칸을 미리 만들어 둔다
            let mut extra: Vec<(u32, u32)> = Vec::new();
            for c in &s.cells {
                if let Some(f) = &c.formula
                    && matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable)
                    && let Some(rg) = f.range.as_deref().and_then(crate::model::CellRange::parse)
                {
                    for r in rg.start.row..=rg.end.row {
                        for col in rg.start.col..=rg.end.col {
                            extra.push((r, col));
                        }
                    }
                }
            }
            let mut keys: Vec<(u32, u32)> = s.cells.iter().map(|c| (c.r.row, c.r.col)).collect();
            let mut values: Vec<Value> = s
                .cells
                .iter()
                .map(|c| Value::from_cell(&c.value, date1904))
                .collect();
            let mut styles: Vec<u32> = s.cells.iter().map(|c| c.style).collect();
            let sorted = keys.windows(2).all(|w| w[0] < w[1]);
            let mut order: Vec<usize> = (0..keys.len()).collect();
            if !sorted {
                order.sort_by_key(|&i| keys[i]);
                keys = order.iter().map(|&i| keys[i]).collect();
                values = order.iter().map(|&i| values[i].clone()).collect();
                styles = order.iter().map(|&i| styles[i]).collect();
            }
            let mut fid = vec![NO_FORMULA; keys.len()];
            for (k, &ci) in order.iter().enumerate() {
                let c = &s.cells[ci];
                let Some(f) = &c.formula else { continue };
                if f.text.is_empty() && f.kind != FormulaKind::DataTable {
                    continue;
                }
                let (expr, parse_error) = if f.kind == FormulaKind::DataTable {
                    (None, None)
                } else {
                    match parser::parse(&f.text) {
                        Ok(e) => (Some(Rc::new(e)), None),
                        Err(e) => (None, Some(e.to_string())),
                    }
                };
                let array = if matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable) {
                    f.range
                        .as_deref()
                        .and_then(crate::model::CellRange::parse)
                        .map(|r| (r.end.row - r.start.row + 1, r.end.col - r.start.col + 1))
                } else {
                    None
                };
                fid[k] = formulas.len() as u32;
                formulas.push(FormulaCell {
                    sheet: si as u32,
                    row: c.r.row,
                    col: c.r.col,
                    text: f.text.clone(),
                    expr,
                    parse_error,
                    array,
                    state: State::Pending,
                    cached: values[k].clone(),
                    array_cached: Vec::new(),
                    volatile: false,
                    unsupported: false,
                    cycle: false,
                    data_table: f.kind == FormulaKind::DataTable,
                    dt: f.data_table.as_ref().map(|d| {
                        let cell = |s: &Option<String>| {
                            s.as_deref()
                                .and_then(|x| {
                                    crate::model::CellRef::parse(x.replace('$', "").as_str())
                                })
                                .map(|r| (r.row, r.col))
                        };
                        DataTableSpec {
                            two_d: d.two_d,
                            row_input: d.row_input,
                            r1: cell(&d.r1),
                            r2: cell(&d.r2),
                        }
                    }),
                });
            }
            // 배열 수식 칸: 없는 칸은 끼워 넣고, 기준 셀의 수식 번호를 가리킨다
            let mut sd = SheetData {
                keys,
                values,
                fid,
                styles,
                formula_pos: Vec::new(),
                hidden_rows: s
                    .rows
                    .iter()
                    .filter(|r| r.hidden)
                    .map(|r| r.index - 1)
                    .collect(),
                filter_mode: s.filter_mode || s.auto_filter.is_some(),
            };
            if !extra.is_empty() {
                extra.sort_unstable();
                extra.dedup();
                for (r, c) in extra {
                    if sd.find(r, c).is_none() {
                        let at = sd.keys.partition_point(|k| *k < (r, c));
                        sd.keys.insert(at, (r, c));
                        sd.values.insert(at, Value::Empty);
                        sd.fid.insert(at, NO_FORMULA);
                        sd.styles.insert(at, 0);
                    }
                }
                for (id, f) in formulas.iter_mut().enumerate() {
                    if f.sheet != si as u32 {
                        continue;
                    }
                    if let Some((h, w)) = f.array {
                        for r in f.row..f.row + h {
                            for c in f.col..f.col + w {
                                if let Some(i) = sd.find(r, c) {
                                    sd.fid[i] = id as u32;
                                    f.array_cached.push(sd.values[i].clone());
                                } else {
                                    f.array_cached.push(Value::Empty);
                                }
                            }
                        }
                    }
                }
            }
            for (i, &(r, c)) in sd.keys.iter().enumerate() {
                if !sd.values[i].is_empty() || sd.fid[i] != NO_FORMULA {
                    used_rows = used_rows.max(r + 1);
                    used_cols = used_cols.max(c + 1);
                }
                if sd.fid[i] != NO_FORMULA {
                    sd.formula_pos.push((r, c, sd.fid[i]));
                }
            }
            sheets.push(sd);
        }
        let mut tables = Vec::new();
        for (si, s) in wb.sheets.iter().enumerate() {
            for t in &s.tables {
                let Some(rg) = crate::model::CellRange::parse(&t.range) else {
                    continue;
                };
                tables.push(TableDef {
                    sheet: si as u32,
                    names: [value::fold(&t.name), value::fold(&t.display_name)],
                    r0: rg.start.row,
                    c0: rg.start.col,
                    r1: rg.end.row,
                    c1: rg.end.col,
                    header_rows: t.header_rows,
                    totals_rows: t.totals_rows,
                    cols: t.columns.iter().map(|c| value::fold(&c.name)).collect(),
                });
            }
        }
        let mut names = Vec::new();
        for d in &wb.defined_names {
            if d.formula.trim().is_empty() {
                continue;
            }
            let expr = match parser::parse(&d.formula) {
                Ok(e) => e,
                Err(_) => Expr::Error(ErrorKind::Name),
            };
            names.push(NameDef {
                folded: value::fold(&d.name),
                scope: d.local_sheet_id,
                expr: Rc::new(expr),
            });
        }
        let externals = wb
            .external_links
            .iter()
            .map(|l| {
                l.sheet_names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let data = l.sheet_data.iter().find(|d| d.sheet_id as usize == i);
                        ExtSheet {
                            folded: value::fold(n),
                            cells: data
                                .map(|d| {
                                    d.cells
                                        .iter()
                                        .map(|(r, v)| {
                                            ((r.row, r.col), Value::from_cell(v, date1904))
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            cached: data.is_some_and(|d| !d.refresh_error),
                        }
                    })
                    .collect()
            })
            .collect();
        let xf_codes = wb
            .styles
            .cell_xfs
            .iter()
            .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
            .collect();
        let seed = match (crate::calc::unix_now() * 1e9) as u64 {
            0 => 0x9E37_79B9_7F4A_7C15,
            n => n,
        } | 1;
        Engine {
            date1904,
            now,
            used_rows,
            used_cols,
            sheets,
            sheet_index,
            names,
            tables,
            externals,
            formulas,
            depth: 0,
            abort: None,
            flags: Flags::default(),
            last_unsupported: None,
            unsupported: BTreeMap::new(),
            rng: seed,
            xf_codes,
            sheet_names: wb.sheets.iter().map(|s| s.name.clone()).collect(),
            excel_functions: false,
            hcell_bugs: false,
            file_name: None,
            iterate: iterate_count(&wb.calc_attrs),
        }
    }

    // -----------------------------------------------------------------------
    // 참조 해석
    // -----------------------------------------------------------------------

    pub fn sheet_count(&self) -> u32 {
        self.sheets.len() as u32
    }

    pub fn sheet_index(&self, name: &str) -> Option<u32> {
        self.sheet_index.get(&value::fold(name)).copied()
    }

    /// (문서, 첫 시트, 마지막 시트)
    pub fn resolve_sheets(
        &self,
        book: Option<u32>,
        spec: &SheetSpec,
        current: u32,
    ) -> Result<(u32, u32, u32), ErrorKind> {
        match book {
            None => match spec {
                SheetSpec::Current => Ok((0, current, current)),
                SheetSpec::One(n) => {
                    let i = self.sheet_index(n).ok_or(ErrorKind::Ref)?;
                    Ok((0, i, i))
                }
                SheetSpec::Span(a, b) => {
                    let i = self.sheet_index(a).ok_or(ErrorKind::Ref)?;
                    let j = self.sheet_index(b).ok_or(ErrorKind::Ref)?;
                    // 거꾸로 쓴 3차원 참조('S3:S1'!A1)는 바꾸지 않는다 — 한셀은 빈 범위로 계산한다 (SUM = 0)
                    Ok((0, i, j))
                }
            },
            Some(0) => Err(ErrorKind::Ref),
            Some(n) => {
                let ext = self.externals.get(n as usize - 1).ok_or(ErrorKind::Ref)?;
                let name = match spec {
                    SheetSpec::One(s) | SheetSpec::Span(s, _) => value::fold(s),
                    SheetSpec::Current => return Err(ErrorKind::Ref),
                };
                let i = ext
                    .iter()
                    .position(|s| s.folded == name)
                    .ok_or(ErrorKind::Ref)? as u32;
                Ok((n, i, i))
            }
        }
    }

    /// 이름의 식: 시트 범위 이름 → 통합 문서 이름 순
    pub fn name_expr(&self, name: &str, scope: Option<u32>, current: u32) -> Option<Rc<Expr>> {
        let f = value::fold(name);
        let want = scope.unwrap_or(current);
        self.names
            .iter()
            .find(|n| n.folded == f && n.scope == Some(want))
            .or_else(|| {
                self.names
                    .iter()
                    .find(|n| n.folded == f && n.scope.is_none())
            })
            .map(|n| n.expr.clone())
            // 이름이 없으면 표 이름: 표의 데이터 행 (`ROWS(Table1)`)
            .or_else(|| {
                self.tables
                    .iter()
                    .any(|t| t.names[0] == f || t.names[1] == f)
                    .then(|| {
                        Rc::new(Expr::Table(crate::calc::ast::TableRef {
                            table: name.to_string(),
                            ..Default::default()
                        }))
                    })
            })
    }

    // -----------------------------------------------------------------------
    // 셀 값
    // -----------------------------------------------------------------------

    pub fn cell_value(&mut self, book: u32, sheet: u32, row: u32, col: u32) -> Value {
        if book != 0 {
            let Some(ext) = self.externals.get(book as usize - 1) else {
                return Value::Error(ErrorKind::Ref);
            };
            let Some(s) = ext.get(sheet as usize) else {
                return Value::Error(ErrorKind::Ref);
            };
            if !s.cached {
                return Value::Error(ErrorKind::Ref);
            }
            return s.cells.get(&(row, col)).cloned().unwrap_or(Value::Empty);
        }
        let Some(sd) = self.sheets.get(sheet as usize) else {
            return Value::Error(ErrorKind::Ref);
        };
        let Some(i) = sd.find(row, col) else {
            return Value::Empty;
        };
        let fid = sd.fid[i];
        if fid != NO_FORMULA {
            self.demand(fid);
            self.inherit(fid);
        }
        self.sheets[sheet as usize].values[i].clone()
    }

    /// 셀이 수식이면 그 번호
    pub fn formula_at(&self, sheet: u32, row: u32, col: u32) -> Option<u32> {
        let sd = self.sheets.get(sheet as usize)?;
        let i = sd.find(row, col)?;
        (sd.fid[i] != NO_FORMULA).then_some(sd.fid[i])
    }

    /// 셀 수식이 부분합 함수(SUBTOTAL·SUBTOTALA·AGGREGATE)를 쓰는가 — 부분합끼리는 겹쳐 세지 않는다
    pub fn cell_has_subtotal(&self, sheet: u32, row: u32, col: u32) -> bool {
        let Some(fid) = self.formula_at(sheet, row, col) else {
            return false;
        };
        let Some(e) = &self.formulas[fid as usize].expr else {
            return false;
        };
        let mut hit = false;
        e.visit_calls(&mut |c| {
            if matches!(c.name.as_str(), "SUBTOTAL" | "SUBTOTALA" | "AGGREGATE") {
                hit = true;
            }
        });
        hit
    }

    pub fn cell_style(&self, sheet: u32, row: u32, col: u32) -> u32 {
        self.sheets
            .get(sheet as usize)
            .and_then(|sd| sd.find(row, col).map(|i| sd.styles[i]))
            .unwrap_or(0)
    }

    pub fn row_hidden(&self, sheet: u32, row: u32) -> bool {
        self.sheets
            .get(sheet as usize)
            .is_some_and(|s| s.hidden_rows.contains(&row))
    }

    pub fn sheet_filtered(&self, sheet: u32) -> bool {
        self.sheets
            .get(sheet as usize)
            .is_some_and(|s| s.filter_mode)
    }

    /// 영역 안의 값이 있는 셀마다 (행 우선). 수식 셀은 먼저 계산한다.
    pub fn for_each_in_area(&mut self, a: &Area, mut f: impl FnMut(u32, u32, &Value)) {
        if a.book != 0 {
            let Some(ext) = self.externals.get(a.book as usize - 1) else {
                return;
            };
            let Some(s) = ext.get(a.sheet as usize) else {
                return;
            };
            let mut cells: Vec<(&(u32, u32), &Value)> = s
                .cells
                .iter()
                .filter(|((r, c), _)| a.r0 <= *r && *r <= a.r1 && a.c0 <= *c && *c <= a.c1)
                .collect();
            cells.sort_by_key(|(k, _)| **k);
            for ((r, c), v) in cells {
                if !v.is_empty() {
                    f(*r, *c, v);
                }
            }
            return;
        }
        for s in a.sheet..=a.sheet_last {
            let Some(sd) = self.sheets.get(s as usize) else {
                continue;
            };
            let (lo, hi) = sd.row_span(a.r0, a.r1);
            let pending: Vec<u32> = (lo..hi)
                .filter(|&i| {
                    let (_, c) = sd.keys[i];
                    a.c0 <= c && c <= a.c1 && sd.fid[i] != NO_FORMULA
                })
                .map(|i| sd.fid[i])
                .collect();
            for fid in pending {
                self.demand(fid);
                self.inherit(fid);
            }
            let sd = &self.sheets[s as usize];
            for i in lo..hi {
                let (r, c) = sd.keys[i];
                if a.c0 <= c && c <= a.c1 {
                    let v = &sd.values[i];
                    if !v.is_empty() {
                        f(r, c, v);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 계산
    // -----------------------------------------------------------------------

    pub(crate) fn mark_volatile(&mut self) {
        self.flags.volatile = true;
    }

    pub(crate) fn mark_unsupported(&mut self, name: &str) {
        self.flags.unsupported = true;
        self.last_unsupported = Some(name.to_string());
    }

    /// 다른 수식 셀을 읽었을 때 그 셀의 성질을 물려받는다
    fn inherit(&mut self, fid: u32) {
        let f = &self.formulas[fid as usize];
        if f.volatile {
            self.flags.volatile = true;
        }
    }

    /// 0 이상 1 미만 난수
    pub fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// 수식 셀의 값이 필요하다
    fn demand(&mut self, fid: u32) {
        let st = self.formulas[fid as usize].state;
        match st {
            State::Done => {}
            State::InProgress => {
                self.formulas[fid as usize].cycle = true;
            }
            State::Pending => {
                if self.abort.is_some() || self.formulas[fid as usize].cycle {
                    return;
                }
                if self.depth >= MAX_DEPTH {
                    self.abort = Some(fid);
                    return;
                }
                self.compute(fid);
            }
        }
    }

    /// 수식 하나를 계산해 셀에 넣는다. 깊이 한도로 멈추면 Pending 으로 남는다.
    fn compute(&mut self, fid: u32) {
        let (sheet, row, col, array, expr, data_table) = {
            let f = &mut self.formulas[fid as usize];
            f.state = State::InProgress;
            (f.sheet, f.row, f.col, f.array, f.expr.clone(), f.data_table)
        };
        let outer = std::mem::take(&mut self.flags);
        let outer_unsup = self.last_unsupported.take();
        self.depth += 1;
        let v = match (&expr, data_table) {
            (Some(e), false) => {
                let mut ctx = Ctx::new(self, sheet, row, col, array.is_some());
                ctx.eval_top(e)
            }
            _ => Value::Error(ErrorKind::Value),
        };
        self.depth -= 1;
        let mine = std::mem::replace(&mut self.flags, outer);
        let unsup_name = std::mem::replace(&mut self.last_unsupported, outer_unsup);
        if self.abort.is_some() {
            self.formulas[fid as usize].state = State::Pending;
            return;
        }
        let f = &mut self.formulas[fid as usize];
        f.volatile = mine.volatile;
        f.unsupported = mine.unsupported || expr.is_none() || data_table;
        f.state = State::Done;
        if f.unsupported
            && let Some(n) = unsup_name
        {
            *self.unsupported.entry(n).or_default() += 1;
        }
        let keep_cached = {
            let f = &self.formulas[fid as usize];
            f.unsupported && !f.cached.is_empty()
        };
        let sd = &mut self.sheets[sheet as usize];
        match array {
            Some((h, w)) => {
                let arr = match &v {
                    Value::Array(a) => Some(a.clone()),
                    _ => None,
                };
                for r in 0..h {
                    for c in 0..w {
                        if let Some(i) = sd.find(row + r, col + c) {
                            if keep_cached {
                                continue;
                            }
                            let x = match &arr {
                                Some(a) => a.get_broadcast(r as usize, c as usize),
                                None => v.clone(),
                            };
                            sd.values[i] = match x {
                                Value::Empty => Value::Number(0.0),
                                o => o,
                            };
                        }
                    }
                }
            }
            None => {
                if let Some(i) = sd.find(row, col)
                    && !keep_cached
                {
                    sd.values[i] = v;
                }
            }
        }
    }

    /// 수식 하나를 (필요한 선행 셀부터) 끝까지 계산한다
    pub(crate) fn run(&mut self, root: u32) {
        let mut stack = vec![root];
        let mut guard = 0usize;
        while let Some(&top) = stack.last() {
            guard += 1;
            if guard > 50_000_000 {
                break;
            }
            if self.formulas[top as usize].state == State::Done {
                stack.pop();
                continue;
            }
            self.compute(top);
            match self.abort.take() {
                Some(dep) => {
                    if stack.contains(&dep) || self.formulas[dep as usize].state != State::Pending {
                        // 계산 중에 드러난 순환: 그 셀은 지금 값을 쓴다
                        self.formulas[dep as usize].cycle = true;
                    } else {
                        stack.push(dep);
                    }
                }
                None => {
                    stack.pop();
                }
            }
        }
    }

    /// 모든 수식을 의존 순서대로 계산한다. 데이터 표는 끝에 따로 계산한다.
    pub fn recalc_all(&mut self) {
        let order = self.topo_order();
        for fid in order {
            self.run(fid);
        }
        if let Some(n) = self.iterate {
            self.iterate_cycles(n);
        }
        let tables: Vec<u32> = (0..self.formulas.len() as u32)
            .filter(|&i| self.formulas[i as usize].dt.is_some())
            .collect();
        for fid in tables {
            self.compute_data_table(fid);
        }
    }

    /// 데이터 표 (What-If 분석): 결과 칸마다 입력 셀에 머리 행·열의 값을 넣고 가장자리 수식을 다시 계산한다.
    /// 1변수 열 입력은 위 행의 수식과 왼쪽 열의 값, 행 입력은 왼쪽 열의 수식과 위 행의 값, 2변수는 왼쪽 위
    /// 모서리의 수식과 위 행(행 입력 r1)·왼쪽 열(열 입력 r2)의 값.
    fn compute_data_table(&mut self, fid: u32) {
        let (sheet, row0, col0, (h, w), spec) = {
            let f = &self.formulas[fid as usize];
            match (f.array, f.dt) {
                (Some(a), Some(d)) => (f.sheet, f.row, f.col, a, d),
                _ => return,
            }
        };
        if row0 == 0 || col0 == 0 {
            return;
        }
        let inputs: Vec<(u32, u32)> = [spec.r1, spec.r2].into_iter().flatten().collect();
        if inputs.is_empty() {
            return;
        }
        let affected = self.dependents_of(sheet, &inputs);
        let mut results = Vec::with_capacity((h * w) as usize);
        for r in row0..row0 + h {
            for c in col0..col0 + w {
                let (edge, sets): ((u32, u32), Vec<InputFrom>) = if spec.two_d {
                    let mut s = Vec::new();
                    if let Some(i) = spec.r1 {
                        s.push((i, (row0 - 1, c)));
                    }
                    if let Some(i) = spec.r2 {
                        s.push((i, (r, col0 - 1)));
                    }
                    ((row0 - 1, col0 - 1), s)
                } else if spec.row_input {
                    (
                        (r, col0 - 1),
                        spec.r1
                            .map(|i| vec![(i, (row0 - 1, c))])
                            .unwrap_or_default(),
                    )
                } else {
                    (
                        (row0 - 1, c),
                        spec.r1
                            .map(|i| vec![(i, (r, col0 - 1))])
                            .unwrap_or_default(),
                    )
                };
                results.push(((r, c), self.what_if(sheet, edge, &sets, &affected)));
            }
        }
        let sd = &mut self.sheets[sheet as usize];
        for ((r, c), v) in results {
            if let Some(i) = sd.find(r, c) {
                sd.values[i] = match v {
                    Value::Empty => Value::Number(0.0),
                    o => o,
                };
            }
        }
        let f = &mut self.formulas[fid as usize];
        f.state = State::Done;
        f.unsupported = false;
    }

    /// 반복 계산: 순환에 든 수식과 그것에 (정적 참조로) 기대는 수식을 파일 값에서 다시 시작해 2×횟수 번,
    /// 시트·행·열 차례로 계산한다. 다른 칸은 그때의 값을 읽는다 — 한셀이 그렇다 (대조 실험 circ: iterateCount
    /// 1·2·3·10 에서 `B3 = B3+1` 이 파일 값 + 2·4·6·20, 뒤 행의 순환 칸을 읽는 앞 행 수식은 한 바퀴 앞 값)
    fn iterate_cycles(&mut self, count: u32) {
        let cyc: Vec<u32> = (0..self.formulas.len() as u32)
            .filter(|&i| self.formulas[i as usize].cycle)
            .collect();
        if cyc.is_empty() {
            return;
        }
        let cells: Vec<(u32, u32, u32)> = cyc
            .iter()
            .map(|&i| {
                let f = &self.formulas[i as usize];
                (f.sheet, f.row, f.col)
            })
            .collect();
        let mut set = self.dependents_of_cells(cells);
        set.extend(cyc);
        set.retain(|&i| {
            let f = &self.formulas[i as usize];
            f.array.is_none() && f.dt.is_none()
        });
        set.sort_by_key(|&i| {
            let f = &self.formulas[i as usize];
            (f.sheet, f.row, f.col)
        });
        set.dedup();
        for &i in &set {
            let (sheet, row, col, cached) = {
                let f = &mut self.formulas[i as usize];
                f.state = State::Done;
                (f.sheet, f.row, f.col, f.cached.clone())
            };
            let sd = &mut self.sheets[sheet as usize];
            if let Some(k) = sd.find(row, col) {
                sd.values[k] = cached;
            }
        }
        for _ in 0..count.saturating_mul(2) {
            for &i in &set {
                self.compute(i);
            }
        }
    }

    /// 입력 셀들에 (직접·간접으로) 기대는 수식 번호
    fn dependents_of(&self, sheet: u32, inputs: &[(u32, u32)]) -> Vec<u32> {
        self.dependents_of_cells(inputs.iter().map(|&(r, c)| (sheet, r, c)).collect())
    }

    /// 칸들(시트, 행, 열)에 (직접·간접으로) 기대는 수식 번호
    fn dependents_of_cells(&self, mut cells: Vec<(u32, u32, u32)>) -> Vec<u32> {
        let areas: Vec<Vec<Area>> = self
            .formulas
            .iter()
            .map(|f| match &f.expr {
                Some(e) => self.static_areas(e, f.sheet, f.row, f.col),
                None => Vec::new(),
            })
            .collect();
        let hits = |a: &Area, s: u32, r: u32, c: u32| {
            a.book == 0
                && (a.sheet..=a.sheet_last).contains(&s)
                && (a.r0..=a.r1).contains(&r)
                && (a.c0..=a.c1).contains(&c)
        };
        let mut on = vec![false; self.formulas.len()];
        let mut changed = true;
        while changed {
            changed = false;
            for (i, list) in areas.iter().enumerate() {
                if on[i] || self.formulas[i].dt.is_some() {
                    continue;
                }
                if list
                    .iter()
                    .any(|a| cells.iter().any(|&(s, r, c)| hits(a, s, r, c)))
                {
                    on[i] = true;
                    changed = true;
                    let f = &self.formulas[i];
                    let (h, w) = f.array.unwrap_or((1, 1));
                    for r in f.row..f.row + h {
                        for c in f.col..f.col + w {
                            cells.push((f.sheet, r, c));
                        }
                    }
                }
            }
        }
        (0..self.formulas.len() as u32)
            .filter(|&i| on[i as usize])
            .collect()
    }

    /// 입력 셀 값을 잠시 바꾸고 가장자리 칸을 다시 계산한 값 (끝나면 모두 되돌린다)
    fn what_if(
        &mut self,
        sheet: u32,
        edge: (u32, u32),
        sets: &[InputFrom],
        affected: &[u32],
    ) -> Value {
        // 넣을 값을 먼저 읽는다
        let vals: Vec<((u32, u32), Value)> = sets
            .iter()
            .map(|&(input, from)| (input, self.cell_value(0, sheet, from.0, from.1)))
            .collect();
        // 되돌릴 것: 입력 셀 값, 영향받는 수식의 상태와 칸 값
        let mut saved_cells: Vec<(u32, usize, Value)> = Vec::new();
        let mut saved_states: Vec<(u32, State)> = Vec::new();
        for &(input, ref v) in &vals {
            let sd = &mut self.sheets[sheet as usize];
            if let Some(i) = sd.find(input.0, input.1) {
                saved_cells.push((sheet, i, sd.values[i].clone()));
                sd.values[i] = v.clone();
            }
        }
        for &g in affected {
            let f = &self.formulas[g as usize];
            saved_states.push((g, f.state));
            let (s, r0, c0, (h, w)) = (f.sheet, f.row, f.col, f.array.unwrap_or((1, 1)));
            for r in r0..r0 + h {
                for c in c0..c0 + w {
                    if let Some(i) = self.sheets[s as usize].find(r, c) {
                        saved_cells.push((s, i, self.sheets[s as usize].values[i].clone()));
                    }
                }
            }
            self.formulas[g as usize].state = State::Pending;
        }
        if let Some(e) = self.formula_at(sheet, edge.0, edge.1) {
            self.run(e);
        }
        let out = self.cell_value(0, sheet, edge.0, edge.1);
        for (s, i, v) in saved_cells.into_iter().rev() {
            self.sheets[s as usize].values[i] = v;
        }
        for (g, st) in saved_states {
            self.formulas[g as usize].state = st;
        }
        out
    }

    /// 셀 하나만 계산한다 (필요한 선행 셀 포함)
    pub fn evaluate_one(&mut self, sheet: u32, row: u32, col: u32) -> Value {
        if let Some(fid) = self.formula_at(sheet, row, col) {
            self.run(fid);
        }
        self.cell_value(0, sheet, row, col)
    }

    /// 임의의 수식을 셀 위치에서 계산한다 (셀에 넣지 않는다)
    pub fn evaluate_expr(&mut self, e: &Expr, sheet: u32, row: u32, col: u32) -> Value {
        // 식이 참조하는 수식 셀을 먼저 계산
        for area in self.static_areas(e, sheet, row, col) {
            let ids: Vec<u32> = self.formulas_in(&area);
            for id in ids {
                self.run(id);
            }
        }
        let mut ctx = Ctx::new(self, sheet, row, col, false);
        ctx.eval_top(e)
    }

    // -----------------------------------------------------------------------
    // 의존 그래프
    // -----------------------------------------------------------------------

    /// 구조적 참조의 영역. 행 종류: 없으면 데이터, #All 전체, #Headers·#Data·#Totals 는 이어 붙이고,
    /// #This Row 는 수식이 있는 행 (데이터 행 밖이면 #VALUE!). 없는 표·열·행 종류는 #REF!
    pub(crate) fn resolve_table(
        &self,
        t: &crate::calc::ast::TableRef,
        sheet: u32,
        row: u32,
        col: u32,
    ) -> Result<Area, ErrorKind> {
        let def = if t.table.is_empty() {
            self.tables.iter().find(|d| {
                d.sheet == sheet && (d.r0..=d.r1).contains(&row) && (d.c0..=d.c1).contains(&col)
            })
        } else {
            let k = value::fold(&t.table);
            self.tables
                .iter()
                .find(|d| d.names[0] == k || d.names[1] == k)
        };
        let d = def.ok_or(ErrorKind::Ref)?;
        let data0 = d.r0 + d.header_rows;
        let data1 = d.r1.checked_sub(d.totals_rows).ok_or(ErrorKind::Ref)?;
        let (r0, r1) = if t.all {
            (d.r0, d.r1)
        } else if t.this_row {
            if row < data0 || row > data1 {
                return Err(ErrorKind::Value);
            }
            (row, row)
        } else {
            let mut parts: Vec<(u32, u32)> = Vec::new();
            let any = t.headers || t.data || t.totals;
            if t.headers {
                if d.header_rows == 0 {
                    return Err(ErrorKind::Ref);
                }
                parts.push((d.r0, data0 - 1));
            }
            if t.data || !any {
                if data0 > data1 {
                    return Err(ErrorKind::Ref);
                }
                parts.push((data0, data1));
            }
            if t.totals {
                if d.totals_rows == 0 {
                    return Err(ErrorKind::Ref);
                }
                parts.push((data1 + 1, d.r1));
            }
            let lo = parts.iter().map(|p| p.0).min().ok_or(ErrorKind::Ref)?;
            let hi = parts.iter().map(|p| p.1).max().ok_or(ErrorKind::Ref)?;
            (lo, hi)
        };
        let (c0, c1) = match &t.cols {
            None => (d.c0, d.c1),
            Some((a, b)) => {
                let find = |n: &str| {
                    let k = value::fold(n);
                    d.cols.iter().position(|c| *c == k).map(|i| d.c0 + i as u32)
                };
                let (x, y) = (
                    find(a).ok_or(ErrorKind::Ref)?,
                    find(b).ok_or(ErrorKind::Ref)?,
                );
                (x.min(y), x.max(y))
            }
        };
        Ok(Area {
            book: 0,
            sheet: d.sheet,
            sheet_last: d.sheet,
            r0,
            c0,
            r1,
            c1,
        })
    }

    /// 식의 정적 참조 영역 (이 문서 안, 이름을 따라간다)
    fn static_areas(&self, e: &Expr, sheet: u32, row: u32, col: u32) -> Vec<Area> {
        let mut out = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        self.collect_areas(e, (sheet, row, col), &mut out, &mut seen_names, 0);
        out
    }

    fn collect_areas(
        &self,
        e: &Expr,
        at: (u32, u32, u32),
        out: &mut Vec<Area>,
        seen: &mut HashSet<String>,
        depth: u32,
    ) {
        let (sheet, row, col) = at;
        if depth > 16 {
            return;
        }
        e.visit_refs(&mut |r| match r {
            RefOrName::Ref(r) => {
                if let Ok((0, s0, s1)) = self.resolve_sheets(r.book, &r.sheet, sheet) {
                    out.push(Area {
                        book: 0,
                        sheet: s0,
                        sheet_last: s1,
                        r0: r.r0,
                        c0: r.c0,
                        r1: r.r1,
                        c1: r.c1,
                    });
                }
            }
            RefOrName::Name(n) => {
                if n.book.is_some() {
                    return;
                }
                let scope = n.sheet.as_ref().and_then(|s| self.sheet_index(s));
                let key = format!("{:?}/{}", scope, value::fold(&n.name));
                if !seen.insert(key) {
                    return;
                }
                if let Some(x) = self.name_expr(&n.name, scope, sheet) {
                    self.collect_areas(&x, at, out, seen, depth + 1);
                }
            }
            RefOrName::Table(t) => {
                if let Ok(a) = self.resolve_table(t, sheet, row, col) {
                    out.push(a);
                }
            }
        });
    }

    /// 영역 안의 수식 번호
    fn formulas_in(&self, a: &Area) -> Vec<u32> {
        let mut out = Vec::new();
        if a.book != 0 {
            return out;
        }
        for s in a.sheet..=a.sheet_last {
            let Some(sd) = self.sheets.get(s as usize) else {
                continue;
            };
            let lo = sd.formula_pos.partition_point(|p| p.0 < a.r0);
            for &(r, c, id) in &sd.formula_pos[lo..] {
                if r > a.r1 {
                    break;
                }
                if a.c0 <= c && c <= a.c1 {
                    out.push(id);
                }
            }
        }
        out
    }

    /// 선행 셀이 먼저 오는 계산 순서. 정적 순환에 든 수식은 `cycle` 로 표시한다.
    pub(crate) fn topo_order(&mut self) -> Vec<u32> {
        let nf = self.formulas.len();
        let mut area_ids: HashMap<Area, usize> = HashMap::new();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); nf];
        let mut area_list: Vec<Area> = Vec::new();
        for (fid, dep) in deps.iter_mut().enumerate() {
            let f = &self.formulas[fid];
            let Some(e) = f.expr.clone() else {
                continue;
            };
            for a in self.static_areas(&e, f.sheet, f.row, f.col) {
                let id = *area_ids.entry(a).or_insert_with(|| {
                    area_list.push(a);
                    nf + area_list.len() - 1
                });
                dep.push(id);
            }
        }
        for a in &area_list {
            let ids = self.formulas_in(a);
            deps.push(ids.into_iter().map(|x| x as usize).collect());
        }
        let n = deps.len();
        let mut color = vec![0u8; n];
        let mut order = Vec::with_capacity(nf);
        let mut cyclic: Vec<usize> = Vec::new();
        for start in 0..nf {
            if color[start] != 0 {
                continue;
            }
            let mut stack: Vec<(usize, usize)> = vec![(start, 0)];
            color[start] = 1;
            while let Some(top) = stack.last_mut() {
                let (node, i) = *top;
                if i < deps[node].len() {
                    top.1 += 1;
                    let next = deps[node][i];
                    match color[next] {
                        0 => {
                            color[next] = 1;
                            stack.push((next, 0));
                        }
                        1 => {
                            // 되돌아가는 간선: 스택의 next..꼭대기가 순환
                            if let Some(p) = stack.iter().position(|&(x, _)| x == next) {
                                cyclic
                                    .extend(stack[p..].iter().map(|&(x, _)| x).filter(|&x| x < nf));
                            }
                        }
                        _ => {}
                    }
                } else {
                    color[node] = 2;
                    stack.pop();
                    if node < nf {
                        order.push(node as u32);
                    }
                }
            }
        }
        for c in cyclic {
            self.formulas[c].cycle = true;
        }
        order
    }
}

/// `calcPr` 의 iterate·iterateCount: 반복 계산이 켜져 있으면 횟수 (iterateCount 가 없으면 100)
fn iterate_count(attrs: &[(String, String)]) -> Option<u32> {
    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    match get("iterate") {
        Some("1") | Some("true") => Some(
            get("iterateCount")
                .and_then(|v| v.parse().ok())
                .unwrap_or(100),
        ),
        _ => None,
    }
}
