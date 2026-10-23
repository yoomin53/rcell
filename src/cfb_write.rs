// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! OLE2 복합 문서(CFB) 쓰기 — 공개 명세 [MS-CFB] 판 3 (512바이트 섹터). 루트 아래 스트림과 한 단계 저장소를 쓴다.
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

/// 스트림 하나
pub struct Stream<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// 루트 바로 아래 저장소 하나와 그 안의 스트림 (한셀 레거시의 피벗 캐시 `_SX_DB_CUR/000N`)
pub struct Storage<'a> {
    pub name: &'a str,
    pub streams: Vec<Stream<'a>>,
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
    created: u64,
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
    d[100..108].copy_from_slice(&e.created.to_le_bytes());
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
        created: 0,
        modified: 0,
    }
}

/// 스트림들로 복합 문서를 만든다 (한셀이 레거시 `.cell` 을 쓰는 파일 배치). `modified` 는 루트 항목의 수정 시각
/// (FILETIME, 0 = 없음).
pub fn write(streams: &[Stream], modified: u64) -> Vec<u8> {
    build(streams, &[], modified, Layout::File)
}

/// [`write`] 에 루트 바로 아래 저장소(한 단계)를 더한다. 디렉터리는 루트 스트림, 그다음 저장소마다 저장소와 그 스트림
/// 차례다. 데이터는 루트 스트림의 거꾸로, 그다음 저장소마다 그 스트림의 거꾸로 쓰고, 섹터는 쓰는 차례로 잡는다 — 미니
/// 스트림도 늘어날 때마다 그 자리의 다음 섹터를 잡는다 (관측: 피벗 문서 pa 의 미니 스트림 6 → 18 → 19, Workbook 7–17;
/// 캐시 둘인 pf3 은 미니 스트림에 0002 가 0001 보다 앞). 저장소 항목의 만든·고친 시각은 `modified` (한셀은 저장 시각)
pub fn write_with_storages(streams: &[Stream], storages: &[Storage], modified: u64) -> Vec<u8> {
    build(streams, storages, modified, Layout::File)
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
    Ok(build(&s, &[], 0, Layout::Memory))
}

/// 섹터 할당: 차례대로 다음 빈 섹터를 준다
struct Alloc {
    fat: Vec<u32>,
    next: usize,
}

impl Alloc {
    fn take(&mut self, mark: u32) -> usize {
        let s = self.next;
        self.fat[s] = mark;
        self.next += 1;
        s
    }
    /// 이어진 사슬 n 섹터
    fn run(&mut self, n: usize) -> Vec<usize> {
        let v: Vec<usize> = (self.next..self.next + n).collect();
        self.next += n;
        v
    }
}

/// 섹터 목록을 FAT 사슬로 잇는다
fn link_chain(fat: &mut [u32], secs: &[usize]) {
    for (k, &s) in secs.iter().enumerate() {
        fat[s] = secs.get(k + 1).map_or(END_OF_CHAIN, |&n| n as u32);
    }
}

fn build(streams: &[Stream], storages: &[Storage], modified: u64, layout: Layout) -> Vec<u8> {
    // 디렉터리 항목: 0 = 루트, 그다음 루트 스트림 (주어진 차례), 저장소마다 저장소와 그 스트림
    let stream_entry = |s: &Stream| Entry {
        name: s.name.to_string(),
        kind: 2,
        color: 1,
        start: END_OF_CHAIN,
        size: s.data.len() as u64,
        ..empty_entry()
    };
    let mut entries: Vec<Entry> = vec![Entry {
        name: "Root Entry".into(),
        kind: 5,
        start: END_OF_CHAIN,
        modified,
        ..empty_entry()
    }];
    // 스트림 항목 번호와 데이터, 쓰는 차례
    let mut data: Vec<(usize, &[u8])> = Vec::new();
    let mut containers: Vec<(usize, Vec<usize>)> = vec![(0, Vec::new())];
    for s in streams {
        containers[0].1.push(entries.len());
        data.push((entries.len(), s.data));
        entries.push(stream_entry(s));
    }
    // 데이터는 루트 스트림의 거꾸로, 그다음 저장소마다 그 스트림의 거꾸로
    let mut order: Vec<(usize, &[u8])> = data.iter().rev().copied().collect();
    for st in storages {
        let si = entries.len();
        containers[0].1.push(si);
        entries.push(Entry {
            name: st.name.to_string(),
            kind: 1,
            color: 1,
            created: modified,
            modified,
            ..empty_entry()
        });
        let mut kids = Vec::new();
        let mut own = Vec::new();
        for s in &st.streams {
            kids.push(entries.len());
            own.push((entries.len(), s.data));
            entries.push(stream_entry(s));
        }
        order.extend(own.into_iter().rev());
        containers.push((si, kids));
    }
    for (parent, kids) in &containers {
        link_tree(&mut entries, *parent, kids);
    }

    let small = |d: &[u8]| !d.is_empty() && d.len() < CUTOFF;
    let mini_total: usize = order
        .iter()
        .filter(|(_, d)| small(d))
        .map(|(_, d)| d.len().div_ceil(MINI))
        .sum();
    let dir_sectors = (entries.len().div_ceil(4)).max(1);
    let mini_fat_sectors = (mini_total * 4).div_ceil(SECTOR);
    let mini_stream_sectors = (mini_total * MINI).div_ceil(SECTOR);
    let big_sectors: usize = order
        .iter()
        .filter(|(_, d)| d.len() >= CUTOFF)
        .map(|(_, d)| d.len().div_ceil(SECTOR))
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

    let mut al = Alloc {
        fat: vec![FREE; fat_sectors * (SECTOR / 4)],
        next: stale,
    };
    let mut fat_locs: Vec<usize> = Vec::new();
    let mut difat_locs: Vec<usize> = Vec::new();
    let put_rest_fat =
        |al: &mut Alloc, fat_locs: &mut Vec<usize>, difat_locs: &mut Vec<usize>, n: usize| {
            for _ in 0..n {
                fat_locs.push(al.take(FAT_SECT));
            }
            for _ in 0..n_difat {
                difat_locs.push(al.take(DIFAT_SECT));
            }
        };
    let dir_secs: Vec<usize>;
    let mini_fat_secs: Vec<usize>;
    let mut mini_secs: Vec<usize> = Vec::new();
    // 미니 스트림 (64바이트 단위)과 미니 FAT
    let mut mini_stream: Vec<u8> = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut put_small = |e: &mut Entry, d: &[u8], mini_stream: &mut Vec<u8>| {
        let first = (mini_stream.len() / MINI) as u32;
        let n = d.len().div_ceil(MINI);
        mini_stream.extend_from_slice(d);
        mini_stream.resize((first as usize + n) * MINI, 0);
        for k in 0..n as u32 {
            mini_fat.push(if k + 1 == n as u32 {
                END_OF_CHAIN
            } else {
                first + k + 1
            });
        }
        e.start = first;
    };
    let mut big_runs: Vec<(usize, Vec<usize>)> = Vec::new();
    match layout {
        Layout::Memory => {
            put_rest_fat(&mut al, &mut fat_locs, &mut difat_locs, fat_sectors);
            dir_secs = al.run(dir_sectors);
            mini_fat_secs = al.run(mini_fat_sectors);
            mini_secs = al.run(mini_stream_sectors);
            for &(i, d) in &order {
                if small(d) {
                    put_small(&mut entries[i], d, &mut mini_stream);
                }
            }
            for &(i, d) in &order {
                if d.len() >= CUTOFF {
                    big_runs.push((i, al.run(d.len().div_ceil(SECTOR))));
                }
            }
        }
        Layout::File => {
            // 흔적 섹터 둘 뒤에 디렉터리 첫 섹터, 첫 FAT, 나머지 디렉터리, 미니 FAT, 그다음 쓰는 차례로 데이터.
            // 나머지 FAT·DIFAT 는 첫 큰 스트림 앞 (큰 스트림이 없으면 끝)
            let mut d = vec![al.next];
            al.next += 1;
            fat_locs.push(al.take(FAT_SECT));
            d.extend(al.run(dir_sectors - 1));
            dir_secs = d;
            mini_fat_secs = al.run(mini_fat_sectors);
            let mut rest_done = false;
            for &(i, d) in &order {
                if small(d) {
                    put_small(&mut entries[i], d, &mut mini_stream);
                    while mini_secs.len() * SECTOR < mini_stream.len() {
                        let s = al.next;
                        al.next += 1;
                        mini_secs.push(s);
                    }
                } else if d.len() >= CUTOFF {
                    if !rest_done {
                        put_rest_fat(&mut al, &mut fat_locs, &mut difat_locs, fat_sectors - 1);
                        rest_done = true;
                    }
                    big_runs.push((i, al.run(d.len().div_ceil(SECTOR))));
                }
            }
            if !rest_done {
                put_rest_fat(&mut al, &mut fat_locs, &mut difat_locs, fat_sectors - 1);
            }
        }
    }
    debug_assert_eq!(al.next, total);
    let mut fat = al.fat;
    link_chain(&mut fat, &dir_secs);
    link_chain(&mut fat, &mini_fat_secs);
    link_chain(&mut fat, &mini_secs);
    for (i, secs) in &big_runs {
        link_chain(&mut fat, secs);
        entries[*i].start = secs[0] as u32;
    }
    if let Some(&s) = mini_secs.first() {
        entries[0].start = s as u32;
        entries[0].size = mini_stream.len() as u64;
    }
    let dir_start = dir_secs[0];
    let mini_fat_start = mini_fat_secs.first().copied();

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
    for (k, &s) in dir_secs.iter().enumerate() {
        out[sector(s)].copy_from_slice(&dir[k * SECTOR..(k + 1) * SECTOR]);
    }
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
    if !mini_fat_secs.is_empty() {
        let mut b = vec![0xFFu8; mini_fat_sectors * SECTOR];
        for (i, v) in mini_fat.iter().enumerate() {
            put_u32(&mut b, i * 4, *v);
        }
        for (k, &s) in mini_fat_secs.iter().enumerate() {
            out[sector(s)].copy_from_slice(&b[k * SECTOR..(k + 1) * SECTOR]);
        }
    }
    for (k, &s) in mini_secs.iter().enumerate() {
        let part = &mini_stream[k * SECTOR..((k + 1) * SECTOR).min(mini_stream.len())];
        let a = sector(s).start;
        out[a..a + part.len()].copy_from_slice(part);
    }
    for (i, secs) in &big_runs {
        let d = order.iter().find(|(j, _)| j == i).map_or(&[][..], |x| x.1);
        let a = SECTOR * (secs[0] + 1);
        out[a..a + d.len()].copy_from_slice(d);
    }
    out
}

/// FAT 섹터가 109개를 넘으면 DIFAT 섹터가 든다 (섹터마다 127개 + 다음 섹터 번호)
fn difat_sectors(fat_sectors: usize) -> usize {
    fat_sectors.saturating_sub(109).div_ceil(127)
}

/// 저장소(루트 포함) `parent` 의 자식들을 이름 순서로 레드·블랙 나무로 잇는다. 균형 나무: 가운데(짝수 개면 앞쪽)가
/// 뿌리(검정), 나머지는 재귀로 왼쪽·오른쪽. 가장 깊은 층은 빨강이다 — 모든 길의 검정 수가 같다.
fn link_tree(entries: &mut [Entry], parent: usize, kids: &[usize]) {
    let mut ids: Vec<usize> = kids.to_vec();
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
    entries[parent].child = root;
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
    fn storages_follow_hancell_pivot_layout() {
        // 한셀의 피벗 문서 pa: 디렉터리 2·4, FAT 3, 미니 FAT 5, 미니 스트림 6 → 18 → 19, Workbook 7–17. 디렉터리는 Root,
        // Workbook, SummaryInformation, _SX_DB_CUR (뿌리, 검정 — 양쪽 빨강), 0001 (관측)
        let wbk = vec![1u8; 5589];
        let sum = vec![2u8; 493];
        let c1 = vec![3u8; 538];
        let c2 = vec![4u8; 341];
        let bytes = write_with_storages(
            &[
                Stream {
                    name: "Workbook",
                    data: &wbk,
                },
                Stream {
                    name: "\u{5}SummaryInformation",
                    data: &sum,
                },
            ],
            &[Storage {
                name: "_SX_DB_CUR",
                streams: vec![Stream {
                    name: "0001",
                    data: &c1,
                }],
            }],
            7,
        );
        let at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!((at(0x30), at(0x4C), at(0x3C)), (2, 3, 5));
        let fat = |s: u32| at(SECTOR * 4 + s as usize * 4);
        assert_eq!(
            (fat(2), fat(6), fat(18), fat(19)),
            (4, 18, 19, END_OF_CHAIN)
        );
        let dir = |i: usize| {
            let s = if i < 4 { 3 } else { 5 };
            SECTOR * s + (i % 4) * 128
        };
        let start =
            |i: usize| u32::from_le_bytes(bytes[dir(i) + 116..dir(i) + 120].try_into().unwrap());
        assert_eq!((start(0), start(1), start(4)), (6, 7, 8));
        let colors: Vec<u8> = (1..=4).map(|i| bytes[dir(i) + 67]).collect();
        assert_eq!(colors, [0, 0, 1, 1]);
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.read("/Workbook").unwrap(), wbk);
        assert_eq!(c.read("/_SX_DB_CUR/0001").unwrap(), c1);
        // 캐시 둘 (pf3): 미니 스트림에 0002 가 0001 보다 앞 (8, 14)
        let bytes = write_with_storages(
            &[Stream {
                name: "\u{5}SummaryInformation",
                data: &sum,
            }],
            &[Storage {
                name: "_SX_DB_CUR",
                streams: vec![
                    Stream {
                        name: "0001",
                        data: &c1,
                    },
                    Stream {
                        name: "0002",
                        data: &c2,
                    },
                ],
            }],
            0,
        );
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.read("/_SX_DB_CUR/0002").unwrap(), c2);
        let start =
            |i: usize| u32::from_le_bytes(bytes[dir(i) + 116..dir(i) + 120].try_into().unwrap());
        assert_eq!((start(3), start(4)), (14, 8));
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
