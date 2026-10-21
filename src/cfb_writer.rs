// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! OLE2 복합 문서(CFB) 쓰기 — 공개 명세 [MS-CFB] 판 3 (512바이트 섹터).
//!
//! 섹터 배치는 한셀이 레거시 `.cell` 을 저장할 때의 배치를 따른다 (관측, 말뭉치 117개):
//!
//! ```text
//! 섹터 0      처음 만든 FAT 의 흔적 (FAT 에서는 빈 섹터): [FATSECT, ENDOFCHAIN, FREE …]
//! 섹터 1      처음 만든 디렉터리의 흔적 (빈 섹터): Root Entry 하나
//! 섹터 2      디렉터리
//! 섹터 3      첫 FAT 섹터
//! 섹터 4      미니 FAT
//! 섹터 5…     미니 스트림 (4096바이트 미만 스트림)
//! 그다음      나머지 FAT 섹터, 큰 스트림들 (디렉터리 순서)
//! ```
//!
//! 디렉터리는 루트 아래 스트림들을 레드·블랙 나무로 잇는다. 한셀의 두 스트림 문서(`Workbook`,
//! `\x05SummaryInformation`)는 첫 스트림이 루트의 자식(검정), 둘째가 그 오른쪽(빨강)이다.

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

/// 스트림들로 복합 문서를 만든다. `modified` 는 루트 항목의 수정 시각 (FILETIME, 0 = 없음).
pub fn write(streams: &[Stream], modified: u64) -> Vec<u8> {
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

    // 미니 스트림: 작은 스트림을 64바이트 단위로 이어 붙인다
    let mut mini_stream: Vec<u8> = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    for (i, s) in streams.iter().enumerate() {
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

    // 섹터 배치: 0 흔적 FAT, 1 흔적 디렉터리, 2 디렉터리, 3 첫 FAT, 4 미니 FAT, 5.. 미니 스트림
    let dir_sectors = (entries.len().div_ceil(4)).max(1);
    let mini_fat_sectors = (mini_fat.len() * 4).div_ceil(SECTOR);
    let mini_stream_sectors = mini_stream.len().div_ceil(SECTOR);
    let big: Vec<usize> = (0..streams.len())
        .filter(|&i| streams[i].data.len() >= CUTOFF)
        .collect();
    let big_sectors: usize = big
        .iter()
        .map(|&i| streams[i].data.len().div_ceil(SECTOR))
        .sum();
    // FAT 섹터 수: 모든 섹터(FAT 자신 포함)를 담을 만큼
    let fixed = 2 + dir_sectors + mini_fat_sectors + mini_stream_sectors + big_sectors;
    let mut fat_sectors = 1;
    while fat_sectors * (SECTOR / 4) < fixed + fat_sectors + difat_sectors(fat_sectors) {
        fat_sectors += 1;
    }
    let n_difat = difat_sectors(fat_sectors);
    let total = fixed + fat_sectors + n_difat;

    let mut fat = vec![FREE; fat_sectors * (SECTOR / 4)];
    let mut next = 2usize;
    let dir_start = next;
    chain(&mut fat, dir_start, dir_sectors);
    next += dir_sectors;
    let mut fat_locs: Vec<usize> = vec![next];
    fat[next] = FAT_SECT;
    next += 1;
    let mini_fat_start = (mini_fat_sectors > 0).then_some(next);
    chain(&mut fat, next, mini_fat_sectors);
    next += mini_fat_sectors;
    let mini_start = (mini_stream_sectors > 0).then_some(next);
    chain(&mut fat, next, mini_stream_sectors);
    next += mini_stream_sectors;
    for _ in 1..fat_sectors {
        fat[next] = FAT_SECT;
        fat_locs.push(next);
        next += 1;
    }
    let mut difat_locs = Vec::new();
    for _ in 0..n_difat {
        fat[next] = DIFAT_SECT;
        difat_locs.push(next);
        next += 1;
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
    let sector = |out: &mut Vec<u8>, n: usize| -> std::ops::Range<usize> {
        let a = SECTOR * (n + 1);
        a..a + SECTOR
    };
    // 흔적 FAT (섹터 0) 과 흔적 디렉터리 (섹터 1)
    {
        let r = sector(&mut out, 0);
        let s = &mut out[r];
        s.fill(0xFF);
        put_u32(s, 0, FAT_SECT);
        put_u32(s, 4, END_OF_CHAIN);
    }
    {
        let r = sector(&mut out, 1);
        let mut root = Entry {
            name: "Root Entry".into(),
            kind: 5,
            color: 0,
            left: NO_STREAM,
            right: NO_STREAM,
            child: NO_STREAM,
            start: END_OF_CHAIN,
            size: 0,
            modified: 0,
        };
        let mut d = dir_entry(&root).to_vec();
        root.kind = 0;
        for _ in 1..4 {
            d.extend_from_slice(&dir_entry(&empty_entry()));
        }
        out[r].copy_from_slice(&d);
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
        let r = sector(&mut out, loc);
        for (i, v) in fat[k * 128..(k + 1) * 128].iter().enumerate() {
            put_u32(&mut out[r.clone()], i * 4, *v);
        }
    }
    // DIFAT (FAT 섹터가 109개를 넘을 때)
    for (k, &loc) in difat_locs.iter().enumerate() {
        let r = sector(&mut out, loc);
        let s = &mut out[r];
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

/// 루트의 자식들을 이름 순서로 레드·블랙 나무로 잇는다. 한 단계짜리 균형 나무: 가운데가 뿌리(검정),
/// 나머지는 재귀로 왼쪽·오른쪽. 깊이가 가장 깊은 잎은 빨강이다 (한셀: 두 스트림이면 둘째가 빨강).
fn link_tree(entries: &mut [Entry]) {
    let mut ids: Vec<usize> = (1..entries.len()).collect();
    ids.sort_by(|&a, &b| name_cmp(&entries[a].name, &entries[b].name));
    fn build(entries: &mut [Entry], ids: &[usize], depth: usize, max: usize) -> u32 {
        if ids.is_empty() {
            return NO_STREAM;
        }
        // 짝수 개면 왼쪽을 하나 적게 (뿌리를 앞쪽에)
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
    }
}
