#!/bin/bash
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

set -e

mkdir -p data/uk-2007-05@100000
cd data/uk-2007-05@100000

BASE="uk-2007-05@100000"
URL_BASE="http://data.law.di.unimi.it/webdata/uk-2007-05%40100000"

echo "Downloading uk-2007-05@100000 dataset..."
curl -O "${URL_BASE}/${BASE}.graph"
curl -O "${URL_BASE}/${BASE}.properties"
curl -O "${URL_BASE}/${BASE}.md5sums"

echo "Verifying checksums..."
md5sum -c "${BASE}.md5sums" 2>/dev/null | grep OK || echo "Checksum verification failed for some files, but graph and properties might be OK."

echo "Done. You can now run the benchmark with:"
echo "cargo bench --bench hit_leiden_suite"
