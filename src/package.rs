// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! OPC(Open Packaging Conventions) 컨테이너 계층.
//!
//! 한셀 2014 이후의 .cell 은 xlsx 와 같은 ZIP 패키지다. 파트 이름은 앞의 `/` 를
//! 뗀 형태(`xl/workbook.xml`)로 통일해서 다룬다.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::ns;
use crate::xml::{self, Element};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    /// 내부 대상이면 패키지 기준으로 정규화한 파트 이름, 외부 대상이면 원문.
    pub target: String,
    pub external: bool,
}

pub struct Package {
    zip: ZipArchive<Cursor<Vec<u8>>>,
    names: Vec<String>,
    /// 소문자 파트 이름 → 실제 ZIP 엔트리 이름 (OPC 파트 이름은 대소문자 무시)
    lookup: HashMap<String, String>,
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}

impl Package {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let zip = ZipArchive::new(Cursor::new(bytes))?;
        let mut names = Vec::new();
        for n in zip.file_names() {
            let n = n?;
            if !n.ends_with('/') {
                names.push(n.into_owned());
            }
        }
        let lookup = names
            .iter()
            .map(|n| (n.to_ascii_lowercase(), n.clone()))
            .collect();
        let mut pkg = Package {
            zip,
            names,
            lookup,
            defaults: HashMap::new(),
            overrides: HashMap::new(),
        };
        pkg.load_content_types()?;
        Ok(pkg)
    }

    fn load_content_types(&mut self) -> Result<()> {
        let Some(root) = self.read_xml("[Content_Types].xml")? else {
            return Err(Error::MissingPart("[Content_Types].xml".into()));
        };
        for e in root.elements() {
            if e.is(ns::CONTENT_TYPES, "Default") {
                if let (Some(ext), Some(ct)) = (e.attr("Extension"), e.attr("ContentType")) {
                    self.defaults
                        .insert(ext.trim().to_ascii_lowercase(), ct.to_string());
                }
            } else if e.is(ns::CONTENT_TYPES, "Override")
                && let (Some(p), Some(ct)) = (e.attr("PartName"), e.attr("ContentType"))
            {
                self.overrides
                    .insert(normalize_part(p).to_ascii_lowercase(), ct.to_string());
            }
        }
        Ok(())
    }

    /// ZIP 안의 모든 파트 이름 (디렉터리 제외).
    pub fn part_names(&self) -> &[String] {
        &self.names
    }

    pub fn has_part(&self, name: &str) -> bool {
        self.lookup
            .contains_key(&normalize_part(name).to_ascii_lowercase())
    }

    pub fn content_type(&self, name: &str) -> Option<&str> {
        let key = normalize_part(name).to_ascii_lowercase();
        if let Some(ct) = self.overrides.get(&key) {
            return Some(ct);
        }
        let ext = key.rsplit_once('.').map(|(_, e)| e)?;
        self.defaults.get(ext).map(String::as_str)
    }

    pub fn read_bytes(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let key = normalize_part(name).to_ascii_lowercase();
        let Some(real) = self.lookup.get(&key) else {
            return Ok(None);
        };
        let mut f = self.zip.by_name(real)?;
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        Ok(Some(buf))
    }

    pub fn read_string(&mut self, name: &str) -> Result<Option<String>> {
        match self.read_bytes(name)? {
            Some(b) => Ok(Some(decode_text(&b, name)?)),
            None => Ok(None),
        }
    }

    /// 파트를 읽어 `mc:AlternateContent` 까지 해석한 DOM 으로 돌려준다.
    pub fn read_xml(&mut self, name: &str) -> Result<Option<Element>> {
        match self.read_string(name)? {
            Some(s) => Ok(Some(xml::parse(&s, name)?)),
            None => Ok(None),
        }
    }

    /// 파트를 `mc:AlternateContent` 를 펼치지 않은 원형 DOM 으로 돌려준다.
    pub fn read_xml_raw(&mut self, name: &str) -> Result<Option<Element>> {
        match self.read_string(name)? {
            Some(s) => Ok(Some(xml::parse_raw(&s, name)?)),
            None => Ok(None),
        }
    }

    /// `source` 파트의 관계 목록. `source` 가 빈 문자열이면 패키지 루트(`_rels/.rels`).
    pub fn relationships(&mut self, source: &str) -> Result<Vec<Relationship>> {
        let rels_part = rels_part_for(source);
        let Some(root) = self.read_xml(&rels_part)? else {
            return Ok(Vec::new());
        };
        let base = parent_dir(&normalize_part(source));
        let mut out = Vec::new();
        for e in root
            .elements()
            .filter(|e| e.is(ns::PKG_REL, "Relationship"))
        {
            let (Some(id), Some(ty), Some(target)) =
                (e.attr("Id"), e.attr("Type"), e.attr("Target"))
            else {
                continue;
            };
            let external = e.attr("TargetMode") == Some("External");
            let target = if external {
                target.to_string()
            } else {
                resolve_target(&base, target)
            };
            out.push(Relationship {
                id: id.to_string(),
                rel_type: ty.to_string(),
                target,
                external,
            });
        }
        Ok(out)
    }
}

/// 앞의 `/` 를 떼고 `\` 를 `/` 로 바꾼다.
pub fn normalize_part(name: &str) -> String {
    name.trim_start_matches('/').replace('\\', "/")
}

fn parent_dir(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

pub fn rels_part_for(source: &str) -> String {
    let source = normalize_part(source);
    if source.is_empty() {
        return "_rels/.rels".into();
    }
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{source}.rels"),
    }
}

/// 관계 대상(상대/절대 경로)을 패키지 기준 파트 이름으로 바꾼다.
pub fn resolve_target(base_dir: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    let mut segs: Vec<&str> = if target.starts_with('/') || base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for s in target.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// XML 파트 바이트를 문자열로. UTF-8(BOM 포함)과 UTF-16 BOM 을 처리한다.
fn decode_text(b: &[u8], part: &str) -> Result<String> {
    let err = |m: &str| Error::Xml {
        part: part.to_string(),
        message: m.to_string(),
    };
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|_| err("잘못된 UTF-16LE"));
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|_| err("잘못된 UTF-16BE"));
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    String::from_utf8(b.to_vec()).map_err(|_| err("잘못된 UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_and_absolute_targets() {
        assert_eq!(
            resolve_target("xl", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_target("xl/worksheets", "../drawings/drawing0.xml"),
            "xl/drawings/drawing0.xml"
        );
        assert_eq!(resolve_target("xl", "/xl/styles.xml"), "xl/styles.xml");
        assert_eq!(resolve_target("", "xl/workbook.xml"), "xl/workbook.xml");
    }

    #[test]
    fn rels_part_location() {
        assert_eq!(rels_part_for(""), "_rels/.rels");
        assert_eq!(
            rels_part_for("/xl/workbook.xml"),
            "xl/_rels/workbook.xml.rels"
        );
    }
}
