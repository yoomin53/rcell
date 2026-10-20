// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 암호화된 OOXML 패키지 풀기 ([MS-OFFCRYPTO] ECMA-376 표준 암호화).
//!
//! 한셀은 보호(시트·통합 문서 보호)가 걸린 문서를 바이너리 `.cell` 로 저장하거나 레거시 문서를 다시
//! XML 로 저장할 때 OLE2 컨테이너 안에 `EncryptionInfo` + `EncryptedPackage` 로 암호화한다 (관측).
//! 암호를 지정하지 않으면 Office 의 기본 암호 `VelvetSweatshop` 이 쓰인다.
//!
//! 표준 암호화(버전 x.2): AES-128 ECB, SHA-1, 키 유도 50,000회. 외부 라이브러리 없이 구현한다.

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
// AES-128 복호 (FIPS 197)
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

/// AES-128 라운드 키 (11 × 16 바이트)
fn expand_key(key: &[u8; 16]) -> [[u8; 16]; 11] {
    let mut w = [[0u8; 4]; 44];
    for i in 0..4 {
        w[i].copy_from_slice(&key[i * 4..i * 4 + 4]);
    }
    let mut rcon: u8 = 1;
    for i in 4..44 {
        let mut t = w[i - 1];
        if i % 4 == 0 {
            t = [
                SBOX[t[1] as usize],
                SBOX[t[2] as usize],
                SBOX[t[3] as usize],
                SBOX[t[0] as usize],
            ];
            t[0] ^= rcon;
            rcon = xtime(rcon);
        }
        for j in 0..4 {
            w[i][j] = w[i - 4][j] ^ t[j];
        }
    }
    let mut rk = [[0u8; 16]; 11];
    for (r, k) in rk.iter_mut().enumerate() {
        for c in 0..4 {
            k[c * 4..c * 4 + 4].copy_from_slice(&w[r * 4 + c]);
        }
    }
    rk
}

/// 블록 하나 복호 (상태는 열 우선 16바이트)
fn decrypt_block(rk: &[[u8; 16]; 11], block: &mut [u8]) {
    let add = |s: &mut [u8], k: &[u8; 16]| {
        for (x, y) in s.iter_mut().zip(k) {
            *x ^= y;
        }
    };
    add(block, &rk[10]);
    for round in (0..10).rev() {
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

/// AES-128 ECB 복호 (길이는 16의 배수로 자른다)
pub fn aes128_ecb_decrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let rk = expand_key(key);
    let mut out = data[..data.len() / 16 * 16].to_vec();
    for b in out.chunks_mut(16) {
        decrypt_block(&rk, b);
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
    if minor != 2 || !(2..=4).contains(&major) {
        return Err(bad(&format!(
            "버전 {major}.{minor} — 표준 암호화(x.2)만 지원합니다 (Agile 미지원)"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
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
        assert_eq!(SBOX[0x00], 0x63);
        assert_eq!(SBOX[0x53], 0xED);
    }

    /// 시험용 AES-128 암호화 (FIPS 197 순방향)
    fn encrypt_block(rk: &[[u8; 16]; 11], block: &mut [u8]) {
        let add = |s: &mut [u8], k: &[u8; 16]| s.iter_mut().zip(k).for_each(|(x, y)| *x ^= y);
        add(block, &rk[0]);
        for (round, key) in rk.iter().enumerate().skip(1) {
            for x in block.iter_mut() {
                *x = SBOX[*x as usize];
            }
            let s = block.to_vec();
            for c in 0..4 {
                for r in 0..4 {
                    block[c * 4 + r] = s[((c + r) % 4) * 4 + r];
                }
            }
            if round < 10 {
                for c in 0..4 {
                    let a = [
                        block[c * 4],
                        block[c * 4 + 1],
                        block[c * 4 + 2],
                        block[c * 4 + 3],
                    ];
                    for r in 0..4 {
                        block[c * 4 + r] = gmul(a[r], 2)
                            ^ gmul(a[(r + 1) % 4], 3)
                            ^ a[(r + 2) % 4]
                            ^ a[(r + 3) % 4];
                    }
                }
            }
            add(block, key);
        }
    }

    fn encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let rk = expand_key(key);
        let mut out = data.to_vec();
        out.resize(data.len().div_ceil(16) * 16, 0);
        out.chunks_mut(16).for_each(|b| encrypt_block(&rk, b));
        out
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
