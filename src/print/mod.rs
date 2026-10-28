// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 인쇄 배치와 PDF (한셀 인쇄·PDF 저장과 같은 페이지, docs/format/print.md).
//!
//! - [`sheet_print`]: 페이지 나누기(자동·수동, 인쇄 영역, 반복 행·열, 숨김, 페이지 순서)와 인쇄 배율 —
//!   한셀 COM `HPageBreaks`·`VPageBreaks`·`Pages.Count` 와 같게.
//! - [`paper`]: 용지 번호 → 크기.

mod paginate;
pub mod paper;

pub use paginate::{Area, Page, Scaling, Settings, SheetPrint, settings, sheet_print, used_end};
