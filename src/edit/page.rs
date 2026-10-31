// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 쪽 설정 편집 (한셀 COM `PageSetup` 대조 실험 — docs/editing.md 20절): 여백·용지·방향·확대·페이지 맞추기·가운데 맞춤·눈금선·
//! 머리글 인쇄·인쇄 영역·반복 제목·머리글/바닥글·메모/오류 인쇄.
//!
//! - 여백은 포인트(÷72 = 인치), 값을 주지 않은 것은 그대로.
//! - 확대율을 주면 `scale`, 페이지 맞추기(`fitWide`·`fitTall`, 0 은 자동)를 주면 `fitToPage` 와 `autoPageBreaks` 를 켜고 `scale` 은
//!   한셀이 저장할 때 다시 셈한다. 맞추기 뒤에 확대율을 주면 `fitToPage` 가 꺼지고 `fitToWidth`·`fitToHeight` 는 그대로 남는다.
//! - 인쇄 영역·반복 제목은 시트 범위 이름 `_xlnm.Print_Area`·`_xlnm.Print_Titles` 로 둔다 (상대 주소도 절대로, 여러 영역은
//!   `,` 로 잇고 각각 시트 이름을 붙인다. 반복 제목은 행 다음 열). 빈 글이면 이름을 지운다.
//! - 머리글/바닥글: 왼쪽·가운데·오른쪽 글을 `&L…&C…&R…` 로 잇는다. 글꼴 지정(`&"이름,모양"`)이 없는 글은 문서 기본 글꼴의
//!   `Regular` 를 앞에 붙이고, 지정한 글꼴 이름은 대문자로, 모양 이름은 영어로(`굵게` → `Bold` …), `&B` 는 글꼴 지정으로 바꾼다.

use super::{err, refs};
use crate::model::{
    DefinedName, HeaderFooter, PageLayout, PageMargins, PageSetup, PrintOptions, Workbook,
};
use serde::Deserialize;

/// 쪽 설정 편집 내용 (준 것만 바꾼다)
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageSetupEdit {
    /// 여백 (포인트)
    #[serde(default)]
    pub margin_left: Option<f64>,
    #[serde(default)]
    pub margin_right: Option<f64>,
    #[serde(default)]
    pub margin_top: Option<f64>,
    #[serde(default)]
    pub margin_bottom: Option<f64>,
    #[serde(default)]
    pub margin_header: Option<f64>,
    #[serde(default)]
    pub margin_footer: Option<f64>,
    /// `portrait` / `landscape`
    #[serde(default)]
    pub orientation: Option<String>,
    #[serde(default)]
    pub paper_size: Option<u32>,
    /// 확대율 (10–400)
    #[serde(default)]
    pub zoom: Option<u32>,
    /// 페이지 맞추기: 가로·세로 쪽 수 (0 은 자동). 둘 중 하나라도 주면 맞추기를 켠다
    #[serde(default)]
    pub fit_wide: Option<u32>,
    #[serde(default)]
    pub fit_tall: Option<u32>,
    #[serde(default)]
    pub center_h: Option<bool>,
    #[serde(default)]
    pub center_v: Option<bool>,
    #[serde(default)]
    pub grid_lines: Option<bool>,
    #[serde(default)]
    pub headings: Option<bool>,
    #[serde(default)]
    pub black_and_white: Option<bool>,
    #[serde(default)]
    pub draft: Option<bool>,
    /// `downThenOver` / `overThenDown`
    #[serde(default)]
    pub order: Option<String>,
    /// 시작 쪽 번호 (0 이면 자동)
    #[serde(default)]
    pub first_page_number: Option<u32>,
    /// 인쇄 영역 (`A1:C10` 또는 `A1:C10,E1:F5`, 빈 글이면 지운다)
    #[serde(default)]
    pub print_area: Option<String>,
    /// 반복할 행 (`1:2`)·열 (`A:B`) (빈 글이면 지운다)
    #[serde(default)]
    pub print_title_rows: Option<String>,
    #[serde(default)]
    pub print_title_cols: Option<String>,
    #[serde(default)]
    pub header_left: Option<String>,
    #[serde(default)]
    pub header_center: Option<String>,
    #[serde(default)]
    pub header_right: Option<String>,
    #[serde(default)]
    pub footer_left: Option<String>,
    #[serde(default)]
    pub footer_center: Option<String>,
    #[serde(default)]
    pub footer_right: Option<String>,
    /// `none` / `atEnd` / `inPlace`
    #[serde(default)]
    pub comments: Option<String>,
    /// `displayed` / `blank` / `dash` / `na`
    #[serde(default)]
    pub errors: Option<String>,
}

/// 한셀의 새 문서 쪽 여백 (인치 — 한셀이 저장한 값)
fn default_margins() -> PageMargins {
    PageMargins {
        left: 0.7479166388511658,
        right: 0.7479166388511658,
        top: 0.9838888645172119,
        bottom: 0.9838888645172119,
        header: 0.5119444727897644,
        footer: 0.5119444727897644,
    }
}

fn default_setup() -> PageSetup {
    PageSetup {
        paper_size: Some(9),
        scale: Some(100),
        first_page_number: Some(1),
        fit_to_width: Some(0),
        fit_to_height: Some(0),
        orientation: Some("portrait".into()),
        use_printer_defaults: true,
        horizontal_dpi: Some(0),
        vertical_dpi: Some(0),
        copies: Some(1),
        ..Default::default()
    }
}

/// 범위 글 하나(`A1:C10`·`A1`·`A:B`·`1:3`) → 절대 주소 (`$A$1:$C$10` …)
fn abs_token(t: &str) -> crate::Result<String> {
    let t = t.trim().replace('$', "");
    let bad = || err(format!("주소가 올바르지 않습니다: {t}"));
    let (a, b) = match t.split_once(':') {
        Some((a, b)) => (a, Some(b)),
        None => (t.as_str(), None),
    };
    let is_col = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic());
    let is_row = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let cell = |s: &str| -> Option<String> {
        let k = s.find(|c: char| c.is_ascii_digit())?;
        (k > 0 && is_col(&s[..k]) && is_row(&s[k..]))
            .then(|| format!("${}${}", s[..k].to_uppercase(), &s[k..]))
    };
    match b {
        None => cell(a).ok_or_else(bad),
        Some(b) => {
            if is_col(a) && is_col(b) {
                Ok(format!("${}:${}", a.to_uppercase(), b.to_uppercase()))
            } else if is_row(a) && is_row(b) {
                Ok(format!("${a}:${b}"))
            } else {
                Ok(format!(
                    "{}:{}",
                    cell(a).ok_or_else(bad)?,
                    cell(b).ok_or_else(bad)?
                ))
            }
        }
    }
}

/// 시트 범위 이름을 바꾸거나 지운다
fn set_local_name(wb: &mut Workbook, si: usize, name: &str, formula: Option<String>) {
    let lid = Some(si as u32);
    let pos = wb
        .defined_names
        .iter()
        .position(|d| d.local_sheet_id == lid && d.name == name);
    match (pos, formula) {
        (Some(i), Some(f)) => wb.defined_names[i].formula = f,
        (Some(i), None) => {
            wb.defined_names.remove(i);
        }
        (None, Some(f)) => wb.defined_names.push(DefinedName {
            name: name.into(),
            local_sheet_id: lid,
            hidden: false,
            formula: f,
            comment: None,
        }),
        (None, None) => {}
    }
}

/// 머리글/바닥글 글을 왼쪽·가운데·오른쪽으로 가른다 (`&&` 는 글자 `&`)
fn split_sections(t: &str) -> [String; 3] {
    let mut out = [String::new(), String::new(), String::new()];
    let mut cur = 1usize; // 표시 없이 시작하면 가운데
    let cs: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '&' && i + 1 < cs.len() {
            match cs[i + 1] {
                'L' => {
                    cur = 0;
                    i += 2;
                    continue;
                }
                'C' => {
                    cur = 1;
                    i += 2;
                    continue;
                }
                'R' => {
                    cur = 2;
                    i += 2;
                    continue;
                }
                '&' => {
                    out[cur].push_str("&&");
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out[cur].push(cs[i]);
        i += 1;
    }
    out
}

/// 한셀 머리글/바닥글 글 다듬기: `&B` 를 글꼴 지정으로, 글꼴 이름은 대문자·모양은 영어로, 글꼴 지정이 없으면 기본 글꼴 `Regular`
fn hf_text(t: &str, default_font: &str) -> String {
    let cs: Vec<char> = t.chars().collect();
    let mut out = String::new();
    let (mut font, mut bold) = (default_font.to_string(), false);
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '&' && i + 1 < cs.len() {
            if cs[i + 1] == '&' {
                out.push_str("&&");
                i += 2;
                continue;
            }
            if cs[i + 1] == '"'
                && let Some(end) = cs[i + 2..].iter().position(|&c| c == '"')
            {
                let spec: String = cs[i + 2..i + 2 + end].iter().collect();
                let (name, style) = spec.split_once(',').unwrap_or((spec.as_str(), "Regular"));
                let style = match style.trim() {
                    "굵게" => "Bold",
                    "기울임꼴" => "Italic",
                    "굵은 기울임꼴" => "Bold Italic",
                    "보통" => "Regular",
                    o => o,
                };
                font = name.to_uppercase();
                bold = style.starts_with("Bold");
                out.push_str(&format!("&\"{font},{style}\""));
                i += 2 + end + 1;
                continue;
            }
            if cs[i + 1] == 'B' {
                bold = !bold;
                out.push_str(&format!(
                    "&\"{font},{}\"",
                    if bold { "Bold" } else { "Regular" }
                ));
                i += 2;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    if out.is_empty() || out.starts_with("&\"") {
        out
    } else {
        format!("&\"{default_font},Regular\"{out}")
    }
}

fn join_sections(s: &[String; 3]) -> Option<String> {
    let mut t = String::new();
    for (mark, x) in ["&L", "&C", "&R"].iter().zip(s) {
        if !x.is_empty() {
            t.push_str(mark);
            t.push_str(x);
        }
    }
    (!t.is_empty()).then_some(t)
}

/// 쪽 설정 편집
pub fn page_setup(wb: &mut Workbook, si: usize, p: &PageSetupEdit) -> crate::Result<()> {
    if si >= wb.sheets.len() {
        return Err(err("시트가 없습니다"));
    }
    if let Some(z) = p.zoom
        && !(10..=400).contains(&z)
    {
        return Err(err("확대율은 10 이상 400 이하여야 합니다"));
    }
    if let Some(o) = &p.orientation
        && o != "portrait"
        && o != "landscape"
    {
        return Err(err("방향은 portrait 또는 landscape 입니다"));
    }
    if let Some(o) = &p.order
        && o != "downThenOver"
        && o != "overThenDown"
    {
        return Err(err("쪽 차례는 downThenOver 또는 overThenDown 입니다"));
    }
    // 인쇄 영역·반복 제목 (이름)
    let sheet_name = wb.sheets[si].name.clone();
    let qualified = |text: &str| -> crate::Result<String> {
        let mut parts = Vec::new();
        for t in text.split(',').filter(|t| !t.trim().is_empty()) {
            parts.push(format!(
                "{}{}",
                refs::prefix_text(&sheet_name, None),
                abs_token(t)?
            ));
        }
        Ok(parts.join(","))
    };
    if let Some(a) = &p.print_area {
        let f = qualified(a)?;
        set_local_name(wb, si, "_xlnm.Print_Area", (!f.is_empty()).then_some(f));
    }
    if p.print_title_rows.is_some() || p.print_title_cols.is_some() {
        let cur = wb
            .defined_names
            .iter()
            .find(|d| d.local_sheet_id == Some(si as u32) && d.name == "_xlnm.Print_Titles")
            .map(|d| d.formula.clone())
            .unwrap_or_default();
        // 지금 이름에서 행·열 몫을 가른다 (`$` 로 시작하는 숫자는 행, 글자는 열)
        let mut rows = String::new();
        let mut cols = String::new();
        for part in cur.split(',') {
            let tail = part.rsplit('!').next().unwrap_or(part);
            if tail
                .trim_start_matches('$')
                .starts_with(|c: char| c.is_ascii_digit())
            {
                rows = part.to_string();
            } else if !tail.is_empty() {
                cols = part.to_string();
            }
        }
        if let Some(r) = &p.print_title_rows {
            rows = qualified(r)?;
        }
        if let Some(c) = &p.print_title_cols {
            cols = qualified(c)?;
        }
        let f = [rows, cols]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        set_local_name(wb, si, "_xlnm.Print_Titles", (!f.is_empty()).then_some(f));
    }
    // 머리글/바닥글
    let default_font = wb
        .styles
        .fonts
        .first()
        .and_then(|f| f.name.clone())
        .unwrap_or_else(|| "맑은 고딕".into());
    let hf_given = [
        &p.header_left,
        &p.header_center,
        &p.header_right,
        &p.footer_left,
        &p.footer_center,
        &p.footer_right,
    ]
    .iter()
    .any(|x| x.is_some());
    let page: &mut PageLayout = &mut wb.sheets[si].page;
    // 여백
    if [
        p.margin_left,
        p.margin_right,
        p.margin_top,
        p.margin_bottom,
        p.margin_header,
        p.margin_footer,
    ]
    .iter()
    .any(|x| x.is_some())
    {
        let m = page.margins.get_or_insert_with(default_margins);
        for (dst, v) in [
            (&mut m.left, p.margin_left),
            (&mut m.right, p.margin_right),
            (&mut m.top, p.margin_top),
            (&mut m.bottom, p.margin_bottom),
            (&mut m.header, p.margin_header),
            (&mut m.footer, p.margin_footer),
        ] {
            if let Some(v) = v {
                *dst = v / 72.0;
            }
        }
    }
    // 용지·방향·확대·맞추기·그 밖
    if p.orientation.is_some()
        || p.paper_size.is_some()
        || p.zoom.is_some()
        || p.fit_wide.is_some()
        || p.fit_tall.is_some()
        || p.black_and_white.is_some()
        || p.draft.is_some()
        || p.order.is_some()
        || p.first_page_number.is_some()
        || p.comments.is_some()
        || p.errors.is_some()
    {
        let s = page.setup.get_or_insert_with(default_setup);
        if let Some(o) = &p.orientation {
            s.orientation = Some(o.clone());
        }
        if let Some(n) = p.paper_size {
            s.paper_size = Some(n);
        }
        // 맞추기가 켜진 채 확대율 100 은 한셀이 받지 않는다 (대조 실험 ps6)
        if let Some(z) = p.zoom
            && !(z == 100 && page.fit_to_page == Some(true))
        {
            s.scale = Some(z);
            if page.fit_to_page == Some(true) {
                page.fit_to_page = Some(false);
            }
        }
        if p.fit_wide.is_some() || p.fit_tall.is_some() {
            page.fit_to_page = Some(true);
            page.auto_page_breaks.get_or_insert(true);
            // 쪽 수가 정해진 뒤 `0`(자동)으로 되돌리는 것은 한셀이 오류로 막는다 — 앞서 한 것은 그대로 남는다 (ps6)
            if let Some(w) = p.fit_wide {
                if w == 0 && s.fit_to_width.unwrap_or(0) != 0 {
                    return Err(err("페이지 맞추기 쪽 수를 자동으로 되돌릴 수 없습니다"));
                }
                s.fit_to_width = Some(w);
            }
            if let Some(t) = p.fit_tall {
                if t == 0 && s.fit_to_height.unwrap_or(0) != 0 {
                    return Err(err("페이지 맞추기 쪽 수를 자동으로 되돌릴 수 없습니다"));
                }
                s.fit_to_height = Some(t);
            }
        }
        if let Some(b) = p.black_and_white {
            s.black_and_white = b;
        }
        if let Some(b) = p.draft {
            s.draft = b;
        }
        if let Some(o) = &p.order {
            s.page_order = (o == "overThenDown").then(|| "overThenDown".to_string());
        }
        if let Some(n) = p.first_page_number {
            s.first_page_number = Some(if n == 0 { 1 } else { n });
            s.use_first_page_number = n != 0;
        }
        match p.comments.as_deref() {
            None => {}
            Some("none") => s.cell_comments = None,
            Some("atEnd") => s.cell_comments = Some("atEnd".into()),
            Some("inPlace") => s.cell_comments = Some("asDisplayed".into()),
            Some(o) => return Err(err(format!("메모 인쇄 방식이 아닙니다: {o}"))),
        }
        match p.errors.as_deref() {
            None => {}
            Some("displayed") => s.errors = None,
            Some("blank") => s.errors = Some("blank".into()),
            Some("dash") => s.errors = Some("dash".into()),
            Some("na") => s.errors = Some("NA".into()),
            Some(o) => return Err(err(format!("오류 인쇄 방식이 아닙니다: {o}"))),
        }
    }
    // 인쇄 옵션
    if p.center_h.is_some()
        || p.center_v.is_some()
        || p.grid_lines.is_some()
        || p.headings.is_some()
    {
        let o = page.print_options.get_or_insert_with(PrintOptions::default);
        if let Some(b) = p.center_h {
            o.horizontal_centered = b;
        }
        if let Some(b) = p.center_v {
            o.vertical_centered = b;
        }
        if let Some(b) = p.grid_lines {
            o.grid_lines = b;
        }
        if let Some(b) = p.headings {
            o.headings = b;
        }
        if !(o.horizontal_centered || o.vertical_centered || o.grid_lines || o.headings) {
            page.print_options = None;
        }
    }
    if hf_given {
        let hf = page.header_footer.get_or_insert_with(|| HeaderFooter {
            scale_with_doc: true,
            align_with_margins: true,
            ..Default::default()
        });
        let mut head = split_sections(hf.odd_header.as_deref().unwrap_or(""));
        let mut foot = split_sections(hf.odd_footer.as_deref().unwrap_or(""));
        let apply = |sec: &mut String, v: &Option<String>| {
            if let Some(v) = v {
                *sec = if v.is_empty() {
                    String::new()
                } else {
                    hf_text(v, &default_font)
                };
            }
        };
        apply(&mut head[0], &p.header_left);
        apply(&mut head[1], &p.header_center);
        apply(&mut head[2], &p.header_right);
        apply(&mut foot[0], &p.footer_left);
        apply(&mut foot[1], &p.footer_center);
        apply(&mut foot[2], &p.footer_right);
        hf.odd_header = join_sections(&head);
        hf.odd_footer = join_sections(&foot);
    }
    // 페이지 맞추기를 켜면 한셀은 그때 배율을 셈해 `scale` 에 둔다 (인쇄 배치 — 대조 실험 ps4·ps5)
    if (p.fit_wide.is_some() || p.fit_tall.is_some())
        && wb.sheets[si].page.fit_to_page == Some(true)
    {
        let both_auto =
            wb.sheets[si].page.setup.as_ref().is_some_and(|s| {
                s.fit_to_width.unwrap_or(0) == 0 && s.fit_to_height.unwrap_or(0) == 0
            });
        // 가로·세로 모두 자동이면 배율 100
        let z = if both_auto {
            100
        } else {
            crate::print::sheet_print(wb, si).page_scale
        };
        if let Some(s) = &mut wb.sheets[si].page.setup {
            s.scale = Some(z);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_text_like_hcell() {
        let f = "맑은 고딕";
        assert_eq!(hf_text("&A 쪽 &P", f), "&\"맑은 고딕,Regular\"&A 쪽 &P");
        assert_eq!(
            hf_text("&\"Arial,굵은 기울임꼴\"&12X", f),
            "&\"ARIAL,Bold Italic\"&12X"
        );
        assert_eq!(
            hf_text("&B굵게&B 일반", f),
            "&\"맑은 고딕,Bold\"굵게&\"맑은 고딕,Regular\" 일반"
        );
        assert_eq!(hf_text("&12큰", f), "&\"맑은 고딕,Regular\"&12큰");
        let s = split_sections("&L왼&C가운데&R오른");
        assert_eq!(s, ["왼", "가운데", "오른"]);
        assert_eq!(join_sections(&s).as_deref(), Some("&L왼&C가운데&R오른"));
    }

    #[test]
    fn absolute_tokens() {
        assert_eq!(abs_token("A1:C10").unwrap(), "$A$1:$C$10");
        assert_eq!(abs_token("a1").unwrap(), "$A$1");
        assert_eq!(abs_token("A:B").unwrap(), "$A:$B");
        assert_eq!(abs_token("1:3").unwrap(), "$1:$3");
        assert!(abs_token("가나").is_err());
    }
}
