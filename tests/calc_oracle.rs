// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 함수 대조 실험: 계산 엔진을 한셀이 계산한 값과 함수마다 비교한다.
//!
//! `examples/calc_oracle_gen` 이 함수 × 인수 경우마다 수식 셀 하나를 둔 문서를 만들고,
//! `tools/probe/calc-oracle.ps1` 이 한셀로 다시 계산(CalculateFull)해 저장한다. 그 폴더를
//! `RCELL_CALC_ORACLE` 로 주면 경우 시트(`T`)의 수식 셀마다 rcell 의 계산과 한셀의 값을 비교한다
//! (숫자는 상대 오차 1e-12, 글·논리·오류 종류는 그대로). `RCELL_CALC_ORACLE_DUMP=<파일>` 이면 모든
//! 경우(문서, 칸, 수식, rcell 값, 한셀 값, 같음)를 탭으로 나눈 줄로 적는다 (근사식 찾기 실험용).
//!
//! ```text
//! cargo run --release --example calc_oracle_gen -- <입력 폴더>
//! powershell -File tools\probe\calc-oracle.ps1 -InDir <입력 폴더> -OutDir <결과 폴더>
//! RCELL_CALC_ORACLE=<결과 폴더> cargo test --release --test calc_oracle -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use rcell::calc::{CalcOptions, CellStatus};
use rcell::model::CellValue;

/// 경우가 시험하는 함수: 앞에서부터 처음 만나는 함수 (SUM·INDEX·ROWS 처럼 감싸는 함수는 그 안의 함수)
fn case_function(formula: &str) -> String {
    let Ok(e) = rcell::calc::parse(formula) else {
        return "(해석 실패)".into();
    };
    let mut names = Vec::new();
    e.visit_calls(&mut |c| names.push(c.name.clone()));
    const WRAPPERS: &[&str] = &["SUM", "INDEX", "ROWS", "COLUMNS"];
    names
        .iter()
        .find(|n| !WRAPPERS.contains(&n.as_str()))
        .or(names.first())
        .cloned()
        .unwrap_or_else(|| "(연산자)".into())
}

fn close(a: &CellValue, b: &CellValue, rel: f64) -> bool {
    match (a, b) {
        (CellValue::Number(x), CellValue::Number(y)) => (x - y).abs() <= rel * x.abs().max(y.abs()),
        _ => false,
    }
}

#[test]
fn functions_match_hancell() {
    let Some(dir) = std::env::var_os("RCELL_CALC_ORACLE").map(PathBuf::from) else {
        eprintln!("RCELL_CALC_ORACLE 미설정 — 건너뜀");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("대조 결과 폴더")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("cell"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "대조 결과 문서가 없음");
    // 함수 → (경우 수, 일치 수, 1e-9 안)
    let mut by_fn: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    // 함수 → 한셀이 #NAME? 을 낸 경우 수 (모두 #NAME? 이면 한셀이 모르는 함수)
    let mut hcell_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    let (mut total, mut matched, mut near) = (0usize, 0usize, 0usize);
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    // RCELL_CALC_ORACLE_DUMP=<파일>: 모든 경우를 탭으로 나눈 줄로 적는다 (문서, 칸, 수식, rcell, 한셀, 같음)
    let dump_path = std::env::var_os("RCELL_CALC_ORACLE_DUMP").map(PathBuf::from);
    let mut dump = String::new();
    for f in &files {
        let book = f.file_stem().unwrap().to_string_lossy().into_owned();
        let wb = rcell::open(f).expect("대조 문서 읽기");
        let Some(t) = wb.sheets.iter().position(|s| s.name == "T") else {
            continue;
        };
        // 한셀은 대조 입력 폴더의 같은 이름 문서를 열었다 (CELL("address") 의 [파일 이름]).
        // 계산 시각은 한셀이 저장한 시각 (연도 없는 날짜 글 `1/15` 의 해)
        let saved = std::fs::metadata(f)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64() / 86_400.0 + 25_569.0 + 9.0 / 24.0);
        let opts = CalcOptions {
            now: saved,
            file_name: f.file_name().map(|n| n.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let (results, report) = rcell::calc::evaluate(&wb, &opts);
        for (k, n) in report.unsupported_functions {
            *unsupported.entry(k).or_default() += n;
        }
        for r in results.iter().filter(|r| r.sheet == t) {
            if r.status == CellStatus::Volatile {
                continue;
            }
            total += 1;
            let func = case_function(&r.formula);
            // XML 은 글 안의 CR 을 LF 로 읽는다 (CHAR(13) 이 파일에서는 "\n") — 그 차이는 같다고 본다
            let ok = rcell::calc::matches_cached(&wb, r).unwrap_or(false)
                || matches!((&r.value, &r.cached), (CellValue::Text(a), CellValue::Text(b))
                    if a.replace("\r\n", "\n").replace('\r', "\n") == *b);
            let nearly = ok || close(&r.value, &r.cached, 1e-9);
            if dump_path.is_some() {
                dump += &format!(
                    "{book}\t{}\t{}\t{:?}\t{:?}\t{}\n",
                    r.cell,
                    r.formula,
                    r.value,
                    r.cached,
                    if ok { 1 } else { 0 }
                );
            }
            if matches!(&r.cached, CellValue::Error(e) if e == "#NAME?") {
                *hcell_name.entry(func.clone()).or_default() += 1;
            }
            let s = by_fn.entry(func.clone()).or_default();
            s.0 += 1;
            if ok {
                s.1 += 1;
                matched += 1;
            }
            if nearly {
                s.2 += 1;
                near += 1;
            }
            if !ok {
                failures.push((
                    func,
                    format!(
                        "{book} {}  ={}  rcell {:?}  한셀 {:?}{}",
                        r.cell,
                        r.formula,
                        r.value,
                        r.cached,
                        if nearly { "  (1e-9 안)" } else { "" }
                    ),
                ));
            }
        }
    }
    if let Some(p) = &dump_path {
        std::fs::write(p, &dump).expect("RCELL_CALC_ORACLE_DUMP 쓰기");
    }
    eprintln!(
        "대조 경우 {total}: 일치 {matched} ({:.2}%), 1e-9 안 {near}, 다름 {}",
        100.0 * matched as f64 / total.max(1) as f64,
        total - matched
    );
    let full = by_fn.values().filter(|s| s.0 == s.1).count();
    eprintln!("함수 {}개 중 모든 경우가 같은 함수 {full}개", by_fn.len());
    if !unsupported.is_empty() {
        eprintln!("구현하지 않은 함수: {unsupported:?}");
    }
    let unknown: Vec<&str> = by_fn
        .iter()
        .filter(|(k, s)| hcell_name.get(*k) == Some(&s.0))
        .map(|(k, _)| k.as_str())
        .collect();
    eprintln!(
        "한셀이 모르는 함수 (모든 경우가 #NAME?, {}개): {}",
        unknown.len(),
        unknown.join(" ")
    );
    let partial: Vec<String> = by_fn
        .iter()
        .filter(|(_, s)| s.0 != s.1)
        .map(|(k, s)| format!("{k} {}/{}", s.1, s.0))
        .collect();
    if !partial.is_empty() {
        eprintln!(
            "다른 경우가 있는 함수 ({}): {}",
            partial.len(),
            partial.join(", ")
        );
    }
    let limit: usize = std::env::var("RCELL_CALC_ORACLE_SHOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    for (_, line) in failures.iter().take(limit) {
        eprintln!("  {line}");
    }
    assert!(
        failures.is_empty(),
        "한셀과 다른 경우 {}개 (위 목록)",
        failures.len()
    );
}
