//! Deterministic generated corpora. Silesia is fetched separately (see corpora/).

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::oracle::hex_sha256;

pub const SPLIT_TRAIN: &str = "train";
pub const SPLIT_HOLDOUT: &str = "holdout";

const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub id: String,
    pub split: &'static str,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

pub fn ensure_generated(dir: &Path, smoke: bool) -> Result<Vec<GeneratedFile>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let specs: Vec<GenSpec> = if smoke {
        vec![GenSpec {
            id: "zeros-1m",
            split: SPLIT_TRAIN,
            bytes: MIB,
            kind: Kind::Zeros,
        }]
    } else {
        vec![
            GenSpec {
                id: "zeros-32m",
                split: SPLIT_TRAIN,
                bytes: 32 * MIB,
                kind: Kind::Zeros,
            },
            GenSpec {
                id: "text-32m",
                split: SPLIT_TRAIN,
                bytes: 32 * MIB,
                kind: Kind::Text,
            },
            GenSpec {
                id: "incomp-32m",
                split: SPLIT_HOLDOUT,
                bytes: 32 * MIB,
                kind: Kind::Xorshift {
                    seed: 0xA5A5_5A5A_u64,
                },
            },
            // Mission 6.1 classes that Silesia does not cover.
            GenSpec {
                id: "jsonlog-16m",
                split: SPLIT_HOLDOUT,
                bytes: 16 * MIB,
                kind: Kind::JsonLogs {
                    seed: 0x5EED_10_65_u64,
                },
            },
            GenSpec {
                id: "smallmsg-8m",
                split: SPLIT_TRAIN,
                bytes: 8 * MIB,
                kind: Kind::SmallMsgs {
                    seed: 0x5A11_3E55_u64,
                },
            },
            GenSpec {
                id: "versions-16m",
                split: SPLIT_TRAIN,
                bytes: 16 * MIB,
                kind: Kind::Versions {
                    seed: 0x7E45_1043_u64,
                },
            },
        ]
    };
    let mut out = Vec::new();
    for spec in specs {
        out.push(materialize(dir, spec)?);
    }
    Ok(out)
}

/// Optional Silesia files (holdout first, then train). Empty if not fetched.
pub fn list_silesia(root: &Path) -> Vec<GeneratedFile> {
    let dir = root.join("corpora").join("data").join("silesia");
    if !dir.is_dir() {
        return Vec::new();
    }
    const HOLDOUT: &[&str] = &["mr", "ooffice", "osdb", "reymont", "sao", "webster"];
    const TRAIN: &[&str] = &["dickens", "mozilla", "nci", "samba", "xml", "x-ray"];
    let mut out = Vec::new();
    for (split, names) in [(SPLIT_HOLDOUT, HOLDOUT), (SPLIT_TRAIN, TRAIN)] {
        for id in names {
            let path = dir.join(id);
            if !path.is_file() {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if bytes.is_empty() {
                continue;
            }
            out.push(GeneratedFile {
                id: (*id).to_string(),
                split,
                path,
                bytes: bytes.len() as u64,
                sha256: hex_sha256(&bytes),
            });
        }
    }
    out
}

struct GenSpec {
    id: &'static str,
    split: &'static str,
    bytes: u64,
    kind: Kind,
}

enum Kind {
    Zeros,
    Text,
    Xorshift {
        seed: u64,
    },
    /// SpaceDB-shaped JSON log records: repetitive keys, varying values.
    /// Product corpus -- this is what actually ships through the codec.
    JsonLogs {
        seed: u64,
    },
    /// Many independent 1-16 KiB messages back to back. Per-frame and
    /// table-setup overhead dominate here; the 128 KiB-block corpora hide it.
    SmallMsgs {
        seed: u64,
    },
    /// tar-of-versions: one base blob repeated with small mutations, so the
    /// redundancy sits far outside a 128 KiB block. The `--long` / LDM class.
    Versions {
        seed: u64,
    },
}

/// Deterministic xorshift64* so every corpus is reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
}

fn materialize(dir: &Path, spec: GenSpec) -> Result<GeneratedFile, String> {
    let path = dir.join(spec.id);
    if !(path.is_file()
        && path
            .metadata()
            .map(|m| m.len() == spec.bytes)
            .unwrap_or(false))
    {
        write_kind(&path, spec.bytes, &spec.kind)?;
    }
    let bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() as u64 != spec.bytes {
        return Err(format!(
            "{}: expected {} bytes, got {}",
            spec.id,
            spec.bytes,
            bytes.len()
        ));
    }
    Ok(GeneratedFile {
        id: spec.id.to_string(),
        split: spec.split,
        path,
        bytes: spec.bytes,
        sha256: hex_sha256(&bytes),
    })
}

fn write_kind(path: &Path, n: u64, kind: &Kind) -> Result<(), String> {
    let f = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut w = BufWriter::new(f);
    match kind {
        Kind::Zeros => {
            let buf = [0u8; 64 * 1024];
            let mut left = n;
            while left > 0 {
                let chunk = left.min(buf.len() as u64) as usize;
                w.write_all(&buf[..chunk]).map_err(|e| e.to_string())?;
                left -= chunk as u64;
            }
        }
        Kind::Text => {
            let para = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
            let mut left = n;
            while left > 0 {
                let chunk = left.min(para.len() as u64) as usize;
                w.write_all(&para[..chunk]).map_err(|e| e.to_string())?;
                left -= chunk as u64;
            }
        }
        Kind::Xorshift { seed } => {
            let mut state = *seed | 1;
            let mut buf = [0u8; 64 * 1024];
            let mut left = n;
            while left > 0 {
                let chunk = left.min(buf.len() as u64) as usize;
                for slot in buf[..chunk].chunks_mut(8) {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    let b = state.to_le_bytes();
                    let ncopy = slot.len();
                    slot.copy_from_slice(&b[..ncopy]);
                }
                w.write_all(&buf[..chunk]).map_err(|e| e.to_string())?;
                left -= chunk as u64;
            }
        }
        Kind::JsonLogs { seed } => {
            let mut r = Rng::new(*seed);
            const LEVELS: [&str; 4] = ["info", "warn", "error", "debug"];
            const OPS: [&str; 6] = [
                "sync.push",
                "sync.pull",
                "store.put",
                "store.get",
                "peer.dial",
                "peer.drop",
            ];
            let mut left = n;
            let mut line = String::new();
            while left > 0 {
                line.clear();
                let ts = 1_700_000_000u64 + r.below(50_000_000);
                let lvl = LEVELS[(r.below(LEVELS.len() as u64)) as usize];
                let op = OPS[(r.below(OPS.len() as u64)) as usize];
                let peer = r.next();
                let ms = r.below(4000);
                let bytes = r.below(1 << 20);
                line.push_str(&format!(
                    "{{\"ts\":{ts},\"level\":\"{lvl}\",\"op\":\"{op}\",\
\"peer\":\"{peer:016x}\",\"dur_ms\":{ms},\"bytes\":{bytes},\"ok\":{}}}\n",
                    if r.below(16) == 0 { "false" } else { "true" }
                ));
                let b = line.as_bytes();
                let take = (left as usize).min(b.len());
                w.write_all(&b[..take]).map_err(|e| e.to_string())?;
                left -= take as u64;
            }
        }
        Kind::SmallMsgs { seed } => {
            let mut r = Rng::new(*seed);
            // A shared vocabulary across messages -- the redundancy a
            // dictionary would capture, which a per-message frame cannot.
            const VOCAB: [&str; 12] = [
                "identity",
                "capability",
                "signature",
                "timestamp",
                "payload",
                "checksum",
                "revision",
                "ancestor",
                "namespace",
                "attribute",
                "reference",
                "delegate",
            ];
            let mut left = n;
            let mut msg = String::new();
            while left > 0 {
                msg.clear();
                let fields = 8 + r.below(48);
                for _ in 0..fields {
                    let k = VOCAB[(r.below(VOCAB.len() as u64)) as usize];
                    msg.push_str(&format!("{k}={:x};", r.next() & 0xFFFF_FFFF));
                }
                msg.push('\n');
                let b = msg.as_bytes();
                let take = (left as usize).min(b.len());
                w.write_all(&b[..take]).map_err(|e| e.to_string())?;
                left -= take as u64;
            }
        }
        Kind::Versions { seed } => {
            // One base blob, then near-copies with sparse mutations. The
            // matching redundancy is megabytes away, outside any 128 KiB block.
            let mut r = Rng::new(*seed);
            const BASE: usize = 512 * 1024;
            let mut base = vec![0u8; BASE];
            for (i, b) in base.iter_mut().enumerate() {
                // Structured, compressible-but-not-trivial base content.
                *b = ((i * 31 + (i >> 7) * 17) % 251) as u8;
            }
            let mut left = n;
            let mut ver = base.clone();
            while left > 0 {
                let take = (left as usize).min(ver.len());
                w.write_all(&ver[..take]).map_err(|e| e.to_string())?;
                left -= take as u64;
                // ~0.1% of bytes change between consecutive versions.
                let muts = ver.len() / 1000;
                for _ in 0..muts {
                    let at = r.below(ver.len() as u64) as usize;
                    ver[at] = (r.next() & 0xFF) as u8;
                }
            }
        }
    }
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}
