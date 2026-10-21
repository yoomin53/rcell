// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 두 레거시 `/Workbook` 스트림의 레코드 비교 — 레거시 쓰기를 한셀의 저장 결과와 대조할 때 쓴다.
//!
//! BOF 로 나눈 부분(전역, 시트…)마다 레코드 종류의 차례를 Myers 차분으로 맞추고, 맞춘 같은 종류끼리 내용을 비교한다.
//! 비교 전에 저장할 때마다 달라지는 칸을 지운다: BOUNDSHEET 의 스트림 위치, FRT 머리의 예약 8바이트(한셀은 초기화되지
//! 않은 값을 쓴다).

use super::biff;

/// 비교 결과
#[derive(Debug, Default)]
pub struct RecDiff {
    pub records_a: usize,
    pub records_b: usize,
    /// 종류와 내용이 같은 레코드 쌍
    pub same: usize,
    /// 종류는 맞췄으나 내용이 다른 쌍
    pub changed: usize,
    pub only_a: usize,
    pub only_b: usize,
    /// 종류별 (내용 다름, A 에만, B 에만)
    pub by_type: std::collections::BTreeMap<u16, (usize, usize, usize)>,
    pub lines: Vec<String>,
}

impl RecDiff {
    /// 한 줄 요약
    pub fn summary(&self) -> String {
        let pct = if self.records_b == 0 {
            100.0
        } else {
            self.same as f64 * 100.0 / self.records_a.max(self.records_b) as f64
        };
        let worst: Vec<String> = {
            let mut v: Vec<(&u16, &(usize, usize, usize))> = self.by_type.iter().collect();
            v.sort_by_key(|(_, (c, a, b))| std::cmp::Reverse(c + a + b));
            v.iter()
                .take(6)
                .map(|(t, (c, a, b))| format!("{t:04x}:{c}/{a}/{b}"))
                .collect()
        };
        format!(
            "레코드 A {} B {} · 같음 {} ({pct:.1}%) · 내용 다름 {} · A에만 {} · B에만 {} · 종류별(다름/A/B) {}",
            self.records_a,
            self.records_b,
            self.same,
            self.changed,
            self.only_a,
            self.only_b,
            worst.join(" ")
        )
    }
}

fn normalized(rtype: u16, data: &[u8]) -> Vec<u8> {
    let mut d = data.to_vec();
    match rtype {
        0x0085 if d.len() >= 4 => d[0..4].fill(0),
        // FRT 머리 (rt, grbitFrt, 예약 8): 예약을 지운다. grbitFrt 1 이면 Ref8U 라 그대로 둔다
        0x0850..=0x08FF if d.len() >= 12 && d[0..2] == rtype.to_le_bytes() && d[2] & 1 == 0 => {
            d[4..12].fill(0)
        }
        _ => {}
    }
    d
}

struct Rec {
    rtype: u16,
    data: Vec<u8>,
}

fn sections(stream: &[u8]) -> Vec<Vec<Rec>> {
    let mut out: Vec<Vec<Rec>> = Vec::new();
    for r in biff::records(stream) {
        if r.rtype == 0x0809 || out.is_empty() {
            out.push(Vec::new());
        }
        out.last_mut().unwrap().push(Rec {
            rtype: r.rtype,
            data: normalized(r.rtype, r.data),
        });
    }
    out
}

/// Myers 차분: 같은 열쇠의 쌍 목록 (차이가 `max_d` 를 넘으면 None)
fn myers(a: &[u64], b: &[u64], max_d: usize) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    let lim = max.min(max_d);
    let off = lim as isize + 1;
    let mut v = vec![0isize; 2 * lim + 3];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=lim as isize {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                // 거꾸로 따라가며 쌍을 모은다
                let mut pairs = Vec::new();
                let (mut x, mut y) = (n, m);
                for dd in (0..=d).rev() {
                    let vv = &trace[dd as usize];
                    let k = x - y;
                    let prev_k = if k == -dd
                        || (k != dd && vv[(k - 1 + off) as usize] < vv[(k + 1 + off) as usize])
                    {
                        k + 1
                    } else {
                        k - 1
                    };
                    let prev_x = if dd == 0 {
                        0
                    } else {
                        vv[(prev_k + off) as usize]
                    };
                    let prev_y = prev_x - prev_k;
                    while x > prev_x.max(0) && y > prev_y.max(0) && x > 0 && y > 0 {
                        if a[(x - 1) as usize] != b[(y - 1) as usize] {
                            break;
                        }
                        pairs.push(((x - 1) as usize, (y - 1) as usize));
                        x -= 1;
                        y -= 1;
                    }
                    if dd == 0 {
                        break;
                    }
                    x = prev_x;
                    y = prev_y;
                }
                pairs.reverse();
                return Some(pairs);
            }
            k += 2;
        }
    }
    None
}

fn hash(rtype: u16, data: Option<&[u8]>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    rtype.hash(&mut h);
    if let Some(d) = data {
        d.hash(&mut h);
    }
    h.finish()
}

fn hex(d: &[u8], at: usize, n: usize) -> String {
    d.iter()
        .skip(at)
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 비교 조건
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// SST 문자열 차례를 보지 않는다: SST 의 문자열을 내용 차례로 늘어놓고 LABELSST 의 번호를 그에 맞춘다.
    /// 한셀의 SST 차례는 문서를 읽어 온 내력(해시 표)에 따라 달라 파일에서 정할 수 없다 (5.7절)
    pub ignore_sst_order: bool,
    /// TXO 첫 u16 의 0x0200 을 보지 않는다: 한셀은 같은 문서를 저장할 때마다 이 비트를 다르게 쓴다
    /// (X.legacy.cell 과 hx 대조, 10.8절)
    pub ignore_txo_flag: bool,
}

/// TXO(0x01B6) 첫 u16 의 0x0200 을 지운다
fn mask_txo_flag(sections: &mut [Vec<Rec>]) {
    for r in sections.iter_mut().flatten() {
        if r.rtype == 0x01B6
            && let Some(b) = r.data.get_mut(1)
        {
            *b &= !0x02;
        }
    }
}

/// SST 의 문자열 하나하나 (머리 8바이트 뒤, 5.7절의 한컴 꼴) — 바이트 그대로
fn sst_strings(d: &[u8]) -> Option<Vec<&[u8]>> {
    let unique = u32::from_le_bytes(d.get(4..8)?.try_into().ok()?) as usize;
    let mut v = Vec::with_capacity(unique);
    let mut p = 8;
    for _ in 0..unique {
        let cch = u16::from_le_bytes(d.get(p..p + 2)?.try_into().ok()?) as usize;
        let flags = *d.get(p + 2)?;
        let mut q = p + 3 + cch * 2;
        if flags & 0x01 != 0 {
            let runs = u16::from_le_bytes(d.get(q..q + 2)?.try_into().ok()?) as usize;
            q += 2 + runs * 4;
        }
        v.push(d.get(p..q)?);
        p = q;
    }
    Some(v)
}

/// SST 를 내용 차례로 바꾸고 LABELSST 의 번호를 고친다
fn canonical_sst(secs: &mut [Vec<Rec>]) {
    let Some(sst) = secs
        .first_mut()
        .and_then(|s| s.iter_mut().find(|r| r.rtype == 0x00FC))
    else {
        return;
    };
    let Some(strings) = sst_strings(&sst.data) else {
        return;
    };
    let mut order: Vec<usize> = (0..strings.len()).collect();
    order.sort_by(|&x, &y| strings[x].cmp(strings[y]));
    let mut rank = vec![0u32; strings.len()];
    for (k, &i) in order.iter().enumerate() {
        rank[i] = k as u32;
    }
    let mut data = sst.data[..8].to_vec();
    for &i in &order {
        data.extend_from_slice(strings[i]);
    }
    sst.data = data;
    for s in secs.iter_mut() {
        for r in s
            .iter_mut()
            .filter(|r| r.rtype == 0x00FD && r.data.len() >= 12)
        {
            let i = u32::from_le_bytes(r.data[8..12].try_into().unwrap()) as usize;
            if let Some(&k) = rank.get(i) {
                r.data[8..12].copy_from_slice(&k.to_le_bytes());
            }
        }
    }
}

/// 두 스트림 비교
pub fn compare(a: &[u8], b: &[u8]) -> RecDiff {
    compare_with(a, b, Options::default())
}

/// 두 스트림 비교 (조건)
pub fn compare_with(a: &[u8], b: &[u8], opts: Options) -> RecDiff {
    let mut sa = sections(a);
    let mut sb = sections(b);
    if opts.ignore_sst_order {
        canonical_sst(&mut sa);
        canonical_sst(&mut sb);
    }
    if opts.ignore_txo_flag {
        mask_txo_flag(&mut sa);
        mask_txo_flag(&mut sb);
    }
    let mut r = RecDiff::default();
    for k in 0..sa.len().max(sb.len()) {
        let empty = Vec::new();
        let ra = sa.get(k).unwrap_or(&empty);
        let rb = sb.get(k).unwrap_or(&empty);
        r.records_a += ra.len();
        r.records_b += rb.len();
        // 1) 내용까지 같은 레코드를 맞춘다
        let ka: Vec<u64> = ra.iter().map(|x| hash(x.rtype, Some(&x.data))).collect();
        let kb: Vec<u64> = rb.iter().map(|x| hash(x.rtype, Some(&x.data))).collect();
        let exact = myers(&ka, &kb, 4000).unwrap_or_default();
        r.same += exact.len();
        // 2) 사이의 남은 레코드를 종류로 맞춘다
        let mut prev = (0usize, 0usize);
        let mut anchors = exact.clone();
        anchors.push((ra.len(), rb.len()));
        for (ea, eb) in anchors {
            let ga: Vec<usize> = (prev.0..ea).collect();
            let gb: Vec<usize> = (prev.1..eb).collect();
            let ta: Vec<u64> = ga.iter().map(|&i| hash(ra[i].rtype, None)).collect();
            let tb: Vec<u64> = gb.iter().map(|&i| hash(rb[i].rtype, None)).collect();
            let pairs = myers(&ta, &tb, 2000).unwrap_or_else(|| {
                (0..ga.len().min(gb.len()))
                    .filter(|&i| ta[i] == tb[i])
                    .map(|i| (i, i))
                    .collect()
            });
            let mut used_a = vec![false; ga.len()];
            let mut used_b = vec![false; gb.len()];
            for (pa, pb) in pairs {
                used_a[pa] = true;
                used_b[pb] = true;
                let (x, y) = (&ra[ga[pa]], &rb[gb[pb]]);
                // 차이가 너무 많아 1) 에서 못 맞춘 같은 레코드
                if x.data == y.data {
                    r.same += 1;
                    continue;
                }
                r.changed += 1;
                r.by_type.entry(x.rtype).or_default().0 += 1;
                let at = x
                    .data
                    .iter()
                    .zip(&y.data)
                    .position(|(p, q)| p != q)
                    .unwrap_or(x.data.len().min(y.data.len()));
                r.lines.push(format!(
                    "#{k} {:04x} {:<12} 다름 @{at} (길이 {}/{})\n    A: {}\n    B: {}",
                    x.rtype,
                    biff::record_name(x.rtype),
                    x.data.len(),
                    y.data.len(),
                    hex(&x.data, at.saturating_sub(4), 24),
                    hex(&y.data, at.saturating_sub(4), 24)
                ));
            }
            for (i, u) in used_a.iter().enumerate() {
                if !u {
                    let x = &ra[ga[i]];
                    r.only_a += 1;
                    r.by_type.entry(x.rtype).or_default().1 += 1;
                    r.lines.push(format!(
                        "#{k} {:04x} {:<12} A에만 (길이 {}) {}",
                        x.rtype,
                        biff::record_name(x.rtype),
                        x.data.len(),
                        hex(&x.data, 0, 24)
                    ));
                }
            }
            for (i, u) in used_b.iter().enumerate() {
                if !u {
                    let y = &rb[gb[i]];
                    r.only_b += 1;
                    r.by_type.entry(y.rtype).or_default().2 += 1;
                    r.lines.push(format!(
                        "#{k} {:04x} {:<12} B에만 (길이 {}) {}",
                        y.rtype,
                        biff::record_name(y.rtype),
                        y.data.len(),
                        hex(&y.data, 0, 24)
                    ));
                }
            }
            prev = (ea + 1, eb + 1);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(recs: &[(u16, &[u8])]) -> Vec<u8> {
        let mut v = Vec::new();
        for (t, d) in recs {
            v.extend_from_slice(&t.to_le_bytes());
            v.extend_from_slice(&(d.len() as u16).to_le_bytes());
            v.extend_from_slice(d);
        }
        v
    }

    #[test]
    fn finds_changed_missing_and_extra_records() {
        let a = stream(&[
            (0x0809, &[1]),
            (0x0031, &[1, 2]),
            (0x0022, &[0]),
            (0x000A, &[]),
        ]);
        let b = stream(&[
            (0x0809, &[1]),
            (0x0031, &[1, 3]),
            (0x0042, &[0xB0]),
            (0x000A, &[]),
        ]);
        let r = compare(&a, &b);
        assert_eq!((r.same, r.changed, r.only_a, r.only_b), (2, 1, 1, 1));
        assert_eq!(compare(&a, &a).same, 4);
    }

    #[test]
    fn sst_order_can_be_ignored() {
        // SST: cstTotal, cstUnique, ("가", "B") 또는 ("B", "가"); LABELSST 가 같은 문자열을 가리킨다
        let s = |t: &str| {
            let mut v = (t.encode_utf16().count() as u16).to_le_bytes().to_vec();
            v.push(0);
            for u in t.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        };
        let sst = |x: &str, y: &str| {
            let mut v = vec![2, 0, 0, 0, 2, 0, 0, 0];
            v.extend(s(x));
            v.extend(s(y));
            v
        };
        let label = |isst: u8| vec![0, 0, 0, 0, 0, 0, 15, 0, isst, 0, 0, 0];
        let (sa, sb) = (sst("가", "B"), sst("B", "가"));
        let (la, lb) = (label(0), label(1));
        let a = stream(&[(0x0809, &[1]), (0x00FC, &sa), (0x0809, &[1]), (0x00FD, &la)]);
        let b = stream(&[(0x0809, &[1]), (0x00FC, &sb), (0x0809, &[1]), (0x00FD, &lb)]);
        assert_eq!(compare(&a, &b).changed, 2);
        let opts = Options {
            ignore_sst_order: true,
            ..Default::default()
        };
        assert_eq!(compare_with(&a, &b, opts).changed, 0);
    }

    #[test]
    fn txo_flag_can_be_ignored() {
        let a = stream(&[(0x0809, &[1]), (0x01B6, &[0, 2, 1, 0])]);
        let b = stream(&[(0x0809, &[1]), (0x01B6, &[0, 0, 1, 0])]);
        assert_eq!(compare(&a, &b).changed, 1);
        let opts = Options {
            ignore_txo_flag: true,
            ..Default::default()
        };
        assert_eq!(compare_with(&a, &b, opts).changed, 0);
    }
}
