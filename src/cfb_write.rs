// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! OLE2 복합 문서(CFB) 쓰기 — 공개 명세 [MS-CFB] 판 3 (512바이트 섹터). 루트 저장소 바로 아래 스트림만 쓴다.
//!
//! 한셀이 쓰는 두 섹터 배치를 따른다 (관측):
//!
//! ```text
//! 파일 배치 (write — 레거시 .cell, 말뭉치 117개)   메모리 배치 (write_streams — xl/vbsProject.bin)
//! 섹터 0   처음 만든 FAT 의 흔적 (FAT 에서는 빈 섹터)   섹터 0…  FAT (+ DIFAT)
//! 섹터 1   처음 만든 디렉터리의 흔적 (Root Entry 하나)  그다음    디렉터리
//! 섹터 2   디렉터리                                   그다음    미니 FAT
//! 섹터 3   첫 FAT 섹터                                그다음    미니 스트림
//! 섹터 4   미니 FAT                                   그다음    큰 스트림
//! 섹터 5…  미니 스트림 (4096바이트 미만 스트림)
//! 그다음   나머지 FAT 섹터, DIFAT, 큰 스트림
//! ```
//!
//! 디렉터리 항목은 주어진 차례(한셀: `Workbook`, `_VBS_PROJECT_W`, `\x05SummaryInformation`)이고, 스트림 데이터는
//! 그 거꾸로 놓인다 (미니 스트림에 SummaryInformation, `_VBS_PROJECT_W`, `Workbook` 차례 — 작은 레거시 문서와
//! 매크로 문서 관측). 루트 아래는 이름 순서의 균형 레드·블랙 나무: 가운데(짝수 개면 앞쪽)가 뿌리, 가장 깊은 층만
//! 빨강 (두 스트림이면 첫째가 뿌리·둘째가 오른쪽 빨강, 세 스트림이면 가운데가 뿌리·양쪽 빨강).

use crate::error::{Error, Result};

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const SECTOR: usize = 512;
const MINI: usize = 64;
const CUTOFF: usize = 4096;
const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const NO_STREAM: u32 = 0xFFFF_FFFF;

/// 스트림 하나 (루트 바로 아래)
pub struct Stream<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// 섹터 배치
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// ole32 가 파일에 쓰는 배치 (흔적 섹터 둘, 디렉터리 2, 첫 FAT 3)
    File,
    /// 메모리에 만든 복합 문서 (FAT 이 앞)
    Memory,
}

/// [MS-CFB] 2.6.4 이름 비교: 길이(UTF-16 단위)가 짧은 것이 먼저, 같으면 대문자로 바꾼 코드 단위 순서
fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let ua: Vec<u16> = a.encode_utf16().collect();
    let ub: Vec<u16> = b.encode_utf16().collect();
    ua.len().cmp(&ub.len()).then_with(|| {
        let up = |s: &[u16]| -> Vec<u16> {
            String::from_utf16_lossy(s)
                .to_uppercase()
                .encode_utf16()
                .collect()
        };
        up(&ua).cmp(&up(&ub))
    })
}

struct Entry {
    name: String,
    kind: u8,
    color: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
    modified: u64,
}

fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn dir_entry(e: &Entry) -> [u8; 128] {
    let mut d = [0u8; 128];
    let units: Vec<u16> = e.name.encode_utf16().take(31).collect();
    for (i, u) in units.iter().enumerate() {
        put_u16(&mut d, i * 2, *u);
    }
    if e.kind != 0 {
        put_u16(&mut d, 64, ((units.len() + 1) * 2) as u16);
    }
    d[66] = e.kind;
    d[67] = e.color;
    put_u32(&mut d, 68, e.left);
    put_u32(&mut d, 72, e.right);
    put_u32(&mut d, 76, e.child);
    d[108..116].copy_from_slice(&e.modified.to_le_bytes());
    put_u32(&mut d, 116, e.start);
    put_u32(&mut d, 120, e.size as u32);
    d
}

fn empty_entry() -> Entry {
    Entry {
        name: String::new(),
        kind: 0,
        color: 0,
        left: NO_STREAM,
        right: NO_STREAM,
        child: NO_STREAM,
        start: 0,
        size: 0,
        modified: 0,
    }
}

/// 스트림들로 복합 문서를 만든다 (한셀이 레거시 `.cell` 을 쓰는 파일 배치). `modified` 는 루트 항목의 수정 시각
/// (FILETIME, 0 = 없음).
pub fn write(streams: &[Stream], modified: u64) -> Vec<u8> {
    build(streams, modified, Layout::File)
}

/// 스트림들로 복합 문서를 만든다 (한셀의 `xl/vbsProject.bin` 같은 메모리 배치: FAT, 디렉터리, 미니 FAT, 미니 스트림,
/// 큰 스트림). 이름은 1–31자.
pub fn write_streams(streams: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    for (name, _) in streams {
        if name.is_empty() || name.encode_utf16().count() > 31 {
            return Err(Error::InvalidValue {
                context: "OLE2(CFB) 쓰기".into(),
                value: (*name).into(),
            });
        }
    }
    let s: Vec<Stream> = streams
        .iter()
        .map(|&(name, data)| Stream { name, data })
        .collect();
    Ok(build(&s, 0, Layout::Memory))
}

fn build(streams: &[Stream], modified: u64, layout: Layout) -> Vec<u8> {
    // 디렉터리 항목: 0 = 루트, 1.. = 스트림 (주어진 순서). 나무는 이름 순서로 균형 있게 잇는다
    let mut entries: Vec<Entry> = vec![Entry {
        name: "Root Entry".into(),
        kind: 5,
        color: 0,
        left: NO_STREAM,
        right: NO_STREAM,
        child: NO_STREAM,
        start: END_OF_CHAIN,
        size: 0,
        modified,
    }];
    for s in streams {
        entries.push(Entry {
            name: s.name.to_string(),
            kind: 2,
            color: 1,
            left: NO_STREAM,
            right: NO_STREAM,
            child: NO_STREAM,
            start: END_OF_CHAIN,
            size: s.data.len() as u64,
            modified: 0,
        });
    }
    link_tree(&mut entries);
    // 데이터는 디렉터리의 거꾸로
    let data_order: Vec<usize> = (0..streams.len()).rev().collect();

    // 미니 스트림: 작은 스트림을 64바이트 단위로 이어 붙인다
    let mut mini_stream: Vec<u8> = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    for &i in &data_order {
        let s = &streams[i];
        if s.data.len() >= CUTOFF || s.data.is_empty() {
            continue;
        }
        let first = (mini_stream.len() / MINI) as u32;
        let n = s.data.len().div_ceil(MINI);
        mini_stream.extend_from_slice(s.data);
        mini_stream.resize((first as usize + n) * MINI, 0);
        for k in 0..n as u32 {
            mini_fat.push(if k + 1 == n as u32 {
                END_OF_CHAIN
            } else {
                first + k + 1
            });
        }
        entries[i + 1].start = first;
    }

    let dir_sectors = (entries.len().div_ceil(4)).max(1);
    let mini_fat_sectors = (mini_fat.len() * 4).div_ceil(SECTOR);
    let mini_stream_sectors = mini_stream.len().div_ceil(SECTOR);
    let big: Vec<usize> = data_order
        .iter()
        .copied()
        .filter(|&i| streams[i].data.len() >= CUTOFF)
        .collect();
    let big_sectors: usize = big
        .iter()
        .map(|&i| streams[i].data.len().div_ceil(SECTOR))
        .sum();
    // FAT 섹터 수: 모든 섹터(FAT·DIFAT 자신 포함)를 담을 만큼
    let stale = if layout == Layout::File { 2 } else { 0 };
    let fixed = stale + dir_sectors + mini_fat_sectors + mini_stream_sectors + big_sectors;
    let mut fat_sectors = 1;
    while fat_sectors * (SECTOR / 4) < fixed + fat_sectors + difat_sectors(fat_sectors) {
        fat_sectors += 1;
    }
    let n_difat = difat_sectors(fat_sectors);
    let total = fixed + fat_sectors + n_difat;

    let mut fat = vec![FREE; fat_sectors * (SECTOR / 4)];
    let mut fat_locs: Vec<usize> = Vec::new();
    let mut difat_locs = Vec::new();
    let mut next = stale;
    fn put_fats(fat: &mut [u32], next: &mut usize, n: usize, locs: &mut Vec<usize>) {
        for _ in 0..n {
            fat[*next] = FAT_SECT;
            locs.push(*next);
            *next += 1;
        }
    }
    if layout == Layout::Memory {
        put_fats(&mut fat, &mut next, fat_sectors, &mut fat_locs);
        for _ in 0..n_difat {
            fat[next] = DIFAT_SECT;
            difat_locs.push(next);
            next += 1;
        }
    }
    let dir_start = next;
    chain(&mut fat, dir_start, dir_sectors);
    next += dir_sectors;
    if layout == Layout::File {
        put_fats(&mut fat, &mut next, 1, &mut fat_locs);
    }
    let mini_fat_start = (mini_fat_sectors > 0).then_some(next);
    chain(&mut fat, next, mini_fat_sectors);
    next += mini_fat_sectors;
    let mini_start = (mini_stream_sectors > 0).then_some(next);
    chain(&mut fat, next, mini_stream_sectors);
    next += mini_stream_sectors;
    if layout == Layout::File {
        put_fats(&mut fat, &mut next, fat_sectors - 1, &mut fat_locs);
        for _ in 0..n_difat {
            fat[next] = DIFAT_SECT;
            difat_locs.push(next);
            next += 1;
        }
    }
    for &i in &big {
        let n = streams[i].data.len().div_ceil(SECTOR);
        entries[i + 1].start = next as u32;
        chain(&mut fat, next, n);
        next += n;
    }
    debug_assert_eq!(next, total);
    if let Some(s) = mini_start {
        entries[0].start = s as u32;
        entries[0].size = mini_stream.len() as u64;
    }

    // 파일
    let mut out = vec![0u8; SECTOR * (total + 1)];
    {
        let h = &mut out[..SECTOR];
        h[..8].copy_from_slice(&SIGNATURE);
        put_u16(h, 0x18, 0x003E);
        put_u16(h, 0x1A, 3);
        put_u16(h, 0x1C, 0xFFFE);
        put_u16(h, 0x1E, 9);
        put_u16(h, 0x20, 6);
        put_u32(h, 0x2C, fat_sectors as u32);
        put_u32(h, 0x30, dir_start as u32);
        put_u32(h, 0x38, CUTOFF as u32);
        put_u32(h, 0x3C, mini_fat_start.map_or(END_OF_CHAIN, |s| s as u32));
        put_u32(h, 0x40, mini_fat_sectors as u32);
        put_u32(
            h,
            0x44,
            difat_locs.first().map_or(END_OF_CHAIN, |&s| s as u32),
        );
        put_u32(h, 0x48, n_difat as u32);
        for k in 0..109 {
            let v = fat_locs.get(k).map_or(FREE, |&s| s as u32);
            put_u32(h, 0x4C + k * 4, v);
        }
    }
    let sector = |n: usize| -> std::ops::Range<usize> {
        let a = SECTOR * (n + 1);
        a..a + SECTOR
    };
    if layout == Layout::File {
        // 흔적 FAT (섹터 0) 과 흔적 디렉터리 (섹터 1)
        {
            let s = &mut out[sector(0)];
            s.fill(0xFF);
            put_u32(s, 0, FAT_SECT);
            put_u32(s, 4, END_OF_CHAIN);
        }
        let root = Entry {
            name: "Root Entry".into(),
            kind: 5,
            start: END_OF_CHAIN,
            ..empty_entry()
        };
        let mut d = dir_entry(&root).to_vec();
        for _ in 1..4 {
            d.extend_from_slice(&dir_entry(&empty_entry()));
        }
        out[sector(1)].copy_from_slice(&d);
    }
    // 디렉터리
    let mut dir: Vec<u8> = Vec::with_capacity(dir_sectors * SECTOR);
    for e in &entries {
        dir.extend_from_slice(&dir_entry(e));
    }
    while dir.len() < dir_sectors * SECTOR {
        dir.extend_from_slice(&dir_entry(&empty_entry()));
    }
    out[SECTOR * (dir_start + 1)..SECTOR * (dir_start + 1 + dir_sectors)].copy_from_slice(&dir);
    // FAT
    for (k, &loc) in fat_locs.iter().enumerate() {
        let r = sector(loc);
        for (i, v) in fat[k * 128..(k + 1) * 128].iter().enumerate() {
            put_u32(&mut out[r.clone()], i * 4, *v);
        }
    }
    // DIFAT (FAT 섹터가 109개를 넘을 때)
    for (k, &loc) in difat_locs.iter().enumerate() {
        let s = &mut out[sector(loc)];
        for i in 0..127 {
            let v = fat_locs.get(109 + k * 127 + i).map_or(FREE, |&x| x as u32);
            put_u32(s, i * 4, v);
        }
        let nxt = difat_locs.get(k + 1).map_or(END_OF_CHAIN, |&x| x as u32);
        put_u32(s, 127 * 4, nxt);
    }
    // 미니 FAT 과 미니 스트림
    if let Some(start) = mini_fat_start {
        let mut b = vec![0xFFu8; mini_fat_sectors * SECTOR];
        for (i, v) in mini_fat.iter().enumerate() {
            put_u32(&mut b, i * 4, *v);
        }
        let a = SECTOR * (start + 1);
        out[a..a + b.len()].copy_from_slice(&b);
    }
    if let Some(start) = mini_start {
        let a = SECTOR * (start + 1);
        out[a..a + mini_stream.len()].copy_from_slice(&mini_stream);
    }
    for &i in &big {
        let a = SECTOR * (entries[i + 1].start as usize + 1);
        out[a..a + streams[i].data.len()].copy_from_slice(streams[i].data);
    }
    out
}

/// FAT 섹터가 109개를 넘으면 DIFAT 섹터가 든다 (섹터마다 127개 + 다음 섹터 번호)
fn difat_sectors(fat_sectors: usize) -> usize {
    fat_sectors.saturating_sub(109).div_ceil(127)
}

fn chain(fat: &mut [u32], start: usize, n: usize) {
    for k in 0..n {
        fat[start + k] = if k + 1 == n {
            END_OF_CHAIN
        } else {
            (start + k + 1) as u32
        };
    }
}

/// 루트의 자식들을 이름 순서로 레드·블랙 나무로 잇는다. 균형 나무: 가운데(짝수 개면 앞쪽)가 뿌리(검정),
/// 나머지는 재귀로 왼쪽·오른쪽. 가장 깊은 층은 빨강이다 — 모든 길의 검정 수가 같다.
fn link_tree(entries: &mut [Entry]) {
    let mut ids: Vec<usize> = (1..entries.len()).collect();
    ids.sort_by(|&a, &b| name_cmp(&entries[a].name, &entries[b].name));
    fn build(entries: &mut [Entry], ids: &[usize], depth: usize, max: usize) -> u32 {
        if ids.is_empty() {
            return NO_STREAM;
        }
        let mid = (ids.len() - 1) / 2;
        let id = ids[mid];
        let left = build(entries, &ids[..mid], depth + 1, max);
        let right = build(entries, &ids[mid + 1..], depth + 1, max);
        entries[id].left = left;
        entries[id].right = right;
        entries[id].color = if depth == max && depth > 0 { 0 } else { 1 };
        id as u32
    }
    let n = ids.len();
    let max = if n == 0 {
        0
    } else {
        (usize::BITS - n.leading_zeros()) as usize - 1
    };
    let root = build(entries, &ids, 0, max);
    entries[0].child = root;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfb::Cfb;

    #[test]
    fn round_trips_small_and_large_streams() {
        let big: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let small = b"\xfe\xff summary".to_vec();
        let bytes = write(
            &[
                Stream {
                    name: "Workbook",
                    data: &big,
                },
                Stream {
                    name: "\u{5}SummaryInformation",
                    data: &small,
                },
            ],
            0,
        );
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.read("/Workbook").unwrap(), big);
        assert_eq!(c.read("/\u{5}SummaryInformation").unwrap(), small);
        // 한셀 배치: 디렉터리 2, 첫 FAT 3, 미니 FAT 4, 미니 스트림 5
        assert_eq!(u32::from_le_bytes(bytes[0x30..0x34].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[0x4C..0x50].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()), 4);
    }

    #[test]
    fn small_streams_are_stored_in_reverse_directory_order() {
        // 작은 레거시 문서: 미니 스트림의 0 번에 SummaryInformation, 그 뒤에 Workbook (관측)
        let wbk = vec![1u8; 3000];
        let sum = vec![2u8; 493];
        let vbs = vec![3u8; 454];
        let bytes = write(
            &[
                Stream {
                    name: "Workbook",
                    data: &wbk,
                },
                Stream {
                    name: "_VBS_PROJECT_W",
                    data: &vbs,
                },
                Stream {
                    name: "\u{5}SummaryInformation",
                    data: &sum,
                },
            ],
            0,
        );
        let dir = &bytes[SECTOR * 3..];
        let start =
            |i: usize| u32::from_le_bytes(dir[i * 128 + 116..i * 128 + 120].try_into().unwrap());
        assert_eq!((start(1), start(2), start(3)), (16, 8, 0));
        // 가운데 _VBS_PROJECT_W 가 뿌리(검정), 양쪽이 빨강
        let colors: Vec<u8> = (1..=3).map(|i| dir[i * 128 + 0x43]).collect();
        assert_eq!(colors, [0, 1, 0]);
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.read("/Workbook").unwrap(), wbk);
        assert_eq!(c.read("/_VBS_PROJECT_W").unwrap(), vbs);
    }

    #[test]
    fn many_fat_sectors_use_difat() {
        // FAT 섹터 109개를 넘기려면 약 7MB
        let big = vec![7u8; 8 * 1024 * 1024];
        let bytes = write(
            &[Stream {
                name: "Workbook",
                data: &big,
            }],
            0,
        );
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.read("/Workbook").unwrap().len(), big.len());
        let m = write_streams(&[("Big", &big)]).unwrap();
        assert_eq!(
            Cfb::open(&m).unwrap().read("/Big").unwrap().len(),
            big.len()
        );
    }

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
        assert!(write_streams(&[("", &tiny)]).is_err());
    }

    #[test]
    fn memory_layout_puts_fat_first() {
        // 한셀의 xl/vbsProject.bin: FAT 0, 디렉터리 1, 미니 FAT 2, 미니 스트림 3 (관측)
        let small = vec![5u8; 454];
        let bytes = write_streams(&[("_VBS_PROJECT_W", &small)]).unwrap();
        let at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!((at(0x4C), at(0x30), at(0x3C)), (0, 1, 2));
        assert_eq!(bytes.len(), 512 * 5);
    }

    #[test]
    fn tree_colors_balance_black_height() {
        // 이름 5개: 정렬 a, bb, ccc, dddd, eeeee → 가운데 ccc 가 뿌리, a·dddd 가 깊이 1 (검정), bb·eeeee 가 깊이 2 (빨강)
        let data = [1u8];
        let names = ["a", "bb", "ccc", "dddd", "eeeee"];
        let streams: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, &data[..])).collect();
        let bytes = write_streams(&streams).unwrap();
        let dir = &bytes[SECTOR * 2..];
        let colors: Vec<u8> = (1..=5).map(|i| dir[i * 128 + 0x43]).collect();
        assert_eq!(colors, [1, 0, 1, 1, 0]);
        let cfb = crate::cfb::Cfb::open(&bytes).unwrap();
        assert_eq!(cfb.streams().len(), 5);
    }
}
