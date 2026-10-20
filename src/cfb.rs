// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 읽기 전용 OLE2 복합 문서(CFB) 리더 — 공개 명세 [MS-CFB] 구현.
//!
//! 레거시 `.cell` 의 `/Workbook` 스트림을 꺼내는 데 쓴다. 외부 크레이트 없이 구현해
//! WebAssembly 빌드에 JS 의존성이 생기지 않게 했다.

use crate::error::{Error, Result};

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
const NO_STREAM: u32 = 0xFFFF_FFFF;

fn bad(m: &str) -> Error {
    Error::InvalidValue {
        context: "OLE2(CFB)".into(),
        value: m.into(),
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

#[derive(Debug, Clone)]
struct DirEntry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// 복합 문서. 스트림 경로는 `/Workbook`, `/폴더/스트림` 꼴이다.
pub struct Cfb<'a> {
    data: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini_stream: Vec<u8>,
    cutoff: u64,
    /// (경로, 디렉터리 항목 번호)
    paths: Vec<(String, usize)>,
    entries: Vec<DirEntry>,
}

impl<'a> Cfb<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self> {
        if data.len() < 512 || data[..8] != SIGNATURE {
            return Err(bad("시그니처가 없습니다"));
        }
        let shift = u16_at(data, 0x1E).ok_or_else(|| bad("헤더"))?;
        if !(7..=16).contains(&shift) {
            return Err(bad("섹터 크기"));
        }
        let sector_size = 1usize << shift;
        let num_fat = u32_at(data, 0x2C).unwrap_or(0) as usize;
        let first_dir = u32_at(data, 0x30).unwrap_or(END_OF_CHAIN);
        let cutoff = u32_at(data, 0x38).unwrap_or(4096) as u64;
        let first_mini_fat = u32_at(data, 0x3C).unwrap_or(END_OF_CHAIN);
        let first_difat = u32_at(data, 0x44).unwrap_or(END_OF_CHAIN);
        let num_difat = u32_at(data, 0x48).unwrap_or(0) as usize;

        let mut cfb = Cfb {
            data,
            sector_size,
            fat: Vec::new(),
            mini_fat: Vec::new(),
            mini_stream: Vec::new(),
            cutoff,
            paths: Vec::new(),
            entries: Vec::new(),
        };

        // DIFAT: 헤더의 109개 + DIFAT 섹터 사슬
        let mut fat_sectors: Vec<u32> = (0..109)
            .filter_map(|i| u32_at(data, 0x4C + i * 4))
            .filter(|&s| s != FREE)
            .collect();
        let mut next = first_difat;
        for _ in 0..num_difat {
            if next == END_OF_CHAIN || next == FREE {
                break;
            }
            let sec = cfb.sector(next).ok_or_else(|| bad("DIFAT 섹터"))?;
            let per = sector_size / 4 - 1;
            for i in 0..per {
                let v = u32_at(sec, i * 4).unwrap_or(FREE);
                if v != FREE {
                    fat_sectors.push(v);
                }
            }
            next = u32_at(sec, per * 4).unwrap_or(END_OF_CHAIN);
        }
        if num_fat > 0 {
            fat_sectors.truncate(num_fat);
        }
        for s in fat_sectors {
            let sec = cfb.sector(s).ok_or_else(|| bad("FAT 섹터"))?;
            cfb.fat
                .extend((0..sector_size / 4).filter_map(|i| u32_at(sec, i * 4)));
        }

        // 디렉터리
        let dir = cfb.chain(first_dir)?;
        for chunk in dir.as_chunks::<128>().0 {
            let name_len = u16_at(chunk, 64).unwrap_or(0) as usize;
            let name = crate::text::utf16le(&chunk[..name_len.saturating_sub(2).min(64)]);
            let size = if sector_size == 512 {
                u32_at(chunk, 120).unwrap_or(0) as u64
            } else {
                u64_at(chunk, 120).unwrap_or(0)
            };
            cfb.entries.push(DirEntry {
                name,
                kind: chunk[66],
                left: u32_at(chunk, 68).unwrap_or(NO_STREAM),
                right: u32_at(chunk, 72).unwrap_or(NO_STREAM),
                child: u32_at(chunk, 76).unwrap_or(NO_STREAM),
                start: u32_at(chunk, 116).unwrap_or(END_OF_CHAIN),
                size,
            });
        }
        let root = cfb
            .entries
            .first()
            .cloned()
            .ok_or_else(|| bad("루트 항목"))?;

        // 미니 스트림과 미니 FAT
        if root.start != END_OF_CHAIN {
            let mut ms = cfb.chain(root.start)?;
            ms.truncate(root.size as usize);
            cfb.mini_stream = ms;
        }
        if first_mini_fat != END_OF_CHAIN {
            let mf = cfb.chain(first_mini_fat)?;
            cfb.mini_fat = (0..mf.len() / 4)
                .filter_map(|i| u32_at(&mf, i * 4))
                .collect();
        }

        // 트리 순회로 경로 만들기
        let mut stack = vec![(root.child, String::new())];
        let mut seen = vec![false; cfb.entries.len()];
        while let Some((id, parent)) = stack.pop() {
            if id == NO_STREAM || id as usize >= cfb.entries.len() || seen[id as usize] {
                continue;
            }
            seen[id as usize] = true;
            let e = cfb.entries[id as usize].clone();
            let path = format!("{parent}/{}", e.name);
            stack.push((e.left, parent.clone()));
            stack.push((e.right, parent.clone()));
            if e.kind == 1 {
                stack.push((e.child, path.clone()));
            }
            cfb.paths.push((path, id as usize));
        }
        cfb.paths.sort();
        Ok(cfb)
    }

    /// 섹터 하나. 파일 끝의 마지막 섹터는 채움 없이 잘려 있을 수 있다 (한셀의 암호화 문서, 관측) —
    /// 스트림은 어차피 크기로 자르므로 있는 만큼만 돌려준다.
    fn sector(&self, n: u32) -> Option<&'a [u8]> {
        let start = (n as usize + 1).checked_mul(self.sector_size)?;
        if start >= self.data.len() {
            return None;
        }
        let end = (start + self.sector_size).min(self.data.len());
        self.data.get(start..end)
    }

    fn chain(&self, start: u32) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut n = start;
        let mut guard = 0;
        while n != END_OF_CHAIN && n != FREE {
            out.extend_from_slice(self.sector(n).ok_or_else(|| bad("섹터 범위"))?);
            n = *self.fat.get(n as usize).ok_or_else(|| bad("FAT 범위"))?;
            guard += 1;
            if guard > self.fat.len() + 1 {
                return Err(bad("FAT 순환"));
            }
        }
        Ok(out)
    }

    fn mini_chain(&self, start: u32) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut n = start;
        let mut guard = 0;
        while n != END_OF_CHAIN && n != FREE {
            let s = n as usize * 64;
            out.extend_from_slice(
                self.mini_stream
                    .get(s..s + 64)
                    .ok_or_else(|| bad("미니 섹터 범위"))?,
            );
            n = *self
                .mini_fat
                .get(n as usize)
                .ok_or_else(|| bad("미니 FAT 범위"))?;
            guard += 1;
            if guard > self.mini_fat.len() + 1 {
                return Err(bad("미니 FAT 순환"));
            }
        }
        Ok(out)
    }

    /// 스트림 경로와 크기.
    pub fn streams(&self) -> Vec<(String, u64)> {
        self.paths
            .iter()
            .filter(|(_, id)| self.entries[*id].kind == 2)
            .map(|(p, id)| (p.clone(), self.entries[*id].size))
            .collect()
    }

    /// 스트림 내용. 이름은 대소문자를 구분하지 않는다.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let (_, id) = self
            .paths
            .iter()
            .find(|(p, id)| p.eq_ignore_ascii_case(path) && self.entries[*id].kind == 2)
            .ok_or_else(|| Error::MissingPart(path.to_string()))?;
        let e = &self.entries[*id];
        let mut data = if e.size < self.cutoff {
            self.mini_chain(e.start)?
        } else {
            self.chain(e.start)?
        };
        data.truncate(e.size as usize);
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MS-CFB 명세대로 최소 복합 문서를 만든다: 섹터 0 = FAT, 1 = 디렉터리, 2 = 데이터.
    fn tiny_cfb(stream: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 512 * 4];
        f[..8].copy_from_slice(&SIGNATURE);
        f[0x18..0x1A].copy_from_slice(&0x3Eu16.to_le_bytes());
        f[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
        f[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
        f[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        f[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
        f[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes()); // FAT 섹터 수
        f[0x30..0x34].copy_from_slice(&1u32.to_le_bytes()); // 디렉터리 시작
        f[0x38..0x3C].copy_from_slice(&0u32.to_le_bytes()); // 미니 스트림 기준 0 → 일반 섹터
        f[0x3C..0x40].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        f[0x44..0x48].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        for i in 0..109 {
            let v = if i == 0 { 0 } else { FREE };
            f[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        // FAT (섹터 0)
        let fat = 512;
        for i in 0..128 {
            let v = match i {
                0 => 0xFFFF_FFFD,
                1 | 2 => END_OF_CHAIN,
                _ => FREE,
            };
            f[fat + i * 4..fat + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        // 디렉터리 (섹터 1)
        let entry = |f: &mut Vec<u8>,
                     idx: usize,
                     name: &str,
                     kind: u8,
                     child: u32,
                     start: u32,
                     size: u32| {
            let o = 1024 + idx * 128;
            let units: Vec<u16> = name.encode_utf16().collect();
            for (k, u) in units.iter().enumerate() {
                f[o + k * 2..o + k * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            f[o + 64..o + 66].copy_from_slice(&(((units.len() + 1) * 2) as u16).to_le_bytes());
            f[o + 66] = kind;
            f[o + 68..o + 72].copy_from_slice(&NO_STREAM.to_le_bytes());
            f[o + 72..o + 76].copy_from_slice(&NO_STREAM.to_le_bytes());
            f[o + 76..o + 80].copy_from_slice(&child.to_le_bytes());
            f[o + 116..o + 120].copy_from_slice(&start.to_le_bytes());
            f[o + 120..o + 124].copy_from_slice(&size.to_le_bytes());
        };
        entry(&mut f, 0, "Root Entry", 5, 1, END_OF_CHAIN, 0);
        entry(&mut f, 1, "Workbook", 2, NO_STREAM, 2, stream.len() as u32);
        // 데이터 (섹터 2)
        f[1536..1536 + stream.len()].copy_from_slice(stream);
        f
    }

    #[test]
    fn reads_stream_from_minimal_compound_file() {
        let bytes = tiny_cfb(b"\x09\x08\x02\x00\x01\x10");
        let c = Cfb::open(&bytes).unwrap();
        assert_eq!(c.streams(), vec![("/Workbook".to_string(), 6)]);
        assert_eq!(c.read("/workbook").unwrap(), b"\x09\x08\x02\x00\x01\x10");
        assert!(c.read("/Nope").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Cfb::open(b"not a compound file").is_err());
    }
}
