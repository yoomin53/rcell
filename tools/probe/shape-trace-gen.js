// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 도형 윤곽 따라 그리기 (2/3): node shape-trace-gen.js <out.rs> [작업 폴더] — 작업 폴더의 trace/*.txt (shape-trace.ps1 출력),
// shape-trace-specs.json, avdefaults.json (shape-avdefaults.js) → src/render/shape_trace.rs
// 모양마다: 이름(prst), 파일, 방식 (bbox: 상자 비율, icon: 가운데·ss 비율 + 바탕 사각형), 제외할 종류
const fs = require("fs");
const dir = process.argv[3] || __dirname; // 작업 폴더: trace/*.txt·avdefaults.json
const SPECS = JSON.parse(fs.readFileSync(__dirname + "/shape-trace-specs.json", "utf8"));
const Q = { actionButtonHome: 0, actionButtonHelp: 1, actionButtonInformation: 2, actionButtonReturn: 3, actionButtonDocument: 4, actionButtonSound: 5, actionButtonMovie: 6, flowChartMagneticDrum: 7, flowChartMagneticTape: 0, verticalScroll: 1, horizontalScroll: 2, ribbon2: 3, ribbon: 4, ellipseRibbon2: 5, ellipseRibbon: 6, gear6: 7, gear9: 0, curvedRightArrow: 1, curvedLeftArrow: 2, curvedUpArrow: 3, curvedDownArrow: 4, irregularSeal1: 5, irregularSeal2: 6, leftRightRibbon: 7, mathNotEqual: 0, uturnArrow: 1, funnel: 2, quadArrow: 3, leftRightUpArrow: 4, bentArrow: 5, sun: 6, swooshArrow: 7, leftCircularArrow: 0, circularArrow: 1, leftRightCircularArrow: 2, wave: 3, noSmoking: 4, cloudCallout: 5, cloud: 6, star5: 0, heart: 1, bevel: 2, star7: 3, doubleWave: 4, star6: 5, dodecagon: 6, heptagon: 7, decagon: 0, leftUpArrow: 1, moon: 2, lightningBolt: 3, star10: 4, smileyFace: 5, squareTabs: 0, plaqueTabs: 1, cornerTabs: 2, mathDivide: 3, flowChartMultidocument: 4, wedgeRectCallout: 5, wedgeRoundRectCallout: 6, wedgeEllipseCallout: 7, mathEqual: 4, flowChartOnlineStorage: 5 };
const CLASSES = { norm: "Norm", darkenLess: "DarkenLess", darken: "Darken", lighten: "Lighten", lightenLess: "LightenLess" };
const area = (r) => r.reduce((s, p, i) => { const q = r[(i + 1) % r.length]; return s + p[0] * q[1] - q[0] * p[1]; }, 0) / 2;
const fmt = (v) => { const s = (Math.round(v * 10000) / 10000).toString(); return s.includes(".") || s.includes("e") ? s : s + ".0"; };
let out = `// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 한셀 화면에서 따라 그린 도형 윤곽 (큰 크기로 그린 도형의 화면을 칠 종류별로 벡터화한 값).
//! 좌표는 \`bbox\` 방식이면 상자의 (가로 비율, 세로 비율), \`icon\` 방식이면 상자 가운데에서 \`ss\`(짧은 변) 비율.
//! 생성: tools/probe/shape-trace-gen.js (손으로 고치지 않는다)
// 화면에서 잰 좌표라 우연히 상수와 비슷한 값이 있다
#![allow(clippy::approx_constant)]

use super::shape::PathFill;

/// 같은 칠을 가진 윤곽 묶음 (구멍은 반대 방향 고리)
pub struct Layer {
    pub fill: PathFill,
    pub stroke: bool,
    pub rings: &'static [&'static [(f32, f32)]],
}

/// 가로세로 비율을 따르는 윤곽: 꼭짓점마다 (x = ax·w + bx·ss, y = ay·h + by·ss) 계수 (ss = 짧은 변)
pub struct LayerLin {
    pub fill: PathFill,
    pub stroke: bool,
    pub rings: &'static [&'static [(f32, f32, f32, f32)]],
}

/// 윤곽 방식: 좌표를 상자 비율로 보나 (\`false\`), 가운데·ss 비율로 보나 (\`true\`). \`lin\` 이 있으면 그것을 쓴다
pub struct Traced {
    pub icon: bool,
    pub layers: &'static [Layer],
    pub lin: &'static [LayerLin],
    /// 한셀이 쓰는 기본 조정값 (이름, 값) — 문서의 조정값이 모두 이와 같을 때만 이 윤곽을 쓴다
    pub adj: &'static [(&'static str, f64)],
}
`;
out += "\nconst NO_LAYERS: &[Layer] = &[];\nconst NO_LIN: &[LayerLin] = &[];\n";
const AV = JSON.parse(fs.readFileSync(dir + "/avdefaults.json", "utf8"));
const FIT = fs.existsSync(dir + "/fit.json") ? JSON.parse(fs.readFileSync(dir + "/fit.json", "utf8")) : {};
const LCLASS = { norm: "Norm", darkenLess: "DarkenLess", darken: "Darken", lighten: "Lighten", lightenLess: "LightenLess" };
const tables = [];
for (const sp of SPECS) {
  const txt = fs.readFileSync(`${dir}/trace/${sp.file}.txt`, "utf8").split(/\r?\n/);
  const classes = [];
  let cur = null;
  for (const l of txt) {
    if (!l) continue;
    if (l.startsWith("R ")) {
      const pts = l.slice(2).trim().split(/\s+/).map((p) => p.split(",").map(Number));
      cur.rings.push(pts);
    } else {
      const m = l.match(/^(\w+)( S)? (\d+)( .*)?$/);
      cur = { name: m[1], stroked: !!m[2], rings: [] };
      classes.push(cur);
    }
  }
  // 상자 (영역 좌표): 지정하지 않으면 선 고리 전체의 테두리 (선 가운데 줄), 선이 없으면 모든 고리
  let box = sp.box;
  // 타일 안 자리: 영역은 (상자 − 10px) 에서 시작하고, 한셀이 도형을 정수 px 로 맞춘 어긋남은 열 [0,1,0,0], 행 [0,1]
  if (!box && Q[sp.prst] !== undefined) {
    const q = Q[sp.prst];
    box = [10 + [0, 1, 0, 0][q % 4], 10 + [0, 1][Math.floor(q / 4)], 320, 320];
  }
  if (!box) {
    const src = classes.find((c) => c.name === "stroke" && c.rings.length) || null;
    const all = src ? src.rings : classes.flatMap((c) => c.rings);
    const xs = all.flat().map((p) => p[0]), ys = all.flat().map((p) => p[1]);
    const adj = src ? 1 : 0; // 선 고리의 테두리는 선 굵기(1px)만큼 크게 잡힌다
    const w = Math.round(Math.max(...xs) - Math.min(...xs)) - adj, h = Math.round(Math.max(...ys) - Math.min(...ys)) - adj;
    const mx = (Math.max(...xs) + Math.min(...xs)) / 2, my = (Math.max(...ys) + Math.min(...ys)) / 2;
    box = [mx - w / 2, my - h / 2, w, h];
  }
  const [bx, by, bw, bh] = box;
  if (process.env.SHOWBOX) console.log(sp.prst, box.map((v) => Math.round(v * 100) / 100).join(" "));
  const ss = Math.min(bw, bh);
  const cx = bx + bw / 2, cy = by + bh / 2;
  const norm = (p) => (sp.icon ? [(p[0] - cx) / ss, (p[1] - cy) / ss] : [(p[0] - bx) / bw, (p[1] - by) / bh]);
  const layers = [];
  for (const c of classes) {
    if (!CLASSES[c.name] || !c.rings.length) continue;
    if ((sp.skip || []).includes(c.name)) continue;
    let rings = c.rings;
    // 가장 큰 고리의 방향이 바깥, 반대는 구멍 (norm 의 구멍은 버린다 — 그 위에 다른 칠이 덮는다)
    const big = rings.reduce((a, b) => (Math.abs(area(a)) >= Math.abs(area(b)) ? a : b));
    const sgn = Math.sign(area(big));
    if (sp.dropHoles) rings = rings.filter((r) => Math.sign(area(r)) === sgn);
    if (sp.minArea) rings = rings.filter((r) => Math.abs(area(r)) >= sp.minArea);
    if (!rings.length) continue;
    layers.push({ fill: CLASSES[c.name], stroke: c.stroked || sp.stroke === true, rings: rings.map((r) => r.map(norm)) });
  }
  const id = sp.prst.replace(/[^A-Za-z0-9]/g, "_").toUpperCase();
  let s = "";
  const lin = FIT[sp.prst];
  if (lin) {
    // 가로세로 비율을 따르는 윤곽 (shape-trace-fit.js 가 푼 계수)
    lin.forEach((l, i) => {
      s += `const ${id}_N${i}: &[&[(f32, f32, f32, f32)]] = &[\n`;
      for (const r of l.rings) s += `    &[${r.map((p) => `(${p.map(fmt).join(", ")})`).join(", ")}],\n`;
      s += `];\n`;
    });
    s += `const ${id}_LIN: &[LayerLin] = &[\n` + lin.map((l, i) => `    LayerLin { fill: PathFill::${LCLASS[l.name]}, stroke: ${l.stroked}, rings: ${id}_N${i} },`).join("\n") + `\n];\n`;
    const adj2 = (AV[sp.prst] || [[]])[0];
    s += `const ${id}_ADJ: &[(&str, f64)] = &[${adj2.map((g) => `("${g[0]}", ${g[1]}.0)`).join(", ")}];\n`;
    out += "\n" + s;
    tables.push({ prst: sp.prst, id, icon: false, lin: true });
    continue;
  }
  layers.forEach((l, i) => {
    s += `const ${id}_L${i}: &[&[(f32, f32)]] = &[\n`;
    for (const r of l.rings) s += `    &[${r.map((p) => `(${fmt(p[0])}, ${fmt(p[1])})`).join(", ")}],\n`;
    s += `];\n`;
  });
  s += `const ${id}: &[Layer] = &[\n` + layers.map((l, i) => `    Layer { fill: PathFill::${l.fill}, stroke: ${l.stroke}, rings: ${id}_L${i} },`).join("\n") + `\n];\n`;
  const adj = (AV[sp.prst] || [[]])[0];
  s += `const ${id}_ADJ: &[(&str, f64)] = &[${adj.map((g) => `("${g[0]}", ${g[1]}.0)`).join(", ")}];\n`;
  out += "\n" + s;
  tables.push({ prst: sp.prst, id, icon: !!sp.icon });
}
out += `\n/// 이름으로 찾는다\npub fn traced(prst: &str) -> Option<Traced> {\n    let t = match prst {\n`;
for (const t of tables) out += `        "${t.prst}" => (${t.lin ? "NO_LAYERS" : t.id}, ${t.lin ? t.id + "_LIN" : "NO_LIN"}, ${t.icon}, ${t.id}_ADJ),\n`;
out += `        _ => return None,\n    };\n    Some(Traced {
        icon: t.2,
        layers: t.0,
        lin: t.1,
        adj: t.3,
    })\n}\n`;
fs.writeFileSync(process.argv[2], out);
console.log("ok", tables.length);
