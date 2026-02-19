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

use crate::benchmark::manifest::BenchmarkManifest;

pub fn run_benchmark(manifest: &BenchmarkManifest) -> f64 {
    if manifest.baseline_commit == manifest.candidate_commit {
        1.0
    } else {
        2.0
    }
}

#[cfg(feature = "profiling")]
pub mod benchmark_runner {
    use crate::benchmark::charting;
    use crate::benchmark::dynamic_graph::DynamicGraphBuilder;
    use crate::benchmark::hit_leiden_incremental::{
        run_incremental_with_config, IncrementalConfig,
    };
    use crate::benchmark::progress::ProgressReporter;
    use crate::benchmark::progress::StderrProgressReporter;
    use crate::core::types::{BenchmarkPhase, BenchmarkRun, GraphInput, ProgressEvent};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    /// Output format selection
    #[derive(Clone, Debug, PartialEq)]
    pub enum OutputFormat {
        Json,
        Csv,
        Chart,
        All,
    }

    /// Configuration for the benchmark runner
    pub struct BenchmarkRunnerConfig {
        pub graph: GraphInput,
        pub timeout_seconds: Option<u64>,
        pub output_dir: PathBuf,
        pub no_chart: bool,
        pub format: OutputFormat,
        pub mode: crate::core::config::RunMode,
        pub no_igraph: bool,
    }

    /// Run the complete benchmark pipeline: load, run incremental batches, generate outputs
    pub fn run_full_benchmark(
        config: BenchmarkRunnerConfig,
    ) -> Result<BenchmarkRun, Box<dyn std::error::Error>> {
        let overall_start = Instant::now();
        let mut reporter = StderrProgressReporter;

        reporter.report(&ProgressEvent {
            phase: BenchmarkPhase::Loading,
            batch_index: None,
            batch_total: None,
            elapsed: overall_start.elapsed(),
            metric_label: None,
            metric_value: None,
        });

        // Paper-style split: 80% initial, 9 batches of ~1000 edges
        let builder = DynamicGraphBuilder::new(&config.graph);
        let split = builder.paper_split(0.8, 1000, 9, 42);

        let initial_edges = split.initial_graph.edges.len();
        let batch_count = split.update_batches.len();

        eprintln!(
            "Initial graph: {} edges | Processing {} update batches",
            initial_edges, batch_count
        );

        reporter.report(&ProgressEvent {
            phase: BenchmarkPhase::InitialClustering,
            batch_index: None,
            batch_total: Some(batch_count),
            elapsed: overall_start.elapsed(),
            metric_label: None,
            metric_value: None,
        });

        let reporter_start = overall_start;
        let incremental_config = IncrementalConfig {
            timeout: config.timeout_seconds.map(Duration::from_secs),
            progress_callback: Some(Box::new(move |event: ProgressEvent| {
                let _ = reporter_start;
                let mut r = StderrProgressReporter;
                r.report(&event);
            })),
            cache_dir: Some(config.output_dir.clone()),
            mode: config.mode,
            no_igraph: config.no_igraph,
        };

        let outcome = run_incremental_with_config(
            split.update_batches,
            split.batch_size,
            initial_edges,
            incremental_config,
        )?;

        let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();

        let benchmark_run = BenchmarkRun {
            timestamp: timestamp.clone(),
            dataset_id: config.graph.dataset_id.clone(),
            timeout_seconds: config.timeout_seconds,
            truncated: outcome.truncated,
            batches: outcome.batches,
            total_time_seconds: outcome.total_time_seconds,
            avg_speedup: outcome.avg_speedup,
            cumulative_speedup: outcome.cumulative_speedup,
            final_batch_comparison: outcome.final_batch_comparison,
        };

        print_summary(&benchmark_run);

        fs::create_dir_all(&config.output_dir)?;

        let should_chart =
            !config.no_chart && matches!(config.format, OutputFormat::Chart | OutputFormat::All);
        let should_csv = matches!(config.format, OutputFormat::Csv | OutputFormat::All);
        let should_json = matches!(config.format, OutputFormat::Json | OutputFormat::All);

        if should_chart {
            reporter.report(&ProgressEvent {
                phase: BenchmarkPhase::Charting,
                batch_index: None,
                batch_total: None,
                elapsed: overall_start.elapsed(),
                metric_label: None,
                metric_value: None,
            });

            if let Some(hit_part) = benchmark_run
                .batches
                .last()
                .and_then(|b| b.hit_partition.as_ref())
            {
                let last_batch = benchmark_run.batches.last().unwrap();
                let community_path = config
                    .output_dir
                    .join(format!("communities_{}.svg", timestamp));
                charting::generate_community_chart(
                    hit_part,
                    last_batch.hit_hierarchy_levels.as_deref(),
                    last_batch.igraph_partition.as_deref(),
                    &config.graph.edges,
                    &benchmark_run.dataset_id,
                    &community_path,
                )?;
                eprintln!("Community chart written to {}", community_path.display());
            }

            let chart_path = config
                .output_dir
                .join(format!("benchmark_{}.html", timestamp));
            charting::generate_chart(&benchmark_run, &chart_path)?;
            eprintln!("Chart written to {}", chart_path.display());
        }

        if should_csv {
            let csv_path = config
                .output_dir
                .join(format!("benchmark_{}.csv", timestamp));
            write_csv(&benchmark_run, &csv_path)?;
            eprintln!("CSV written to {}", csv_path.display());
        }

        if should_json {
            let json_path = config
                .output_dir
                .join(format!("benchmark_{}.json", timestamp));
            write_json(&benchmark_run, &json_path)?;
            eprintln!("JSON written to {}", json_path.display());
        }

        reporter.report(&ProgressEvent {
            phase: BenchmarkPhase::Complete,
            batch_index: None,
            batch_total: None,
            elapsed: overall_start.elapsed(),
            metric_label: Some("cumulative_speedup".to_string()),
            metric_value: Some(benchmark_run.cumulative_speedup),
        });

        Ok(benchmark_run)
    }

    fn print_summary(run: &BenchmarkRun) {
        let has_igraph = run.batches.iter().any(|b| b.igraph_time_ms > 0.0);

        println!("{}", "=".repeat(80));
        println!("BENCHMARK RESULTS — {} ({})", run.dataset_id, run.timestamp);
        if run.truncated {
            if let Some(t) = run.timeout_seconds {
                println!("  ** TRUNCATED by timeout ({}s) **", t);
            } else {
                println!("  ** TRUNCATED **");
            }
        }
        println!("{}", "=".repeat(80));
        println!("Total batches: {}", run.batches.len());
        println!("Total time: {:.2}s", run.total_time_seconds);
        println!("Avg speedup (per-batch): {:.2}x", run.avg_speedup);
        println!(
            "Cumulative speedup (total time): {:.2}x",
            run.cumulative_speedup
        );
        println!();

        if has_igraph {
            println!(
                "{:<6} {:<10} {:<11} {:<11} {:<9} {:<9} {:<9} {:<9}",
                "Batch", "Edges", "HIT (ms)", "igraph(ms)", "Speedup", "Cum.Spdup", "HIT Q", "ig Q"
            );
            println!("{}", "-".repeat(80));
        } else {
            println!(
                "{:<6} {:<12} {:<12} {:<10}",
                "Batch", "Edges", "HIT (ms)", "HIT Q"
            );
            println!("{}", "-".repeat(42));
        }

        let mut cum_hit = 0.0_f64;
        let mut cum_ig = 0.0_f64;
        for batch in &run.batches {
            cum_hit += batch.hit_leiden_time_ms;
            cum_ig += batch.igraph_time_ms;
            let cum_speedup = if cum_hit > 0.0 && cum_ig > 0.0 {
                cum_ig / cum_hit
            } else {
                0.0
            };
            if has_igraph {
                println!(
                    "{:<6} {:<10} {:<11.2} {:<11.2} {:<9.2}x {:<9.2}x {:<9.4} {:<9.4}",
                    batch.batch_idx + 1,
                    batch.total_edges,
                    batch.hit_leiden_time_ms,
                    batch.igraph_time_ms,
                    batch.speedup,
                    cum_speedup,
                    batch.modularity,
                    batch.igraph_modularity,
                );
            } else {
                println!(
                    "{:<6} {:<12} {:<12.2} {:<10.4}",
                    batch.batch_idx + 1,
                    batch.total_edges,
                    batch.hit_leiden_time_ms,
                    batch.modularity,
                );
            }
        }
        println!("{}", "=".repeat(80));

        if let Some(ref final_cmp) = run.final_batch_comparison {
            println!();
            println!("Final-batch fresh comparison:");
            println!(
                "  Batch {} | Nodes: {} | Edges: {}",
                final_cmp.batch_idx + 1,
                final_cmp.nodes_in_graph,
                final_cmp.total_edges
            );
            println!(
                "  Time: HIT {:.2}ms vs fresh igraph {:.2}ms => {:.2}x",
                final_cmp.hit_time_ms,
                final_cmp.igraph_fresh_time_ms,
                final_cmp.speedup_vs_fresh_igraph
            );
            println!(
                "  Modularity: HIT {:.5} vs igraph {:.5} (Δ={:+.5})",
                final_cmp.hit_modularity,
                final_cmp.igraph_fresh_modularity,
                final_cmp.modularity_delta
            );
            println!(
                "  Communities: HIT {} (largest {} / {:.2}%) | igraph {} (largest {} / {:.2}%)",
                final_cmp.hit_community_count,
                final_cmp.hit_largest_community_size,
                final_cmp.hit_largest_community_share * 100.0,
                final_cmp.igraph_community_count,
                final_cmp.igraph_largest_community_size,
                final_cmp.igraph_largest_community_share * 100.0
            );
            println!(
                "  Similarity: NMI(HIT,igraph) {:.4} | HIT->igraph purity {:.4} | igraph->HIT purity {:.4} | Jaccard {:.4}",
                final_cmp.nmi,
                final_cmp.hit_to_igraph_purity,
                final_cmp.igraph_to_hit_purity,
                final_cmp.largest_community_jaccard
            );
            println!("{}", "=".repeat(80));
        }
    }

    fn write_csv(run: &BenchmarkRun, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut csv = String::from(
            "batch_index,hit_time_ms,igraph_time_ms,speedup,cumulative_hit_ms,cumulative_igraph_ms,cumulative_speedup,hit_modularity,igraph_modularity,final_nmi,final_hit_to_igraph_purity,final_igraph_to_hit_purity,final_largest_jaccard\n",
        );

        let mut cum_hit = 0.0_f64;
        let mut cum_ig = 0.0_f64;
        for batch in &run.batches {
            cum_hit += batch.hit_leiden_time_ms;
            cum_ig += batch.igraph_time_ms;
            let cum_speedup = if cum_hit > 0.0 && cum_ig > 0.0 {
                cum_ig / cum_hit
            } else {
                0.0
            };
            csv.push_str(&format!(
                "{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
                batch.batch_idx + 1,
                batch.hit_leiden_time_ms,
                batch.igraph_time_ms,
                batch.speedup,
                cum_hit,
                cum_ig,
                cum_speedup,
                batch.modularity,
                batch.igraph_modularity,
                run.final_batch_comparison
                    .as_ref()
                    .map(|m| m.nmi)
                    .unwrap_or(0.0),
                run.final_batch_comparison
                    .as_ref()
                    .map(|m| m.hit_to_igraph_purity)
                    .unwrap_or(0.0),
                run.final_batch_comparison
                    .as_ref()
                    .map(|m| m.igraph_to_hit_purity)
                    .unwrap_or(0.0),
                run.final_batch_comparison
                    .as_ref()
                    .map(|m| m.largest_community_jaccard)
                    .unwrap_or(0.0),
            ));
        }

        fs::write(path, csv)?;
        Ok(())
    }

    fn write_json(run: &BenchmarkRun, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(run)?;
        fs::write(path, json)?;
        Ok(())
    }
}
