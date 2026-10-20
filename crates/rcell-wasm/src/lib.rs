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

use std::alloc::{Layout, alloc, dealloc};

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
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

fn run(r: rcell::Result<Vec<u8>>) -> u64 {
    match r {
        Ok(b) => out(b'O', &b),
        Err(e) => out(b'E', e.to_string().as_bytes()),
    }
}

/// 파일 바이트 → 뷰어 HTML 조각 (시트 탭 포함).
///
/// # Safety
/// `ptr`/`len` 은 `rcell_alloc` 으로 받은 유효한 범위여야 한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rcell_render(ptr: *const u8, len: usize) -> u64 {
    run(rcell::from_bytes(input(ptr, len))
        .map(|wb| rcell::render::workbook_fragment(&wb, Default::default()).into_bytes()))
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
