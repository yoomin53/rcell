// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 표시 형식 엔진을 한셀의 실제 표시 결과와 대조한다.
//!
//! `tools/probe/numfmt-oracle.ps1 -Out <경로>` 로 만든 `<경로>.cell` 과 `<경로>.tsv` 가 필요하다.
//!
//! ```text
//! RCELL_NUMFMT_ORACLE=C:\rcell-corpus\numfmt cargo test --test numfmt_oracle -- --nocapture
//! ```

use std::collections::HashMap;

#[test]
fn matches_hancell_display_text() {
    let Some(prefix) = std::env::var_os("RCELL_NUMFMT_ORACLE") else {
        eprintln!("RCELL_NUMFMT_ORACLE 미설정 — 건너뜀");
        return;
    };
    let prefix = prefix.to_string_lossy().into_owned();
    let wb = rcell::open(format!("{prefix}.cell")).expect("정답 .cell");
    let tsv = std::fs::read_to_string(format!("{prefix}.tsv")).expect("정답 .tsv");
    let expected: HashMap<String, String> = tsv
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, t)| (a.to_string(), t.to_string()))
        .collect();
    let sheet = &wb.sheets[0];
    let mut fails = Vec::new();
    let mut checked = 0;
    for cell in &sheet.cells {
        let addr = cell.r.to_string();
        let Some(want) = expected.get(&addr) else {
            continue;
        };
        let xf = wb.styles.xf(cell.style).expect("셀 서식");
        let code = rcell::numfmt::code_for(&wb.styles, xf.num_fmt_id);
        let got = rcell::numfmt::format(&cell.value, &code, wb.date1904);
        checked += 1;
        let ok = if !want.is_empty() && want.chars().all(|c| c == '#') {
            got.overflow
        } else {
            got.text == *want
        };
        if !ok {
            fails.push(format!(
                "{addr} [{code}] {:?} → {:?} (한셀: {want:?})",
                cell.value, got.text
            ));
        }
    }
    eprintln!("비교 {checked}개, 불일치 {}개", fails.len());
    for f in fails.iter().take(80) {
        eprintln!("  {f}");
    }
    assert!(fails.is_empty());
}
