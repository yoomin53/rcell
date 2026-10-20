// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
// 브라우저 뷰어: rcell.wasm (C ABI, import 없음) 을 불러 파일을 그린다.

(function () {
  "use strict";
  var wasm = null;
  var current = null; // { name, bytes }
  var utf8 = new TextDecoder("utf-8");

  function status(msg, err) {
    var s = document.getElementById("status");
    s.textContent = msg || "";
    s.className = err ? "err" : "";
  }

  // 결과: (포인터 << 32) | 길이, 첫 바이트 'O' 성공 / 'E' 오류
  function take(packed) {
    var big = BigInt.asUintN(64, BigInt(packed));
    var p = Number(big >> 32n);
    var n = Number(big & 0xffffffffn);
    var view = new Uint8Array(wasm.memory.buffer, p, n);
    var ok = view[0] === 0x4f;
    var body = view.slice(1);
    wasm.rcell_free(p, n);
    return { ok: ok, body: body };
  }

  function call(fn, bytes) {
    var ptr = wasm.rcell_alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
    var packed = fn(ptr, bytes.length);
    wasm.rcell_free(ptr, bytes.length);
    return take(packed);
  }

  function render(name, bytes) {
    var t0 = performance.now();
    var r = call(wasm.rcell_render, bytes);
    if (!r.ok) {
      status(name + ": " + utf8.decode(r.body), true);
      return;
    }
    document.getElementById("view").innerHTML = utf8.decode(r.body);
    document.getElementById("drop").hidden = true;
    current = { name: name, bytes: bytes };
    document.getElementById("save-cell").disabled = false;
    document.getElementById("save-xlsx").disabled = false;
    var info = call(wasm.rcell_info, bytes);
    var meta = "";
    if (info.ok) {
      var wb = JSON.parse(utf8.decode(info.body));
      meta = " · 시트 " + wb.sheets.length + " · 작성 " + (wb.application || "?");
    }
    status(name + meta + " · " + Math.round(performance.now() - t0) + "ms");
  }

  function save(fn, ext, mime) {
    if (!current) return;
    var r = call(fn, current.bytes);
    if (!r.ok) {
      status(utf8.decode(r.body), true);
      return;
    }
    var a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([r.body], { type: mime }));
    a.download = current.name.replace(/(\.legacy|\.bin)?\.(cell|xlsx|xlsb)$/i, "") + ext;
    a.click();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 1000);
  }

  function openFile(file) {
    file.arrayBuffer().then(function (buf) { render(file.name, new Uint8Array(buf)); });
  }

  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest(".rc-tabs button");
    if (!b) return;
    var book = b.closest(".rc-book");
    book.querySelectorAll(".rc-tabs button").forEach(function (x) {
      x.setAttribute("aria-selected", x === b);
    });
    book.querySelectorAll(".rc-wrap").forEach(function (w) {
      w.classList.toggle("rc-hidden", w.dataset.sheet !== b.dataset.sheet);
    });
  });

  document.getElementById("file").addEventListener("change", function (e) {
    if (e.target.files[0]) openFile(e.target.files[0]);
  });
  var drop = document.getElementById("drop");
  ["dragover", "dragenter"].forEach(function (ev) {
    document.addEventListener(ev, function (e) { e.preventDefault(); drop.classList.add("over"); });
  });
  document.addEventListener("dragleave", function () { drop.classList.remove("over"); });
  document.addEventListener("drop", function (e) {
    e.preventDefault();
    drop.classList.remove("over");
    if (e.dataTransfer.files[0]) openFile(e.dataTransfer.files[0]);
  });
  document.getElementById("save-cell").onclick = function () {
    save(wasm.rcell_to_cell, ".cell", "application/octet-stream");
  };
  document.getElementById("save-xlsx").onclick = function () {
    save(wasm.rcell_to_xlsx, ".xlsx",
      "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
  };

  status("엔진 불러오는 중…");
  fetch("rcell.wasm")
    .then(function (r) { return r.arrayBuffer(); })
    .then(function (b) { return WebAssembly.instantiate(b, {}); })
    .then(function (m) {
      wasm = m.instance.exports;
      document.getElementById("rc-css").textContent = utf8.decode(take(wasm.rcell_css()).body);
      status("");
      // ?file=경로 로 바로 열기 (데모/테스트용)
      var q = new URLSearchParams(location.search).get("file");
      if (q) {
        fetch(q).then(function (r) { return r.arrayBuffer(); })
          .then(function (b) { render(q.split("/").pop(), new Uint8Array(b)); })
          .catch(function (e) { status(String(e), true); });
      }
    })
    .catch(function (e) { status("rcell.wasm 을 불러오지 못했습니다: " + e, true); });
})();
