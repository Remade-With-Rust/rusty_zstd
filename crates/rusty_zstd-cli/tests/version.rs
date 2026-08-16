// rusty_zstd-cli integration: the M0 binary must speak --version.
use std::process::Command;

#[test]
fn version_exits_zero() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let out = Command::new(exe)
        .arg("--version")
        .output()
        .expect("spawn rzstd");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rzstd"), "{stdout}");
    assert!(stdout.contains("M6"), "{stdout}");
}

#[test]
fn help_exits_zero() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let out = Command::new(exe)
        .arg("--help")
        .output()
        .expect("spawn rzstd");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--long"), "{stdout}");
    assert!(stdout.contains("--rsyncable"), "{stdout}");
    assert!(stdout.contains("--seekable"), "{stdout}");
    assert!(
        stdout.contains("--target-compressed-block-size"),
        "{stdout}"
    );
    assert!(stdout.contains("--threads"), "{stdout}");
    assert!(stdout.contains("--single-thread"), "{stdout}");
    assert!(stdout.contains("--list"), "{stdout}");
}

#[test]
fn compress_decompress_file_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-cli-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("hello.txt");
    let zst = dir.join("hello.txt.zst");
    let round = dir.join("hello.txt.out");
    std::fs::write(&src, b"hello rusty_zstd M3").unwrap();
    let c = Command::new(exe)
        .args([
            "-z",
            "-f",
            "-3",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            round.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(std::fs::read(&round).unwrap(), b"hello rusty_zstd M3");
    let t = Command::new(exe)
        .args(["-t", zst.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        t.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&t.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn show_default_cparams_prints_zstd_line() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-cparams-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    std::fs::write(&src, vec![b'x'; 1024]).unwrap();
    let c = Command::new(exe)
        .args([
            "--show-default-cparams",
            "-f",
            "-3",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let err = String::from_utf8_lossy(&c.stderr);
    assert!(err.contains("windowLog="), "{err}");
    assert!(err.contains("strategy="), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fast_flag_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-fast-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"fast path rusty_zstd").unwrap();
    let c = Command::new(exe)
        .args([
            "--fast=3",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(d.status.success());
    assert_eq!(std::fs::read(&out).unwrap(), b"fast path rusty_zstd");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ultra_requires_flag_and_roundtrips() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-ultra-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"ultra path rusty_zstd M3").unwrap();
    let denied = Command::new(exe)
        .args([
            "-20",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !denied.status.success(),
        "levels 20-22 must require --ultra"
    );
    let c = Command::new(exe)
        .args([
            "--ultra",
            "-20",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(d.status.success());
    assert_eq!(std::fs::read(&out).unwrap(), b"ultra path rusty_zstd M3");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn zstd_option_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-zstdopt-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"zstd equals option rusty_zstd").unwrap();
    let c = Command::new(exe)
        .args([
            "--zstd=windowLog=12,strategy=1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(d.status.success());
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"zstd equals option rusty_zstd"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn train_and_dict_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-train-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut samples = Vec::new();
    for i in 0..6 {
        let p = dir.join(format!("s{i}.txt"));
        let mut body = b"the quick brown fox jumps over the lazy dog. rzstd train. ".repeat(8);
        body.push(b'0' + i);
        std::fs::write(&p, &body).unwrap();
        samples.push(p);
    }
    let dict = dir.join("t.dict");
    let mut train_args = vec![
        "--train".to_string(),
        "--maxdict=2048".to_string(),
        "-f".to_string(),
        "-o".to_string(),
        dict.to_string_lossy().into_owned(),
    ];
    for s in &samples {
        train_args.push(s.to_string_lossy().into_owned());
    }
    let t = Command::new(exe).args(&train_args).output().unwrap();
    assert!(
        t.status.success(),
        "train stderr={}",
        String::from_utf8_lossy(&t.stderr)
    );
    let src = dir.join("payload.txt");
    std::fs::write(&src, std::fs::read(&samples[0]).unwrap()).unwrap();
    let zst = dir.join("payload.txt.zst");
    let out = dir.join("payload.out");
    let c = Command::new(exe)
        .args([
            "-D",
            dict.to_str().unwrap(),
            "-f",
            "-3",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "compress -D stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-D",
            dict.to_str().unwrap(),
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "decompress -D stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&src).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn patch_from_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-patch-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let pref = dir.join("ref.bin");
    let src = dir.join("new.bin");
    let zst = dir.join("new.bin.zst");
    let out = dir.join("new.out");
    std::fs::write(&pref, b"the quick brown fox jumps over the lazy dog").unwrap();
    std::fs::write(
        &src,
        b"the quick brown fox jumps over the lazy dog and then some",
    )
    .unwrap();
    let pf = format!("--patch-from={}", pref.to_str().unwrap());
    let c = Command::new(exe)
        .args([
            "-f",
            "-3",
            &pf,
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "patch compress stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            &pf,
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "patch decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"the quick brown fox jumps over the lazy dog and then some"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-long-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    let mut body = Vec::new();
    let pat = b"long-distance-match-pattern-0123456789abcdef";
    body.extend_from_slice(pat);
    body.extend_from_slice(&[7u8; 4096]);
    body.extend_from_slice(pat);
    std::fs::write(&src, &body).unwrap();
    let c = Command::new(exe)
        .args([
            "--long=18",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "long compress stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "--long=18",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "long decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(std::fs::read(&out).unwrap(), body);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rsyncable_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-rsync-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(
        &src,
        b"rsyncable rusty_zstd block cut payload. ".repeat(200),
    )
    .unwrap();
    let c = Command::new(exe)
        .args([
            "--rsyncable",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "rsyncable compress stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "rsyncable decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"rsyncable rusty_zstd block cut payload. ".repeat(200)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn seekable_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-seek-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"seekable rusty_zstd independent frames. ".repeat(80)).unwrap();
    let c = Command::new(exe)
        .args([
            "--seekable",
            "--max-frame-size=64",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "seekable compress stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "seekable decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"seekable rusty_zstd independent frames. ".repeat(80)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn long_default_flag_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-longdef-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"default --long window 27 rusty_zstd. ".repeat(40)).unwrap();
    let c = Command::new(exe)
        .args([
            "--long",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "long default stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "--long",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "long default decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"default --long window 27 rusty_zstd. ".repeat(40)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn target_cblock_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-cblock-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(
        &src,
        b"target compressed block size rusty_zstd. ".repeat(400),
    )
    .unwrap();
    let c = Command::new(exe)
        .args([
            "--target-compressed-block-size=256",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "target-cblock stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "target-cblock decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"target compressed block size rusty_zstd. ".repeat(400)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn zstd_enable_ldm_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = std::env::temp_dir().join(format!(
        "rzstd-eldm-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"enableLdm via --zstd= rusty_zstd. ".repeat(80)).unwrap();
    let c = Command::new(exe)
        .args([
            "--zstd=enableLdm=1,ldmHashLog=12,ldmMinMatch=64",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "enableLdm stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "enableLdm decompress stderr={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"enableLdm via --zstd= rusty_zstd. ".repeat(80)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rzstd-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn threads_and_single_thread_roundtrip() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = tmp_dir("mt");
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    // > 512 KiB so -T2 --jobsize=524288 emits concatenated frames.
    std::fs::write(&src, b"mt rusty_zstd payload. ".repeat(30_000)).unwrap();
    let c = Command::new(exe)
        .args([
            "-T2",
            "--jobsize=524288",
            "--overlap-log=1",
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "T2 stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(exe)
        .args([
            "-d",
            "-f",
            "-o",
            out.to_str().unwrap(),
            zst.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "T2 d={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"mt rusty_zstd payload. ".repeat(30_000)
    );
    let listed = Command::new(exe)
        .args(["-l", zst.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(listed.status.success());
    let line = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .find(|l| {
            l.trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        })
        .unwrap_or("")
        .to_string();
    let n_frames: u32 = line
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert!(n_frames >= 2, "T2 frames via -l: {line}");
    let zst2 = dir.join("st.zst");
    let c2 = Command::new(exe)
        .args([
            "--single-thread",
            "-1",
            "-f",
            "-o",
            zst2.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(c2.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn list_and_bench() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = tmp_dir("listb");
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    std::fs::write(&src, b"list bench rusty_zstd. ".repeat(40)).unwrap();
    assert!(Command::new(exe)
        .args([
            "-1",
            "-f",
            "-o",
            zst.to_str().unwrap(),
            src.to_str().unwrap()
        ])
        .status()
        .unwrap()
        .success());
    let l = Command::new(exe)
        .args(["-l", zst.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        l.status.success(),
        "list={}",
        String::from_utf8_lossy(&l.stderr)
    );
    let stdout = String::from_utf8_lossy(&l.stdout);
    assert!(stdout.contains("Frames"), "{stdout}");
    assert!(
        stdout.contains("XXH64") || stdout.contains("----"),
        "{stdout}"
    );
    let b = Command::new(exe)
        .args(["-b1", "-i0", src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        b.status.success(),
        "bench={}",
        String::from_utf8_lossy(&b.stderr)
    );
    assert!(
        String::from_utf8_lossy(&b.stdout).contains("rzstd"),
        "{:?}",
        b
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recursive_and_rm() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = tmp_dir("rec");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let a = sub.join("a.txt");
    std::fs::write(&a, b"recursive rusty_zstd").unwrap();
    let c = Command::new(exe)
        .args(["-r", "-1", "-f", "--rm", sub.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "rec stderr={}",
        String::from_utf8_lossy(&c.stderr)
    );
    assert!(!a.exists());
    assert!(sub.join("a.txt.zst").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn env_clevel_and_aliases() {
    let rz = env!("CARGO_BIN_EXE_rzstd");
    let unz = env!("CARGO_BIN_EXE_unzstd");
    let cat = env!("CARGO_BIN_EXE_zstdcat");
    let mt = env!("CARGO_BIN_EXE_zstdmt");
    let dir = tmp_dir("alias");
    let src = dir.join("a.txt");
    let zst = dir.join("a.txt.zst");
    let out = dir.join("a.txt.out");
    std::fs::write(&src, b"alias rusty_zstd env").unwrap();
    let c = Command::new(rz)
        .env("ZSTD_CLEVEL", "1")
        .args(["-f", "-o", zst.to_str().unwrap(), src.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "env={}",
        String::from_utf8_lossy(&c.stderr)
    );
    let d = Command::new(unz)
        .args(["-f", "-o", out.to_str().unwrap(), zst.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        d.status.success(),
        "unzstd={}",
        String::from_utf8_lossy(&d.stderr)
    );
    assert_eq!(std::fs::read(&out).unwrap(), b"alias rusty_zstd env");
    let cat_out = Command::new(cat)
        .args(["-f", zst.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        cat_out.status.success(),
        "cat={}",
        String::from_utf8_lossy(&cat_out.stderr)
    );
    assert_eq!(cat_out.stdout, b"alias rusty_zstd env");
    let mt_zst = dir.join("mt.zst");
    let m = Command::new(mt)
        .args([
            "-1",
            "-f",
            "-o",
            mt_zst.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        m.status.success(),
        "zstdmt={}",
        String::from_utf8_lossy(&m.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn directory_without_r_errors() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let dir = tmp_dir("nordir");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("a.txt"), b"x").unwrap();
    let c = Command::new(exe)
        .args(["-1", "-f", sub.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!c.status.success());
    let err = String::from_utf8_lossy(&c.stderr);
    assert!(err.contains("directory"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn format_not_zstd_errors() {
    let exe = env!("CARGO_BIN_EXE_rzstd");
    let out = Command::new(exe).args(["--format=gzip"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not in this build"), "{err}");
}
