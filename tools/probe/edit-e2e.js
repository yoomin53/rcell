// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// 편집 대조 (corpus): 문서마다 편집 대본 몇 개를 rcell 과 한셀로 각각 실행해 결과를 비교한다.
//   1. 원본을 작업 폴더로 복사한다 (한셀은 연 문서 옆에 잠금 파일을 만들므로 corpus 의 파일은 열지 않는다)
//   2. rcell edit 사본 대본 → X.S.rcell.cell (편집 + 다시 계산 + 저장)
//   3. 한셀 (edit-oracle.ps1): 사본에 같은 대본 → X.S.hcell.cell, 그리고 X.S.rcell.cell 을 열어 다시 저장
//      → X.S.rcell.as.cell (rcell 이 쓴 것을 한셀이 읽은 결과)
//   4. rcell diff X.S.hcell.cell X.S.rcell.as.cell — 둘이 같으면 rcell 의 편집이 한셀과 같다
//   5. 차이가 나면 한셀 결과도 다시 저장해(X.S.hcell.as.cell) 한셀이 다시 연 결과끼리 한 번 더 비교한다
//      (한셀은 문서를 열면서 기록 없는 행을 자동 높이로 맞추는 등 고치는 것이 있다)
//
// 사용: node tools/probe/edit-e2e.js <corpus 폴더> <작업 폴더> [--skip-hcell] [--diff-only] [--reuse-hcell] [--only 이름] [--scripts 대본,…]
//   --reuse-hcell : 한셀 편집 결과(X.S.hcell.cell)는 그대로 두고 rcell 쪽과 한셀 다시 저장만 새로 한다
// (rcell 은 target/release/rcell(.exe), 한셀은 COM 자동화 — 한셀이 설치된 Windows 에서)
"use strict";
const fs = require("fs");
const path = require("path");
const { execFileSync, spawnSync } = require("child_process");

const root = path.resolve(__dirname, "..", "..");
const exe = path.join(root, "target", "release", process.platform === "win32" ? "rcell.exe" : "rcell");
const oracle = path.join(__dirname, "edit-oracle.ps1");
const [corpus, work] = process.argv.slice(2);
const diffOnly = process.argv.includes("--diff-only");
const skipHcell = diffOnly || process.argv.includes("--skip-hcell");
const only = process.argv.includes("--only") ? process.argv[process.argv.indexOf("--only") + 1] : null;
if (!corpus || !work) {
  console.error("사용: node tools/probe/edit-e2e.js <corpus 폴더> <작업 폴더> [--skip-hcell] [--only 이름]");
  process.exit(2);
}

// 대상 문서 (corpus 안의 경로)
const DOCS = [
  ...[
    "Auto_Expense_Report", "Average_Selling_Price", "Basic_Invoice", "Buy_vs_Lease_Car",
    "CM_and_BreakEven_Analysis", "Cash_Flow_Sensitivity", "Closing_Costs", "College_Monthly_Budget",
    "Expense_Report", "Holiday_Budget", "Home_Rent_or_Buy", "Inventory_Control", "Loan_Calculators",
    "Marketing_Budget", "Mortgage_Refinancing", "Portfolio_Allocation", "Price_Estimate_Services",
    "Purchase_Order", "Qtrly_Budget_Analysis", "Retirement_Budget", "Retirement_Income_Tax",
    "Sales_Analysis", "Sales_Forecast_by_Market_Share", "Shipping_Order", "Start_up_Budget",
    "Statement_of_Net_Worth", "Trade_Show_Budget", "Unit_Sales_Forecast",
  ].map((n) => `templates/${n}.cell`),
  ...["values", "formulas", "fonts", "fills_borders", "align_numfmt", "structure", "strings", "bigref"].map((n) => `${n}.cell`),
  ...["cf_dv", "cfdv_x", "filter2", "outline", "pictures", "view_print", "odd_anchor"].map((n) => `craft/${n}.cell`),
  ...["chart_base", "chart_pie", "chart_scatter", "chart_series2", "chart_line"].map((n) => `craft-charts/${n}.cell`),
];

// 편집 대본 (첫 시트 = 0번)
const SCRIPTS = {
  cells: [
    ["AA1", "123"], ["AA2", "1,234.5"], ["AA3", "10%"], ["AA4", "2024-01-15"], ["AA5", "12:30"],
    ["AA6", "'00123"], ["AA7", "TRUE"], ["AA8", "#N/A"], ["AA9", "=sum(aa1:aa3)"], ["AA10", "=AA4+30"],
    ["AA11", "=AA9*2"], ["AA12", "글자"], ["AB1", "=A1"], ["AB2", "=TODAY()-TODAY()"],
    ["AB3", "=PMT(0.05,10,1000)"], ["AB4", "=IF(AA1>100,\"큼\",\"작음\")"], ["AB5", "=COUNTA(A:A)"],
    ["AB6", "0 1/2"], ["AB7", "1/2"], ["AB8", "$1,000"], ["AB9", "= aa1 + ab8"], ["AB10", "2024년 3월 1일"],
  ].map(([cell, input]) => ({ op: "set", sheet: 0, cell, input })),
  rows: [
    { op: "insertRows", sheet: 0, row: 3, count: 2 },
    { op: "deleteRows", sheet: 0, row: 8, count: 1 },
    { op: "insertRows", sheet: 0, row: 1, count: 1 },
  ],
  cols: [
    { op: "insertCols", sheet: 0, col: "B", count: 1 },
    { op: "deleteCols", sheet: 0, col: "D", count: 2 },
  ],
  format: [
    { op: "format", sheet: 0, range: "A1:C3", bold: true, italic: true, fill: "FFFF00", fontColor: "C00000" },
    { op: "format", sheet: 0, range: "B5:D8", hAlign: "center", vAlign: "top", wrap: true, numFmt: "#,##0.00" },
    { op: "border", sheet: 0, range: "A10:C12", preset: "all" },
    { op: "border", sheet: 0, range: "E2:F4", preset: "outline", style: "medium", color: "FF0000" },
    { op: "merge", sheet: 0, range: "E14:G15", center: true },
    { op: "colWidth", sheet: 0, col: "B", width: 18 },
    { op: "rowHeight", sheet: 0, row: 4, height: 25 },
    { op: "format", sheet: 0, range: "A20:B21", fontName: "굴림", fontSize: 14, underline: true, strike: true },
  ],
  sheets: [
    { op: "copy", sheet: 0, from: "A1:D6", to: "AF1" },
    { op: "fillDown", sheet: 0, range: "AF6:AI9" },
    { op: "move", sheet: 0, from: "A1:B2", to: "AH20" },
    { op: "addSheet", name: "새 시트", after: 0 },
    { op: "renameSheet", sheet: 0, name: "편집됨" },
    { op: "moveSheet", sheet: 1, to: 0 },
  ],
  fill: [
    { op: "autoFill", sheet: 0, range: "A3:B4", to: "A3:B12" },
    { op: "autoFill", sheet: 0, range: "C2", to: "C2:H2" },
    { op: "autoFill", sheet: 0, range: "D6:D7", to: "D1:D7" },
    { op: "series", sheet: 0, range: "AA1:AA8", step: 3 },
    { op: "set", sheet: 0, cell: "AB1", input: "월" },
    { op: "autoFill", sheet: 0, range: "AB1", to: "AB1:AB9" },
  ],
  replace: [
    { op: "replace", sheet: 0, find: "1", replace: "7", range: "A1:Z200" },
    { op: "replace", sheet: 0, find: "SUM", replace: "sum", matchCase: true, range: "A1:Z200" },
    { op: "replace", find: "합계", replace: "총계", all: true, range: "A1:Z200" },
    { op: "replace", sheet: 0, find: "a", replace: "Ä", wholeCell: true, range: "A1:Z200" },
  ],
  shift: [
    { op: "insertCells", sheet: 0, range: "B3:C4", shift: "down" },
    { op: "deleteCells", sheet: 0, range: "D6:E6", shift: "left" },
    { op: "insertCells", sheet: 0, range: "A10:B10", shift: "right" },
    { op: "deleteCells", sheet: 0, range: "C2", shift: "up" },
    { op: "hideRows", sheet: 0, row: 7, count: 2 },
    { op: "hideCols", sheet: 0, col: "F" },
    { op: "tabColor", sheet: 0, color: "00B050" },
  ],
  paste: [
    { op: "pasteSpecial", sheet: 0, from: "A1:D6", to: "AF1", paste: "values" },
    { op: "pasteSpecial", sheet: 0, from: "A1:D6", to: "AK1", paste: "formats" },
    { op: "pasteSpecial", sheet: 0, from: "A1:D6", to: "AP1", paste: "all", transpose: true },
    { op: "pasteSpecial", sheet: 0, from: "B2:C5", to: "AF10", paste: "formulas", skipBlanks: true },
    { op: "pasteSpecial", sheet: 0, from: "C3:D4", to: "AF1", paste: "values", operation: "add" },
  ],
  objects: [
    { op: "addHyperlink", sheet: 0, cell: "AA3", address: "https://example.com/", tooltip: "팁" },
    { op: "addName", name: "범위1", refersTo: "$A$1:$C$4" },
    { op: "deleteCf", sheet: 0, range: "A1:B20" },
    { op: "deleteValidation", sheet: 0, range: "A1:B20" },
    { op: "deleteComment", sheet: 0, range: "A1:D20" },
  ],
  data: [
    { op: "removeDuplicates", sheet: 0, range: "B2:F40", columns: [1, 2], header: true },
    { op: "textToColumns", sheet: 0, range: "C3:C30", dest: "AK3", comma: true, space: true },
  ],
  filter: [
    { op: "autoFilter", sheet: 0, range: "A1:F30", field: 3, criteria1: ">=1" },
    { op: "autoFilter", sheet: 0, range: "A1:F30", field: 2, criteria1: "항목 2" },
    { op: "autoFilter", sheet: 0, range: "A1:F30", field: 1, criteria1: "3", operator: "top" },
  ],
  copysheet: [
    { op: "copySheet", sheet: 0 },
    { op: "copySheet", sheet: 0, to: 0, name: "사본 A" },
  ],
  view: [
    { op: "setView", sheet: 0, zoom: 130, gridlines: false, headings: false },
    { op: "setView", sheet: 0, view: "pageBreakPreview", zoom: 90 },
  ],
  page: [
    { op: "pageSetup", sheet: 0, marginLeft: 30, marginTop: 50, orientation: "landscape", printArea: "A1:C10,E1:F5", printTitleRows: "1:2", centerH: true, gridLines: true },
    { op: "pageSetup", sheet: 0, headerCenter: "&A 쪽 &P", footerRight: "&P/&N", comments: "atEnd", errors: "blank" },
    { op: "pageSetup", sheet: 0, fitWide: 1, fitTall: 0 },
  ],
  protect: [
    { op: "protectSheet", sheet: 0, password: "pw", formattingCells: true, sorting: true },
    { op: "unprotectSheet", sheet: 0, password: "bad" },
    { op: "protectWorkbook", password: "pw" },
  ],
  addshape: [
    { op: "addShape", sheet: 0, kind: "rect", left: 300, top: 300, width: 100, height: 60 },
    { op: "addShape", sheet: 0, kind: "ellipse", left: 50.5, top: 70.25, width: 90.5, height: 45.75 },
    { op: "addShape", sheet: 0, kind: "textbox", left: 200, top: 20, width: 90, height: 30 },
  ],
  shapes: [
    { op: "moveShape", sheet: 0, shape: 1, left: 100.5, top: 60, width: 300 },
    { op: "moveShape", sheet: 0, shape: 2, left: 20, top: 200 },
    { op: "deleteShape", sheet: 0, shape: 3 },
  ],
  sort: [
    { op: "sort", sheet: 0, range: "A1:H40", keys: [{ by: "B" }], header: true },
    { op: "sort", sheet: 0, range: "B3:F30", keys: [{ by: "C", desc: true }, { by: "B" }] },
  ],
};
// --scripts 이름,이름 : 그 대본만
const pick = process.argv.includes("--scripts") ? process.argv[process.argv.indexOf("--scripts") + 1].split(",") : null;
if (pick) for (const k of Object.keys(SCRIPTS)) if (!pick.includes(k)) delete SCRIPTS[k];

fs.mkdirSync(work, { recursive: true });
const jobs = [];
const resave = [];
let rcellFail = 0;
for (const rel of DOCS) {
  const src = path.join(corpus, rel);
  if (!fs.existsSync(src)) continue;
  const base = path.basename(rel).replace(/\.cell$/, "");
  if (only && base !== only) continue;
  const copy = path.join(work, base + ".src.cell");
  if (!diffOnly) fs.copyFileSync(src, copy);
  for (const [name, edits] of Object.entries(SCRIPTS)) {
    const script = path.join(work, `${base}.${name}.json`);
    fs.writeFileSync(script, JSON.stringify({ edits }));
    const out = path.join(work, `${base}.${name}.rcell.cell`);
    const r = spawnSync(exe, ["edit", copy, script, out], { encoding: "utf8" });
    const msg = (r.stderr || "").trim();
    if (r.status !== 0 || /실패 [1-9]/.test(msg)) {
      rcellFail++;
      console.log(`rcell ${base}.${name}: ${msg.split("\n").join(" | ")}`);
    }
    jobs.push({ in: copy, script, out: path.join(work, `${base}.${name}.hcell.cell`) });
    if (fs.existsSync(out)) resave.push({ in: out, script: "", out: path.join(work, `${base}.${name}.rcell.as.cell`) });
  }
}
fs.writeFileSync(path.join(work, "jobs.json"), JSON.stringify(jobs, null, 1));
fs.writeFileSync(path.join(work, "resave.json"), JSON.stringify(resave, null, 1));
console.log(`문서 ${jobs.length / Object.keys(SCRIPTS).length}, 작업 ${jobs.length}, rcell 실패 ${rcellFail}`);

if (!skipHcell) {
  const reuse = process.argv.includes("--reuse-hcell");
  for (const [list, extra] of [["jobs.json", []], ["resave.json", ["-Resave"]]]) {
    if (reuse && list === "jobs.json") continue;
    execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", oracle, "-List", path.join(work, list), ...extra], {
      stdio: "inherit",
    });
  }
}

// 비교
let same = 0, total = 0, items = 0;
const bySCRIPT = {};
const lines = [];
const differ = [];
for (const j of jobs) {
  const a = j.out;
  const b = a.replace(/\.hcell\.cell$/, ".rcell.as.cell");
  const key = path.basename(a).replace(/\.hcell\.cell$/, "");
  const sname = key.split(".").pop();
  bySCRIPT[sname] = bySCRIPT[sname] || { same: 0, total: 0 };
  if (!fs.existsSync(a) || !fs.existsSync(b)) {
    lines.push(`${key}: 결과 없음 (한셀 ${fs.existsSync(a)}, rcell ${fs.existsSync(b)})`);
    continue;
  }
  total++;
  bySCRIPT[sname].total++;
  const r = spawnSync(exe, ["diff", a, b, "--max", "2000"], { encoding: "utf8", maxBuffer: 1 << 26 });
  const out = (r.stdout || "").trim();
  if (r.status !== 0 && r.status !== 1) {
    lines.push(`${key}: rcell diff 실행 실패 (${r.status}) ${(r.stderr || r.error || "").toString().trim()}`);
    continue;
  }
  if (r.status === 0) {
    same++;
    bySCRIPT[sname].same++;
    continue;
  }
  // 편집 시간(app.xml TotalTime)은 한셀 자동화가 편집에 쓴 분 수라 편집 결과가 아니다 — 그것만 다르면 같다고 본다
  // 보기 설정 대본은 한셀 창을 보이게 해 한셀이 저장한 문서의 창 크기(windowWidth·windowHeight)가 채워진다 — 편집 결과가 아니라 같다고 본다
  const ignoreWindow = sname === "view";
  const diffs = out
    ? out
        .split(/\n(?=\S)/)
        .filter((d) => !d.startsWith("… 외") && !d.startsWith("차이 ") && !d.startsWith("편집 시간") && !(ignoreWindow && d.startsWith("통합 문서 보기")))
    : [];
  if (!diffs.length) {
    same++;
    bySCRIPT[sname].same++;
    lines.push(`${key}: 편집 시간만 다름 (같다고 봄)`);
    continue;
  }
  differ.push({ key, sname, a, b });
  items += diffs.length;
  lines.push(`${key}: 차이 ${diffs.length}`);
  for (const d of diffs.slice(0, 5)) lines.push("    " + d.replace(/\n/g, "\n    "));
}
// 한셀은 문서를 열면서 고치는 것이 있다 (기록 없는 행을 자동 높이로 맞추고 그 행에 걸린 도형 표지를 옮기는 등).
// 한셀이 직접 편집한 결과는 다시 열지 않았으므로, 차이가 난 것은 한셀 결과도 다시 저장해(X.S.hcell.as.cell)
// 한셀이 다시 연 결과끼리 한 번 더 비교한다
if (differ.length && !skipHcell) {
  const list = differ.map((d) => ({ in: d.a, script: "", out: d.a.replace(/\.hcell\.cell$/, ".hcell.as.cell") }));
  fs.writeFileSync(path.join(work, "resave-hcell.json"), JSON.stringify(list, null, 1));
  execFileSync("powershell", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", oracle, "-List", path.join(work, "resave-hcell.json"), "-Resave"], {
    stdio: "inherit",
  });
}
let same2 = 0;
const by2 = {};
for (const d of differ) {
  const a2 = d.a.replace(/\.hcell\.cell$/, ".hcell.as.cell");
  if (!fs.existsSync(a2)) continue;
  const r = spawnSync(exe, ["diff", a2, d.b], { encoding: "utf8", maxBuffer: 1 << 26 });
  if (r.status === 0) {
    same2++;
    by2[d.sname] = (by2[d.sname] || 0) + 1;
    lines.push(`${d.key}: 한셀이 다시 연 결과끼리는 같음`);
  }
}
fs.writeFileSync(path.join(work, "report.txt"), lines.join("\n") + "\n");
console.log(lines.slice(0, 200).join("\n"));
for (const [k, v] of Object.entries(bySCRIPT)) {
  console.log(`  ${k}: ${v.same}/${v.total} (한셀이 다시 연 결과끼리 ${v.same + (by2[k] || 0)})`);
}
console.log(
  `\n편집 대조: ${total}건 중 같음 ${same}, 한셀이 다시 연 결과끼리 같음 ${same + same2}, 차이 항목 ${items} (자세한 것: ${path.join(work, "report.txt")})`,
);
