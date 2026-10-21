// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 저작자 표기 검사.
//!
//! 모든 소스 파일 머리말에 저작자(yoomin53)와 라이선스(SPDX)가 있어야 한다.
//! 새 파일을 추가할 때 머리말을 빠뜨리면 `cargo test` 가 실패한다.

use std::fs;
use std::path::{Path, PathBuf};

const AUTHOR: &str = "Copyright 2026 yoomin53";
const SPDX: &str = "SPDX-License-Identifier: Apache-2.0";
const CHECKED_EXT: &[&str] = &["rs", "toml", "ts", "js", "py", "ps1", "sh"];
const SKIP_DIRS: &[&str] = &["target", "target-dev", ".git", "node_modules", "pkg"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            // 숨은 폴더(.git, 도구가 만든 작업 폴더 등)는 소스 트리가 아니다 (.github 에는 검사 대상 파일이 없다)
            if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                walk(&p, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| CHECKED_EXT.contains(&x))
            && name != "Cargo.lock"
        {
            out.push(p);
        }
    }
}

#[test]
fn every_source_file_credits_the_author() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    walk(root, &mut files);
    assert!(!files.is_empty());
    let missing: Vec<String> = files
        .iter()
        .filter(|p| {
            let head: String = fs::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .take(6)
                .collect::<Vec<_>>()
                .join("\n");
            !(head.contains(AUTHOR) && head.contains(SPDX))
        })
        .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "저작자 머리말이 없는 파일:\n  {}\n\n각 파일 맨 위에 다음 두 줄을 넣으세요:\n  {AUTHOR} (https://github.com/yoomin53)\n  {SPDX}",
        missing.join("\n  ")
    );
}

#[test]
fn notice_and_manifest_credit_the_author() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let notice = fs::read_to_string(root.join("NOTICE")).expect("NOTICE 파일");
    assert!(notice.contains("yoomin53"));
    assert_eq!(
        env!("CARGO_PKG_AUTHORS"),
        "yoomin53 (https://github.com/yoomin53)"
    );
    assert_eq!(env!("CARGO_PKG_LICENSE"), "Apache-2.0");
}
