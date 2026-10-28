// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 정렬 (정렬 대조 실험 — docs/editing.md 12절).
//!
//! - 값의 차례: 숫자 < 글 < 논리값(FALSE < TRUE) < 오류(`#NULL!` `#DIV/0!` `#VALUE!` `#REF!` `#NAME?` `#NUM!`
//!   `#N/A`) < 빈 칸. 내림차순은 빈 칸만 그대로 끝에 두고 뒤집는다. 같은 값은 원래 차례 (안정 정렬). 글은
//!   [`super::collate`] 로 비교한다.
//! - 줄(행, 왼쪽에서 오른쪽이면 열)마다 범위 안의 셀(값·수식·서식), 메모, 하이퍼링크가 옮겨진다. 수식의 상대 참조는
//!   복사처럼 옮긴 만큼 바뀐다. 행 높이·숨김, 유효성은 그대로.
//! - 병합된 셀이나 배열 수식이 있는 범위는 정렬할 수 없다.
//! - 조건부 서식: 줄을 끝 자리부터 채우는 차례로, 원본 줄에 걸친 규칙마다 한 번씩 센다. 범위가 그대로인 규칙은
//!   우선순위가 그 수만큼 내려가고, 옮긴 몫이 범위 밖에 놓이는 규칙은 그 몫을 범위에 더하고 처음 더한 때의 차례
//!   번호를 우선순위로 받아 목록 끝으로 간다 (데이터 막대가 든 묶음은 넓히지 않는다).
//! - 옮겨진 메모의 상자는 한 칸 왼쪽·한 칸 위로 간다 (열은 앞 열 너비 + 1px, 행은 앞 행 높이 + 1px 만큼).

use std::cmp::Ordering;

use super::err;
use super::refs::{self, Area};
use super::structure::{unshare, update_dimension};
use crate::model::{CellRange, CellRef, CellValue, ConditionalFormat, FormulaKind, Workbook};

/// 정렬 열쇠: 왼쪽에서 오른쪽 정렬이 아니면 열 번호, 맞으면 행 번호 (0부터, 시트 기준)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    pub index: u32,
    pub desc: bool,
}

/// 정렬 설정
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortOptions {
    pub keys: Vec<SortKey>,
    /// 첫 줄은 머리글 (정렬하지 않는다)
    pub header: bool,
    /// 대소문자 구분 (소문자가 앞)
    pub match_case: bool,
    /// 왼쪽에서 오른쪽으로 (열을 정렬한다)
    pub left_to_right: bool,
}

/// 오류 값의 차례 (한셀 정렬)
fn error_rank(e: &str) -> u8 {
    match e {
        "#NULL!" => 0,
        "#DIV/0!" => 1,
        "#VALUE!" => 2,
        "#REF!" => 3,
        "#NAME?" => 4,
        "#NUM!" => 5,
        "#N/A" => 6,
        _ => 7,
    }
}

/// 두 값의 차례 (빈 칸은 언제나 끝)
pub fn compare_values(a: &CellValue, b: &CellValue, desc: bool, match_case: bool) -> Ordering {
    fn class(v: &CellValue) -> u8 {
        match v {
            CellValue::Number(_) | CellValue::Date(_) => 0,
            CellValue::Text(_) => 1,
            CellValue::Bool(_) => 2,
            CellValue::Error(_) => 3,
            CellValue::Empty => 4,
        }
    }
    let (ca, cb) = (class(a), class(b));
    if ca == 4 || cb == 4 {
        return ca.cmp(&cb).then(Ordering::Equal);
    }
    let o = ca.cmp(&cb).then_with(|| match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (CellValue::Text(x), CellValue::Text(y)) => super::collate::compare(x, y, match_case),
        (CellValue::Bool(x), CellValue::Bool(y)) => x.cmp(y),
        (CellValue::Error(x), CellValue::Error(y)) => error_rank(x).cmp(&error_rank(y)),
        _ => Ordering::Equal,
    });
    if desc { o.reverse() } else { o }
}

/// 범위를 정렬한다
pub fn sort(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    opt: &SortOptions,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    if opt.keys.is_empty() {
        return Err(err("정렬 기준이 없습니다"));
    }
    // 보호된 시트: 정렬이 막혔으면 할 수 없다. 잠긴 칸은 따지지 않는다 (한셀 자동화 — corpus 대조: 정렬을 허용한
    // 보호 시트의 잠긴 칸도 정렬한다)
    super::guard_action(wb, si, "sort")?;
    // 열·행 전체 범위는 쓰는 범위까지
    let mut range = range;
    let whole_cols = range.start.row == 0 && range.end.row >= refs::MAX_ROWS - 1;
    let whole_rows = range.start.col == 0 && range.end.col >= refs::MAX_COLS - 1;
    if let Some(used) = wb.sheets[si].used_range() {
        if whole_cols {
            range.end.row = used.end.row.max(range.start.row);
        }
        if whole_rows {
            range.end.col = used.end.col.max(range.start.col);
        }
    }
    let sheet = &wb.sheets[si];
    let (r0, c0, r1, c1) = (
        range.start.row,
        range.start.col,
        range.end.row,
        range.end.col,
    );
    let inside = |r: CellRef| r.row >= r0 && r.row <= r1 && r.col >= c0 && r.col <= c1;
    let overlaps = |a: (u32, u32, u32, u32)| !(a.2 < r0 || a.0 > r1 || a.3 < c0 || a.1 > c1);
    let merged = sheet
        .merges
        .iter()
        .any(|m| overlaps((m.start.row, m.start.col, m.end.row, m.end.col)));
    if sheet.cells.iter().any(|c| {
        inside(c.r)
            && c.formula
                .as_ref()
                .is_some_and(|f| matches!(f.kind, FormulaKind::Array | FormulaKind::DataTable))
    }) {
        return Err(err("배열 수식이 있는 범위는 정렬할 수 없습니다"));
    }
    let ltr = opt.left_to_right;
    for k in &opt.keys {
        let ok = if ltr {
            (r0..=r1).contains(&k.index)
        } else {
            (c0..=c1).contains(&k.index)
        };
        if !ok {
            return Err(err("정렬 기준이 범위 밖입니다"));
        }
    }
    // 정렬할 줄 (머리글 빼고)
    let (first, last) = if ltr { (c0, c1) } else { (r0, r1) };
    let first = first + opt.header as u32;
    if first > last {
        return Ok(());
    }
    // 정렬 기준 값 (수식은 지금 값 — 다시 계산해 둔다)
    crate::calc::recalculate(wb);
    let sheet = &wb.sheets[si];
    // 필터가 걸린 시트에서는 숨긴 행을 그 자리에 두고 보이는 행만 정렬한다 (corpus 대조 — 필터 문서)
    let filtered = !ltr && super::filter::has_criteria(sheet);
    let hidden: std::collections::HashSet<u32> = if filtered {
        sheet
            .rows
            .iter()
            .filter(|r| r.hidden)
            .map(|r| r.index - 1)
            .collect()
    } else {
        Default::default()
    };
    let lines: Vec<u32> = (first..=last).filter(|l| !hidden.contains(l)).collect();
    let value_at = |line: u32, key: u32| -> CellValue {
        let r = if ltr {
            CellRef {
                row: key,
                col: line,
            }
        } else {
            CellRef {
                row: line,
                col: key,
            }
        };
        sheet.cell(r).map(|c| c.value.clone()).unwrap_or_default()
    };
    let vals: Vec<Vec<CellValue>> = lines
        .iter()
        .map(|&l| opt.keys.iter().map(|k| value_at(l, k.index)).collect())
        .collect();
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&a, &b| {
        for (k, key) in opt.keys.iter().enumerate() {
            let o = compare_values(&vals[a][k], &vals[b][k], key.desc, opt.match_case);
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    // 병합된 셀이 있으면 줄을 옮길 수 없다 (한셀은 차례가 그대로면 오류 없이 지나간다 — corpus 대조)
    if merged && order.iter().enumerate().any(|(d, &s)| d != s) {
        return Err(err("병합된 셀이 있는 범위는 정렬할 수 없습니다"));
    }
    // 원본 줄 → 새 줄
    let mut dest_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for (d, &s) in order.iter().enumerate() {
        dest_of.insert(lines[s], lines[d]);
    }
    let line_of = |r: CellRef| if ltr { r.col } else { r.row };
    let cross_ok = |r: CellRef| {
        if ltr {
            r.row >= r0 && r.row <= r1
        } else {
            r.col >= c0 && r.col <= c1
        }
    };
    let new_pos = |r: CellRef| -> Option<CellRef> {
        if !cross_ok(r) {
            return None;
        }
        let d = *dest_of.get(&line_of(r))?;
        Some(if ltr {
            CellRef { row: r.row, col: d }
        } else {
            CellRef { row: d, col: r.col }
        })
    };

    // 조건부 서식은 옮기기 전 범위로 계산한다
    let cfs = sorted_cf(
        &wb.sheets[si].conditional_formats,
        &order,
        &lines,
        ltr,
        (r0, c0, r1, c1),
    );
    // 메모 상자를 다시 둘 배치 (옮기기 전)
    let geo = super::structure::comment_geometry(wb, si, 0);

    // 테두리는 자리에 남는다 (한셀 — 테두리 서식 문서의 정렬 대조): 옮기기 전 자리마다 테두리
    let old_borders: Vec<(CellRef, u32)> = {
        let sheet = &wb.sheets[si];
        let mut v = Vec::new();
        for c in &sheet.cells {
            if new_pos(c.r).is_some() {
                v.push((c.r, wb.styles.xf(c.style).map_or(0, |x| x.border_id)));
            }
        }
        v
    };

    let sheet = &mut wb.sheets[si];
    sheet.conditional_formats = cfs;
    // 셀
    for c in &mut sheet.cells {
        let Some(to) = new_pos(c.r) else { continue };
        let (dr, dc) = (
            to.row as i64 - c.r.row as i64,
            to.col as i64 - c.r.col as i64,
        );
        if let Some(f) = &mut c.formula
            && !f.text.is_empty()
            && (dr != 0 || dc != 0)
        {
            f.text = crate::formula_text::shift_references(&f.text, dr, dc);
        }
        c.r = to;
    }
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    unshare(sheet);
    keep_borders(wb, si, &old_borders, &new_pos);
    let sheet = &mut wb.sheets[si];
    // 하이퍼링크 (한 칸짜리)
    for h in &mut sheet.hyperlinks {
        if let Some(rg) = CellRange::parse(&h.range)
            && rg.start == rg.end
            && let Some(to) = new_pos(rg.start)
        {
            h.range = Area::rect(to.row, to.col, to.row, to.col).text();
        }
    }
    // 메모: 칸을 따라가고 상자는 한 칸 왼쪽·위로. 옮긴 메모는 목록 끝에 새 자리 차례로
    let mut kept = Vec::new();
    let mut moved = Vec::new();
    for mut c in std::mem::take(&mut sheet.comments) {
        match new_pos(c.cell) {
            Some(to) => {
                c.cell = to;
                if let (Some(s), Some((g, _))) = (&mut c.shape, &geo) {
                    s.anchor = sorted_box(g, s.anchor);
                }
                moved.push(c);
            }
            None => kept.push(c),
        }
    }
    moved.sort_by_key(|c| (c.cell.row, c.cell.col));
    rename_moved(&kept, &mut moved, &lines, &order, ltr);
    kept.extend(moved);
    sheet.comments = kept;
    update_dimension(sheet);
    Ok(())
}

/// 정렬로 옮긴 셀에 그 자리에 있던 테두리를 다시 준다
fn keep_borders(
    wb: &mut Workbook,
    si: usize,
    old: &[(CellRef, u32)],
    moved: &dyn Fn(CellRef) -> Option<CellRef>,
) {
    let old_at: std::collections::HashMap<CellRef, u32> = old.iter().copied().collect();
    // 셀이 옮겨 온 자리와 셀이 있던 자리
    let mut spots: Vec<CellRef> = old.iter().map(|&(r, _)| r).collect();
    spots.extend(
        wb.sheets[si]
            .cells
            .iter()
            .map(|c| c.r)
            .filter(|&r| moved(r).is_some()),
    );
    spots.sort_by_key(|r| (r.row, r.col));
    spots.dedup();
    for at in spots {
        let sheet = &wb.sheets[si];
        let base = sheet.effective_style(at.row, at.col);
        let border_of = |st: u32| wb.styles.xf(st).map_or(0, |x| x.border_id);
        let want = old_at.get(&at).copied().unwrap_or_else(|| border_of(base));
        let (cur, exists) = match sheet.cell(at) {
            Some(c) => (c.style, true),
            None => (base, false),
        };
        if border_of(cur) == want {
            continue;
        }
        let mut xf = wb.styles.xf(cur).cloned().unwrap_or_default();
        xf.border_id = want;
        let n = super::style::intern_xf(&mut wb.styles, xf);
        let sheet = &mut wb.sheets[si];
        if exists {
            if let Some(c) = sheet.cells.iter_mut().find(|c| c.r == at) {
                c.style = n;
            }
        } else {
            super::cell_mut(sheet, at).style = n;
        }
    }
}

/// 옮긴 메모의 이름 (`메모 N`): 한셀은 새 줄을 위에서부터 채우며 줄마다 원본 줄의 메모를 새 자리에 만들고 그 줄에
/// 있던 원래 메모를 지운다 (원본과 새 줄이 같으면 그대로). 만들 때 이름이 아직 다른 메모에 쓰이고 있으면 새 번호를
/// 받는다 — 앞에서 준 번호 다음부터 비어 있는 가장 작은 번호 (정렬 대조 실험의 메모 이름)
fn rename_moved(
    kept: &[crate::model::Comment],
    moved: &mut [crate::model::Comment],
    lines: &[u32],
    order: &[usize],
    ltr: bool,
) {
    let name = |c: &crate::model::Comment| -> Option<String> {
        c.shape
            .as_ref()
            .and_then(|s| s.name.clone())
            .or_else(|| c.shape_name.clone())
    };
    let num = |n: Option<String>| {
        n.as_deref()
            .and_then(|n| n.strip_prefix("메모 "))
            .and_then(|n| n.parse::<u32>().ok())
    };
    let line_of = |r: CellRef| if ltr { r.col } else { r.row };
    // 원래 줄 (옮기기 전)
    let src_line = |c: &crate::model::Comment| -> u32 {
        let d = line_of(c.cell);
        let di = lines.iter().position(|&l| l == d).unwrap_or(0);
        lines[order[di]]
    };
    let mut live: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for c in kept.iter().chain(moved.iter()) {
        if let Some(n) = num(name(c)) {
            *live.entry(n).or_default() += 1;
        }
    }
    let originals: Vec<(u32, Option<u32>)> =
        moved.iter().map(|c| (src_line(c), num(name(c)))).collect();
    let mut next = 1u32;
    for (di, &d) in lines.iter().enumerate() {
        let s = lines[order[di]];
        if s == d {
            continue;
        }
        // 이 줄로 오는 메모를 만든다 (열 차례 — moved 는 새 자리 차례)
        for c in moved.iter_mut().filter(|c| line_of(c.cell) == d) {
            let Some(n) = num(name(c)) else { continue };
            if live.get(&n).copied().unwrap_or(0) > 0 {
                let mut m = next;
                while live.get(&m).copied().unwrap_or(0) > 0 {
                    m += 1;
                }
                next = m + 1;
                let new = format!("메모 {m}");
                if let Some(sh) = &mut c.shape
                    && sh.name.is_some()
                {
                    sh.name = Some(new.clone());
                }
                if c.shape_name.is_some() {
                    c.shape_name = Some(new);
                }
                *live.entry(m).or_default() += 1;
            } else {
                *live.entry(n).or_default() += 1;
            }
        }
        // 이 줄에 있던 원래 메모를 지운다
        for &(l, n) in &originals {
            if l == d
                && let Some(n) = n
                && let Some(k) = live.get_mut(&n)
            {
                *k = k.saturating_sub(1);
            }
        }
    }
}

/// 정렬로 옮겨진 메모의 상자 (정렬 대조 실험): 앵커의 칸마다 한 칸 앞(숨긴 행·열은 건너뛴다)으로, 오프셋은 1px
/// 줄인 뒤 앵커 좌표(열 너비·행 높이 − 1px)로 넘치거나 모자란 오프셋을 이웃 칸으로 옮긴다. 앞 칸이 없으면 첫 칸에
/// 두고 왼쪽·위는 2px, 오른쪽·아래는 첫 칸 너비 + 1px 줄인다. 기본 너비가 아닌 열에서는 1px 다를 수 있다 (docs/editing.md 13절)
fn sorted_box(g: &crate::render::Geometry, a: [u32; 8]) -> [u32; 8] {
    let size = |list: &[(u32, u32)]| -> Vec<i64> {
        list.iter().map(|p| (p.1 as i64 - 1).max(0)).collect()
    };
    let (cols, rows) = (size(&g.cols), size(&g.rows));
    let back = |sz: &[i64], i: u32, off: u32, first: bool| -> (u32, u32) {
        let visible = |j: i64| sz.get(j as usize).is_some_and(|&w| w > 0);
        let mut j = i as i64 - 1;
        while j >= 0 && !visible(j) {
            j -= 1;
        }
        if j < 0 {
            // 앞 칸이 없으면: 왼쪽·위는 2px, 오른쪽·아래는 첫 칸 너비 + 1px 줄인다
            let d = if first {
                2
            } else {
                sz.first().copied().unwrap_or(0) as u32 + 1
            };
            return (0, off.saturating_sub(d));
        }
        let mut o = off as i64 - 1;
        // 앵커 좌표로 고른다
        loop {
            if o < 0 {
                let mut k = j - 1;
                while k >= 0 && !visible(k) {
                    k -= 1;
                }
                if k < 0 {
                    o = 0;
                    break;
                }
                j = k;
                o += sz[j as usize];
            } else if visible(j) && o >= sz[j as usize] && (j as usize) + 1 < sz.len() {
                o -= sz[j as usize];
                j += 1;
                while (j as usize) + 1 < sz.len() && !visible(j) {
                    j += 1;
                }
            } else {
                break;
            }
        }
        (j as u32, o as u32)
    };
    let (lc, lo) = back(&cols, a[0], a[1], true);
    let (tr, to) = back(&rows, a[2], a[3], true);
    let (rc, ro) = back(&cols, a[4], a[5], false);
    let (br, bo) = back(&rows, a[6], a[7], false);
    [lc, lo, tr, to, rc, ro, br, bo]
}

/// 정렬 뒤의 조건부 서식 목록. `order[d]` = d 번째 새 줄에 오는 원본 줄 (lines 기준 번호)
fn sorted_cf(
    list: &[ConditionalFormat],
    order: &[usize],
    lines: &[u32],
    ltr: bool,
    rect: (u32, u32, u32, u32),
) -> Vec<ConditionalFormat> {
    let (r0, c0, r1, c1) = rect;
    // 줄 l 의 범위 안 몫 (첫 행, 첫 열, 끝 행, 끝 열)
    let line_rect = |l: u32| if ltr { (r0, l, r1, l) } else { (l, c0, l, c1) };
    let inter = |a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)| {
        (!(a.2 < b.0 || b.2 < a.0 || a.3 < b.1 || b.3 < a.1))
            .then(|| (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3)))
    };
    let contains = |a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)| {
        a.0 <= b.0 && a.1 <= b.1 && a.2 >= b.2 && a.3 >= b.3
    };
    let mut areas: Vec<Vec<(u32, u32, u32, u32)>> = list
        .iter()
        .map(|cf| {
            refs::parse_sqref(&cf.sqref)
                .iter()
                .map(|a| a.bounds())
                .collect()
        })
        .collect();
    let mut pieces: Vec<Vec<(u32, u32, u32, u32)>> = vec![Vec::new(); list.len()];
    // 처음 넓힌 때 규칙마다 받은 차례 번호, 넓힌 줄 (목록 차례)
    let mut new_prio: Vec<Option<Vec<u32>>> = vec![None; list.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut k = 0u32;
    for d in (0..order.len()).rev() {
        let s = lines[order[d]];
        let dl = lines[d];
        let src = line_rect(s);
        let mut group = Vec::new();
        for (b, cf) in list.iter().enumerate() {
            let cut: Vec<(u32, u32, u32, u32)> =
                areas[b].iter().filter_map(|&a| inter(a, src)).collect();
            if cut.is_empty() {
                continue;
            }
            // 규칙마다 하나씩 센다 (묶음 안에서는 우선순위 차례)
            let mut idx: Vec<usize> = (0..cf.rules.len()).collect();
            idx.sort_by_key(|&i| cf.rules[i].priority);
            let mut ks = vec![0u32; cf.rules.len()];
            for i in idx {
                k += 1;
                ks[i] = k;
            }
            if super::cfpaste::has_databar(cf) {
                continue;
            }
            let moved: Vec<(u32, u32, u32, u32)> = cut
                .iter()
                .map(|&(a, bcol, c, e)| {
                    if ltr {
                        (a, dl, c, dl)
                    } else {
                        (dl, bcol, dl, e)
                    }
                })
                .collect();
            let all: Vec<(u32, u32, u32, u32)> =
                areas[b].iter().chain(&pieces[b]).copied().collect();
            let mut changed = false;
            for m in moved {
                if !all.iter().any(|&a| contains(a, m)) && !pieces[b].contains(&m) {
                    pieces[b].push(m);
                    changed = true;
                }
            }
            if changed && new_prio[b].is_none() {
                new_prio[b] = Some(ks);
                group.push(b);
            }
        }
        if !group.is_empty() {
            groups.push(group);
        }
    }
    let mut out: Vec<ConditionalFormat> = Vec::new();
    for (b, cf) in list.iter().enumerate() {
        if new_prio[b].is_some() {
            continue;
        }
        let mut m = cf.clone();
        for r in &mut m.rules {
            r.priority += k;
        }
        out.push(m);
    }
    for group in groups.iter().rev() {
        for &b in group {
            let mut m = list[b].clone();
            if let Some(ps) = &new_prio[b] {
                for (r, &p) in m.rules.iter_mut().zip(ps) {
                    r.priority = p;
                }
            }
            let mut ps = std::mem::take(&mut pieces[b]);
            ps.sort();
            let mut list_areas = std::mem::take(&mut areas[b]);
            for p in ps {
                merge_into(&mut list_areas, p);
            }
            let text: Vec<Area> = list_areas
                .iter()
                .map(|&(a, b, c, d)| Area::rect(a, b, c, d))
                .collect();
            m.sqref = refs::sqref_text(&text);
            out.push(m);
        }
    }
    out
}

/// 영역 목록에 한 조각을 더한다: 맞닿은 영역과 이어 붙여 (처음 이은 영역 자리에) 두고, 없으면 끝에
fn merge_into(list: &mut Vec<(u32, u32, u32, u32)>, piece: (u32, u32, u32, u32)) {
    use super::cfpaste::join;
    let mut cur = piece;
    let mut at: Option<usize> = None;
    while let Some(i) = list
        .iter()
        .position(|&a| join(a, cur).is_some() || join(cur, a).is_some())
    {
        let a = list.remove(i);
        cur = join(a, cur).or_else(|| join(cur, a)).unwrap_or(cur);
        at = Some(at.map_or(i, |x: usize| x.min(i)));
    }
    match at {
        Some(i) => list.insert(i.min(list.len()), cur),
        None => list.push(cur),
    }
}
