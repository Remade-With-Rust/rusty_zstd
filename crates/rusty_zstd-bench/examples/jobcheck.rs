//! Did the MT arm actually RUN? resolve_job_size does `.max(overlap)`, and at
//! L19/L22 the overlap is the whole window — so an explicit small job_size may
//! be silently raised above the source and fall back to oneshot.
fn main() {
    for lvl in [1i32, 3, 13, 19, 22] {
        for &req in &[0usize, 1 << 20, 4 << 20] {
            let p = rusty_zstd::compression_params(lvl, Some(4 << 20)).unwrap();
            let ov = rusty_zstd::overlap_size(p.window_log, 0, p.strategy);
            let job = rusty_zstd::resolve_job_size(req, p.window_log, ov);
            let ov1 = rusty_zstd::overlap_size(p.window_log, 1, p.strategy);
            let job1 = rusty_zstd::resolve_job_size(req, p.window_log, ov1);
            println!(
                "L{lvl:<2} wlog {:>2} | req {:>5} KiB | overlap(def) {:>7} KiB -> job {:>7} KiB {:<12} | overlap(ovlog=1) {} -> job {:>7} KiB",
                p.window_log, req >> 10, ov >> 10, job >> 10,
                if job >= (4 << 20) { "[>=4MiB SRC: ONESHOT]" } else { "" },
                ov1, job1 >> 10
            );
        }
    }
}
