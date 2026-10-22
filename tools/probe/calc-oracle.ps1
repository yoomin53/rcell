# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 계산 대조 실험: 폴더의 문서를 한셀로 열어 다시 계산(Application.CalculateFull)하고 XML .cell 로 저장한다.
# 한셀은 문서를 열거나 XML·바이너리로 저장할 때 수식을 다시 계산하지 않는다 — 파일에 있던 캐시 값
# (rcell 이 채운 <v>0</v>)이 그대로 남는다. Calculate 는 바뀐 셀이 없어 아무것도 하지 않고, CalculateFull 은
# 모든 수식을 계산한다 (관측). 저장한 파일의 <v> 가 한셀의 계산 값 = 정답이다 (tests/calc_oracle.rs).
#
# 어떤 수식은 한셀을 죽이거나 멈추게 하므로(docs/format/calc.md) 문서마다 따로 프로세스를 띄우고 제한
# 시간이 지나면 그 프로세스와 그것이 띄운 한셀만 끝낸다. 다른 작업이 띄운 한셀은 건드리지 않는다.
#
# 사용: powershell -File calc-oracle.ps1 -InDir <폴더> -OutDir <결과 폴더> [-Filter *.cell] [-TimeoutSec 180]
# 주의: UTF-8(BOM) 으로 저장해야 Windows PowerShell 5.1 이 한글을 바르게 읽는다.
param(
    [string]$InDir,
    [string]$OutDir,
    [string]$Filter = "*.cell",
    [int]$TimeoutSec = 180,
    # 아래는 내부용 (문서 하나를 처리하는 작업 프로세스)
    [switch]$Worker,
    [string]$In,
    [string]$Out,
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
    # 같은 순간에 다른 작업도 한셀을 띄웠으면 어느 것이 내 것인지 모르므로 적지 않는다 (끝내지 않는다)
    if ($mine.Count -ne 1) { $mine = @() }
    Set-Content -Path $PidFile -Value ($mine -join ",") -Encoding ascii
    try {
        try { $app.Visible = $false } catch {}
        try { $app.DisplayAlerts = $false } catch {}
        $wb = $app.Workbooks.Open($In)
        $app.CalculateFull()
        # 바뀐 것이 없으면 한셀은 SaveAs 에서 파일을 그대로 복사한다 (관측) — 다시 쓰게 만든다
        try { $wb.Saved = $false } catch {}
        if (Test-Path $Out) { Remove-Item $Out -Force }
        $wb.SaveAs($Out, -2006)
        $wb.Close($false)
    } finally {
        try { $app.Quit() } catch {}
        try { [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null } catch {}
    }
    exit 0
}

if (-not $InDir -or -not $OutDir) { throw "사용: calc-oracle.ps1 -InDir <폴더> -OutDir <결과 폴더>" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$pidFile = Join-Path ([IO.Path]::GetTempPath()) ("rcell-calc-oracle-" + [Guid]::NewGuid().ToString("N") + ".pid")
foreach ($f in Get-ChildItem $InDir -File -Filter $Filter) {
    $out = Join-Path $OutDir $f.Name
    if (Test-Path $out) { Remove-Item $out -Force }
    if (Test-Path $pidFile) { Remove-Item $pidFile -Force }
    $started = Get-Date
    $argList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"", "-Worker",
        "-In", "`"$($f.FullName)`"", "-Out", "`"$out`"", "-PidFile", "`"$pidFile`"")
    $p = Start-Process powershell -ArgumentList $argList -PassThru -WindowStyle Hidden
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        # 시간 초과: 작업 프로세스와 그것이 띄운 한셀(시작 뒤에 생긴 것)만 끝낸다
        try { $p.Kill() } catch {}
        if (Test-Path $pidFile) {
            foreach ($id in ((Get-Content $pidFile) -split ",")) {
                if (-not $id) { continue }
                $h = Get-CimInstance Win32_Process -Filter "ProcessId=$id"
                if ($h -and $h.Name -eq "HCell.exe" -and $h.CreationDate -ge $started.AddSeconds(-2)) {
                    Invoke-CimMethod -InputObject $h -MethodName Terminate | Out-Null
                }
            }
        }
        Write-Host "TIMEOUT $($f.Name)"
        continue
    }
    if (Test-Path $out) { Write-Host "OK   $($f.Name)" } else { Write-Host "FAIL $($f.Name) (종료 코드 $($p.ExitCode))" }
    # 죽은 한셀이 남았으면 (이 작업이 띄운 것만) 정리
    if (Test-Path $pidFile) {
        foreach ($id in ((Get-Content $pidFile) -split ",")) {
            if (-not $id) { continue }
            $h = Get-CimInstance Win32_Process -Filter "ProcessId=$id"
            if ($h -and $h.Name -eq "HCell.exe" -and $h.CreationDate -ge $started.AddSeconds(-2)) {
                Start-Sleep -Seconds 2
                if (Get-CimInstance Win32_Process -Filter "ProcessId=$id") {
                    Invoke-CimMethod -InputObject $h -MethodName Terminate | Out-Null
                    Write-Host "     (남은 한셀 $id 정리)"
                }
            }
        }
    }
}
if (Test-Path $pidFile) { Remove-Item $pidFile -Force }
