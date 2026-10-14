# cli-board.ps1 -- the README's "default deployment" table: both CLIs at their
# real defaults over the 19-file corpus capped at 8 MiB per file.
#
#   pwsh bench/cli-board.ps1 [-Levels 1,3,9,19] [-Runs 10]
#
# Method (codec-measurement):
#   * one invocation per run over all 19 files: `zstd -<L> -f -q <files>` and
#     `rzstd -<L> -f -q <files>`, then `-d -f -q <files>.zst` -- no flags beyond
#     the level, so both use their defaults (one thread, content checksum);
#   * each arm has its OWN staged copy of the corpus;
#   * every process pinned to one core (affinity 0x4) at High priority, timed
#     with a Stopwatch (wall: this is the whole-program number, I/O included);
#   * arms alternate which goes first, run by run;
#   * work parity: each CLI decodes the OTHER's output and it must equal the
#     original, every level (a mismatch voids the row);
#   * reported, C time / our time (above 1x we are faster):
#       best  -- C's fastest run over our fastest run. The floor is what
#                survives a loaded host, so this is the headline estimator;
#       med   -- median of the paired per-run ratios;
#       range -- min-max of the paired ratios, so the spread is visible.
#     Raw per-run times go to <Work>/runs.csv.
param([int[]]$Levels = @(1, 3, 9, 19), [int]$Runs = 10, [string]$Work = "$env:TEMP\rzstd-cli-board")
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$C = Join-Path $root 'third_party\zstd\extracted\zstd-v1.5.7-win64\zstd.exe'
$US = Join-Path $root 'target\release\rzstd.exe'
foreach ($x in @($C, $US)) { if (-not (Test-Path $x)) { throw "missing $x" } }
$cap = 8MB
$srcs = @(Get-ChildItem (Join-Path $root 'corpora\data\silesia') -File) +
        @(Get-ChildItem (Join-Path $root 'corpora\data\generated') -File) |
        Where-Object { $_.Name -ne 'SHA256SUMS' } | Sort-Object Name
if ($srcs.Count -ne 19) { throw "expected 19 corpus files, found $($srcs.Count)" }

function Stage($dir) {
  if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
  New-Item -ItemType Directory -Force $dir | Out-Null
  foreach ($s in $srcs) {
    $n = [Math]::Min($s.Length, $cap)
    $buf = New-Object byte[] $n
    $fs = [IO.File]::OpenRead($s.FullName); [void]$fs.Read($buf, 0, $n); $fs.Close()
    [IO.File]::WriteAllBytes((Join-Path $dir $s.Name), $buf)
  }
}
function Run($exe, $argv, $dir) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $exe -ArgumentList $argv -WorkingDirectory $dir -PassThru -WindowStyle Hidden
  $null = $p.Handle
  try { $p.ProcessorAffinity = [IntPtr]4; $p.PriorityClass = 'High' } catch {}
  $p.WaitForExit(); $sw.Stop()
  if ($p.ExitCode -ne 0) { throw "$exe $argv exited $($p.ExitCode)" }
  $sw.Elapsed.TotalMilliseconds
}
function Hash($path) { (Get-FileHash -Algorithm SHA256 $path).Hash }

$master = Join-Path $Work 'master'; Stage $master
$total = ($srcs | ForEach-Object { [Math]::Min($_.Length, $cap) } | Measure-Object -Sum).Sum
$want = @{}; foreach ($s in $srcs) { $want[$s.Name] = Hash (Join-Path $master $s.Name) }
$dirs = @{ c = (Join-Path $Work 'c'); us = (Join-Path $Work 'us') }
Set-Content (Join-Path $Work 'runs.csv') 'level,run,enc_c_ms,enc_us_ms,dec_c_ms,dec_us_ms'
Stage $dirs.c; Stage $dirs.us
$names = $srcs | ForEach-Object { $_.Name }
"method: 19 files capped at 8 MiB ($total bytes), defaults, pinned core 2 High, wall, $Runs alternating runs per level"
"{0,-5} {1,-32} {2,-32} {3,10} {4,10}" -f 'level', 'encode C/us best (med; range)', 'decode C/us best (med; range)', 'C bytes', 'us bytes'
foreach ($L in $Levels) {
  $enc = @(); $dec = @(); $tec = @(); $teu = @(); $tdc = @(); $tdu = @()
  for ($r = 0; $r -lt $Runs; $r++) {
    $order = if ($r % 2 -eq 0) { @('c', 'us') } else { @('us', 'c') }
    $te = @{}; $td = @{}
    foreach ($a in $order) {
      $exe = if ($a -eq 'c') { $C } else { $US }
      $te[$a] = Run $exe (@("-$L", '-f', '-q') + $names) $dirs[$a]
    }
    foreach ($a in $order) {
      $exe = if ($a -eq 'c') { $C } else { $US }
      $td[$a] = Run $exe (@('-d', '-f', '-q') + ($names | ForEach-Object { "$_.zst" })) $dirs[$a]
    }
    $enc += $te.c / $te.us; $dec += $td.c / $td.us
    $tec += $te.c; $teu += $te.us; $tdc += $td.c; $tdu += $td.us
    Add-Content (Join-Path $Work 'runs.csv') ("L{0},{1},{2:F1},{3:F1},{4:F1},{5:F1}" -f $L, $r, $te.c, $te.us, $td.c, $td.us)
  }
  # Work parity: each CLI decodes the OTHER's frames; both must give back the original.
  $ok = $true
  foreach ($n in $names) {
    foreach ($pair in @(@($C, $dirs.us), @($US, $dirs.c))) {
      $out = Join-Path $Work "x-$n"
      Run $pair[0] @('-d', '-f', '-q', (Join-Path $pair[1] "$n.zst"), '-o', $out) $Work | Out-Null
      if ((Hash $out) -ne $want[$n]) { $ok = $false; "VOID L$L ${n}: cross-decode mismatch" }
      Remove-Item $out
    }
  }
  $cb = ($names | ForEach-Object { (Get-Item (Join-Path $dirs.c "$_.zst")).Length } | Measure-Object -Sum).Sum
  $ub = ($names | ForEach-Object { (Get-Item (Join-Path $dirs.us "$_.zst")).Length } | Measure-Object -Sum).Sum
  function Med($xs) { $s = @($xs | Sort-Object); if ($s.Count % 2) { $s[[int]($s.Count / 2)] } else { ($s[$s.Count / 2 - 1] + $s[$s.Count / 2]) / 2 } }
  function Cell($tc, $tu, $ratios) {
    $best = ($tc | Measure-Object -Minimum).Minimum / ($tu | Measure-Object -Minimum).Minimum
    $m = $ratios | Measure-Object -Minimum -Maximum
    "{0:N2}x ({1:N2}; {2:N2}-{3:N2})" -f $best, (Med $ratios), $m.Minimum, $m.Maximum
  }
  "{0,-5} {1,-32} {2,-32} {3,10} {4,10}{5}" -f "L$L", (Cell $tec $teu $enc), (Cell $tdc $tdu $dec),
    $cb, $ub, $(if ($ok) { '' } else { '  VOID' })
}
