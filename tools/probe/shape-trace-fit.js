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
// ---- 호가 있는 윤곽: 꼭짓점 맞추기 ----
const CORNER_DEG = 40;
function turnDeg(p, q, r) {
  const a1 = Math.atan2(q[1] - p[1], q[0] - p[0]), a2 = Math.atan2(r[1] - q[1], r[0] - q[0]);
  let d = ((a2 - a1) * 180) / Math.PI;
  while (d > 180) d -= 360;
  while (d < -180) d += 360;
  return Math.abs(d);
}
function cornerIdx(r) {
  const n = r.length, out = [];
  for (let i = 0; i < n; i++) if (turnDeg(r[(i + n - 1) % n], r[i], r[(i + 1) % n]) > CORNER_DEG) out.push(i);
  return out;
}
// 폴리선을 호 길이 기준 같은 간격 n 개 (양 끝 뺀 안쪽 점)로
function resample(pts, n) {
  if (n === 0) return [];
  const cum = [0];
  for (let i = 1; i < pts.length; i++) cum.push(cum[i - 1] + Math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]));
  const total = cum[cum.length - 1] || 1;
  const out = [];
  for (let k = 1; k <= n; k++) {
    const t = (total * k) / (n + 1);
    let j = 1;
    while (j < cum.length - 1 && cum[j] < t) j++;
    const f = (t - cum[j - 1]) / ((cum[j] - cum[j - 1]) || 1);
    out.push([pts[j - 1][0] + (pts[j][0] - pts[j - 1][0]) * f, pts[j - 1][1] + (pts[j][1] - pts[j - 1][1]) * f]);
  }
  return out;
}
function align3(a, b, d) {
  const rings = [a, b, d];
  const cs = rings.map(cornerIdx);
  if (!(cs[0].length === cs[1].length && cs[1].length === cs[2].length)) return null;
  if (cs[0].length === 0) {
    // 매끈한 닫힌 곡선: 가장 왼쪽(같으면 위) 점에서 시작하는 하나의 마디
    const rot = rings.map((r) => {
      let s = 0;
      r.forEach((p, i) => { if (p[0] < r[s][0] - 1e-9 || (Math.abs(p[0] - r[s][0]) < 1e-9 && p[1] < r[s][1])) s = i; });
      return r.slice(s).concat(r.slice(0, s)).concat([r[s]]);
    });
    const n = Math.max(...rot.map((r) => r.length - 2));
    return rot.map((r) => [r[0]].concat(resample(r, n)));
  }
  // 가장 위(같으면 왼쪽) 꼭짓점에서 시작
  const rot = rings.map((r, j) => {
    let s = cs[j][0];
    for (const i of cs[j]) if (r[i][1] < r[s][1] - 1e-9 || (Math.abs(r[i][1] - r[s][1]) < 1e-9 && r[i][0] < r[s][0])) s = i;
    return r.slice(s).concat(r.slice(0, s));
  });
  const rc = rot.map(cornerIdx);
  const out = [[], [], []];
  const m = rc[0].length;
  for (let k = 0; k < m; k++) {
    const segs = rot.map((r, j) => {
      const i0 = rc[j][k], i1 = rc[j][(k + 1) % m];
      const seg = [];
      for (let i = i0; ; i = (i + 1) % r.length) { seg.push(r[i]); if (i === i1) break; }
      return seg;
    });
    const n = Math.max(...segs.map((s) => s.length - 2));
    segs.forEach((s, j) => { out[j].push(s[0]); for (const p of resample(s, Math.max(0, n))) out[j].push(p); });
  }
  return out;
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
      let [a, b, d] = [rs[0][i], rs[1][i], rs[2][i]];
      if (!(a.length === b.length && b.length === d.length)) {
        // 꼭짓점 수가 다르면 (호) 꼭짓점(꺾임 40도 넘는 점)끼리 맞추고 그 사이를 같은 수로 다시 뽑는다
        const al = align3(a, b, d);
        if (!al) { ok = false; break; }
        [a, b, d] = al;
      }
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
