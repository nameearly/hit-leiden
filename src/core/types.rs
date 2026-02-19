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

#[derive(Clone, Debug, PartialEq)]
pub enum GraphFormat {
    EdgeList,
    CsrBinary,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GraphSourceType {
    File,
    Neo4jSnapshot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphDataset {
    pub dataset_id: String,
    pub source_uri: String,
    pub is_weighted: bool,
    pub node_count: usize,
    pub edge_count: usize,
    pub checksum: String,
    pub format: GraphFormat,
    pub source_type: GraphSourceType,
    pub source_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunMode {
    Deterministic,
    Throughput,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunConfiguration {
    pub config_id: String,
    pub mode: RunMode,
    pub acceleration_enabled: bool,
    pub seed: Option<u64>,
    pub max_iterations: usize,
    pub quality_tolerance: f64,
    pub pinned_profile_id: Option<String>,
    pub graph_source: GraphSourceType,
}

impl Default for RunConfiguration {
    fn default() -> Self {
        Self {
            config_id: "default".to_string(),
            mode: RunMode::Deterministic,
            acceleration_enabled: false,
            seed: None,
            max_iterations: 10,
            quality_tolerance: 0.001,
            pinned_profile_id: None,
            graph_source: GraphSourceType::File,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BackendType {
    PureRust,
    NativeAccel,
    CudaAccel,
    RocmAccel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunExecution {
    pub run_id: String,
    pub dataset_id: String,
    pub config_id: String,
    pub started_at: u64, // timestamp
    pub completed_at: Option<u64>,
    pub status: RunStatus,
    pub backend: BackendType,
    pub graph_source_resolved: GraphSourceType,
    pub fallback_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartitionResult {
    pub run_id: String,
    pub node_to_community: Vec<usize>,
    /// Hierarchical community assignments by level (all vectors are node-indexed).
    /// Level 0 is the finest partition after the first movement pass;
    /// subsequent levels are progressively coarser communities of communities.
    pub hierarchy_levels: Vec<Vec<usize>>,
    pub community_count: usize,
    pub quality_score: f64,
    pub iteration_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValidationReport {
    pub run_id: String,
    pub hard_invariants_passed: bool,
    pub deterministic_identity_passed: Option<bool>,
    pub quality_delta_vs_reference: Option<f64>,
    pub equivalence_passed: bool,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunOutcome {
    pub execution: RunExecution,
    pub partition: Option<PartitionResult>,
    pub validation: Option<ValidationReport>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphInput {
    pub dataset_id: String,
    pub node_count: usize,
    pub edges: Vec<(usize, usize, Option<f64>)>,
}

impl GraphInput {
    pub fn empty(dataset_id: impl Into<String>) -> Self {
        Self {
            dataset_id: dataset_id.into(),
            node_count: 0,
            edges: Vec::new(),
        }
    }
}

// --- Benchmark & Profiling types (feature: profiling) ---

/// Execution phase during benchmark
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, PartialEq)]
pub enum BenchmarkPhase {
    Loading,
    InitialClustering,
    IncrementalBatch,
    Charting,
    Complete,
}

#[cfg(feature = "profiling")]
impl std::fmt::Display for BenchmarkPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchmarkPhase::Loading => write!(f, "Loading"),
            BenchmarkPhase::InitialClustering => write!(f, "InitialClustering"),
            BenchmarkPhase::IncrementalBatch => write!(f, "IncrementalBatch"),
            BenchmarkPhase::Charting => write!(f, "Charting"),
            BenchmarkPhase::Complete => write!(f, "Complete"),
        }
    }
}

/// A discrete status update emitted during benchmark execution
#[cfg(feature = "profiling")]
#[derive(Clone, Debug)]
pub struct ProgressEvent {
    pub phase: BenchmarkPhase,
    pub batch_index: Option<usize>,
    pub batch_total: Option<usize>,
    pub elapsed: std::time::Duration,
    pub metric_label: Option<String>,
    pub metric_value: Option<f64>,
}

/// Metadata and results for a complete benchmark run
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct BenchmarkRun {
    pub timestamp: String,
    pub dataset_id: String,
    pub timeout_seconds: Option<u64>,
    pub truncated: bool,
    pub batches: Vec<BatchResult>,
    pub total_time_seconds: f64,
    pub avg_speedup: f64,
    pub cumulative_speedup: f64,
    pub final_batch_comparison: Option<FinalBatchComparison>,
}

/// Final-batch apples-to-apples comparison between HIT-Leiden and fresh igraph Leiden
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct FinalBatchComparison {
    pub batch_idx: usize,
    pub total_edges: usize,
    pub nodes_in_graph: usize,
    pub hit_time_ms: f64,
    pub igraph_fresh_time_ms: f64,
    pub speedup_vs_fresh_igraph: f64,
    pub hit_modularity: f64,
    pub igraph_fresh_modularity: f64,
    pub modularity_delta: f64,
    pub hit_community_count: usize,
    pub igraph_community_count: usize,
    pub hit_largest_community_size: usize,
    pub igraph_largest_community_size: usize,
    pub hit_largest_community_share: f64,
    pub igraph_largest_community_share: f64,
    pub largest_community_jaccard: f64,
    pub nmi: f64,
    pub hit_to_igraph_purity: f64,
    pub igraph_to_hit_purity: f64,
}

/// Which external profiler was used
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Profiler {
    Perf,
    Samply,
}

#[cfg(feature = "profiling")]
impl std::fmt::Display for Profiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Profiler::Perf => write!(f, "perf"),
            Profiler::Samply => write!(f, "samply"),
        }
    }
}

/// A single function's contribution to profiling samples
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct Hotspot {
    pub function_name: String,
    pub file_path: Option<String>,
    pub line_number: Option<u32>,
    pub sample_count: u64,
    pub percentage: f64,
    pub callers: Vec<String>,
}

/// Represents a single profiling session with all output files
#[cfg(feature = "profiling")]
#[derive(Clone, Debug)]
pub struct ProfilingCapture {
    pub timestamp: String,
    pub binary_name: String,
    pub profiler: Profiler,
    pub duration_seconds: f64,
    pub native_output_path: std::path::PathBuf,
    pub pprof_path: Option<std::path::PathBuf>,
    pub sample_count: u64,
    pub hotspots: Vec<Hotspot>,
}

/// Direction of a timing change between two profiling runs
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum ChangeDirection {
    Faster,
    Slower,
    Unchanged,
}

/// A single function's timing change between two runs
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct HotspotDiff {
    pub function_name: String,
    pub file_path: Option<String>,
    pub baseline_percentage: f64,
    pub candidate_percentage: f64,
    pub delta_percentage: f64,
    pub relative_change: f64,
    pub direction: ChangeDirection,
}

/// Side-by-side diff between two profiling captures
#[cfg(feature = "profiling")]
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProfilingComparison {
    pub diffs: Vec<HotspotDiff>,
    pub threshold_percent: f64,
}

/// Results from a single batch update
#[cfg_attr(feature = "profiling", derive(serde::Serialize))]
#[derive(Clone, Debug)]
pub struct BatchResult {
    pub batch_idx: usize,
    pub edges_added: usize,
    pub total_edges: usize,
    pub nodes_in_graph: usize,
    pub hit_leiden_time_ms: f64,
    /// igraph (C library) Leiden timing in milliseconds (0.0 if not run)
    pub igraph_time_ms: f64,
    /// Speedup of HIT-Leiden vs igraph: igraph_time / hit_time (0.0 if not run)
    pub speedup: f64,
    pub hit_leiden_iterations: usize,
    /// HIT-Leiden modularity
    pub modularity: f64,
    /// igraph Leiden modularity (0.0 if not run)
    pub igraph_modularity: f64,
    /// Community assignment per node from HIT-Leiden (last batch only, if available)
    #[cfg_attr(feature = "profiling", serde(skip))]
    pub hit_partition: Option<Vec<usize>>,
    /// Optional hierarchical partitions from HIT-Leiden (node-indexed levels).
    #[cfg_attr(feature = "profiling", serde(skip))]
    pub hit_hierarchy_levels: Option<Vec<Vec<usize>>>,
    /// Community assignment per node from igraph Leiden (last batch only, if available)
    #[cfg_attr(feature = "profiling", serde(skip))]
    pub igraph_partition: Option<Vec<usize>>,
}

/// Aggregate results across all incremental batches
#[derive(Clone, Debug)]
pub struct IncrementalOutcome {
    pub batches: Vec<BatchResult>,
    pub total_time_seconds: f64,
    pub avg_speedup: f64,
    pub cumulative_speedup: f64,
    pub truncated: bool,
    #[cfg(feature = "profiling")]
    pub final_batch_comparison: Option<FinalBatchComparison>,
}
