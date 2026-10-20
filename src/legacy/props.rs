// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 문서 속성: OLE 속성 집합 `\x05SummaryInformation` ([MS-OLEPS]).
//!
//! 머리: `0xFFFE u16, 판 u16, 시스템 u32, CLSID[16] (한셀은 FMTID 를 넣는다), 집합 수 u32`,
//! 집합마다 `FMTID[16], 위치 u32`. 집합: `크기 u32, 속성 수 u32, (PID u32, 위치 u32) × n`,
//! 값은 `종류 u16, 채움 u16` + 내용 (0x1E 코드 페이지 문자열, 0x1F UTF-16 문자열 `cch u32` (끝 NUL 포함),
//! 0x40 FILETIME, 0x02 i16, 0x03 i32). 한셀은 코드 페이지 1200(UTF-16)으로 쓴다.

use crate::model::DocProperties;

/// FMTID_SummaryInformation {F29F85E0-4FF9-1068-AB91-08002B27B3D9}
const FMTID_SUMMARY: [u8; 16] = [
    0xE0, 0x85, 0x9F, 0xF2, 0xF9, 0x4F, 0x68, 0x10, 0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9,
];

enum Value {
    Text(String),
    Time(u64),
}

/// `\x05SummaryInformation` 스트림 → 문서 속성
pub fn parse_summary(d: &[u8]) -> DocProperties {
    let mut p = DocProperties::default();
    let u32_at = |i: usize| {
        d.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if d.get(0..2) != Some(&[0xFE, 0xFF]) {
        return p;
    }
    let sets = u32_at(24).unwrap_or(0) as usize;
    let Some(base) = (0..sets.min(8)).find_map(|k| {
        let at = 28 + k * 20;
        (d.get(at..at + 16) == Some(&FMTID_SUMMARY[..]))
            .then(|| u32_at(at + 16))
            .flatten()
    }) else {
        return p;
    };
    let base = base as usize;
    let n = u32_at(base + 4).unwrap_or(0) as usize;
    let mut codepage = 1200u32;
    let mut props: Vec<(u32, usize)> = Vec::new();
    for i in 0..n.min(256) {
        let (Some(pid), Some(off)) = (u32_at(base + 8 + i * 8), u32_at(base + 12 + i * 8)) else {
            break;
        };
        props.push((pid, base + off as usize));
    }
    // 코드 페이지(PID 1)를 먼저
    if let Some(&(_, at)) = props.iter().find(|(pid, _)| *pid == 1)
        && d.get(at..at + 2) == Some(&[0x02, 0x00])
    {
        codepage = d
            .get(at + 4..at + 6)
            .map_or(1200, |b| u16::from_le_bytes([b[0], b[1]]) as u32);
    }
    let value = |at: usize| -> Option<Value> {
        let vt = d
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))?;
        match vt {
            0x1F => {
                let cch = u32_at(at + 4)? as usize;
                let b = d.get(at + 8..at + 8 + cch * 2)?;
                let w: Vec<u16> = b
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                Some(Value::Text(
                    String::from_utf16_lossy(&w)
                        .trim_end_matches('\0')
                        .to_string(),
                ))
            }
            0x1E => {
                let cb = u32_at(at + 4)? as usize;
                let b = d.get(at + 8..at + 8 + cb)?;
                let s = if codepage == 1200 {
                    let w: Vec<u16> = b
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    String::from_utf16_lossy(&w)
                } else {
                    // 그 밖의 코드 페이지는 ASCII 만 믿는다
                    b.iter()
                        .map(|&c| if c.is_ascii() { c as char } else { '?' })
                        .collect()
                };
                Some(Value::Text(s.trim_end_matches('\0').to_string()))
            }
            0x40 => {
                let lo = u32_at(at + 4)? as u64;
                let hi = u32_at(at + 8)? as u64;
                Some(Value::Time((hi << 32) | lo))
            }
            _ => None,
        }
    };
    for &(pid, at) in &props {
        let text = || match value(at) {
            Some(Value::Text(s)) if !s.is_empty() => Some(s),
            _ => None,
        };
        let time = || match value(at) {
            Some(Value::Time(t)) if t != 0 => Some(filetime_text(t)),
            _ => None,
        };
        match pid {
            2 => p.title = text(),
            3 => p.subject = text(),
            4 => p.creator = text(),
            5 => p.keywords = text(),
            6 => p.description = text(),
            8 => p.last_modified_by = text(),
            9 => p.revision = text(),
            0x0B => p.last_printed = time(),
            0x0C => p.created = time(),
            0x0D => p.modified = time(),
            _ => {}
        }
    }
    p
}

/// FILETIME (1601-01-01 UTC 부터 100ns) → `YYYY-MM-DDTHH:MM:SSZ`
pub fn filetime_text(ft: u64) -> String {
    let secs = (ft / 10_000_000) as i64 - 11_644_473_600;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // 1970-01-01 부터의 날 수 → 그레고리력 (Howard Hinnant 의 civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_to_w3cdtf() {
        assert_eq!(filetime_text(0x01BC_262E_3049_6A00), "1997-03-01T10:49:08Z");
        assert_eq!(filetime_text(0x01BC_595E_1ABE_2C80), "1997-05-05T14:10:37Z");
        assert_eq!(
            filetime_text(116_444_736_000_000_000),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn reads_summary_information_property_set() {
        // 제목(0x1F), 작성자(0x1F), 만든 날짜(0x40)
        let mut set = Vec::new();
        let mut vals: Vec<(u32, Vec<u8>)> = Vec::new();
        let wstr = |s: &str| {
            let w: Vec<u16> = s.encode_utf16().chain([0]).collect();
            let mut v = vec![0x1F, 0, 0, 0];
            v.extend_from_slice(&(w.len() as u32).to_le_bytes());
            v.extend(w.iter().flat_map(|c| c.to_le_bytes()));
            while v.len() % 4 != 0 {
                v.push(0);
            }
            v
        };
        vals.push((2, wstr("자동차 경비 보고서")));
        vals.push((4, wstr("yoomin53")));
        let mut t = vec![0x40, 0, 0, 0];
        t.extend_from_slice(&0x01BC_262E_3049_6A00u64.to_le_bytes());
        vals.push((0x0C, t));
        let head = 8 + vals.len() * 8;
        let mut off = head;
        let mut body = Vec::new();
        set.extend_from_slice(&0u32.to_le_bytes()); // 크기 (나중에)
        set.extend_from_slice(&(vals.len() as u32).to_le_bytes());
        for (pid, v) in &vals {
            set.extend_from_slice(&pid.to_le_bytes());
            set.extend_from_slice(&(off as u32).to_le_bytes());
            off += v.len();
            body.extend_from_slice(v);
        }
        set.extend(body);
        let size = set.len() as u32;
        set[0..4].copy_from_slice(&size.to_le_bytes());
        let mut d = vec![0xFE, 0xFF, 0, 0, 0x0D, 0, 0, 0];
        d.extend_from_slice(&FMTID_SUMMARY);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&FMTID_SUMMARY);
        d.extend_from_slice(&48u32.to_le_bytes());
        d.extend(set);

        let p = parse_summary(&d);
        assert_eq!(p.title.as_deref(), Some("자동차 경비 보고서"));
        assert_eq!(p.creator.as_deref(), Some("yoomin53"));
        assert_eq!(p.created.as_deref(), Some("1997-03-01T10:49:08Z"));
        assert_eq!(p.subject, None);
    }
}
