//! One operational pass over every M6 CLI-completeness function.
//!
//! Each check spawns `rzstd` (or an alias) and asserts an observable result:
//! round-trip bytes, frame count, stderr text, or keep/rm file semantics.
//! Not a speed claim.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn exe() -> &'static str {
    env!("CARGO_BIN_EXE_rzstd")
}

fn tmp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rzstd-m6ops-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ok(tag: &str, out: &Output) {
    assert!(
        out.status.success(),
        "{tag} status={:?} stderr={} stdout={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

fn rzstd(args: &[&str]) -> Output {
    Command::new(exe())
        .env_remove("ZSTD_CLEVEL")
        .env_remove("ZSTD_NBTHREADS")
        .args(args)
        .output()
        .unwrap()
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn zstd_frames(zst: &Path) -> u32 {
    let out = rzstd(&["--list", p(zst)]);
    ok("list frames", &out);
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| {
            l.trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
        .unwrap_or("")
        .to_string();
    line.split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn roundtrip(tag: &str, zst: &Path, src_bytes: &[u8], extra_d: &[&str]) {
    let outp = zst.with_extension("out");
    let mut dargs = vec!["-d", "-f", "-o", p(&outp), p(zst)];
    dargs.splice(1..1, extra_d.iter().copied());
    let d = rzstd(&dargs);
    ok(tag, &d);
    assert_eq!(std::fs::read(&outp).unwrap(), src_bytes, "{tag} bytes");
}

#[test]
fn all_cli_completeness_ops() {
    let dir = tmp();
    let small = b"cli completeness rusty_zstd. ".repeat(40);
    let big = b"cli completeness rusty_zstd. ".repeat(25_000);
    let src_s = dir.join("s.txt");
    let src_b = dir.join("b.txt");
    std::fs::write(&src_s, &small).unwrap();
    std::fs::write(&src_b, &big).unwrap();

    // -H / --help
    let h = rzstd(&["-H"]);
    ok("-H", &h);
    let help = String::from_utf8_lossy(&h.stdout);
    assert!(help.contains("--threads"), "{help}");
    assert!(help.contains("--single-thread"), "{help}");
    assert!(help.contains("--list"), "{help}");
    assert!(help.contains("--jobsize"), "{help}");
    assert!(help.contains("ZSTD_CLEVEL"), "{help}");
    assert!(help.contains("unzstd"), "{help}");

    // --format=zstd accepted; --max (level 22 + ultra)
    let z_max = dir.join("max.zst");
    ok(
        "--max --format=zstd",
        &rzstd(&["--format=zstd", "--max", "-f", "-o", p(&z_max), p(&src_s)]),
    );
    roundtrip("--max decode", &z_max, &small, &[]);

    // -t integrity
    let t = rzstd(&["-t", p(&z_max)]);
    ok("-t", &t);
    assert!(
        String::from_utf8_lossy(&t.stderr).contains("OK"),
        "t stderr={}",
        String::from_utf8_lossy(&t.stderr)
    );

    // -c stdout compress + decompress
    let c_out = rzstd(&["-c", "-1", "-f", p(&src_s)]);
    ok("-c compress", &c_out);
    let z_c = dir.join("c.zst");
    std::fs::write(&z_c, &c_out.stdout).unwrap();
    roundtrip("-c bytes", &z_c, &small, &[]);

    // -T# / --threads=# / --jobsize= / -B#  => concatenated frames on >512 KiB
    let z_t = dir.join("t2.zst");
    ok(
        "--threads=2 --jobsize=512K",
        &rzstd(&[
            "--threads=2",
            "--jobsize=512K",
            "--overlap-log=1",
            "-1",
            "-f",
            "-o",
            p(&z_t),
            p(&src_b),
        ]),
    );
    assert!(zstd_frames(&z_t) >= 2, "threads=2 should emit >=2 frames");
    roundtrip("T2 decode", &z_t, &big, &[]);

    let z_b = dir.join("bflag.zst");
    ok(
        "-T2 -B524288",
        &rzstd(&[
            "-T2",
            "-B524288",
            "--overlap-log=1",
            "-1",
            "-f",
            "-o",
            p(&z_b),
            p(&src_b),
        ]),
    );
    assert!(zstd_frames(&z_b) >= 2, "-B job split");
    roundtrip("-B decode", &z_b, &big, &[]);

    // --overlap-log=9 (full window prefix between jobs)
    let z_ov = dir.join("ov.zst");
    ok(
        "--overlap-log=9",
        &rzstd(&[
            "-T2",
            "--jobsize=524288",
            "--overlap-log=9",
            "-1",
            "-f",
            "-o",
            p(&z_ov),
            p(&src_b),
        ]),
    );
    roundtrip("overlap-log=9 decode", &z_ov, &big, &[]);

    // --single-thread: oneshot, one frame even when src > job size
    let z_st = dir.join("st.zst");
    ok(
        "--single-thread",
        &rzstd(&[
            "--single-thread",
            "-T2",
            "--jobsize=524288",
            "-1",
            "-f",
            "-o",
            p(&z_st),
            p(&src_b),
        ]),
    );
    assert_eq!(zstd_frames(&z_st), 1, "--single-thread is not -T1");
    roundtrip("single-thread decode", &z_st, &big, &[]);

    // -T0 auto workers still job-splits
    let z_t0 = dir.join("t0.zst");
    ok(
        "-T0",
        &rzstd(&[
            "-T0",
            "--jobsize=524288",
            "--overlap-log=1",
            "-1",
            "-f",
            "-o",
            p(&z_t0),
            p(&src_b),
        ]),
    );
    assert!(zstd_frames(&z_t0) >= 2, "-T0 job split");
    roundtrip("-T0 decode", &z_t0, &big, &[]);

    // --zstd=nbWorkers,jobSize,overlapLog
    let z_kv = dir.join("kv.zst");
    ok(
        "--zstd=nbWorkers",
        &rzstd(&[
            "--zstd=nbWorkers=2,jobSize=524288,overlapLog=1",
            "-1",
            "-f",
            "-o",
            p(&z_kv),
            p(&src_b),
        ]),
    );
    assert!(zstd_frames(&z_kv) >= 2, "nbWorkers via --zstd=");
    roundtrip("zstd nbWorkers decode", &z_kv, &big, &[]);

    // ZSTD_NBTHREADS env (no -T)
    let z_env_t = dir.join("envt.zst");
    let env_t = Command::new(exe())
        .env_remove("ZSTD_CLEVEL")
        .env("ZSTD_NBTHREADS", "2")
        .args([
            "--jobsize=524288",
            "--overlap-log=1",
            "-1",
            "-f",
            "-o",
            p(&z_env_t),
            p(&src_b),
        ])
        .output()
        .unwrap();
    ok("ZSTD_NBTHREADS=2", &env_t);
    assert!(zstd_frames(&z_env_t) >= 2, "ZSTD_NBTHREADS job split");
    roundtrip("ZSTD_NBTHREADS decode", &z_env_t, &big, &[]);

    // ZSTD_CLEVEL env; flags win over env
    let z_env_l = dir.join("envl.zst");
    let env_l = Command::new(exe())
        .env("ZSTD_CLEVEL", "1")
        .env_remove("ZSTD_NBTHREADS")
        .args(["-f", "-o", p(&z_env_l), p(&src_s)])
        .output()
        .unwrap();
    ok("ZSTD_CLEVEL=1", &env_l);
    roundtrip("ZSTD_CLEVEL decode", &z_env_l, &small, &[]);

    let show = Command::new(exe())
        .env("ZSTD_CLEVEL", "19")
        .env_remove("ZSTD_NBTHREADS")
        .args([
            "--show-default-cparams",
            "-1",
            "-f",
            "-o",
            p(&dir.join("flagwin.zst")),
            p(&src_s),
        ])
        .output()
        .unwrap();
    ok("flag wins ZSTD_CLEVEL", &show);
    let cparams = String::from_utf8_lossy(&show.stderr);
    assert!(
        cparams.contains("strategy=1"),
        " -1 must win over ZSTD_CLEVEL=19: {cparams}"
    );

    // malformed env: warn + ignore
    let bad = Command::new(exe())
        .env("ZSTD_CLEVEL", "nope")
        .env("ZSTD_NBTHREADS", "xyz")
        .args(["-1", "-f", "-o", p(&dir.join("badenv.zst")), p(&src_s)])
        .output()
        .unwrap();
    ok("malformed env ignored", &bad);
    let warn = String::from_utf8_lossy(&bad.stderr);
    assert!(warn.contains("ignoring invalid ZSTD_CLEVEL"), "{warn}");
    assert!(warn.contains("ignoring invalid ZSTD_NBTHREADS"), "{warn}");

    // -l already used; --list header
    let listed = rzstd(&["-l", p(&z_max)]);
    ok("-l", &listed);
    let ls = String::from_utf8_lossy(&listed.stdout);
    assert!(ls.contains("Frames"), "{ls}");
    assert!(ls.contains("XXH64"), "{ls}");

    // -b# -e# -i0  (two levels, one loop each)
    let bench = rzstd(&["-b1", "-e2", "-i0", p(&src_s)]);
    ok("-b -e -i0", &bench);
    let bs = String::from_utf8_lossy(&bench.stdout);
    assert!(bs.contains("1#rzstd"), "{bs}");
    assert!(bs.contains("2#rzstd"), "{bs}");

    // -r recurse (keep); --rm; -k
    let rec = dir.join("rec");
    let keepd = rec.join("keep");
    let rmd = rec.join("rm");
    std::fs::create_dir_all(&keepd).unwrap();
    std::fs::create_dir_all(&rmd).unwrap();
    let keep_src = keepd.join("a.txt");
    let rm_src = rmd.join("b.txt");
    std::fs::write(&keep_src, b"keep me").unwrap();
    std::fs::write(&rm_src, b"delete me").unwrap();
    ok("-r -k", &rzstd(&["-r", "-k", "-1", "-f", p(&keepd)]));
    assert!(keep_src.is_file(), "-k must keep source");
    assert!(keepd.join("a.txt.zst").is_file());
    ok("-r --rm", &rzstd(&["-r", "--rm", "-1", "-f", p(&rmd)]));
    assert!(!rm_src.exists(), "--rm must delete source");
    assert!(rmd.join("b.txt.zst").is_file());

    // --train -r on a sample directory
    let samples = dir.join("samples");
    std::fs::create_dir_all(&samples).unwrap();
    for i in 0..6 {
        let mut body = b"the quick brown fox jumps over the lazy dog. rzstd train. ".repeat(8);
        body.push(b'0' + i);
        std::fs::write(samples.join(format!("s{i}.txt")), body).unwrap();
    }
    let dict = dir.join("t.dict");
    ok(
        "--train -r",
        &rzstd(&[
            "--train",
            "-r",
            "--maxdict=2048",
            "-f",
            "-o",
            p(&dict),
            p(&samples),
        ]),
    );
    assert!(dict.is_file());
    let z_d = dir.join("dict.zst");
    ok(
        "compress -D after train -r",
        &rzstd(&["-D", p(&dict), "-1", "-f", "-o", p(&z_d), p(&src_s)]),
    );
    let out_d = dir.join("dict.out");
    ok(
        "decompress -D",
        &rzstd(&["-d", "-D", p(&dict), "-f", "-o", p(&out_d), p(&z_d)]),
    );
    assert_eq!(std::fs::read(&out_d).unwrap(), small);

    // -M / --memory: --long=16 window is 64 KiB; src > window so Window_Descriptor is present.
    let z_long = dir.join("long.zst");
    ok(
        "--long=16",
        &rzstd(&["--long=16", "-1", "-f", "-o", p(&z_long), p(&src_b)]),
    );
    let too_small = rzstd(&[
        "-d",
        "-M",
        "32768",
        "-f",
        "-o",
        p(&dir.join("m.fail")),
        p(&z_long),
    ]);
    assert!(
        !too_small.status.success(),
        "-M 32768 must reject 64 KiB window: {}",
        String::from_utf8_lossy(&too_small.stderr)
    );
    assert!(
        String::from_utf8_lossy(&too_small.stderr).contains("window"),
        "memory cap stderr={}",
        String::from_utf8_lossy(&too_small.stderr)
    );
    roundtrip("-M 2MiB", &z_long, &big, &["--memory=2MiB"]);

    // aliases: unzstd / zstdcat / zstdmt
    let z_al = dir.join("alias.zst");
    ok(
        "alias src",
        &rzstd(&["-1", "-f", "-o", p(&z_al), p(&src_s)]),
    );
    let unz_out = dir.join("unz.out");
    let unz = Command::new(env!("CARGO_BIN_EXE_unzstd"))
        .args(["-f", "-o", p(&unz_out), p(&z_al)])
        .output()
        .unwrap();
    ok("unzstd", &unz);
    assert_eq!(std::fs::read(&unz_out).unwrap(), small);

    let cat = Command::new(env!("CARGO_BIN_EXE_zstdcat"))
        .args(["-f", p(&z_al)])
        .output()
        .unwrap();
    ok("zstdcat", &cat);
    assert_eq!(cat.stdout, small);

    let z_mt = dir.join("mt.zst");
    let mt = Command::new(env!("CARGO_BIN_EXE_zstdmt"))
        .args(["-1", "-f", "-o", p(&z_mt), p(&src_s)])
        .output()
        .unwrap();
    ok("zstdmt", &mt);
    roundtrip("zstdmt decode", &z_mt, &small, &[]);

    let _ = std::fs::remove_dir_all(&dir);
}
