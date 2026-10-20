// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 오류 타입.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O 오류: {0}")]
    Io(#[from] std::io::Error),

    #[error("ZIP 컨테이너 오류: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("XML 오류 ({part}): {message}")]
    Xml { part: String, message: String },

    /// OLE2 복합 문서(CFB)를 OOXML 패키지로 열려고 할 때.
    #[error(
        "OLE2 복합 문서 형식입니다 (레거시 .cell 또는 암호화 문서) — ZIP 패키지 기능은 쓸 수 없습니다"
    )]
    LegacyOrEncrypted,

    #[error("암호가 설정된 문서는 아직 지원하지 않습니다")]
    Encrypted,

    #[error("암호화 해제 실패: {0}")]
    Decrypt(String),

    #[error("암호가 맞지 않습니다 (rcell --password 로 문서 암호를 주세요)")]
    WrongPassword,

    #[error("알 수 없는 파일 형식입니다 (ZIP/OLE2 시그니처 없음)")]
    UnknownFormat,

    #[error("필수 파트가 없습니다: {0}")]
    MissingPart(String),

    #[error("잘못된 값 ({context}): {value}")]
    InvalidValue { context: String, value: String },
}

pub type Result<T> = std::result::Result<T, Error>;
