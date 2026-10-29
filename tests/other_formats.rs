// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 그 밖의 형식: CSV·텍스트 읽기·쓰기, Excel 97-2003 `.xls` (BIFF8) 쓰기·읽기.
//! 규칙과 한셀 대조 결과는 `docs/format/other-formats.md`.
//!
//! ```text
//! RCELL_CORPUS=C:\rcell-corpus cargo test --release --test other_formats -- --nocapture
//! ```

use std::path::{Path, PathBuf};

use rcell::csv::{self, Delimiter};
use rcell::model::{CellRef, CellValue};

fn at(wb: &rcell::Workbook, a1: &str) -> CellValue {
    wb.sheets[0]
        .cell(CellRef::parse(a1).unwrap())
        .map(|c| c.value.clone())
        .unwrap_or_default()
}

fn formula(wb: &rcell::Workbook, a1: &str) -> Option<String> {
    wb.sheets[0]
        .cell(CellRef::parse(a1).unwrap())
        .and_then(|c| c.formula.as_ref())
        .map(|f| f.text.clone())
}

#[test]
fn csv_import_values() {
    let text = "이름,수량,단가,합계\r\n사과,3,\"1,200\",=B2*C2\r\n\"배, 큰 것\",2,800,=B3*C3\r\n\"따옴표 \"\"안\"\"\",+1+2,1/2,TRUE\r\n";
    let wb = csv::read(text.as_bytes(), Delimiter::Comma, "판매").unwrap();
    assert_eq!(wb.sheets[0].name, "판매");
    assert_eq!(at(&wb, "A1"), CellValue::Text("이름".into()));
    assert_eq!(at(&wb, "B2"), CellValue::Number(3.0));
    // 따옴표 안 천 단위 쉼표는 수 (표시 형식 #,##0)
    assert_eq!(at(&wb, "C2"), CellValue::Number(1200.0));
    assert_eq!(formula(&wb, "D2").as_deref(), Some("B2*C2"));
    assert_eq!(at(&wb, "D2"), CellValue::Number(3600.0));
    assert_eq!(at(&wb, "A3"), CellValue::Text("배, 큰 것".into()));
    assert_eq!(at(&wb, "A4"), CellValue::Text("따옴표 \"안\"".into()));
    // `+` 로 시작하는 글은 수식
    assert_eq!(formula(&wb, "B4").as_deref(), Some("1+2"));
    assert_eq!(at(&wb, "D4"), CellValue::Bool(true));
}

#[test]
fn csv_import_encodings() {
    // UTF-8 BOM, UTF-16LE BOM, 코드 페이지 949 모두 같은 글
    let s = "가나,다\r\n";
    let mut utf8 = vec![0xEF, 0xBB, 0xBF];
    utf8.extend_from_slice(s.as_bytes());
    let mut utf16 = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        utf16.extend_from_slice(&u.to_le_bytes());
    }
    let cp949 = [0xB0, 0xA1, 0xB3, 0xAA, b',', 0xB4, 0xD9, b'\r', b'\n'];
    for bytes in [s.as_bytes().to_vec(), utf8, utf16, cp949.to_vec()] {
        assert_eq!(csv::decode(&bytes), s);
    }
}

#[test]
fn csv_tab_and_space_delimiters() {
    assert_eq!(
        Delimiter::from_path(Path::new("a.csv")),
        Some(Delimiter::Comma)
    );
    assert_eq!(
        Delimiter::from_path(Path::new("a.TXT")),
        Some(Delimiter::Tab)
    );
    assert_eq!(
        Delimiter::from_path(Path::new("a.prn")),
        Some(Delimiter::Space)
    );
    let wb = csv::read("1\t2\ta b\r\n".as_bytes(), Delimiter::Tab, "").unwrap();
    assert_eq!(wb.sheets[0].name, "Sheet1");
    assert_eq!(at(&wb, "C1"), CellValue::Text("a b".into()));
}

#[test]
fn csv_export_bytes() {
    let mut wb = rcell::edit::new_workbook();
    for (a1, input) in [
        ("B1", "제목"),
        ("A2", "1500"),
        ("B2", "1,500"),
        ("C2", "a,b"),
        ("D2", "say \"hi\""),
        ("A3", "=A2*2"),
        ("C3", "50%"),
    ] {
        rcell::edit::set_input(&mut wb, 0, CellRef::parse(a1).unwrap(), input).unwrap();
    }
    rcell::calc::recalculate(&mut wb);
    // CSV 는 A 열부터, 텍스트는 쓰인 첫 열부터. 줄 끝 CRLF, 표시된 글 그대로
    let csv_bytes = csv::write(&wb, Delimiter::Comma).unwrap();
    let mut want = b",\xC1\xA6\xB8\xF1,,\r\n".to_vec();
    want.extend_from_slice(b"1500,\"1,500\",\"a,b\",\"say \"\"hi\"\"\"\r\n3000,,50%,\r\n");
    assert_eq!(csv_bytes, want);
    let text = csv::write(&wb, Delimiter::Tab).unwrap();
    let mut want = b"\t\xC1\xA6\xB8\xF1\t\t\r\n".to_vec();
    want.extend_from_slice(b"1500\t1,500\ta,b\t\"say \"\"hi\"\"\"\r\n3000\t\t50%\t\r\n");
    assert_eq!(text, want);
    // 다시 읽으면 같은 값
    let back = csv::read(&csv_bytes, Delimiter::Comma, "Sheet1").unwrap();
    assert_eq!(at(&back, "B2"), CellValue::Number(1500.0));
    assert_eq!(at(&back, "D2"), CellValue::Text("say \"hi\"".into()));
    assert_eq!(at(&back, "C3"), CellValue::Number(0.5));
}

/// BIFF 레코드 (종류, 내용) 목록
fn records(stream: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 4 <= stream.len() {
        let rt = u16::from_le_bytes([stream[p], stream[p + 1]]);
        let len = u16::from_le_bytes([stream[p + 2], stream[p + 3]]) as usize;
        let end = (p + 4 + len).min(stream.len());
        out.push((rt, &stream[p + 4..end]));
        p = end;
    }
    out
}

fn sample() -> rcell::Workbook {
    let mut wb = rcell::edit::new_workbook();
    for (a1, input) in [
        ("A1", "품목"),
        ("B1", "금액"),
        ("A2", "사과"),
        ("B2", "1200"),
        ("A3", "배"),
        ("B3", "800"),
        ("B4", "=SUM(B2:B3)"),
        ("C2", "2026-10-28"),
        ("D2", "12.5%"),
        ("E2", "=IF(B2>1000,\"큼\",\"작음\")"),
    ] {
        rcell::edit::set_input(&mut wb, 0, CellRef::parse(a1).unwrap(), input).unwrap();
    }
    rcell::calc::recalculate(&mut wb);
    wb
}

#[test]
fn xls_write_is_biff8() {
    let wb = sample();
    let (bytes, _) = rcell::legacy::write_xls(&wb, &Default::default()).unwrap();
    let stream = rcell::container::read_entry(&bytes, "/Workbook").unwrap();
    assert!(rcell::legacy::xls::is_biff8(&stream));
    let recs = records(&stream);
    // BOF: BIFF8 (0x0600), 통합 문서 전역
    assert_eq!(recs[0].0, 0x0809);
    assert_eq!(&recs[0].1[..4], &[0x00, 0x06, 0x05, 0x00]);
    // 한컴 전용 레코드가 없다
    let hancom = [
        0xA030u16, 0x0900, 0xA001, 0xA011, 0xA020, 0xB002, 0x7EF0, 0x7EF8,
    ];
    assert!(recs.iter().all(|(rt, _)| !hancom.contains(rt)));
    // XF 는 20 바이트, 그 뒤 XFCRC (0x087C)
    let xfs: Vec<_> = recs.iter().filter(|(rt, _)| *rt == 0x00E0).collect();
    assert!(xfs.len() >= 16 && xfs.iter().all(|(_, d)| d.len() == 20));
    assert!(recs.iter().any(|(rt, _)| *rt == 0x087C));
    // 행 번호는 u16: DIMENSIONS 는 첫 행 u32·끝 행 u32·첫 열·끝 열 (BIFF8 도 u32 행), ROW 는 16 바이트
    assert!(
        recs.iter()
            .filter(|(rt, _)| *rt == 0x0208)
            .all(|(_, d)| d.len() == 16)
    );
    // SST 에 글 넷 (품목, 금액, 사과, 배)
    let sst = recs.iter().find(|(rt, _)| *rt == 0x00FC).unwrap().1;
    assert_eq!(u32::from_le_bytes(sst[4..8].try_into().unwrap()), 4);
}

#[test]
fn xls_round_trip() {
    let wb = sample();
    let (bytes, _) = rcell::legacy::write_xls(&wb, &Default::default()).unwrap();
    let back = rcell::from_bytes(bytes).unwrap();
    assert_eq!(back.application.as_deref(), Some("Excel"));
    for a1 in ["A1", "B1", "A2", "B2", "A3", "B3", "B4", "C2", "D2", "E2"] {
        assert_eq!(at(&back, a1), at(&wb, a1), "{a1}");
    }
    assert_eq!(formula(&back, "B4").as_deref(), Some("SUM(B2:B3)"));
    assert_eq!(
        formula(&back, "E2").as_deref(),
        Some("IF(B2>1000,\"큼\",\"작음\")")
    );
    let opts = rcell::compare::Options {
        ignore_hancom_ext: true,
        ..Default::default()
    };
    let diffs = rcell::compare::compare(&wb, &back, opts);
    assert!(diffs.is_empty(), "{}", diffs.join("\n"));
}

fn files_with_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.retain(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !n.starts_with("~$") && n.ends_with(suffix))
    });
    files.sort();
    files
}

/// 차트가 아닌 도형 수 (묶음 안 포함)
fn plain_shapes(s: &[rcell::model::Shape]) -> usize {
    s.iter()
        .filter(|x| x.chart.is_none())
        .map(|x| 1 + plain_shapes(&x.children))
        .sum()
}

/// `.xls` 가 담지 못하는 것 (한셀의 `.xls` 도 같다): (문서, 차이 머리의 끝)
const XLS_LOSSES: &[(&str, &str)] = &[
    // BIFF8 CF 에는 차등 서식 번호가 없어 서식 없는 규칙도 빈 차등 서식이 된다 (한셀이 .xls 를 열 때와 같다)
    ("cf_dv", "조건부 서식"),
    // 차등 서식의 안쪽 가로·세로 선은 레거시 DXFN 의 한컴 확장에만 있다
    ("dxfprobe", "조건부 서식"),
    // 값 목록 필터의 빈 칸 (레거시 0x009F 종류 0x0C)
    ("filter2", "자동 필터"),
    // 실선이 아닌 한컴 외곽선·윗줄은 FONT 의 한컴 확장에만 있다
    ("fonts_crafted", "서식"),
    // 탭 색은 팔레트 번호로만 (SHEETEXT 20바이트)
    ("view_print", "시트 속성"),
    // HLINK 의 표시 이름은 .xls 를 열 때 도구 설명이 된다 (한셀의 .xls 저장은 도구 설명을 그 자리에 쓰고, 한셀은
    // 그 자리를 도구 설명으로 읽는다)
    ("structure", "하이퍼링크"),
    ("view_print", "하이퍼링크"),
];

/// 말뭉치의 한셀 레거시 문서를 `.xls` 로 바꿔 읽은 결과가 레거시를 읽은 결과와 같다 (차트·도형 위치 밖,
/// [`XLS_LOSSES`] 밖). `.xls` 는 행 65536·열 256 까지라 그 밖에 칸이 있는 문서는 건너뛴다. 그리기 앵커의
/// 가로·세로 오프셋은 BIFF8 이 1/32 정밀도라 1 픽셀까지 어긋날 수 있어 도형은 (차트가 아닌 것의) 개수만 본다
#[test]
fn corpus_xls_matches_legacy() {
    let Some(root) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let opts = rcell::compare::Options {
        ignore_hancom_ext: true,
        ignore_chart: true,
        ..Default::default()
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for dir in [root.clone(), root.join("craft"), root.join("templates")] {
        for legacy in files_with_suffix(&dir, ".legacy.cell") {
            let bytes = std::fs::read(&legacy).unwrap();
            let mut a = rcell::from_bytes(bytes.clone()).expect("레거시 읽기");
            let beyond = a
                .sheets
                .iter()
                .any(|s| s.cells.iter().any(|c| c.r.row >= 65536 || c.r.col >= 256));
            if beyond {
                eprintln!("--  {} — BIFF8 범위 밖", legacy.display());
                continue;
            }
            let xls = match rcell::legacy::xls_from_legacy_file(&bytes) {
                Ok(x) => x,
                Err(e) => {
                    failures.push(format!("{}: 쓰기 {e}", legacy.display()));
                    continue;
                }
            };
            let mut b = match rcell::from_bytes(xls) {
                Ok(b) => b,
                Err(e) => {
                    failures.push(format!("{}: 읽기 {e}", legacy.display()));
                    continue;
                }
            };
            checked += 1;
            for (sa, sb) in a.sheets.iter_mut().zip(b.sheets.iter_mut()) {
                let (na, nb) = (plain_shapes(&sa.shapes), plain_shapes(&sb.shapes));
                if na != nb {
                    failures.push(format!(
                        "{}: {} 도형 {na} → {nb}",
                        legacy.display(),
                        sa.name
                    ));
                }
                sa.shapes.clear();
                sb.shapes.clear();
            }
            b.application = a.application.clone();
            let stem = legacy
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .trim_end_matches(".legacy.cell");
            let diffs: Vec<String> = rcell::compare::compare(&a, &b, opts)
                .into_iter()
                .filter(|d| {
                    let head = d.lines().next().unwrap_or("");
                    !XLS_LOSSES
                        .iter()
                        .any(|(f, cat)| *f == stem && head.ends_with(cat))
                })
                .collect();
            if !diffs.is_empty() {
                failures.push(format!(
                    "{}:\n  {}",
                    legacy.display(),
                    diffs
                        .iter()
                        .take(8)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n  ")
                ));
            }
        }
    }
    eprintln!("레거시 ↔ xls: {checked}개");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
