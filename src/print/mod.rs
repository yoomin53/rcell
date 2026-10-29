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
mod pdfimage;

pub use font::FontDb;
pub use paginate::{
    Area, Page, Scaling, Settings, SheetPrint, settings, sheet_print, sheet_print_with, used_end,
};

use crate::model::Workbook;
use crate::render::page::{
    ErrorPrint, FontKey, Op, PageFrame, SheetPainter, TextMeasure, objects_ops,
};
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
    /// 쓸 페이지 수 한도 (None 이면 [`MAX_PAGES`])
    pub max_pages: Option<usize>,
    /// 흑백 인쇄·간단히 인쇄 설정을 따른다 (한셀 PDF 저장은 둘 다 따르지 않는다 — 기본 거짓)
    pub quality_settings: bool,
    /// 한셀이 문서를 열 때 다시 계산하는 칸 ([`open_recalc`]). 기본은 저장된 값 그대로
    pub recalc: OpenRecalc,
}

/// 한셀이 문서를 열 때 다시 계산하는 범위 (값을 0·틀린 값으로 적은 문서 20 칸 실험 — 형식·계산 엔진 번호별)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenRecalc {
    /// 저장된 값 그대로 (휘발성 함수도, 값이 없는 수식도 — XML·xlsx 의 엔진 번호가 한셀 2024 의 904 이상)
    #[default]
    None,
    /// 휘발성 함수(NOW·TODAY·RAND·RANDBETWEEN·OFFSET·INDIRECT)를 쓰거나 그런 칸에 기대는 칸만 (레거시·xls 의 904)
    Volatile,
    /// 모두 (엔진 번호가 없거나 낮은 XML·레거시, ODS)
    Full,
}

/// 한셀 2024 계산 엔진 번호 (`calcPr/@hs:hclCalcId`, 레거시 0xA101)
pub const HCELL_CALC_ID: u32 = 904;

/// 문서 바이트와 읽은 모델로 한셀이 열 때 다시 계산하는 범위를 정한다.
/// XML·xlsx: 엔진 번호(`hs:hclCalcId`, 없으면 Excel 의 `calcId`)가 904 보다 작거나 없으면 모두, 아니면 하지 않는다
/// (904 → 하지 않음, 903 → 모두, Excel 191029 → 하지 않음, 둘 다 없음 → 모두). 레거시·xls: 0xA101 이 904 이상이면
/// 휘발성 칸만, 아니면 모두. ODS: 모두. 바이너리 .cell 은 한셀 COM 이 열지 못해 재지 못했다 — 저장된 값 그대로
pub fn open_recalc(bytes: &[u8], wb: &Workbook) -> OpenRecalc {
    let id = wb.hancom.calc_id.or_else(|| {
        wb.calc_attrs
            .iter()
            .find(|(k, _)| k == "calcId")
            .and_then(|(_, v)| v.trim().parse().ok())
    });
    let current = id.is_some_and(|i| i >= HCELL_CALC_ID);
    match crate::detect(bytes) {
        crate::FileKind::Cfb => {
            let encrypted = crate::container::list_entries(bytes).is_ok_and(|e| {
                e.iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("/EncryptionInfo"))
            });
            if encrypted {
                // 암호화된 OOXML: 안은 XML 패키지
                if current {
                    OpenRecalc::None
                } else {
                    OpenRecalc::Full
                }
            } else if wb.hancom.calc_id.is_some_and(|i| i >= HCELL_CALC_ID) {
                OpenRecalc::Volatile
            } else {
                OpenRecalc::Full
            }
        }
        crate::FileKind::Zip if crate::ods::is_ods(bytes) => OpenRecalc::Full,
        crate::FileKind::Zip => {
            let binary = crate::Package::from_bytes(bytes.to_vec())
                .is_ok_and(|p| p.has_part("xl/workbook.bin"));
            if binary || current {
                OpenRecalc::None
            } else {
                OpenRecalc::Full
            }
        }
        crate::FileKind::Unknown => OpenRecalc::None,
    }
}

/// [`OpenRecalc`] 대로 칸 값을 다시 계산한다. 순환 참조·구현하지 않은 함수·해석하지 못한 수식의 칸은 저장된 값
/// (한셀 CalculateFull 은 순환 참조에서 멈춘다)
pub fn apply_open_recalc(wb: &mut Workbook, mode: OpenRecalc) {
    if mode == OpenRecalc::None {
        return;
    }
    use crate::calc::CellStatus;
    let (results, _) = crate::calc::evaluate(wb, &crate::calc::CalcOptions::default());
    let mut new: std::collections::HashMap<(usize, u32, u32), crate::model::CellValue> =
        std::collections::HashMap::new();
    for r in results {
        let take = match r.status {
            CellStatus::Volatile => true,
            CellStatus::Computed => mode == OpenRecalc::Full,
            _ => false,
        };
        if take {
            new.insert((r.sheet, r.cell.row, r.cell.col), r.value);
        }
    }
    for (si, sheet) in wb.sheets.iter_mut().enumerate() {
        for cell in &mut sheet.cells {
            if let Some(v) = new.remove(&(si, cell.r.row, cell.r.col)) {
                cell.value = v;
            }
        }
    }
}

/// PDF 페이지 수 기본 한도
pub const MAX_PAGES: usize = 5000;

/// 장치 단위 (600 dpi) 1pt
const DEV_PT: f64 = 25.0 / 3.0;

/// 한셀 PDF 의 용지: 그리기는 참 용지 크기의 600 dpi 장치 단위(Letter 5100×6600), PDF 용지는 mm 정수로
/// 반올림한 크기를 pt 정수로 자른 것(Letter 215.9×279.4 → 216×279 → MediaBox 612×790). 내용은 둘의 비율로
/// 줄여 넣는다 (Letter 세로 790/(6600×0.12) — 한셀 PDF 의 `cm`)
pub fn page_size(s: &Settings) -> PageSize {
    let mm = |pt: f64| (pt / 72.0 * 25.4).round();
    let (w, h) = (mm(s.paper.width), mm(s.paper.height));
    PageSize {
        width: (w / 25.4 * 72.0).floor(),
        height: (h / 25.4 * 72.0).floor(),
        dev_w: (s.paper.width / 72.0 * 600.0).round(),
        dev_h: (s.paper.height / 72.0 * 600.0).round(),
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
    frame_with(sp, page, size, None)
}

/// 행·열 머리글 크기 (인쇄 px): 너비 = 글꼴 0 숫자 폭 8 글자, 높이 = 기본 행 높이, 둘 다 배율 (한셀 PDF 머리글 실험)
fn heading_px(wb: &Workbook, z: u32) -> (f64, f64) {
    let m = crate::render::book_metrics(wb);
    let w = (8.0 * m.k * z as f64 / 100.0).round();
    let h = (crate::render::row_px(m.row_pt) as f64 * z as f64 / 100.0).round();
    (w, h)
}

/// [`frame`] + 행·열 머리글 (인쇄 px 너비, 높이)
pub fn frame_with(
    sp: &SheetPrint,
    page: &Page,
    size: PageSize,
    heading: Option<(f64, f64)>,
) -> PageFrame {
    let s = &sp.settings;
    let x0 = (s.left * DEV_PT).round();
    let y0 = (s.top * DEV_PT).round();
    let (hw, hh) = heading.unwrap_or((0.0, 0.0));
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
    let w: f64 = (cols.iter().map(|c| c.1).sum::<f64>() + hw) * 6.25;
    let h: f64 = (rows.iter().map(|r| r.1).sum::<f64>() + hh) * 6.25;
    let avail_w = size.dev_w - x0 - (s.right * DEV_PT).round();
    let avail_h = size.dev_h - y0 - (s.bottom * DEV_PT).round();
    // 가운데 맞춤: 화면 px 로 (쓸 자리 px 를 내림, 남는 px 의 반을 내림) 옮긴다 — Holiday_Budget·center 실험
    let center = |avail: f64, content: f64| {
        let px = ((avail / 6.25).floor() - content / 6.25) / 2.0;
        (px.floor().max(0.0) * 6.25 + 0.5).floor()
    };
    let dx = if s.center_h { center(avail_w, w) } else { 0.0 };
    let dy = if s.center_v { center(avail_h, h) } else { 0.0 };
    let z = if sp.tall { 100 } else { sp.print_scale };
    PageFrame {
        cols: bounds(x0 + dx + (hw * 6.25).round(), &cols),
        col_x100: {
            let mut acc = 0.0;
            cols.iter()
                .map(|&(c, _)| {
                    let at = acc;
                    acc += sp.col_px.get(c as usize).copied().unwrap_or(0) as f64;
                    (c, at)
                })
                .collect()
        },
        origin_x: x0 + dx + (hw * 6.25).round(),
        rows: bounds(y0 + dy + (hh * 6.25).round(), &rows),
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

/// 행·열 머리글 (가는 선 상자, 가운데 글자: 열 이름·행 번호)
fn heading_ops(
    fr: &PageFrame,
    hw: f64,
    hh: f64,
    font: &(String, f64),
    fonts: &FontSet,
    ops: &mut Vec<Op>,
) {
    let (Some(c0), Some(r0)) = (fr.cols.first(), fr.rows.first()) else {
        return;
    };
    let (x_left, y_top) = (c0.1 - hw, r0.1 - hh);
    let (x_right, y_bottom) = (
        fr.cols.last().map_or(c0.2, |c| c.2),
        fr.rows.last().map_or(r0.2, |r| r.2),
    );
    let key = FontKey {
        name: font.0.clone(),
        bold: false,
        italic: false,
    };
    let size = fr.font_size(font.1);
    let line = |ops: &mut Vec<Op>, x0: f64, y0: f64, x1: f64, y1: f64| {
        ops.push(Op::Line {
            x0: x0 - 6.0,
            y0: y0 - 6.0,
            x1: x1 - 6.0,
            y1: y1 - 6.0,
            width: 6.0,
            color: [0, 0, 0],
            dash: Vec::new(),
        })
    };
    let text = |ops: &mut Vec<Op>, s: String, x0: f64, x1: f64, y0: f64, y1: f64| {
        let w = fonts.width(&key, size, &s);
        ops.push(Op::Text {
            x: (x0 + x1) / 2.0 - w / 2.0 - 8.0 * fr.px / 6.25,
            y: (y0 + y1) / 2.0 + 0.386 * size - 11.5 * fr.zoom as f64 / 99.0,
            size,
            font: key.clone(),
            color: [0, 0, 0],
            text: s,
            underline: false,
            strike: false,
            stretch: 1.0,
            angle: 0.0,
        })
    };
    // 바깥·안 선
    line(ops, x_left, y_top, x_right, y_top);
    line(ops, x_left, y_top, x_left, y_bottom);
    line(ops, x_left, r0.1, x_right, r0.1);
    line(ops, c0.1, y_top, c0.1, y_bottom);
    for c in &fr.cols {
        line(ops, c.2, y_top, c.2, r0.1);
        text(
            ops,
            crate::model::CellRef::col_name(c.0),
            c.1,
            c.2,
            y_top,
            r0.1,
        );
    }
    for r in &fr.rows {
        line(ops, x_left, r.2, c0.1, r.2);
        text(ops, (r.0 + 1).to_string(), x_left, c0.1, r.1, r.2);
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
        // 기준선: 머리글은 여백 + 가장 큰 글꼴의 오름, 바닥글은 여백 − 내림 − 줄 높이표의 남는 반 (맑은 고딕 11pt
        // 바닥글이 내림보다 장치 8 위, 8pt 는 0 — 첫·짝수 머리글 실험)
        let (asc, desc, lead) =
            items
                .iter()
                .fold((0.0f64, 0.0f64, 0.0f64), |(a, d, l), (key, sz, r)| {
                    let (fa, fd) = fonts.ascent_descent(key);
                    let (fa, fd) = (fa * sz / 1000.0, fd * sz / 1000.0);
                    let line = crate::render::FontMetrics::new(
                        Some(&key.name),
                        Some(r.size.unwrap_or(default_font.1)),
                        key.bold,
                    )
                    .row_pt
                        * DEV_PT;
                    (a.max(fa), d.max(fd), l.max((line - fa - fd) / 2.0))
                });
        let y = if header {
            edge + asc
        } else {
            edge - desc - lead.max(0.0)
        };
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
                angle: 0.0,
            });
            x += w;
        }
    }
}

/// 통합 문서를 PDF 로 (보이는 시트들, 페이지 순서대로)
pub fn workbook_pdf(wb: &Workbook, db: &FontDb, opts: &PdfOptions) -> Vec<u8> {
    let calculated;
    let wb = if opts.recalc == OpenRecalc::None {
        wb
    } else {
        let mut w = wb.clone();
        apply_open_recalc(&mut w, opts.recalc);
        calculated = w;
        &calculated
    };
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
    // 인쇄 차례에서 한 페이지짜리 시트가 이미 나왔는가 (배율 100 의 99 는 첫째에만)
    let mut single_seen = false;
    for si in sheets {
        let Some(sheet) = wb.sheets.get(si) else {
            continue;
        };
        let sp = paginate::sheet_print_with(wb, si, !single_seen);
        single_seen |= sp.pages.len() == 1;
        let size = page_size(&sp.settings);
        let po = sheet.page.print_options.as_ref();
        let setup = sheet.page.setup.as_ref();
        let hfd = sheet.page.header_footer.as_ref();
        let total = sp.pages.len() as u32;
        let first_no = sp.settings.first_page_number.unwrap_or(1);
        // 그릴 범위 (모든 페이지의 행·열)
        let max_r = sp
            .pages
            .iter()
            .flat_map(|p| p.rows.iter().chain(&p.title_rows))
            .copied()
            .max()
            .unwrap_or(0);
        let max_c = sp
            .pages
            .iter()
            .flat_map(|p| p.cols.iter().chain(&p.title_cols))
            .copied()
            .max()
            .unwrap_or(0);
        let painter = SheetPainter::new(wb, sheet, max_r, max_c);
        for (pi, page) in sp.pages.iter().enumerate() {
            // 아주 큰 범위(마지막 칸에 값이 있는 문서 — 80만 페이지)는 앞 페이지들만
            if pages.len() >= opts.max_pages.unwrap_or(MAX_PAGES) {
                break;
            }
            let headings = po.is_some_and(|o| o.headings);
            let z = if sp.tall { 100 } else { sp.print_scale };
            let hpx = headings.then(|| heading_px(wb, z));
            let mut fr = frame_with(&sp, page, size, hpx);
            fr.gridlines = po.is_some_and(|o| o.grid_lines);
            fr.black_and_white = opts.quality_settings && setup.is_some_and(|s| s.black_and_white);
            fr.draft = opts.quality_settings && setup.is_some_and(|s| s.draft);
            fr.errors = match setup.and_then(|s| s.errors.as_deref()) {
                Some("blank") => ErrorPrint::Blank,
                Some("dash") => ErrorPrint::Dash,
                Some("NA") => ErrorPrint::Na,
                _ => ErrorPrint::Displayed,
            };
            let mut ops = painter.page_ops(&fr, &fonts);
            ops.extend(objects_ops(wb, sheet, &fr, &fonts));
            if let Some((hw, hh)) = hpx {
                heading_ops(&fr, hw * 6.25, hh * 6.25, &default_font, &fonts, &mut ops);
            }
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
