#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build etym.onym from a data dir holding wordnet/ and wikt-en.jsonl.gz. This is the overlay
# producer; the onym-data repository drives it, where recipes/fetch-sources.sh produces the inputs
# and recipes/build.sh calls this script with an explicit data dir.
#
# Usage: tools/build-overlay.sh [data-dir] [out-file]
#   data-dir default: tools/data            (holds wordnet/ and wikt-en.jsonl.gz)
#   out-file default: tools/data/etym.onym

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
DATA="${1:-$HERE/data}"
OUT="${2:-$DATA/etym.onym}"

[ -d "$DATA/wordnet" ] || { echo "missing $DATA/wordnet; run onym-data/recipes/fetch-sources.sh" >&2; exit 1; }
[ -f "$DATA/wikt-en.jsonl.gz" ] || { echo "missing $DATA/wikt-en.jsonl.gz; run onym-data/recipes/fetch-sources.sh" >&2; exit 1; }

cargo build --release --manifest-path "$HERE/etym-build/Cargo.toml"

# Stream the gz through the tool so the 3 GB plain JSONL is never written to disk.
zcat "$DATA/wikt-en.jsonl.gz" | "$HERE/etym-build/target/release/etym-build" \
  --wordnet "$DATA/wordnet" \
  --out "$OUT" \
  --source "kaikki.org English wiktextract"

echo "==> wrote $OUT"
