// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 작은 OLE2 복합 문서(CFB) 쓰기 — 공개 명세 [MS-CFB] 버전 3 (512 바이트 섹터).
//!
//! 루트 저장소 바로 아래 스트림만 쓴다. 한컴 매크로 프로젝트 파트(`xl/vbsProject.bin`: 스트림
//! `_VBS_PROJECT_W` 하나)를 레거시 원본에서 만들 때 쓴다. 4096 바이트보다 작은 스트림은 미니 스트림에,
//! 나머지는 일반 섹터에 둔다. FAT 섹터 109개(약 6.8 MB)를 넘는 문서는 쓰지 않는다 (DIFAT 섹터 없음).

use crate::error::{Error, Result};

const SECTOR: usize = 512;
const MINI: usize = 64;
const CUTOFF: usize = 4096;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const NO_STREAM: u32 = 0xFFFF_FFFF;

fn bad(m: &str) -> Error {
    Error::InvalidValue {
        context: "OLE2(CFB) 쓰기".into(),
        value: m.into(),
    }
}

/// 디렉터리 이름 순서: 짧은 이름이 앞, 길이가 같으면 대문자로 바꾼 UTF-16 단위 순 ([MS-CFB] 2.6.4)
fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let ua: Vec<u16> = a.to_uppercase().encode_utf16().collect();
    let ub: Vec<u16> = b.to_uppercase().encode_utf16().collect();
    a.encode_utf16()
        .count()
        .cmp(&b.encode_utf16().count())
        .then(ua.cmp(&ub))
}

/// 정렬된 항목으로 균형 이진 트리를 만든다: (왼쪽, 오른쪽, 깊이). 마지막 덜 찬 층만 빨강으로 칠하면
/// 모든 경로의 검정 수가 같다 (적흑 트리 조건)
fn build_tree(sorted: &[usize], depth: u32, out: &mut [(u32, u32, u32)]) -> u32 {
    if sorted.is_empty() {
        return NO_STREAM;
    }
    let mid = sorted.len() / 2;
    let me = sorted[mid];
    let left = build_tree(&sorted[..mid], depth + 1, out);
    let right = build_tree(&sorted[mid + 1..], depth + 1, out);
    out[me] = (left, right, depth);
    me as u32
}

fn chain(fat: &mut [u32], start: usize, count: usize) {
    for k in 0..count {
        fat[start + k] = if k + 1 == count {
            END_OF_CHAIN
        } else {
            (start + k + 1) as u32
        };
    }
}

/// 루트 저장소 아래 스트림들로 복합 문서를 만든다. 이름은 31자까지.
pub fn write_streams(streams: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    for (name, _) in streams {
        if name.is_empty() || name.encode_utf16().count() > 31 {
            return Err(bad(name));
        }
    }
    // 미니 스트림 (작은 스트림)
    let mut mini = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut starts = vec![END_OF_CHAIN; streams.len()];
    for (i, (_, d)) in streams.iter().enumerate() {
        if d.len() >= CUTOFF || d.is_empty() {
            continue;
        }
        let first = mini_fat.len();
        let count = d.len().div_ceil(MINI);
        mini_fat.resize(first + count, 0);
        chain(&mut mini_fat, first, count);
        mini.extend_from_slice(d);
        mini.resize(mini_fat.len() * MINI, 0);
        starts[i] = first as u32;
    }
    // 섹터 배치: FAT, 디렉터리, 미니 FAT, 미니 스트림, 큰 스트림
    let entries = streams.len() + 1;
    let dir_n = (entries * 128).div_ceil(SECTOR);
    let minifat_n = (mini_fat.len() * 4).div_ceil(SECTOR);
    let ministream_n = mini.len().div_ceil(SECTOR);
    let large_n: usize = streams
        .iter()
        .filter(|(_, d)| d.len() >= CUTOFF)
        .map(|(_, d)| d.len().div_ceil(SECTOR))
        .sum();
    let data_n = dir_n + minifat_n + ministream_n + large_n;
    let mut fat_n = 1;
    while fat_n * (SECTOR / 4) < data_n + fat_n {
        fat_n += 1;
    }
    if fat_n > 109 {
        return Err(bad("FAT 섹터 109개 초과"));
    }
    let mut fat = vec![FREE; fat_n * (SECTOR / 4)];
    for f in fat.iter_mut().take(fat_n) {
        *f = FAT_SECT;
    }
    let mut next = fat_n;
    let dir_start = next;
    chain(&mut fat, next, dir_n);
    next += dir_n;
    let minifat_start = if minifat_n > 0 {
        next as u32
    } else {
        END_OF_CHAIN
    };
    chain(&mut fat, next, minifat_n);
    next += minifat_n;
    let ministream_start = if ministream_n > 0 {
        next as u32
    } else {
        END_OF_CHAIN
    };
    chain(&mut fat, next, ministream_n);
    next += ministream_n;
    for (i, (_, d)) in streams.iter().enumerate() {
        if d.len() >= CUTOFF {
            let n = d.len().div_ceil(SECTOR);
            starts[i] = next as u32;
            chain(&mut fat, next, n);
            next += n;
        }
    }

    // 머리
    let mut out = Vec::with_capacity((1 + fat_n + data_n) * SECTOR);
    out.extend_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    out.extend_from_slice(&[0; 16]);
    for v in [0x003Eu16, 0x0003, 0xFFFE, 9, 6] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0; 6]);
    for v in [
        0u32,
        fat_n as u32,
        dir_start as u32,
        0,
        CUTOFF as u32,
        minifat_start,
        minifat_n as u32,
        END_OF_CHAIN,
        0,
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for k in 0..109 {
        let v = if k < fat_n { k as u32 } else { FREE };
        out.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(out.len(), SECTOR);
    for v in &fat {
        out.extend_from_slice(&v.to_le_bytes());
    }

    // 디렉터리: 0 = 루트, 1.. = 스트림 (적흑 트리는 루트의 자식에서 시작)
    let mut sorted: Vec<usize> = (1..entries).collect();
    sorted.sort_by(|&a, &b| name_cmp(streams[a - 1].0, streams[b - 1].0));
    let mut tree = vec![(NO_STREAM, NO_STREAM, 0u32); entries];
    let top = build_tree(&sorted, 0, &mut tree);
    // 꽉 찬 층 수: 이 깊이부터는 빨강
    let full = usize::BITS - (sorted.len() + 1).leading_zeros() - 1;
    let entry = |out: &mut Vec<u8>,
                 name: &str,
                 kind: u8,
                 black: bool,
                 (l, r, c): (u32, u32, u32),
                 start: u32,
                 size: u64| {
        let mut e = [0u8; 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (k, u) in units.iter().enumerate() {
            e[2 * k..2 * k + 2].copy_from_slice(&u.to_le_bytes());
        }
        if !name.is_empty() {
            e[0x40..0x42].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        }
        e[0x42] = kind;
        e[0x43] = black as u8;
        e[0x44..0x48].copy_from_slice(&l.to_le_bytes());
        e[0x48..0x4C].copy_from_slice(&r.to_le_bytes());
        e[0x4C..0x50].copy_from_slice(&c.to_le_bytes());
        e[0x74..0x78].copy_from_slice(&start.to_le_bytes());
        e[0x78..0x80].copy_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&e);
    };
    entry(
        &mut out,
        "Root Entry",
        5,
        true,
        (NO_STREAM, NO_STREAM, top),
        ministream_start,
        mini.len() as u64,
    );
    for (i, (name, d)) in streams.iter().enumerate() {
        let (l, r, depth) = tree[i + 1];
        entry(
            &mut out,
            name,
            2,
            depth < full,
            (l, r, NO_STREAM),
            starts[i],
            d.len() as u64,
        );
    }
    for _ in entries..dir_n * SECTOR / 128 {
        entry(
            &mut out,
            "",
            0,
            false,
            (NO_STREAM, NO_STREAM, NO_STREAM),
            0,
            0,
        );
    }
    // 미니 FAT, 미니 스트림, 큰 스트림
    let pad = |out: &mut Vec<u8>, fill: u8| {
        while !out.len().is_multiple_of(SECTOR) {
            out.push(fill);
        }
    };
    for v in &mini_fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    pad(&mut out, 0xFF);
    out.extend_from_slice(&mini);
    pad(&mut out, 0);
    for (_, d) in streams.iter().filter(|(_, d)| d.len() >= CUTOFF) {
        out.extend_from_slice(d);
        pad(&mut out, 0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_streams_read_back() {
        let small: Vec<u8> = (0..454u32).map(|i| (i * 7) as u8).collect();
        let large: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let tiny = b"x".to_vec();
        let bytes = write_streams(&[
            ("_VBS_PROJECT_W", &small),
            ("Big", &large),
            ("a", &tiny),
            ("Empty", &[]),
        ])
        .unwrap();
        let cfb = crate::cfb::Cfb::open(&bytes).unwrap();
        assert_eq!(cfb.read("/_VBS_PROJECT_W").unwrap(), small);
        assert_eq!(cfb.read("/Big").unwrap(), large);
        assert_eq!(cfb.read("/a").unwrap(), tiny);
        assert_eq!(cfb.read("/Empty").unwrap(), Vec::<u8>::new());
        let mut names: Vec<String> = cfb.streams().into_iter().map(|s| s.0).collect();
        names.sort();
        assert_eq!(names, ["/Big", "/Empty", "/_VBS_PROJECT_W", "/a"]);
    }

    #[test]
    fn tree_colors_balance_black_height() {
        // 이름 5개: 깊이 0·1 은 검정, 깊이 2 는 빨강
        let data = [1u8];
        let names = ["a", "bb", "ccc", "dddd", "eeeee"];
        let streams: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, &data[..])).collect();
        let bytes = write_streams(&streams).unwrap();
        let dir = &bytes[SECTOR * 2..];
        let colors: Vec<u8> = (1..=5).map(|i| dir[i * 128 + 0x43]).collect();
        // 정렬 순서 a, bb, ccc, dddd, eeeee → 가운데 ccc 가 뿌리, bb·eeeee 가 깊이 1, a·dddd 가 깊이 2 (빨강)
        assert_eq!(colors, [0, 1, 1, 0, 1]);
        let cfb = crate::cfb::Cfb::open(&bytes).unwrap();
        assert_eq!(cfb.streams().len(), 5);
    }
}
