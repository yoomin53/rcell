// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 헤드리스 Edge 하나를 띄워 DevTools 프로토콜로 HTML 여러 개를 차례로 그린다 (visual-diff.ps1 -Cdp).
// 갈무리마다 Edge 를 새로 띄우면 시작·끝에 몇 분씩 걸릴 때가 있어서다.
//
//   node cdp-shot.js <msedge.exe> <프로필 폴더> <목록 TSV>
//
// 목록 줄: HTML 경로<TAB>PNG 경로<TAB>폭<TAB>높이. 다 그린 줄마다 "ok <PNG>" 를 쓴다.
"use strict";
const fs = require("fs");
const path = require("path");
const { spawn } = require("child_process");

const [edge, profile, list] = process.argv.slice(2);
const jobs = fs
  .readFileSync(list, "utf8")
  .replace(/^﻿/, "")
  .split(/\r?\n/)
  .filter(Boolean)
  .map((l) => l.split("\t"));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  fs.mkdirSync(profile, { recursive: true });
  const portFile = path.join(profile, "DevToolsActivePort");
  fs.rmSync(portFile, { force: true });
  const proc = spawn(
    edge,
    [
      "--headless=new",
      "--disable-gpu",
      "--hide-scrollbars",
      "--no-first-run",
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-sync",
      "--disable-extensions",
      "--enable-features=NetworkServiceInProcess2,NetworkServiceInProcess",
      "--force-device-scale-factor=1",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  // 포트 파일: 첫 줄 포트, 둘째 줄 브라우저 경로
  let port, wsPath;
  for (let i = 0; i < 6000 && !wsPath; i++) {
    try {
      const t = fs.readFileSync(portFile, "utf8").split(/\r?\n/);
      if (t[1]) [port, wsPath] = t;
    } catch {}
    if (!wsPath) await sleep(100);
  }
  if (!wsPath) throw new Error("Edge DevTools 포트를 못 찾음");
  const ws = new WebSocket(`ws://127.0.0.1:${port}${wsPath}`);
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = rej;
  });
  let id = 0;
  const pending = new Map();
  const waiters = [];
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { res, rej } = pending.get(m.id);
      pending.delete(m.id);
      if (m.error) rej(new Error(m.error.message));
      else res(m.result);
    } else if (m.method) {
      for (let i = waiters.length - 1; i >= 0; i--) {
        const w = waiters[i];
        if (w.method === m.method && w.session === m.sessionId) {
          waiters.splice(i, 1);
          w.res(m.params);
        }
      }
    }
  };
  const send = (method, params = {}, sessionId) =>
    new Promise((res, rej) => {
      const msg = { id: ++id, method, params };
      if (sessionId) msg.sessionId = sessionId;
      pending.set(msg.id, { res, rej });
      ws.send(JSON.stringify(msg));
    });
  const once = (method, session) => new Promise((res) => waiters.push({ method, session, res }));

  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId: s } = await send("Target.attachToTarget", { targetId, flatten: true });
  await send("Page.enable", {}, s);
  for (const [html, png, w, h] of jobs) {
    await send(
      "Emulation.setDeviceMetricsOverride",
      { width: +w, height: +h, deviceScaleFactor: 1, mobile: false },
      s,
    );
    const loaded = once("Page.loadEventFired", s);
    await send("Page.navigate", { url: "file:///" + html.replace(/\\/g, "/") }, s);
    await loaded;
    await send(
      "Runtime.evaluate",
      { expression: "document.fonts.ready.then(() => 1)", awaitPromise: true },
      s,
    );
    // 그리기 한 번 기다린다
    await send(
      "Runtime.evaluate",
      {
        expression: "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(1))))",
        awaitPromise: true,
      },
      s,
    );
    const { data } = await send(
      "Page.captureScreenshot",
      { format: "png", clip: { x: 0, y: 0, width: +w, height: +h, scale: 1 } },
      s,
    );
    fs.writeFileSync(png, Buffer.from(data, "base64"));
    console.log("ok " + png);
  }
  try {
    await send("Browser.close");
  } catch {}
  ws.close();
  setTimeout(() => {
    try {
      proc.kill();
    } catch {}
    process.exit(0);
  }, 2000);
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
