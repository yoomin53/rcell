// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한컴 확장 어휘 스캐너 — 리버스 엔지니어링 진척도 측정 도구.
//!
//! 패키지의 모든 XML 파트를 `mc:AlternateContent` 를 펼치지 않은 원형으로 훑어서
//! 한컴 네임스페이스의 요소·속성, 관측된 값, rcell 모델링 여부를 모은다.
//! `modeled == false` 인 항목이 다음 분석 대상이다.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::error::Result;
use crate::ns;
use crate::package::Package;
use crate::vocab;
use crate::xml::Element;

/// rcell 이 의미를 해석해 모델에 담는 한컴 확장 (`부모/이름`, 속성은 `부모/@이름`).
const MODELED: &[&str] = &[
    "calcPr/@hclCalcId",
    "font/@extension",
    "font/size",
    "font/ratio",
    "font/spacing",
    "font/offset",
    "font/underlineType",
    "font/underlineShape",
    "font/underlineColor",
    "font/strikeoutType",
    "font/strikeoutShape",
    "font/strikeoutColor",
    "font/outlineType",
    "font/shadowType",
    "font/shadowColor",
    "font/shadowOffsetX",
    "font/shadowOffsetY",
    "font/shadeColor",
    "font/emboss",
    "font/engrave",
    "font/superscript",
    "font/subscript",
    "font/useFontSpace",
    "font/useKerning",
    "font/diacsymMark",
    "xf/@applyExtension",
    "dxf/@applyExtension",
    "dxf/parashape",
    "xf/parashape",
    "parashape/@lineSpacingType",
    "parashape/@lineSpacing",
    "parashape/@prevSpacing",
    "parashape/@nextSpacing",
    "parashape/@breakLatinWord",
    "parashape/@breakNonLatinWord",
    "parashape/@condense",
    "parashape/@textAlignment",
    "parashape/@fontLineHeight",
    "parashape/@leftMargin",
    "parashape/@rightMargin",
    "parashape/@indentation",
    "parashape/@abstractNumId",
    "parashape/@abstractNumType",
    "xf/accountUnderline",
    "dxf/accountUnderline",
];

const MAX_VALUES: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct VocabEntry {
    pub namespace: String,
    /// `부모/이름` 또는 `부모/@속성`
    pub path: String,
    pub count: usize,
    /// 관측된 서로 다른 값 (최대 8개)
    pub values: BTreeSet<String>,
    /// 알려진 열거 어휘(`vocab`)에 없는 값 — 새로운 사실이므로 분석 대상
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub unknown_values: BTreeSet<String>,
    pub parts: BTreeSet<String>,
    pub modeled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartInfo {
    pub name: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub parts: Vec<PartInfo>,
    /// 문서에서 선언된 모든 네임스페이스 URI → 선언 횟수
    pub namespaces: BTreeMap<String, usize>,
    pub hancom: Vec<VocabEntry>,
}

impl ScanReport {
    /// 모델링된 한컴 어휘 비율 (0.0–1.0). 관측된 어휘가 없으면 1.0.
    pub fn coverage(&self) -> f64 {
        if self.hancom.is_empty() {
            return 1.0;
        }
        let m = self.hancom.iter().filter(|e| e.modeled).count();
        m as f64 / self.hancom.len() as f64
    }

    /// 다른 파일의 결과를 합친다 (여러 샘플을 한 번에 스캔할 때).
    pub fn merge(&mut self, other: ScanReport) {
        for (k, v) in other.namespaces {
            *self.namespaces.entry(k).or_default() += v;
        }
        let mut map: BTreeMap<(String, String), VocabEntry> = std::mem::take(&mut self.hancom)
            .into_iter()
            .map(|e| ((e.namespace.clone(), e.path.clone()), e))
            .collect();
        for e in other.hancom {
            match map.get_mut(&(e.namespace.clone(), e.path.clone())) {
                Some(x) => {
                    x.count += e.count;
                    x.parts.extend(e.parts);
                    x.unknown_values.extend(e.unknown_values);
                    for v in e.values {
                        if x.values.len() < MAX_VALUES {
                            x.values.insert(v);
                        }
                    }
                }
                None => {
                    map.insert((e.namespace.clone(), e.path.clone()), e);
                }
            }
        }
        self.hancom = map.into_values().collect();
    }
}

fn is_xml_part(name: &str, content_type: Option<&str>) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".xml")
        || lower.ends_with(".rels")
        || content_type.is_some_and(|c| c.ends_with("xml"))
}

pub fn scan_package(pkg: &mut Package) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut vocab: BTreeMap<(String, String), VocabEntry> = BTreeMap::new();
    let names: Vec<String> = pkg.part_names().to_vec();

    for name in names {
        let ct = pkg.content_type(&name).map(str::to_string);
        let xml = is_xml_part(&name, ct.as_deref());
        report.parts.push(PartInfo {
            name: name.clone(),
            content_type: ct,
        });
        if !xml {
            continue;
        }
        let Some(root) = pkg.read_xml_raw(&name)? else {
            continue;
        };
        collect(&root, "", &name, &mut report.namespaces, &mut vocab);
    }
    report.hancom = vocab.into_values().collect();
    Ok(report)
}

fn note(
    entries: &mut BTreeMap<(String, String), VocabEntry>,
    ns_uri: &str,
    path: String,
    value: Option<&str>,
    part: &str,
) {
    // 공유 문자열 런 글꼴(x:rPr)은 셀 글꼴(x:font)과 같은 확장을 쓴다 (CRPrElt 심볼).
    let canonical = match path.strip_prefix("rPr/") {
        Some(rest) => format!("font/{rest}"),
        None => path.clone(),
    };
    let modeled = MODELED.contains(&canonical.as_str());
    let known_values = vocab::for_path(&canonical);
    let e = entries
        .entry((ns_uri.to_string(), path.clone()))
        .or_insert_with(|| VocabEntry {
            namespace: ns_uri.to_string(),
            path,
            count: 0,
            values: BTreeSet::new(),
            unknown_values: BTreeSet::new(),
            parts: BTreeSet::new(),
            modeled,
        });
    e.count += 1;
    e.parts.insert(part.to_string());
    if let Some(v) = value {
        if e.values.len() < MAX_VALUES {
            e.values.insert(v.to_string());
        }
        if known_values.is_some_and(|k| !k.contains(&v)) {
            e.unknown_values.insert(v.to_string());
        }
    }
}

fn collect(
    el: &Element,
    parent: &str,
    part: &str,
    namespaces: &mut BTreeMap<String, usize>,
    vocab: &mut BTreeMap<(String, String), VocabEntry>,
) {
    for (_, uri) in &el.ns_decls {
        *namespaces.entry(uri.clone()).or_default() += 1;
    }
    // mc:AlternateContent / mc:Choice / mc:Fallback 는 경로에서 투명하게 취급한다.
    let transparent = el.ns.as_deref() == Some(ns::MC);
    let here = if transparent {
        parent
    } else {
        el.name.as_str()
    };

    if let Some(uri) = el.ns.as_deref()
        && ns::is_hancom(uri)
    {
        note(
            vocab,
            uri,
            format!("{parent}/{}", el.name),
            el.attr("val"),
            part,
        );
    }
    for a in &el.attrs {
        if let Some(uri) = a.ns.as_deref()
            && ns::is_hancom(uri)
        {
            note(
                vocab,
                uri,
                format!("{here}/@{}", a.name),
                Some(&a.value),
                part,
            );
        }
    }
    for c in el.elements() {
        collect(c, here, part, namespaces, vocab);
    }
}
