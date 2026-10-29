// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! ODF 스프레드시트 `.ods` (OASIS ODF 1.2) 읽기·쓰기 — 시트, 칸 값·수식, 병합, 열 너비·숨김, 행 높이·숨김.
//!
//! 한셀의 `.ods` 저장(FileFormat 60)은 `content.xml`·`styles.xml`·`meta.xml`·`settings.xml`·매니페스트 (`mimetype`
//! 없음), 수식은 `of:=` 뒤에 A1 참조 그대로 (`[.A1]` 꼴이 아니다)와 인수 구분 `;`, 다른 시트 참조는 `#REF.` 로 잃는다.
//! 칸 서식은 표시 형식을 잃는다 (docs/format/other-formats.md 8절). rcell 은 값·수식·구조만 옮긴다.

use std::io::{Cursor, Read, Write};

use crate::error::{Error, Result};
use crate::model::{Cell, CellRange, CellRef, CellValue, ColInfo, Formula, RowInfo, Workbook};
use crate::xml::Element;

const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const NS_TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const NS_STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

/// 한셀 기본 열 너비 (글자 9.00390625 ↔ 1.942935cm)
const CM_PER_CHAR: f64 = 1.942935 / 9.003_906_25;

/// ZIP 바이트가 ODF 스프레드시트인가 (`content.xml` 이 있고 OPC `[Content_Types].xml` 이 없다)
pub fn is_ods(bytes: &[u8]) -> bool {
    let Ok(mut z) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return false;
    };
    z.by_name("content.xml").is_ok() && z.by_name("[Content_Types].xml").is_err()
}

fn entry(bytes: &[u8], name: &str) -> Result<String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::InvalidValue {
        context: "ods".into(),
        value: e.to_string(),
    })?;
    let mut f = z
        .by_name(name)
        .map_err(|_| Error::MissingPart(name.into()))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// 길이 (`1.94cm`, `0.45in`, `12pt`, `5mm`) → pt
fn length_pt(v: &str) -> Option<f64> {
    let v = v.trim();
    let (n, unit) = v.split_at(v.find(|c: char| c.is_ascii_alphabetic())?);
    let n: f64 = n.parse().ok()?;
    Some(match unit {
        "cm" => n / 2.54 * 72.0,
        "mm" => n / 25.4 * 72.0,
        "in" => n * 72.0,
        "pt" => n,
        "pc" => n * 12.0,
        "px" => n * 0.75,
        _ => return None,
    })
}

/// ODF 수식 (`of:=SUM([.A1:.A3];1)` 또는 한셀 꼴 `of:=SUM(A1:A3;1)`) → 셀 수식 글 (`=` 없이)
pub fn formula_from_odf(f: &str) -> String {
    let f = f
        .strip_prefix("of:")
        .or_else(|| f.strip_prefix("oooc:"))
        .unwrap_or(f);
    let f = f.strip_prefix('=').unwrap_or(f);
    let mut out = String::with_capacity(f.len());
    let mut chars = f.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == '"' {
                        break;
                    }
                }
            }
            ';' => out.push(','),
            '[' => {
                // [시트.A1] / [.A1:.B2] / [$'시트 1'.$A$1]
                let mut r = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    r.push(d);
                }
                let parts: Vec<String> = r
                    .split(':')
                    .map(|p| {
                        let p = p.trim_start_matches('$');
                        match p.rfind('.') {
                            Some(i) if i > 0 => format!("{}!{}", &p[..i], &p[i + 1..]),
                            Some(_) => p[1..].to_string(),
                            None => p.to_string(),
                        }
                    })
                    .collect();
                out.push_str(&parts.join(":"));
            }
            // 한셀 꼴의 다른 시트 참조 `시트.A1` (괄호 없음): 이름(숫자로 시작하지 않는 낱말이나 따옴표 이름) 뒤의
            // `.` 이 칸 참조 앞이면 `!`. 한셀이 잃은 참조 `#REF.` 는 `#REF!`
            '.' if out.ends_with("#REF") => out.push('!'),
            '.' if cell_ref_ahead(chars.clone()) && sheet_name_before(&out) => out.push('!'),
            _ => out.push(c),
        }
    }
    out
}

/// 다음 글이 칸 참조 (`$A$1`, `B12`) 로 시작하는가 (`STDEV.S(` 같은 함수 이름은 아니다)
fn cell_ref_ahead(mut it: impl Iterator<Item = char>) -> bool {
    let mut c = it.next();
    if c == Some('$') {
        c = it.next();
    }
    let mut letters = 0;
    while c.is_some_and(|x| x.is_ascii_alphabetic()) {
        letters += 1;
        c = it.next();
    }
    if c == Some('$') {
        c = it.next();
    }
    let mut digits = 0;
    while c.is_some_and(|x| x.is_ascii_digit()) {
        digits += 1;
        c = it.next();
    }
    (1..=3).contains(&letters) && digits > 0 && !c.is_some_and(|x| x.is_alphanumeric() || x == '(')
}

/// 글 끝이 시트 이름인가: 따옴표로 닫혔거나, 끝의 낱말(글자·숫자·`_`)이 숫자로 시작하지 않는다
fn sheet_name_before(out: &str) -> bool {
    if out.ends_with('\'') {
        return true;
    }
    let word: String = out
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    word.chars()
        .last()
        .is_some_and(|first| !first.is_ascii_digit())
}

/// 이름 정의의 범위 주소 (`$Variables.$B$1`, `$시트.$A$1:.$B$2`) → `Variables!$B$1`
fn range_address(a: &str) -> String {
    let mut sheet = String::new();
    let parts: Vec<String> = a
        .split(':')
        .map(|p| match p.rfind('.') {
            Some(i) => {
                if i > 0 {
                    sheet = p[..i].trim_start_matches('$').to_string();
                }
                p[i + 1..].to_string()
            }
            None => p.to_string(),
        })
        .collect();
    let sheet = if sheet.is_empty() {
        String::new()
    } else {
        format!(
            "{}!",
            crate::legacy::formula::quote_sheet(sheet.trim_matches('\''))
        )
    };
    format!("{sheet}{}", parts.join(":"))
}

/// 셀 수식 글 → ODF (한셀 꼴: `of:=` + A1 참조 그대로, 인수 구분 `;`)
pub fn formula_to_odf(f: &str) -> String {
    let mut out = String::from("of:=");
    let mut in_str = false;
    for c in f.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                out.push(c);
            }
            ',' if !in_str => out.push(';'),
            _ => out.push(c),
        }
    }
    out
}

/// `PT12H30M15S` → 하루의 비율
fn duration_days(v: &str) -> Option<f64> {
    let v = v.strip_prefix("PT").or_else(|| v.strip_prefix("-PT"))?;
    let (mut h, mut m, mut s) = (0.0, 0.0, 0.0);
    let mut num = String::new();
    for c in v.chars() {
        match c {
            'H' => h = num.parse().ok()?,
            'M' => m = num.parse().ok()?,
            'S' => s = num.parse().ok()?,
            _ => {
                num.push(c);
                continue;
            }
        }
        num.clear();
    }
    Some((h * 3600.0 + m * 60.0 + s) / 86400.0)
}

/// `2024-03-15` (`T12:00:00` 이 붙을 수 있다) → 1900 날짜 체계 일련번호
fn date_serial(v: &str) -> Option<f64> {
    let (d, t) = v.split_once('T').unwrap_or((v, ""));
    let mut it = d.split('-');
    let (y, m, day): (i64, i64, i64) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    // 1899-12-30 기준 일 수 (1900-03-01 이후는 1900 체계 일련번호와 같다)
    let days_from_civil = |y: i64, m: i64, d: i64| -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    };
    let serial = (days_from_civil(y, m, day) - days_from_civil(1899, 12, 30)) as f64;
    let frac = if t.is_empty() {
        0.0
    } else {
        let mut p = t.split(':');
        let h: f64 = p.next()?.parse().ok()?;
        let mi: f64 = p.next().unwrap_or("0").parse().ok()?;
        let s: f64 = p.next().unwrap_or("0").parse().ok()?;
        (h * 3600.0 + mi * 60.0 + s) / 86400.0
    };
    Some(serial + frac)
}

fn attr<'a>(e: &'a Element, ns: &str, name: &str) -> Option<&'a str> {
    e.attr_ns(ns, name)
}

fn paragraphs(cell: &Element) -> String {
    let mut lines = Vec::new();
    for p in cell.elements().filter(|e| e.is(NS_TEXT, "p")) {
        lines.push(p.text());
    }
    lines.join("\n")
}

/// `.ods` 바이트 → 통합 문서
pub fn read(bytes: &[u8]) -> Result<Workbook> {
    let content = entry(bytes, "content.xml")?;
    let root = crate::xml::parse_raw(&content, "content.xml")?;
    // 자동 스타일: 열 너비, 행 높이
    let mut col_width: std::collections::HashMap<String, f64> = Default::default();
    let mut row_height: std::collections::HashMap<String, (f64, bool)> = Default::default();
    if let Some(auto) = root.child_ns(NS_OFFICE, "automatic-styles") {
        for st in auto.elements().filter(|e| e.is(NS_STYLE, "style")) {
            let Some(name) = attr(st, NS_STYLE, "name") else {
                continue;
            };
            if let Some(p) = st.child_ns(NS_STYLE, "table-column-properties")
                && let Some(w) = attr(p, NS_STYLE, "column-width").and_then(length_pt)
            {
                col_width.insert(name.to_string(), w / 72.0 * 2.54 / CM_PER_CHAR);
            }
            if let Some(p) = st.child_ns(NS_STYLE, "table-row-properties")
                && let Some(h) = attr(p, NS_STYLE, "row-height").and_then(length_pt)
            {
                let optimal = attr(p, NS_STYLE, "use-optimal-row-height") == Some("true");
                row_height.insert(name.to_string(), (h, !optimal));
            }
        }
    }
    let spreadsheet = root
        .child_ns(NS_OFFICE, "body")
        .and_then(|b| b.child_ns(NS_OFFICE, "spreadsheet"))
        .ok_or_else(|| Error::MissingPart("office:spreadsheet".into()))?;
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("ODF".into());
    let template = wb.sheets[0].clone();
    wb.sheets.clear();
    for (ti, table) in spreadsheet
        .elements()
        .filter(|e| e.is(NS_TABLE, "table"))
        .enumerate()
    {
        let mut sheet = template.clone();
        sheet.name = attr(table, NS_TABLE, "name")
            .map(str::to_string)
            .unwrap_or_else(|| format!("Sheet{}", ti + 1));
        sheet.sheet_id = ti as u32 + 1;
        sheet.view.tab_selected = ti == 0;
        let mut col = 0u32;
        let mut row = 0u32;
        let mut rows_parent: Vec<&Element> = Vec::new();
        for e in table.elements() {
            if e.is(NS_TABLE, "table-column")
                || e.is(NS_TABLE, "table-columns")
                || e.is(NS_TABLE, "table-column-group")
            {
                let cols: Vec<&Element> = if e.is(NS_TABLE, "table-column") {
                    vec![e]
                } else {
                    e.elements()
                        .filter(|c| c.is(NS_TABLE, "table-column"))
                        .collect()
                };
                for c in cols {
                    let n: u32 = attr(c, NS_TABLE, "number-columns-repeated")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    let width = attr(c, NS_TABLE, "style-name")
                        .and_then(|s| col_width.get(s))
                        .copied();
                    let hidden = attr(c, NS_TABLE, "visibility") == Some("collapse");
                    let default = width.is_none_or(|w| (w - 9.003_906_25).abs() < 0.01);
                    if (hidden || !default) && col < 16384 {
                        let max = (col + n).min(16384);
                        sheet.cols.push(ColInfo {
                            min: col + 1,
                            max,
                            width: width.map(|w| (w * 256.0).round() / 256.0),
                            hidden,
                            custom_width: !default,
                            ..Default::default()
                        });
                    }
                    col = col.saturating_add(n);
                }
            } else if e.is(NS_TABLE, "table-row") {
                rows_parent.push(e);
            } else if e.is(NS_TABLE, "table-rows")
                || e.is(NS_TABLE, "table-header-rows")
                || e.is(NS_TABLE, "table-row-group")
            {
                rows_parent.extend(e.elements().filter(|r| r.is(NS_TABLE, "table-row")));
            }
        }
        for r in rows_parent {
            let n: u32 = attr(r, NS_TABLE, "number-rows-repeated")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let height = attr(r, NS_TABLE, "style-name")
                .and_then(|s| row_height.get(s))
                .copied();
            let hidden = attr(r, NS_TABLE, "visibility") == Some("collapse");
            let has_cells = r.elements().any(|c| {
                c.is(NS_TABLE, "table-cell")
                    && (attr(c, NS_OFFICE, "value-type").is_some()
                        || attr(c, NS_TABLE, "formula").is_some())
            });
            // 끝의 빈 행 반복 (1048576 행까지 채우는 것)은 건너뛴다
            if !has_cells && n > 1000 {
                row = row.saturating_add(n);
                continue;
            }
            for k in 0..n {
                let ri = row + k;
                if ri >= 1_048_576 {
                    break;
                }
                if let Some((h, custom)) = height.filter(|h| h.1) {
                    sheet.rows.push(RowInfo {
                        index: ri + 1,
                        height: Some((h * 4.0).round() / 4.0),
                        custom_height: custom,
                        hidden,
                        ..Default::default()
                    });
                } else if hidden {
                    sheet.rows.push(RowInfo {
                        index: ri + 1,
                        hidden,
                        ..Default::default()
                    });
                }
                let mut ci = 0u32;
                for c in r.elements() {
                    let covered = c.is(NS_TABLE, "covered-table-cell");
                    if !(c.is(NS_TABLE, "table-cell") || covered) {
                        continue;
                    }
                    let rep: u32 = attr(c, NS_TABLE, "number-columns-repeated")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1);
                    let vt = attr(c, NS_OFFICE, "value-type");
                    let formula = attr(c, NS_TABLE, "formula").map(formula_from_odf);
                    if !covered && (vt.is_some() || formula.is_some()) && rep < 1000 {
                        let num =
                            |a: &str| attr(c, NS_OFFICE, a).and_then(|v| v.parse::<f64>().ok());
                        let value = match vt {
                            Some("float" | "percentage" | "currency") => {
                                num("value").map_or(CellValue::Empty, CellValue::Number)
                            }
                            Some("date") => attr(c, NS_OFFICE, "date-value")
                                .and_then(date_serial)
                                .map_or(CellValue::Empty, CellValue::Number),
                            Some("time") => attr(c, NS_OFFICE, "time-value")
                                .and_then(duration_days)
                                .map_or(CellValue::Empty, CellValue::Number),
                            Some("boolean") => {
                                CellValue::Bool(attr(c, NS_OFFICE, "boolean-value") == Some("true"))
                            }
                            Some("string") => CellValue::Text(
                                attr(c, NS_OFFICE, "string-value")
                                    .map(str::to_string)
                                    .unwrap_or_else(|| paragraphs(c)),
                            ),
                            _ => CellValue::Empty,
                        };
                        for k in 0..rep {
                            let cc = ci + k;
                            if cc >= 16384 {
                                break;
                            }
                            sheet.cells.push(Cell {
                                r: CellRef { row: ri, col: cc },
                                style: 0,
                                value: value.clone(),
                                formula: formula.clone().map(|text| Formula {
                                    text,
                                    ..Default::default()
                                }),
                            });
                        }
                    }
                    if !covered {
                        let cs: u32 = attr(c, NS_TABLE, "number-columns-spanned")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        let rs: u32 = attr(c, NS_TABLE, "number-rows-spanned")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        if (cs > 1 || rs > 1) && k == 0 {
                            sheet.merges.push(CellRange {
                                start: CellRef { row: ri, col: ci },
                                end: CellRef {
                                    row: ri + rs - 1,
                                    col: ci + cs - 1,
                                },
                            });
                        }
                    }
                    ci = ci.saturating_add(rep);
                }
            }
            row = row.saturating_add(n);
        }
        sheet.cells.sort_by_key(|c| (c.r.row, c.r.col));
        crate::edit::structure::update_dimension(&mut sheet);
        wb.sheets.push(sheet);
    }
    // 이름 정의: 범위 (`table:named-range`) 와 식 (`table:named-expression`)
    if let Some(names) = spreadsheet.child_ns(NS_TABLE, "named-expressions") {
        for n in names.elements() {
            let Some(name) = attr(n, NS_TABLE, "name") else {
                continue;
            };
            let formula = if n.is(NS_TABLE, "named-range") {
                attr(n, NS_TABLE, "cell-range-address").map(range_address)
            } else if n.is(NS_TABLE, "named-expression") {
                attr(n, NS_TABLE, "expression").map(formula_from_odf)
            } else {
                None
            };
            if let Some(formula) = formula {
                wb.defined_names.push(crate::model::DefinedName {
                    name: name.to_string(),
                    formula,
                    ..Default::default()
                });
            }
        }
    }
    if wb.sheets.is_empty() {
        wb.sheets.push(template);
    }
    if wb
        .sheets
        .iter()
        .any(|s| s.cells.iter().any(|c| c.formula.is_some()))
    {
        crate::calc::recalculate(&mut wb);
    }
    Ok(wb)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 통합 문서 → `.ods` 바이트 (한셀처럼 `mimetype` 없이 내용·스타일·메타·설정·매니페스트)
pub fn write(wb: &Workbook) -> Result<Vec<u8>> {
    // 값이 없는 수식이 있으면 다시 계산한 사본으로 쓴다 (ODS 는 수식의 값을 담는다 — 한셀은 열 때 다시 계산하지 않는다)
    let computed;
    let wb = if wb.sheets.iter().any(|s| {
        s.cells
            .iter()
            .any(|c| c.formula.is_some() && c.value.is_empty())
    }) {
        let mut w = wb.clone();
        crate::calc::recalculate(&mut w);
        computed = w;
        &computed
    } else {
        wb
    };
    const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/""#;
    let mut auto = String::new();
    let mut body = String::new();
    let mut col_styles: Vec<String> = Vec::new();
    let mut row_styles: Vec<String> = Vec::new();
    let style_of = |list: &mut Vec<String>,
                    key: String,
                    prefix: &str,
                    auto: &mut String,
                    make: &dyn Fn(&str) -> String|
     -> String {
        let k = match list.iter().position(|x| *x == key) {
            Some(k) => k,
            None => {
                list.push(key.clone());
                let name = format!("{prefix}{}", list.len());
                auto.push_str(&make(&name));
                list.len() - 1
            }
        };
        format!("{prefix}{}", k + 1)
    };
    for sheet in &wb.sheets {
        body.push_str(&format!(
            r#"<table:table table:name="{}">"#,
            esc(&sheet.name)
        ));
        let last_col = sheet.cells.iter().map(|c| c.r.col).max().unwrap_or(0).max(
            sheet
                .cols
                .iter()
                .map(|c| c.max)
                .max()
                .unwrap_or(1)
                .saturating_sub(1),
        );
        let mut c = 0u32;
        while c <= last_col {
            let info = sheet.cols.iter().find(|x| x.min <= c + 1 && c < x.max);
            let width = info
                .and_then(|i| i.width)
                .unwrap_or(sheet.default_col_width.unwrap_or(9.003_906_25));
            let w = format!("{:.6}cm", width * CM_PER_CHAR);
            let name = style_of(&mut col_styles, w.clone(), "co", &mut auto, &|n| {
                format!(
                    r#"<style:style style:name="{n}" style:family="table-column"><style:table-column-properties style:column-width="{w}"/></style:style>"#
                )
            });
            let hidden = if info.is_some_and(|i| i.hidden) {
                r#" table:visibility="collapse""#
            } else {
                ""
            };
            body.push_str(&format!(
                r#"<table:table-column table:style-name="{name}"{hidden}/>"#
            ));
            c += 1;
        }
        let last_row = sheet
            .cells
            .iter()
            .map(|c| c.r.row)
            .chain(sheet.merges.iter().map(|m| m.end.row))
            .max()
            .unwrap_or(0);
        let mut i = 0usize;
        for r in 0..=last_row {
            let info = sheet.rows.iter().find(|x| x.index == r + 1);
            let h = info
                .and_then(|i| i.height)
                .unwrap_or(sheet.default_row_height.unwrap_or(16.5));
            let custom = info.is_some_and(|i| i.custom_height);
            let key = format!("{h:.2}pt/{custom}");
            let name = style_of(&mut row_styles, key, "ro", &mut auto, &|n| {
                format!(
                    r#"<style:style style:name="{n}" style:family="table-row"><style:table-row-properties style:row-height="{:.4}cm" style:use-optimal-row-height="{}"/></style:style>"#,
                    h / 72.0 * 2.54,
                    !custom
                )
            });
            let hidden = if info.is_some_and(|i| i.hidden) {
                r#" table:visibility="collapse""#
            } else {
                ""
            };
            body.push_str(&format!(
                r#"<table:table-row table:style-name="{name}"{hidden}>"#
            ));
            // 이 행의 칸: 값·수식이 있는 칸과 병합의 첫 칸
            let mut entries: Vec<(u32, Option<&Cell>)> = Vec::new();
            while i < sheet.cells.len() && sheet.cells[i].r.row == r {
                let cell = &sheet.cells[i];
                i += 1;
                if !cell.value.is_empty() || cell.formula.is_some() {
                    entries.push((cell.r.col, Some(cell)));
                }
            }
            for m in sheet.merges.iter().filter(|m| m.start.row == r) {
                if !entries.iter().any(|e| e.0 == m.start.col) {
                    entries.push((m.start.col, None));
                }
            }
            entries.sort_by_key(|e| e.0);
            let mut col = 0u32;
            for (at, cell) in entries {
                let here = CellRef { row: r, col: at };
                if at > col {
                    body.push_str(&format!(
                        r#"<table:table-cell table:number-columns-repeated="{}"/>"#,
                        at - col
                    ));
                }
                let covered = sheet
                    .merges
                    .iter()
                    .any(|m| m.contains(here) && m.start != here);
                let tag = if covered {
                    "table:covered-table-cell"
                } else {
                    "table:table-cell"
                };
                let mut a = String::new();
                if let Some(m) = sheet.merges.iter().find(|m| m.start == here) {
                    a.push_str(&format!(
                        r#" table:number-columns-spanned="{}" table:number-rows-spanned="{}""#,
                        m.end.col - m.start.col + 1,
                        m.end.row - m.start.row + 1
                    ));
                }
                if let Some(f) = cell.and_then(|c| c.formula.as_ref()) {
                    a.push_str(&format!(
                        r#" table:formula="{}""#,
                        esc(&formula_to_odf(&f.text))
                    ));
                }
                let empty = CellValue::Empty;
                let text = match cell.map_or(&empty, |c| &c.value) {
                    CellValue::Number(x) => {
                        a.push_str(&format!(r#" office:value-type="float" office:value="{x}""#));
                        cell.map(|c| crate::csv::shown(wb, c)).unwrap_or_default()
                    }
                    CellValue::Bool(b) => {
                        a.push_str(&format!(
                            r#" office:value-type="boolean" office:boolean-value="{b}""#
                        ));
                        if *b { "TRUE" } else { "FALSE" }.into()
                    }
                    CellValue::Text(t) => {
                        a.push_str(r#" office:value-type="string""#);
                        t.clone()
                    }
                    CellValue::Error(e) => {
                        a.push_str(r#" office:value-type="string""#);
                        e.clone()
                    }
                    CellValue::Date(d) => {
                        a.push_str(&format!(
                            r#" office:value-type="date" office:date-value="{}""#,
                            esc(d)
                        ));
                        d.clone()
                    }
                    CellValue::Empty => String::new(),
                };
                body.push_str(&format!("<{tag}{a}>"));
                if cell.is_some_and(|c| !c.value.is_empty()) {
                    for line in text.split('\n') {
                        body.push_str(&format!("<text:p>{}</text:p>", esc(line)));
                    }
                }
                body.push_str(&format!("</{tag}>"));
                col = at + 1;
            }
            body.push_str("</table:table-row>");
        }
        body.push_str("</table:table>");
    }
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {NS} office:version="1.2"><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {NS} office:version="1.2"><office:styles><style:default-style style:family="table-cell"><style:text-properties fo:font-size="11pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let meta = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta {NS} office:version="1.2"><office:meta><meta:generator>rcell</meta:generator></office:meta></office:document-meta>"#
    );
    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/></manifest:manifest>"#;
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zerr = |e: zip::result::ZipError| Error::InvalidValue {
        context: "ods".into(),
        value: e.to_string(),
    };
    // ODF 는 첫 항목으로 압축하지 않은 mimetype 을 둔다 (한셀은 쓰지 않지만 다른 프로그램을 위해)
    z.start_file("mimetype", stored).map_err(zerr)?;
    z.write_all(b"application/vnd.oasis.opendocument.spreadsheet")?;
    for (name, data) in [
        ("META-INF/manifest.xml", manifest.to_string()),
        ("content.xml", content),
        ("styles.xml", styles),
        ("meta.xml", meta),
    ] {
        z.start_file(name, deflated).map_err(zerr)?;
        z.write_all(data.as_bytes())?;
    }
    Ok(z.finish().map_err(zerr)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas() {
        assert_eq!(formula_from_odf("of:=SUM([.A1:.A3];1)"), "SUM(A1:A3,1)");
        assert_eq!(
            formula_from_odf("of:=IF(A1>5;\"a;b\";2)"),
            "IF(A1>5,\"a;b\",2)"
        );
        assert_eq!(
            formula_from_odf("of:=[$'Sheet 2'.$A$1]+1"),
            "'Sheet 2'!$A$1+1"
        );
        assert_eq!(
            formula_to_odf("IF(A1>5,\"a,b\",2)"),
            "of:=IF(A1>5;\"a,b\";2)"
        );
        // 한셀 꼴: 괄호 없는 다른 시트 참조, 잃은 참조, 점이 든 함수 이름과 소수
        assert_eq!(formula_from_odf("of:=Sheet2.A1*2+1.5"), "Sheet2!A1*2+1.5");
        assert_eq!(formula_from_odf("of:=STDEV.S(A1:A3)"), "STDEV.S(A1:A3)");
        assert_eq!(
            formula_from_odf("of:=CONCATENATE(#REF.;1)"),
            "CONCATENATE(#REF!,1)"
        );
        assert_eq!(range_address("$Variables.$B$1"), "Variables!$B$1");
    }

    #[test]
    fn dates() {
        assert_eq!(date_serial("2024-03-15"), Some(45366.0));
        assert_eq!(date_serial("1900-03-01"), Some(61.0));
        assert_eq!(duration_days("PT12H00M00S"), Some(0.5));
    }

    #[test]
    fn round_trip() {
        let mut wb = crate::edit::new_workbook();
        for (a1, v) in [
            ("A1", "10"),
            ("A2", "20"),
            ("B1", "글"),
            ("A3", "=SUM(A1:A2)"),
            ("C1", "TRUE"),
        ] {
            crate::edit::set_input(&mut wb, 0, CellRef::parse(a1).unwrap(), v).unwrap();
        }
        wb.sheets[0].merges.push(CellRange::parse("D1:E2").unwrap());
        crate::calc::recalculate(&mut wb);
        let bytes = write(&wb).unwrap();
        assert!(is_ods(&bytes));
        let back = read(&bytes).unwrap();
        let s = &back.sheets[0];
        let at = |a: &str| s.cell(CellRef::parse(a).unwrap()).map(|c| c.value.clone());
        assert_eq!(at("A3"), Some(CellValue::Number(30.0)));
        assert_eq!(at("B1"), Some(CellValue::Text("글".into())));
        assert_eq!(at("C1"), Some(CellValue::Bool(true)));
        assert_eq!(
            s.cell(CellRef::parse("A3").unwrap())
                .and_then(|c| c.formula.as_ref())
                .map(|f| f.text.as_str()),
            Some("SUM(A1:A2)")
        );
        assert_eq!(s.merges, vec![CellRange::parse("D1:E2").unwrap()]);
    }
}
