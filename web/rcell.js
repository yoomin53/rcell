// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
// 브라우저 편집기: rcell.wasm (C ABI, import 없음) 으로 문서를 열고, 그리고, 고치고, 저장한다.
// 편집 규칙(입력·참조 조정·서식)은 모두 wasm 안의 rcell::edit 가 한다 — 여기서는 선택·키보드·마우스·클립보드만.

(function () {
  "use strict";
  var MAX_ROWS = 1048576, MAX_COLS = 16384;
  var wasm = null;
  var utf8 = new TextDecoder("utf-8");
  var enc = new TextEncoder();
  var doc = null; // { id, name, info }
  var sheet = 0;
  var view = null; // 그린 시트: { grid, merges, used, prot, canvas, zoom, cols, rows, colAt, rowAt }
  var sel = { anchor: { r: 0, c: 0 }, focus: { r: 0, c: 0 }, active: { r: 0, c: 0 } };
  var editing = null; // { r, c, orig, bar(수식 입력줄에서), dirty }
  var clipMark = null; // { sheet, r0, c0, r1, c1, cut, tsv }
  var minRows = 60, minCols = 26;
  var cur = null; // 활성 칸 정보 (rcell_doc_cell)
  var drag = null; // 마우스 끌기
  var refPick = null; // 수식 입력 중 칸 고르기: { start, end, r0, c0 }

  function $(id) { return document.getElementById(id); }

  function status(msg, err) {
    var s = $("status");
    s.textContent = msg || "";
    s.className = err ? "err" : "";
  }

  // ---------------------------------------------------------------- wasm 호출
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

  function withBytes(bytes, f) {
    var p = wasm.rcell_alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, p, bytes.length).set(bytes);
    try {
      return f(p, bytes.length);
    } finally {
      wasm.rcell_free(p, bytes.length);
    }
  }

  function json(r) {
    if (!r.ok) throw new Error(utf8.decode(r.body));
    return JSON.parse(utf8.decode(r.body));
  }

  // ---------------------------------------------------------------- 주소
  function colName(c) {
    var s = "";
    c += 1;
    while (c > 0) {
      var m = (c - 1) % 26;
      s = String.fromCharCode(65 + m) + s;
      c = Math.floor((c - 1) / 26);
    }
    return s;
  }
  function a1(r, c) { return colName(c) + (r + 1); }
  function parseA1(s) {
    var m = /^\s*\$?([A-Za-z]{1,3})\$?(\d{1,7})\s*$/.exec(s);
    if (!m) return null;
    var c = 0;
    for (var i = 0; i < m[1].length; i++) c = c * 26 + (m[1].toUpperCase().charCodeAt(i) - 64);
    var r = parseInt(m[2], 10);
    if (r < 1 || r > MAX_ROWS || c > MAX_COLS) return null;
    return { r: r - 1, c: c - 1 };
  }
  function range() {
    var a = sel.anchor, f = sel.focus;
    var r = { r0: Math.min(a.r, f.r), c0: Math.min(a.c, f.c), r1: Math.max(a.r, f.r), c1: Math.max(a.c, f.c) };
    return expandMerges(r);
  }
  function rangeText(g) {
    if (g.c0 === 0 && g.c1 >= MAX_COLS - 1) return (g.r0 + 1) + ":" + (g.r1 + 1);
    if (g.r0 === 0 && g.r1 >= MAX_ROWS - 1) return colName(g.c0) + ":" + colName(g.c1);
    var s = a1(g.r0, g.c0);
    return g.r0 === g.r1 && g.c0 === g.c1 ? s : s + ":" + a1(g.r1, g.c1);
  }
  function mergeAt(r, c) {
    if (!view) return null;
    for (var i = 0; i < view.merges.length; i++) {
      var m = view.merges[i];
      if (r >= m[0] && r <= m[2] && c >= m[1] && c <= m[3]) return m;
    }
    return null;
  }
  // 범위가 걸친 병합을 모두 담도록 넓힌다
  function expandMerges(g) {
    if (!view || !view.merges.length) return g;
    var changed = true;
    while (changed) {
      changed = false;
      for (var i = 0; i < view.merges.length; i++) {
        var m = view.merges[i];
        if (m[2] < g.r0 || m[0] > g.r1 || m[3] < g.c0 || m[1] > g.c1) continue;
        if (m[0] < g.r0) { g.r0 = m[0]; changed = true; }
        if (m[1] < g.c0) { g.c0 = m[1]; changed = true; }
        if (m[2] > g.r1) { g.r1 = m[2]; changed = true; }
        if (m[3] > g.c1) { g.c1 = m[3]; changed = true; }
      }
    }
    return g;
  }

  // ---------------------------------------------------------------- 문서
  function openBytes(name, bytes) {
    var res;
    try {
      res = withBytes(bytes, function (p, n) { return json(take(wasm.rcell_doc_open(p, n))); });
    } catch (e) {
      status(name + ": " + e.message, true);
      return;
    }
    if (doc) wasm.rcell_doc_close(doc.id);
    doc = { id: res.doc, name: name, info: res.info };
    sheet = firstEditable(res.info, res.info.active);
    sel = { anchor: { r: 0, c: 0 }, focus: { r: 0, c: 0 }, active: { r: 0, c: 0 } };
    clipMark = null;
    show();
    status(name + " · 시트 " + res.info.sheets.length + " · 작성 " + (res.info.application || "?"));
  }

  function newDoc() {
    var res = json(take(wasm.rcell_doc_new()));
    if (doc) wasm.rcell_doc_close(doc.id);
    doc = { id: res.doc, name: "새 문서.cell", info: res.info };
    sheet = 0;
    sel = { anchor: { r: 0, c: 0 }, focus: { r: 0, c: 0 }, active: { r: 0, c: 0 } };
    show();
    status("새 문서");
  }

  function firstEditable(info, want) {
    var s = info.sheets[want];
    if (s && s.visible && !s.chart) return want;
    for (var i = 0; i < info.sheets.length; i++) if (info.sheets[i].visible && !info.sheets[i].chart) return i;
    return want || 0;
  }

  function show() {
    $("drop").hidden = true;
    $("area").hidden = false;
    $("tools").hidden = false;
    $("fbar").hidden = false;
    ["save-cell", "save-bin", "save-xlsx"].forEach(function (b) { $(b).disabled = false; });
    minRows = 60;
    minCols = 26;
    render();
    drawTabs();
    $("area").focus();
  }

  function applyInfo(info) {
    if (!info) return;
    doc.info = info;
    $("undo").disabled = !info.canUndo;
    $("redo").disabled = !info.canRedo;
  }

  // wasm 에는 시계가 없다 — 편집(다시 계산) 전에 지금 시각과 시간대를 알린다
  function tick() {
    wasm.rcell_set_clock(Date.now(), -new Date().getTimezoneOffset());
  }

  // 편집 명령 하나 (rcell::edit::Edit JSON). 성공하면 다시 그린다
  function edit(cmd, keepSel) {
    if (!doc) return null;
    tick();
    var r = withBytes(enc.encode(JSON.stringify(cmd)), function (p, n) { return take(wasm.rcell_doc_edit(doc.id, p, n)); });
    if (!r.ok) {
      status(utf8.decode(r.body), true);
      return null;
    }
    var info = JSON.parse(utf8.decode(r.body));
    applyInfo(info);
    if (typeof info.sheet === "number" && !keepSel) sheet = info.sheet;
    clipMark = clipMark && !clipMark.cut ? clipMark : null;
    render();
    drawTabs();
    status("");
    return info;
  }

  function undo(redo) {
    if (!doc) return;
    var r = take(wasm.rcell_doc_undo(doc.id, redo ? 1 : 0));
    if (!r.ok) {
      status(utf8.decode(r.body), true);
      return;
    }
    applyInfo(JSON.parse(utf8.decode(r.body)));
    if (sheet >= doc.info.sheets.length) sheet = firstEditable(doc.info, 0);
    clipMark = null;
    render();
    drawTabs();
  }

  function save(fmt, ext, mime) {
    if (!doc) return;
    commitEdit();
    var r = take(wasm.rcell_doc_save(doc.id, fmt));
    if (!r.ok) {
      status(utf8.decode(r.body), true);
      return;
    }
    var a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([r.body], { type: mime }));
    a.download = doc.name.replace(/(\.legacy|\.bin)?\.(cell|xlsx|xlsb)$/i, "") + ext;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 1000);
    status(a.download + " 저장 (" + r.body.length.toLocaleString() + " 바이트)");
    return r.body;
  }

  // ---------------------------------------------------------------- 그리기
  function render() {
    var area = $("area");
    var st = area.scrollTop, sl = area.scrollLeft;
    var need = range();
    var mr = Math.max(minRows, Math.min(need.r1, sel.active.r + 1) + 30);
    var mc = Math.max(minCols, Math.min(need.c1, sel.active.c + 1) + 8);
    var res;
    try {
      res = json(take(wasm.rcell_doc_render(doc.id, sheet, mr, mc)));
    } catch (e) {
      status(e.message, true);
      return;
    }
    area.innerHTML = res.html;
    var canvas = area.querySelector(".rc-canvas");
    var g = res.grid;
    view = {
      grid: g,
      merges: res.merges || [],
      used: res.used,
      prot: res.protected,
      canvas: canvas,
      zoom: (g.zoom || 100) / 100,
      cols: {},
      rows: {},
      colList: g.cols,
      rowList: g.rows,
    };
    g.cols.forEach(function (x) { view.cols[x[0]] = [x[1], x[2]]; });
    g.rows.forEach(function (x) { view.rows[x[0]] = [x[1], x[2]]; });
    // 머리글 칸 (선택한 행·열 강조)
    var head = canvas.querySelectorAll("thead th");
    view.colTh = {};
    for (var i = 1; i < head.length && i - 1 < g.cols.length; i++) view.colTh[g.cols[i - 1][0]] = head[i];
    view.rowTh = {};
    canvas.querySelectorAll("tbody tr[data-r]").forEach(function (tr) {
      var th = tr.querySelector("th");
      if (th) view.rowTh[tr.getAttribute("data-r")] = th;
    });
    var layer = document.createElement("div");
    layer.className = "ed-layer";
    canvas.appendChild(layer);
    view.layer = layer;
    area.scrollTop = st;
    area.scrollLeft = sl;
    drawSelection();
    updateCell();
  }

  function colBox(c) {
    var x = view.cols[c];
    if (x) return x;
    var list = view.colList;
    if (!list.length) return [view.grid.header_w, 0];
    var last = list[list.length - 1];
    if (c > last[0]) return [last[1] + last[2] + (c - last[0] - 1) * 64, 64];
    // 숨긴 열: 다음 보이는 열의 왼쪽, 너비 0
    for (var i = 0; i < list.length; i++) if (list[i][0] > c) return [list[i][1], 0];
    return [last[1], 0];
  }
  function rowBox(r) {
    var y = view.rows[r];
    if (y) return y;
    var list = view.rowList;
    if (!list.length) return [view.grid.header_h, 0];
    var last = list[list.length - 1];
    if (r > last[0]) return [last[1] + last[2] + (r - last[0] - 1) * 20, 20];
    for (var i = 0; i < list.length; i++) if (list[i][0] > r) return [list[i][1], 0];
    return [last[1], 0];
  }
  function lastRenderedRow() { var l = view.rowList; return l.length ? l[l.length - 1][0] : 0; }
  function lastRenderedCol() { var l = view.colList; return l.length ? l[l.length - 1][0] : 0; }

  function box(g) {
    var r1 = Math.min(g.r1, lastRenderedRow());
    var c1 = Math.min(g.c1, lastRenderedCol());
    var x0 = colBox(g.c0), y0 = rowBox(g.r0), x1 = colBox(c1), y1 = rowBox(r1);
    return { left: x0[0], top: y0[0], width: x1[0] + x1[1] - x0[0], height: y1[0] + y1[1] - y0[0] };
  }

  function place(el, b, cls) {
    el.className = cls;
    el.style.left = b.left - 1 + "px";
    el.style.top = b.top - 1 + "px";
    el.style.width = Math.max(b.width + 1, 3) + "px";
    el.style.height = Math.max(b.height + 1, 3) + "px";
    view.layer.appendChild(el);
  }

  function drawSelection() {
    if (!view) return;
    var layer = view.layer;
    layer.innerHTML = "";
    var g = range();
    var multi = g.r0 !== g.r1 || g.c0 !== g.c1;
    var m = mergeAt(sel.active.r, sel.active.c);
    if (multi && !(m && m[0] === g.r0 && m[1] === g.c0 && m[2] === g.r1 && m[3] === g.c1)) {
      place(document.createElement("div"), box(g), "ed-sel");
    }
    var act = m ? { r0: m[0], c0: m[1], r1: m[2], c1: m[3] } : { r0: sel.active.r, c0: sel.active.c, r1: sel.active.r, c1: sel.active.c };
    place(document.createElement("div"), box(act), "ed-act");
    if (clipMark && clipMark.sheet === sheet) place(document.createElement("div"), box(clipMark), "ed-clip");
    if (refPick && refPick.r0 !== undefined) place(document.createElement("div"), box(refPick.g), "ed-ref");
    // 머리글 강조
    Object.keys(view.colTh).forEach(function (c) {
      view.colTh[c].classList.toggle("ed-hdr", c >= g.c0 && c <= g.c1);
    });
    Object.keys(view.rowTh).forEach(function (r) {
      view.rowTh[r].classList.toggle("ed-hdr", r >= g.r0 && r <= g.r1);
    });
    $("namebox").value = multi ? rangeText(g) : a1(sel.active.r, sel.active.c);
  }

  // 활성 칸이 보이게 스크롤한다 (머리글과 틀 고정 영역을 비켜서)
  function scrollIntoView() {
    var area = $("area");
    var b = box({ r0: sel.active.r, c0: sel.active.c, r1: sel.active.r, c1: sel.active.c });
    var z = view.zoom;
    var hw = view.grid.header_w * z, hh = view.grid.header_h * z;
    var fr = view.grid.frozen;
    if (fr) {
      hh = rowBox(fr[0])[0] * z;
      hw = colBox(fr[1])[0] * z;
    }
    var l = b.left * z, t = b.top * z, w = b.width * z, h = b.height * z;
    if (!fr || sel.active.r >= fr[0]) {
      if (t - hh < area.scrollTop) area.scrollTop = Math.max(0, t - hh);
      else if (t + h > area.scrollTop + area.clientHeight) area.scrollTop = t + h - area.clientHeight + 2;
    }
    if (!fr || sel.active.c >= fr[1]) {
      if (l - hw < area.scrollLeft) area.scrollLeft = Math.max(0, l - hw);
      else if (l + w > area.scrollLeft + area.clientWidth) area.scrollLeft = l + w - area.clientWidth + 2;
    }
  }

  // 활성 칸 정보 → 수식 입력줄·도구 막대
  function updateCell() {
    if (!doc || !view) return;
    try {
      cur = json(take(wasm.rcell_doc_cell(doc.id, sheet, sel.active.r, sel.active.c)));
    } catch (e) {
      cur = null;
      return;
    }
    if (!editing) $("formula").value = cur.input;
    var s = cur.style;
    $("bold").setAttribute("aria-pressed", !!s.bold);
    $("italic").setAttribute("aria-pressed", !!s.italic);
    $("underline").setAttribute("aria-pressed", !!s.underline);
    $("strike").setAttribute("aria-pressed", !!s.strike);
    $("wrap").setAttribute("aria-pressed", !!s.wrap);
    ["left", "center", "right"].forEach(function (k) { $("h-" + k).setAttribute("aria-pressed", s.hAlign === k); });
    ["top", "center", "bottom"].forEach(function (k) { $("v-" + k).setAttribute("aria-pressed", s.vAlign === k); });
    setSelect($("font-name"), s.fontName);
    setSelect($("font-size"), s.fontSize != null ? String(s.fontSize) : null);
    setSelect($("num-fmt"), s.numFmt);
    if (s.fontColor) $("font-color").value = s.fontColor.slice(0, 7);
  }

  function setSelect(el, v) {
    if (v == null) return;
    var found = false;
    for (var i = 0; i < el.options.length; i++) {
      if (el.options[i].value === v || el.options[i].text === v) {
        el.selectedIndex = i;
        found = true;
        break;
      }
    }
    if (!found) {
      var o = document.createElement("option");
      o.value = v;
      o.text = v;
      el.appendChild(o);
      el.value = v;
    }
  }

  // ---------------------------------------------------------------- 시트 탭
  function drawTabs() {
    var tabs = $("tabs");
    tabs.innerHTML = "";
    if (!doc) return;
    doc.info.sheets.forEach(function (s) {
      if (!s.visible) return;
      var b = document.createElement("button");
      b.setAttribute("role", "tab");
      b.setAttribute("aria-selected", s.index === sheet);
      b.textContent = s.name;
      b.title = s.chart ? "차트 시트 (편집할 수 없음)" : "두 번 눌러 이름 바꾸기";
      b.onclick = function () { switchSheet(s.index); };
      b.ondblclick = function () { renameSheet(s.index); };
      b.oncontextmenu = function (e) {
        e.preventDefault();
        switchSheet(s.index);
        showMenu(e.clientX, e.clientY, [
          ["시트 추가", function () { addSheet(); }],
          ["이름 바꾸기", function () { renameSheet(s.index); }],
          ["삭제", function () { deleteSheet(s.index); }],
          null,
          ["왼쪽으로 옮기기", function () { moveSheet(s.index, -1); }],
          ["오른쪽으로 옮기기", function () { moveSheet(s.index, 1); }],
        ]);
      };
      tabs.appendChild(b);
    });
    var add = document.createElement("button");
    add.textContent = "+";
    add.title = "시트 추가";
    add.onclick = addSheet;
    tabs.appendChild(add);
  }

  function switchSheet(i) {
    commitEdit();
    if (i === sheet) return;
    sheet = i;
    json(take(wasm.rcell_doc_select(doc.id, i)));
    sel = { anchor: { r: 0, c: 0 }, focus: { r: 0, c: 0 }, active: { r: 0, c: 0 } };
    minRows = 60;
    minCols = 26;
    $("area").scrollTop = 0;
    $("area").scrollLeft = 0;
    render();
    drawTabs();
  }

  function addSheet() {
    var info = edit({ op: "addSheet", after: sheet }, true);
    if (info) switchSheet(info.sheet);
  }
  function renameSheet(i) {
    var old = doc.info.sheets[i].name;
    var name = window.prompt("시트 이름", old);
    if (name == null || name === old) return;
    edit({ op: "renameSheet", sheet: i, name: name }, true);
  }
  function deleteSheet(i) {
    if (!window.confirm("시트 '" + doc.info.sheets[i].name + "' 을(를) 지울까요? (실행 취소할 수 있습니다)")) return;
    var info = edit({ op: "deleteSheet", sheet: i }, true);
    if (info) {
      sheet = -1;
      switchSheet(firstEditable(doc.info, Math.min(i, doc.info.sheets.length - 1)));
    }
  }
  function moveSheet(i, d) {
    var to = i + d;
    if (to < 0 || to >= doc.info.sheets.length) return;
    var info = edit({ op: "moveSheet", sheet: i, to: to }, true);
    if (info) {
      sheet = to;
      drawTabs();
      render();
    }
  }

  // ---------------------------------------------------------------- 선택·이동
  function setActive(r, c, extend) {
    r = Math.max(0, Math.min(MAX_ROWS - 1, r));
    c = Math.max(0, Math.min(MAX_COLS - 1, c));
    // 병합된 칸 안으로 들어가면 병합의 왼쪽 위로
    var m = mergeAt(r, c);
    if (m && !extend) { r = m[0]; c = m[1]; }
    if (extend) {
      sel.focus = { r: r, c: c };
    } else {
      sel.anchor = { r: r, c: c };
      sel.focus = { r: r, c: c };
      sel.active = { r: r, c: c };
    }
    ensureRendered();
    drawSelection();
    scrollIntoView();
    updateCell();
  }

  // 그린 범위 밖으로 가면 더 그린다
  function ensureRendered() {
    var g = range();
    var r = Math.max(sel.focus.r, sel.active.r), c = Math.max(sel.focus.c, sel.active.c);
    if ((r > lastRenderedRow() - 5 && g.r1 < MAX_ROWS - 1) || (c > lastRenderedCol() - 3 && g.c1 < MAX_COLS - 1)) {
      minRows = Math.max(minRows, r + 40);
      minCols = Math.max(minCols, c + 12);
      render();
    }
  }

  // 숨긴 행·열을 건너뛰는 다음 칸
  function step(pos, d, rows) {
    var map = rows ? view.rows : view.cols;
    var last = rows ? lastRenderedRow() : lastRenderedCol();
    var max = rows ? MAX_ROWS - 1 : MAX_COLS - 1;
    var p = pos;
    for (var i = 0; i < 64; i++) {
      p += d;
      if (p < 0) return 0;
      if (p > max) return max;
      if (p > last || map[p]) return p;
    }
    return p;
  }

  function move(dr, dc, extend) {
    var from = extend ? sel.focus : sel.active;
    var m = !extend && mergeAt(from.r, from.c);
    var r = from.r, c = from.c;
    if (dr > 0 && m) r = m[2];
    if (dc > 0 && m) c = m[3];
    if (dr) r = step(r, dr, true);
    if (dc) c = step(c, dc, false);
    setActive(r, c, extend);
  }

  function jump(dr, dc, extend) {
    var from = extend ? sel.focus : sel.active;
    var res = json(take(wasm.rcell_doc_jump(doc.id, sheet, from.r, from.c, dr, dc)));
    setActive(res.row, res.col, extend);
  }

  // Enter·Tab: 여러 칸을 골랐으면 그 안에서 돈다
  function cycle(dr, dc) {
    var g = range();
    if (g.r0 === g.r1 && g.c0 === g.c1) {
      move(dr, dc, false);
      return;
    }
    var r = sel.active.r + dr, c = sel.active.c + dc;
    if (dc) {
      if (c > g.c1) { c = g.c0; r += 1; }
      if (c < g.c0) { c = g.c1; r -= 1; }
      if (r > g.r1) r = g.r0;
      if (r < g.r0) r = g.r1;
    } else {
      if (r > g.r1) { r = g.r0; c += 1; }
      if (r < g.r0) { r = g.r1; c -= 1; }
      if (c > g.c1) c = g.c0;
      if (c < g.c0) c = g.c1;
    }
    sel.active = { r: r, c: c };
    drawSelection();
    scrollIntoView();
    updateCell();
  }

  // ---------------------------------------------------------------- 칸 안 편집
  function editor() {
    var el = $("editor");
    if (!el) {
      el = document.createElement("textarea");
      el.id = "editor";
      el.spellcheck = false;
      el.setAttribute("aria-label", "칸 편집");
      el.addEventListener("keydown", onEditorKey);
      el.addEventListener("input", function () {
        if (editing) {
          editing.dirty = true;
          $("formula").value = el.value;
          fitEditor();
        }
      });
    }
    return el;
  }

  function startEdit(initial, keepContent) {
    if (!doc || !view) return;
    if (editing) return;
    var r = sel.active.r, c = sel.active.c;
    var el = editor();
    var orig = cur ? cur.input : "";
    editing = { r: r, c: c, orig: orig, bar: false, dirty: initial != null };
    el.value = keepContent ? orig : initial || "";
    var m = mergeAt(r, c);
    var b = box(m ? { r0: m[0], c0: m[1], r1: m[2], c1: m[3] } : { r0: r, c0: c, r1: r, c1: c });
    el.style.left = b.left - 1 + "px";
    el.style.top = b.top - 1 + "px";
    el.style.minWidth = b.width + 3 + "px";
    el.style.minHeight = b.height + 3 + "px";
    el.style.width = b.width + 3 + "px";
    el.style.height = b.height + 3 + "px";
    if (cur && cur.style) {
      el.style.fontFamily = "'" + (cur.style.fontName || "맑은 고딕") + "','Malgun Gothic',sans-serif";
      el.style.fontSize = (cur.style.fontSize || 11) + "pt";
      el.style.fontWeight = cur.style.bold ? "bold" : "normal";
      el.style.fontStyle = cur.style.italic ? "italic" : "normal";
    }
    view.layer.appendChild(el);
    $("formula").value = el.value;
    fitEditor();
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }

  function fitEditor() {
    var el = $("editor");
    if (!el || !editing) return;
    el.style.width = "0px";
    el.style.width = Math.max(parseFloat(el.style.minWidth) || 0, el.scrollWidth + 6) + "px";
    el.style.height = "0px";
    el.style.height = Math.max(parseFloat(el.style.minHeight) || 0, el.scrollHeight + 2) + "px";
  }

  function editText() {
    if (!editing) return "";
    return editing.bar ? $("formula").value : editor().value;
  }

  // 편집을 끝낸다 (입력을 칸에 넣는다). 실패하면 편집을 계속한다
  function commitEdit(dr, dc) {
    if (!editing) return true;
    var text = editText();
    var e = editing;
    var changed = e.dirty && text !== e.orig;
    if (changed) {
      var info = edit({ op: "set", sheet: sheet, cell: a1(e.r, e.c), input: text }, true);
      if (!info) {
        // 수식 오류: 편집을 계속한다
        (e.bar ? $("formula") : editor()).focus();
        return false;
      }
    }
    endEdit();
    if (dr || dc) cycle(dr || 0, dc || 0);
    else updateCell();
    return true;
  }

  function cancelEdit() {
    endEdit();
    updateCell();
  }

  function endEdit() {
    editing = null;
    refPick = null;
    var el = $("editor");
    if (el && el.parentNode) el.parentNode.removeChild(el);
    $("area").focus();
    drawSelection();
  }

  // 수식 입력 중: 칸을 누르면 참조를 넣는다 (커서가 연산자·괄호·쉼표·= 뒤에 있을 때)
  function wantsRef() {
    if (!editing) return false;
    var el = editing.bar ? $("formula") : editor();
    var t = el.value;
    if (t.charAt(0) !== "=") return false;
    var pos = el.selectionStart;
    if (refPick && refPick.start !== undefined && pos === refPick.end) return true;
    var before = t.slice(0, pos).replace(/\s+$/, "");
    return /[=+\-*/^&(,:<>;]$/.test(before);
  }

  function insertRef(g) {
    var el = editing.bar ? $("formula") : editor();
    var t = el.value;
    var start, end;
    if (refPick && refPick.start !== undefined && el.selectionStart === refPick.end) {
      start = refPick.start;
      end = refPick.end;
    } else {
      start = end = el.selectionStart;
    }
    var txt = rangeText(g);
    el.value = t.slice(0, start) + txt + t.slice(end);
    refPick = { start: start, end: start + txt.length, g: g, r0: g.r0 };
    el.setSelectionRange(refPick.end, refPick.end);
    editing.dirty = true;
    if (!editing.bar) $("formula").value = el.value;
    else editor().value = el.value;
    fitEditor();
    drawSelection();
  }

  function onEditorKey(e) {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Enter" && e.altKey) {
      // 칸 안 줄 바꿈
      e.preventDefault();
      var el = e.target;
      var p = el.selectionStart;
      el.value = el.value.slice(0, p) + "\n" + el.value.slice(el.selectionEnd);
      el.setSelectionRange(p + 1, p + 1);
      editing.dirty = true;
      $("formula").value = el.value;
      fitEditor();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      commitEdit(e.shiftKey ? -1 : 1, 0);
    } else if (e.key === "Tab") {
      e.stopPropagation();
      e.preventDefault();
      commitEdit(0, e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancelEdit();
    } else if (!editing.dirty || editing.quick) {
      // 입력을 시작한 뒤 화살표는 칸을 옮긴다 (F2 로 시작했으면 글 안에서 움직인다)
      var arrows = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
      if (arrows[e.key] && editing.quick) {
        if (wantsRef()) {
          e.preventDefault();
          var base = refPick && refPick.g ? { r: refPick.g.r0, c: refPick.g.c0 } : { r: editing.r, c: editing.c };
          var r = Math.max(0, base.r + arrows[e.key][0]), c = Math.max(0, base.c + arrows[e.key][1]);
          insertRef({ r0: r, c0: c, r1: r, c1: c });
          return;
        }
        e.preventDefault();
        commitEdit(arrows[e.key][0], arrows[e.key][1]);
      }
    }
  }

  // ---------------------------------------------------------------- 마우스
  function canvasPoint(e) {
    var rect = view.canvas.getBoundingClientRect();
    return { x: (e.clientX - rect.left) / view.zoom, y: (e.clientY - rect.top) / view.zoom };
  }
  function colAtX(x) {
    var l = view.colList;
    if (!l.length) return 0;
    if (x < l[0][1]) return l[0][0];
    for (var i = 0; i < l.length; i++) if (x < l[i][1] + l[i][2]) return l[i][0];
    var last = l[l.length - 1];
    return Math.min(MAX_COLS - 1, last[0] + 1 + Math.floor((x - last[1] - last[2]) / 64));
  }
  function rowAtY(y) {
    var l = view.rowList;
    if (!l.length) return 0;
    if (y < l[0][1]) return l[0][0];
    for (var i = 0; i < l.length; i++) if (y < l[i][1] + l[i][2]) return l[i][0];
    var last = l[l.length - 1];
    return Math.min(MAX_ROWS - 1, last[0] + 1 + Math.floor((y - last[1] - last[2]) / 20));
  }
  // 머리글 경계 (크기 바꾸기): 경계 ±4px 안이면 그 열·행
  function colEdge(x) {
    var l = view.colList;
    for (var i = 0; i < l.length; i++) if (Math.abs(x - (l[i][1] + l[i][2])) <= 4) return l[i][0];
    return -1;
  }
  function rowEdge(y) {
    var l = view.rowList;
    for (var i = 0; i < l.length; i++) if (Math.abs(y - (l[i][1] + l[i][2])) <= 3) return l[i][0];
    return -1;
  }

  function onMouseDown(e) {
    if (!view || e.button !== 0) return;
    if (e.target.id === "editor") return;
    var p = canvasPoint(e);
    var g = view.grid;
    var inColHdr = p.y < g.header_h && p.x >= g.header_w;
    var inRowHdr = p.x < g.header_w && p.y >= g.header_h;
    if (g.header_h && p.y < g.header_h && p.x < g.header_w) {
      // 모서리: 모두 선택
      commitEdit();
      sel.anchor = { r: 0, c: 0 };
      sel.focus = { r: MAX_ROWS - 1, c: MAX_COLS - 1 };
      sel.active = { r: 0, c: 0 };
      drawSelection();
      updateCell();
      e.preventDefault();
      return;
    }
    if (inColHdr && g.header_h) {
      var ce = colEdge(p.x);
      if (ce >= 0) {
        commitEdit();
        drag = { kind: "colsize", col: ce, start: colBox(ce)[0], x: p.x };
        e.preventDefault();
        return;
      }
      var c = colAtX(p.x);
      if (editing && wantsRef()) {
        insertRef({ r0: 0, c0: c, r1: MAX_ROWS - 1, c1: c });
        e.preventDefault();
        return;
      }
      commitEdit();
      if (e.shiftKey) sel.focus = { r: MAX_ROWS - 1, c: c };
      else {
        sel.anchor = { r: 0, c: c };
        sel.focus = { r: MAX_ROWS - 1, c: c };
        sel.active = { r: 0, c: c };
      }
      drag = { kind: "cols" };
      drawSelection();
      updateCell();
      e.preventDefault();
      return;
    }
    if (inRowHdr && g.header_w) {
      var re = rowEdge(p.y);
      if (re >= 0) {
        commitEdit();
        drag = { kind: "rowsize", row: re, start: rowBox(re)[0], y: p.y };
        e.preventDefault();
        return;
      }
      var r = rowAtY(p.y);
      if (editing && wantsRef()) {
        insertRef({ r0: r, c0: 0, r1: r, c1: MAX_COLS - 1 });
        e.preventDefault();
        return;
      }
      commitEdit();
      if (e.shiftKey) sel.focus = { r: r, c: MAX_COLS - 1 };
      else {
        sel.anchor = { r: r, c: 0 };
        sel.focus = { r: r, c: MAX_COLS - 1 };
        sel.active = { r: r, c: 0 };
      }
      drag = { kind: "rows" };
      drawSelection();
      updateCell();
      e.preventDefault();
      return;
    }
    var rr = rowAtY(p.y), cc = colAtX(p.x);
    if (editing && wantsRef()) {
      insertRef({ r0: rr, c0: cc, r1: rr, c1: cc });
      drag = { kind: "ref", r: rr, c: cc };
      e.preventDefault();
      return;
    }
    if (!commitEdit()) return;
    if (e.shiftKey) setActive(rr, cc, true);
    else setActive(rr, cc, false);
    drag = { kind: "cells" };
    $("area").focus();
    e.preventDefault();
  }

  function onMouseMove(e) {
    if (!view) return;
    if (!drag) {
      // 머리글 경계에서는 크기 바꾸기 커서
      if (e.target.closest && e.target.closest("#area")) {
        var p0 = canvasPoint(e);
        var g0 = view.grid;
        var cur0 = "";
        if (p0.y < g0.header_h && p0.x >= g0.header_w && colEdge(p0.x) >= 0) cur0 = "col-resize";
        else if (p0.x < g0.header_w && p0.y >= g0.header_h && rowEdge(p0.y) >= 0) cur0 = "row-resize";
        $("area").style.cursor = cur0;
      }
      return;
    }
    var p = canvasPoint(e);
    if (drag.kind === "colsize" || drag.kind === "rowsize") {
      showGuide(drag.kind === "colsize" ? p.x : null, drag.kind === "rowsize" ? p.y : null);
      return;
    }
    if (drag.kind === "ref") {
      var g = { r0: Math.min(drag.r, rowAtY(p.y)), c0: Math.min(drag.c, colAtX(p.x)), r1: Math.max(drag.r, rowAtY(p.y)), c1: Math.max(drag.c, colAtX(p.x)) };
      insertRef(g);
      return;
    }
    if (drag.kind === "cols") sel.focus = { r: MAX_ROWS - 1, c: colAtX(p.x) };
    else if (drag.kind === "rows") sel.focus = { r: rowAtY(p.y), c: MAX_COLS - 1 };
    else sel.focus = { r: rowAtY(p.y), c: colAtX(p.x) };
    drawSelection();
  }

  function showGuide(x, y) {
    var el = $("ed-guide") || document.createElement("div");
    el.id = "ed-guide";
    el.className = "ed-guide";
    if (x != null) {
      el.style.left = x + "px";
      el.style.top = "0px";
      el.style.width = "1px";
      el.style.height = view.canvas.scrollHeight + "px";
    } else {
      el.style.top = y + "px";
      el.style.left = "0px";
      el.style.height = "1px";
      el.style.width = view.canvas.scrollWidth + "px";
    }
    view.layer.appendChild(el);
  }

  function onMouseUp(e) {
    if (!drag) return;
    var d = drag;
    drag = null;
    if (!view) return;
    var p = canvasPoint(e);
    if (d.kind === "colsize") {
      var px = Math.max(0, Math.round(p.x - d.start));
      var g = range();
      var whole = g.r0 === 0 && g.r1 >= MAX_ROWS - 1 && d.col >= g.c0 && d.col <= g.c1;
      var c0 = whole ? g.c0 : d.col, n = whole ? g.c1 - g.c0 + 1 : 1;
      edit({ op: "colWidthPx", sheet: sheet, col: colName(c0), count: n, px: px }, true);
    } else if (d.kind === "rowsize") {
      var h = Math.max(0, Math.round(p.y - d.start));
      var g2 = range();
      var whole2 = g2.c0 === 0 && g2.c1 >= MAX_COLS - 1 && d.row >= g2.r0 && d.row <= g2.r1;
      var r0 = whole2 ? g2.r0 : d.row, n2 = whole2 ? g2.r1 - g2.r0 + 1 : 1;
      edit({ op: "rowHeight", sheet: sheet, row: r0 + 1, count: n2, height: Math.round(h * 0.75 * 100) / 100 }, true);
    } else if (d.kind === "ref") {
      // 계속 입력
    } else {
      ensureRendered();
      updateCell();
    }
  }

  function onDblClick(e) {
    if (!view || e.target.id === "editor") return;
    var p = canvasPoint(e);
    if (p.x < view.grid.header_w || p.y < view.grid.header_h) return;
    startEdit(null, true);
    editing.quick = false;
  }

  // ---------------------------------------------------------------- 키보드
  function onKey(e) {
    // 칸 편집기·입력줄에서 처리한 키가 문서로 올라오면 무시한다 (편집을 끝낸 뒤라도)
    if (isField(e.target) || e.defaultPrevented) return;
    if (!doc || !view || editing) return;
    if (document.activeElement && /^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement.tagName)) return;
    var ctrl = e.ctrlKey || e.metaKey;
    var k = e.key;
    var arrows = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
    if (arrows[k]) {
      e.preventDefault();
      if (ctrl) jump(arrows[k][0], arrows[k][1], e.shiftKey);
      else move(arrows[k][0], arrows[k][1], e.shiftKey);
      return;
    }
    if (ctrl) {
      var lk = k.toLowerCase();
      if (lk === "z") { e.preventDefault(); undo(e.shiftKey); return; }
      if (lk === "y") { e.preventDefault(); undo(true); return; }
      if (lk === "b") { e.preventDefault(); toggle("bold"); return; }
      if (lk === "i") { e.preventDefault(); toggle("italic"); return; }
      if (lk === "u") { e.preventDefault(); toggle("underline"); return; }
      if (lk === "d") { e.preventDefault(); fillSel(true); return; }
      if (lk === "r") { e.preventDefault(); fillSel(false); return; }
      if (lk === "a") {
        e.preventDefault();
        sel.anchor = { r: 0, c: 0 };
        sel.focus = { r: MAX_ROWS - 1, c: MAX_COLS - 1 };
        drawSelection();
        return;
      }
      if (lk === "s") { e.preventDefault(); save(0, ".cell", "application/octet-stream"); return; }
      if (k === "Home") { e.preventDefault(); setActive(0, 0, e.shiftKey); return; }
      if (k === "End") {
        e.preventDefault();
        var u = view.used || [0, 0];
        setActive(u[0], u[1], e.shiftKey);
        return;
      }
      return;
    }
    if (k === "Enter") { e.preventDefault(); cycle(e.shiftKey ? -1 : 1, 0); return; }
    if (k === "Tab") { e.preventDefault(); cycle(0, e.shiftKey ? -1 : 1); return; }
    if (k === "Home") { e.preventDefault(); setActive(sel.active.r, 0, e.shiftKey); return; }
    if (k === "PageDown" || k === "PageUp") {
      e.preventDefault();
      var rowsPerPage = Math.max(1, Math.floor($("area").clientHeight / 20 / view.zoom) - 2);
      var from = e.shiftKey ? sel.focus : sel.active;
      setActive(from.r + (k === "PageDown" ? rowsPerPage : -rowsPerPage), from.c, e.shiftKey);
      return;
    }
    if (k === "F2") { e.preventDefault(); startEdit(null, true); editing.quick = false; return; }
    if (k === "Delete") { e.preventDefault(); clearSel(); return; }
    if (k === "Backspace") { e.preventDefault(); startEdit("", false); editing.quick = true; return; }
    if (k === "Escape") { clipMark = null; drawSelection(); return; }
    if (k.length === 1 && !e.altKey) {
      // 글자를 치면 새로 입력 (칸 내용을 바꾼다)
      e.preventDefault();
      startEdit(k, false);
      editing.quick = true;
    }
  }

  // 한글 조합 입력: 조합이 시작되면 편집기로 넘긴다
  function onCompositionStart() {
    if (!doc || editing) return;
    if (document.activeElement !== $("area")) return;
    startEdit("", false);
    editing.quick = true;
  }

  // ---------------------------------------------------------------- 명령
  function selCmd(extra) {
    var g = range();
    var o = { sheet: sheet, range: rangeText(g) };
    for (var k in extra) o[k] = extra[k];
    return o;
  }

  function toggle(prop) {
    var on = !(cur && cur.style && cur.style[prop]);
    var p = {};
    p[prop] = on;
    format(p);
  }

  function format(patch) {
    commitEdit();
    var o = selCmd(patch);
    o.op = "format";
    edit(o, true);
  }

  function clearSel() {
    var o = selCmd({});
    o.op = "clear";
    edit(o, true);
  }

  function fillSel(down) {
    var o = selCmd({});
    o.op = down ? "fillDown" : "fillRight";
    edit(o, true);
  }

  function border(kind) {
    var o = selCmd({});
    o.op = "border";
    if (kind === "thick") {
      o.preset = "outline";
      o.style = "medium";
    } else o.preset = kind;
    edit(o, true);
  }

  function mergeToggle() {
    var g = range();
    var has = view.merges.some(function (m) { return !(m[2] < g.r0 || m[0] > g.r1 || m[3] < g.c0 || m[1] > g.c1); });
    var o = selCmd({});
    if (has) o.op = "unmerge";
    else {
      o.op = "merge";
      o.center = true;
      // 왼쪽 위 칸 말고 값이 있는 칸이 있으면 묻는다 (한셀: 왼쪽 위 칸의 값만 남는다)
      if (!window.confirm("선택한 범위를 병합하면 왼쪽 위 칸의 값만 남습니다. 병합할까요?")) return;
    }
    edit(o, true);
  }

  function rowsCols(kind) {
    var g = range();
    var o = { sheet: sheet };
    if (kind === "insRow" || kind === "delRow") {
      o.op = kind === "insRow" ? "insertRows" : "deleteRows";
      o.row = g.r0 + 1;
      o.count = Math.min(g.r1 - g.r0 + 1, 10000);
    } else {
      o.op = kind === "insCol" ? "insertCols" : "deleteCols";
      o.col = colName(g.c0);
      o.count = Math.min(g.c1 - g.c0 + 1, 1000);
    }
    edit(o, true);
  }

  // ---------------------------------------------------------------- 클립보드
  function copySel(cut) {
    var g = range();
    var r1 = Math.min(g.r1, Math.max(lastRenderedRow(), view.used ? view.used[0] : 0));
    var c1 = Math.min(g.c1, Math.max(lastRenderedCol(), view.used ? view.used[1] : 0));
    var r = take(wasm.rcell_doc_copy(doc.id, sheet, g.r0, g.c0, r1, c1, cut ? 1 : 0));
    if (!r.ok) {
      status(utf8.decode(r.body), true);
      return null;
    }
    var tsv = utf8.decode(r.body);
    clipMark = { sheet: sheet, r0: g.r0, c0: g.c0, r1: r1, c1: c1, cut: !!cut, tsv: tsv };
    drawSelection();
    status((cut ? "잘라 냄: " : "복사함: ") + rangeText(clipMark) + " — 붙일 칸을 고르고 Ctrl+V");
    return tsv;
  }

  function norm(t) { return (t || "").replace(/\r\n/g, "\n").replace(/\n$/, ""); }

  function pasteAt(text) {
    var g = range();
    var res;
    tick();
    if (clipMark && (text == null || norm(text) === norm(clipMark.tsv))) {
      res = json(take(wasm.rcell_doc_paste(doc.id, sheet, g.r0, g.c0, g.r1, g.c1)));
      if (clipMark.cut) clipMark = null;
    } else if (text != null) {
      res = withBytes(enc.encode(text), function (p, n) { return json(take(wasm.rcell_doc_paste_text(doc.id, sheet, g.r0, g.c0, p, n))); });
    } else return;
    applyInfo(res);
    if (res.range) {
      sel.anchor = { r: res.range[0], c: res.range[1] };
      sel.focus = { r: res.range[2], c: res.range[3] };
      sel.active = { r: res.range[0], c: res.range[1] };
    }
    render();
    drawTabs();
  }

  function safePaste(text) {
    try {
      pasteAt(text);
    } catch (e) {
      status(e.message, true);
    }
  }

  // ---------------------------------------------------------------- 메뉴
  function showMenu(x, y, items) {
    var m = $("menu");
    m.innerHTML = "";
    items.forEach(function (it) {
      if (!it) {
        m.appendChild(document.createElement("hr"));
        return;
      }
      var b = document.createElement("button");
      b.textContent = it[0];
      b.onclick = function () {
        hideMenu();
        it[1]();
      };
      m.appendChild(b);
    });
    m.hidden = false;
    m.style.left = Math.min(x, window.innerWidth - 200) + "px";
    m.style.top = Math.min(y, window.innerHeight - m.offsetHeight - 8) + "px";
  }
  function hideMenu() { $("menu").hidden = true; }

  function cellMenu(e) {
    if (!view) return;
    e.preventDefault();
    var p = canvasPoint(e);
    var g = range();
    var r = rowAtY(p.y), c = colAtX(p.x);
    if (r < g.r0 || r > g.r1 || c < g.c0 || c > g.c1) setActive(r, c, false);
    showMenu(e.clientX, e.clientY, [
      ["잘라내기 (Ctrl+X)", function () { copySel(true); }],
      ["복사 (Ctrl+C)", function () { var t = copySel(false); if (t && navigator.clipboard) navigator.clipboard.writeText(t).catch(function () {}); }],
      ["붙이기 (Ctrl+V)", function () {
        if (navigator.clipboard && navigator.clipboard.readText) {
          navigator.clipboard.readText().then(safePaste, function () { safePaste(null); });
        } else safePaste(null);
      }],
      null,
      ["행 삽입", function () { rowsCols("insRow"); }],
      ["행 삭제", function () { rowsCols("delRow"); }],
      ["열 삽입", function () { rowsCols("insCol"); }],
      ["열 삭제", function () { rowsCols("delCol"); }],
      null,
      ["내용 지우기 (Delete)", clearSel],
      ["서식 지우기", function () { var o = selCmd({}); o.op = "clearFormats"; edit(o, true); }],
      ["아래로 채우기 (Ctrl+D)", function () { fillSel(true); }],
      ["오른쪽으로 채우기 (Ctrl+R)", function () { fillSel(false); }],
      ["병합 / 병합 풀기", mergeToggle],
    ]);
  }

  // ---------------------------------------------------------------- 연결
  function wire() {
    var area = $("area");
    area.addEventListener("mousedown", onMouseDown);
    area.addEventListener("dblclick", onDblClick);
    area.addEventListener("contextmenu", cellMenu);
    area.addEventListener("scroll", function () {
      if (!view) return;
      var near = area.scrollTop + area.clientHeight > area.scrollHeight - 300;
      var nearX = area.scrollLeft + area.clientWidth > area.scrollWidth - 300;
      if ((near && lastRenderedRow() < MAX_ROWS - 1) || (nearX && lastRenderedCol() < MAX_COLS - 1)) {
        if (near) minRows = lastRenderedRow() + 80;
        if (nearX) minCols = lastRenderedCol() + 20;
        if (!editing) render();
      }
    });
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    document.addEventListener("keydown", onKey);
    document.addEventListener("compositionstart", onCompositionStart, true);
    document.addEventListener("mousedown", function (e) {
      if (!e.target.closest || !e.target.closest("#menu")) hideMenu();
    });
    document.addEventListener("copy", function (e) {
      if (!doc || editing || isField(document.activeElement)) return;
      var t = copySel(false);
      if (t != null) {
        e.clipboardData.setData("text/plain", t);
        e.preventDefault();
      }
    });
    document.addEventListener("cut", function (e) {
      if (!doc || editing || isField(document.activeElement)) return;
      var t = copySel(true);
      if (t != null) {
        e.clipboardData.setData("text/plain", t);
        e.preventDefault();
      }
    });
    document.addEventListener("paste", function (e) {
      if (!doc || editing || isField(document.activeElement)) return;
      e.preventDefault();
      safePaste(e.clipboardData.getData("text/plain"));
    });

    // 수식 입력줄
    var f = $("formula");
    f.addEventListener("focus", function () {
      if (!doc || editing) return;
      editing = { r: sel.active.r, c: sel.active.c, orig: cur ? cur.input : "", bar: true, dirty: false };
    });
    f.addEventListener("input", function () {
      if (editing) editing.dirty = true;
    });
    f.addEventListener("keydown", function (e) {
      if (e.isComposing || e.keyCode === 229) return;
      if (!editing) return;
      if (e.key === "Enter") { e.preventDefault(); commitEdit(1, 0); }
      else if (e.key === "Tab") { e.preventDefault(); commitEdit(0, 1); }
      else if (e.key === "Escape") { e.preventDefault(); cancelEdit(); }
    });
    f.addEventListener("blur", function () {
      // 칸을 눌러 참조를 넣는 중이 아니면 입력을 넣는다
      setTimeout(function () {
        if (editing && editing.bar && document.activeElement !== f && !drag) commitEdit();
      }, 0);
    });

    // 이름 상자: 주소로 가기
    $("namebox").addEventListener("keydown", function (e) {
      if (e.key !== "Enter") return;
      e.preventDefault();
      var t = e.target.value.split(":");
      var a = parseA1(t[0]), b = t[1] ? parseA1(t[1]) : a;
      if (!a || !b) {
        status("셀 주소가 아닙니다: " + e.target.value, true);
        return;
      }
      setActive(a.r, a.c, false);
      if (t[1]) setActive(b.r, b.c, true);
      $("area").focus();
    });

    // 도구 막대
    $("undo").onclick = function () { undo(false); };
    $("redo").onclick = function () { undo(true); };
    $("bold").onclick = function () { toggle("bold"); };
    $("italic").onclick = function () { toggle("italic"); };
    $("underline").onclick = function () { toggle("underline"); };
    $("strike").onclick = function () { toggle("strike"); };
    $("wrap").onclick = function () { toggle("wrap"); };
    $("font-name").onchange = function (e) { format({ fontName: e.target.value }); };
    $("font-size").onchange = function (e) { format({ fontSize: parseFloat(e.target.value) }); };
    $("font-color").onchange = function (e) { format({ fontColor: e.target.value.slice(1).toUpperCase() }); };
    $("fill-color").onchange = function (e) { format({ fill: e.target.value.slice(1).toUpperCase() }); };
    $("no-fill").onclick = function () { format({ fill: "" }); };
    $("border").onchange = function (e) {
      if (e.target.value) border(e.target.value);
      e.target.value = "";
    };
    ["left", "center", "right"].forEach(function (k) { $("h-" + k).onclick = function () { format({ hAlign: cur && cur.style.hAlign === k ? "general" : k }); }; });
    ["top", "center", "bottom"].forEach(function (k) { $("v-" + k).onclick = function () { format({ vAlign: k }); }; });
    $("merge").onclick = mergeToggle;
    $("num-fmt").onchange = function (e) { format({ numFmt: e.target.value }); };
    $("ins-row").onclick = function () { rowsCols("insRow"); };
    $("del-row").onclick = function () { rowsCols("delRow"); };
    $("ins-col").onclick = function () { rowsCols("insCol"); };
    $("del-col").onclick = function () { rowsCols("delCol"); };

    // 파일
    $("file").addEventListener("change", function (e) {
      if (e.target.files[0]) openFile(e.target.files[0]);
    });
    $("new").onclick = newDoc;
    $("save-cell").onclick = function () { save(0, ".cell", "application/octet-stream"); };
    $("save-bin").onclick = function () { save(2, ".bin.cell", "application/octet-stream"); };
    $("save-xlsx").onclick = function () {
      save(1, ".xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
    };
    var drop = $("drop");
    ["dragover", "dragenter"].forEach(function (ev) {
      document.addEventListener(ev, function (e) { e.preventDefault(); drop.classList.add("over"); });
    });
    document.addEventListener("dragleave", function () { drop.classList.remove("over"); });
    document.addEventListener("drop", function (e) {
      e.preventDefault();
      drop.classList.remove("over");
      if (e.dataTransfer.files[0]) openFile(e.dataTransfer.files[0]);
    });
    window.addEventListener("beforeunload", function (e) {
      if (doc && doc.info.canUndo) {
        e.preventDefault();
        e.returnValue = "";
      }
    });
  }

  function isField(el) {
    return el && /^(INPUT|SELECT|TEXTAREA)$/.test(el.tagName);
  }

  function openFile(file) {
    file.arrayBuffer().then(function (buf) { openBytes(file.name, new Uint8Array(buf)); });
  }

  // 자동 시험(브라우저 스모크 시험)용 손잡이
  window.rcellEditor = {
    ready: false,
    edit: function (cmd) { return edit(cmd); },
    select: function (ref) {
      var t = ref.split(":");
      var a = parseA1(t[0]), b = t[1] ? parseA1(t[1]) : a;
      setActive(a.r, a.c, false);
      if (t[1]) setActive(b.r, b.c, true);
    },
    type: function (text) {
      startEdit("", false);
      editor().value = text;
      editing.dirty = true;
      return commitEdit(1, 0);
    },
    cell: function (ref) {
      var a = parseA1(ref);
      return json(take(wasm.rcell_doc_cell(doc.id, sheet, a.r, a.c)));
    },
    copy: function (cut) { return copySel(cut); },
    paste: function (text) { pasteAt(text == null ? null : text); },
    undo: function () { undo(false); },
    redo: function () { undo(true); },
    save: function (fmt) {
      var r = take(wasm.rcell_doc_save(doc.id, fmt));
      return r.ok ? r.body : null;
    },
    info: function () { return doc && doc.info; },
    sheet: function () { return sheet; },
    switchSheet: function (i) { switchSheet(i); },
    newDoc: newDoc,
  };

  status("엔진 불러오는 중…");
  wire();
  fetch("rcell.wasm")
    .then(function (r) { return r.arrayBuffer(); })
    .then(function (b) { return WebAssembly.instantiate(b, {}); })
    .then(function (m) {
      wasm = m.instance.exports;
      tick();
      $("rc-css").textContent = utf8.decode(take(wasm.rcell_css()).body);
      status("");
      window.rcellEditor.ready = true;
      // ?file=경로 로 바로 열기 (데모/시험용)
      var q = new URLSearchParams(location.search).get("file");
      if (q) {
        fetch(q).then(function (r) { return r.arrayBuffer(); })
          .then(function (b) { openBytes(q.split("/").pop(), new Uint8Array(b)); })
          .catch(function (e) { status(String(e), true); });
      }
    })
    .catch(function (e) { status("rcell.wasm 을 불러오지 못했습니다: " + e, true); });
})();
