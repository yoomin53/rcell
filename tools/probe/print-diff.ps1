# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 한셀 PDF(print-oracle.ps1 -PdfDir)와 rcell PDF(`rcell pdf`)를 페이지마다 그림으로 바꿔 픽셀로 비교한다.
# 그림은 Windows.Data.Pdf (Windows 10 이상 기본 PDF 렌더러)로 그린다 — 두 PDF 를 같은 렌더러로 그리므로 글꼴
# 래스터화 차이가 줄어든다. 결과 TSV 와 차이 그림(다른 픽셀을 빨강으로)을 남긴다.
#
#   파일<TAB>페이지<TAB>한셀 페이지 수<TAB>rcell 페이지 수<TAB>비교 픽셀<TAB>다른 픽셀 %<TAB>잉크 겹침 %
#
# "다른 픽셀" = RGB 한 채널이라도 차이가 -Tol(기본 48) 넘는 것. "잉크 겹침" = 한셀 그림의 어두운 픽셀 중 rcell 그림의
# 같은 자리 ±1px 안에도 어두운 픽셀이 있는 비율 (visual-diff.ps1 과 같은 잣대).
#
# 사용: powershell -File print-diff.ps1 -PdfDir <한셀 PDF 폴더> -InDir <문서 폴더> -Rcell <rcell.exe> -OutDir <출력 폴더>
#   -Dpi : 그림 해상도 (기본 96)
#   -Only : 파일 이름 정규식
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다. 결과와 한셀 PDF 는 저장소에 넣지 않는다.
param(
    [Parameter(Mandatory = $true)][string]$PdfDir,
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$Rcell,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [int]$Dpi = 96,
    [int]$Tol = 48,
    [string]$Only = ""
)
$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class RcPdfDiff {
    static byte[] Load(Bitmap b, out int stride) {
        var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        stride = d.Stride; var buf = new byte[stride * b.Height];
        Marshal.Copy(d.Scan0, buf, 0, buf.Length); b.UnlockBits(d); return buf;
    }
    static bool Dark(byte[] p, int o) { return p[o] + p[o + 1] + p[o + 2] < 3 * 140; }
    // 돌려주는 값: 비교 픽셀, 다른 픽셀, 한셀 잉크, 겹친 잉크
    public static double[] Compare(string hcell, string rcell, int tol, string diffPng) {
        using (var a = new Bitmap(hcell)) using (var b = new Bitmap(rcell)) {
            int sa, sb; var pa = Load(a, out sa); var pb = Load(b, out sb);
            int w = Math.Max(a.Width, b.Width), h = Math.Max(a.Height, b.Height);
            long n = 0, bad = 0, ink = 0, hit = 0;
            using (var o = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
                var od = o.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
                var po = new byte[od.Stride * h];
                for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
                    int io = y * od.Stride + x * 4;
                    bool ina = x < a.Width && y < a.Height, inb = x < b.Width && y < b.Height;
                    n++;
                    if (!ina || !inb) { bad++; po[io + 2] = 255; po[io + 3] = 255; continue; }
                    int ia = y * sa + x * 4, ib = y * sb + x * 4;
                    int d = Math.Max(Math.Abs(pa[ia] - pb[ib]), Math.Max(Math.Abs(pa[ia + 1] - pb[ib + 1]), Math.Abs(pa[ia + 2] - pb[ib + 2])));
                    int g = (pa[ia] + pa[ia + 1] + pa[ia + 2]) / 3; g = 160 + g * 95 / 255;
                    if (d > tol) { bad++; po[io] = 0; po[io + 1] = 0; po[io + 2] = 255; } else { po[io] = (byte)g; po[io + 1] = (byte)g; po[io + 2] = (byte)g; }
                    po[io + 3] = 255;
                    if (Dark(pa, ia)) {
                        ink++;
                        bool found = false;
                        for (int dy = -1; dy <= 1 && !found; dy++) for (int dx = -1; dx <= 1 && !found; dx++) {
                            int xx = x + dx, yy = y + dy;
                            if (xx < 0 || yy < 0 || xx >= b.Width || yy >= b.Height) continue;
                            if (Dark(pb, yy * sb + xx * 4)) found = true;
                        }
                        if (found) hit++;
                    }
                }
                Marshal.Copy(po, 0, od.Scan0, po.Length); o.UnlockBits(od);
                if (diffPng != "") o.Save(diffPng, ImageFormat.Png);
            }
            return new double[] { n, bad, ink, hit };
        }
    }
}
"@
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
$asTaskAction = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' })[0]
function Await($op, [Type]$t) { $task = $asTask.MakeGenericMethod($t).Invoke($null, @($op)); $task.Wait() | Out-Null; $task.Result }
function AwaitAction($op) { $task = $asTaskAction.Invoke($null, @($op)); $task.Wait() | Out-Null }

# PDF → 페이지 그림들 (경로 목록)
function Rasterize([string]$pdf, [string]$prefix) {
    $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($pdf)) ([Windows.Storage.StorageFile])
    $doc = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
    $out = @()
    for ($i = 0; $i -lt $doc.PageCount; $i++) {
        $page = $doc.GetPage($i)
        $opt = New-Object Windows.Data.Pdf.PdfPageRenderOptions
        # page.Size 는 DIP (1/96 인치)
        $opt.DestinationWidth = [uint32][Math]::Round($page.Size.Width * $Dpi / 96)
        $opt.DestinationHeight = [uint32][Math]::Round($page.Size.Height * $Dpi / 96)
        $opt.BackgroundColor = [Windows.UI.Color]::FromArgb(255, 255, 255, 255)
        $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
        AwaitAction ($page.RenderToStreamAsync($stream, $opt))
        $net = [System.IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($stream.GetInputStreamAt(0))
        $png = $prefix + ($i + 1) + ".png"
        $fs = [IO.File]::Create($png)
        $net.CopyTo($fs)
        $fs.Close(); $net.Close(); $page.Dispose()
        $out += $png
    }
    return , $out
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$rows = New-Object System.Collections.Generic.List[string]
$tot = @(0.0, 0.0, 0.0, 0.0)
$pagesSame = 0; $docs = 0
foreach ($h in (Get-ChildItem $PdfDir -Filter "*.pdf" | Sort-Object Name)) {
    $name = $h.BaseName
    if ($Only -and $name -notmatch $Only) { continue }
    $doc = Join-Path $InDir $name
    if (-not (Test-Path $doc)) { continue }
    $mine = Join-Path $OutDir "$name.rcell.pdf"
    & $Rcell pdf $doc $mine
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $mine)) { Write-Host "FAIL rcell $name"; continue }
    $docs++
    $ha = Rasterize $h.FullName (Join-Path $OutDir "$name.hcell.p")
    $ra = Rasterize ((Resolve-Path $mine).Path) (Join-Path $OutDir "$name.rcell.p")
    if ($ha.Count -eq $ra.Count) { $pagesSame++ }
    $n = [Math]::Min($ha.Count, $ra.Count)
    for ($i = 0; $i -lt $n; $i++) {
        $r = [RcPdfDiff]::Compare($ha[$i], $ra[$i], $Tol, (Join-Path $OutDir ("{0}.diff.p{1}.png" -f $name, ($i + 1))))
        for ($k = 0; $k -lt 4; $k++) { $tot[$k] += $r[$k] }
        $inkp = if ($r[2] -gt 0) { 100 * $r[3] / $r[2] } else { 100 }
        $rows.Add(("{0}`t{1}`t{2}`t{3}`t{4}`t{5:F2}`t{6:F1}" -f $name, ($i + 1), $ha.Count, $ra.Count, [long]$r[0], (100 * $r[1] / $r[0]), $inkp))
    }
    Write-Host ("{0}: pages {1}/{2}" -f $name, $ha.Count, $ra.Count)
}
$inkAll = if ($tot[2] -gt 0) { 100 * $tot[3] / $tot[2] } else { 100 }
$rows.Add(("전체`t-`t문서 {0}`t페이지 수 같음 {1}`t{2}`t{3:F2}`t{4:F1}" -f $docs, $pagesSame, [long]$tot[0], (100 * $tot[1] / [Math]::Max(1, $tot[0])), $inkAll))
[IO.File]::WriteAllLines((Join-Path $OutDir "print-diff.tsv"), $rows, (New-Object Text.UTF8Encoding $false))
$rows | Select-Object -Last 1
