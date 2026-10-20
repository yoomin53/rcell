// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 워크시트 파트(`xl/worksheets/sheetN.xml`) 파서.

use std::collections::HashMap;

use crate::formula_text::shift_references;
use crate::model::{
    AutoFilter, Cell, CellRange, CellRef, CellValue, CfRule, Cfvo, ColInfo, ColorScale,
    ConditionalFormat, DataBar, DataValidation, FilterColumn, Formula, FormulaKind, HeaderFooter,
    Hyperlink, IconSet, PageLayout, PageMargins, PageSetup, Pane, PrintOptions, RichText, RowInfo,
    Sheet, SheetProtection, SheetView,
};
use crate::ns;
use crate::package::Relationship;
use crate::parser::shared_strings::parse_rich_text;
use crate::parser::styles::parse_color;
use crate::xml::Element;

/// 워크시트 DOM 을 `sheet` 에 채운다. `sheet` 의 이름·ID·파트는 호출자가 미리 넣는다.
pub fn parse_worksheet(
    root: &Element,
    sheet: &mut Sheet,
    shared: &[RichText],
    rels: &[Relationship],
) {
    let rel_by_id: HashMap<&str, &Relationship> = rels.iter().map(|r| (r.id.as_str(), r)).collect();

    sheet.dimension = root
        .sml_child("dimension")
        .and_then(|d| d.attr("ref"))
        .map(str::to_string);

    if let Some(pr) = root.sml_child("sheetPr") {
        sheet.filter_mode = pr.attr_bool("filterMode").unwrap_or(false);
        if let Some(o) = pr.sml_child("outlinePr") {
            sheet.summary_above = !o.attr_bool("summaryBelow").unwrap_or(true);
            sheet.summary_left = !o.attr_bool("summaryRight").unwrap_or(true);
        }
        sheet.code_name = pr.attr("codeName").map(str::to_string);
        sheet.tab_color = pr.sml_child("tabColor").map(parse_color);
        if let Some(p) = pr.sml_child("pageSetUpPr") {
            sheet.page.auto_page_breaks = p.attr_bool("autoPageBreaks");
            sheet.page.fit_to_page = p.attr_bool("fitToPage");
        }
    }

    sheet.show_grid_lines = true;
    if let Some(view) = root
        .sml_child("sheetViews")
        .and_then(|v| v.sml_child("sheetView"))
    {
        sheet.show_grid_lines = view.attr_bool("showGridLines").unwrap_or(true);
        sheet.view = parse_sheet_view(view);
    }

    if let Some(f) = root.sml_child("sheetFormatPr") {
        sheet.default_col_width = f.attr_f64("defaultColWidth");
        sheet.default_row_height = f.attr_f64("defaultRowHeight");
    }

    if let Some(cols) = root.sml_child("cols") {
        sheet.cols = cols
            .sml_children("col")
            .map(|c| ColInfo {
                min: c.attr_u32("min").unwrap_or(1),
                max: c.attr_u32("max").unwrap_or(1),
                width: c.attr_f64("width"),
                style: c.attr_u32("style"),
                hidden: c.attr_bool("hidden").unwrap_or(false),
                custom_width: c.attr_bool("customWidth").unwrap_or(false),
                outline_level: c.attr_u32("outlineLevel").unwrap_or(0).min(7) as u8,
                collapsed: c.attr_bool("collapsed").unwrap_or(false),
            })
            .collect();
    }

    if let Some(data) = root.sml_child("sheetData") {
        parse_sheet_data(data, sheet, shared);
    }

    if let Some(m) = root.sml_child("mergeCells") {
        sheet.merges = m
            .sml_children("mergeCell")
            .filter_map(|c| c.attr("ref").and_then(CellRange::parse))
            .collect();
    }

    if let Some(h) = root.sml_child("hyperlinks") {
        sheet.hyperlinks = h
            .sml_children("hyperlink")
            .map(|l| Hyperlink {
                range: l.attr("ref").unwrap_or("").to_string(),
                location: l.attr("location").map(str::to_string),
                target: l
                    .rel_id()
                    .and_then(|id| rel_by_id.get(id))
                    .map(|r| r.target.clone()),
                tooltip: l.attr("tooltip").map(str::to_string),
                display: l.attr("display").map(str::to_string),
            })
            .collect();
    }

    if let Some(p) = root.sml_child("sheetProtection") {
        sheet.protection = Some(SheetProtection {
            password: p.attr("password").map(str::to_string),
            flags: p
                .attrs
                .iter()
                .filter(|a| a.ns.is_none() && a.name != "password")
                .filter(|a| matches!(a.value.as_str(), "0" | "1" | "true" | "false"))
                .map(|a| (a.name.clone(), crate::xml::parse_bool(&a.value)))
                .collect(),
        });
    }
    if let Some(af) = root.sml_child("autoFilter") {
        sheet.auto_filter = Some(AutoFilter {
            range: af.attr("ref").unwrap_or("").to_string(),
            columns: af
                .sml_children("filterColumn")
                .map(parse_filter_column)
                .collect(),
        });
    }
    sheet.conditional_formats = root
        .sml_children("conditionalFormatting")
        .map(|cf| ConditionalFormat {
            sqref: cf.attr("sqref").unwrap_or("").to_string(),
            rules: cf.sml_children("cfRule").map(parse_cf_rule).collect(),
        })
        .collect();
    if let Some(dvs) = root.sml_child("dataValidations") {
        sheet.data_validations = dvs
            .sml_children("dataValidation")
            .map(|d| {
                let s = |n: &str| d.attr(n).map(str::to_string);
                DataValidation {
                    sqref: d.attr("sqref").unwrap_or("").to_string(),
                    kind: d.attr("type").unwrap_or("none").to_string(),
                    operator: s("operator"),
                    formula1: d.sml_child("formula1").map(|f| f.text()),
                    formula2: d.sml_child("formula2").map(|f| f.text()),
                    allow_blank: d.attr_bool("allowBlank").unwrap_or(false),
                    hide_drop_down: d.attr_bool("showDropDown").unwrap_or(false),
                    show_input: d.attr_bool("showInputMessage").unwrap_or(false),
                    show_error: d.attr_bool("showErrorMessage").unwrap_or(false),
                    error_style: s("errorStyle"),
                    error_title: s("errorTitle"),
                    error: s("error"),
                    prompt_title: s("promptTitle"),
                    prompt: s("prompt"),
                }
            })
            .collect();
    }
    parse_page_layout(root, &mut sheet.page);

    for d in root.sml_children("drawing") {
        if let Some(r) = d.rel_id().and_then(|id| rel_by_id.get(id)) {
            sheet.drawing_parts.push(r.target.clone());
        }
    }

    // x14 확장: <x:extLst><x:ext uri="{05C60535-...}"><x14:sparklineGroups>
    if let Some(ext_lst) = root.sml_child("extLst") {
        for ext in ext_lst.sml_children("ext") {
            if let Some(groups) = ext.child_ns(ns::X14, "sparklineGroups") {
                sheet.sparkline_groups += groups
                    .elements()
                    .filter(|g| g.is(ns::X14, "sparklineGroup"))
                    .count();
            }
        }
    }
}

fn parse_sheet_view(v: &Element) -> SheetView {
    let s = |name: &str| v.attr(name).map(str::to_string);
    let mut view = SheetView {
        show_row_col_headers: v.attr_bool("showRowColHeaders").unwrap_or(true),
        show_zeros: v.attr_bool("showZeros").unwrap_or(true),
        show_formulas: v.attr_bool("showFormulas").unwrap_or(false),
        tab_selected: v.attr_bool("tabSelected").unwrap_or(false),
        view: s("view"),
        top_left_cell: s("topLeftCell"),
        zoom_scale: v.attr_u32("zoomScale"),
        zoom_scale_normal: v.attr_u32("zoomScaleNormal"),
        zoom_scale_sheet_layout_view: v.attr_u32("zoomScaleSheetLayoutView"),
        ..Default::default()
    };
    if let Some(p) = v.sml_child("pane") {
        view.pane = Some(Pane {
            x_split: p.attr_f64("xSplit").unwrap_or(0.0),
            y_split: p.attr_f64("ySplit").unwrap_or(0.0),
            top_left_cell: p.attr("topLeftCell").map(str::to_string),
            active_pane: p.attr("activePane").map(str::to_string),
            state: p.attr("state").map(str::to_string),
        });
    }
    // 틀이 나뉘면 selection 이 여럿이다: 활성 창의 것을 쓴다
    let active = view.pane.as_ref().and_then(|p| p.active_pane.clone());
    let sel = v
        .sml_children("selection")
        .find(|e| e.attr("pane").map(str::to_string) == active)
        .or_else(|| v.sml_child("selection"));
    if let Some(sel) = sel {
        view.active_cell = sel.attr("activeCell").map(str::to_string);
        view.sqref = sel.attr("sqref").map(str::to_string);
    }
    view
}

fn parse_filter_column(c: &Element) -> FilterColumn {
    let mut fc = FilterColumn {
        col_id: c.attr_u32("colId").unwrap_or(0),
        ..Default::default()
    };
    if let Some(f) = c.sml_child("filters") {
        fc.blank = f.attr_bool("blank").unwrap_or(false);
        fc.values = f
            .sml_children("filter")
            .filter_map(|x| x.attr("val").map(str::to_string))
            .collect();
    }
    if let Some(cf) = c.sml_child("customFilters") {
        fc.custom_and = cf.attr_bool("and").unwrap_or(false);
        fc.custom = cf
            .sml_children("customFilter")
            .map(|x| {
                (
                    x.attr("operator").unwrap_or("equal").to_string(),
                    x.attr("val").unwrap_or("").to_string(),
                )
            })
            .collect();
    }
    if let Some(t) = c.sml_child("top10") {
        fc.top10 = Some((
            t.attr_bool("top").unwrap_or(true),
            t.attr_bool("percent").unwrap_or(false),
            t.attr_f64("val").unwrap_or(10.0),
        ));
    }
    fc.dynamic = c
        .sml_child("dynamicFilter")
        .and_then(|d| d.attr("type"))
        .map(str::to_string);
    fc
}

fn parse_cfvo(e: &Element) -> Cfvo {
    Cfvo {
        kind: e.attr("type").unwrap_or("").to_string(),
        val: e.attr("val").map(str::to_string),
        gte: e.attr_bool("gte").unwrap_or(true),
    }
}

fn parse_cf_rule(r: &Element) -> CfRule {
    let s = |n: &str| r.attr(n).map(str::to_string);
    let cfvos = |e: &Element| e.sml_children("cfvo").map(parse_cfvo).collect::<Vec<_>>();
    CfRule {
        kind: r.attr("type").unwrap_or("").to_string(),
        priority: r.attr_u32("priority").unwrap_or(0),
        dxf_id: r.attr_u32("dxfId"),
        stop_if_true: r.attr_bool("stopIfTrue").unwrap_or(false),
        operator: s("operator"),
        formulas: r.sml_children("formula").map(|f| f.text()).collect(),
        text: s("text"),
        time_period: s("timePeriod"),
        rank: r.attr_u32("rank"),
        percent: r.attr_bool("percent").unwrap_or(false),
        bottom: r.attr_bool("bottom").unwrap_or(false),
        above_average: r.attr_bool("aboveAverage"),
        equal_average: r.attr_bool("equalAverage").unwrap_or(false),
        std_dev: r.attr_u32("stdDev"),
        color_scale: r.sml_child("colorScale").map(|c| ColorScale {
            cfvos: cfvos(c),
            colors: c.sml_children("color").map(parse_color).collect(),
        }),
        data_bar: r.sml_child("dataBar").map(|d| DataBar {
            cfvos: cfvos(d),
            color: d.sml_child("color").map(parse_color),
            show_value: d.attr_bool("showValue").unwrap_or(true),
            min_length: d.attr_u32("minLength"),
            max_length: d.attr_u32("maxLength"),
        }),
        icon_set: r.sml_child("iconSet").map(|i| IconSet {
            set: i.attr("iconSet").unwrap_or("3TrafficLights1").to_string(),
            cfvos: cfvos(i),
            show_value: i.attr_bool("showValue").unwrap_or(true),
            reverse: i.attr_bool("reverse").unwrap_or(false),
            percent: i.attr_bool("percent").unwrap_or(true),
        }),
    }
}

fn parse_page_layout(root: &Element, page: &mut PageLayout) {
    if let Some(p) = root.sml_child("printOptions") {
        page.print_options = Some(PrintOptions {
            horizontal_centered: p.attr_bool("horizontalCentered").unwrap_or(false),
            vertical_centered: p.attr_bool("verticalCentered").unwrap_or(false),
            headings: p.attr_bool("headings").unwrap_or(false),
            grid_lines: p.attr_bool("gridLines").unwrap_or(false),
        });
    }
    if let Some(m) = root.sml_child("pageMargins") {
        let f = |n: &str, d: f64| m.attr_f64(n).unwrap_or(d);
        page.margins = Some(PageMargins {
            left: f("left", 0.7),
            right: f("right", 0.7),
            top: f("top", 0.75),
            bottom: f("bottom", 0.75),
            header: f("header", 0.3),
            footer: f("footer", 0.3),
        });
    }
    if let Some(p) = root.sml_child("pageSetup") {
        page.setup = Some(PageSetup {
            paper_size: p.attr_u32("paperSize"),
            scale: p.attr_u32("scale"),
            first_page_number: p.attr_u32("firstPageNumber"),
            fit_to_width: p.attr_u32("fitToWidth"),
            fit_to_height: p.attr_u32("fitToHeight"),
            orientation: p.attr("orientation").map(str::to_string),
            page_order: p.attr("pageOrder").map(str::to_string),
            use_printer_defaults: p.attr_bool("usePrinterDefaults").unwrap_or(true),
            black_and_white: p.attr_bool("blackAndWhite").unwrap_or(false),
            draft: p.attr_bool("draft").unwrap_or(false),
            use_first_page_number: p.attr_bool("useFirstPageNumber").unwrap_or(false),
            horizontal_dpi: p.attr_u32("horizontalDpi"),
            vertical_dpi: p.attr_u32("verticalDpi"),
            copies: p.attr_u32("copies"),
        });
    }
    if let Some(h) = root.sml_child("headerFooter") {
        let t = |n: &str| h.sml_child(n).map(|e| e.text());
        page.header_footer = Some(HeaderFooter {
            different_odd_even: h.attr_bool("differentOddEven").unwrap_or(false),
            different_first: h.attr_bool("differentFirst").unwrap_or(false),
            scale_with_doc: h.attr_bool("scaleWithDoc").unwrap_or(true),
            align_with_margins: h.attr_bool("alignWithMargins").unwrap_or(true),
            odd_header: t("oddHeader"),
            odd_footer: t("oddFooter"),
            even_header: t("evenHeader"),
            even_footer: t("evenFooter"),
            first_header: t("firstHeader"),
            first_footer: t("firstFooter"),
        });
    }
    let breaks = |name: &str| -> Vec<u32> {
        root.sml_child(name)
            .map(|b| {
                b.sml_children("brk")
                    .filter_map(|e| e.attr_u32("id"))
                    .collect()
            })
            .unwrap_or_default()
    };
    page.row_breaks = breaks("rowBreaks");
    page.col_breaks = breaks("colBreaks");
}

fn parse_sheet_data(data: &Element, sheet: &mut Sheet, shared: &[RichText]) {
    let mut next_row: u32 = 0;
    for row in data.sml_children("row") {
        // r 생략 시 직전 행 다음 행 (ECMA-376 18.3.1.73)
        let row_idx = row
            .attr_u32("r")
            .filter(|&r| r > 0)
            .map(|r| r - 1)
            .unwrap_or(next_row);
        next_row = row_idx + 1;

        let info = RowInfo {
            index: row_idx + 1,
            height: row.attr_f64("ht"),
            custom_height: row.attr_bool("customHeight").unwrap_or(false),
            hidden: row.attr_bool("hidden").unwrap_or(false),
            style: row
                .attr_bool("customFormat")
                .unwrap_or(false)
                .then(|| row.attr_u32("s"))
                .flatten(),
            outline_level: row.attr_u32("outlineLevel").unwrap_or(0).min(7) as u8,
            collapsed: row.attr_bool("collapsed").unwrap_or(false),
        };
        if info.height.is_some()
            || info.hidden
            || info.style.is_some()
            || info.outline_level > 0
            || info.collapsed
        {
            sheet.rows.push(info);
        }

        let mut next_col: u32 = 0;
        for c in row.sml_children("c") {
            let r = c.attr("r").and_then(CellRef::parse).unwrap_or(CellRef {
                row: row_idx,
                col: next_col,
            });
            next_col = r.col + 1;
            sheet.cells.push(parse_cell(c, r, shared));
        }
    }
    sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
    expand_shared_formulas(&mut sheet.cells);
}

/// 공유 수식 종속 셀(`<f t="shared" si=".."/>`)에 기준 셀 원문을 위치 차이만큼 옮겨 채운다.
/// 기준 셀은 공유 범위의 왼쪽 위라서 행 우선 순서에서 먼저 나온다.
fn expand_shared_formulas(cells: &mut [Cell]) {
    let mut masters: HashMap<u32, (CellRef, String)> = HashMap::new();
    for c in cells.iter_mut() {
        let Some(f) = c.formula.as_mut() else {
            continue;
        };
        let (FormulaKind::Shared, Some(si)) = (f.kind, f.shared_index) else {
            continue;
        };
        if !f.text.is_empty() {
            masters.insert(si, (c.r, f.text.clone()));
        } else if let Some((m, text)) = masters.get(&si) {
            f.text = shift_references(
                text,
                c.r.row as i64 - m.row as i64,
                c.r.col as i64 - m.col as i64,
            );
        }
    }
}

fn parse_cell(c: &Element, r: CellRef, shared: &[RichText]) -> Cell {
    let style = c.attr_u32("s").unwrap_or(0);
    let raw = c.sml_child("v").map(|v| v.text());
    let value = match (c.attr("t").unwrap_or("n"), raw) {
        ("inlineStr", _) => c
            .sml_child("is")
            .map(|is| CellValue::Text(parse_rich_text(is).text))
            .unwrap_or_default(),
        (_, None) => CellValue::Empty,
        ("s", Some(v)) => v
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|i| shared.get(i))
            .map(|s| CellValue::Text(s.text.clone()))
            .unwrap_or(CellValue::Error(format!("#SST?{}", v.trim()))),
        ("str", Some(v)) => CellValue::Text(v),
        ("b", Some(v)) => CellValue::Bool(v.trim() == "1" || v.trim() == "true"),
        ("e", Some(v)) => CellValue::Error(v),
        ("d", Some(v)) => CellValue::Date(v),
        (_, Some(v)) => v
            .trim()
            .parse::<f64>()
            .map(CellValue::Number)
            .unwrap_or(CellValue::Text(v)),
    };
    let formula = c.sml_child("f").map(|f| Formula {
        text: f.text(),
        kind: match f.attr("t") {
            Some("shared") => FormulaKind::Shared,
            Some("array") => FormulaKind::Array,
            Some("dataTable") => FormulaKind::DataTable,
            _ => FormulaKind::Normal,
        },
        shared_index: f.attr_u32("si"),
        range: f.attr("ref").map(str::to_string),
    });
    Cell {
        r,
        style,
        value,
        formula,
    }
}
