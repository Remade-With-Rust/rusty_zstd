#!/usr/bin/env bash
# Fetch Silesia into corpora/data/silesia/ and record SHA-256.
# Generated corpora are created by rzstd-bench; this script is optional.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA="$ROOT/corpora/data"
URL="${SILESIA_URL:-http://sun.aei.polsl.pl/~sdeor/corpus/silesia.zip}"
mkdir -p "$DATA"
ZIP="$DATA/silesia.zip"
if [[ ! -f "$ZIP" ]]; then
  echo "downloading $URL"
  curl -L --fail -o "$ZIP" "$URL"
fi
shasum -a 256 "$ZIP" | tee "$DATA/SHA256SUMS"
mkdir -p "$DATA/silesia"
unzip -o "$ZIP" -d "$DATA/silesia"
echo "silesia extracted under $DATA/silesia"
