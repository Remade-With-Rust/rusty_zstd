//! ONE ARM PER PROCESS. The only contamination-proof way to board these.
//!
//!   cargo run --release -p rusty_zstd-bench --example armone -- <arm> <level>
//!
//! Every arm here is THREE-state: unset resolves through an env knob or a
//! dispatch, and `set_*(true|false)` FORCES. There is no public "unset", so a
//! single process cannot measure arm B after touching arm A and still trust its
//! baseline. A first attempt did exactly that and produced an identical
//! +10,462 at L13 for thirteen unrelated arms -- one stuck forced arm, read
//! thirteen times as if it were each arm's own result.
//!
//! So: baseline measured first, exactly one setter called, process exits.
use rusty_zstd as rz;
const IDS: &[&str] = &[
    "jsonlog-16m",
    "smallmsg-8m",
    "mr",
    "ooffice",
    "osdb",
    "reymont",
    "sao",
    "webster",
    "dickens",
    "mozilla",
    "nci",
    "samba",
    "xml",
    "x-ray",
];
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let arm = a.get(1).cloned().unwrap_or_default();
    let lvl: i32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    f[..n].to_vec()
                })
        })
        .collect();
    let go = || -> usize {
        srcs.iter()
            .map(|s| {
                rz::compress_with(
                    s,
                    rz::CompressOptions {
                        level: lvl,
                        checksum: false,
                    },
                )
                .unwrap()
                .len()
            })
            .sum()
    };
    let base = go(); // untouched: nothing set yet
    type S = fn(bool);
    let f: S = match arm.as_str() {
        "lazy_fill" => rz::set_lazy_fill_arm,
        "lazy_gain" => rz::set_lazy_gain_arm,
        "row" => rz::set_row_arm,
        "walk_cont" => rz::set_walk_cont_arm,
        "rep_reprobe" => rz::set_rep_reprobe_arm,
        "chain_tag" => rz::set_chain_tag_arm,
        "wide_chain" => rz::set_wide_chain_arm,
        "prime_bt" => rz::set_prime_bt_arm,
        "prime_bt_tree" => rz::set_prime_bt_tree_arm,
        "step_probe" => rz::set_step_probe_arm,
        "replen_pipe" => rz::set_replen_pipe_arm,
        "raw_skip" => rz::set_raw_skip_arm,
        "dfast_bext" => rz::set_dfast_bext_arm,
        "opt_mlbits" => rz::set_opt_mlbits_arm,
        "opt_rep" => rz::set_opt_rep_arm,
        "long_tag" => rz::set_long_tag_arm,
        "bt_depth_cached" => rz::set_bt_depth_cached_arm,
        _ => {
            println!("unknown arm {arm}");
            return;
        }
    };
    let setting = a.get(4).map(|s| s.as_str() == "on").unwrap_or(true);
    f(setting);
    let d = go() as i64 - base as i64;
    println!(
        "{}	{}	{}	{}	{}",
        arm,
        lvl,
        base,
        if setting { "on" } else { "off" },
        d
    );
}
