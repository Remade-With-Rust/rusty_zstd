//! `rzstd` -- M6: compress / decompress / test / train / list / bench, MT, aliases.
//!
//! Installed name is `rzstd` so it does not collide with the C oracle on PATH.
//! Aliases: `unzstd` (`-d`), `zstdcat` (`-dcf`), `zstdmt` (`-T0`).

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rusty_zstd::{
    compress_seekable_adv, compress_with_advanced, compression_params, decompress_using_dict_with,
    decompress_using_prefix_with, decompress_with, default_nb_workers, inspect_frames, train,
    AdvancedOptions, DecompressOptions, Dictionary, FrameKind, TrainOptions, DEFAULT_FRAME_SIZE,
    DEFAULT_LONG_WINDOW_LOG, DEFAULT_MAX_DICT, NB_WORKERS_MAX, VERSION,
};
use thoth::symbols::status;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Compress,
    Decompress,
    Test,
    Train,
    List,
    Bench,
}

#[derive(Clone)]
struct Args {
    mode: Mode,
    level: i32,
    stdout: bool,
    output: Option<PathBuf>,
    force: bool,
    quiet: bool,
    keep: bool,
    checksum: bool,
    zstd: Option<String>,
    show_cparams: bool,
    dict_path: Option<PathBuf>,
    patch_from: Option<PathBuf>,
    write_dict_id: bool,
    train: TrainOptions,
    long_window: Option<u32>,
    rsyncable: bool,
    target_cblock: u32,
    seekable: bool,
    max_frame_size: usize,
    threads: Option<u32>,
    single_thread: bool,
    job_size: usize,
    overlap_log: u32,
    recursive: bool,
    memory: Option<u64>,
    bench_secs: u32,
    bench_end: Option<i32>,
    files: Vec<PathBuf>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{fail} rzstd: {e}", fail = status::FAIL);
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = parse_args()?;
    if args.recursive {
        args.files = collect_paths(&args.files, true)?;
    } else {
        for f in &args.files {
            if f.is_dir() {
                return Err(format!("{} is a directory (use -r)", f.display()));
            }
        }
    }
    if args.mode == Mode::Train {
        return run_train(&args);
    }
    if args.mode == Mode::List {
        return run_list(&args);
    }
    if args.mode == Mode::Bench {
        return run_bench(&args);
    }
    if args.dict_path.is_some() && args.patch_from.is_some() {
        return Err("use either -D or --patch-from, not both".into());
    }
    if args.files.is_empty() {
        return run_stdio(&args);
    }
    if args.output.is_some() && args.files.len() > 1 {
        return Err("-o with multiple inputs is not supported".into());
    }
    for f in &args.files {
        run_file(&args, f)?;
    }
    Ok(())
}

fn load_dict(path: &Path) -> Result<Dictionary, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Dictionary::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn load_prefix(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn compress_src(src: &[u8], args: &Args) -> Result<Vec<u8>, String> {
    if args.seekable && (args.dict_path.is_some() || args.patch_from.is_some()) {
        return Err("--seekable with -D / --patch-from is not supported".into());
    }
    let mut params =
        compression_params(args.level, Some(src.len() as u64)).map_err(|e| e.to_string())?;
    if let Some(ref spec) = args.zstd {
        params
            .apply_zstd_option_string(spec)
            .map_err(|_| format!("invalid --zstd={spec}"))?;
    }
    if let Some(w) = args.long_window {
        params.window_log = w;
    }
    if args.show_cparams {
        eprintln!("--zstd={}", params.to_zstd_option_string());
    }
    let mut ldm = params.ldm_params();
    if args.long_window.is_some() || args.rsyncable {
        ldm.enable = true;
    }
    let mut workers = args.threads.unwrap_or(params.nb_workers);
    if args.single_thread {
        workers = 0;
    } else if workers == 0 && args.threads == Some(0) {
        workers = default_nb_workers();
    }
    let job_size = if args.job_size > 0 {
        args.job_size
    } else {
        params.job_size as usize
    };
    let overlap_log = if args.overlap_log > 0 {
        args.overlap_log
    } else {
        params.overlap_log
    };
    let adv = AdvancedOptions {
        ldm,
        rsyncable: args.rsyncable,
        target_cblock_size: args.target_cblock,
        nb_workers: workers,
        job_size,
        overlap_log,
        ..AdvancedOptions::default()
    };
    if args.seekable {
        return compress_seekable_adv(src, params, args.checksum, args.max_frame_size, adv)
            .map_err(|e| e.to_string());
    }
    if let Some(ref dp) = args.dict_path {
        let dict = load_dict(dp)?;
        compress_with_advanced(
            src,
            params,
            args.checksum,
            Some(&dict),
            &[],
            args.write_dict_id,
            adv,
        )
        .map_err(|e| e.to_string())
    } else if let Some(ref pp) = args.patch_from {
        let prefix = load_prefix(pp)?;
        compress_with_advanced(src, params, args.checksum, None, &prefix, false, adv)
            .map_err(|e| e.to_string())
    } else {
        compress_with_advanced(
            src,
            params,
            args.checksum,
            None,
            &[],
            args.write_dict_id,
            adv,
        )
        .map_err(|e| e.to_string())
    }
}

fn decomp_opts(args: &Args) -> DecompressOptions {
    let mut opts = DecompressOptions::default();
    if let Some(w) = args.long_window {
        opts.window_max = 1u64.checked_shl(w.min(63)).unwrap_or(u64::MAX);
    }
    if let Some(m) = args.memory {
        opts.window_max = m;
    }
    // `--no-check` on a DECOMPRESS is `ZSTD_d_forceIgnoreChecksum`: skip
    // verification of the stored xxh64. The 4 bytes are still consumed, so
    // concatenated frames keep parsing. On high-ratio content this is the
    // majority of decode time (61% on a 32 MiB zeros frame), so it is a real
    // lever -- at the cost of the frame's own corruption detection.
    if !args.checksum {
        opts.force_ignore_checksum = true;
    }
    opts
}

fn decompress_src(src: &[u8], args: &Args) -> Result<Vec<u8>, String> {
    let opts = decomp_opts(args);
    if let Some(ref dp) = args.dict_path {
        let dict = load_dict(dp)?;
        decompress_using_dict_with(src, &dict, opts).map_err(|e| e.to_string())
    } else if let Some(ref pp) = args.patch_from {
        let prefix = load_prefix(pp)?;
        decompress_using_prefix_with(src, &prefix, opts).map_err(|e| e.to_string())
    } else {
        decompress_with(src, opts).map_err(|e| e.to_string())
    }
}

fn run_stdio(args: &Args) -> Result<(), String> {
    if args.mode == Mode::Compress && io::stdout().is_terminal() && !args.force && !args.stdout {
        return Err("won't write compressed data to a terminal (use -c or -f)".into());
    }
    let mut src = Vec::new();
    io::stdin()
        .read_to_end(&mut src)
        .map_err(|e| format!("stdin: {e}"))?;
    match args.mode {
        Mode::Compress => {
            let zst = compress_src(&src, args)?;
            io::stdout()
                .write_all(&zst)
                .map_err(|e| format!("stdout: {e}"))?;
        }
        Mode::Decompress => {
            let raw = decompress_src(&src, args)?;
            io::stdout()
                .write_all(&raw)
                .map_err(|e| format!("stdout: {e}"))?;
        }
        Mode::Test => {
            let _ = decompress_src(&src, args)?;
            if !args.quiet {
                eprintln!("stdin: OK");
            }
        }
        Mode::Train => return Err("--train needs sample files".into()),
        Mode::List => {
            list_bytes(&src, "stdin")?;
        }
        Mode::Bench => {
            bench_bytes(&src, "stdin", args)?;
        }
    }
    Ok(())
}

fn run_file(args: &Args, path: &Path) -> Result<(), String> {
    let src = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match args.mode {
        Mode::Compress => {
            let zst = compress_src(&src, args)?;
            if args.stdout {
                io::stdout()
                    .write_all(&zst)
                    .map_err(|e| format!("stdout: {e}"))?;
                return Ok(());
            }
            let out = args
                .output
                .clone()
                .unwrap_or_else(|| PathBuf::from(format!("{}.zst", path.display())));
            write_out(&out, &zst, args.force)?;
            if !args.keep {
                fs::remove_file(path).map_err(|e| format!("rm {}: {e}", path.display()))?;
            }
        }
        Mode::Decompress => {
            let raw = decompress_src(&src, args)?;
            if args.stdout {
                io::stdout()
                    .write_all(&raw)
                    .map_err(|e| format!("stdout: {e}"))?;
                return Ok(());
            }
            let out = match &args.output {
                Some(p) => p.clone(),
                None => strip_zst(path)?,
            };
            write_out(&out, &raw, args.force)?;
            if !args.keep {
                fs::remove_file(path).map_err(|e| format!("rm {}: {e}", path.display()))?;
            }
        }
        Mode::Test => {
            let _ = decompress_src(&src, args).map_err(|e| format!("{}: {e}", path.display()))?;
            if !args.quiet {
                eprintln!("{}: OK", path.display());
            }
        }
        Mode::Train => return Err("--train is handled in run()".into()),
        Mode::List | Mode::Bench => return Err("list/bench is handled in run()".into()),
    }
    Ok(())
}

fn run_train(args: &Args) -> Result<(), String> {
    if args.files.is_empty() {
        return Err("--train requires sample files".into());
    }
    let mut owned = Vec::new();
    for f in &args.files {
        owned.push(fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    let refs: Vec<&[u8]> = owned.iter().map(|s| s.as_slice()).collect();
    let dict = train(&refs, args.train).map_err(|e| e.to_string())?;
    if args.stdout {
        io::stdout()
            .write_all(&dict)
            .map_err(|e| format!("stdout: {e}"))?;
        return Ok(());
    }
    let out = args
        .output
        .clone()
        .ok_or_else(|| " --train requires -o FILE (or -c)".to_string())?;
    write_out(&out, &dict, args.force)?;
    Ok(())
}

fn write_out(path: &Path, data: &[u8], force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!(
            "{} already exists (use -f to overwrite)",
            path.display()
        ));
    }
    let mut f = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.write_all(data)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

fn strip_zst(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{}: not a utf-8 name", path.display()))?;
    if let Some(stem) = name.strip_suffix(".zst") {
        if stem.is_empty() {
            return Err(format!(
                "{}: empty stem after stripping .zst",
                path.display()
            ));
        }
        Ok(path.with_file_name(stem))
    } else {
        Err(format!(
            "{}: cannot guess output name (not *.zst); use -o",
            path.display()
        ))
    }
}

fn apply_train_kv(opts: &mut TrainOptions, spec: &str) -> Result<(), String> {
    if spec.is_empty() {
        return Ok(());
    }
    for part in spec.split(',') {
        let (k, v) = part
            .split_once('=')
            .ok_or_else(|| format!("invalid trainer option {part}"))?;
        match k {
            "k" => opts.k = v.parse().map_err(|_| format!("invalid k={v}"))?,
            "d" => opts.d = v.parse().map_err(|_| format!("invalid d={v}"))?,
            "steps" => opts.steps = v.parse().map_err(|_| format!("invalid steps={v}"))?,
            "f" => opts.f = v.parse().map_err(|_| format!("invalid f={v}"))?,
            "accel" => opts.accel = v.parse().map_err(|_| format!("invalid accel={v}"))?,
            "split" => opts.split = v.parse().map_err(|_| format!("invalid split={v}"))?,
            "s" | "selectivity" => {
                opts.selectivity = v.parse().map_err(|_| format!("invalid selectivity={v}"))?;
            }
            "shrink" => {}
            _ => return Err(format!("unknown trainer option {k}")),
        }
    }
    Ok(())
}

fn parse_args() -> Result<Args, String> {
    let mut mode = Mode::Compress;
    let mut level = env_i32("ZSTD_CLEVEL").unwrap_or(rusty_zstd::DEFAULT_CLEVEL);
    let mut ultra = false;
    let mut stdout = false;
    let mut output = None;
    let mut force = false;
    let mut quiet = false;
    let mut keep = true;
    let mut checksum = true;
    let mut zstd = None;
    let mut show_cparams = false;
    let mut dict_path = None;
    let mut patch_from = None;
    let mut write_dict_id = true;
    let mut train_opts = TrainOptions::fastcover();
    let mut long_window = None;
    let mut rsyncable = false;
    let mut target_cblock = 0u32;
    let mut seekable = false;
    let mut max_frame_size = DEFAULT_FRAME_SIZE;
    let mut threads = env_u32("ZSTD_NBTHREADS");
    let mut single_thread = false;
    let mut job_size = 0usize;
    let mut overlap_log = 0u32;
    let mut recursive = false;
    let mut memory = None;
    let mut bench_secs = 1u32;
    let mut bench_end = None;
    let mut files = Vec::new();
    let mut argv = std::env::args().skip(1).peekable();

    match bin_stem().as_str() {
        "unzstd" => mode = Mode::Decompress,
        "zstdcat" => {
            mode = Mode::Decompress;
            stdout = true;
            force = true;
        }
        "zstdmt" => threads = Some(0),
        _ => {}
    }

    if argv.peek().is_none() {
        print_help();
        std::process::exit(0);
    }

    while let Some(a) = argv.next() {
        match a.as_str() {
            "-h" | "-H" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("rzstd {VERSION}");
                println!("rusty_zstd {VERSION} (M6 compress/decompress/train/long/seekable/mt)");
                std::process::exit(0);
            }
            "-z" | "--compress" => mode = Mode::Compress,
            "-d" | "--decompress" | "--uncompress" => mode = Mode::Decompress,
            "-t" | "--test" => mode = Mode::Test,
            "-l" | "--list" => mode = Mode::List,
            "-r" | "--recursive" => recursive = true,
            "--single-thread" => {
                single_thread = true;
                threads = None;
            }
            "--max" => {
                ultra = true;
                level = 22;
            }
            "-c" | "--stdout" => stdout = true,
            "-f" | "--force" => force = true,
            "-q" | "--quiet" => quiet = true,
            "-k" | "--keep" => keep = true,
            "--rm" => keep = false,
            "-C" | "--check" => checksum = true,
            "--no-check" => checksum = false,
            "--ultra" => ultra = true,
            "--show-default-cparams" => show_cparams = true,
            "--no-dictID" | "--no-dictid" => write_dict_id = false,
            "--rsyncable" => rsyncable = true,
            "--seekable" => seekable = true,
            "--long" => long_window = Some(DEFAULT_LONG_WINDOW_LOG),
            "--threads" => {
                let n = argv.next().ok_or("--threads requires a count")?;
                threads = Some(parse_threads(&n)?);
            }
            "--jobsize" => {
                let n = argv.next().ok_or("--jobsize requires a size")?;
                job_size = parse_size(&n)? as usize;
            }
            "--overlap-log" => {
                let n = argv.next().ok_or("--overlap-log requires 0..=9")?;
                overlap_log = parse_overlap(&n)?;
            }
            "--memory" | "-M" => {
                let n = argv.next().ok_or("-M / --memory requires a size")?;
                memory = Some(parse_size(&n)?);
            }
            "--target-compressed-block-size" => {
                let n = argv
                    .next()
                    .ok_or("--target-compressed-block-size requires a size")?;
                target_cblock = n
                    .parse()
                    .map_err(|_| "invalid --target-compressed-block-size")?;
            }
            "--max-frame-size" => {
                let n = argv.next().ok_or("--max-frame-size requires a size")?;
                max_frame_size = parse_frame_size(&n)?;
            }
            "-o" => {
                let p = argv.next().ok_or("-o requires a path")?;
                output = Some(PathBuf::from(p));
            }
            "-D" => {
                let p = argv.next().ok_or("-D requires a dictionary path")?;
                dict_path = Some(PathBuf::from(p));
            }
            "--patch-from" => {
                let p = argv.next().ok_or("--patch-from requires a path")?;
                patch_from = Some(PathBuf::from(p));
            }
            "--train" => {
                mode = Mode::Train;
                train_opts = TrainOptions::fastcover();
            }
            "--train-cover" => {
                mode = Mode::Train;
                train_opts = TrainOptions::cover();
            }
            "--train-fastcover" => {
                mode = Mode::Train;
                train_opts = TrainOptions::fastcover();
            }
            "--train-legacy" => {
                mode = Mode::Train;
                train_opts = TrainOptions::legacy();
            }
            "--maxdict" => {
                let n = argv.next().ok_or("--maxdict requires a size")?;
                train_opts.max_dict = n.parse().map_err(|_| "invalid --maxdict")?;
            }
            "--dictID" | "--dictid" => {
                let n = argv.next().ok_or("--dictID requires an integer")?;
                train_opts.dict_id = Some(n.parse().map_err(|_| "invalid --dictID")?);
            }
            "--fast" => level = -1,
            s if s.starts_with("--fast=") => {
                let n: i32 = s[7..].parse().map_err(|_| "invalid --fast=")?;
                if n < 1 {
                    return Err("--fast requires a positive integer".into());
                }
                level = -n;
            }
            s if s.starts_with("--zstd=") => {
                zstd = Some(s[7..].to_string());
            }
            s if s.starts_with("--long=") => {
                long_window = Some(parse_window_log(&s[7..])?);
            }
            s if s.starts_with("--target-compressed-block-size=") => {
                target_cblock = s[31..]
                    .parse()
                    .map_err(|_| "invalid --target-compressed-block-size=")?;
            }
            s if s.starts_with("--max-frame-size=") => {
                max_frame_size = parse_frame_size(&s[17..])?;
            }
            s if s.starts_with("--threads=") => {
                threads = Some(parse_threads(&s[10..])?);
            }
            s if s.starts_with("--jobsize=") => {
                job_size = parse_size(&s[10..])? as usize;
            }
            s if s.starts_with("--overlap-log=") => {
                overlap_log = parse_overlap(&s[14..])?;
            }
            s if s.starts_with("--memory=") => {
                memory = Some(parse_size(&s[9..])?);
            }
            s if s.starts_with("-M") && s.len() > 2 => {
                memory = Some(parse_size(&s[2..])?);
            }
            s if s.starts_with("-T") && s.len() > 2 => {
                threads = Some(parse_threads(&s[2..])?);
            }
            "-T" => {
                let n = argv.next().ok_or("-T requires a thread count")?;
                threads = Some(parse_threads(&n)?);
            }
            s if s.starts_with("-B") && s.len() > 2 => {
                job_size = parse_size(&s[2..])? as usize;
            }
            "-B" => {
                let n = argv.next().ok_or("-B requires a job size")?;
                job_size = parse_size(&n)? as usize;
            }
            "-b" | "--bench" => mode = Mode::Bench,
            s if s.starts_with("-b") && s.len() > 2 && s.as_bytes()[2].is_ascii_digit() => {
                mode = Mode::Bench;
                level = s[2..].parse().map_err(|_| "invalid -b#")?;
            }
            s if s.starts_with("-e") && s.len() > 2 => {
                bench_end = Some(s[2..].parse().map_err(|_| "invalid -e#")?);
            }
            "-e" => {
                let n = argv.next().ok_or("-e requires a level")?;
                bench_end = Some(n.parse().map_err(|_| "invalid -e")?);
            }
            s if s.starts_with("-i") && s.len() > 2 => {
                bench_secs = s[2..].parse().map_err(|_| "invalid -i#")?;
            }
            "-i" => {
                let n = argv.next().ok_or("-i requires seconds")?;
                bench_secs = n.parse().map_err(|_| "invalid -i")?;
            }
            s if s.starts_with("--format=") => {
                if s[9..] != *"zstd" {
                    return Err(format!(
                        "--format={} is not in this build (only zstd)",
                        &s[9..]
                    ));
                }
            }
            "--zstd" => {
                let p = argv.next().ok_or("--zstd requires key=value,...")?;
                zstd = Some(p);
            }
            s if s.starts_with("-D") && s.len() > 2 => {
                dict_path = Some(PathBuf::from(&s[2..]));
            }
            s if s.starts_with("--patch-from=") => {
                patch_from = Some(PathBuf::from(&s[13..]));
            }
            s if s.starts_with("--train-cover=") => {
                mode = Mode::Train;
                train_opts = TrainOptions::cover();
                apply_train_kv(&mut train_opts, &s[14..])?;
            }
            s if s.starts_with("--train-fastcover=") => {
                mode = Mode::Train;
                train_opts = TrainOptions::fastcover();
                apply_train_kv(&mut train_opts, &s[19..])?;
            }
            s if s.starts_with("--train-legacy=") => {
                mode = Mode::Train;
                train_opts = TrainOptions::legacy();
                apply_train_kv(&mut train_opts, &s[15..])?;
            }
            s if s.starts_with("--maxdict=") => {
                train_opts.max_dict = s[10..].parse().map_err(|_| "invalid --maxdict=")?;
            }
            s if s.starts_with("--dictID=") || s.starts_with("--dictid=") => {
                let v = s.split_once('=').map(|(_, v)| v).unwrap_or("");
                train_opts.dict_id = Some(v.parse().map_err(|_| "invalid --dictID=")?);
            }
            s if s.starts_with('-') && s.len() > 1 && s.as_bytes()[1].is_ascii_digit() => {
                let n: i32 = s[1..].parse().map_err(|_| format!("invalid level {s}"))?;
                if n > 22 {
                    return Err("level out of range (max 22; use --ultra for 20-22)".into());
                }
                level = n;
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown option {s}"));
            }
            _ => files.push(PathBuf::from(a)),
        }
    }

    if !(rusty_zstd::MIN_CLEVEL..=rusty_zstd::MAX_CLEVEL).contains(&level) {
        return Err("compression level out of range (-7..=22)".into());
    }
    if level >= 20 && !ultra {
        return Err("levels 20-22 require --ultra".into());
    }
    Ok(Args {
        mode,
        level,
        stdout,
        output,
        force,
        quiet,
        keep,
        checksum,
        zstd,
        show_cparams,
        dict_path,
        patch_from,
        write_dict_id,
        train: train_opts,
        long_window,
        rsyncable,
        target_cblock,
        seekable,
        max_frame_size,
        threads,
        single_thread,
        job_size,
        overlap_log,
        recursive,
        memory,
        bench_secs,
        bench_end,
        files,
    })
}

fn print_help() {
    println!("rzstd {VERSION} -- pure-Rust zstd CLI (M6)");
    println!();
    println!("Usage:");
    println!("  rzstd [options] [-o file] [file ...]");
    println!("  rzstd -d [options] [-o file] [file.zst ...]");
    println!("  rzstd -t file.zst");
    println!("  rzstd -l file.zst");
    println!("  rzstd -b# file");
    println!("  rzstd --train [-o dict] [--maxdict=#] sample ...");
    println!();
    println!("  -z, --compress              compress (default)");
    println!("  -d, --decompress            decompress");
    println!("  -t, --test                  test integrity");
    println!("  -l, --list                  list frame info");
    println!(
        "  -b#                         in-process bench (same timer as rzstd-bench --m7-speed)"
    );
    println!("  -c, --stdout                write to stdout");
    println!("  -o FILE                     write to FILE");
    println!("  -#                          compression level 1-19 (default 3)");
    println!("  --ultra                     allow levels 20-22");
    println!("  --max                       level 22 (implies --ultra)");
    println!("  --fast[=#]                  negative level (default 1 => -1)");
    println!(
        "  --zstd=k=v,...              windowLog,... strategy, enableLdm, nbWorkers, jobSize, overlapLog"
    );
    println!("  --show-default-cparams      print --zstd=... for this input size");
    println!("  --long[=#]                  LDM; windowLog (default {DEFAULT_LONG_WINDOW_LOG})");
    println!("  --rsyncable                 rolling-hash block cuts (enables LDM)");
    println!("  --target-compressed-block-size=#  cap uncompressed block from target csize");
    println!("  --seekable                  independent frames + skippable seek table");
    println!("  --max-frame-size=#          seekable frame size (default {DEFAULT_FRAME_SIZE})");
    println!("  -T#, --threads=#            MT workers (0 = CPU count, cap {NB_WORKERS_MAX})");
    println!("  --single-thread             one-shot path (not -T1)");
    println!("  --jobsize=# / -B#           MT job size (min 512 KiB)");
    println!("  --overlap-log=#             0=default 1=none 9=full window");
    println!("  -M#, --memory=#             decoder window cap");
    println!("  -r, --recursive             operate on directories");
    println!("  -D FILE                     dictionary (raw or trained)");
    println!("  --patch-from=FILE           prefix matching (no Dictionary_ID)");
    println!("  --no-dictID                 omit Dictionary_ID in the frame");
    println!("  --train                     train fastcover (d=8,steps=4)");
    println!("  --train-cover[=k=..]        COVER trainer");
    println!("  --train-fastcover[=k=..]    fastcover trainer");
    println!("  --train-legacy[=s=#]        legacy trainer");
    println!("  --maxdict=#                 max dictionary size (default {DEFAULT_MAX_DICT})");
    println!("  --dictID=#                  force Dictionary_ID");
    println!("  -f, --force                 overwrite");
    println!("  --rm                        remove source after success");
    println!("  -k, --keep                  keep source (default)");
    println!("  -q, --quiet                 no OK lines on -t");
    println!("  --format=zstd               only format in this build");
    println!("  -h, -H, --help              this help");
    println!("  -V, --version               version");
    println!();
    println!("Env: ZSTD_CLEVEL, ZSTD_NBTHREADS. Aliases: unzstd, zstdcat, zstdmt.");
    println!("See docs/plans/rusty-zstd-mission.md");
}

fn parse_window_log(s: &str) -> Result<u32, String> {
    let n: u32 = s.parse().map_err(|_| "invalid --long=")?;
    if !(10..=31).contains(&n) {
        return Err("--long windowLog must be 10..=31".into());
    }
    Ok(n)
}

fn parse_frame_size(s: &str) -> Result<usize, String> {
    let n: usize = s.parse().map_err(|_| "invalid --max-frame-size")?;
    if n == 0 {
        return Err("--max-frame-size must be > 0".into());
    }
    Ok(n)
}

fn bin_stem() -> String {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| {
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned().to_ascii_lowercase())
        })
        .unwrap_or_default()
}

fn env_i32(name: &str) -> Option<i32> {
    match std::env::var(name) {
        Ok(s) if s.is_empty() => None,
        Ok(s) => match s.parse() {
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!("rzstd: ignoring invalid {name}");
                None
            }
        },
        Err(_) => None,
    }
}

fn env_u32(name: &str) -> Option<u32> {
    match std::env::var(name) {
        Ok(s) if s.is_empty() => None,
        Ok(s) => match s.parse() {
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!("rzstd: ignoring invalid {name}");
                None
            }
        },
        Err(_) => None,
    }
}

fn parse_threads(s: &str) -> Result<u32, String> {
    let n: u32 = s.parse().map_err(|_| "invalid thread count")?;
    if n > NB_WORKERS_MAX {
        return Ok(NB_WORKERS_MAX);
    }
    Ok(n)
}

fn parse_overlap(s: &str) -> Result<u32, String> {
    let n: u32 = s.parse().map_err(|_| "invalid --overlap-log")?;
    if n > 9 {
        return Err("--overlap-log must be 0..=9".into());
    }
    Ok(n)
}

fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    let (num, mul) = if let Some(n) = lower.strip_suffix("kib") {
        (n, 1024u64)
    } else if let Some(n) = lower.strip_suffix("kb") {
        (n, 1000)
    } else if let Some(n) = lower.strip_suffix("mib") {
        (n, 1024 * 1024)
    } else if let Some(n) = lower.strip_suffix("mb") {
        (n, 1000 * 1000)
    } else if let Some(n) = lower.strip_suffix("gib") {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = lower.strip_suffix("gb") {
        (n, 1000 * 1000 * 1000)
    } else if let Some(n) = lower.strip_suffix('k') {
        (n, 1024)
    } else if let Some(n) = lower.strip_suffix('m') {
        (n, 1024 * 1024)
    } else if let Some(n) = lower.strip_suffix('g') {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1)
    } else {
        (lower.as_str(), 1)
    };
    let v: u64 = num
        .trim()
        .parse()
        .map_err(|_| format!("invalid size {s}"))?;
    v.checked_mul(mul)
        .ok_or_else(|| format!("size overflow {s}"))
}

fn collect_paths(inputs: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for p in inputs {
        if p.is_dir() {
            if !recursive {
                return Err(format!("{} is a directory (use -r)", p.display()));
            }
            walk_files(p, &mut out)?;
        } else {
            out.push(p.clone());
        }
    }
    Ok(out)
}

fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for ent in rd {
        let ent = ent.map_err(|e| format!("{}: {e}", dir.display()))?;
        let p = ent.path();
        let ft = ent
            .file_type()
            .map_err(|e| format!("{}: {e}", p.display()))?;
        if ft.is_dir() {
            walk_files(&p, out)?;
        } else if ft.is_file() {
            out.push(p);
        }
    }
    Ok(())
}

fn run_list(args: &Args) -> Result<(), String> {
    if args.files.is_empty() {
        let mut src = Vec::new();
        io::stdin()
            .read_to_end(&mut src)
            .map_err(|e| format!("stdin: {e}"))?;
        return list_bytes(&src, "stdin");
    }
    if !args.quiet {
        println!(
            "{:>6} {:>6} {:>12} {:>14} {:>6}  Filename",
            "Frames", "Skips", "Compressed", "Uncompressed", "Check"
        );
    }
    for f in &args.files {
        let src = fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        list_bytes(&src, &f.display().to_string())?;
    }
    Ok(())
}

fn list_bytes(src: &[u8], name: &str) -> Result<(), String> {
    let frames = inspect_frames(src).map_err(|e| format!("{name}: {e}"))?;
    let mut n_z = 0u32;
    let mut n_s = 0u32;
    let mut csize = 0u64;
    let mut usize_sum = 0u64;
    let mut usize_known = true;
    let mut check = "----";
    for fr in &frames {
        csize += fr.compressed_size as u64;
        match fr.kind {
            FrameKind::Skippable { .. } => n_s += 1,
            FrameKind::Zstd(h) => {
                n_z += 1;
                match h.content_size {
                    Some(n) => usize_sum += n,
                    None => usize_known = false,
                }
                if h.checksum {
                    check = "XXH64";
                }
            }
        }
    }
    let u_disp = if usize_known {
        usize_sum.to_string()
    } else {
        "-".into()
    };
    println!("{n_z:>6} {n_s:>6} {csize:>12} {u_disp:>14} {check:>6}  {name}");
    Ok(())
}

fn run_bench(args: &Args) -> Result<(), String> {
    let end = args.bench_end.unwrap_or(args.level).max(args.level);
    if args.files.is_empty() {
        let mut src = Vec::new();
        io::stdin()
            .read_to_end(&mut src)
            .map_err(|e| format!("stdin: {e}"))?;
        for lv in args.level..=end {
            let mut a = args.clone();
            a.level = lv;
            bench_bytes(&src, "stdin", &a)?;
        }
        return Ok(());
    }
    for f in &args.files {
        let src = fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?;
        for lv in args.level..=end {
            let mut a = args.clone();
            a.level = lv;
            bench_bytes(&src, &f.display().to_string(), &a)?;
        }
    }
    Ok(())
}

fn bench_bytes(src: &[u8], name: &str, args: &Args) -> Result<(), String> {
    let min = std::time::Duration::from_secs(u64::from(args.bench_secs));
    let mut zst_len = 0usize;
    let t = rusty_zstd::time_loops(min, || {
        let zst = compress_src(src, args)?;
        zst_len = zst.len();
        let raw = decompress_src(&zst, args)?;
        if raw != src {
            return Err(format!("{name}: bench mismatch"));
        }
        Ok(())
    })?;
    println!(
        "{:>2}#rzstd : {:>8} -> {:>8} bytes, {} loops, {:.0} ms  {name}",
        args.level,
        src.len(),
        zst_len,
        t.loops,
        t.wall_ms
    );
    Ok(())
}
