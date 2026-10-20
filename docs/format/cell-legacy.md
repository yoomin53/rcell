<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 한셀 레거시 바이너리 `.cell` 형식

> 작성: yoomin53 · 대상: 한셀 2024(13.0.0.711) 저장 형식 `xlCell8`(-2005), 한셀 2010 이하와 같은 계열
>
> 이 문서는 한셀이 저장한 파일을 차분 분석해 얻은 **관찰 결과**다. 근거 수준 표기:
> **관측**(한셀이 쓴 파일과 한셀의 읽기 결과로 확인), **BIFF8**(공개 명세 [MS-XLS] 와 같음),
> **추정**(맥락으로 짐작). 구현: `src/legacy/`, 검증: `tests/real_samples.rs`.

## 1. 개요

레거시 `.cell` 은 **Excel BIFF8 을 바탕으로 한 한셀 고유 변형**이다. 레코드 틀·대부분의 레코드
배치·수식 토큰은 BIFF8 과 같고, 다음이 다르다.

| 항목 | BIFF8 (.xls) | 한셀 레거시 |
|---|---|---|
| BOF 버전 | 0x0600 | **0x1001** (연도 필드 0x07D8 = 2008) |
| 행 번호 | u16 | **u32** (1,048,576 행) |
| 열 번호 | u16 (256 열) | u16, 수식에서는 14비트 (16,384 열) |
| 문자열 | 1바이트/2바이트 플래그 | 대부분 **플래그와 상관없이 UTF-16** |
| 긴 레코드 | CONTINUE 로 분할 (8224 바이트) | **분할하지 않음**, 확장 길이 필드 |
| 서식 있는 문자열(SST) | cRun 이 문자 앞, 플래그 0x08 | cRun 이 **문자 뒤**, 플래그 **0x01** |
| FONT / XF | 고정 길이 | 뒤에 **한컴 확장 영역** (RGB 색, 장평·자간, 문단 모양 참조) |
| 한컴 전용 레코드 | 없음 | `0xA001` 문단 모양 표, `0xA011` 연결 표, `0xB002` 행 반복, `0xB00E` 넓은 열 정보, `0xA030`, `0xA101`, `0x0900` |

이 문서대로 구현한 rcell 의 해석은, 한셀이 같은 파일을 읽어 XML 로 다시 저장한 결과와
샘플 10종(서식·수식·글꼴·채우기·테두리·정렬·구조·큰 좌표·문자열 3000개 등)과
한셀에 딸린 서식 파일 28종(행 반복·공유 수식·외부 연결·열 서식 포함)에서 모두 일치한다.

## 2. 컨테이너

OLE2 복합 문서([MS-CFB])다 (관측).

| 스트림 | 내용 |
|---|---|
| `/Workbook` | 레코드 스트림 (아래) |
| `/\x05SummaryInformation` | 문서 속성 ([MS-OLEPS] 속성 집합) — 아래 |

`DocumentSummaryInformation` 은 쓰지 않는다 (분류·회사 등은 저장되지 않는다).

**SummaryInformation** (관측, 한셀이 다시 열어 쓴 `docProps/core.xml` 과 대조): 머리의 CLSID 자리에
FMTID_SummaryInformation 을 넣는다. 코드 페이지(PID 1) = 1200, 문자열은 모두 `VT_LPWSTR`(0x1F, `cch u32` + UTF-16,
4바이트 맞춤). PID 2 제목, 3 주제, 4 만든 이, 5 키워드, 6 설명, 8 마지막 수정자, 9 수정 번호, 0x0B 마지막 인쇄,
0x0C 만든 날짜, 0x0D 저장 날짜 (FILETIME, UTC — 한셀 XML 은 시간대 없이 `…T10:49:08.000`), 0x12 프로그램 이름,
그리고 한컴 사용자 PID 0xFD–0xFF (사전 PID 0: 운영 체제, 버전 문자열).

## 3. 레코드 틀

`[타입 u16][길이 u16][데이터]`, 리틀 엔디언 (BIFF8).

**확장 길이 (관측):** 한셀은 긴 레코드를 CONTINUE(0x003C)로 나누지 않는다. 데이터 길이가
0x8000 이상이면 길이 u16 의 최상위 비트를 세우고 u16 을 하나 더 쓴다.

```
길이 = (첫 u16 & 0x7FFF) | (둘째 u16 << 15)        데이터는 6바이트 머리 뒤에서 시작
예: fc 00 5c de 07 00 …  → SST, 길이 0x5E5C | (7 << 15) = 253,532
```

## 4. 문자열 표기

| 위치 | 표기 | 근거 |
|---|---|---|
| SST, BOUNDSHEET, FONT, FORMAT, NAME, STRING, LABEL, NOTE 작성자, CODENAME, STYLE | 길이(u8 또는 u16) + 플래그 u8 + **항상 UTF-16LE** (플래그 0 이어도) | 관측 |
| 수식의 PtgStr | 길이 **u16** + 플래그 u8 (비트 0 = UTF-16, 0 이면 1바이트 문자) | 관측 |

## 5. 통합 문서 전역 레코드

순서: BOF → `0xA030` → WRITEACCESS → CODEPAGE(1200) → CODENAME → 보호 → WINDOW1 → DATEMODE →
`0x0900` → PRECISION → FONT* → FORMAT* → `0xA001`* → `0xA011`* → XF* → STYLE* → TABLESTYLES →
USESELFS → BOUNDSHEET* → `0xA101` → SUPBOOK/EXTERNSHEET/NAME → SST → EOF (관측).

### 5.1 BOF (0x0809, 16바이트)

`vers u16 = 0x1001, dt u16 (0x0005 통합 문서, 0x0010 워크시트), rupBuild u16, rupYear u16 = 0x07D8, …`

### 5.2 BOUNDSHEET (0x0085)

`lbPlyPos u32 (시트 BOF 의 스트림 위치), hsState u8 (0 보임, 1 숨김, 2 매우 숨김), dt u8, cch u8, flag u8, 이름 UTF-16[cch]` (BIFF8 + UTF-16)

### 5.3 FONT (0x0031)

앞부분은 BIFF8 과 같다: `dyHeight u16(1/20pt), grbit u16(0x02 기울임, 0x08 취소선, 0x10 외곽선, 0x20 그림자,
0x40 condense, 0x80 extend),
icv u16, bls u16(400/700), sss u16(1 위첨자, 2 아래첨자), uls u8(1 한 줄, 2 두 줄), bFamily u8,
bCharSet u8, 예약 u8, cch u8, flag u8, 이름 UTF-16[cch]`.
글꼴 번호 4 는 비워 둔다 (BIFF8 규칙과 같음).

이름 뒤에 **확장 플래그 u8 (1) + 확장 78바이트** (관측). 오프셋은 확장 영역 시작 기준:

| 오프셋 | 크기 | 필드 | XML 대응 |
|---|---|---|---|
| 0 | u32 | 크기 (1/100 pt) | `x:sz` |
| 4 | u16 | 플래그: 0x01 양각, 0x02 음각, 0x10 커닝, 0x08 기본 켜짐(미상) | `hs:emboss` `hs:engrave` `hs:useKerning` |
| 6 | u16 | 밑줄 위치 (1 아래, 3 위) | `hs:underlineType` |
| 8 | u16 | 미상 (0) | |
| 10 | u32 | 밑줄 모양 (`ST_UnderlineShape` 번호) | `hs:underlineShape` |
| 14 | u32 | 외곽선 (`ST_OutlineType`) | `hs:outlineType` |
| 18 | u32 | 그림자 (`ST_ShadowType`: 0 없음, 1 비연속, 2 연속) | `hs:shadowType` |
| 22 | i32 | 그림자 X (%) | `hs:shadowOffsetX` |
| 26 | i32 | 그림자 Y (%) | `hs:shadowOffsetY` |
| 30 | u32 | 취소선 종류 | `hs:strikeoutType` |
| 34 | u32 | 강조점 (`ST_DiacsymMark`) | `hs:diacsymMark` |
| 38 | u32 | 취소선 모양 (`ST_UnderlineShape`) | `hs:strikeoutShape` |
| 42 | COLORREF | 글자 색 | `x:color` |
| 46 | COLORREF | 밑줄 색 | `hs:underlineColor` |
| 50 | COLORREF | 음영 색 (0xFFFFFFFF = 없음) | `hs:shadeColor` |
| 54 | COLORREF | 그림자 색 (기본 C0C0C0) | `hs:shadowColor` |
| 58 | i32 | 상대 크기 (%) | `hs:size` |
| 62 | i32 | 장평 (%) | `hs:ratio` |
| 66 | i32 | 자간 (%) | `hs:spacing` |
| 70 | i32 | 글자 위치 (%) | `hs:offset` |
| 74 | COLORREF | 취소선 색 | `hs:strikeoutColor` |

COLORREF 는 `0x00BBGGRR` 이다. `0xFFFFFFFF` = 지정 안 함, `0xDFFFFFFF` = 자동 (관측).

한셀은 외곽선 종류(오프셋 14)가 있으면 실선이 아니어도 ECMA 외곽선(`x:outline`)을 켠다. grbit 0x10 은 실선일 때만
켜져 있으므로 둘 중 하나라도 있으면 외곽선이다 (글꼴 대조 실험). 그림자 종류(오프셋 18)는 `x:shadow` 를 켜지 않는다.

### 5.4 FORMAT (0x041E)

`ifmt u16, cch u16, flag u8, UTF-16[cch]`. 한국어 로캘 내장 형식(5–8 ₩ 통화, 23–26 $ 통화,
41–44 회계)도 FORMAT 레코드로 기록된다 (관측) — 내장 번호 표의 근거로 쓴다 (`src/numfmt.rs`).

### 5.5 XF (0x00E0, 54 또는 58바이트)

처음 20바이트는 **BIFF8 XF 와 같다** (글꼴·표시 형식 번호, 잠금/숨김/스타일 비트와 부모,
가로·세로 정렬과 줄 바꿈, 회전, 들여쓰기·축소·읽기 순서, 적용 비트, 테두리 스타일과 팔레트 색,
채우기 무늬, 무늬 색). 한셀의 기본 세로 정렬은 가운데(`alcV = 1`)다. 보호 칸의 0x0008(f123Prefix)이
XML 의 `quotePrefix` 다. 그라데이션 채우기는 XFEXT(0x087D, 5.8절)에 있다.

그 뒤에 **한컴 확장** (관측):

```
n u16                   (2 또는 3)
[para_ref u32]          n == 3 일 때만: 0xA011 표의 번호 → 문단 모양
COLORREF × 8            [0] 무늬 배경, [1] 채우기(전경), [2] 위, [3] 왼쪽, [4] 아래, [5] 오른쪽, [6] 대각선, [7] 미상
```

BIFF 처럼 스타일 XF(대개 15개) 다음에 셀 XF 가 온다. 셀은 XF 의 절대 번호를 가리킨다.

### 5.6 문단 모양 표 0xA001 (37 또는 41바이트) — 한컴 전용

| 오프셋 | 크기 | 필드 | 비고 |
|---|---|---|---|
| 0 | i32 | 왼쪽 여백 | 들여쓰기 1단계 = 1134 |
| 4 | i32 | 오른쪽 여백 | |
| 8 | i32 | 첫 줄 들여쓰기(+)/내어쓰기(−) | |
| 12 | i32 | 문단 위 간격 | |
| 16 | i32 | 문단 아래 간격 | |
| 20 | i32 | 줄 간격 값 | 기본 75 |
| 24 | u8 | 줄 간격 종류 (0 percent, 1 fixed, 2 betweenLines) | `atLeast` 는 2 로 저장된다 |
| 25 | u8 | 정렬 (한글 문단 번호: 1 왼쪽, 2 오른쪽, 3 가운데) | XF 의 정렬이 우선 |
| 26 | u8 | 영어 단어 나눔 (0 keepWord, 2 breakWord) | |
| 27 | u8 | 한글 단어 나눔 (0 keepWord, 1 breakWord) | |
| 28 | i32 | 최소 공백 (%) | |
| 32 | u16 | 세로 기준 (0 baseLine, 1 top, 2 center, 3 bottom) | |
| 34 | u8 | 미상 | |
| 35 | u8 | 문단 번호 종류 | 0 이 아니면 41바이트 |
| 36 | u8 | 글꼴에 어울리는 줄 높이 | |
| 37 | i32 | 문단 번호 ID | 41바이트 형식만 |

0번 항목은 기본 문단 모양이다. **0xA011** (8바이트: `u32 종류(1), u32 0xA001 번호`) 목록이
XF 의 `para_ref` 를 0xA001 번호로 잇는다 (관측: para_ref 0,1,2 → 0xA001 1,2,3).

한셀은 XF 의 BIFF 들여쓰기 칸(cIndent)을 쓰지 않고, 왼쪽(오른쪽 정렬이면 오른쪽) 여백 ÷ 1134 로
들여쓰기 단계를 계산한다 (관측).

### 5.7 SST (0x00FC)

`cstTotal u32, cstUnique u32`, 그 뒤 문자열마다 (관측, BIFF8 과 다름):

```
cch u16, flags u8, UTF-16 문자[cch],
flags & 0x01 이면: cRun u16, (ich u16, ifnt u16) × cRun
```

문자열 순서는 처음 나온 순서가 아니다 (해시 순서로 추정, XML 형식과 같다).

### 5.8 그 밖의 전역 레코드

| 레코드 | 내용 | 근거 |
|---|---|---|
| 0xA030 (4) | `01 00 00 00` | 관측, 의미 미상 |
| 0x0900 (2) | `01 00` | 관측, 의미 미상 |
| 0xA101 (4) | u32 = XML `calcPr/@hs:hclCalcId` (관측 904, 계산 엔진 번호로 추정). 1 은 값이 없는 것으로 본다 | 관측 |
| 0xA020 (가변) | 문단 번호 정의 하나: `종류 u8, 시작 번호 u16, cch u16, 플래그 u8, 번호 모양 UTF-16` (`^1.`) → XML `x:abstractNums/x:abstractNum`. `abstractNumType` = 종류 + 1 (관측 0 → 1), `lvlText` 는 `^` 를 `%5e` 로 쓴다. 한셀은 저장할 때마다 문단 모양에서 이 목록을 새로 만든다 (번호 하나) | 관측 |
| EXTERNNAME (0x0023) | 바로 앞 SUPBOOK 의 이름 하나: `옵션 u16, 예약 u32, cch u16, UTF-16` (BIFF8 과 달리 플래그 바이트 없음). 추가 기능 SUPBOOK(`01 00 01 3a`) 아래면 함수 이름 — PtgNameX 가 1부터 가리킨다 (7절) | 관측 |
| STYLE (0x0293) | BIFF8 과 같다. 한셀은 BIFF 상용구(내장 3–7 쉼표·통화·백분율 → XF 16–20)를 늘 쓴다. 셀 서식이 쓰지 않는 스타일은 다시 저장할 때 사라진다 (XML 에는 쓰이는 스타일과 표준 스타일만 쓴다) | 관측 |
| TABLESTYLES (0x088E) | FRT 머리 12, `cts u32` (기본 제공 144 + 사용자 정의 수), 기본 표·피벗 스타일 이름 길이 `u16 × 2`, 이름 두 개 (UTF-16, 플래그 바이트 없음) | [MS-XLS] + 관측 |
| TABLESTYLE (0x088F) | FRT 머리 12, `grbit u16` (0x02 피벗, 0x04 표), `ctse u32`, `cchName u16`, 이름 | [MS-XLS] + 관측 |
| TABLESTYLEELEMENT (0x0890) | FRT 머리 12, `tseType u32` (ST_TableStyleType 순서: 0 wholeTable, 1 headerRow … 26 pageFieldLabels, 27 pageFieldValues), `size u32` (줄무늬 폭), `index u32` (DXF 0x088D 순서) | [MS-XLS] + 관측 |
| SUPBOOK (0x01AE) | `ctab u16, cch u16` — `cch` 가 `0x0401` 이면 이 통합 문서, `0x3A01` 이면 추가 기능, 그 밖이면 외부 문서: `flag u8, 가상 경로 UTF-16[cch], (cch u16, flag u8, 시트 이름 UTF-16) × ctab` | BIFF8 + 관측 |
| EXTERNSHEET (0x0017) | BIFF8 과 같다: `n u16, (iSupBook u16, itabFirst u16, itabLast u16) × n` | BIFF8 |
| NAME (0x0018) | BIFF8 Lbl + 이름 UTF-16. 수식이 없는 NAME 은 정의되지 않은 이름(`=XYZ`)이나 `_xlfn.` 자리 | 관측 |
| XFEXT (0x087D) | FRT 머리 12, `예약 u16, ixfe u16, 예약 u16, cexts u16`, ExtProp(`종류 u16, cb u16 (머리 포함), 값`) × cexts. 한셀은 그라데이션 채우기(종류 6)만 쓴다: `종류 u32 (0 직선, 1 경로), 각도·왼쪽·오른쪽·위·아래 f64, 정지점 수 u32, (색 종류 u16 — 2 RGB, 값 u32, 위치 f64, 밝기 f64) × n`. 다른 색은 XF 의 한컴 확장에 있다 | [MS-XLS] + 관측 (그라데이션 대조 실험) |

## 6. 워크시트 레코드

| 레코드 | 배치 | 근거 |
|---|---|---|
| DIMENSIONS (14) | `rwMic u32, rwMac u32, colMic u16, colMac u16, 예약 u16` | 관측 |
| ROW (18) | `rw u32, colMic u16, colMac u16, miyRw u16(twip), 예약 u16, 미사용 u16, flags u32` (0x20 숨김, 0x40 높이 지정, 0x80 서식, 비트 16–27 XF) | 관측 |
| 셀 머리 (8) | `rw u32, col u16, ixfe u16` | 관측 |
| LABELSST (12) | 머리 + `isst u32` | 관측 |
| NUMBER (16) | 머리 + f64 | 관측 |
| RK (12) | 머리 + RK u32 | BIFF8 RK |
| BOOLERR (10) | 머리 + 값 u8 + 오류 여부 u8 | 관측 |
| FORMULA | 머리 + 결과 8 + grbit u16 + chn u32 + cce u16 + 토큰 | 관측 |
| STRING (0x0207) | `cch u16, flag u8, UTF-16` — 직전 수식의 문자열 결과 | 관측 |
| COLINFO (12) | `colFirst u16, colLast u16, coldx u16 (1/256 문자), ixfe u16 (열 기본 서식), grbit u16, 예약 u16` | BIFF8 |
| **0xB00E** (12) | COLINFO 와 같은 배치, 257번째 열(0x0100)부터 16384번째 열까지 — 한컴 전용 | 관측 |
| **0xB002** (8) | **행 반복**: `rwFirst u32, rwLast u32`. `rwFirst` 행(ROW 속성과 셀 전부)을 `rwFirst+1 ..= rwLast` 행에 그대로 되풀이한다. 반복된 행의 수식은 행 상대 참조가 옮긴 행 수만큼 움직인다. 같은 행이 이어지는 서식 파일에서 자주 쓴다 | 관측 (한셀 해석과 대조) |
| DEFAULTROWHEIGHT, DEFCOLWIDTH | BIFF8 과 같다 | BIFF8 |
| MERGECELLS | `n u16`, `(rwFirst u32, rwLast u32, colFirst u16, colLast u16) × n` | 관측 |
| HLINK | 범위(행 u32) + [MS-OSHARED] 하이퍼링크 객체 | 관측 |
| WINDOW2 (22) | 아래 6.1절 | 관측 |
| OBJ (0x005D) | 하위 레코드 목록. 첫 `ftCmo`: `ft u16 (0x15), cb u16, ot u16, id u16, grbit u16, …` — `ot` 0x19 = 메모 | BIFF8 |
| TXO (0x01B6) | **한컴 변형**: 머리 10바이트 + `cch u16, flag u8, UTF-16[cch]` (CONTINUE 없이 바로) + `(ich u16, ifnt u16)` 런 — 바로 앞 OBJ 의 내용 | 관측 |
| NOTE (0x001C) | `rw u32, col u16, grbit u16 (0x0002 항상 표시), idObj u16, 작성자 (cch u16, flag u8, UTF-16)` — 시트 끝에 모여 있고 `idObj` 로 OBJ/TXO 를 가리킨다 | 관측 |
| MSODRAWING (0x00EC) | OfficeArt(Escher). 메모·도형의 위치 `OfficeArtClientAnchor`(0xF010)는 **22바이트**: `flag u16, col1 u16, dx1 u16, rw1 u32, dy1 u16, col2 u16, dx2 u16, rw2 u32, dy2 u16` (행 u32) | 관측 |
| SHEETEXT (0x0862) | 탭 색: FRT 머리 12 + `cb u32` + `icvPlain u32`(팔레트) + **COLORREF**(한컴, 실제 RGB) | BIFF8 FRT + 관측 |
| CODENAME (0x01BA) | `cch u16, flag u8, UTF-16` → `sheetPr/@codeName` | 관측 |

### 6.1 보기·인쇄 레코드 (한셀 해석과 대조)

| 레코드 | 배치 | XML |
|---|---|---|
| WINDOW2 (22) | `grbit u16, rwTop u32, colLeft u16, icvHdr u32, 예약 u16, 확대율 u16 (14), 확대율 u16 (16), …` — grbit: 0x01 수식 표시, 0x02 눈금선, 0x04 머리글, 0x08 고정, 0x10 0 표시, 0x100 고정(나누기 없음), 0x200 선택, 0x800 페이지 나누기 미리 보기 | 한셀은 (14) 를 `zoomScaleNormal`, (16) 을 `zoomScaleSheetLayoutView` 로 읽는다 (BIFF8 과 반대 순서) |
| SCL (0x00A0) | `num u16, den u16` | `zoomScale` |
| PANE (0x0041, 14) | `x u16, y u32, rwTop u32, colLeft u16, pnnAct u16` — 고정이면 x·y 는 열·행 수, 나누기면 1/20 pt | `pane`. 상태는 WINDOW2 의 0x08·0x100 |
| SELECTION (0x001D) | `pnn u8, rwAct u32, colAct u16, irefAct u16, cref u16, (rwFirst u32, rwLast u32, colFirst u16, colLast u16) × cref` — 창마다 하나 | 활성 창(pnnAct)의 것 |
| WSBOOL (0x0081) | 0x0001 자동 페이지 나누기, 0x0100 페이지에 맞추기 | `pageSetUpPr` |
| PRINTHEADERS / PRINTGRIDLINES / HCENTER / VCENTER | `u16` | `printOptions` |
| LEFT/RIGHT/TOP/BOTTOMMARGIN (0x26–0x29) | `f64` 인치 | `pageMargins` |
| SETUP (0x00A1, 34) | BIFF8 과 같다: `paper, scale, pageStart, fitW, fitH, grbit, res, vres u16, 머리글 여백 f64, 바닥글 여백 f64, copies u16`. grbit: 0x01 오른쪽 먼저, 0x02 세로, 0x04 프린터 설정 없음, 0x08 흑백, 0x40 방향 없음, 0x80 첫 페이지 번호 지정 | `pageSetup` |
| HEADER / FOOTER (0x14/0x15) | 비면 길이 0, 아니면 `cch u16, flag u8, UTF-16` | `oddHeader`/`oddFooter` |
| HeaderFooter FRT (0x089C) | FRT 머리 12 + GUID 16 + `flags u16` (0x01 홀짝 다름, 0x02 첫 페이지 다름, 0x04 문서와 함께 크기 조정, 0x08 여백에 맞춤) + `cch u16 ×4` + 문자열 4개(`cch u16, flag u8, UTF-16`: 짝수 머리글·바닥글, 첫 머리글·바닥글) | `headerFooter` |
| HORIZONTALPAGEBREAKS (0x001B) | `n u16, (rw u32, colStart u16, colEnd u16) × n` | `rowBreaks` |
| VERTICALPAGEBREAKS (0x001A) | `n u16, (col u16, rwStart u16, rwEnd u32) × n` (8바이트, 관측) | `colBreaks` |

한셀이 레거시를 읽을 때의 변환 (관측):

- **여백**: 인치 값을 HWPUNIT(1/7200 인치) 정수로 **잘라서** 들이고 f32 로 다시 쓴다
  (머리글 여백 0.51180553 → 3684 → 0.51166666). 저장할 때도 같은 방식으로 잘라 쓴다.
- **머리글·바닥글 글꼴 코드**: 레거시에는 `&"맑은 고딕, REGULAR "` 처럼 모양이 대문자 + 공백으로
  기록되고, 한셀은 `&"맑은 고딕,Regular"` 로 읽는다. 저장할 때는 글꼴 코드가 없는 구역에 기본 글꼴 코드를 넣는다.
- **`printOptions`**: 켠 항목이 하나도 없으면 XML 에 쓰지 않는다.
- 나눈 창의 활성 창이 `bottomLeft` 면 저장할 때 `topLeft`(3)로 바뀐다. `draft` 와
  `autoPageBreaks="0"` 은 저장되지 않는다.

## 6.2 그리기 (OfficeArt) — 한셀 해석과 대조

시트의 MSODRAWING(0x00EC) 내용을 **이어 붙이면** [MS-ODRAW] OfficeArt 스트림 하나가 된다:
`DgContainer(F002) → FDG(F008) + SpgrContainer(F003) → SpContainer(F004)…`. 첫 SpContainer 는 그룹 전체(patriarch,
FSP 플래그 0x0005)이고, 나머지 도형마다 SpContainer 하나 뒤에 OBJ 레코드(+ 글이 있으면 ClientTextbox F00D 와 TXO)가
온다. **OBJ 를 문서 순서대로 도형에 하나씩 짝짓는다** (메모 도형도 포함, `ot` 0x19 는 메모, 0x1E 는 도형, 0x05 차트).

| 레코드 | 내용 |
|---|---|
| FSP (F00A) | `inst` = 모양 종류(MSOSPT: 0 사용자 정의, 1 사각형, 202 글상자 …), `spid u32`, `flags u32` (0x1 그룹, 0x4 patriarch) |
| OPT (F00B) | `inst` = 속성 수. `(pid u16 (0x8000 = 복합), 값 u32)` × n 뒤에 복합 데이터 |
| ClientAnchor (F010) | **한컴 22바이트**: `flag u16, col1 u16, dx1 u16, rw1 u32, dy1 u16, col2 u16, dx2 u16, rw2 u32, dy2 u16` |

쓰는 속성: `0x0081`–`0x0084` 글 여백(EMU), `0x0085` 줄 바꿈(2 = 안 함), `0x0087` 세로 맞춤(1·4 가운데, 2·5 아래),
`0x0181` 채우기 색(COLORREF), `0x01BF` 채우기 켬(0x10, 사용 0x100000), `0x01C0` 선 색, `0x01CB` 선 두께(EMU),
`0x01CE` 점선, `0x01FF` 선 켬(0x08, 사용 0x80000), `0x0380` 이름(UTF-16), `0x0381` 설명(UTF-16),
`0x03BF` 숨김(0x2, 사용 0x20000), `0x0104` 그림 번호(pib, 그림 저장소의 1부터 번호).

**그림** (`ot` 0x08, 모양 75 = 그림 틀): 그림 파일은 전역의 MSODRAWINGGROUP(0x00EB) 에 있다 —
`DggContainer(F000) → FDGG(F006) + BStoreContainer(F001, inst = 개수) → FBSE(F007)…`.
FBSE: `btWin32 u8, btMacOS u8, rgbUid[16], tag u16, size u32, cRef u32 (0 = 빈 자리), foDelay u32, 예약 u8, cbName u8, 예약 u8 ×2`,
이름, 그리고 **그림 레코드가 그 안에 바로 들어 있다**: F01D/F02A JPEG, F01E PNG, F01F DIB, F029 TIFF
(`rgbUid` — inst 의 낮은 비트가 1 이면 하나 더 — `tag u8`, 파일), F01A EMF, F01B WMF, F01C PICT
(`rgbUid`, OfficeArtMetafileHeader 34바이트: `cbSize, rcBounds, ptSize, cbSave, 압축 u8 (0 = zlib, 0xFE 없음), filter`, 파일).
DIB 는 BITMAPFILEHEADER 를 붙여 BMP 로 꺼낸다. 한셀은 그림을 저장할 때마다 PNG 를 다시 압축하고 JPEG 도 다시
쓰므로(크기·형식은 같다) 비교는 형식과 픽셀 크기로 한다.

자동 필터 단추는 콤보 상자(`ot` 0x14) 도형으로 저장된다 — 그리기 개체가 아니다 (6.4절).

**앵커 단위** (서식 파일 390개 오프셋과 대조):

- `flag`: 0 → `twoCellAnchor`, 2 → `editAs="oneCell"`, 3 → `editAs="absolute"`.
- `dx` 는 열의 **(픽셀 너비 − 1)** 의 1/32768, `dy` 는 행의 **(픽셀 높이 − 1)** 의 1/8192 이다.
  열 픽셀은 Excel 식 `trunc(((trunc(256·w) + trunc(128/MDW)) / 256) · MDW)` (MDW = 기본 글꼴 숫자 폭),
  행 픽셀은 `round(pt × 4/3)`. 오프셋 = `round(d × 실효 픽셀 / 단위)` 픽셀 × 9525 EMU.
- 한셀은 그리기 위치를 내부에서 절대 좌표로 들고 있다가 앵커를 다시 계산하므로 XML 에 1 EMU 씩 어긋난 값
  (85724 = 9px − 1 등)이 나오기도 한다. rcell 은 픽셀 단위로 비교한다.
- ROW 레코드의 높이는 지정(0x40)이 아니어도 그 행의 높이다 (앵커 계산에 쓰인다).

**한컴 차트 0x7EF0** (+ 끝 표시 0x7EF8 `ff ff ff ff`): `u32, u16, 차트 번호 u16` 다음에 ClientAnchor 와 같은 22바이트 앵커,
그 뒤에 원본 범위 수식, 차트 내용("Vt" 객체 저장, 뿌리 VtChart), 항목·계열 수식 — [`cell-legacy-chart.md`](cell-legacy-chart.md)
(`src/legacy/vtchart.rs` 가 읽는다; 통합 문서 읽기에는 아직 앵커만 쓴다). 한셀은 차트를 다른 그리기 개체보다 먼저 둔다.

## 6.3 조건부 서식·데이터 유효성 — 조작한 XML 원본과 대조

| 레코드 | 배치 |
|---|---|
| DXF FRT 0x088D (전역) | 차등 서식 표: FRT 머리 12 + `u32` + `cprops u16` + XFProp 목록 — **XLSB BrtDXF 와 같은 꼴, 같은 번호** ([cell-binary.md](cell-binary.md) 2.2절). 다른 점: 글꼴 크기(0x24)는 1/100 pt, XFPropColor 는 `icv` 0xFF·알파 0, 자동 색은 RGB 종류에 값 `FF FF FF DF`, 글꼴 이름(0x18)은 바른 UTF-16. 표시 형식·맞춤은 이 표에 쓰지 않는다(CF 의 DXFN 에만 있다). 보호(0x2C) 값은 틀리게 쓴다. 레코드 순서가 dxfId |
| CONDFMT 0x01B0 | `ccf u16, (nID << 1) u16, 경계 Ref8 (행 u32), n u16, Ref8 × n` |
| CF 0x01B1 | `ct u8 (1 값 비교, 2 수식), cp u8, cce1 u16, cce2 u16`, DXFN(+한컴 확장, 아래), `dxfId u32`, [안쪽 선 확장], 수식 1·2 |
| CONDFMT12 0x0879 | FRT 머리 12 뒤에 CONDFMT 와 같은 배치 (뒤따르는 CF12 의 범위) |
| CFEX 0x087B | FRT 머리 12, `fIsCF12 u32, nID u16` — CF12 면 다음 CF12 의 서식(nID), 아니면 `icf u16, cp u8, 틀 u8, 우선순위 u16, 플래그 u8 (0x02 멈춤)` … 끝 16바이트 = 틀 매개 변수 (글 규칙: 연산 u16, 상위/하위: 플래그 u8(0x01 위, 0x02 %) + 순위 u16) |
| CF12 0x087A | FRT 머리 12, `ct u8 (3 색 눈금, 4 막대, 6 아이콘), cp, cce1, cce2, cbdxf u32 + DXFN12`, 수식, 활성 수식(4), 플래그 u8, 우선순위 u16, 틀 u16, 매개 변수(cb u8 + 16), 길이 u16, 종류별 내용 (CFGradient / CFDatabar / CFMultistate: CFVO = 종류 u8, cce u16 + 토큰, 숫자 f64 — 최소·최대는 숫자 없음) |
| DVAL 0x01B2 / DV 0x01BE | DV: 플래그 u32 (XLSB BrtDVal 과 같은 비트), 입력 제목·오류 제목·입력·오류 (`cch u16, flag u8, UTF-16`), 수식 1·2 (`cce u16, 예약 u16`), 범위 (`n u16, Ref8 × n`) |

틀(CFTemp): 0x01 수식, 0x05 상위/하위, 0x07 고유, 0x08 글, 0x09 빈 칸, 0x0F–0x18 날짜, 0x19/0x1A 평균 위/아래, 0x1B 중복.

CF 레코드의 DXFN (관측, 차등 서식 대조 실험):

```
flags u32, flags2 u16   [MS-XLS] DXFN 그대로 — 블록 비트 ibitAtrNum 0x02000000, Fnt 0x04000000, Alc 0x08000000,
                        Bdr 0x10000000, Pat 0x20000000, Prot 0x40000000; 낮은 비트들은 '지정 안 함'(Ninch)
[표시 형식]  flags2 & 1 이면 DXFNumUsr (cb u16 자신 포함, cch u16, flag u8, 문자 — flag 0 이면 1바이트씩), 아니면 u16
[글꼴]       DXFFntD 118바이트 (BIFF8 그대로)
[맞춤]       DXFALC 8바이트 — 가로·세로 맞춤 번호가 표준보다 1 크다 (아래)
[테두리]     DXFBdr 8바이트
[무늬]       DXFPat 4바이트
[보호]       DXFProt 2바이트 (0x01 잠금, 0x02 숨김)
한컴 확장    글꼴 블록이 있으면 비트마스크 u32 + 값들 (크기 1/100 pt, 색 RGBA, 밑줄·취소선 모양, 언어별 글꼴 이름 …),
             테두리 블록이 있으면 RGBA × 4 (왼쪽·오른쪽·위·아래) + 2바이트, 무늬 블록이 있으면 RGBA × 2
dxfId u32
[안쪽 선]    테두리 블록이 있을 때만: 가로·세로 선마다 있음 u8 (+ 있으면 선 종류 u8, RGBA 4) → 2·7·12 바이트
수식 1·2
```

표준 블록은 크기가 정해져 있어 앞에서부터 읽을 수 있지만, 한컴 확장의 길이는 알 수 없으므로 dxfId 는 수식 쪽에서
거꾸로 찾는다. 안쪽 선 확장의 길이는 구조가 맞는 나눔 가운데 dxfId 가 가리키는 DXF(0x088D)의 안쪽 선과 맞는 가장 긴
것으로 고른다 — 검은 선은 색 바이트가 모두 0 이라 더 짧은 (틀린) 나눔도 구조가 맞는다.

DXFALC 의 맞춤 번호 (한셀): 가로 일반 1, 왼쪽 2, 가운데 3, 오른쪽 4, 채우기 5, 양쪽 6, 선택 영역 가운데 7, 분산 0
(8 이 3비트에서 넘친다), 세로 위 1, 가운데 2, 아래 3, 양쪽 4, 분산 5. XF 의 맞춤은 표준 그대로다. 한셀은 이 블록의
들여쓰기·읽기 순서를 쓰지 않는다 (맞춤 대조 실험).

CFEX 의 DXFN12 (`cbdxf u32` + DXFN + 한컴 확장 + dxfId + XFExtNoFRT): 차등 서식의 그라데이션은 여기 XFExtNoFRT
(`0, 0xFFFF, 0, cexts u16`, ExtProp 종류 6) 에만 있다.

**한셀은 레거시의 조건부 서식을 잘못 읽는다** (rcell 은 원본과 같게 읽는다): 글 포함·빈 칸 규칙의 수식을 `#N/A`
로, CONDFMT12 의 범위를 16비트 행으로 읽어 깨뜨리고, 차등 서식의 사용자 표시 형식을 쓰레기 값으로 만든다 (1바이트
문자를 UTF-16 으로 읽는다: `0.000` → `\⸰\〰\࠰`). DXFALC 의 세로 맞춤을 읽지 않고, 같은 내용이 된 차등 서식을 하나로
합친다. 평균 아래(`aboveAverage="0"`)도 잃는다. 데이터 유효성은 바르게 읽는다.

## 6.4 개요·자동 필터·보호 — 조작한 XML 원본과 대조

| 레코드 | 배치 |
|---|---|
| ROW 0x0208 / COLINFO 0x007D | 개요 수준: ROW 플래그 비트 0–2 (`0x10` 접힘), COLINFO grbit 비트 8–10 (`0x1000` 접힘) — BIFF8 과 같다 |
| WSBOOL 0x0081 | `0x0040` 요약 행 아래, `0x0080` 요약 열 오른쪽 — 한셀은 언제나 `0x04C1` 을 써서 **요약 위치를 잃는다** |
| PROTECT 0x0012 / WINDOWPROTECT 0x0019 / PASSWORD 0x0013 (전역) | 통합 문서 보호: 구조, 창, 16비트 암호 해시. 한셀은 WINDOWPROTECT 를 언제나 0 으로 써서 **창 잠금을 잃는다** |
| PROTECT / OBJPROTECT 0x0063 / SCENPROTECT 0x00DD / PASSWORD (시트) | 시트 보호, 개체, 시나리오, 암호 해시 |
| FEATHDR 0x0867 (isf = 2) | FRT 머리 12, `isf u16, 예약 u8, cbHdrData u32 (0xFFFFFFFF)`, `EnhancedProtection u32` — 비트 0–14 가 `objects` … `selectUnlockedCells` 의 **허용** 여부 (OOXML 속성과 반대) |
| FILTERMODE 0x009B / AUTOFILTERINFO 0x009D | 필터 적용 중 (빈 레코드) / 단추 수 `u16`. 범위는 그 시트의 숨은 내장 이름 `_xlnm._FilterDatabase`(0x0D) |
| AUTOFILTER 0x009E | `iEntry u16, grbit u16 (wJoin 비트 0–1: 0 = and, fSimple1/2, 0x10 상위/하위, 0x20 위, 0x40 백분율, 비트 7–15 개수)`, DOPER 10바이트 × 2 (`vt u8`: 2 RK, 4 f64, 6 문자열, 8 논리/오류, 0x0C 빈 칸, 0x0E 빈 칸 아님; `grbitSign u8`: 1 `<`, 2 `=`, 3 `<=`, 4 `>`, 5 `<>`, 6 `>=`; 문자열은 6번째 바이트가 cch), 이어서 문자열 (`flag u8 + UTF-16`). 상위/하위면 DOPER 1 에 기준값 |
| 0x009F (한컴) | 값 목록 필터의 값 하나: `iEntry u16, 종류 u8 (0 문자열, 0x0C 빈 칸)`, 문자열이면 `cch u16, flag u8, UTF-16`. 바로 앞 AUTOFILTER 를 대신한다 (값이 둘 이하면 AUTOFILTER 에도 `=` 조건 두 개를 `or` 로 남긴다) |
| AUTOFILTER12 0x087E | FrtRefHeader (범위 Ref8U), `iEntry u16, fHideArrow u32, ft u32, cft u32, cCriteria u32, cDateGroupings u32, …` — 동적 필터의 종류(cft): 1 평균 위, 2 평균 아래, 8 내일, 9 오늘, 10 어제, 11–13 다음 주/이번 주/지난주, 14–16 달, 17–19 분기, 20–22 해, 23 올해 누계, 24–27 Q1–Q4, 28–39 M1–M12. 조건 자체(AF12Criteria)는 쓰지 않는다 |

자동 필터의 단추는 콤보 상자 OBJ(`ot` 0x14)와 OfficeArt 도형으로 저장된다 — 그리기 개체가 아니므로 건너뛴다.

**한셀은 레거시의 자동 필터를 일부 잃고 읽는다** (rcell 은 원본과 같게 읽는다): 값 목록의 빈 칸(0x009F 종류 0x0C)과
상위/하위의 백분율(grbit 0x40)을 버리고, 값 두 개짜리 목록은 사용자 지정 `=` 조건으로 보여 준다(같은 필터).
시트 보호에서 잠기지 않은 셀 선택을 막으면 잠긴 셀 선택도 막힌 것으로 저장한다 (Excel 과 같은 규칙).

## 7. 수식 토큰

BIFF8 Ptg 와 같고 다음만 다르다 (관측, `src/legacy/formula.rs`):

| 토큰 | 크기(바이트, 토큰 번호 제외) | 비고 |
|---|---|---|
| PtgRef / PtgRefN / PtgRefErr | 6 | `rw u32, col u16 (비트 14 열 상대, 15 행 상대)` |
| PtgArea / PtgAreaN / PtgAreaErr | 12 | `rw u32 ×2, col u16 ×2`, 열 전체 = 행 0–1048575 |
| PtgRef3d | 8 | `ixti u16, rw u32, col u16` |
| PtgArea3d | 14 | |
| PtgStr | 3 + 문자 | `cch u16, flag u8` |
| PtgFunc / PtgFuncVar | BIFF8 | 함수 번호 0–367 은 BIFF8 Ftab, **IFERROR = 480** (한셀 확장). 그 밖의 Excel 2007 함수와 한컴 함수는 사용자 함수(PtgFuncVar 255)로 쓰고 첫 인자가 이름이다 (아래) |
| PtgName | 4 | NAME 순서 번호 u32 (1부터) |
| PtgNameX | 6 | `ixti u16, 이름 번호 u32` — XTI 의 SUPBOOK 아래 EXTERNNAME (1부터) |
| PtgExp | 6 | `rw u32, col u16` — 공유 수식(SHRFMLA, 범위 행 u32)의 기준 셀 |

Excel 2007 에서 생긴 함수(SUMIFS, AVERAGEIF …)는 `PtgName` 이 가리키는 NAME `_xlfn.이름` (grbit 0x000B 또는 0x038B) 이나
추가 기능 SUPBOOK 의 EXTERNNAME(`PtgNameX`)을 첫 인자로 한 PtgFuncVar 255 다. 한컴 함수(WON, JUNJA, RANKAVG, RANKEQ,
ERRORTYPE)도 같은 꼴이며 NAME 의 grbit 가 0 이다. XML 은 AVERAGEIF·AVERAGEIFS·COUNTIFS·SUMIFS·IFERROR 만 `_xlfn.` 없이
쓴다 (함수 대조 실험).

한셀의 레거시 저장은 grbit 0x038B 인 `_xlfn.` 이름을 PtgName 번호로 셀 때 grbit 0x000B 이름을 빼고 센다. 한셀 자신도
같은 방식으로 읽으므로 대개 맞게 보이지만, 번호가 엇갈리는 문서에서는 수식이 `#N/A` 가 된다. rcell 은 NAME 순서 그대로
읽는다.

### 7.1 외부 문서 참조

XTI 가 외부 SUPBOOK 을 가리키면 XML 표기로 `[n]시트!A1` 이 된다. `n` 은 외부 연결 번호(1부터)다.

- 가상 경로 ([MS-XLS] 2.5.277): U+0001 로 시작하면 부호화된 경로다. U+0001 + 드라이브 문자(`@` 는 UNC),
  U+0002 = 같은 드라이브의 루트, U+0003 = 하위 폴더, U+0004 = 상위 폴더.
  한셀은 U+0002 의 드라이브를 문서가 있는 드라이브로 채워 `file:///C:/…` 로 저장한다.
- 따옴표: 시트 이름 **또는 외부 문서 파일 이름**에 따옴표가 필요하면 `[n]시트` 전체를 감싼다
  (`'[1]Intro'!` ← "Auto Expense Report.hcdt", `[2]Intro!` ← "RecoveredExternalLink1").
- 한셀은 문서를 열 때 **어떤 수식·이름도 쓰지 않는 외부 연결을 버리고** 번호를 앞으로 당긴다.
- 가상 경로가 부호화되어 있지 않으면(U+0001 로 시작하지 않으면, 예: `RecoveredExternalLink1`) **경로를 잃은 연결**
  이다: XML 에서는 관계 종류가 `http://schemas.microsoft.com/office/2006/relationships/xlExternalLinkPath/xlPathMissing`.
  한셀은 이 종류와 외부 값 캐시의 갱신 오류 표시(`refreshError`)에 따라 이름 참조의 따옴표를 다르게 쓴다.

## 8. 한셀이 레거시 형식에서 잃어버리는 정보 (관측)

레거시로 저장한 뒤 한셀로 다시 열었을 때 원본과 달라지는 것. rcell 은 파일에 남아 있는 값은 그대로 읽는다.

| 정보 | 현상 |
|---|---|
| 상대 크기·장평·자간·글자 위치 | 파일에는 기록되지만(오프셋 58–73) **한셀이 다시 열 때 100/100/0/0 으로 읽는다** |
| 줄 간격 종류 `atLeast` | `betweenLines` 와 같은 값(2)으로 저장되어 구분이 사라진다 |
| `hs:useFontSpace` | 레거시에 저장되지 않는다 |
| 문단 번호 ID | 0 으로 저장된다 |
| 도형 안 글 | 파일(TXO)에는 있지만 **한셀이 다시 열 때 읽지 않는다** (rcell 은 읽는다) |
| 사용자 정의 모양 | 꼭짓점(0x0145)이 저장되지 않아 모양이 사라진다 (`custGeom` 없음) |
| 개요 요약 위치·창 잠금 | WSBOOL·WINDOWPROTECT 를 고정값으로 써서 잃는다 (6.4절) |
| 테마 | 레거시에는 테마가 없다 (THEME 0x0896 을 쓰지 않는다). 한셀은 다시 열 때 자기 기본 테마를 붙인다 |
| 외부 값 캐시 | XCT/CRN 을 쓰지 않는다. 다시 열면 시트마다 `refreshError="0"` 인 빈 캐시 |
| 자동 필터의 빈 칸·백분율 | 파일에는 있지만 한셀이 다시 열 때 읽지 않는다 (6.4절) |
| 사용자 정의 표 스타일 | 파일(0x088E–0x0890)에는 있지만 한셀이 다시 열 때 읽지 않는다 — 기본 스타일 이름만 남는다 |
| 조건부 서식의 차등 서식 | 세로 맞춤을 읽지 않고 사용자 표시 형식을 깨뜨린다 (6.3절) |

## 9. 검증 방법

1. `tools/probe/make-corpus.ps1` — 한셀 COM 으로 기능별 문서를 세 형식으로 저장.
2. `tools/probe/convert.ps1` — 한셀이 레거시 파일을 열어 XML 로 다시 저장 (정답).
3. `rcell diff X.legacy.cell X.legacy.as.cell --ignore-font-scale` — rcell 의 해석과 비교.
4. 값을 바꿔 가며 조작한 XML 을 한셀로 레거시 변환해 필드 오프셋을 확정 (`rcell patch`).
