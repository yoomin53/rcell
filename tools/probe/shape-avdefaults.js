// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 도형 윤곽 따라 그리기 (보조): node shape-avdefaults.js <작업 폴더> — 큰 모양 문서를 풀어 둔 폴더(shp2-x·shp3-x·shp-x/xl/drawings)에서
// 한셀이 도형마다 적는 기본 조정값을 모아 <작업 폴더>/avdefaults.json 으로 쓴다
const fs = require("fs");
const dir = process.argv[2] || __dirname;
const res = {};
for (const [d, n] of [["shp2-x", 5], ["shp3-x", 4], ["shp-x", 5]]) {
  for (let k = 0; k < n; k++) {
    const x = fs.readFileSync(`${dir}/${d}/xl/drawings/drawing${k}.xml`, "utf8");
    for (const m of x.matchAll(/<a:prstGeom prst="([^"]+)">(<a:avLst\/>|<a:avLst>(.*?)<\/a:avLst>)/g)) {
      const gds = m[3] ? [...m[3].matchAll(/<a:gd name="([^"]+)" fmla="val (-?\d+)"/g)].map((g) => [g[1], +g[2]]) : [];
      const key = m[1];
      const s = JSON.stringify(gds);
      (res[key] ||= new Set()).add(s);
    }
  }
}
const out = {};
for (const k in res) out[k] = [...res[k]].map((s) => JSON.parse(s));
fs.writeFileSync(dir + "/avdefaults.json", JSON.stringify(out));
for (const k of Object.keys(out).sort()) console.log(k.padEnd(28), out[k].map((a) => a.map((g) => g.join("=")).join(",") || "-").join(" | "));
