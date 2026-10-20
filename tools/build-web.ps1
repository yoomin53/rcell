# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 웹 뷰어 빌드: rcell-wasm 을 wasm32 로 빌드해 web/rcell.wasm 으로 복사한다.
# 사용: powershell -File tools/build-web.ps1  →  web/ 폴더를 아무 정적 서버로 열면 된다.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    rustup target add wasm32-unknown-unknown | Out-Null
    cargo build -p rcell-wasm --target wasm32-unknown-unknown --release
    Copy-Item "target/wasm32-unknown-unknown/release/rcell_wasm.wasm" "web/rcell.wasm" -Force
    "web/rcell.wasm 준비 완료 — 예: python -m http.server -d web 8000"
} finally {
    Pop-Location
}
