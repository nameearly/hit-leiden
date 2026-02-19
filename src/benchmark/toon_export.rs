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

use crate::benchmark::profiling::load_hotspots_auto;
use crate::core::types::{ChangeDirection, Hotspot, HotspotDiff, ProfilingComparison};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Export a single profiling run as TOON text
pub fn export_toon(
    input_path: &Path,
    top_n: usize,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (hotspots, total_samples) = load_hotspots_auto(input_path, top_n)?;

    let toon_output = format_hotspots_toon(&hotspots, total_samples);

    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out_path, &toon_output)?;
        eprintln!("TOON written to {}", out_path.display());
    } else {
        print!("{}", toon_output);
    }

    Ok(())
}

/// Compare two profiling runs and output TOON diff
pub fn compare_toon(
    baseline_path: &Path,
    candidate_path: &Path,
    threshold: f64,
    top_n: usize,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (baseline_hotspots, _) = load_hotspots_auto(baseline_path, top_n * 2)?;
    let (candidate_hotspots, _) = load_hotspots_auto(candidate_path, top_n * 2)?;

    let comparison = compute_comparison(&baseline_hotspots, &candidate_hotspots, threshold, top_n);
    let toon_output = format_comparison_toon(&comparison);

    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out_path, &toon_output)?;
        eprintln!("TOON comparison written to {}", out_path.display());
    } else {
        print!("{}", toon_output);
    }

    Ok(())
}

/// Generate hotspots.toon from a Vec<Hotspot> directly (used during profiling)
pub fn write_hotspots_toon(
    hotspots: &[Hotspot],
    total_samples: u64,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let toon_output = format_hotspots_toon(hotspots, total_samples);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, toon_output)?;
    Ok(())
}

/// Format hotspots in TOON v3.0 tabular format
fn format_hotspots_toon(hotspots: &[Hotspot], total_samples: u64) -> String {
    // TOON tabular format: header line with field names, then data rows
    let mut output = String::new();
    output.push_str(&format!(
        "# Profiling Hotspots ({} total samples)\n",
        total_samples
    ));
    output.push_str(&format!(
        "hotspots[{}]{{function_name,percentage,sample_count,file_path}}:\n",
        hotspots.len()
    ));

    for h in hotspots {
        let file_path = h.file_path.as_deref().unwrap_or("");
        output.push_str(&format!(
            "  {},{:.1},{},{}\n",
            h.function_name, h.percentage, h.sample_count, file_path
        ));
    }

    output
}

/// Compute a ProfilingComparison between baseline and candidate hotspots
fn compute_comparison(
    baseline: &[Hotspot],
    candidate: &[Hotspot],
    threshold: f64,
    top_n: usize,
) -> ProfilingComparison {
    // Build maps of function_name -> percentage
    let baseline_map: HashMap<&str, f64> = baseline
        .iter()
        .map(|h| (h.function_name.as_str(), h.percentage))
        .collect();

    let candidate_map: HashMap<&str, f64> = candidate
        .iter()
        .map(|h| (h.function_name.as_str(), h.percentage))
        .collect();

    // Collect all function names
    let mut all_functions: Vec<&str> = baseline_map
        .keys()
        .chain(candidate_map.keys())
        .copied()
        .collect();
    all_functions.sort();
    all_functions.dedup();

    let mut diffs: Vec<HotspotDiff> = Vec::new();

    for func_name in all_functions {
        let base_pct = baseline_map.get(func_name).copied().unwrap_or(0.0);
        let cand_pct = candidate_map.get(func_name).copied().unwrap_or(0.0);
        let delta = cand_pct - base_pct;
        let relative = if base_pct > 0.0 {
            (delta / base_pct) * 100.0
        } else if cand_pct > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        if relative.abs() < threshold && !relative.is_infinite() {
            continue;
        }

        let direction = if delta < -0.01 {
            ChangeDirection::Faster
        } else if delta > 0.01 {
            ChangeDirection::Slower
        } else {
            ChangeDirection::Unchanged
        };

        diffs.push(HotspotDiff {
            function_name: func_name.to_string(),
            file_path: None,
            baseline_percentage: base_pct,
            candidate_percentage: cand_pct,
            delta_percentage: delta,
            relative_change: relative,
            direction,
        });
    }

    // Sort by absolute delta, descending
    diffs.sort_by(|a, b| {
        b.delta_percentage
            .abs()
            .partial_cmp(&a.delta_percentage.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    diffs.truncate(top_n);

    ProfilingComparison {
        diffs,
        threshold_percent: threshold,
    }
}

/// Format a ProfilingComparison in TOON v3.0 tabular format
fn format_comparison_toon(comparison: &ProfilingComparison) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "# Profiling Comparison (threshold: {:.1}%)\n",
        comparison.threshold_percent
    ));
    output.push_str(&format!(
        "diffs[{}]{{function_name,direction,baseline_pct,candidate_pct,delta_pct,relative_change_pct}}:\n",
        comparison.diffs.len()
    ));

    for d in &comparison.diffs {
        let direction_str = match d.direction {
            ChangeDirection::Faster => "FASTER",
            ChangeDirection::Slower => "SLOWER",
            ChangeDirection::Unchanged => "UNCHANGED",
        };
        output.push_str(&format!(
            "  {},{},{:.1},{:.1},{:+.1},{:+.1}\n",
            d.function_name,
            direction_str,
            d.baseline_percentage,
            d.candidate_percentage,
            d.delta_percentage,
            d.relative_change,
        ));
    }

    output
}
