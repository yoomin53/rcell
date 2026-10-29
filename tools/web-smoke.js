// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 웹 편집기 스모크 시험: web/ 를 작은 정적 서버로 띄우고 헤드리스 Edge(또는 Chrome)를 원격 디버깅(CDP)으로
// 조종해 데모 문서를 연 뒤, 마우스·키보드로 칸을 고르고 입력하고(수식 포함), 도구 막대로 서식을 바꾸고, 행을
// 넣고, 되돌리고, 다시 계산을 확인하고, .cell 로 저장한다. 저장한 파일은 rcell 로 다시 읽어 확인한다.
//
// 사용: node tools/web-smoke.js [브라우저 경로] [결과 폴더]
//   (먼저 sh tools/build-web.sh 로 web/rcell.wasm 을 만들고, cargo build --release 로 rcell 을 만든다)
// 의존성 없음 (Node 22 의 내장 WebSocket).
"use strict";
const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const { spawn, execFileSync } = require("child_process");

const root = path.resolve(__dirname, "..");
const web = path.join(root, "web");
const browser =
  process.argv[2] ||
  [
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "/usr/bin/microsoft-edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ].find((p) => fs.existsSync(p));
const outDir = process.argv[3] || fs.mkdtempSync(path.join(os.tmpdir(), "rcell-smoke-"));
const rcell = path.join(root, "target", "release", process.platform === "win32" ? "rcell.exe" : "rcell");
fs.mkdirSync(outDir, { recursive: true });

const results = [];
function check(name, ok, detail) {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? "통과" : "실패"}  ${name}${detail !== undefined ? "  — " + JSON.stringify(detail) : ""}`);
}

// ------------------------------------------------------------------ 정적 서버
const types = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".wasm": "application/wasm", ".cell": "application/octet-stream" };
const server = http.createServer((req, res) => {
  const u = decodeURIComponent(new URL(req.url, "http://x").pathname);
  const p = path.join(web, u === "/" ? "index.html" : u);
  if (!p.startsWith(web) || !fs.existsSync(p)) {
    res.writeHead(404);
    res.end();
    return;
  }
  res.writeHead(200, { "content-type": types[path.extname(p)] || "application/octet-stream" });
  fs.createReadStream(p).pipe(res);
});

// ------------------------------------------------------------------ CDP
class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.id = 0;
    this.wait = new Map();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && this.wait.has(msg.id)) {
        const { ok, fail } = this.wait.get(msg.id);
        this.wait.delete(msg.id);
        msg.error ? fail(new Error(msg.error.message)) : ok(msg.result);
      }
    };
  }
  send(method, params = {}) {
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((ok, fail) => this.wait.set(id, { ok, fail }));
  }
  async eval(expr) {
    const r = await this.send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text + " " + JSON.stringify(r.exceptionDetails.exception));
    return r.result.value;
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(cdp, expr, ms = 15000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try {
      if (await cdp.eval(expr)) return true;
    } catch (_) {}
    await sleep(100);
  }
  return false;
}

async function key(cdp, k, opts = {}) {
  const map = { Enter: 13, Tab: 9, Escape: 27, ArrowDown: 40, ArrowUp: 38, ArrowLeft: 37, ArrowRight: 39, Delete: 46, F2: 113 };
  const code = map[k] || (k.length === 1 ? k.toUpperCase().charCodeAt(0) : 0);
  const mods = (opts.ctrl ? 2 : 0) | (opts.shift ? 8 : 0);
  const base = { key: k, code: k.length === 1 ? "Key" + k.toUpperCase() : k, windowsVirtualKeyCode: code, modifiers: mods };
  await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
  await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

async function typeText(cdp, text) {
  for (const ch of text) {
    await cdp.send("Input.dispatchKeyEvent", { type: "keyDown", text: ch, unmodifiedText: ch, key: ch });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: ch });
  }
}

// 칸 가운데의 화면 좌표
async function cellPoint(cdp, ref) {
  return cdp.eval(`(() => {
    const m = /^([A-Z]+)(\\d+)$/.exec(${JSON.stringify(ref)});
    let c = 0; for (const ch of m[1]) c = c * 26 + ch.charCodeAt(0) - 64; c -= 1;
    const r = +m[2] - 1;
    const canvas = document.querySelector('#area .rc-canvas');
    const rect = canvas.getBoundingClientRect();
    const tr = canvas.querySelector('tr[data-r="' + r + '"]');
    const cols = [...canvas.querySelectorAll('col[data-c]')];
    let x = 0, w = 0, acc = canvas.querySelector('col:not([data-c])') ? parseFloat(canvas.querySelector('col:not([data-c])').style.width) : 0;
    for (const col of cols) { const cw = parseFloat(col.style.width); if (+col.dataset.c === c) { x = acc; w = cw; break; } acc += cw; }
    return { x: rect.left + x + w / 2, y: tr.getBoundingClientRect().top + tr.getBoundingClientRect().height / 2 };
  })()`);
}

async function click(cdp, ref, opts = {}) {
  const p = await cellPoint(cdp, ref);
  const ev = { x: p.x, y: p.y, button: "left", clickCount: opts.double ? 2 : 1, modifiers: opts.shift ? 8 : 0 };
  await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", ...ev });
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", ...ev });
  await sleep(50);
}

async function main() {
  if (!browser) throw new Error("Edge/Chrome 을 찾지 못했습니다 — 경로를 첫 인수로 주세요");
  if (!fs.existsSync(path.join(web, "rcell.wasm"))) throw new Error("web/rcell.wasm 이 없습니다 — sh tools/build-web.sh");
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "rcell-smoke-profile-"));
  const dbg = 9200 + Math.floor(Math.random() * 500);
  const proc = spawn(browser, [
    "--headless=new", `--remote-debugging-port=${dbg}`, `--user-data-dir=${profile}`,
    "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--window-size=1400,900", "about:blank",
  ], { stdio: "ignore" });
  let cdp;
  try {
    let target;
    for (let i = 0; i < 100 && !target; i++) {
      try {
        const list = await (await fetch(`http://127.0.0.1:${dbg}/json/list`)).json();
        target = list.find((t) => t.type === "page");
      } catch (_) {
        await sleep(200);
      }
    }
    if (!target) throw new Error("브라우저 원격 디버깅에 연결하지 못했습니다");
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
    cdp = new Cdp(ws);
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${port}/index.html?file=samples/demo.cell` });
    check("편집기와 wasm 을 불러온다", await until(cdp, "window.rcellEditor && window.rcellEditor.ready"));
    check("데모 문서를 연다", await until(cdp, "!!document.querySelector('#area .rc-canvas') && !!rcellEditor.info()"));
    const info = await cdp.eval("rcellEditor.info()");
    check("시트 탭", info.sheets.length >= 1, info.sheets.map((s) => s.name));

    // 1. 마우스로 칸을 고르고 값을 입력한다 (B4 = 1250 → 9999)
    const before = await cdp.eval("rcellEditor.cell('F4')");
    await click(cdp, "B4");
    check("칸 고르기 (마우스)", (await cdp.eval("document.getElementById('namebox').value")) === "B4");
    await typeText(cdp, "9999");
    await key(cdp, "Enter");
    await sleep(100);
    const b4 = await cdp.eval("rcellEditor.cell('B4')");
    check("값 입력 (키보드)", b4.input === "9999", b4.input);
    const after = await cdp.eval("rcellEditor.cell('F4')");
    check("다시 계산 (F4 = SUM(B4:E4))", after.display !== before.display, [before.display, after.display]);
    const nb = await cdp.eval("document.getElementById('namebox').value");
    check("Enter 로 아래 칸으로", nb === "B5", nb);

    // 2. 수식 입력 (소문자·공백 → 한셀 꼴)
    await click(cdp, "H4");
    await typeText(cdp, "=sum(b4:e4) * 2");
    await key(cdp, "Enter");
    await sleep(100);
    const h4 = await cdp.eval("rcellEditor.cell('H4')");
    check("수식 입력과 고르기", h4.input === "=SUM(B4:E4)*2", h4.input);
    check("수식 값", Number(String(h4.display).replace(/,/g, "")) === 2 * Number(String(after.display).replace(/,/g, "")), h4.display);

    // 3. 날짜·백분율·글자 접두
    await click(cdp, "H5");
    await typeText(cdp, "2026-10-23");
    await key(cdp, "Tab");
    await typeText(cdp, "12.5%");
    await key(cdp, "Tab");
    await typeText(cdp, "'00123");
    await key(cdp, "Enter");
    await sleep(100);
    const h5 = await cdp.eval("rcellEditor.cell('H5')"), i5 = await cdp.eval("rcellEditor.cell('I5')"), j5 = await cdp.eval("rcellEditor.cell('J5')");
    check("날짜 입력", h5.display === "2026-10-23" && h5.style.numFmt === "yyyy-mm-dd", [h5.display, h5.style.numFmt]);
    check("백분율 입력", i5.display === "12.50%", i5.display);
    check("작은따옴표 글", j5.display === "00123" && j5.input === "'00123", [j5.display, j5.input]);

    // 4. 도구 막대: 굵게 (범위 선택 → 단추)
    await click(cdp, "H4");
    await click(cdp, "J5", { shift: true });
    await cdp.eval("document.getElementById('bold').click()");
    await sleep(100);
    check("굵게 (도구 막대)", (await cdp.eval("rcellEditor.cell('I4')")).style.bold === true);

    // 5. 행 삽입 → 수식 참조가 따라간다
    await click(cdp, "A3");
    await cdp.eval("document.getElementById('ins-row').click()");
    await sleep(100);
    const h5b = await cdp.eval("rcellEditor.cell('H5')");
    check("행 삽입과 참조 조정", h5b.input === "=SUM(B5:E5)*2", h5b.input);

    // 6. 되돌리기·다시 실행 (Ctrl+Z / Ctrl+Y)
    await click(cdp, "C10");
    await key(cdp, "z", { ctrl: true });
    await sleep(100);
    check("되돌리기", (await cdp.eval("rcellEditor.cell('H4')")).input === "=SUM(B4:E4)*2");
    await key(cdp, "y", { ctrl: true });
    await sleep(100);
    check("다시 실행", (await cdp.eval("rcellEditor.cell('H5')")).input === "=SUM(B5:E5)*2");

    // 7. 복사·붙여넣기 (앱 안) → 상대 참조가 옮겨진다
    await cdp.eval("rcellEditor.select('H5'); rcellEditor.copy(false); rcellEditor.select('H6'); rcellEditor.paste(null)");
    await sleep(100);
    const h6 = await cdp.eval("rcellEditor.cell('H6')");
    check("복사·붙여넣기 (상대 참조)", h6.input === "=SUM(B6:E6)*2", h6.input);
    // 다른 프로그램에서 온 글(TSV)
    await cdp.eval("rcellEditor.select('L1'); rcellEditor.paste('1\\t2\\n3\\t=L1+M1+L2')");
    await sleep(100);
    const m2 = await cdp.eval("rcellEditor.cell('M2')");
    check("TSV 붙여넣기", m2.input === "=L1+M1+L2" && m2.display === "6", [m2.input, m2.display]);

    // 8. Ctrl+화살표, Delete
    await click(cdp, "L1");
    await key(cdp, "ArrowDown", { ctrl: true });
    check("Ctrl+↓", (await cdp.eval("document.getElementById('namebox').value")) === "L2");
    await key(cdp, "Delete");
    await sleep(100);
    check("Delete 로 지우기", (await cdp.eval("rcellEditor.cell('L2')")).input === "");

    const shot1 = await cdp.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(outDir, "smoke-edit.png"), Buffer.from(shot1.data, "base64"));

    // 9. 시트 추가
    await cdp.eval("rcellEditor.edit({op:'addSheet', name:'메모'})");
    const info2 = await cdp.eval("rcellEditor.info()");
    check("시트 추가", info2.sheets.some((s) => s.name === "메모"), info2.sheets.map((s) => s.name));

    // 10. 저장 → rcell 로 다시 읽기
    const b64 = await cdp.eval("(() => { const b = rcellEditor.save(0); let s = ''; for (let i = 0; i < b.length; i += 8192) s += String.fromCharCode.apply(null, b.subarray(i, i + 8192)); return btoa(s); })()");
    const saved = path.join(outDir, "smoke.cell");
    fs.writeFileSync(saved, Buffer.from(b64, "base64"));
    check("저장 (.cell)", fs.statSync(saved).size > 1000, fs.statSync(saved).size);
    for (const fmt of [1, 2]) {
      const n = await cdp.eval(`rcellEditor.save(${fmt}).length`);
      check(`저장 (${fmt === 1 ? ".xlsx" : ".bin.cell"})`, n > 1000, n);
    }
    if (fs.existsSync(rcell)) {
      const csv = execFileSync(rcell, ["csv", saved, "-s", "0"]).toString("utf8");
      // rcell csv 는 한셀의 CSV 저장처럼 표시된 글을 쓴다 (#,##0 → "9,999")
      check("저장한 문서를 rcell 로 다시 읽기", /9,?999/.test(csv), csv.split("\n")[4]);
      const calc = spawnSyncText(rcell, ["calc", saved]);
      check("저장한 값 = rcell 다시 계산", calc.status === 0, calc.out.trim().split("\n").pop());
    }
    const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(outDir, "smoke.png"), Buffer.from(shot.data, "base64"));
  } finally {
    try { if (cdp) await cdp.send("Browser.close"); } catch (_) {}
    try { proc.kill(); } catch (_) {}
    server.close();
  }
  const fail = results.filter((r) => !r.ok).length;
  console.log(`\n웹 편집기 스모크 시험: ${results.length - fail}/${results.length} 통과 (결과: ${outDir})`);
  process.exit(fail ? 1 : 0);
}

function spawnSyncText(cmd, args) {
  const r = require("child_process").spawnSync(cmd, args, { encoding: "utf8" });
  return { status: r.status, out: (r.stdout || "") + (r.stderr || "") };
}

main().catch((e) => {
  console.error("오류:", e.message);
  try { server.close(); } catch (_) {}
  process.exit(2);
});
