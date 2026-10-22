// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한컴 차트 레코드 0x7EF0 의 Vt 아카이브 쓰기 — [`super::vtchart`] 읽기의 반대.
//!
//! 객체 나무([`WObj`])를 배치 표([`super::vtchart::CLASSES`])의 차례대로 쓴다. 읽기와 같은 규칙
//! (docs/format/cell-legacy-chart.md 3절): 포인터는 객체 번호 u32 (없음 0xFFFFFFFF, 새 객체는 다음 번호 + 클래스
//! 참조 + 본문, 이미 쓴 객체는 그 번호), 클래스 참조는 번호 u32 (처음이면 이름·판이 붙는다), 기반 부분은 번호 없이
//! 클래스 참조 + 본문. 나무에 없는 필드는 0 (포인터는 없음, 묶음·배열은 그 꼴의 0)으로 쓴다 — 지은 쪽은 한셀이
//! 0 이 아닌 값을 쓰는 필드만 채운다. 같이 쓰는 객체(글꼴 이름 문자열 …)는 열쇠로 묶어 두 번째부터 앞 객체
//! 참조로 쓴다.

use std::collections::HashMap;

use super::vtchart::{BASE, ClassDef, Cond, Count, Item, Spec, StructDef, class_def};

/// 필드 값
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum W {
    U8(u8),
    U16(u16),
    // 지은 차트에는 없는 꼴 — 읽은 아카이브를 그대로 옮길 때(시험)만 쓴다
    #[cfg_attr(not(test), allow(dead_code))]
    I16(i16),
    U32(u32),
    // 지은 차트에는 없는 꼴 — 읽은 아카이브를 그대로 옮길 때(시험)만 쓴다
    #[cfg_attr(not(test), allow(dead_code))]
    I32(i32),
    F32(f32),
    F64(f64),
    /// VtColor: 0xRRGGBB, 플래그 (종류는 늘 2)
    Color(u32, u8),
    /// 없음 포인터
    Null,
    /// 새 객체
    New(Box<WObj>),
    /// 같이 쓰는 객체: 처음 나오면 새 객체, 같은 열쇠가 다시 나오면 앞 객체 참조
    Shared(String, Box<WObj>),
    /// 앞 객체 참조 (객체 번호) — 읽은 아카이브를 그대로 옮길 때
    #[cfg_attr(not(test), allow(dead_code))]
    Back(u32),
    /// 기반 클래스 부분
    Emb(Box<WObj>),
    /// 묶음 (Pen·Brush …)
    Group(Vec<(&'static str, W)>),
    Arr(Vec<W>),
    /// VtString 내용 (cp949 + UTF-16 으로 부호화한다)
    Str(String),
    /// VtString 내용 바이트 그대로 (cb 뒤) — 읽은 아카이브를 그대로 옮길 때
    #[cfg_attr(not(test), allow(dead_code))]
    RawStr(Vec<u8>),
    /// 길이 u32 + 바이트
    Bytes(Vec<u8>),
}

/// 객체 하나 (기반 부분은 [`BASE`] 필드의 [`W::Emb`])
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WObj {
    pub class: &'static str,
    pub fields: Vec<(&'static str, W)>,
}

impl WObj {
    pub fn new(class: &'static str) -> WObj {
        WObj {
            class,
            fields: Vec::new(),
        }
    }
    /// 필드를 넣거나 바꾼다
    pub fn set(mut self, name: &'static str, v: W) -> WObj {
        self.put(name, v);
        self
    }
    pub fn put(&mut self, name: &'static str, v: W) {
        match self.fields.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = v,
            None => self.fields.push((name, v)),
        }
    }
    pub fn u16(self, name: &'static str, v: u16) -> WObj {
        self.set(name, W::U16(v))
    }
    pub fn u32(self, name: &'static str, v: u32) -> WObj {
        self.set(name, W::U32(v))
    }
    pub fn f32(self, name: &'static str, v: f32) -> WObj {
        self.set(name, W::F32(v))
    }
    pub fn ptr(self, name: &'static str, o: WObj) -> WObj {
        self.set(name, W::New(Box::new(o)))
    }
    /// 기반 부분
    pub fn base(self, o: WObj) -> WObj {
        self.set(BASE, W::Emb(Box::new(o)))
    }
    /// 이름으로 필드 (자기 것만)
    pub fn get_mut(&mut self, name: &str) -> Option<&mut W> {
        self.fields
            .iter_mut()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v)
    }
}

/// 묶음을 짓는 도우미
#[derive(Debug, Clone, Default)]
pub(crate) struct WGroup(pub Vec<(&'static str, W)>);

impl WGroup {
    pub fn new() -> WGroup {
        WGroup::default()
    }
    pub fn set(mut self, name: &'static str, v: W) -> WGroup {
        match self.0.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = v,
            None => self.0.push((name, v)),
        }
        self
    }
    pub fn u16(self, name: &'static str, v: u16) -> WGroup {
        self.set(name, W::U16(v))
    }
    pub fn f32(self, name: &'static str, v: f32) -> WGroup {
        self.set(name, W::F32(v))
    }
    pub fn color(self, name: &'static str, rgb: u32) -> WGroup {
        self.set(name, W::Color(rgb, 0))
    }
    pub fn group(self, name: &'static str, g: WGroup) -> WGroup {
        self.set(name, W::Group(g.0))
    }
    pub fn into_w(self) -> W {
        W::Group(self.0)
    }
}

/// VtString 내용: cp949 글자 + NUL, 0, UTF-16LE 글자 + NUL16 (3.4절). cp949 에 없는 글자는 `?`
pub(crate) fn vt_string_bytes(s: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(s.len() * 3 + 4);
    for c in s.chars() {
        match crate::calc::cp949::encode(c) {
            Some(k) => b.extend_from_slice(&k.to_be_bytes()),
            None if (c as u32) < 0x80 => b.push(c as u8),
            None => b.push(b'?'),
        }
    }
    b.extend_from_slice(&[0, 0]);
    for u in s.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0]);
    b
}

/// 아카이브 쓰기
struct Ser {
    out: Vec<u8>,
    classes: Vec<&'static str>,
    next_id: u32,
    shared: HashMap<String, u32>,
}

impl Ser {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    /// 클래스 참조: 정의된 번호, 처음이면 이름(u16 길이 + ASCII + NUL)과 판
    fn class_ref(&mut self, def: &'static ClassDef) {
        if let Some(i) = self.classes.iter().position(|c| *c == def.name) {
            self.u32(i as u32);
            return;
        }
        self.u32(self.classes.len() as u32);
        self.u16(def.name.len() as u16 + 1);
        self.out.extend_from_slice(def.name.as_bytes());
        self.u8(0);
        self.u16(def.version);
        self.classes.push(def.name);
    }

    /// 클래스 참조 + 본문
    fn object(&mut self, o: &WObj) {
        let def = class_def(o.class).unwrap_or_else(|| panic!("배치를 모르는 클래스 {}", o.class));
        self.class_ref(def);
        self.specs(def.specs, &o.fields);
    }

    /// 기본값만 있는 객체 (나무에 없는 기반 부분)
    fn default_object(&mut self, class: &'static str) {
        self.object(&WObj::new(class));
    }

    fn new_object(&mut self, o: &WObj) {
        let id = self.next_id;
        self.next_id += 1;
        self.u32(id);
        self.object(o);
    }

    fn specs(&mut self, specs: &'static [Spec], fields: &[(&'static str, W)]) {
        for s in specs {
            self.spec(s, fields);
        }
    }

    fn value_of<'f>(fields: &'f [(&'static str, W)], name: &str) -> Option<&'f W> {
        fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn count_of(fields: &[(&'static str, W)], name: &str) -> u64 {
        match Self::value_of(fields, name) {
            Some(W::U8(v)) => u64::from(*v),
            Some(W::U16(v)) => u64::from(*v),
            Some(W::U32(v)) => u64::from(*v),
            _ => 0,
        }
    }

    fn spec(&mut self, s: &'static Spec, fields: &[(&'static str, W)]) {
        use Spec as S;
        match s {
            S::U8(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::U8(v)) => *v,
                    _ => 0,
                };
                self.u8(v);
            }
            S::U16(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::U16(v)) => *v,
                    _ => 0,
                };
                self.u16(v);
            }
            S::I16(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::I16(v)) => *v,
                    _ => 0,
                };
                self.out.extend_from_slice(&v.to_le_bytes());
            }
            S::U32(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::U32(v)) => *v,
                    _ => 0,
                };
                self.u32(v);
            }
            S::I32(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::I32(v)) => *v,
                    _ => 0,
                };
                self.out.extend_from_slice(&v.to_le_bytes());
            }
            S::F32(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::F32(v)) => *v,
                    _ => 0.0,
                };
                self.out.extend_from_slice(&v.to_le_bytes());
            }
            S::F64(n) => {
                let v = match Self::value_of(fields, n) {
                    Some(W::F64(v)) => *v,
                    _ => 0.0,
                };
                self.out.extend_from_slice(&v.to_le_bytes());
            }
            S::Color(n) => {
                let (rgb, flags) = match Self::value_of(fields, n) {
                    Some(W::Color(rgb, f)) => (*rgb, *f),
                    _ => (0, 0),
                };
                self.u16(2);
                self.u8((rgb >> 16) as u8);
                self.u8((rgb >> 8) as u8);
                self.u8(rgb as u8);
                self.u8(flags);
            }
            S::Ptr(n, _) => {
                let v = Self::value_of(fields, n).cloned().unwrap_or(W::Null);
                self.ptr(&v);
            }
            S::Emb(n, class) => match Self::value_of(fields, n) {
                Some(W::Emb(o)) => self.object(o),
                _ => self.default_object(class),
            },
            S::Group(n, def) => {
                let empty = Vec::new();
                let g = match Self::value_of(fields, n) {
                    Some(W::Group(g)) => g,
                    _ => &empty,
                };
                self.group(def, g);
            }
            S::Arr(n, count, item) => {
                let n_items = match count {
                    Count::Fixed(k) => *k as u64,
                    Count::Field(f) => Self::count_of(fields, f),
                    Count::Product(a, b) => Self::count_of(fields, a) * Self::count_of(fields, b),
                };
                let given: &[W] = match Self::value_of(fields, n) {
                    Some(W::Arr(items)) => items,
                    _ => &[],
                };
                for k in 0..n_items as usize {
                    match (item, given.get(k)) {
                        (Item::U16, Some(W::U16(v))) => self.u16(*v),
                        (Item::U16, _) => self.u16(0),
                        (Item::Ptr(_), Some(v)) => {
                            let v = v.clone();
                            self.ptr(&v);
                        }
                        (Item::Ptr(_), None) => self.u32(0xFFFF_FFFF),
                    }
                }
            }
            S::When(cond, specs) => {
                let Cond::Eq(name, v) = cond;
                if Self::count_of(fields, name) == *v {
                    self.specs(specs, fields);
                }
            }
            S::VStr(n) => {
                let raw = match Self::value_of(fields, n) {
                    Some(W::Str(s)) => vt_string_bytes(s),
                    Some(W::RawStr(b)) => b.clone(),
                    _ => vt_string_bytes(""),
                };
                self.u32(raw.len() as u32);
                self.out.extend_from_slice(&raw);
            }
            S::LBytes(n) => {
                let b = match Self::value_of(fields, n) {
                    Some(W::Bytes(b)) => b.clone(),
                    _ => Vec::new(),
                };
                self.u32(b.len() as u32);
                self.out.extend_from_slice(&b);
            }
        }
    }

    fn group(&mut self, def: &'static StructDef, fields: &[(&'static str, W)]) {
        self.specs(def.specs, fields);
    }

    fn ptr(&mut self, v: &W) {
        match v {
            W::New(o) => self.new_object(o),
            W::Shared(key, o) => match self.shared.get(key) {
                Some(&id) => self.u32(id),
                None => {
                    self.shared.insert(key.clone(), self.next_id);
                    self.new_object(o);
                }
            },
            W::Back(id) => self.u32(*id),
            _ => self.u32(0xFFFF_FFFF),
        }
    }
}

/// 뿌리 객체(VtChart)의 아카이브 바이트 (길이 필드 없이)
pub(crate) fn write_archive(root: &WObj) -> Vec<u8> {
    let mut s = Ser {
        out: Vec::new(),
        classes: Vec::new(),
        next_id: 0,
        shared: HashMap::new(),
    };
    s.new_object(root);
    s.out
}

/// 읽은 아카이브를 나무로 (쓰기 대조 시험용: 다시 쓰면 같은 바이트여야 한다)
#[cfg(test)]
pub(crate) fn from_record(rec: &super::vtchart::VtRecord, payload: &[u8]) -> WObj {
    rec.archive.to_wobj(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn vt_string_matches_hancell_layout() {
        // 한글은 cp949 두 바이트씩, 그다음 00 00, UTF-16LE, 00 00
        let b = vt_string_bytes("돋움");
        assert_eq!(b.len(), 4 + 2 + 4 + 2);
        assert_eq!(&b[4..6], &[0, 0]);
        assert_eq!(&b[6..10], &[0xCB, 0xB3, 0xC0, 0xC6]);
        assert_eq!(vt_string_bytes(""), vec![0, 0, 0, 0]);
        assert_eq!(
            vt_string_bytes("A1"),
            vec![b'A', b'1', 0, 0, b'A', 0, b'1', 0, 0, 0]
        );
    }

    fn legacy_files(dir: &Path) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    !n.starts_with("~$") && n.to_ascii_lowercase().ends_with(".legacy.cell")
                })
            })
            .collect();
        v.sort();
        v
    }

    /// 말뭉치의 모든 0x7EF0: 읽은 나무를 다시 쓰면 아카이브가 바이트까지 같다 (RCELL_CORPUS)
    #[test]
    fn corpus_archives_round_trip_through_writer() {
        let Some(root) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
            eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
            return;
        };
        let mut checked = 0;
        let mut failures = Vec::new();
        for sub in ["", "craft", "templates", "craft-charts", "craft-charts2"] {
            for path in legacy_files(&root.join(sub)) {
                let bytes = std::fs::read(&path).unwrap();
                let stream = crate::container::read_entry(&bytes, "/Workbook").unwrap();
                for rec in crate::legacy::biff::records(&stream) {
                    if rec.rtype != super::super::vtchart::RECORD_TYPE {
                        continue;
                    }
                    let parsed = super::super::vtchart::parse(rec.data, &[]).unwrap();
                    let tree = from_record(&parsed, rec.data);
                    let out = write_archive(&tree);
                    let blob = &rec.data[parsed.envelope.blob.clone()];
                    checked += 1;
                    if out != blob {
                        let at = out.iter().zip(blob).position(|(a, b)| a != b);
                        failures.push(format!(
                            "{}: 길이 {} / {}, 처음 다른 자리 {:?}",
                            path.display(),
                            out.len(),
                            blob.len(),
                            at
                        ));
                    }
                }
            }
        }
        eprintln!("다시 쓴 아카이브: {checked}");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
