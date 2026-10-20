// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 컨테이너 공통 접근 (ZIP 파트 / OLE2 스트림). 분석 도구와 레거시 파서가 쓴다.

use std::io::{Cursor, Read};

use crate::error::{Error, Result};
use crate::{FileKind, detect};

/// 컨테이너 항목 이름과 크기. ZIP 은 파트 이름, OLE2 는 `/` 로 시작하는 스트림 경로.
pub fn list_entries(bytes: &[u8]) -> Result<Vec<(String, u64)>> {
    match detect(bytes) {
        FileKind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let f = zip.by_index(i)?;
                if !f.is_dir() {
                    out.push((f.name()?.into_owned(), f.size()));
                }
            }
            Ok(out)
        }
        FileKind::Cfb => Ok(crate::cfb::Cfb::open(bytes)?
            .streams()
            .into_iter()
            .map(|(p, n)| (display_path(&p), n))
            .collect()),
        FileKind::Unknown => Err(Error::UnknownFormat),
    }
}

/// 항목 하나의 바이트.
pub fn read_entry(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    match detect(bytes) {
        FileKind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
            zip.by_name(name.trim_start_matches('/'))?
                .read_to_end(&mut buf)?;
        }
        FileKind::Cfb => {
            let path = if name.starts_with('/') {
                name.to_string()
            } else {
                format!("/{name}")
            };
            buf = crate::cfb::Cfb::open(bytes)?.read(&path)?;
        }
        FileKind::Unknown => return Err(Error::UnknownFormat),
    }
    Ok(buf)
}

/// OLE2 스트림 경로를 표시용으로. 이름의 제어 문자(예: U+0005)는 역슬래시 + x + 16진 두 자리로 표기.
pub fn display_path(p: &str) -> String {
    p.chars()
        .map(|c| {
            if (c as u32) < 0x20 {
                format!("\\x{:02x}", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// ZIP 패키지의 파트를 바꾸거나 추가한 새 패키지를 만든다. 항목 순서는 원본을 따른다.
/// 리버스 엔지니어링에서 원하는 값을 넣은 문서를 만들 때 쓴다.
pub fn replace_zip_parts(bytes: &[u8], replacements: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    use std::io::Write;
    use zip::write::{SimpleFileOptions, ZipWriter};

    let mut src = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut out = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut done = vec![false; replacements.len()];
    for i in 0..src.len() {
        let mut f = src.by_index(i)?;
        let name = f.name()?.into_owned();
        if f.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        if let Some(k) = replacements.iter().position(|(n, _)| *n == name) {
            data = replacements[k].1.clone();
            done[k] = true;
        }
        out.start_file(name, opts)?;
        out.write_all(&data)?;
    }
    for (k, (name, data)) in replacements.iter().enumerate() {
        if !done[k] {
            out.start_file(name.clone(), opts)?;
            out.write_all(data)?;
        }
    }
    Ok(out.finish()?.into_inner())
}
