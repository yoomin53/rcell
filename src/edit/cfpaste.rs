// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 붙여넣기·채우기·옮기기 때 조건부 서식 규칙의 우선순위와 합치기 (조건부 서식 대조 실험 — 규칙 종류 12가지 ×
//! 복사·채우기·옮기기, 그리고 여러 규칙 묶음):
//!
//! - 붙인 규칙은 맨 위 우선순위(1부터)를 받고 시트의 다른 규칙은 그만큼 내려간다. 한 번에 붙인 규칙끼리는 원래
//!   우선순위 차례. 데이터 막대가 든 묶음은 한 번 더 붙는다 (같은 규칙 묶음 둘 — 그 우선순위는 첫 묶음들 다음).
//! - 채우기는 채울 줄마다, 줄 안에서는 묶음 차례·우선순위 차례로 규칙 하나씩 맨 위에 넣는다. 규칙 하나뿐인 묶음
//!   (데이터 막대 빼고)은 원래 규칙의 범위를 넓히고, 데이터 막대와 다른 규칙이 함께 든 묶음은 줄마다 따로,
//!   그 밖에는 채운 줄 전체를 한 묶음으로 둔다 (우선순위는 마지막 줄의 것 — 나머지 번호는 비어 남는다).
//! - 옮기기는 옮긴 규칙에 맨 위 우선순위를 주고, 데이터 막대가 없고 서식이 필요한 규칙마다 서식이 있는 묶음은
//!   옮긴 몫을 원래 묶음에 합친다 (범위는 행 차례). 범위에 따라 결과가 달라지지 않는 규칙만 든 묶음은 같은 규칙의
//!   다른 묶음도 합치고, 그때 수식은 옮긴 몫의 왼쪽 위 칸에 맞춘다.

use super::refs::{self, Area};
use crate::model::{CfRule, ConditionalFormat, Sheet};

/// 서식(dxf)이 있어야 하는 규칙 종류
fn needs_dxf(kind: &str) -> bool {
    !matches!(kind, "colorScale" | "dataBar" | "iconSet")
}

/// 범위에 따라 결과가 달라지는 규칙 (합치면 뜻이 바뀐다)
fn range_dependent(kind: &str) -> bool {
    matches!(
        kind,
        "duplicateValues"
            | "uniqueValues"
            | "top10"
            | "aboveAverage"
            | "colorScale"
            | "dataBar"
            | "iconSet"
    )
}

pub(crate) fn has_databar(cf: &ConditionalFormat) -> bool {
    cf.rules.iter().any(|r| r.kind == "dataBar")
}

/// 옮긴 몫을 원래 묶음에 합칠 수 있는가
pub(crate) fn mergeable(cf: &ConditionalFormat) -> bool {
    !has_databar(cf)
        && cf
            .rules
            .iter()
            .all(|r| !needs_dxf(&r.kind) || r.dxf_id.is_some())
}

fn shift(sheet: &mut Sheet, k: u32) {
    for cf in &mut sheet.conditional_formats {
        for r in &mut cf.rules {
            r.priority += k;
        }
    }
}

/// 맞닿은 두 범위 (같은 열 폭으로 위아래, 같은 행 높이로 좌우)를 하나로
pub(crate) fn join(
    x: (u32, u32, u32, u32),
    y: (u32, u32, u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let vertical = x.1 == y.1 && x.3 == y.3 && x.2 + 1 == y.0;
    let horizontal = x.0 == y.0 && x.2 == y.2 && x.3 + 1 == y.1;
    (vertical || horizontal).then_some((x.0, x.1, y.2, y.3))
}

/// 범위들을 맞닿은 것끼리 이어 붙여 행 차례로 (같은 행이면 열 차례) 적는다 — 채운 줄 AF7·AF8·AF9 는 AF7:AF9
pub(crate) fn sorted_sqref(areas: Vec<Area>) -> String {
    let mut b: Vec<(u32, u32, u32, u32)> = areas.iter().map(|a| a.bounds()).collect();
    loop {
        let pair = (0..b.len())
            .flat_map(|i| (0..b.len()).map(move |j| (i, j)))
            .filter(|&(i, j)| i != j)
            .find_map(|(i, j)| join(b[i], b[j]).map(|n| (i, j, n)));
        let Some((i, j, n)) = pair else { break };
        b[i] = n;
        b.remove(j);
    }
    b.sort_by_key(|x| (x.0, x.1));
    let list: Vec<Area> = b
        .iter()
        .map(|&(r0, c0, r1, c1)| Area::rect(r0, c0, r1, c1))
        .collect();
    refs::sqref_text(&list)
}

/// 원래 우선순위 차례로 `first` 부터 번호를 준다
fn renumber(rules: Vec<&mut CfRule>, first: u32) -> u32 {
    let mut list = rules;
    list.sort_by_key(|r| r.priority);
    let mut p = first;
    for r in list {
        r.priority = p;
        p += 1;
    }
    p
}

/// 붙여넣기: 붙인 묶음들을 시트에 넣는다 (`blocks` 의 규칙은 원래 우선순위)
pub(crate) fn place_pasted(sheet: &mut Sheet, mut blocks: Vec<ConditionalFormat>) {
    if blocks.is_empty() {
        return;
    }
    let mut dups: Vec<ConditionalFormat> =
        blocks.iter().filter(|b| has_databar(b)).cloned().collect();
    let k: usize = blocks.iter().chain(&dups).map(|b| b.rules.len()).sum();
    shift(sheet, k as u32);
    let next = renumber(
        blocks.iter_mut().flat_map(|b| b.rules.iter_mut()).collect(),
        1,
    );
    renumber(
        dups.iter_mut().flat_map(|b| b.rules.iter_mut()).collect(),
        next,
    );
    sheet.conditional_formats.extend(blocks);
    sheet.conditional_formats.extend(dups);
}

/// 채우기: `lines[i][b]` 는 i 번째 채울 줄에 놓은 b 번째 묶음 (규칙은 원래 우선순위)
pub(crate) fn place_filled(sheet: &mut Sheet, lines: Vec<Vec<ConditionalFormat>>) {
    let Some(first) = lines.first() else { return };
    let nb = first.len();
    if nb == 0 {
        return;
    }
    let total: usize = lines.iter().flatten().map(|b| b.rules.len()).sum();
    // 원래 묶음 (규칙 하나뿐인 묶음을 넓힐 때) — 우선순위 목록으로 찾는다
    let source: Vec<Option<usize>> = first
        .iter()
        .map(|b| {
            let key: Vec<u32> = b.rules.iter().map(|r| r.priority).collect();
            sheet
                .conditional_formats
                .iter()
                .position(|c| c.rules.iter().map(|r| r.priority).collect::<Vec<_>>() == key)
        })
        .collect();
    shift(sheet, total as u32);
    // 넣는 차례 j (0부터) 의 우선순위는 total − j
    let mut prio: Vec<Vec<Vec<u32>>> = vec![vec![Vec::new(); nb]; lines.len()];
    let mut j = 0usize;
    for (i, line) in lines.iter().enumerate() {
        for (b, block) in line.iter().enumerate() {
            let mut order: Vec<usize> = (0..block.rules.len()).collect();
            order.sort_by_key(|&k| block.rules[k].priority);
            let mut p = vec![0; block.rules.len()];
            for k in order {
                p[k] = (total - j) as u32;
                j += 1;
            }
            prio[i][b] = p;
        }
    }
    let last = lines.len() - 1;
    for b in 0..nb {
        let block = &lines[0][b];
        let areas: Vec<Area> = lines
            .iter()
            .flat_map(|l| refs::parse_sqref(&l[b].sqref))
            .collect();
        if block.rules.len() == 1
            && !has_databar(block)
            && let Some(si) = source[b]
        {
            let src = &mut sheet.conditional_formats[si];
            let mut all = refs::parse_sqref(&src.sqref);
            all.extend(areas);
            src.sqref = sorted_sqref(all);
            src.rules[0].priority = prio[last][b][0];
            let moved = sheet.conditional_formats.remove(si);
            sheet.conditional_formats.push(moved);
            continue;
        }
        if has_databar(block) && block.rules.len() > 1 {
            for (i, line) in lines.iter().enumerate() {
                let mut m = line[b].clone();
                for (k, r) in m.rules.iter_mut().enumerate() {
                    r.priority = prio[i][b][k];
                }
                sheet.conditional_formats.push(m);
            }
            continue;
        }
        let mut m = block.clone();
        m.sqref = sorted_sqref(areas);
        for (k, r) in m.rules.iter_mut().enumerate() {
            r.priority = prio[last][b][k];
        }
        sheet.conditional_formats.push(m);
    }
}

/// 같은 규칙인가 (우선순위 차례로 맞대어 — 채우기로 차례가 뒤집힌 묶음은 다르다. 우선순위 값은 빼고, 수식은
/// `d` 만큼 옮겨 맞춘 뒤)
fn same_rules(a: &ConditionalFormat, b: &ConditionalFormat, d: (i64, i64)) -> bool {
    let by_priority = |c: &ConditionalFormat| {
        let mut v: Vec<CfRule> = c.rules.clone();
        v.sort_by_key(|r| r.priority);
        v
    };
    let (ra, rb) = (by_priority(a), by_priority(b));
    ra.len() == rb.len()
        && ra.iter().zip(&rb).all(|(x, y)| {
            let mut y = y.clone();
            y.priority = x.priority;
            for f in &mut y.formulas {
                *f = crate::formula_text::shift_references(f, d.0, d.1);
            }
            *x == y
        })
}

fn top_left(sqref: &str) -> Option<(i64, i64)> {
    refs::parse_sqref(sqref).first().map(|a| {
        let b = a.bounds();
        (b.0 as i64, b.1 as i64)
    })
}

/// 옮기기: `moved` 는 (원래 묶음 규칙들의 우선순위, 옮긴 몫 — 수식은 옮긴 몫에 맞춘 것). 원래 묶음(남은 몫)은
/// 이미 시트에 있으면 우선순위로 찾는다. 같은 시트로 옮기면 `same_sheet`
pub(crate) fn place_moved(
    sheet: &mut Sheet,
    moved: Vec<(Vec<u32>, ConditionalFormat)>,
    same_sheet: bool,
) {
    let moved: Vec<(Option<usize>, ConditionalFormat)> = moved
        .into_iter()
        .map(|(key, m)| {
            let at = sheet
                .conditional_formats
                .iter()
                .position(|c| c.rules.iter().map(|r| r.priority).collect::<Vec<_>>() == key);
            (at, m)
        })
        .collect();
    if moved.is_empty() {
        return;
    }
    let dups: Vec<ConditionalFormat> = moved
        .iter()
        .filter(|(_, b)| has_databar(b))
        .map(|(_, b)| b.clone())
        .collect();
    let k: usize = moved
        .iter()
        .map(|(_, b)| &b.rules)
        .chain(dups.iter().map(|b| &b.rules))
        .map(Vec::len)
        .sum();
    shift(sheet, k as u32);
    // 새 우선순위 (원래 우선순위 차례)
    let mut moved = moved;
    let next = renumber(
        moved
            .iter_mut()
            .flat_map(|(_, b)| b.rules.iter_mut())
            .collect(),
        1,
    );
    let mut dups = dups;
    renumber(
        dups.iter_mut().flat_map(|b| b.rules.iter_mut()).collect(),
        next,
    );
    let mut remove: Vec<usize> = Vec::new();
    for (src, m) in moved {
        let target = src.filter(|_| same_sheet && mergeable(&m));
        let Some(si) = target else {
            sheet.conditional_formats.push(m);
            continue;
        };
        // 같은 규칙의 다른 묶음 (범위에 따라 달라지지 않는 규칙만)
        let anchor = top_left(&m.sqref).unwrap_or((0, 0));
        let others: Vec<usize> = if m.rules.iter().any(|r| range_dependent(&r.kind)) {
            Vec::new()
        } else {
            (0..sheet.conditional_formats.len())
                .filter(|&i| i != si && !remove.contains(&i))
                .filter(|&i| {
                    let c = &sheet.conditional_formats[i];
                    top_left(&c.sqref)
                        .is_some_and(|t| same_rules(&m, c, (anchor.0 - t.0, anchor.1 - t.1)))
                })
                .collect()
        };
        let mut areas = refs::parse_sqref(&sheet.conditional_formats[si].sqref);
        areas.extend(refs::parse_sqref(&m.sqref));
        for &i in &others {
            areas.extend(refs::parse_sqref(&sheet.conditional_formats[i].sqref));
        }
        let block = &mut sheet.conditional_formats[si];
        if others.is_empty() && !block.sqref.is_empty() {
            // 남은 몫의 수식 그대로, 우선순위만 새것
            for (r, n) in block.rules.iter_mut().zip(&m.rules) {
                r.priority = n.priority;
            }
        } else {
            block.rules = m.rules.clone();
        }
        block.sqref = sorted_sqref(areas);
        remove.extend(others);
    }
    remove.sort_unstable();
    for i in remove.into_iter().rev() {
        sheet.conditional_formats.remove(i);
    }
    sheet.conditional_formats.extend(dups);
}
