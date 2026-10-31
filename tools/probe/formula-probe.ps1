# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

# 수식 글 대조: 목록의 수식을 한셀 새 문서의 A열에 하나씩 넣고 한셀이 돌려주는 글을 적는다.
#   Formula (영문), FormulaLocal (현지화), FormulaR1C1, 그리고 입력이 거부됐는지.
# 출력은 탭으로 나눈 줄: 번호 \t 입력 \t Formula \t FormulaLocal \t FormulaR1C1 \t 오류
# 한셀이 멈출 수 있어 별도 프로세스로 돌리고 제한 시간이 지나면 그 프로세스와 그것이 띄운 한셀만 끝낸다
# (calc-oracle.ps1 과 같은 규칙).
#
# 사용: powershell -File formula-probe.ps1 -List <수식 목록.txt> -Out <결과.tsv> [-Sheet 이름] [-TimeoutSec 120]
#   목록: 한 줄에 수식 하나 (`=` 로 시작하든 말든 상관없다). `#sheet 이름` 줄은 그 이름의 시트를 만든다.
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [string]$List,
    [string]$Out,
    [int]$TimeoutSec = 120,
    # 값도 적는다 (Value2 를 17자리로): 자동 계산으로 돌리므로 느리다
    [switch]$Values,
    [switch]$Worker,
    [string]$PidFile
)
$ErrorActionPreference = "Stop"

function Get-HCellIds {
    @(Get-CimInstance Win32_Process -Filter "Name='HCell.exe'" | ForEach-Object { $_.ProcessId })
}

if ($Worker) {
    $before = Get-HCellIds
    $app = New-Object -ComObject HCell.Application
    Start-Sleep -Milliseconds 500
    $mine = @(Get-HCellIds | Where-Object { $before -notcontains $_ })
    if ($mine.Count -ne 1) { $mine = @() }
    Set-Content -Path $PidFile -Value ($mine -join ",") -Encoding ascii
    $lines = [IO.File]::ReadAllLines($List, [Text.Encoding]::UTF8) | Where-Object { $_ -ne "" }
    # 한 줄 쓸 때마다 내보낸다 (한셀이 멈춰도 그때까지의 결과가 남는다)
    $sw = New-Object System.IO.StreamWriter($Out, $false, (New-Object Text.UTF8Encoding($false)))
    $sw.AutoFlush = $true
    try {
        try { $app.Visible = $false } catch {}
        try { $app.DisplayAlerts = $false } catch {}
        $wb = $app.Workbooks.Add()
        if (-not $Values) { try { $app.Calculation = -4135 } catch {} }
        $ws = $wb.Worksheets.Item(1)
        # 참조에 쓸 시트 몇 개와 이름 (수식 문법 시험용): Data, S1, S2, 이름 nm = Data!$A$1:$A$3
        foreach ($nm in @("Data", "S1", "S2")) {
            $s = $wb.Worksheets.Add([Type]::Missing, $wb.Worksheets.Item($wb.Worksheets.Count))
            $s.Name = $nm
        }
        $ws = $wb.Worksheets.Item(1)
        try { [void]$wb.Names.Add("nm", '=Data!$A$1:$A$3') } catch {}
        try { [void]$wb.Names.Add("nm2", '=S1!$B$2') } catch {}
        $row = 0
        foreach ($l in $lines) {
            $row++
            $f = $l
            if (-not $f.StartsWith("=")) { $f = "=" + $f }
            $err = ""
            $fm = ""; $fl = ""; $fr = ""
            try {
                $c = $ws.Cells.Item($row, 2)
                $c.Formula = $f
                $fm = [string]$c.Formula
                $fl = [string]$c.FormulaLocal
                try { $fr = [string]$c.FormulaR1C1 } catch { $fr = "(R1C1 실패)" }
                if ($Values) { try { $v2 = $c.Value2; if ($v2 -is [double]) { $fr = $fr + "`t" + $v2.ToString("R", [Globalization.CultureInfo]::InvariantCulture) } else { $fr = $fr + "`t" + [string]$v2 } } catch {} }
            } catch {
                $err = $_.Exception.Message -replace "[\r\n\t]+", " "
            }
            $sw.WriteLine(($row.ToString() + "`t" + ($l -replace "`t", " ") + "`t" + ($fm -replace "[\r\n\t]+", " ") + "`t" + ($fl -replace "[\r\n\t]+", " ") + "`t" + ($fr -replace "[\r\n\t]+", " ") + "`t" + $err))
        }
        $sw.Close()
        $wb.Close($false)
    } finally {
        try { $app.Quit() } catch {}
        try { [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null } catch {}
    }
    exit 0
}

if (-not $List -or -not $Out) { throw "사용: formula-probe.ps1 -List <목록> -Out <결과.tsv>" }
$pidFile = Join-Path ([IO.Path]::GetTempPath()) ("rcell-formula-probe-" + [Guid]::NewGuid().ToString("N") + ".pid")
if (Test-Path $Out) { Remove-Item $Out -Force }
$started = Get-Date
$argList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Worker",
    "-List", "`"$((Resolve-Path $List).Path)`"", "-Out", "`"$([IO.Path]::GetFullPath($Out))`"", "-PidFile", "`"$pidFile`"")
if ($Values) { $argList += "-Values" }
$p = Start-Process powershell -ArgumentList $argList -PassThru -WindowStyle Hidden
if (-not $p.WaitForExit($TimeoutSec * 1000)) {
    try { $p.Kill() } catch {}
    Write-Host "TIMEOUT"
}
if (Test-Path $pidFile) {
    foreach ($id in ((Get-Content $pidFile) -split ",")) {
        if (-not $id) { continue }
        $h = Get-CimInstance Win32_Process -Filter "ProcessId=$id"
        if ($h -and $h.Name -eq "HCell.exe" -and $h.CreationDate -ge $started.AddSeconds(-2)) {
            Start-Sleep -Seconds 2
            if (Get-CimInstance Win32_Process -Filter "ProcessId=$id") {
                Invoke-CimMethod -InputObject $h -MethodName Terminate | Out-Null
                Write-Host "(남은 한셀 $id 정리)"
            }
        }
    }
    Remove-Item $pidFile -Force
}
if (Test-Path $Out) { Write-Host "OK $Out" } else { Write-Host "FAIL" }
