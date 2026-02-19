// Copyright 2026 naadir jeewa
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use criterion::{criterion_group, criterion_main, Criterion};
use hit_leiden::{run, GraphInput, RunConfig, RunMode};
use lender::prelude::*;
use std::path::Path;
use webgraph::prelude::*;

fn load_uk_2007() -> GraphInput {
    let path = Path::new("data/uk-2007-05@100000/uk-2007-05@100000");
    if !path.with_extension("graph").exists() {
        println!(
            "uk-2007-05@100000 dataset not found at {:?}. Skipping.",
            path
        );
        return GraphInput::empty("uk-2007-05@100000");
    }

    println!("Loading uk-2007-05@100000 dataset...");
    let graph = webgraph::graphs::bvgraph::sequential::BvGraphSeq::with_basename(path)
        .load()
        .expect("Failed to load webgraph");
    let num_nodes = graph.num_nodes();

    let mut edges = Vec::with_capacity(graph.num_arcs_hint().unwrap_or(0) as usize);
    for_![(src, succ) in graph {
        for dst in succ {
            // Only add edges once to avoid duplicates when InMemoryGraph makes it undirected
            if src <= dst {
                edges.push((src, dst, None::<f64>));
            }
        }
    }];

    println!(
        "Loaded {} nodes and {} undirected edges",
        num_nodes,
        edges.len()
    );

    GraphInput {
        dataset_id: "uk-2007-05@100000".to_string(),
        node_count: num_nodes,
        edges,
    }
}

fn bench_run(c: &mut Criterion) {
    let graph = load_uk_2007();
    if graph.node_count == 0 {
        return;
    }

    let mut group = c.benchmark_group("uk-2007-05@100000");
    group.sample_size(10);

    group.bench_function("hit_leiden_run_throughput", |b| {
        let config = RunConfig {
            mode: RunMode::Throughput,
            max_iterations: 1,
            quality_tolerance: 1.0, // single-pass: measure throughput, not convergence
            ..RunConfig::default()
        };
        b.iter(|| run(&graph, &config).expect("run"));
    });
    group.finish();
}

criterion_group!(benches, bench_run);
criterion_main!(benches);
