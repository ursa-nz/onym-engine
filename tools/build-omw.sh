#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build omw.onym from a data dir holding the ground WNDB base, the decompressed OEWN LMF, and the OMW
# components. This is the translations overlay producer; the onym-data repository drives it, where
# recipes/fetch-sources.sh produces the inputs and recipes/build.sh calls this script with an
# explicit data dir. The base must be the freshly ground one whose index.sense the overlay keys to.
#
# Usage: tools/build-omw.sh [data-dir] [out-file]
#   data-dir default: tools/data   (holds wordnet/, oewn-plus.xml, and omw/*.xml)
#   out-file default: data-dir/omw.onym

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
DATA="${1:-$HERE/data}"
OUT="${2:-$DATA/omw.onym}"

[ -f "$DATA/wordnet/index.sense" ] || { echo "missing $DATA/wordnet/index.sense; run onym-data/recipes/build.sh first" >&2; exit 1; }
[ -f "$DATA/oewn-plus.xml" ] || { echo "missing $DATA/oewn-plus.xml (the decompressed OEWN LMF)" >&2; exit 1; }
components=("$DATA"/omw/*.xml)
[ -e "${components[0]}" ] || { echo "missing $DATA/omw/*.xml; run onym-data/recipes/fetch-sources.sh" >&2; exit 1; }

cargo build --release --manifest-path "$HERE/omw-build/Cargo.toml"

"$HERE/omw-build/target/release/omw-build" \
  --wordnet "$DATA/wordnet" \
  --lmf "$DATA/oewn-plus.xml" \
  --out "$OUT" \
  "${components[@]}"

echo "==> wrote $OUT"
