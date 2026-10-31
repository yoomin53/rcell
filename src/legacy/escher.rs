// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 그리기: OfficeArt(Escher) 레코드 ([MS-ODRAW]).
//!
//! 시트의 MSODRAWING 레코드 내용을 이어 붙이면 OfficeArt 스트림 하나가 된다:
//! `DgContainer(F002) → SpgrContainer(F003) → SpContainer(F004)…`. 첫 SpContainer 는 그룹 전체
//! (patriarch) 이고, 나머지 도형마다 SpContainer 하나와 그 뒤의 OBJ 레코드(+ TXO)가 짝을 이룬다.

/// OfficeArt 레코드 머리: ver/inst u16, type u16, len u32
#[derive(Debug, Clone, Copy)]
struct Header {
    inst: u16,
    rtype: u16,
    len: usize,
}

fn header(d: &[u8], at: usize) -> Option<Header> {
    let h = d.get(at..at + 8)?;
    let vi = u16::from_le_bytes([h[0], h[1]]);
    Some(Header {
        inst: vi >> 4,
        rtype: u16::from_le_bytes([h[2], h[3]]),
        len: u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize,
    })
}

/// 도형 하나 (SpContainer)
#[derive(Debug, Clone, Default)]
pub struct EscherShape {
    /// FSP 의 모양 종류 (MSOSPT)
    pub shape_type: u16,
    pub spid: u32,
    /// FSP 플래그 (0x0001 그룹, 0x0002 자식, 0x0004 patriarch, 0x0040 가로 뒤집기, 0x0080 세로 뒤집기 …)
    pub flags: u32,
    /// OPT 속성: (pid, 값, 복합 데이터)
    pub props: Vec<(u16, u32, Vec<u8>)>,
    /// OfficeArtClientAnchor (한컴: 행 u32) — col1, dx1, rw1, dy1, col2, dx2, rw2, dy2.
    /// dx 는 열 너비의 1/32768, dy 는 행 높이의 1/8192 (관측)
    pub anchor: Option<[u32; 8]>,
    /// 앵커 플래그: 0 칸과 함께 이동·크기 조절, 2 이동만, 3 고정 (BIFF8)
    pub anchor_flag: u16,
    /// OfficeArtChildAnchor (그룹 안 좌표)
    pub child_anchor: Option<[i32; 4]>,
    /// 그룹의 자식 좌표 범위 (OfficeArtFSPGR: left, top, right, bottom)
    pub group_rect: Option<[i32; 4]>,
    /// 그룹이면 자식 (SpgrContainer)
    pub children: Vec<EscherShape>,
    /// ClientTextbox 가 있으면 TXO 가 따른다
    pub has_textbox: bool,
}

impl EscherShape {
    pub fn prop(&self, pid: u16) -> Option<u32> {
        self.props.iter().find(|p| p.0 == pid).map(|p| p.1)
    }

    pub fn complex(&self, pid: u16) -> Option<&[u8]> {
        self.props
            .iter()
            .find(|p| p.0 == pid)
            .map(|p| p.2.as_slice())
    }

    /// UTF-16 문자열 속성 (wzName 0x0380 등)
    pub fn string(&self, pid: u16) -> Option<String> {
        let b = self.complex(pid)?;
        let s = crate::text::utf16le(b);
        Some(s.trim_end_matches('\0').to_string())
    }

    pub fn is_group(&self) -> bool {
        self.flags & 0x0001 != 0
    }

    pub fn is_patriarch(&self) -> bool {
        self.flags & 0x0004 != 0
    }
}

/// OfficeArt 스트림 → 맨 위 그룹의 도형들 (patriarch 제외, 문서 순서)
pub fn parse(stream: &[u8]) -> Vec<EscherShape> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(h) = header(stream, at) {
        let body = at + 8;
        let end = (body + h.len).min(stream.len());
        match h.rtype {
            // DgContainer: 안으로
            0xF002 => {
                out.extend(parse(&stream[body..end]));
            }
            // SpgrContainer: 첫 SpContainer 는 그룹 자신
            0xF003 => {
                let mut items = group(&stream[body..end]);
                if items.first().is_some_and(|s| s.is_patriarch()) {
                    let p = items.remove(0);
                    out.extend(p.children);
                    out.extend(items);
                } else {
                    out.extend(items);
                }
            }
            _ => {}
        }
        at = end;
    }
    out
}

/// SpgrContainer 내용: SpContainer(도형) 또는 SpgrContainer(하위 그룹)의 나열.
/// 하위 그룹은 첫 SpContainer 가 그룹 도형이고 나머지가 자식이다.
fn group(d: &[u8]) -> Vec<EscherShape> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(h) = header(d, at) {
        let body = at + 8;
        let end = (body + h.len).min(d.len());
        match h.rtype {
            0xF004 => out.push(shape(&d[body..end])),
            0xF003 => {
                let mut items = group(&d[body..end]);
                if !items.is_empty() {
                    let mut g = items.remove(0);
                    g.children = items;
                    out.push(g);
                }
            }
            _ => {}
        }
        at = end;
    }
    out
}

fn shape(d: &[u8]) -> EscherShape {
    let mut s = EscherShape::default();
    let mut at = 0;
    while let Some(h) = header(d, at) {
        let body = at + 8;
        let end = (body + h.len).min(d.len());
        let b = &d[body..end];
        let u16_at = |i: usize| b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]));
        let u32_at = |i: usize| {
            b.get(i..i + 4)
                .map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
        };
        match h.rtype {
            // FSP: spid u32, flags u32 (모양 종류는 inst)
            0xF00A => {
                s.shape_type = h.inst;
                s.spid = u32_at(0).unwrap_or(0);
                s.flags = u32_at(4).unwrap_or(0);
            }
            // OPT / 보조 OPT / 3차 OPT: inst = 속성 수
            0xF00B | 0xF121 | 0xF122 => {
                let n = h.inst as usize;
                let mut complex_at = n * 6;
                for i in 0..n {
                    let (Some(id), Some(v)) = (u16_at(i * 6), u32_at(i * 6 + 2)) else {
                        break;
                    };
                    let pid = id & 0x3FFF;
                    let data = if id & 0x8000 != 0 {
                        let len = v as usize;
                        let x = b
                            .get(complex_at..complex_at + len)
                            .unwrap_or_default()
                            .to_vec();
                        complex_at += len;
                        x
                    } else {
                        Vec::new()
                    };
                    s.props.push((pid, v, data));
                }
            }
            // OfficeArtClientAnchor (한컴 22바이트: 행 u32)
            0xF010 if b.len() >= 22 => {
                s.anchor_flag = u16_at(0).unwrap_or(0);
                let col1 = u16_at(2).unwrap_or(0) as u32;
                let dx1 = u16_at(4).unwrap_or(0) as u32;
                let rw1 = u32_at(6).unwrap_or(0);
                let dy1 = u16_at(10).unwrap_or(0) as u32;
                let col2 = u16_at(12).unwrap_or(0) as u32;
                let dx2 = u16_at(14).unwrap_or(0) as u32;
                let rw2 = u32_at(16).unwrap_or(0);
                let dy2 = u16_at(20).unwrap_or(0) as u32;
                s.anchor = Some([col1, dx1, rw1, dy1, col2, dx2, rw2, dy2]);
            }
            // OfficeArtFSPGR
            0xF009 => {
                let v: Vec<i32> = (0..4).map(|i| u32_at(i * 4).unwrap_or(0) as i32).collect();
                s.group_rect = Some([v[0], v[1], v[2], v[3]]);
            }
            // OfficeArtChildAnchor: left, top, right, bottom (i32)
            0xF00F => {
                let v: Vec<i32> = (0..4).map(|i| u32_at(i * 4).unwrap_or(0) as i32).collect();
                s.child_anchor = Some([v[0], v[1], v[2], v[3]]);
            }
            0xF00D => s.has_textbox = true,
            _ => {}
        }
        at = end;
    }
    s
}

/// 그림 저장소(BStore)의 그림 하나
#[derive(Debug, Clone)]
pub struct Blip {
    /// [`crate::image::ext_and_type`] 의 형식 이름 (`png`, `jpeg`, `bmp`, `emf` …)
    pub format: &'static str,
    pub data: Vec<u8>,
}

/// MSODRAWINGGROUP 의 OfficeArtDggContainer 에서 그림 저장소를 꺼낸다.
/// 색인 = 도형 속성 `pib`(0x0104) − 1. 빈 자리(cRef 0, 해석할 수 없는 그림)는 `None`.
pub fn blip_store(dgg: &[u8]) -> Vec<Option<Blip>> {
    let mut out = Vec::new();
    let Some(root) = header(dgg, 0).filter(|h| h.rtype == 0xF000) else {
        return out;
    };
    let end = (8 + root.len).min(dgg.len());
    let mut at = 8;
    while let Some(h) = header(dgg, at).filter(|_| at + 8 <= end) {
        let body = at + 8;
        if h.rtype == 0xF001 {
            let bend = (body + h.len).min(end);
            let mut i = body;
            while let Some(e) = header(dgg, i).filter(|_| i + 8 <= bend) {
                let b = &dgg[i + 8..(i + 8 + e.len).min(bend)];
                out.push(match e.rtype {
                    // FBSE: btWin32, btMacOS, rgbUid[16], tag u16, size u32, cRef u32, foDelay u32,
                    // 예약 u8, cbName u8, 예약 u8 ×2, 이름, 이어서 그림 레코드
                    0xF007 if b.len() >= 36 => {
                        let cref = u32::from_le_bytes([b[24], b[25], b[26], b[27]]);
                        let name_len = b[33] as usize;
                        (cref != 0)
                            .then(|| b.get(36 + name_len..))
                            .flatten()
                            .and_then(blip)
                    }
                    0xF018..=0xF117 => blip(&dgg[i..(i + 8 + e.len).min(bend)]),
                    _ => None,
                });
                i += 8 + e.len;
            }
        }
        at = body + h.len;
    }
    out
}

/// OfficeArtBlip 레코드 하나 (머리 포함) → 그림 파일.
/// 비트맵: rgbUid (inst 의 낮은 비트가 1 이면 하나 더), tag u8, 파일.
/// 메타파일: rgbUid, OfficeArtMetafileHeader 34바이트 (cbSize, rcBounds, ptSize, cbSave, 압축 u8 (0 deflate, 0xFE 없음), filter), 파일.
fn blip(rec: &[u8]) -> Option<Blip> {
    let h = header(rec, 0)?;
    let d = rec.get(8..(8 + h.len).min(rec.len()))?;
    let uids = 16 * (1 + (h.inst & 1) as usize);
    let (format, meta) = match h.rtype {
        0xF01A => ("emf", true),
        0xF01B => ("wmf", true),
        0xF01C => ("pict", true),
        0xF01D | 0xF02A => ("jpeg", false),
        0xF01E => ("png", false),
        0xF01F => ("bmp", false),
        0xF029 => ("tiff", false),
        _ => return None,
    };
    if !meta {
        let data = d.get(uids + 1..)?;
        if format == "bmp" {
            // DIB → BMP: BITMAPFILEHEADER 14바이트 (픽셀 위치 = 머리 + 머리 크기 + 색표)
            let hdr = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?) as usize;
            let bits = u16::from_le_bytes(data.get(14..16)?.try_into().ok()?) as usize;
            let used = u32::from_le_bytes(data.get(32..36)?.try_into().ok()?) as usize;
            let colors = if used != 0 {
                used
            } else if bits <= 8 {
                1 << bits
            } else {
                0
            };
            let off = (14 + hdr + colors * 4) as u32;
            let mut f = b"BM".to_vec();
            f.extend_from_slice(&(14 + data.len() as u32).to_le_bytes());
            f.extend_from_slice(&[0, 0, 0, 0]);
            f.extend_from_slice(&off.to_le_bytes());
            f.extend_from_slice(data);
            return Some(Blip { format, data: f });
        }
        return Some(Blip {
            format,
            data: data.to_vec(),
        });
    }
    let mh = d.get(uids..uids + 34)?;
    let size = u32::from_le_bytes(mh[0..4].try_into().ok()?) as usize;
    let saved = d.get(uids + 34..)?;
    let data = if mh[32] == 0 {
        use std::io::Read;
        let mut out = Vec::with_capacity(size);
        flate2::read::ZlibDecoder::new(saved)
            .read_to_end(&mut out)
            .ok()?;
        out
    } else {
        saved.to_vec()
    };
    Some(Blip { format, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ver_inst: u16, rtype: u16, body: &[u8]) -> Vec<u8> {
        let mut v = ver_inst.to_le_bytes().to_vec();
        v.extend_from_slice(&rtype.to_le_bytes());
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    fn fbse(cref: u32, blip: &[u8]) -> Vec<u8> {
        let mut b = vec![6, 6];
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&[0xFF, 0]);
        b.extend_from_slice(&(blip.len() as u32).to_le_bytes());
        b.extend_from_slice(&cref.to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend_from_slice(blip);
        rec(0x0062, 0xF007, &b)
    }

    #[test]
    fn blip_store_extracts_bitmaps_dibs_and_compressed_metafiles() {
        use std::io::Write;
        // PNG (inst 0x6E0: uid 하나), 빈 자리, DIB (0x7A9: uid 둘), 압축한 EMF (0x3D4)
        let png = b"\x89PNG\r\n\x1a\nrest";
        let mut png_blip = vec![0; 16];
        png_blip.push(0xFF);
        png_blip.extend_from_slice(png);
        let mut dib = 40u32.to_le_bytes().to_vec();
        dib.extend_from_slice(&2u32.to_le_bytes());
        dib.extend_from_slice(&1u32.to_le_bytes());
        dib.extend_from_slice(&[1, 0, 24, 0]);
        dib.resize(40 + 8, 0);
        let mut dib_blip = vec![0; 32];
        dib_blip.push(0xFF);
        dib_blip.extend_from_slice(&dib);
        let emf = b"\x01\0\0\0 EMF metafile bytes".to_vec();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&emf).unwrap();
        let packed = z.finish().unwrap();
        let mut emf_blip = vec![0; 16];
        emf_blip.extend_from_slice(&(emf.len() as u32).to_le_bytes());
        emf_blip.extend_from_slice(&[0; 24]);
        emf_blip.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        emf_blip.extend_from_slice(&[0, 0xFE]);
        emf_blip.extend_from_slice(&packed);

        let mut store = fbse(1, &rec(0x6E00, 0xF01E, &png_blip));
        store.extend(fbse(0, &[]));
        store.extend(fbse(1, &rec(0x7A90, 0xF01F, &dib_blip)));
        store.extend(fbse(1, &rec(0x3D40, 0xF01A, &emf_blip)));
        let mut dgg = rec(0x0000, 0xF006, &[0; 16]);
        dgg.extend(rec(0x004F, 0xF001, &store));
        let blips = blip_store(&rec(0x000F, 0xF000, &dgg));

        assert_eq!(blips.len(), 4);
        let b0 = blips[0].as_ref().unwrap();
        assert_eq!((b0.format, &b0.data[..]), ("png", &png[..]));
        assert!(blips[1].is_none());
        let b2 = blips[2].as_ref().unwrap();
        assert_eq!(b2.format, "bmp");
        assert_eq!(&b2.data[..2], b"BM");
        // 픽셀 위치 = 14 + 40 (24비트라 색표 없음)
        assert_eq!(u32::from_le_bytes(b2.data[10..14].try_into().unwrap()), 54);
        let info = crate::image::info(&b2.data).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        let b3 = blips[3].as_ref().unwrap();
        assert_eq!((b3.format, &b3.data[..]), ("emf", &emf[..]));
    }
}
