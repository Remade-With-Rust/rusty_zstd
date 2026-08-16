//! Compression level -> strategy / window / hash table (facebook/zstd v1.5.7 `clevels.h`).

use crate::error::Error;
use crate::{MAX_CLEVEL, MIN_CLEVEL};

/// Match finder (libzstd `ZSTD_strategy`, values 1..=9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Strategy {
    /// Hash table + skip (`ZSTD_fast`).
    Fast = 1,
    /// Double hash (`ZSTD_dfast`).
    DFast = 2,
    /// Hash chain, greedy (`ZSTD_greedy`).
    Greedy = 3,
    /// Hash chain, lazy.
    Lazy = 4,
    /// Hash chain, lazy2.
    Lazy2 = 5,
    /// Binary tree, lazy2.
    BtLazy2 = 6,
    /// Binary tree, optimal parse.
    BtOpt = 7,
    /// Binary tree, stronger optimal.
    BtUltra = 8,
    /// Binary tree, ultra optimal (levels 20-22).
    BtUltra2 = 9,
}

impl Strategy {
    /// Parse libzstd strategy id 1..=9.
    pub fn from_id(id: i32) -> Option<Self> {
        Some(match id {
            1 => Strategy::Fast,
            2 => Strategy::DFast,
            3 => Strategy::Greedy,
            4 => Strategy::Lazy,
            5 => Strategy::Lazy2,
            6 => Strategy::BtLazy2,
            7 => Strategy::BtOpt,
            8 => Strategy::BtUltra,
            9 => Strategy::BtUltra2,
            _ => return None,
        })
    }

    /// libzstd numeric id.
    pub fn id(self) -> u8 {
        self as u8
    }

    /// ASCII name (`fast` .. `btultra2`).
    pub fn name(self) -> &'static str {
        match self {
            Strategy::Fast => "fast",
            Strategy::DFast => "dfast",
            Strategy::Greedy => "greedy",
            Strategy::Lazy => "lazy",
            Strategy::Lazy2 => "lazy2",
            Strategy::BtLazy2 => "btlazy2",
            Strategy::BtOpt => "btopt",
            Strategy::BtUltra => "btultra",
            Strategy::BtUltra2 => "btultra2",
        }
    }
}

/// libzstd `ZSTD_compressionParameters` for a level and size hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionParameters {
    /// `windowLog` (power of two window).
    pub window_log: u32,
    /// `chainLog`.
    pub chain_log: u32,
    /// `hashLog`.
    pub hash_log: u32,
    /// `searchLog` (chain attempts = 2^searchLog).
    pub search_log: u32,
    /// `minMatch` (3..=7).
    pub min_match: u32,
    /// `targetLength` (fast skip / parser target).
    pub target_length: u32,
    /// Strategy C would pick.
    pub strategy: Strategy,
    /// `--zstd=enableLdm` (also set by CLI `--long` / `--rsyncable`).
    pub enable_ldm: bool,
    /// `--zstd=ldmHashLog`. `0` = default from windowLog.
    pub ldm_hash_log: u32,
    /// `--zstd=ldmMinMatch`. `0` = 64.
    pub ldm_min_match: u32,
    /// `--zstd=ldmHashRateLog`. `0` = default from windowLog.
    pub ldm_hash_rate_log: u32,
    /// `--zstd=nbWorkers`. `0` = leave unset (CLI `-T` / `--single-thread` win).
    pub nb_workers: u32,
    /// `--zstd=jobSize`. `0` = default.
    pub job_size: u32,
    /// `--zstd=overlapLog`. `0` = default by strategy.
    pub overlap_log: u32,
}

impl CompressionParameters {
    /// facebook/zstd `--show-default-cparams` / `--zstd=` line (no prefix).
    pub fn to_zstd_option_string(self) -> alloc::string::String {
        alloc::format!(
            "windowLog={},chainLog={},hashLog={},searchLog={},minMatch={},targetLength={},strategy={}",
            self.window_log,
            self.chain_log,
            self.hash_log,
            self.search_log,
            self.min_match,
            self.target_length,
            self.strategy.id()
        )
    }

    /// Apply a comma-separated `--zstd=key=value,...` spec (prefix optional).
    pub fn apply_zstd_option_string(&mut self, spec: &str) -> Result<(), Error> {
        let spec = spec.strip_prefix("--zstd=").unwrap_or(spec);
        for part in spec.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (k, v) = part.split_once('=').ok_or(Error::Corruption)?;
            let val: i32 = v.trim().parse().map_err(|_| Error::Corruption)?;
            self.apply_zstd_kv(k.trim(), val)?;
        }
        Ok(())
    }

    /// LDM knobs parsed from `--zstd=` (CLI `--long` may still force `enable`).
    pub fn ldm_params(self) -> crate::ldm::LdmParams {
        crate::ldm::LdmParams {
            enable: self.enable_ldm,
            hash_log: self.ldm_hash_log,
            min_match: self.ldm_min_match,
            hash_rate_log: self.ldm_hash_rate_log,
        }
    }

    /// Apply one `--zstd=key=value` assignment. Unknown keys are errors.
    pub fn apply_zstd_kv(&mut self, key: &str, value: i32) -> Result<(), Error> {
        match key {
            "windowLog" | "wlog" => self.window_log = value.max(10) as u32,
            "chainLog" | "clog" => self.chain_log = value.max(6) as u32,
            "hashLog" | "hlog" => self.hash_log = value.max(6) as u32,
            "searchLog" | "slog" => self.search_log = value.max(1) as u32,
            "minMatch" | "mml" => self.min_match = (value as u32).clamp(3, 7),
            "targetLength" | "tlen" => self.target_length = value.max(0) as u32,
            "strategy" | "strat" => {
                self.strategy = Strategy::from_id(value).ok_or(Error::Corruption)?;
            }
            "enableLdm" => self.enable_ldm = value != 0,
            "ldmHashLog" | "ldmhlog" => self.ldm_hash_log = value.max(0) as u32,
            "ldmMinMatch" | "ldmmml" => self.ldm_min_match = value.max(0) as u32,
            "ldmHashRateLog" | "ldmhrlog" => self.ldm_hash_rate_log = value.max(0) as u32,
            "ldmBucketSizeLog" | "ldmblog" => {}
            "nbWorkers" => self.nb_workers = value.max(0) as u32,
            "jobSize" => self.job_size = value.max(0) as u32,
            "overlapLog" | "ovlog" => self.overlap_log = (value as u32).min(9),
            _ => return Err(Error::Corruption),
        }
        Ok(())
    }
}

const KB: u64 = 1024;

/// Tuple: window, chain, hash, search, minMatch, targetLength, strategy.
type Row = (u32, u32, u32, u32, u32, u32, Strategy);

fn table_for_size(src_hint: Option<u64>) -> &'static [Row; 23] {
    let id = match src_hint {
        None => 0,
        Some(n) if n <= 16 * KB => 3,
        Some(n) if n <= 128 * KB => 2,
        Some(n) if n <= 256 * KB => 1,
        Some(_) => 0,
    };
    match id {
        3 => &TABLE_16K,
        2 => &TABLE_128K,
        1 => &TABLE_256K,
        _ => &TABLE_DEFAULT,
    }
}

// facebook/zstd v1.5.7 lib/compress/clevels.h -- all 23 rows.
const TABLE_DEFAULT: [Row; 23] = [
    (19, 12, 13, 1, 6, 1, Strategy::Fast),
    (19, 13, 14, 1, 7, 0, Strategy::Fast),
    (20, 15, 16, 1, 6, 0, Strategy::Fast),
    (21, 16, 17, 1, 5, 0, Strategy::DFast),
    (21, 18, 18, 1, 5, 0, Strategy::DFast),
    (21, 18, 19, 3, 5, 2, Strategy::Greedy),
    (21, 18, 19, 3, 5, 4, Strategy::Lazy),
    (21, 19, 20, 4, 5, 8, Strategy::Lazy),
    (21, 19, 20, 4, 5, 16, Strategy::Lazy2),
    (22, 20, 21, 4, 5, 16, Strategy::Lazy2),
    (22, 21, 22, 5, 5, 16, Strategy::Lazy2),
    (22, 21, 22, 6, 5, 16, Strategy::Lazy2),
    (22, 22, 23, 6, 5, 32, Strategy::Lazy2),
    (22, 22, 22, 4, 5, 32, Strategy::BtLazy2),
    (22, 22, 23, 5, 5, 32, Strategy::BtLazy2),
    (22, 23, 23, 6, 5, 32, Strategy::BtLazy2),
    (22, 22, 22, 5, 5, 48, Strategy::BtOpt),
    (23, 23, 22, 5, 4, 64, Strategy::BtOpt),
    (23, 23, 22, 6, 3, 64, Strategy::BtUltra),
    (23, 24, 22, 7, 3, 256, Strategy::BtUltra2),
    (25, 25, 23, 7, 3, 256, Strategy::BtUltra2),
    (26, 26, 24, 7, 3, 512, Strategy::BtUltra2),
    (27, 27, 25, 9, 3, 999, Strategy::BtUltra2),
];
const TABLE_256K: [Row; 23] = [
    (18, 12, 13, 1, 5, 1, Strategy::Fast),
    (18, 13, 14, 1, 6, 0, Strategy::Fast),
    (18, 14, 14, 1, 5, 0, Strategy::DFast),
    (18, 16, 16, 1, 4, 0, Strategy::DFast),
    (18, 16, 17, 3, 5, 2, Strategy::Greedy),
    (18, 17, 18, 5, 5, 2, Strategy::Greedy),
    (18, 18, 19, 3, 5, 4, Strategy::Lazy),
    (18, 18, 19, 4, 4, 4, Strategy::Lazy),
    (18, 18, 19, 4, 4, 8, Strategy::Lazy2),
    (18, 18, 19, 5, 4, 8, Strategy::Lazy2),
    (18, 18, 19, 6, 4, 8, Strategy::Lazy2),
    (18, 18, 19, 5, 4, 12, Strategy::BtLazy2),
    (18, 19, 19, 7, 4, 12, Strategy::BtLazy2),
    (18, 18, 19, 4, 4, 16, Strategy::BtOpt),
    (18, 18, 19, 4, 3, 32, Strategy::BtOpt),
    (18, 18, 19, 6, 3, 128, Strategy::BtOpt),
    (18, 19, 19, 6, 3, 128, Strategy::BtUltra),
    (18, 19, 19, 8, 3, 256, Strategy::BtUltra),
    (18, 19, 19, 6, 3, 128, Strategy::BtUltra2),
    (18, 19, 19, 8, 3, 256, Strategy::BtUltra2),
    (18, 19, 19, 10, 3, 512, Strategy::BtUltra2),
    (18, 19, 19, 12, 3, 512, Strategy::BtUltra2),
    (18, 19, 19, 13, 3, 999, Strategy::BtUltra2),
];
const TABLE_128K: [Row; 23] = [
    (17, 12, 12, 1, 5, 1, Strategy::Fast),
    (17, 12, 13, 1, 6, 0, Strategy::Fast),
    (17, 13, 15, 1, 5, 0, Strategy::Fast),
    (17, 15, 16, 2, 5, 0, Strategy::DFast),
    (17, 17, 17, 2, 4, 0, Strategy::DFast),
    (17, 16, 17, 3, 4, 2, Strategy::Greedy),
    (17, 16, 17, 3, 4, 4, Strategy::Lazy),
    (17, 16, 17, 3, 4, 8, Strategy::Lazy2),
    (17, 16, 17, 4, 4, 8, Strategy::Lazy2),
    (17, 16, 17, 5, 4, 8, Strategy::Lazy2),
    (17, 16, 17, 6, 4, 8, Strategy::Lazy2),
    (17, 17, 17, 5, 4, 8, Strategy::BtLazy2),
    (17, 18, 17, 7, 4, 12, Strategy::BtLazy2),
    (17, 18, 17, 3, 4, 12, Strategy::BtOpt),
    (17, 18, 17, 4, 3, 32, Strategy::BtOpt),
    (17, 18, 17, 6, 3, 256, Strategy::BtOpt),
    (17, 18, 17, 6, 3, 128, Strategy::BtUltra),
    (17, 18, 17, 8, 3, 256, Strategy::BtUltra),
    (17, 18, 17, 10, 3, 512, Strategy::BtUltra),
    (17, 18, 17, 5, 3, 256, Strategy::BtUltra2),
    (17, 18, 17, 7, 3, 512, Strategy::BtUltra2),
    (17, 18, 17, 9, 3, 512, Strategy::BtUltra2),
    (17, 18, 17, 11, 3, 999, Strategy::BtUltra2),
];
const TABLE_16K: [Row; 23] = [
    (14, 12, 13, 1, 5, 1, Strategy::Fast),
    (14, 14, 15, 1, 5, 0, Strategy::Fast),
    (14, 14, 15, 1, 4, 0, Strategy::Fast),
    (14, 14, 15, 2, 4, 0, Strategy::DFast),
    (14, 14, 14, 4, 4, 2, Strategy::Greedy),
    (14, 14, 14, 3, 4, 4, Strategy::Lazy),
    (14, 14, 14, 4, 4, 8, Strategy::Lazy2),
    (14, 14, 14, 6, 4, 8, Strategy::Lazy2),
    (14, 14, 14, 8, 4, 8, Strategy::Lazy2),
    (14, 15, 14, 5, 4, 8, Strategy::BtLazy2),
    (14, 15, 14, 9, 4, 8, Strategy::BtLazy2),
    (14, 15, 14, 3, 4, 12, Strategy::BtOpt),
    (14, 15, 14, 4, 3, 24, Strategy::BtOpt),
    (14, 15, 14, 5, 3, 32, Strategy::BtUltra),
    (14, 15, 15, 6, 3, 64, Strategy::BtUltra),
    (14, 15, 15, 7, 3, 256, Strategy::BtUltra),
    (14, 15, 15, 5, 3, 48, Strategy::BtUltra2),
    (14, 15, 15, 6, 3, 128, Strategy::BtUltra2),
    (14, 15, 15, 7, 3, 256, Strategy::BtUltra2),
    (14, 15, 15, 8, 3, 256, Strategy::BtUltra2),
    (14, 15, 15, 8, 3, 512, Strategy::BtUltra2),
    (14, 15, 15, 9, 3, 512, Strategy::BtUltra2),
    (14, 15, 15, 10, 3, 999, Strategy::BtUltra2),
];

fn row_to_params(row: Row) -> CompressionParameters {
    CompressionParameters {
        window_log: row.0,
        chain_log: row.1,
        hash_log: row.2,
        search_log: row.3,
        min_match: row.4,
        target_length: row.5,
        strategy: row.6,
        enable_ldm: false,
        ldm_hash_log: 0,
        ldm_min_match: 0,
        ldm_hash_rate_log: 0,
        nb_workers: 0,
        job_size: 0,
        overlap_log: 0,
    }
}

/// Parameters C would pick at `level` for an optional size hint (`None` = unknown / large).
pub fn compression_params(
    level: i32,
    src_hint: Option<u64>,
) -> Result<CompressionParameters, Error> {
    if !(MIN_CLEVEL..=MAX_CLEVEL).contains(&level) {
        return Err(Error::InvalidLevel);
    }
    let table = table_for_size(src_hint);
    let mut p = if level < 0 {
        let mut cp = row_to_params(table[0]);
        cp.target_length = (-level) as u32;
        cp
    } else if level == 0 {
        row_to_params(table[3])
    } else {
        row_to_params(table[level as usize])
    };
    if let Some(n) = src_hint {
        if n > 0 {
            let src_log = (n.max(64) - 1).ilog2() + 1;
            if p.window_log > src_log {
                p.window_log = src_log.max(10);
            }
        }
    }
    if p.window_log < 10 {
        p.window_log = 10;
    }
    p.hash_log = p.hash_log.clamp(6, 24);
    if p.chain_log > 24 {
        p.chain_log = 24;
    }
    p.min_match = p.min_match.clamp(3, 7);
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level1_large_is_fast() {
        let p = compression_params(1, Some(32 * 1024 * 1024)).unwrap();
        assert_eq!(p.strategy, Strategy::Fast);
        assert_eq!(p.window_log, 19);
        assert_eq!(p.min_match, 7);
    }

    #[test]
    fn level3_large_is_dfast() {
        let p = compression_params(3, Some(32 * 1024 * 1024)).unwrap();
        assert_eq!(p.strategy, Strategy::DFast);
        assert_eq!(p.window_log, 21);
    }

    #[test]
    fn level19_large_is_btultra2() {
        let p = compression_params(19, Some(32 * 1024 * 1024)).unwrap();
        assert_eq!(p.strategy, Strategy::BtUltra2);
        assert_eq!(p.window_log, 23);
        assert_eq!(p.min_match, 3);
    }

    #[test]
    fn negative_sets_target_length() {
        let p = compression_params(-7, Some(32 * 1024 * 1024)).unwrap();
        assert_eq!(p.strategy, Strategy::Fast);
        assert_eq!(p.target_length, 7);
    }

    #[test]
    fn zstd_option_roundtrip_keys() {
        let mut p = compression_params(3, Some(1 << 20)).unwrap();
        p.apply_zstd_kv("wlog", 18).unwrap();
        p.apply_zstd_kv("strategy", 4).unwrap();
        assert_eq!(p.window_log, 18);
        assert_eq!(p.strategy, Strategy::Lazy);
        let s = p.to_zstd_option_string();
        assert!(s.contains("windowLog=18"));
        assert!(s.contains("strategy=4"));
        p.apply_zstd_kv("enableLdm", 1).unwrap();
        p.apply_zstd_kv("ldmHashLog", 12).unwrap();
        p.apply_zstd_kv("ldmMinMatch", 64).unwrap();
        p.apply_zstd_kv("ldmHashRateLog", 7).unwrap();
        p.apply_zstd_kv("ldmBucketSizeLog", 3).unwrap();
        assert!(p.enable_ldm);
        assert_eq!(p.ldm_hash_log, 12);
        assert_eq!(p.ldm_min_match, 64);
        assert_eq!(p.ldm_hash_rate_log, 7);
        p.apply_zstd_kv("nbWorkers", 4).unwrap();
        p.apply_zstd_kv("jobSize", 524288).unwrap();
        p.apply_zstd_kv("overlapLog", 9).unwrap();
        assert_eq!(p.nb_workers, 4);
        assert_eq!(p.job_size, 524288);
        assert_eq!(p.overlap_log, 9);
    }

    #[test]
    fn every_level_resolves() {
        for level in crate::MIN_CLEVEL..=crate::MAX_CLEVEL {
            let p = compression_params(level, Some(1 << 20)).unwrap();
            assert!(p.window_log >= 10);
            assert!((1..=9).contains(&p.strategy.id()));
        }
    }
}
