#!/usr/bin/env sh
# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 웹 뷰어 빌드: rcell-wasm 을 wasm32 로 빌드해 web/rcell.wasm 으로 복사한다.
set -eu
cd "$(dirname "$0")/.."
rustup target add wasm32-unknown-unknown >/dev/null
cargo build -p rcell-wasm --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/rcell_wasm.wasm web/rcell.wasm
echo "web/rcell.wasm 준비 완료 — 예: python3 -m http.server -d web 8000"
