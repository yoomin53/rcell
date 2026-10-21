# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 한셀 화면 갈무리: 한셀(COM)로 문서를 열어 보이는 시트마다 확대율 100%, 행·열 머리글 끔, 왼쪽 위(A1)로
# 스크롤한 창을 PNG 로 저장하고, 그림 안의 A1 왼쪽 위 위치와 격자 영역 크기를 TSV 로 적는다.
# rcell 그림과의 비교는 visual-diff.ps1.
#
#   #dir<TAB>문서 폴더
#   파일<TAB>시트번호(1부터)<TAB>시트이름<TAB>그림<TAB>A1x<TAB>A1y<TAB>격자 너비<TAB>격자 높이<TAB>머리글<TAB>눈금선<TAB>보이는 범위
#
# 사용: powershell -File screen-capture.ps1 -InDir <폴더> -OutDir <출력 폴더> [-Filter *.cell] [-Width 1600] [-Height 1000]
#   -Only : 파일 이름 정규식 (일부만)
# 주의: 갈무리하는 동안 한셀 창이 잠깐 보인다 (PrintWindow 는 가려진 창도 그린다). 한셀 인스턴스를 따로 띄우고
#       끝나면 닫는다. 확대율·스크롤·선택만 바꾸고 문서는 저장하지 않는다.
param(
    [Parameter(Mandatory = $true)][string]$InDir,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$Filter = "*.cell",
    [string]$Only = "",
    [int]$Width = 1600,
    [int]$Height = 1000
)
$ErrorActionPreference = "Stop"
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class RcCap {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static int[] Rect(IntPtr hwnd) { RECT r; GetWindowRect(hwnd, out r); return new int[] { r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top }; }
    public static void Shot(IntPtr hwnd, string path) {
        RECT r; GetWindowRect(hwnd, out r);
        using (var bmp = new Bitmap(r.Right - r.Left, r.Bottom - r.Top, PixelFormat.Format32bppArgb)) {
            using (var g = Graphics.FromImage(bmp)) {
                IntPtr hdc = g.GetHdc();
                PrintWindow(hwnd, hdc, 2);
                g.ReleaseHdc(hdc);
            }
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
"@

# 한셀이 바쁠 때(RPC_E_CALL_REJECTED 등) 잠시 뒤 다시 부른다
function Retry([scriptblock]$action) {
    for ($i = 0; ; $i++) {
        try { return & $action } catch {
            if ($i -ge 5) { throw }
            Start-Sleep -Milliseconds (300 * ($i + 1))
        }
    }
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("#dir`t$((Resolve-Path $InDir).Path)")
$files = Get-ChildItem $InDir -File -Filter $Filter |
    Where-Object { $_.Name -notmatch '\.(bin|as|rcell)\.' -and (-not $Only -or $_.Name -match $Only) } | Sort-Object Name
$app = New-Object -ComObject HCell.Application
try {
    try { $app.DisplayAlerts = $false } catch {}
    foreach ($f in $files) {
        $wb = $null
        try {
            $wb = Retry { $app.Workbooks.Open($f.FullName) }
            $app.Visible = $true
            $hwnd = [IntPtr][long]$app.Hwnd
            try { $app.WindowState = -4143 } catch {}   # xlNormal
            [RcCap]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $Width, $Height, 0x0040) | Out-Null
            $k = 0
            foreach ($ws in @($wb.Worksheets)) {
                $k++
                if ([int]$ws.Visible -ne -1) { continue }
                Retry { $ws.Activate() } | Out-Null
                $win = $app.ActiveWindow
                try { $win.Zoom = 100 } catch {}
                # 머리글을 끄면 A1 이 격자 영역 왼쪽 위에 온다 (문서 설정은 저장하지 않는다)
                $heads = [bool]$win.DisplayHeadings
                try { $win.DisplayHeadings = $false } catch {}
                # 선택 칸 표시가 화면에 남지 않게 먼 칸을 고른 뒤 왼쪽 위로 스크롤한다
                try { $ws.Range("XFD1048576").Select() | Out-Null } catch {}
                try { $win.ScrollRow = 1; $win.ScrollColumn = 1 } catch {}
                Start-Sleep -Milliseconds 900
                $png = "{0}.{1}.png" -f $f.Name, $k
                [RcCap]::Shot($hwnd, (Join-Path $OutDir $png))
                $rect = [RcCap]::Rect($hwnd)
                $ax = [int]$win.PointsToScreenPixelsX(0) - $rect[0]
                $ay = [int]$win.PointsToScreenPixelsY(0) - $rect[1]
                $gw = [int]([double]$win.UsableWidth * 4 / 3)
                $gh = [int]([double]$win.UsableHeight * 4 / 3)
                $vr = try { $win.VisibleRange.Address($false, $false) } catch { "" }
                $lines.Add(("{0}`t{1}`t{2}`t{3}`t{4}`t{5}`t{6}`t{7}`t{8}`t{9}`t{10}" -f $f.Name, $k, $ws.Name, $png, $ax, $ay, $gw, $gh, $heads, [bool]$win.DisplayGridlines, $vr))
            }
            $app.Visible = $false
            Write-Host "OK   $($f.Name) (dpi $([RcCap]::GetDpiForWindow($hwnd)))"
        } catch {
            Write-Host "FAIL $($f.Name): $($_.Exception.Message)"
        } finally {
            if ($wb) { try { $wb.Close($false) } catch {} }
        }
    }
} finally {
    try { $app.Visible = $false } catch {}
    # New-Object 마다 새 한셀 프로세스가 뜨고 빈 "통합 문서 1" 을 연다 — 이 인스턴스는 우리 것이므로 닫는다
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
[IO.File]::WriteAllLines((Join-Path $OutDir "captures.tsv"), $lines, (New-Object Text.UTF8Encoding $false))
"wrote $($lines.Count) captures to $OutDir"
