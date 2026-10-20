# Copyright 2026 yoomin53 (https://github.com/yoomin53)
# SPDX-License-Identifier: Apache-2.0
#
# rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현
#
# 설치된 한셀로 문서를 열어 다른 형식으로 다시 저장한다 (차분 분석용).
# 직접 만든 XML .cell 에 원하는 hs: 값을 넣고 레거시/바이너리 형식으로 바꿔
# 각 필드가 바이너리의 어디에 기록되는지 대응시킬 때 쓴다.
#
#   -Format: cell(XML, -2006) | bin(-2007) | legacy(-2005) | xlsx(51) | xls(56)
#   -Text  : 지정하면 첫 시트 셀 범위의 표시 문자열(Range.Text)을 TSV 로 출력
#
# 사용: powershell -File convert.ps1 -In a.cell -Out a.legacy.cell -Format legacy
param(
    [Parameter(Mandatory = $true)][string]$In,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Format = "legacy",
    [string]$Text = ""
)
$ErrorActionPreference = "Stop"
$codes = @{ "cell" = -2006; "bin" = -2007; "legacy" = -2005; "xlsx" = 51; "xls" = 56 }
$app = New-Object -ComObject HCell.Application
try {
    try { $app.Visible = $false } catch {}
    try { $app.DisplayAlerts = $false } catch {}
    $wb = $app.Workbooks.Open((Resolve-Path $In).Path)
    if ($Text) {
        $ws = $wb.Worksheets.Item(1)
        foreach ($row in $ws.Range($Text).Rows) {
            ($row.Cells | ForEach-Object { $_.Text }) -join "`t"
        }
    }
    if (Test-Path $Out) { Remove-Item $Out -Force }
    $wb.SaveAs([IO.Path]::GetFullPath($Out), $codes[$Format])
    $wb.Close($false)
} finally {
    try { $app.Quit() } catch {}
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($app) | Out-Null
}
