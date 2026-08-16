//! Dual gate: dictionaries and `--patch-from` both directions vs C zstd.
//!
//! Skips when the pinned oracle is not on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_zstd::{
    compress_using_dict, compress_using_prefix, decompress_using_dict, decompress_using_prefix,
    train, Dictionary, TrainOptions,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/rusty_zstd -> repo root")
        .to_path_buf()
}

fn find_oracle() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTY_ZSTD_ORACLE") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let extracted = repo_root().join("third_party/zstd/extracted");
    walk(&extracted).ok()?.into_iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("zstd.exe") || n.eq_ignore_ascii_case("zstd"))
    })
}

fn walk(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for ent in fs::read_dir(dir)? {
        let p = ent?.path();
        if p.is_dir() {
            out.extend(walk(&p)?);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rzstd-m4-{tag}-{}", nonce()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn c_run(oracle: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(oracle)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "zstd {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn c_decompress_dict(oracle: &Path, zst: &[u8], dict: &[u8]) -> Result<Vec<u8>, String> {
    let dir = scratch_dir("cd");
    let inn = dir.join("in.zst");
    let dpath = dir.join("d.dict");
    let out = dir.join("out.bin");
    fs::write(&inn, zst).map_err(|e| e.to_string())?;
    fs::write(&dpath, dict).map_err(|e| e.to_string())?;
    c_run(
        oracle,
        &[
            "-d",
            "-f",
            "-q",
            "-D",
            dpath.to_str().ok_or("dict path")?,
            "-o",
            out.to_str().ok_or("out path")?,
            inn.to_str().ok_or("in path")?,
        ],
    )?;
    let raw = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(raw)
}

fn c_compress_dict(oracle: &Path, src: &[u8], dict: &[u8]) -> Result<Vec<u8>, String> {
    let dir = scratch_dir("cc");
    let inn = dir.join("in.bin");
    let dpath = dir.join("d.dict");
    let out = dir.join("out.zst");
    fs::write(&inn, src).map_err(|e| e.to_string())?;
    fs::write(&dpath, dict).map_err(|e| e.to_string())?;
    c_run(
        oracle,
        &[
            "-q",
            "-f",
            "-3",
            "-D",
            dpath.to_str().ok_or("dict path")?,
            "-o",
            out.to_str().ok_or("out path")?,
            inn.to_str().ok_or("in path")?,
        ],
    )?;
    let zst = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(zst)
}

fn c_decompress_prefix(oracle: &Path, zst: &[u8], prefix: &[u8]) -> Result<Vec<u8>, String> {
    let dir = scratch_dir("pd");
    let inn = dir.join("in.zst");
    let pref = dir.join("ref.bin");
    let out = dir.join("out.bin");
    fs::write(&inn, zst).map_err(|e| e.to_string())?;
    fs::write(&pref, prefix).map_err(|e| e.to_string())?;
    let pf = format!("--patch-from={}", pref.to_str().ok_or("pref")?);
    c_run(
        oracle,
        &[
            "-d",
            "-f",
            "-q",
            &pf,
            "-o",
            out.to_str().ok_or("out")?,
            inn.to_str().ok_or("in")?,
        ],
    )?;
    let raw = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(raw)
}

fn c_compress_prefix(oracle: &Path, src: &[u8], prefix: &[u8]) -> Result<Vec<u8>, String> {
    let dir = scratch_dir("pc");
    let inn = dir.join("in.bin");
    let pref = dir.join("ref.bin");
    let out = dir.join("out.zst");
    fs::write(&inn, src).map_err(|e| e.to_string())?;
    fs::write(&pref, prefix).map_err(|e| e.to_string())?;
    let pf = format!("--patch-from={}", pref.to_str().ok_or("pref")?);
    c_run(
        oracle,
        &[
            "-q",
            "-f",
            "-3",
            &pf,
            "-o",
            out.to_str().ok_or("out")?,
            inn.to_str().ok_or("in")?,
        ],
    )?;
    let zst = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(zst)
}

fn c_train(oracle: &Path, samples: &[&[u8]], maxdict: usize) -> Result<Vec<u8>, String> {
    let dir = scratch_dir("tr");
    let mut paths = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        let p = dir.join(format!("s{i}.bin"));
        fs::write(&p, s).map_err(|e| e.to_string())?;
        paths.push(p);
    }
    let dout = dir.join("c.dict");
    let max = format!("--maxdict={maxdict}");
    let mut args: Vec<String> = vec![
        "--train".into(),
        "-q".into(),
        "-f".into(),
        max,
        "-o".into(),
        dout.to_string_lossy().into_owned(),
    ];
    for p in &paths {
        args.push(p.to_string_lossy().into_owned());
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    c_run(oracle, &arg_refs)?;
    let dict = fs::read(&dout).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    Ok(dict)
}

fn assert_bytes(got: &[u8], want: &[u8], tag: &str) {
    if got == want {
        return;
    }
    let pos = got
        .iter()
        .zip(want.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    panic!(
        "{tag} mismatch at {pos} got={} want={}",
        got.len(),
        want.len()
    );
}

fn sample_set() -> Vec<Vec<u8>> {
    let base = b"the quick brown fox jumps over the lazy dog. rusty_zstd M4 dictionary. ";
    (0..8)
        .map(|i| {
            let mut v = base.repeat(6);
            v.extend_from_slice(format!("#{i}\n").as_bytes());
            v
        })
        .collect()
}

#[test]
fn raw_dict_both_directions() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let dict_bytes = b"the quick brown fox jumps over the lazy dog. rusty_zstd raw dict prefix.";
    let dict = Dictionary::raw(*dict_bytes);
    let src = b"the quick brown fox jumps over the lazy dog. extra payload bytes here.";

    let zst = compress_using_dict(src, &dict, 3).expect("us compress raw dict");
    assert_bytes(
        &decompress_using_dict(&zst, &dict).expect("us d"),
        src,
        "us raw",
    );
    assert_bytes(
        &c_decompress_dict(&oracle, &zst, dict_bytes).expect("C -d -D raw"),
        src,
        "C d raw",
    );

    let czst = c_compress_dict(&oracle, src, dict_bytes).expect("C -D raw");
    assert_bytes(
        &decompress_using_dict(&czst, &dict).expect("us d C-raw"),
        src,
        "us d C raw",
    );
}

#[test]
fn trained_dict_us_train_c_uses() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let owned = sample_set();
    let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
    let dict_bytes = train(
        &refs,
        TrainOptions {
            max_dict: 2048,
            k: 64,
            d: 8,
            steps: 2,
            ..TrainOptions::fastcover()
        },
    )
    .expect("us train");
    let dict = Dictionary::from_bytes(&dict_bytes).expect("parse us dict");
    let src = owned[0].as_slice();

    let zst = compress_using_dict(src, &dict, 3).expect("us compress trained");
    assert_bytes(
        &decompress_using_dict(&zst, &dict).expect("us d trained"),
        src,
        "us trained",
    );
    assert_bytes(
        &c_decompress_dict(&oracle, &zst, &dict_bytes).expect("C -d our trained dict"),
        src,
        "C d our trained",
    );

    let czst = c_compress_dict(&oracle, src, &dict_bytes).expect("C compress our dict");
    assert_bytes(
        &decompress_using_dict(&czst, &dict).expect("us d C+our dict"),
        src,
        "us d C+our",
    );
    assert_bytes(
        &c_decompress_dict(&oracle, &czst, &dict_bytes).expect("C d C+our dict"),
        src,
        "C roundtrip our dict",
    );
}

#[test]
fn trained_dict_c_train_us_uses() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let owned = sample_set();
    let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
    let dict_bytes = match c_train(&oracle, &refs, 2048) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skip: C --train failed: {e}");
            return;
        }
    };
    let dict = Dictionary::from_bytes(&dict_bytes).expect("parse C dict");
    let src = owned[1].as_slice();

    let czst = c_compress_dict(&oracle, src, &dict_bytes).expect("C compress C dict");
    assert_bytes(
        &decompress_using_dict(&czst, &dict).expect("us d C-trained"),
        src,
        "us d C-trained",
    );

    let zst = compress_using_dict(src, &dict, 3).expect("us compress C dict");
    assert_bytes(
        &decompress_using_dict(&zst, &dict).expect("us d us+C dict"),
        src,
        "us+C dict",
    );
    assert_bytes(
        &c_decompress_dict(&oracle, &zst, &dict_bytes).expect("C d us+C dict"),
        src,
        "C d us+C dict",
    );
}

#[test]
fn patch_from_both_directions() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let prefix = b"the quick brown fox jumps over the lazy dog. rusty_zstd prefix.";
    let src = b"the quick brown fox jumps over the lazy dog. rusty_zstd prefix. NEW TAIL";

    let zst = compress_using_prefix(src, prefix, 3).expect("us prefix");
    assert_bytes(
        &decompress_using_prefix(&zst, prefix).expect("us d prefix"),
        src,
        "us prefix",
    );
    assert_bytes(
        &c_decompress_prefix(&oracle, &zst, prefix).expect("C --patch-from -d"),
        src,
        "C d prefix",
    );

    let czst = c_compress_prefix(&oracle, src, prefix).expect("C --patch-from");
    assert_bytes(
        &decompress_using_prefix(&czst, prefix).expect("us d C prefix"),
        src,
        "us d C prefix",
    );
}
