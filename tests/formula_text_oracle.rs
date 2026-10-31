// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 수식 글 왕복 대조: 한셀이 입력 문서의 수식을 읽고 다시 저장한 글(`<f>`)과, rcell 이 같은 입력을
//! 한셀의 저장 꼴로 고른 글(`rcell::edit::normalize_formula`)을 칸마다 견준다.
//!
//! `examples/calc_oracle_gen` 이 만든 입력 폴더와 `tools/probe/calc-oracle.ps1` 이 만든 결과 폴더 (같은 이름의
//! 문서)를 쓴다 — 계산 대조 실험과 같은 자료라 수식 글 비교는 공짜다.
//!
//! ```text
//! RCELL_FORMULA_IN=<입력 폴더> RCELL_FORMULA_OUT=<결과 폴더> RCELL_FORMULA_DUMP=<파일> \
//!   cargo test --release --test formula_text_oracle -- --nocapture
//! ```
//!
//! `RCELL_FORMULA_DUMP` 이면 다른 경우를 `문서\t칸\t입력\t한셀\trcell` 줄로 적는다.

use std::path::PathBuf;

#[test]
fn formula_text_matches_hancell() {
    let (Some(din), Some(dout)) = (
        std::env::var_os("RCELL_FORMULA_IN").map(PathBuf::from),
        std::env::var_os("RCELL_FORMULA_OUT").map(PathBuf::from),
    ) else {
        eprintln!("RCELL_FORMULA_IN·RCELL_FORMULA_OUT 미설정 — 건너뜀");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dout)
        .expect("결과 폴더")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("cell"))
        })
        .collect();
    files.sort();
    let dump_path = std::env::var_os("RCELL_FORMULA_DUMP").map(PathBuf::from);
    let mut dump = String::new();
    let (mut total, mut same) = (0usize, 0usize);
    for f in &files {
        let name = f.file_name().unwrap();
        let Ok(win) = rcell::open(din.join(name)) else {
            continue;
        };
        let Ok(wout) = rcell::open(f) else {
            continue;
        };
        let book = f.file_stem().unwrap().to_string_lossy().into_owned();
        let (Some(ti), Some(to)) = (
            win.sheets.iter().position(|s| s.name == "T"),
            wout.sheets.iter().position(|s| s.name == "T"),
        ) else {
            continue;
        };
        for c in &win.sheets[ti].cells {
            let Some(fi) = c.formula.as_ref() else {
                continue;
            };
            let Some(fo) = wout.sheets[to].cell(c.r).and_then(|x| x.formula.as_ref()) else {
                continue;
            };
            total += 1;
            let mine = rcell::edit::normalize_formula(&win, &fi.text)
                .unwrap_or_else(|e| format!("(오류: {e})"));
            if mine == fo.text {
                same += 1;
            } else {
                dump.push_str(&format!(
                    "{book}\t{}\t{}\t{}\t{}\n",
                    c.r, fi.text, fo.text, mine
                ));
            }
        }
    }
    eprintln!(
        "수식 글 {total}: 일치 {same} ({:.2}%), 다름 {}",
        100.0 * same as f64 / total.max(1) as f64,
        total - same
    );
    if let Some(p) = dump_path {
        std::fs::write(p, dump).expect("덤프 쓰기");
    }
}

/// 입력 글 → 한셀의 `Formula` 대조: `tools/probe/formula-probe.ps1` 가 만든 탭 파일
/// (번호, 입력, Formula, FormulaLocal, FormulaR1C1, 오류) 를 읽어 rcell 이 입력을 고른 글과 견준다.
/// 한셀이 입력을 거부했으면 rcell 도 거부해야 한다. `RCELL_FORMULA_PROBE=<tsv>`,
/// 다른 줄은 `RCELL_FORMULA_DUMP` 에 `입력\t한셀\trcell` 로 적는다.
#[test]
fn typed_formula_text_matches_hancell() {
    use rcell::model::{DefinedName, Sheet, Workbook};
    let Some(tsv) = std::env::var_os("RCELL_FORMULA_PROBE").map(PathBuf::from) else {
        eprintln!("RCELL_FORMULA_PROBE 미설정 — 건너뜀");
        return;
    };
    let text = std::fs::read_to_string(&tsv).expect("탭 파일");
    // 한셀 시험 문서와 같은 시트·이름
    let mut wb = Workbook::default();
    for n in ["Sheet1", "Data", "S1", "S2"] {
        wb.sheets.push(Sheet {
            name: n.into(),
            ..Default::default()
        });
    }
    // 이름: 통합 문서 nm·nm2, 시트 범위 loc (Data), nm (S1) — tools/probe/formula-probe.ps1 과 같다
    for (n, f, sheet) in [
        ("nm", "Data!$A$1:$A$3", None),
        ("nm2", "S1!$B$2", None),
        ("loc", "Data!$B$1", Some(1)),
        ("nm", "S1!$C$3", Some(2)),
    ] {
        wb.defined_names.push(DefinedName {
            name: n.into(),
            formula: f.into(),
            local_sheet_id: sheet,
            ..Default::default()
        });
    }
    let dump_path = std::env::var_os("RCELL_FORMULA_DUMP").map(PathBuf::from);
    let mut dump = String::new();
    let (mut total, mut same) = (0usize, 0usize);
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 6 {
            continue;
        }
        let (input, hc, err) = (cols[1], cols[2], cols[5]);
        let typed = input.strip_prefix('=').unwrap_or(input);
        total += 1;
        let mine = rcell::edit::normalize_formula(&wb, typed);
        let ok = if !err.is_empty() {
            mine.is_err()
        } else {
            matches!(&mine, Ok(m) if format!("={m}") == hc)
        };
        if ok {
            same += 1;
        } else {
            let hc_show = if err.is_empty() {
                hc.to_string()
            } else {
                "(거부)".to_string()
            };
            let mine_show = match &mine {
                Ok(m) => format!("={m}"),
                Err(_) => "(거부)".to_string(),
            };
            dump.push_str(&format!("{input}\t{hc_show}\t{mine_show}\n"));
        }
    }
    eprintln!(
        "입력 수식 {total}: 일치 {same} ({:.2}%), 다름 {}",
        100.0 * same as f64 / total.max(1) as f64,
        total - same
    );
    if let Some(p) = dump_path {
        std::fs::write(p, dump).expect("덤프 쓰기");
    }
}
