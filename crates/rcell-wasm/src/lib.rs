// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 브라우저용 WebAssembly 인터페이스 (wasm-bindgen 없이 C ABI).
//!
//! JS 쪽 사용법 (`web/rcell.js`):
//! 1. `rcell_alloc(n)` 으로 메모리를 받아 파일 바이트를 복사한다.
//! 2. `rcell_render(ptr, n)` 등을 부르면 결과를 `(포인터 << 32) | 길이` 로 돌려준다.
//! 3. 결과를 읽은 뒤 `rcell_free(포인터, 길이)` 로 돌려준다.
//!
//! 결과 바이트의 첫 글자: `O` = 성공, `E` = 오류 메시지 (나머지는 UTF-8 또는 파일 바이트).
//!
//! 편집기: `rcell_doc_open` 으로 문서를 열어 번호를 받고, `rcell_doc_edit` (편집 명령 JSON — `rcell::edit::Edit`),
//! `rcell_doc_render`, `rcell_doc_cell`, `rcell_doc_undo` … 로 다룬 뒤 `rcell_doc_save` 로 저장한다. 편집마다
//! 수식을 다시 계산하고, 되돌리기를 위해 편집 앞 문서를 남긴다 (그림 같은 이진 파트는 편집하지 않으므로 빼고).

use std::alloc::{Layout, alloc, dealloc};
use std::cell::RefCell;

use rcell::Workbook;
use rcell::edit::{self, Clip};
use rcell::model::{CellRange, CellRef, CellValue, RawPart};
use serde_json::{Value, json};

/// JS 가 입력을 쓸 메모리.
#[unsafe(no_mangle)]
pub extern "C" fn rcell_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::dangling().as_ptr();
    }
    unsafe { alloc(Layout::from_size_align(len, 1).unwrap()) }
}

/// `rcell_alloc` 이나 결과로 받은 메모리를 돌려준다.
///
/// # Safety
/// `ptr` 과 `len` 은 이 모듈이 준 값 그대로여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_free(ptr: *mut u8, len: usize) {
    if len != 0 {
        unsafe { dealloc(ptr, Layout::from_size_align(len, 1).unwrap()) }
    }
}

fn out(tag: u8, body: &[u8]) -> u64 {
    let len = body.len() + 1;
    let ptr = rcell_alloc(len);
    unsafe {
        *ptr = tag;
        std::ptr::copy_nonoverlapping(body.as_ptr(), ptr.add(1), body.len());
    }
    ((ptr as u64) << 32) | len as u64
}

fn input(ptr: *const u8, len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

fn run(r: rcell::Result<Vec<u8>>) -> u64 {
    match r {
        Ok(b) => out(b'O', &b),
        Err(e) => out(b'E', e.to_string().as_bytes()),
    }
}

fn run_json(r: rcell::Result<Value>) -> u64 {
    run(r.map(|v| serde_json::to_vec(&v).unwrap_or_default()))
}

/// 파일 바이트 → 뷰어 HTML 조각 (시트 탭 포함).
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_render(ptr: *const u8, len: usize) -> u64 {
    let bytes = input(ptr, len);
    // 스파크라인처럼 모델에 없는 그리기 정보는 원본 패키지에서 다시 읽는다 (XML 문서만)
    let extras = rcell::render::Extras::from_bytes(&bytes);
    run(rcell::from_bytes(bytes).map(|wb| {
        rcell::render::workbook_fragment_with(&wb, Default::default(), &extras).into_bytes()
    }))
}

/// 파일 바이트 → 문서 요약 JSON (셀 목록 제외).
///
/// # Safety
/// `rcell_render` 와 같다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_info(ptr: *const u8, len: usize) -> u64 {
    run(rcell::from_bytes(input(ptr, len)).map(|mut wb| {
        for s in &mut wb.sheets {
            s.cells.clear();
        }
        wb.shared_strings.clear();
        serde_json::to_vec(&wb).unwrap_or_default()
    }))
}

/// 파일 바이트 → XML `.cell` 바이트 (레거시/바이너리 → 최신 형식 변환).
///
/// # Safety
/// `rcell_render` 와 같다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_to_cell(ptr: *const u8, len: usize) -> u64 {
    run(rcell::from_bytes(input(ptr, len))
        .and_then(|wb| rcell::writer::write(&wb, rcell::writer::Flavor::Cell)))
}

/// 뷰어 CSS (렌더러와 같은 소스).
#[unsafe(no_mangle)]
pub extern "C" fn rcell_css() -> u64 {
    out(b'O', rcell::render::CSS.as_bytes())
}

/// 파일 바이트 → `.xlsx` 바이트.
///
/// # Safety
/// `rcell_render` 와 같다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_to_xlsx(ptr: *const u8, len: usize) -> u64 {
    run(rcell::from_bytes(input(ptr, len))
        .and_then(|wb| rcell::writer::write(&wb, rcell::writer::Flavor::Xlsx)))
}

// ---------------------------------------------------------------------------
// 편집기
// ---------------------------------------------------------------------------

/// 되돌리기 단계 수
const UNDO_LIMIT: usize = 100;

struct Doc {
    wb: Workbook,
    extras: rcell::render::Extras,
    undo: Vec<Workbook>,
    redo: Vec<Workbook>,
    /// 복사·잘라 낸 범위 (잘라 냈으면 원본 시트와 범위)
    clip: Option<(Clip, Option<(usize, CellRange)>)>,
}

thread_local! {
    static DOCS: RefCell<Vec<Option<Doc>>> = const { RefCell::new(Vec::new()) };
}

fn err(m: impl Into<String>) -> rcell::Error {
    rcell::Error::Edit(m.into())
}

/// 편집하지 않는 이진 파트 (그림·매크로)는 되돌리기 사본에서 뺀다
fn is_binary(p: &RawPart) -> bool {
    !(p.name.ends_with(".xml") || p.name.ends_with(".rels") || p.name.ends_with(".vml"))
}

fn snapshot(wb: &mut Workbook) -> Workbook {
    let parts = std::mem::take(&mut wb.raw_parts);
    let mut s = wb.clone();
    s.raw_parts = parts.iter().filter(|p| !is_binary(p)).cloned().collect();
    wb.raw_parts = parts;
    s
}

/// 되돌리기 사본을 되살린다 (빼 둔 이진 파트는 지금 문서의 것을 쓴다)
fn restore(mut snap: Workbook, current: &Workbook) -> Workbook {
    snap.raw_parts
        .extend(current.raw_parts.iter().filter(|p| is_binary(p)).cloned());
    snap
}

fn with_doc<T>(id: u32, f: impl FnOnce(&mut Doc) -> rcell::Result<T>) -> rcell::Result<T> {
    DOCS.with(|d| {
        let mut docs = d.borrow_mut();
        let doc = docs
            .get_mut(id as usize)
            .and_then(Option::as_mut)
            .ok_or_else(|| err("문서가 열려 있지 않습니다"))?;
        f(doc)
    })
}

fn info_json(doc: &Doc) -> Value {
    let wb = &doc.wb;
    let active = wb
        .sheets
        .iter()
        .position(|s| s.view.tab_selected)
        .or_else(|| {
            wb.sheets
                .iter()
                .position(|s| s.state == rcell::model::SheetState::Visible)
        })
        .unwrap_or(0);
    json!({
        "sheets": wb.sheets.iter().enumerate().map(|(i, s)| json!({
            "index": i,
            "name": s.name,
            "visible": s.state == rcell::model::SheetState::Visible,
            "chart": s.kind != rcell::model::SheetKind::Worksheet,
        })).collect::<Vec<_>>(),
        "active": active,
        "canUndo": !doc.undo.is_empty(),
        "canRedo": !doc.redo.is_empty(),
        "application": wb.application,
        "fonts": wb.styles.fonts.first().and_then(|f| f.name.clone()),
        // 통합 문서 구조 보호: 시트 추가·삭제·이름 바꾸기·옮기기를 끈다
        "structure": wb.protection.as_ref().is_some_and(|p| p.lock_structure),
    })
}

thread_local! {
    /// 브라우저의 시간대 (분, UTC 기준) — NOW()·TODAY()
    static TZ: std::cell::Cell<i32> = const { std::cell::Cell::new(9 * 60) };
}

/// 지금 시각과 시간대를 알린다 (`Date.now()`, `-new Date().getTimezoneOffset()`). wasm32 에는 시계가 없어
/// 편집하기 전에 부른다 (NOW()·TODAY()·RAND() 의 씨앗)
#[unsafe(no_mangle)]
pub extern "C" fn rcell_set_clock(unix_ms: f64, utc_offset_min: i32) {
    rcell::calc::set_clock(Some(unix_ms / 1000.0));
    TZ.with(|t| t.set(utc_offset_min));
}

fn recalc(wb: &mut Workbook) {
    let opts = rcell::calc::CalcOptions {
        utc_offset_minutes: TZ.with(|t| t.get()),
        ..Default::default()
    };
    rcell::calc::recalculate_with(wb, &opts);
}

/// 파일 바이트 → 편집할 문서. `{"doc": 번호, "info": …}`
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_doc_open(ptr: *const u8, len: usize) -> u64 {
    let bytes = input(ptr, len);
    let extras = rcell::render::Extras::from_bytes(&bytes);
    run_json(rcell::from_bytes(bytes).map(|wb| {
        let doc = Doc {
            wb,
            extras,
            undo: Vec::new(),
            redo: Vec::new(),
            clip: None,
        };
        let info = info_json(&doc);
        let id = DOCS.with(|d| {
            let mut docs = d.borrow_mut();
            match docs.iter().position(Option::is_none) {
                Some(i) => {
                    docs[i] = Some(doc);
                    i
                }
                None => {
                    docs.push(Some(doc));
                    docs.len() - 1
                }
            }
        });
        json!({ "doc": id, "info": info })
    }))
}

/// 빈 새 문서 (시트 하나, 한셀 기본 글꼴)
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_new() -> u64 {
    let wb = edit::new_workbook();
    let doc = Doc {
        wb,
        extras: Default::default(),
        undo: Vec::new(),
        redo: Vec::new(),
        clip: None,
    };
    let info = info_json(&doc);
    let id = DOCS.with(|d| {
        let mut docs = d.borrow_mut();
        docs.push(Some(doc));
        docs.len() - 1
    });
    run_json(Ok(json!({ "doc": id, "info": info })))
}

/// 문서를 닫는다
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_close(id: u32) {
    DOCS.with(|d| {
        if let Some(slot) = d.borrow_mut().get_mut(id as usize) {
            *slot = None;
        }
    });
}

/// 시트 목록·활성 시트·되돌리기 여부
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_info(id: u32) -> u64 {
    run_json(with_doc(id, |d| Ok(info_json(d))))
}

/// 시트 탭을 고른다 (저장할 때 선택된 시트 — 되돌리기에 넣지 않는다)
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_select(id: u32, sheet: u32) -> u64 {
    run_json(with_doc(id, |d| {
        if (sheet as usize) < d.wb.sheets.len() {
            edit::select_sheet(&mut d.wb, sheet as usize);
        }
        Ok(info_json(d))
    }))
}

/// 시트 하나의 HTML 과 칸 배치: `{"html": …, "grid": {headerW…}}`. `min_rows`·`min_cols` 까지는 그린다
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_render(id: u32, sheet: u32, min_rows: u32, min_cols: u32) -> u64 {
    run_json(with_doc(id, |d| {
        let s =
            d.wb.sheets
                .get(sheet as usize)
                .ok_or_else(|| err("시트가 없습니다"))?;
        let opts = rcell::render::Options {
            min_cells: (min_rows, min_cols),
            max_rows: 5000.max(min_rows + 1),
            max_cols: 300.max(min_cols + 1),
            ..Default::default()
        };
        let (html, grid) = rcell::render::sheet_table_grid(&d.wb, s, opts, &d.extras);
        let merges: Vec<[u32; 4]> = s
            .merges
            .iter()
            .map(|m| [m.start.row, m.start.col, m.end.row, m.end.col])
            .collect();
        let used = s.used_range().map(|r| [r.end.row, r.end.col]);
        Ok(json!({
            "html": html,
            "grid": grid,
            "merges": merges,
            "used": used,
            "protected": s.protection.as_ref().is_some_and(|p| p.flag("sheet")),
            // 보호된 시트에서 막힌 동작 — 한셀 화면처럼 단추를 끈다
            "blocked": s
                .protection
                .as_ref()
                .filter(|p| p.flag("sheet"))
                .map(|p| {
                    rcell::model::SHEET_PROTECTION_FLAGS
                        .iter()
                        .map(|(k, _)| *k)
                        .filter(|k| *k != "sheet" && p.flag(k))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        }))
    }))
}

fn style_json(wb: &Workbook, style: u32) -> Value {
    let st = &wb.styles;
    let xf = st.xf(style).cloned().unwrap_or_default();
    let font = st
        .fonts
        .get(xf.font_id as usize)
        .cloned()
        .unwrap_or_default();
    let fill = st
        .fills
        .get(xf.fill_id as usize)
        .cloned()
        .unwrap_or_default();
    let theme = wb.theme.clone().unwrap_or_default();
    let color = |c: Option<&rcell::model::Color>| -> Option<String> {
        c.and_then(|c| rcell::theme::resolve(c, &theme))
    };
    let al = xf.alignment.clone().unwrap_or_default();
    json!({
        "bold": font.bold,
        "italic": font.italic,
        "underline": font.underline.is_some(),
        "strike": font.strike,
        "fontName": font.name,
        "fontSize": font.size,
        "fontColor": color(font.color.as_ref()),
        "fill": if fill.pattern_type.as_deref() == Some("solid") { color(fill.fg_color.as_ref()) } else { None },
        "hAlign": al.horizontal.unwrap_or_else(|| "general".into()),
        "vAlign": al.vertical.unwrap_or_else(|| "bottom".into()),
        "wrap": al.wrap_text,
        "numFmt": rcell::numfmt::code_for(st, xf.num_fmt_id),
    })
}

/// 칸 하나: 수식 입력줄 글, 화면 글, 서식, 병합 범위
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_cell(id: u32, sheet: u32, row: u32, col: u32) -> u64 {
    run_json(with_doc(id, |d| {
        let si = sheet as usize;
        let s = d.wb.sheets.get(si).ok_or_else(|| err("시트가 없습니다"))?;
        let r = CellRef { row, col };
        let style = edit::style_at(s, r);
        let value = s.cell(r).map(|c| c.value.clone()).unwrap_or_default();
        let code =
            d.wb.styles
                .xf(style)
                .map(|x| rcell::numfmt::code_for(&d.wb.styles, x.num_fmt_id))
                .unwrap_or_else(|| "General".into());
        let shown = rcell::numfmt::format(&value, &code, d.wb.date1904).text;
        let merge = s
            .merges
            .iter()
            .find(|m| m.contains(r))
            .map(|m| [m.start.row, m.start.col, m.end.row, m.end.col]);
        Ok(json!({
            "a1": r.to_string(),
            "input": edit::input_text(&d.wb, si, r),
            "display": shown,
            "style": style_json(&d.wb, style),
            "merge": merge,
        }))
    }))
}

/// 편집하고 (실패하면 되돌리고) 다시 계산한다
fn apply_edit(
    d: &mut Doc,
    f: impl FnOnce(&mut Workbook) -> rcell::Result<usize>,
) -> rcell::Result<Value> {
    let before = snapshot(&mut d.wb);
    match f(&mut d.wb) {
        Ok(sheet) => {
            recalc(&mut d.wb);
            d.undo.push(before);
            if d.undo.len() > UNDO_LIMIT {
                d.undo.remove(0);
            }
            d.redo.clear();
            let mut info = info_json(d);
            info["sheet"] = json!(sheet);
            Ok(info)
        }
        Err(e) => {
            d.wb = restore(before, &d.wb);
            Err(e)
        }
    }
}

/// 편집 명령 하나 (JSON — `rcell::edit::Edit`). 결과: 문서 정보 + `"sheet"` (바뀐 시트)
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_doc_edit(id: u32, ptr: *const u8, len: usize) -> u64 {
    let bytes = input(ptr, len);
    run_json(with_doc(id, |d| {
        let e: edit::Edit = serde_json::from_slice(&bytes)
            .map_err(|e| err(format!("편집 명령을 읽지 못했습니다: {e}")))?;
        apply_edit(d, |wb| edit::apply_ui(wb, &e))
    }))
}

/// 되돌리기 (`redo` 가 0 이 아니면 다시 실행)
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_undo(id: u32, redo: u32) -> u64 {
    run_json(with_doc(id, |d| {
        let (from, to) = if redo != 0 {
            (&mut d.redo, &mut d.undo)
        } else {
            (&mut d.undo, &mut d.redo)
        };
        let snap = from.pop().ok_or_else(|| err("되돌릴 것이 없습니다"))?;
        to.push(snapshot(&mut d.wb));
        d.wb = restore(snap, &d.wb);
        Ok(info_json(d))
    }))
}

/// 화면에 보이는 글 (TSV 칸 하나 — 탭·줄 바꿈·따옴표가 있으면 따옴표로 감싼다)
fn tsv_field(wb: &Workbook, si: usize, r: CellRef) -> String {
    let s = &wb.sheets[si];
    let Some(c) = s.cell(r) else {
        return String::new();
    };
    let code = wb
        .styles
        .xf(c.style)
        .map(|x| rcell::numfmt::code_for(&wb.styles, x.num_fmt_id))
        .unwrap_or_else(|| "General".into());
    let t = match &c.value {
        CellValue::Empty => String::new(),
        v => rcell::numfmt::format(v, &code, wb.date1904).text,
    };
    if t.contains(['\t', '\n', '"']) {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t
    }
}

/// 범위를 복사(`cut` 이면 잘라 내기)한다. 결과: 클립보드에 넣을 TSV 글
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_copy(
    id: u32,
    sheet: u32,
    r0: u32,
    c0: u32,
    r1: u32,
    c1: u32,
    cut: u32,
) -> u64 {
    run(with_doc(id, |d| {
        let si = sheet as usize;
        let range = CellRange {
            start: CellRef { row: r0, col: c0 },
            end: CellRef { row: r1, col: c1 },
        };
        let clip = edit::copy(&d.wb, si, range)?;
        let mut tsv = String::new();
        for r in r0..=r1.min(r0 + 10_000) {
            let line: Vec<String> = (c0..=c1.min(c0 + 1_000))
                .map(|c| tsv_field(&d.wb, si, CellRef { row: r, col: c }))
                .collect();
            tsv.push_str(&line.join("\t"));
            tsv.push_str("\r\n");
        }
        d.clip = Some((clip, (cut != 0).then_some((si, range))));
        Ok(tsv.into_bytes())
    }))
}

/// 복사한(잘라 낸) 것을 범위에 붙인다. 범위가 복사한 크기의 배수면 되풀이해 채운다
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_paste(id: u32, sheet: u32, r0: u32, c0: u32, r1: u32, c1: u32) -> u64 {
    run_json(with_doc(id, |d| {
        let (clip, cut) = d.clip.clone().ok_or_else(|| err("복사한 것이 없습니다"))?;
        let si = sheet as usize;
        let at = CellRef { row: r0, col: c0 };
        let fill = CellRange {
            start: at,
            end: CellRef { row: r1, col: c1 },
        };
        let mut info = match cut {
            Some((src_sheet, src)) => {
                let v = apply_edit(d, |wb| {
                    edit::move_cells(wb, src_sheet, src, si, at)?;
                    Ok(si)
                })?;
                d.clip = None;
                v
            }
            None => apply_edit(d, |wb| {
                edit::paste(wb, si, at, &clip, Some(fill))?;
                Ok(si)
            })?,
        };
        let (h, w) = match cut {
            Some((_, src)) => (
                src.end.row - src.start.row + 1,
                src.end.col - src.start.col + 1,
            ),
            None => {
                let (fh, fw) = (r1 - r0 + 1, c1 - c0 + 1);
                if fh % clip.rows == 0 && fw % clip.cols == 0 {
                    (fh, fw)
                } else {
                    (clip.rows, clip.cols)
                }
            }
        };
        info["range"] = json!([r0, c0, r0 + h - 1, c0 + w - 1]);
        Ok(info)
    }))
}

/// 글(TSV)을 칸마다 입력해 붙인다 (다른 프로그램에서 복사한 표)
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_doc_paste_text(
    id: u32,
    sheet: u32,
    row: u32,
    col: u32,
    ptr: *const u8,
    len: usize,
) -> u64 {
    let text = String::from_utf8_lossy(&input(ptr, len)).into_owned();
    run_json(with_doc(id, |d| {
        let si = sheet as usize;
        let mut end = CellRef { row, col };
        let mut info = apply_edit(d, |wb| {
            end = edit::paste_text(wb, si, CellRef { row, col }, &text)?.end;
            Ok(si)
        })?;
        info["range"] = json!([row, col, end.row, end.col]);
        Ok(info)
    }))
}

/// Ctrl+방향키로 갈 칸 `{"row":…,"col":…}`
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_jump(id: u32, sheet: u32, row: u32, col: u32, dr: i32, dc: i32) -> u64 {
    run_json(with_doc(id, |d| {
        let s =
            d.wb.sheets
                .get(sheet as usize)
                .ok_or_else(|| err("시트가 없습니다"))?;
        let r = edit::jump(s, CellRef { row, col }, dr.signum(), dc.signum());
        Ok(json!({ "row": r.row, "col": r.col }))
    }))
}

/// 문서를 저장한 바이트: `format` 0 = XML `.cell`, 1 = `.xlsx`, 2 = 바이너리 `.cell`
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_save(id: u32, format: u32) -> u64 {
    run(with_doc(id, |d| match format {
        0 => rcell::writer::write(&d.wb, rcell::writer::Flavor::Cell),
        1 => rcell::writer::write(&d.wb, rcell::writer::Flavor::Xlsx),
        2 => rcell::binary::writer::write(&d.wb, &Default::default()),
        _ => Err(err("저장 형식")),
    }))
}

thread_local! {
    /// PDF 에 넣을 글꼴 (브라우저에는 시스템 글꼴이 없어 JS 가 `rcell_font_add` 로 넣는다)
    static FONTS: RefCell<rcell::print::FontDb> = RefCell::new(rcell::print::FontDb::new());
}

/// PDF 에 쓸 글꼴 파일(TrueType·OpenType·모음)을 더한다. 글꼴이 하나도 없으면 PDF 는 글꼴 이름만 적는다
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_font_add(ptr: *const u8, len: usize) {
    let bytes = input(ptr, len);
    FONTS.with(|f| f.borrow_mut().add_data(bytes));
}

fn pdf_of(wb: &Workbook) -> Vec<u8> {
    FONTS.with(|f| {
        let opts = rcell::print::PdfOptions::default();
        rcell::print::workbook_pdf(wb, &f.borrow(), &opts)
    })
}

/// 파일 바이트 → PDF (한셀 PDF 저장처럼 보이는 시트 모두)
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_pdf(ptr: *const u8, len: usize) -> u64 {
    run(rcell::from_bytes(input(ptr, len)).map(|wb| pdf_of(&wb)))
}

/// 편집 중인 문서 → PDF
#[unsafe(no_mangle)]
pub extern "C" fn rcell_doc_pdf(id: u32) -> u64 {
    run(with_doc(id, |d| Ok(pdf_of(&d.wb))))
}
