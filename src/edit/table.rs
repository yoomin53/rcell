// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표(ListObject) 만들기 (한셀 COM `ListObjects.Add` 대조 실험 tb1·tb2 — docs/editing.md 24절).
//!
//! 한셀 COM 은 `ListObjects.Add` 를 `InvokeMember` 로 불러야 하고(PowerShell 직접 호출은 실패) 객체를 돌려주지 않는다.
//! - 이름은 `표N`(쓰지 않은 가장 작은 N), 번호는 가장 큰 번호 + 1, 스타일 `TableStyleMedium2`(줄무늬 행만), 열 이름은 머리글 칸 글.
//! - 범위가 한 행(머리글뿐)이면 그 아래에 빈 칸 한 행을 끼워 넣는다 (`A1` → `A1:A2`, 아래 칸은 한 칸씩 밀린다). 표를 만들면 그 범위의 시트 자동 필터는 없어진다.
//! - 표 안 테두리 서식: 요약 행 테두리 `totalsRowBorderDxfId`, 데이터 행이 둘 이상이면 열마다 `dataDxfId` — 둘 다 가로 안쪽선
//!   검정 차등 서식이고 문서에 이미 있으면 그 번호를 쓴다. 첫 열에는 `요약` 글(`totalsRowLabel`)이 붙는다.
//! - 머리글 없는 범위(`xlNo`)·요약 행·머리글 숨기기는 하지 않는다 (한셀은 머리글 행을 끼워 넣고 요약 칸에 수식을 쓴다).

use super::{err, refs};
use crate::model::{
    AutoFilter, Border, BorderEdge, CellRange, CellRef, Color, Dxf, Table, TableColumn,
    TableStyleInfo, Workbook,
};
use serde::Deserialize;

/// 표 만들기 설정
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableEdit {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    /// 줄무늬 행 (기본 참)
    #[serde(default)]
    pub stripes: Option<bool>,
    #[serde(default)]
    pub first_column: Option<bool>,
    #[serde(default)]
    pub last_column: Option<bool>,
    #[serde(default)]
    pub column_stripes: Option<bool>,
}

fn h_border() -> Dxf {
    Dxf {
        border: Some(Border {
            horizontal: Some(BorderEdge {
                style: None,
                color: Some(Color {
                    rgb: Some("ff000000".into()),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn is_h_border(d: &Dxf) -> bool {
    d.font.is_none()
        && d.fill.is_none()
        && d.num_fmt.is_none()
        && d.alignment.is_none()
        && d.border.as_ref().is_some_and(|b| {
            b.left.is_none()
                && b.right.is_none()
                && b.top.is_none()
                && b.bottom.is_none()
                && b.vertical.is_none()
                && b.horizontal.as_ref().is_some_and(|e| {
                    e.style.is_none()
                        && e.color.as_ref().and_then(|c| c.rgb.as_deref()) == Some("ff000000")
                })
        })
}

fn touches(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    !(a.2 < b.0 || b.2 < a.0 || a.3 < b.1 || b.3 < a.1)
}

/// 표 만들기 (`range` 첫 행이 머리글)
pub fn add_table(
    wb: &mut Workbook,
    si: usize,
    range: CellRange,
    o: &TableEdit,
) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    let (r0, mut r1, c0, c1) = (
        range.start.row,
        range.end.row,
        range.start.col,
        range.end.col,
    );
    // 한 행 범위(머리글뿐)는 그 아래에 빈 칸 한 행을 끼워 넣는다 (한셀 — 아래 칸을 한 칸씩 민다)
    if r1 == r0 {
        super::cells::shift_cells(
            wb,
            si,
            CellRange {
                start: CellRef {
                    row: r0 + 1,
                    col: c0,
                },
                end: CellRef {
                    row: r0 + 1,
                    col: c1,
                },
            },
            super::cells::ShiftDir::Down,
        )?;
        r1 += 1;
    }
    let rect = (r0, c0, r1, c1);
    let area = refs::Area::rect(r0, c0, r1, c1);
    let overlaps = wb.sheets[si]
        .tables
        .iter()
        .any(|t| refs::parse_area(&t.range).is_some_and(|a| touches(a.bounds(), rect)));
    if overlaps {
        return Err(err("표가 겹칩니다"));
    }
    // 열 이름: 머리글 칸 글
    let mut cols: Vec<String> = Vec::new();
    for c in c0..=c1 {
        let t = super::find::display_text(wb, si, CellRef { row: r0, col: c })
            .filter(|t| !t.is_empty())
            .ok_or_else(|| err("머리글 칸이 비어 있어 표를 만들 수 없습니다"))?;
        if cols.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            return Err(err("머리글이 겹쳐 표를 만들 수 없습니다"));
        }
        cols.push(t);
    }
    let id = wb
        .sheets
        .iter()
        .flat_map(|s| s.tables.iter().map(|t| t.id))
        .max()
        .unwrap_or(0)
        + 1;
    let all_names: Vec<String> = wb
        .sheets
        .iter()
        .flat_map(|s| s.tables.iter().map(|t| t.name.clone()))
        .collect();
    let name = match &o.name {
        Some(n) => {
            if all_names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                return Err(err(format!("같은 이름의 표가 있습니다: {n}")));
            }
            n.clone()
        }
        None => (1..)
            .map(|n| format!("표{n}"))
            .find(|n| !all_names.iter().any(|x| x.eq_ignore_ascii_case(n)))
            .unwrap_or_default(),
    };
    // 차등 서식: 요약 행 테두리와 (데이터 행이 둘 이상이면) 데이터 테두리
    let data_rows = r1 - r0;
    let mut dxf_ids: Vec<u32> = Vec::new();
    for _ in 0..(1 + usize::from(data_rows >= 2)) {
        let found = wb
            .styles
            .dxfs
            .iter()
            .enumerate()
            .position(|(i, d)| is_h_border(d) && !dxf_ids.contains(&(i as u32)));
        let n = match found {
            Some(i) => i as u32,
            None => {
                wb.styles.dxfs.push(h_border());
                (wb.styles.dxfs.len() - 1) as u32
            }
        };
        dxf_ids.push(n);
    }
    let text = area.text();
    let table = Table {
        id,
        name: name.clone(),
        display_name: name,
        range: text.clone(),
        totals_shown: false,
        dxfs: [None, None, None, None, None, Some(dxf_ids[0])],
        auto_filter: Some(AutoFilter {
            range: text,
            columns: Vec::new(),
            sort_state: None,
        }),
        columns: cols
            .iter()
            .enumerate()
            .map(|(i, n)| TableColumn {
                id: i as u32 + 1,
                name: n.clone(),
                totals_label: (i == 0).then(|| "요약".to_string()),
                dxfs: [None, dxf_ids.get(1).copied(), None],
                ..Default::default()
            })
            .collect(),
        style: Some(TableStyleInfo {
            name: Some(
                o.style
                    .clone()
                    .unwrap_or_else(|| "TableStyleMedium2".into()),
            ),
            first_column: o.first_column.unwrap_or(false),
            last_column: o.last_column.unwrap_or(false),
            row_stripes: o.stripes.unwrap_or(true),
            column_stripes: o.column_stripes.unwrap_or(false),
        }),
        ..Default::default()
    };
    // 그 범위의 시트 자동 필터는 표가 가져간다
    let sheet = &mut wb.sheets[si];
    let drop_filter = sheet
        .auto_filter
        .as_ref()
        .and_then(|af| refs::parse_area(&af.range))
        .is_some_and(|a| touches(a.bounds(), rect));
    if drop_filter {
        sheet.auto_filter = None;
        sheet.filter_mode = false;
        let lid = Some(si as u32);
        wb.defined_names
            .retain(|d| !(d.local_sheet_id == lid && d.name == "_xlnm._FilterDatabase"));
    }
    wb.sheets[si].tables.push(table);
    Ok(())
}
