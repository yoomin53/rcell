# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 한셀 화면 갈무리(screen-capture.ps1)와 rcell 그림을 픽셀로 비교한다.
# 갈무리마다: rcell html (머리글 없이, 격자 영역 크기만큼 채워) → 헤드리스 Edge 로 그림 → 한셀 그림의 A1 부터
# 같은 크기를 잘라 비교한다. 결과 TSV 와 차이 그림(다른 픽셀을 빨강으로)을 남긴다.
#
#   파일<TAB>시트<TAB>비교 픽셀<TAB>다른 픽셀 %<TAB>평균 차이(0–255)<TAB>잉크 겹침 %
#
# "다른 픽셀" = RGB 중 한 채널이라도 차이가 -Tol(기본 48) 넘는 것. "잉크 겹침" = 한셀에서 어두운(글자·선) 픽셀
# 가운데 rcell 의 같은 자리 ±1px 안에도 어두운 픽셀이 있는 비율 (글자 모양보다 자리를 본다).
#
# 사용: powershell -File visual-diff.ps1 -Captures <갈무리 폴더> -Rcell <rcell.exe> -OutDir <출력 폴더>
#   -Edge : msedge.exe 경로 (기본 Program Files (x86))
#   -Only : 파일 이름 정규식
#   -Skip : 뺄 갈무리 정규식 ("파일|시트이름" 에 맞춘다 — 한셀이 그리지 않은 빈 화면 등)
#   -OldCli : --no-headers/--fill 이 없는 옛 rcell (머리글은 그림 안에서 A1 을 찾아 밀어낸다)
#   -PerShot : 갈무리마다 Edge 를 띄워 그린다 (기본은 node cdp-shot.js 로 Edge 하나에서 — 그림은 같다)
# 갈무리에 확대율 열(screen-capture.ps1 -KeepZoom)이 있고 100 이 아니면 rcell 그림도 그 확대율로 그린다.
param(
    [Parameter(Mandatory = $true)][string]$Captures,
    [Parameter(Mandatory = $true)][string]$Rcell,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$Edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    [string]$Only = "",
    [string]$Skip = "",
    [int]$Tol = 48,
    [switch]$OldCli,
    [switch]$PerShot
)
$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class RcDiff {
    static byte[] Load(Bitmap b, out int stride) {
        var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        stride = d.Stride; var buf = new byte[stride * b.Height];
        Marshal.Copy(d.Scan0, buf, 0, buf.Length); b.UnlockBits(d); return buf;
    }
    static bool Dark(byte[] p, int o) { return p[o] + p[o + 1] + p[o + 2] < 3 * 140; }
    // 돌려주는 값: 비교 픽셀, 다른 픽셀, 차이 합, 한셀 잉크, 겹친 잉크
    public static double[] Compare(string hcell, int ax, int ay, string rcell, int w, int h, int tol, string diffPng) {
        using (var a = new Bitmap(hcell)) using (var b = new Bitmap(rcell)) {
            int sa, sb; var pa = Load(a, out sa); var pb = Load(b, out sb);
            w = Math.Min(w, Math.Min(a.Width - ax, b.Width)); h = Math.Min(h, Math.Min(a.Height - ay, b.Height));
            long n = 0, bad = 0, ink = 0, hit = 0; double sum = 0;
            using (var o = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
                var od = o.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
                var po = new byte[od.Stride * h];
                for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
                    int ia = (ay + y) * sa + (ax + x) * 4, ib = y * sb + x * 4, io = y * od.Stride + x * 4;
                    int d = Math.Max(Math.Abs(pa[ia] - pb[ib]), Math.Max(Math.Abs(pa[ia + 1] - pb[ib + 1]), Math.Abs(pa[ia + 2] - pb[ib + 2])));
                    n++; sum += d;
                    int g = (pa[ia] + pa[ia + 1] + pa[ia + 2]) / 3; g = 160 + g * 95 / 255;
                    if (d > tol) { bad++; po[io] = 0; po[io + 1] = 0; po[io + 2] = 255; } else { po[io] = (byte)g; po[io + 1] = (byte)g; po[io + 2] = (byte)g; }
                    po[io + 3] = 255;
                    if (Dark(pa, ia)) {
                        ink++;
                        bool found = false;
                        for (int dy = -1; dy <= 1 && !found; dy++) for (int dx = -1; dx <= 1 && !found; dx++) {
                            int xx = x + dx, yy = y + dy;
                            if (xx < 0 || yy < 0 || xx >= w || yy >= h) continue;
                            if (Dark(pb, yy * sb + xx * 4)) found = true;
                        }
                        if (found) hit++;
                    }
                }
                Marshal.Copy(po, 0, od.Scan0, po.Length); o.UnlockBits(od);
                if (diffPng != "") o.Save(diffPng, ImageFormat.Png);
            }
            return new double[] { n, bad, sum, ink, hit };
        }
    }
    public static void Crop(string src, int x, int y, int w, int h, string dst) {
        using (var a = new Bitmap(src)) {
            w = Math.Min(w, a.Width - x); h = Math.Min(h, a.Height - y);
            using (var o = a.Clone(new Rectangle(x, y, w, h), PixelFormat.Format32bppArgb)) o.Save(dst, ImageFormat.Png);
        }
    }
}
"@

New-Item -ItemType Directory -Force $OutDir | Out-Null
$profile = Join-Path $OutDir "edge-profile"
$tsv = Get-Content (Join-Path $Captures "captures.tsv") -Encoding UTF8
$dir = ($tsv | Where-Object { $_ -like "#dir*" } | Select-Object -First 1) -replace "^#dir`t", ""
$out = New-Object System.Collections.Generic.List[string]
$jobs = New-Object System.Collections.Generic.List[object]
$tot = @(0.0, 0.0, 0.0, 0.0, 0.0)
foreach ($line in $tsv) {
    if ($line -like "#*" -or -not $line) { continue }
    $p = $line -split "`t"
    if ($Only -and $p[0] -notmatch $Only) { continue }
    if ($Skip -and ("{0}|{1}" -f $p[0], $p[2]) -match $Skip) { continue }
    $file, $k, $name, $png, $ax, $ay, $gw, $gh = $p[0], [int]$p[1], $p[2], $p[3], [int]$p[4], [int]$p[5], [int]$p[6], [int]$p[7]
    $zoom = if ($p.Count -gt 11 -and $p[11]) { [int]$p[11] } else { 100 }
    $stem = "{0}.{1}" -f $file, $k
    $html = Join-Path $OutDir "$stem.html"
    if ($OldCli) {
        & $Rcell html (Join-Path $dir $file) --sheet ($k - 1) -o $html
    } else {
        # 확대율이 작으면 그만큼 더 많은 칸을 채운다
        $fw = [int][math]::Ceiling($gw * 100 / $zoom); $fh = [int][math]::Ceiling($gh * 100 / $zoom)
        & $Rcell html (Join-Path $dir $file) --sheet ($k - 1) --no-headers --fill ("{0}x{1}" -f $fw, $fh) -o $html
    }
    if ($LASTEXITCODE -ne 0) { Write-Host "FAIL rcell $stem"; continue }
    # 탭·테두리(·확대율 100 이면 확대율)를 빼고 A1(본문 첫 칸)의 왼쪽 위를 그림 왼쪽 위로 민다
    $t = [IO.File]::ReadAllText($html, [Text.Encoding]::UTF8)
    $zcss = if ($zoom -eq 100) { ".rc-canvas{zoom:100%!important}" } else { "" }
    $t = $t.Replace("<style>body{margin:16px;background:#fafafa}", "<style>body{margin:0;background:#fff;overflow:hidden}.rc-tabs{display:none}.rc-wrap{border:0!important;max-height:none!important;overflow:visible!important}$zcss")
    $t = $t.Replace("</body>", "<script>(function(){var w=document.querySelector('.rc-wrap:not(.rc-hidden)');var td=w&&w.querySelector('tbody td');if(!td)return;var c=w.querySelector('.rc-canvas');c.style.position='relative';var z=parseFloat(getComputedStyle(c).zoom)||1;var r=td.getBoundingClientRect();var dx=-r.left/z,dy=-r.top/z;c.style.left=dx+'px';c.style.top=dy+'px';var q=td.getBoundingClientRect();var kx=dx?(q.left-r.left)/dx:1,ky=dy?(q.top-r.top)/dy:1;if(kx&&ky){c.style.left=(dx-q.left/kx)+'px';c.style.top=(dy-q.top/ky)+'px';}})();</script></body>")
    [IO.File]::WriteAllText($html, $t, (New-Object Text.UTF8Encoding $false))
    $shot = Join-Path $OutDir "$stem.rcell.png"
    if (Test-Path $shot) { Remove-Item $shot }
    $jobs.Add([pscustomobject]@{ File = $file; Name = $name; Png = $png; Ax = $ax; Ay = $ay; W = $gw; H = $gh; Stem = $stem; Html = $html; Shot = $shot })
    if ($PerShot) {
        # Edge 는 진행 메시지를 표준 오류로 쓴다 — 오류로 보지 않게. 네트워크 서비스를 따로 띄우면 그것이 죽었다 다시 뜨기를
        # 기다리느라 한 장에 2~11분 걸릴 때가 있다 — 같은 프로세스에서, 배경 네트워크·구성 요소 갱신 없이
        $ErrorActionPreference = "Continue"
        & $Edge --headless=new --disable-gpu --hide-scrollbars --no-first-run --disable-background-networking --disable-component-update --disable-sync --disable-extensions --enable-features=NetworkServiceInProcess2,NetworkServiceInProcess --force-device-scale-factor=1 "--user-data-dir=$profile" "--window-size=$gw,$gh" "--screenshot=$shot" ("file:///" + $html.Replace("\", "/")) 2>&1 | Out-Null
        $ErrorActionPreference = "Stop"
        for ($i = 0; $i -lt 50 -and -not (Test-Path $shot); $i++) { Start-Sleep -Milliseconds 200 }
    }
}
if (-not $PerShot -and $jobs.Count -gt 0) {
    # Edge 하나로 모두 그린다 (cdp-shot.js — 장마다 Edge 를 띄우고 끄는 데 몇 분씩 걸릴 때가 있다. 그림은 같다)
    $list = Join-Path $OutDir "shots.tsv"
    [IO.File]::WriteAllLines($list, [string[]]($jobs | ForEach-Object { "{0}`t{1}`t{2}`t{3}" -f $_.Html, $_.Shot, $_.W, $_.H }), (New-Object Text.UTF8Encoding $false))
    $ErrorActionPreference = "Continue"
    & node (Join-Path $PSScriptRoot "cdp-shot.js") $Edge $profile $list 2>&1 | Out-Null
    $ErrorActionPreference = "Stop"
}
foreach ($j in $jobs) {
    if (-not (Test-Path $j.Shot)) { Write-Host "FAIL edge $($j.Stem)"; continue }
    $hcell = Join-Path $Captures $j.Png
    [RcDiff]::Crop($hcell, $j.Ax, $j.Ay, $j.W, $j.H, (Join-Path $OutDir "$($j.Stem).hcell.png"))
    $r = [RcDiff]::Compare($hcell, $j.Ax, $j.Ay, $j.Shot, $j.W, $j.H, $Tol, (Join-Path $OutDir "$($j.Stem).diff.png"))
    for ($i = 0; $i -lt 5; $i++) { $tot[$i] += $r[$i] }
    $inkp = if ($r[3] -gt 0) { 100 * $r[4] / $r[3] } else { 100 }
    $out.Add(("{0}`t{1}`t{2}`t{3:F2}`t{4:F2}`t{5:F1}" -f $j.File, $j.Name, [int]$r[0], (100 * $r[1] / $r[0]), ($r[2] / $r[0]), $inkp))
}
$inkAll = if ($tot[3] -gt 0) { 100 * $tot[4] / $tot[3] } else { 100 }
$out.Add(("전체`t-`t{0}`t{1:F2}`t{2:F2}`t{3:F1}" -f [long]$tot[0], (100 * $tot[1] / [math]::Max(1, $tot[0])), ($tot[2] / [math]::Max(1, $tot[0])), $inkAll))
[IO.File]::WriteAllLines((Join-Path $OutDir "visual-diff.tsv"), $out, (New-Object Text.UTF8Encoding $false))
$out
