// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 자동 필터를 다시 적용한다: 필터 범위의 데이터 행마다 열 조건(값 목록·빈 칸, 사용자 지정 조건, 상위/하위,
//! 평균 위/아래)을 모두 만족하면 보이고 아니면 숨긴다. 한셀은 행·열을 넣거나 지워 필터 범위의 첫 칸(머리글)이
//! 옮겨질 때 필터를 다시 적용한다 (필터 대조 실험: 1행·A열 삽입은 다시 적용, 범위 안 행 삽입·삭제는 그대로).
//! 날짜 기간 같은 다른 동적 필터는 조건으로 보지 않는다.

use super::refs;
use crate::model::{CellRef, CellValue, FilterColumn, RowInfo, Sheet, Workbook};

/// 필터에 조건이 있는가 (조건 없는 자동 필터는 행을 숨기지 않는다)
pub(crate) fn has_criteria(sheet: &Sheet) -> bool {
    sheet.auto_filter.as_ref().is_some_and(|af| {
        af.columns.iter().any(|c| {
            !c.values.is_empty()
                || c.blank
                || !c.custom.is_empty()
                || c.top10.is_some()
                || c.dynamic.is_some()
        })
    })
}

/// 칸에 보이는 글 (표시 형식을 적용한 것)
fn shown(wb: &Workbook, sheet: &Sheet, r: CellRef) -> String {
    let Some(c) = sheet.cell(r) else {
        return String::new();
    };
    let code = wb
        .styles
        .xf(c.style)
        .map(|x| crate::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    match &c.value {
        CellValue::Empty => String::new(),
        CellValue::Text(t) => t.clone(),
        v => crate::numfmt::format(v, &code, wb.date1904).text,
    }
}

fn number(sheet: &Sheet, r: CellRef) -> Option<f64> {
    match sheet.cell(r).map(|c| &c.value) {
        Some(CellValue::Number(n)) => Some(*n),
        _ => None,
    }
}

/// `*`·`?` 와일드카드 (대소문자 무시)
fn wild(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    fn go(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|k| go(&p[1..], &t[k..])),
            Some('?') => !t.is_empty() && go(&p[1..], &t[1..]),
            Some('~') if p.len() > 1 => t.first() == Some(&p[1]) && go(&p[2..], &t[1..]),
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(&p, &t)
}

/// 사용자 지정 조건 하나
fn custom_one(op: &str, val: &str, num: Option<f64>, text: &str) -> bool {
    let cmp = match (val.trim().parse::<f64>().ok(), num) {
        (Some(v), Some(n)) => n.partial_cmp(&v),
        (Some(_), None) if op != "equal" && op != "notEqual" => return false,
        _ => {
            if matches!(op, "equal" | "notEqual") {
                let m = wild(val, text);
                return (op == "equal") == m;
            }
            Some(text.to_lowercase().cmp(&val.to_lowercase()))
        }
    };
    let Some(o) = cmp else { return false };
    use std::cmp::Ordering::*;
    match op {
        "notEqual" => o != Equal,
        "greaterThan" => o == Greater,
        "greaterThanOrEqual" => o != Less,
        "lessThan" => o == Less,
        "lessThanOrEqual" => o != Greater,
        _ => o == Equal,
    }
}

/// 열 조건 하나를 데이터 행들에 맞춰 판정하는 함수
fn column_test(
    wb: &Workbook,
    sheet: &Sheet,
    col: u32,
    rows: &[u32],
    fc: &mut FilterColumn,
) -> Box<dyn Fn(u32) -> bool> {
    let nums: Vec<f64> = rows
        .iter()
        .filter_map(|&r| number(sheet, CellRef { row: r, col }))
        .collect();
    let texts: std::collections::HashMap<u32, String> = rows
        .iter()
        .map(|&r| (r, shown(wb, sheet, CellRef { row: r, col })))
        .collect();
    let numbers: std::collections::HashMap<u32, f64> = rows
        .iter()
        .filter_map(|&r| Some((r, number(sheet, CellRef { row: r, col })?)))
        .collect();
    // 값 목록·빈 칸
    let values: Option<Vec<String>> = (!fc.values.is_empty() || fc.blank)
        .then(|| fc.values.iter().map(|v| v.to_lowercase()).collect());
    let blank = fc.blank;
    let custom = fc.custom.clone();
    let custom_and = fc.custom_and;
    // 상위/하위·평균: 저장된 경계값 (없으면 지금 데이터로)
    fill_column(fc, &nums);
    let top = fc.top10.and_then(|(top, _, _)| Some((top, fc.filter_val?)));
    let avg = match fc.dynamic.as_deref() {
        Some(d @ ("aboveAverage" | "belowAverage")) => {
            fc.dynamic_val.map(|v| (d == "aboveAverage", v))
        }
        _ => None,
    };
    Box::new(move |r: u32| {
        let text = texts.get(&r).cloned().unwrap_or_default();
        let num = numbers.get(&r).copied();
        if let Some(vals) = &values {
            let ok = if text.is_empty() {
                blank
            } else {
                vals.contains(&text.to_lowercase())
            };
            if !ok {
                return false;
            }
        }
        if !custom.is_empty() {
            let mut res = custom.iter().map(|(op, v)| custom_one(op, v, num, &text));
            let ok = if custom_and {
                res.all(|x| x)
            } else {
                res.any(|x| x)
            };
            if !ok {
                return false;
            }
        }
        if let Some((top, edge)) = top {
            match num {
                Some(n) if (top && n >= edge) || (!top && n <= edge) => {}
                _ => return false,
            }
        }
        if let Some((above, a)) = avg {
            match num {
                Some(n) if (above && n > a) || (!above && n < a) => {}
                _ => return false,
            }
        }
        true
    })
}

/// 열의 상위/하위 경계값(`filterVal`)·평균(`val`)이 없으면 숫자들로 채운다. 상위/하위 개수는 백분율이면 데이터
/// 수 × 백분율의 버림 (적어도 1)
fn fill_column(fc: &mut FilterColumn, nums: &[f64]) {
    if nums.is_empty() {
        return;
    }
    if let Some((top, percent, val)) = fc.top10
        && fc.filter_val.is_none()
    {
        let mut v = nums.to_vec();
        v.sort_by(|a, b| if top { b.total_cmp(a) } else { a.total_cmp(b) });
        let k = if percent {
            ((v.len() as f64 * val / 100.0).floor() as usize).max(1)
        } else {
            (val as usize).max(1)
        }
        .min(v.len());
        fc.filter_val = Some(v[k - 1]);
    }
    if matches!(fc.dynamic.as_deref(), Some("aboveAverage" | "belowAverage"))
        && fc.dynamic_val.is_none()
    {
        fc.dynamic_val = Some(nums.iter().sum::<f64>() / nums.len() as f64);
    }
}

/// 자동 필터의 빠진 경계값을 편집 앞 데이터로 채운다 — 한셀은 문서를 열 때 계산해 두고 편집해도 다시 계산하지
/// 않는다 (필터 대조 실험: 마지막 데이터 행을 지워도 상위 3 경계값은 연 때의 값)
pub(crate) fn fill_thresholds(wb: &mut Workbook) {
    for si in 0..wb.sheets.len() {
        let sheet = &wb.sheets[si];
        let Some(mut af) = sheet.auto_filter.clone() else {
            continue;
        };
        let needs = af.columns.iter().any(|c| {
            (c.top10.is_some() && c.filter_val.is_none())
                || (matches!(c.dynamic.as_deref(), Some("aboveAverage" | "belowAverage"))
                    && c.dynamic_val.is_none())
        });
        let Some(area) = refs::parse_area(&af.range) else {
            continue;
        };
        if !needs {
            continue;
        }
        let (r0, c0, r1, _) = area.bounds();
        for fc in &mut af.columns {
            let col = c0 + fc.col_id;
            let nums: Vec<f64> = (r0 + 1..=r1)
                .filter_map(|r| number(sheet, CellRef { row: r, col }))
                .collect();
            fill_column(fc, &nums);
        }
        wb.sheets[si].auto_filter = Some(af);
    }
}

/// 시트의 자동 필터를 다시 적용한다 (행 숨김을 조건에 맞게)
pub(crate) fn reapply(wb: &mut Workbook, si: usize) {
    let sheet = &wb.sheets[si];
    if !has_criteria(sheet) {
        return;
    }
    let Some(mut af) = sheet.auto_filter.clone() else {
        return;
    };
    let Some(area) = refs::parse_area(&af.range) else {
        return;
    };
    let (r0, c0, r1, _) = area.bounds();
    let rows: Vec<u32> = (r0 + 1..=r1).collect();
    let tests: Vec<Box<dyn Fn(u32) -> bool>> = af
        .columns
        .iter_mut()
        .map(|fc| column_test(wb, sheet, c0 + fc.col_id, &rows, fc))
        .collect();
    let hide: Vec<(u32, bool)> = rows
        .iter()
        .map(|&r| (r, !tests.iter().all(|t| t(r))))
        .collect();
    let sheet = &mut wb.sheets[si];
    for (r, h) in hide {
        match sheet.rows.iter_mut().find(|x| x.index == r + 1) {
            Some(info) => info.hidden = h,
            None if h => sheet.rows.push(RowInfo {
                index: r + 1,
                hidden: true,
                ..Default::default()
            }),
            None => {}
        }
    }
    sheet.rows.sort_by_key(|r| r.index);
    sheet.auto_filter = Some(af);
    sheet.filter_mode = true;
}

/// 조건 글 하나 → (연산자, 값). `>=5`·`<>x`·`=x`·`x` (연산자가 없으면 같음)
fn parse_criterion(s: &str) -> (&'static str, String) {
    for (p, op) in [
        ("<>", "notEqual"),
        ("<=", "lessThanOrEqual"),
        (">=", "greaterThanOrEqual"),
        ("<", "lessThan"),
        (">", "greaterThan"),
        ("=", "equal"),
    ] {
        if let Some(rest) = s.strip_prefix(p) {
            // `<>` 만이면 빈 칸이 아닌 것 (한셀은 값 한 칸 공백으로 둔다)
            if op == "notEqual" && rest.is_empty() {
                return (op, " ".into());
            }
            return (op, rest.to_string());
        }
    }
    ("equal", s.to_string())
}

/// 자동 필터 켜기·끄기·조건 걸기 (한셀 `Range.AutoFilter` 대조 실험 af4·af5 — docs/editing.md 22절).
/// `field` 가 없으면 켜고(이미 켜져 있으면 끈다), 있으면(1부터, 범위 안 열) 그 열에 조건을 건다: 조건 없이 `field` 만 주면 그
/// 열의 조건을 지운다. `operator` 는 `and`(기본)·`or`(두 조건 결합)·`top`·`bottom`·`topPercent`·`bottomPercent`(상위/하위 개수·백분율).
/// 같음 조건은 그 열의 보이는 값 중에 있으면 값 목록 필터로, 없거나 `*`·`?` 가 있으면 사용자 지정 조건으로 둔다.
pub fn auto_filter(
    wb: &mut Workbook,
    si: usize,
    range: crate::model::CellRange,
    field: Option<u32>,
    criteria1: Option<&str>,
    operator: Option<&str>,
    criteria2: Option<&str>,
) -> crate::Result<()> {
    use super::err;
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let name = wb.sheets[si].name.clone();
    let text = refs::Area::rect(
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    )
    .text();
    let abs = format!(
        "{}{}",
        refs::prefix_text(&name, None),
        abs_range(range.start, range.end)
    );
    let set_db_name = |wb: &mut Workbook, on: bool| {
        let lid = Some(si as u32);
        let pos = wb
            .defined_names
            .iter()
            .position(|d| d.local_sheet_id == lid && d.name == "_xlnm._FilterDatabase");
        match (pos, on) {
            (Some(i), true) => wb.defined_names[i].formula = abs.clone(),
            (Some(i), false) => {
                wb.defined_names.remove(i);
            }
            (None, true) => wb.defined_names.push(crate::model::DefinedName {
                name: "_xlnm._FilterDatabase".into(),
                local_sheet_id: lid,
                hidden: true,
                formula: abs.clone(),
                comment: None,
            }),
            (None, false) => {}
        }
    };
    let show_rows = |wb: &mut Workbook, r0: u32, r1: u32| {
        for r in &mut wb.sheets[si].rows {
            if r.index > r0 + 1 && r.index <= r1 + 1 {
                r.hidden = false;
            }
        }
    };
    let Some(field) = field else {
        // 켜기·끄기
        if wb.sheets[si].auto_filter.is_some() {
            if has_criteria(&wb.sheets[si]) {
                show_rows(wb, range.start.row, range.end.row);
            }
            wb.sheets[si].auto_filter = None;
            wb.sheets[si].filter_mode = false;
            set_db_name(wb, false);
        } else {
            wb.sheets[si].auto_filter = Some(crate::model::AutoFilter {
                range: text,
                columns: Vec::new(),
                sort_state: None,
            });
            wb.sheets[si].filter_mode = true;
            set_db_name(wb, true);
        }
        return Ok(());
    };
    let width = range.end.col - range.start.col + 1;
    if field < 1 || field > width {
        return Err(err("필터 열이 범위를 벗어났습니다"));
    }
    if wb.sheets[si].auto_filter.is_none() {
        wb.sheets[si].auto_filter = Some(crate::model::AutoFilter {
            range: text,
            columns: Vec::new(),
            sort_state: None,
        });
        wb.sheets[si].filter_mode = true;
        set_db_name(wb, true);
    }
    let col_id = field - 1;
    let col = range.start.col + col_id;
    let (r0, r1) = (range.start.row, range.end.row);
    let mut fc = FilterColumn {
        col_id,
        ..Default::default()
    };
    match (criteria1, operator) {
        (None, _) => {}
        (Some(c1), Some(op @ ("top" | "bottom" | "topPercent" | "bottomPercent"))) => {
            let n: f64 = c1
                .trim()
                .parse()
                .map_err(|_| err("상위/하위 값이 숫자가 아닙니다"))?;
            fc.top10 = Some((op.starts_with("top"), op.ends_with("Percent"), n));
        }
        (Some(c1), op) => {
            let (o1, v1) = parse_criterion(c1);
            match criteria2 {
                Some(c2) => {
                    let (o2, v2) = parse_criterion(c2);
                    fc.custom = vec![(o1.to_string(), v1), (o2.to_string(), v2)];
                    fc.custom_and = op != Some("or");
                }
                None => {
                    // 같음: 보이는 값 중에 있으면 값 목록
                    let sheet = &wb.sheets[si];
                    let hidden: std::collections::HashSet<u32> = sheet
                        .rows
                        .iter()
                        .filter(|r| r.hidden)
                        .map(|r| r.index - 1)
                        .collect();
                    let found = (r0 + 1..=r1)
                        .filter(|r| !hidden.contains(r))
                        .map(|r| shown(wb, sheet, CellRef { row: r, col }))
                        .find(|t| t.to_lowercase() == v1.to_lowercase());
                    let wildcard = v1.contains(['*', '?']);
                    match found {
                        Some(t) if o1 == "equal" && !wildcard && !v1.is_empty() => {
                            fc.values = vec![t]
                        }
                        _ => {
                            fc.custom = vec![(o1.to_string(), v1)];
                            fc.custom_and = true;
                        }
                    }
                }
            }
        }
    }
    let af = wb.sheets[si].auto_filter.as_mut().unwrap();
    af.columns.retain(|c| c.col_id != col_id);
    let cleared = fc.values.is_empty()
        && !fc.blank
        && fc.custom.is_empty()
        && fc.top10.is_none()
        && fc.dynamic.is_none();
    if !cleared {
        af.columns.push(fc);
        af.columns.sort_by_key(|c| c.col_id);
    }
    if has_criteria(&wb.sheets[si]) {
        reapply(wb, si);
    } else {
        show_rows(wb, r0, r1);
    }
    wb.sheets[si].filter_mode = true;
    Ok(())
}

/// 모든 필터 조건을 지우고 숨긴 행을 보인다 (`ShowAllData` — 자동 필터는 켜 둔다)
pub fn show_all_data(wb: &mut Workbook, si: usize) -> crate::Result<()> {
    let sheet = wb
        .sheets
        .get_mut(si)
        .ok_or_else(|| super::err("시트가 없습니다"))?;
    let Some(af) = &mut sheet.auto_filter else {
        return Ok(());
    };
    af.columns.clear();
    if let Some(area) = refs::parse_area(&af.range) {
        let (r0, _, r1, _) = area.bounds();
        for r in &mut sheet.rows {
            if r.index > r0 + 1 && r.index <= r1 + 1 {
                r.hidden = false;
            }
        }
    }
    Ok(())
}

/// `$A$1:$E$12` 꼴
fn abs_range(a: CellRef, b: CellRef) -> String {
    let c = CellRef::col_name;
    format!("${}${}:${}${}", c(a.col), a.row + 1, c(b.col), b.row + 1)
}
