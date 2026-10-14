# codec-board.ps1 -- the README's codec-only table: in-process, no checksum, no
# I/O, six silesia files capped at 8 MiB (C's own `zstd -b` asserts on whole
# mozilla at L3, so the house `rzstd-bench --m7-speed` cannot take them all).
#
#   cargo build --release -p rusty_zstd-bench --example codec1
#   pwsh bench/codec-board.ps1 [-Levels 1,3,9,19]
#
# Per cell, in m7-speed's shape: C, us, C -- C is `zstd -b<L> -i3 -T1
# --no-progress` (its own timer, best of its loops), us is the `codec1` example
# (`bench_roundtrip_clocked`, the m7-speed arm, best of two 3 s runs). Every
# process pinned to one core (affinity 0x4) at High priority; each arm keeps
# its best MB/s. Reported: our MB/s / C's MB/s (above 1x we are faster).
param([int[]]$Levels = @(1, 3, 9, 19),
      [string[]]$Files = @('dickens', 'samba', 'x-ray', 'mozilla', 'nci', 'xml'),
      [string]$Work = "$env:TEMP\rzstd-codec-board")
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$zexe = Join-Path $root 'third_party\zstd\extracted\zstd-v1.5.7-win64\zstd.exe'
$usexe = Join-Path $root 'target\release\examples\codec1.exe'
foreach ($x in @($zexe, $usexe)) { if (-not (Test-Path $x)) { throw "missing $x" } }
New-Item -ItemType Directory -Force $Work | Out-Null
foreach ($f in $Files) {
  $src = Join-Path $root "corpora\data\silesia\$f"; $n = [Math]::Min((Get-Item $src).Length, 8MB)
  $buf = New-Object byte[] $n; $fs = [IO.File]::OpenRead($src); [void]$fs.Read($buf, 0, $n); $fs.Close()
  [IO.File]::WriteAllBytes((Join-Path $Work $f), $buf)
}
function RunP($exe, $argv) {
  $o = [IO.Path]::GetTempFileName()
  $p = Start-Process -FilePath $exe -ArgumentList $argv -PassThru -WindowStyle Hidden `
       -RedirectStandardOutput $o -RedirectStandardError "$o.e"
  $null = $p.Handle
  try { $p.ProcessorAffinity = [IntPtr]4; $p.PriorityClass = 'High' } catch {}
  $p.WaitForExit()
  $t = (Get-Content $o -Raw) + (Get-Content "$o.e" -Raw)
  Remove-Item $o, "$o.e" -ErrorAction SilentlyContinue
  if ($p.ExitCode -ne 0) { throw "$exe $argv exited $($p.ExitCode): $t" }
  $t
}
$best = @{}
function Keep($k, $v) { if (-not $best.ContainsKey($k) -or $v -gt $best[$k]) { $best[$k] = $v } }
foreach ($L in $Levels) {
  foreach ($f in $Files) {
    $path = Join-Path $Work $f
    foreach ($arm in @('c', 'us', 'c')) {
      if ($arm -eq 'c') {
        $t = RunP $zexe @("-b$L", '-i3', '-T1', '--no-progress', $path)
        $m = [regex]::Matches($t, '([0-9.]+) MB/s, +([0-9.]+) MB/s')
        if ($m.Count -eq 0) { throw "zstd -b output not parsed: $t" }
        $last = $m[$m.Count - 1]
        Keep "$L|$f|c|enc" ([double]$last.Groups[1].Value); Keep "$L|$f|c|dec" ([double]$last.Groups[2].Value)
      } else {
        $t = RunP $usexe @($path, "$L")
        $v = $t.Trim() -split '\s+'
        Keep "$L|$f|us|enc" ([double]$v[0]); Keep "$L|$f|us|dec" ([double]$v[1])
      }
    }
  }
}
"method: in-process, capped 8 MiB, pinned core 2 High, C-us-C per cell, best of each; cells = our MB/s / C's MB/s"
foreach ($op in @('enc', 'dec')) {
  "--- $op"
  "{0,-5}" -f 'level' + (($Files | ForEach-Object { "{0,10}" -f $_ }) -join '')
  foreach ($L in $Levels) {
    "{0,-5}" -f "L$L" + (($Files | ForEach-Object { "{0,10}" -f ("{0:N2}x" -f ($best["$L|$_|us|$op"] / $best["$L|$_|c|$op"])) }) -join '')
  }
  "raw MB/s (us / C):"
  foreach ($L in $Levels) {
    "  L$L " + (($Files | ForEach-Object { "{0}:{1:F0}/{2:F0}" -f $_, $best["$L|$_|us|$op"], $best["$L|$_|c|$op"] }) -join '  ')
  }
}
