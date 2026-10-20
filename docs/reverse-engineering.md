<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 리버스 엔지니어링 원칙과 방법

> 작성: yoomin53

## 1. 목표

한셀이 만드는 모든 `.cell` 파일을 한셀과 같은 의미로 읽고, 그리고, 쓰는 것.
여기서 "완벽한 리버스 엔지니어링"은 다음 지표로 잰다.

| 지표 | 측정 방법 |
|---|---|
| 어휘 커버리지 | `rcell scan` 의 한컴 확장 모델링 비율 (목표 100%) |
| 읽기 정확도 | 한셀이 저장한 샘플 전부가 오류 없이 열림 (`tests/real_samples.rs`) |
| 렌더링 정확도 | 한셀 PDF 출력과 rcell 렌더 결과의 픽셀 차이 |
| 쓰기 정확도 | rcell 이 저장한 파일을 한셀이 경고 없이 열고, 한셀이 그 파일을 읽은 결과가 원본을 읽은 결과와 같음 (7절 판정 기준) |

## 2. 클린룸 원칙

rcell 은 **포맷의 독자 구현**이지 한셀의 복제가 아니다.

1. **저장소에는 우리가 작성한 코드와 문서만 넣는다.** 디컴파일 결과, 어셈블리 덤프,
   한컴 DLL 에서 뽑은 코드 조각, Ghidra 프로젝트 파일은 커밋하지 않는다
   (`.gitignore` 에 `re-workspace/`, `*.gpr`, `*.rep/` 이 있다).
2. **분석의 산출물은 "사실"이다.** 요소 이름, 속성 값의 범위, 기본값, 단위처럼
   포맷 호환에 필요한 정보만 `docs/format/` 에 문장으로 옮긴다.
3. **한컴 샘플 파일은 커밋하지 않는다.** 테스트 픽스처는 관찰한 구조를 따라 직접 작성한다
   (`tests/common/mod.rs`). 실제 파일 회귀 테스트는 로컬 폴더로 돌린다.
4. **목적은 호환성이다.** 라이선스 검사 우회, DRM 해제, 한셀 기능 자체의 복제는 범위 밖이다.

법적 근거: 저작권법 제101조의4 (프로그램코드역분석)는 정당하게 사용 권한을 얻은 사람이
호환에 필요한 정보를 얻기 위해 필요한 범위에서 코드를 분석하는 것을 허용한다. 단, 그 정보를
호환 목적 외로 쓰거나, 표현이 실질적으로 비슷한 프로그램을 만드는 데 쓸 수 없다.
위 원칙은 이 조건을 지키기 위한 것이다. (법률 자문이 아니다.)

## 3. 정보원 (신뢰도 순)

1. **한셀이 저장한 파일** — 가장 확실하다. 같은 문서에서 옵션 하나만 바꿔 저장한 뒤
   두 파일을 비교한다 (차분 분석). 한셀 COM 자동화(`HCell.Application`, Excel 호환
   객체 모델)로 셀마다 속성 하나씩 설정해 저장하면 자동으로 할 수 있다.
2. **공개 표준** — ECMA-376 (OOXML), [MS-XLSX], [MS-OI29500].
3. **export 심볼** — `HncOOXML.dll` 은 C++ 인터페이스를 이름으로 export 한다
   (예: `OOXML::SPREADSHEET::CParashape::GetleftMargin(unsigned int&)`). 속성 이름과
   값 타입을 디컴파일 없이 얻는다 → 문서에 "심볼" 로 표시.
4. **바이너리의 문자열 테이블** — UTF-16 문자열에서 요소·속성·열거값 이름 어휘를 얻는다.
   이름만 알 뿐 소속과 의미는 모른다 → 문서에 "문자열" 로 표시.
5. **REA + Ghidra 정적 분석** — 1~4 로 풀리지 않는 의미(열거값 대응, 기본값, 단위)를
   확인할 때만 쓴다. 예: REA `trace` 로 `lineSpacingType` 을 참조하는 함수가
   `CParashape::ReadAttribute` / `WriteElement` 임을 확인.

Ghidra headless(`analyzeHeadless.bat`)는 `Program Files (x86)` 처럼 괄호가 있는 경로를
처리하지 못하므로, 분석할 DLL 은 `re-workspace/bin/` 에 복사해서 쓴다.

## 4. 분석 대상 모듈 (한컴오피스 2024, `HOffice130\Bin`)

| 파일 | 역할 (관찰) |
|---|---|
| `HncOOXML.dll` | OOXML 읽기/쓰기. `hs:` 네임스페이스 어휘가 여기 있다 |
| `HNCFilter.dll`, `HncFilterFoundation.dll` | 필터 공통 계층 (hs 네임스페이스 문자열 포함) |
| `HCellBook.dll`, `HCellApp.dll` | 한셀 문서 모델·앱 (레거시 바이너리 `.cell` 처리 후보) |
| `HCell.tlb`, `HclAutoXl.tlb` | 자동화(COM) 형식 라이브러리 — 차분 분석용 문서 자동 생성에 쓸 수 있다 |

## 5. 작업 환경

```powershell
# 도구 (Windows)
winget install Git.Git Rustlang.Rustup OpenJS.NodeJS.22 EclipseAdoptium.Temurin.21.JDK
rustup default stable-x86_64-pc-windows-gnu   # MSVC 빌드 도구가 없으면
# Ghidra: https://github.com/NationalSecurityAgency/ghidra/releases (SHA-256 확인)
# REA:    npx rea-agents setup   (MCP 클라이언트에 서버로 등록)
```

분석 작업 폴더는 저장소 밖(또는 `.gitignore` 된 `re-workspace/`)에 둔다.

## 6. 작업 순서

1. `rcell scan 샘플폴더/` 로 모델링 안 된(✘) 한컴 어휘를 찾는다.
2. 한셀에서 그 속성을 바꿔 가며 문서를 저장해 값의 범위를 관찰한다.
3. 그래도 모호하면 REA 로 `HncOOXML.dll` 에서 해당 문자열을 참조하는 코드를 찾아
   값 대응(예: 열거 문자열 ↔ 정수)을 확인한다.
4. 결과를 `docs/format/` 에 확인 수준(관측/문자열/추정)과 함께 적는다.
5. `src/` 에 모델과 파서를 추가하고, 관찰한 구조로 픽스처를 만들어 테스트한다.
6. `rcell scan` 결과가 ✔ 로 바뀌었는지 확인한다.

## 7. 한셀을 정답으로 쓰는 차분 분석

설치된 한셀의 COM 자동화(`HCell.Application`, Excel 호환 객체 모델)로 한셀 자체를 정답 생성기로 쓴다.
모든 스크립트는 `tools/probe/` 에 있고, 만들어진 파일은 저장소 밖에 둔다.

| 기법 | 도구 | 무엇을 알아내나 |
|---|---|---|
| **같은 문서, 세 형식** | `make-corpus.ps1` | 기능별 문서를 XML·바이너리·레거시로 함께 저장 → 형식 간 대응 |
| **한셀의 읽기 결과** | `convert.ps1 -Format cell` | 한셀이 레거시/바이너리 파일을 열어 XML 로 다시 저장 → rcell 해석과 `rcell diff` |
| **값 조작 → 형식 변환** | `rcell patch` + `convert.ps1 -Format legacy` | XML 에 `hs:ratio=51` 같은 고유 값을 넣고 한셀로 레거시 변환 → 바이너리 필드 오프셋 확정 |
| **화면 표시** | `numfmt-oracle.ps1` | 값 × 서식 코드의 `Range.Text` → 표시 형식 엔진 정답 |
| **저장 검증** | `rcell convert` + `resave.ps1` | rcell 이 쓴 파일을 한셀이 다시 저장한 것(HCell(rcell(원본)))과 한셀이 원본을 다시 저장한 것(HCell(원본))을 `rcell diff` — 아래 "판정 기준" |
| **레거시 그리기 해부** | `rcell escher X.legacy.cell` | 시트별 OfficeArt 도형(종류·이름·앵커·속성)과 OBJ·TXO 를 나란히 → 한셀 XML 과 대조 |
| **실전 문서 일괄** | `batch-formats.ps1` | 한셀 서식 파일 같은 실제 문서를 세 형식으로 저장 + 한셀의 레거시 해석 → 아직 모르는 레코드를 찾는다 |
| **렌더링 비교** | `Worksheet.ExportAsFixedFormat` | 한셀 PDF 출력과 rcell HTML 을 나란히 본다 |

### 판정 기준: HCell(원본) 대 HCell(rcell(원본))

한셀은 저장할 때마다 일부 값을 스스로 바꾼다 (요약 열 위치, 머리글 글꼴 코드, 나눈 창의 활성 창, 외부 연결
이름의 펼침 …). 그래서 rcell 이 쓴 파일을 한셀이 다시 저장한 결과를 **원본 파일과** 비교하면 한셀 자신의 변화가
섞인다. 정답은 같은 한셀이 원본을 읽어 다시 저장한 결과다:

| 원본 | 정답 HCell(원본) | 비교 대상 HCell(rcell(원본)) |
|---|---|---|
| XML `.cell` | `hs/X.hs.cell` (원본을 한셀로 다시 저장) | `rcell-out/X.rcell.as.cell` |
| 레거시 | `X.legacy.as.cell` (한셀의 레거시 읽기) — 한셀이 레거시에서 못 읽는 것은 `--ignore-font-scale --ignore-shape-detail --ignore-cf --ignore-filter` | `rcell-out/X.legacy.rcell.as.cell` |
| 바이너리 | 한셀이 바이너리를 COM 으로 다시 열지 못하므로 같은 문서의 XML(`X.cell`)로 대신한다 (`--ignore-hancom`, 바이너리가 담지 못하는 외부 연결 상태 제외) | `rcell-out/X.bin.rcell.as.cell` |

`tests/real_samples.rs` 의 `writer_output_is_read_identically_by_hancell` 이 이 기준으로 비교한다.

### 함정

- **바뀐 것이 없는 문서를 같은 형식으로 `SaveAs` 하면 한셀은 파일을 그대로 복사한다** (다시 쓰지 않는다).
  이것을 모르고 "한셀이 다시 저장한 파일"과 비교하면 rcell 이 쓴 파일을 자기 자신과 비교하는 셈이 된다.
  `resave.ps1`·`batch-formats.ps1` 은 `SaveAs` 전에 `Workbook.Saved = False` 로 다시 쓰게 만든다
  (확인: 저장본의 `docProps/app.xml` 이 한셀의 것으로 바뀐다).
- Windows PowerShell 5.1 은 BOM 없는 UTF-8 스크립트의 한글을 잘못 읽는다 → 스크립트를 UTF-8(BOM)으로 저장.
- PowerShell 의 COM 속성 대입은 처음 넣은 값의 타입을 기억한다. `Value2` 에 정수를 넣으면
  이후 문자열 대입이 실패하므로 숫자는 `[double]` 로 넣는다.
- 한셀은 자기가 저장한 바이너리 `.cell`(xlCellb)을 COM 으로 다시 열지 못한다 (서버 오류).
- COM 오류 뒤에는 `HCell.exe -Automation -Embedding` 프로세스가 남는다. 이 명령줄을 가진
  프로세스만 골라 정리한다 (사용자가 띄운 한셀 창은 건드리지 않는다).
- 한셀은 넣은 서식 코드를 내장 번호로 바꿔 저장하기도 한다 → 표시 형식 대조는 저장된 파일의
  실제 서식 코드로 한다.
