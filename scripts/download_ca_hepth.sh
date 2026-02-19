#!/usr/bin/env bash
# Copyright 2026 naadir jeewa
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

set -euo pipefail

# Download and prepare ca-HepTh dataset from SNAP
# Output: data/ca-HepTh/ in BvGraph format

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="$PROJECT_ROOT/data/ca-HepTh"
SNAP_URL="https://snap.stanford.edu/data/ca-HepTh.txt.gz"

if [ -f "$DATA_DIR/ca-HepTh.graph" ]; then
    echo "Dataset already exists at $DATA_DIR"
    exit 0
fi

mkdir -p "$DATA_DIR"
TMPFILE=$(mktemp)
trap 'rm -f "$TMPFILE" "$TMPFILE.txt"' EXIT

echo "Downloading ca-HepTh from SNAP..."
curl -fsSL "$SNAP_URL" -o "$TMPFILE"

echo "Extracting..."
gunzip -c "$TMPFILE" > "$TMPFILE.txt"

echo "Converting to BvGraph format via webgraph..."
# webgraph from arcs handles # comment lines by default
if ! command -v webgraph &>/dev/null; then
    echo "webgraph CLI not found. Attempting to install..."
    cargo install webgraph-cli 2>/dev/null || {
        echo "Error: Cannot install webgraph CLI. Install manually: cargo install webgraph-cli"
        exit 1
    }
fi

# --labels remaps arXiv paper IDs (non-contiguous, up to ~68K) to sequential 0..N-1
webgraph from arcs --labels "$DATA_DIR/ca-HepTh" < "$TMPFILE.txt"

echo "Dataset prepared at $DATA_DIR"
echo "Files:"
ls -la "$DATA_DIR/"
