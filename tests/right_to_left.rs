// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 오른쪽에서 왼쪽 시트: 보기 속성이 저장·읽기를 거쳐도 남고, 렌더러는 표를 거울로 그린다.

mod common;

use rcell::writer::{Flavor, write};

#[test]
fn right_to_left_sheet_roundtrips_and_mirrors() {
    let mut a = rcell::from_bytes(common::hancom_style_cell()).unwrap();
    assert!(!a.sheets[0].view.right_to_left);
    let ltr = rcell::render::sheet_table(&a, &a.sheets[0], Default::default());
    assert!(!ltr.contains("rc-rtl\""));
    a.sheets[0].view.right_to_left = true;
    let b = rcell::from_bytes(write(&a, Flavor::Cell).unwrap()).unwrap();
    assert!(b.sheets[0].view.right_to_left);
    let html = rcell::render::sheet_table(&b, &b.sheets[0], Default::default());
    assert!(html.contains("rc-rtl\""));
    // 안쪽 내용은 한 번 더 뒤집어 바로 세운다
    assert!(html.contains("class=\"rc-flip\""));
}
