// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! OOXML 기반 .cell (한셀 2014 이후) 파서.

pub mod chart;
pub mod comments;
pub mod drawing;
pub mod shared_strings;
pub mod sheet;
pub mod styles;

use crate::error::{Error, Result};
use crate::model::{
    CellValue, DefinedName, ExternalLink, FileVersion, Sheet, SheetKind, SheetState, Workbook,
};
use crate::ns;
use crate::package::{Package, Relationship};

/// 관계 타입 URI 끝부분으로 판정한다 (Transitional/Strict 공통).
fn rel_is(r: &Relationship, suffix: &str) -> bool {
    r.rel_type.ends_with(suffix)
}

/// docProps/core.xml (OPC Core Properties): dc:title … dcterms:modified. 이름공간과 상관없이 지역 이름으로 읽는다.
pub(crate) fn read_core_properties(
    pkg: &mut Package,
    root_rels: &[crate::package::Relationship],
) -> Result<crate::model::DocProperties> {
    let mut p = crate::model::DocProperties::default();
    let Some(rel) = root_rels
        .iter()
        .find(|r| r.rel_type.ends_with("/metadata/core-properties") && !r.external)
    else {
        return Ok(p);
    };
    let Some(root) = pkg.read_xml(&rel.target)? else {
        return Ok(p);
    };
    for e in root.elements() {
        let v = e.text().trim().to_string();
        if v.is_empty() {
            continue;
        }
        let slot = match e.name.as_str() {
            "title" => &mut p.title,
            "subject" => &mut p.subject,
            "creator" => &mut p.creator,
            "keywords" => &mut p.keywords,
            "description" => &mut p.description,
            "lastModifiedBy" => &mut p.last_modified_by,
            "revision" => &mut p.revision,
            "category" => &mut p.category,
            "contentStatus" => &mut p.content_status,
            "created" => &mut p.created,
            "modified" => &mut p.modified,
            "lastPrinted" => &mut p.last_printed,
            _ => continue,
        };
        *slot = Some(v);
    }
    Ok(p)
}

pub fn read_workbook(pkg: &mut Package) -> Result<Workbook> {
    let mut wb = Workbook::default();

    let root_rels = pkg.relationships("")?;
    let wb_part = root_rels
        .iter()
        .find(|r| rel_is(r, "/officeDocument") && !r.external)
        .map(|r| r.target.clone())
        .unwrap_or_else(|| "xl/workbook.xml".into());

    // 바이너리 OOXML (한셀 xlCellb / Excel .xlsb)
    if wb_part.to_ascii_lowercase().ends_with(".bin") {
        return crate::binary::reader::read_workbook(pkg, &wb_part);
    }

    if let Some(app) = root_rels.iter().find(|r| rel_is(r, "/extended-properties"))
        && let Some(props) = pkg.read_xml(&app.target)?
    {
        let field = |name: &str| {
            props
                .child_ns(ns::EXTENDED_PROPS, name)
                .map(|e| e.text().trim().to_string())
        };
        wb.application = field("Application");
        wb.app_version = field("AppVersion");
    }
    wb.properties = read_core_properties(pkg, &root_rels)?;

    let root = pkg
        .read_xml(&wb_part)?
        .ok_or_else(|| Error::MissingPart(wb_part.clone()))?;
    let wb_rels = pkg.relationships(&wb_part)?;

    if let Some(fv) = root.sml_child("fileVersion") {
        wb.file_version = Some(FileVersion {
            app_name: fv.attr("appName").map(str::to_string),
            last_edited: fv.attr("lastEdited").map(str::to_string),
            lowest_edited: fv.attr("lowestEdited").map(str::to_string),
            rup_build: fv.attr("rupBuild").map(str::to_string),
        });
    }
    if let Some(pr) = root.sml_child("workbookPr") {
        wb.date1904 = pr.attr_bool("date1904").unwrap_or(false);
    }
    if let Some(p) = root.sml_child("workbookProtection") {
        wb.protection = Some(crate::model::WorkbookProtection {
            lock_structure: p.attr_bool("lockStructure").unwrap_or(false),
            lock_windows: p.attr_bool("lockWindows").unwrap_or(false),
            password: p.attr("workbookPassword").map(str::to_string),
        });
    }
    if let Some(calc) = root.sml_child("calcPr") {
        for a in calc
            .attrs
            .iter()
            .filter(|a| a.ns.as_deref() == Some(ns::HS))
        {
            if a.name == "hclCalcId" {
                wb.hancom.calc_id = a.value.trim().parse().ok();
            } else {
                wb.hancom
                    .other
                    .insert(format!("calcPr/{}", a.name), a.value.clone());
            }
        }
    }
    if let Some(names) = root.sml_child("definedNames") {
        wb.defined_names = names
            .sml_children("definedName")
            .map(|d| DefinedName {
                name: d.attr("name").unwrap_or("").to_string(),
                local_sheet_id: d.attr_u32("localSheetId"),
                hidden: d.attr_bool("hidden").unwrap_or(false),
                formula: d.text(),
            })
            .collect();
    }
    // 외부 연결: externalReference 순서가 수식의 [n] 번호다
    if let Some(refs) = root.sml_child("externalReferences") {
        for er in refs.sml_children("externalReference") {
            let mut link = ExternalLink::default();
            if let Some(rel) = er
                .rel_id()
                .and_then(|id| wb_rels.iter().find(|r| r.id == id))
                && let Some(e) = pkg.read_xml(&rel.target)?
                && let Some(book) = e.sml_child("externalBook")
            {
                let rels = pkg.relationships(&rel.target)?;
                let path = book
                    .rel_id()
                    .and_then(|id| rels.iter().find(|r| r.id == id));
                link.target = path.map(|r| r.target.clone()).unwrap_or_default();
                link.path_missing = path.is_some_and(|r| r.rel_type.ends_with("/xlPathMissing"));
                if let Some(names) = book.sml_child("sheetNames") {
                    link.sheet_names = names
                        .sml_children("sheetName")
                        .map(|s| s.attr("val").unwrap_or("").to_string())
                        .collect();
                }
                if let Some(set) = book.sml_child("sheetDataSet") {
                    link.sheet_data = set
                        .sml_children("sheetData")
                        .map(|sd| crate::model::ExternalSheetData {
                            sheet_id: sd.attr_u32("sheetId").unwrap_or(0),
                            refresh_error: sd.attr_bool("refreshError").unwrap_or(false),
                            cells: sd
                                .sml_children("row")
                                .flat_map(|row| row.sml_children("cell"))
                                .filter_map(|c| {
                                    let r = crate::model::CellRef::parse(c.attr("r")?)?;
                                    let v = c.sml_child("v").map(|v| v.text()).unwrap_or_default();
                                    let value = match c.attr("t").unwrap_or("n") {
                                        "s" | "str" | "inlineStr" => CellValue::Text(v),
                                        "b" => CellValue::Bool(v.trim() == "1"),
                                        "e" => CellValue::Error(v),
                                        _ => v
                                            .trim()
                                            .parse()
                                            .map(CellValue::Number)
                                            .unwrap_or(CellValue::Empty),
                                    };
                                    Some((r, value))
                                })
                                .collect(),
                        })
                        .collect();
                }
            }
            wb.external_links.push(link);
        }
    }
    wb.hancom.vbs_project_part = wb_rels
        .iter()
        .find(|r| r.rel_type == ns::REL_HANCOM_VBS_PROJECT)
        .map(|r| r.target.clone());

    if let Some(sst) = wb_rels.iter().find(|r| rel_is(r, "/sharedStrings"))
        && let Some(e) = pkg.read_xml(&sst.target)?
    {
        wb.shared_strings = shared_strings::parse_shared_strings(&e);
    }
    if let Some(th) = wb_rels.iter().find(|r| rel_is(r, "/theme"))
        && let Some(e) = pkg.read_xml(&th.target)?
    {
        let mut theme = crate::theme::parse_theme(&e);
        theme.source = pkg.read_bytes(&th.target)?;
        wb.theme = Some(theme);
    }
    if let Some(st) = wb_rels.iter().find(|r| rel_is(r, "/styles"))
        && let Some(e) = pkg.read_xml(&st.target)?
    {
        wb.styles = styles::parse_styles(&e);
    }

    let Some(sheets) = root.sml_child("sheets") else {
        return Ok(wb);
    };
    for s in sheets.sml_children("sheet") {
        let Some(rel) = s
            .rel_id()
            .and_then(|id| wb_rels.iter().find(|r| r.id == id))
        else {
            continue;
        };
        let mut sheet = Sheet {
            name: s.attr("name").unwrap_or("").to_string(),
            sheet_id: s.attr_u32("sheetId").unwrap_or(0),
            state: match s.attr("state") {
                Some("hidden") => SheetState::Hidden,
                Some("veryHidden") => SheetState::VeryHidden,
                _ => SheetState::Visible,
            },
            kind: if rel_is(rel, "/worksheet") {
                SheetKind::Worksheet
            } else if rel_is(rel, "/chartsheet") {
                SheetKind::Chartsheet
            } else {
                SheetKind::Other(rel.rel_type.clone())
            },
            part: rel.target.clone(),
            show_grid_lines: true,
            ..Default::default()
        };
        if sheet.kind == SheetKind::Worksheet
            && let Some(ws) = pkg.read_xml(&rel.target)?
        {
            let rels = pkg.relationships(&rel.target)?;
            sheet::parse_worksheet(&ws, &mut sheet, &wb.shared_strings, &rels);
            read_sheet_comments(pkg, &ws, &rels, &mut sheet)?;
            read_sheet_drawings(pkg, &rels, &mut wb.raw_parts, &mut sheet)?;
        }
        wb.sheets.push(sheet);
    }
    finish_selection(&root, &mut wb);
    Ok(wb)
}

/// 시트의 메모 파트(관계 `comments`)와 VML(`legacyDrawing`)을 읽는다. XML·바이너리 시트가 함께 쓴다.
pub fn read_sheet_comments(
    pkg: &mut Package,
    ws: &crate::xml::Element,
    rels: &[Relationship],
    sheet: &mut Sheet,
) -> Result<()> {
    let Some(c) = rels.iter().find(|r| rel_is(r, "/comments")) else {
        return Ok(());
    };
    if let Some(e) = pkg.read_xml(&c.target)? {
        sheet.comments = comments::parse_comments(&e);
    }
    let vml = ws
        .sml_child("legacyDrawing")
        .and_then(|d| d.rel_id())
        .and_then(|id| rels.iter().find(|r| r.id == id))
        .or_else(|| rels.iter().find(|r| rel_is(r, "/vmlDrawing")));
    if let Some(v) = vml
        && let Some(e) = pkg.read_xml(&v.target)?
    {
        comments::apply_vml(&e, &mut sheet.comments);
    }
    Ok(())
}

/// 시트의 그리기 파트(관계 `drawing`)를 읽는다. 차트·그림 파트는 관계 파일까지 그대로 보관한다.
/// XML·바이너리 시트가 함께 쓴다.
pub fn read_sheet_drawings(
    pkg: &mut Package,
    rels: &[Relationship],
    raw_parts: &mut Vec<crate::model::RawPart>,
    sheet: &mut Sheet,
) -> Result<()> {
    for d in rels.iter().filter(|r| rel_is(r, "/drawing")) {
        let Some(root) = pkg.read_xml_raw(&d.target)? else {
            continue;
        };
        let drels = pkg.relationships(&d.target)?;
        let mut shapes = drawing::parse_drawing(&root, &drels);
        // 차트 파트를 읽어 그리기·비교에 쓴다 (저장은 원본 파트 그대로).
        // c14:style 을 읽도록 mc:AlternateContent 는 parse_chart 가 펼친다
        fn charts(pkg: &mut Package, list: &mut [crate::model::Shape]) -> Result<()> {
            for s in list {
                if s.kind == crate::model::ShapeKind::Chart
                    && let Some(t) = s.target.clone()
                    && let Some(e) = pkg.read_xml_raw(&t)?
                {
                    s.chart = Some(chart::parse_chart(&e));
                }
                charts(pkg, &mut s.children)?;
            }
            Ok(())
        }
        charts(pkg, &mut shapes)?;
        let targets: Vec<String> = shapes
            .iter()
            .flat_map(|s| s.raw_rels.iter())
            .filter(|r| !r.3)
            .map(|r| r.2.clone())
            .collect();
        for t in targets {
            copy_part_tree(pkg, &t, raw_parts)?;
        }
        sheet.shapes.extend(shapes);
    }
    Ok(())
}

/// 파트와 그 관계 파일, 관계가 가리키는 내부 파트를 모두 그대로 복사한다.
fn copy_part_tree(
    pkg: &mut Package,
    name: &str,
    out: &mut Vec<crate::model::RawPart>,
) -> Result<()> {
    if out.iter().any(|p| p.name == name) {
        return Ok(());
    }
    let Some(data) = pkg.read_bytes(name)? else {
        return Ok(());
    };
    out.push(crate::model::RawPart {
        name: name.to_string(),
        content_type: pkg.content_type(name).map(str::to_string),
        data,
    });
    let rels_name = crate::package::rels_part_for(name);
    if let Some(data) = pkg.read_bytes(&rels_name)? {
        out.push(crate::model::RawPart {
            name: rels_name,
            content_type: None,
            data,
        });
        for r in pkg.relationships(name)? {
            if !r.external {
                copy_part_tree(pkg, &r.target, out)?;
            }
        }
    }
    Ok(())
}

fn finish_selection(root: &crate::xml::Element, wb: &mut Workbook) {
    if let Some(v) = root
        .sml_child("bookViews")
        .and_then(|b| b.sml_child("workbookView"))
    {
        let int = |n: &str| v.attr(n).and_then(|s| s.parse::<i64>().ok());
        wb.book_view = Some(crate::model::BookView {
            x_window: int("xWindow").unwrap_or(0) as i32,
            y_window: int("yWindow").unwrap_or(0) as i32,
            window_width: int("windowWidth").unwrap_or(0).max(0) as u32,
            window_height: int("windowHeight").unwrap_or(0).max(0) as u32,
            tab_ratio: int("tabRatio").map(|t| t.max(0) as u32),
            active_tab: int("activeTab").unwrap_or(0).max(0) as u32,
            first_sheet: int("firstSheet").unwrap_or(0).max(0) as u32,
        });
    }
    // 선택된 시트가 없으면 활성 탭(workbookView/@activeTab, 기본 0)이 선택된 것이다
    if !wb.sheets.iter().any(|s| s.view.tab_selected) {
        let active = root
            .sml_child("bookViews")
            .and_then(|b| b.sml_child("workbookView"))
            .and_then(|v| v.attr_u32("activeTab"))
            .unwrap_or(0) as usize;
        if let Some(s) = wb.sheets.get_mut(active) {
            s.view.tab_selected = true;
        }
    }
}
