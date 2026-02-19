#!/usr/bin/env python3
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

"""Run igraph Leiden community detection on an edge list.

Reads a TSV edge file (src\tdst[\tweight]), runs leidenalg via python-igraph,
outputs JSON result to stdout. Designed to be called as a subprocess from the
HIT-Leiden Rust benchmark tool.
"""
import argparse
import json
import sys
import time


def main():
    parser = argparse.ArgumentParser(
        description="Run igraph Leiden and output JSON results"
    )
    parser.add_argument("--edge-file", required=True, help="TSV edge list file")
    parser.add_argument("--num-nodes", required=True, type=int, help="Number of nodes")
    parser.add_argument(
        "--resolution", default=1.0, type=float, help="Resolution parameter (gamma)"
    )
    args = parser.parse_args()

    try:
        import igraph as ig
    except ImportError:
        print("Error: python-igraph not installed", file=sys.stderr)
        sys.exit(1)

    try:
        import leidenalg
    except ImportError:
        print("Error: leidenalg not installed", file=sys.stderr)
        sys.exit(1)

    print(
        f"igraph {ig.__version__}, leidenalg {leidenalg.__version__}",
        file=sys.stderr,
    )

    # Read edge list
    edges = []
    weights = []
    with open(args.edge_file) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            src, dst = int(parts[0]), int(parts[1])
            w = float(parts[2]) if len(parts) > 2 else 1.0
            edges.append((src, dst))
            weights.append(w)

    print(
        f"Loaded {len(edges)} edges, {args.num_nodes} nodes",
        file=sys.stderr,
    )

    g = ig.Graph(n=args.num_nodes, edges=edges, directed=False)
    g.es["weight"] = weights

    # Run Leiden -- time only the algorithm, not graph construction
    start = time.perf_counter()
    partition = leidenalg.find_partition(
        g,
        leidenalg.ModularityVertexPartition,
        weights="weight",
        n_iterations=-1,  # iterate until stable
        seed=42,
    )
    elapsed_ms = (time.perf_counter() - start) * 1000.0

    membership = partition.membership
    modularity = partition.modularity
    num_communities = len(set(membership))

    print(
        f"Leiden completed in {elapsed_ms:.2f}ms "
        f"(Q={modularity:.4f}, {num_communities} communities)",
        file=sys.stderr,
    )

    result = {
        "time_ms": elapsed_ms,
        "modularity": modularity,
        "num_communities": num_communities,
        "partition": membership,
    }
    json.dump(result, sys.stdout)


if __name__ == "__main__":
    main()
