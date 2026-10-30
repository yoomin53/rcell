// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// (임시 — gen_adv2.js 가 만든다)

#[derive(Debug)]
pub struct Hinted {
    pub names: &'static [&'static str],
    pub sizes: &'static [u8],
    pub regular: &'static [[u8; 96]],
    pub bold: &'static [[u8; 96]],
    pub nat_regular: &'static [[u16; 96]],
    pub nat_bold: &'static [[u16; 96]],
}

pub static HINTED: [Hinted; 0] = [];
