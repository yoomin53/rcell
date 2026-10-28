// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 인쇄 배치와 PDF (한셀 인쇄·PDF 저장과 같은 페이지, docs/format/print.md).
//!
//! - [`sheet_print`]: 페이지 나누기(자동·수동, 인쇄 영역, 반복 행·열, 숨김, 페이지 순서)와 인쇄 배율 —
//!   한셀 COM `HPageBreaks`·`VPageBreaks`·`Pages.Count` 와 같게.
//! - [`workbook_pdf`]: 보이는 시트들을 PDF 로. 한셀 PDF 처럼 600 dpi 장치 단위로 자리를 정한다: 용지는 mm 정수로
//!   반올림한 크기(MediaBox 는 pt 정수로 자른 값), 여백은 장치 단위로 반올림, 행·열 경계는 인쇄 px × 6.25 를 쌓아
//!   반올림한 자리 ([`frame`]).
//! - [`paper`]: 용지 번호 → 크기. [`hf`]: 머리글·바닥글 코드. [`font`]: 시스템 글꼴 찾기와 넣기.

pub mod font;
pub mod hf;
mod paginate;
pub mod paper;
mod pdf;

pub use font::FontDb;
pub use paginate::{Area, Page, Scaling, Settings, SheetPrint, settings, sheet_print, used_end};

use crate::model::Workbook;
use crate::render::page::{ErrorPrint, FontKey, Op, PageFrame, TextMeasure, page_ops};
use pdf::{FontSet, PageSize};

/// PDF 만들기 설정
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    /// 넣을 시트 (0부터). None 이면 보이는 시트 모두 (한셀 PDF 저장과 같다)
    pub sheets: Option<Vec<usize>>,
    /// `&F` (파일 이름)·`&Z` (폴더 경로, 끝에 구분자)
    pub file_name: String,
    pub path: String,
    /// `&D`·`&T` 에 넣을 날짜·시간 글 (한셀 꼴: `2026-10-28`, `06:13 오후`)
    pub date: String,
    pub time: String,
}

/// 장치 단위 (600 dpi) 1pt
const DEV_PT: f64 = 25.0 / 3.0;

/// 한셀 PDF 의 용지: mm 정수로 반올림 (Letter 215.9×279.4 → 216×279 → MediaBox 612×790)
pub fn page_size(s: &Settings) -> PageSize {
    let mm = |pt: f64| (pt / 72.0 * 25.4).round();
    let (w, h) = (mm(s.paper.width), mm(s.paper.height));
    PageSize {
        width: (w / 25.4 * 72.0).floor(),
        height: (h / 25.4 * 72.0).floor(),
        dev_w: (w / 25.4 * 600.0).round(),
        dev_h: (h / 25.4 * 600.0).round(),
    }
}

/// 장치 단위 경계: 시작 + Σ(인쇄 px × 6.25) 를 반올림 (반은 내림 — 한셀 PDF 의 선 자리)
fn bounds(start: f64, sizes: &[(u32, f64)]) -> Vec<(u32, f64, f64)> {
    let mut out = Vec::new();
    let mut cum = 0.0;
    let at = |c: f64| start + (c * 6.25 - 0.5).ceil();
    for &(i, px) in sizes {
        let a = at(cum);
        cum += px;
        out.push((i, a, at(cum)));
    }
    out
}

/// 페이지 한 장의 자리 (장치 단위)
pub fn frame(sp: &SheetPrint, page: &Page, size: PageSize) -> PageFrame {
    let s = &sp.settings;
    let x0 = (s.left * DEV_PT).round();
    let y0 = (s.top * DEV_PT).round();
    let cols: Vec<(u32, f64)> = page
        .title_cols
        .iter()
        .chain(&page.cols)
        .map(|&c| (c, sp.col_print_px(c)))
        .collect();
    let rows: Vec<(u32, f64)> = page
        .title_rows
        .iter()
        .chain(&page.rows)
        .map(|&r| (r, sp.row_print_px(r)))
        .collect();
    let w: f64 = cols.iter().map(|c| c.1).sum::<f64>() * 6.25;
    let h: f64 = rows.iter().map(|r| r.1).sum::<f64>() * 6.25;
    let avail_w = size.dev_w - x0 - (s.right * DEV_PT).round();
    let avail_h = size.dev_h - y0 - (s.bottom * DEV_PT).round();
    let dx = if s.center_h {
        ((avail_w - w) / 2.0).floor().max(0.0)
    } else {
        0.0
    };
    let dy = if s.center_v {
        ((avail_h - h) / 2.0).floor().max(0.0)
    } else {
        0.0
    };
    let z = if sp.tall { 100 } else { sp.print_scale };
    PageFrame {
        cols: bounds(x0 + dx, &cols),
        rows: bounds(y0 + dy, &rows),
        row_dev: (0..sp.row_px.len() as u32)
            .map(|r| sp.row_print_px(r) * 6.25)
            .collect(),
        col_dev: (0..sp.col_px.len() as u32)
            .map(|c| sp.col_print_px(c) * 6.25)
            .collect(),
        px: 6.25 * z as f64 / 100.0,
        zoom: z,
        gridlines: false,
        black_and_white: false,
        draft: false,
        errors: ErrorPrint::Displayed,
    }
}

/// 머리글·바닥글 한 줄 (구역 셋)을 그린다. `baseline_of`: (첫 조각의 오름, 내림) → 기준선
#[allow(clippy::too_many_arguments)]
fn hf_ops(
    sec: &hf::HfSections,
    default_font: &(String, f64),
    left: f64,
    right: f64,
    header: bool,
    edge: f64,
    fonts: &FontSet,
    ops: &mut Vec<Op>,
) {
    for (k, runs) in [&sec.left, &sec.center, &sec.right].into_iter().enumerate() {
        if runs.iter().all(|r| r.text.is_empty()) {
            continue;
        }
        let items: Vec<(FontKey, f64, &hf::HfRun)> = runs
            .iter()
            .map(|r| {
                let key = FontKey {
                    name: r.font.clone().unwrap_or_else(|| default_font.0.clone()),
                    bold: r.bold,
                    italic: r.italic,
                };
                let pt = r.size.unwrap_or(default_font.1);
                ((key), (pt * DEV_PT).round().max(1.0), r)
            })
            .collect();
        let widths: Vec<f64> = items
            .iter()
            .map(|(key, sz, r)| fonts.width(key, *sz, &r.text))
            .collect();
        let total: f64 = widths.iter().sum();
        let mut x = match k {
            0 => left,
            1 => (left + right) / 2.0 - total / 2.0,
            _ => right - total,
        };
        // 기준선: 머리글은 여백 + 가장 큰 글꼴의 오름, 바닥글은 여백 − 내림
        let (asc, desc) = items.iter().fold((0.0f64, 0.0f64), |(a, d), (key, sz, _)| {
            let (fa, fd) = fonts.ascent_descent(key);
            (a.max(fa * sz / 1000.0), d.max(fd * sz / 1000.0))
        });
        let y = if header { edge + asc } else { edge - desc };
        for ((key, sz, r), w) in items.iter().zip(&widths) {
            let c = r
                .color
                .as_deref()
                .and_then(|h| {
                    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
                    Some([p(0)?, p(2)?, p(4)?])
                })
                .unwrap_or([0, 0, 0]);
            let (size, dy) = if r.superscript || r.subscript {
                (
                    sz * 0.66,
                    if r.superscript { -sz * 0.33 } else { sz * 0.15 },
                )
            } else {
                (*sz, 0.0)
            };
            ops.push(Op::Text {
                x,
                y: y + dy,
                size,
                font: key.clone(),
                color: c,
                text: r.text.clone(),
                underline: r.underline,
                strike: r.strike,
                stretch: 1.0,
            });
            x += w;
        }
    }
}

/// 통합 문서를 PDF 로 (보이는 시트들, 페이지 순서대로)
pub fn workbook_pdf(wb: &Workbook, db: &FontDb, opts: &PdfOptions) -> Vec<u8> {
    let fonts = FontSet::new(db);
    let sheets: Vec<usize> = opts.sheets.clone().unwrap_or_else(|| {
        wb.sheets
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.state == crate::model::SheetState::Visible
                    && s.kind == crate::model::SheetKind::Worksheet
            })
            .map(|(i, _)| i)
            .collect()
    });
    let f0 = wb.styles.fonts.first();
    let default_font = (
        f0.and_then(|f| f.name.clone())
            .unwrap_or_else(|| "맑은 고딕".into()),
        f0.and_then(|f| f.size).unwrap_or(11.0),
    );
    let mut pages: Vec<(PageSize, Vec<Op>)> = Vec::new();
    for si in sheets {
        let Some(sheet) = wb.sheets.get(si) else {
            continue;
        };
        let sp = sheet_print(wb, si);
        let size = page_size(&sp.settings);
        let po = sheet.page.print_options.as_ref();
        let setup = sheet.page.setup.as_ref();
        let hfd = sheet.page.header_footer.as_ref();
        let total = sp.pages.len() as u32;
        let first_no = sp.settings.first_page_number.unwrap_or(1);
        for (pi, page) in sp.pages.iter().enumerate() {
            let mut fr = frame(&sp, page, size);
            fr.gridlines = po.is_some_and(|o| o.grid_lines);
            fr.black_and_white = setup.is_some_and(|s| s.black_and_white);
            fr.draft = setup.is_some_and(|s| s.draft);
            let mut ops = page_ops(wb, sheet, &fr, &fonts);
            // 머리글·바닥글
            if let Some(h) = hfd {
                let no = first_no + pi as u32;
                let fields = hf::HfFields {
                    page: no,
                    pages: total + first_no - 1,
                    sheet: sheet.name.clone(),
                    file: opts.file_name.clone(),
                    path: opts.path.clone(),
                    date: opts.date.clone(),
                    time: opts.time.clone(),
                };
                let (head, foot) = if h.different_first && pi == 0 {
                    (h.first_header.as_deref(), h.first_footer.as_deref())
                } else if h.different_odd_even && no.is_multiple_of(2) {
                    (h.even_header.as_deref(), h.even_footer.as_deref())
                } else {
                    (h.odd_header.as_deref(), h.odd_footer.as_deref())
                };
                let s = &sp.settings;
                let left = (s.left * DEV_PT).round();
                let right = size.dev_w - (s.right * DEV_PT).round();
                if let Some(t) = head {
                    let sec = hf::parse(t, &fields);
                    let edge = (s.header * DEV_PT).round();
                    hf_ops(
                        &sec,
                        &default_font,
                        left,
                        right,
                        true,
                        edge,
                        &fonts,
                        &mut ops,
                    );
                }
                if let Some(t) = foot {
                    let sec = hf::parse(t, &fields);
                    let edge = size.dev_h - (s.footer * DEV_PT).round();
                    hf_ops(
                        &sec,
                        &default_font,
                        left,
                        right,
                        false,
                        edge,
                        &fonts,
                        &mut ops,
                    );
                }
            }
            pages.push((size, ops));
        }
    }
    if pages.is_empty() {
        // 빈 문서도 빈 A4 한 장
        let s = paper::paper(Some(9));
        let size = page_size(&Settings {
            paper: s,
            ..settings_default()
        });
        pages.push((size, Vec::new()));
    }
    pdf::write_pdf(&pages, &fonts, &opts.file_name)
}

/// 지금 날짜·시간을 한셀 머리글 꼴로 (`2026-10-28`, `06:13 오후`). 시간대는 `RCELL_TZ`(분, 예: 540) 가 없으면 UTC
pub fn now_strings() -> (String, String) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let tz: i64 = std::env::var("RCELL_TZ")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let t = secs + tz * 60;
    let days = t.div_euclid(86_400);
    let sod = t.rem_euclid(86_400);
    // 1970-01-01 부터 날짜 (그레고리력)
    let (y, m, d) = {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + era * 400 + i64::from(m <= 2), m, d)
    };
    let (hh, mm) = (sod / 3600, sod % 3600 / 60);
    let (ap, h12) = if hh < 12 {
        ("오전", if hh == 0 { 12 } else { hh })
    } else {
        ("오후", if hh == 12 { 12 } else { hh - 12 })
    };
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!("{h12:02}:{mm:02} {ap}"),
    )
}

/// 빈 설정 (페이지가 없을 때 용지만 쓰려고)
fn settings_default() -> Settings {
    Settings {
        paper: paper::paper(Some(9)),
        left: 53.86,
        right: 53.86,
        top: 70.87,
        bottom: 70.87,
        header: 36.85,
        footer: 36.85,
        scaling: Scaling::Zoom(100),
        center_h: false,
        center_v: false,
        over_then_down: false,
        areas: Vec::new(),
        title_rows: None,
        title_cols: None,
        row_breaks: Vec::new(),
        col_breaks: Vec::new(),
        first_page_number: None,
    }
}
