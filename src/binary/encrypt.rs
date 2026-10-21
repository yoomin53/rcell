// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 구조 잠금 문서의 암호화 컨테이너 — 한셀은 통합 문서 구조 잠금(`lockStructure`)이 걸린 문서를 바이너리
//! `.cell` 로 저장하면 ZIP 패키지를 기본 암호(`VelvetSweatshop`)의 표준 암호화로 감싸 OLE2 복합 문서에 넣는다
//! (관측: 코퍼스 outline, COM 으로 만든 구조 잠금 문서 3개. 창 잠금만 걸린 문서는 감싸지 않는다).
//!
//! 복합 문서는 [MS-CFB] 판 3 (512바이트 섹터), 스트림은 `EncryptionInfo`(224 바이트, 미니 스트림)와
//! `EncryptedPackage` 둘뿐이다. 섹터 배치 (관측, 패키지 5.6 KB·99 KB·706 KB):
//!
//! ```text
//! 섹터 0   FAT
//! 섹터 1   디렉터리 (루트, EncryptionInfo, EncryptedPackage, 빈 항목)
//! 섹터 2   미니 FAT
//! 섹터 3   미니 스트림 (EncryptionInfo)
//! 섹터 4…  EncryptedPackage. 8바이트 머리 다음 4096바이트씩 쓰고, 쓰기 전에 FAT 이 모자라면 그 자리에
//!          FAT 섹터를 하나 더 잡는다 → FAT 섹터 0, 125, 254, 383, 512, 633, 762, … (관측과 같다)
//! ```
//!
//! 디렉터리 나무는 루트의 자식이 `EncryptionInfo`(검정), 그 오른쪽이 `EncryptedPackage`(빨강)다.
//! 한셀은 salt 와 검증자를 같은 16바이트로 쓰고 한 번 실행하는 동안 같은 값을 쓴다 (관측) — rcell 은 패키지의
//! SHA-1 에서 만든다. 한셀은 루트에 저장 시각을 적고 파일을 스트림 끝에서 자르지만, rcell 은 시각을 0 으로 두고
//! 마지막 섹터를 채운다.

use crate::crypto;

const SECTOR: usize = 512;
const MINI: usize = 64;
/// 이보다 작은 스트림은 미니 스트림에 둔다
const CUTOFF: usize = 4096;
const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const DIFAT_SECT: u32 = 0xFFFF_FFFC;
const NO_STREAM: u32 = 0xFFFF_FFFF;
/// 머리에 바로 적는 FAT 섹터 자리 수
const HEADER_DIFAT: usize = 109;
/// FAT 섹터 하나가 덮는 섹터 수
const PER_FAT: usize = SECTOR / 4;
/// 패키지를 쓰는 조각 (관측한 FAT 섹터 자리에서 거꾸로 맞춘 값)
const CHUNK: usize = 4096;

/// ZIP 패키지를 한셀처럼 감싼다 (기본 암호)
pub fn wrap(package: &[u8]) -> Vec<u8> {
    let h = crypto::sha1(package);
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&h[..16]);
    let (info, enc) = crypto::encrypt_package(package, crypto::DEFAULT_PASSWORD, &salt, &salt);
    container(&info, &enc)
}

fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// 디렉터리 항목 (128 바이트)
#[allow(clippy::too_many_arguments)]
fn dir_entry(
    d: &mut [u8],
    name: &str,
    kind: u8,
    color: u8,
    right: u32,
    child: u32,
    start: u32,
    size: usize,
) {
    let units: Vec<u16> = name.encode_utf16().collect();
    for (i, u) in units.iter().enumerate() {
        put_u16(d, i * 2, *u);
    }
    if !units.is_empty() {
        put_u16(d, 64, ((units.len() + 1) * 2) as u16);
    }
    d[66] = kind;
    d[67] = color;
    put_u32(d, 68, NO_STREAM);
    put_u32(d, 72, right);
    put_u32(d, 76, child);
    put_u32(d, 116, start);
    put_u32(d, 120, size as u32);
}

/// `EncryptionInfo`(미니 스트림에 들어갈 크기) + `EncryptedPackage` → 복합 문서.
/// 패키지가 4096 바이트보다 작으면 [MS-CFB] 대로 패키지도 미니 스트림에 넣는다 (한셀에서는 보지 못한 크기)
fn container(info: &[u8], pkg: &[u8]) -> Vec<u8> {
    debug_assert!(info.len() < CUTOFF);
    let small = pkg.len() < CUTOFF;
    let info_mini = info.len().div_ceil(MINI);
    let pkg_mini = if small { pkg.len().div_ceil(MINI) } else { 0 };
    let mini_n = info_mini + pkg_mini;
    debug_assert!(mini_n <= PER_FAT);
    let mini_len = mini_n * MINI;
    let mini_secs = mini_len.div_ceil(SECTOR);
    // 섹터 0 FAT, 1 디렉터리, 2 미니 FAT, 3.. 미니 스트림, 그다음 패키지 (FAT·DIFAT 섹터가 끼어든다)
    let mut fats = vec![0usize];
    let mut difats: Vec<usize> = Vec::new();
    let mut pkg_secs: Vec<usize> = Vec::new();
    let mut next = 3 + mini_secs;
    let big = if small { &[][..] } else { pkg };
    let first = big.len().min(8);
    let ends = std::iter::once(first).filter(|&e| e > 0).chain(
        (first..big.len())
            .step_by(CHUNK)
            .map(|o| (o + CHUNK).min(big.len())),
    );
    for end in ends {
        let want = end.div_ceil(SECTOR);
        while next + want.saturating_sub(pkg_secs.len()) > fats.len() * PER_FAT {
            if fats.len() >= HEADER_DIFAT + difats.len() * (PER_FAT - 1) {
                difats.push(next);
            } else {
                fats.push(next);
            }
            next += 1;
        }
        while pkg_secs.len() < want {
            pkg_secs.push(next);
            next += 1;
        }
    }

    // FAT
    let mut fat = vec![FREE; fats.len() * PER_FAT];
    for &f in &fats {
        fat[f] = FAT_SECT;
    }
    for &d in &difats {
        fat[d] = DIFAT_SECT;
    }
    fat[1] = END_OF_CHAIN;
    fat[2] = END_OF_CHAIN;
    let chain = |fat: &mut Vec<u32>, secs: &[usize]| {
        for w in secs.windows(2) {
            fat[w[0]] = w[1] as u32;
        }
        if let Some(&last) = secs.last() {
            fat[last] = END_OF_CHAIN;
        }
    };
    let mini_chain: Vec<usize> = (3..3 + mini_secs).collect();
    chain(&mut fat, &mini_chain);
    chain(&mut fat, &pkg_secs);

    let mut out = vec![0u8; SECTOR * (1 + next)];
    let at = |s: usize| SECTOR * (1 + s);

    // 머리
    out[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    put_u16(&mut out, 24, 0x003E);
    put_u16(&mut out, 26, 3);
    put_u16(&mut out, 28, 0xFFFE);
    put_u16(&mut out, 30, 9);
    put_u16(&mut out, 32, 6);
    put_u32(&mut out, 44, fats.len() as u32);
    put_u32(&mut out, 48, 1);
    put_u32(&mut out, 56, 4096);
    put_u32(&mut out, 60, 2);
    put_u32(&mut out, 64, 1);
    put_u32(
        &mut out,
        68,
        difats.first().map_or(END_OF_CHAIN, |&d| d as u32),
    );
    put_u32(&mut out, 72, difats.len() as u32);
    for i in 0..HEADER_DIFAT {
        let v = fats.get(i).map_or(FREE, |&f| f as u32);
        put_u32(&mut out, 76 + i * 4, v);
    }
    // DIFAT 섹터: FAT 자리 127개 + 다음 DIFAT 섹터
    for (k, &d) in difats.iter().enumerate() {
        let base = at(d);
        for i in 0..PER_FAT - 1 {
            let v = fats
                .get(HEADER_DIFAT + k * (PER_FAT - 1) + i)
                .map_or(FREE, |&f| f as u32);
            put_u32(&mut out, base + i * 4, v);
        }
        let nxt = difats.get(k + 1).map_or(END_OF_CHAIN, |&n| n as u32);
        put_u32(&mut out, base + (PER_FAT - 1) * 4, nxt);
    }
    for (i, &f) in fats.iter().enumerate() {
        for j in 0..PER_FAT {
            put_u32(&mut out, at(f) + j * 4, fat[i * PER_FAT + j]);
        }
    }

    // 디렉터리
    let d = at(1);
    let root_start = if mini_secs > 0 { 3 } else { END_OF_CHAIN };
    dir_entry(
        &mut out[d..d + 128],
        "Root Entry",
        5,
        0,
        NO_STREAM,
        1,
        root_start,
        mini_len,
    );
    dir_entry(
        &mut out[d + 128..d + 256],
        "EncryptionInfo",
        2,
        1,
        2,
        NO_STREAM,
        0,
        info.len(),
    );
    let pkg_start = if small {
        info_mini as u32
    } else {
        pkg_secs.first().map_or(END_OF_CHAIN, |&s| s as u32)
    };
    dir_entry(
        &mut out[d + 256..d + 384],
        "EncryptedPackage",
        2,
        0,
        NO_STREAM,
        NO_STREAM,
        pkg_start,
        pkg.len(),
    );
    dir_entry(
        &mut out[d + 384..d + 512],
        "",
        0,
        0,
        NO_STREAM,
        NO_STREAM,
        0,
        0,
    );

    // 미니 FAT (EncryptionInfo 사슬, 작은 패키지 사슬), 미니 스트림
    let m = at(2);
    for i in 0..PER_FAT {
        let v = if i + 1 == info_mini || i + 1 == mini_n {
            END_OF_CHAIN
        } else if i + 1 < mini_n {
            (i + 1) as u32
        } else {
            FREE
        };
        put_u32(&mut out, m + i * 4, v);
    }
    let mut mini = info.to_vec();
    mini.resize(info_mini * MINI, 0);
    if small {
        mini.extend_from_slice(pkg);
    }
    for (k, part) in mini.chunks(SECTOR).enumerate() {
        let o = at(3 + k);
        out[o..o + part.len()].copy_from_slice(part);
    }
    // 패키지
    for (part, &s) in big.chunks(SECTOR).zip(&pkg_secs) {
        let o = at(s);
        out[o..o + part.len()].copy_from_slice(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FAT 섹터 자리 (머리의 DIFAT 표)
    fn fat_sectors(b: &[u8]) -> Vec<u32> {
        let n = u32::from_le_bytes(b[44..48].try_into().unwrap()) as usize;
        (0..n.min(HEADER_DIFAT))
            .map(|i| u32::from_le_bytes(b[76 + i * 4..80 + i * 4].try_into().unwrap()))
            .collect()
    }

    // 한셀이 쓴 구조 잠금 문서의 FAT 섹터 자리 (관측): EncryptedPackage 5,608 / 100,952 / 722,952 바이트
    #[test]
    fn fat_sectors_land_where_hancell_puts_them() {
        let info = vec![0u8; 224];
        for (len, want) in [
            (5_608usize, vec![0u32]),
            (100_952, vec![0, 125]),
            (
                722_952,
                vec![0, 125, 254, 383, 512, 633, 762, 891, 1020, 1149, 1278, 1407],
            ),
        ] {
            let pkg = vec![0x11u8; len];
            let b = container(&info, &pkg);
            assert_eq!(fat_sectors(&b), want, "{len}");
        }
    }

    #[test]
    fn wrapped_package_opens_again() {
        // 4096 바이트보다 작은 패키지는 미니 스트림에 들어간다
        for n in [20_000u32, 3_000] {
            let plain: Vec<u8> = (0..n).map(|i| (i % 253) as u8).collect();
            let b = wrap(&plain);
            let entries = crate::container::list_entries(&b).unwrap();
            let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
            assert!(names.contains(&"/EncryptionInfo") && names.contains(&"/EncryptedPackage"));
            let info = crate::container::read_entry(&b, "/EncryptionInfo").unwrap();
            assert_eq!(info.len(), 224);
            let pkg = crate::container::read_entry(&b, "/EncryptedPackage").unwrap();
            assert_eq!(
                crypto::decrypt_package(&info, &pkg, crypto::DEFAULT_PASSWORD).unwrap(),
                plain,
                "{n}"
            );
        }
    }

    #[test]
    fn many_fat_sectors_use_difat() {
        // FAT 섹터 109개를 넘으면 DIFAT 섹터를 잡는다 (패키지 약 7 MB 넘게)
        let info = vec![0u8; 224];
        let pkg = vec![0x22u8; 7_400_000];
        let b = container(&info, &pkg);
        let difat_n = u32::from_le_bytes(b[72..76].try_into().unwrap());
        assert_eq!(difat_n, 1);
        let entries = crate::container::list_entries(&b).unwrap();
        assert!(
            entries
                .iter()
                .any(|(n, s)| n == "/EncryptedPackage" && *s == 7_400_000)
        );
        assert_eq!(
            crate::container::read_entry(&b, "/EncryptedPackage").unwrap(),
            pkg
        );
    }
}
