// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 도형 윤곽 따라 그리기 (보조, 가로세로 비율): node shape-trace-fit.js <작업 폴더> <이름,이름…> —
// 같은 도형을 정사각형(320×320px)·가로로 긴(480×240)·세로로 긴(240×480) 크기로 따라 그린 윤곽(trace/이름.txt, W_이름.txt,
// T_이름.txt)에서 꼭짓점마다 x = ax·w + bx·ss, y = ay·h + by·ss (ss = 짧은 변) 의 계수를 푼다. 세 윤곽의 고리·꼭짓점 수가 같은
// 곧은 윤곽 도형만 된다. 결과는 <작업 폴더>/fit.json (shape-trace-gen.js 가 읽는다) 이고, 맞지 않는 도형은 목록으로 알린다.
const fs = require("fs");
const dir = process.argv[2];
const names = process.argv[3].split(",");
const CLASSES = ["norm", "darkenLess", "darken", "lighten", "lightenLess"];
const area = (r) => r.reduce((s, p, i) => { const q = r[(i + 1) % r.length]; return s + p[0] * q[1] - q[0] * p[1]; }, 0) / 2;
function parse(file) {
  const out = {};
  let cur = null;
  for (const l of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    if (!l) continue;
    if (l.startsWith("R ")) cur.rings.push(l.slice(2).trim().split(/\s+/).map((p) => p.split(",").map(Number)));
    else {
      const m = l.match(/^(\w+)( S)? (\d+)( .*)?$/);
      cur = { stroked: !!m[2], rings: [] };
      out[m[1]] = cur;
    }
  }
  return out;
}
// 선 고리 전체의 테두리 (선 굵기 1px 만큼 크게 잡히므로 뺀다)
function box(t) {
  const all = (t.stroke ? t.stroke.rings : Object.values(t).flatMap((c) => c.rings)).flat();
  const xs = all.map((p) => p[0]), ys = all.map((p) => p[1]);
  const adj = t.stroke ? 1 : 0;
  const w = Math.round(Math.max(...xs) - Math.min(...xs)) - adj, h = Math.round(Math.max(...ys) - Math.min(...ys)) - adj;
  return [(Math.max(...xs) + Math.min(...xs)) / 2 - w / 2, (Math.max(...ys) + Math.min(...ys)) / 2 - h / 2, w, h];
}
const result = {};
const bad = [];
for (const n of names) {
  const sets = [
    { t: parse(`${dir}/trace/${n}.txt`), W: 320, H: 320 },
    { t: parse(`${dir}/trace/W_${n}.txt`), W: 480, H: 240 },
    { t: parse(`${dir}/trace/T_${n}.txt`), W: 240, H: 480 },
  ];
  for (const s of sets) s.box = box(s.t);
  const layers = [];
  let ok = true, worst = 0;
  for (const c of CLASSES) {
    const rs = sets.map((s) => (s.t[c] ? s.t[c].rings.filter((r) => Math.abs(area(r)) >= 12) : []));
    if (rs.every((r) => r.length === 0)) continue;
    if (!(rs[0].length === rs[1].length && rs[1].length === rs[2].length)) { ok = false; break; }
    const rings = [];
    for (let i = 0; i < rs[0].length; i++) {
      const [a, b, d] = [rs[0][i], rs[1][i], rs[2][i]];
      if (!(a.length === b.length && b.length === d.length)) { ok = false; break; }
      const ring = [];
      for (let k = 0; k < a.length; k++) {
        // 상자 안 좌표 (px)
        const P = [a, b, d].map((r, j) => [r[k][0] - sets[j].box[0], r[k][1] - sets[j].box[1]]);
        // x = α·W + β·ss : 320(α+β)=x1, 480α+240β=x2, 240(α+β)=x3  → 최소제곱
        const fit = (v1, v2, v3, c2, c3) => {
          // 방정식 (계수 W 쪽, ss 쪽, 값): (320,320,v1) (c2.w, c2.s, v2) (c3.w, c3.s, v3)
          const rows = [[320, 320, v1], [c2[0], c2[1], v2], [c3[0], c3[1], v3]];
          let saa = 0, sab = 0, sbb = 0, sav = 0, sbv = 0;
          for (const [p, q, v] of rows) { saa += p * p; sab += p * q; sbb += q * q; sav += p * v; sbv += q * v; }
          const det = saa * sbb - sab * sab;
          const al = (sav * sbb - sbv * sab) / det, be = (saa * sbv - sab * sav) / det;
          const res = rows.map(([p, q, v]) => Math.abs(al * p + be * q - v));
          return [al, be, Math.max(...res)];
        };
        const fx = fit(P[0][0], P[1][0], P[2][0], [480, 240], [240, 240]);
        const fy = fit(P[0][1], P[1][1], P[2][1], [240, 240], [480, 240]);
        worst = Math.max(worst, fx[2], fy[2]);
        ring.push([fx[0], fx[1], fy[0], fy[1]]);
      }
      rings.push(ring);
    }
    if (!ok) break;
    layers.push({ name: c, stroked: sets[0].t[c].stroked, rings });
  }
  if (!ok || worst > 2.2) bad.push(`${n}${ok ? ` (오차 ${worst.toFixed(1)}px)` : " (꼭짓점 수가 다르다)"}`);
  else result[n] = layers;
  console.log(n.padEnd(22), ok ? `worst ${worst.toFixed(2)}px` : "mismatch");
}
fs.writeFileSync(`${dir}/fit.json`, JSON.stringify(result));
console.log("안 되는 도형:", bad.join(", ") || "-");
