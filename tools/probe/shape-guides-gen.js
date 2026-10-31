// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

// 가이드 수식 모양 표 만들기: node shape-guides-gen.js <presetShapeDefinitions.xml> <모양,모양,…> > src/render/shape_guides.rs
// ECMA-376 1부의 미리 정의된 모양 정의(표준 문서, 저장소에는 넣지 않는다)에서 고른 모양의 avLst·gdLst·pathLst 만 줄여 Rust 문자열로 쓴다.
// 정의가 한셀 화면과 맞는지는 모양마다 따로 대조한 것만 고른다 (tools/probe/shape-trace.ps1 의 갈무리와 visual-diff.ps1).
const fs = require("fs");
const [spec, list] = [fs.readFileSync(process.argv[2], "utf8"), process.argv[3].split(",")];
const squeeze = (x) =>
  x
    .replace(/<!--[^]*?-->/g, "")
    .replace(/\s+xmlns="[^"]*"/g, "")
    .replace(/\s+extrusionOk="[^"]*"/g, "")
    .replace(/\s*\/>/g, "/>")
    .replace(/>\s+</g, "><")
    .trim();
// 자주 나오는 글을 한 글자로 (크기를 3분의 1 쯤으로) — 읽을 때 shape_guides::definition 이 되돌린다
const TOKENS = [
  ['<gd name="', "Á"],
  ['" fmla="', "Â"],
  ['"/>', "Ã"],
  ['<pt x="', "Ä"],
  ['" y="', "Å"],
  ["<moveTo>", "Æ"],
  ["</moveTo>", "Ç"],
  ["<lnTo>", "È"],
  ["</lnTo>", "É"],
  ['<arcTo wR="', "Ê"],
  ['" hR="', "Ë"],
  ['" stAng="', "Ì"],
  ['" swAng="', "Í"],
  ["<cubicBezTo>", "Î"],
  ["</cubicBezTo>", "Ï"],
  ["<quadBezTo>", "Ð"],
  ["</quadBezTo>", "Ñ"],
  ["<close/>", "Ò"],
  ["<path", "Ó"],
  ["</path>", "Ô"],
  ["</gdLst>", "Õ"],
  ["<gdLst>", "Ö"],
  ["</avLst>", "Ø"],
  ["<avLst>", "Ù"],
  ["</pathLst>", "Ú"],
  ["<pathLst>", "Û"],
];
const pack = (x) => TOKENS.reduce((t, [a, b]) => t.split(a).join(b), x);
let out = `// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 가이드 수식으로 짠 모양 정의 (생성 파일 — \`tools/probe/shape-guides-gen.js\`). ECMA-376 의 모양 정의 구조(조정값 → 가이드 수식 →
//! 경로: moveTo·lnTo·arcTo·close)를 따르고, 한셀 화면 갈무리와 모양·가로세로 비율마다 대조한 것만 둔다. 그리는 쪽은 \`shape.rs\` 의
//! \`guide_paths\` (사용자 정의 모양과 같은 수식 계산기).

/// 모양 이름 → \`<g>\` 요소 (avLst·gdLst·pathLst, 기본 이름공간 DrawingML main)
pub fn definition(prst: &str) -> Option<&'static str> {
    Some(match prst {
`;
const consts = [];
for (const prst of list) {
  const m = spec.match(new RegExp(`<${prst}>[^]*?</${prst}>`));
  if (!m) throw new Error("정의가 없는 모양: " + prst);
  const get = (t) => {
    const r = m[0].match(new RegExp(`<${t}[^>]*?(/>|>[^]*?</${t}>)`));
    return r ? squeeze(r[0]) : "";
  };
  const body = ["avLst", "gdLst", "pathLst"].map(get).join("");
  const name = prst.toUpperCase();
  out += `        "${prst}" => ${name},\n`;
  consts.push(`const ${name}: &str = r#"${pack(`<g xmlns="http://schemas.openxmlformats.org/drawingml/2006/main">${body}</g>`)}"#;`);
}
out += `        _ => return None,\n    })\n}\n\n`;
out += "/// 줄인 글자 → 원래 글 (생성기의 TOKENS 를 거꾸로)\npub fn expand(packed: &str) -> String {\n    let mut s = packed.to_string();\n";
for (const [a, b] of TOKENS.slice().reverse()) out += `    s = s.replace('${b}', ${JSON.stringify(a)});\n`;
out += "    s\n}\n\n" + consts.join("\n") + "\n";
process.stdout.write(out);
