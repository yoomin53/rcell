// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표 파트 (`xl/tables/tableN.xml`, `x:table`) — 비교와 다른 형식 변환에 쓰는 모델. XML 원본의 파트는
//! 그대로 옮겨 쓴다 (docs/format/cell-2024.md 7.10절).

use crate::model::{AutoFilter, Table, TableColumn, TableStyleInfo};
use crate::parser::sheet::parse_filter_column;
use crate::xml::Element;

pub fn parse_table(root: &Element) -> Table {
    let s = |e: &Element, n: &str| e.attr(n).map(str::to_string);
    let d = Table::default();
    Table {
        id: root.attr_u32("id").unwrap_or(0),
        name: root.attr("name").unwrap_or("").to_string(),
        display_name: root
            .attr("displayName")
            .or(root.attr("name"))
            .unwrap_or("")
            .to_string(),
        range: root.attr("ref").unwrap_or("").to_string(),
        header_rows: root.attr_u32("headerRowCount").unwrap_or(d.header_rows),
        totals_rows: root.attr_u32("totalsRowCount").unwrap_or(d.totals_rows),
        totals_shown: root.attr_bool("totalsRowShown").unwrap_or(d.totals_shown),
        table_type: s(root, "tableType").filter(|t| t != "worksheet"),
        comment: s(root, "comment"),
        connection_id: root.attr_u32("connectionId"),
        dxfs: [
            "headerRowDxfId",
            "dataDxfId",
            "totalsRowDxfId",
            "tableBorderDxfId",
            "headerRowBorderDxfId",
            "totalsRowBorderDxfId",
        ]
        .map(|n| root.attr_u32(n)),
        cell_styles: ["headerRowCellStyle", "dataCellStyle", "totalsRowCellStyle"]
            .map(|n| s(root, n)),
        auto_filter: root.sml_child("autoFilter").map(|af| AutoFilter {
            range: af.attr("ref").unwrap_or("").to_string(),
            columns: af
                .sml_children("filterColumn")
                .map(parse_filter_column)
                .collect(),
        }),
        columns: root
            .sml_child("tableColumns")
            .into_iter()
            .flat_map(|c| c.sml_children("tableColumn"))
            .map(|c| TableColumn {
                id: c.attr_u32("id").unwrap_or(0),
                name: c.attr("name").unwrap_or("").to_string(),
                unique_name: s(c, "uniqueName"),
                totals_function: s(c, "totalsRowFunction").filter(|f| f != "none"),
                totals_label: s(c, "totalsRowLabel"),
                totals_formula: c.sml_child("totalsRowFormula").map(|f| f.text()),
                calculated_formula: c.sml_child("calculatedColumnFormula").map(|f| f.text()),
                dxfs: ["headerRowDxfId", "dataDxfId", "totalsRowDxfId"].map(|n| c.attr_u32(n)),
                query_table_field_id: c.attr_u32("queryTableFieldId"),
            })
            .collect(),
        style: root.sml_child("tableStyleInfo").map(|t| TableStyleInfo {
            name: s(t, "name"),
            first_column: t.attr_bool("showFirstColumn").unwrap_or(false),
            last_column: t.attr_bool("showLastColumn").unwrap_or(false),
            row_stripes: t.attr_bool("showRowStripes").unwrap_or(false),
            column_stripes: t.attr_bool("showColumnStripes").unwrap_or(false),
        }),
        part: None,
    }
}
