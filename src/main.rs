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

#[cfg(feature = "profiling")]
use clap::Parser;

#[cfg(feature = "profiling")]
use hit_leiden::cli::benchmark::benchmark_cli::{
    BenchmarkArgs, Cli, Commands, OutputFormatArg, ProfileArgs,
};

fn main() {
    #[cfg(not(feature = "profiling"))]
    {
        eprintln!("Error: This binary requires the 'profiling' feature.");
        eprintln!("Run with: cargo run --features profiling -- <command>");
        std::process::exit(1);
    }

    #[cfg(feature = "profiling")]
    {
        let cli = Cli::parse();
        let exit_code = match cli.command {
            Commands::Benchmark(args) => run_benchmark(args),
            Commands::Profile(args) => run_profile(args),
        };
        std::process::exit(exit_code);
    }
}

#[cfg(feature = "profiling")]
fn run_benchmark(args: BenchmarkArgs) -> i32 {
    use hit_leiden::benchmark::runner::benchmark_runner::{
        run_full_benchmark, BenchmarkRunnerConfig, OutputFormat,
    };
    use hit_leiden::cli::benchmark::benchmark_cli::RunModeArg;
    use hit_leiden::core::config::RunMode;

    // Validate dataset path
    let graph_path = &args.path;
    if !graph_path.with_extension("graph").exists() && !graph_path.exists() {
        eprintln!(
            "Error: Dataset not found at {}. Run 'cargo make data' to download.",
            graph_path.display()
        );
        return 1;
    }

    // Load graph via webgraph
    let graph = match load_webgraph(graph_path) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Error: Failed to load graph: {}", e);
            return 1;
        }
    };

    let format = match args.format {
        OutputFormatArg::Json => OutputFormat::Json,
        OutputFormatArg::Csv => OutputFormat::Csv,
        OutputFormatArg::Chart => OutputFormat::Chart,
        OutputFormatArg::All => OutputFormat::All,
    };

    let mode = match args.mode {
        RunModeArg::Deterministic => RunMode::Deterministic,
        RunModeArg::Throughput => RunMode::Throughput,
    };

    let config = BenchmarkRunnerConfig {
        graph,
        timeout_seconds: args.timeout,
        output_dir: args.output_dir,
        no_chart: args.no_chart,
        format,
        mode,
        no_igraph: args.no_igraph,
    };

    match run_full_benchmark(config) {
        Ok(run) => {
            if run.truncated {
                2 // timeout with partial results
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            1
        }
    }
}

#[cfg(feature = "profiling")]
fn run_profile(args: ProfileArgs) -> i32 {
    use hit_leiden::cli::benchmark::benchmark_cli::ProfileSubcommands;

    match args.subcommand {
        Some(ProfileSubcommands::Export(export_args)) => run_profile_export(export_args),
        Some(ProfileSubcommands::Compare(compare_args)) => run_profile_compare(compare_args),
        None => run_profile_record(args),
    }
}

#[cfg(feature = "profiling")]
fn run_profile_record(args: ProfileArgs) -> i32 {
    use hit_leiden::benchmark::profiling::run_profiling;
    use hit_leiden::cli::benchmark::benchmark_cli::ProfilerArg;
    use hit_leiden::core::types::Profiler;

    let profiler = match args.profiler {
        ProfilerArg::Perf => Profiler::Perf,
        ProfilerArg::Samply => Profiler::Samply,
    };

    match run_profiling(
        args.binary.as_binary_name(),
        &profiler,
        args.duration,
        args.frequency,
        &args.output_dir,
        args.top_n,
    ) {
        Ok(capture) => {
            eprintln!(
                "\nProfiling complete. {} samples collected in {:.1}s",
                capture.sample_count, capture.duration_seconds
            );
            eprintln!("Output: {}", capture.native_output_path.display());
            if let Some(ref pprof) = capture.pprof_path {
                eprintln!("pprof: {}", pprof.display());
            }
            0
        }
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

#[cfg(feature = "profiling")]
fn run_profile_export(args: hit_leiden::cli::benchmark::benchmark_cli::ExportArgs) -> i32 {
    use hit_leiden::benchmark::toon_export::export_toon;

    match export_toon(&args.input, args.top_n, args.output.as_deref()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

#[cfg(feature = "profiling")]
fn run_profile_compare(args: hit_leiden::cli::benchmark::benchmark_cli::CompareArgs) -> i32 {
    use hit_leiden::benchmark::toon_export::compare_toon;

    match compare_toon(
        &args.baseline,
        &args.candidate,
        args.threshold,
        args.top_n,
        args.output.as_deref(),
    ) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", e);
            1
        }
    }
}

#[cfg(feature = "profiling")]
fn load_webgraph(
    path: &std::path::Path,
) -> Result<hit_leiden::core::types::GraphInput, Box<dyn std::error::Error>> {
    use lender::prelude::*;
    use webgraph::prelude::*;

    let graph = webgraph::graphs::bvgraph::sequential::BvGraphSeq::with_basename(path)
        .load()
        .map_err(|e| format!("Failed to load webgraph: {}", e))?;

    let num_nodes = graph.num_nodes();
    let mut edges = Vec::with_capacity(graph.num_arcs_hint().unwrap_or(0) as usize);
    let mut self_loops = 0usize;

    for_![(src, succ) in graph {
        for dst in succ {
            if src == dst {
                self_loops += 1;
                continue; // Skip self-loops
            }
            if src < dst {
                edges.push((src, dst, None::<f64>));
            }
        }
    }];

    eprintln!(
        "Loaded {} nodes, {} undirected edges (skipped {} self-loops)",
        num_nodes,
        edges.len(),
        self_loops
    );

    Ok(hit_leiden::core::types::GraphInput {
        dataset_id: path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        node_count: num_nodes,
        edges,
    })
}
