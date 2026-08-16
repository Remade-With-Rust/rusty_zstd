# Corpora

Blobs are **not** in git. Recipes and hashes are.

## Generated (always available)

Created by `rzstd-bench` under `corpora/data/generated/` (gitignored).

| id | split | size | recipe |
|---|---|---:|---|
| `zeros-32m` | train | 32 MiB | `0x00` bytes |
| `text-32m` | train | 32 MiB | repeating `The quick brown fox jumps over the lazy dog. 0123456789.\n` |
| `incomp-32m` | holdout | 32 MiB | xorshift64 (`seed = 0xA5A55A5A`, never zeroed) |
| `zeros-1m` | train | 1 MiB | zeros; `--smoke` only |

SHA-256 is computed at generation and written on each ledger line. Re-generating the same recipe must match.

**Holdout discipline:** M1+ exit gates use `split=holdout` only. Train files may be used to debug, never to pass a gate.

## Silesia (optional, real content)

Standard lossless bar. Not required for M0 plumbing.

Zip (canonical distribution):

- URL: `http://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip`
- Fetch: `pwsh scripts/fetch-corpora.ps1` (records SHA-256 into `corpora/data/SHA256SUMS` on first success)

Train (tuning, later): `dickens`, `mozilla`, `nci`, `samba`, `xml`, `x-ray`

Holdout (gates): `mr`, `ooffice`, `osdb`, `reymont`, `sao`, `webster`

Until the zip is fetched and hashed in `SHA256SUMS`, Silesia is absent and the generated set is the board.
