// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 넥셀(.nxl) 의 매크로 프로젝트 스트림 `/_VBS_PROJECT` ↔ 한셀의 `/_VBS_PROJECT_W`.
//!
//! 둘 다 머리 256바이트가 같고, 뒤에 글들이 온다 — 넥셀은 맨 앞에 빈 글이 하나 더 있다 (한셀이 매크로 문서를
//! 레거시와 .nxl 로 저장한 것, 표본 하나):
//!
//! - `_W`: 글마다 글자 수 u32 + UTF-16, 줄 끝 LF, 모듈 글은 `CLASS 이름` 줄로 시작해 `End Class\n` 로 끝난다
//! - 넥셀: 글마다 길이 (u8, 0xFF 면 u16, 0xFFFF 면 u32 — 한셀은 늘 0xFF + u16) + 코드 페이지 949, 줄 끝 CRLF, `CLASS` 줄의 글과 `End Class`
//!   줄이 없다 (첫 줄은 빈 줄)

/// 머리 길이
const HEAD: usize = 256;
/// 넥셀이 버리는 모듈 이름 — 한셀은 되읽을 때 이 이름을 붙인다 (표본 하나: 통합 문서 모듈)
const MODULE: &str = "CLS_GM_MODULE";

fn utf16_strings(b: &[u8]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 4 <= b.len() {
        let n = u32::from_le_bytes(b[p..p + 4].try_into().ok()?) as usize;
        let s = b.get(p + 4..p + 4 + n * 2)?;
        out.push(crate::text::utf16le(s));
        p += 4 + n * 2;
    }
    (p == b.len()).then_some(out)
}

fn archive_strings(b: &[u8]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        let mut n = b[p] as usize;
        p += 1;
        if n == 0xFF {
            n = u16::from_le_bytes([*b.get(p)?, *b.get(p + 1)?]) as usize;
            p += 2;
            if n == 0xFFFF {
                n = u32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?) as usize;
                p += 4;
            }
        }
        out.push(crate::calc::cp949::decode(b.get(p..p + n)?));
        p += n;
    }
    Some(out)
}

/// 넥셀 `/_VBS_PROJECT` → `/_VBS_PROJECT_W`
pub(crate) fn from_nexcel(b: &[u8]) -> Option<Vec<u8>> {
    let strings = archive_strings(b.get(HEAD..)?)?;
    let mut o = b[..HEAD].to_vec();
    // 넥셀은 맨 앞에 빈 글이 하나 더 있다
    for s in strings.into_iter().skip(1) {
        let w = if s.is_empty() {
            String::new()
        } else {
            format!("CLASS {MODULE}{}End Class\n", s.replace("\r\n", "\n"))
        };
        let u: Vec<u16> = w.encode_utf16().collect();
        o.extend_from_slice(&(u.len() as u32).to_le_bytes());
        for c in u {
            o.extend_from_slice(&c.to_le_bytes());
        }
    }
    Some(o)
}

/// `/_VBS_PROJECT_W` → 넥셀 `/_VBS_PROJECT`
pub(crate) fn to_nexcel(b: &[u8]) -> Option<Vec<u8>> {
    let strings = utf16_strings(b.get(HEAD..)?)?;
    let mut o = b[..HEAD].to_vec();
    o.extend_from_slice(&[0xFF, 0, 0]);
    for s in strings {
        let mut t = s.as_str();
        if let Some(rest) = t.strip_prefix("CLASS ") {
            t = rest.find('\n').map_or("", |i| &rest[i..]);
        }
        let t = t.strip_suffix("End Class\n").unwrap_or(t);
        let bytes = crate::calc::cp949::encode_lossy(&t.replace('\n', "\r\n"));
        let n = bytes.len();
        // 한셀은 짧은 글도 0xFF + u16 꼴로 쓴다
        if n < 0xFFFF {
            o.push(0xFF);
            o.extend_from_slice(&(n as u16).to_le_bytes());
        } else {
            o.extend_from_slice(&[0xFF, 0xFF, 0xFF]);
            o.extend_from_slice(&(n as u32).to_le_bytes());
        }
        o.extend_from_slice(&bytes);
    }
    Some(o)
}
