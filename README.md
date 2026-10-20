<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# rcell

**한컴 셀(한셀) `.cell` 스프레드시트 포맷의 클린룸 오픈소스 구현**

만든 사람: **yoomin53** (<https://github.com/yoomin53>) · 라이선스: Apache-2.0

[rhwp](https://github.com/edwardkim/rhwp) 가 한글(HWP) 문서를 누구나 열 수 있게 했듯이,
rcell 은 한셀 문서를 한컴오피스 없이 읽고, 그리고, 저장한다. 브라우저에서도 동작한다.

> English: rcell is a clean-room, Apache-2.0 implementation of the Hancom Cell (`.cell`)
> spreadsheet format in Rust, by yoomin53. It reads all three `.cell` variants (XML,
> binary, and the legacy BIFF-based format), renders sheets to HTML, writes `.cell`/`.xlsx`,
> and ships a WebAssembly browser viewer.

---

## 무엇을 하나

| 기능 | 상태 | 한셀과 대조한 검증 |
|---|---|---|
| **XML `.cell`** (한셀 2014 이후 기본) 읽기 | ✅ | 한컴 확장 어휘 100% 모델링 |
| **바이너리 `.cell`** (xlCellb, BIFF12) 읽기 | ✅ | 같은 문서의 XML 형식과 샘플 8종 차이 0 |
| **레거시 `.cell`** (한셀 2010 이하 계열, OLE2 + BIFF 변형) 읽기 | ✅ | 한셀의 해석과 샘플 10종 차이 0 |
| `.xlsx` / `.xlsb` 읽기 | ✅ | |
| **XML `.cell` / `.xlsx` 저장**, 형식 변환 | ✅ | 한셀이 원본을 읽은 결과와 rcell 이 쓴 파일을 읽은 결과가 같다: 131쌍 (세 형식 원본, 알려진 한계 제외) |
| **표시 형식 엔진** (`#,##0`, 날짜, 분수, 조건, 한국어 요일 …) | ✅ | 한셀 화면 표시 742개 모두 일치 |
| **HTML 렌더링** (글꼴·장평·자간·채우기·테두리·병합·정렬) | ✅ | 한셀 PDF 출력과 화면 비교 |
| **웹 뷰어** (WebAssembly, 설치·서버 없음) | ✅ | |
| 한컴 확장: 장평·자간·그림자·외곽선·강조점·양각/음각·문단 모양 | ✅ | 열거형 전체 값을 한셀 바이너리 변환표에서 확인 |
| 수식 원문 (레거시/바이너리 토큰 → 텍스트) | ✅ | 수식 43종 일치 |
| 시트 보기·인쇄 설정, 메모, 그리기 개체·차트, 조건부 서식·데이터 유효성 | ✅ | 세 형식 읽기·저장, 조작한 문서로 한셀과 대조 |
| 개요(그룹)·자동 필터·시트/통합 문서 보호, 테마·문서 속성·외부 연결 캐시 보존 | ✅ | 세 형식 읽기·저장, 한셀 판정과 차이 0 |
| 암호 설정 문서 (ECMA-376 표준 암호화, AES-128) | ✅ 열기 | 한셀이 보호 문서에 거는 기본 암호는 자동으로 푼다, `--password` |
| 수식 계산, 레거시 차트(VtChart) 내용 | 📋 계획 | |

포맷 명세 (리버스 엔지니어링 결과):
- [XML `.cell`](docs/format/cell-2024.md) — `hs:` 한컴 확장, `mc:AlternateContent`, 열거형 값
- [레거시 `.cell`](docs/format/cell-legacy.md) — BIFF8 과의 차이, FONT/XF 확장 영역, 문단 모양 표, 확장 레코드 길이
- [바이너리 `.cell`](docs/format/cell-binary.md) — XLSB 와의 관계

## 빠른 시작

```bash
cargo build --release

rcell info    문서.cell                  # 요약 (세 형식 모두)
rcell html    문서.cell -o 문서.html     # 한셀 화면처럼 HTML 로
rcell csv     문서.cell -s 0             # 첫 시트를 CSV 로
rcell convert 옛문서.cell 새문서.cell    # 레거시/바이너리 → 최신 XML .cell
rcell convert 문서.cell 문서.xlsx        # Excel 형식으로
rcell dump    문서.cell --no-cells       # 문서 모델 JSON
rcell diff    a.cell b.cell              # 두 문서의 의미 비교 (형식이 달라도 됨)
```

리버스 엔지니어링 도구:

```bash
rcell scan    폴더/                     # 한컴 확장 어휘 커버리지
rcell streams 문서.cell                  # ZIP 파트 / OLE2 스트림 목록
rcell records 옛문서.cell --only 0031    # 레거시 BIFF 레코드 (바이너리는 --part xl/styles.bin)
rcell hexdump 옛문서.cell /Workbook      # 16진수
rcell patch   a.cell b.cell --part xl/styles.xml=새.xml   # 파트 교체 (값 조작 실험)
```

### 웹 뷰어

```bash
sh tools/build-web.sh        # 또는 powershell -File tools/build-web.ps1
python3 -m http.server -d web 8000   # http://localhost:8000
```

`.cell` 파일을 끌어 놓으면 브라우저 안에서 바로 그리고, 최신 `.cell` 이나 `.xlsx` 로 저장할 수 있다.
파일은 브라우저 밖으로 나가지 않는다. 저장소 설정에서 Pages 를 켜면 `pages.yml` 이 자동 배포한다.

### 라이브러리로 쓰기

```rust
let wb = rcell::open("문서.cell")?;               // XML / 바이너리 / 레거시 자동 판별
for sheet in &wb.sheets {
    for cell in &sheet.cells {
        let xf = wb.styles.xf(cell.style).unwrap();
        let code = rcell::numfmt::code_for(&wb.styles, xf.num_fmt_id);
        let shown = rcell::numfmt::format(&cell.value, &code, wb.date1904);
        println!("{}!{} = {}", sheet.name, cell.r, shown.text);   // 한셀 화면과 같은 문자열
    }
}
rcell::save(&wb, "새문서.cell")?;
```

## 프로젝트 구조

```
src/
  lib.rs            공개 API (open, from_bytes, save)
  model.rs          문서 모델 (한컴 확장 포함)
  xml.rs package.rs ns.rs        XML DOM, OPC 패키지, 네임스페이스
  parser/           XML .cell 파서
  binary/           바이너리 .cell (BIFF12) 파서
  legacy/           레거시 .cell: biff(레코드), reader, formula(Ptg 디코더), ftab(함수 번호)
  cfb.rs            OLE2 복합 문서 리더 (MS-CFB)
  writer.rs         XML .cell / .xlsx 저장
  numfmt.rs         표시 형식 엔진
  theme.rs render.rs             테마 색, HTML 렌더러
  compare.rs scan.rs vocab.rs    의미 비교, 한컴 어휘 스캐너, 열거형 어휘
crates/rcell-wasm/  WebAssembly 인터페이스 (C ABI, import 없음)
web/                브라우저 뷰어
tools/probe/        한셀 COM 자동화로 차분 분석 샘플·정답 생성
tests/              단위·통합·회귀 테스트 (한셀 정답 대조 포함)
docs/               포맷 명세, 분석 방법, 로드맵
```

## 리버스 엔지니어링 원칙 (클린룸)

rcell 에는 **한컴 소프트웨어의 코드가 한 줄도 들어가지 않는다.**

- 포맷 지식은 한셀이 저장한 파일, 공개 표준(ECMA-376, MS-XLS, MS-XLSB, MS-CFB), 그리고
  **한셀 자체를 정답으로 쓰는 차분 실험**에서 얻는다 (COM 자동화로 값을 바꿔 저장·변환·표시).
- 바이너리 분석([REA](https://github.com/morluto/rea) + Ghidra)은 열거형 이름 같은 **사실 확인에만**
  쓰고, 분석 산출물(디컴파일 결과 등)은 저장소에 넣지 않는다.
- 한컴이 만든 샘플 파일은 저작물이므로 커밋하지 않는다. 테스트 픽스처는 직접 만든다.

방법과 근거: [docs/reverse-engineering.md](docs/reverse-engineering.md)

## 검증

```bash
cargo test                                   # 단위·통합 테스트 (한컴 파일 불필요)
RCELL_CORPUS=<샘플 폴더> cargo test            # 레거시/바이너리/저장 결과를 한셀 정답과 대조
RCELL_NUMFMT_ORACLE=<경로> cargo test --test numfmt_oracle   # 표시 형식 742개 대조
```

샘플과 정답은 `tools/probe/make-corpus.ps1`, `convert.ps1`, `numfmt-oracle.ps1` 로
한셀이 설치된 Windows 에서 만든다.

## 기여

[CONTRIBUTING.md](CONTRIBUTING.md) 를 읽어 주세요. 모든 소스 파일은 다음 머리말로 시작해야 하며,
빠뜨리면 `cargo test` 가 실패한다.

```rust
// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
```

## 상표 고지

한컴, 한셀, Hancom, Hancom Cell 은 (주)한글과컴퓨터의 상표입니다. rcell 은 (주)한글과컴퓨터와
관련이 없으며 후원이나 보증을 받지 않은 독립 프로젝트입니다.

## 라이선스

Copyright 2026 **yoomin53** (<https://github.com/yoomin53>)

[Apache License 2.0](LICENSE). 재배포하거나 파생 저작물을 만들 때는 [NOTICE](NOTICE) 의
저작자 표기를 함께 포함해야 합니다 (Apache-2.0 제4조 (d)항).
