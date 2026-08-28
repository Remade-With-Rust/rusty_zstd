//! CEILING PROBE for a decode-ahead / prefetch pipeline.
//!
//! `ZSTD_decompressSequencesLong` exists to hide ONE load: the match source, a
//! random access into the output window. If the whole window fits in cache the
//! load never stalls and the pipeline can win nothing. So price the WINDOW
//! before building the pipeline -- it bounds how far any match can reach.
fn main(){
    println!("{:<8}{:>12}{:>14}{:>16}", "level", "input MiB", "window_log", "window bytes");
    for (lvl, mib) in [(1i32,8usize),(3,8),(3,32),(5,8),(9,8),(19,8),(19,64),(22,64)] {
        let n = mib << 20;
        let p = rusty_zstd::compression_params(lvl, Some(n as u64)).unwrap();
        let w = 1u64 << p.window_log;
        println!("{:<8}{:>12}{:>14}{:>16}", format!("L{lvl}"), mib, p.window_log,
            if w >= (1<<20) { format!("{:.1} MiB", w as f64/(1u64<<20) as f64) }
            else { format!("{} KiB", w/1024) });
    }
    println!("
  A window that fits in L2 (~256 KiB-1 MiB here) never stalls on the match load.");
    println!("  A prefetch pipeline can only pay where the window EXCEEDS cache.");
}
