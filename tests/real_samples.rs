// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 실제 한셀 파일 회귀 테스트.
//!
//! 한셀로 만든 파일은 저장소에 넣지 않으므로, 로컬 폴더를 환경 변수로 지정해 실행한다.
//!
//! ```text
//! RCELL_SAMPLES=C:\samples cargo test --test real_samples -- --nocapture
//! RCELL_CORPUS=C:\rcell-corpus cargo test --test real_samples -- --nocapture
//! ```
//!
//! `RCELL_CORPUS` 는 `tools/probe/make-corpus.ps1` 로 만든 폴더다. `X.legacy.cell` 과
//! 한셀이 그 파일을 읽어 XML 로 다시 저장한 `X.legacy.as.cell` 쌍을 비교해,
//! rcell 의 레거시 해석이 한셀의 해석과 같은지 확인한다.

use std::path::{Path, PathBuf};

fn files_with_suffix(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("폴더")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            // `~$…` 는 한셀이 문서를 연 동안 만드는 잠금 파일이다
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.starts_with("~$") && n.to_ascii_lowercase().ends_with(suffix))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn every_local_sample_parses() {
    let Some(dir) = std::env::var_os("RCELL_SAMPLES").map(PathBuf::from) else {
        eprintln!("RCELL_SAMPLES 미설정 — 건너뜀");
        return;
    };
    let mut failures = Vec::new();
    for f in files_with_suffix(&dir, ".cell") {
        match rcell::open(&f) {
            Ok(wb) => eprintln!(
                "OK  {} — 시트 {}, 셀 {}",
                f.display(),
                wb.sheets.len(),
                wb.sheets.iter().map(|s| s.cells.len()).sum::<usize>()
            ),
            Err(rcell::Error::Encrypted) => eprintln!("--  {} — 암호화 (미지원)", f.display()),
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    assert!(failures.is_empty(), "실패:\n{}", failures.join("\n"));
}

#[test]
fn legacy_reading_matches_hancell() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    // 한셀은 레거시의 상대 크기·장평 등과 도형 안 글을 읽지 못한다 (docs/format/cell-legacy.md).
    // 차트(0x7EF0)는 내용까지 비교한다 (docs/format/cell-legacy-chart.md 9절)
    let opts = rcell::compare::Options {
        ignore_font_scale: true,
        ignore_shape_detail: true,
        // 한셀의 레거시 조건부 서식·자동 필터 읽기는 틀리거나 값을 잃는다
        // → 아래 legacy_cf_matches_xml_source 에서 원본과 비교
        ignore_cf: true,
        ignore_filter: true,
        // 한셀은 레거시의 사용자 정의 표 스타일을 읽지 않는다
        ignore_table_styles: true,
        ..Default::default()
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    // craft: 조작한 XML 을 한셀로 바꾼 것, templates: 한셀 서식 파일 (batch-formats.ps1),
    // craft-charts*: 차트 대조 실험
    for dir in [
        dir.clone(),
        dir.join("craft"),
        dir.join("templates"),
        dir.join("craft-charts"),
        dir.join("craft-charts2"),
    ] {
        if !dir.is_dir() {
            continue;
        }
        for legacy in files_with_suffix(&dir, ".legacy.cell") {
            let name = legacy.file_name().unwrap().to_string_lossy().into_owned();
            let truth = legacy.with_file_name(name.replace(".legacy.cell", ".legacy.as.cell"));
            if !truth.exists() {
                continue;
            }
            let a = rcell::open(&legacy).expect("레거시 읽기");
            let b = rcell::open(&truth).expect("한셀 XML 읽기");
            let diffs = rcell::compare::compare(&a, &b, opts);
            checked += 1;
            eprintln!("{:>4} 차이  {}", diffs.len(), legacy.display());
            if !diffs.is_empty() {
                failures.push(format!("{}:\n{}", legacy.display(), diffs.join("\n")));
            }
        }
    }
    eprintln!("비교한 쌍: {checked}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// 레거시의 조건부 서식·데이터 유효성·자동 필터·사용자 정의 표 스타일은 같은 문서의 XML 원본(X.cell)과 비교한다.
#[test]
fn legacy_cf_matches_xml_source() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let mut failures = Vec::new();
    for dir in [dir.clone(), dir.join("craft"), dir.join("templates")] {
        if !dir.is_dir() {
            continue;
        }
        for legacy in files_with_suffix(&dir, ".legacy.cell") {
            let name = legacy.file_name().unwrap().to_string_lossy().into_owned();
            let twin = legacy.with_file_name(name.replace(".legacy.cell", ".cell"));
            if !twin.exists() {
                continue;
            }
            let a = rcell::open(&legacy).expect("레거시 읽기");
            let b = rcell::open(&twin).expect("XML 원본 읽기");
            let diffs: Vec<String> = rcell::compare::compare(&a, &b, Default::default())
                .into_iter()
                .filter(|d| {
                    d.contains("조건부 서식")
                        || d.contains("유효성")
                        || d.contains("자동 필터")
                        // 한셀은 레거시의 사용자 정의 표 스타일을 읽지 않는다 → XML 원본만 정답이 된다
                        || d.starts_with("표 스타일")
                })
                .collect();
            if !diffs.is_empty() {
                failures.push(format!("{}:\n{}", legacy.display(), diffs.join("\n")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn binary_reading_matches_xml_twin() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    // 바이너리 OOXML(xlCellb)은 한컴 확장을 저장하지 않는다. 한셀이 이 형식을 COM 으로
    // 다시 열지 못하므로, 같은 문서를 XML 로 저장한 쌍둥이 파일을 정답으로 쓴다.
    let opts = rcell::compare::Options {
        ignore_hancom_ext: true,
        ..Default::default()
    };
    // craft 는 넣지 않는다: 한셀의 바이너리 저장이 일부 값을 잃는다 (docs/format/cell-binary.md)
    let mut failures = Vec::new();
    let bins = files_with_suffix(&dir, ".bin.cell").into_iter().chain(
        Some(dir.join("templates"))
            .filter(|d| d.is_dir())
            .into_iter()
            .flat_map(|d| files_with_suffix(&d, ".bin.cell")),
    );
    for bin in bins {
        let name = bin.file_name().unwrap().to_string_lossy().into_owned();
        let twin = bin.with_file_name(name.replace(".bin.cell", ".cell"));
        if !twin.exists() {
            continue;
        }
        let a = rcell::open(&bin).expect("바이너리 읽기");
        let b = rcell::open(&twin).expect("XML 읽기");
        // 한셀은 바이너리에 외부 연결의 상태(경로 없음, 값 캐시)를 쓰지 않는다 (docs/format/cell-binary.md)
        let diffs: Vec<String> = rcell::compare::compare(&a, &b, opts)
            .into_iter()
            .filter(|d| !d.starts_with("외부 연결 상태"))
            .collect();
        eprintln!("{:>4} 차이  {}", diffs.len(), bin.display());
        if !diffs.is_empty() {
            failures.push(format!("{}:\n{}", bin.display(), diffs.join("\n")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// 레거시 쓰기 왕복: rcell(X) → 레거시 → rcell 이 다시 읽은 것이, X 에 레거시의 손실(`legacy::lossy::legacy_view`:
/// 창 잠금, 요약 위치, 외부 값 캐시, 줄 간격 atLeast, HWPUNIT 여백, 머리글 기본 글꼴 코드, 도형 선 기본값,
/// 차트 내용)을 적용한 것과 같아야 한다. 입력은 말뭉치의 XML 원본(X.cell)과 한셀의 레거시(X.legacy.cell) 둘 다.
#[test]
fn legacy_writer_round_trip() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for base in [
        dir.clone(),
        dir.join("craft"),
        dir.join("templates"),
        dir.join("craft-charts"),
        dir.join("craft-charts2"),
    ] {
        if !base.is_dir() {
            continue;
        }
        for legacy in files_with_suffix(&base, ".legacy.cell") {
            let name = legacy.file_name().unwrap().to_string_lossy().into_owned();
            let xml = legacy.with_file_name(name.replace(".legacy.cell", ".cell"));
            for input in [xml, legacy] {
                if !input.exists() {
                    continue;
                }
                let a = rcell::open(&input).expect("원본 읽기");
                let bytes = rcell::legacy::write(&a).expect("레거시 쓰기");
                let b = rcell::from_bytes(bytes).expect("쓴 레거시 다시 읽기");
                let expected = rcell::legacy::lossy::legacy_view(&a);
                let diffs = rcell::compare::compare(&expected, &b, Default::default());
                checked += 1;
                eprintln!("{:>4} 차이  {}", diffs.len(), input.display());
                if !diffs.is_empty() {
                    failures.push(format!("{}:\n{}", input.display(), diffs.join("\n")));
                }
            }
        }
    }
    eprintln!("왕복한 문서: {checked}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// rcell 이 쓴 레거시를 한셀이 열어 XML 로 다시 저장한 것(HCell(rcell(X)))이 한셀이 자기 레거시를 읽은 것
/// (X.legacy.as.cell = HCell(HCell 레거시))과 같아야 한다. 한셀이 레거시에서 못 읽는 것은 레거시 판정과 같은
/// 옵션으로 뺀다. 차트 내용은 아직 쓰지 않으므로 정답에서 차트를 뺀다.
///
/// `RCELL_LEGACY_ORACLE` = rcell 이 쓴 `폴더_X.legacy.cell` 을 한셀로 다시 저장한 `폴더_X.legacy.as.cell` 이 있는 폴더
/// (`rcell convert X.cell 폴더_X.legacy.cell --format legacy` 뒤 `tools/probe/resave.ps1 -Filter *.legacy.cell`).
#[test]
fn legacy_writer_output_is_read_identically_by_hancell() {
    let (Some(dir), Some(oracle)) = (
        std::env::var_os("RCELL_CORPUS").map(PathBuf::from),
        std::env::var_os("RCELL_LEGACY_ORACLE").map(PathBuf::from),
    ) else {
        eprintln!("RCELL_CORPUS·RCELL_LEGACY_ORACLE 미설정 — 건너뜀");
        return;
    };
    let opts = rcell::compare::Options {
        ignore_font_scale: true,
        ignore_shape_detail: true,
        ignore_cf: true,
        ignore_filter: true,
        ignore_table_styles: true,
        ..Default::default()
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for resaved in files_with_suffix(&oracle, ".legacy.as.cell") {
        let name = resaved.file_name().unwrap().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".legacy.as.cell");
        let (sub, x) = ["craft-charts2_", "craft-charts_", "craft_", "templates_"]
            .iter()
            .find_map(|p| stem.strip_prefix(p).map(|x| (p.trim_end_matches('_'), x)))
            .unwrap_or(("", stem));
        let truth = dir.join(sub).join(format!("{x}.legacy.as.cell"));
        if !truth.exists() {
            continue;
        }
        let mut a = rcell::open(&truth).expect("한셀이 읽은 한셀 레거시");
        let b = rcell::open(&resaved).expect("한셀이 읽은 rcell 레거시");
        for s in &mut a.sheets {
            s.shapes
                .retain(|x| x.kind != rcell::model::ShapeKind::Chart);
        }
        let diffs = rcell::compare::compare(&a, &b, opts);
        checked += 1;
        eprintln!("{:>4} 차이  {}", diffs.len(), resaved.display());
        if !diffs.is_empty() {
            failures.push(format!("{}:\n{}", resaved.display(), diffs.join("\n")));
        }
    }
    eprintln!("비교한 쌍: {checked}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn writer_output_is_read_identically_by_hancell() {
    let Some(dir) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
        eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
        return;
    };
    // rcell-out/X.rcell.cell: rcell 이 쓴 파일, X.rcell.as.cell: 한셀이 그것을 열어 다시 저장한 파일
    // (tools/probe/resave.ps1 — 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 복사하므로 다시 쓰게 만든다).
    // 정답은 한셀이 원본을 읽은 결과다: 한셀은 저장할 때마다 일부 값을 스스로 바꾸므로(요약 열 위치,
    // 머리글 글꼴 코드 …) 원본 파일과 직접 비교하지 않고 HCell(원본) 과 HCell(rcell(원본)) 을 비교한다.
    //   XML 원본    → hs/X.hs.cell (원본을 한셀이 다시 저장한 것)
    //   레거시 원본 → X.legacy.as.cell (한셀의 레거시 읽기, 그 손실은 옵션으로 뺀다)
    //   바이너리    → hs/X.hs.cell (한셀이 같은 문서를 XML 로 다시 저장한 것; 한셀은 바이너리를 COM 으로
    //                 다시 열지 못한다). 한컴 확장·외부 연결 상태는 바이너리에 없다. craft 는 넣지 않는다:
    //                 한셀의 바이너리 저장이 조작한 값 일부를 잃는다 (docs/format/cell-binary.md)
    let legacy_opts = rcell::compare::Options {
        ignore_font_scale: true,
        ignore_shape_detail: true,
        ignore_cf: true,
        ignore_filter: true,
        ignore_table_styles: true,
        ..Default::default()
    };
    let bin_opts = rcell::compare::Options {
        ignore_hancom_ext: true,
        ..Default::default()
    };
    // RCELL_OUT 을 주면 `<폴더>/rcell-out` 대신 `RCELL_OUT/<폴더>` 의 결과를 쓴다 (말뭉치 밖에서 새로 만든 결과)
    let out_root = std::env::var_os("RCELL_OUT").map(PathBuf::from);
    let mut failures = Vec::new();
    let (mut checked, mut chart_stale, mut stale_total) = (0, 0, 0);
    for (sub, base) in [
        ("", dir.clone()),
        ("craft", dir.join("craft")),
        ("templates", dir.join("templates")),
        ("craft-charts", dir.join("craft-charts")),
        ("craft-charts2", dir.join("craft-charts2")),
    ] {
        let out = match &out_root {
            Some(r) => r.join(sub),
            None => base.join("rcell-out"),
        };
        if !out.is_dir() {
            continue;
        }
        for resaved in files_with_suffix(&out, ".rcell.as.cell") {
            let name = resaved.file_name().unwrap().to_string_lossy().into_owned();
            let stem = name.trim_end_matches(".rcell.as.cell");
            let (truth, opts) = if let Some(b) = stem.strip_suffix(".legacy") {
                (base.join(format!("{b}.legacy.as.cell")), legacy_opts)
            } else if let Some(b) = stem.strip_suffix(".bin") {
                if base.ends_with("craft") {
                    continue;
                }
                let hs = base.join("hs").join(format!("{b}.hs.cell"));
                (
                    if hs.exists() {
                        hs
                    } else {
                        base.join(format!("{b}.cell"))
                    },
                    bin_opts,
                )
            } else {
                (
                    base.join("hs").join(format!("{stem}.hs.cell")),
                    Default::default(),
                )
            };
            if !truth.exists() {
                continue;
            }
            let mut a = rcell::open(&truth).expect("한셀이 읽은 원본");
            let b = rcell::open(&resaved).expect("한셀 재저장본");
            // 레거시 차트(0x7EF0)를 쓰기 전의 rcell 이 만든 결과(rcell 출력에 차트 파트가 없음)는 차트를 빼고 본다
            let written = out.join(format!("{stem}.rcell.cell"));
            if stem.ends_with(".legacy") && !has_chart_parts(&written) && has_charts(&a) {
                chart_stale += 1;
                for s in &mut a.sheets {
                    s.shapes
                        .retain(|x| x.kind != rcell::model::ShapeKind::Chart);
                }
            }
            // 한셀 재저장본은 그것을 만들 때의 rcell 출력(X.rcell.cell)을 판정한다. 지금의 rcell 이 같은 원본에서
            // 다르게 쓰는 항목은 그 재저장본으로 판정할 수 없다 → 빼고 알린다 (rcell-out 을 다시 만들면 사라진다)
            let stale = stale_labels(&base.join(format!("{stem}.cell")), &written);
            let all = rcell::compare::compare(&a, &b, opts);
            let before = all.len();
            let diffs: Vec<String> = all
                .into_iter()
                .filter(|d| !(stem.ends_with(".bin") && d.starts_with("외부 연결 상태")))
                .filter(|d| !stale.contains(d.lines().next().unwrap_or("")))
                .collect();
            checked += 1;
            if !stale.is_empty() {
                stale_total += 1;
                eprintln!(
                    "     (재저장본이 지난 출력: {} 항목 제외, 차이 {before} 중)  {}",
                    stale.len(),
                    resaved.display()
                );
            }
            eprintln!("{:>4} 차이  {}", diffs.len(), resaved.display());
            if !diffs.is_empty() {
                failures.push(format!("{}:\n{}", resaved.display(), diffs.join("\n")));
            }
        }
    }
    eprintln!(
        "비교한 쌍: {checked} (차트를 쓰기 전의 레거시 결과 {chart_stale}개는 차트를 빼고 비교, 지난 출력으로 일부 항목을 뺀 쌍: {stale_total})"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn has_charts(wb: &rcell::Workbook) -> bool {
    wb.sheets.iter().any(|s| {
        s.shapes
            .iter()
            .any(|x| x.kind == rcell::model::ShapeKind::Chart)
    })
}

/// 패키지에 차트 파트(`xl/charts/`)가 있는지
fn has_chart_parts(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|b| rcell::Package::from_bytes(b).ok())
        .is_some_and(|p| p.part_names().iter().any(|n| n.starts_with("xl/charts/")))
}

/// 지금의 rcell 이 `source` 에서 쓰는 결과와 보관된 rcell 출력 `stored` 가 의미상 다른 비교 항목의 이름.
/// 파일이 없으면 빈 집합 (판정에서 뺄 것이 없다).
fn stale_labels(source: &Path, stored: &Path) -> std::collections::BTreeSet<String> {
    let (Ok(src), Ok(old)) = (rcell::open(source), rcell::open(stored)) else {
        return Default::default();
    };
    let bytes = rcell::writer::write(&src, rcell::writer::Flavor::Cell).expect("rcell 쓰기");
    let fresh = rcell::from_bytes(bytes).expect("rcell 출력 읽기");
    rcell::compare::compare(&old, &fresh, Default::default())
        .iter()
        .filter_map(|d| d.lines().next().map(str::to_string))
        .collect()
}
