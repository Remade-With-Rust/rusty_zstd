# Pinned C zstd oracle

rusty_zstd **never links libzstd**. The bench crate shells out to a facebook/zstd CLI whose version is pinned here.

| Field | Value |
|---|---|
| Tag | `v1.5.7` |
| Source | https://github.com/facebook/zstd/releases/tag/v1.5.7 |
| Windows zip | https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-v1.5.7-win64.zip |
| Zip SHA-256 | `acb4e8111511749dc7a3ebedca9b04190e37a17afeb73f55d4425dbf0b90fad9` |
| `zstd.exe` SHA-256 | `8076aae03feac7c66b319579e82172eed168deed2a3f25e5e2d3c60f55e84111` |

Fetch and verify:

```powershell
pwsh scripts/fetch-oracle.ps1
```

Override: environment variable `RUSTY_ZSTD_ORACLE` = full path to `zstd` / `zstd.exe`. The harness still requires `--version` to contain `1.5.7`. On Windows it also requires the exe SHA-256 above (skip the SHA check by using a non-Windows host, or replace the pin in `crates/rusty_zstd-bench/src/oracle.rs` in lockstep with this file).

Linux/macOS: install `zstd` 1.5.7 from the distro or build the tagged source; point `RUSTY_ZSTD_ORACLE` at the binary. Do not use a PATH `zstd` that might be our `rzstd`.
