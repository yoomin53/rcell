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
    super::guard_cells(wb, si, range)?;
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
    if removed == 0 {
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
        if kept > 0 {
            if dest.col + kept > crate::edit::refs::MAX_COLS {
                return Err(err("텍스트 나누기 결과가 시트를 넘습니다"));
            }
            super::guard_cells(
                wb,
                si,
                CellRange {
                    start: dest,
                    end: CellRef {
                        row: dest.row + rows - 1,
                        col: dest.col + kept - 1,
                    },
                },
            )?;
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
                    let new =
                        super::style::Restyler::default().apply(&mut wb.styles, style, &|_, x| {
                            x.num_fmt_id = id;
                            x.apply_number_format = Some(true);
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
