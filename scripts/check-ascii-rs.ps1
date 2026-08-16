# Fail if any .rs file under the given roots contains a non-ASCII byte.
# Usage: pwsh scripts/check-ascii-rs.ps1 [path ...]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Roots
)

$ErrorActionPreference = "Stop"

if (-not $Roots -or $Roots.Count -eq 0) {
    $Roots = @()
    if (Test-Path "src") { $Roots += "src" }
    if (Test-Path "crates") { $Roots += "crates" }
    if ($Roots.Count -eq 0) { $Roots = @(".") }
}

$offenders = 0
foreach ($root in $Roots) {
    Get-ChildItem -Path $root -Recurse -Filter *.rs -File | ForEach-Object {
        $bytes = [System.IO.File]::ReadAllBytes($_.FullName)
        $hits = @()
        for ($i = 0; $i -lt $bytes.Length; $i++) {
            if ($bytes[$i] -gt 0x7F) {
                $hits += "  offset $i byte 0x{0:X2}" -f $bytes[$i]
                if ($hits.Count -ge 20) { break }
            }
        }
        if ($hits.Count -gt 0) {
            Write-Host "non-ASCII in $($_.FullName):"
            $hits | ForEach-Object { Write-Host $_ }
            $offenders++
        }
    }
}

if ($offenders -gt 0) {
    Write-Host ""
    Write-Host "error: $offenders .rs file(s) contain non-ASCII bytes."
    Write-Host "Route glyphs through thoth::symbols and keep source as \u{…} escapes."
    exit 1
}

Write-Host "ok: all scanned .rs files are ASCII"
