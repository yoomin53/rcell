// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 암호화된 OOXML 패키지 풀기·감싸기 ([MS-OFFCRYPTO] ECMA-376 표준 암호화와 Agile 암호화).
//!
//! 한셀은 통합 문서 구조 잠금(`lockStructure`)이 걸린 문서를 바이너리 `.cell` 로 저장하거나 레거시 문서를 다시
//! XML 로 저장할 때 OLE2 컨테이너 안에 `EncryptionInfo` + `EncryptedPackage` 로 암호화한다 (관측 — 시트 보호나
//! 창 잠금만 걸린 문서는 감싸지 않는다: 코퍼스 filter2·filter3). 감싸기는 [`encrypt_package`],
//! 컨테이너는 [`crate::binary::encrypt`].
//! 암호를 지정하지 않으면 Office 의 기본 암호 `VelvetSweatshop` 이 쓰인다. 사용자가 문서 암호를 걸어 저장해도
//! 한셀은 같은 표준 암호화를 쓴다 (관측: 한셀 2024, `EncryptionInfo` 224 바이트). Agile 암호화는 Excel 이 만든
//! 암호 문서(.xlsx)를 위한 것이다.
//!
//! 표준 암호화(버전 x.2): AES-128 ECB, SHA-1, 키 유도 50,000회.
//! Agile 암호화(버전 4.4, Excel 2010 이후의 암호 문서): XML 로 적은 매개 변수(AES-128/192/256 CBC,
//! SHA-1/256/384/512, spinCount), 4096 바이트 조각마다 IV 를 새로 만든다. 외부 라이브러리 없이 구현한다.

use crate::error::{Error, Result};

/// 암호 없이 보호만 걸린 문서에 Office·한셀이 쓰는 기본 암호
pub const DEFAULT_PASSWORD: &str = "VelvetSweatshop";

// ---------------------------------------------------------------------------
// SHA-1 (FIPS 180-4)
// ---------------------------------------------------------------------------

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// SHA-256 / SHA-384 / SHA-512 (FIPS 180-4)
// ---------------------------------------------------------------------------

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

const K512: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

fn sha512_core(data: &[u8], mut h: [u64; 8]) -> [u64; 8] {
    let mut msg = data.to_vec();
    let bits = (data.len() as u128).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 128 != 112 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(128) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            w[i] = u64::from_be_bytes(block[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    let h = sha512_core(
        data,
        [
            0x6a09e667f3bcc908,
            0xbb67ae8584caa73b,
            0x3c6ef372fe94f82b,
            0xa54ff53a5f1d36f1,
            0x510e527fade682d1,
            0x9b05688c2b3e6c1f,
            0x1f83d9abfb41bd6b,
            0x5be0cd19137e2179,
        ],
    );
    let mut out = [0u8; 64];
    for (i, v) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn sha384(data: &[u8]) -> [u8; 48] {
    let h = sha512_core(
        data,
        [
            0xcbbb9d5dc1059ed8,
            0x629a292a367cd507,
            0x9159015a3070dd17,
            0x152fecd8f70e5939,
            0x67332667ffc00b31,
            0x8eb44a8768581511,
            0xdb0c2e0d64f98fa7,
            0x47b5481dbefa4fa4,
        ],
    );
    let mut out = [0u8; 48];
    for (i, v) in h.iter().take(6).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Agile 암호화의 해시 알고리즘
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_name(n: &str) -> Option<Hash> {
        match n.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Some(Hash::Sha1),
            "SHA256" => Some(Hash::Sha256),
            "SHA384" => Some(Hash::Sha384),
            "SHA512" => Some(Hash::Sha512),
            _ => None,
        }
    }
    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Hash::Sha1 => sha1(data).to_vec(),
            Hash::Sha256 => sha256(data).to_vec(),
            Hash::Sha384 => sha384(data).to_vec(),
            Hash::Sha512 => sha512(data).to_vec(),
        }
    }
}

// ---------------------------------------------------------------------------
// AES 복호 (FIPS 197, 키 128/192/256 비트)
// ---------------------------------------------------------------------------

const SBOX: [u8; 256] = {
    // GF(2^8) 역원 + 아핀 변환으로 컴파일 때 만든다
    let mut s = [0u8; 256];
    let mut p: u8 = 1;
    let mut q: u8 = 1;
    loop {
        // p ← p · 3, q ← q / 3
        p = p ^ (p << 1) ^ if p & 0x80 != 0 { 0x1B } else { 0 };
        q ^= q << 1;
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
        s[p as usize] = x ^ 0x63;
        if p == 1 {
            break;
        }
    }
    s[0] = 0x63;
    s
};

const INV_SBOX: [u8; 256] = {
    let mut inv = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        inv[SBOX[i] as usize] = i as u8;
        i += 1;
    }
    inv
};

fn xtime(x: u8) -> u8 {
    (x << 1) ^ if x & 0x80 != 0 { 0x1B } else { 0 }
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    r
}

/// AES 라운드 키 (키 16·24·32 바이트 → 라운드 10·12·14, 라운드 키 Nr + 1 개). 다른 길이면 None
fn expand_key_any(key: &[u8]) -> Option<Vec<[u8; 16]>> {
    let nk = match key.len() {
        16 => 4,
        24 => 6,
        32 => 8,
        _ => return None,
    };
    let nr = nk + 6;
    let total = 4 * (nr + 1);
    let mut w = vec![[0u8; 4]; total];
    for i in 0..nk {
        w[i].copy_from_slice(&key[i * 4..i * 4 + 4]);
    }
    let mut rcon: u8 = 1;
    for i in nk..total {
        let mut t = w[i - 1];
        if i % nk == 0 {
            t = [
                SBOX[t[1] as usize],
                SBOX[t[2] as usize],
                SBOX[t[3] as usize],
                SBOX[t[0] as usize],
            ];
            t[0] ^= rcon;
            rcon = xtime(rcon);
        } else if nk > 6 && i % nk == 4 {
            t = t.map(|b| SBOX[b as usize]);
        }
        for j in 0..4 {
            w[i][j] = w[i - nk][j] ^ t[j];
        }
    }
    Some(
        (0..=nr)
            .map(|r| {
                let mut k = [0u8; 16];
                for c in 0..4 {
                    k[c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
                }
                k
            })
            .collect(),
    )
}

/// AES-128 라운드 키 (11 × 16 바이트)
fn expand_key(key: &[u8; 16]) -> [[u8; 16]; 11] {
    let v = expand_key_any(key).unwrap_or_default();
    let mut rk = [[0u8; 16]; 11];
    for (d, s) in rk.iter_mut().zip(v) {
        *d = s;
    }
    rk
}

/// 블록 하나 복호 (상태는 열 우선 16바이트). 라운드 수 = 라운드 키 수 - 1
fn decrypt_block(rk: &[[u8; 16]], block: &mut [u8]) {
    let add = |s: &mut [u8], k: &[u8; 16]| {
        for (x, y) in s.iter_mut().zip(k) {
            *x ^= y;
        }
    };
    let nr = rk.len() - 1;
    add(block, &rk[nr]);
    for round in (0..nr).rev() {
        // InvShiftRows
        let s = block.to_vec();
        for c in 0..4 {
            for r in 0..4 {
                block[c * 4 + r] = s[((c + 4 - r) % 4) * 4 + r];
            }
        }
        // InvSubBytes
        for x in block.iter_mut() {
            *x = INV_SBOX[*x as usize];
        }
        add(block, &rk[round]);
        if round > 0 {
            // InvMixColumns
            for c in 0..4 {
                let a = [
                    block[c * 4],
                    block[c * 4 + 1],
                    block[c * 4 + 2],
                    block[c * 4 + 3],
                ];
                block[c * 4] = gmul(a[0], 14) ^ gmul(a[1], 11) ^ gmul(a[2], 13) ^ gmul(a[3], 9);
                block[c * 4 + 1] = gmul(a[0], 9) ^ gmul(a[1], 14) ^ gmul(a[2], 11) ^ gmul(a[3], 13);
                block[c * 4 + 2] = gmul(a[0], 13) ^ gmul(a[1], 9) ^ gmul(a[2], 14) ^ gmul(a[3], 11);
                block[c * 4 + 3] = gmul(a[0], 11) ^ gmul(a[1], 13) ^ gmul(a[2], 9) ^ gmul(a[3], 14);
            }
        }
    }
}

/// AES CBC 복호 (키 16·24·32 바이트, IV 16 바이트). 길이는 16의 배수로 자른다
pub fn aes_cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    let rk = expand_key_any(key)?;
    let mut prev: [u8; 16] = iv.get(..16)?.try_into().ok()?;
    let mut out = data[..data.len() / 16 * 16].to_vec();
    for b in out.chunks_mut(16) {
        let ct: [u8; 16] = b.try_into().ok()?;
        decrypt_block(&rk, b);
        for (x, y) in b.iter_mut().zip(prev) {
            *x ^= y;
        }
        prev = ct;
    }
    Some(out)
}

/// AES-128 ECB 복호 (길이는 16의 배수로 자른다)
pub fn aes128_ecb_decrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let rk = expand_key(key);
    let mut out = data[..data.len() / 16 * 16].to_vec();
    for b in out.chunks_mut(16) {
        decrypt_block(&rk, b);
    }
    out
}

/// 블록 하나 암호 (FIPS 197 5.1, 상태는 열 우선 16바이트)
fn encrypt_block(rk: &[[u8; 16]], block: &mut [u8]) {
    let add = |s: &mut [u8], k: &[u8; 16]| {
        for (x, y) in s.iter_mut().zip(k) {
            *x ^= y;
        }
    };
    let nr = rk.len() - 1;
    add(block, &rk[0]);
    for (round, key) in rk.iter().enumerate().skip(1) {
        // SubBytes
        for x in block.iter_mut() {
            *x = SBOX[*x as usize];
        }
        // ShiftRows: 행 r 을 왼쪽으로 r 칸
        let s = block.to_vec();
        for c in 0..4 {
            for r in 0..4 {
                block[c * 4 + r] = s[((c + r) % 4) * 4 + r];
            }
        }
        if round < nr {
            // MixColumns
            for c in 0..4 {
                let a = [
                    block[c * 4],
                    block[c * 4 + 1],
                    block[c * 4 + 2],
                    block[c * 4 + 3],
                ];
                block[c * 4] = gmul(a[0], 2) ^ gmul(a[1], 3) ^ a[2] ^ a[3];
                block[c * 4 + 1] = a[0] ^ gmul(a[1], 2) ^ gmul(a[2], 3) ^ a[3];
                block[c * 4 + 2] = a[0] ^ a[1] ^ gmul(a[2], 2) ^ gmul(a[3], 3);
                block[c * 4 + 3] = gmul(a[0], 3) ^ a[1] ^ a[2] ^ gmul(a[3], 2);
            }
        }
        add(block, key);
    }
}

/// AES-128 ECB 암호. 길이가 16의 배수가 아니면 0 으로 채운다
pub fn aes128_ecb_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let rk = expand_key(key);
    let mut out = data.to_vec();
    out.resize(data.len().div_ceil(16) * 16, 0);
    for b in out.chunks_mut(16) {
        encrypt_block(&rk, b);
    }
    out
}

// ---------------------------------------------------------------------------
// ECMA-376 표준 암호화
// ---------------------------------------------------------------------------

/// [MS-OFFCRYPTO] 2.3.4.7 키 유도 (SHA-1, 50,000회, keySize 비트)
fn derive_key(password: &str, salt: &[u8], key_bits: u32) -> Vec<u8> {
    let pw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut h = sha1(&[salt, &pw].concat());
    for i in 0u32..50_000 {
        h = sha1(&[&i.to_le_bytes()[..], &h].concat());
    }
    let hfinal = sha1(&[&h[..], &0u32.to_le_bytes()].concat());
    let derive = |pad: u8| {
        let mut buf = [pad; 64];
        for (b, x) in buf.iter_mut().zip(hfinal) {
            *b ^= x;
        }
        sha1(&buf)
    };
    let x = [derive(0x36), derive(0x5C)].concat();
    x[..(key_bits / 8) as usize].to_vec()
}

/// `EncryptionInfo` + `EncryptedPackage` → 원래 ZIP 패키지.
/// 표준 암호화(AES-128)만 다룬다. 암호가 틀리면 오류.
pub fn decrypt_package(info: &[u8], package: &[u8], password: &str) -> Result<Vec<u8>> {
    let bad = |m: &str| Error::Decrypt(m.to_string());
    let u16_at = |i: usize| info.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| {
        info.get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (major, minor) = (
        u16_at(0).ok_or_else(|| bad("짧음"))?,
        u16_at(2).unwrap_or(0),
    );
    if (major, minor) == (4, 4) {
        return decrypt_agile(info, package, password);
    }
    if minor != 2 || !(2..=4).contains(&major) {
        return Err(bad(&format!(
            "버전 {major}.{minor} — 표준 암호화(x.2)와 Agile 암호화(4.4)만 지원합니다"
        )));
    }
    // 머리: Flags u32, HeaderSize u32, EncryptionHeader(HeaderSize), EncryptionVerifier
    let header_size = u32_at(8).ok_or_else(|| bad("머리"))? as usize;
    let h = 12;
    let alg = u32_at(h + 8).unwrap_or(0);
    let key_bits = u32_at(h + 16).unwrap_or(128);
    if alg != 0x660E || key_bits != 128 {
        return Err(bad(&format!(
            "알고리즘 {alg:#x}/{key_bits} — AES-128 만 지원합니다"
        )));
    }
    let v = h + header_size;
    let salt_size = u32_at(v).ok_or_else(|| bad("검증자"))? as usize;
    let salt = info
        .get(v + 4..v + 4 + salt_size)
        .ok_or_else(|| bad("salt"))?;
    let ev = info
        .get(v + 4 + salt_size..v + 20 + salt_size)
        .ok_or_else(|| bad("verifier"))?;
    let evh = info
        .get(v + 24 + salt_size..v + 56 + salt_size)
        .ok_or_else(|| bad("verifier hash"))?;
    let key: [u8; 16] = derive_key(password, salt, key_bits)
        .try_into()
        .map_err(|_| bad("키"))?;
    let verifier = aes128_ecb_decrypt(&key, ev);
    let vh = aes128_ecb_decrypt(&key, evh);
    if sha1(&verifier)[..] != vh[..20] {
        return Err(Error::WrongPassword);
    }
    let size = package
        .get(..8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
        .ok_or_else(|| bad("패키지"))? as usize;
    let mut plain = aes128_ecb_decrypt(&key, &package[8..]);
    plain.truncate(size);
    Ok(plain)
}

/// 표준 암호화의 CSP 이름 (한셀 2024 관측)
const AES_CSP: &str = "Microsoft Enhanced RSA and AES Cryptographic Provider";

/// ZIP 패키지를 표준 암호화(AES-128 ECB, SHA-1)로 감싼다 → (`EncryptionInfo`, `EncryptedPackage`).
/// `EncryptionInfo` 는 한셀 2024 가 쓰는 꼴 그대로다: 버전 4.2, 플래그 0x24 (fCryptoAPI | fAES), 머리 140 바이트
/// (AES-128 0x660E, SHA-1 0x8004, 키 128비트, 공급자 0x18, CSP 이름), 검증자 해시 뒤는 0 — 모두 224 바이트.
/// `EncryptedPackage` 는 원래 길이(u64) + 16바이트 단위로 0 을 채운 암호문이다.
pub fn encrypt_package(
    plain: &[u8],
    password: &str,
    salt: &[u8; 16],
    verifier: &[u8; 16],
) -> (Vec<u8>, Vec<u8>) {
    let key: [u8; 16] = derive_key(password, salt, 128)
        .try_into()
        .unwrap_or([0; 16]);
    let csp: Vec<u8> = AES_CSP
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut info = Vec::with_capacity(224);
    info.extend_from_slice(&[0x04, 0x00, 0x02, 0x00]);
    info.extend_from_slice(&0x24u32.to_le_bytes());
    info.extend_from_slice(&(32 + csp.len() as u32).to_le_bytes());
    for v in [0x24u32, 0, 0x660E, 0x8004, 128, 0x18, 0, 0] {
        info.extend_from_slice(&v.to_le_bytes());
    }
    info.extend_from_slice(&csp);
    info.extend_from_slice(&16u32.to_le_bytes());
    info.extend_from_slice(salt);
    info.extend_from_slice(&aes128_ecb_encrypt(&key, verifier));
    info.extend_from_slice(&20u32.to_le_bytes());
    info.extend_from_slice(&aes128_ecb_encrypt(&key, &sha1(verifier)));
    let mut package = (plain.len() as u64).to_le_bytes().to_vec();
    package.extend_from_slice(&aes128_ecb_encrypt(&key, plain));
    (info, package)
}

// ---------------------------------------------------------------------------
// Agile 암호화 ([MS-OFFCRYPTO] 2.3.4.10–2.3.4.15)
// ---------------------------------------------------------------------------

/// Base64 (RFC 4648, 공백 무시)
fn base64(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let chars: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(chars.len() * 3 / 4);
    for chunk in chunks_of(&chars, 4) {
        let mut acc = 0u32;
        for &c in chunk {
            acc = (acc << 6) | val(c)?;
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]),
            3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
            2 => out.push((acc >> 4) as u8),
            _ => return None,
        }
    }
    Some(out)
}

fn chunks_of(v: &[u8], n: usize) -> impl Iterator<Item = &[u8]> {
    v.chunks(n)
}

/// 키 하나의 매개 변수 (keyData 또는 p:encryptedKey)
struct AgileParams {
    hash: Hash,
    key_bytes: usize,
    block_size: usize,
    salt: Vec<u8>,
}

fn agile_params(e: &crate::xml::Element) -> Result<AgileParams> {
    let bad = |m: &str| Error::Decrypt(format!("Agile {m}"));
    let alg = e.attr("cipherAlgorithm").unwrap_or("AES");
    let chaining = e.attr("cipherChaining").unwrap_or("ChainingModeCBC");
    if alg != "AES" || chaining != "ChainingModeCBC" {
        return Err(bad(&format!("{alg}/{chaining} — AES CBC 만 지원합니다")));
    }
    let hash = e
        .attr("hashAlgorithm")
        .and_then(Hash::from_name)
        .ok_or_else(|| bad("해시 알고리즘"))?;
    let key_bits = e.attr_u32("keyBits").ok_or_else(|| bad("keyBits"))? as usize;
    Ok(AgileParams {
        hash,
        key_bytes: key_bits / 8,
        block_size: e.attr_u32("blockSize").unwrap_or(16) as usize,
        salt: e
            .attr("saltValue")
            .and_then(base64)
            .ok_or_else(|| bad("saltValue"))?,
    })
}

/// 길이를 맞춘다: 길면 자르고, 짧으면 0x36 으로 채운다 (2.3.4.11)
fn fit(mut v: Vec<u8>, n: usize) -> Vec<u8> {
    v.resize(n, 0x36);
    v
}

/// Agile: EncryptionInfo 4.4 = 버전 u16 ×2, 예약 u32 (0x40), XML. 암호 키 → 비밀 키 → 4096 바이트 조각마다 복호
fn decrypt_agile(info: &[u8], package: &[u8], password: &str) -> Result<Vec<u8>> {
    let bad = |m: &str| Error::Decrypt(format!("Agile {m}"));
    let xml = std::str::from_utf8(info.get(8..).ok_or_else(|| bad("짧음"))?)
        .map_err(|_| bad("XML 인코딩"))?;
    let root = crate::xml::parse_raw(xml.trim_start_matches('﻿'), "EncryptionInfo")?;
    let mut key_data = None;
    let mut enc_key = None;
    root.walk(&mut |e| match e.name.as_str() {
        "keyData" => key_data = Some(e),
        "encryptedKey" if e.attr("spinCount").is_some() => enc_key = Some(e),
        _ => {}
    });
    let key_data = agile_params(key_data.ok_or_else(|| bad("keyData 없음"))?)?;
    let ek = enc_key.ok_or_else(|| bad("암호 키 암호기 없음 (인증서 암호는 지원하지 않습니다)"))?;
    let pw_params = agile_params(ek)?;
    let spin = ek.attr_u32("spinCount").unwrap_or(100_000);
    let field = |n: &str| ek.attr(n).and_then(base64).ok_or_else(|| bad(n));
    let (evhi, evhv, ekv) = (
        field("encryptedVerifierHashInput")?,
        field("encryptedVerifierHashValue")?,
        field("encryptedKeyValue")?,
    );
    // 2.3.4.11: H0 = H(salt + 암호), Hn = H(n + Hn-1), 블록 키마다 H(Hspin + 블록 키)
    let h = pw_params.hash;
    let pw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut hn = h.digest(&[&pw_params.salt[..], &pw].concat());
    for i in 0..spin {
        hn = h.digest(&[&i.to_le_bytes()[..], &hn].concat());
    }
    let derive = |block_key: [u8; 8]| {
        fit(
            h.digest(&[&hn[..], &block_key].concat()),
            pw_params.key_bytes,
        )
    };
    let iv = fit(pw_params.salt.clone(), pw_params.block_size);
    let dec =
        |key: &[u8], data: &[u8]| aes_cbc_decrypt(key, &iv, data).ok_or_else(|| bad("키 길이"));
    let input = dec(
        &derive([0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79]),
        &evhi,
    )?;
    let value = dec(
        &derive([0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e]),
        &evhv,
    )?;
    let salt_size = ek.attr_u32("saltSize").unwrap_or(16) as usize;
    let hash_size = ek
        .attr_u32("hashSize")
        .unwrap_or(h.digest(&[]).len() as u32) as usize;
    let verifier = h.digest(input.get(..salt_size).unwrap_or(&input));
    if verifier.get(..hash_size) != value.get(..hash_size) {
        return Err(Error::WrongPassword);
    }
    let secret = dec(
        &derive([0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6]),
        &ekv,
    )?;
    let secret = secret
        .get(..key_data.key_bytes)
        .ok_or_else(|| bad("비밀 키"))?;
    // 2.3.4.15: 크기 u64 + 4096 바이트 조각, 조각 i 의 IV = H(keyData salt + i)
    let size = package
        .get(..8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
        .ok_or_else(|| bad("패키지"))? as usize;
    let body = &package[8..];
    let mut plain = Vec::with_capacity(body.len().min(size.saturating_add(4096)));
    for (i, seg) in body.chunks(4096).enumerate() {
        let iv = fit(
            key_data
                .hash
                .digest(&[&key_data.salt[..], &(i as u32).to_le_bytes()].concat()),
            key_data.block_size,
        );
        plain.extend(aes_cbc_decrypt(secret, &iv, seg).ok_or_else(|| bad("조각"))?);
    }
    if plain.len() < size {
        return Err(bad("패키지가 짧다"));
    }
    plain.truncate(size);
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(
            sha1(b"abc").to_vec(),
            hex("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(
            sha1(b"").to_vec(),
            hex("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            sha1(long).to_vec(),
            hex("84983e441c3bd26ebaae4aa1f95129e5e54670f1")
        );
    }

    #[test]
    fn aes128_fips197_vector() {
        // FIPS 197 부록 C.1
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let ct = hex("69c4e0d86a7b0430d8cdb78070b4c55a");
        assert_eq!(
            aes128_ecb_decrypt(&key, &ct),
            hex("00112233445566778899aabbccddeeff")
        );
        assert_eq!(
            aes128_ecb_encrypt(&key, &hex("00112233445566778899aabbccddeeff")),
            ct
        );
        assert_eq!(SBOX[0x00], 0x63);
        assert_eq!(SBOX[0x53], 0xED);
    }

    #[test]
    fn hancell_style_standard_encryption() {
        let plain: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let salt = [0x5Au8; 16];
        let (info, package) = encrypt_package(&plain, DEFAULT_PASSWORD, &salt, &salt);
        // 한셀 2024 의 EncryptionInfo 와 같은 길이·머리
        assert_eq!(info.len(), 224);
        assert_eq!(&info[..12], &hex("04 00 02 00 24 00 00 00 8c 00 00 00")[..]);
        assert_eq!(package.len(), 8 + 5008);
        assert_eq!(
            decrypt_package(&info, &package, DEFAULT_PASSWORD).unwrap(),
            plain
        );
        assert!(matches!(
            decrypt_package(&info, &package, "다른 암호"),
            Err(Error::WrongPassword)
        ));
    }

    fn encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
        aes128_ecb_encrypt(key, data)
    }

    #[test]
    fn sha2_vectors() {
        assert_eq!(
            sha256(b"abc").to_vec(),
            hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(
            sha384(b"abc").to_vec(),
            hex(
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
            )
        );
        assert_eq!(
            sha512(b"abc").to_vec(),
            hex(
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
            )
        );
        // 블록 경계 (56·112 바이트 넘김)
        let long = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
        assert_eq!(
            sha512(long).to_vec(),
            hex(
                "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"
            )
        );
    }

    #[test]
    fn aes192_256_fips197_vectors() {
        // FIPS 197 부록 C.2, C.3 (CBC 의 IV 0 = ECB 한 블록)
        let pt = hex("00112233445566778899aabbccddeeff");
        let k192 = hex("000102030405060708090a0b0c0d0e0f1011121314151617");
        let k256 = hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        assert_eq!(
            aes_cbc_decrypt(&k192, &[0; 16], &hex("dda97ca4864cdfe06eaf70a0ec0d7191")).unwrap(),
            pt
        );
        assert_eq!(
            aes_cbc_decrypt(&k256, &[0; 16], &hex("8ea2b7ca516745bfeafc49904b496089")).unwrap(),
            pt
        );
        assert!(aes_cbc_decrypt(&[0; 20], &[0; 16], &pt).is_none());
    }

    #[test]
    fn base64_decoding() {
        assert_eq!(base64("TWFu").unwrap(), b"Man");
        assert_eq!(base64("TWE=").unwrap(), b"Ma");
        assert_eq!(base64("TQ==").unwrap(), b"M");
        assert!(base64("T").is_none());
        assert!(base64("T$==").is_none());
    }

    #[test]
    fn agile_sha512_aes256_two_segments() {
        // Node crypto 로 [MS-OFFCRYPTO] 대로 따로 만든 벡터 (spinCount 1000, 5000 바이트 = 조각 2개)
        let info = hex(include_str!("../tests/fixtures/agile_sha512.info.hex").trim());
        let pkg = hex(include_str!("../tests/fixtures/agile_sha512.pkg.hex").trim());
        let plain = decrypt_package(&info, &pkg, "rcell-테스트").unwrap();
        assert_eq!(plain.len(), 5000);
        assert_eq!(&plain[..4], b"PK");
        assert_eq!(
            sha256(&plain).to_vec(),
            hex("ea5acc53870ff6545cf76e4afdaaee8911372ee2565145efba8b8b4012fd634c")
        );
        assert!(matches!(
            decrypt_package(&info, &pkg, "rcell-테스트!"),
            Err(Error::WrongPassword)
        ));
    }

    #[test]
    fn agile_sha1_aes128() {
        let info = hex(include_str!("../tests/fixtures/agile_sha1.info.hex").trim());
        let pkg = hex(include_str!("../tests/fixtures/agile_sha1.pkg.hex").trim());
        let plain = decrypt_package(&info, &pkg, "pw").unwrap();
        assert_eq!(plain.len(), 100);
        assert_eq!(
            sha256(&plain).to_vec(),
            hex("abbec861a4c822cded9bf32313999822d1258ecccd348d0c64e52ebdc59e0962")
        );
    }

    #[test]
    fn standard_encryption_roundtrip() {
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        assert_eq!(
            encrypt(&key, &hex("00112233445566778899aabbccddeeff")),
            hex("69c4e0d86a7b0430d8cdb78070b4c55a")
        );
        // EncryptionInfo 3.2: 머리(AES-128, SHA-1) + 검증자, EncryptedPackage: 크기 u64 + 암호문
        let salt = [7u8; 16];
        let key: [u8; 16] = derive_key(DEFAULT_PASSWORD, &salt, 128).try_into().unwrap();
        let verifier = [0x5Au8; 16];
        let mut info = vec![3, 0, 2, 0, 0x24, 0, 0, 0, 32, 0, 0, 0];
        for v in [0x24u32, 0, 0x660E, 0x8004, 128, 0x18, 0, 0] {
            info.extend_from_slice(&v.to_le_bytes());
        }
        info.extend_from_slice(&16u32.to_le_bytes());
        info.extend_from_slice(&salt);
        info.extend(encrypt(&key, &verifier));
        info.extend_from_slice(&20u32.to_le_bytes());
        info.extend(encrypt(&key, &sha1(&verifier)));
        let plain = b"PK\x03\x04 rcell encrypted package test".to_vec();
        let mut package = (plain.len() as u64).to_le_bytes().to_vec();
        package.extend(encrypt(&key, &plain));

        assert_eq!(
            decrypt_package(&info, &package, DEFAULT_PASSWORD).unwrap(),
            plain
        );
        assert!(matches!(
            decrypt_package(&info, &package, "wrong"),
            Err(Error::WrongPassword)
        ));
    }
}
