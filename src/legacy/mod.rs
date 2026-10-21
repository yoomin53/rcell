// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 바이너리 `.cell` (OLE2 복합 문서 + BIFF 계열 레코드).
//!
//! 한셀 2024 의 저장 형식 `xlCell8`(-2005)로 만들어지는 파일이다. BIFF8 을 바탕으로
//! 행 번호 32비트화, UTF-16 문자열, 글꼴/셀 서식 확장 영역, 한셀 전용 레코드를 더했다.
//! 형식 분석: `docs/format/cell-legacy.md`.

mod art;
pub mod biff;
mod cf;
mod dxf;
pub mod encode;
pub mod escher;
pub mod formula;
pub mod ftab;
pub mod props;
pub mod reader;
pub mod recdiff;
mod sheetx;
pub(crate) mod vtchart;
pub(crate) mod vtchart_map;
pub mod writer;

pub use writer::{Report, write_with_report};

/// 통합 문서를 레거시 OLE2 바이트로 쓴다 (한셀 `xlCell8`, -2005 와 같은 꼴).
pub fn write(wb: &Workbook) -> Result<Vec<u8>> {
    writer::write(wb)
}

use crate::container;
use crate::error::{Error, Result};
use crate::model::Workbook;

/// OLE2 바이트에서 레거시 통합 문서를 읽는다.
pub fn read(bytes: &[u8]) -> Result<Workbook> {
    let entries = container::list_entries(bytes)?;
    let has = |n: &str| entries.iter().any(|(e, _)| e.eq_ignore_ascii_case(n));
    if has("/EncryptionInfo") || has("/EncryptedPackage") {
        return Err(Error::Encrypted);
    }
    if !has("/Workbook") {
        return Err(Error::MissingPart("/Workbook".into()));
    }
    let stream = container::read_entry(bytes, "/Workbook")?;
    let mut wb = reader::read_workbook_stream(&stream)?;
    // 문서 속성: 목록은 이름의 제어 문자를  로 표기한다 (container::display_path)
    if has("/\\x05SummaryInformation") {
        let s = container::read_entry(bytes, "/\u{5}SummaryInformation")?;
        wb.properties = props::parse_summary(&s);
    }
    Ok(wb)
}
