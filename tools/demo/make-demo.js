// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
//
// README 스크린샷·온라인 데모용 문서를 만든다 (내용은 모두 이 스크립트가 지어낸 것 — 한셀 파일이 아니다).
// 사용: node tools/demo/make-demo.js [출력.cell]   (기본: web/samples/demo.cell)
// 그다음: rcell html web/samples/demo.cell -o demo.html  또는 웹 뷰어에서 ?file=samples/demo.cell
const fs = require("fs");
const path = require("path");
const zlib = require("zlib");
const out = process.argv[2] || path.join(__dirname, "..", "..", "web", "samples", "demo.cell");
const parts = new Map();
const w = (p, s) => parts.set(p, Buffer.from(s, "utf8"));
const DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>';
const SML = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const A = "http://schemas.openxmlformats.org/drawingml/2006/main";
const C = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const XDR = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

const regions = ["서울", "부산", "대구", "인천", "광주", "대전"];
const q = [
  [1250, 1380, 1420, 1610],
  [860, 910, 880, 990],
  [540, 520, 610, 650],
  [720, 790, 830, 870],
  [410, 430, 400, 460],
  [480, 510, 560, 590],
];
const yoy = [0.124, 0.058, 0.091, 0.137, -0.032, 0.046];
const quarters = ["1분기", "2분기", "3분기", "4분기"];

// 공유 문자열
const sst = [];
const s = (t) => {
  let i = sst.indexOf(t);
  if (i < 0) i = sst.push(t) - 1;
  return i;
};
const cols = "ABCDEFG";
const cell = (ref, v, st, f) => {
  if (typeof v === "string") return `<c r="${ref}" s="${st}" t="s"><v>${s(v)}</v></c>`;
  return `<c r="${ref}" s="${st}">${f ? `<f>${f}</f>` : ""}<v>${v}</v></c>`;
};
let rows = "";
rows += `<row r="1" ht="30" customHeight="1">${cell("A1", "rcell 데모 — 2026 지역별 분기 매출", 1)}${[..."BCDEFG"].map((c) => `<c r="${c}1" s="1"/>`).join("")}</row>`;
rows += `<row r="2" ht="18" customHeight="1">${cell("A2", "단위: 백만 원 · 합계는 수식, 전년 대비는 비율", 9)}</row>`;
rows += `<row r="3" ht="22" customHeight="1">${["지역", ...quarters, "합계", "전년 대비"].map((h, i) => cell(`${cols[i]}3`, h, 2)).join("")}</row>`;
regions.forEach((name, k) => {
  const r = 4 + k;
  const total = q[k].reduce((a, b) => a + b, 0);
  rows += `<row r="${r}" ht="20" customHeight="1">${cell(`A${r}`, name, 3)}${q[k].map((v, i) => cell(`${cols[1 + i]}${r}`, v, 4)).join("")}${cell(`F${r}`, total, 4, `SUM(B${r}:E${r})`)}${cell(`G${r}`, yoy[k], 5)}</row>`;
});
const colTotals = [0, 1, 2, 3].map((i) => q.reduce((a, row) => a + row[i], 0));
const grand = colTotals.reduce((a, b) => a + b, 0);
rows += `<row r="10" ht="22" customHeight="1">${cell("A10", "합계", 6)}${colTotals.map((v, i) => cell(`${cols[1 + i]}10`, v, 7, `SUM(${cols[1 + i]}4:${cols[1 + i]}9)`)).join("")}${cell("F10", grand, 7, "SUM(F4:F9)")}${cell("G10", 0.081, 8)}</row>`;

const sheet = `${DECL}<worksheet xmlns="${SML}" xmlns:r="${R}"><sheetPr><tabColor rgb="FF2F5597"/></sheetPr><dimension ref="A1:G10"/>` +
  `<sheetViews><sheetView tabSelected="1" showGridLines="1" workbookViewId="0"><selection activeCell="A1" sqref="A1"/></sheetView></sheetViews>` +
  `<sheetFormatPr defaultRowHeight="16.5"/><cols><col min="1" max="1" width="11" customWidth="1"/><col min="2" max="5" width="11.5" customWidth="1"/><col min="6" max="6" width="13" customWidth="1"/><col min="7" max="7" width="11.5" customWidth="1"/></cols>` +
  `<sheetData>${rows}</sheetData><mergeCells count="2"><mergeCell ref="A1:G1"/><mergeCell ref="A2:G2"/></mergeCells>` +
  `<conditionalFormatting sqref="B4:E9"><cfRule type="colorScale" priority="3"><colorScale><cfvo type="min"/><cfvo type="percentile" val="50"/><cfvo type="max"/><color rgb="FFF8696B"/><color rgb="FFFFEB84"/><color rgb="FF63BE7B"/></colorScale></cfRule></conditionalFormatting>` +
  `<conditionalFormatting sqref="F4:F9"><cfRule type="dataBar" priority="2"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF5B9BD5"/></dataBar></cfRule></conditionalFormatting>` +
  `<conditionalFormatting sqref="G4:G9"><cfRule type="iconSet" priority="1"><iconSet iconSet="3Arrows"><cfvo type="percent" val="0"/><cfvo type="num" val="0"/><cfvo type="num" val="0.1"/></iconSet></cfRule></conditionalFormatting>` +
  `<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><drawing r:id="rId1"/></worksheet>`;
w("xl/worksheets/sheet1.xml", sheet);
w("xl/worksheets/_rels/sheet1.xml.rels", `${DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/drawing" Target="../drawings/drawing1.xml"/></Relationships>`);

// 차트: 지역별 분기 매출 (묶은 세로 막대)
const colors = ["2F5597", "5B9BD5", "9DC3E6", "F4B183"];
const strCache = (arr) => `<c:strCache><c:ptCount val="${arr.length}"/>${arr.map((t, i) => `<c:pt idx="${i}"><c:v>${t}</c:v></c:pt>`).join("")}</c:strCache>`;
const numCache = (arr) => `<c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="${arr.length}"/>${arr.map((v, i) => `<c:pt idx="${i}"><c:v>${v}</c:v></c:pt>`).join("")}</c:numCache>`;
const series = quarters.map((qn, i) =>
  `<c:ser><c:idx val="${i}"/><c:order val="${i}"/><c:tx><c:strRef><c:f>'분기 매출'!$${cols[1 + i]}$3</c:f>${strCache([qn])}</c:strRef></c:tx>` +
  `<c:spPr><a:solidFill><a:srgbClr val="${colors[i]}"/></a:solidFill></c:spPr>` +
  `<c:cat><c:strRef><c:f>'분기 매출'!$A$4:$A$9</c:f>${strCache(regions)}</c:strRef></c:cat>` +
  `<c:val><c:numRef><c:f>'분기 매출'!$${cols[1 + i]}$4:$${cols[1 + i]}$9</c:f>${numCache(q.map((r) => r[i]))}</c:numRef></c:val></c:ser>`
).join("");
const chart = `${DECL}<c:chartSpace xmlns:c="${C}" xmlns:a="${A}" xmlns:r="${R}"><c:chart>` +
  `<c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:rPr sz="1400" b="1"/><a:t>지역별 분기 매출</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/>` +
  `<c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>${series}<c:gapWidth val="80"/><c:axId val="1"/><c:axId val="2"/></c:barChart>` +
  `<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx>` +
  `<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:crossAx val="1"/></c:valAx></c:plotArea>` +
  `<c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>`;
w("xl/charts/chart1.xml", chart);

const marker = (tag, c, co, r, ro) => `<xdr:${tag}><xdr:col>${c}</xdr:col><xdr:colOff>${co}</xdr:colOff><xdr:row>${r}</xdr:row><xdr:rowOff>${ro}</xdr:rowOff></xdr:${tag}>`;
const drawing = `${DECL}<xdr:wsDr xmlns:xdr="${XDR}" xmlns:a="${A}" xmlns:r="${R}" xmlns:c="${C}">` +
  `<xdr:twoCellAnchor>${marker("from", 7, 190500, 0, 0)}${marker("to", 14, 0, 16, 0)}<xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="차트 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="${C}"><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>` +
  `<xdr:twoCellAnchor>${marker("from", 0, 0, 11, 95250)}${marker("to", 6, 571500, 15, 0)}<xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="3" name="글상자 1"/><xdr:cNvSpPr txBox="1"/></xdr:nvSpPr>` +
  `<xdr:spPr><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="F5FCEA"/></a:solidFill><a:ln w="12700"><a:solidFill><a:srgbClr val="70AD47"/></a:solidFill></a:ln></xdr:spPr>` +
  `<xdr:txBody><a:bodyPr anchor="ctr" lIns="91440" rIns="91440"/><a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1200" b="1"><a:solidFill><a:srgbClr val="375623"/></a:solidFill><a:latin typeface="맑은 고딕"/></a:rPr><a:t>한셀 .cell 문서를 한컴오피스 없이</a:t></a:r></a:p>` +
  `<a:p><a:pPr algn="ctr"/><a:r><a:rPr sz="1000"><a:solidFill><a:srgbClr val="375623"/></a:solidFill><a:latin typeface="맑은 고딕"/></a:rPr><a:t>표시 형식 · 조건부 서식 · 차트 · 글상자를 rcell 이 그렸습니다</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>`;
w("xl/drawings/drawing1.xml", drawing);
w("xl/drawings/_rels/drawing1.xml.rels", `${DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/chart" Target="../charts/chart1.xml"/></Relationships>`);

const styles = `${DECL}<styleSheet xmlns="${SML}"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts>` +
  `<fonts count="5"><font><sz val="11"/><name val="맑은 고딕"/></font><font><b/><sz val="16"/><color rgb="FF1F3864"/><name val="맑은 고딕"/></font><font><b/><sz val="11"/><color rgb="FFFFFFFF"/><name val="맑은 고딕"/></font><font><b/><sz val="11"/><name val="맑은 고딕"/></font><font><i/><sz val="10"/><color rgb="FF7F7F7F"/><name val="맑은 고딕"/></font></fonts>` +
  `<fills count="5"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF2F5597"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDEEBF7"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFF2F2F2"/></patternFill></fill></fills>` +
  `<borders count="3"><border><left/><right/><top/><bottom/><diagonal/></border>` +
  `<border><left style="thin"><color rgb="FFBFBFBF"/></left><right style="thin"><color rgb="FFBFBFBF"/></right><top style="thin"><color rgb="FFBFBFBF"/></top><bottom style="thin"><color rgb="FFBFBFBF"/></bottom><diagonal/></border>` +
  `<border><left style="thin"><color rgb="FFBFBFBF"/></left><right style="thin"><color rgb="FFBFBFBF"/></right><top style="thin"><color rgb="FF2F5597"/></top><bottom style="double"><color rgb="FF2F5597"/></bottom><diagonal/></border></borders>` +
  `<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="10">` +
  `<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>` +
  `<xf numFmtId="0" fontId="1" fillId="3" borderId="0" xfId="0" applyFont="1" applyFill="1" applyAlignment="1"><alignment horizontal="left" vertical="center" indent="1"/></xf>` +
  `<xf numFmtId="0" fontId="2" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment horizontal="center" vertical="center"/></xf>` +
  `<xf numFmtId="0" fontId="0" fillId="0" borderId="1" xfId="0" applyBorder="1" applyAlignment="1"><alignment horizontal="center" vertical="center"/></xf>` +
  `<xf numFmtId="3" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1" applyBorder="1" applyAlignment="1"><alignment vertical="center"/></xf>` +
  `<xf numFmtId="164" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1" applyBorder="1" applyAlignment="1"><alignment vertical="center"/></xf>` +
  `<xf numFmtId="0" fontId="3" fillId="4" borderId="2" xfId="0" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment horizontal="center" vertical="center"/></xf>` +
  `<xf numFmtId="3" fontId="3" fillId="4" borderId="2" xfId="0" applyNumberFormat="1" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment vertical="center"/></xf>` +
  `<xf numFmtId="164" fontId="3" fillId="4" borderId="2" xfId="0" applyNumberFormat="1" applyFont="1" applyFill="1" applyBorder="1" applyAlignment="1"><alignment vertical="center"/></xf>` +
  `<xf numFmtId="0" fontId="4" fillId="0" borderId="0" xfId="0" applyFont="1" applyAlignment="1"><alignment horizontal="right" vertical="center"/></xf>` +
  `</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="0"/></styleSheet>`;
w("xl/styles.xml", styles);
w("xl/sharedStrings.xml", `${DECL}<sst xmlns="${SML}" count="${sst.length}" uniqueCount="${sst.length}">${sst.map((t) => `<si><t>${t}</t></si>`).join("")}</sst>`);
w("xl/workbook.xml", `${DECL}<workbook xmlns="${SML}" xmlns:r="${R}"><workbookPr/><bookViews><workbookView activeTab="0"/></bookViews><sheets><sheet name="분기 매출" sheetId="1" r:id="rId1"/></sheets></workbook>`);
w("xl/_rels/workbook.xml.rels", `${DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="${R}/styles" Target="styles.xml"/><Relationship Id="rId3" Type="${R}/sharedStrings" Target="sharedStrings.xml"/></Relationships>`);
w("_rels/.rels", `${DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="${R}/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="${R}/extended-properties" Target="docProps/app.xml"/></Relationships>`);
w("docProps/app.xml", `${DECL}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>rcell</Application></Properties>`);
w("[Content_Types].xml", `${DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>` +
  `<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>` +
  `<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>` +
  `<Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>`);

// ZIP (항목 이름은 / 구분, deflate)
const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (b) => {
  let c = 0xffffffff;
  for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const files = [...parts.keys()].sort((x, y) => (x === "[Content_Types].xml" ? -1 : y === "[Content_Types].xml" ? 1 : x.localeCompare(y)));
const chunks = [];
const central = [];
let offset = 0;
for (const name of files) {
  const data = parts.get(name);
  const comp = zlib.deflateRawSync(data);
  const nameBuf = Buffer.from(name, "utf8");
  const crc = crc32(data);
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(20, 4);
  local.writeUInt16LE(0x0800, 6); // UTF-8 이름
  local.writeUInt16LE(8, 8);
  local.writeUInt32LE(0, 10);
  local.writeUInt32LE(crc, 14);
  local.writeUInt32LE(comp.length, 18);
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(nameBuf.length, 26);
  local.writeUInt16LE(0, 28);
  chunks.push(local, nameBuf, comp);
  const cen = Buffer.alloc(46);
  cen.writeUInt32LE(0x02014b50, 0);
  cen.writeUInt16LE(20, 4);
  cen.writeUInt16LE(20, 6);
  cen.writeUInt16LE(0x0800, 8);
  cen.writeUInt16LE(8, 10);
  cen.writeUInt32LE(0, 12);
  cen.writeUInt32LE(crc, 16);
  cen.writeUInt32LE(comp.length, 20);
  cen.writeUInt32LE(data.length, 24);
  cen.writeUInt16LE(nameBuf.length, 28);
  cen.writeUInt32LE(offset, 42);
  central.push(cen, nameBuf);
  offset += 30 + nameBuf.length + comp.length;
}
const cenBuf = Buffer.concat(central);
const end = Buffer.alloc(22);
end.writeUInt32LE(0x06054b50, 0);
end.writeUInt16LE(files.length, 8);
end.writeUInt16LE(files.length, 10);
end.writeUInt32LE(cenBuf.length, 12);
end.writeUInt32LE(offset, 16);
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, Buffer.concat([...chunks, cenBuf, end]));
console.log(`${files.length} parts -> ${out}`);
