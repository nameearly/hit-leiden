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

use crate::core::config::{RunConfig, RunMode};
#[cfg(feature = "profiling")]
use crate::core::types::FinalBatchComparison;
use crate::core::types::{BatchResult, GraphInput, IncrementalOutcome};
#[cfg(feature = "profiling")]
use std::collections::HashMap;
use std::time::Instant;

#[cfg(feature = "profiling")]
use std::path::PathBuf;
#[cfg(feature = "profiling")]
use std::time::Duration;

#[cfg(feature = "profiling")]
use crate::benchmark::igraph_baseline::IgraphLeidenBaseline;
#[cfg(feature = "profiling")]
use crate::benchmark::igraph_cache as ig_disk_cache;
#[cfg(feature = "profiling")]
use crate::core::types::{BenchmarkPhase, ProgressEvent};

/// Configuration for an incremental benchmark run
#[cfg(feature = "profiling")]
pub struct IncrementalConfig {
    pub timeout: Option<Duration>,
    pub progress_callback: Option<Box<dyn FnMut(ProgressEvent)>>,
    /// Directory for persistent igraph cache files.
    /// When set, igraph results are saved to / loaded from disk.
    pub cache_dir: Option<PathBuf>,
    /// Execution mode: Deterministic (single-threaded) or Throughput (parallel)
    pub mode: RunMode,
    /// Skip igraph baseline comparison
    pub no_igraph: bool,
}

#[cfg(feature = "profiling")]
impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            timeout: None,
            progress_callback: None,
            cache_dir: None,
            mode: RunMode::Deterministic,
            no_igraph: false,
        }
    }
}

/// Run HIT-Leiden incrementally across batches and compare against igraph baseline
pub fn run_incremental(
    batches: Vec<GraphInput>,
    batch_size: usize,
    initial_edge_count: usize,
) -> Result<IncrementalOutcome, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    let overall_start = Instant::now();
    let mut prev_total_edges = initial_edge_count;

    #[cfg(feature = "profiling")]
    let mut igraph_result: Option<IgResult> = None;

    for (idx, batch_graph) in batches.iter().enumerate() {
        let batch_result = run_single_batch(
            batch_graph,
            idx,
            batch_size,
            &mut prev_total_edges,
            #[cfg(feature = "profiling")]
            &mut igraph_result,
            RunMode::Deterministic,
        )?;

        eprintln!(
            "Batch {}: +{} edges | Total: {:.0} | HIT: {:.2}ms | igraph: {:.2}ms | Speedup: {:.2}x",
            idx,
            batch_result.edges_added,
            batch_result.total_edges,
            batch_result.hit_leiden_time_ms,
            batch_result.igraph_time_ms,
            batch_result.speedup
        );

        results.push(batch_result);
    }

    #[cfg(feature = "profiling")]
    {
        Ok(build_outcome(results, overall_start, false, None))
    }

    #[cfg(not(feature = "profiling"))]
    {
        Ok(build_outcome(results, overall_start, false))
    }
}

/// Run incremental benchmark with progress callbacks and timeout support
#[cfg(feature = "profiling")]
pub fn run_incremental_with_config(
    batches: Vec<GraphInput>,
    batch_size: usize,
    initial_edge_count: usize,
    mut config: IncrementalConfig,
) -> Result<IncrementalOutcome, Box<dyn std::error::Error>> {
    let mut results = Vec::new();
    let overall_start = Instant::now();
    let mut prev_total_edges = initial_edge_count;
    let batch_total = batches.len();
    let mut truncated = false;

    let mode = config.mode;
    let mut callback = config.progress_callback.take();
    let emit = |cb: &mut Option<Box<dyn FnMut(ProgressEvent)>>, event: ProgressEvent| {
        if let Some(ref mut f) = cb {
            f(event);
        }
    };

    // igraph availability check and disk cache loading
    let igraph_available = if config.no_igraph {
        false
    } else {
        let available = IgraphLeidenBaseline::is_available();
        if !available {
            eprintln!(
                "Warning: python-igraph/leidenalg not found. \
                 Run 'cargo make install-igraph' to install. igraph comparison will be skipped."
            );
        }
        available
    };

    let mut igraph_result: Option<IgResult> =
        load_cached_igraph_result(igraph_available, &config, &batches);

    // Compute igraph upfront on first batch if available but not cached
    if igraph_available && igraph_result.is_none() {
        if let Some(first_batch) = batches.first() {
            eprintln!("[pre-run] Running igraph Leiden baseline (C via Python)...");
            match IgraphLeidenBaseline::run(&first_batch.edges, first_batch.node_count) {
                Ok(result) => {
                    eprintln!(
                        "[pre-run] igraph completed in {:.2}ms (modularity: {:.4})",
                        result.time_ms, result.modularity
                    );
                    igraph_result = Some(IgResult {
                        time_ms: result.time_ms,
                        modularity: result.modularity,
                        partition: result.partition,
                    });
                }
                Err(e) => {
                    eprintln!("[pre-run] igraph failed: {} (skipping)", e);
                }
            }
        }
    }

    for (idx, batch_graph) in batches.iter().enumerate() {
        // Check timeout before each batch
        if let Some(timeout_dur) = config.timeout {
            if overall_start.elapsed() >= timeout_dur {
                truncated = true;
                eprintln!(
                    "Warning: Timeout ({}s) exceeded after batch {}/{}. Partial results written.",
                    timeout_dur.as_secs(),
                    idx,
                    batch_total
                );
                emit(
                    &mut callback,
                    ProgressEvent {
                        phase: BenchmarkPhase::Complete,
                        batch_index: Some(idx),
                        batch_total: Some(batch_total),
                        elapsed: overall_start.elapsed(),
                        metric_label: Some("truncated".to_string()),
                        metric_value: Some(1.0),
                    },
                );
                break;
            }
        }

        // Emit progress event for batch start
        emit(
            &mut callback,
            ProgressEvent {
                phase: BenchmarkPhase::IncrementalBatch,
                batch_index: Some(idx + 1),
                batch_total: Some(batch_total),
                elapsed: overall_start.elapsed(),
                metric_label: None,
                metric_value: None,
            },
        );

        let batch_result = run_single_batch(
            batch_graph,
            idx,
            batch_size,
            &mut prev_total_edges,
            &mut igraph_result,
            mode,
        )?;

        // Save igraph to disk after first computation
        if let (Some(ref ig), Some(ref cache_dir)) = (&igraph_result, &config.cache_dir) {
            if idx == 0 && igraph_available {
                let _ = ig_disk_cache::save(
                    cache_dir,
                    &ig_disk_cache::CachedIgraphLeiden {
                        dataset_id: batch_graph.dataset_id.clone(),
                        node_count: batch_graph.node_count,
                        edge_count: batch_graph.edges.len(),
                        time_ms: ig.time_ms,
                        modularity: ig.modularity,
                        partition: ig.partition.clone(),
                    },
                );
            }
        }

        // Emit progress with speedup metric
        emit(
            &mut callback,
            ProgressEvent {
                phase: BenchmarkPhase::IncrementalBatch,
                batch_index: Some(idx + 1),
                batch_total: Some(batch_total),
                elapsed: overall_start.elapsed(),
                metric_label: Some("speedup".to_string()),
                metric_value: Some(batch_result.speedup),
            },
        );

        eprintln!(
            "Batch {}: +{} edges | Total: {:.0} | HIT: {:.2}ms | igraph: {:.2}ms | Speedup: {:.2}x",
            idx,
            batch_result.edges_added,
            batch_result.total_edges,
            batch_result.hit_leiden_time_ms,
            batch_result.igraph_time_ms,
            batch_result.speedup
        );

        results.push(batch_result);
    }

    #[cfg(feature = "profiling")]
    let final_batch_comparison = if config.no_igraph {
        None
    } else {
        compute_final_batch_comparison(&mut results, &batches)?
    };

    Ok(build_outcome(
        results,
        overall_start,
        truncated,
        final_batch_comparison,
    ))
}

/// In-memory igraph Leiden result reused across batches within a single run
#[cfg(feature = "profiling")]
struct IgResult {
    time_ms: f64,
    modularity: f64,
    partition: Vec<usize>,
}

#[cfg(feature = "profiling")]
fn load_cached_igraph_result(
    igraph_available: bool,
    config: &IncrementalConfig,
    batches: &[GraphInput],
) -> Option<IgResult> {
    if !igraph_available {
        return None;
    }
    let cache_dir = config.cache_dir.as_ref()?;
    let first_batch = batches.first()?;
    let cached = ig_disk_cache::load(cache_dir, &first_batch.dataset_id, first_batch.edges.len())?;
    Some(IgResult {
        time_ms: cached.time_ms,
        modularity: cached.modularity,
        partition: cached.partition,
    })
}

/// Execute a single batch: run HIT-Leiden and igraph baseline, return BatchResult
fn run_single_batch(
    batch_graph: &GraphInput,
    idx: usize,
    batch_size: usize,
    prev_total_edges: &mut usize,
    #[cfg(feature = "profiling")] igraph_result: &mut Option<IgResult>,
    mode: RunMode,
) -> Result<BatchResult, Box<dyn std::error::Error>> {
    // Run HIT-Leiden
    let config = RunConfig {
        mode,
        ..Default::default()
    };

    eprintln!("[batch {}] Running HIT-Leiden ({:?} mode)...", idx, mode);
    let start = Instant::now();
    let outcome = crate::run(batch_graph, &config)?;
    let hit_leiden_ms = start.elapsed().as_secs_f64() * 1000.0;

    let hit_leiden_iterations = outcome
        .partition
        .as_ref()
        .map(|p| p.iteration_count)
        .unwrap_or(0);

    let modularity = outcome
        .partition
        .as_ref()
        .map(|p| p.quality_score)
        .unwrap_or(0.0);

    let hit_partition = outcome
        .partition
        .as_ref()
        .map(|p| p.node_to_community.clone());

    let hit_hierarchy_levels = outcome
        .partition
        .as_ref()
        .map(|p| p.hierarchy_levels.clone());

    eprintln!(
        "[batch {}] HIT-Leiden completed in {:.2}ms (modularity: {:.4})",
        idx, hit_leiden_ms, modularity
    );

    // igraph Leiden baseline — reuse from memory/disk if available, otherwise compute once
    // Skipped entirely when igraph_result is None (no_igraph or unavailable).
    #[cfg(feature = "profiling")]
    let (igraph_ms, igraph_mod, igraph_part) = {
        if let Some(ref cached) = igraph_result {
            eprintln!(
                "[batch {}] Using cached igraph result ({:.2}ms, Q={:.4})",
                idx, cached.time_ms, cached.modularity
            );
            (
                cached.time_ms,
                cached.modularity,
                Some(cached.partition.clone()),
            )
        } else {
            (0.0, 0.0, None)
        }
    };

    #[cfg(not(feature = "profiling"))]
    let (igraph_ms, igraph_mod, igraph_part) = (0.0, 0.0, None::<Vec<usize>>);

    let speedup = if igraph_ms > 0.0 && hit_leiden_ms > 0.0 {
        igraph_ms / hit_leiden_ms
    } else {
        0.0
    };

    let current_total = batch_graph.edges.len();
    let edges_added = current_total.saturating_sub(*prev_total_edges);
    *prev_total_edges = current_total;

    Ok(BatchResult {
        batch_idx: idx,
        edges_added: if edges_added == 0 {
            batch_size.min(current_total)
        } else {
            edges_added
        },
        total_edges: current_total,
        nodes_in_graph: batch_graph.node_count,
        hit_leiden_time_ms: hit_leiden_ms,
        igraph_time_ms: igraph_ms,
        speedup,
        hit_leiden_iterations,
        modularity,
        igraph_modularity: igraph_mod,
        hit_partition,
        hit_hierarchy_levels,
        igraph_partition: igraph_part,
    })
}

#[cfg(not(feature = "profiling"))]
fn build_outcome(
    results: Vec<BatchResult>,
    overall_start: Instant,
    truncated: bool,
) -> IncrementalOutcome {
    let total_seconds = overall_start.elapsed().as_secs_f64();
    let hit_total: f64 = results.iter().map(|r| r.hit_leiden_time_ms).sum();
    let igraph_total: f64 = results.iter().map(|r| r.igraph_time_ms).sum();

    let cumulative_speedup = if hit_total > 0.0 && igraph_total > 0.0 {
        igraph_total / hit_total
    } else {
        0.0
    };

    let avg_speedup = if !results.is_empty() {
        results.iter().map(|r| r.speedup).sum::<f64>() / results.len() as f64
    } else {
        0.0
    };

    IncrementalOutcome {
        batches: results,
        total_time_seconds: total_seconds,
        avg_speedup,
        cumulative_speedup,
        truncated,
    }
}

#[cfg(feature = "profiling")]
fn build_outcome(
    results: Vec<BatchResult>,
    overall_start: Instant,
    truncated: bool,
    final_batch_comparison: Option<FinalBatchComparison>,
) -> IncrementalOutcome {
    let total_seconds = overall_start.elapsed().as_secs_f64();
    let hit_total: f64 = results.iter().map(|r| r.hit_leiden_time_ms).sum();
    let igraph_total: f64 = results.iter().map(|r| r.igraph_time_ms).sum();

    let cumulative_speedup = if hit_total > 0.0 && igraph_total > 0.0 {
        igraph_total / hit_total
    } else {
        0.0
    };

    let avg_speedup = if !results.is_empty() {
        results.iter().map(|r| r.speedup).sum::<f64>() / results.len() as f64
    } else {
        0.0
    };

    IncrementalOutcome {
        batches: results,
        total_time_seconds: total_seconds,
        avg_speedup,
        cumulative_speedup,
        truncated,
        final_batch_comparison,
    }
}

#[cfg(feature = "profiling")]
fn compute_final_batch_comparison(
    results: &mut [BatchResult],
    batches: &[GraphInput],
) -> Result<Option<FinalBatchComparison>, Box<dyn std::error::Error>> {
    let Some(last_batch) = results.last_mut() else {
        return Ok(None);
    };
    let Some(hit_partition) = last_batch.hit_partition.clone() else {
        return Ok(None);
    };
    let Some(final_graph) = batches.get(last_batch.batch_idx) else {
        return Ok(None);
    };

    // Fresh igraph run on final batch for apples-to-apples comparison
    if !IgraphLeidenBaseline::is_available() {
        eprintln!("[final batch] igraph not available, skipping final comparison.");
        return Ok(None);
    }

    eprintln!("[final batch] Running fresh igraph Leiden for apples-to-apples comparison...");
    let igraph_result = match IgraphLeidenBaseline::run(&final_graph.edges, final_graph.node_count)
    {
        Ok(result) => {
            last_batch.igraph_partition = Some(result.partition.clone());
            last_batch.igraph_time_ms = result.time_ms;
            last_batch.igraph_modularity = result.modularity;
            result
        }
        Err(e) => {
            eprintln!("[final batch] igraph fresh run failed: {}", e);
            return Ok(None);
        }
    };

    let ig_partition = &igraph_result.partition;
    let n = hit_partition.len().min(ig_partition.len());
    if n == 0 {
        return Ok(None);
    }

    let hit_sizes = community_sizes(&hit_partition[..n]);
    let ig_sizes = community_sizes(&ig_partition[..n]);

    let hit_largest = hit_sizes.values().copied().max().unwrap_or(0);
    let ig_largest = ig_sizes.values().copied().max().unwrap_or(0);

    let hit_largest_id = hit_sizes
        .iter()
        .max_by(|(ida, sa), (idb, sb)| sa.cmp(sb).then_with(|| idb.cmp(ida)))
        .map(|(id, _)| *id)
        .unwrap_or(0);
    let ig_largest_id = ig_sizes
        .iter()
        .max_by(|(ida, sa), (idb, sb)| sa.cmp(sb).then_with(|| idb.cmp(ida)))
        .map(|(id, _)| *id)
        .unwrap_or(0);

    let (nmi, hit_to_ig_purity, ig_to_hit_purity, largest_jaccard) = contingency_metrics(
        &hit_partition[..n],
        &ig_partition[..n],
        hit_largest_id,
        ig_largest_id,
    );

    let speedup_vs_fresh_igraph = if last_batch.hit_leiden_time_ms > 0.0 {
        igraph_result.time_ms / last_batch.hit_leiden_time_ms
    } else {
        0.0
    };

    let cmp = FinalBatchComparison {
        batch_idx: last_batch.batch_idx,
        total_edges: last_batch.total_edges,
        nodes_in_graph: n,
        hit_time_ms: last_batch.hit_leiden_time_ms,
        igraph_fresh_time_ms: igraph_result.time_ms,
        speedup_vs_fresh_igraph,
        hit_modularity: last_batch.modularity,
        igraph_fresh_modularity: igraph_result.modularity,
        modularity_delta: last_batch.modularity - igraph_result.modularity,
        hit_community_count: hit_sizes.len(),
        igraph_community_count: ig_sizes.len(),
        hit_largest_community_size: hit_largest,
        igraph_largest_community_size: ig_largest,
        hit_largest_community_share: hit_largest as f64 / n as f64,
        igraph_largest_community_share: ig_largest as f64 / n as f64,
        largest_community_jaccard: largest_jaccard,
        nmi,
        hit_to_igraph_purity: hit_to_ig_purity,
        igraph_to_hit_purity: ig_to_hit_purity,
    };

    Ok(Some(cmp))
}

#[cfg(feature = "profiling")]
fn community_sizes(partition: &[usize]) -> HashMap<usize, usize> {
    let mut sizes: HashMap<usize, usize> = HashMap::new();
    for &c in partition {
        *sizes.entry(c).or_default() += 1;
    }
    sizes
}

#[cfg(feature = "profiling")]
fn contingency_metrics(
    hit_partition: &[usize],
    ig_partition: &[usize],
    hit_largest_id: usize,
    ig_largest_id: usize,
) -> (f64, f64, f64, f64) {
    let n = hit_partition.len().min(ig_partition.len());
    if n == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let mut hit_sizes: HashMap<usize, usize> = HashMap::new();
    let mut ig_sizes: HashMap<usize, usize> = HashMap::new();
    let mut contingency: HashMap<(usize, usize), usize> = HashMap::new();

    for i in 0..n {
        let h = hit_partition[i];
        let s = ig_partition[i];
        *hit_sizes.entry(h).or_default() += 1;
        *ig_sizes.entry(s).or_default() += 1;
        *contingency.entry((h, s)).or_default() += 1;
    }

    let n_f = n as f64;
    let h_hit = entropy(&hit_sizes, n_f);
    let h_ig = entropy(&ig_sizes, n_f);

    let mut mi = 0.0f64;
    for (&(h, s), &count_hs) in &contingency {
        let p_hs = count_hs as f64 / n_f;
        let p_h = *hit_sizes.get(&h).unwrap_or(&0) as f64 / n_f;
        let p_s = *ig_sizes.get(&s).unwrap_or(&0) as f64 / n_f;
        if p_hs > 0.0 && p_h > 0.0 && p_s > 0.0 {
            mi += p_hs * (p_hs / (p_h * p_s)).ln();
        }
    }

    let nmi = if h_hit > 0.0 && h_ig > 0.0 {
        mi / (h_hit * h_ig).sqrt()
    } else {
        0.0
    };

    let mut best_by_hit: HashMap<usize, usize> = HashMap::new();
    let mut best_by_ig: HashMap<usize, usize> = HashMap::new();
    for (&(h, s), &count_hs) in &contingency {
        best_by_hit
            .entry(h)
            .and_modify(|v| *v = (*v).max(count_hs))
            .or_insert(count_hs);
        best_by_ig
            .entry(s)
            .and_modify(|v| *v = (*v).max(count_hs))
            .or_insert(count_hs);
    }

    let hit_to_ig_purity = best_by_hit.values().sum::<usize>() as f64 / n_f;
    let ig_to_hit_purity = best_by_ig.values().sum::<usize>() as f64 / n_f;

    let inter = contingency
        .get(&(hit_largest_id, ig_largest_id))
        .copied()
        .unwrap_or(0);
    let hit_largest = hit_sizes.get(&hit_largest_id).copied().unwrap_or(0);
    let ig_largest = ig_sizes.get(&ig_largest_id).copied().unwrap_or(0);
    let union = hit_largest + ig_largest - inter;
    let largest_jaccard = if union > 0 {
        inter as f64 / union as f64
    } else {
        0.0
    };

    (nmi, hit_to_ig_purity, ig_to_hit_purity, largest_jaccard)
}

#[cfg(feature = "profiling")]
fn entropy(counts: &HashMap<usize, usize>, total: f64) -> f64 {
    counts
        .values()
        .filter_map(|&count| {
            let p = count as f64 / total;
            if p > 0.0 {
                Some(-p * p.ln())
            } else {
                None
            }
        })
        .sum()
}
