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
pub mod cf_eval;
pub mod cfb;
pub mod chart_svg;
pub mod compare;
pub mod container;
pub mod crypto;
pub mod error;
pub mod formula_text;
pub mod image;
pub mod legacy;
pub mod model;
pub mod ns;
pub mod numfmt;
pub mod package;
pub mod parser;
pub mod render;
pub mod scan;
pub mod text;
pub mod theme;
pub mod units;
pub mod vocab;
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
        _ => {
            let mut pkg = open_package(bytes)?;
            parser::read_workbook(&mut pkg)
        }
    }
}

/// 통합 문서를 XML `.cell` 로 저장한다 (확장자가 .xlsx 면 Excel 형식).
pub fn save(wb: &Workbook, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let flavor = match path.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("xlsx") => writer::Flavor::Xlsx,
        _ => writer::Flavor::Cell,
    };
    std::fs::write(path, writer::write(wb, flavor)?)?;
    Ok(())
}

/// 파일에서 통합 문서를 읽는다.
pub fn open(path: impl AsRef<Path>) -> Result<Workbook> {
    from_bytes(std::fs::read(path)?)
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
    from_bytes_with_password(std::fs::read(path)?, Some(password))
}
