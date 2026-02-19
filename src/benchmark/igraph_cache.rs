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

//! Persistent disk cache for igraph Leiden baseline results.
//!
//! igraph Leiden is deterministic for a given graph (seed=42), so we run it
//! once and cache the result (timing, modularity, partition) to disk.
//! Subsequent benchmark runs load the cached result instead of re-running
//! the Python subprocess.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct CachedIgraphLeiden {
    pub dataset_id: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub time_ms: f64,
    pub modularity: f64,
    pub partition: Vec<usize>,
}

/// Derive the cache file path for a given dataset and edge count.
fn cache_path(output_dir: &Path, dataset_id: &str, edge_count: usize) -> PathBuf {
    output_dir.join(format!("igraph_cache_{}_{}.json", dataset_id, edge_count))
}

/// Try to load a cached igraph Leiden result from disk.
pub fn load(output_dir: &Path, dataset_id: &str, edge_count: usize) -> Option<CachedIgraphLeiden> {
    let path = cache_path(output_dir, dataset_id, edge_count);
    let data = fs::read_to_string(&path).ok()?;
    let cached: CachedIgraphLeiden = serde_json::from_str(&data).ok()?;

    // Validate that the cache matches what we expect
    if cached.dataset_id != dataset_id || cached.edge_count != edge_count {
        eprintln!(
            "igraph cache mismatch at {} (expected {}/{}, got {}/{}), re-running.",
            path.display(),
            dataset_id,
            edge_count,
            cached.dataset_id,
            cached.edge_count,
        );
        return None;
    }

    eprintln!(
        "Loaded cached igraph Leiden result from {} ({:.2}ms, Q={:.4}, {} communities)",
        path.display(),
        cached.time_ms,
        cached.modularity,
        {
            let mut seen = std::collections::HashSet::new();
            for &c in &cached.partition {
                seen.insert(c);
            }
            seen.len()
        }
    );
    Some(cached)
}

/// Save an igraph Leiden result to disk for future reuse.
pub fn save(
    output_dir: &Path,
    result: &CachedIgraphLeiden,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(output_dir)?;
    let path = cache_path(output_dir, &result.dataset_id, result.edge_count);
    let json = serde_json::to_string(result)?;
    fs::write(&path, json)?;
    eprintln!("Saved igraph cache to {}", path.display());
    Ok(())
}
