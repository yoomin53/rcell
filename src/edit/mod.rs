// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 문서 편집 — 한셀처럼 셀 입력, 지우기, 행·열 삽입·삭제, 서식, 병합, 열 너비·행 높이, 시트 추가·이름
//! 바꾸기·삭제·옮기기, 복사·붙여넣기·잘라 붙이기·채우기.
//!
//! 규칙은 한셀 COM 대조 실험에서 확인한 것이다 (docs/editing.md). 편집 뒤 값은 [`crate::calc::recalculate`]
//! 로 다시 계산한다 (편집 함수는 계산하지 않는다).
//!
//! ```no_run
//! use rcell::edit::{self, Edit};
//! let mut wb = rcell::open("문서.cell")?;
//! edit::set_input(&mut wb, 0, "B2".parse().unwrap(), "=SUM(A1:A3)")?;
//! edit::insert_rows(&mut wb, 0, 4, 2)?;           // 5–6행 삽입 (0부터)
//! let e: Edit = serde_json::from_str(r#"{"op":"format","range":"A1:C1","bold":true}"#).unwrap();
//! edit::apply(&mut wb, &e)?;
//! rcell::calc::recalculate(&mut wb);
//! rcell::save(&wb, "새문서.cell")?;
//! # Ok::<(), rcell::Error>(())
//! ```

pub mod clip;
pub mod input;
pub mod refs;
pub mod structure;
pub mod style;

use std::str::FromStr;

use serde::Deserialize;

pub use clip::{Clip, copy, fill, move_cells, paste};
pub use refs::Shift;
pub use style::{BorderPreset, FormatPatch};

use crate::model::{
    Cell, CellRange, CellRef, CellValue, ColInfo, Formula, FormulaKind, RowInfo, Sheet, Workbook,
};
use input::Input;
use style::Restyler;

fn err(m: impl Into<String>) -> crate::Error {
    crate::Error::Edit(m.into())
}

impl FromStr for CellRef {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<CellRef> {
        CellRef::parse(s).ok_or_else(|| err(format!("셀 주소가 아닙니다: {s}")))
    }
}

/// 범위 글 (`A1:C3`, `A1`, `A:C`, `2:5`) → 범위 (열·행 전체는 시트 끝까지)
pub fn parse_range(s: &str) -> crate::Result<CellRange> {
    let a = refs::parse_area(s).ok_or_else(|| err(format!("범위가 아닙니다: {s}")))?;
    let (r0, c0, r1, c1) = a.bounds();
    Ok(CellRange {
        start: CellRef {
            row: r0.min(r1),
            col: c0.min(c1),
        },
        end: CellRef {
            row: r0.max(r1),
            col: c0.max(c1),
        },
    })
}

/// 열 이름 → 번호 (0부터)
pub fn parse_col(s: &str) -> crate::Result<u32> {
    CellRef::parse(&format!("{}1", s.trim()))
        .map(|r| r.col)
        .ok_or_else(|| err(format!("열 이름이 아닙니다: {s}")))
}

// ---------------------------------------------------------------------------
// 편집 명령 (JSON — `rcell edit` 와 웹 편집기)
// ---------------------------------------------------------------------------

/// 시트 지정: 이름 또는 번호 (0부터)
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SheetSel {
    Index(usize),
    Name(String),
}

fn one() -> u32 {
    1
}

/// 편집 명령 하나. 행은 1부터(한셀 화면과 같다), 열은 이름(`C`), 셀·범위는 A1 표기.
/// `sheet` 를 빼면 첫 시트
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Edit {
    /// 셀에 입력 (한셀 입력 규칙 — 숫자·날짜·백분율·`'글`·논리값·오류·`=수식`)
    Set {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
        input: String,
    },
    /// 내용 지우기 (Delete 키)
    Clear {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 서식 지우기
    ClearFormats {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 모두 지우기 (내용 + 서식 + 병합)
    ClearAll {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    InsertRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    DeleteRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    InsertCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    DeleteCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 글꼴·채우기·맞춤·줄 바꿈·표시 형식
    Format {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(flatten)]
        patch: FormatPatch,
    },
    Border {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        preset: BorderPreset,
        /// `thin`(기본)/`medium`/`thick`/`dashed`/`dotted`/`double`/`hair`
        #[serde(default)]
        style: Option<String>,
        /// `RRGGBB` (없으면 자동)
        #[serde(default)]
        color: Option<String>,
    },
    Merge {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        /// 병합하고 가운데 맞춤
        #[serde(default)]
        center: bool,
    },
    Unmerge {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 열 너비 (한셀 COM `ColumnWidth` 와 같은 글자 수 — 저장 값은 여백을 더한 것)
    ColWidth {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
        width: f64,
    },
    /// 열 너비 (화면 px — 웹 편집기에서 끌어 바꾼 너비)
    ColWidthPx {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
        px: u32,
    },
    /// 행 높이 (pt)
    RowHeight {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
        height: f64,
    },
    AddSheet {
        #[serde(default)]
        name: Option<String>,
        /// 이 시트 뒤에 (없으면 맨 끝)
        #[serde(default)]
        after: Option<SheetSel>,
    },
    RenameSheet {
        sheet: SheetSel,
        name: String,
    },
    DeleteSheet {
        sheet: SheetSel,
    },
    /// 시트를 옮긴다 (`to` = 옮긴 뒤의 자리, 0부터)
    MoveSheet {
        sheet: SheetSel,
        to: usize,
    },
    /// 복사해 붙이기 (`to` 는 붙일 왼쪽 위 칸 또는 채울 범위)
    Copy {
        #[serde(default)]
        sheet: Option<SheetSel>,
        from: String,
        to: String,
        #[serde(default)]
        to_sheet: Option<SheetSel>,
    },
    /// 잘라 붙이기 (셀 옮기기)
    Move {
        #[serde(default)]
        sheet: Option<SheetSel>,
        from: String,
        to: String,
        #[serde(default)]
        to_sheet: Option<SheetSel>,
    },
    FillDown {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    FillRight {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 글 붙여넣기 (탭·줄 바꿈으로 나눈 표 — 칸마다 입력 규칙으로 읽는다)
    PasteText {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
        text: String,
    },
}

/// 편집 대본 (`{"edits": [...]}`)
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Script {
    pub edits: Vec<Edit>,
}

/// 시트 번호
pub fn sheet_index(wb: &Workbook, sel: Option<&SheetSel>) -> crate::Result<usize> {
    match sel {
        None => (!wb.sheets.is_empty())
            .then_some(0)
            .ok_or_else(|| err("시트가 없습니다")),
        Some(SheetSel::Index(i)) => (*i < wb.sheets.len())
            .then_some(*i)
            .ok_or_else(|| err(format!("시트 번호가 없습니다: {i}"))),
        Some(SheetSel::Name(n)) => wb
            .sheets
            .iter()
            .position(|s| refs::same_sheet(&s.name, n))
            .ok_or_else(|| err(format!("시트가 없습니다: {n}"))),
    }
}

/// 편집 하나를 적용한다. 바뀐(또는 새로 만든) 시트 번호를 돌려준다
pub fn apply(wb: &mut Workbook, e: &Edit) -> crate::Result<usize> {
    let si = |wb: &Workbook, s: &Option<SheetSel>| sheet_index(wb, s.as_ref());
    match e {
        Edit::Set { sheet, cell, input } => {
            let i = si(wb, sheet)?;
            set_input(wb, i, cell.parse()?, input)?;
            Ok(i)
        }
        Edit::Clear { sheet, range } => {
            let i = si(wb, sheet)?;
            clear(wb, i, parse_range(range)?, ClearKind::Contents)?;
            Ok(i)
        }
        Edit::ClearFormats { sheet, range } => {
            let i = si(wb, sheet)?;
            clear(wb, i, parse_range(range)?, ClearKind::Formats)?;
            Ok(i)
        }
        Edit::ClearAll { sheet, range } => {
            let i = si(wb, sheet)?;
            clear(wb, i, parse_range(range)?, ClearKind::All)?;
            Ok(i)
        }
        Edit::InsertRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            insert_rows(wb, i, row.saturating_sub(1), *count)?;
            Ok(i)
        }
        Edit::DeleteRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            delete_rows(wb, i, row.saturating_sub(1), *count)?;
            Ok(i)
        }
        Edit::InsertCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            insert_cols(wb, i, parse_col(col)?, *count)?;
            Ok(i)
        }
        Edit::DeleteCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            delete_cols(wb, i, parse_col(col)?, *count)?;
            Ok(i)
        }
        Edit::Format {
            sheet,
            range,
            patch,
        } => {
            let i = si(wb, sheet)?;
            format(wb, i, parse_range(range)?, patch)?;
            Ok(i)
        }
        Edit::Border {
            sheet,
            range,
            preset,
            style,
            color,
        } => {
            let i = si(wb, sheet)?;
            border(
                wb,
                i,
                parse_range(range)?,
                *preset,
                style.as_deref().unwrap_or("thin"),
                color.as_deref(),
            )?;
            Ok(i)
        }
        Edit::Merge {
            sheet,
            range,
            center,
        } => {
            let i = si(wb, sheet)?;
            merge(wb, i, parse_range(range)?, *center)?;
            Ok(i)
        }
        Edit::Unmerge { sheet, range } => {
            let i = si(wb, sheet)?;
            unmerge(wb, i, parse_range(range)?)?;
            Ok(i)
        }
        Edit::ColWidth {
            sheet,
            col,
            count,
            width,
        } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            let stored = stored_col_width(wb, *width);
            set_col_width(wb, i, c, c + count.max(&1) - 1, stored)?;
            Ok(i)
        }
        Edit::ColWidthPx {
            sheet,
            col,
            count,
            px,
        } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            let k = crate::render::book_metrics(wb).k;
            let stored = (*px as f64 / k * 256.0).round() / 256.0;
            set_col_width(wb, i, c, c + count.max(&1) - 1, stored)?;
            Ok(i)
        }
        Edit::RowHeight {
            sheet,
            row,
            count,
            height,
        } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            set_row_height(wb, i, r, r + count.max(&1) - 1, *height)?;
            Ok(i)
        }
        Edit::AddSheet { name, after } => {
            let at = match after {
                Some(s) => sheet_index(wb, Some(s))? + 1,
                None => wb.sheets.len(),
            };
            add_sheet(wb, name.as_deref(), at)
        }
        Edit::RenameSheet { sheet, name } => {
            let i = sheet_index(wb, Some(sheet))?;
            rename_sheet(wb, i, name)?;
            Ok(i)
        }
        Edit::DeleteSheet { sheet } => {
            let i = sheet_index(wb, Some(sheet))?;
            delete_sheet(wb, i)?;
            Ok(i.min(wb.sheets.len().saturating_sub(1)))
        }
        Edit::MoveSheet { sheet, to } => {
            let i = sheet_index(wb, Some(sheet))?;
            move_sheet(wb, i, *to)?;
            Ok((*to).min(wb.sheets.len() - 1))
        }
        Edit::Copy {
            sheet,
            from,
            to,
            to_sheet,
        } => {
            let i = si(wb, sheet)?;
            let d = match to_sheet {
                Some(s) => sheet_index(wb, Some(s))?,
                None => i,
            };
            let c = clip::copy(wb, i, parse_range(from)?)?;
            let dst = parse_range(to)?;
            clip::paste(wb, d, dst.start, &c, Some(dst))?;
            Ok(d)
        }
        Edit::Move {
            sheet,
            from,
            to,
            to_sheet,
        } => {
            let i = si(wb, sheet)?;
            let d = match to_sheet {
                Some(s) => sheet_index(wb, Some(s))?,
                None => i,
            };
            clip::move_cells(wb, i, parse_range(from)?, d, parse_range(to)?.start)?;
            Ok(d)
        }
        Edit::FillDown { sheet, range } => {
            let i = si(wb, sheet)?;
            clip::fill(wb, i, parse_range(range)?, true)?;
            Ok(i)
        }
        Edit::FillRight { sheet, range } => {
            let i = si(wb, sheet)?;
            clip::fill(wb, i, parse_range(range)?, false)?;
            Ok(i)
        }
        Edit::PasteText { sheet, cell, text } => {
            let i = si(wb, sheet)?;
            paste_text(wb, i, cell.parse()?, text)?;
            Ok(i)
        }
    }
}

// ---------------------------------------------------------------------------
// 셀 다루기
// ---------------------------------------------------------------------------

fn find(sheet: &Sheet, r: CellRef) -> Result<usize, usize> {
    sheet
        .cells
        .binary_search_by(|c| (c.r.row, c.r.col).cmp(&(r.row, r.col)))
}

/// 셀 레코드 (없으면 그 칸의 서식 — 행 서식 → 열 서식 — 으로 만든다)
fn cell_mut(sheet: &mut Sheet, r: CellRef) -> &mut Cell {
    let i = match find(sheet, r) {
        Ok(i) => i,
        Err(i) => {
            let style = sheet.effective_style(r.row, r.col);
            sheet.cells.insert(
                i,
                Cell {
                    r,
                    style,
                    value: CellValue::Empty,
                    formula: None,
                },
            );
            i
        }
    };
    &mut sheet.cells[i]
}

/// 칸의 서식 번호 (셀이 없으면 행·열 서식)
pub fn style_at(sheet: &Sheet, r: CellRef) -> u32 {
    sheet
        .cell(r)
        .map(|c| c.style)
        .unwrap_or_else(|| sheet.effective_style(r.row, r.col))
}

/// 칸이 배열 수식·데이터 표 범위 안인가 (그 범위)
fn array_at(sheet: &Sheet, r: CellRef) -> Option<CellRange> {
    sheet.cells.iter().find_map(|c| {
        let f = c.formula.as_ref()?;
        if !matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable) {
            return None;
        }
        let a = CellRange::parse(f.range.as_deref()?)?;
        a.contains(r).then_some(a)
    })
}

/// 같은 공유 수식 묶음의 셀들을 일반 수식으로 (묶음의 한 칸이 바뀔 때)
fn unshare_group(sheet: &mut Sheet, si_: Option<u32>) {
    let Some(si_) = si_ else { return };
    for c in &mut sheet.cells {
        if let Some(f) = &mut c.formula
            && f.kind == FormulaKind::Shared
            && f.shared_index == Some(si_)
        {
            f.kind = FormulaKind::Normal;
            f.shared_index = None;
            f.range = None;
        }
    }
}

/// 수식 글에서 시트·이름·표 이름의 문서 표기
fn canon_name(wb: &Workbook) -> impl Fn(&str) -> Option<String> + '_ {
    move |s: &str| {
        wb.sheets
            .iter()
            .map(|x| &x.name)
            .chain(wb.defined_names.iter().map(|d| &d.name))
            .chain(
                wb.sheets
                    .iter()
                    .flat_map(|x| x.tables.iter().map(|t| &t.display_name)),
            )
            .find(|n| refs::same_sheet(n, s))
            .cloned()
    }
}

/// 입력한 수식을 한셀이 저장하는 꼴로 고르고 문법을 확인한다 (`=` 없이)
pub fn normalize_formula(wb: &Workbook, text: &str) -> crate::Result<String> {
    let canon = canon_name(wb);
    let f = refs::normalize(text, &canon);
    crate::calc::parse(&f).map_err(|e| err(format!("수식 오류: {e}")))?;
    Ok(f)
}

/// 셀에 입력한다 (한셀 입력 규칙). 빈 글이면 내용을 지운다
pub fn set_input(wb: &mut Workbook, si: usize, r: CellRef, text: &str) -> crate::Result<()> {
    let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
    if let Some(a) = array_at(sheet, r)
        && (a.start != a.end)
    {
        return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
    }
    let style = style_at(sheet, r);
    let code = wb
        .styles
        .xf(style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    let parsed = input::parse(text, &code, wb.date1904);
    match parsed {
        Input::Empty => {
            clear(wb, si, CellRange { start: r, end: r }, ClearKind::Contents)?;
        }
        Input::Formula(f) => {
            let text = normalize_formula(wb, &f)?;
            let inherit = if code.eq_ignore_ascii_case("general") {
                formula_format(wb, si, &text)
            } else {
                None
            };
            let new_style = match inherit {
                Some(id) => Restyler::default().apply(&mut wb.styles, style, &|_, x| {
                    x.num_fmt_id = id;
                    x.apply_number_format = Some(true);
                }),
                None => style,
            };
            let sheet = &mut wb.sheets[si];
            let shared = sheet
                .cell(r)
                .and_then(|c| c.formula.as_ref())
                .and_then(|f| {
                    (f.kind == FormulaKind::Shared)
                        .then_some(f.shared_index)
                        .flatten()
                });
            unshare_group(sheet, shared);
            let c = cell_mut(sheet, r);
            c.style = new_style;
            c.value = CellValue::Empty;
            c.formula = Some(Formula {
                text,
                ..Default::default()
            });
        }
        Input::Value {
            value,
            num_fmt,
            quote_prefix,
        } => {
            let cur_qp = wb.styles.xf(style).is_some_and(|x| x.quote_prefix);
            let new_style = if num_fmt.is_some() || cur_qp != quote_prefix {
                Restyler::default().apply(&mut wb.styles, style, &|_, x| {
                    if let Some(id) = num_fmt {
                        x.num_fmt_id = id;
                        x.apply_number_format = Some(true);
                    }
                    x.quote_prefix = quote_prefix;
                })
            } else {
                style
            };
            let sheet = &mut wb.sheets[si];
            let shared = sheet
                .cell(r)
                .and_then(|c| c.formula.as_ref())
                .and_then(|f| {
                    (f.kind == FormulaKind::Shared)
                        .then_some(f.shared_index)
                        .flatten()
                });
            unshare_group(sheet, shared);
            let c = cell_mut(sheet, r);
            c.style = new_style;
            c.value = value;
            c.formula = None;
        }
    }
    structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

/// 수식 결과 칸의 표시 형식 (일반 서식 칸에 수식을 넣을 때 한셀이 붙이는 것 — 입력 대조 실험 2):
/// 날짜·시각 함수(TODAY·DATE → 14, NOW → 22, TIME → 18), 재무 함수(PMT·NPV·FV·PV → 8), 그 밖에는 처음 참조한
/// 칸의 형식 (더하기·빼기·곱하기·나누기·부호·괄호·SUM·MAX·MIN·AVERAGE·ROUND 를 거쳐서. 곱하기·나누기는
/// 백분율을 버리고, SUM 은 날짜 범위의 형식을 버린다)
pub fn formula_format(wb: &Workbook, si: usize, text: &str) -> Option<u32> {
    use crate::calc::ast::{BinaryOp, Expr, SheetSpec, UnaryOp};
    let e = crate::calc::parse(text).ok()?;
    let cat_of = |id: u32| input::category(&crate::numfmt::code_for(&wb.styles, id));
    let ref_fmt = |r: &crate::calc::ast::RefExpr, dates_ok: bool| -> Option<u32> {
        if r.book.is_some() {
            return None;
        }
        let s = match &r.sheet {
            SheetSpec::Current => si,
            SheetSpec::One(n) => wb
                .sheets
                .iter()
                .position(|x| refs::same_sheet(&x.name, n))?,
            SheetSpec::Span(..) => return None,
        };
        let sheet = wb.sheets.get(s)?;
        let id_of = |st: u32| wb.styles.xf(st).map(|x| x.num_fmt_id).unwrap_or(0);
        if r.r0 == r.r1 && r.c0 == r.c1 {
            Some(id_of(style_at(
                sheet,
                CellRef {
                    row: r.r0,
                    col: r.c0,
                },
            )))
            .filter(|&i| i != 0)
        } else {
            let lo = sheet.cells.partition_point(|c| c.r.row < r.r0);
            sheet.cells[lo..]
                .iter()
                .take_while(|c| c.r.row <= r.r1)
                .filter(|c| c.r.col >= r.c0 && c.r.col <= r.c1)
                .map(|c| id_of(c.style))
                .find(|&i| i != 0)
                .filter(|&i| dates_ok || cat_of(i) != input::Cat::Date)
        }
    };
    fn walk(
        e: &Expr,
        ref_fmt: &dyn Fn(&crate::calc::ast::RefExpr, bool) -> Option<u32>,
        cat_of: &dyn Fn(u32) -> input::Cat,
    ) -> Option<u32> {
        match e {
            Expr::Ref(r) => ref_fmt(r, true),
            Expr::Paren(a) | Expr::Unary(UnaryOp::Neg | UnaryOp::Plus, a) => {
                walk(a, ref_fmt, cat_of)
            }
            Expr::Binary(BinaryOp::Add | BinaryOp::Sub, a, b) => {
                walk(a, ref_fmt, cat_of).or_else(|| walk(b, ref_fmt, cat_of))
            }
            Expr::Binary(BinaryOp::Mul | BinaryOp::Div, a, b) => walk(a, ref_fmt, cat_of)
                .or_else(|| walk(b, ref_fmt, cat_of))
                .filter(|&i| cat_of(i) != input::Cat::Percent),
            Expr::Call(c) => match c.name.as_str() {
                "TODAY" | "DATE" => Some(14),
                "NOW" => Some(22),
                "TIME" => Some(18),
                "PMT" | "NPV" | "FV" | "PV" | "IPMT" | "PPMT" => Some(8),
                "SUM" => c.args.iter().find_map(|a| match a {
                    Expr::Ref(r) if r.r0 != r.r1 || r.c0 != r.c1 => ref_fmt(r, false),
                    _ => walk(a, ref_fmt, cat_of),
                }),
                "MAX" | "MIN" | "AVERAGE" | "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                    c.args.iter().find_map(|a| walk(a, ref_fmt, cat_of))
                }
                _ => None,
            },
            _ => None,
        }
    }
    walk(&e, &ref_fmt, &cat_of)
}

/// 지우기 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearKind {
    /// 내용만 (Delete 키)
    Contents,
    /// 서식만
    Formats,
    /// 내용·서식·병합
    All,
}

/// 범위를 지운다
pub fn clear(wb: &mut Workbook, si: usize, range: CellRange, kind: ClearKind) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    let inside = |r: CellRef| r.row >= r0 && r.row <= r1 && r.col >= c0 && r.col <= c1;
    if kind != ClearKind::Formats {
        // 배열 수식은 범위 전체를 지울 때만
        for c in &sheet.cells {
            if let Some(f) = &c.formula
                && matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable)
                && let Some(a) = f.range.as_deref().and_then(CellRange::parse)
            {
                let any = (a.start.row..=a.end.row)
                    .any(|r| (a.start.col..=a.end.col).any(|c| inside(CellRef { row: r, col: c })));
                let all = inside(a.start) && inside(a.end);
                if any && !all {
                    return Err(err("배열 수식의 일부를 바꿀 수 없습니다"));
                }
            }
        }
    }
    let shared: Vec<u32> = sheet
        .cells
        .iter()
        .filter(|c| inside(c.r))
        .filter_map(|c| c.formula.as_ref())
        .filter(|f| f.kind == FormulaKind::Shared)
        .filter_map(|f| f.shared_index)
        .collect();
    for s in shared {
        unshare_group(sheet, Some(s));
    }
    let lo = sheet.cells.partition_point(|c| c.r.row < r0);
    let hi = sheet.cells.partition_point(|c| c.r.row <= r1);
    for c in &mut sheet.cells[lo..hi] {
        if !inside(c.r) {
            continue;
        }
        if kind != ClearKind::Formats {
            c.value = CellValue::Empty;
            c.formula = None;
        }
        if kind != ClearKind::Contents {
            c.style = 0;
        }
    }
    // 내용·서식이 모두 없는 셀 레코드는 없앤다 (행·열 서식이 있는 칸은 기본 서식을 남겨 둔다)
    let rows_styled: std::collections::HashSet<u32> = sheet
        .rows
        .iter()
        .filter(|r| r.style.is_some_and(|s| s != 0))
        .map(|r| r.index - 1)
        .collect();
    let cols_styled = sheet.cols.iter().any(|c| c.style.is_some_and(|s| s != 0));
    sheet.cells.retain(|c| {
        !(inside(c.r)
            && c.value.is_empty()
            && c.formula.is_none()
            && c.style == 0
            && !rows_styled.contains(&c.r.row)
            && !cols_styled)
    });
    if kind == ClearKind::All {
        sheet.merges.retain(|m| !(inside(m.start) && inside(m.end)));
    }
    structure::update_dimension(sheet);
    Ok(())
}

/// 탭·줄 바꿈으로 나눈 글을 `at` 부터 칸마다 입력한다 (다른 프로그램에서 복사한 표)
pub fn paste_text(
    wb: &mut Workbook,
    si: usize,
    at: CellRef,
    text: &str,
) -> crate::Result<CellRange> {
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    let mut end = at;
    for (i, line) in text.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        for (j, field) in split_tsv_line(line).into_iter().enumerate() {
            let r = CellRef {
                row: at.row + i as u32,
                col: at.col + j as u32,
            };
            if r.row >= refs::MAX_ROWS || r.col >= refs::MAX_COLS {
                continue;
            }
            set_input(wb, si, r, &field)?;
            end.row = end.row.max(r.row);
            end.col = end.col.max(r.col);
        }
    }
    Ok(CellRange { start: at, end })
}

/// TSV 한 줄 (큰따옴표로 감싼 칸은 따옴표를 푼다)
fn split_tsv_line(line: &str) -> Vec<String> {
    line.split('\t')
        .map(|f| {
            if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
                f[1..f.len() - 1].replace("\"\"", "\"")
            } else {
                f.to_string()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 행·열
// ---------------------------------------------------------------------------

pub fn insert_rows(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    structure::shift(wb, si, Shift::InsRows { at, n: n.max(1) })
}

pub fn delete_rows(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    structure::shift(wb, si, Shift::DelRows { at, n: n.max(1) })
}

pub fn insert_cols(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    structure::shift(wb, si, Shift::InsCols { at, n: n.max(1) })
}

pub fn delete_cols(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    structure::shift(wb, si, Shift::DelCols { at, n: n.max(1) })
}

/// 한셀 COM `ColumnWidth`(글자 수) → 저장하는 열 너비: 여백 6.22 px 를 글자 수로 더해 1/256 로 자른다
/// (대조 실험: 20 → 20.77734375, 8.43 → 9.20703125 — 맑은 고딕 11, 숫자 폭 8 px)
pub fn stored_col_width(wb: &Workbook, chars: f64) -> f64 {
    if chars < 1.0 {
        return (chars * 256.0).floor() / 256.0;
    }
    let k = crate::render::book_metrics(wb).k;
    let pad = (6.22 / k * 256.0).round();
    ((chars * 256.0).floor() + pad) / 256.0
}

/// 열 너비 (저장 값, 글자 수). 0 이하면 숨긴다
pub fn set_col_width(
    wb: &mut Workbook,
    si: usize,
    c0: u32,
    c1: u32,
    width: f64,
) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    let (a, b) = (c0 + 1, c1.min(refs::MAX_COLS - 1) + 1);
    let mut out: Vec<ColInfo> = Vec::new();
    let mut base: Vec<(u32, ColInfo)> = Vec::new();
    for c in std::mem::take(&mut sheet.cols) {
        if c.max < a || c.min > b {
            out.push(c);
            continue;
        }
        if c.min < a {
            out.push(ColInfo {
                max: a - 1,
                ..c.clone()
            });
        }
        if c.max > b {
            out.push(ColInfo {
                min: b + 1,
                ..c.clone()
            });
        }
        for k in c.min.max(a)..=c.max.min(b) {
            base.push((k, c.clone()));
        }
    }
    for k in a..=b {
        let mut info = base
            .iter()
            .find(|(i, _)| *i == k)
            .map(|(_, c)| c.clone())
            .unwrap_or(ColInfo {
                min: k,
                max: k,
                ..Default::default()
            });
        info.min = k;
        info.max = k;
        if width <= 0.0 {
            info.hidden = true;
        } else {
            info.width = Some(width);
            info.hidden = false;
        }
        info.custom_width = true;
        out.push(info);
    }
    out.sort_by_key(|c| c.min);
    sheet.cols = out;
    Ok(())
}

/// 행 높이 (pt). 0 이하면 숨긴다
pub fn set_row_height(
    wb: &mut Workbook,
    si: usize,
    r0: u32,
    r1: u32,
    height: f64,
) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    for r in r0..=r1.min(refs::MAX_ROWS - 1) {
        let i = match sheet.rows.binary_search_by_key(&(r + 1), |x| x.index) {
            Ok(i) => i,
            Err(i) => {
                sheet.rows.insert(
                    i,
                    RowInfo {
                        index: r + 1,
                        ..Default::default()
                    },
                );
                i
            }
        };
        let info = &mut sheet.rows[i];
        if height <= 0.0 {
            info.hidden = true;
        } else {
            info.height = Some((height * 100.0).round() / 100.0);
            info.custom_height = true;
            info.hidden = false;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 서식
// ---------------------------------------------------------------------------

/// 범위의 칸마다 서식을 바꾼다. 행 전체·열 전체 범위는 행·열 서식과 그 안의 셀을 바꾼다
fn restyle(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    f: &dyn Fn(&mut crate::model::Styles, &mut crate::model::Xf, CellRef),
) -> crate::Result<()> {
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    let whole_rows = c0 == 0 && c1 >= refs::MAX_COLS - 1;
    let whole_cols = r0 == 0 && r1 >= refs::MAX_ROWS - 1;
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    // 칸마다 서식이 위치에 따라 다를 수 있다 (테두리) → (서식, 자리 종류) 별로 기억한다
    let mut memo: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    let kind_of = |r: CellRef| -> u8 {
        (r.row == r0) as u8
            | ((r.row == r1) as u8) << 1
            | ((r.col == c0) as u8) << 2
            | ((r.col == c1) as u8) << 3
    };
    let mut apply_one = |styles: &mut crate::model::Styles, old: u32, at: CellRef| -> u32 {
        let key = (old, kind_of(at));
        if let Some(&n) = memo.get(&key) {
            return n;
        }
        let mut xf = styles.xf(old).cloned().unwrap_or_default();
        f(styles, &mut xf, at);
        let n = style::intern_xf(styles, xf);
        memo.insert(key, n);
        n
    };
    if whole_rows || whole_cols {
        // 행·열 서식 + 이미 있는 셀
        let sheet = &wb.sheets[si];
        let mut cells: Vec<(usize, u32, CellRef)> = Vec::new();
        for (i, c) in sheet.cells.iter().enumerate() {
            if c.r.row >= r0 && c.r.row <= r1 && c.r.col >= c0 && c.r.col <= c1 {
                cells.push((i, c.style, c.r));
            }
        }
        let rows_info: Vec<(u32, Option<u32>)> = if whole_rows {
            (r0..=r1.min(r0 + 100_000))
                .map(|r| {
                    (
                        r,
                        sheet
                            .rows
                            .iter()
                            .find(|x| x.index == r + 1)
                            .and_then(|x| x.style),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let cols_info: Vec<(u32, Option<u32>)> = if whole_cols && !whole_rows {
            (c0..=c1)
                .map(|c| {
                    (
                        c,
                        sheet
                            .cols
                            .iter()
                            .find(|x| (x.min..=x.max).contains(&(c + 1)))
                            .and_then(|x| x.style),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut new_cells = Vec::new();
        for (i, st, r) in cells {
            new_cells.push((i, apply_one(&mut wb.styles, st, r)));
        }
        let mut new_rows = Vec::new();
        for (r, st) in rows_info {
            new_rows.push((
                r,
                apply_one(&mut wb.styles, st.unwrap_or(0), CellRef { row: r, col: c0 }),
            ));
        }
        let mut new_cols = Vec::new();
        for (c, st) in cols_info {
            new_cols.push((
                c,
                apply_one(&mut wb.styles, st.unwrap_or(0), CellRef { row: r0, col: c }),
            ));
        }
        let sheet = &mut wb.sheets[si];
        for (i, st) in new_cells {
            sheet.cells[i].style = st;
        }
        for (r, st) in new_rows {
            match sheet.rows.binary_search_by_key(&(r + 1), |x| x.index) {
                Ok(i) => sheet.rows[i].style = Some(st),
                Err(i) => sheet.rows.insert(
                    i,
                    RowInfo {
                        index: r + 1,
                        style: Some(st),
                        ..Default::default()
                    },
                ),
            }
        }
        if !new_cols.is_empty() {
            for (c, st) in new_cols {
                let cur = sheet
                    .cols
                    .iter()
                    .find(|x| (x.min..=x.max).contains(&(c + 1)))
                    .cloned();
                let mut info = cur.unwrap_or_else(|| ColInfo {
                    width: None,
                    ..Default::default()
                });
                info.min = c + 1;
                info.max = c + 1;
                info.style = Some(st);
                let mut rest: Vec<ColInfo> = Vec::new();
                for x in std::mem::take(&mut sheet.cols) {
                    if (x.min..=x.max).contains(&(c + 1)) {
                        if x.min < c + 1 {
                            rest.push(ColInfo {
                                max: c,
                                ..x.clone()
                            });
                        }
                        if x.max > c + 1 {
                            rest.push(ColInfo {
                                min: c + 2,
                                ..x.clone()
                            });
                        }
                    } else {
                        rest.push(x);
                    }
                }
                rest.push(info);
                rest.sort_by_key(|x| x.min);
                sheet.cols = rest;
            }
        }
        return Ok(());
    }
    if (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64 > 2_000_000 {
        return Err(err("범위가 너무 큽니다"));
    }
    for r in r0..=r1 {
        for c in c0..=c1 {
            let at = CellRef { row: r, col: c };
            let old = style_at(&wb.sheets[si], at);
            let new = apply_one(&mut wb.styles, old, at);
            if new != old || wb.sheets[si].cell(at).is_some() {
                cell_mut(&mut wb.sheets[si], at).style = new;
            }
        }
    }
    structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

/// 글꼴·채우기·맞춤·줄 바꿈·표시 형식
pub fn format(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    p: &FormatPatch,
) -> crate::Result<()> {
    if p.is_empty() {
        return Ok(());
    }
    restyle(wb, si, range, &|st, xf, _| style::patch_xf(st, xf, p))
}

/// 테두리
pub fn border(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    preset: BorderPreset,
    line: &str,
    color: Option<&str>,
) -> crate::Result<()> {
    let e = style::edge(line, color);
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    restyle(wb, si, range, &|st, xf, at| {
        let set = style::edges_for(
            preset,
            &e,
            at.row == r0,
            at.row == r1,
            at.col == c0,
            at.col == c1,
        );
        if !set.is_empty() {
            style::patch_border(st, xf, &set);
        }
    })
}

/// 병합 (왼쪽 위 칸의 값만 남는다 — 한셀). 겹치는 병합은 먼저 푼다
pub fn merge(wb: &mut Workbook, si: usize, range: CellRange, center: bool) -> crate::Result<()> {
    if range.start == range.end {
        return Ok(());
    }
    {
        let sheet = wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
        for r in range.start.row..=range.end.row.min(range.start.row + 10_000) {
            for c in range.start.col..=range.end.col.min(range.start.col + 1_000) {
                if let Some(a) = array_at(sheet, CellRef { row: r, col: c })
                    && a.start != a.end
                {
                    return Err(err("배열 수식이 있는 칸은 병합할 수 없습니다"));
                }
            }
        }
    }
    let tl = range.start;
    {
        let sheet = &mut wb.sheets[si];
        let overlap = |m: &CellRange| {
            !(m.end.row < range.start.row
                || m.start.row > range.end.row
                || m.end.col < range.start.col
                || m.start.col > range.end.col)
        };
        sheet.merges.retain(|m| !overlap(m));
        for c in &mut sheet.cells {
            if range.contains(c.r) && c.r != tl {
                c.value = CellValue::Empty;
                c.formula = None;
            }
        }
        sheet.merges.push(range);
    }
    if center {
        let p = FormatPatch {
            h_align: Some("center".into()),
            ..Default::default()
        };
        format(wb, si, range, &p)?;
    }
    Ok(())
}

/// 병합 풀기 (범위와 겹치는 병합 모두)
pub fn unmerge(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| err("시트가 없습니다"))?;
    sheet.merges.retain(|m| {
        m.end.row < range.start.row
            || m.start.row > range.end.row
            || m.end.col < range.start.col
            || m.start.col > range.end.col
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// 시트
// ---------------------------------------------------------------------------

/// 시트 이름 규칙 (한셀·Excel): 1–31자, `: \ / ? * [ ]` 없이, 작은따옴표로 시작·끝나지 않고, 겹치지 않게
pub fn check_sheet_name(wb: &Workbook, name: &str, except: Option<usize>) -> crate::Result<()> {
    if name.is_empty() || name.chars().count() > 31 {
        return Err(err("시트 이름은 1–31자여야 합니다"));
    }
    if name.contains([':', '\\', '/', '?', '*', '[', ']']) {
        return Err(err("시트 이름에 : \\ / ? * [ ] 를 쓸 수 없습니다"));
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(err("시트 이름은 작은따옴표로 시작하거나 끝날 수 없습니다"));
    }
    if wb
        .sheets
        .iter()
        .enumerate()
        .any(|(i, s)| Some(i) != except && refs::same_sheet(&s.name, name))
    {
        return Err(err(format!("같은 이름의 시트가 있습니다: {name}")));
    }
    Ok(())
}

/// 한셀의 새 시트 인쇄 설정 (여백 19·25·13 mm, A4 세로 — 시트 추가 대조 실험)
fn hcell_page() -> crate::model::PageLayout {
    use crate::model::{HeaderFooter, PageLayout, PageMargins, PageSetup};
    PageLayout {
        margins: Some(PageMargins {
            left: 0.7480555772781372,
            right: 0.7480555772781372,
            top: 0.9843055605888367,
            bottom: 0.9843055605888367,
            header: 0.511805534362793,
            footer: 0.511805534362793,
        }),
        setup: Some(PageSetup {
            paper_size: Some(9),
            scale: Some(100),
            first_page_number: Some(1),
            fit_to_width: Some(0),
            fit_to_height: Some(0),
            orientation: Some("portrait".into()),
            use_printer_defaults: true,
            horizontal_dpi: Some(0),
            vertical_dpi: Some(0),
            copies: Some(1),
            ..Default::default()
        }),
        header_footer: Some(HeaderFooter {
            scale_with_doc: true,
            align_with_margins: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 시트 번호가 바뀌었다: 시트 범위 이름·통합 문서 창의 활성 시트를 맞춘다 (`map(old) = new`, None = 없어짐)
fn remap_sheet_indexes(wb: &mut Workbook, map: &dyn Fn(usize) -> Option<usize>) {
    wb.defined_names.retain_mut(|d| match d.local_sheet_id {
        Some(i) => match map(i as usize) {
            Some(n) => {
                d.local_sheet_id = Some(n as u32);
                true
            }
            None => false,
        },
        None => true,
    });
    if let Some(v) = &mut wb.view {
        if let Some(n) = map(v.active_tab as usize) {
            v.active_tab = n as u32;
        }
        if let Some(n) = map(v.first_sheet as usize) {
            v.first_sheet = n as u32;
        } else {
            v.first_sheet = 0;
        }
    }
}

/// 시트를 더한다 (`at` 자리에). 이름이 없으면 한셀처럼 쓰지 않은 가장 작은 `SheetN`
pub fn add_sheet(wb: &mut Workbook, name: Option<&str>, at: usize) -> crate::Result<usize> {
    let at = at.min(wb.sheets.len());
    let name = match name {
        Some(n) => {
            check_sheet_name(wb, n, None)?;
            n.to_string()
        }
        None => (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|n| !wb.sheets.iter().any(|s| refs::same_sheet(&s.name, n)))
            .unwrap_or_default(),
    };
    let sheet_id = wb.sheets.iter().map(|s| s.sheet_id).max().unwrap_or(0) + 1;
    let model = wb
        .sheets
        .get(at.saturating_sub(1))
        .or_else(|| wb.sheets.first());
    let part = (1..)
        .map(|n| format!("xl/worksheets/sheet{}.xml", sheet_id + n - 1))
        .find(|p| !wb.sheets.iter().any(|s| &s.part == p))
        .unwrap_or_default();
    let sheet = Sheet {
        name,
        sheet_id,
        part,
        show_grid_lines: true,
        default_col_width: model.and_then(|m| m.default_col_width),
        default_row_height: model.and_then(|m| m.default_row_height),
        page: hcell_page(),
        dimension: Some("A1".into()),
        ..Default::default()
    };
    wb.sheets.insert(at, sheet);
    remap_sheet_indexes(wb, &|i| Some(if i >= at { i + 1 } else { i }));
    Ok(at)
}

/// 시트 이름을 바꾼다 (그 시트를 가리키는 수식·이름·차트·하이퍼링크도)
pub fn rename_sheet(wb: &mut Workbook, si: usize, name: &str) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    check_sheet_name(wb, name, Some(si))?;
    let old = wb.sheets[si].name.clone();
    if old == name {
        return Ok(());
    }
    structure::map_all_formulas(wb, &|f, _| refs::rename_sheet(f, &old, name));
    wb.sheets[si].name = name.to_string();
    Ok(())
}

/// 시트를 지운다 (그 시트를 가리키는 참조는 `#REF!`). 보이는 시트가 하나뿐이면 지울 수 없다
pub fn delete_sheet(wb: &mut Workbook, si: usize) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let visible = wb
        .sheets
        .iter()
        .filter(|s| s.state == crate::model::SheetState::Visible)
        .count();
    if visible <= 1 && wb.sheets[si].state == crate::model::SheetState::Visible {
        return Err(err("시트가 하나뿐이면 지울 수 없습니다"));
    }
    let name = wb.sheets[si].name.clone();
    let was_selected = wb.sheets[si].view.tab_selected;
    let after: Vec<String> = wb
        .sheets
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != si)
        .map(|(_, s)| s.name.clone())
        .collect();
    wb.sheets.remove(si);
    structure::map_all_formulas(wb, &|f, _| refs::delete_sheet(f, &name, &after, si));
    remap_sheet_indexes(wb, &|i| match i.cmp(&si) {
        std::cmp::Ordering::Less => Some(i),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(i - 1),
    });
    if let Some(v) = &mut wb.view
        && v.active_tab as usize >= wb.sheets.len()
    {
        v.active_tab = wb.sheets.len().saturating_sub(1) as u32;
    }
    if was_selected || !wb.sheets.iter().any(|s| s.view.tab_selected) {
        let pick = (si..wb.sheets.len())
            .chain((0..si.min(wb.sheets.len())).rev())
            .find(|&i| wb.sheets[i].state == crate::model::SheetState::Visible);
        if let Some(p) = pick {
            for (i, s) in wb.sheets.iter_mut().enumerate() {
                s.view.tab_selected = i == p;
            }
            if let Some(v) = &mut wb.view {
                v.active_tab = p as u32;
            }
        }
    }
    Ok(())
}

/// 시트를 옮긴다 (`to` = 옮긴 뒤의 자리)
pub fn move_sheet(wb: &mut Workbook, from: usize, to: usize) -> crate::Result<()> {
    if from >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let to = to.min(wb.sheets.len() - 1);
    if from == to {
        return Ok(());
    }
    let s = wb.sheets.remove(from);
    wb.sheets.insert(to, s);
    remap_sheet_indexes(wb, &|i| {
        Some(if i == from {
            to
        } else if from < to && i > from && i <= to {
            i - 1
        } else if to < from && i >= to && i < from {
            i + 1
        } else {
            i
        })
    });
    Ok(())
}

#[cfg(test)]
mod tests;
