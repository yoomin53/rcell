// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 바이너리 `.cell` 쓰기 검증.
//!
//! - 왕복: rcell(X) → 바이너리 → rcell 이 다시 읽은 것 == rcell(X) (한컴 확장은 형식에 자리가 없다)
//! - 레코드 대조: rcell 이 X.cell 에서 쓴 바이너리 대 한셀이 같은 문서를 저장한 X.bin.cell
//! - 한셀의 읽기: 한셀이 rcell 의 바이너리를 연 결과(`*.binw.as.cell`) 대 한셀이 자기 바이너리를 연 결과
//!   (`X.bin.as.cell`) — `RCELL_BINW_ORACLE` 폴더가 있을 때 (tools/probe/binw-oracle.ps1)
//!
//! ```text
//! RCELL_CORPUS=C:\rcell-corpus cargo test --release --test write_binary -- --nocapture
//! ```

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rcell::binary::writer::{Options, write_report};
use rcell::compare::{Options as CmpOptions, compare};

fn roundtrip_diffs(a: &rcell::Workbook) -> (Vec<String>, Vec<String>) {
    let out = write_report(a, &Options::default()).expect("바이너리 쓰기");
    let b = rcell::from_bytes(out.bytes).expect("바이너리 다시 읽기");
    let opts = CmpOptions {
        ignore_hancom_ext: true,
        ..Default::default()
    };
    // 한셀의 바이너리도 외부 연결의 상태(값 캐시, 경로 없음)는 담지 않는다 (docs/format/cell-binary.md)
    let diffs = compare(a, &b, opts)
        .into_iter()
        .filter(|d| !d.starts_with("외부 연결 상태"))
        .collect();
    (diffs, out.warnings)
}

#[test]
fn fixture_roundtrips_through_binary() {
    let a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let (diffs, warnings) = roundtrip_diffs(&a);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn binary_is_detected_by_name() {
    use rcell::SaveFormat;
    assert_eq!(
        SaveFormat::from_path(Path::new("a.bin.cell")),
        SaveFormat::Binary
    );
    assert_eq!(
        SaveFormat::from_path(Path::new("a.XLSB")),
        SaveFormat::Binary
    );
    assert_eq!(SaveFormat::from_path(Path::new("a.cell")), SaveFormat::Cell);
    assert_eq!(SaveFormat::from_path(Path::new("a.xlsx")), SaveFormat::Xlsx);
}

/// 코퍼스의 (X.cell, X.bin.cell) 쌍
fn corpus_pairs() -> Option<Vec<(PathBuf, PathBuf)>> {
    let root = PathBuf::from(std::env::var_os("RCELL_CORPUS")?);
    let mut out = Vec::new();
    for d in ["", "templates", "craft", "craft-charts", "craft-charts2"] {
        let dir = root.join(d);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".bin.cell"))
            })
            .collect();
        v.sort();
        for bin in v {
            let name = bin.file_name().unwrap().to_string_lossy().into_owned();
            let xml = bin.with_file_name(name.replace(".bin.cell", ".cell"));
            if xml.exists() {
                out.push((xml, bin));
            }
        }
    }
    Some(out)
}

/// (a) 코퍼스 문서 전부: XML 원본·한셀 바이너리·레거시 원본을 rcell 로 읽어 바이너리로 쓰고 다시 읽으면 같다
#[test]
fn corpus_roundtrips_through_binary() {
    let Some(pairs) = corpus_pairs() else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for (xml, bin) in &pairs {
        let legacy = PathBuf::from(xml.to_string_lossy().replace(".cell", ".legacy.cell"));
        for src in [xml, bin, &legacy] {
            if !src.exists() {
                continue;
            }
            let Ok(mut a) = rcell::open(src) else {
                continue;
            };
            // 레거시 차트(한컴 0x7EF0)는 아직 모델로 읽지 못해 어느 쓰기도 쓰지 않는다 (알려진 한계, real_samples 와 같다)
            for s in &mut a.sheets {
                s.shapes.retain(|x| {
                    !(x.kind == rcell::model::ShapeKind::Chart
                        && x.chart.is_none()
                        && x.raw.is_none())
                });
            }
            let (diffs, warnings) = roundtrip_diffs(&a);
            checked += 1;
            eprintln!("{:>4} 차이  {}", diffs.len(), src.display());
            if !diffs.is_empty() || !warnings.is_empty() {
                failures.push(format!(
                    "{}:\n{}\n{}",
                    src.display(),
                    diffs.join("\n"),
                    warnings.join("\n")
                ));
            }
        }
    }
    eprintln!("왕복한 문서: {checked}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// (b) 레코드 대조: rcell 이 X.cell 에서 쓴 바이너리 대 한셀의 X.bin.cell
#[test]
fn corpus_records_match_hancell() {
    let Some(pairs) = corpus_pairs() else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let mut total = (0usize, 0usize, 0usize);
    let mut differing: BTreeMap<u32, usize> = BTreeMap::new();
    let mut only_a: BTreeMap<u32, usize> = BTreeMap::new();
    let mut only_b: BTreeMap<u32, usize> = BTreeMap::new();
    let mut xml_diff: BTreeMap<String, usize> = BTreeMap::new();
    let mut part_diff: BTreeMap<String, usize> = BTreeMap::new();
    let verbose = std::env::var_os("RCELL_PARITY_VERBOSE").is_some();
    for (xml, bin) in &pairs {
        let a = rcell::open(xml).expect("XML 읽기");
        // 한셀은 저장한 폴더를 BrtAbsPath15 에 쓴다 — 같은 폴더를 넣어 비교한다
        let dir = format!("{}\\", xml.parent().unwrap().display()).replace('/', "\\");
        let out = write_report(
            &a,
            &Options {
                abs_path: Some(dir),
            },
        )
        .expect("바이너리 쓰기");
        let rep =
            rcell::binary::parity::compare(&out.bytes, &std::fs::read(bin).unwrap()).expect("대조");
        let (b, m, i) = rep.totals();
        total = (total.0 + b, total.1 + m, total.2 + i);
        eprintln!(
            "{:>6}/{:<6} 같음  {}",
            i,
            b,
            bin.file_name().unwrap().to_string_lossy()
        );
        if verbose {
            eprint!("{}", rcell::binary::parity::format(&rep, 4));
        }
        for p in &rep.parts {
            for (t, n) in &p.differing {
                *differing.entry(*t).or_default() += n;
            }
            for (t, n) in &p.only_a {
                *only_a.entry(*t).or_default() += n;
            }
            for (t, n) in &p.only_b {
                *only_b.entry(*t).or_default() += n;
            }
            if p.xml_equal == Some(false) {
                let k = p.name.chars().filter(|c| !c.is_ascii_digit()).collect();
                *xml_diff.entry(k).or_default() += 1;
            }
        }
        for n in rep.only_a.iter().chain(&rep.only_b) {
            let k = n.chars().filter(|c| !c.is_ascii_digit()).collect();
            *part_diff.entry(k).or_default() += 1;
        }
    }
    let name = |t: &u32| format!("{}({t})", rcell::binary::biff12::record_name(*t));
    eprintln!(
        "한셀 레코드 {}, 종류가 맞은 레코드 {}, 내용까지 같은 레코드 {} ({:.1}%)",
        total.0,
        total.1,
        total.2,
        100.0 * total.2 as f64 / total.0.max(1) as f64
    );
    let show = |title: &str, m: &BTreeMap<u32, usize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let s: Vec<String> = v.iter().map(|(t, n)| format!("{}×{n}", name(t))).collect();
        eprintln!("{title}: {}", s.join(" "));
    };
    show("내용 다름", &differing);
    show("rcell 에만", &only_a);
    show("한셀에만", &only_b);
    eprintln!("다른 XML 파트: {xml_diff:?}");
    eprintln!("한쪽에만 있는 파트: {part_diff:?}");
    // 회귀 막기: 한셀 레코드의 90% 이상이 바이트까지 같아야 한다
    assert!(
        total.2 as f64 >= 0.9 * total.0 as f64,
        "레코드 일치율이 낮다: {}/{}",
        total.2,
        total.0
    );
}

/// (c) 한셀의 읽기: `RCELL_BINW_ORACLE` 폴더의 `X.binw.as.cell` (한셀이 rcell 의 바이너리를 .xlsb 로 열어 XML 로
/// 저장한 것) 대 코퍼스의 `X.bin.as.cell` (한셀이 자기 바이너리를 같은 길로 연 것)
#[test]
fn hancell_reads_rcell_binary_like_its_own() {
    let (Some(pairs), Some(oracle)) = (
        corpus_pairs(),
        std::env::var_os("RCELL_BINW_ORACLE").map(PathBuf::from),
    ) else {
        eprintln!("RCELL_CORPUS·RCELL_BINW_ORACLE 미설정 — 건너뜀");
        return;
    };
    let opts = CmpOptions {
        ignore_hancom_ext: true,
        ..Default::default()
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for (xml, bin) in &pairs {
        let stem = xml
            .file_name()
            .unwrap()
            .to_string_lossy()
            .trim_end_matches(".cell")
            .to_string();
        let sub = xml
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| n != "rcell-corpus")
            .unwrap_or_default();
        let ours = oracle.join(&sub).join(format!("{stem}.binw.as.cell"));
        let theirs = bin.with_file_name(format!("{stem}.bin.as.cell"));
        if !ours.exists() || !theirs.exists() {
            continue;
        }
        let a = rcell::open(&theirs).expect("한셀이 읽은 한셀 바이너리");
        let b = rcell::open(&ours).expect("한셀이 읽은 rcell 바이너리");
        let diffs = compare(&a, &b, opts);
        checked += 1;
        eprintln!("{:>4} 차이  {}", diffs.len(), ours.display());
        if !diffs.is_empty() {
            failures.push(format!("{}:\n{}", ours.display(), diffs.join("\n")));
        }
    }
    eprintln!("비교한 쌍: {checked}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
