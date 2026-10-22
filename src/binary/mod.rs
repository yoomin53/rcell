// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 바이너리 OOXML `.cell` (한셀 저장 형식 `xlCellb`, -2007).
//!
//! ZIP 패키지 구조는 XML `.cell` 과 같고, 파트 내용이 BIFF12 레코드다 ([MS-XLSB]).
//! 형식 분석: `docs/format/cell-binary.md`.

pub mod biff12;
pub mod encrypt;
pub mod formula;
pub mod parity;
pub mod pivot;
pub mod reader;
pub mod writer;
