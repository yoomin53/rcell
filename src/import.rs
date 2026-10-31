// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀이 칸으로 읽는 그 밖의 파일: HTML(`.htm`·`.mht`, 한셀 웹 저장 틀 포함), XML 스프레드시트 2003, dBASE.
//! 한셀이 읽는 만큼만 읽는다 (docs/format/other-formats.md 15절 — 한셀 COM `Open` 으로 대조):
//! HTML 은 표만 (값 해석은 텍스트 가져오기와 같고, `<th>`·`<b>` 굵게, 맞춤, 배경색, `colspan`·`rowspan` 병합,
//! 칸 안 `<br>` 은 행으로 쪼갠다), XML 2003 은 글·수 칸(수는 서식의 표시 형식)과 글꼴·채우기·가로 맞춤·열 너비 (굵게·행 높이·병합·
//! 논리값·오류·날짜·수식은 읽지 않는다), dBASE 는 필드 이름 머리 행 + 글·수 (날짜 필드는 20230315 수, 논리 필드는 비운다).
//! SYLK·DIF 는 한셀도 형식으로 읽지 않고 텍스트로 연다 ([`crate::csv`]).

use std::collections::BTreeMap;
use std::path::Path;

use crate::csv::{StyleCache, field_cell};
use crate::edit::style::FormatPatch;
use crate::model::{Cell, CellRange, CellRef, CellValue, ColInfo, Workbook};

/// 알아낸 형식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Html,
    Mht,
    Xml2003,
    Dbf,
}

/// 내용과 확장자로 형식을 알아낸다 (모르면 None)
pub fn sniff(bytes: &[u8], path: Option<&Path>) -> Option<Kind> {
    let ext = path
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if is_dbf(bytes) && (ext == "dbf" || ext.is_empty()) {
        return Some(Kind::Dbf);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("mime-version")
        || ((ext == "mht" || ext == "mhtml") && head.contains("boundary"))
    {
        return Some(Kind::Mht);
    }
    if head.contains("urn:schemas-microsoft-com:office:spreadsheet") && head.contains("<workbook") {
        return Some(Kind::Xml2003);
    }
    if matches!(ext.as_str(), "htm" | "html") || head.contains("<html") || head.contains("<table") {
        return Some(Kind::Html);
    }
    None
}

/// 파일 이름과 내용으로 연다
pub fn open(bytes: &[u8], kind: Kind, path: &Path) -> crate::Result<Workbook> {
    let stem = crate::csv::sheet_name_for(path);
    match kind {
        Kind::Dbf => dbf(bytes, &stem),
        Kind::Xml2003 => xml_spreadsheet(bytes),
        Kind::Html => html_document(bytes, &stem, path.parent(), None),
        Kind::Mht => {
            let parts = mht_parts(bytes);
            let main = parts
                .iter()
                .find(|p| p.is_html)
                .map(|p| p.body.clone())
                .unwrap_or_default();
            html_document(&main, &stem, None, Some(&parts))
        }
    }
}

// ---------------------------------------------------------------------------
// dBASE
// ---------------------------------------------------------------------------

/// 949 바이트 → 글: 짝이 맞지 않는 두 바이트는 U+FFFD 하나 (한셀 dBASE 읽기 — 텍스트 파일은 ?)
fn ansi(b: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] < 0x80 {
            out.push(b[i] as char);
            i += 1;
        } else if i + 1 < b.len() {
            let t = crate::calc::cp949::decode(&b[i..i + 2]);
            out.push_str(&if t == "?" { "\u{FFFD}".to_string() } else { t });
            i += 2;
        } else {
            out.push('\u{FFFD}');
            i += 1;
        }
    }
    out
}

fn is_dbf(b: &[u8]) -> bool {
    if b.len() < 34 {
        return false;
    }
    let ver = b[0];
    if !matches!(ver, 0x02..=0x05 | 0x30..=0x32 | 0x83 | 0x8B | 0xCB | 0xF5 | 0xFB) {
        return false;
    }
    let hdr = u16::from_le_bytes([b[8], b[9]]) as usize;
    let rec = u16::from_le_bytes([b[10], b[11]]) as usize;
    hdr >= 33 && hdr <= b.len() && (hdr - 33).is_multiple_of(32) && rec > 0 && b[hdr - 1] == 0x0D
}

/// dBASE III/IV → 통합 문서: 첫 행은 필드 이름, 글(C)·수(N·F)·날짜(D, 20230315 수)·논리(L, 빈 칸)·메모(M, 빈 칸).
/// 열 너비는 필드 길이로 (한셀: 글자 수 n → (8.2 n − 5.48) / 7)
pub fn dbf(b: &[u8], sheet_name: &str) -> crate::Result<Workbook> {
    if !is_dbf(b) {
        return Err(crate::Error::InvalidValue {
            context: "dBASE".into(),
            value: "머리가 맞지 않는다".into(),
        });
    }
    let nrec = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
    let hdr = u16::from_le_bytes([b[8], b[9]]) as usize;
    let rlen = u16::from_le_bytes([b[10], b[11]]) as usize;
    // 필드 설명
    let mut fields: Vec<(String, u8, usize)> = Vec::new();
    let mut at = 32;
    while at + 32 < hdr && b[at] != 0x0D {
        let name: Vec<u8> = b[at..at + 11]
            .iter()
            .copied()
            .take_while(|&c| c != 0)
            .collect();
        fields.push((ansi(&name), b[at + 11], b[at + 16] as usize));
        at += 32;
    }
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("rcell".into());
    wb.sheets[0].name = sheet_name.to_string();
    let mut cache = StyleCache::new();
    let mut cells: BTreeMap<(u32, u32), Cell> = BTreeMap::new();
    for (c, (name, _, _)) in fields.iter().enumerate() {
        put(
            &mut wb, &mut cache, &mut cells, 0, c as u32, name, None, true,
        );
    }
    let mut row = 1u32;
    for i in 0..nrec {
        let o = hdr + i * rlen;
        if o + rlen > b.len() {
            break;
        }
        if b[o] == 0x2A {
            continue; // 지운 레코드
        }
        let mut p = o + 1;
        for (c, (_, ty, len)) in fields.iter().enumerate() {
            let raw = &b[p..(p + len).min(b.len())];
            p += len;
            let text = ansi(raw);
            let text = text.trim_end_matches([' ', '\0']);
            match ty {
                b'N' | b'F' | b'D' => {
                    let t = text.trim();
                    if let Ok(v) = t.parse::<f64>() {
                        cells.insert(
                            (row, c as u32),
                            Cell {
                                r: CellRef { row, col: c as u32 },
                                style: 0,
                                value: CellValue::Number(v),
                                formula: None,
                            },
                        );
                    }
                }
                b'L' | b'M' | b'B' | b'G' | b'P' | b'Y' => {}
                _ => {
                    if !text.is_empty() {
                        cells.insert(
                            (row, c as u32),
                            Cell {
                                r: CellRef { row, col: c as u32 },
                                style: 0,
                                value: CellValue::Text(text.to_string()),
                                formula: None,
                            },
                        );
                    }
                }
            }
        }
        row += 1;
    }
    for (c, (_, _, len)) in fields.iter().enumerate() {
        let w = ((8.2 * *len as f64 - 5.48) / 7.0).max(0.66);
        wb.sheets[0].cols.push(ColInfo {
            min: c as u32 + 1,
            max: c as u32 + 1,
            width: Some((w * 100.0).round() / 100.0),
            style: None,
            hidden: false,
            custom_width: true,
            outline_level: 0,
            collapsed: false,
        });
    }
    finish(&mut wb, 0, cells, Vec::new());
    Ok(wb)
}

/// 칸 하나를 (값 해석 없이 글이면 `as_text`) 넣는다
#[allow(clippy::too_many_arguments)]
fn put(
    wb: &mut Workbook,
    cache: &mut StyleCache,
    cells: &mut BTreeMap<(u32, u32), Cell>,
    row: u32,
    col: u32,
    field: &str,
    patch: Option<&FormatPatch>,
    as_text: bool,
) {
    let r = CellRef { row, col };
    let cell = if as_text {
        if field.is_empty() {
            return;
        }
        Cell {
            r,
            style: 0,
            value: CellValue::Text(field.to_string()),
            formula: None,
        }
    } else {
        match field_cell(wb, cache, r, field, patch, false) {
            Some(c) => c,
            None => return,
        }
    };
    cells.insert((row, col), cell);
}

/// 모은 칸·병합을 시트에 넣고 사용 범위를 갱신한다
fn finish(wb: &mut Workbook, si: usize, cells: BTreeMap<(u32, u32), Cell>, merges: Vec<CellRange>) {
    let sheet = &mut wb.sheets[si];
    sheet.cells = cells.into_values().collect();
    sheet.merges = merges;
    crate::edit::structure::update_dimension(sheet);
    if sheet.cells.iter().any(|c| c.formula.is_some()) {
        crate::calc::recalculate(wb);
    }
}

// ---------------------------------------------------------------------------
// XML 스프레드시트 2003
// ---------------------------------------------------------------------------

const SS: &str = "urn:schemas-microsoft-com:office:spreadsheet";

#[derive(Default, Clone)]
struct XStyle {
    font_name: Option<String>,
    font_size: Option<f64>,
    font_color: Option<String>,
    fill: Option<String>,
    h_align: Option<String>,
    v_align: Option<String>,
    num_fmt: Option<String>,
}

/// XML 스프레드시트 2003 → 통합 문서 (한셀이 읽는 만큼)
pub fn xml_spreadsheet(b: &[u8]) -> crate::Result<Workbook> {
    let text = crate::csv::decode(b);
    // 한셀은 글 칸의 문자 참조(`&#10;`)를 풀지 않는다
    let text = text.replace("&#", "&amp;#");
    let root = crate::xml::parse_raw(&text, "xml2003")?;
    let ss = |e: &crate::xml::Element, n: &str| e.attr_ns(SS, n).map(str::to_string);
    // 스타일
    let mut styles: std::collections::HashMap<String, XStyle> = Default::default();
    root.walk(&mut |e| {
        if e.is(SS, "Style")
            && let Some(id) = ss(e, "ID")
        {
            let mut s = XStyle::default();
            for c in e.elements() {
                if c.is(SS, "Font") {
                    s.font_name = ss(c, "FontName");
                    s.font_size = ss(c, "Size").and_then(|v| v.parse().ok());
                    s.font_color = ss(c, "Color").map(|v| v.trim_start_matches('#').to_string());
                } else if c.is(SS, "Interior") && ss(c, "Pattern").as_deref() == Some("Solid") {
                    s.fill = ss(c, "Color").map(|v| v.trim_start_matches('#').to_string());
                } else if c.is(SS, "Alignment") {
                    s.h_align = ss(c, "Horizontal").map(|v| v.to_ascii_lowercase());
                    s.v_align = ss(c, "Vertical").map(|v| match v.as_str() {
                        "Center" => "center".to_string(),
                        "Top" => "top".to_string(),
                        _ => "bottom".to_string(),
                    });
                } else if c.is(SS, "NumberFormat") {
                    s.num_fmt = ss(c, "Format");
                }
            }
            styles.insert(id, s);
        }
    });
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("rcell".into());
    let sheets: Vec<&crate::xml::Element> =
        root.elements().filter(|e| e.is(SS, "Worksheet")).collect();
    let mut cache = StyleCache::new();
    for (si, ws) in sheets.iter().enumerate() {
        let name = ss(ws, "Name").unwrap_or_else(|| format!("Sheet{}", si + 1));
        if si == 0 {
            wb.sheets[0].name = name;
        } else {
            let at = wb.sheets.len();
            crate::edit::add_sheet(&mut wb, Some(&name), at)?;
        }
        let Some(table) = ws.elements().find(|e| e.is(SS, "Table")) else {
            continue;
        };
        let mut cells: BTreeMap<(u32, u32), Cell> = BTreeMap::new();
        let mut cols: Vec<ColInfo> = Vec::new();
        let mut ci = 0u32;
        for c in table.elements().filter(|e| e.is(SS, "Column")) {
            if let Some(i) = ss(c, "Index").and_then(|v| v.parse::<u32>().ok()) {
                ci = i - 1;
            }
            let span = ss(c, "Span")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0);
            if let Some(w) = ss(c, "Width").and_then(|v| v.parse::<f64>().ok()) {
                let chars = ((w * 4.0 / 3.0 - 5.48) / 7.0).max(0.0);
                cols.push(ColInfo {
                    min: ci + 1,
                    max: ci + 1 + span,
                    width: Some((chars * 100.0).round() / 100.0),
                    style: None,
                    hidden: false,
                    custom_width: true,
                    outline_level: 0,
                    collapsed: false,
                });
            }
            ci += 1 + span;
        }
        let mut ri = 0u32;
        for row in table.elements().filter(|e| e.is(SS, "Row")) {
            if let Some(i) = ss(row, "Index").and_then(|v| v.parse::<u32>().ok()) {
                ri = i - 1;
            }
            let mut cj = 0u32;
            for cell in row.elements().filter(|e| e.is(SS, "Cell")) {
                if let Some(i) = ss(cell, "Index").and_then(|v| v.parse::<u32>().ok()) {
                    cj = i - 1;
                }
                let st = ss(cell, "StyleID").and_then(|id| styles.get(&id));
                let data = cell.elements().find(|e| e.is(SS, "Data"));
                let ty = data.and_then(|d| ss(d, "Type")).unwrap_or_default();
                let txt = data.map(|d| d.text()).unwrap_or_default();
                let patch = st.map(|s| FormatPatch {
                    font_name: s.font_name.clone(),
                    font_size: s.font_size,
                    font_color: s.font_color.clone(),
                    fill: s.fill.clone(),
                    h_align: s.h_align.clone().filter(|h| h != "automatic"),
                    v_align: s.v_align.clone(),
                    num_fmt: None,
                    ..Default::default()
                });
                let r = CellRef { row: ri, col: cj };
                let make = |wb: &mut Workbook,
                            cache: &mut StyleCache,
                            value: CellValue,
                            fmt: Option<&str>| {
                    let mut p = patch.clone().unwrap_or_default();
                    p.num_fmt = fmt.map(str::to_string);
                    let style = crate::csv::styled(wb, cache, &p);
                    Cell {
                        r,
                        style,
                        value,
                        formula: None,
                    }
                };
                // 글(String)·수(Number) 칸만 값을 읽는다 (논리값·오류·날짜 형식 칸은 빈 칸)
                let c = match ty.as_str() {
                    "String" | "DateTime" => {
                        Some(make(&mut wb, &mut cache, CellValue::Text(txt), None))
                    }
                    "Number" => txt.trim().parse::<f64>().ok().map(|v| {
                        make(
                            &mut wb,
                            &mut cache,
                            CellValue::Number(v),
                            st.and_then(|s| s.num_fmt.as_deref()),
                        )
                    }),
                    _ => None,
                };
                if let Some(c) = c {
                    cells.insert((ri, cj), c);
                }
                cj += 1;
            }
            ri += 1;
        }
        wb.sheets[si].cols = cols;
        finish(&mut wb, si, cells, Vec::new());
    }
    Ok(wb)
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

/// MIME 부분: 헤더 `Content-Location`, 본문(풀린 글)
struct Part {
    location: String,
    is_html: bool,
    body: Vec<u8>,
}

/// `.mht` → 부분들 (quoted-printable·base64 풀기)
fn mht_parts(b: &[u8]) -> Vec<Part> {
    let text = String::from_utf8_lossy(b).into_owned();
    let boundary = text
        .lines()
        .find_map(|l| {
            let i = l.to_ascii_lowercase().find("boundary=")?;
            Some(
                l[i + 9..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches(';')
                    .trim_matches('"')
                    .to_string(),
            )
        })
        .unwrap_or_default();
    if boundary.is_empty() {
        return Vec::new();
    }
    let marker = format!("--{boundary}");
    let mut out = Vec::new();
    for seg in text.split(&marker).skip(1) {
        let seg = seg.trim_start_matches(['\r', '\n']);
        if seg.starts_with("--") {
            break;
        }
        let Some(cut) = seg
            .find("\r\n\r\n")
            .map(|i| (i, 4))
            .or_else(|| seg.find("\n\n").map(|i| (i, 2)))
        else {
            continue;
        };
        let (head, body) = (&seg[..cut.0], &seg[cut.0 + cut.1..]);
        let hdr = |name: &str| {
            head.lines()
                .find(|l| l.to_ascii_lowercase().starts_with(&format!("{name}:")))
                .map(|l| l[name.len() + 1..].trim().to_string())
        };
        let enc = hdr("content-transfer-encoding")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let ctype = hdr("content-type").unwrap_or_default().to_ascii_lowercase();
        let raw = match enc.as_str() {
            "quoted-printable" => unqp(body),
            "base64" => Vec::new(),
            _ => body.as_bytes().to_vec(),
        };
        out.push(Part {
            location: hdr("content-location").unwrap_or_default(),
            is_html: ctype.starts_with("text/html"),
            body: raw,
        });
    }
    out
}

fn unqp(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if i + 1 < b.len() && (b[i + 1] == b'\r' || b[i + 1] == b'\n') {
                i += 2;
                if i < b.len() && b[i - 1] == b'\r' && b[i] == b'\n' {
                    i += 1;
                }
                continue;
            }
            if i + 2 < b.len()
                && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// HTML 한 문서 (또는 한셀 웹 저장의 틀 문서 — 시트 쪽들을 따라가 시트로)
fn html_document(
    bytes: &[u8],
    stem: &str,
    dir: Option<&Path>,
    mht: Option<&[Part]>,
) -> crate::Result<Workbook> {
    let text = crate::csv::decode(bytes);
    // 틀 문서: x:ExcelWorksheet 마다 이름과 시트 쪽
    let mut sheets: Vec<(String, Vec<u8>)> = Vec::new();
    if text.contains("x:WorksheetSource") {
        let mut rest = text.as_str();
        while let Some(i) = rest.find("<x:ExcelWorksheet>") {
            let seg_end = rest[i..]
                .find("</x:ExcelWorksheet>")
                .map_or(rest.len(), |e| i + e);
            let seg = &rest[i..seg_end];
            let name = between(seg, "<x:Name>", "</x:Name>").unwrap_or_default();
            let href = between(seg, "HRef=\"", "\"")
                .unwrap_or_default()
                .replace('\\', "/");
            let file = href.rsplit('/').next().unwrap_or("").to_string();
            let data = if let Some(parts) = mht {
                parts
                    .iter()
                    .find(|p| {
                        p.location.replace('\\', "/").rsplit('/').next() == Some(file.as_str())
                    })
                    .map(|p| p.body.clone())
            } else {
                dir.and_then(|d| std::fs::read(d.join(&href)).ok())
            };
            if let Some(d) = data {
                sheets.push((unescape(&name), d));
            }
            rest = &rest[seg_end..];
        }
    }
    if sheets.is_empty() {
        sheets.push((stem.to_string(), bytes.to_vec()));
    }
    let mut wb = crate::edit::new_workbook();
    wb.application = Some("rcell".into());
    let mut cache = StyleCache::new();
    for (si, (name, data)) in sheets.iter().enumerate() {
        let name = if name.is_empty() {
            format!("Sheet{}", si + 1)
        } else {
            name.clone()
        };
        if si == 0 {
            wb.sheets[0].name = name;
        } else {
            let at = wb.sheets.len();
            crate::edit::add_sheet(&mut wb, Some(&name), at)?;
        }
        let tables = parse_tables(&crate::csv::decode(data));
        let (cells, merges) = layout(&mut wb, &mut cache, &tables);
        finish(&mut wb, si, cells, merges);
    }
    Ok(wb)
}

fn between(s: &str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(s[i..j].to_string())
}

fn unescape(s: &str) -> String {
    decode_entities(s)
}

/// HTML 문자 참조 풀기 (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;` `&nbsp;` `&#N;` `&#xH;`)
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(end) = tail.find(';').filter(|&e| e <= 10) {
            let name = &tail[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16)
                    .ok()
                    .and_then(char::from_u32),
                n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = ch {
                out.push(c);
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

#[derive(Default, Clone)]
struct HCell {
    lines: Vec<String>,
    rowspan: u32,
    colspan: u32,
    th: bool,
    bold: bool,
    align: Option<String>,
    fill: Option<String>,
}

type HRow = Vec<HCell>;

/// 표들 → 행·칸 (바깥 표만; 안쪽 표는 칸 글로 합친다)
fn parse_tables(s: &str) -> Vec<Vec<HRow>> {
    let mut tables: Vec<Vec<HRow>> = Vec::new();
    let mut depth = 0u32;
    let mut cell: Option<HCell> = None;
    let mut text = String::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    let flush_text = |cell: &mut Option<HCell>, text: &mut String| {
        if let Some(c) = cell.as_mut() {
            let t = decode_entities(text);
            // 공백 접기: 줄 안의 연속 공백은 하나 (줄 나눔은 `<br>` 이 한다)
            let mut folded = String::new();
            let mut last_space = true;
            for ch in t.chars() {
                if ch.is_whitespace() {
                    if !last_space {
                        folded.push(' ');
                    }
                    last_space = true;
                } else {
                    folded.push(ch);
                    last_space = false;
                }
            }
            if let Some(l) = c.lines.last_mut() {
                l.push_str(&folded);
            }
        }
        text.clear();
    };
    while i < b.len() {
        if b[i] != '<' {
            text.push(b[i]);
            i += 1;
            continue;
        }
        // 주석·조건부 주석·선언
        if b[i..].starts_with(&['<', '!', '-', '-']) {
            let end = find(&b, i + 4, "-->").map_or(b.len(), |e| e + 3);
            i = end;
            continue;
        }
        if b.get(i + 1) == Some(&'!') || b.get(i + 1) == Some(&'?') {
            i = find(&b, i, ">").map_or(b.len(), |e| e + 1);
            continue;
        }
        let Some(end) = find(&b, i, ">") else { break };
        let tag: String = b[i + 1..end].iter().collect();
        i = end + 1;
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim_end_matches('/');
        let name: String = body
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let attrs = &body[name.len().min(body.len())..];
        // 스크립트·스타일 본문은 건너뛴다
        if !closing && (name == "script" || name == "style") {
            let close = format!("</{name}");
            let low: String = b[i..].iter().collect::<String>().to_ascii_lowercase();
            i = low
                .find(&close)
                .map_or(b.len(), |p| i + low[..p].chars().count());
            continue;
        }
        match (name.as_str(), closing) {
            ("table", false) => {
                depth += 1;
                if depth == 1 {
                    tables.push(Vec::new());
                } else {
                    text.push(' ');
                }
            }
            ("table", true) => {
                flush_text(&mut cell, &mut text);
                if let (1, Some(c)) = (depth, cell.take())
                    && let Some(row) = tables.last_mut().and_then(|t| t.last_mut())
                {
                    row.push(c);
                }
                depth = depth.saturating_sub(1);
            }
            ("tr", false) if depth == 1 => {
                flush_text(&mut cell, &mut text);
                if let Some(c) = cell.take()
                    && let Some(row) = tables.last_mut().and_then(|t| t.last_mut())
                {
                    row.push(c);
                }
                if let Some(t) = tables.last_mut() {
                    t.push(Vec::new());
                }
            }
            ("td" | "th", false) if depth == 1 => {
                flush_text(&mut cell, &mut text);
                if let Some(c) = cell.take()
                    && let Some(row) = tables.last_mut().and_then(|t| t.last_mut())
                {
                    row.push(c);
                }
                if tables.last().is_some_and(|t| t.is_empty()) {
                    tables.last_mut().unwrap().push(Vec::new());
                }
                let style = attr(attrs, "style")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let css = |k: &str| {
                    style.split(';').find_map(|d| {
                        let (n, v) = d.split_once(':')?;
                        (n.trim() == k).then(|| v.trim().to_string())
                    })
                };
                let fill = css("background-color")
                    .or_else(|| css("background"))
                    .or_else(|| attr(attrs, "bgcolor"))
                    .and_then(|v| color(&v));
                let align = css("text-align")
                    .or_else(|| attr(attrs, "align"))
                    .map(|a| a.to_ascii_lowercase())
                    .filter(|a| matches!(a.as_str(), "left" | "center" | "right" | "justify"));
                cell = Some(HCell {
                    lines: vec![String::new()],
                    rowspan: attr(attrs, "rowspan")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1)
                        .max(1),
                    colspan: attr(attrs, "colspan")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1)
                        .max(1),
                    th: name == "th",
                    bold: false,
                    align,
                    fill,
                });
            }
            ("td" | "th", true) if depth == 1 => {
                flush_text(&mut cell, &mut text);
                if let Some(c) = cell.take()
                    && let Some(row) = tables.last_mut().and_then(|t| t.last_mut())
                {
                    row.push(c);
                }
            }
            ("br", _) if depth >= 1 => {
                flush_text(&mut cell, &mut text);
                if let Some(c) = cell.as_mut() {
                    c.lines.push(String::new());
                }
            }
            ("b" | "strong", false) if depth >= 1 => {
                flush_text(&mut cell, &mut text);
                if let Some(c) = cell.as_mut() {
                    c.bold = true;
                }
            }
            _ => {}
        }
    }
    flush_text(&mut cell, &mut text);
    if let Some(c) = cell.take()
        && let Some(row) = tables.last_mut().and_then(|t| t.last_mut())
    {
        row.push(c);
    }
    tables
}

fn find(b: &[char], from: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    (from..b.len().saturating_sub(p.len() - 1)).find(|&i| b[i..i + p.len()] == p[..])
}

/// 태그 속성 값 (`name="v"`·`name='v'`·`name=v`)
fn attr(attrs: &str, name: &str) -> Option<String> {
    let low = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find(name) {
        let i = from + i;
        let before_ok = i == 0 || !low.as_bytes()[i - 1].is_ascii_alphanumeric();
        let rest = attrs[i + name.len()..].trim_start();
        if before_ok && let Some(v) = rest.strip_prefix('=') {
            let v = v.trim_start();
            let val = if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
                v[1..].split(q).next().unwrap_or("").to_string()
            } else {
                v.split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or("")
                    .to_string()
            };
            return Some(decode_entities(&val));
        }
        from = i + name.len();
    }
    None
}

/// CSS 색 → `RRGGBB` (`#rrggbb`·`#rgb`·`rgb(r,g,b)`·몇 가지 이름)
fn color(v: &str) -> Option<String> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(h) = v.strip_prefix('#') {
        return match h.len() {
            6 if h.chars().all(|c| c.is_ascii_hexdigit()) => Some(h.to_ascii_uppercase()),
            3 if h.chars().all(|c| c.is_ascii_hexdigit()) => Some(
                h.chars()
                    .flat_map(|c| [c, c])
                    .collect::<String>()
                    .to_ascii_uppercase(),
            ),
            _ => None,
        };
    }
    if let Some(inner) = v.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let n: Vec<u8> = inner
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        if n.len() == 3 {
            return Some(format!("{:02X}{:02X}{:02X}", n[0], n[1], n[2]));
        }
    }
    Some(
        match v.as_str() {
            "white" => "FFFFFF",
            "black" => "000000",
            "red" => "FF0000",
            "yellow" => "FFFF00",
            "lime" => "00FF00",
            "green" => "008000",
            "blue" => "0000FF",
            "aqua" | "cyan" => "00FFFF",
            "fuchsia" | "magenta" => "FF00FF",
            "gray" | "grey" => "808080",
            "silver" => "C0C0C0",
            "navy" => "000080",
            "maroon" => "800000",
            "purple" => "800080",
            "olive" => "808000",
            "teal" => "008080",
            "orange" => "FFA500",
            _ => return None,
        }
        .to_string(),
    )
}

/// 표들을 한 시트에 차례로 놓는다. 칸 안 `<br>` 은 행을 쪼개고 (줄이 모자란 칸은 그 행들에 걸쳐 병합),
/// `colspan`·`rowspan` 은 병합이다. 표 칸은 모두 줄 바꿈 맞춤, `<th>` 는 굵게·가운데
fn layout(
    wb: &mut Workbook,
    cache: &mut StyleCache,
    tables: &[Vec<HRow>],
) -> (BTreeMap<(u32, u32), Cell>, Vec<CellRange>) {
    let mut cells = BTreeMap::new();
    let mut merges = Vec::new();
    let mut row0 = 0u32;
    for table in tables {
        // 자리 차지(rowspan·colspan) → 칸의 (표 행, 열)
        let mut occupied: std::collections::HashSet<(usize, u32)> = Default::default();
        let mut place: Vec<(usize, u32, &HCell)> = Vec::new();
        for (r, row) in table.iter().enumerate() {
            let mut c = 0u32;
            for cell in row {
                while occupied.contains(&(r, c)) {
                    c += 1;
                }
                for dr in 0..cell.rowspan as usize {
                    for dc in 0..cell.colspan {
                        occupied.insert((r + dr, c + dc));
                    }
                }
                place.push((r, c, cell));
                c += cell.colspan;
            }
        }
        // 표 행마다 줄 수 (rowspan 1 인 칸 가운데 가장 많은 줄)
        let mut lines = vec![1u32; table.len()];
        for (r, _, cell) in &place {
            if cell.rowspan == 1 {
                lines[*r] = lines[*r].max(cell.lines.len() as u32);
            }
        }
        let base: Vec<u32> = lines
            .iter()
            .scan(row0, |acc, l| {
                let b = *acc;
                *acc += l;
                Some(b)
            })
            .collect();
        for (r, c, cell) in place {
            let span_rows: u32 = (r..(r + cell.rowspan as usize).min(table.len()))
                .map(|k| lines[k])
                .sum();
            let patch = FormatPatch {
                bold: (cell.th || cell.bold).then_some(true),
                h_align: cell
                    .align
                    .clone()
                    .or_else(|| cell.th.then(|| "center".to_string())),
                fill: cell.fill.clone(),
                wrap: Some(true),
                ..Default::default()
            };
            let single = cell.lines.len() <= 1;
            let top = base[r];
            let line_rows = if single {
                span_rows
            } else {
                cell.lines.len() as u32
            };
            for j in 0..line_rows {
                let text = if single {
                    cell.lines.first().map_or("", |s| s.as_str())
                } else {
                    cell.lines.get(j as usize).map_or("", |s| s.as_str())
                };
                let rr = top + j;
                for dc in 0..cell.colspan {
                    let (row, col) = (rr, c + dc);
                    let first = dc == 0 && (j == 0 || !single);
                    let mut cl = if first && !text.trim().is_empty() {
                        field_cell(
                            wb,
                            cache,
                            CellRef { row, col },
                            text.trim(),
                            Some(&patch),
                            false,
                        )
                    } else {
                        None
                    };
                    // HTML 은 지수 꼴 수(1E+15)에 지수 서식(0.00E+00)을 붙이지 않는다 — 일반 (한셀 대조)
                    if let Some(c) = cl.as_mut()
                        && wb.styles.xf(c.style).is_some_and(|x| x.num_fmt_id == 11)
                    {
                        c.style = crate::csv::styled(wb, cache, &patch);
                    }
                    let cl = cl.unwrap_or_else(|| Cell {
                        r: CellRef { row, col },
                        style: crate::csv::styled(wb, cache, &patch),
                        value: CellValue::Empty,
                        formula: None,
                    });
                    cells.insert((row, col), cl);
                }
                if !single && cell.colspan > 1 {
                    merges.push(CellRange {
                        start: CellRef { row: rr, col: c },
                        end: CellRef {
                            row: rr,
                            col: c + cell.colspan - 1,
                        },
                    });
                }
            }
            if single && (span_rows > 1 || cell.colspan > 1) {
                merges.push(CellRange {
                    start: CellRef { row: top, col: c },
                    end: CellRef {
                        row: top + span_rows - 1,
                        col: c + cell.colspan - 1,
                    },
                });
            }
        }
        row0 = base
            .last()
            .map_or(row0, |b| b + lines.last().copied().unwrap_or(1));
    }
    (cells, merges)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet_text(wb: &Workbook) -> Vec<(u32, u32, String)> {
        wb.sheets[0]
            .cells
            .iter()
            .filter(|c| !matches!(c.value, CellValue::Empty))
            .map(|c| (c.r.row, c.r.col, crate::csv::shown(wb, c)))
            .collect()
    }

    #[test]
    fn html_tables_merge_split_and_type() {
        let h = "<table><tr><th>이름</th><th colspan=2>값</th></tr><tr><td>abc</td><td>1234.5</td><td>2023-03-15</td></tr>\
                 <tr><td rowspan=2 align=right><b>굵게</b></td><td>12%</td><td>TRUE</td></tr><tr><td>1,234</td><td>=1+1</td></tr>\
                 <tr><td>줄<br>바꿈</td><td style=\"background-color:#ffff00\">007</td><td>&lt;b&gt;&amp;</td></tr></table>\
                 <table><tr><td>둘째</td></tr></table>";
        let wb = html_document(h.as_bytes(), "t", None, None).unwrap();
        let s = sheet_text(&wb);
        let at = |r, c| {
            s.iter()
                .find(|x| x.0 == r && x.1 == c)
                .map(|x| x.2.as_str())
        };
        assert_eq!(at(0, 0), Some("이름"));
        assert_eq!(at(1, 1), Some("1234.5"));
        assert_eq!(at(1, 2), Some("2023-03-15"));
        assert_eq!(at(2, 1), Some("12%"));
        assert_eq!(at(2, 2), Some("TRUE"));
        assert_eq!(at(3, 1), Some("1,234"));
        assert_eq!(at(3, 2), Some("2"));
        assert_eq!(at(4, 0), Some("줄"));
        assert_eq!(at(5, 0), Some("바꿈"));
        assert_eq!(at(4, 1), Some("7"));
        assert_eq!(at(4, 2), Some("<b>&"));
        assert_eq!(at(6, 0), Some("둘째"));
        let m: Vec<(u32, u32, u32, u32)> = wb.sheets[0]
            .merges
            .iter()
            .map(|r| (r.start.row, r.start.col, r.end.row, r.end.col))
            .collect();
        // B1:C1, A3:A4, B5:B6, C5:C6
        for want in [(0, 1, 0, 2), (2, 0, 3, 0), (4, 1, 5, 1), (4, 2, 5, 2)] {
            assert!(m.contains(&want), "{want:?} in {m:?}");
        }
    }

    #[test]
    fn dbf_reads_header_and_types() {
        let mut b = vec![0u8; 32];
        b[0] = 3;
        let fields: [(&str, u8, u8); 3] = [("NAME", b'C', 6), ("AMT", b'N', 6), ("DT", b'D', 8)];
        let hdr = 32 + 32 * fields.len() + 1;
        b[4] = 2;
        b[8] = hdr as u8;
        b[10] = (1 + 6 + 6 + 8) as u8;
        for (n, t, l) in fields {
            let mut f = vec![0u8; 32];
            f[..n.len()].copy_from_slice(n.as_bytes());
            f[11] = t;
            f[16] = l;
            b.extend(f);
        }
        b.push(0x0D);
        for (n, a, d) in [("ab", "  1.50", "20230315"), ("cd", "", "")] {
            b.push(b' ');
            for (v, l) in [(n, 6), (a, 6), (d, 8)] {
                let mut x = v.as_bytes().to_vec();
                x.resize(l, b' ');
                b.extend(x);
            }
        }
        b.push(0x1A);
        let wb = dbf(&b, "t").unwrap();
        let s = sheet_text(&wb);
        assert!(s.contains(&(0, 0, "NAME".to_string())));
        assert!(s.contains(&(1, 0, "ab".to_string())));
        assert!(s.contains(&(1, 1, "1.5".to_string())));
        assert!(s.contains(&(1, 2, "20230315".to_string())));
        assert!(!s.iter().any(|x| x.0 == 2 && x.1 == 1));
    }

    #[test]
    fn xml2003_reads_strings_numbers_and_styles() {
        let x = r##"<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet">
 <Styles><Style ss:ID="s1"><NumberFormat ss:Format="#,##0.00"/></Style></Styles>
 <Worksheet ss:Name="A"><Table><Row><Cell><Data ss:Type="String">abc&#10;d</Data></Cell><Cell ss:StyleID="s1"><Data ss:Type="Number">1234.5</Data></Cell><Cell><Data ss:Type="Boolean">1</Data></Cell></Row></Table></Worksheet>
 <Worksheet ss:Name="B"><Table><Row><Cell ss:Index="2"><Data ss:Type="String">x</Data></Cell></Row></Table></Worksheet></Workbook>"##;
        let wb = xml_spreadsheet(x.as_bytes()).unwrap();
        assert_eq!(wb.sheets.len(), 2);
        assert_eq!(wb.sheets[1].name, "B");
        let s = sheet_text(&wb);
        assert!(s.contains(&(0, 0, "abc&#10;d".to_string())), "{s:?}");
        assert!(s.contains(&(0, 1, "1,234.50".to_string())), "{s:?}");
        assert!(!s.iter().any(|x| x.1 == 2));
    }
}
