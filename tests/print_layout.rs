// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 인쇄 페이지 나누기 대조 (docs/format/print.md).
//!
//! - `hcell_page_breaks`: 코드로 만든 문서들의 페이지 수·페이지 나누기·인쇄 배율이 한셀(13.0.0.711, 기본 프린터
//!   Hancom PDF)의 COM `Pages.Count`·`HPageBreaks`·`VPageBreaks` 와 PDF 글자 크기에서 잰 값과 같은지. 같은 문서를
//!   `RCELL_PRINT_CASES=<폴더>` 로 저장해 `tools/probe/print-oracle.ps1` 로 한셀에 다시 물을 수 있다.
//! - `oracle`: `RCELL_PRINT_ORACLE=<print-oracle.ps1 이 쓴 TSV>` 가 있으면 그 문서들 모두를 대조한다.

use rcell::Workbook;
use rcell::model::{
    CellRef, CellValue, ColInfo, DefinedName, Fill, Font, PageLayout, PageMargins, PageSetup,
    PrintOptions, RowInfo, Sheet, Styles, Xf,
};

/// 한셀 새 문서와 같은 바탕: 글꼴 0 = 맑은 고딕 11, 표준 열 너비 (72px), 행 높이 자동 (16.4pt = 22px)
fn base() -> Workbook {
    Workbook {
        styles: Styles {
            fonts: vec![Font {
                name: Some("맑은 고딕".into()),
                size: Some(11.0),
                ..Default::default()
            }],
            fills: vec![Fill::default()],
            borders: vec![Default::default()],
            cell_xfs: vec![Xf::default()],
            ..Default::default()
        },
        sheets: vec![Sheet {
            name: "Sheet1".into(),
            page: PageLayout {
                margins: Some(PageMargins {
                    left: 53.86 / 72.0,
                    right: 53.86 / 72.0,
                    top: 70.87 / 72.0,
                    bottom: 70.87 / 72.0,
                    header: 36.85 / 72.0,
                    footer: 36.85 / 72.0,
                }),
                setup: Some(PageSetup {
                    paper_size: Some(9),
                    scale: Some(100),
                    orientation: Some("portrait".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// 범위에 값 1 을 채운다 (`A1:D200`)
fn fill(wb: &mut Workbook, range: &str) {
    let (a, b) = range.split_once(':').unwrap();
    let (a, b) = (CellRef::parse(a).unwrap(), CellRef::parse(b).unwrap());
    let s = &mut wb.sheets[0];
    for r in a.row..=b.row {
        for c in a.col..=b.col {
            s.cells.push(rcell::model::Cell {
                r: CellRef { row: r, col: c },
                style: 0,
                value: CellValue::Number(1.0),
                formula: None,
            });
        }
    }
    s.cells.sort_by_key(|c| (c.r.row, c.r.col));
    s.cells.dedup_by_key(|c| (c.r.row, c.r.col));
}

/// 행 높이 (pt, 직접 정함)
fn rows(wb: &mut Workbook, from: u32, to: u32, pt: f64) {
    let s = &mut wb.sheets[0];
    for r in from..=to {
        s.rows.retain(|x| x.index != r);
        s.rows.push(RowInfo {
            index: r,
            height: Some(pt),
            custom_height: true,
            ..Default::default()
        });
    }
    s.rows.sort_by_key(|r| r.index);
}

/// 열 너비 (글자, 직접 정함 — 맑은 고딕 11 은 1 글자 8px)
fn cols(wb: &mut Workbook, from: u32, to: u32, width: f64) {
    wb.sheets[0].cols.push(ColInfo {
        min: from,
        max: to,
        width: Some(width),
        custom_width: true,
        ..Default::default()
    });
}

fn setup(wb: &mut Workbook) -> &mut PageSetup {
    wb.sheets[0].page.setup.as_mut().unwrap()
}

fn margins(wb: &mut Workbook) -> &mut PageMargins {
    wb.sheets[0].page.margins.as_mut().unwrap()
}

fn name(wb: &mut Workbook, n: &str, f: &str) {
    wb.defined_names.push(DefinedName {
        name: n.into(),
        local_sheet_id: Some(0),
        formula: f.into(),
        ..Default::default()
    });
}

/// (이름, 문서, 페이지 수, 가로 나누기, 세로 나누기, 인쇄 배율 — 0 이면 재지 않음)
type Case = (
    &'static str,
    Workbook,
    usize,
    &'static str,
    &'static str,
    u32,
);

/// 한셀에서 잰 사실들 (COM 페이지 나누기 실험, PDF 글자 크기 → 배율)
fn cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    let mut add = |n, wb, p, h, v, z| out.push((n, wb, p, h, v, z));

    let mut wb = base();
    fill(&mut wb, "A1:D200");
    add("default", wb, 5, "45,89,133,177", "", 97);

    // 시작이 B5 여도 페이지는 1행부터
    let mut wb = base();
    fill(&mut wb, "B5:D200");
    add("start_b5", wb, 5, "45,89,133,177", "", 0);

    // 행 높이 15pt (20px): 1/100 인치로 48 행 (960 ≤ 972), 그릴 배율 97
    let mut wb = base();
    fill(&mut wb, "A1:C200");
    rows(&mut wb, 1, 200, 15.0);
    add("rows15", wb, 5, "49,97,145,193", "", 97);

    // 위 여백에 따라 (행 15pt)
    for (top, h, z) in [
        (20.0, "53,105,157", 97),
        (50.0, "51,101,151", 97),
        (100.0, "47,93,139,185", 97),
        (200.0, "40,79,118,157,196", 97),
        (400.0, "26,51,76,101,126,151,176", 97),
    ] {
        let mut wb = base();
        fill(&mut wb, "A1:B200");
        rows(&mut wb, 1, 200, 15.0);
        margins(&mut wb).top = top / 72.0;
        let pages = h.split(',').count() + 1;
        add(
            Box::leak(format!("top{top}").into_boxed_str()),
            wb,
            pages,
            h,
            "",
            z,
        );
    }

    // 행 높이별 배율 (가장 큰 페이지가 1:1 로 들어갈 때까지 round(px × z/100) 를 줄인다)
    for (pt, n, h, z) in [
        (4.5, 400, "163,325", 91),
        (7.5, 240, "98,195", 94),
        (9.75, 185, "75,149", 96),
        (12.0, 150, "61,121", 96),
        (22.5, 67, "33,65", 98),
        (30.0, 60, "25,49", 96),
        (45.0, 40, "17,33", 97),
    ] {
        let mut wb = base();
        fill(&mut wb, &format!("A1:B{n}"));
        rows(&mut wb, 1, n, pt);
        add(
            Box::leak(format!("rowpt{pt}").into_boxed_str()),
            wb,
            3,
            h,
            "",
            z,
        );
    }

    // 배율 지정
    for (zoom, h, z) in [
        (50, "89,177,265", 0),
        (75, "59,117,175,233,291", 0),
        (90, "50,99,148,197,246,295", 0),
        (99, "45,89,133,177,221,265", 0),
        (150, "30,59,88,117,146,175,204,233,262,291", 0),
    ] {
        let mut wb = base();
        fill(&mut wb, "A1:D300");
        setup(&mut wb).scale = Some(zoom);
        let pages = h.split(',').count() + 1;
        add(
            Box::leak(format!("zoom{zoom}").into_boxed_str()),
            wb,
            pages,
            h,
            "",
            z,
        );
    }
    // 배율 90·50·150 에 행 15pt — 그릴 배율도 줄어든다
    for (zoom, h, z) in [
        (90, "55,109,163,217,271", 87),
        (75, "65,129,193,257", 72),
        (50, "98,195,292", 47),
        (120, "41,81,121,161,201,241,281", 117),
        (150, "33,65,97,129,161,193,225,257,289", 147),
    ] {
        let mut wb = base();
        fill(&mut wb, "A1:B300");
        rows(&mut wb, 1, 300, 15.0);
        setup(&mut wb).scale = Some(zoom);
        let pages = h.split(',').count() + 1;
        add(
            Box::leak(format!("zoom{zoom}_rows15").into_boxed_str()),
            wb,
            pages,
            h,
            "",
            z,
        );
    }

    // 숨긴 행·열
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    for r in 10..=20 {
        wb.sheets[0].rows.push(RowInfo {
            index: r,
            hidden: true,
            ..Default::default()
        });
    }
    wb.sheets[0].cols.push(ColInfo {
        min: 3,
        max: 4,
        hidden: true,
        ..Default::default()
    });
    add("hidden", wb, 15, "56,100,144,188", "12,21", 0);

    // 수동 나누기 (20행 위, E열 왼쪽)
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    wb.sheets[0].page.row_breaks = vec![19];
    wb.sheets[0].page.col_breaks = vec![4];
    add("manual", wb, 24, "20m,64,108,152,196", "5m,14,23", 0);

    // 반복 행·열
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    name(&mut wb, "_xlnm.Print_Titles", "Sheet1!$A:$B,Sheet1!$1:$3");
    add("titles", wb, 20, "45,86,127,168", "10,17,24", 0);
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    name(&mut wb, "_xlnm.Print_Titles", "Sheet1!$10:$12");
    add("titles_mid", wb, 15, "45,86,127,168", "10,19", 0);
    let mut wb = base();
    fill(&mut wb, "A1:B200");
    rows(&mut wb, 1, 200, 15.0);
    name(&mut wb, "_xlnm.Print_Titles", "Sheet1!$1:$3");
    add("titles_rows15", wb, 5, "49,94,139,184", "", 97);
    // 제목이 첫 페이지 뒤에 있으면 그 앞 페이지에도 붙고, 제목 행이 든 페이지에만 없다
    let mut wb = base();
    fill(&mut wb, "A1:B200");
    name(&mut wb, "_xlnm.Print_Titles", "Sheet1!$100:$102");
    add("titles_late", wb, 5, "42,83,127,168", "", 0);
    let mut wb = base();
    fill(&mut wb, "A1:Z20");
    name(&mut wb, "_xlnm.Print_Titles", "Sheet1!$M:$N");
    add("titles_late_cols", wb, 4, "", "8,17,24", 0);
    // 맞추기 1×3 은 수동 나누기를 버리고, 1×(제한 없음) 은 남긴다
    let mut wb = base();
    fill(&mut wb, "A1:B200");
    wb.sheets[0].page.row_breaks = vec![19];
    wb.sheets[0].page.fit_to_page = Some(true);
    setup(&mut wb).fit_to_width = Some(1);
    setup(&mut wb).fit_to_height = Some(3);
    add("fit_manual", wb, 3, "68,135", "", 0);
    let mut wb = base();
    fill(&mut wb, "A1:B200");
    wb.sheets[0].page.row_breaks = vec![19];
    wb.sheets[0].page.fit_to_page = Some(true);
    setup(&mut wb).fit_to_width = Some(1);
    setup(&mut wb).fit_to_height = Some(0);
    add("fit_manual_wide", wb, 6, "20m,64,108,152,196", "", 0);
    // 행·열 머리글은 나누기를 바꾸지 않는다
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    wb.sheets[0].page.print_options = Some(PrintOptions {
        headings: true,
        ..Default::default()
    });
    add("headings", wb, 15, "45,89,133,177", "10,19", 0);

    // 인쇄 영역
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    name(&mut wb, "_xlnm.Print_Area", "Sheet1!$C$5:$H$120");
    add("area", wb, 3, "49,93", "", 0);
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    name(
        &mut wb,
        "_xlnm.Print_Area",
        "Sheet1!$A$1:$C$10,Sheet1!$E$5:$F$80",
    );
    add("area2", wb, 3, "11,49", "4", 0);

    // 페이지보다 큰 행
    let mut wb = base();
    fill(&mut wb, "A1:B60");
    rows(&mut wb, 5, 5, 409.0);
    rows(&mut wb, 30, 30, 400.0);
    add("tallrow", wb, 3, "21,41", "", 0);
    let mut wb = base();
    fill(&mut wb, "A1:B60");
    rows(&mut wb, 5, 5, 409.0);
    margins(&mut wb).top = 300.0 / 72.0;
    add("tallrow_top300", wb, 3, "6,35", "", 0);

    // 넓은 열 (페이지보다 넓으면 혼자 한 페이지)
    let mut wb = base();
    fill(&mut wb, "A1:F10");
    cols(&mut wb, 3, 3, 150.0);
    add("widecol", wb, 3, "", "3,4", 0);

    // 용지·방향
    for (paper, landscape, n, h, v) in [
        (9, true, 16, "29,57,85,113,141,169,197", "15"),
        (1, false, 15, "42,83,124,165", "10,19"),
        (5, false, 12, "55,109,163", "10,19"),
        (11, false, 40, "29,57,85,113,141,169,197", "7,13,19,25"),
        (13, false, 24, "38,75,112,149,186", "8,15,22"),
    ] {
        let mut wb = base();
        fill(&mut wb, "A1:Z200");
        setup(&mut wb).paper_size = Some(paper);
        if landscape {
            setup(&mut wb).orientation = Some("landscape".into());
        }
        add(
            Box::leak(format!("paper{paper}{}", if landscape { "l" } else { "" }).into_boxed_str()),
            wb,
            n,
            h,
            v,
            0,
        );
    }
    for (paper, n, h, v) in [
        (8, 10, "67,133,199,265", "15"),
        (12, 18, "57,113,169,225,281", "12,23"),
    ] {
        let mut wb = base();
        fill(&mut wb, "A1:Z300");
        setup(&mut wb).paper_size = Some(paper);
        add(
            Box::leak(format!("paper{paper}").into_boxed_str()),
            wb,
            n,
            h,
            v,
            0,
        );
    }

    // 페이지 순서는 나누기를 바꾸지 않는다
    let mut wb = base();
    fill(&mut wb, "A1:Z200");
    setup(&mut wb).page_order = Some("overThenDown".into());
    add("order", wb, 15, "45,89,133,177", "10,19", 0);

    // 페이지에 맞추기 (열 A 300px, 행 15pt): 95 부터 줄여 찾고, 25 밑이면 12
    for (n, wide, tall, pages, h, z) in [
        (20, 1, 1, 1, "", 95),
        (60, 1, 1, 1, "", 77),
        (100, 1, 1, 1, "", 46),
        (160, 1, 1, 1, "", 27),
        (200, 1, 1, 1, "", 12),
        (400, 1, 1, 2, "389", 12),
        (200, 1, 2, 2, "102", 46),
        (200, 1, 3, 3, "68,135", 67),
        (200, 1, 4, 4, "51,101,151", 92),
        (200, 1, 0, 5, "50,99,148,197", 95),
    ] {
        let mut wb = base();
        fill(&mut wb, &format!("A1:B{n}"));
        rows(&mut wb, 1, n, 15.0);
        cols(&mut wb, 1, 1, (300.0 - 6.2) / 8.0 + 6.2 / 8.0);
        wb.sheets[0].page.fit_to_page = Some(true);
        setup(&mut wb).fit_to_width = Some(wide);
        setup(&mut wb).fit_to_height = Some(tall);
        add(
            Box::leak(format!("fit{n}_{wide}x{tall}").into_boxed_str()),
            wb,
            pages,
            h,
            "",
            z,
        );
    }

    // 머리글이 있어도 나누기는 같다
    let mut wb = base();
    fill(&mut wb, "A1:B200");
    margins(&mut wb).top = 20.0 / 72.0;
    margins(&mut wb).header = 10.0 / 72.0;
    wb.sheets[0].page.header_footer = Some(rcell::model::HeaderFooter {
        odd_header: Some("&C&40BIG".into()),
        ..Default::default()
    });
    add("header_big", wb, 5, "48,95,142,189", "", 0);

    // 첫 페이지·짝수 페이지 머리글 (PDF 비교용 — 나누기는 그대로)
    let mut wb = base();
    fill(&mut wb, "A1:D150");
    wb.sheets[0].page.header_footer = Some(rcell::model::HeaderFooter {
        different_first: true,
        different_odd_even: true,
        scale_with_doc: true,
        align_with_margins: true,
        first_header: Some("&C첫 쪽 &P".into()),
        odd_header: Some("&L홀수 &P&R&\"함초롬바탕,굵게\"&14&A".into()),
        even_header: Some("&R짝수 &P".into()),
        odd_footer: Some("&C&P / &N".into()),
        even_footer: Some("&C&8&K0000FF&P / &N".into()),
        ..Default::default()
    });
    add("hf_first_even", wb, 4, "45,89,133", "", 97);

    // 빈 시트
    add("empty", base(), 1, "", "", 0);

    // 가운데 맞춤·눈금선은 나누기를 바꾸지 않는다
    let mut wb = base();
    fill(&mut wb, "A1:D200");
    wb.sheets[0].page.print_options = Some(PrintOptions {
        horizontal_centered: true,
        vertical_centered: true,
        grid_lines: true,
        headings: false,
    });
    add("centered", wb, 5, "45,89,133,177", "", 0);
    out
}

fn list(b: &[(u32, bool)]) -> String {
    b.iter()
        .map(|(i, m)| format!("{i}{}", if *m { "m" } else { "" }))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn hcell_page_breaks() {
    let mut bad = Vec::new();
    let all = cases();
    if let Some(dir) = std::env::var_os("RCELL_PRINT_CASES") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (n, wb, ..) in &all {
            rcell::save(wb, dir.join(format!("{n}.cell"))).unwrap();
        }
    }
    for (n, wb, pages, h, v, z) in &all {
        let p = rcell::print::sheet_print(wb, 0);
        let got = (p.pages.len(), list(&p.h_breaks), list(&p.v_breaks));
        let want = (*pages, h.to_string(), v.to_string());
        if got != want {
            bad.push(format!("{n}: rcell {got:?} 한셀 {want:?}"));
        }
        if *z != 0 && p.print_scale != *z {
            bad.push(format!("{n}: 그릴 배율 rcell {} 한셀 {z}", p.print_scale));
        }
    }
    assert!(
        bad.is_empty(),
        "{} / {}:\n{}",
        bad.len(),
        all.len(),
        bad.join("\n")
    );
}

/// print-oracle.ps1 이 쓴 TSV 의 문서 모두 (한셀 COM 페이지 수·나누기)
#[test]
fn oracle() {
    let Some(path) = std::env::var_os("RCELL_PRINT_ORACLE") else {
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut dir = std::path::PathBuf::new();
    let mut total = 0;
    let mut bad = Vec::new();
    let mut cache: Option<(String, Workbook)> = None;
    for line in text.lines() {
        if let Some(d) = line.strip_prefix("#dir\t") {
            dir = d.into();
            continue;
        }
        let p: Vec<&str> = line.split('\t').collect();
        if p.len() < 6 {
            continue;
        }
        let (file, k, pages, h, v) = (p[0], p[1].parse::<usize>().unwrap(), p[3], p[4], p[5]);
        if cache.as_ref().is_none_or(|c| c.0 != file) {
            match rcell::open(dir.join(file)) {
                Ok(wb) => cache = Some((file.to_string(), wb)),
                Err(e) => {
                    bad.push(format!("{file}: {e}"));
                    continue;
                }
            }
        }
        let wb = &cache.as_ref().unwrap().1;
        total += 1;
        let sp = rcell::print::sheet_print(wb, k - 1);
        let got = (
            sp.pages.len().to_string(),
            list(&sp.h_breaks),
            list(&sp.v_breaks),
        );
        if got != (pages.to_string(), h.to_string(), v.to_string()) {
            bad.push(format!(
                "{file} {k}: rcell {got:?} 한셀 ({pages}, {h}, {v})"
            ));
        }
    }
    eprintln!("인쇄 페이지 대조: {} / {total} 같음", total - bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
