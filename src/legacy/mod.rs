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
pub mod lossy;
pub(crate) mod pivot;
pub(crate) mod pivotw;
pub mod props;
pub mod reader;
pub mod recdiff;
mod sheetx;
pub(crate) mod vtbuild;
pub(crate) mod vtchart;
pub(crate) mod vtchart_map;
pub(crate) mod vtwrite;
pub mod writer;

pub use writer::{Report, WriteOptions, write_with_options, write_with_report};

/// 통합 문서를 레거시 OLE2 바이트로 쓴다 (한셀 `xlCell8`, -2005 와 같은 꼴).
pub fn write(wb: &Workbook) -> Result<Vec<u8>> {
    writer::write(wb)
}

use crate::container;
use crate::error::{Error, Result};
use crate::model::Workbook;

/// 진단: 레거시 파일의 한컴 차트 레코드(0x7EF0)마다 Vt 아카이브를 `경로 = 값` 줄로 (리버스 엔지니어링용)
pub fn chart_dumps(bytes: &[u8]) -> Result<Vec<String>> {
    chart_dumps_with(bytes, false)
}

/// [`chart_dumps`] — `offsets` 면 레코드의 스트림 위치와 값마다 레코드 안 자리를 붙인다 (값을 바꿔 보는 실험용)
pub fn chart_dumps_with(bytes: &[u8], offsets: bool) -> Result<Vec<String>> {
    let stream = container::read_entry(bytes, "/Workbook")?;
    let recs = biff::records(&stream);
    // 원본 범위 수식의 ixti 는 시트 순서 번호 (cell-legacy-chart.md 2절): BOUNDSHEET 이름
    let names: Vec<String> = recs
        .iter()
        .filter(|r| r.rtype == 0x0085 && r.data.len() > 8)
        .map(|r| {
            // 이름은 플래그와 상관없이 UTF-16 (reader 와 같다)
            let d = &r.data[6..];
            let cch = d[0] as usize;
            crate::text::utf16le(d.get(2..2 + cch * 2).unwrap_or(&[]))
        })
        .collect();
    // 항목·계열 수식의 ixti 는 XTI 번호: 내부 SUPBOOK(`시트 수, 01 04`)을 가리키는 시트 하나짜리 XTI → 시트 번호
    let internal: Vec<bool> = recs
        .iter()
        .filter(|r| r.rtype == 0x01AE)
        .map(|r| r.data.get(2..4) == Some(&[0x01, 0x04][..]))
        .collect();
    let xti: Vec<Option<u16>> = recs
        .iter()
        .find(|r| r.rtype == 0x0017)
        .map(|r| {
            let (xtis, _) = r.data.get(2..).unwrap_or(&[]).as_chunks::<6>();
            xtis.iter()
                .map(|x| {
                    let v = |k: usize| u16::from_le_bytes([x[k], x[k + 1]]);
                    (internal.get(v(0) as usize) == Some(&true) && v(2) == v(4)).then_some(v(2))
                })
                .collect()
        })
        .unwrap_or_default();
    let sheets = vtchart::Sheets {
        names: &names,
        xti: Some(&xti),
    };
    Ok(recs
        .iter()
        .filter(|r| r.rtype == vtchart::RECORD_TYPE)
        .map(|r| match vtchart::parse_with(r.data, sheets) {
            Ok(rec) if offsets => format!(
                "record = /Workbook @0x{:x} (내용 @0x{:x})
{}",
                r.offset,
                r.offset + 4,
                vtchart::dump_with(&rec, true)
            ),
            Ok(rec) => vtchart::dump(&rec),
            Err(e) => format!("읽기 실패: {e}\n"),
        })
        .collect())
}

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
    // 피벗: 캐시 스트림(_SX_DB_CUR)과 시트의 피벗 레코드 → XML 피벗 파트
    pivot::read(bytes, &stream, &mut wb);
    // 문서 속성: 목록은 이름의 제어 문자를  로 표기한다 (container::display_path)
    if has("/\\x05SummaryInformation") {
        let s = container::read_entry(bytes, "/\u{5}SummaryInformation")?;
        (wb.properties, wb.total_time) = props::parse_summary(&s);
    }
    // 한컴 매크로 프로젝트: XML 의 `xl/vbsProject.bin` 안 스트림과 바이트가 같다 (패키지 파트 실험)
    if has(crate::ns::VBS_PROJECT_STREAM) {
        wb.hancom.vbs_project = Some(container::read_entry(bytes, crate::ns::VBS_PROJECT_STREAM)?);
    }
    Ok(wb)
}
