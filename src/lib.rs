// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! # rcell
//!
//! 한컴 셀(Hancom Cell, 한셀) `.cell` 스프레드시트 포맷의 클린룸 오픈소스 구현.
//!
//! 한셀 2014 이후의 `.cell` 은 OOXML SpreadsheetML ZIP 패키지에 한컴 확장
//! 네임스페이스(`http://schemas.haansoft.com/office/spreadsheet/8.0`)를 더한 형식이다.
//! 한셀 2010 이하의 OLE2 바이너리 `.cell` 은 아직 지원하지 않는다.
//!
//! ```no_run
//! let wb = rcell::open("문서.cell")?;
//! for sheet in &wb.sheets {
//!     println!("{}: 셀 {}개", sheet.name, sheet.cells.len());
//! }
//! # Ok::<(), rcell::Error>(())
//! ```
//!
//! 작성자: yoomin53 (<https://github.com/yoomin53>)

pub mod binary;
pub mod calc;
pub mod cf_eval;
pub mod cfb;
pub mod cfb_write;
pub mod chart_svg;
pub mod chart_xml;
pub mod compare;
pub mod container;
pub mod crypto;
pub mod csv;
pub mod edit;
pub mod error;
pub mod formula_text;
pub mod image;
pub mod legacy;
pub mod model;
pub mod ns;
pub mod numfmt;
pub mod ods;
pub mod package;
pub mod parser;
pub mod print;
pub mod render;
pub mod scan;
pub mod text;
pub mod theme;
pub mod units;
pub mod vocab;
pub mod webpage;
pub mod writer;
pub mod xml;

use std::path::Path;

pub use error::{Error, Result};
pub use model::Workbook;
pub use package::Package;

/// 파일 컨테이너 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// ZIP (OPC) — 한셀 2014 이후 .cell, .xlsx
    Zip,
    /// OLE2 복합 문서 — 한셀 2010 이하 .cell, .xls, 암호화된 OOXML
    Cfb,
    Unknown,
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
const CFB_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub fn detect(bytes: &[u8]) -> FileKind {
    if bytes.starts_with(ZIP_MAGIC) {
        FileKind::Zip
    } else if bytes.starts_with(CFB_MAGIC) {
        FileKind::Cfb
    } else {
        FileKind::Unknown
    }
}

/// 바이트에서 OOXML 패키지를 연다. 형식이 맞지 않으면 원인을 알려주는 오류를 낸다.
pub fn open_package(bytes: Vec<u8>) -> Result<Package> {
    match detect(&bytes) {
        FileKind::Zip => Package::from_bytes(bytes),
        FileKind::Cfb => Err(Error::LegacyOrEncrypted),
        FileKind::Unknown => Err(Error::UnknownFormat),
    }
}

/// 바이트에서 통합 문서를 읽는다. XML `.cell`, 레거시 바이너리 `.cell` 을 모두 받는다.
pub fn from_bytes(bytes: Vec<u8>) -> Result<Workbook> {
    from_bytes_with_password(bytes, None)
}

/// 암호화된 문서(OLE2 안의 `EncryptionInfo`/`EncryptedPackage`)는 주어진 암호로, 없으면
/// 기본 암호(`VelvetSweatshop` — 보호만 걸린 문서)로 풀어서 읽는다.
pub fn from_bytes_with_password(bytes: Vec<u8>, password: Option<&str>) -> Result<Workbook> {
    match detect(&bytes) {
        FileKind::Cfb => {
            let entries = container::list_entries(&bytes)?;
            let has = |n: &str| entries.iter().any(|(e, _)| e.eq_ignore_ascii_case(n));
            if has("/EncryptionInfo") && has("/EncryptedPackage") {
                let info = container::read_entry(&bytes, "/EncryptionInfo")?;
                let package = container::read_entry(&bytes, "/EncryptedPackage")?;
                let plain = match password {
                    Some(p) => crypto::decrypt_package(&info, &package, p)?,
                    None => crypto::decrypt_package(&info, &package, crypto::DEFAULT_PASSWORD)?,
                };
                return from_bytes_with_password(plain, None);
            }
            legacy::read(&bytes)
        }
        _ if ods::is_ods(&bytes) => ods::read(&bytes),
        _ => {
            let mut pkg = open_package(bytes)?;
            parser::read_workbook(&mut pkg)
        }
    }
}

/// 저장 형식
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    /// XML `.cell` (한셀 2014 이후 기본 형식)
    Cell,
    /// Excel `.xlsx`
    Xlsx,
    /// 바이너리 `.cell` (한셀 xlCellb, -2007 — BIFF12 레코드, [MS-XLSB])
    Binary,
    /// 레거시 `.cell` (한셀 xlCell8, -2005 — OLE2 + BIFF8 계열 레코드)
    Legacy,
    /// CSV (한셀 FileFormat 6 — 활성 시트, 코드 페이지 949, 쉼표)
    Csv,
    /// 텍스트 (한셀 FileFormat -4158 — 활성 시트, 코드 페이지 949, 탭)
    Text,
    /// PRN (한셀 FileFormat 36 — 활성 시트, 코드 페이지 949, 공백)
    Prn,
    /// Excel 97-2003 `.xls` (한셀 FileFormat 56 — BIFF8, [MS-XLS])
    Xls,
    /// 넥셀 `.nxl` (한셀 FileFormat -2003 — BIFF8 틀 + 한컴 확장)
    Nxl,
    /// ODF 스프레드시트 `.ods` (한셀 FileFormat 60)
    Ods,
    /// 웹 페이지 `.htm` (한셀 FileFormat 44 — `이름.htm` 과 `이름.files/` 폴더)
    Htm,
    /// 웹 보관 파일 `.mht` (한셀 FileFormat 45 — MIME 한 덩이)
    Mht,
}

impl SaveFormat {
    /// 파일 이름으로 고른다: `.xlsx` → Excel, `.xlsb`·`*.bin.cell` → 바이너리, `*.legacy.cell` → 레거시,
    /// `.xls` → Excel 97-2003, `.nxl` → 넥셀, `.ods` → ODF, `.csv` → CSV, `.txt`·`.tsv` → 탭 텍스트, `.prn` → 공백 텍스트, 나머지 → XML `.cell`
    pub fn from_path(path: &Path) -> SaveFormat {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.ends_with(".xlsx") {
            SaveFormat::Xlsx
        } else if name.ends_with(".xlsb") || name.ends_with(".bin.cell") {
            SaveFormat::Binary
        } else if name.ends_with(".legacy.cell") {
            SaveFormat::Legacy
        } else if name.ends_with(".xls") {
            SaveFormat::Xls
        } else if name.ends_with(".nxl") {
            SaveFormat::Nxl
        } else if name.ends_with(".ods") {
            SaveFormat::Ods
        } else if name.ends_with(".htm") || name.ends_with(".html") {
            SaveFormat::Htm
        } else if name.ends_with(".mht") || name.ends_with(".mhtml") {
            SaveFormat::Mht
        } else if name.ends_with(".csv") {
            SaveFormat::Csv
        } else if name.ends_with(".txt") || name.ends_with(".tsv") {
            SaveFormat::Text
        } else if name.ends_with(".prn") {
            SaveFormat::Prn
        } else {
            SaveFormat::Cell
        }
    }
}

/// 통합 문서를 저장한다. 형식은 파일 이름으로 정한다 ([`SaveFormat::from_path`]).
pub fn save(wb: &Workbook, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    save_as(wb, path, SaveFormat::from_path(path))
}

/// 통합 문서를 주어진 형식으로 저장한다.
pub fn save_as(wb: &Workbook, path: impl AsRef<Path>, format: SaveFormat) -> Result<()> {
    if format == SaveFormat::Htm {
        return webpage::save_htm(wb, path.as_ref());
    }
    let bytes = match format {
        SaveFormat::Cell => writer::write(wb, writer::Flavor::Cell)?,
        SaveFormat::Xlsx => writer::write(wb, writer::Flavor::Xlsx)?,
        SaveFormat::Binary => binary::writer::write(wb, &Default::default())?,
        SaveFormat::Legacy => {
            legacy::write_with_options(wb, &legacy::WriteOptions::for_path(path.as_ref()))?.0
        }
        SaveFormat::Csv => csv::write(wb, csv::Delimiter::Comma)?,
        SaveFormat::Text => csv::write(wb, csv::Delimiter::Tab)?,
        SaveFormat::Prn => csv::write(wb, csv::Delimiter::Space)?,
        SaveFormat::Xls => legacy::write_xls(wb, &legacy::WriteOptions::for_path(path.as_ref()))?.0,
        SaveFormat::Nxl => legacy::write_nxl(wb, &legacy::WriteOptions::for_path(path.as_ref()))?.0,
        SaveFormat::Ods => ods::write(wb)?,
        SaveFormat::Mht => webpage::write_mht(wb),
        SaveFormat::Htm => unreachable!("위에서 처리"),
    };
    std::fs::write(path, bytes)?;
    Ok(())
}

/// 통합 문서를 바이너리 `.cell` (한셀 xlCellb) 로 저장한다.
pub fn save_binary(wb: &Workbook, path: impl AsRef<Path>) -> Result<()> {
    save_as(wb, path, SaveFormat::Binary)
}

/// 통합 문서를 레거시 `.cell` (OLE2 + BIFF 계열, 한셀 `xlCell8` -2005) 로 저장한다.
pub fn save_legacy(wb: &Workbook, path: impl AsRef<Path>) -> Result<()> {
    save_as(wb, path, SaveFormat::Legacy)
}

/// 파일에서 통합 문서를 읽는다. ZIP·OLE2 가 아니고 확장자가 `.csv`·`.txt`·`.tsv`·`.out`·`.prn` 이면
/// 텍스트 파일로 연다 (시트 이름은 파일 이름 — [`csv::read`]).
pub fn open(path: impl AsRef<Path>) -> Result<Workbook> {
    open_path(path.as_ref(), None)
}

fn open_path(path: &Path, password: Option<&str>) -> Result<Workbook> {
    let bytes = std::fs::read(path)?;
    if detect(&bytes) == FileKind::Unknown
        && let Some(d) = csv::Delimiter::from_path(path)
    {
        return csv::read(&bytes, d, &csv::sheet_name_for(path));
    }
    from_bytes_with_password(bytes, password)
}

/// 암호화된 OLE2 컨테이너면 풀어서 안의 ZIP 패키지를, 아니면 그대로 돌려준다 (분석 도구용).
pub fn decrypt_if_needed(bytes: Vec<u8>, password: Option<&str>) -> Result<Vec<u8>> {
    if detect(&bytes) != FileKind::Cfb {
        return Ok(bytes);
    }
    let entries = container::list_entries(&bytes)?;
    let has = |n: &str| entries.iter().any(|(e, _)| e.eq_ignore_ascii_case(n));
    if !(has("/EncryptionInfo") && has("/EncryptedPackage")) {
        return Ok(bytes);
    }
    let info = container::read_entry(&bytes, "/EncryptionInfo")?;
    let package = container::read_entry(&bytes, "/EncryptedPackage")?;
    crypto::decrypt_package(
        &info,
        &package,
        password.unwrap_or(crypto::DEFAULT_PASSWORD),
    )
}

/// 암호가 걸린 파일을 연다.
pub fn open_with_password(path: impl AsRef<Path>, password: &str) -> Result<Workbook> {
    open_path(path.as_ref(), Some(password))
}
