// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 두 바이너리 `.cell` 패키지를 레코드 단위로 맞대어 본다 — rcell 이 쓴 것(A)과 한셀이 쓴 것(B).
//!
//! 파트는 이름으로 짝짓되, 한셀이 세션 전체의 번호를 붙이는 파트(그리기·차트·그림·메모·VML)는 종류 안의
//! 순서로 짝짓는다. `*.bin` 파트는 레코드 종류열을 맞춘 뒤 짝지은 레코드의 내용을 바이트로 비교하고,
//! XML 파트는 속성 순서·공백·파트 번호를 무시한 정규형으로 비교한다.

use std::collections::BTreeMap;

use super::biff12::{self, Record};
use crate::error::Result;

/// 파트 하나의 비교 결과
#[derive(Debug, Clone, Default)]
pub struct PartReport {
    /// A 쪽 파트 이름 (B 쪽 이름이 다르면 `a ↔ b`)
    pub name: String,
    pub binary: bool,
    pub records_a: usize,
    pub records_b: usize,
    /// 종류가 같아 짝지은 레코드 수와 그중 내용까지 같은 수
    pub matched: usize,
    pub identical: usize,
    /// 종류별: 내용이 다른 짝 수
    pub differing: BTreeMap<u32, usize>,
    /// 종류별: 한쪽에만 있는 레코드 수
    pub only_a: BTreeMap<u32, usize>,
    pub only_b: BTreeMap<u32, usize>,
    /// 처음 몇 개 차이의 설명
    pub samples: Vec<String>,
    /// XML 파트: 정규형이 같은지
    pub xml_equal: Option<bool>,
}

/// 패키지 비교 결과
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub parts: Vec<PartReport>,
    /// 한쪽에만 있는 파트
    pub only_a: Vec<String>,
    pub only_b: Vec<String>,
}

impl Report {
    /// B(한셀)의 레코드 수, 짝지은 수, 내용까지 같은 수 (바이너리 파트 전체)
    pub fn totals(&self) -> (usize, usize, usize) {
        self.parts
            .iter()
            .filter(|p| p.binary)
            .fold((0, 0, 0), |t, p| {
                (t.0 + p.records_b, t.1 + p.matched, t.2 + p.identical)
            })
    }
}

/// 한셀이 문서와 상관없는 번호를 붙이는 파트: (종류 열쇠, 번호)
fn part_key(name: &str) -> (String, u64) {
    let numbered = ["xl/drawings/", "xl/charts/", "xl/media/", "xl/comments"];
    if !numbered.iter().any(|p| name.starts_with(p)) {
        return (name.to_string(), 0);
    }
    let mut key = String::new();
    let mut num = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            if num.is_empty() {
                key.push('#');
            }
            num.push(c);
        } else {
            key.push(c);
        }
    }
    (key, num.parse().unwrap_or(0))
}

fn zip_parts(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        if f.is_dir() {
            continue;
        }
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        let name = f.name()?.into_owned();
        out.push((name, data));
    }
    Ok(out)
}

/// 패키지 둘을 비교한다. 암호화된 패키지는 기본 암호로 풀어서 본다.
pub fn compare(a: &[u8], b: &[u8]) -> Result<Report> {
    let a = crate::decrypt_if_needed(a.to_vec(), None)?;
    let b = crate::decrypt_if_needed(b.to_vec(), None)?;
    let pa = zip_parts(&a)?;
    let pb = zip_parts(&b)?;
    // 종류 열쇠마다 번호 순으로 짝짓는다
    let group = |parts: &[(String, Vec<u8>)]| {
        let mut m: BTreeMap<String, Vec<(u64, usize)>> = BTreeMap::new();
        for (i, (n, _)) in parts.iter().enumerate() {
            let (k, num) = part_key(n);
            m.entry(k).or_default().push((num, i));
        }
        for v in m.values_mut() {
            v.sort();
        }
        m
    };
    let ga = group(&pa);
    let gb = group(&pb);
    let mut rep = Report::default();
    for (k, va) in &ga {
        let vb = gb.get(k).map(Vec::as_slice).unwrap_or(&[]);
        for (j, (_, ia)) in va.iter().enumerate() {
            let (na, da) = &pa[*ia];
            match vb.get(j) {
                Some((_, ib)) => {
                    let (nb, db) = &pb[*ib];
                    rep.parts.push(compare_part(na, nb, da, db));
                }
                None => rep.only_a.push(na.clone()),
            }
        }
        for (_, ib) in vb.iter().skip(va.len()) {
            rep.only_b.push(pb[*ib].0.clone());
        }
    }
    for (k, vb) in &gb {
        if !ga.contains_key(k) {
            rep.only_b.extend(vb.iter().map(|(_, i)| pb[*i].0.clone()));
        }
    }
    Ok(rep)
}

fn hex(b: &[u8], max: usize) -> String {
    let s: Vec<String> = b.iter().take(max).map(|x| format!("{x:02x}")).collect();
    let more = if b.len() > max { " …" } else { "" };
    format!("{}{more}", s.join(" "))
}

fn compare_part(na: &str, nb: &str, da: &[u8], db: &[u8]) -> PartReport {
    let name = if na == nb {
        na.to_string()
    } else {
        format!("{na} ↔ {nb}")
    };
    if !na.ends_with(".bin") {
        let equal = if na.ends_with(".xml") || na.ends_with(".rels") || na.ends_with(".vml") {
            canonical(da) == canonical(db)
        } else {
            da == db
        };
        return PartReport {
            name,
            binary: false,
            xml_equal: Some(equal),
            ..Default::default()
        };
    }
    let ra = biff12::records(da);
    let rb = biff12::records(db);
    let mut rep = PartReport {
        name,
        binary: true,
        records_a: ra.len(),
        records_b: rb.len(),
        ..Default::default()
    };
    for step in align(&ra, &rb) {
        match step {
            Step::Pair(i, j) => {
                rep.matched += 1;
                if ra[i].data == rb[j].data {
                    rep.identical += 1;
                } else {
                    let seen = rep.differing.entry(ra[i].rtype).or_default();
                    *seen += 1;
                    // 종류마다 처음 3개까지
                    if *seen <= 3 && rep.samples.len() < 60 {
                        let at = ra[i]
                            .data
                            .iter()
                            .zip(rb[j].data)
                            .position(|(x, y)| x != y)
                            .unwrap_or(ra[i].data.len().min(rb[j].data.len()));
                        rep.samples.push(format!(
                            "{} ({}) #{i}/#{j} @{at}\n      A: {}\n      B: {}",
                            ra[i].name(),
                            ra[i].rtype,
                            hex(ra[i].data, 64),
                            hex(rb[j].data, 64)
                        ));
                    }
                }
            }
            Step::OnlyA(i) => *rep.only_a.entry(ra[i].rtype).or_default() += 1,
            Step::OnlyB(j) => *rep.only_b.entry(rb[j].rtype).or_default() += 1,
        }
    }
    rep
}

enum Step {
    Pair(usize, usize),
    OnlyA(usize),
    OnlyB(usize),
}

/// 레코드 종류열 맞추기: 같으면 짝짓고, 다르면 앞쪽 64개 안에서 가장 가까운 다시 맞는 자리를 찾는다
fn align(a: &[Record], b: &[Record]) -> Vec<Step> {
    const WINDOW: usize = 64;
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].rtype == b[j].rtype {
            out.push(Step::Pair(i, j));
            i += 1;
            j += 1;
            continue;
        }
        // (건너뛸 A 수, 건너뛸 B 수) 중 합이 가장 작은 것
        let mut best: Option<(usize, usize)> = None;
        'search: for total in 1..=2 * WINDOW {
            for da in 0..=total.min(WINDOW) {
                let db = total - da;
                if db > WINDOW {
                    continue;
                }
                if i + da < a.len() && j + db < b.len() && a[i + da].rtype == b[j + db].rtype {
                    best = Some((da, db));
                    break 'search;
                }
            }
        }
        match best {
            Some((da, db)) => {
                out.extend((i..i + da).map(Step::OnlyA));
                out.extend((j..j + db).map(Step::OnlyB));
                i += da;
                j += db;
            }
            None => {
                out.push(Step::OnlyA(i));
                out.push(Step::OnlyB(j));
                i += 1;
                j += 1;
            }
        }
    }
    out.extend((i..a.len()).map(Step::OnlyA));
    out.extend((j..b.len()).map(Step::OnlyB));
    out
}

/// docProps/core.xml 의 `cp:revision`·`dcterms:modified`·`cp:lastModifiedBy`
fn is_save_stamp(e: &crate::xml::Element) -> bool {
    const CP: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
    match e.ns.as_deref() {
        Some(CP) => matches!(e.name.as_str(), "revision" | "lastModifiedBy"),
        Some("http://purl.org/dc/terms/") => e.name == "modified",
        _ => false,
    }
}

/// XML 정규형: 요소 이름(네임스페이스 URI), 정렬한 속성, 공백 아닌 글. 파트 이름 속성의 숫자는 지운다
fn canonical(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let Ok(root) = crate::xml::parse_raw(&text, "parity") else {
        return text.into_owned();
    };
    fn walk(e: &crate::xml::Element, out: &mut String) {
        out.push('<');
        if let Some(ns) = &e.ns {
            out.push('{');
            out.push_str(ns);
            out.push('}');
        }
        out.push_str(&e.name);
        let mut attrs: Vec<(String, String)> = e
            .attrs
            .iter()
            .map(|a| {
                let v = if matches!(a.name.as_str(), "Target" | "PartName") {
                    a.value
                        .chars()
                        .filter(|c| !c.is_ascii_digit())
                        .collect::<String>()
                } else {
                    a.value.clone()
                };
                (format!("{}{}", a.ns.as_deref().unwrap_or(""), a.name), v)
            })
            .collect();
        attrs.sort();
        for (k, v) in attrs {
            out.push_str(&format!(" {k}={v:?}"));
        }
        out.push('>');
        let mut kids: Vec<String> = Vec::new();
        for c in &e.children {
            match c {
                // 저장할 때마다 한셀이 고치는 문서 속성 (수정 번호 +1, 수정 날짜, 수정한 사람 — rcell diff 처럼 뺀다)
                crate::xml::Node::Element(x) if is_save_stamp(x) => {}
                crate::xml::Node::Element(x) => {
                    let mut s = String::new();
                    walk(x, &mut s);
                    kids.push(s);
                }
                crate::xml::Node::Text(t) if !t.trim().is_empty() => kids.push(t.clone()),
                _ => {}
            }
        }
        // 관계·콘텐츠 타입은 순서가 뜻이 없다
        if matches!(e.name.as_str(), "Relationships" | "Types") {
            kids.sort();
        }
        for k in kids {
            out.push_str(&k);
        }
        out.push_str("</>");
    }
    let mut s = String::new();
    walk(&root, &mut s);
    s
}

/// 사람이 읽을 보고서
pub fn format(rep: &Report, max_samples: usize) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let name = |t: &u32| format!("{}({t})", biff12::record_name(*t));
    for p in &rep.parts {
        if !p.binary {
            let _ = writeln!(
                s,
                "{} {}",
                if p.xml_equal == Some(true) {
                    "="
                } else {
                    "≠"
                },
                p.name
            );
            continue;
        }
        let _ = writeln!(
            s,
            "{} {}: 레코드 A {} / B {}, 짝 {}, 같음 {}",
            if p.identical == p.records_a && p.records_a == p.records_b {
                "="
            } else {
                "≠"
            },
            p.name,
            p.records_a,
            p.records_b,
            p.matched,
            p.identical
        );
        if !p.differing.is_empty() {
            let v: Vec<String> = p
                .differing
                .iter()
                .map(|(t, n)| format!("{}×{n}", name(t)))
                .collect();
            let _ = writeln!(s, "    내용 다름: {}", v.join(" "));
        }
        if !p.only_a.is_empty() {
            let v: Vec<String> = p
                .only_a
                .iter()
                .map(|(t, n)| format!("{}×{n}", name(t)))
                .collect();
            let _ = writeln!(s, "    A 에만: {}", v.join(" "));
        }
        if !p.only_b.is_empty() {
            let v: Vec<String> = p
                .only_b
                .iter()
                .map(|(t, n)| format!("{}×{n}", name(t)))
                .collect();
            let _ = writeln!(s, "    B 에만: {}", v.join(" "));
        }
        for x in p.samples.iter().take(max_samples) {
            let _ = writeln!(s, "    {x}");
        }
    }
    for n in &rep.only_a {
        let _ = writeln!(s, "+ A 에만 있는 파트 {n}");
    }
    for n in &rep.only_b {
        let _ = writeln!(s, "- B 에만 있는 파트 {n}");
    }
    let (b, m, i) = rep.totals();
    let _ = writeln!(s, "B 레코드 {b}, 짝 {m}, 내용까지 같음 {i}");
    s
}
