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

/// 표: 표 파트(`xl/tables/table1.bin`)와 구조적 참조(PtgList)가 바이너리를 거쳐 그대로 돌아온다
#[test]
fn tables_and_structured_references_roundtrip() {
    use rcell::model::{Cell, CellRef, CellValue, Formula, Table, TableColumn, TableStyleInfo};
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    let put = |s: &mut rcell::model::Sheet, row: u32, col: u32, v: CellValue, f: Option<&str>| {
        s.cells.push(Cell {
            r: CellRef { row, col },
            style: 0,
            value: v,
            formula: f.map(|t| Formula {
                text: t.into(),
                ..Default::default()
            }),
        });
    };
    {
        let s = &mut a.sheets[1];
        put(s, 0, 4, CellValue::Text("안녕 & 반가워".into()), None);
        put(s, 0, 5, CellValue::Text("굵게 보통".into()), None);
        put(s, 1, 4, CellValue::Number(1.0), None);
        put(s, 1, 5, CellValue::Number(3.0), None);
        put(s, 2, 4, CellValue::Number(2.0), None);
        put(s, 2, 5, CellValue::Number(4.0), None);
        s.cells.sort_by_key(|c| (c.r.row, c.r.col));
        s.tables.push(Table {
            id: 1,
            name: "표1".into(),
            display_name: "표1".into(),
            range: "E1:F3".into(),
            totals_shown: false,
            columns: vec![
                TableColumn {
                    id: 1,
                    name: "안녕 & 반가워".into(),
                    ..Default::default()
                },
                TableColumn {
                    id: 2,
                    name: "굵게 보통".into(),
                    ..Default::default()
                },
            ],
            style: Some(TableStyleInfo {
                name: Some("TableStyleMedium2".into()),
                row_stripes: true,
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    {
        let s = &mut a.sheets[0];
        put(s, 0, 3, CellValue::Number(7.0), Some("SUM(표1[굵게 보통])"));
        put(s, 1, 3, CellValue::Number(6.0), Some("COUNTA(표1[])"));
        s.cells.sort_by_key(|c| (c.r.row, c.r.col));
    }
    let out = write_report(&a, &Options::default()).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let t = rcell::container::read_entry(&out.bytes, "xl/tables/table1.bin").unwrap();
    // BrtBeginList(343) 로 시작해 BrtEndList(344) 로 끝난다
    assert_eq!(&t[..2], &[0xD7, 0x02]);
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
                    .is_some_and(|n| !n.starts_with("~$") && n.ends_with(".bin.cell"))
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

/// 레코드 대조 한 벌의 합계
#[derive(Default)]
struct Parity {
    /// 한셀 레코드 수, 종류가 맞은 수, 내용까지 같은 수
    total: (usize, usize, usize),
    differing: BTreeMap<u32, usize>,
    only_a: BTreeMap<u32, usize>,
    only_b: BTreeMap<u32, usize>,
    xml_diff: BTreeMap<String, usize>,
    part_diff: BTreeMap<String, usize>,
}

fn parity(pairs: &[(PathBuf, PathBuf)], exact: bool, verbose: bool) -> Parity {
    let mut s = Parity::default();
    for (xml, bin) in pairs {
        let a = rcell::open(xml).expect("XML 읽기");
        // 한셀은 저장한 폴더를 BrtAbsPath15 에 쓴다 — 같은 폴더를 넣어 비교한다
        let dir = format!("{}\\", xml.parent().unwrap().display()).replace('/', "\\");
        let out = write_report(
            &a,
            &Options {
                abs_path: Some(dir),
                exact,
            },
        )
        .expect("바이너리 쓰기");
        let rep =
            rcell::binary::parity::compare(&out.bytes, &std::fs::read(bin).unwrap()).expect("대조");
        let (b, m, i) = rep.totals();
        s.total = (s.total.0 + b, s.total.1 + m, s.total.2 + i);
        if verbose {
            eprintln!(
                "{:>6}/{:<6} 같음  {}",
                i,
                b,
                bin.file_name().unwrap().to_string_lossy()
            );
            eprint!("{}", rcell::binary::parity::format(&rep, 4));
        }
        for p in &rep.parts {
            for (t, n) in &p.differing {
                *s.differing.entry(*t).or_default() += n;
            }
            for (t, n) in &p.only_a {
                *s.only_a.entry(*t).or_default() += n;
            }
            for (t, n) in &p.only_b {
                *s.only_b.entry(*t).or_default() += n;
            }
            if p.xml_equal == Some(false) {
                let k = p.name.chars().filter(|c| !c.is_ascii_digit()).collect();
                *s.xml_diff.entry(k).or_default() += 1;
            }
        }
        for n in rep.only_a.iter().chain(&rep.only_b) {
            let k = n.chars().filter(|c| !c.is_ascii_digit()).collect();
            *s.part_diff.entry(k).or_default() += 1;
        }
    }
    s
}

fn show_parity(title: &str, s: &Parity) {
    let name = |t: &u32| format!("{}({t})", rcell::binary::biff12::record_name(*t));
    eprintln!(
        "[{title}] 한셀 레코드 {}, 종류가 맞은 레코드 {}, 내용까지 같은 레코드 {} ({:.2}%)",
        s.total.0,
        s.total.1,
        s.total.2,
        100.0 * s.total.2 as f64 / s.total.0.max(1) as f64
    );
    let show = |what: &str, m: &BTreeMap<u32, usize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let x: Vec<String> = v.iter().map(|(t, n)| format!("{}×{n}", name(t))).collect();
        eprintln!("    {what}: {}", x.join(" "));
    };
    show("내용 다름", &s.differing);
    show("rcell 에만", &s.only_a);
    show("한셀에만", &s.only_b);
    eprintln!("    다른 XML 파트: {:?}", s.xml_diff);
    eprintln!("    한쪽에만 있는 파트: {:?}", s.part_diff);
}

/// (b) 레코드 대조: rcell 이 X.cell 에서 쓴 바이너리 대 한셀의 X.bin.cell — 한셀처럼 잃는 쓰기(`exact`)와 기본 쓰기
#[test]
fn corpus_records_match_hancell() {
    let Some(pairs) = corpus_pairs() else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let verbose = std::env::var_os("RCELL_PARITY_VERBOSE").is_some();
    let exact = parity(&pairs, true, verbose);
    let lossless = parity(&pairs, false, false);
    show_parity("exact", &exact);
    show_parity("기본", &lossless);
    // 회귀 막기: 한셀 레코드의 98% 이상이 바이트까지 같아야 한다
    assert!(
        exact.total.2 as f64 >= 0.98 * exact.total.0 as f64,
        "레코드 일치율이 낮다: {}/{}",
        exact.total.2,
        exact.total.0
    );
}

/// (c) 한셀의 읽기: `RCELL_BINW_ORACLE` 폴더의 `X.binx.as.cell`·`X.binw.as.cell` (한셀이 rcell 의 `exact`·기본 바이너리를
/// .xlsb 로 열어 XML 로 저장한 것, tools/probe/binw-oracle.ps1) 대 코퍼스의 `X.bin.as.cell` (한셀이 자기 바이너리를 같은
/// 길로 연 것). `exact` 는 차이가 없어야 하고, 기본 쓰기는 한셀의 바이너리 저장이 잃는 값(차등 서식 맞춤·표시 형식,
/// draft, 구조 잠금 없는 통합 문서 보호)에서만 달라야 한다 — 한셀은 rcell 이 더 쓴 값을 그대로 읽는다
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
    let mut checked = (0, 0);
    let mut lossless_extra: BTreeMap<String, usize> = BTreeMap::new();
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
        let theirs = bin.with_file_name(format!("{stem}.bin.as.cell"));
        if !theirs.exists() {
            continue;
        }
        let a = rcell::open(&theirs).expect("한셀이 읽은 한셀 바이너리");
        for (suffix, exact) in [("binx", true), ("binw", false)] {
            let ours = oracle.join(&sub).join(format!("{stem}.{suffix}.as.cell"));
            if !ours.exists() {
                continue;
            }
            let b = rcell::open(&ours).expect("한셀이 읽은 rcell 바이너리");
            let diffs = compare(&a, &b, opts);
            if exact {
                checked.0 += 1;
                eprintln!("{:>4} 차이  {}", diffs.len(), ours.display());
                if !diffs.is_empty() {
                    failures.push(format!("{}:\n{}", ours.display(), diffs.join("\n")));
                }
                continue;
            }
            checked.1 += 1;
            for d in diffs {
                let head = d.lines().next().unwrap_or("").to_string();
                let allowed = head.ends_with("조건부 서식")
                    || head.ends_with("인쇄")
                    || head == "통합 문서 보호";
                if allowed {
                    *lossless_extra.entry(head).or_default() += 1;
                } else {
                    failures.push(format!("{} (기본 쓰기):\n{d}", ours.display()));
                }
            }
        }
    }
    eprintln!(
        "비교한 쌍: exact {}, 기본 {} — 기본 쓰기에서 한셀 바이너리보다 더 남은 값: {lossless_extra:?}",
        checked.0, checked.1
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
