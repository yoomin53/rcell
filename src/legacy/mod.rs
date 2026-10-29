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
pub mod xls;
mod xls_chart;

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

/// 통합 문서를 Excel 97-2003 `.xls` (BIFF8) 로 쓴다: 레거시로 쓴 `/Workbook` 을 [`xls::to_biff8`] 로 바꾸고 문서
/// 속성과 함께 담는다 (매크로 프로젝트·피벗 캐시는 담지 않는다)
pub fn write_xls(wb: &Workbook, opts: &WriteOptions) -> Result<(Vec<u8>, Report)> {
    let mut o = opts.clone();
    o.xls = true;
    let (bytes, mut report) = write_with_options(wb, &o)?;
    // 차트: 레거시가 만든 XTI 로 계열 참조를 쓴다
    let stream = container::read_entry(&bytes, "/Workbook")?;
    let xti = xls::sheet_xti(&stream);
    let hints: Vec<xls::SheetHints> = wb
        .sheets
        .iter()
        .enumerate()
        .map(|(si, s)| xls::SheetHints {
            summary_above: s.summary_above,
            summary_left: s.summary_left,
            charts: s
                .shapes
                .iter()
                .filter(|x| x.kind == crate::model::ShapeKind::Chart)
                .filter_map(|x| x.chart.as_ref())
                .map(|ch| {
                    let of = |name: Option<&str>| -> Option<u16> {
                        let i = match name {
                            Some(n) => wb.sheets.iter().position(|x| x.name == n)?,
                            None => si,
                        };
                        xti.get(&(i as u16)).copied()
                    };
                    xls_chart::write(ch, wb, si, &of)
                })
                .collect(),
        })
        .collect();
    let tips: Vec<xls::LinkTips> = wb
        .sheets
        .iter()
        .map(|s| {
            s.hyperlinks
                .iter()
                .filter_map(|h| {
                    let tip = h.tooltip.clone()?;
                    let r = crate::model::CellRange::parse(&h.range)?;
                    Some(((r.start.row, r.start.col), tip))
                })
                .collect()
        })
        .collect();
    let out = xls_file(&bytes, &hints, &tips)?;
    fn has_chart(s: &[crate::model::Shape]) -> bool {
        s.iter()
            .any(|x| x.chart.is_some() || has_chart(&x.children))
    }
    if wb.sheets.iter().any(|s| {
        s.shapes.iter().any(|x| {
            x.children
                .iter()
                .any(|c| has_chart(std::slice::from_ref(c)))
        })
    }) {
        report
            .warnings
            .push(".xls: 묶음 안의 차트는 쓰지 않는다".into());
    }
    if wb.raw_parts.iter().any(|p| p.name.contains("pivotTable")) {
        report
            .warnings
            .push(".xls: 피벗 표는 아직 쓰지 않는다".into());
    }
    Ok((out, report))
}

/// 레거시 `.cell` 파일 바이트 → `.xls` 파일 바이트 (`/Workbook` 을 BIFF8 로, 문서 속성은 그대로)
pub fn xls_from_legacy_file(bytes: &[u8]) -> Result<Vec<u8>> {
    xls_file(bytes, &[], &[])
}

fn xls_file(bytes: &[u8], hints: &[xls::SheetHints], tips: &[xls::LinkTips]) -> Result<Vec<u8>> {
    let stream = container::read_entry(bytes, "/Workbook")?;
    let wbs = xls::to_biff8_with(&stream, hints, tips)?;
    let summary = container::read_entry(bytes, "/\u{5}SummaryInformation").ok();
    let mut streams = vec![crate::cfb_write::Stream {
        name: "Workbook",
        data: &wbs,
    }];
    if let Some(s) = &summary {
        streams.push(crate::cfb_write::Stream {
            name: "\u{5}SummaryInformation",
            data: s,
        });
    }
    Ok(crate::cfb_write::write_with_storages(&streams, &[], 0))
}

/// `.xls` 파일 바이트 → 레거시 `.cell` 파일 바이트 (`/Workbook` 을 레거시 레코드로 — 분석용)
pub fn legacy_from_xls_file(bytes: &[u8]) -> Result<Vec<u8>> {
    let stream = container::read_entry(bytes, "/Workbook")?;
    let h = xls::to_hancom(&stream)?;
    let summary = container::read_entry(bytes, "/SummaryInformation").ok();
    let mut streams = vec![crate::cfb_write::Stream {
        name: "Workbook",
        data: &h,
    }];
    if let Some(s) = &summary {
        streams.push(crate::cfb_write::Stream {
            name: "SummaryInformation",
            data: s,
        });
    }
    Ok(crate::cfb_write::write_with_storages(&streams, &[], 0))
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
    let mut stream = container::read_entry(bytes, "/Workbook")?;
    // Excel 97-2003 `.xls` (BIFF8): 레거시 레코드로 바꿔 같은 리더로 읽는다 (차트는 따로 — 차트 부분 스트림)
    let xls = xls::is_biff8(&stream);
    let xls_charts = xls.then(|| xls::charts(&stream));
    if xls {
        stream = xls::to_hancom(&stream)?;
    }
    let mut wb = reader::read_workbook_stream(&stream)?;
    if let Some(xc) = &xls_charts {
        let ctx = xls_chart::ReadCtx {
            xti_names: &xc.xti_names,
            font: xc.font.clone(),
            palette: &xc.palette,
        };
        for (sheet, charts) in wb.sheets.iter_mut().zip(&xc.sheets) {
            let shapes = sheet
                .shapes
                .iter_mut()
                .filter(|s| s.kind == crate::model::ShapeKind::Chart);
            for (shape, recs) in shapes.zip(charts) {
                // 한셀은 .xls 의 차트 도형 잠금을 읽지 않는다
                shape.locks.clear();
                shape.chart = xls_chart::read(recs, &ctx);
            }
        }
    }
    if xls_charts.is_some() {
        xls_chart::apply_cell_formats(&mut wb);
    }
    if xls {
        wb.application = Some("Excel".into());
        // 표 스타일 표(TABLESTYLES)가 없는 파일 (넥셀 .nxl 등): 한셀은 새 문서의 기본 표·피벗 스타일을 쓴다
        if wb.styles.table_styles.is_none() {
            wb.styles.table_styles = Some(crate::model::TableStyles {
                default_table_style: Some("TableStyleMedium2".into()),
                default_pivot_style: Some("PivotStyleLight16".into()),
                styles: Vec::new(),
            });
        }
    }
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
