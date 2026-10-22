// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 계산 엔진을 한셀의 계산 결과와 대조한다.
//!
//! 한셀이 저장한 파일의 수식 셀에는 한셀이 계산한 값(`<v>`, 레거시 FORMULA 결과, XLSB 캐시)이 있다.
//! 코퍼스의 모든 문서(세 형식)를 읽어 다시 계산하고, 수식 셀마다 계산 값과 캐시 값을 비교한다
//! (숫자는 상대 오차 1e-12 또는 셀 서식으로 같은 모양, 글·논리·오류 종류는 그대로).
//!
//! ```text
//! RCELL_CORPUS=C:\rcell-corpus cargo test --release --test calc_corpus -- --nocapture
//! ```
//!
//! 캐시 값을 믿을 수 없는 파일은 뺀다: 함수 번호 실험(`probe*`)의 입력은 rcell 이 `<v>0</v>` 를 채워
//! 만들었고, 한셀은 XML·바이너리를 저장할 때 다시 계산하지 않으므로(관측, `docs/format/calc.md`)
//! 그 0 이 그대로 남는다. 레거시로 저장한 파일도 계산 전의 값(0)을 쓴다. 한셀이 레거시를 다시 열어
//! XML 로 저장한 `*.legacy.as.cell` 은 다시 계산한 값이라 비교한다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rcell::calc::{CalcOptions, CellStatus};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cell"))
        {
            out.push(p);
        }
    }
}

/// 캐시 값이 한셀의 계산 결과가 아닌 파일 (위 설명)
fn untrusted(rel: &str) -> bool {
    let r = rel.replace('\\', "/");
    let first = r.split('/').next().unwrap_or("");
    if matches!(
        first,
        "probe-in" | "probe2-in" | "probe4-in" | "probe" | "probe2"
    ) {
        return true;
    }
    first == "probe3" && r.ends_with(".legacy.cell")
}

#[derive(Default)]
struct Totals {
    files: usize,
    unreadable: usize,
    formula_cells: usize,
    compared: usize,
    matched: usize,
    mismatched: usize,
    volatile_mismatched: usize,
    no_cache: usize,
    unsupported: usize,
}

#[test]
fn corpus_formulas_match_hancell() {
    let Some(root) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut t = Totals::default();
    let mut skipped_untrusted = 0usize;
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_fn: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let opts = CalcOptions::default();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
        if untrusted(&rel) {
            skipped_untrusted += 1;
            continue;
        }
        let wb = match rcell::open(f) {
            Ok(w) => w,
            Err(e) => {
                t.unreadable += 1;
                eprintln!("읽기 실패 {rel}: {e}");
                continue;
            }
        };
        t.files += 1;
        let (results, report) = rcell::calc::evaluate(&wb, &opts);
        t.formula_cells += results.len();
        for (k, v) in report.unsupported_functions {
            *unsupported.entry(k).or_default() += v;
        }
        for r in &results {
            if r.status == CellStatus::Unsupported || r.status == CellStatus::ParseError {
                t.unsupported += 1;
                continue;
            }
            let Some(ok) = rcell::calc::matches_cached(&wb, r) else {
                t.no_cache += 1;
                continue;
            };
            t.compared += 1;
            // 수식에 쓴 함수별 일치
            if let Ok(e) = rcell::calc::parse(&r.formula) {
                let mut names = Vec::new();
                e.visit_calls(&mut |c| names.push(c.name.clone()));
                names.sort();
                names.dedup();
                for n in names {
                    let s = by_fn.entry(n).or_default();
                    s.0 += 1;
                    if ok {
                        s.1 += 1;
                    }
                }
            }
            if ok {
                t.matched += 1;
            } else if r.status == CellStatus::Volatile {
                t.volatile_mismatched += 1;
            } else {
                t.mismatched += 1;
                let sheet = wb
                    .sheets
                    .get(r.sheet)
                    .map(|s| s.name.as_str())
                    .unwrap_or("?");
                failures.push(format!(
                    "{rel} {sheet}!{} ={}  계산 {:?}  한셀 {:?}",
                    r.cell, r.formula, r.value, r.cached
                ));
            }
        }
    }
    eprintln!(
        "파일 {} (읽기 실패 {}, 캐시를 믿을 수 없어 뺀 파일 {skipped_untrusted})",
        t.files, t.unreadable
    );
    eprintln!(
        "수식 셀 {}: 비교 {} — 일치 {} ({:.3}%), 불일치 {}, 휘발성 불일치 {}; 캐시 없음 {}, 지원 안 함 {}",
        t.formula_cells,
        t.compared,
        t.matched,
        100.0 * t.matched as f64 / t.compared.max(1) as f64,
        t.mismatched,
        t.volatile_mismatched,
        t.no_cache,
        t.unsupported
    );
    if !unsupported.is_empty() {
        eprintln!("지원하지 않는 함수: {unsupported:?}");
    }
    let partial: Vec<String> = by_fn
        .iter()
        .filter(|(_, (n, ok))| n != ok)
        .map(|(k, (n, ok))| format!("{k} {ok}/{n}"))
        .collect();
    if !partial.is_empty() {
        eprintln!("불일치가 있는 함수: {}", partial.join(", "));
    }
    for f in failures.iter().take(80) {
        eprintln!("  {f}");
    }
    assert!(
        failures.is_empty(),
        "한셀과 다른 수식 셀 {}개 (위 목록)",
        failures.len()
    );
}
