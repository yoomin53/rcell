// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 데이터 도구: 중복 항목 제거, 텍스트 나누기 (한셀 COM `Range.RemoveDuplicates`·`Range.TextToColumns` 대조 실험 —
//! docs/editing.md 18절).
//!
//! - 중복 항목 제거: 범위의 행을 고른 열의 값으로 견준다 (글은 대소문자 무시, 공백은 그대로, 수와 글 `1`·`'1` 은 다름,
//!   빈 칸끼리·같은 논리값·오류끼리 같음, 표시 형식이 달라도 값이 같아야 같음). 첫 행만 남기고 범위 안 열의 칸을
//!   (수식 글 그대로, 서식과 함께) 위로 당기며, 모자란 끝 행은 내용·서식을 지운다. 범위 밖 열과 행 속성은 그대로.
//!   병합이 걸쳐 있으면 할 수 없다.
//! - 텍스트 나누기: 한 열의 칸마다 글을 구분 문자로 나눠 목적지(없으면 그 열)에 입력 규칙대로 넣는다. 열 형식이 글이면
//!   `@` 서식의 글로, 건너뜀이면 넣지 않는다. 빈 필드는 그 칸의 내용을 지운다. 큰따옴표(또는 작은따옴표)로 묶인 필드 안의
//!   구분 문자는 글자 그대로, 짝 없는 따옴표는 끝까지가 한 필드, 따옴표 두 개는 따옴표 하나.

use super::{cell_mut, err, set_input};
use crate::model::{CellRange, CellRef, CellValue, Workbook};

/// 비교 키의 한 부분
#[derive(PartialEq, Eq, Hash, Clone)]
enum Part {
    Empty,
    Num(u64),
    Text(String),
    Bool(bool),
    Err(String),
}

fn part_of(v: &CellValue) -> Part {
    match v {
        CellValue::Empty => Part::Empty,
        CellValue::Number(n) => Part::Num(if *n == 0.0 { 0 } else { n.to_bits() }),
        CellValue::Text(t) => Part::Text(t.to_lowercase()),
        CellValue::Bool(b) => Part::Bool(*b),
        CellValue::Error(e) => Part::Err(e.to_uppercase()),
        CellValue::Date(d) => Part::Text(d.to_lowercase()),
    }
}

/// 중복 항목 제거. `columns` 는 범위 안 열 번호(1부터), 비면 모든 열. `header` 면 첫 행은 견주지 않는다
pub fn remove_duplicates(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    columns: &[u32],
    header: bool,
) -> crate::Result<usize> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    {
        let sheet = &wb.sheets[si];
        if sheet
            .merges
            .iter()
            .any(|m| !(m.end.row < r0 || m.start.row > r1 || m.end.col < c0 || m.start.col > c1))
        {
            return Err(err("병합된 셀이 있으면 중복 항목을 제거할 수 없습니다"));
        }
        if sheet.cells.iter().any(|c| {
            c.formula.as_ref().is_some_and(|f| {
                matches!(
                    f.kind,
                    crate::model::FormulaKind::Array | crate::model::FormulaKind::DataTable
                )
            }) && (r0..=r1).contains(&c.r.row)
                && (c0..=c1).contains(&c.r.col)
        }) {
            return Err(err("배열 수식이 있으면 중복 항목을 제거할 수 없습니다"));
        }
    }
    // 한셀 자동화는 시트 보호를 보지 않는다 (보호된 시트에서도 지운다 — 실험)
    crate::calc::recalculate(wb);
    super::structure::unshare(&mut wb.sheets[si]);
    let width = c1 - c0 + 1;
    let cols: Vec<u32> = if columns.is_empty() {
        (0..width).collect()
    } else {
        columns
            .iter()
            .filter(|&&k| k >= 1 && k <= width)
            .map(|k| k - 1)
            .collect()
    };
    let first = r0 + header as u32;
    // 남길 행 → 새 행
    let mut seen: std::collections::HashSet<Vec<Part>> = Default::default();
    let mut map: std::collections::HashMap<u32, u32> = Default::default();
    let mut next = first;
    {
        let sheet = &wb.sheets[si];
        for r in first..=r1 {
            let key: Vec<Part> = cols
                .iter()
                .map(|&k| {
                    part_of(
                        &sheet
                            .cell(CellRef {
                                row: r,
                                col: c0 + k,
                            })
                            .map(|c| c.value.clone())
                            .unwrap_or_default(),
                    )
                })
                .collect();
            if seen.insert(key) {
                map.insert(r, next);
                next += 1;
            }
        }
    }
    let removed = (r1 + 1 - next) as usize;
    // 범위에 값이 하나도 없으면 아무것도 하지 않는다 (한셀 — 빈 칸뿐인 범위는 그대로, 서식도 지우지 않는다)
    let any_value = wb.sheets[si].cells.iter().any(|c| {
        (r0..=r1).contains(&c.r.row)
            && (c0..=c1).contains(&c.r.col)
            && (!c.value.is_empty() || c.formula.is_some())
    });
    if removed == 0 || !any_value {
        return Ok(0);
    }
    let sheet = &mut wb.sheets[si];
    let inside = |r: CellRef| r.row >= first && r.row <= r1 && r.col >= c0 && r.col <= c1;
    sheet.cells.retain_mut(|c| {
        if !inside(c.r) {
            return true;
        }
        match map.get(&c.r.row) {
            Some(&n) => {
                c.r.row = n;
                true
            }
            None => false,
        }
    });
    // 모자란 끝 칸은 내용과 서식을 지운다 — 한셀은 기본 서식(0번)으로 되돌린다 (열 서식이 있어도)
    for row in next..=r1 {
        for col in c0..=c1 {
            if sheet.effective_style(row, col) != 0 {
                sheet.cells.push(crate::model::Cell {
                    r: CellRef { row, col },
                    style: 0,
                    value: CellValue::Empty,
                    formula: None,
                });
            }
        }
    }
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    // 메모·하이퍼링크는 칸을 따라간다 (지워진 행의 것은 없어진다)
    sheet.comments.retain_mut(|m| {
        if !inside(m.cell) {
            return true;
        }
        match map.get(&m.cell.row) {
            Some(&n) => {
                m.cell.row = n;
                true
            }
            None => false,
        }
    });
    sheet.hyperlinks.retain_mut(|h| {
        let Some(rg) = CellRange::parse(&h.range) else {
            return true;
        };
        if rg.start != rg.end || !inside(rg.start) {
            return true;
        }
        match map.get(&rg.start.row) {
            Some(&n) => {
                h.range = super::refs::Area::rect(n, rg.start.col, n, rg.start.col).text();
                true
            }
            None => false,
        }
    });
    super::structure::update_dimension(sheet);
    Ok(removed)
}

/// 텍스트 나누기의 열 형식
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldKind {
    #[default]
    General,
    Text,
    Skip,
}

/// 텍스트 나누기 설정 (구분 문자로 나누기)
#[derive(Debug, Clone, Default)]
pub struct SplitOptions {
    pub tab: bool,
    pub semicolon: bool,
    pub comma: bool,
    pub space: bool,
    pub other: Option<char>,
    /// 연속된 구분 문자를 하나로
    pub consecutive: bool,
    /// 묶는 따옴표 (None = 없음)
    pub qualifier: Option<char>,
    /// 필드 번호(1부터)별 열 형식 (없으면 일반)
    pub fields: Vec<(u32, FieldKind)>,
}

fn is_delim(c: char, o: &SplitOptions) -> bool {
    (o.tab && c == '\t')
        || (o.semicolon && c == ';')
        || (o.comma && c == ',')
        || (o.space && c == ' ')
        || o.other == Some(c)
}

/// 글 한 줄을 필드로 나눈다
pub fn split_fields(line: &str, o: &SplitOptions) -> Vec<String> {
    let any = o.tab || o.semicolon || o.comma || o.space || o.other.is_some();
    if !any {
        return vec![line.to_string()];
    }
    let ch: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let mut field = String::new();
        if o.qualifier.is_some() && ch.get(i) == o.qualifier.as_ref() {
            let q = ch[i];
            i += 1;
            // 닫는 따옴표까지 (따옴표 두 개 = 따옴표 하나, 짝이 없으면 끝까지)
            while i < ch.len() {
                if ch[i] == q {
                    if ch.get(i + 1) == Some(&q) {
                        field.push(q);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                field.push(ch[i]);
                i += 1;
            }
        }
        while i < ch.len() && !is_delim(ch[i], o) {
            field.push(ch[i]);
            i += 1;
        }
        out.push(field);
        if i >= ch.len() {
            break;
        }
        // 구분 문자 (연속이면 이어진 것 모두)
        i += 1;
        if o.consecutive {
            while i < ch.len() && is_delim(ch[i], o) {
                i += 1;
            }
            if i >= ch.len() {
                break;
            }
        }
    }
    out
}

/// 텍스트 나누기: 한 열의 `range` 칸마다 나눠 `dest`(없으면 범위의 첫 칸)부터 오른쪽으로 넣는다
pub fn text_to_columns(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    dest: Option<CellRef>,
    o: &SplitOptions,
) -> crate::Result<()> {
    // 한셀 자동화는 시트 보호를 보지 않는다 (잠긴 칸도 덮어쓴다 — 실험): 입력 규칙을 쓰는 동안 보호를 잠시 푼다
    let kept = wb.sheets.get_mut(si).and_then(|s| s.protection.take());
    let r = text_to_columns_inner(wb, si, range, dest, o);
    if let Some(s) = wb.sheets.get_mut(si) {
        s.protection = kept;
    }
    r
}

fn text_to_columns_inner(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    dest: Option<CellRef>,
    o: &SplitOptions,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    if range.start.col != range.end.col {
        return Err(err("텍스트 나누기는 한 열만 고를 수 있습니다"));
    }
    let dest = dest.unwrap_or(range.start);
    let rows = range.end.row - range.start.row + 1;
    // 원본 글을 먼저 모두 읽는다 (목적지가 원본과 겹쳐도 되도록)
    let mut lines: Vec<Option<String>> = Vec::new();
    for k in 0..rows {
        let r = CellRef {
            row: range.start.row + k,
            col: range.start.col,
        };
        lines.push(super::find::display_text(wb, si, r));
    }
    let kind_of = |n: u32| {
        o.fields
            .iter()
            .find(|(k, _)| *k == n)
            .map_or(FieldKind::General, |(_, f)| *f)
    };
    // 쓸 칸 모두의 보호를 먼저 본다
    let max_fields = lines
        .iter()
        .flatten()
        .map(|l| split_fields(l, o).len())
        .max()
        .unwrap_or(0) as u32;
    if max_fields > 0 {
        let kept = (1..=max_fields)
            .filter(|&n| kind_of(n) != FieldKind::Skip)
            .count() as u32;
        if kept > 0 && dest.col + kept > crate::edit::refs::MAX_COLS {
            return Err(err("텍스트 나누기 결과가 시트를 넘습니다"));
        }
    }
    for (k, line) in lines.iter().enumerate() {
        let Some(line) = line else { continue };
        let row = dest.row + k as u32;
        let mut col = dest.col;
        for (n, field) in split_fields(line, o).into_iter().enumerate() {
            let kind = kind_of(n as u32 + 1);
            if kind == FieldKind::Skip {
                continue;
            }
            let at = CellRef { row, col };
            col += 1;
            match kind {
                FieldKind::Text if !field.is_empty() => {
                    let style = super::style_at(&wb.sheets[si], at);
                    let id = super::style::num_fmt_id(&mut wb.styles, "@");
                    // 줄 바꿈이 든 글은 자동 줄 바꿈도 켠다 (`set_input` 과 같다)
                    let wrap = field.contains('\n');
                    let new =
                        super::style::Restyler::default().apply(&mut wb.styles, style, &|_, x| {
                            x.num_fmt_id = id;
                            x.apply_number_format = Some(true);
                            if wrap {
                                x.alignment.get_or_insert_with(Default::default).wrap_text = true;
                                x.apply_alignment = Some(true);
                            }
                        });
                    let c = cell_mut(&mut wb.sheets[si], at);
                    c.style = new;
                    c.value = CellValue::Text(field);
                    c.formula = None;
                }
                _ => set_input(wb, si, at, &field)?,
            }
        }
    }
    super::structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

/// 부분합 함수 (한셀 `Range.Subtotal` 대조: SUBTOTAL 함수 번호, 소계 글)
fn subtotal_fn(name: &str) -> Option<(u8, &'static str)> {
    Some(match name {
        "sum" => (9, "합계"),
        "count" => (3, "개수"),
        "average" => (1, "평균"),
        "max" => (4, "최댓값"),
        "min" => (5, "최솟값"),
        "product" => (6, "곱"),
        "countNums" => (2, "수치 개수"),
        "stdev" => (7, "표본 표준 편차"),
        "stdevp" => (8, "표준 편차"),
        "var" => (10, "표본 분산"),
        "varp" => (11, "분산"),
        _ => return None,
    })
}

/// 부분합 설정
#[derive(Debug, Clone)]
pub struct SubtotalOptions {
    /// 기준 열 (범위 안 번호, 1부터)
    pub group_by: u32,
    pub func: String,
    /// 합계를 낼 열 (범위 안 번호)
    pub totals: Vec<u32>,
    /// 이미 있는 부분합을 바꾼다
    pub replace: bool,
    pub page_breaks: bool,
    pub summary_below: bool,
}

/// 칸이 든 영역 (빈 행·빈 열로 둘러싸인 덩어리 — 한셀 `CurrentRegion`)
pub fn current_region(sheet: &crate::model::Sheet, at: CellRef) -> CellRange {
    let filled = |r: i64, c: i64| -> bool {
        r >= 0
            && c >= 0
            && sheet
                .cell(CellRef {
                    row: r as u32,
                    col: c as u32,
                })
                .is_some_and(|x| !x.value.is_empty() || x.formula.is_some())
    };
    let (mut r0, mut c0, mut r1, mut c1) =
        (at.row as i64, at.col as i64, at.row as i64, at.col as i64);
    loop {
        let mut grew = false;
        // 둘러싼 행·열(모서리 포함)에 칸이 있으면 넓힌다
        if (c0 - 1..=c1 + 1).any(|c| filled(r0 - 1, c)) {
            r0 -= 1;
            grew = true;
        }
        if (c0 - 1..=c1 + 1).any(|c| filled(r1 + 1, c)) {
            r1 += 1;
            grew = true;
        }
        if (r0 - 1..=r1 + 1).any(|r| filled(r, c0 - 1)) {
            c0 -= 1;
            grew = true;
        }
        if (r0 - 1..=r1 + 1).any(|r| filled(r, c1 + 1)) {
            c1 += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    CellRange {
        start: CellRef {
            row: r0 as u32,
            col: c0 as u32,
        },
        end: CellRef {
            row: r1 as u32,
            col: c1 as u32,
        },
    }
}

/// 새로 넣을 행
struct Item {
    /// 넣을 자리 (원래 행 번호 — 그 행 앞)
    at: u32,
    /// 같은 자리일 때 차례
    seq: u8,
    /// 묶음 번호 (None = 전체 행)
    group: Option<usize>,
}

/// 부분합 (한셀 `Range.Subtotal` 대조 실험 — docs/editing.md 18절): 범위의 첫 행은 머리글. 기준 열 값이 이어진 묶음마다
/// 끝(위에 두면 처음)에 행을 넣고 기준 열에 `값 합계`, 합계 열에 `SUBTOTAL(번호,묶음 범위)` 를 둔다. 맨 끝(위에 두면 머리글
/// 아래)에는 `총 합계`(그 밖의 함수는 `전체 …`)와 전체 범위의 `SUBTOTAL`. 개요: 자료 행 2단계, 부분합 행 1단계, 전체 행
/// 0단계, 요약 행마다 `collapsed`.
pub fn subtotal(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    o: &SubtotalOptions,
) -> crate::Result<()> {
    let (code, label) =
        subtotal_fn(&o.func).ok_or_else(|| err(format!("부분합 함수가 아닙니다: {}", o.func)))?;
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    // 한셀은 고른 범위가 아니라 그 첫 칸의 현재 영역에 부분합을 낸다 (열 번호도 영역 안 번호)
    let range = current_region(&wb.sheets[si], range.start);
    let width = range.end.col - range.start.col + 1;
    if o.group_by < 1 || o.group_by > width || o.totals.iter().any(|&t| t < 1 || t > width) {
        return Err(err("부분합 열이 범위를 벗어났습니다"));
    }
    super::guard_cells(wb, si, range)?;
    let c0 = range.start.col;
    let gcol = c0 + o.group_by - 1;
    let mut r1 = range.end.row;
    // 이미 있는 부분합 행은 지운다
    if o.replace {
        let rows: Vec<u32> = {
            let sheet = &wb.sheets[si];
            (range.start.row..=r1)
                .filter(|&r| {
                    sheet.cells.iter().any(|c| {
                        c.r.row == r
                            && c.formula.as_ref().is_some_and(|f| {
                                f.text.to_ascii_uppercase().starts_with("SUBTOTAL(")
                            })
                    })
                })
                .collect()
        };
        for &r in rows.iter().rev() {
            super::structure::shift(wb, si, super::refs::Shift::DelRows { at: r, n: 1 })?;
            r1 -= 1;
        }
        for r in &mut wb.sheets[si].rows {
            if (range.start.row + 1..=r1 + 1).contains(&r.index) {
                r.outline_level = 0;
                r.collapsed = false;
            }
        }
    }
    if !o.replace
        && wb.sheets[si].cells.iter().any(|c| {
            (range.start.row..=r1).contains(&c.r.row)
                && c.formula
                    .as_ref()
                    .is_some_and(|f| f.text.to_ascii_uppercase().starts_with("SUBTOTAL("))
        })
    {
        return Err(err(
            "이미 부분합이 있는 범위에 겹쳐 내는 것은 지원하지 않습니다",
        ));
    }
    crate::calc::recalculate(wb);
    let first = range.start.row + 1;
    if first > r1 {
        return Err(err("부분합을 낼 자료가 없습니다"));
    }
    // 묶음: 기준 열 값이 이어진 행들 (시작 행, 끝 행)
    let mut groups: Vec<(u32, u32)> = Vec::new();
    {
        let sheet = &wb.sheets[si];
        let key = |r: u32| {
            part_of(
                &sheet
                    .cell(CellRef { row: r, col: gcol })
                    .map(|c| c.value.clone())
                    .unwrap_or_default(),
            )
        };
        let mut start = first;
        for r in first + 1..=r1 + 1 {
            if r > r1 || key(r) != key(start) {
                groups.push((start, r - 1));
                start = r;
            }
        }
    }
    let below = o.summary_below;
    // 묶음 글과 합계 열의 표시 형식 (묶음 마지막 칸의 것)
    let mut keys: Vec<String> = Vec::new();
    let mut fmts: Vec<Vec<u32>> = Vec::new();
    // 표시 형식: 묶음에서 General 이 아닌 첫 칸의 것 (한셀 — 합계 열도 기준 열도), 없으면 General. 전체 행은 전체 범위에서
    let fmt_of = |wb: &Workbook, rows: std::ops::RangeInclusive<u32>, col: u32| -> u32 {
        let sheet = &wb.sheets[si];
        rows.map(|r| {
            let st = sheet
                .cell(CellRef { row: r, col })
                .map_or_else(|| sheet.effective_style(r, col), |c| c.style);
            wb.styles.xf(st).map_or(0, |x| x.num_fmt_id)
        })
        .find(|&f| f != 0)
        .unwrap_or(0)
    };
    let mut key_fmts: Vec<u32> = Vec::new();
    for &(a, b) in &groups {
        keys.push(
            super::find::display_text(wb, si, CellRef { row: a, col: gcol }).unwrap_or_default(),
        );
        key_fmts.push(fmt_of(wb, a..=b, gcol));
        fmts.push(
            o.totals
                .iter()
                .map(|&t| fmt_of(wb, a..=b, c0 + t - 1))
                .collect(),
        );
    }
    let all_key_fmt = fmt_of(wb, first..=r1, gcol);
    let all_fmts: Vec<u32> = o
        .totals
        .iter()
        .map(|&t| fmt_of(wb, first..=r1, c0 + t - 1))
        .collect();
    // 넣을 행: 아래 모드는 묶음 끝 다음 · 맨 끝에 전체, 위 모드는 묶음 시작 앞 · 머리글 바로 아래에 전체 (먼저)
    let mut items: Vec<Item> = groups
        .iter()
        .enumerate()
        .filter(|(g, _)| !keys[*g].is_empty())
        .map(|(g, &(a, b))| Item {
            at: if below { b + 1 } else { a },
            seq: if below { 0 } else { 1 },
            group: Some(g),
        })
        .collect();
    items.push(Item {
        at: if below { r1 + 1 } else { first },
        seq: if below { 1 } else { 0 },
        group: None,
    });
    items.sort_by_key(|i| (i.at, i.seq));
    // 아래에서 위로 넣는다 (자리는 원래 행 번호)
    for it in items.iter().rev() {
        super::structure::shift(wb, si, super::refs::Shift::InsRows { at: it.at, n: 1 })?;
    }
    let ats: Vec<u32> = items.iter().map(|i| i.at).collect();
    let moved = |r: u32| r + ats.iter().filter(|&&a| a <= r).count() as u32;
    let new_rows: Vec<u32> = items
        .iter()
        .enumerate()
        .map(|(k, i)| i.at + k as u32)
        .collect();
    // 새 행이 물려받은 칸·행 속성은 지운다
    {
        let sheet = &mut wb.sheets[si];
        sheet.cells.retain(|c| !new_rows.contains(&c.r.row));
        sheet.hyperlinks.retain(|h| {
            CellRange::parse(&h.range).is_none_or(|g| !new_rows.contains(&g.start.row))
        });
    }
    let bold = {
        let mut font = wb.styles.font_of(0).cloned().unwrap_or_default();
        font.bold = true;
        let fid = super::style::intern_font(&mut wb.styles, font);
        let mut xf = wb.styles.xf(0).cloned().unwrap_or_default();
        xf.font_id = fid;
        xf.apply_font = Some(true);
        super::style::intern_xf(&mut wb.styles, xf)
    };
    let mut summary_rows: Vec<u32> = Vec::new();
    let mut total_row = 0;
    for (k, it) in items.iter().enumerate() {
        let row = new_rows[k];
        let (text, (ra, rb), fm): (String, (u32, u32), &[u32]) = match it.group {
            Some(g) => (
                format!("{} {}", keys[g], label),
                (moved(groups[g].0), moved(groups[g].1)),
                &fmts[g],
            ),
            None => {
                total_row = row;
                (
                    if o.func == "sum" {
                        "총 합계".to_string()
                    } else {
                        format!("전체 {label}")
                    },
                    (moved(first), moved(r1)),
                    &all_fmts,
                )
            }
        };
        summary_rows.push(row);
        let key_fmt = it.group.map_or(all_key_fmt, |g| key_fmts[g]);
        let label_style = if key_fmt != 0 {
            let mut xf = wb.styles.xf(bold).cloned().unwrap_or_default();
            xf.num_fmt_id = key_fmt;
            xf.apply_number_format = Some(true);
            super::style::intern_xf(&mut wb.styles, xf)
        } else {
            bold
        };
        let c = cell_mut(&mut wb.sheets[si], CellRef { row, col: gcol });
        c.style = label_style;
        c.value = CellValue::Text(text);
        c.formula = None;
        for (j, &t) in o.totals.iter().enumerate() {
            let col = c0 + t - 1;
            let name = CellRef::col_name(col);
            let mut xf = wb.styles.xf(0).cloned().unwrap_or_default();
            xf.num_fmt_id = fm.get(j).copied().unwrap_or(0);
            xf.apply_number_format = Some(true);
            let st = super::style::intern_xf(&mut wb.styles, xf);
            let c = cell_mut(&mut wb.sheets[si], CellRef { row, col });
            c.style = st;
            c.value = CellValue::Empty;
            c.formula = Some(crate::model::Formula {
                text: format!("SUBTOTAL({code},{name}{}:{name}{})", ra + 1, rb + 1),
                ..Default::default()
            });
        }
    }
    // 개요: 자료 행 2단계, 부분합 행 1단계, 전체 행 0단계, 요약 행마다 collapsed (한셀은 맨 바깥 요약 행에도)
    let sheet = &mut wb.sheets[si];
    sheet.summary_above = !below;
    let set_row = |sheet: &mut crate::model::Sheet, r: u32, level: u8, collapsed: bool| {
        let i = match sheet.rows.binary_search_by_key(&(r + 1), |x| x.index) {
            Ok(i) => i,
            Err(i) => {
                sheet.rows.insert(
                    i,
                    crate::model::RowInfo {
                        index: r + 1,
                        ..Default::default()
                    },
                );
                i
            }
        };
        sheet.rows[i].outline_level = level;
        sheet.rows[i].collapsed = collapsed;
    };
    for (g, &(a, b)) in groups.iter().enumerate() {
        for r in moved(a)..=moved(b) {
            set_row(sheet, r, if keys[g].is_empty() { 1 } else { 2 }, false);
        }
    }
    for (k, it) in items.iter().enumerate() {
        set_row(
            sheet,
            new_rows[k],
            if it.group.is_some() { 1 } else { 0 },
            true,
        );
    }
    if below {
        set_row(sheet, total_row + 1, 0, true);
    } else {
        set_row(sheet, range.start.row, 0, true);
    }
    if o.page_breaks {
        // 아래 모드: 묶음마다 부분합 행 뒤, 마지막 묶음은 전체 행 뒤. 위 모드: 마지막 묶음을 뺀 묶음의 끝 자료 행 뒤
        let mut ids: Vec<u32> = Vec::new();
        if below {
            for (k, it) in items.iter().enumerate() {
                if it.group.is_some() && it.group != Some(groups.len() - 1) {
                    ids.push(new_rows[k] + 1);
                }
            }
            ids.push(total_row + 1);
        } else {
            for &(_, b) in &groups[..groups.len() - 1] {
                ids.push(moved(b) + 1);
            }
        }
        for id in ids {
            if !sheet.page.row_breaks.contains(&id) {
                sheet.page.row_breaks.push(id);
            }
        }
        sheet.page.row_breaks.sort();
    }
    super::structure::update_dimension(sheet);
    Ok(())
}

/// 통합 설정
#[derive(Debug, Clone)]
pub struct ConsolidateOptions {
    pub func: String,
    pub top_row: bool,
    pub left_column: bool,
}

/// 통합할 원본 (시트 번호, 범위)
pub type ConsolidateSource = (usize, CellRange);

fn agg(func: &str, v: &[f64]) -> Option<CellValue> {
    if v.is_empty() {
        return None;
    }
    let n = v.len() as f64;
    let sum: f64 = v.iter().sum();
    Some(CellValue::Number(match func {
        "sum" => sum,
        "count" | "countNums" => n,
        "average" => sum / n,
        "max" => v.iter().cloned().fold(f64::MIN, f64::max),
        "min" => v.iter().cloned().fold(f64::MAX, f64::min),
        "product" => v.iter().product(),
        "stdev" | "var" if v.len() < 2 => return None,
        "stdev" => {
            let m = sum / n;
            (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        }
        "stdevp" => {
            let m = sum / n;
            (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n).sqrt()
        }
        "var" => {
            let m = sum / n;
            v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)
        }
        "varp" => {
            let m = sum / n;
            v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / n
        }
        _ => return Some(CellValue::Error("#DIV/0!".into())),
    }))
}

/// 통합 (한셀 `Range.Consolidate` 대조 실험 — docs/editing.md 18절): 원본 범위들을 `dest` 부터 값으로 합친다 (연결 없이).
/// 왼쪽 열 이름표(`left_column`)·위쪽 행 이름표(`top_row`)가 같은 것끼리(대소문자 무시, 처음 나온 차례) 모으고, 없는 쪽은
/// 칸 자리로 모은다. 함수는 수만 센다.
pub fn consolidate(
    wb: &mut Workbook,
    si: usize,
    dest: CellRef,
    sources: &[ConsolidateSource],
    o: &ConsolidateOptions,
) -> crate::Result<()> {
    if si >= wb.sheets.len() || sources.is_empty() {
        return Err(err("통합할 범위가 없습니다"));
    }
    crate::calc::recalculate(wb);
    // 이름표: (쓸 값 — 원본 칸의 값 그대로, 숫자는 숫자), 대소문자 무시 키(표시 글))
    type Label = (CellValue, String);
    let key = |wb: &Workbook, s: usize, r: u32, c: u32| -> Option<Label> {
        let at = CellRef { row: r, col: c };
        let t = super::find::display_text(wb, s, at)?;
        let v = wb.sheets[s].cell(at).map(|x| x.value.clone())?;
        Some((v, t.to_lowercase()))
    };
    let num = |wb: &Workbook, s: usize, r: u32, c: u32| -> Option<f64> {
        match wb.sheets[s]
            .cell(CellRef { row: r, col: c })
            .map(|x| &x.value)
        {
            Some(CellValue::Number(n)) => Some(*n),
            _ => None,
        }
    };
    let mut row_labels: Vec<Label> = Vec::new();
    let mut col_labels: Vec<Label> = Vec::new();
    // (행 번호, 열 번호) → 값들
    let mut cells: std::collections::BTreeMap<(usize, usize), Vec<f64>> = Default::default();
    for &(s, rg) in sources {
        if s >= wb.sheets.len() {
            return Err(err("시트가 없습니다"));
        }
        let (r0, c0, r1, c1) = (rg.start.row, rg.start.col, rg.end.row, rg.end.col);
        let dr = r0 + o.top_row as u32;
        let dc = c0 + o.left_column as u32;
        // 열 번호: 위쪽 이름표가 있으면 이름표로, 없으면 자리로
        let col_ids: Vec<usize> = (dc..=c1)
            .map(|c| {
                if o.top_row {
                    match key(wb, s, r0, c) {
                        Some(l) => {
                            col_labels
                                .iter()
                                .position(|x| x.1 == l.1)
                                .unwrap_or_else(|| {
                                    col_labels.push(l);
                                    col_labels.len() - 1
                                })
                        }
                        None => usize::MAX,
                    }
                } else {
                    (c - dc) as usize
                }
            })
            .collect();
        for r in dr..=r1 {
            let rid = if o.left_column {
                match key(wb, s, r, c0) {
                    Some(l) => row_labels
                        .iter()
                        .position(|x| x.1 == l.1)
                        .unwrap_or_else(|| {
                            row_labels.push(l);
                            row_labels.len() - 1
                        }),
                    None => continue,
                }
            } else {
                (r - dr) as usize
            };
            for (k, c) in (dc..=c1).enumerate() {
                if col_ids[k] == usize::MAX {
                    continue;
                }
                if let Some(v) = num(wb, s, r, c) {
                    cells.entry((rid, col_ids[k])).or_default().push(v);
                }
            }
        }
    }
    let n_rows = if o.left_column {
        row_labels.len()
    } else {
        cells.keys().map(|k| k.0 + 1).max().unwrap_or(0)
    };
    let n_cols = if o.top_row {
        col_labels.len()
    } else {
        cells.keys().map(|k| k.1 + 1).max().unwrap_or(0)
    };
    let (hr, hc) = (o.top_row as u32, o.left_column as u32);
    // 한셀 자동화는 시트 보호를 보지 않는다 (보호된 시트에서도 덮어쓴다 — 실험)
    let put = |wb: &mut Workbook, r: u32, c: u32, v: CellValue| {
        let cell = cell_mut(&mut wb.sheets[si], CellRef { row: r, col: c });
        cell.value = v;
        cell.formula = None;
    };
    // 이름표 칸은 비어 있을 때만 쓴다 (한셀 — 이미 내용이 든 이름표 칸은 그대로)
    let empty_at = |wb: &Workbook, at: CellRef| {
        wb.sheets[si]
            .cell(at)
            .is_none_or(|c| c.value.is_empty() && c.formula.is_none())
    };
    if o.top_row {
        for (j, l) in col_labels.iter().enumerate() {
            let at = CellRef {
                row: dest.row,
                col: dest.col + hc + j as u32,
            };
            if empty_at(wb, at) {
                put(wb, at.row, at.col, l.0.clone());
            }
        }
    }
    if o.left_column {
        for (i, l) in row_labels.iter().enumerate() {
            let at = CellRef {
                row: dest.row + hr + i as u32,
                col: dest.col,
            };
            if empty_at(wb, at) {
                put(wb, at.row, at.col, l.0.clone());
            }
        }
    }
    // 값 칸은 모두 쓴다 (결과가 없으면 비운다)
    for i in 0..n_rows {
        for j in 0..n_cols {
            let at = CellRef {
                row: dest.row + hr + i as u32,
                col: dest.col + hc + j as u32,
            };
            match cells.get(&(i, j)).and_then(|x| agg(&o.func, x)) {
                Some(v) => put(wb, at.row, at.col, v),
                None => {
                    if let Some(c) = wb.sheets[si].cells.iter_mut().find(|c| c.r == at) {
                        c.value = CellValue::Empty;
                        c.formula = None;
                    }
                }
            }
        }
    }
    super::structure::update_dimension(&mut wb.sheets[si]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comma() -> SplitOptions {
        SplitOptions {
            comma: true,
            qualifier: Some('"'),
            ..Default::default()
        }
    }

    #[test]
    fn split_like_hcell() {
        let o = comma();
        assert_eq!(split_fields("a,b,c", &o), ["a", "b", "c"]);
        assert_eq!(split_fields("a,,b", &o), ["a", "", "b"]);
        assert_eq!(split_fields("\"q,r\",s", &o), ["q,r", "s"]);
        assert_eq!(split_fields("\"a\"\"b\",c", &o), ["a\"b", "c"]);
        assert_eq!(split_fields("\"x,y", &o), ["x,y"]);
        assert_eq!(split_fields("p\"q,r", &o), ["p\"q", "r"]);
        assert_eq!(split_fields(" x , y ", &o), [" x ", " y "]);
        let c = SplitOptions {
            consecutive: true,
            ..comma()
        };
        assert_eq!(split_fields("x,,,y", &c), ["x", "y"]);
        let none = SplitOptions::default();
        assert_eq!(split_fields("a,b", &none), ["a,b"]);
        let sp = SplitOptions {
            space: true,
            comma: true,
            ..Default::default()
        };
        assert_eq!(split_fields("x y,z", &sp), ["x", "y", "z"]);
    }
}
