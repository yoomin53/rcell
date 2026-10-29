// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 레거시 `.cell` 의 한컴 차트 레코드 0x7EF0.
//!
//! 논리 레코드 하나(CONTINUE 합침)에 차트 하나가 통째로 들어 있다 (관측, 표본 77개):
//!
//! ```text
//! 머리 43바이트    판 u32 (1), 종류 u16 (0x67), 차트 번호 u16, 앵커 22바이트, 예약 11바이트 (0), u16 (0x11)
//! 원본 범위 수식   cce u16 + 토큰
//! Vt 아카이브      길이 u32 + 객체 그래프 (뿌리 VtChart)
//! 꼬리             항목 수식, 계열 수 u16, 계열마다 값 수식 + 이름 수식
//! ```
//!
//! 아카이브는 객체 그래프를 깊이 우선(전위)으로 쓴 것이다. 포인터는 객체 번호 u32 — 0xFFFFFFFF 없음,
//! 이미 나온 번호면 앞 객체 참조, 다음 번호면 새 객체(클래스 참조 + 본문). 클래스 참조는 번호 u32 이고,
//! 처음 나오는 클래스는 그 자리에 이름(u16 길이 + ASCII + NUL)과 스키마 판 u16 이 붙는다. 기반 클래스
//! 부분은 번호 없이 클래스 참조 + 본문으로 박히며, 본문의 어디에 오는지는 클래스마다 다르다.
//! 필드 배치는 [`CLASSES`] 표에 있고 읽기는 표를 순서대로 따르기만 한다 (찾기·건너뛰기·추측 없음).
//! 표본 77개(만든 차트 69 + 한셀 서식 파일 차트 8) 모두 아카이브가 길이 필드가 가리키는 자리에서 끝나고
//! 레코드가 꼬리까지 남김없이 소비된다. 형식: `docs/format/cell-legacy-chart.md`.
//!
//! 읽기([`super::reader`])는 모든 시트를 읽은 뒤 이 레코드를 [`super::vtchart_map`] 으로 차트 모델로 바꾼다.

// 아카이브를 살펴보는 API 의 일부(스키마 판·위치·정수 꼴·박힌 객체·머리 필드 …)는 차트 변환이 쓰지 않고
// 시험과 진단만 쓴다
#![allow(dead_code)]

use std::fmt;
use std::ops::Range;

use super::formula::quote_sheet;
use crate::model::CellRef;

/// 레코드 종류
pub(crate) const RECORD_TYPE: u16 = 0x7EF0;
/// 기반 클래스 부분(번호 없이 박힌 객체)의 필드 이름
pub(crate) const BASE: &str = "$base";
/// 포인터의 '없음'
const NULL_ID: u32 = 0xFFFF_FFFF;
/// 객체 중첩 한도 (표본 최대 13단계, 기반 부분 포함). 악성 입력이 스택을 넘치게 하지 못하게 한다
const MAX_DEPTH: usize = 64;
/// 읽는 중인 객체 번호의 자리 표시
const UNFINISHED: usize = usize::MAX;

// ---------------------------------------------------------------------------
// 오류
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum VtErrorKind {
    /// 레코드(또는 아카이브·수식) 끝을 넘어 읽으려 함
    Truncated { need: usize },
    /// 머리가 판 1, 종류 0x67 이 아님
    NotChart { version: u32, kind: u16 },
    /// 아카이브 길이가 레코드를 넘음
    BlobTooLong(u32),
    /// 정의되지 않은 클래스 번호
    BadClassIndex { index: u32, defined: usize },
    /// 클래스 이름이 비었거나 NUL 로 끝나지 않음
    BadClassName,
    /// 배치를 모르는 클래스
    UnknownClass(String),
    /// 객체 번호가 앞 번호도, 다음 새 번호도 아님
    BadObjectId { id: u32, next: u32 },
    /// 아직 다 읽지 않은 객체(자기 조상)를 가리킴
    Unfinished(u32),
    /// 포인터 대상·기반 부분의 클래스가 배치와 다름
    ClassMismatch {
        expected: &'static [&'static str],
        found: &'static str,
    },
    /// 객체 중첩이 한도를 넘음
    TooDeep,
    /// 배열 개수를 정하는 필드가 없음
    BadCount(&'static str),
    /// 뿌리 포인터가 없음
    NullRoot,
    /// 아카이브가 길이 필드와 다른 자리에서 끝남
    ArchiveEnd { end: usize, blob_end: usize },
    /// 모르는 수식 토큰
    BadToken(u8),
    /// 수식의 피연산자가 모자라거나 남음
    BadFormula(&'static str),
    /// 꼬리 뒤에 남는 바이트
    TrailingBytes(usize),
}

/// 해석 실패: 레코드 안 위치와 까닭
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VtError {
    pub offset: usize,
    pub kind: VtErrorKind,
}

impl fmt::Display for VtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use VtErrorKind::*;
        f.write_str("한컴 차트(0x7EF0): ")?;
        match &self.kind {
            Truncated { need } => write!(f, "{need}바이트를 더 읽을 수 없음"),
            NotChart { version, kind } => {
                write!(f, "차트 머리가 아님 (판 {version}, 종류 0x{kind:x})")
            }
            BlobTooLong(n) => write!(f, "아카이브 길이 {n} 이 레코드를 넘음"),
            BadClassIndex { index, defined } => {
                write!(f, "클래스 번호 {index} (정의된 클래스 {defined}개)")
            }
            BadClassName => f.write_str("클래스 이름이 NUL 로 끝나지 않음"),
            UnknownClass(n) => write!(f, "배치를 모르는 클래스 {n}"),
            BadObjectId { id, next } => write!(f, "객체 번호 {id} (다음 새 번호 {next})"),
            Unfinished(id) => write!(f, "다 읽지 않은 객체 {id} 를 가리킴"),
            ClassMismatch { expected, found } => {
                write!(f, "{} 자리에 {found}", expected.join("|"))
            }
            TooDeep => write!(f, "객체 중첩이 {MAX_DEPTH}단계를 넘음"),
            BadCount(n) => write!(f, "배열 {n} 의 개수를 알 수 없음"),
            NullRoot => f.write_str("뿌리 객체가 없음"),
            ArchiveEnd { end, blob_end } => {
                write!(
                    f,
                    "아카이브가 0x{end:x} 에서 끝남 (길이 필드로는 0x{blob_end:x})"
                )
            }
            BadToken(p) => write!(f, "모르는 수식 토큰 0x{p:02x}"),
            BadFormula(m) => write!(f, "수식 {m}"),
            TrailingBytes(n) => write!(f, "꼬리 뒤에 {n}바이트가 남음"),
        }?;
        write!(f, " @0x{:x}", self.offset)
    }
}

impl std::error::Error for VtError {}

type Res<T> = std::result::Result<T, VtError>;

// ---------------------------------------------------------------------------
// 바이트 읽기
// ---------------------------------------------------------------------------

/// 레코드 읽기. 위치는 레코드 처음부터의 절대 위치이고 `end` 를 넘어 읽지 않는다
#[derive(Clone)]
struct Rd<'p> {
    b: &'p [u8],
    pos: usize,
    end: usize,
}

impl<'p> Rd<'p> {
    fn new(b: &'p [u8], pos: usize, end: usize) -> Self {
        Rd {
            b,
            pos,
            end: end.min(b.len()),
        }
    }
    fn left(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }
    fn err(&self, kind: VtErrorKind) -> VtError {
        VtError {
            offset: self.pos,
            kind,
        }
    }
    fn take(&mut self, n: usize) -> Res<&'p [u8]> {
        let truncated = || VtErrorKind::Truncated { need: n };
        if n > self.left() {
            return Err(self.err(truncated()));
        }
        let s = self
            .b
            .get(self.pos..self.pos + n)
            .ok_or_else(|| self.err(truncated()))?;
        self.pos += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Res<[u8; N]> {
        let at = self.pos;
        <[u8; N]>::try_from(self.take(N)?).map_err(|_| VtError {
            offset: at,
            kind: VtErrorKind::Truncated { need: N },
        })
    }
    fn u8(&mut self) -> Res<u8> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Res<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn i16(&mut self) -> Res<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn i32(&mut self) -> Res<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Res<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    fn f64(&mut self) -> Res<f64> {
        Ok(f64::from_le_bytes(self.array()?))
    }
}

// ---------------------------------------------------------------------------
// 배치 표의 꼴
// ---------------------------------------------------------------------------

/// 필드 하나의 배치. 본문은 이것들을 순서대로 읽은 것이다
#[derive(Debug, Clone, Copy)]
pub(crate) enum Spec {
    U8(&'static str),
    U16(&'static str),
    I16(&'static str),
    U32(&'static str),
    I32(&'static str),
    F32(&'static str),
    F64(&'static str),
    /// VtColor 6바이트: 종류 u16 (늘 2), R, G, B, 플래그 u8
    Color(&'static str),
    /// 객체 포인터 u32 와 기대 클래스 (빈 목록 = 검사 안 함)
    Ptr(&'static str, &'static [&'static str]),
    /// 번호 없이 박힌 객체 (클래스 참조 + 본문) — 기반 클래스 부분은 이름이 [`BASE`]
    Emb(&'static str, &'static str),
    /// 클래스 표시 없이 그 자리에 오는 묶음 (Pen·Brush …)
    Group(&'static str, &'static StructDef),
    /// 같은 꼴 항목의 배열
    Arr(&'static str, Count, Item),
    /// 앞 필드 값에 따라 있는 필드들 (같은 객체에 붙는다)
    When(Cond, &'static [Spec]),
    /// VtString 내용: cb u32 + cb 바이트 (cp949 글자 NUL, 0x00, UTF-16LE 글자 NUL16)
    VStr(&'static str),
    /// 길이 u32 + 바이트
    LBytes(&'static str),
}

/// 배열 항목의 꼴
#[derive(Debug, Clone, Copy)]
pub(crate) enum Item {
    U16,
    /// 포인터와 기대 클래스 (빈 목록 = 아무것)
    Ptr(&'static [&'static str]),
}

impl Item {
    /// 항목 하나의 최소 바이트 수 (개수를 믿기 전에 남은 바이트와 비교한다)
    fn min_size(self) -> u64 {
        match self {
            Item::U16 => 2,
            Item::Ptr(_) => 4,
        }
    }
}

/// 배열 개수
#[derive(Debug, Clone, Copy)]
pub(crate) enum Count {
    Fixed(usize),
    /// 같은 객체에서 앞서 읽은 필드
    Field(&'static str),
    /// 앞서 읽은 두 필드의 곱 (VtMatrix 폭 × 높이)
    Product(&'static str, &'static str),
}

impl Count {
    fn eval(self, fields: &[Field]) -> Option<u64> {
        let get = |name: &str| own(fields, name)?.as_count();
        match self {
            Count::Fixed(n) => Some(n as u64),
            Count::Field(f) => get(f),
            Count::Product(a, b) => get(a)?.checked_mul(get(b)?),
        }
    }
}

/// 조건
#[derive(Debug, Clone, Copy)]
pub(crate) enum Cond {
    /// 같은 객체에서 앞서 읽은 필드가 이 값
    Eq(&'static str, u64),
}

impl Cond {
    fn holds(self, fields: &[Field]) -> bool {
        match self {
            Cond::Eq(name, v) => own(fields, name).and_then(Value::as_count) == Some(v),
        }
    }
}

/// 묶음 (클래스 표시 없는 구조)
#[derive(Debug)]
pub(crate) struct StructDef {
    pub name: &'static str,
    pub specs: &'static [Spec],
}

/// 클래스 배치
#[derive(Debug)]
pub(crate) struct ClassDef {
    pub name: &'static str,
    /// 표본의 스키마 판 (파일의 판이 다르면 경고)
    pub version: u16,
    pub specs: &'static [Spec],
}

/// 이름으로 클래스 배치
pub(crate) fn class_def(name: &str) -> Option<&'static ClassDef> {
    CLASSES.iter().find(|c| c.name == name)
}

// ---------------------------------------------------------------------------
// 배치 표 (표본 77개에서 확정, 의미는 docs/format/cell-legacy-chart.md)
// ---------------------------------------------------------------------------

use Spec as S;

/// 기반 클래스 부분
const fn base(class: &'static str) -> Spec {
    Spec::Emb(BASE, class)
}

/// 선 (22바이트)
static PEN: StructDef = StructDef {
    name: "Pen",
    specs: &[
        S::U16("u16_a"), // 늘 1
        S::U16("style"), // 0 선 없음, 1 실선, 9 등고선 항목
        S::U16("u16_b"), // 늘 0
        S::U16("u16_c"), // 계열 선 2, 그 밖 0
        S::Color("color"),
        S::F32("width"), // twip (20 = 1pt, 0 = 가는 선·없음)
        S::F32("limit"), // 늘 3.0
    ],
};

/// 칠 (16바이트)
static BRUSH: StructDef = StructDef {
    name: "Brush",
    specs: &[
        S::U16("style"), // 1 단색, 0 없음
        S::Color("fill"),
        S::Color("pattern"), // 무늬 색
        S::U16("index"),     // 무늬 번호, 늘 0
    ],
};

/// 그라데이션 (14바이트)
static GRADIENT: StructDef = StructDef {
    name: "Gradient",
    specs: &[S::U16("style"), S::Color("from"), S::Color("to")],
};

/// 그림자 (26바이트)
static SHADOW: StructDef = StructDef {
    name: "Shadow",
    specs: &[
        S::U16("style"),
        S::Group("brush", &BRUSH),
        S::F32("offsetX"),
        S::F32("offsetY"),
    ],
};

/// 테두리 (18바이트)
static FRAME: StructDef = StructDef {
    name: "Frame",
    specs: &[
        S::U16("style"),
        S::Color("color"),
        S::Color("spaceColor"),
        S::F32("width"), // twip
    ],
};

/// 분산형 X 축 눈금 (24바이트, VtAxis.xy == 1 일 때만)
static XY_SCALE: StructDef = StructDef {
    name: "XYScale",
    specs: &[
        S::F64("f64_a"),
        S::F64("f64_b"),
        S::U16("u16_a"),
        S::U16("u16_b"),
        S::U16("u16_c"),
        S::U8("u8_a"),
        S::U8("u8_b"),
    ],
};

/// 3차원 벽 (44바이트)
static WALL: StructDef = StructDef {
    name: "Wall",
    specs: &[
        S::U16("u16_a"),
        S::Group("brush", &BRUSH),
        S::Group("pen", &PEN),
        S::F32("width"),
    ],
};

/// 3차원 바닥 (44바이트)
static FLOOR: StructDef = StructDef {
    name: "Floor",
    specs: &[
        S::U16("u16_a"),
        S::Group("brush", &BRUSH),
        S::Group("pen", &PEN),
        S::U16("u16_b"),
        S::U16("u16_c"),
    ],
};

/// 클래스 39개, 흔한 차트의 클래스 번호 순서
pub(crate) static CLASSES: &[ClassDef] = &[
    // 아카이브의 뿌리
    ClassDef {
        name: "VtChart",
        version: 6,
        specs: &[
            S::Ptr("dataGrid", &["VtDataGrid"]),
            S::Ptr("backdrop", &["VtBackdrop"]), // 차트 영역
            S::Ptr("footnote", &["VtChartFootnote"]),
            S::Ptr("legend", &["VtChartLegend"]),
            S::Ptr("plot", &["VtChartPlot"]),
            S::Ptr("seriesList", &["VtArray"]), // VtSeries 배열 (기본 자리 계열 포함)
            S::Ptr("title", &["VtChartTitle"]),
            S::Ptr("list", &["VtList"]),
            S::U16("u16_a"),
            S::U16("u16_b"),
            S::F32("f32_a"), // 0, 일부 3차원 차트 90
            S::Ptr("ssLink", &["VtChSsLink"]),
            S::U16("mask"), // 0xFFFF, 영역·누적·100% 0xFF3F, 가로 막대 0xFFCF
            S::U16("u16_c"),
            S::U16("u16_d"),
            S::U16("u16_e"),
            S::U16("u16_f"),
            // 0 3차원 막대, 1 막대, 2 3차원 꺾은선, 3 꺾은선 (분산형·거품형도), 4 3차원 영역, 5 영역,
            // 11 가로 막대, 13 3차원 원형, 14 원형, 15 도넛형, 18 방사형 (주식형은 1)
            S::U16("chartType"),
            S::U32("u32_a"),
            base("VtWindow"),
        ],
    },
    // 차트 데이터 캐시: 기반 VtMatrix 가 먼저 온다
    ClassDef {
        name: "VtDataGrid",
        version: 1,
        specs: &[
            base("VtMatrix"),
            S::U16("labelCols"), // 항목 이름표 열 수 (1, 항목이 없으면 0)
            S::U16("labelRows"), // 계열 이름표 행 수
            S::U16("rows"),      // 데이터 행 = 항목 수
            S::U16("cols"),      // 데이터 열 = 계열 수
            S::U16("u16_a"),     // 1, 원형·도넛형 0
            S::Arr("reserved", Count::Fixed(8), Item::U16),
        ],
    },
    // 열 우선 값 행렬: 0열 = 항목 이름표, 0행 = 계열 이름, [0][0] 없음
    ClassDef {
        name: "VtMatrix",
        version: 1,
        specs: &[
            base("VtCollection"),
            S::U16("width"),
            S::U16("height"),
            S::Arr(
                "cells",
                Count::Product("width", "height"),
                Item::Ptr(&["VtString", "VtDouble"]),
            ),
        ],
    },
    ClassDef {
        name: "VtCollection",
        version: 1,
        specs: &[
            S::U16("count"), // 행렬의 빈 칸 아닌 칸 수
            base("VtObject"),
        ],
    },
    // 클래스 계층의 뿌리, 내용 없음
    ClassDef {
        name: "VtObject",
        version: 1,
        specs: &[],
    },
    ClassDef {
        name: "VtString",
        version: 2,
        specs: &[
            S::VStr("text"),
            S::U8("pad"), // 초기화되지 않은 바이트 (값은 아무것)
            base("VtValue"),
        ],
    },
    // 셀 값의 기반 (VtString·VtDouble·VtBytes)
    ClassDef {
        name: "VtValue",
        version: 1,
        specs: &[base("VtObject")],
    },
    ClassDef {
        name: "VtDouble",
        version: 1,
        specs: &[
            S::F64("value"),
            S::U16("flags"), // 늘 0xFFFF
            base("VtValue"),
        ],
    },
    // 배경: 그림자 + 테두리 + 채우기
    ClassDef {
        name: "VtBackdrop",
        version: 1,
        specs: &[
            S::U16("u16_a"),
            S::Group("shadow", &SHADOW),
            S::U16("u16_b"),
            S::Group("frame", &FRAME),
            S::U16("u16_c"), // 차트 영역 배경 1 (테두리 그림), 그 밖 0
            S::Ptr("fill", &["VtFill"]),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtFill",
        version: 1,
        specs: &[
            S::U16("style"),
            S::Group("brush", &BRUSH),
            S::Group("gradient", &GRADIENT),
            S::U16("u16_a"),
            S::Ptr("picture", &["VtPicture"]),
            S::U16("u16_b"),
            base("VtObject"),
        ],
    },
    // 채우기 그림 (표본에서 늘 빔)
    ClassDef {
        name: "VtPicture",
        version: 1,
        specs: &[
            S::U16("u16_a"),
            S::U16("u16_b"),
            S::Ptr("data", &[]),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtChartFootnote",
        version: 1,
        specs: &[base("VtChartText")],
    },
    // 글 요소 = 글 상자 + 구역
    ClassDef {
        name: "VtChartText",
        version: 1,
        specs: &[
            S::Ptr("textBlock", &["VtTextBlock"]),
            base("VtChartSection"),
        ],
    },
    // 글 상자: 배치 플래그, 배경, 글꼴, 글
    ClassDef {
        name: "VtTextBlock",
        version: 2,
        specs: &[
            S::U16("u16_a"),
            S::U16("u16_b"), // 사용자 글이 있거나 보일 때 1
            S::U16("u16_c"),
            S::U16("u16_d"),
            S::U16("u16_e"), // 0, 사용자가 쓴 값 축 제목 2
            S::U16("u16_f"),
            S::Ptr("backdrop", &["VtBackdrop"]), // 자기 배경이 없으면 없음
            S::Ptr("font", &["VtFont"]),
            S::U16("u16_g"),
            S::U16("u16_h"), // 늘 100
            S::Arr("reserved1", Count::Fixed(10), Item::U16),
            S::Ptr("text", &["VtString"]),
            S::Arr("reserved2", Count::Fixed(13), Item::U16),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtFont",
        version: 1,
        specs: &[
            S::Ptr("name", &["VtString"]), // 흔히 앞 문자열 참조
            S::F32("size"),                // pt
            S::U32("style"),               // 0, 3
            S::Color("color"),
            base("VtObject"),
        ],
    },
    // 위치가 있는 차트 요소
    ClassDef {
        name: "VtChartSection",
        version: 1,
        specs: &[
            S::U16("u16_a"), // 늘 160
            S::U16("u16_b"), // 늘 150
            S::U16("u16_c"),
            S::U16("visible"), // 1 보임
            S::F32("x1"),
            S::F32("y1"),
            S::F32("x2"),
            S::F32("y2"),
            S::U16("location"), // 범례 1 위, 3 왼쪽, 4 오른쪽 (오른쪽 위도), 6 아래; 각주 5; 제목·그림 영역 1
            S::Ptr("backdrop", &["VtBackdrop"]),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtChartLegend",
        version: 1,
        specs: &[
            S::Ptr("font", &["VtFont"]),
            S::U16("u16_a"),
            S::U16("u16_b"),
            S::U16("u16_c"),
            S::U16("u16_d"),
            S::U16("u16_e"),
            base("VtChartSection"),
        ],
    },
    // 그림 영역: 막대 배치, 벽, 3차원 보기, 조명, 축
    ClassDef {
        name: "VtChartPlot",
        version: 5,
        specs: &[
            S::Ptr("ptr_a", &["VtArray"]), // 늘 빈 배열
            S::F32("f32_a"),
            S::F32("f32_b"),
            S::F32("overlap"),  // -겹치기/100 (누적·100% 막대 -1)
            S::F32("gap"),      // 간격 너비/100 (막대 1.5, 그 밖 1, 도넛형 0)
            S::F32("gapDepth"), // 1, 3차원 1.5, 3차원 원형·도넛형 0
            S::F32("f32_c"),
            S::Group("wall", &WALL),
            S::Group("floor", &FLOOR),
            S::F32("rotation"),  // 3차원 rotY (도)
            S::F32("elevation"), // 3차원 rotX (도)
            S::F32("f32_d"),
            S::F32("f32_e"),
            S::F32("f32_f"),
            S::U32("u32_a"),
            S::Ptr("light", &["VtLight3"]),
            S::Ptr("axisX", &["VtAxis"]),
            S::Ptr("axesY", &["VtArray"]),  // VtAxis 하나
            S::Ptr("axesY2", &["VtArray"]), // VtAxis 하나 (보조 축, 숨김)
            S::Ptr("axisZ", &["VtAxis"]),
            S::U16("u16_a"),
            S::F32("f32_g"), // 90, 3차원 원형 70
            S::F32("f32_h"),
            S::F32("f32_i"), // 0.33, 도넛형 0.01
            S::F32("f32_j"),
            S::F32("f32_k"),
            S::U16("u16_b"),
            S::F32("f32_l"),
            S::U16("u16_c"),
            S::Ptr("surface", &["VtSurfaceDesc"]),
            S::Arr("u16s", Count::Fixed(8), Item::U16),
            base("VtChartSection"), // 그림 영역의 보임·위치·배경
        ],
    },
    // 포인터 배열: 머리, 기반 부분, 그다음 항목
    ClassDef {
        name: "VtArray",
        version: 2,
        specs: &[
            S::U16("size"), // 항목 자리 수
            S::U16("u16_a"),
            S::U32("u32_a"),
            S::U16("nonNull"), // 없음이 아닌 항목 수
            base("VtObject"),
            S::Arr("items", Count::Field("size"), Item::Ptr(&[])),
        ],
    },
    ClassDef {
        name: "VtLight3",
        version: 1,
        specs: &[
            S::Ptr("sources", &["VtArray"]), // VtInfLight3 배열
            S::F32("ambient"),
            S::F32("edge"),
            S::U16("edgeVisible"),
            base("VtObject"),
        ],
    },
    // 광원: 방향 + 세기
    ClassDef {
        name: "VtInfLight3",
        version: 1,
        specs: &[
            S::F32("x"),
            S::F32("y"),
            S::F32("z"),
            S::F32("intensity"),
            base("VtObject"),
        ],
    },
    // 축 (X, Y, Y2, Z, 표면 축)
    ClassDef {
        name: "VtAxis",
        version: 6,
        specs: &[
            S::U16("hidden"), // 1 축 숨김 (Y2 는 기본 숨김)
            S::U16("u16_a"),  // 1, 제목을 쓰면 0
            S::Ptr("bytes", &["VtBytes"]),
            S::U16("u16_b"),
            S::U16("xy"), // 분산형·거품형의 X 축 1 → xyScale 이 붙는다
            S::Group("axisPen", &PEN),
            S::Group("majorGridPen", &PEN), // style 1 = 주 눈금선 있음
            S::Group("minorGridPen", &PEN),
            S::F32("f32_a"),
            S::U32("u32_a"),
            S::Ptr("title", &["VtTextBlock"]),
            S::Ptr("labels", &["VtArray"]), // VtAxisScaleBlock 배열
            S::U16("u16_c"),
            S::F32("f32_b"), // 0, 방사형 X 축 90
            S::U16("u16_d"),
            S::F32("f32_c"), // 0, 방사형 Y 축 90
            S::U16("u16_e"),
            S::U16("u16_f"),
            S::U16("u16_g"),
            S::U16("u16_h"),
            S::U16("u16_i"),
            S::U16("u16_j"),
            S::U16("u16_k"), // 원형이 아닌 차트의 X 축 1
            S::U16("u16_l"),
            S::U16("u16_m"),
            S::U16("u16_n"),
            S::U16("u16_o"),
            S::U16("u16_p"),
            S::U16("u16_q"), // 영역형 X 축 1
            S::F32("f32_d"),
            S::When(Cond::Eq("xy", 1), &[S::Group("xyScale", &XY_SCALE)]),
            S::U16("u16_r"),     // 100% 누적 값 축 2
            S::U16("u16_s"),     // 4, 표면 축 2, 방사형 0
            S::U16("tickStyle"), // 주 눈금: 3 바깥 (보이는 X·Y 축), 1, 0 (표면 축·원형 X 축)
            S::Arr("reserved", Count::Fixed(7), Item::U16),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtBytes",
        version: 1,
        specs: &[S::LBytes("data"), base("VtValue")],
    },
    // 축 이름표 단계 (VtAxis.labels 의 항목)
    ClassDef {
        name: "VtAxisScaleBlock",
        version: 1,
        specs: &[
            S::Ptr("slots", &["VtArray"]), // 없음 5개
            S::U16("u16_a"),
            S::U16("u16_b"), // Y 축 1, X·Z 축 0
            base("VtValueBlock"),
        ],
    },
    // 축 이름표: 표시 형식 + 글 상자 (기반 부분)
    ClassDef {
        name: "VtValueBlock",
        version: 1,
        specs: &[
            S::Ptr("ptr_a", &[]),
            S::Ptr("textFormat", &["VtTextFormat"]),
            S::U16("u16_a"),
            S::Ptr("formatCode", &["VtString"]),
            S::U8("u8_a"),
            S::U8("u8_b"),
            S::U8("u8_c"),
            base("VtTextBlock"),
        ],
    },
    ClassDef {
        name: "VtTextFormat",
        version: 2,
        specs: &[
            base("VtObject"),
            S::U16("u16_a"),
            S::Ptr("group", &["VtFormatGroup"]),
        ],
    },
    // 네 구역 표시 형식 (양수; 음수; 0; 글자)
    ClassDef {
        name: "VtFormatGroup",
        version: 2,
        specs: &[
            base("VtObject"),
            S::U16("u16_a"),
            S::Ptr("section1", &["VtFormatter"]),
            S::Ptr("section2", &["VtFormatter"]),
            S::Ptr("section3", &["VtFormatter"]),
            S::Ptr("section4", &["VtFormatter"]),
        ],
    },
    // 표시 형식 구역 하나
    ClassDef {
        name: "VtFormatter",
        version: 3,
        specs: &[
            base("VtObject"),
            S::U16("u16_a"),
            S::U16("u16_b"),
            S::U16("u16_c"),
            S::U16("u16_d"),
            S::Arr("u16s", Count::Fixed(9), Item::U16),
            S::Color("color"),
            S::Ptr("code", &["VtString"]), // 내부 부호화한 서식 코드 (G/표준 → "Zlg표g준n")
            S::Ptr("digits1", &["VtDigitSet"]),
            S::Ptr("digits2", &["VtDigitSet"]),
            S::Ptr("digits3", &["VtDigitSet"]),
        ],
    },
    ClassDef {
        name: "VtDigitSet",
        version: 2,
        specs: &[
            base("VtObject"),
            S::U16("u16_a"),
            S::U16("u16_b"),
            S::U16("u16_c"),
        ],
    },
    // 표면·등고선 설명 (표본의 차트 종류에서는 쓰지 않음)
    ClassDef {
        name: "VtSurfaceDesc",
        version: 1,
        specs: &[
            S::U16("u16_a"),
            S::Group("brush", &BRUSH),
            S::Group("pen", &PEN),
            S::Arr("reserved1", Count::Fixed(3), Item::U16),
            S::Ptr("list", &["VtArray"]),
            S::Ptr("axis", &["VtAxis"]),
            S::U16("u16_b"),
            S::Ptr("item1", &["VtCLineItem"]),
            S::Ptr("item2", &["VtCLineItem"]),
            S::Arr("u16s", Count::Fixed(7), Item::U16),
            base("VtObject"),
        ],
    },
    // 등고선 띠 항목
    ClassDef {
        name: "VtCLineItem",
        version: 1,
        specs: &[
            S::Arr("reserved", Count::Fixed(4), Item::U16),
            S::U16("u16_a"),
            S::Group("brush", &BRUSH),
            S::Group("pen", &PEN),
            S::Ptr("ptr_a", &[]),
            base("VtObject"),
        ],
    },
    // 계열 하나
    ClassDef {
        name: "VtSeries",
        version: 5,
        specs: &[
            S::Group("linePen", &PEN), // 계열 색, 40 twip
            S::Group("edgePen", &PEN), // 막대·영역 테두리 (지정하면 style 1 + 두께)
            S::U16("u16_a"),
            S::Group("brush", &BRUSH), // 계열 채우기 색
            S::Ptr("ptr_a", &[]),
            S::Ptr("points", &["VtArray"]), // 요소별 서식 (늘 빔)
            S::Arr("u16s1", Count::Fixed(4), Item::U16),
            S::Group("pen3", &PEN),
            S::Arr("u16s2", Count::Fixed(18), Item::U16), // [4]–[6] 계열 순서, 누적이면 [6]/[7] 1
            S::Group("markerPen", &PEN),
            S::U16("u16_b"),
            S::Ptr("name", &["VtString"]),
            S::Ptr("seriesLabel", &["VtSeriesLabel"]),
            S::Ptr("textBlock", &["VtTextBlock"]),
            S::U16("labelComponents"), // 데이터 레이블: 1 값, 2 백분율, 8 항목
            S::Ptr("valueFormat", &["VtTextFormat"]),
            S::Ptr("percentFormat", &["VtTextFormat"]),
            S::Ptr("dataLabel", &["VtSeriesLabel"]), // 레이블을 보이면 u16_b = 1
            S::U16("u16_c"),
            S::U16("u16_d"), // 1, 표식을 지정하면·거품형 0
            S::U16("u16_e"),
            S::Group("pen5", &PEN),
            S::F32("markerSize"),
            S::U16("showLine"), // 꺾은선·분산형·방사형 1
            S::Color("markerColor"),
            S::Ptr("picture", &["VtPicture"]),
            S::U16("markerStyle"),
            S::U16("u16_f"),
            S::U16("u16_g"),
            S::F32("f32_a"),
            S::Color("color2"),
            S::Color("color3"),
            S::U16("index"), // 계열 번호
            S::U32("typeCount"),
            S::Arr("typeTable", Count::Field("typeCount"), Item::U16),
            S::Arr("u16s3", Count::Fixed(10), Item::U16), // [5] 2 = 부드러운 선, [8] 레이블 위치
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtSeriesLabel",
        version: 1,
        specs: &[base("VtTextBlock")],
    },
    // 요소 하나의 서식 (VtSeries.points 의 항목 — 한셀이 COM 으로 만든 원형 차트: 요소마다 색)
    ClassDef {
        name: "VtSeriesPoint",
        version: 1,
        specs: &[
            S::Ptr("label", &["VtSeriesLabel"]),
            S::F32("explode"), // 쪼개기 (반지름의 비율: 0.25 = 25%)
            S::Arr("u16s", Count::Fixed(4), Item::U16),
            S::Ptr("element", &["VtSeriesPointElement"]),
            S::Ptr("ptr_a", &[]),
            base("VtObject"),
        ],
    },
    // 요소 서식의 본문: VtSeries 의 칠·선·표식 부분과 같은 꼴
    ClassDef {
        name: "VtSeriesPointElement",
        version: 1,
        specs: &[
            S::U16("u16_a"),
            S::Group("brush", &BRUSH), // 요소 채우기 색
            S::Group("edgePen", &PEN),
            S::Ptr("ptr_a", &[]),
            S::U16("u16_b"),
            S::Group("markerPen", &PEN),
            S::F32("markerSize"),
            S::U16("showLine"),
            S::Color("markerColor"),
            S::Ptr("picture", &["VtPicture"]),
            S::U16("markerStyle"),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtChartTitle",
        version: 1,
        specs: &[base("VtChartText")],
    },
    // 목록 (표본에서 늘 빔 — 항목 꼴은 추정)
    ClassDef {
        name: "VtList",
        version: 2,
        specs: &[
            S::U32("u32_a"),
            S::U16("count"),
            base("VtObject"),
            S::Arr("items", Count::Field("count"), Item::Ptr(&[])),
        ],
    },
    // 차트 ↔ 시트 연결
    ClassDef {
        name: "VtChSsLink",
        version: 2,
        specs: &[
            S::U16("u16_a"),
            S::Ptr("dataSource", &["VtDataSourceSsLink"]),
            base("VtObject"),
        ],
    },
    ClassDef {
        name: "VtDataSourceSsLink",
        version: 1,
        specs: &[
            S::Ptr("ptr_a", &[]),
            S::Ptr("ptr_b", &[]),
            base("VtDataSource"),
        ],
    },
    ClassDef {
        name: "VtDataSource",
        version: 1,
        specs: &[base("VtObject")],
    },
    // VtChart 의 기반
    ClassDef {
        name: "VtWindow",
        version: 3,
        specs: &[base("VtObject"), S::U16("u16_a")],
    },
];

// ---------------------------------------------------------------------------
// 읽은 값
// ---------------------------------------------------------------------------

/// VtColor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VtColor {
    /// 늘 2
    pub kind: u16,
    pub rgb: [u8; 3],
    /// 0; 계열 색 사본 0x80, 일부 0x20 (뜻은 모름)
    pub flags: u8,
}

impl VtColor {
    /// `rrggbb`
    pub fn hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.rgb[0], self.rgb[1], self.rgb[2])
    }
}

/// VtString 내용
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VtStr {
    /// UTF-16 부분 (cp949 부분과 표본에서 늘 같다). 모양이 다르면 없음 (경고)
    pub text: Option<String>,
    /// cb 바이트의 레코드 안 범위
    pub raw: Range<usize>,
}

/// 포인터
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ptr {
    /// 객체 번호 (읽은 순서, 뿌리 0)
    pub id: u32,
    /// 앞 객체 참조였으면 true, 이 자리에서 새로 읽은 객체면 false
    pub back_ref: bool,
}

/// 필드 값
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    U8(u8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    F64(f64),
    Color(VtColor),
    /// 객체 포인터 (없음 = None)
    Ptr(Option<Ptr>),
    /// 번호 없이 박힌 객체 (기반 클래스 부분): 노드 번호
    Embedded(usize),
    /// 묶음 (Pen·Brush …)
    Group(Vec<Field>),
    Array(Vec<Value>),
    Str(VtStr),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn as_u8(&self) -> Option<u8> {
        match *self {
            Value::U8(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_u16(&self) -> Option<u16> {
        match *self {
            Value::U16(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_i16(&self) -> Option<i16> {
        match *self {
            Value::I16(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_u32(&self) -> Option<u32> {
        match *self {
            Value::U32(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_i32(&self) -> Option<i32> {
        match *self {
            Value::I32(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Value::F32(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::F64(v) => Some(v),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<VtColor> {
        match *self {
            Value::Color(c) => Some(c),
            _ => None,
        }
    }
    /// 포인터의 대상 번호 (없음·포인터 아님 = None)
    pub fn ptr_id(&self) -> Option<u32> {
        match *self {
            Value::Ptr(Some(p)) => Some(p.id),
            _ => None,
        }
    }
    /// 없음 포인터인지
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Ptr(None))
    }
    /// 부호 없는 정수 필드 (배열 개수·조건)
    fn as_count(&self) -> Option<u64> {
        match *self {
            Value::U8(v) => Some(v.into()),
            Value::U16(v) => Some(v.into()),
            Value::U32(v) => Some(v.into()),
            _ => None,
        }
    }
}

/// 이름 붙은 필드
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Field {
    pub name: &'static str,
    pub value: Value,
    /// 레코드 안에서 필드가 시작하는 자리 (`rcell vtdump --offsets`)
    pub at: usize,
}

/// 같은 목록에서 이름으로 (기반 부분은 찾지 않는다)
fn own<'f>(fields: &'f [Field], name: &str) -> Option<&'f Value> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}

/// 객체 하나 (번호 있는 객체와 기반 부분)
#[derive(Debug, Clone, PartialEq)]
struct Node {
    id: Option<u32>,
    /// [`Archive::classes`] 번호
    class: usize,
    offset: usize,
    end: usize,
    fields: Vec<Field>,
}

/// 아카이브에 정의된 클래스 (정의 순서 = 클래스 번호)
#[derive(Debug, Clone, Copy)]
pub(crate) struct ClassInfo {
    pub def: &'static ClassDef,
    /// 파일에 적힌 스키마 판
    pub version: u16,
    /// 정의가 있던 자리
    pub offset: usize,
}

/// 읽은 Vt 아카이브
#[derive(Debug, Clone)]
pub(crate) struct Archive {
    pub classes: Vec<ClassInfo>,
    nodes: Vec<Node>,
    /// 객체 번호 → 노드
    by_id: Vec<usize>,
    root: usize,
    /// 앞 객체 참조 수
    pub back_refs: usize,
    /// 가장 깊은 객체 중첩 (뿌리 1)
    pub max_depth: usize,
    /// 아카이브 끝 (= 길이 필드가 가리키는 자리)
    pub end: usize,
    /// 구조는 맞지만 값이 예상과 다른 것 (VtColor 종류, VtString 모양, 배열 개수 …)
    pub warnings: Vec<String>,
    /// VtString 의 pad 바이트 자리 (레코드 안 위치) — 한셀이 초기화하지 않아 저장마다 다르다
    pub pads: Vec<usize>,
}

impl Archive {
    /// 뿌리 VtChart
    pub fn root(&self) -> Obj<'_> {
        Obj {
            arch: self,
            node: &self.nodes[self.root],
        }
    }
    /// 번호로 객체
    pub fn object(&self, id: u32) -> Option<Obj<'_>> {
        self.node_obj(*self.by_id.get(id as usize)?)
    }
    /// 번호 있는 객체 수
    pub fn object_count(&self) -> usize {
        self.by_id.len()
    }
    /// 번호 순서로 모든 객체
    pub fn objects(&self) -> impl Iterator<Item = Obj<'_>> {
        self.by_id.iter().filter_map(|&i| self.node_obj(i))
    }
    /// 포인터나 박힌 객체 값이 가리키는 객체
    pub fn resolve(&self, v: &Value) -> Option<Obj<'_>> {
        match *v {
            Value::Ptr(Some(p)) => self.object(p.id),
            Value::Embedded(i) => self.node_obj(i),
            _ => None,
        }
    }
    fn node_obj(&self, i: usize) -> Option<Obj<'_>> {
        Some(Obj {
            arch: self,
            node: self.nodes.get(i)?,
        })
    }

    /// 자체 검사 (경고만): 배열의 nonNull, 데이터 표의 칸 수와 크기
    fn self_check(&mut self) {
        let mut w = Vec::new();
        for o in self.objects() {
            match o.class() {
                "VtArray" => {
                    let real = o.array("items").map_or(0, non_null);
                    if o.get("nonNull").and_then(Value::as_count) != Some(real as u64) {
                        w.push(format!(
                            "VtArray #{} 의 nonNull {:?} ≠ 실제 {real}",
                            o.id().unwrap_or(0),
                            o.u16("nonNull")
                        ));
                    }
                }
                "VtDataGrid" => {
                    let real = o.array("cells").map_or(0, non_null);
                    if o.get("count").and_then(Value::as_count) != Some(real as u64) {
                        w.push(format!(
                            "VtCollection count {:?} ≠ 빈 칸 아닌 칸 {real}",
                            o.u16("count")
                        ));
                    }
                    let sum = |a: &str, b: &str| Some(u32::from(o.u16(a)?) + u32::from(o.u16(b)?));
                    if o.u16("width").map(u32::from) != sum("labelCols", "cols")
                        || o.u16("height").map(u32::from) != sum("labelRows", "rows")
                    {
                        w.push("VtDataGrid 크기가 이름표 + 데이터 수와 다름".into());
                    }
                }
                _ => {}
            }
        }
        self.warnings.extend(w);
    }
}

fn non_null(items: &[Value]) -> usize {
    items.iter().filter(|v| !v.is_null()).count()
}

#[cfg(test)]
impl Archive {
    /// 쓰기 나무로 (다시 쓰면 같은 바이트가 되는지 보는 시험용). `payload` 는 레코드 전체
    pub(crate) fn to_wobj(&self, payload: &[u8]) -> super::vtwrite::WObj {
        self.node_wobj(&self.nodes[self.root], payload)
    }

    fn node_wobj(&self, node: &Node, payload: &[u8]) -> super::vtwrite::WObj {
        let class = self.classes[node.class].def.name;
        let mut o = super::vtwrite::WObj::new(class);
        o.fields = node
            .fields
            .iter()
            .map(|f| (f.name, self.value_w(&f.value, payload)))
            .collect();
        o
    }

    fn value_w(&self, v: &Value, payload: &[u8]) -> super::vtwrite::W {
        use super::vtwrite::W;
        match v {
            Value::U8(x) => W::U8(*x),
            Value::U16(x) => W::U16(*x),
            Value::I16(x) => W::I16(*x),
            Value::U32(x) => W::U32(*x),
            Value::I32(x) => W::I32(*x),
            Value::F32(x) => W::F32(*x),
            Value::F64(x) => W::F64(*x),
            Value::Color(c) => W::Color(
                (u32::from(c.rgb[0]) << 16) | (u32::from(c.rgb[1]) << 8) | u32::from(c.rgb[2]),
                c.flags,
            ),
            Value::Ptr(None) => W::Null,
            Value::Ptr(Some(p)) if p.back_ref => W::Back(p.id),
            Value::Ptr(Some(p)) => W::New(Box::new(
                self.node_wobj(&self.nodes[self.by_id[p.id as usize]], payload),
            )),
            Value::Embedded(i) => W::Emb(Box::new(self.node_wobj(&self.nodes[*i], payload))),
            Value::Group(fields) => W::Group(
                fields
                    .iter()
                    .map(|f| (f.name, self.value_w(&f.value, payload)))
                    .collect(),
            ),
            Value::Array(items) => W::Arr(items.iter().map(|x| self.value_w(x, payload)).collect()),
            Value::Str(s) => W::RawStr(payload[s.raw.clone()].to_vec()),
            Value::Bytes(b) => W::Bytes(b.clone()),
        }
    }
}

/// 필드 읽기. 객체는 자기 필드에 없으면 기반 클래스 부분을 차례로 찾는다
pub(crate) trait Fields<'a>: Copy {
    fn archive(self) -> &'a Archive;
    /// 이름으로 필드 값
    fn get(self, name: &str) -> Option<&'a Value>;

    fn u8(self, name: &str) -> Option<u8> {
        self.get(name)?.as_u8()
    }
    fn u16(self, name: &str) -> Option<u16> {
        self.get(name)?.as_u16()
    }
    fn i16(self, name: &str) -> Option<i16> {
        self.get(name)?.as_i16()
    }
    fn u32(self, name: &str) -> Option<u32> {
        self.get(name)?.as_u32()
    }
    fn i32(self, name: &str) -> Option<i32> {
        self.get(name)?.as_i32()
    }
    fn f32(self, name: &str) -> Option<f32> {
        self.get(name)?.as_f32()
    }
    fn f64(self, name: &str) -> Option<f64> {
        self.get(name)?.as_f64()
    }
    fn color(self, name: &str) -> Option<VtColor> {
        self.get(name)?.as_color()
    }
    /// 포인터를 따라간 객체 (없음 포인터·다른 꼴 = None)
    fn ptr(self, name: &str) -> Option<Obj<'a>> {
        match self.get(name)? {
            v @ Value::Ptr(_) => self.archive().resolve(v),
            _ => None,
        }
    }
    /// 번호 없이 박힌 객체
    fn embedded(self, name: &str) -> Option<Obj<'a>> {
        match self.get(name)? {
            v @ Value::Embedded(_) => self.archive().resolve(v),
            _ => None,
        }
    }
    /// 묶음 (Pen·Brush …)
    fn group(self, name: &str) -> Option<Group<'a>> {
        match self.get(name)? {
            Value::Group(fields) => Some(Group {
                arch: self.archive(),
                fields,
            }),
            _ => None,
        }
    }
    fn array(self, name: &str) -> Option<&'a [Value]> {
        match self.get(name)? {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
    /// u16 배열
    fn u16s(self, name: &str) -> Option<Vec<u16>> {
        self.array(name)?.iter().map(Value::as_u16).collect()
    }
    /// 포인터 배열을 따라간 객체들 (없음 = None)
    fn ptrs(self, name: &str) -> Option<Vec<Option<Obj<'a>>>> {
        let arch = self.archive();
        Some(self.array(name)?.iter().map(|v| arch.resolve(v)).collect())
    }
    /// 글: VtString 내용 필드, 또는 VtString 을 가리키는 포인터
    fn string(self, name: &str) -> Option<&'a str> {
        match self.get(name)? {
            Value::Str(s) => s.text.as_deref(),
            v @ Value::Ptr(_) => match self.archive().resolve(v)?.get("text")? {
                Value::Str(s) => s.text.as_deref(),
                _ => None,
            },
            _ => None,
        }
    }
}

/// 객체 보기
#[derive(Clone, Copy)]
pub(crate) struct Obj<'a> {
    arch: &'a Archive,
    node: &'a Node,
}

impl fmt::Debug for Obj<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.node.id {
            Some(id) => write!(f, "#{id} {}", self.class()),
            None => write!(f, "{} (기반 부분)", self.class()),
        }
    }
}

impl<'a> Obj<'a> {
    /// 객체 번호 (기반 부분은 None)
    pub fn id(self) -> Option<u32> {
        self.node.id
    }
    pub fn class(self) -> &'static str {
        self.arch
            .classes
            .get(self.node.class)
            .map_or("", |c| c.def.name)
    }
    /// 파일에 적힌 스키마 판
    pub fn version(self) -> u16 {
        self.arch
            .classes
            .get(self.node.class)
            .map_or(0, |c| c.version)
    }
    /// 시작 위치 (번호 있는 객체는 포인터 자리, 기반 부분은 클래스 참조 자리)
    pub fn offset(self) -> usize {
        self.node.offset
    }
    pub fn end(self) -> usize {
        self.node.end
    }
    /// 자기 필드 (기반 부분은 [`BASE`] 필드 하나로 들어 있다)
    pub fn own_fields(self) -> &'a [Field] {
        &self.node.fields
    }
    /// 기반 클래스 부분
    pub fn base(self) -> Option<Obj<'a>> {
        match own(&self.node.fields, BASE)? {
            v @ Value::Embedded(_) => self.arch.resolve(v),
            _ => None,
        }
    }
    /// 이 클래스이거나 기반 부분에 이 클래스가 있는지
    pub fn is_a(self, class: &str) -> bool {
        let mut o = Some(self);
        while let Some(x) = o {
            if x.class() == class {
                return true;
            }
            o = x.base();
        }
        false
    }
}

impl<'a> Fields<'a> for Obj<'a> {
    fn archive(self) -> &'a Archive {
        self.arch
    }
    fn get(self, name: &str) -> Option<&'a Value> {
        // 기반 부분은 자기보다 먼저 만들어진 노드라 고리가 없다
        let mut o = self;
        loop {
            if let Some(v) = own(&o.node.fields, name) {
                return Some(v);
            }
            o = o.base()?;
        }
    }
}

/// 묶음 보기
#[derive(Clone, Copy)]
pub(crate) struct Group<'a> {
    arch: &'a Archive,
    fields: &'a [Field],
}

impl<'a> Group<'a> {
    pub fn fields(self) -> &'a [Field] {
        self.fields
    }
}

impl<'a> Fields<'a> for Group<'a> {
    fn archive(self) -> &'a Archive {
        self.arch
    }
    fn get(self, name: &str) -> Option<&'a Value> {
        own(self.fields, name)
    }
}

// ---------------------------------------------------------------------------
// 아카이브 읽기
// ---------------------------------------------------------------------------

struct ArchiveReader<'p> {
    r: Rd<'p>,
    classes: Vec<ClassInfo>,
    nodes: Vec<Node>,
    by_id: Vec<usize>,
    back_refs: usize,
    depth: usize,
    max_depth: usize,
    warnings: Vec<String>,
    pads: Vec<usize>,
}

/// `payload[start..start + len]` 의 아카이브. 뿌리는 VtChart 이고 길이 필드 자리에서 끝나야 한다
fn parse_archive(payload: &[u8], start: usize, len: usize) -> Res<Archive> {
    let mut a = ArchiveReader {
        r: Rd::new(payload, start, start.saturating_add(len)),
        classes: Vec::new(),
        nodes: Vec::new(),
        by_id: Vec::new(),
        back_refs: 0,
        depth: 0,
        max_depth: 0,
        warnings: Vec::new(),
        pads: Vec::new(),
    };
    let Some(root) = a.read_ptr(&["VtChart"])? else {
        return Err(VtError {
            offset: start,
            kind: VtErrorKind::NullRoot,
        });
    };
    if a.r.pos != a.r.end {
        return Err(a.r.err(VtErrorKind::ArchiveEnd {
            end: a.r.pos,
            blob_end: a.r.end,
        }));
    }
    let root = a.by_id.get(root.id as usize).copied().unwrap_or(UNFINISHED);
    if root >= a.nodes.len() {
        return Err(VtError {
            offset: start,
            kind: VtErrorKind::NullRoot,
        });
    }
    let mut arch = Archive {
        classes: a.classes,
        nodes: a.nodes,
        by_id: a.by_id,
        root,
        back_refs: a.back_refs,
        max_depth: a.max_depth,
        end: a.r.pos,
        warnings: a.warnings,
        pads: a.pads,
    };
    arch.self_check();
    Ok(arch)
}

impl ArchiveReader<'_> {
    fn warn(&mut self, msg: String) {
        let at = self.r.pos;
        self.warnings.push(format!("{msg} @0x{at:x}"));
    }

    /// 클래스 참조: 번호 u32, 처음이면 이름과 판이 붙는다
    fn read_class(&mut self) -> Res<(usize, &'static ClassDef)> {
        let at = self.r.pos;
        let index = self.r.u32()?;
        let defined = self.classes.len();
        if let Some(c) = self.classes.get(index as usize) {
            return Ok((index as usize, c.def));
        }
        let fail = |kind| VtError { offset: at, kind };
        if index as usize != defined {
            return Err(fail(VtErrorKind::BadClassIndex { index, defined }));
        }
        let len = self.r.u16()? as usize;
        let raw = self.r.take(len)?;
        let name = match raw.split_last() {
            Some((0, name)) if !name.is_empty() => name,
            _ => return Err(fail(VtErrorKind::BadClassName)),
        };
        let def = CLASSES
            .iter()
            .find(|c| c.name.as_bytes() == name)
            .ok_or_else(|| {
                fail(VtErrorKind::UnknownClass(
                    String::from_utf8_lossy(name).into_owned(),
                ))
            })?;
        let version = self.r.u16()?;
        if version != def.version {
            self.warn(format!("{} 판 {version} (표본 {})", def.name, def.version));
        }
        self.classes.push(ClassInfo {
            def,
            version,
            offset: at,
        });
        Ok((defined, def))
    }

    /// 클래스 참조 + 본문. 번호 있는 객체면 `id`, 기반 부분이면 None
    fn read_object(&mut self, id: Option<u32>, at: usize) -> Res<usize> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(VtError {
                offset: at,
                kind: VtErrorKind::TooDeep,
            });
        }
        self.max_depth = self.max_depth.max(self.depth);
        let (class, def) = self.read_class()?;
        let mut fields = Vec::with_capacity(def.specs.len());
        self.read_specs(def.specs, &mut fields)?;
        self.depth -= 1;
        self.nodes.push(Node {
            id,
            class,
            offset: at,
            end: self.r.pos,
            fields,
        });
        Ok(self.nodes.len() - 1)
    }

    fn check_class(&self, node: usize, expect: &'static [&'static str], at: usize) -> Res<()> {
        if expect.is_empty() {
            return Ok(());
        }
        let found = self
            .nodes
            .get(node)
            .and_then(|n| self.classes.get(n.class))
            .map_or("", |c| c.def.name);
        if expect.contains(&found) {
            Ok(())
        } else {
            Err(VtError {
                offset: at,
                kind: VtErrorKind::ClassMismatch {
                    expected: expect,
                    found,
                },
            })
        }
    }

    /// 포인터: 없음, 앞 객체 참조, 또는 새 객체 (번호는 읽은 순서대로 0부터)
    fn read_ptr(&mut self, expect: &'static [&'static str]) -> Res<Option<Ptr>> {
        let at = self.r.pos;
        let id = self.r.u32()?;
        if id == NULL_ID {
            return Ok(None);
        }
        let next = self.by_id.len();
        if let Some(&node) = self.by_id.get(id as usize) {
            if node == UNFINISHED {
                return Err(VtError {
                    offset: at,
                    kind: VtErrorKind::Unfinished(id),
                });
            }
            self.check_class(node, expect, at)?;
            self.back_refs += 1;
            return Ok(Some(Ptr { id, back_ref: true }));
        }
        if id as usize != next {
            return Err(VtError {
                offset: at,
                kind: VtErrorKind::BadObjectId {
                    id,
                    next: next as u32,
                },
            });
        }
        self.by_id.push(UNFINISHED);
        let node = self.read_object(Some(id), at)?;
        if let Some(slot) = self.by_id.get_mut(next) {
            *slot = node;
        }
        self.check_class(node, expect, at)?;
        Ok(Some(Ptr {
            id,
            back_ref: false,
        }))
    }

    fn read_specs(&mut self, specs: &'static [Spec], out: &mut Vec<Field>) -> Res<()> {
        for s in specs {
            self.read_spec(s, out)?;
        }
        Ok(())
    }

    fn read_spec(&mut self, s: &'static Spec, out: &mut Vec<Field>) -> Res<()> {
        let at = self.r.pos;
        let (name, value) = match s {
            S::U8(n) => {
                if *n == "pad" {
                    self.pads.push(self.r.pos);
                }
                (*n, Value::U8(self.r.u8()?))
            }
            S::U16(n) => (*n, Value::U16(self.r.u16()?)),
            S::I16(n) => (*n, Value::I16(self.r.i16()?)),
            S::U32(n) => (*n, Value::U32(self.r.u32()?)),
            S::I32(n) => (*n, Value::I32(self.r.i32()?)),
            S::F32(n) => (*n, Value::F32(self.r.f32()?)),
            S::F64(n) => (*n, Value::F64(self.r.f64()?)),
            S::Color(n) => (*n, Value::Color(self.read_color()?)),
            S::Ptr(n, expect) => (*n, Value::Ptr(self.read_ptr(expect)?)),
            S::Emb(n, class) => {
                let at = self.r.pos;
                let node = self.read_object(None, at)?;
                self.check_class(node, std::slice::from_ref(class), at)?;
                (*n, Value::Embedded(node))
            }
            S::Group(n, def) => {
                let mut fields = Vec::with_capacity(def.specs.len());
                self.read_specs(def.specs, &mut fields)?;
                (*n, Value::Group(fields))
            }
            S::Arr(n, count, item) => {
                let items = self.read_array(n, *count, *item, out)?;
                (*n, Value::Array(items))
            }
            S::When(cond, specs) => {
                if cond.holds(out) {
                    self.read_specs(specs, out)?;
                }
                return Ok(());
            }
            S::VStr(n) => (*n, Value::Str(self.read_vstr()?)),
            S::LBytes(n) => {
                let len = self.r.u32()? as usize;
                (*n, Value::Bytes(self.r.take(len)?.to_vec()))
            }
        };
        out.push(Field { name, value, at });
        Ok(())
    }

    fn read_color(&mut self) -> Res<VtColor> {
        let kind = self.r.u16()?;
        let [r, g, b, flags] = self.r.array::<4>()?;
        if kind != 2 {
            self.warn(format!("VtColor 종류 {kind}"));
        }
        Ok(VtColor {
            kind,
            rgb: [r, g, b],
            flags,
        })
    }

    /// 개수가 남은 바이트로 채울 수 없으면 할당하기 전에 멈춘다
    fn read_array(
        &mut self,
        name: &'static str,
        count: Count,
        item: Item,
        siblings: &[Field],
    ) -> Res<Vec<Value>> {
        let n = count
            .eval(siblings)
            .ok_or_else(|| self.r.err(VtErrorKind::BadCount(name)))?;
        let need = n.saturating_mul(item.min_size());
        if need > self.r.left() as u64 {
            return Err(self.r.err(VtErrorKind::Truncated {
                need: usize::try_from(need).unwrap_or(usize::MAX),
            }));
        }
        let mut items = Vec::with_capacity(n as usize);
        for _ in 0..n {
            items.push(match item {
                Item::U16 => Value::U16(self.r.u16()?),
                Item::Ptr(expect) => Value::Ptr(self.read_ptr(expect)?),
            });
        }
        Ok(items)
    }

    fn read_vstr(&mut self) -> Res<VtStr> {
        let cb = self.r.u32()? as usize;
        let start = self.r.pos;
        let text = vt_string_text(self.r.take(cb)?);
        match &text {
            None => self.warn("VtString 내용의 모양이 다름".into()),
            Some(t) if t.contains('\0') => self.warn("VtString 의 UTF-16 부분에 NUL".into()),
            Some(_) => {}
        }
        Ok(VtStr {
            text,
            raw: start..start + cb,
        })
    }
}

/// `[cp949 글자][00][00][UTF-16LE 글자][00 00]` 의 UTF-16 부분. 모양이 다르면 None.
/// cp949 는 바이트 0 을 쓰지 않으므로 첫 0 이 cp949 부분의 끝이다
fn vt_string_text(raw: &[u8]) -> Option<String> {
    let z = raw.iter().position(|&b| b == 0)?;
    let n = raw.len();
    let shaped = n >= z + 4
        && raw.get(z + 1) == Some(&0)
        && (n - z).is_multiple_of(2)
        && raw[n - 2..] == [0, 0];
    shaped.then(|| crate::text::utf16le(&raw[z + 2..n - 2]))
}

// ---------------------------------------------------------------------------
// 수식 (BIFF8 꼴 RPN, 한컴 폭: 행 u32, 열 u16)
// ---------------------------------------------------------------------------

/// 수식 토큰
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    /// ptgRef3d 0x3A (0x5A, 0x7A): ixti u16, 행 u32, 열 u16
    Ref3d {
        ptg: u8,
        ixti: u16,
        row: u32,
        col: u16,
    },
    /// ptgArea3d 0x3B (0x5B, 0x7B): ixti u16, 행 u32 ×2, 열 u16 ×2
    Area3d {
        ptg: u8,
        ixti: u16,
        rw_first: u32,
        rw_last: u32,
        col_first: u16,
        col_last: u16,
    },
    /// ptgUnion 0x10
    Union,
    /// ptgStr 0x17: cch u16, 플래그 u8 (비트 0 = UTF-16), 글자
    Str { cch: u16, flags: u8, value: String },
    /// ptgParen 0x15
    Paren,
}

/// 수식의 참조 하나 (열은 하위 14비트만)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChartRef {
    pub ixti: u16,
    /// ixti 를 푼 시트 번호 (BOUNDSHEET 순서). 풀 수 없으면 None (글은 `#REF`)
    pub sheet: Option<u16>,
    pub rw_first: u32,
    pub rw_last: u32,
    pub col_first: u16,
    pub col_last: u16,
    /// `시트!$C$2:$C$6`
    pub text: String,
}

/// cce u16 + 토큰
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct ChartFormula {
    /// cce 의 레코드 안 위치
    pub offset: usize,
    pub cce: u16,
    pub tokens: Vec<Token>,
    /// 한셀 XML 꼴 수식 (`=` 없음). 합집합이 있으면 전체를 괄호로 감싼다. 빈 수식은 ""
    pub text: String,
    pub refs: Vec<ChartRef>,
}

/// 계열 하나의 수식
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SeriesFormulas {
    pub values: ChartFormula,
    /// 참조, 글자(ptgStr) 또는 빈 수식
    pub name: ChartFormula,
}

/// 수식의 ixti 를 시트 번호로 푸는 방법. 원본 범위의 ixti 는 곧 시트 번호이고, 항목·계열 수식의 ixti 는
/// EXTERNSHEET 의 XTI 번호다 (차트 수식 ixti 대조 실험: 첫 시트의 차트가 둘째 시트 `Data` 를 가리키는 문서를 한셀이
/// 레거시로 저장하면 원본 범위 ixti 1, 항목·계열 ixti 0, EXTERNSHEET 는 XTI 하나 (내부, 1, 1))
#[derive(Clone, Copy)]
pub(crate) struct Sheets<'a> {
    /// BOUNDSHEET 순서 시트 이름
    pub names: &'a [String],
    /// XTI 번호 → 내부 시트 번호 (내부 문서의 시트 하나가 아니면 None). None 이면 XTI 표를 모르는 것으로 보고
    /// 항목·계열의 ixti 도 시트 번호로 본다 (진단·시험용)
    pub xti: Option<&'a [Option<u16>]>,
}

impl Sheets<'_> {
    /// 원본 범위면 ixti 그대로, 아니면 XTI 표로. 시트 목록 밖이면 None
    fn resolve(&self, ixti: u16, source: bool) -> Option<u16> {
        let sheet = match self.xti {
            Some(t) if !source => t.get(ixti as usize).copied().flatten()?,
            _ => ixti,
        };
        ((sheet as usize) < self.names.len()).then_some(sheet)
    }
}

/// 시트 부분: 푼 시트의 이름, 풀 수 없으면 `#REF`
fn sheet_text(sheet: Option<u16>, sheet_names: &[String]) -> String {
    match sheet.and_then(|s| sheet_names.get(s as usize)) {
        Some(n) => quote_sheet(n),
        None => "#REF".into(),
    }
}

/// `$C$1`: 열 필드 하위 14비트 = 열, 0x4000 열 상대, 0x8000 행 상대 (BIFF8 과 같다고 본다 — 표본은 모두 절대)
fn cell_text(row: u32, col: u16) -> String {
    format!(
        "{}{}{}{}",
        if col & 0x4000 != 0 { "" } else { "$" },
        CellRef::col_name(u32::from(col & 0x3FFF)),
        if col & 0x8000 != 0 { "" } else { "$" },
        u64::from(row) + 1
    )
}

/// cce u16 + 토큰. `source` 는 원본 범위 수식 (ixti 가 시트 번호)
fn read_formula(r: &mut Rd<'_>, sheets: Sheets<'_>, source: bool) -> Res<ChartFormula> {
    let offset = r.pos;
    let cce = r.u16()?;
    let start = r.pos;
    r.take(cce as usize)?;
    let mut f = parse_formula(Rd::new(r.b, start, r.pos), sheets, source)?;
    f.offset = offset;
    f.cce = cce;
    Ok(f)
}

/// 토큰열 → 수식.
/// RPN 에서 피연산자는 원문과 같은 순서로 나온다 — 합집합은 스택 위 이웃한 두 구간을 잇고(사이에 `,`),
/// 괄호는 구간 양끝에 붙는다. 그래서 문자열을 거듭 이어 붙이지 않고 끝에서 한 번에 만든다
/// (괄호가 수만 개 겹쳐도 선형).
fn parse_formula(mut r: Rd<'_>, sheets: Sheets<'_>, source: bool) -> Res<ChartFormula> {
    let sheet_names = sheets.names;
    let mut f = ChartFormula::default();
    // 피연산자마다 (글, 여는 괄호 수, 닫는 괄호 수), 스택은 피연산자 번호 구간
    let mut operands: Vec<(String, u32, u32)> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    while r.left() > 0 {
        let at = r.pos;
        let ptg = r.u8()?;
        let text = match ptg {
            0x3A | 0x5A | 0x7A => {
                let (ixti, row, col) = (r.u16()?, r.u32()?, r.u16()?);
                let sheet = sheets.resolve(ixti, source);
                let text = format!("{}!{}", sheet_text(sheet, sheet_names), cell_text(row, col));
                f.refs.push(ChartRef {
                    ixti,
                    sheet,
                    rw_first: row,
                    rw_last: row,
                    col_first: col & 0x3FFF,
                    col_last: col & 0x3FFF,
                    text: text.clone(),
                });
                f.tokens.push(Token::Ref3d {
                    ptg,
                    ixti,
                    row,
                    col,
                });
                text
            }
            0x3B | 0x5B | 0x7B => {
                let ixti = r.u16()?;
                let sheet = sheets.resolve(ixti, source);
                let (rw_first, rw_last) = (r.u32()?, r.u32()?);
                let (col_first, col_last) = (r.u16()?, r.u16()?);
                let text = format!(
                    "{}!{}:{}",
                    sheet_text(sheet, sheet_names),
                    cell_text(rw_first, col_first),
                    cell_text(rw_last, col_last)
                );
                f.refs.push(ChartRef {
                    ixti,
                    sheet,
                    rw_first,
                    rw_last,
                    col_first: col_first & 0x3FFF,
                    col_last: col_last & 0x3FFF,
                    text: text.clone(),
                });
                f.tokens.push(Token::Area3d {
                    ptg,
                    ixti,
                    rw_first,
                    rw_last,
                    col_first,
                    col_last,
                });
                text
            }
            0x17 => {
                let cch = r.u16()?;
                let flags = r.u8()?;
                let value = if flags & 0x01 != 0 {
                    crate::text::utf16le(r.take(cch as usize * 2)?)
                } else {
                    r.take(cch as usize)?.iter().map(|&b| b as char).collect()
                };
                let text = format!("\"{}\"", value.replace('"', "\"\""));
                f.tokens.push(Token::Str { cch, flags, value });
                text
            }
            0x10 => {
                let (Some(b), Some(a)) = (stack.pop(), stack.pop()) else {
                    return Err(VtError {
                        offset: at,
                        kind: VtErrorKind::BadFormula("합집합의 피연산자가 둘보다 적음"),
                    });
                };
                stack.push((a.0, b.1));
                f.tokens.push(Token::Union);
                continue;
            }
            0x15 => {
                let Some(&(first, last)) = stack.last() else {
                    return Err(VtError {
                        offset: at,
                        kind: VtErrorKind::BadFormula("괄호의 피연산자가 없음"),
                    });
                };
                if let Some(o) = operands.get_mut(first) {
                    o.1 += 1;
                }
                if let Some(o) = operands.get_mut(last) {
                    o.2 += 1;
                }
                f.tokens.push(Token::Paren);
                continue;
            }
            _ => {
                return Err(VtError {
                    offset: at,
                    kind: VtErrorKind::BadToken(ptg),
                });
            }
        };
        stack.push((operands.len(), operands.len()));
        operands.push((text, 0, 0));
    }
    if stack.len() > 1 {
        return Err(r.err(VtErrorKind::BadFormula("피연산자가 남음")));
    }
    // 스택에 하나 남았으면 그 구간이 피연산자 전부다: 이웃마다 합집합 하나 (`,`)
    let union = f.tokens.contains(&Token::Union);
    let mut s = String::new();
    if union {
        s.push('(');
    }
    for (i, (t, open, close)) in operands.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.extend(std::iter::repeat_n('(', *open as usize));
        s.push_str(t);
        s.extend(std::iter::repeat_n(')', *close as usize));
    }
    if union {
        s.push(')');
    }
    f.text = s;
    Ok(f)
}

// ---------------------------------------------------------------------------
// 레코드
// ---------------------------------------------------------------------------

/// 차트 앵커 (OfficeArt ClientAnchor 와 같은 22바이트, 열·행은 0부터)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ChartAnchor {
    /// 늘 0x0606
    pub flags: u16,
    pub col1: u16,
    pub dx1: u16,
    pub row1: u32,
    pub dy1: u16,
    pub col2: u16,
    pub dx2: u16,
    pub row2: u32,
    pub dy2: u16,
}

/// 레코드 머리
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Envelope {
    /// 늘 1
    pub version: u32,
    /// 늘 0x67
    pub kind: u16,
    /// 차트 번호 (만든 파일 0, 서식 파일 1–8; 뜻은 모름)
    pub chart_index: u16,
    pub anchor: ChartAnchor,
    /// 늘 0
    pub reserved: [u8; 11],
    /// 늘 0x11
    pub u16_a: u16,
    /// 아카이브의 레코드 안 범위
    pub blob: Range<usize>,
}

/// 0x7EF0 레코드 하나
#[derive(Debug, Clone)]
pub(crate) struct VtRecord {
    pub envelope: Envelope,
    /// 원본 범위 (차트 데이터 전체; 하나로 말할 수 없으면 빈 수식)
    pub source: ChartFormula,
    pub archive: Archive,
    /// 항목 범위 (없으면 빈 수식)
    pub categories: ChartFormula,
    /// 실제로 쓰는 계열 (아카이브의 계열 목록에는 기본 자리 계열이 더 있다)
    pub series: Vec<SeriesFormulas>,
    /// 레코드 길이 (남김없이 소비했다)
    pub len: usize,
}

impl VtRecord {
    /// 뿌리 VtChart
    pub fn root(&self) -> Obj<'_> {
        self.archive.root()
    }
    /// 번호로 객체
    pub fn object(&self, id: u32) -> Option<Obj<'_>> {
        self.archive.object(id)
    }
}

/// 0x7EF0 논리 레코드(CONTINUE 합침) 하나를 읽는다 — XTI 표를 모를 때 (항목·계열의 ixti 도 시트 번호로 본다:
/// 진단·시험용). 문서를 읽을 때는 [`parse_with`]
pub(crate) fn parse(payload: &[u8], sheet_names: &[String]) -> Result<VtRecord, VtError> {
    parse_with(
        payload,
        Sheets {
            names: sheet_names,
            xti: None,
        },
    )
}

/// 0x7EF0 논리 레코드(CONTINUE 합침) 하나를 읽는다. 수식의 ixti 는 [`Sheets`] 로 푼다
pub(crate) fn parse_with(payload: &[u8], sheets: Sheets<'_>) -> Result<VtRecord, VtError> {
    let mut r = Rd::new(payload, 0, payload.len());
    let version = r.u32()?;
    let kind = r.u16()?;
    let chart_index = r.u16()?;
    let anchor = ChartAnchor {
        flags: r.u16()?,
        col1: r.u16()?,
        dx1: r.u16()?,
        row1: r.u32()?,
        dy1: r.u16()?,
        col2: r.u16()?,
        dx2: r.u16()?,
        row2: r.u32()?,
        dy2: r.u16()?,
    };
    let reserved = r.array::<11>()?;
    let u16_a = r.u16()?;
    if version != 1 || kind != 0x67 {
        return Err(VtError {
            offset: 0,
            kind: VtErrorKind::NotChart { version, kind },
        });
    }
    let source = read_formula(&mut r, sheets, true)?;
    let blob_len = r.u32()?;
    let start = r.pos;
    if blob_len as usize > r.left() {
        return Err(VtError {
            offset: start - 4,
            kind: VtErrorKind::BlobTooLong(blob_len),
        });
    }
    let archive = parse_archive(payload, start, blob_len as usize)?;
    r.take(blob_len as usize)?;
    let categories = read_formula(&mut r, sheets, false)?;
    let n = r.u16()?;
    let mut series = Vec::new();
    for _ in 0..n {
        let values = read_formula(&mut r, sheets, false)?;
        let name = read_formula(&mut r, sheets, false)?;
        series.push(SeriesFormulas { values, name });
    }
    if r.left() > 0 {
        return Err(r.err(VtErrorKind::TrailingBytes(r.left())));
    }
    Ok(VtRecord {
        envelope: Envelope {
            version,
            kind,
            chart_index,
            anchor,
            reserved,
            u16_a,
            blob: start..start + blob_len as usize,
        },
        source,
        archive,
        categories,
        series,
        len: payload.len(),
    })
}

// ---------------------------------------------------------------------------
// 진단: 나무를 줄로
// ---------------------------------------------------------------------------

/// 레코드를 `경로 = 값` 줄로 (리버스 엔지니어링·쓰기 대조용). 새 객체는 `#번호 클래스`, 앞 객체 참조는
/// `@번호`, 기반 부분은 경로에 `:클래스` 를 붙인다
pub(crate) fn dump(rec: &VtRecord) -> String {
    dump_with(rec, false)
}

/// [`dump`] — `offsets` 면 값마다 레코드 안 자리(`@0x…`)를 붙인다 (값을 바꿔 보는 실험용)
pub(crate) fn dump_with(rec: &VtRecord, offsets: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let e = &rec.envelope;
    let a = &e.anchor;
    let _ = writeln!(
        out,
        "envelope = v{} kind {:#x} chart {} anchor {:#06x} ({},{})+({},{}) .. ({},{})+({},{}) reserved {:?} u16_a {:#x}",
        e.version,
        e.kind,
        e.chart_index,
        a.flags,
        a.col1,
        a.row1,
        a.dx1,
        a.dy1,
        a.col2,
        a.row2,
        a.dx2,
        a.dy2,
        e.reserved,
        e.u16_a
    );
    let _ = writeln!(out, "source = {}", rec.source.text);
    let _ = writeln!(out, "categories = {}", rec.categories.text);
    for (i, s) in rec.series.iter().enumerate() {
        let _ = writeln!(out, "series[{i}] = {} | {}", s.values.text, s.name.text);
    }
    let arch = &rec.archive;
    let _ = writeln!(out, "root = #0 VtChart");
    dump_fields(arch, &arch.nodes[arch.root].fields, "", offsets, &mut out);
    out
}

/// 고정 크기 값의 바이트 수 (배열 항목의 자리 셈)
fn scalar_size(v: &Value) -> Option<usize> {
    Some(match v {
        Value::U8(_) => 1,
        Value::U16(_) | Value::I16(_) => 2,
        Value::U32(_) | Value::I32(_) | Value::F32(_) => 4,
        Value::F64(_) => 8,
        Value::Color(_) => 6,
        _ => return None,
    })
}

fn dump_value(arch: &Archive, path: &str, v: &Value, at: Option<usize>, out: &mut String) {
    use std::fmt::Write as _;
    let path = &match (at, scalar_size(v)) {
        (Some(a), Some(_)) => format!("{path} @0x{a:x}"),
        _ => path.to_string(),
    };
    match v {
        Value::U8(x) => {
            let _ = writeln!(out, "{path} = {x}");
        }
        Value::U16(x) => {
            let _ = writeln!(out, "{path} = {x}");
        }
        Value::I16(x) => {
            let _ = writeln!(out, "{path} = {x}");
        }
        Value::U32(x) => {
            let _ = writeln!(out, "{path} = {x}");
        }
        Value::I32(x) => {
            let _ = writeln!(out, "{path} = {x}");
        }
        Value::F32(x) => {
            let _ = writeln!(out, "{path} = {x:?}f");
        }
        Value::F64(x) => {
            let _ = writeln!(out, "{path} = {x:?}d");
        }
        Value::Color(c) => {
            let _ = writeln!(out, "{path} = {}/{:#04x} k{}", c.hex(), c.flags, c.kind);
        }
        Value::Ptr(None) => {
            let _ = writeln!(out, "{path} = null");
        }
        Value::Ptr(Some(p)) if p.back_ref => {
            let _ = writeln!(out, "{path} = @{}", p.id);
        }
        Value::Ptr(Some(p)) => {
            let Some(&ni) = arch.by_id.get(p.id as usize) else {
                return;
            };
            let node = &arch.nodes[ni];
            let class = arch.classes.get(node.class).map_or("?", |c| c.def.name);
            let _ = writeln!(out, "{path} = #{} {class}", p.id);
            dump_fields(arch, &node.fields, path, at.is_some(), out);
        }
        Value::Embedded(ni) => {
            let node = &arch.nodes[*ni];
            let class = arch.classes.get(node.class).map_or("?", |c| c.def.name);
            dump_fields(
                arch,
                &node.fields,
                &format!("{path}:{class}"),
                at.is_some(),
                out,
            );
        }
        Value::Group(fields) => dump_fields(arch, fields, path, at.is_some(), out),
        Value::Array(items) => {
            let _ = writeln!(out, "{path} = [{}]", items.len());
            let mut pos = at;
            for (i, it) in items.iter().enumerate() {
                dump_value(arch, &format!("{path}[{i}]"), it, pos, out);
                pos = pos.zip(scalar_size(it)).map(|(p, n)| p + n);
            }
        }
        Value::Str(s) => {
            let _ = writeln!(out, "{path} = {:?}", s.text.as_deref().unwrap_or("?"));
        }
        Value::Bytes(b) => {
            let _ = writeln!(out, "{path} = bytes {b:02x?}");
        }
    }
}

fn dump_fields(arch: &Archive, fields: &[Field], path: &str, offsets: bool, out: &mut String) {
    for f in fields {
        let p = if f.name == BASE {
            path.to_string()
        } else if path.is_empty() {
            f.name.to_string()
        } else {
            format!("{path}.{}", f.name)
        };
        dump_value(arch, &p, &f.value, offsets.then_some(f.at), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    // -----------------------------------------------------------------------
    // 손으로 만든 레코드
    // -----------------------------------------------------------------------

    /// 리틀 엔디언 쓰기 + 아카이브의 클래스·객체 번호
    #[derive(Default)]
    struct W {
        b: Vec<u8>,
        classes: Vec<&'static str>,
        next_id: u32,
    }

    impl W {
        fn u8(&mut self, v: u8) -> &mut Self {
            self.b.push(v);
            self
        }
        fn u16(&mut self, v: u16) -> &mut Self {
            self.bytes(&v.to_le_bytes())
        }
        fn u32(&mut self, v: u32) -> &mut Self {
            self.bytes(&v.to_le_bytes())
        }
        fn f32(&mut self, v: f32) -> &mut Self {
            self.bytes(&v.to_le_bytes())
        }
        fn f64(&mut self, v: f64) -> &mut Self {
            self.bytes(&v.to_le_bytes())
        }
        fn bytes(&mut self, v: &[u8]) -> &mut Self {
            self.b.extend_from_slice(v);
            self
        }
        fn null(&mut self) -> &mut Self {
            self.u32(NULL_ID)
        }
        /// 클래스 참조 (처음이면 이름과 판을 붙여 정의)
        fn class(&mut self, name: &'static str) -> &mut Self {
            if let Some(i) = self.classes.iter().position(|c| *c == name) {
                return self.u32(i as u32);
            }
            let version = class_def(name).map_or(1, |c| c.version);
            let index = self.classes.len() as u32;
            self.classes.push(name);
            self.u32(index)
                .u16(name.len() as u16 + 1)
                .bytes(name.as_bytes())
                .u8(0)
                .u16(version)
        }
        /// 새 객체: 번호 + 클래스 참조
        fn object(&mut self, class: &'static str) -> u32 {
            let id = self.next_id;
            self.next_id += 1;
            self.u32(id).class(class);
            id
        }
        /// VtString: cp949 자리(아무 바이트) + UTF-16
        fn string(&mut self, ansi: &[u8], text: &str) -> u32 {
            let id = self.object("VtString");
            let mut raw = ansi.to_vec();
            raw.extend([0, 0]);
            raw.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            raw.extend([0, 0]);
            self.u32(raw.len() as u32).bytes(&raw).u8(0x5B);
            self.class("VtValue").class("VtObject");
            id
        }
        fn double(&mut self, v: f64) -> u32 {
            let id = self.object("VtDouble");
            self.f64(v).u16(0xFFFF).class("VtValue").class("VtObject");
            id
        }
    }

    fn area(ixti: u16, r1: u32, r2: u32, c1: u16, c2: u16) -> Vec<u8> {
        let mut w = W::default();
        w.u8(0x3B).u16(ixti).u32(r1).u32(r2).u16(c1).u16(c2);
        w.b
    }

    fn cell(ixti: u16, row: u32, col: u16) -> Vec<u8> {
        let mut w = W::default();
        w.u8(0x3A).u16(ixti).u32(row).u16(col);
        w.b
    }

    /// ptgStr (UTF-16)
    fn lit(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut w = W::default();
        w.u8(0x17).u16(units.len() as u16).u8(1);
        for u in units {
            w.u16(u);
        }
        w.b
    }

    /// cce u16 + 토큰
    fn fmla(tokens: &[u8]) -> Vec<u8> {
        let mut w = W::default();
        w.u16(tokens.len() as u16).bytes(tokens);
        w.b
    }

    fn names() -> Vec<String> {
        vec!["Sheet1".into(), "My Sheet".into()]
    }

    fn formula(tokens: &[u8]) -> Res<ChartFormula> {
        let f = fmla(tokens);
        read_formula(
            &mut Rd::new(&f, 0, f.len()),
            Sheets {
                names: &names(),
                xti: None,
            },
            false,
        )
    }

    /// 원본 범위: C1:C3 ∪ 'My Sheet'!A1:A3
    fn source() -> Vec<u8> {
        [area(0, 0, 2, 2, 2), area(1, 0, 2, 0, 0), vec![0x10]].concat()
    }

    /// 꼬리: 항목 A2:A3, 계열 둘 (값 C2:C3 이름 C1; 값 없음 이름 "이름XY")
    fn tail() -> Vec<u8> {
        let mut t = fmla(&area(0, 1, 2, 0, 0));
        t.extend(2u16.to_le_bytes());
        t.extend(fmla(&area(0, 1, 2, 2, 2)));
        t.extend(fmla(&cell(0, 0, 2)));
        t.extend(fmla(&[]));
        t.extend(fmla(&lit("이름XY")));
        t
    }

    /// 빈 꼬리: 항목 없음, 계열 0
    fn empty_tail() -> Vec<u8> {
        vec![0; 4]
    }

    fn record(source: &[u8], archive: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut w = W::default();
        w.u32(1).u16(0x67).u16(3);
        w.u16(0x0606).u16(6).u16(0).u32(1).u16(0);
        w.u16(13).u16(0).u32(16).u16(0);
        w.bytes(&[0; 11]).u16(0x11);
        w.u16(source.len() as u16).bytes(source);
        w.u32(archive.len() as u32).bytes(archive);
        w.bytes(tail);
        w.b
    }

    /// VtDataGrid (폭, 높이): 칸 = 새 VtString, 그 문자열의 앞 객체 참조, `third` 가 쓴 것
    fn grid_with(w: &mut W, size: (u16, u16), third: impl FnOnce(&mut W)) {
        w.object("VtDataGrid");
        w.class("VtMatrix")
            .class("VtCollection")
            .u16(3)
            .class("VtObject");
        w.u16(size.0).u16(size.1);
        let s = w.string(b"?", "가");
        w.u32(s);
        third(w);
        w.u16(0).u16(1).u16(2).u16(1).u16(1);
        for _ in 0..8 {
            w.u16(0);
        }
    }

    fn grid(w: &mut W) {
        grid_with(w, (1, 3), |w| {
            w.double(1.5);
        });
    }

    fn null(w: &mut W) {
        w.null();
    }

    fn window(w: &mut W) {
        w.class("VtWindow").class("VtObject").u16(9);
    }

    /// VtChart: dataGrid·seriesList·기반 부분만 정하고 나머지 포인터는 없음
    fn chart_with(
        w: &mut W,
        grid: impl FnOnce(&mut W),
        series: impl FnOnce(&mut W),
        base: impl FnOnce(&mut W),
    ) {
        w.object("VtChart");
        grid(w);
        for _ in 0..4 {
            w.null(); // backdrop footnote legend plot
        }
        series(w);
        w.null().null(); // title list
        w.u16(7).u16(1).f32(90.0);
        w.null(); // ssLink
        w.u16(0xFF3F).u16(1).u16(1).u16(1).u16(1);
        w.u16(5).u32(0); // chartType u32_a
        base(w);
    }

    fn chart(w: &mut W, grid: impl FnOnce(&mut W), series: impl FnOnce(&mut W)) {
        chart_with(w, grid, series, window);
    }

    fn sample_archive() -> Vec<u8> {
        let mut w = W::default();
        chart(&mut w, grid, null);
        w.b
    }

    fn sample() -> Vec<u8> {
        record(&source(), &sample_archive(), &tail())
    }

    /// 원본 범위의 ixti 는 시트 번호, 항목·계열의 ixti 는 XTI 번호 (차트 수식 ixti 대조 실험의 꼴: XTI 0 = 둘째 시트)
    #[test]
    fn tail_formulas_resolve_through_xti() {
        let src = [area(1, 0, 2, 2, 2), area(1, 0, 2, 0, 0), vec![0x10]].concat();
        let p = record(&src, &sample_archive(), &tail());
        let xti = [Some(1u16)];
        let sheets = Sheets {
            names: &names(),
            xti: Some(&xti),
        };
        let rec = parse_with(&p, sheets).unwrap();
        assert_eq!(
            rec.source.text,
            "('My Sheet'!$C$1:$C$3,'My Sheet'!$A$1:$A$3)"
        );
        assert_eq!(rec.categories.text, "'My Sheet'!$A$2:$A$3");
        assert_eq!(rec.series[0].values.refs[0].sheet, Some(1));
        assert_eq!(rec.series[0].values.refs[0].ixti, 0);
        // XTI 표 밖, 외부 문서의 XTI 는 #REF
        let none = [None];
        let rec = parse_with(
            &p,
            Sheets {
                names: &names(),
                xti: Some(&none),
            },
        )
        .unwrap();
        assert_eq!(rec.categories.text, "#REF!$A$2:$A$3");
        assert_eq!(rec.categories.refs[0].sheet, None);
        // XTI 표를 모르면 ixti 를 시트 번호로
        assert_eq!(
            parse(&p, &names()).unwrap().categories.text,
            "Sheet1!$A$2:$A$3"
        );
    }

    /// 아카이브만 바꾼 레코드의 오류
    fn archive_err(build: impl FnOnce(&mut W)) -> VtError {
        let mut w = W::default();
        build(&mut w);
        parse(&record(&source(), &w.b, &tail()), &names()).unwrap_err()
    }

    // -----------------------------------------------------------------------
    // 단위 시험
    // -----------------------------------------------------------------------

    #[test]
    fn reads_little_endian_primitives() {
        let mut w = W::default();
        w.u8(0xAB).u16(0x1234).u32(0xDEAD_BEEF).f32(1.5).f64(-2.25);
        w.u16(0xFFFE).u32(0xFFFF_FFFE);
        let b = w.b;
        let mut r = Rd::new(&b, 0, b.len());
        assert_eq!(r.u8().unwrap(), 0xAB);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0xDEAD_BEEF);
        assert_eq!(r.f32().unwrap(), 1.5);
        assert_eq!(r.f64().unwrap(), -2.25);
        assert_eq!(r.i16().unwrap(), -2);
        assert_eq!(r.i32().unwrap(), -2);
        assert_eq!(r.left(), 0);
        let e = r.u8().unwrap_err();
        assert_eq!(
            (e.offset, e.kind),
            (b.len(), VtErrorKind::Truncated { need: 1 })
        );
        // end 가 버퍼 안이면 거기서 멈춘다
        let mut r = Rd::new(&b, 1, 3);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert!(r.u8().is_err());
        assert!(Rd::new(&b, 0, 2).u32().is_err());
        assert!(Rd::new(&b, 5, 2).u8().is_err());
        // VtString 내용: cp949 NUL, 0x00, UTF-16 NUL16
        assert_eq!(vt_string_text(b"AB\0\0A\0B\0\0\0").as_deref(), Some("AB"));
        assert_eq!(vt_string_text(&[0, 0, 0, 0]).as_deref(), Some(""));
        assert_eq!(vt_string_text(b"AB\0\0A\0B\0\0"), None); // 홀수
        assert_eq!(vt_string_text(b"AB"), None); // NUL 없음
        assert_eq!(vt_string_text(b"A\0\x01A\0\0\0"), None); // 둘째 0 이 아님
        assert_eq!(vt_string_text(b"A\0\0A\0\x01\0"), None); // 끝이 NUL16 이 아님
    }

    #[test]
    fn renders_chart_formulas() {
        let f = formula(&[area(0, 0, 5, 2, 2), area(0, 0, 5, 0, 0), vec![0x10]].concat()).unwrap();
        assert_eq!(f.text, "(Sheet1!$C$1:$C$6,Sheet1!$A$1:$A$6)");
        assert_eq!((f.cce, f.offset, f.tokens.len()), (31, 0, 3));
        assert_eq!(
            f.refs[1],
            ChartRef {
                ixti: 0,
                sheet: Some(0),
                rw_first: 0,
                rw_last: 5,
                col_first: 0,
                col_last: 0,
                text: "Sheet1!$A$1:$A$6".into()
            }
        );
        // 상대 비트: 0x4000 열, 0x8000 행 (참조 목록의 열은 14비트)
        assert_eq!(formula(&cell(0, 0, 0x4002)).unwrap().text, "Sheet1!C$1");
        assert_eq!(formula(&cell(0, 0, 0x8002)).unwrap().text, "Sheet1!$C1");
        let rel = formula(&cell(0, 9, 0xC002)).unwrap();
        assert_eq!(
            (rel.text.as_str(), rel.refs[0].col_first),
            ("Sheet1!C10", 2)
        );
        assert_eq!(
            rel.tokens,
            [Token::Ref3d {
                ptg: 0x3A,
                ixti: 0,
                row: 9,
                col: 0xC002
            }]
        );
        assert_eq!(
            formula(&cell(0, u32::MAX, 0)).unwrap().text,
            "Sheet1!$A$4294967296"
        );
        // 따옴표가 필요한 시트 이름, 목록 밖 ixti
        assert_eq!(formula(&cell(1, 0, 0)).unwrap().text, "'My Sheet'!$A$1");
        assert_eq!(formula(&cell(7, 0, 0)).unwrap().text, "#REF!$A$1");
        // 값 클래스 Area3d (0x5B), 배열 클래스 Ref3d (0x7A)
        let mut v = area(0, 1, 5, 1, 1);
        v[0] = 0x5B;
        assert_eq!(formula(&v).unwrap().text, "Sheet1!$B$2:$B$6");
        let mut v = cell(0, 1, 1);
        v[0] = 0x7A;
        assert_eq!(formula(&v).unwrap().text, "Sheet1!$B$2");
        // 글자: UTF-16 과 1바이트, 따옴표 두 번
        assert_eq!(formula(&lit("이름\"XY")).unwrap().text, "\"이름\"\"XY\"");
        assert_eq!(
            formula(&[0x17, 2, 0, 0, b'A', 0xE9]).unwrap().text,
            "\"A\u{e9}\""
        );
        // 합집합과 괄호: 오른쪽으로 쌓인 합집합, 괄호 친 합집합, 겹친 괄호
        let (a, b, c) = (cell(0, 0, 0), cell(0, 0, 1), cell(0, 0, 2));
        let abc = "Sheet1!$A$1,Sheet1!$B$1,Sheet1!$C$1";
        let t = [a.clone(), b.clone(), c.clone(), vec![0x10, 0x10]].concat();
        assert_eq!(formula(&t).unwrap().text, format!("({abc})"));
        let t = [
            a.clone(),
            b.clone(),
            vec![0x10, 0x15],
            c.clone(),
            vec![0x10],
        ]
        .concat();
        assert_eq!(
            formula(&t).unwrap().text,
            "((Sheet1!$A$1,Sheet1!$B$1),Sheet1!$C$1)"
        );
        let t = [a.clone(), vec![0x15], b, c, vec![0x15, 0x10, 0x10]].concat();
        assert_eq!(
            formula(&t).unwrap().text,
            "((Sheet1!$A$1),Sheet1!$B$1,(Sheet1!$C$1))"
        );
        assert_eq!(
            formula(&[a, vec![0x15, 0x15]].concat()).unwrap().text,
            "((Sheet1!$A$1))"
        );
        let empty = formula(&[]).unwrap();
        assert_eq!((empty.text.as_str(), empty.cce), ("", 0));
        // 괄호 수만 개도 선형
        let deep = [cell(0, 0, 0), vec![0x15; 60_000]].concat();
        assert_eq!(
            formula(&deep).unwrap().text.len(),
            "Sheet1!$A$1".len() + 120_000
        );
    }

    #[test]
    fn rejects_malformed_formulas() {
        let kind = |t: &[u8]| formula(t).unwrap_err().kind;
        assert_eq!(kind(&[0x01]), VtErrorKind::BadToken(0x01));
        assert_eq!(kind(&[0x24, 0, 0]), VtErrorKind::BadToken(0x24)); // ptgRef: 차트 수식에는 없다
        let bad = |k| matches!(k, VtErrorKind::BadFormula(_));
        assert!(bad(kind(&[cell(0, 0, 0), vec![0x10]].concat())));
        assert!(bad(kind(&[0x15])));
        assert!(bad(kind(&[0x10])));
        assert!(bad(kind(&[cell(0, 0, 0), cell(0, 0, 1)].concat())));
        let truncated = |k| matches!(k, VtErrorKind::Truncated { .. });
        assert!(truncated(kind(&area(0, 0, 1, 0, 0)[..10])));
        assert!(truncated(kind(&cell(0, 0, 0)[..8])));
        assert!(truncated(kind(&[0x17, 5, 0, 1, b'a'])));
        assert!(truncated(kind(&[0x17, 5])));
        // cce 가 남은 바이트보다 길다
        let short = [9u8, 0, 0x3A];
        let e = read_formula(
            &mut Rd::new(&short, 0, 3),
            Sheets {
                names: &[],
                xti: None,
            },
            false,
        )
        .unwrap_err();
        assert_eq!((e.offset, e.kind), (2, VtErrorKind::Truncated { need: 9 }));
    }

    #[test]
    fn parses_hand_built_chart() {
        let p = sample();
        let rec = parse(&p, &names()).unwrap();
        let e = &rec.envelope;
        assert_eq!(
            (e.version, e.kind, e.chart_index, e.u16_a),
            (1, 0x67, 3, 0x11)
        );
        assert_eq!(
            e.anchor,
            ChartAnchor {
                flags: 0x0606,
                col1: 6,
                dx1: 0,
                row1: 1,
                dy1: 0,
                col2: 13,
                dx2: 0,
                row2: 16,
                dy2: 0
            }
        );
        assert_eq!(rec.source.text, "(Sheet1!$C$1:$C$3,'My Sheet'!$A$1:$A$3)");
        assert_eq!(rec.source.offset, 0x2B);
        assert_eq!(e.blob.start, 0x2B + 2 + source().len() + 4);
        assert_eq!(e.blob.len(), sample_archive().len());

        // 클래스는 처음 쓰일 때 정의되고 그 뒤로는 번호만 온다
        let a = &rec.archive;
        assert_eq!(a.end, e.blob.end);
        let classes: Vec<_> = a.classes.iter().map(|c| (c.def.name, c.version)).collect();
        assert_eq!(
            classes,
            [
                ("VtChart", 6),
                ("VtDataGrid", 1),
                ("VtMatrix", 1),
                ("VtCollection", 1),
                ("VtObject", 1),
                ("VtString", 2),
                ("VtValue", 1),
                ("VtDouble", 1),
                ("VtWindow", 3)
            ]
        );
        // VtChart 정의 (4 + 2 + 8 + 2) 뒤 dataGrid 포인터 (4) 다음
        assert_eq!(a.classes[1].offset, e.blob.start + 4 + 4 + 2 + 8 + 2 + 4);
        // 가장 깊은 곳: VtChart → VtDataGrid → 기반 VtMatrix → 칸 VtString → VtValue → VtObject
        assert_eq!((a.object_count(), a.back_refs, a.max_depth), (4, 1, 6));
        assert!(a.warnings.is_empty(), "{:?}", a.warnings);

        let root = rec.root();
        assert_eq!(
            (root.id(), root.class(), root.version()),
            (Some(0), "VtChart", 6)
        );
        assert_eq!((root.offset(), root.end()), (e.blob.start, e.blob.end));
        assert_eq!(root.u16("chartType"), Some(5));
        assert_eq!(root.u16("mask"), Some(0xFF3F));
        assert_eq!(root.f32("f32_a"), Some(90.0));
        assert_eq!(root.u32("chartType"), None); // 꼴이 다르면 없음
        assert_eq!(root.u16("nothing"), None);
        // 자기 필드가 기반 부분의 같은 이름보다 먼저 (VtWindow.u16_a = 9)
        assert_eq!(root.u16("u16_a"), Some(7));
        let window = root.base().unwrap();
        assert_eq!(
            (window.id(), window.class(), window.u16("u16_a")),
            (None, "VtWindow", Some(9))
        );
        assert!(root.is_a("VtWindow") && root.is_a("VtObject") && !root.is_a("VtValue"));
        assert_eq!(root.get("backdrop"), Some(&Value::Ptr(None)));
        assert!(root.ptr("backdrop").is_none());
        assert!(root.ptr("chartType").is_none());

        // 기반 부분을 따라 찾기: width·height 는 VtMatrix, count 는 VtCollection
        let grid = root.ptr("dataGrid").unwrap();
        assert_eq!((grid.id(), grid.class()), (Some(1), "VtDataGrid"));
        assert_eq!(
            [
                grid.u16("width"),
                grid.u16("height"),
                grid.u16("count"),
                grid.u16("rows")
            ],
            [Some(1), Some(3), Some(3), Some(2)]
        );
        assert_eq!(grid.u16s("reserved"), Some(vec![0; 8]));
        assert_eq!(grid.u16s("cells"), None);
        assert!(grid.is_a("VtCollection"));
        assert!(grid.embedded(BASE).is_some_and(|m| m.class() == "VtMatrix"));

        // 앞 객체 참조: 두 칸이 같은 VtString
        let cells = grid.ptrs("cells").unwrap();
        assert_eq!(cells.len(), 3);
        let s = cells[0].unwrap();
        assert_eq!(
            (s.id(), s.class(), s.string("text")),
            (Some(2), "VtString", Some("가"))
        );
        assert_eq!(s.u8("pad"), Some(0x5B));
        assert_eq!(cells[1].and_then(|c| c.id()), Some(2));
        let raw = grid.array("cells").unwrap();
        assert_eq!(
            raw[0],
            Value::Ptr(Some(Ptr {
                id: 2,
                back_ref: false
            }))
        );
        assert_eq!(
            raw[1],
            Value::Ptr(Some(Ptr {
                id: 2,
                back_ref: true
            }))
        );
        let d = cells[2].unwrap();
        assert_eq!(
            (d.class(), d.f64("value"), d.u16("flags")),
            ("VtDouble", Some(1.5), Some(0xFFFF))
        );
        let Some(Value::Str(vs)) = s.get("text") else {
            panic!("VtString.text");
        };
        assert_eq!(&p[vs.raw.clone()], b"?\0\0\x00\xAC\0\0");

        // 번호로 찾기
        assert_eq!(rec.object(3).map(|o| o.class()), Some("VtDouble"));
        assert!(rec.object(4).is_none());
        let ids: Vec<_> = a.objects().map(|o| o.id()).collect();
        assert_eq!(ids, [Some(0), Some(1), Some(2), Some(3)]);

        // 꼬리
        assert_eq!(rec.categories.text, "Sheet1!$A$2:$A$3");
        assert_eq!(rec.series.len(), 2);
        assert_eq!(rec.series[0].values.text, "Sheet1!$C$2:$C$3");
        assert_eq!(rec.series[0].name.text, "Sheet1!$C$1");
        assert_eq!(
            (rec.series[1].values.cce, rec.series[1].values.text.as_str()),
            (0, "")
        );
        assert_eq!(rec.series[1].name.text, "\"이름XY\"");
        assert_eq!(rec.len, p.len());
    }

    /// 손으로 만든 레코드 → 차트 모델: 캐시의 글은 데이터 표, 형식·빈 칸은 시트의 셀, 값 수식이 없는 계열은 빠진다
    #[test]
    fn hand_built_chart_maps_to_model() {
        use crate::legacy::vtchart_map::{Book, map};
        use crate::model::{Cell, CellValue, ChartTx, Sheet, Styles, Xf};
        let rec = parse(&sample(), &names()).unwrap();
        let cell = |row, col, style, value| Cell {
            r: CellRef { row, col },
            style,
            value,
            formula: None,
        };
        let sheet = Sheet {
            name: "Sheet1".into(),
            cells: vec![
                cell(0, 2, 0, CellValue::Text("가".into())),
                cell(1, 0, 0, CellValue::Number(1.0)),
                cell(1, 2, 1, CellValue::Number(1.5)),
                cell(2, 0, 0, CellValue::Number(2.0)),
            ],
            ..Default::default()
        };
        let styles = Styles {
            cell_xfs: vec![
                Xf::default(),
                Xf {
                    num_fmt_id: 10,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let sheets = [sheet];
        let ch = map(
            &rec,
            &Book {
                sheets: &sheets,
                styles: &styles,
            },
        );
        // 종류 5 = 영역, 그림 영역이 없으면 축도 없다
        assert_eq!((ch.kind.as_str(), ch.three_d), ("area", false));
        assert_eq!(ch.grouping.as_deref(), Some("standard"));
        assert!(ch.axes.is_empty() && ch.legend.is_none() && ch.title_full.is_none());
        assert_eq!(ch.series.len(), 1, "값 수식이 없는 둘째 계열은 빠진다");
        let s = &ch.series[0];
        assert_eq!((s.idx, s.order), (Some(0), Some(0)));
        // 이름: 데이터 표 0행
        let Some(ChartTx::Ref(name)) = &s.tx else {
            panic!("이름 {:?}", s.tx);
        };
        assert_eq!(name.formula.as_deref(), Some("Sheet1!$C$1"));
        assert_eq!(s.name.as_deref(), Some("가"));
        // 항목: 숫자 칸만 → numRef, 이름표 열이 없어 캐시 점은 없다
        let cat = s.cat.as_ref().unwrap();
        let cache = cat.cache.as_ref().unwrap();
        assert_eq!(
            (
                cat.kind.as_str(),
                cache.format_code.as_deref(),
                cache.pt_count
            ),
            ("numRef", Some("G/표준"), Some(2))
        );
        // 값: C2 는 0.00% 형식, C3 은 빈 칸 (빈 값 + General), 묶음 형식은 섞여서 General
        let val = s.val.as_ref().unwrap().cache.as_ref().unwrap();
        let pts: Vec<_> = val
            .points
            .iter()
            .map(|p| (p.idx, p.value.as_str(), p.format_code.as_deref()))
            .collect();
        assert_eq!(pts, [(0, "가", Some("0.00%")), (1, "", Some("General"))]);
        assert_eq!(val.format_code.as_deref(), Some("General"));
        // 한셀이 늘 채우는 값
        assert_eq!(
            (ch.c14_style, ch.style, ch.disp_blanks_as.as_deref()),
            (Some(102), Some(2), Some("gap"))
        );
    }

    /// seriesList 자리에 VtAxis 하나를 담은 VtArray (묶음, 조건부 XYScale, 고정 배열)
    fn axis_list(w: &mut W, xy: u16) {
        w.object("VtArray");
        w.u16(1).u16(1).u32(3).u16(1).class("VtObject");
        w.object("VtAxis");
        w.u16(0).u16(1).null().u16(10).u16(xy);
        for style in [1, 0, 0] {
            w.u16(1).u16(style).u16(0).u16(0);
            w.u16(2).bytes(&[0x80, 0x80, 0x80]).u8(0x20);
            w.f32(20.0).f32(3.0);
        }
        w.f32(60.0).u32(0).null().null();
        w.u16(1).f32(0.0).u16(1).f32(90.0);
        for _ in 0..13 {
            w.u16(0);
        }
        w.f32(1.0);
        if xy == 1 {
            w.f64(0.0).f64(2.5).u16(1).u16(1).u16(1).u8(1).u8(1);
        }
        w.u16(2).u16(4).u16(3);
        for _ in 0..7 {
            w.u16(0);
        }
        w.class("VtObject");
    }

    #[test]
    fn reads_groups_conditions_and_arrays() {
        for xy in [0, 1] {
            let mut w = W::default();
            chart(&mut w, null, |w| axis_list(w, xy));
            let rec = parse(&record(&[], &w.b, &empty_tail()), &[]).unwrap();
            assert_eq!(
                (rec.source.text.as_str(), rec.categories.text.as_str()),
                ("", "")
            );
            assert!(rec.series.is_empty() && rec.archive.warnings.is_empty());
            let list = rec.root().ptr("seriesList").unwrap();
            assert_eq!((list.u16("size"), list.u16("nonNull")), (Some(1), Some(1)));
            let ax = list.ptrs("items").unwrap()[0].unwrap();
            assert_eq!(ax.class(), "VtAxis");
            let pen = ax.group("axisPen").unwrap();
            assert_eq!(
                (pen.u16("style"), pen.f32("width"), pen.fields().len()),
                (Some(1), Some(20.0), 7)
            );
            let c = pen.color("color").unwrap();
            assert_eq!((c.kind, c.hex(), c.flags), (2, "808080".to_string(), 0x20));
            assert_eq!(
                ax.group("minorGridPen").and_then(|g| g.u16("style")),
                Some(0)
            );
            assert_eq!(ax.f32("f32_c"), Some(90.0));
            assert_eq!((ax.u16("u16_r"), ax.u16("tickStyle")), (Some(2), Some(3)));
            assert_eq!(ax.u16s("reserved"), Some(vec![0; 7]));
            let xys = ax.group("xyScale");
            assert_eq!(xys.is_some(), xy == 1);
            if let Some(g) = xys {
                assert_eq!((g.f64("f64_b"), g.u8("u8_b")), (Some(2.5), Some(1)));
            }
            assert!(ax.ptr("bytes").is_none() && ax.get("title").is_some_and(Value::is_null));
            assert!(ax.group("hidden").is_none() && ax.array("axisPen").is_none());
        }
    }

    #[test]
    fn rejects_malformed_archives() {
        use VtErrorKind as K;
        let p = sample();
        let err = |p: &[u8]| parse(p, &names()).unwrap_err();
        let at = |needle: &[u8]| p.windows(needle.len()).position(|w| w == needle).unwrap();
        let grid_name = at(b"VtDataGrid\0");
        let patched = |i: usize, v: u8| {
            let mut q = p.clone();
            q[i] = v;
            err(&q)
        };
        let e = patched(grid_name - 6, 9); // 클래스 번호 1 → 9
        assert_eq!(
            (e.offset, e.kind),
            (
                grid_name - 6,
                K::BadClassIndex {
                    index: 9,
                    defined: 1
                }
            )
        );
        assert_eq!(
            patched(grid_name + 9, b'z').kind,
            K::UnknownClass("VtDataGriz".into())
        );
        assert_eq!(patched(grid_name + 10, b'X').kind, K::BadClassName);
        assert_eq!(
            patched(4, 0x68).kind,
            K::NotChart {
                version: 1,
                kind: 0x68
            }
        );
        assert_eq!(
            patched(0, 2).kind,
            K::NotChart {
                version: 2,
                kind: 0x67
            }
        );
        // 꼬리 뒤 바이트, 아카이브 길이
        assert_eq!(
            err(&[p.clone(), vec![0]].concat()).kind,
            K::TrailingBytes(1)
        );
        let blob_len_at = 0x2B + 2 + source().len();
        let mut q = p.clone();
        q[blob_len_at..blob_len_at + 4].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert_eq!(err(&q).kind, K::BlobTooLong(0x7FFF_FFFF));
        let arch = sample_archive();
        let start = blob_len_at + 4;
        let longer = record(&source(), &[arch.clone(), vec![0]].concat(), &tail());
        assert_eq!(
            err(&longer).kind,
            K::ArchiveEnd {
                end: start + arch.len(),
                blob_end: start + arch.len() + 1
            }
        );
        let shorter = record(
            &source(),
            &arch[..arch.len() - 1],
            &[vec![0], tail()].concat(),
        );
        assert!(matches!(err(&shorter).kind, K::Truncated { .. }));
        // 뿌리
        assert_eq!(
            err(&record(&source(), &[0xFF; 4], &tail())).kind,
            K::NullRoot
        );
        let mut bad_root = arch.clone();
        bad_root[0] = 1;
        assert_eq!(
            err(&record(&source(), &bad_root, &tail())).kind,
            K::BadObjectId { id: 1, next: 0 }
        );
        // 자기 조상을 가리키는 포인터, 건너뛴 번호
        let e = archive_err(|w| {
            chart(
                w,
                |w| {
                    w.u32(0);
                },
                null,
            )
        });
        assert_eq!(e.kind, K::Unfinished(0));
        let e = archive_err(|w| {
            chart(
                w,
                |w| {
                    w.u32(5);
                },
                null,
            )
        });
        assert_eq!(e.kind, K::BadObjectId { id: 5, next: 1 });
        // 클래스 검사: 포인터, 기반 부분, 행렬 칸, 앞 객체 참조
        let e = archive_err(|w| {
            chart(
                w,
                |w| {
                    w.string(b"x", "x");
                },
                null,
            )
        });
        assert_eq!(
            e.kind,
            K::ClassMismatch {
                expected: &["VtDataGrid"],
                found: "VtString"
            }
        );
        let e = archive_err(|w| {
            chart_with(w, grid, null, |w| {
                w.class("VtValue").class("VtObject");
            })
        });
        assert_eq!(
            e.kind,
            K::ClassMismatch {
                expected: &["VtWindow"],
                found: "VtValue"
            }
        );
        let bytes_cell = |w: &mut W| {
            w.object("VtBytes");
            w.u32(0).class("VtValue").class("VtObject");
        };
        assert_eq!(
            archive_err(|w| chart(w, |w| grid_with(w, (1, 3), bytes_cell), null)).kind,
            K::ClassMismatch {
                expected: &["VtString", "VtDouble"],
                found: "VtBytes"
            }
        );
        let e = archive_err(|w| {
            chart(w, grid, |w| {
                w.u32(1); // seriesList → 앞의 VtDataGrid
            })
        });
        assert_eq!(
            e.kind,
            K::ClassMismatch {
                expected: &["VtArray"],
                found: "VtDataGrid"
            }
        );
    }

    #[test]
    fn counts_are_checked_before_allocating() {
        use VtErrorKind as K;
        // 거대한 문자열 길이, 배열 크기, 행렬 폭 × 높이 — 남은 바이트로 채울 수 없으면 할당 전에 실패
        let huge_string = |w: &mut W| {
            w.object("VtString");
            w.u32(0xFFFF_FFF0);
        };
        let e = archive_err(|w| chart(w, |w| grid_with(w, (1, 3), huge_string), null));
        assert_eq!(e.kind, K::Truncated { need: 0xFFFF_FFF0 });
        let e = archive_err(|w| {
            chart(w, grid, |w| {
                w.object("VtArray");
                w.u16(0xFFFF).u16(1).u32(3).u16(0).class("VtObject");
            })
        });
        assert_eq!(e.kind, K::Truncated { need: 0xFFFF * 4 });
        let e = archive_err(|w| chart(w, |w| grid_with(w, (0xFFFF, 0xFFFF), null), null));
        assert_eq!(
            e.kind,
            K::Truncated {
                need: 0xFFFF * 0xFFFF * 4
            }
        );
    }

    /// seriesList 아래로 VtArray 를 `depth` 겹 쌓는다
    fn nested_arrays(w: &mut W, depth: usize) {
        for i in 0..depth {
            let n = u16::from(i + 1 < depth);
            w.object("VtArray");
            w.u16(n).u16(1).u32(3).u16(n).class("VtObject");
        }
    }

    #[test]
    fn nesting_is_bounded() {
        let build = |depth| {
            let mut w = W::default();
            chart(&mut w, grid, |w| nested_arrays(w, depth));
            record(&source(), &w.b, &tail())
        };
        // 뿌리 1 + 배열 40 + 가장 안쪽 배열의 기반 부분 1
        let rec = parse(&build(40), &names()).unwrap();
        assert_eq!(
            (rec.archive.max_depth, rec.archive.object_count()),
            (42, 44)
        );
        let e = parse(&build(5000), &names()).unwrap_err();
        assert_eq!(e.kind, VtErrorKind::TooDeep);
    }

    fn next(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// 무작위로 고친다: 자르기, 그럴듯한 u32 (번호·개수·길이) 넣기, 바이트 바꾸기
    fn mutate(p: &mut Vec<u8>, s: &mut u64) {
        if p.is_empty() {
            return;
        }
        let i = next(s) as usize % p.len();
        match next(s) % 4 {
            0 => p.truncate(i),
            1 => {
                let v =
                    [0, 1, 2, 3, 0x10, 0x7FFF_FFFF, NULL_ID, next(s) as u32][next(s) as usize % 8];
                for (k, b) in v.to_le_bytes().into_iter().enumerate() {
                    if let Some(x) = p.get_mut(i + k) {
                        *x = b;
                    }
                }
            }
            _ => {
                for _ in 0..1 + next(s) % 4 {
                    let j = next(s) as usize % p.len();
                    p[j] = next(s) as u8;
                }
            }
        }
    }

    #[test]
    fn prefixes_and_mutations_never_panic() {
        let p = sample();
        for k in 0..p.len() {
            assert!(parse(&p[..k], &names()).is_err(), "앞 {k}바이트");
        }
        let mut seed = 0x2545_F491_4F6C_DD1D;
        let styles = crate::model::Styles::default();
        let empty = crate::legacy::vtchart_map::Book {
            sheets: &[],
            styles: &styles,
        };
        for _ in 0..20_000 {
            let mut q = p.clone();
            mutate(&mut q, &mut seed);
            if let Ok(rec) = parse(&q, &names()) {
                assert_eq!((rec.len, rec.archive.end), (q.len(), rec.envelope.blob.end));
                let _ = crate::legacy::vtchart_map::map(&rec, &empty);
            }
        }
    }

    /// 고정 크기 묶음의 바이트 수
    fn fixed_size(specs: &[Spec]) -> Option<usize> {
        specs
            .iter()
            .map(|s| match s {
                S::U8(_) => Some(1),
                S::U16(_) | S::I16(_) => Some(2),
                S::U32(_) | S::I32(_) | S::F32(_) => Some(4),
                S::F64(_) => Some(8),
                S::Color(_) => Some(6),
                S::Group(_, d) => fixed_size(d.specs),
                _ => None,
            })
            .sum()
    }

    /// 클래스 이름은 표에 있고, 개수·조건 필드는 같은 목록에서 앞서 읽은 정수다
    fn check_specs(class: &str, specs: &[Spec], ints: &mut Vec<&'static str>) {
        let known = |c: &str| assert!(class_def(c).is_some(), "{class}: 모르는 클래스 {c}");
        let need = |f: &str, ints: &[&str]| {
            assert!(ints.contains(&f), "{class}: {f} 가 앞의 정수 필드가 아님");
        };
        for s in specs {
            match s {
                S::U8(n) | S::U16(n) | S::U32(n) => ints.push(n),
                S::Ptr(_, cls) => cls.iter().for_each(|c| known(c)),
                S::Emb(_, c) => known(c),
                S::Group(_, d) => check_specs(class, d.specs, &mut Vec::new()),
                S::Arr(_, count, item) => {
                    match count {
                        Count::Field(f) => need(f, ints),
                        Count::Product(a, b) => {
                            need(a, ints);
                            need(b, ints);
                        }
                        Count::Fixed(_) => {}
                    }
                    if let Item::Ptr(cls) = item {
                        cls.iter().for_each(|c| known(c));
                    }
                }
                S::When(Cond::Eq(f, _), inner) => {
                    need(f, ints);
                    check_specs(class, inner, ints);
                }
                _ => {}
            }
        }
    }

    #[test]
    fn layout_table_is_consistent() {
        assert_eq!(CLASSES.len(), 41);
        for (i, c) in CLASSES.iter().enumerate() {
            assert!(
                CLASSES[..i].iter().all(|d| d.name != c.name),
                "{} 두 번",
                c.name
            );
            check_specs(c.name, c.specs, &mut Vec::new());
            // VtObject 말고는 기반 부분이 꼭 하나, 따라가면 VtObject 에 닿는다
            let mut name = c.name;
            for _ in 0..CLASSES.len() {
                if name == "VtObject" {
                    break;
                }
                let bases: Vec<_> = class_def(name)
                    .unwrap()
                    .specs
                    .iter()
                    .filter_map(|s| match s {
                        S::Emb(BASE, b) => Some(*b),
                        _ => None,
                    })
                    .collect();
                assert_eq!(bases.len(), 1, "{name} 의 기반 부분");
                name = bases[0];
            }
            assert_eq!(name, "VtObject", "{} 의 기반 사슬", c.name);
        }
        let sizes = [
            &PEN, &BRUSH, &GRADIENT, &SHADOW, &FRAME, &XY_SCALE, &WALL, &FLOOR,
        ]
        .map(|d| (d.name, fixed_size(d.specs)));
        assert_eq!(
            sizes,
            [
                ("Pen", Some(22)),
                ("Brush", Some(16)),
                ("Gradient", Some(14)),
                ("Shadow", Some(26)),
                ("Frame", Some(18)),
                ("XYScale", Some(24)),
                ("Wall", Some(44)),
                ("Floor", Some(44))
            ]
        );
    }

    #[test]
    fn error_messages_name_the_position() {
        let e = archive_err(|w| {
            chart(
                w,
                |w| {
                    w.u32(0);
                },
                null,
            )
        });
        let msg = e.to_string();
        assert!(msg.contains("다 읽지 않은 객체 0"), "{msg}");
        assert!(msg.ends_with(&format!("@0x{:x}", e.offset)), "{msg}");
    }

    // -----------------------------------------------------------------------
    // 말뭉치 (RCELL_CORPUS)
    // -----------------------------------------------------------------------

    /// `/Workbook` 의 논리 레코드: 읽기와 같은 틀 (biff::records 의 확장 길이 + CONTINUE 합침)
    fn logical_records(stream: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut out: Vec<(u16, Vec<u8>)> = Vec::new();
        for rec in crate::legacy::biff::records(stream) {
            if rec.rtype == 0x003C
                && let Some(last) = out.last_mut()
            {
                last.1.extend_from_slice(rec.data);
                continue;
            }
            out.push((rec.rtype, rec.data.to_vec()));
        }
        out
    }

    /// 레거시 파일의 시트 이름(BOUNDSHEET 순서), XTI(EXTERNSHEET), 0x7EF0 레코드
    struct Book {
        sheets: Vec<String>,
        xti: Vec<[u16; 3]>,
        charts: Vec<Vec<u8>>,
    }

    fn read_book(path: &Path) -> Book {
        let bytes = std::fs::read(path).unwrap();
        let stream = crate::container::read_entry(&bytes, "/Workbook").unwrap();
        let mut book = Book {
            sheets: Vec::new(),
            xti: Vec::new(),
            charts: Vec::new(),
        };
        let mut global = true;
        for (t, d) in logical_records(&stream) {
            match t {
                0x000A => global = false,
                // BOUNDSHEET: 위치 u32, 상태 u8, 종류 u8, cch u8, 플래그 u8, UTF-16
                0x0085 if global => {
                    let cch = d.get(6).copied().unwrap_or(0) as usize;
                    let name = d.get(8..8 + cch * 2).unwrap_or(&[]);
                    book.sheets.push(crate::text::utf16le(name));
                }
                // EXTERNSHEET: 개수 u16, (SUPBOOK u16, 첫 시트 u16, 끝 시트 u16) × 개수
                0x0017 if global => {
                    let n = d
                        .get(..2)
                        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
                    for c in d.get(2..).unwrap_or(&[]).as_chunks::<6>().0.iter().take(n) {
                        book.xti
                            .push([0, 2, 4].map(|i| u16::from_le_bytes([c[i], c[i + 1]])));
                    }
                }
                RECORD_TYPE => book.charts.push(d),
                _ => {}
            }
        }
        book
    }

    fn legacy_files(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
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

    fn all_refs(rec: &VtRecord) -> impl Iterator<Item = &ChartRef> {
        let tail = rec
            .series
            .iter()
            .flat_map(|s| s.values.refs.iter().chain(&s.name.refs));
        rec.source
            .refs
            .iter()
            .chain(&rec.categories.refs)
            .chain(tail)
    }

    /// 말뭉치의 모든 레거시 파일에서 0x7EF0 을 읽는다: 아카이브는 길이 필드 자리에서 끝나고 레코드는 남김없이
    /// 소비되어야 한다. 무작위로 고친 레코드도 (오류든 아니든) 멈추지 않아야 한다 — 레코드마다
    /// `RCELL_VT_FUZZ` 번 (기본 200).
    #[test]
    fn corpus_charts_parse_exactly() {
        let Some(root) = std::env::var_os("RCELL_CORPUS").map(PathBuf::from) else {
            eprintln!("RCELL_CORPUS 미설정 — 건너뜀");
            return;
        };
        let rounds: usize = std::env::var("RCELL_VT_FUZZ")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let (mut files, mut with_charts, mut charts, mut objects) = (0, 0, 0, 0);
        let (mut depth, mut warnings, mut back_refs) = (0, 0, 0);
        let (mut refs, mut xti_same) = (0, 0);
        let mut failures = Vec::new();
        let mut seed = 0x9E37_79B9_7F4A_7C15;
        for sub in ["", "craft", "templates", "craft-charts", "craft-charts2"] {
            let dir = root.join(sub);
            if !dir.is_dir() {
                continue;
            }
            for f in legacy_files(&dir) {
                files += 1;
                let book = read_book(&f);
                with_charts += usize::from(!book.charts.is_empty());
                for (k, data) in book.charts.iter().enumerate() {
                    charts += 1;
                    match parse(data, &book.sheets) {
                        Ok(rec) => {
                            assert_eq!(rec.archive.end, rec.envelope.blob.end);
                            assert_eq!(rec.len, data.len());
                            objects += rec.archive.object_count();
                            back_refs += rec.archive.back_refs;
                            depth = depth.max(rec.archive.max_depth);
                            warnings += rec.archive.warnings.len();
                            // ixti 를 EXTERNSHEET 로 풀어도 같은 시트인지 (진단)
                            for r in all_refs(&rec) {
                                refs += 1;
                                let x = book.xti.get(r.ixti as usize);
                                xti_same += usize::from(
                                    x.is_some_and(|x| x[1] == r.ixti && x[2] == r.ixti),
                                );
                            }
                            eprintln!(
                                "OK   {}#{k}  객체 {} 클래스 {} 계열 {}/{} | {}",
                                f.display(),
                                rec.archive.object_count(),
                                rec.archive.classes.len(),
                                rec.series.len(),
                                rec.root()
                                    .ptr("seriesList")
                                    .and_then(|l| l.u16("size"))
                                    .unwrap_or(0),
                                rec.source.text
                            );
                        }
                        Err(e) => failures.push(format!("{}#{k}: {e}", f.display())),
                    }
                    // 고친 레코드가 읽히면 차트 모델로 바꾸는 것도 멈추지 않아야 한다
                    let styles = crate::model::Styles::default();
                    let empty = crate::legacy::vtchart_map::Book {
                        sheets: &[],
                        styles: &styles,
                    };
                    for _ in 0..rounds {
                        let mut q = data.clone();
                        mutate(&mut q, &mut seed);
                        if let Ok(rec) = parse(&q, &book.sheets) {
                            let _ = crate::legacy::vtchart_map::map(&rec, &empty);
                        }
                    }
                }
            }
        }
        eprintln!(
            "레거시 파일 {files} (차트 있음 {with_charts}), 0x7EF0 {charts} — 정확히 읽음 {}, 객체 {objects}, \
             앞 객체 참조 {back_refs}, 최대 중첩 {depth}, 경고 {warnings}; 수식 참조 {refs} 중 XTI 의 시트 = ixti {xti_same}",
            charts - failures.len()
        );
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        if root.join("craft-charts").is_dir() {
            assert!(charts > 0, "차트 레코드를 찾지 못했다");
        }
    }

    // -----------------------------------------------------------------------
    // JS 참조 구현과 대조 (저장소 밖: RCELL_VT_SAMPLES = 표본 .bin 폴더들, RCELL_VT_JSON = JSON 덤프 폴더)
    // -----------------------------------------------------------------------

    type J = serde_json::Value;

    /// JSON 덤프의 번호 있는 객체 (자식 목록과 박힌 객체 안까지)
    fn index_json<'j>(n: &'j J, map: &mut HashMap<u64, &'j J>) {
        if let Some(id) = n["id"].as_u64() {
            map.insert(id, n);
        }
        for c in n["children"].as_array().into_iter().flatten() {
            index_json(c, map);
        }
        for v in n["fields"].as_object().into_iter().flat_map(|m| m.values()) {
            if v.get("cls").is_some() {
                index_json(v, map);
            }
        }
    }

    /// 필드 (기반 부분까지)
    fn jfield<'j>(n: Option<&'j J>, name: &str) -> Option<&'j J> {
        let mut n = n?;
        loop {
            let f = n.get("fields")?;
            if let Some(v) = f.get(name) {
                return Some(v);
            }
            n = f.get(BASE)?;
        }
    }

    fn jptr<'j>(map: &HashMap<u64, &'j J>, n: Option<&'j J>, name: &str) -> Option<&'j J> {
        map.get(&jfield(n, name)?.get("ref")?.as_u64()?).copied()
    }

    fn jitems<'j>(map: &HashMap<u64, &'j J>, n: Option<&'j J>, name: &str) -> Vec<Option<&'j J>> {
        let items = jfield(n, name).and_then(J::as_array);
        let one = |v: &J| map.get(&v.get("ref")?.as_u64()?).copied();
        items
            .map(|a| a.iter().map(one).collect())
            .unwrap_or_default()
    }

    /// VtString 포인터의 글
    fn jtext(map: &HashMap<u64, &J>, n: Option<&J>, name: &str) -> Option<String> {
        let s = jptr(map, n, name)?;
        Some(s["fields"]["text"]["text"].as_str()?.to_string())
    }

    fn show<T: ToString>(v: Option<T>) -> Option<String> {
        v.map(|x| x.to_string())
    }

    fn ju(j: Option<&J>) -> Option<String> {
        show(j?.as_u64())
    }

    fn jf32(j: Option<&J>) -> Option<String> {
        show(j?.as_f64().map(|v| v as f32))
    }

    fn jcolor(j: Option<&J>) -> Option<String> {
        let j = j?;
        Some(format!("{}/{}", j["rgb"].as_str()?, j["flags"].as_u64()?))
    }

    fn color(c: Option<VtColor>) -> Option<String> {
        c.map(|c| format!("{}/{}", c.hex(), c.flags))
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn note(d: &mut Vec<String>, ok: bool, path: &str, ours: impl fmt::Debug, js: &J) {
        if !ok {
            d.push(format!("{path}: rcell {ours:?} / js {js}"));
        }
    }

    /// 객체 하나의 모든 자기 필드 (포인터는 번호만, 박힌 객체는 안까지)
    fn cmp_obj(o: Obj<'_>, j: &J, path: &str, d: &mut Vec<String>) {
        let id_ok = j["id"].as_u64() == o.id().map(u64::from);
        note(d, id_ok, &format!("{path}.id"), o.id(), &j["id"]);
        if j["cls"].as_str() != Some(o.class()) {
            note(d, false, &format!("{path}.cls"), o.class(), &j["cls"]);
            return;
        }
        let pos = [o.version() as usize, o.offset(), o.end()];
        let jpos = [&j["ver"], &j["off"], &j["end"]].map(|v| v.as_u64().map(|x| x as usize));
        note(
            d,
            jpos == pos.map(Some),
            &format!("{path} 판·위치"),
            pos,
            &J::Null,
        );
        let mut ours: Vec<&str> = o.own_fields().iter().map(|f| f.name).collect();
        let mut theirs: Vec<&str> = j["fields"]
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        ours.sort_unstable();
        theirs.sort_unstable();
        let names_ok = ours == theirs;
        note(
            d,
            names_ok,
            &format!("{path} 필드 이름"),
            &ours,
            &J::from(theirs.join(",")),
        );
        for f in o.own_fields() {
            let p = format!("{path}.{}", f.name);
            cmp_val(o.archive(), &f.value, &j["fields"][f.name], &p, d);
        }
    }

    fn cmp_val(arch: &Archive, v: &Value, j: &J, path: &str, d: &mut Vec<String>) {
        let ok = match v {
            Value::U8(x) => j.as_u64() == Some((*x).into()),
            Value::U16(x) => j.as_u64() == Some((*x).into()),
            Value::U32(x) => j.as_u64() == Some((*x).into()),
            Value::I16(x) => j.as_i64() == Some((*x).into()),
            Value::I32(x) => j.as_i64() == Some((*x).into()),
            // JS 의 JSON 은 NaN·무한을 null 로 쓴다
            Value::F32(x) => j
                .as_f64()
                .map_or(j.is_null() && !x.is_finite(), |y| y as f32 == *x),
            Value::F64(x) => j.as_f64().map_or(j.is_null() && !x.is_finite(), |y| {
                (y - x).abs() <= x.abs() * 4.0 * f64::EPSILON
            }),
            Value::Color(c) => {
                j["kind"].as_u64() == Some(c.kind.into())
                    && j["rgb"].as_str() == Some(c.hex().as_str())
                    && j["flags"].as_u64() == Some(c.flags.into())
            }
            Value::Ptr(None) => j.is_null(),
            Value::Ptr(Some(p)) => j["ref"].as_u64() == Some(p.id.into()),
            Value::Embedded(i) => {
                match arch.node_obj(*i) {
                    Some(o) => cmp_obj(o, j, path, d),
                    None => d.push(format!("{path}: 박힌 객체 {i} 없음")),
                }
                return;
            }
            Value::Group(fields) => {
                let n = j.as_object().map_or(usize::MAX, |m| m.len());
                note(
                    d,
                    n == fields.len(),
                    &format!("{path} 묶음 크기"),
                    fields.len(),
                    j,
                );
                for f in fields {
                    let p = format!("{path}.{}", f.name);
                    cmp_val(arch, &f.value, &j[f.name], &p, d);
                }
                return;
            }
            Value::Array(items) => {
                let ja = j.as_array().map_or(&[][..], Vec::as_slice);
                note(
                    d,
                    ja.len() == items.len(),
                    &format!("{path} 길이"),
                    items.len(),
                    j,
                );
                for (i, (x, y)) in items.iter().zip(ja).enumerate() {
                    cmp_val(arch, x, y, &format!("{path}[{i}]"), d);
                }
                return;
            }
            Value::Str(s) => {
                j["cb"].as_u64() == Some(s.raw.len() as u64)
                    && match &s.text {
                        Some(t) => j["text"].as_str() == Some(t.as_str()),
                        None => j.get("text").is_none() && j.get("raw").is_some(),
                    }
            }
            Value::Bytes(b) => j.as_str() == Some(hex(b).as_str()),
        };
        note(d, ok, path, v, j);
    }

    fn cmp_formula(f: &ChartFormula, j: &J, path: &str, d: &mut Vec<String>) {
        note(
            d,
            j["cce"] == f.cce,
            &format!("{path}.cce"),
            f.cce,
            &j["cce"],
        );
        let off_ok = j["off"].as_u64() == Some(f.offset as u64);
        note(d, off_ok, &format!("{path}.off"), f.offset, &j["off"]);
        note(
            d,
            j["text"] == f.text.as_str(),
            &format!("{path}.text"),
            &f.text,
            &j["text"],
        );
        let jt = j["tokens"].as_array().map_or(&[][..], Vec::as_slice);
        let n_ok = jt.len() == f.tokens.len();
        note(
            d,
            n_ok,
            &format!("{path}.tokens"),
            f.tokens.len(),
            &j["tokens"],
        );
        for (i, (t, j)) in f.tokens.iter().zip(jt).enumerate() {
            let ok = match t {
                Token::Ref3d {
                    ptg,
                    ixti,
                    row,
                    col,
                } => {
                    j["name"] == "Ref3d"
                        && j["ptg"] == *ptg
                        && j["ixti"] == *ixti
                        && j["row"] == *row
                        && j["col"] == *col
                }
                Token::Area3d {
                    ptg,
                    ixti,
                    rw_first,
                    rw_last,
                    col_first,
                    col_last,
                } => {
                    j["name"] == "Area3d"
                        && j["ptg"] == *ptg
                        && j["ixti"] == *ixti
                        && j["rwFirst"] == *rw_first
                        && j["rwLast"] == *rw_last
                        && j["colFirst"] == *col_first
                        && j["colLast"] == *col_last
                }
                Token::Union => j["name"] == "Union" && j["ptg"] == 0x10,
                Token::Paren => j["name"] == "Paren" && j["ptg"] == 0x15,
                Token::Str { cch, flags, value } => {
                    j["name"] == "Str"
                        && j["cch"] == *cch
                        && j["flags"] == *flags
                        && j["value"] == value.as_str()
                }
            };
            note(d, ok, &format!("{path}.tokens[{i}]"), t, j);
        }
        let jr = j["refs"].as_array().map_or(&[][..], Vec::as_slice);
        note(
            d,
            jr.len() == f.refs.len(),
            &format!("{path}.refs"),
            f.refs.len(),
            &j["refs"],
        );
        for (i, (r, j)) in f.refs.iter().zip(jr).enumerate() {
            let ok = j["sheet"] == r.ixti
                && j["rwFirst"] == r.rw_first
                && j["rwLast"] == r.rw_last
                && j["colFirst"] == r.col_first
                && j["colLast"] == r.col_last
                && j["text"] == r.text.as_str();
            note(d, ok, &format!("{path}.refs[{i}]"), r, j);
        }
    }

    /// 덤프 전체: 머리, 클래스 목록, 통계, 모든 객체의 모든 필드, 수식
    fn cmp_full(rec: &VtRecord, j: &J) -> Vec<String> {
        let mut d = Vec::new();
        let e = &rec.envelope;
        let je = &j["envelope"];
        let a = &e.anchor;
        let ours = [
            u64::from(e.version),
            e.kind.into(),
            e.chart_index.into(),
            e.u16_a.into(),
            a.flags.into(),
            a.col1.into(),
            a.dx1.into(),
            a.row1.into(),
            a.dy1.into(),
            a.col2.into(),
            a.dx2.into(),
            a.row2.into(),
            a.dy2.into(),
            e.blob.start as u64,
            e.blob.len() as u64,
            rec.len as u64,
            rec.len as u64,
        ];
        let ja = &je["anchor"];
        let theirs = [
            &je["u32_version"],
            &je["u16_kind"],
            &je["chartIndex"],
            &je["u16_a"],
            &ja["flags"],
            &ja["col1"],
            &ja["dx1"],
            &ja["row1"],
            &ja["dy1"],
            &ja["col2"],
            &ja["dx2"],
            &ja["row2"],
            &ja["dy2"],
            &je["blob"]["offset"],
            &je["blob"]["length"],
            &j["length"],
            &j["consumed"],
        ]
        .map(J::as_u64);
        note(&mut d, theirs == ours.map(Some), "머리", ours, je);
        let reserved_ok = je["reserved"] == hex(&e.reserved).as_str();
        note(&mut d, reserved_ok, "머리.reserved", e.reserved, je);
        let arch = &rec.archive;
        let classes: Vec<J> = arch
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| serde_json::json!({"index": i, "name": c.def.name, "version": c.version}))
            .collect();
        let classes_ok = j["classes"] == J::Array(classes);
        note(
            &mut d,
            classes_ok,
            "클래스 목록",
            arch.classes.len(),
            &j["classes"],
        );
        let st = &j["stats"];
        let ours = [
            arch.object_count(),
            arch.classes.len(),
            arch.back_refs,
            arch.end,
        ];
        let theirs = [
            &st["objects"],
            &st["classes"],
            &st["backReferences"],
            &st["archiveEnd"],
        ]
        .map(|v| v.as_u64().map(|x| x as usize));
        note(&mut d, theirs == ours.map(Some), "통계", ours, st);
        let jw = j["warnings"].as_array().map_or(0, Vec::len);
        note(
            &mut d,
            jw == arch.warnings.len(),
            "경고 수",
            &arch.warnings,
            &j["warnings"],
        );
        let mut map = HashMap::new();
        index_json(&j["archive"], &mut map);
        let count_ok = map.len() == arch.object_count();
        note(
            &mut d,
            count_ok,
            "객체 수",
            arch.object_count(),
            &J::from(map.len()),
        );
        for o in arch.objects() {
            let id = o.id().unwrap_or(u32::MAX);
            match map.get(&u64::from(id)) {
                Some(jo) => cmp_obj(o, jo, &format!("#{id}"), &mut d),
                None => d.push(format!("#{id}: JSON 에 없음")),
            }
        }
        cmp_formula(&rec.source, &j["sourceFormula"], "원본", &mut d);
        let jt = &j["tail"];
        cmp_formula(&rec.categories, &jt["categories"], "항목", &mut d);
        let n_ok = jt["seriesCount"] == rec.series.len();
        note(
            &mut d,
            n_ok,
            "계열 수",
            rec.series.len(),
            &jt["seriesCount"],
        );
        for (i, s) in rec.series.iter().enumerate() {
            let js = &jt["series"][i];
            cmp_formula(&s.values, &js["values"], &format!("계열[{i}].값"), &mut d);
            cmp_formula(&s.name, &js["name"], &format!("계열[{i}].이름"), &mut d);
        }
        d
    }

    /// 골라 본 값 (접근 API 로): 차트 종류, mask, 계열 목록, 데이터 표 크기와 칸 값, 축, 제목·각주·범례,
    /// 그림 영역, 꼬리 수식
    fn cmp_targeted(rec: &VtRecord, j: &J) -> (usize, usize, Vec<String>) {
        let mut map = HashMap::new();
        index_json(&j["archive"], &mut map);
        let map = &map;
        // 비교한 수, 그중 rcell 쪽에 값이 있던 수 (둘 다 없음으로 맞는 것과 구별)
        let (mut checks, mut filled) = (0, 0);
        let mut d = Vec::new();
        let mut chk = |what: &str, ours: Option<String>, js: Option<String>| {
            checks += 1;
            filled += usize::from(ours.is_some());
            if ours != js {
                d.push(format!("{what}: rcell {ours:?} / js {js:?}"));
            }
        };
        let root = rec.root();
        let jr = Some(&j["archive"]);
        chk(
            "chartType",
            show(root.u16("chartType")),
            ju(jfield(jr, "chartType")),
        );
        chk("mask", show(root.u16("mask")), ju(jfield(jr, "mask")));

        // 계열 목록
        let list = root.ptr("seriesList");
        let jlist = jptr(map, jr, "seriesList");
        let size = list.and_then(|l| l.u16("size"));
        chk("seriesList.size", show(size), ju(jfield(jlist, "size")));
        let series = list.and_then(|l| l.ptrs("items")).unwrap_or_default();
        let jseries = jitems(map, jlist, "items");
        chk(
            "seriesList.items",
            show(Some(series.len())),
            show(Some(jseries.len())),
        );
        for (i, (o, jo)) in series.iter().zip(&jseries).enumerate() {
            let name = o.and_then(|o| o.string("name")).map(String::from);
            chk(&format!("series[{i}].name"), name, jtext(map, *jo, "name"));
            let fill = o
                .and_then(|o| o.group("brush"))
                .and_then(|g| g.color("fill"));
            let jfill = jfield(*jo, "brush").and_then(|b| b.get("fill"));
            chk(
                &format!("series[{i}].brush.fill"),
                color(fill),
                jcolor(jfill),
            );
            for f in ["index", "showLine", "markerStyle", "labelComponents"] {
                let v = o.and_then(|o| o.u16(f));
                chk(&format!("series[{i}].{f}"), show(v), ju(jfield(*jo, f)));
            }
        }

        // 데이터 표: 크기 (기반 VtMatrix·VtCollection 까지), 칸 값
        let grid = root.ptr("dataGrid");
        let jg = jptr(map, jr, "dataGrid");
        for f in [
            "labelCols",
            "labelRows",
            "rows",
            "cols",
            "u16_a",
            "width",
            "height",
            "count",
        ] {
            let v = grid.and_then(|g| g.u16(f));
            chk(&format!("dataGrid.{f}"), show(v), ju(jfield(jg, f)));
        }
        let cells = grid.and_then(|g| g.ptrs("cells")).unwrap_or_default();
        let jcells = jitems(map, jg, "cells");
        chk(
            "dataGrid.cells",
            show(Some(cells.len())),
            show(Some(jcells.len())),
        );
        for (i, (c, jc)) in cells.iter().zip(&jcells).enumerate() {
            let ours = match c {
                None => Some("null".to_string()),
                Some(o) if o.class() == "VtString" => o.string("text").map(|t| format!("s:{t}")),
                Some(o) => o.f64("value").map(|v| format!("d:{v:.12e}")),
            };
            let js = match jc {
                None => Some("null".to_string()),
                Some(n) if n["cls"] == "VtString" => n["fields"]["text"]["text"]
                    .as_str()
                    .map(|t| format!("s:{t}")),
                Some(n) => n["fields"]["value"].as_f64().map(|v| format!("d:{v:.12e}")),
            };
            chk(&format!("dataGrid.cells[{i}]"), ours, js);
        }

        // 축: X, Z, 표면 축, Y·Y2 배열의 항목
        let plot = root.ptr("plot");
        let jplot = jptr(map, jr, "plot");
        let surface = plot.and_then(|p| p.ptr("surface"));
        let mut axes = vec![
            (
                "axisX".to_string(),
                plot.and_then(|p| p.ptr("axisX")),
                jptr(map, jplot, "axisX"),
            ),
            (
                "axisZ".to_string(),
                plot.and_then(|p| p.ptr("axisZ")),
                jptr(map, jplot, "axisZ"),
            ),
            (
                "surface.axis".to_string(),
                surface.and_then(|s| s.ptr("axis")),
                jptr(map, jptr(map, jplot, "surface"), "axis"),
            ),
        ];
        for list in ["axesY", "axesY2"] {
            let ours = plot
                .and_then(|p| p.ptr(list))
                .and_then(|a| a.ptrs("items"))
                .unwrap_or_default();
            let js = jitems(map, jptr(map, jplot, list), "items");
            chk(
                &format!("{list}.items"),
                show(Some(ours.len())),
                show(Some(js.len())),
            );
            for (i, (o, jo)) in ours.into_iter().zip(js).enumerate() {
                axes.push((format!("{list}[{i}]"), o, jo));
            }
        }
        for (name, ax, jax) in &axes {
            for f in [
                "hidden",
                "u16_a",
                "xy",
                "tickStyle",
                "u16_r",
                "u16_s",
                "u16_k",
                "u16_q",
            ] {
                let v = ax.and_then(|a| a.u16(f));
                chk(&format!("{name}.{f}"), show(v), ju(jfield(*jax, f)));
            }
            for f in ["f32_a", "f32_b", "f32_c", "f32_d"] {
                let v = ax.and_then(|a| a.f32(f));
                chk(&format!("{name}.{f}"), show(v), jf32(jfield(*jax, f)));
            }
            for pen in ["axisPen", "majorGridPen", "minorGridPen"] {
                let g = ax.and_then(|a| a.group(pen));
                let jgp = jfield(*jax, pen);
                chk(
                    &format!("{name}.{pen}.style"),
                    show(g.and_then(|g| g.u16("style"))),
                    ju(jgp.and_then(|g| g.get("style"))),
                );
                chk(
                    &format!("{name}.{pen}.color"),
                    color(g.and_then(|g| g.color("color"))),
                    jcolor(jgp.and_then(|g| g.get("color"))),
                );
            }
            chk(
                &format!("{name}.xyScale"),
                show(ax.map(|a| a.group("xyScale").is_some())),
                show(jax.map(|j| jfield(Some(j), "xyScale").is_some())),
            );
            let title = ax.and_then(|a| a.ptr("title"));
            let jtitle = jptr(map, *jax, "title");
            chk(
                &format!("{name}.title.text"),
                title.and_then(|t| t.string("text")).map(String::from),
                jtext(map, jtitle, "text"),
            );
            chk(
                &format!("{name}.title.font.size"),
                show(
                    title
                        .and_then(|t| t.ptr("font"))
                        .and_then(|f| f.f32("size")),
                ),
                jf32(jfield(jptr(map, jtitle, "font"), "size")),
            );
            chk(
                &format!("{name}.labels.size"),
                show(ax.and_then(|a| a.ptr("labels")).and_then(|l| l.u16("size"))),
                ju(jfield(jptr(map, *jax, "labels"), "size")),
            );
        }

        // 제목·각주 (기반 VtChartText 의 글 상자, VtChartSection 의 보임·위치), 범례
        for name in ["title", "footnote", "legend"] {
            let o = root.ptr(name);
            let jo = jptr(map, jr, name);
            for f in ["visible", "location"] {
                let v = o.and_then(|o| o.u16(f));
                chk(&format!("{name}.{f}"), show(v), ju(jfield(jo, f)));
            }
            let (font, jfont) = if name == "legend" {
                (o.and_then(|o| o.ptr("font")), jptr(map, jo, "font"))
            } else {
                let tb = o.and_then(|o| o.ptr("textBlock"));
                let jtb = jptr(map, jo, "textBlock");
                chk(
                    &format!("{name}.text"),
                    tb.and_then(|t| t.string("text")).map(String::from),
                    jtext(map, jtb, "text"),
                );
                let shown = tb.and_then(|t| t.u16("u16_b"));
                chk(
                    &format!("{name}.u16_b"),
                    show(shown),
                    ju(jfield(jtb, "u16_b")),
                );
                (tb.and_then(|t| t.ptr("font")), jptr(map, jtb, "font"))
            };
            let size = font.and_then(|f| f.f32("size"));
            chk(
                &format!("{name}.font.size"),
                show(size),
                jf32(jfield(jfont, "size")),
            );
            chk(
                &format!("{name}.font.name"),
                font.and_then(|f| f.string("name")).map(String::from),
                jtext(map, jfont, "name"),
            );
        }

        // 그림 영역
        for f in ["overlap", "gap", "gapDepth", "rotation", "elevation"] {
            let v = plot.and_then(|p| p.f32(f));
            chk(&format!("plot.{f}"), show(v), jf32(jfield(jplot, f)));
        }
        for f in ["visible", "location"] {
            let v = plot.and_then(|p| p.u16(f));
            chk(&format!("plot.{f}"), show(v), ju(jfield(jplot, f)));
        }

        // 꼬리 수식
        let text = |j: &J| j["text"].as_str().map(String::from);
        let jt = &j["tail"];
        chk(
            "source",
            Some(rec.source.text.clone()),
            text(&j["sourceFormula"]),
        );
        chk(
            "categories",
            Some(rec.categories.text.clone()),
            text(&jt["categories"]),
        );
        chk(
            "seriesCount",
            show(Some(rec.series.len())),
            ju(jt.get("seriesCount")),
        );
        for (i, s) in rec.series.iter().enumerate() {
            let js = &jt["series"][i];
            let values = Some(s.values.text.clone());
            chk(
                &format!("tail.series[{i}].values"),
                values,
                text(&js["values"]),
            );
            let name = Some(s.name.text.clone());
            chk(&format!("tail.series[{i}].name"), name, text(&js["name"]));
        }
        (checks, filled, d)
    }

    /// 표본 이름 → 말뭉치 레거시 파일과 그 안의 차트 순서 (`이름_1` = 두 번째 차트)
    fn corpus_source(corpus: &Path, name: &str) -> Option<(PathBuf, usize)> {
        let find = |stem: &str| {
            ["templates", "craft-charts", "craft-charts2"]
                .iter()
                .map(|d| corpus.join(d).join(format!("{stem}.legacy.cell")))
                .find(|p| p.is_file())
        };
        if let Some(p) = find(name) {
            return Some((p, 0));
        }
        let (stem, k) = name.rsplit_once('_')?;
        Some((find(stem)?, k.parse().ok()?))
    }

    #[test]
    #[ignore = "저장소 밖 JS 참조 구현의 덤프가 필요하다 (RCELL_VT_SAMPLES, RCELL_VT_JSON, RCELL_CORPUS)"]
    fn matches_js_reference_dumps() {
        let var = |k| std::env::var_os(k);
        let (Some(samples), Some(json_dir), Some(corpus)) = (
            var("RCELL_VT_SAMPLES"),
            var("RCELL_VT_JSON"),
            var("RCELL_CORPUS"),
        ) else {
            eprintln!("RCELL_VT_SAMPLES / RCELL_VT_JSON / RCELL_CORPUS 미설정 — 건너뜀");
            return;
        };
        let (json_dir, corpus) = (PathBuf::from(json_dir), PathBuf::from(corpus));
        let mut bins: Vec<PathBuf> = std::env::split_paths(&samples)
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "bin"))
            .collect();
        bins.sort();
        let mut books: HashMap<PathBuf, Book> = HashMap::new();
        let (mut n, mut same_bytes, mut full_ok, mut targeted_ok) = (0, 0, 0, 0);
        let (mut checks, mut filled, mut fields) = (0, 0, 0);
        let mut failures = Vec::new();
        for bin in &bins {
            n += 1;
            let name = bin.file_stem().unwrap().to_string_lossy().into_owned();
            let data = std::fs::read(bin).unwrap();
            let Some((src, k)) = corpus_source(&corpus, &name) else {
                failures.push(format!("{name}: 말뭉치 파일을 찾지 못함"));
                continue;
            };
            let book = books.entry(src.clone()).or_insert_with(|| read_book(&src));
            if book.charts.get(k) == Some(&data) {
                same_bytes += 1;
            } else {
                failures.push(format!(
                    "{name}: {}#{k} 의 0x7EF0 과 바이트가 다름",
                    src.display()
                ));
            }
            let rec = match parse(&data, &book.sheets) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let text = std::fs::read_to_string(json_dir.join(format!("{name}.json"))).unwrap();
            let j: J = serde_json::from_str(&text).unwrap();
            if n == 1 {
                // 대조 자체의 검사: JSON 의 값 하나를 바꾸면 두 비교 모두 차이를 낸다
                let mut bad = j.clone();
                bad["archive"]["fields"]["chartType"] = J::from(99);
                assert!(!cmp_full(&rec, &bad).is_empty() && !cmp_targeted(&rec, &bad).2.is_empty());
            }
            let full = cmp_full(&rec, &j);
            let (c, f, targeted) = cmp_targeted(&rec, &j);
            checks += c;
            filled += f;
            fields += rec
                .archive
                .nodes
                .iter()
                .map(|n| n.fields.len())
                .sum::<usize>();
            full_ok += usize::from(full.is_empty());
            targeted_ok += usize::from(targeted.is_empty());
            let ok = full.is_empty() && targeted.is_empty();
            eprintln!(
                "{:<4} {name}: 전체 비교 차이 {}, 골라 본 값 {c}개 중 차이 {}",
                if ok { "OK" } else { "DIFF" },
                full.len(),
                targeted.len()
            );
            for x in full.iter().chain(&targeted).take(8) {
                eprintln!("       {x}");
            }
            if !ok {
                failures.push(format!("{name}: 차이 {}", full.len() + targeted.len()));
            }
        }
        eprintln!(
            "표본 {n}: 말뭉치 0x7EF0 과 같은 바이트 {same_bytes}, JS 덤프와 전체(최상위 필드 {fields}개)가 같음 \
             {full_ok}, 골라 본 값 (모두 {checks}개, 값 있음 {filled}) 이 같음 {targeted_ok}"
        );
        assert!(n > 0, "표본 .bin 이 없다");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
