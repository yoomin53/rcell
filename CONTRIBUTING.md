<!--
  Copyright 2026 yoomin53 (https://github.com/yoomin53)
  SPDX-License-Identifier: Apache-2.0
-->

# 기여 안내

rcell 은 yoomin53 이 만든 프로젝트입니다. 기여를 환영합니다.

## 1. 저작자 머리말 (필수)

모든 소스 파일(`.rs`, `.toml`, `.ts`, `.js`, `.py`, `.ps1`, `.sh`)은 다음 머리말로 시작해야 합니다.

```rust
// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
```

`tests/attribution.rs` 가 이를 검사하므로 머리말이 없으면 `cargo test` 와 CI 가 실패합니다.
기여자 본인의 이름은 머리말 아래에 `// Contributor: 이름` 으로 덧붙일 수 있습니다.
기존 저작자 표기와 [NOTICE](NOTICE) 는 지우거나 바꿀 수 없습니다 (Apache-2.0 제4조).

## 2. 클린룸 규칙 (필수)

- 한컴 소프트웨어의 디컴파일 결과, 어셈블리, 코드 조각을 커밋하지 마세요.
- 한셀로 만든 실제 문서를 커밋하지 마세요. 테스트 픽스처는 직접 작성합니다.
- 포맷에서 새로 알아낸 사실은 `docs/format/` 에 확인 수준(관측/문자열/추정)과 함께 적어 주세요.

자세한 내용: [docs/reverse-engineering.md](docs/reverse-engineering.md)

## 3. 검사

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
RCELL_CORPUS=말뭉치폴더 cargo test --test real_samples -- --nocapture   # 한셀 정답 대조 (tools/probe 로 만든 말뭉치)
```

## 4. 라이선스

기여한 코드는 Apache License 2.0 으로 배포되는 것에 동의한 것으로 봅니다 (Apache-2.0 제5조).
