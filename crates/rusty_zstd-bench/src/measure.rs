//! Pinned child-process measurement (codec-measurement shape).
//!
//! Windows: affinity mask 4 (CPU 2), High priority, CPU time + wall + peak RSS.
//! Other OS: wall only, method line must say so.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::ledger::{CBench, Oneshot};
use crate::oracle::{file_sha256, Oracle};

pub struct PinSample {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub cpu_ms: Option<f64>,
    pub wall_ms: f64,
    pub peak_rss_bytes: Option<u64>,
}

pub fn pin_command(exe: &Path, args: &[String], stdin: Option<&[u8]>) -> Result<PinSample, String> {
    #[cfg(windows)]
    {
        pin_command_windows(exe, args, stdin)
    }
    #[cfg(not(windows))]
    {
        pin_command_unix(exe, args, stdin)
    }
}

#[cfg(not(windows))]
fn pin_command_unix(
    exe: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<PinSample, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let wall = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
    if let Some(bytes) = stdin {
        use std::io::Write;
        if let Some(mut sin) = child.stdin.take() {
            sin.write_all(bytes).map_err(|e| e.to_string())?;
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("wait {}: {e}", exe.display()))?;
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
    Ok(PinSample {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        cpu_ms: None,
        wall_ms,
        peak_rss_bytes: None,
    })
}

#[cfg(windows)]
fn pin_command_windows(
    exe: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<PinSample, String> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, SetPriorityClass, SetProcessAffinityMask, HIGH_PRIORITY_CLASS,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let wall = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
    let handle = child.as_raw_handle() as HANDLE;
    unsafe {
        let _ = SetProcessAffinityMask(handle, 4);
        let _ = SetPriorityClass(handle, HIGH_PRIORITY_CLASS);
    }
    if let Some(bytes) = stdin {
        use std::io::Write;
        if let Some(mut sin) = child.stdin.take() {
            sin.write_all(bytes).map_err(|e| e.to_string())?;
        }
    }

    let stdout_h = child.stdout.take();
    let stderr_h = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut so) = stdout_h {
            let _ = so.read_to_end(&mut buf);
        }
        buf
    });
    let t_err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut se) = stderr_h {
            let _ = se.read_to_end(&mut buf);
        }
        buf
    });

    let status = child.wait().map_err(|e| e.to_string())?;
    let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;
    let stdout_buf = t_out.join().unwrap_or_default();
    let stderr_buf = t_err.join().unwrap_or_default();

    let mut cpu_ms = None;
    let mut peak_rss = None;
    unsafe {
        let mut create = zero_filetime();
        let mut exit = zero_filetime();
        let mut kernel = zero_filetime();
        let mut user = zero_filetime();
        if GetProcessTimes(handle, &mut create, &mut exit, &mut kernel, &mut user) != 0 {
            cpu_ms = Some(filetime_ms(kernel) + filetime_ms(user));
        }
        let mut pmc = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(handle, &mut pmc, pmc.cb) != 0 {
            peak_rss = Some(pmc.PeakWorkingSetSize as u64);
        }
    }
    Ok(PinSample {
        status: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        cpu_ms,
        wall_ms,
        peak_rss_bytes: peak_rss,
    })
}

#[cfg(windows)]
fn zero_filetime() -> windows_sys::Win32::Foundation::FILETIME {
    windows_sys::Win32::Foundation::FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }
}

#[cfg(windows)]
fn filetime_ms(ft: windows_sys::Win32::Foundation::FILETIME) -> f64 {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | u64::from(ft.dwLowDateTime);
    ticks as f64 / 10_000.0
}

/// Pin this process (affinity=4, High) so in-process us arms match C child pinning.
pub fn pin_current_process() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, SetPriorityClass, SetProcessAffinityMask, HIGH_PRIORITY_CLASS,
        };
        let h = GetCurrentProcess();
        let _ = SetProcessAffinityMask(h, 4);
        let _ = SetPriorityClass(h, HIGH_PRIORITY_CLASS);
    }
}

/// CPU cycles this thread has actually executed.
///
/// Frequency-invariant work measure (codec-measurement 2/15). A thermally
/// throttled box runs the same code in the same number of cycles but more
/// wall milliseconds, and cycles do not accrue while descheduled -- so this
/// is the only figure here that survives the mid-session throttling observed
/// on this machine (C's own unchanged binary read 442 -> 201 MB/s across one
/// 5-minute session). Returns `None` off Windows; callers fall back to wall.
pub fn thread_cycles() -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::GetCurrentThread;
        use windows_sys::Win32::System::WindowsProgramming::QueryThreadCycleTime;
        let mut cycles: u64 = 0;
        if QueryThreadCycleTime(GetCurrentThread(), &mut cycles) == 0 {
            return None;
        }
        Some(cycles)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Peak working set of THIS process, in bytes.
///
/// Mission 7 sets an RSS target (<= 1.2x C at the same windowLog / nbWorkers)
/// that had no instrument behind it: peak RSS was captured for the C child but
/// never for the in-process `us` arm. Peak is monotonic for the process
/// lifetime, so a board reports the high-water mark across the whole run --
/// read it per level, not as a per-file figure.
pub fn current_peak_rss() -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut pmc = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) == 0 {
            return None;
        }
        Some(pmc.PeakWorkingSetSize as u64)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Current process user+kernel CPU time in milliseconds.
pub fn process_cpu_ms() -> Option<f64> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let h = GetCurrentProcess();
        let mut create = zero_filetime();
        let mut exit = zero_filetime();
        let mut kernel = zero_filetime();
        let mut user = zero_filetime();
        if GetProcessTimes(h, &mut create, &mut exit, &mut kernel, &mut user) == 0 {
            return None;
        }
        Some(filetime_ms(kernel) + filetime_ms(user))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub struct Roundtrip {
    pub ok: bool,
    pub compressed_bytes: u64,
    pub oneshot: Oneshot,
}

pub fn oneshot_roundtrip(oracle: &Oracle, src: &Path, level: i32) -> Result<Roundtrip, String> {
    let parent = src.parent().ok_or("src has no parent")?;
    let zst = parent.join(format!(
        "{}.L{level}.zst",
        src.file_name().unwrap().to_string_lossy()
    ));
    let raw = parent.join(format!(
        "{}.L{level}.out",
        src.file_name().unwrap().to_string_lossy()
    ));

    let c_args = vec![
        format!("-{level}"),
        "-T1".into(),
        "-f".into(),
        "-o".into(),
        zst.to_string_lossy().into_owned(),
        src.to_string_lossy().into_owned(),
    ];
    let compress = pin_command(&oracle.path, &c_args, None)?;
    if compress.status != 0 {
        return Err(format!("compress: {}", compress.stderr.trim()));
    }
    let compressed_bytes = std::fs::metadata(&zst)
        .map_err(|e| format!("stat {}: {e}", zst.display()))?
        .len();

    let d_args = vec![
        "-d".into(),
        "-f".into(),
        "-o".into(),
        raw.to_string_lossy().into_owned(),
        zst.to_string_lossy().into_owned(),
    ];
    let decompress = pin_command(&oracle.path, &d_args, None)?;
    if decompress.status != 0 {
        return Err(format!("decompress: {}", decompress.stderr.trim()));
    }

    let src_hash = file_sha256(src)?;
    let out_hash = file_sha256(&raw)?;
    let ok = src_hash == out_hash;

    let _ = std::fs::remove_file(&zst);
    let _ = std::fs::remove_file(&raw);

    Ok(Roundtrip {
        ok,
        compressed_bytes,
        oneshot: Oneshot {
            compress_cpu_ms: compress.cpu_ms,
            compress_wall_ms: compress.wall_ms,
            compress_peak_rss_bytes: compress.peak_rss_bytes,
            decompress_cpu_ms: decompress.cpu_ms,
            decompress_wall_ms: decompress.wall_ms,
            decompress_peak_rss_bytes: decompress.peak_rss_bytes,
        },
    })
}

/// Parse facebook/zstd `-b` result line.
///
/// Progress uses `\r` rewrites. Piped stdout is often one logical line with
/// many carriage returns; take the **last** fragment that contains two `MB/s`
/// figures.
pub fn parse_zstd_bench(stdout: &str) -> Result<CBench, String> {
    let fragments: Vec<&str> = stdout
        .split(['\n', '\r'])
        .filter(|s| !s.trim().is_empty())
        .collect();
    let line = fragments
        .iter()
        .rev()
        .find(|l| l.matches("MB/s").count() >= 2 && l.contains("->"))
        .copied()
        .ok_or_else(|| format!("no zstd -b result line with two MB/s in: {stdout:?}"))?;

    let after_arrow = line.split_once("->").map(|(_, r)| r).ok_or("missing ->")?;
    let mut mbps = Vec::new();
    for token in after_arrow.split(',') {
        let t = token.trim();
        if t.contains("MB/s") {
            if let Some(num) = t.split_whitespace().next() {
                if let Ok(v) = num.parse::<f64>() {
                    mbps.push(v);
                }
            }
        }
    }
    if mbps.len() < 2 {
        return Err(format!("could not parse two MB/s figures from {line:?}"));
    }
    let compressed = after_arrow
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(CBench {
        compress_mbps: mbps[0],
        decompress_mbps: mbps[1],
        compressed_bytes_reported: compressed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_typical_b_line() {
        let out =
            " 3#zeros-32m       :  33554432 ->       112 (x299593.14),  123.4 MB/s ,  456.7 MB/s\n";
        let b = parse_zstd_bench(out).unwrap();
        assert!((b.compress_mbps - 123.4).abs() < 0.01);
        assert!((b.decompress_mbps - 456.7).abs() < 0.01);
        assert_eq!(b.compressed_bytes_reported, 112);
    }

    #[test]
    fn parse_cr_progress_takes_last_complete() {
        let out = " |-zeros-32m :  33554432 ->      1043 (x32171.1), 4169.6 MB/s \r |-zeros-32m :  33554432 ->      1043 (x32171.1), 4169.6 MB/s, 7614.4 MB/s\r 1#\n";
        let b = parse_zstd_bench(out).unwrap();
        assert!((b.compress_mbps - 4169.6).abs() < 0.01);
        assert!((b.decompress_mbps - 7614.4).abs() < 0.01);
        assert_eq!(b.compressed_bytes_reported, 1043);
    }
}
