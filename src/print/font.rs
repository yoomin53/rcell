// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! PDF 글꼴: 시스템 글꼴 찾기(이름 표만 읽는다), 글자 폭, 글꼴 파일 부분 집합 넣기.
//!
//! 한셀 PDF 는 쓴 글꼴을 모두 부분 집합 TrueType(CIDFontType2, Identity-H)으로 넣고, 굵은 글꼴은 따로 넣는다
//! (맑은 고딕 굵게 = malgunbd.ttf). 글꼴에 없는 글자(Arial 칸의 한글 등)는 맑은 고딕으로 쓴다.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 글꼴 파일 하나 (모음 파일이면 그 안의 번호)
#[derive(Debug, Clone)]
struct FaceInfo {
    src: Src,
    index: u32,
    /// 소문자·공백 없앤 가족 이름들 (이름 표 1·16) 과 전체 이름 (4)
    names: Vec<String>,
    bold: bool,
    italic: bool,
}

#[derive(Debug, Clone)]
enum Src {
    Path(PathBuf),
    Data(Arc<Vec<u8>>),
}

/// 쓸 수 있는 글꼴 목록
#[derive(Debug, Default)]
pub struct FontDb {
    faces: Vec<FaceInfo>,
    /// 읽은 글꼴 파일
    loaded: std::sync::Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>,
}

/// 찾은 글꼴: 목록 번호와 흉내 낼 굵게·기울임
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceRef {
    pub face: usize,
    pub synth_bold: bool,
    pub synth_italic: bool,
}

/// 글꼴 이름 비교용: 소문자, 공백 없이
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 한셀이 문서 글꼴 이름 대신 찾는 이름 (가족 이름이 문서의 이름과 다른 글꼴)
fn aliases(name: &str) -> &'static [&'static str] {
    match norm(name).as_str() {
        "함초롬돋움" | "hcrdotum" => &["함초롬돋움", "HCR Dotum"],
        "함초롬바탕" | "hcrbatang" => &["함초롬바탕", "HCR Batang"],
        "맑은고딕" | "malgungothic" => &["맑은 고딕", "Malgun Gothic"],
        _ => &[],
    }
}

/// 글꼴이 없을 때 차례로 찾는 대체 글꼴 (한셀은 없는 글자를 맑은 고딕으로 쓴다)
const FALLBACK: &[&str] = &[
    "맑은 고딕",
    "Malgun Gothic",
    "함초롬돋움",
    "굴림",
    "Noto Sans CJK KR",
    "Noto Sans KR",
    "NanumGothic",
    "Apple SD Gothic Neo",
    "DejaVu Sans",
    "Arial",
];

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]))
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(o)?,
        *b.get(o + 1)?,
        *b.get(o + 2)?,
        *b.get(o + 3)?,
    ]))
}

/// 이름 표에서 가족·전체 이름
fn parse_names(t: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let (Some(count), Some(base)) = (be16(t, 2), be16(t, 4)) else {
        return out;
    };
    for i in 0..count as usize {
        let r = 6 + i * 12;
        let (Some(pid), Some(eid), Some(nid), Some(len), Some(off)) = (
            be16(t, r),
            be16(t, r + 2),
            be16(t, r + 6),
            be16(t, r + 8),
            be16(t, r + 10),
        ) else {
            break;
        };
        if !matches!(nid, 1 | 4 | 16) {
            continue;
        }
        let s = base as usize + off as usize;
        let Some(bytes) = t.get(s..s + len as usize) else {
            continue;
        };
        let text = match (pid, eid) {
            (0, _) | (3, 0 | 1 | 10) => {
                let u: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                String::from_utf16_lossy(&u)
            }
            (1, 0) => bytes.iter().map(|&b| b as char).collect(),
            _ => continue,
        };
        let n = norm(&text);
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// 글꼴 파일의 한 얼굴(offset 의 표 목록)에서 이름·굵게·기울임을 읽는다 (필요한 표만)
fn read_face<R: Read + Seek>(f: &mut R, offset: u64) -> Option<(Vec<String>, bool, bool)> {
    let mut head = [0u8; 12];
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(&mut head).ok()?;
    let n = be16(&head, 4)? as usize;
    let mut dir = vec![0u8; n * 16];
    f.read_exact(&mut dir).ok()?;
    let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
        let i = (0..n).find(|&i| &dir[i * 16..i * 16 + 4] == tag)?;
        let off = be32(&dir, i * 16 + 8)? as u64;
        let len = (be32(&dir, i * 16 + 12)? as usize).min(1 << 20);
        let mut buf = vec![0u8; len];
        f.seek(SeekFrom::Start(off)).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let names = parse_names(&table(b"name")?);
    let (bold, italic) = match table(b"OS/2") {
        Some(os2) => {
            let weight = be16(&os2, 4).unwrap_or(400);
            let sel = be16(&os2, 62).unwrap_or(0);
            (sel & 0x20 != 0 || weight >= 600, sel & 0x01 != 0)
        }
        None => (false, false),
    };
    Some((names, bold, italic))
}

/// 파일(또는 바이트)의 얼굴들
fn faces_in<R: Read + Seek>(f: &mut R) -> Vec<(u32, Vec<String>, bool, bool)> {
    let mut tag = [0u8; 12];
    if f.read_exact(&mut tag).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if &tag[..4] == b"ttcf" {
        let n = be32(&tag, 8).unwrap_or(0).min(64) as usize;
        let mut offs = vec![0u8; n * 4];
        if f.read_exact(&mut offs).is_err() {
            return out;
        }
        for i in 0..n {
            let o = be32(&offs, i * 4).unwrap_or(0) as u64;
            if let Some((names, b, it)) = read_face(f, o) {
                out.push((i as u32, names, b, it));
            }
        }
    } else if let Some((names, b, it)) = read_face(f, 0) {
        out.push((0, names, b, it));
    }
    out
}

impl FontDb {
    /// 빈 목록 (웹 어셈블리: 쓸 글꼴을 [`FontDb::add_data`] 로 넣는다)
    pub fn new() -> FontDb {
        FontDb::default()
    }

    /// 운영 체제 글꼴 폴더를 훑은 목록
    pub fn system() -> FontDb {
        let mut db = FontDb::new();
        for dir in system_dirs() {
            db.add_dir(&dir);
        }
        db
    }

    /// 폴더 안의 글꼴 파일 (하위 폴더 포함)
    pub fn add_dir(&mut self, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                self.add_dir(&p);
                continue;
            }
            let ext = p
                .extension()
                .and_then(|x| x.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(ext.as_deref(), Some("ttf" | "ttc" | "otf" | "otc")) {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&p) else {
                continue;
            };
            for (index, names, bold, italic) in faces_in(&mut f) {
                self.faces.push(FaceInfo {
                    src: Src::Path(p.clone()),
                    index,
                    names,
                    bold,
                    italic,
                });
            }
        }
    }

    /// 글꼴 파일 바이트를 넣는다 (TrueType·OpenType·모음)
    pub fn add_data(&mut self, data: Vec<u8>) {
        let data = Arc::new(data);
        let mut cur = std::io::Cursor::new(data.as_slice());
        for (index, names, bold, italic) in faces_in(&mut cur) {
            self.faces.push(FaceInfo {
                src: Src::Data(data.clone()),
                index,
                names,
                bold,
                italic,
            });
        }
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// 다른 목록의 글꼴을 뒤에 붙인다 (앞의 것을 먼저 찾는다)
    pub fn extend(&mut self, other: FontDb) {
        self.faces.extend(other.faces);
    }

    /// 가족 이름이 `name` 인 얼굴 중 굵게·기울임이 맞는 것
    fn exact(&self, name: &str, bold: bool, italic: bool) -> Option<FaceRef> {
        let mut want = vec![norm(name)];
        want.extend(aliases(name).iter().map(|a| norm(a)));
        let cands: Vec<usize> = (0..self.faces.len())
            .filter(|&i| self.faces[i].names.iter().any(|n| want.contains(n)))
            .collect();
        if cands.is_empty() {
            return None;
        }
        let score = |i: usize| {
            let f = &self.faces[i];
            (f.bold == bold) as u8 * 2 + (f.italic == italic) as u8
        };
        let best = *cands
            .iter()
            .max_by_key(|&&i| (score(i), std::cmp::Reverse(i)))?;
        let f = &self.faces[best];
        Some(FaceRef {
            face: best,
            synth_bold: bold && !f.bold,
            synth_italic: italic && !f.italic,
        })
    }

    /// 문서 글꼴 이름 → 넣을 얼굴. 없으면 대체 글꼴
    pub fn select(&self, name: &str, bold: bool, italic: bool) -> Option<FaceRef> {
        self.exact(name, bold, italic)
            .or_else(|| self.fallback(bold, italic))
    }

    /// 대체 글꼴 (없는 글자를 쓸 글꼴)
    pub fn fallback(&self, bold: bool, italic: bool) -> Option<FaceRef> {
        FALLBACK
            .iter()
            .find_map(|n| self.exact(n, bold, italic))
            .or_else(|| {
                (!self.faces.is_empty()).then_some(FaceRef {
                    face: 0,
                    synth_bold: bold,
                    synth_italic: italic,
                })
            })
    }

    /// 얼굴의 글꼴 파일 바이트와 모음 번호
    pub fn data(&self, face: usize) -> Option<(Arc<Vec<u8>>, u32)> {
        let f = self.faces.get(face)?;
        match &f.src {
            Src::Data(d) => Some((d.clone(), f.index)),
            Src::Path(p) => {
                let mut cache = self.loaded.lock().ok()?;
                if let Some(d) = cache.get(p) {
                    return Some((d.clone(), f.index));
                }
                let d = Arc::new(std::fs::read(p).ok()?);
                cache.insert(p.clone(), d.clone());
                Some((d, f.index))
            }
        }
    }
}

/// 운영 체제 글꼴 폴더들
fn system_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return out;
    }
    if cfg!(windows) {
        let win =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        out.push(win.join("Fonts"));
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/System/Library/Fonts"));
        out.push(PathBuf::from("/Library/Fonts"));
        if let Some(h) = std::env::var_os("HOME") {
            out.push(PathBuf::from(h).join("Library/Fonts"));
        }
    } else {
        out.push(PathBuf::from("/usr/share/fonts"));
        out.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(h) = std::env::var_os("HOME") {
            out.push(PathBuf::from(&h).join(".fonts"));
            out.push(PathBuf::from(&h).join(".local/share/fonts"));
        }
    }
    out
}

/// 글꼴 측정값 (단위: em 의 1/1000)
#[derive(Debug, Clone)]
pub struct Metrics {
    pub ascent: f64,
    pub descent: f64,
    pub cap_height: f64,
    pub bbox: [f64; 4],
    pub italic_angle: f64,
    pub upem: f64,
    pub ps_name: String,
    pub serif: bool,
    pub fixed: bool,
}

/// 읽은 글꼴 하나: 글자 → 글리프·폭
pub struct LoadedFace {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
    pub metrics: Metrics,
    /// 글자 → (글리프, 폭 1/1000 em)
    cmap: HashMap<char, Option<(u16, f64)>>,
}

impl LoadedFace {
    pub fn load(data: Arc<Vec<u8>>, index: u32) -> Option<LoadedFace> {
        let face = ttf_parser::Face::parse(&data, index).ok()?;
        let upem = face.units_per_em() as f64;
        let k = 1000.0 / upem;
        let os2 = face.tables().os2;
        let ascent = os2.map_or(face.ascender() as f64, |o| o.windows_ascender() as f64) * k;
        let descent =
            -(os2.map_or(-face.descender() as f64, |o| o.windows_descender() as f64)).abs() * k;
        let bb = face.global_bounding_box();
        let ps_name = face
            .names()
            .into_iter()
            .find(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
            .and_then(|n| n.to_string())
            .unwrap_or_else(|| "Font".into())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>();
        let metrics = Metrics {
            ascent,
            descent,
            cap_height: face.capital_height().map_or(ascent * 0.7, |v| v as f64 * k),
            bbox: [
                bb.x_min as f64 * k,
                bb.y_min as f64 * k,
                bb.x_max as f64 * k,
                bb.y_max as f64 * k,
            ],
            italic_angle: face.italic_angle() as f64,
            upem,
            ps_name: if ps_name.is_empty() {
                "Font".into()
            } else {
                ps_name
            },
            serif: false,
            fixed: face.is_monospaced(),
        };
        Some(LoadedFace {
            data,
            index,
            metrics,
            cmap: HashMap::new(),
        })
    }

    /// VDMX 표의 (오름, 내림) — 글꼴 높이 `ppem`(장치 px) 에서 GDI 가 쓰는 맞춘 값 (1:1 비율 묶음). 표나 그 크기가 없으면 None
    pub fn vdmx(&self, ppem: u32) -> Option<(f64, f64)> {
        let face = ttf_parser::Face::parse(&self.data, self.index).ok()?;
        let t = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"VDMX"))?;
        let u16at = |o: usize| t.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        let i16at = |o: usize| u16at(o).map(|v| v as i16);
        let n_ratios = u16at(4)? as usize;
        // 1:1 비율 (xRatio·yStartRatio·yEndRatio 가 1) 묶음, 없으면 모든 비율(0)을 받는 묶음
        let mut pick = None;
        for i in 0..n_ratios {
            let r = t.get(6 + i * 4..10 + i * 4)?;
            if r[1] == 1 && r[2] == 1 && r[3] == 1 {
                pick = Some(i);
                break;
            }
            if r[1] == 0 && pick.is_none() {
                pick = Some(i);
            }
        }
        let group = u16at(6 + n_ratios * 4 + pick? * 2)? as usize;
        // 묶음: 항목 수(u16), 가장 작은 크기(u8), 가장 큰 크기(u8), 항목 (크기 u16, 오름 i16, 내림 i16)
        let recs = u16at(group)? as usize;
        let (first, last) = (*t.get(group + 2)? as u32, *t.get(group + 3)? as u32);
        if ppem < first || ppem > last {
            return None;
        }
        (0..recs).find_map(|k| {
            let o = group + 4 + k * 6;
            (u16at(o)? as u32 == ppem)
                .then(|| Some((i16at(o + 2)? as f64, -(i16at(o + 4)? as f64))))?
        })
    }

    /// 글자의 (글리프, 폭 1/1000 em). 글꼴에 없으면 None
    pub fn glyph(&mut self, ch: char) -> Option<(u16, f64)> {
        if let Some(g) = self.cmap.get(&ch) {
            return *g;
        }
        let face = ttf_parser::Face::parse(&self.data, self.index).ok();
        let g = face.and_then(|face| {
            let id = face.glyph_index(ch)?;
            let adv = face.glyph_hor_advance(id).unwrap_or(0) as f64 * 1000.0
                / face.units_per_em() as f64;
            Some((id.0, adv))
        });
        self.cmap.insert(ch, g);
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VDMX 표: Windows 의 Arial 이 있으면 한셀 WMF 글자 실험에서 읽은 값과 맞는지 (없으면 건너뜀)
    #[test]
    fn vdmx_ascent_descent_of_arial() {
        let Ok(data) = std::fs::read("C:/Windows/Fonts/arial.ttf") else {
            return;
        };
        let f = LoadedFace::load(Arc::new(data), 0).unwrap();
        assert_eq!(f.vdmx(48), Some((45.0, 10.0)));
        assert_eq!(f.vdmx(91), Some((81.0, 20.0)));
        assert_eq!(f.vdmx(103), Some((92.0, 23.0)));
        assert_eq!(f.vdmx(7), None);
    }
}
