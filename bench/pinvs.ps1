# Generic paired A/B for two command lines, pinned, High priority, CPU time.
# Ported from rusty_h264 bench/pinvs.ps1 (codec-measurement).
# The Rust harness (rzstd-bench) is the standing timer; this script is the
# Windows shape for later same-binary A/B once a codec arm exists.
#
#   pwsh bench/pinvs.ps1 -AExe zstd.exe -AArgs @('-T1','-1','-c','file') `
#                        -BExe zstd.exe -BArgs @('-T1','-1','-c','file') -Pairs 15

param([string]$AExe, [string[]]$AArgs, [string]$BExe, [string[]]$BArgs,
      [int]$Pairs = 15, [string]$ALabel = 'A', [string]$BLabel = 'B',
      [int]$FloorMs = 0)

function Run($exe, $argv) {
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $exe -ArgumentList $argv -PassThru -WindowStyle Hidden
  $null = $p.Handle
  $p.ProcessorAffinity = [IntPtr]4; $p.PriorityClass = 'High'; $p.WaitForExit()
  $sw.Stop()
  [pscustomobject]@{ Cpu = $p.TotalProcessorTime.TotalMilliseconds; Wall = $sw.Elapsed.TotalMilliseconds }
}
$wins = 0; $ratios = @(); $ta_all = @(); $tb_all = @(); $wa_all = @(); $wb_all = @()
1..$Pairs | ForEach-Object {
  if ($_ % 2 -eq 0) { $a = Run $AExe $AArgs; $b = Run $BExe $BArgs }
  else              { $b = Run $BExe $BArgs; $a = Run $AExe $AArgs }
  $ta = $a.Cpu; $tb = $b.Cpu; $wa = $a.Wall; $wb = $b.Wall
  if ($ta -gt 0 -and $tb -gt 0 -and $wa -gt 0 -and $wb -gt 0) {
    $r = $ta / $tb; $ratios += $r; $ta_all += $ta; $tb_all += $tb
    $wa_all += $wa; $wb_all += $wb
    if ($tb -lt $ta) { $wins++ }
    $ba = $ta / $wa; $bb = $tb / $wb
    "pair {0,2}: {1} cpu {2,8:N0} wall {3,8:N0} busy {4:N2}   {5} cpu {6,8:N0} wall {7,8:N0} busy {8:N2}   ratio {9:N3}" -f `
      $_, $ALabel, $ta, $wa, $ba, $BLabel, $tb, $wb, $bb, $r
    if ($ba -gt 1.15 -or $bb -gt 1.15) {
      "         !! cores-busy {0:N2}/{1:N2} on a 1-core pin: nested threads thrashing -- not a 1T number" -f $ba, $bb
    }
    if ($_ -eq 1 -and $FloorMs -gt 0) {
      $wmin = [Math]::Min($wa, $wb)
      if ($wmin -gt 2.5 * $FloorMs) {
        "!! LOADED: pair-1 wall {0:N0} ms is {1:N1}x the floor ({2} ms). Abort -- do not quote." -f `
          $wmin, ($wmin / $FloorMs), $FloorMs
        exit 2
      }
    }
  } else { "pair {0,2}: INSTRUMENT FAILED - dropped" -f $_ }
}
$n = $ratios.Count
if ($n -eq 0) { "ALL PAIRS FAILED - no usable samples"; exit 1 }
$med = ($ratios | Sort-Object)[[int]($n/2)]
$z = ($wins - $n/2.0) / (0.5 * [Math]::Sqrt($n))
"---"
function Med($xs) { ($xs | Sort-Object)[[int]($xs.Count/2)] }
$ma = Med $ta_all; $mb = Med $tb_all
$mwa = Med $wa_all; $mwb = Med $wb_all
"{0} median CPU {1:N0} ms  wall {2:N0} ms  busy {3:N2}   {4} median CPU {5:N0} ms  wall {6:N0} ms  busy {7:N2}" -f `
  $ALabel, $ma, $mwa, ($ma / $mwa), $BLabel, $mb, $mwb, ($mb / $mwb)
$minMed = [Math]::Min($ma,$mb)
if ($minMed -lt 500) {
  "!! WORKLOAD TOO SHORT: median arm {0:N0} ms is ~{1:N0} scheduler ticks (15.6 ms each)." -f $minMed, ($minMed/15.6)
  "!! The ratio below is timer QUANTISATION, not a measurement. Lengthen the workload"
  "!! until BOTH arms run >= ~15 s (codec-measurement 5)."
}
if ($n -lt $Pairs) {
  "!! {0} of {1} pairs were DROPPED (instrument returned 0/non-finite)." -f ($Pairs-$n), $Pairs
}
"median ratio {0}/{1} = {2:N3}x   {1} faster in {3}/{4}, z={5:N2}" -f `
  $ALabel, $BLabel, $med, $wins, $n, $z
