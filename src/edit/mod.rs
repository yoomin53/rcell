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

pub mod cells;
mod cfpaste;
pub mod clip;
pub mod collate;
mod collate_data;
mod collate_kana;
pub mod data;
pub mod fill;
pub mod filter;
pub mod find;
pub mod input;
pub mod objects;
pub mod page;
pub mod protect;
pub mod refs;
pub mod shapes;
pub mod sort;
pub mod special;
pub mod structure;
pub mod style;
pub mod table;
pub mod view;

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

fn yes() -> bool {
    true
}

fn one() -> u32 {
    1
}

fn shift_down() -> cells::ShiftDir {
    cells::ShiftDir::Down
}

fn shift_up() -> cells::ShiftDir {
    cells::ShiftDir::Up
}

fn one_f() -> f64 {
    1.0
}

/// 텍스트 나누기의 필드 형식 (`col` 은 필드 번호 1부터, `kind` 는 `general` `text` `skip`)
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct FieldSpec {
    pub col: u32,
    #[serde(default)]
    pub kind: String,
}

/// 편집 명령 하나. 행은 1부터(한셀 화면과 같다), 열은 이름(`C`), 셀·범위는 A1 표기.
/// `sheet` 를 빼면 첫 시트
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
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
        #[serde(default, alias = "to_sheet")]
        to_sheet: Option<SheetSel>,
    },
    /// 잘라 붙이기 (셀 옮기기)
    Move {
        #[serde(default)]
        sheet: Option<SheetSel>,
        from: String,
        to: String,
        #[serde(default, alias = "to_sheet")]
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
    /// 정렬 (12절). 열쇠는 열 이름 (왼쪽에서 오른쪽이면 행 번호)
    Sort {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        keys: Vec<SortBy>,
        /// 첫 행(열)은 머리글
        #[serde(default)]
        header: bool,
        #[serde(default)]
        match_case: bool,
        #[serde(default)]
        left_to_right: bool,
    },
    /// 자동 채우기 (채우기 핸들 — 13절): `range` 씨앗으로 `to` 를 채운다
    AutoFill {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        to: String,
        /// `default` `copy` `series` `formats` `values` `days` `weekdays` `months` `years`
        #[serde(default, rename = "type")]
        kind: fill::FillType,
    },
    /// 연속 데이터 채우기 (연속 데이터 대화상자)
    Series {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        /// 행 방향(오른쪽으로). 빼면 범위 모양으로 (열이 행보다 많으면 행 방향)
        #[serde(default)]
        rows: Option<bool>,
        /// `linear` `growth` `date` `autoFill`
        #[serde(default, rename = "type")]
        kind: fill::SeriesType,
        /// `day` `weekday` `month` `year`
        #[serde(default)]
        date: fill::DateUnit,
        #[serde(default = "one_f")]
        step: f64,
        #[serde(default)]
        stop: Option<f64>,
        #[serde(default)]
        trend: bool,
    },
    /// 바꾸기 (14절). `all` 이면 모든 시트, `range` 가 있으면 그 안에서만
    Replace {
        #[serde(default)]
        sheet: Option<SheetSel>,
        find: String,
        #[serde(default)]
        replace: String,
        #[serde(default)]
        match_case: bool,
        /// 셀 전체 (한셀: 공백으로 나뉜 낱말 전체)
        #[serde(default)]
        whole_cell: bool,
        #[serde(default)]
        all: bool,
        #[serde(default)]
        range: Option<String>,
    },
    /// 선택하여 붙이기 (16절): `paste` = `all` `values` `formats` `formulas` `allExceptBorders`
    /// `formulasAndNumberFormats` `valuesAndNumberFormats`, `operation` = `none` `add` `subtract` `multiply` `divide`
    PasteSpecial {
        #[serde(default)]
        sheet: Option<SheetSel>,
        from: String,
        to: String,
        #[serde(default, alias = "to_sheet")]
        to_sheet: Option<SheetSel>,
        #[serde(default)]
        paste: special::PasteWhat,
        #[serde(default)]
        operation: special::PasteOp,
        #[serde(default)]
        skip_blanks: bool,
        #[serde(default)]
        transpose: bool,
    },
    /// 메모 더하기 (`author` 가 있으면 글 앞에 `작성자:` 줄)
    AddComment {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
        text: String,
        #[serde(default)]
        author: String,
    },
    /// 메모 고치기 (글 전체)
    EditComment {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
        text: String,
    },
    /// 범위의 메모 지우기
    DeleteComment {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 하이퍼링크 더하기 (`address` 바깥 주소 또는 `location` 문서 안 자리)
    AddHyperlink {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
        #[serde(default)]
        address: Option<String>,
        #[serde(default)]
        location: Option<String>,
        #[serde(default)]
        tooltip: Option<String>,
        #[serde(default)]
        display: Option<String>,
    },
    /// 범위의 하이퍼링크 지우기
    RemoveHyperlink {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 이름 정의 (`local` 이면 `sheet` 의 시트 범위)
    AddName {
        #[serde(default)]
        sheet: Option<SheetSel>,
        name: String,
        refers_to: String,
        #[serde(default)]
        local: bool,
    },
    /// 이름 지우기
    DeleteName {
        #[serde(default)]
        sheet: Option<SheetSel>,
        name: String,
        #[serde(default)]
        local: bool,
    },
    /// 데이터 유효성 더하기 (`type`: `whole` `decimal` `list` `date` `time` `textLength` `custom`)
    AddValidation {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        operator: Option<String>,
        #[serde(default)]
        formula1: String,
        #[serde(default)]
        formula2: Option<String>,
        /// `stop`(기본) `warning` `information`
        #[serde(default)]
        error_style: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// 범위의 데이터 유효성 지우기
    DeleteValidation {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 조건부 서식 규칙 더하기 (`type`: `cellIs` `expression` …, 서식은 `fill` `fontColor` `bold`)
    AddCf {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        operator: Option<String>,
        #[serde(default)]
        formula1: Option<String>,
        #[serde(default)]
        formula2: Option<String>,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        font_color: Option<String>,
        #[serde(default)]
        bold: bool,
    },
    /// 범위의 조건부 서식 지우기
    DeleteCf {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
    },
    /// 열 너비 자동 맞춤
    AutoFitCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 중복 항목 제거 (`columns` 는 범위 안 열 번호 1부터 — 비면 모든 열, `header` 면 첫 행은 견주지 않는다)
    RemoveDuplicates {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(default)]
        columns: Vec<u32>,
        #[serde(default)]
        header: bool,
    },
    /// 텍스트 나누기 (한 열을 구분 문자로 — `dest` 가 없으면 그 열에서 시작)
    TextToColumns {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(default)]
        dest: Option<String>,
        #[serde(default)]
        tab: bool,
        #[serde(default)]
        semicolon: bool,
        #[serde(default)]
        comma: bool,
        #[serde(default)]
        space: bool,
        /// 그 밖의 구분 문자 (글자 하나)
        #[serde(default)]
        other: Option<String>,
        #[serde(default)]
        consecutive: bool,
        /// 묶는 따옴표: `"` (기본), `'`, 빈 글(없음)
        #[serde(default)]
        qualifier: Option<String>,
        #[serde(default)]
        fields: Vec<FieldSpec>,
    },
    /// 부분합 (`group_by`·`totals` 는 범위 안 열 번호 1부터, `func`: sum count average max min product countNums stdev stdevp var varp)
    Subtotal {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        group_by: u32,
        #[serde(default)]
        func: Option<String>,
        totals: Vec<u32>,
        #[serde(default = "yes")]
        replace: bool,
        #[serde(default)]
        page_breaks: bool,
        #[serde(default = "yes")]
        summary_below: bool,
    },
    /// 도형 더하기 (`kind`: `rect` `diamond` `roundRect` `triangle` `ellipse` `textbox`, 위치·크기는 포인트 — 19절)
    AddShape {
        #[serde(default)]
        sheet: Option<SheetSel>,
        kind: String,
        left: f64,
        top: f64,
        width: f64,
        height: f64,
    },
    /// 도형·그림·차트 옮기기·크기 바꾸기 (`shape`: 번호 1부터 또는 이름, 위치·크기는 포인트)
    MoveShape {
        #[serde(default)]
        sheet: Option<SheetSel>,
        shape: shapes::ShapeSel,
        #[serde(default)]
        left: Option<f64>,
        #[serde(default)]
        top: Option<f64>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
    },
    /// 도형·그림·차트 지우기
    DeleteShape {
        #[serde(default)]
        sheet: Option<SheetSel>,
        shape: shapes::ShapeSel,
    },
    /// 통합 (`sources`: `시트!A1:B5` 꼴 범위 글들, `dest` 부터 값으로 합친다)
    Consolidate {
        #[serde(default)]
        sheet: Option<SheetSel>,
        dest: String,
        sources: Vec<String>,
        #[serde(default)]
        func: Option<String>,
        #[serde(default)]
        top_row: bool,
        #[serde(default)]
        left_column: bool,
    },
    /// 행 높이 자동 맞춤
    AutoFitRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// 셀 삽입 (`shift`: `down` 아래로 밀기, `right` 오른쪽으로 밀기)
    InsertCells {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(default = "shift_down")]
        shift: cells::ShiftDir,
    },
    /// 셀 삭제 (`shift`: `up` 위로 당기기, `left` 왼쪽으로 당기기)
    DeleteCells {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(default = "shift_up")]
        shift: cells::ShiftDir,
    },
    /// 행 숨기기
    HideRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// 행 숨기기 취소
    UnhideRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// 행 그룹 (개요 수준 +1)
    GroupRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// 행 그룹 해제
    UngroupRows {
        #[serde(default)]
        sheet: Option<SheetSel>,
        row: u32,
        #[serde(default = "one")]
        count: u32,
    },
    /// 열 숨기기
    HideCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 열 숨기기 취소
    UnhideCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 열 그룹
    GroupCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 열 그룹 해제
    UngroupCols {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default = "one")]
        count: u32,
    },
    /// 시트 숨기기
    HideSheet {
        sheet: SheetSel,
    },
    /// 시트 숨기기 취소
    UnhideSheet {
        sheet: SheetSel,
    },
    /// 틀 고정 (`cell` 의 왼쪽·위)
    Freeze {
        #[serde(default)]
        sheet: Option<SheetSel>,
        cell: String,
    },
    /// 틀 고정 풀기
    Unfreeze {
        #[serde(default)]
        sheet: Option<SheetSel>,
    },
    /// 자동 필터 켜기·끄기·조건 걸기 (`field` 없이 `range` 만이면 켜고 이미 켜져 있으면 끈다, `field`(1부터)와 `criteria1` 로
    /// 조건, `criteria1` 없이 `field` 만이면 그 열 조건 지우기 — 22절)
    AutoFilter {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(default)]
        field: Option<u32>,
        #[serde(default)]
        criteria1: Option<String>,
        /// `and`(기본) / `or` / `top` / `bottom` / `topPercent` / `bottomPercent`
        #[serde(default)]
        operator: Option<String>,
        #[serde(default)]
        criteria2: Option<String>,
    },
    /// 필터 조건을 모두 지우고 숨긴 행을 보인다 (자동 필터는 그대로)
    ShowAllData {
        #[serde(default)]
        sheet: Option<SheetSel>,
    },
    /// 표(ListObject) 만들기 (`range` 첫 행이 머리글 — 24절)
    AddTable {
        #[serde(default)]
        sheet: Option<SheetSel>,
        range: String,
        #[serde(flatten)]
        o: table::TableEdit,
    },
    /// 쪽 설정 (여백·용지·방향·확대·페이지 맞추기·인쇄 영역·반복 제목·머리글/바닥글 …, 준 것만 바꾼다 — 20절)
    PageSetup {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(flatten)]
        p: Box<page::PageSetupEdit>,
    },
    /// 시트 보호 걸기 (암호는 16비트 해시로 둔다, 허용할 동작은 `formattingCells` … — 21절)
    ProtectSheet {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(flatten)]
        o: Box<protect::ProtectOptions>,
    },
    /// 시트 보호 풀기 (암호가 있으면 맞아야 한다)
    UnprotectSheet {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(default)]
        password: Option<String>,
    },
    /// 통합 문서 구조 보호 걸기 (`windows` 는 한셀이 저장하지 않는다)
    ProtectWorkbook {
        #[serde(default)]
        password: Option<String>,
        #[serde(default = "yes")]
        structure: bool,
        #[serde(default)]
        windows: bool,
    },
    /// 통합 문서 보호 풀기
    UnprotectWorkbook {
        #[serde(default)]
        password: Option<String>,
    },
    /// 보기 설정 (확대율 12–400, 눈금선·머리글·0 표시, 보기 방식 `normal`/`pageBreakPreview`) — 그 시트가 활성이 된다
    SetView {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(default)]
        zoom: Option<u32>,
        #[serde(default)]
        gridlines: Option<bool>,
        #[serde(default)]
        headings: Option<bool>,
        #[serde(default)]
        zeros: Option<bool>,
        #[serde(default)]
        view: Option<String>,
    },
    /// 시트 복사 (`to`: 복사본이 놓일 자리 — 원본 목록 기준 0부터, 기본은 원본 바로 뒤. 23절)
    CopySheet {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(default)]
        to: Option<usize>,
        #[serde(default)]
        name: Option<String>,
    },
    /// 시트 탭 색 (`RRGGBB`, 빈 글이면 없앤다)
    TabColor {
        #[serde(default)]
        sheet: Option<SheetSel>,
        #[serde(default)]
        color: String,
    },
    /// 자동 필터로 정렬 (필터 범위를 첫 행을 머리글로 두고 한 열로 정렬하고 `sortState` 를 남긴다)
    SortFilter {
        #[serde(default)]
        sheet: Option<SheetSel>,
        col: String,
        #[serde(default)]
        desc: bool,
        #[serde(default)]
        match_case: bool,
    },
}

/// 정렬 열쇠 하나 (`{"by":"B","desc":true}`)
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SortBy {
    pub by: ColOrRow,
    #[serde(default)]
    pub desc: bool,
}

/// 열 이름 또는 행 번호 (1부터)
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ColOrRow {
    Num(u32),
    Name(String),
}

impl ColOrRow {
    /// 열 번호 (0부터)
    pub fn col(&self) -> crate::Result<u32> {
        match self {
            ColOrRow::Num(n) if *n >= 1 => Ok(n - 1),
            ColOrRow::Num(_) => Err(err("열 번호는 1부터입니다")),
            ColOrRow::Name(s) => match s.trim().parse::<u32>() {
                Ok(n) if n >= 1 => Ok(n - 1),
                _ => parse_col(s),
            },
        }
    }

    /// 행 번호 (0부터)
    pub fn row(&self) -> crate::Result<u32> {
        let n = match self {
            ColOrRow::Num(n) => *n,
            ColOrRow::Name(s) => s
                .trim()
                .parse()
                .map_err(|_| err(format!("행 번호가 아닙니다: {s}")))?,
        };
        n.checked_sub(1).ok_or_else(|| err("행 번호는 1부터입니다"))
    }
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

/// 한셀 화면(UI)에서만 막히는 편집: 한셀 자동화(COM)는 보호된 시트의 열 너비와 구조가 보호된 통합 문서의 시트
/// 추가·이름 바꾸기·삭제를 그대로 하지만, 한셀 화면은 보호 설정에 따라 단추·메뉴를 끈다 (보호 대조 실험 — docs/editing.md
/// 7절). [`apply`] 는 한셀 자동화와 같은 결과를 내고, 웹 편집기는 [`apply_ui`] 로 한셀 화면처럼 막는다
pub fn check_ui(wb: &Workbook, e: &Edit) -> crate::Result<()> {
    match e {
        Edit::ColWidth { sheet, .. } | Edit::ColWidthPx { sheet, .. } => {
            guard_action(wb, sheet_index(wb, sheet.as_ref())?, "formatColumns")
        }
        Edit::AddSheet { .. } | Edit::RenameSheet { .. } | Edit::DeleteSheet { .. } => {
            guard_structure(wb)
        }
        _ => Ok(()),
    }
}

/// 한셀 화면과 같은 보호 규칙으로 편집한다 ([`check_ui`] 뒤 [`apply`])
pub fn apply_ui(wb: &mut Workbook, e: &Edit) -> crate::Result<usize> {
    check_ui(wb, e)?;
    apply(wb, e)
}

/// 편집 하나를 적용한다. 바뀐(또는 새로 만든) 시트 번호를 돌려준다. 보호 규칙은 한셀 자동화와 같다 (화면에서만
/// 막히는 것은 [`check_ui`])
pub fn apply(wb: &mut Workbook, e: &Edit) -> crate::Result<usize> {
    filter::fill_thresholds(wb);
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
            let stored = stored_col_width(*width);
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
        Edit::Sort {
            sheet,
            range,
            keys,
            header,
            match_case,
            left_to_right,
        } => {
            let i = si(wb, sheet)?;
            let keys = keys
                .iter()
                .map(|k| {
                    let index = if *left_to_right {
                        k.by.row()?
                    } else {
                        k.by.col()?
                    };
                    Ok(sort::SortKey {
                        index,
                        desc: k.desc,
                    })
                })
                .collect::<crate::Result<Vec<_>>>()?;
            let opt = sort::SortOptions {
                keys,
                header: *header,
                match_case: *match_case,
                left_to_right: *left_to_right,
            };
            sort::sort(wb, i, parse_range(range)?, &opt)?;
            Ok(i)
        }
        Edit::AutoFill {
            sheet,
            range,
            to,
            kind,
        } => {
            let i = si(wb, sheet)?;
            fill::auto_fill(wb, i, parse_range(range)?, parse_range(to)?, *kind)?;
            Ok(i)
        }
        Edit::Series {
            sheet,
            range,
            rows,
            kind,
            date,
            step,
            stop,
            trend,
        } => {
            let i = si(wb, sheet)?;
            let rg = parse_range(range)?;
            let by_rows = rows.unwrap_or(rg.end.col - rg.start.col > rg.end.row - rg.start.row);
            let o = fill::SeriesOptions {
                rows: by_rows,
                kind: *kind,
                date: *date,
                step: *step,
                stop: *stop,
                trend: *trend,
            };
            fill::data_series(wb, i, rg, &o)?;
            Ok(i)
        }
        Edit::Replace {
            sheet,
            find,
            replace,
            match_case,
            whole_cell,
            all,
            range,
        } => {
            let i = si(wb, sheet)?;
            let o = find::FindOptions {
                what: find.clone(),
                match_case: *match_case,
                whole: *whole_cell,
                in_values: false,
                all_sheets: *all,
                range: range.as_deref().map(parse_range).transpose()?,
            };
            find::replace_all(wb, i, &o, replace)?;
            Ok(i)
        }
        Edit::HideRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            view::hide_rows(wb, i, r, r + count.max(&1) - 1, true)?;
            Ok(i)
        }
        Edit::UnhideRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            view::hide_rows(wb, i, r, r + count.max(&1) - 1, false)?;
            Ok(i)
        }
        Edit::GroupRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            view::group_rows(wb, i, r, r + count.max(&1) - 1, true)?;
            Ok(i)
        }
        Edit::UngroupRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            view::group_rows(wb, i, r, r + count.max(&1) - 1, false)?;
            Ok(i)
        }
        Edit::HideCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            view::hide_cols(wb, i, c, c + count.max(&1) - 1, true)?;
            Ok(i)
        }
        Edit::UnhideCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            view::hide_cols(wb, i, c, c + count.max(&1) - 1, false)?;
            Ok(i)
        }
        Edit::GroupCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            view::group_cols(wb, i, c, c + count.max(&1) - 1, true)?;
            Ok(i)
        }
        Edit::UngroupCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            view::group_cols(wb, i, c, c + count.max(&1) - 1, false)?;
            Ok(i)
        }
        Edit::PasteSpecial {
            sheet,
            from,
            to,
            to_sheet,
            paste,
            operation,
            skip_blanks,
            transpose,
        } => {
            let i = si(wb, sheet)?;
            let d = match to_sheet {
                Some(s) => sheet_index(wb, Some(s))?,
                None => i,
            };
            let o = special::PasteOptions {
                what: *paste,
                op: *operation,
                skip_blanks: *skip_blanks,
                transpose: *transpose,
            };
            special::paste_special(wb, i, parse_range(from)?, d, parse_range(to)?, &o)?;
            Ok(d)
        }
        Edit::AddComment {
            sheet,
            cell,
            text,
            author,
        } => {
            let i = si(wb, sheet)?;
            objects::add_comment(wb, i, cell.parse()?, author, text)?;
            Ok(i)
        }
        Edit::EditComment { sheet, cell, text } => {
            let i = si(wb, sheet)?;
            objects::edit_comment(wb, i, cell.parse()?, text)?;
            Ok(i)
        }
        Edit::DeleteComment { sheet, range } => {
            let i = si(wb, sheet)?;
            objects::delete_comments(wb, i, parse_range(range)?)?;
            Ok(i)
        }
        Edit::AddHyperlink {
            sheet,
            cell,
            address,
            location,
            tooltip,
            display,
        } => {
            let i = si(wb, sheet)?;
            objects::add_hyperlink(
                wb,
                i,
                cell.parse()?,
                address.as_deref(),
                location.as_deref(),
                tooltip.as_deref(),
                display.as_deref(),
            )?;
            Ok(i)
        }
        Edit::RemoveHyperlink { sheet, range } => {
            let i = si(wb, sheet)?;
            objects::remove_hyperlinks(wb, i, parse_range(range)?)?;
            Ok(i)
        }
        Edit::AddName {
            sheet,
            name,
            refers_to,
            local,
        } => {
            let i = si(wb, sheet)?;
            // 시트 없는 참조: 시트를 고르지 않았으면 활성 시트 (한셀 Names.Add 관측)
            let target = match sheet {
                Some(_) => i,
                None => wb
                    .view
                    .as_ref()
                    .map(|v| v.active_tab as usize)
                    .filter(|&a| a < wb.sheets.len())
                    .unwrap_or(i),
            };
            objects::add_name(wb, target, local.then_some(i), name, refers_to)?;
            Ok(i)
        }
        Edit::DeleteName { sheet, name, local } => {
            let i = si(wb, sheet)?;
            objects::delete_name(wb, local.then_some(i), name)?;
            Ok(i)
        }
        Edit::AddValidation {
            sheet,
            range,
            kind,
            operator,
            formula1,
            formula2,
            error_style,
            prompt,
            error,
        } => {
            let i = si(wb, sheet)?;
            let f1 = if kind == "list" && !formula1.starts_with('=') {
                format!("\"{formula1}\"")
            } else {
                formula1.trim_start_matches('=').to_string()
            };
            let dv = crate::model::DataValidation {
                kind: kind.clone(),
                operator: operator.clone(),
                formula1: Some(f1),
                formula2: formula2
                    .as_ref()
                    .map(|f| f.trim_start_matches('=').to_string()),
                allow_blank: true,
                show_input: true,
                show_error: true,
                error_style: error_style.clone(),
                prompt: prompt.clone(),
                error: error.clone(),
                ..Default::default()
            };
            objects::add_validation(wb, i, parse_range(range)?, dv)?;
            Ok(i)
        }
        Edit::DeleteValidation { sheet, range } => {
            let i = si(wb, sheet)?;
            objects::delete_validation(wb, i, parse_range(range)?)?;
            Ok(i)
        }
        Edit::AddCf {
            sheet,
            range,
            kind,
            operator,
            formula1,
            formula2,
            fill,
            font_color,
            bold,
        } => {
            let i = si(wb, sheet)?;
            let formulas = formula1.iter().chain(formula2.iter()).cloned().collect();
            objects::add_cf(
                wb,
                i,
                parse_range(range)?,
                kind,
                operator.as_deref(),
                formulas,
                fill.as_deref(),
                font_color.as_deref(),
                *bold,
            )?;
            Ok(i)
        }
        Edit::DeleteCf { sheet, range } => {
            let i = si(wb, sheet)?;
            objects::delete_cf(wb, i, parse_range(range)?)?;
            Ok(i)
        }
        Edit::AutoFitCols { sheet, col, count } => {
            let i = si(wb, sheet)?;
            let c = parse_col(col)?;
            objects::autofit_cols(wb, i, c, c + count.max(&1) - 1)?;
            Ok(i)
        }
        Edit::RemoveDuplicates {
            sheet,
            range,
            columns,
            header,
        } => {
            let i = si(wb, sheet)?;
            data::remove_duplicates(wb, i, parse_range(range)?, columns, *header)?;
            Ok(i)
        }
        Edit::TextToColumns {
            sheet,
            range,
            dest,
            tab,
            semicolon,
            comma,
            space,
            other,
            consecutive,
            qualifier,
            fields,
        } => {
            let i = si(wb, sheet)?;
            let opt = data::SplitOptions {
                tab: *tab,
                semicolon: *semicolon,
                comma: *comma,
                space: *space,
                other: other.as_deref().and_then(|t| t.chars().next()),
                consecutive: *consecutive,
                qualifier: match qualifier.as_deref() {
                    None => Some('"'),
                    Some(q) => q.chars().next(),
                },
                fields: fields
                    .iter()
                    .map(|f| {
                        (
                            f.col,
                            match f.kind.as_str() {
                                "text" => data::FieldKind::Text,
                                "skip" => data::FieldKind::Skip,
                                _ => data::FieldKind::General,
                            },
                        )
                    })
                    .collect(),
            };
            let d = dest.as_deref().map(|t| t.parse::<CellRef>()).transpose()?;
            data::text_to_columns(wb, i, parse_range(range)?, d, &opt)?;
            Ok(i)
        }
        Edit::Subtotal {
            sheet,
            range,
            group_by,
            func,
            totals,
            replace,
            page_breaks,
            summary_below,
        } => {
            let i = si(wb, sheet)?;
            let o = data::SubtotalOptions {
                group_by: *group_by,
                func: func.clone().unwrap_or_else(|| "sum".into()),
                totals: totals.clone(),
                replace: *replace,
                page_breaks: *page_breaks,
                summary_below: *summary_below,
            };
            data::subtotal(wb, i, parse_range(range)?, &o)?;
            Ok(i)
        }
        Edit::MoveShape {
            sheet,
            shape,
            left,
            top,
            width,
            height,
        } => {
            let i = si(wb, sheet)?;
            shapes::move_shape(wb, i, shape, *left, *top, *width, *height)?;
            Ok(i)
        }
        Edit::AddShape {
            sheet,
            kind,
            left,
            top,
            width,
            height,
        } => {
            let i = si(wb, sheet)?;
            shapes::add_shape(wb, i, kind, *left, *top, *width, *height)?;
            Ok(i)
        }
        Edit::DeleteShape { sheet, shape } => {
            let i = si(wb, sheet)?;
            shapes::delete_shape(wb, i, shape)?;
            Ok(i)
        }
        Edit::Consolidate {
            sheet,
            dest,
            sources,
            func,
            top_row,
            left_column,
        } => {
            let i = si(wb, sheet)?;
            let mut src = Vec::new();
            for t in sources {
                // `시트!A1:B5` (시트 이름은 작은따옴표로 묶을 수 있다), 시트가 없으면 대상 시트
                let (sh, r) = match t.rfind('!') {
                    Some(k) => {
                        let name = t[..k].trim_matches('\'').to_string();
                        (sheet_index(wb, Some(&SheetSel::Name(name)))?, &t[k + 1..])
                    }
                    None => (i, t.as_str()),
                };
                src.push((sh, parse_range(&r.replace('$', ""))?));
            }
            let o = data::ConsolidateOptions {
                func: func.clone().unwrap_or_else(|| "sum".into()),
                top_row: *top_row,
                left_column: *left_column,
            };
            data::consolidate(wb, i, dest.parse::<CellRef>()?, &src, &o)?;
            Ok(i)
        }
        Edit::AutoFitRows { sheet, row, count } => {
            let i = si(wb, sheet)?;
            let r = row.saturating_sub(1);
            objects::autofit_rows(wb, i, r, r + count.max(&1) - 1)?;
            Ok(i)
        }
        Edit::InsertCells {
            sheet,
            range,
            shift,
        } => {
            let i = si(wb, sheet)?;
            if !matches!(shift, cells::ShiftDir::Down | cells::ShiftDir::Right) {
                return Err(err("셀 삽입은 아래(down)·오른쪽(right)으로 밉니다"));
            }
            cells::shift_cells(wb, i, parse_range(range)?, *shift)?;
            Ok(i)
        }
        Edit::DeleteCells {
            sheet,
            range,
            shift,
        } => {
            let i = si(wb, sheet)?;
            if !matches!(shift, cells::ShiftDir::Up | cells::ShiftDir::Left) {
                return Err(err("셀 삭제는 위(up)·왼쪽(left)으로 당깁니다"));
            }
            cells::shift_cells(wb, i, parse_range(range)?, *shift)?;
            Ok(i)
        }
        Edit::HideSheet { sheet } => {
            let i = sheet_index(wb, Some(sheet))?;
            view::hide_sheet(wb, i, true)?;
            Ok(i)
        }
        Edit::UnhideSheet { sheet } => {
            let i = sheet_index(wb, Some(sheet))?;
            view::hide_sheet(wb, i, false)?;
            Ok(i)
        }
        Edit::Freeze { sheet, cell } => {
            let i = si(wb, sheet)?;
            view::freeze(wb, i, cell.parse()?)?;
            Ok(i)
        }
        Edit::Unfreeze { sheet } => {
            let i = si(wb, sheet)?;
            view::unfreeze(wb, i)?;
            Ok(i)
        }
        Edit::AutoFilter {
            sheet,
            range,
            field,
            criteria1,
            operator,
            criteria2,
        } => {
            let i = si(wb, sheet)?;
            filter::auto_filter(
                wb,
                i,
                parse_range(range)?,
                *field,
                criteria1.as_deref(),
                operator.as_deref(),
                criteria2.as_deref(),
            )?;
            Ok(i)
        }
        Edit::ShowAllData { sheet } => {
            let i = si(wb, sheet)?;
            filter::show_all_data(wb, i)?;
            Ok(i)
        }
        Edit::AddTable { sheet, range, o } => {
            let i = si(wb, sheet)?;
            table::add_table(wb, i, parse_range(range)?, o)?;
            Ok(i)
        }
        Edit::PageSetup { sheet, p } => {
            let i = si(wb, sheet)?;
            page::page_setup(wb, i, p)?;
            Ok(i)
        }
        Edit::ProtectSheet { sheet, o } => {
            let i = si(wb, sheet)?;
            protect::protect_sheet(wb, i, o)?;
            Ok(i)
        }
        Edit::UnprotectSheet { sheet, password } => {
            let i = si(wb, sheet)?;
            protect::unprotect_sheet(wb, i, password)?;
            Ok(i)
        }
        Edit::ProtectWorkbook {
            password,
            structure,
            windows: _,
        } => {
            protect::protect_workbook(wb, password, *structure)?;
            Ok(0)
        }
        Edit::UnprotectWorkbook { password } => {
            protect::unprotect_workbook(wb, password)?;
            Ok(0)
        }
        Edit::SetView {
            sheet,
            zoom,
            gridlines,
            headings,
            zeros,
            view,
        } => {
            let i = si(wb, sheet)?;
            view::set_view(wb, i, *zoom, *gridlines, *headings, *zeros, view.as_deref())?;
            Ok(i)
        }
        Edit::CopySheet { sheet, to, name } => {
            let i = si(wb, sheet)?;
            copy_sheet(wb, i, *to, name.as_deref())
        }
        Edit::TabColor { sheet, color } => {
            let i = si(wb, sheet)?;
            view::tab_color(wb, i, color)?;
            Ok(i)
        }
        Edit::SortFilter {
            sheet,
            col,
            desc,
            match_case,
        } => {
            let i = si(wb, sheet)?;
            sort::sort_filter(wb, i, parse_col(col)?, *desc, *match_case)?;
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

// ---------------------------------------------------------------------------
// 보호 (한셀: 보호된 시트의 잠긴 칸·막힌 동작은 "변경하려는 셀은 보호되어 있으므로 읽기 전용입니다")
// ---------------------------------------------------------------------------

fn sheet_protected(wb: &Workbook, si: usize) -> Option<&crate::model::SheetProtection> {
    wb.sheets
        .get(si)?
        .protection
        .as_ref()
        .filter(|p| p.flag("sheet"))
}

/// 보호된 시트에서 범위 안에 잠긴 칸이 있으면 편집할 수 없다
pub fn guard_cells(wb: &Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    if sheet_protected(wb, si).is_none() {
        return Ok(());
    }
    let sheet = &wb.sheets[si];
    let locked = |st: u32| {
        wb.styles
            .xf(st)
            .and_then(|x| x.protection.as_ref())
            .and_then(|p| p.locked)
            != Some(false)
    };
    let (r0, r1, c0, c1) = (
        range.start.row,
        range.end.row,
        range.start.col,
        range.end.col,
    );
    // 셀 기록이 있는 칸, 행 서식이 있는 행, 그 밖의 칸(열 정보의 열 서식, 없으면 기본 서식)으로 나눠 본다 —
    // 행 전체·열 전체 범위도 칸마다 보지 않는다
    let cells = sheet
        .cells
        .iter()
        .filter(|c| (r0..=r1).contains(&c.r.row) && (c0..=c1).contains(&c.r.col))
        .any(|c| locked(c.style));
    let styled_rows: Vec<(u32, u32)> = sheet
        .rows
        .iter()
        .filter_map(|r| Some((r.index - 1, r.style?)))
        .filter(|(i, _)| (r0..=r1).contains(i))
        .collect();
    let rows = styled_rows.iter().any(|&(_, st)| locked(st));
    let plain_rows = (r1 - r0 + 1) as usize > styled_rows.len();
    let cols = plain_rows && {
        let mut covered = 0u64;
        let mut any = false;
        for ci in &sheet.cols {
            let (a, b) = ((ci.min - 1).max(c0), (ci.max - 1).min(c1));
            if a > b {
                continue;
            }
            covered += (b - a + 1) as u64;
            any |= locked(ci.style.unwrap_or(0));
        }
        any || (covered < (c1 - c0 + 1) as u64 && locked(0))
    };
    let any_locked = cells || rows || cols;
    if any_locked {
        return Err(err(
            "변경하려는 셀은 보호되어 있으므로 읽기 전용입니다 (시트 보호)",
        ));
    }
    Ok(())
}

/// 보호된 시트에서 막힌 동작 (`formatCells`, `insertRows` … — [`crate::model::SHEET_PROTECTION_FLAGS`])
pub fn guard_action(wb: &Workbook, si: usize, flag: &str) -> crate::Result<()> {
    match sheet_protected(wb, si) {
        Some(p) if p.flag(flag) => Err(err("보호된 시트에서는 할 수 없습니다 (시트 보호)")),
        _ => Ok(()),
    }
}

/// 통합 문서 구조가 보호되면 시트를 더하거나 지우거나 옮기거나 이름을 바꿀 수 없다
pub fn guard_structure(wb: &Workbook) -> crate::Result<()> {
    if wb.protection.as_ref().is_some_and(|p| p.lock_structure) {
        return Err(err("통합 문서 구조가 보호되어 있습니다"));
    }
    Ok(())
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

/// 수식 입력줄에 보일 글: 수식은 `=…` (배열 수식은 `{=…}`), 날짜는 `yyyy-mm-dd`(시각이 있으면 ` h:mm:ss`),
/// 시각은 `h:mm:ss`, 백분율 서식은 `10%`, 글자 접두가 있는 글은 `'…`, 숫자는 유효숫자 15자리
pub fn input_text(wb: &Workbook, si: usize, r: CellRef) -> String {
    let Some(sheet) = wb.sheets.get(si) else {
        return String::new();
    };
    let Some(c) = sheet.cell(r) else {
        return String::new();
    };
    if let Some(f) = c.formula.as_ref().filter(|f| !f.text.is_empty()) {
        return match f.kind {
            FormulaKind::Array => format!("{{={}}}", f.text),
            _ => format!("={}", f.text),
        };
    }
    let xf = wb.styles.xf(c.style);
    let code = xf
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    match &c.value {
        CellValue::Empty => String::new(),
        CellValue::Text(t) if xf.is_some_and(|x| x.quote_prefix) => format!("'{t}"),
        CellValue::Number(v) => {
            let v = *v;
            match input::category(&code) {
                input::Cat::Date | input::Cat::Time if (0.0..2_958_466.0).contains(&v) => {
                    let date = crate::calc::date::ymd_from_serial(v.floor(), wb.date1904);
                    let (h, m, s) = crate::calc::date::hms_from_serial(v);
                    let time = format!("{h}:{m:02}:{s:02}");
                    match (input::category(&code), date) {
                        (input::Cat::Time, _) if v < 1.0 => time,
                        (_, Some((y, mo, d))) if v.fract() == 0.0 => {
                            format!("{y:04}-{mo:02}-{d:02}")
                        }
                        (_, Some((y, mo, d))) => format!("{y:04}-{mo:02}-{d:02} {time}"),
                        _ => crate::model::format_general(v),
                    }
                }
                input::Cat::Percent => format!(
                    "{}%",
                    crate::model::format_general(crate::calc::value::round15(v * 100.0))
                ),
                _ => crate::model::format_general(v),
            }
        }
        v => v.display(),
    }
}

/// Ctrl+방향키: 값이 이어진 끝으로, 빈 칸에서는 다음 값이 있는 칸으로, 없으면 시트 끝으로 (`dr`·`dc` 는 −1·0·1)
pub fn jump(sheet: &Sheet, r: CellRef, dr: i32, dc: i32) -> CellRef {
    let filled = |row: i64, col: i64| -> bool {
        row >= 0
            && col >= 0
            && sheet
                .cell(CellRef {
                    row: row as u32,
                    col: col as u32,
                })
                .is_some_and(|c| !c.value.is_empty() || c.formula.is_some())
    };
    let max_r = refs::MAX_ROWS as i64 - 1;
    let max_c = refs::MAX_COLS as i64 - 1;
    let inside = |row: i64, col: i64| (0..=max_r).contains(&row) && (0..=max_c).contains(&col);
    let (mut row, mut col) = (r.row as i64, r.col as i64);
    let (dr, dc) = (dr as i64, dc as i64);
    if !inside(row + dr, col + dc) {
        return r;
    }
    // 이 줄에서 값이 있는 칸들 (시트 끝까지 하나씩 보지 않도록)
    let line: std::collections::BTreeSet<i64> = sheet
        .cells
        .iter()
        .filter(|c| !c.value.is_empty() || c.formula.is_some())
        .filter(|c| {
            if dr != 0 {
                c.r.col as i64 == col
            } else {
                c.r.row as i64 == row
            }
        })
        .map(|c| {
            if dr != 0 {
                c.r.row as i64
            } else {
                c.r.col as i64
            }
        })
        .collect();
    let pos = if dr != 0 { row } else { col };
    let step = dr + dc;
    let edge = if step > 0 {
        if dr != 0 { max_r } else { max_c }
    } else {
        0
    };
    let target = if filled(row, col) && filled(row + dr, col + dc) {
        // 이어진 값의 끝
        let mut p = pos;
        while p != edge && line.contains(&(p + step)) {
            p += step;
        }
        p
    } else {
        // 다음 값이 있는 칸 (없으면 끝)
        let next = if step > 0 {
            line.range(pos + 1..).next().copied()
        } else {
            line.range(..pos).next_back().copied()
        };
        next.unwrap_or(edge)
    };
    if dr != 0 {
        row = target;
    } else {
        col = target;
    }
    CellRef {
        row: row as u32,
        col: col as u32,
    }
}

/// 입력한 수식을 한셀이 저장하는 꼴로 고르고 문법을 확인한다 (`=` 없이)
pub fn normalize_formula(wb: &Workbook, text: &str) -> crate::Result<String> {
    // `==1` 은 수식이 아니다 (한셀: 입력은 오류, CSV 열기는 글)
    if text.trim_start().starts_with('=') {
        return Err(err(format!("수식 오류: ={text}")));
    }
    let canon = canon_name(wb);
    let local = |sheet: &str, name: &str| {
        wb.defined_names.iter().any(|d| {
            refs::same_sheet(&d.name, name)
                && d.local_sheet_id
                    .and_then(|i| wb.sheets.get(i as usize))
                    .is_some_and(|s| refs::same_sheet(&s.name, sheet))
        })
    };
    let n = refs::normalize_scoped(text, &canon, &local);
    if n.invalid {
        return Err(err(format!("수식 오류: ={text}")));
    }
    let expr = crate::calc::parse(&n.text).map_err(|e| err(format!("수식 오류: {e}")))?;
    // 한셀은 입력한 수식의 인수 개수가 함수와 맞지 않으면 거부한다 (`LEN(,"")`, `COUNTIF("x")`; 한셀이 모르는 함수는 그대로)
    let mut bad_arity = false;
    // 존재하지 않는 표의 구조적 참조, 참조가 아닌 값의 합집합 (`1,5`, `(1,2)`) 도 거부한다
    expr.visit_refs(&mut |r| {
        if let crate::calc::ast::RefOrName::Table(t) = r
            && !t.table.is_empty()
            && !wb.sheets.iter().any(|s| {
                s.tables
                    .iter()
                    .any(|x| refs::same_sheet(&x.display_name, &t.table))
            })
        {
            bad_arity = true;
        }
    });
    fn bad_union(e: &crate::calc::ast::Expr) -> bool {
        use crate::calc::ast::{BinaryOp, Expr};
        match e {
            Expr::Binary(BinaryOp::Union, a, b) => {
                !(crate::calc::eval::is_reference_expr(a)
                    && crate::calc::eval::is_reference_expr(b))
                    || bad_union(a)
                    || bad_union(b)
            }
            Expr::Binary(_, a, b) => bad_union(a) || bad_union(b),
            Expr::Unary(_, a) | Expr::Paren(a) => bad_union(a),
            Expr::Call(c) => c.args.iter().any(bad_union),
            _ => false,
        }
    }
    if bad_union(&expr) {
        bad_arity = true;
    }
    expr.visit_calls(&mut |c| {
        if let Some(spec) = c.func
            && !crate::calc::functions::HCELL_UNKNOWN.contains(&c.name.as_str())
            && (c.args.len() < spec.min as usize || c.args.len() > spec.max as usize)
        {
            bad_arity = true;
        }
    });
    if bad_arity {
        return Err(err(format!("수식 오류: ={text}")));
    }
    Ok(n.text)
}

/// 한셀 `FormulaLocal` 의 함수 이름 (한국어판): `Formula` 와 같되 세 함수만 다르다 — DBCS → JUNJA,
/// DOLLAR → WON, USDOLLAR → DOLLAR (함수 목록 전체를 한셀에 넣어 대조한 결과). 글자 상수 안은 건드리지 않는다
pub fn formula_local(text: &str) -> String {
    let ch: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_str) = (0, false);
    while i < ch.len() {
        let c = ch[i];
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            i += 1;
        } else if !in_str && (c.is_alphanumeric() || c == '_' || c == '.') {
            let mut j = i;
            while j < ch.len() && (ch[j].is_alphanumeric() || ch[j] == '_' || ch[j] == '.') {
                j += 1;
            }
            let w: String = ch[i..j].iter().collect();
            let mut k = j;
            while k < ch.len() && ch[k].is_whitespace() {
                k += 1;
            }
            let local = if ch.get(k) == Some(&'(') {
                match w.to_ascii_uppercase().as_str() {
                    "DBCS" => "JUNJA",
                    "DOLLAR" => "WON",
                    "USDOLLAR" => "DOLLAR",
                    _ => &w,
                }
                .to_string()
            } else {
                w
            };
            out.push_str(&local);
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// [`normalize_formula`] 과 같고 닫지 않은 괄호도 오류 (한셀의 CSV 열기: `=SUM(` 은 글)
pub fn normalize_formula_strict(wb: &Workbook, text: &str) -> crate::Result<String> {
    if refs::normalize_checked(text, &canon_name(wb)).unclosed {
        return Err(err(format!("수식 오류: ={text}")));
    }
    normalize_formula(wb, text)
}

/// 셀에 입력한다 (한셀 입력 규칙). 빈 글이면 내용을 지운다
pub fn set_input(wb: &mut Workbook, si: usize, r: CellRef, text: &str) -> crate::Result<()> {
    guard_cells(wb, si, CellRange { start: r, end: r })?;
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
            // 줄 바꿈이 든 글을 넣으면 한셀은 칸 서식에 자동 줄 바꿈을 켠다 (`'` 앞붙임·`\r\n` 도 같다 — 실험)
            let cur_wrap = wb
                .styles
                .xf(style)
                .and_then(|x| x.alignment.as_ref())
                .is_some_and(|a| a.wrap_text);
            let need_wrap = !cur_wrap && matches!(&value, CellValue::Text(t) if t.contains('\n'));
            let new_style = if num_fmt.is_some() || cur_qp != quote_prefix || need_wrap {
                Restyler::default().apply(&mut wb.styles, style, &|_, x| {
                    if let Some(id) = num_fmt {
                        x.num_fmt_id = id;
                        x.apply_number_format = Some(true);
                    }
                    x.quote_prefix = quote_prefix;
                    if need_wrap {
                        x.alignment.get_or_insert_with(Default::default).wrap_text = true;
                        x.apply_alignment = Some(true);
                    }
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
/// 숫자·백분율 형식을 버리고, SUM 은 날짜 범위의 형식을 버린다)
pub fn formula_format(wb: &Workbook, si: usize, text: &str) -> Option<u32> {
    use crate::calc::ast::{BinaryOp, Expr, SheetSpec, UnaryOp};
    let e = crate::calc::parse(text).ok()?;
    // 통화 기호가 있는 숫자 형식은 통화 (곱해도 남는다)
    let cat_of = |id: u32| {
        let code = crate::numfmt::code_for(&wb.styles, id);
        match input::category(&code) {
            input::Cat::Number if code.contains(['₩', '$', '€', '£', '¥', '￦']) => {
                input::Cat::Currency
            }
            c => c,
        }
    };
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
            // 곱하기·나누기·거듭제곱: 숫자·백분율 형식은 버리고 통화·날짜·시각·지수·분수는 남긴다. 같은 형식끼리면
            // 백분율·지수·분수만 남는다 (입력 대조 실험 3: X*2, X/2, X^2, X*X)
            Expr::Binary(BinaryOp::Mul | BinaryOp::Div | BinaryOp::Pow, a, b) => {
                use input::Cat;
                let (fa, fb) = (walk(a, ref_fmt, cat_of), walk(b, ref_fmt, cat_of));
                match (fa, fb) {
                    (Some(x), Some(y)) if cat_of(x) == cat_of(y) => {
                        matches!(cat_of(x), Cat::Percent | Cat::Scientific | Cat::Fraction)
                            .then_some(x)
                    }
                    (Some(x), _) | (None, Some(x)) => {
                        (!matches!(cat_of(x), Cat::Number | Cat::Percent)).then_some(x)
                    }
                    _ => None,
                }
            }
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
    match kind {
        ClearKind::Formats => guard_action(wb, si, "formatCells")?,
        _ => guard_cells(wb, si, range)?,
    }
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
    guard_action(wb, si, "insertRows")?;
    structure::shift(wb, si, Shift::InsRows { at, n: n.max(1) })
}

pub fn delete_rows(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    guard_action(wb, si, "deleteRows")?;
    // 행 삭제가 허용된 보호 시트도 지울 행의 칸이 모두 잠기지 않아야 한다 (한셀 — 보호 대조 실험)
    guard_cells(
        wb,
        si,
        CellRange {
            start: CellRef { row: at, col: 0 },
            end: CellRef {
                row: (at + n.max(1) - 1).min(refs::MAX_ROWS - 1),
                col: refs::MAX_COLS - 1,
            },
        },
    )?;
    structure::shift(wb, si, Shift::DelRows { at, n: n.max(1) })
}

pub fn insert_cols(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    guard_action(wb, si, "insertColumns")?;
    structure::shift(wb, si, Shift::InsCols { at, n: n.max(1) })
}

pub fn delete_cols(wb: &mut Workbook, si: usize, at: u32, n: u32) -> crate::Result<()> {
    guard_action(wb, si, "deleteColumns")?;
    // 열 삭제가 허용된 보호 시트도 지울 열의 칸이 모두 잠기지 않아야 한다
    guard_cells(
        wb,
        si,
        CellRange {
            start: CellRef { row: 0, col: at },
            end: CellRef {
                row: refs::MAX_ROWS - 1,
                col: (at + n.max(1) - 1).min(refs::MAX_COLS - 1),
            },
        },
    )?;
    structure::shift(wb, si, Shift::DelCols { at, n: n.max(1) })
}

/// 한셀 COM `ColumnWidth`(글자 수) → 저장하는 열 너비: 1/256 로 자르고 199/256 을 더한다 — 글꼴과 상관없다
/// (대조 실험: 20 → 20.77734375, 8.43 → 9.20703125, Arial 10 문서에서도 18 → 18.77734375)
pub fn stored_col_width(chars: f64) -> f64 {
    if chars < 1.0 {
        return (chars * 256.0).floor() / 256.0;
    }
    ((chars * 256.0).floor() + 199.0) / 256.0
}

/// 열 너비 (저장 값, 글자 수). 0 이하면 숨긴다
pub fn set_col_width(
    wb: &mut Workbook,
    si: usize,
    c0: u32,
    c1: u32,
    width: f64,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    // 한셀 자동화(COM `ColumnWidth`)는 열 서식이 막힌 보호 시트에서도 너비를 바꾼다 — 화면에서는 막힌다 (`check_ui`)
    let k = crate::render::book_metrics(wb).k;
    let old: Vec<f64> = (c0..=c1.min(c0 + 16_384))
        .map(|c| col_px_of(wb, si, c, k))
        .collect();
    let before = one_cell_before(wb, si);
    let sheet = &mut wb.sheets[si];
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
    let new: Vec<f64> = (c0..=c1.min(c0 + 16_384))
        .map(|c| col_px_of(wb, si, c, k))
        .collect();
    structure::scale_offsets(&mut wb.sheets[si], false, c0, &old, &new);
    one_cell_after(wb, si, &before);
    Ok(())
}

/// 열 너비·행 높이를 바꾸기 전의 배치: 크기를 지키는 그림(`editAs="oneCell"`)의 크기, 메모 상자를 둘 화면 자리
pub(crate) struct Before {
    sizes: Vec<Option<(i64, i64)>>,
    old: Option<crate::render::Geometry>,
    ext: (u32, u32),
}

pub(crate) fn one_cell_before(wb: &Workbook, si: usize) -> Before {
    let sheet = &wb.sheets[si];
    let pictures = sheet
        .shapes
        .iter()
        .any(|s| s.anchor.edit_as.as_deref() == Some("oneCell"));
    let boxes = sheet.comments.iter().any(|c| c.shape.is_some());
    if !pictures && !boxes {
        return Before {
            sizes: Vec::new(),
            old: None,
            ext: (0, 0),
        };
    }
    let (r1, c1) = structure::shapes_extent(sheet);
    let (r2, c2) = structure::comments_extent(sheet);
    let ext = (r1.max(r2) + 64, c1.max(c2) + 32);
    let g = crate::render::sheet_geometry(wb, sheet, ext.0, ext.1);
    Before {
        sizes: structure::one_cell_sizes(sheet, &g),
        old: Some(g),
        ext,
    }
}

/// 행·열을 숨기거나 보인 뒤: 크기를 지키는 그림의 끝 표지와 메모 상자를 다시 정한다 (한셀 — 7·8행을 숨기면 그 행에 걸친 그림
/// 끝이 8행에서 10행으로)
pub(crate) fn one_cell_pictures_after(wb: &mut Workbook, si: usize, before: &Before) {
    if before.old.is_none() {
        return;
    }
    let g = crate::render::sheet_geometry(wb, &wb.sheets[si], before.ext.0, before.ext.1);
    structure::keep_one_cell_sizes(&mut wb.sheets[si], &before.sizes, &g);
    structure::snap_from_hidden(&mut wb.sheets[si], &before.sizes, &g);
    // 메모 상자는 화면 자리를 지킨다 (숨긴 행·열 때문에 칸 표지만 바뀐다 — 한셀 대조 실험 cb2)
    if let Some(old) = &before.old {
        structure::keep_comment_boxes(&mut wb.sheets[si], old, &g);
    }
}

fn one_cell_after(wb: &mut Workbook, si: usize, before: &Before) {
    let Some(old) = &before.old else { return };
    let g = crate::render::sheet_geometry(wb, &wb.sheets[si], before.ext.0, before.ext.1);
    structure::keep_one_cell_sizes(&mut wb.sheets[si], &before.sizes, &g);
    structure::keep_comment_boxes(&mut wb.sheets[si], old, &g);
}

/// 열의 화면 너비 (px, 렌더러와 같은 규칙)
fn col_px_of(wb: &Workbook, si: usize, c: u32, k: f64) -> f64 {
    let sheet = &wb.sheets[si];
    match sheet
        .cols
        .iter()
        .find(|x| (x.min..=x.max).contains(&(c + 1)))
    {
        Some(x) if x.hidden => 0.0,
        Some(x) if x.width.is_some() => crate::render::col_px(x.width.unwrap_or(0.0), k) as f64,
        _ => crate::render::default_col_px(sheet, k) as f64,
    }
}

/// 행 높이 (pt). 0 이하면 숨긴다
pub fn set_row_height(
    wb: &mut Workbook,
    si: usize,
    r0: u32,
    r1: u32,
    height: f64,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    guard_action(wb, si, "formatRows")?;
    let r1 = r1.min(refs::MAX_ROWS - 1);
    let span = r1.min(r0 + 100_000);
    let geo = |wb: &Workbook| -> Vec<f64> {
        let g = crate::render::sheet_geometry(wb, &wb.sheets[si], span + 1, 1);
        (r0..=span).map(|r| g.rows[r as usize].1 as f64).collect()
    };
    let old = geo(wb);
    let before = one_cell_before(wb, si);
    let sheet = &mut wb.sheets[si];
    for r in r0..=r1 {
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
    let new = geo(wb);
    structure::scale_offsets(&mut wb.sheets[si], true, r0, &old, &new);
    one_cell_after(wb, si, &before);
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
    let skip = format_skips(&wb.sheets[si], range);
    for r in r0..=r1 {
        for c in c0..=c1 {
            let at = CellRef { row: r, col: c };
            if skip(at) {
                continue;
            }
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

/// 서식을 바꾸지 않는 칸: 조건이 있는 자동 필터 범위 안의 숨긴 행, 범위 밖으로 걸친 병합의 칸 (한셀은 이런 칸을
/// 건너뛴다 — 서식 대조 실험: 필터가 걸린 시트에서는 숨긴 행을 건너뛰고, 필터가 없거나 조건이 없으면 개요로 접은
/// 행·열도 바꾼다. 숨긴 열은 언제나 바꾼다)
fn format_skips(sheet: &Sheet, range: CellRange) -> impl Fn(CellRef) -> bool + use<> {
    let filtered = filter::has_criteria(sheet)
        .then(|| {
            sheet
                .auto_filter
                .as_ref()
                .and_then(|af| refs::parse_area(&af.range))
                .map(|a| {
                    let (r0, _, r1, _) = a.bounds();
                    (r0 + 1, r1)
                })
        })
        .flatten();
    let hidden_rows: std::collections::HashSet<u32> = sheet
        .rows
        .iter()
        .filter(|r| r.hidden)
        .map(|r| r.index - 1)
        .filter(|&i| filtered.is_some_and(|(a, b)| (a..=b).contains(&i)))
        .collect();
    let partial: Vec<CellRange> = sheet
        .merges
        .iter()
        .filter(|m| {
            let overlaps = !(m.end.row < range.start.row
                || m.start.row > range.end.row
                || m.end.col < range.start.col
                || m.start.col > range.end.col);
            overlaps && !(range.contains(m.start) && range.contains(m.end))
        })
        .copied()
        .collect();
    move |at: CellRef| hidden_rows.contains(&at.row) || partial.iter().any(|m| m.contains(at))
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
    guard_action(wb, si, "formatCells")?;
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
    guard_action(wb, si, "formatCells")?;
    let e = style::edge(line, color);
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    let whole = (c0 == 0 && c1 >= refs::MAX_COLS - 1) || (r0 == 0 && r1 >= refs::MAX_ROWS - 1);
    if whole || (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64 > 1_000_000 {
        return restyle(wb, si, range, &|st, xf, at| {
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
        });
    }
    let skip = format_skips(&wb.sheets[si], range);
    let mut plan: Vec<(CellRef, style::EdgeSet)> = Vec::new();
    if preset == BorderPreset::None {
        let set = style::edges_for(preset, &e, true, true, true, true);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let at = CellRef { row: r, col: c };
                if !skip(at) {
                    plan.push((at, set.clone()));
                }
            }
        }
        apply_border_plan(wb, si, plan);
        structure::update_dimension(&mut wb.sheets[si]);
        return Ok(());
    }
    use Side::{Bottom, Left, Right, Top};
    let cr = |row: u32, col: u32| CellRef { row, col };
    let sheet = &wb.sheets[si];
    let styles = &wb.styles;
    // 변 묶음 (한셀 COM Borders 의 여섯 묶음): 왼쪽, 위, 아래, 오른쪽, 안쪽 세로, 안쪽 가로
    let on: [bool; 6] = match preset {
        BorderPreset::All => [true, true, true, true, c1 > c0, r1 > r0],
        BorderPreset::Outline => [true, true, true, true, false, false],
        BorderPreset::Inside => [false, false, false, false, c1 > c0, r1 > r0],
        BorderPreset::Left => [true, false, false, false, false, false],
        BorderPreset::Top => [false, true, false, false, false, false],
        BorderPreset::Bottom => [false, false, true, false, false, false],
        BorderPreset::Right => [false, false, false, true, false, false],
        BorderPreset::None => [false; 6],
    };
    // 색을 주지 않으면 묶음마다 첫 칸의 그 변 색 — 선이 있으면 선 색(이웃 칸의 맞닿은 선 포함), 없으면 칸 서식에
    // 남은 색. 한셀 문서의 기본 테두리는 왼쪽·오른쪽·아래 검정, 위 자동이다 (테두리 대조 실험)
    let colors: [crate::model::Color; 6] = match &e.color {
        Some(c) => std::array::from_fn(|_| c.clone()),
        None => {
            // 안쪽 선은 건너뛰는 칸(범위 밖으로 걸친 병합)이 없는 첫 행에서 본다
            let rr = (r0..=r1)
                .find(|&r| (c0..=c1).all(|c| !skip(cr(r, c))))
                .unwrap_or(r0);
            // 안쪽 세로: 첫 칸 사이에 선이 있고 그 행의 칸 사이(끝 칸의 오른쪽 변까지)에 있는 선이 모두 같은 색이면 그 색
            // (선 없는 칸 사이는 보지 않는다 — H10 오른쪽만 빨강이면 빨강), 아니면 끝 칸 왼쪽 변의 색
            let lines: Vec<Option<crate::model::BorderEdge>> = (c0..=c1)
                .map(|c| line_between(sheet, styles, cr(rr, c), Right))
                .collect();
            let inside_v = match lines.first() {
                Some(Some(first))
                    if lines
                        .iter()
                        .flatten()
                        .all(|l| same_color(&edge_color(l), &edge_color(first))) =>
                {
                    edge_color(first)
                }
                _ => record_color(sheet, styles, cr(rr, c1), Left),
            };
            // 안쪽 가로: 그 행 끝 칸의 서식에 남은 아래 변 색 (첫 행 아래 선이 빨강·파랑·초록이면 초록, 끝 칸에 선이 없으면
            // 기본 검정 — 테두리 대조 실험. 아래 칸의 위 선은 보지 않는다)
            let inside_h = record_color(sheet, styles, cr(rr, c1), Bottom);
            [
                side_color(sheet, styles, cr(r0, c0), Left),
                side_color(sheet, styles, cr(r0, c0), Top),
                side_color(sheet, styles, cr(r1, c0), Bottom),
                side_color(sheet, styles, cr(r0, c1), Right),
                inside_v,
                inside_h,
            ]
        }
    };
    let want: [crate::model::BorderEdge; 6] = std::array::from_fn(|g| crate::model::BorderEdge {
        style: e.style.clone(),
        color: Some(colors[g].clone()),
    });
    // 묶음의 칸 사이 선이 모두 이미 같은 선이면 그 묶음은 그대로 둔다 (한셀 — 이웃 칸의 선이 같아도)
    let same_all = |cells: &mut dyn Iterator<Item = (CellRef, Side)>,
                    w: &crate::model::BorderEdge| {
        let mut ok = true;
        for (at, side) in cells {
            if !line_between(sheet, styles, at, side).is_some_and(|l| same_line(&l, w)) {
                ok = false;
                break;
            }
        }
        ok
    };
    let mut apply = on;
    if on[0] {
        apply[0] = !same_all(&mut (r0..=r1).map(|r| (cr(r, c0), Left)), &want[0]);
    }
    if on[1] {
        apply[1] = !same_all(&mut (c0..=c1).map(|c| (cr(r0, c), Top)), &want[1]);
    }
    if on[2] {
        apply[2] = !same_all(&mut (c0..=c1).map(|c| (cr(r1, c), Bottom)), &want[2]);
    }
    if on[3] {
        apply[3] = !same_all(&mut (r0..=r1).map(|r| (cr(r, c1), Right)), &want[3]);
    }
    if on[4] {
        apply[4] = !same_all(
            &mut (r0..=r1).flat_map(|r| (c0..c1).map(move |c| (cr(r, c), Right))),
            &want[4],
        );
    }
    if on[5] {
        apply[5] = !same_all(
            &mut (r0..r1).flat_map(|r| (c0..=c1).map(move |c| (cr(r, c), Bottom))),
            &want[5],
        );
    }
    let mut neighbors: Vec<(CellRef, style::EdgeSet)> = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            let at = cr(r, c);
            if skip(at) {
                continue;
            }
            let pick = |outer: bool, go: usize, gi: usize| -> Option<usize> {
                let g = if outer { go } else { gi };
                apply[g].then_some(g)
            };
            let mut set = style::EdgeSet::default();
            for (side, g) in [
                (Left, pick(c == c0, 0, 4)),
                (Top, pick(r == r0, 1, 5)),
                (Bottom, pick(r == r1, 2, 5)),
                (Right, pick(c == c1, 3, 4)),
            ] {
                let Some(g) = g else { continue };
                *side.slot(&mut set) = Some(Some(want[g].clone()));
                // 범위 가장자리: 이웃 칸의 맞닿은 선은 새 선과 다르면 지우고 같으면 둔다 (한셀은 칸 사이 선을 하나로
                // 다룬다 — B10 위 선을 그으면 B9 의 아래 선이 없어진다, 테두리 대조 실험)
                if g < 4
                    && let Some(n) = side.step(at)
                {
                    let theirs = border_ref(sheet, styles, n).and_then(|b| side.opposite().of(b));
                    if is_line(theirs) && !theirs.is_some_and(|t| same_line(t, &want[g])) {
                        let mut ns = style::EdgeSet::default();
                        *side.opposite().slot(&mut ns) = Some(None);
                        neighbors.push((n, ns));
                    }
                }
            }
            if !set.is_empty() {
                plan.push((at, set));
            }
        }
    }
    plan.extend(neighbors);
    apply_border_plan(wb, si, plan);
    structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

/// 칸 서식 바꾸기 목록을 차례로 적용한다 (같은 서식·같은 바꾸기는 한 번만 계산)
fn apply_border_plan(wb: &mut Workbook, si: usize, plan: Vec<(CellRef, style::EdgeSet)>) {
    let mut memo: std::collections::HashMap<(u32, String), u32> = std::collections::HashMap::new();
    for (at, set) in plan {
        let old = style_at(&wb.sheets[si], at);
        let key = (old, format!("{set:?}"));
        let new = match memo.get(&key) {
            Some(&n) => n,
            None => {
                let mut xf = wb.styles.xf(old).cloned().unwrap_or_default();
                style::patch_border(&mut wb.styles, &mut xf, &set);
                let n = style::intern_xf(&mut wb.styles, xf);
                memo.insert(key, n);
                n
            }
        };
        if new != old || wb.sheets[si].cell(at).is_some() {
            cell_mut(&mut wb.sheets[si], at).style = new;
        }
    }
}

/// 칸의 변
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }

    /// 이 변 너머의 칸 (시트 밖이면 None)
    fn step(self, at: CellRef) -> Option<CellRef> {
        let (row, col) = (at.row, at.col);
        match self {
            Side::Left => col.checked_sub(1).map(|col| CellRef { row, col }),
            Side::Right => (col + 1 < refs::MAX_COLS).then_some(CellRef { row, col: col + 1 }),
            Side::Top => row.checked_sub(1).map(|row| CellRef { row, col }),
            Side::Bottom => (row + 1 < refs::MAX_ROWS).then_some(CellRef { row: row + 1, col }),
        }
    }

    fn of(self, b: &crate::model::Border) -> Option<&crate::model::BorderEdge> {
        match self {
            Side::Left => b.left.as_ref(),
            Side::Right => b.right.as_ref(),
            Side::Top => b.top.as_ref(),
            Side::Bottom => b.bottom.as_ref(),
        }
    }

    fn slot(self, s: &mut style::EdgeSet) -> &mut Option<Option<crate::model::BorderEdge>> {
        match self {
            Side::Left => &mut s.left,
            Side::Right => &mut s.right,
            Side::Top => &mut s.top,
            Side::Bottom => &mut s.bottom,
        }
    }
}

fn border_ref<'a>(
    sheet: &Sheet,
    styles: &'a crate::model::Styles,
    at: CellRef,
) -> Option<&'a crate::model::Border> {
    styles
        .xf(style_at(sheet, at))
        .and_then(|x| styles.borders.get(x.border_id as usize))
}

fn is_line(e: Option<&crate::model::BorderEdge>) -> bool {
    e.is_some_and(|e| e.style.as_deref().is_some_and(|s| s != "none"))
}

/// 칸 사이의 선: 칸의 변에 선이 있으면 그것, 없으면 이웃 칸의 맞닿은 변의 선
fn line_between(
    sheet: &Sheet,
    styles: &crate::model::Styles,
    at: CellRef,
    side: Side,
) -> Option<crate::model::BorderEdge> {
    let own = border_ref(sheet, styles, at).and_then(|b| side.of(b));
    if is_line(own) {
        return own.cloned();
    }
    let n = side.step(at)?;
    let theirs = border_ref(sheet, styles, n).and_then(|b| side.opposite().of(b));
    if is_line(theirs) {
        theirs.cloned()
    } else {
        None
    }
}

fn auto_color() -> crate::model::Color {
    crate::model::Color {
        auto: true,
        ..Default::default()
    }
}

fn edge_color(e: &crate::model::BorderEdge) -> crate::model::Color {
    e.color.clone().unwrap_or_else(auto_color)
}

/// 칸 서식에 남은 변의 색 (선이 없어도 색은 있을 수 있다 — 없으면 자동)
fn record_color(
    sheet: &Sheet,
    styles: &crate::model::Styles,
    at: CellRef,
    side: Side,
) -> crate::model::Color {
    border_ref(sheet, styles, at)
        .and_then(|b| side.of(b))
        .and_then(|e| e.color.clone())
        .unwrap_or_else(auto_color)
}

/// 변의 색: 칸 사이에 선이 있으면 그 선의 색, 없으면 칸 서식에 남은 색
fn side_color(
    sheet: &Sheet,
    styles: &crate::model::Styles,
    at: CellRef,
    side: Side,
) -> crate::model::Color {
    match line_between(sheet, styles, at, side) {
        Some(l) => edge_color(&l),
        None => record_color(sheet, styles, at, side),
    }
}

fn same_color(a: &crate::model::Color, b: &crate::model::Color) -> bool {
    let key = |c: &crate::model::Color| {
        (
            c.auto,
            c.rgb.as_ref().map(|s| s.to_ascii_uppercase()),
            c.theme,
            c.indexed,
            c.tint.map(|t| (t * 1e6).round() as i64),
        )
    };
    key(a) == key(b)
}

fn same_line(a: &crate::model::BorderEdge, b: &crate::model::BorderEdge) -> bool {
    a.style == b.style && same_color(&edge_color(a), &edge_color(b))
}

/// 병합 (왼쪽 위 칸의 값만 남는다 — 한셀). 겹치는 병합은 먼저 푼다
pub fn merge(wb: &mut Workbook, si: usize, range: CellRange, center: bool) -> crate::Result<()> {
    if range.start == range.end {
        return Ok(());
    }
    guard_action(wb, si, "formatCells")?;
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
        // 왼쪽 위 칸이 비었으면 처음 만나는 값(행 우선)을 왼쪽 위 칸으로 옮긴다 (한셀 — 병합 대조 실험)
        let first = sheet
            .cells
            .iter()
            .filter(|c| range.contains(c.r) && (!c.value.is_empty() || c.formula.is_some()))
            .map(|c| (c.r, c.value.clone(), c.formula.clone()))
            .next();
        if let Some((at, value, formula)) = first
            && at != tl
        {
            // 값 칸의 서식(글꼴·채우기·맞춤·표시 형식)도 함께 오고, 테두리는 왼쪽 위 칸의 것을 둔다 (병합 대조 실험)
            let from = style_at(sheet, at);
            let own = style_at(sheet, tl);
            let own_border = wb.styles.xf(own).map(|x| (x.border_id, x.apply_border));
            let style = match (wb.styles.xf(from).cloned(), own_border) {
                (Some(mut xf), Some((border_id, apply_border))) if from != own => {
                    xf.border_id = border_id;
                    xf.apply_border = apply_border;
                    style::intern_xf(&mut wb.styles, xf)
                }
                _ => from,
            };
            let c = cell_mut(sheet, tl);
            c.value = value;
            c.formula = formula;
            c.style = style;
        }
        for c in &mut sheet.cells {
            if range.contains(c.r) && c.r != tl {
                c.value = CellValue::Empty;
                c.formula = None;
            }
        }
        sheet.merges.push(range);
        // 자동 높이가 적힌 행: 첫 행은 그 높이를 고정 높이로, 나머지 행은 높이를 지운다 (한셀 — 병합 대조 실험)
        for info in &mut sheet.rows {
            let i = info.index - 1;
            if i < range.start.row || i > range.end.row || info.custom_height {
                continue;
            }
            if i == range.start.row {
                info.custom_height = info.height.is_some();
            } else {
                info.height = None;
            }
        }
    }
    // 병합 안쪽의 칸 사이 선은 지운다 (한셀 — 병합 대조 실험)
    clear_inner_edges(wb, si, range);
    if center {
        let p = FormatPatch {
            h_align: Some("center".into()),
            ..Default::default()
        };
        format(wb, si, range, &p)?;
    }
    Ok(())
}

/// 범위 안쪽의 칸 사이 선(바깥 선이 아닌 변)을 지운다
fn clear_inner_edges(wb: &mut Workbook, si: usize, range: CellRange) {
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    if (r1 - r0 + 1) as u64 * (c1 - c0 + 1) as u64 > 100_000 {
        return;
    }
    let mut memo: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            let at = CellRef { row: r, col: c };
            let old = style_at(&wb.sheets[si], at);
            let kind = (r > r0) as u8
                | ((r < r1) as u8) << 1
                | ((c > c0) as u8) << 2
                | ((c < c1) as u8) << 3;
            let has = wb
                .styles
                .xf(old)
                .and_then(|x| wb.styles.borders.get(x.border_id as usize))
                .is_some_and(|b| {
                    let on = |e: &Option<crate::model::BorderEdge>| {
                        e.as_ref()
                            .is_some_and(|e| e.style.as_deref().is_some_and(|s| s != "none"))
                    };
                    (r > r0 && on(&b.top))
                        || (r < r1 && on(&b.bottom))
                        || (c > c0 && on(&b.left))
                        || (c < c1 && on(&b.right))
                });
            if !has {
                continue;
            }
            let new = *memo.entry((old, kind)).or_insert_with(|| {
                let mut xf = wb.styles.xf(old).cloned().unwrap_or_default();
                let set = style::EdgeSet {
                    top: (r > r0).then_some(None),
                    bottom: (r < r1).then_some(None),
                    left: (c > c0).then_some(None),
                    right: (c < c1).then_some(None),
                    clear_diagonal: false,
                };
                style::patch_border(&mut wb.styles, &mut xf, &set);
                style::intern_xf(&mut wb.styles, xf)
            });
            cell_mut(&mut wb.sheets[si], at).style = new;
        }
    }
}

/// 병합 풀기 (범위와 겹치는 병합 모두)
pub fn unmerge(wb: &mut Workbook, si: usize, range: CellRange) -> crate::Result<()> {
    guard_action(wb, si, "formatCells")?;
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
    // 활성 시트는 선택된 시트를 따른다. 탭 줄의 첫 시트는 자리 그대로 (한셀)
    let selected = wb.sheets.iter().position(|s| s.view.tab_selected);
    let n = wb.sheets.len();
    if let Some(v) = &mut wb.view {
        if let Some(i) = selected {
            v.active_tab = i as u32;
        }
        if v.first_sheet as usize >= n {
            v.first_sheet = 0;
        }
    }
}

/// 시트 하나를 선택한다 (시트 탭)
pub fn select_sheet(wb: &mut Workbook, si: usize) {
    for (i, s) in wb.sheets.iter_mut().enumerate() {
        s.view.tab_selected = i == si;
    }
    if let Some(v) = &mut wb.view {
        v.active_tab = si as u32;
    }
}

/// 코드 이름이 없는 워크시트에 쓰지 않은 가장 작은 `SheetN` 을 차례로 준다 — 한셀은 문서를 열 때 코드 이름을
/// 정하므로, 시트를 더하거나 지우거나 옮긴 뒤에도 처음 순서대로의 이름이 남는다 (편집 대조 실험: 코드 이름 없는
/// 문서에 시트를 더해 앞으로 옮기면 원래 시트가 `Sheet1`, 새 시트가 `Sheet2`)
fn ensure_code_names(wb: &mut Workbook) {
    let missing = |s: &Sheet| s.kind == crate::model::SheetKind::Worksheet && s.code_name.is_none();
    if !wb.sheets.iter().any(missing) {
        return;
    }
    for i in 0..wb.sheets.len() {
        if !missing(&wb.sheets[i]) {
            continue;
        }
        let name = (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|n| {
                !wb.sheets.iter().any(|s| {
                    s.code_name
                        .as_deref()
                        .is_some_and(|c| refs::same_sheet(c, n))
                })
            })
            .unwrap_or_default();
        wb.sheets[i].code_name = Some(name);
    }
}

/// 시트를 더한다 (`at` 자리에). 이름이 없으면 한셀처럼 쓰지 않은 가장 작은 `SheetN`
/// 한셀의 새 통합 문서 (시트 하나 `Sheet1`, 기본 글꼴 맑은 고딕 11, 표준 스타일, 세로 가운데 맞춤)
pub fn new_workbook() -> Workbook {
    use crate::model::{Alignment, Fill, Font, NamedStyle, Styles, Xf};
    let mut wb = Workbook {
        application: Some("rcell".into()),
        styles: Styles {
            fonts: vec![Font {
                name: Some("맑은 고딕".into()),
                size: Some(11.0),
                ..Default::default()
            }],
            fills: vec![
                Fill {
                    pattern_type: Some("none".into()),
                    ..Default::default()
                },
                Fill {
                    pattern_type: Some("gray125".into()),
                    ..Default::default()
                },
            ],
            borders: vec![Default::default()],
            cell_style_xfs: vec![Default::default()],
            cell_xfs: vec![Xf {
                xf_id: Some(0),
                alignment: Some(Alignment {
                    vertical: Some("center".into()),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            cell_styles: vec![NamedStyle {
                name: "표준".into(),
                xf_id: 0,
                builtin_id: Some(0),
                i_level: None,
            }],
            table_styles: Some(crate::model::TableStyles {
                default_table_style: Some("TableStyleMedium2".into()),
                default_pivot_style: Some("PivotStyleLight16".into()),
                styles: Vec::new(),
            }),
            ..Default::default()
        },
        // 한셀의 새 문서는 탭 영역 비율 500
        view: Some(crate::model::WorkbookView {
            tab_ratio: 500,
            ..Default::default()
        }),
        code_name: Some("ThisWorkbook".into()),
        ..Default::default()
    };
    let _ = add_sheet(&mut wb, Some("Sheet1"), 0);
    wb.sheets[0].view.tab_selected = true;
    wb.sheets[0].default_col_width = Some(9.00390625);
    wb.sheets[0].default_row_height = Some(16.5);
    wb
}

pub fn add_sheet(wb: &mut Workbook, name: Option<&str>, at: usize) -> crate::Result<usize> {
    // 구조가 보호된 통합 문서: 한셀 자동화는 시트를 더한다 — 화면에서는 막힌다 (`check_ui`)
    ensure_code_names(wb);
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
    // 코드 이름: 문서의 시트에 코드 이름이 있으면 한셀처럼 쓰지 않은 가장 작은 `SheetN`
    let code_name = wb.sheets.iter().any(|s| s.code_name.is_some()).then(|| {
        (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|n| {
                !wb.sheets.iter().any(|s| {
                    s.code_name
                        .as_deref()
                        .is_some_and(|c| refs::same_sheet(c, n))
                })
            })
            .unwrap_or_default()
    });
    let sheet = Sheet {
        name,
        sheet_id,
        part,
        code_name,
        show_grid_lines: true,
        default_col_width: model.and_then(|m| m.default_col_width),
        default_row_height: model.and_then(|m| m.default_row_height),
        default_row_custom: model.is_some_and(|m| m.default_row_custom),
        page: hcell_page(),
        dimension: Some("A1".into()),
        ..Default::default()
    };
    wb.sheets.insert(at, sheet);
    remap_sheet_indexes(wb, &|i| Some(if i >= at { i + 1 } else { i }));
    // 한셀 자동화는 새 시트를 선택하지 않는다 (시트 대조 실험 — 화면에서 더하면 웹 편집기가 새 시트로 간다)
    Ok(at)
}

/// 시트 복사 (한셀 `Worksheet.Copy` 대조 실험 cs1 — docs/editing.md 23절): 복사본은 `원본 (2)` 처럼 `(N)` 을 붙인 이름(가장 작은
/// N ≥ 2) 으로 `at`(원본 목록 기준 자리, 기본은 원본 바로 뒤) 앞에 놓이고 선택된다. 그 시트의 지역 이름과 그 시트를 가리키는
/// 전역 이름은 복사본의 지역 이름으로도 두며 (참조는 복사본으로), 복사본 안의 수식은 그대로다.
pub fn copy_sheet(
    wb: &mut Workbook,
    si: usize,
    at: Option<usize>,
    name: Option<&str>,
) -> crate::Result<usize> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    // 구조가 보호된 통합 문서에서 한셀의 시트 복사는 아무 일도 하지 않는다 (시트 추가는 되지만 — 대조 실험)
    if wb.protection.as_ref().is_some_and(|p| p.lock_structure) {
        return Ok(si);
    }
    ensure_code_names(wb);
    let src = wb.sheets[si].clone();
    let name = match name {
        Some(n) => {
            check_sheet_name(wb, n, None)?;
            n.to_string()
        }
        None => (2..)
            .map(|n| format!("{} ({n})", src.name))
            .find(|n| !wb.sheets.iter().any(|s| refs::same_sheet(&s.name, n)))
            .unwrap_or_default(),
    };
    let at = at.unwrap_or(si + 1).min(wb.sheets.len());
    let sheet_id = wb.sheets.iter().map(|s| s.sheet_id).max().unwrap_or(0) + 1;
    let part = (1..)
        .map(|n| format!("xl/worksheets/sheet{}.xml", sheet_id + n - 1))
        .find(|p| !wb.sheets.iter().any(|s| &s.part == p))
        .unwrap_or_default();
    let code_name = src.code_name.as_ref().map(|_| {
        (1..)
            .map(|n| format!("Sheet{n}"))
            .find(|n| {
                !wb.sheets.iter().any(|s| {
                    s.code_name
                        .as_deref()
                        .is_some_and(|c| refs::same_sheet(c, n))
                })
            })
            .unwrap_or_default()
    });
    let mut copy = src.clone();
    copy.name = name.clone();
    copy.sheet_id = sheet_id;
    copy.part = part;
    copy.code_name = code_name;
    copy.view.tab_selected = false;
    // 복사본의 메모 상자 이름은 `메모 0` 이 된다 (한셀 — 가끔 쓰레기 값이 들어가기도 한다, corpus 대조)
    for c in &mut copy.comments {
        if let Some(sh) = &mut c.shape
            && sh.name.is_some()
        {
            sh.name = Some("메모 0".into());
        }
    }
    // 복사본 안에서 원본 시트를 이름으로 가리키는 수식은 복사본을 가리키게 한다 (다른 시트를 가리키는 참조는 그대로)
    for c in &mut copy.cells {
        if let Some(f) = &mut c.formula {
            f.text = refs::rename_sheet(&f.text, &src.name, &name);
        }
    }
    // 차트: 복사본의 차트는 파트를 따로 두고 (계열 참조 중 원본 시트는 복사본으로)
    let retarget = |x: &str| refs::rename_sheet(x, &src.name, &name);
    let mut new_parts: Vec<crate::model::RawPart> = Vec::new();
    {
        fn chart_no(n: &str) -> Option<u32> {
            n.rsplit("charts/chart")
                .next()
                .and_then(|t| t.strip_suffix(".xml"))
                .and_then(|t| t.parse().ok())
        }
        let mut next = wb
            .raw_parts
            .iter()
            .filter_map(|p| chart_no(&p.name))
            .max()
            .unwrap_or(0)
            + 1;
        fn walk(
            list: &mut [crate::model::Shape],
            wb: &Workbook,
            next: &mut u32,
            new_parts: &mut Vec<crate::model::RawPart>,
            f: &dyn Fn(&str) -> String,
        ) {
            for sh in list {
                if let Some(c) = &mut sh.chart {
                    structure::map_chart(c, f);
                }
                for rel in &mut sh.raw_rels {
                    if !rel.1.ends_with("/chart") {
                        continue;
                    }
                    let file = rel.2.rsplit('/').next().unwrap_or("").to_string();
                    let Some(part) = wb
                        .raw_parts
                        .iter()
                        .find(|p| p.name.ends_with(&format!("charts/{file}")))
                    else {
                        continue;
                    };
                    let new_file = format!("chart{next}.xml");
                    let new_name = format!("xl/charts/{new_file}");
                    let data = structure::patch_chart_formulas(&part.data, f)
                        .unwrap_or_else(|| part.data.clone());
                    new_parts.push(crate::model::RawPart {
                        name: new_name.clone(),
                        content_type: part.content_type.clone(),
                        data,
                    });
                    // 차트 파트의 관계 파일도 같이
                    if let Some(rp) = wb
                        .raw_parts
                        .iter()
                        .find(|p| p.name == format!("xl/charts/_rels/{file}.rels"))
                    {
                        new_parts.push(crate::model::RawPart {
                            name: format!("xl/charts/_rels/{new_file}.rels"),
                            content_type: rp.content_type.clone(),
                            data: rp.data.clone(),
                        });
                    }
                    rel.2 = rel.2.replacen(&file, &new_file, 1);
                    if let Some(t) = &mut sh.target {
                        *t = t.replacen(&file, &new_file, 1);
                    }
                    *next += 1;
                }
                walk(&mut sh.children, wb, next, new_parts, f);
            }
        }
        walk(&mut copy.shapes, wb, &mut next, &mut new_parts, &retarget);
    }
    wb.raw_parts.extend(new_parts);
    // 이름: 원본의 지역 이름과 원본을 가리키는 전역 이름
    let mut extra: Vec<crate::model::DefinedName> = Vec::new();
    for d in &wb.defined_names {
        let mine = d.local_sheet_id == Some(si as u32);
        let refers = d.local_sheet_id.is_none()
            && !d.name.starts_with("_xlnm.")
            && refs::rename_sheet(&d.formula, &src.name, &name) != d.formula;
        if mine || refers {
            let mut n = d.clone();
            n.formula = refs::rename_sheet(&d.formula, &src.name, &name);
            extra.push(n);
        }
    }
    wb.sheets.insert(at, copy);
    remap_sheet_indexes(wb, &|i| Some(if i >= at { i + 1 } else { i }));
    for mut d in extra {
        d.local_sheet_id = Some(at as u32);
        wb.defined_names.push(d);
    }
    // 한셀 자동화의 복사는 복사본을 선택한다
    select_sheet(wb, at);
    Ok(at)
}

/// 시트 이름을 바꾼다 (그 시트를 가리키는 수식·이름·차트·하이퍼링크도)
pub fn rename_sheet(wb: &mut Workbook, si: usize, name: &str) -> crate::Result<()> {
    // 구조가 보호된 통합 문서: 한셀 자동화는 이름을 바꾼다 — 화면에서는 막힌다 (`check_ui`)
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
    // 구조가 보호된 통합 문서: 한셀 자동화는 시트를 지운다 — 화면에서는 막힌다 (`check_ui`)
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
    ensure_code_names(wb);
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

/// 시트를 옮긴다 (`to` = 옮긴 뒤의 자리). 옮긴 시트를 선택한다
pub fn move_sheet(wb: &mut Workbook, from: usize, to: usize) -> crate::Result<()> {
    guard_structure(wb)?;
    if from >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let to = to.min(wb.sheets.len() - 1);
    if from == to {
        return Ok(());
    }
    ensure_code_names(wb);
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
    // 한셀 자동화는 옮긴 시트를 선택한다 (시트 대조 실험 — 시트를 더한 뒤 앞으로 옮기면 그 시트가 선택된다)
    select_sheet(wb, to);
    Ok(())
}

#[cfg(test)]
mod tests;
