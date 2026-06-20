#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 ursa.nz <code@ursa.nz>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fetch the two source datasets the etymology overlay is built from, into a local directory the
# engine never ships. WordNet is pinned by checksum (it is frozen); wiktextract is dated and its
# checksum recorded at fetch time, because kaikki.org republishes it as Wiktionary changes.
#
# Usage: tools/fetch-data.sh [dest-dir]   (default: tools/data)

set -euo pipefail

DEST="${1:-$(cd "$(dirname "$0")" && pwd)/data}"
mkdir -p "$DEST"

# The WordNet 3.0 database, Debian's wordnet-base, pinned exactly as the Onym Flatpak manifest pins
# it so the overlay joins against identical bytes.
WORDNET_DEB_URL="https://deb.debian.org/debian/pool/main/w/wordnet/wordnet-base_3.0-41_all.deb"
WORDNET_DEB_SHA256="e50d14b2ee444eaf36ef2a3bd38e50c623b47ba22b2301adc4a5f12736da9264"

# The wiktextract English extract from kaikki.org: English headwords with parsed etymology prose.
WIKT_URL="https://kaikki.org/dictionary/English/kaikki.org-dictionary-English.jsonl.gz"

echo "==> WordNet (Debian wordnet-base 3.0-41)"
if [ ! -d "$DEST/wordnet" ]; then
  curl -sSL -o "$DEST/wordnet-base.deb" "$WORDNET_DEB_URL"
  echo "$WORDNET_DEB_SHA256  $DEST/wordnet-base.deb" | sha256sum -c -
  ( cd "$DEST" && ar x wordnet-base.deb && tar -xJf data.tar.xz )
  mv "$DEST/usr/share/wordnet" "$DEST/wordnet"
  rm -rf "$DEST/usr" "$DEST/control.tar.xz" "$DEST/data.tar.xz" "$DEST/debian-binary" "$DEST/wordnet-base.deb"
fi
echo "    $DEST/wordnet ($(ls "$DEST/wordnet" | wc -l) files)"

echo "==> wiktextract (kaikki.org English)"
curl -sSL -o "$DEST/wikt-en.jsonl.gz" "$WIKT_URL"
WIKT_SHA256="$(sha256sum "$DEST/wikt-en.jsonl.gz" | cut -d' ' -f1)"
WIKT_DATE="$(curl -sI "$WIKT_URL" | awk -F': ' 'tolower($1)=="last-modified"{print $2}' | tr -d '\r')"
echo "    $DEST/wikt-en.jsonl.gz ($(du -h "$DEST/wikt-en.jsonl.gz" | cut -f1))"

# Record what was fetched, for the overlay's provenance.
cat > "$DEST/SOURCES.txt" <<EOF
wordnet-base.deb  $WORDNET_DEB_URL
                  sha256 $WORDNET_DEB_SHA256
wikt-en.jsonl.gz  $WIKT_URL
                  last-modified $WIKT_DATE
                  sha256 $WIKT_SHA256
EOF
echo "==> wrote $DEST/SOURCES.txt"
