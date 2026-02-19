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

use crate::benchmark::hardware_profile::HardwareProfile;

pub fn run_compare(baseline: &str, candidate: &str) -> crate::core::report::BenchmarkOutcome {
    let profile = HardwareProfile {
        id: "pinned-linux-x86_64".to_string(),
        pinned: true,
    };
    crate::compare_baseline(baseline, candidate, "default-suite", &profile)
}

#[cfg(feature = "profiling")]
pub mod benchmark_cli {
    use clap::{Parser, Subcommand, ValueEnum};
    use std::path::PathBuf;

    #[derive(Clone, Debug, ValueEnum)]
    pub enum OutputFormatArg {
        Json,
        Csv,
        Chart,
        All,
    }

    #[derive(Clone, Debug, ValueEnum)]
    pub enum RunModeArg {
        Deterministic,
        Throughput,
    }

    #[derive(Clone, Debug, ValueEnum)]
    pub enum ProfilerArg {
        Perf,
        Samply,
    }

    #[derive(Clone, Debug, ValueEnum)]
    pub enum ProfileBinaryArg {
        ProfileRun,
        ProfileIncremental,
    }

    impl ProfileBinaryArg {
        pub fn as_binary_name(&self) -> &str {
            match self {
                ProfileBinaryArg::ProfileRun => "profile_run",
                ProfileBinaryArg::ProfileIncremental => "profile_incremental",
            }
        }
    }

    #[derive(Parser, Debug)]
    #[command(
        name = "hit-leiden",
        about = "HIT-Leiden benchmark and profiling tools"
    )]
    pub struct Cli {
        #[command(subcommand)]
        pub command: Commands,
    }

    #[derive(Subcommand, Debug)]
    pub enum Commands {
        /// Run the time-bounded benchmark suite with progress output and chart generation
        Benchmark(BenchmarkArgs),
        /// Run the profiling harness, wrapping an external profiler
        Profile(ProfileArgs),
    }

    #[derive(Parser, Debug)]
    pub struct BenchmarkArgs {
        /// Graph source (e.g. "file")
        #[arg(long)]
        pub source: String,

        /// Path to graph data
        #[arg(long)]
        pub path: PathBuf,

        /// Wall-clock timeout in seconds (omit for no timeout)
        #[arg(long)]
        pub timeout: Option<u64>,

        /// Where to write outputs
        #[arg(long, default_value = "artifacts/benchmark")]
        pub output_dir: PathBuf,

        /// Skip chart generation
        #[arg(long)]
        pub no_chart: bool,

        /// Output format
        #[arg(long, value_enum, default_value = "all")]
        pub format: OutputFormatArg,

        /// Execution mode: deterministic (single-threaded) or throughput (parallel)
        #[arg(long, value_enum, default_value = "deterministic")]
        pub mode: RunModeArg,

        /// Skip igraph baseline comparison (only run HIT-Leiden)
        #[arg(long)]
        pub no_igraph: bool,
    }

    #[derive(Parser, Debug)]
    pub struct ProfileArgs {
        #[command(subcommand)]
        pub subcommand: Option<ProfileSubcommands>,

        /// Which binary to profile
        #[arg(long, value_enum, default_value = "profile-run")]
        pub binary: ProfileBinaryArg,

        /// External profiler to use
        #[arg(long, value_enum, default_value = "perf")]
        pub profiler: ProfilerArg,

        /// Profiling duration in seconds
        #[arg(long, default_value = "10")]
        pub duration: u64,

        /// Sampling frequency (Hz)
        #[arg(long, default_value = "99")]
        pub frequency: u32,

        /// Where to write outputs
        #[arg(long, default_value = "artifacts/profiling")]
        pub output_dir: PathBuf,

        /// Number of hotspots in summary
        #[arg(long, default_value = "20")]
        pub top_n: usize,
    }

    #[derive(Subcommand, Debug)]
    pub enum ProfileSubcommands {
        /// Export profiling data as TOON text for AI consumption
        Export(ExportArgs),
        /// Compare two profiling runs and output a TOON-formatted diff
        Compare(CompareArgs),
    }

    #[derive(Parser, Debug)]
    pub struct ExportArgs {
        /// Path to pprof .pb.gz or samply .json.gz file
        #[arg(long)]
        pub input: PathBuf,

        /// Number of hotspots to include
        #[arg(long, default_value = "20")]
        pub top_n: usize,

        /// Write to file instead of stdout
        #[arg(long)]
        pub output: Option<PathBuf>,
    }

    #[derive(Parser, Debug)]
    pub struct CompareArgs {
        /// Path to baseline .pb.gz or .json.gz
        #[arg(long)]
        pub baseline: PathBuf,

        /// Path to candidate .pb.gz or .json.gz
        #[arg(long)]
        pub candidate: PathBuf,

        /// Minimum relative change (%) to report
        #[arg(long, default_value = "10.0")]
        pub threshold: f64,

        /// Number of functions to include
        #[arg(long, default_value = "20")]
        pub top_n: usize,

        /// Write to file instead of stdout
        #[arg(long)]
        pub output: Option<PathBuf>,
    }
}
