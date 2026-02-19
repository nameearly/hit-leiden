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

use crate::core::types::{Hotspot, Profiler, ProfilingCapture};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the complete profiling pipeline: record with external profiler, post-process, extract hotspots
pub fn run_profiling(
    binary_name: &str,
    profiler: &Profiler,
    duration_secs: u64,
    frequency: u32,
    output_dir: &Path,
    top_n: usize,
) -> Result<ProfilingCapture, Box<dyn std::error::Error>> {
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let session_dir = output_dir.join(format!("profile_{}_{}", binary_name, timestamp));
    fs::create_dir_all(&session_dir)?;

    // Resolve binary path
    let binary_path = resolve_binary(binary_name)?;

    eprintln!(
        "Profiling {} with {} for {}s...",
        binary_name, profiler, duration_secs
    );

    let capture = match profiler {
        Profiler::Perf => run_perf_pipeline(
            &binary_path,
            binary_name,
            duration_secs,
            frequency,
            &session_dir,
            top_n,
            &timestamp,
        )?,
        Profiler::Samply => run_samply_pipeline(
            &binary_path,
            binary_name,
            duration_secs,
            &session_dir,
            top_n,
            &timestamp,
        )?,
    };

    Ok(capture)
}

/// Resolve binary name to target/release/{binary} path, building if necessary
fn resolve_binary(binary_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let binary_path = PathBuf::from(format!("target/release/{}", binary_name));

    if !binary_path.exists() {
        eprintln!("Building {} in release mode...", binary_name);
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--features",
                "profiling",
                "--bin",
                binary_name,
            ])
            .status()?;

        if !status.success() {
            return Err(format!("Failed to build binary '{}'", binary_name).into());
        }
    }

    if !binary_path.exists() {
        return Err(format!(
            "Binary not found at {}. Build may have failed.",
            binary_path.display()
        )
        .into());
    }

    Ok(binary_path)
}

// --- Perf Pipeline ---

fn run_perf_pipeline(
    binary_path: &Path,
    binary_name: &str,
    duration_secs: u64,
    frequency: u32,
    session_dir: &Path,
    top_n: usize,
    timestamp: &str,
) -> Result<ProfilingCapture, Box<dyn std::error::Error>> {
    let perf_data = session_dir.join("perf.data");

    // T019: Invoke perf record
    eprintln!("[profiling] Recording with perf...");
    check_tool_exists(
        "perf",
        "Error: 'perf' not found. Install linux-tools-common or equivalent.",
    )?;

    let status = Command::new("perf")
        .args([
            "record",
            "--call-graph",
            "dwarf",
            "-F",
            &frequency.to_string(),
            "-o",
            &perf_data.to_string_lossy(),
            "--",
            &binary_path.to_string_lossy(),
        ])
        .env("PERF_DURATION", duration_secs.to_string())
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                "Error: perf_event_open failed. Try: sudo sysctl kernel.perf_event_paranoid=1"
                    .to_string()
            } else {
                format!("Error running perf: {}", e)
            }
        })?;

    if !status.success() {
        return Err("Error: perf record failed. Check perf_event_paranoid setting.".into());
    }

    // T020: Run perf script and collapse with inferno
    eprintln!("[profiling] Running perf script...");
    let perf_script_output = Command::new("perf")
        .args(["script", "-i", &perf_data.to_string_lossy()])
        .output()?;

    if !perf_script_output.status.success() {
        return Err("Error: perf script failed".into());
    }

    eprintln!("[profiling] Collapsing stacks with inferno...");
    let folded = collapse_perf_stacks(&perf_script_output.stdout)?;

    // T023: Extract hotspots from folded stacks
    let (hotspots, total_samples) = extract_hotspots_from_folded(&folded, top_n);

    if total_samples < 10 {
        return Err(format!(
            "Error: Only {} samples collected. Increase --duration or --frequency.",
            total_samples
        )
        .into());
    }

    // T021: Convert folded stacks to pprof
    eprintln!("[profiling] Generating pprof protobuf...");
    let pprof_path = session_dir.join("profile.pb.gz");
    write_pprof(&folded, &pprof_path, duration_secs, frequency)?;
    eprintln!("[profiling] pprof written to {}", pprof_path.display());

    // Print hotspot summary
    print_hotspot_table(&hotspots, total_samples);

    // Auto-generate hotspots.toon for AI consumption
    let toon_path = session_dir.join("hotspots.toon");
    if let Err(e) =
        crate::benchmark::toon_export::write_hotspots_toon(&hotspots, total_samples, &toon_path)
    {
        eprintln!("[profiling] Warning: Failed to write hotspots.toon: {}", e);
    } else {
        eprintln!("[profiling] TOON written to {}", toon_path.display());
    }

    Ok(ProfilingCapture {
        timestamp: timestamp.to_string(),
        binary_name: binary_name.to_string(),
        profiler: Profiler::Perf,
        duration_seconds: duration_secs as f64,
        native_output_path: perf_data,
        pprof_path: Some(pprof_path),
        sample_count: total_samples,
        hotspots,
    })
}

/// Collapse perf script output to folded stacks using inferno
fn collapse_perf_stacks(
    perf_script: &[u8],
) -> Result<Vec<(String, u64)>, Box<dyn std::error::Error>> {
    use inferno::collapse::perf::Folder;
    use inferno::collapse::Collapse;

    let mut folder = Folder::default();
    let mut folded_output = Vec::new();
    folder.collapse(perf_script, &mut folded_output)?;

    let folded_str = String::from_utf8(folded_output)?;
    let mut stacks = Vec::new();

    for line in folded_str.lines() {
        if let Some((stack, count_str)) = line.rsplit_once(' ') {
            if let Ok(count) = count_str.parse::<u64>() {
                stacks.push((stack.to_string(), count));
            }
        }
    }

    Ok(stacks)
}

/// Extract top-N hotspots from folded stacks (by leaf frame = self time)
fn extract_hotspots_from_folded(folded: &[(String, u64)], top_n: usize) -> (Vec<Hotspot>, u64) {
    let mut function_samples: HashMap<String, u64> = HashMap::new();
    let mut total_samples: u64 = 0;

    for (stack, count) in folded {
        total_samples += count;
        // Leaf frame is the last element in the stack
        if let Some(leaf) = stack.rsplit(';').next() {
            *function_samples.entry(leaf.to_string()).or_insert(0) += count;
        }
    }

    let mut sorted: Vec<(String, u64)> = function_samples.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let hotspots: Vec<Hotspot> = sorted
        .into_iter()
        .take(top_n)
        .map(|(name, count)| {
            let percentage = if total_samples > 0 {
                (count as f64 / total_samples as f64) * 100.0
            } else {
                0.0
            };
            Hotspot {
                function_name: name,
                file_path: None,
                line_number: None,
                sample_count: count,
                percentage,
                callers: Vec::new(),
            }
        })
        .collect();

    (hotspots, total_samples)
}

/// Convert folded stacks to pprof protobuf format and write gzipped
fn write_pprof(
    folded: &[(String, u64)],
    output_path: &Path,
    duration_secs: u64,
    frequency: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use prost::Message;
    use std::io::Write;

    mod pprof_proto {
        include!(concat!(env!("OUT_DIR"), "/perftools.profiles.rs"));
    }

    // String interning helper
    struct StringTable {
        table: Vec<String>,
        map: HashMap<String, i64>,
    }

    impl StringTable {
        fn new() -> Self {
            let mut st = StringTable {
                table: vec!["".to_string()],
                map: HashMap::new(),
            };
            st.map.insert("".to_string(), 0);
            st
        }

        fn intern(&mut self, s: &str) -> i64 {
            if let Some(&idx) = self.map.get(s) {
                return idx;
            }
            let idx = self.table.len() as i64;
            self.table.push(s.to_string());
            self.map.insert(s.to_string(), idx);
            idx
        }

        fn into_table(self) -> Vec<String> {
            self.table
        }
    }

    let mut strings = StringTable::new();
    let samples_idx = strings.intern("samples");
    let count_idx = strings.intern("count");

    let mut functions: Vec<pprof_proto::Function> = Vec::new();
    let mut function_map: HashMap<String, u64> = HashMap::new();
    let mut locations: Vec<pprof_proto::Location> = Vec::new();
    let mut location_map: HashMap<String, u64> = HashMap::new();
    let mut samples: Vec<pprof_proto::Sample> = Vec::new();

    let mut next_func_id: u64 = 1;
    let mut next_loc_id: u64 = 1;

    for (stack, count) in folded {
        let frames: Vec<&str> = stack.split(';').collect();
        let mut location_ids = Vec::new();

        // pprof expects leaf-first order
        for frame in frames.iter().rev() {
            let func_id = if let Some(&id) = function_map.get(*frame) {
                id
            } else {
                let id = next_func_id;
                next_func_id += 1;
                let name_idx = strings.intern(frame);
                functions.push(pprof_proto::Function {
                    id,
                    name: name_idx,
                    system_name: name_idx,
                    filename: 0,
                    start_line: 0,
                });
                function_map.insert(frame.to_string(), id);
                id
            };

            let loc_id = if let Some(&id) = location_map.get(*frame) {
                id
            } else {
                let id = next_loc_id;
                next_loc_id += 1;
                locations.push(pprof_proto::Location {
                    id,
                    mapping_id: 0,
                    address: 0,
                    line: vec![pprof_proto::Line {
                        function_id: func_id,
                        line: 0,
                    }],
                    is_folded: false,
                });
                location_map.insert(frame.to_string(), id);
                id
            };

            location_ids.push(loc_id);
        }

        samples.push(pprof_proto::Sample {
            location_id: location_ids,
            value: vec![*count as i64],
            label: vec![],
        });
    }

    let cpu_idx = strings.intern("cpu");
    let ns_idx = strings.intern("nanoseconds");

    let profile = pprof_proto::Profile {
        sample_type: vec![pprof_proto::ValueType {
            r#type: samples_idx,
            unit: count_idx,
        }],
        sample: samples,
        mapping: vec![],
        location: locations,
        function: functions,
        string_table: strings.into_table(),
        drop_frames: 0,
        keep_frames: 0,
        time_nanos: 0,
        duration_nanos: (duration_secs * 1_000_000_000) as i64,
        period_type: Some(pprof_proto::ValueType {
            r#type: cpu_idx,
            unit: ns_idx,
        }),
        period: (1_000_000_000 / frequency as i64),
        comment: vec![],
        default_sample_type: 0,
    };

    let mut buf = Vec::new();
    profile.encode(&mut buf)?;

    let file = fs::File::create(output_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&buf)?;
    encoder.finish()?;

    Ok(())
}

// --- Samply Pipeline ---

fn run_samply_pipeline(
    binary_path: &Path,
    binary_name: &str,
    duration_secs: u64,
    session_dir: &Path,
    top_n: usize,
    timestamp: &str,
) -> Result<ProfilingCapture, Box<dyn std::error::Error>> {
    let json_gz_path = session_dir.join("profile.json.gz");

    // T022: Invoke samply record
    eprintln!("[profiling] Recording with samply...");
    check_tool_exists(
        "samply",
        "Error: 'samply' not found. Install via 'cargo install samply'.",
    )?;

    let status = Command::new("samply")
        .args([
            "record",
            "--save-only",
            "--unstable-presymbolicate",
            "-o",
            &json_gz_path.to_string_lossy(),
            "--",
            &binary_path.to_string_lossy(),
        ])
        .status()?;

    if !status.success() {
        return Err("Error: samply record failed".into());
    }

    // T022a: Parse samply JSON output
    eprintln!("[profiling] Parsing samply JSON output...");
    let (hotspots, total_samples) = parse_samply_json(&json_gz_path, top_n)?;

    if total_samples < 10 {
        return Err(format!(
            "Error: Only {} samples collected. Increase --duration or --frequency.",
            total_samples
        )
        .into());
    }

    // Print hotspot summary
    print_hotspot_table(&hotspots, total_samples);

    // Auto-generate hotspots.toon for AI consumption
    let toon_path = session_dir.join("hotspots.toon");
    if let Err(e) =
        crate::benchmark::toon_export::write_hotspots_toon(&hotspots, total_samples, &toon_path)
    {
        eprintln!("[profiling] Warning: Failed to write hotspots.toon: {}", e);
    } else {
        eprintln!("[profiling] TOON written to {}", toon_path.display());
    }

    Ok(ProfilingCapture {
        timestamp: timestamp.to_string(),
        binary_name: binary_name.to_string(),
        profiler: Profiler::Samply,
        duration_seconds: duration_secs as f64,
        native_output_path: json_gz_path,
        pprof_path: None,
        sample_count: total_samples,
        hotspots,
    })
}

/// Parse Firefox Profiler JSON format from samply output
/// Walks: samples.stack → stackTable.frame → frameTable.func → funcTable.name → stringTable
pub fn parse_samply_json(
    path: &Path,
    top_n: usize,
) -> Result<(Vec<Hotspot>, u64), Box<dyn std::error::Error>> {
    let file = fs::File::open(path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut reader = std::io::BufReader::new(decoder);
    let mut json_str = String::new();
    reader.read_to_string(&mut json_str)?;

    let profile: serde_json::Value = serde_json::from_str(&json_str)?;

    // Navigate to first thread's data
    let threads = profile
        .get("threads")
        .and_then(|t| t.as_array())
        .ok_or("Missing threads array")?;

    let mut function_samples: HashMap<String, u64> = HashMap::new();
    let mut total_samples: u64 = 0;

    for thread in threads {
        let string_table = thread
            .get("stringTable")
            .and_then(|s| s.as_array())
            .unwrap_or(&Vec::new())
            .clone();

        let func_table_name = thread.pointer("/funcTable/name").and_then(|n| n.as_array());

        let frame_table_func = thread
            .pointer("/frameTable/func")
            .and_then(|f| f.as_array());

        let stack_table_frame = thread
            .pointer("/stackTable/frame")
            .and_then(|f| f.as_array());

        let stack_table_prefix = thread
            .pointer("/stackTable/prefix")
            .and_then(|p| p.as_array());

        let samples_stack = thread.pointer("/samples/stack").and_then(|s| s.as_array());

        let (
            Some(func_names),
            Some(frame_funcs),
            Some(stack_frames),
            Some(_stack_prefixes),
            Some(sample_stacks),
        ) = (
            func_table_name,
            frame_table_func,
            stack_table_frame,
            stack_table_prefix,
            samples_stack,
        )
        else {
            continue;
        };

        // Resolve function name for a given stack index (leaf frame = self time)
        let resolve_func_name = |stack_idx: usize| -> Option<String> {
            let frame_idx = stack_frames.get(stack_idx)?.as_u64()? as usize;
            let func_idx = frame_funcs.get(frame_idx)?.as_u64()? as usize;
            let name_idx = func_names.get(func_idx)?.as_u64()? as usize;
            string_table.get(name_idx)?.as_str().map(|s| s.to_string())
        };

        for sample_stack in sample_stacks {
            total_samples += 1;
            let name = sample_stack
                .as_u64()
                .and_then(|idx| resolve_func_name(idx as usize));
            if let Some(name) = name {
                *function_samples.entry(name).or_insert(0) += 1;
            }
        }
    }

    let mut sorted: Vec<(String, u64)> = function_samples.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let hotspots: Vec<Hotspot> = sorted
        .into_iter()
        .take(top_n)
        .map(|(name, count)| {
            let percentage = if total_samples > 0 {
                (count as f64 / total_samples as f64) * 100.0
            } else {
                0.0
            };
            Hotspot {
                function_name: name,
                file_path: None,
                line_number: None,
                sample_count: count,
                percentage,
                callers: Vec::new(),
            }
        })
        .collect();

    Ok((hotspots, total_samples))
}

// --- Shared Utilities ---

fn check_tool_exists(tool: &str, error_msg: &str) -> Result<(), Box<dyn std::error::Error>> {
    match Command::new("which").arg(tool).output() {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(error_msg.into()),
    }
}

/// Print formatted hotspot table to stdout
pub fn print_hotspot_table(hotspots: &[Hotspot], total_samples: u64) {
    println!();
    println!(
        "Top {} Hotspots ({} total samples)",
        hotspots.len(),
        total_samples
    );
    println!("{}", "=".repeat(80));
    println!("{:<6} {:<8} {:<10} Function", "Rank", "%", "Samples");
    println!("{}", "-".repeat(80));

    for (i, h) in hotspots.iter().enumerate() {
        println!(
            "{:<6} {:<8.1} {:<10} {}",
            i + 1,
            h.percentage,
            h.sample_count,
            h.function_name
        );
    }
    println!("{}", "=".repeat(80));
}

/// Load hotspots from a pprof .pb.gz file
pub fn load_hotspots_from_pprof(
    path: &Path,
    top_n: usize,
) -> Result<(Vec<Hotspot>, u64), Box<dyn std::error::Error>> {
    use flate2::read::GzDecoder;
    use prost::Message;

    mod pprof_proto {
        include!(concat!(env!("OUT_DIR"), "/perftools.profiles.rs"));
    }

    let file = fs::File::open(path)?;
    let mut decoder = GzDecoder::new(file);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;

    let profile = pprof_proto::Profile::decode(&*buf).map_err(|e| {
        format!(
            "Error: Cannot parse pprof file at {}. File may be corrupted or incompatible. ({})",
            path.display(),
            e
        )
    })?;

    let mut function_samples: HashMap<String, u64> = HashMap::new();
    let mut total_samples: u64 = 0;

    for sample in &profile.sample {
        let count = sample.value.first().copied().unwrap_or(0) as u64;
        total_samples += count;

        // Leaf location (first in the list) = self time
        let name = sample
            .location_id
            .first()
            .and_then(|&loc_id| profile.location.iter().find(|l| l.id == loc_id))
            .and_then(|loc| loc.line.first())
            .and_then(|line| profile.function.iter().find(|f| f.id == line.function_id))
            .and_then(|func| profile.string_table.get(func.name as usize).cloned());
        if let Some(name) = name {
            *function_samples.entry(name).or_insert(0) += count;
        }
    }

    let mut sorted: Vec<(String, u64)> = function_samples.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let hotspots: Vec<Hotspot> = sorted
        .into_iter()
        .take(top_n)
        .map(|(name, count)| {
            let percentage = if total_samples > 0 {
                (count as f64 / total_samples as f64) * 100.0
            } else {
                0.0
            };
            Hotspot {
                function_name: name,
                file_path: None,
                line_number: None,
                sample_count: count,
                percentage,
                callers: Vec::new(),
            }
        })
        .collect();

    Ok((hotspots, total_samples))
}

/// Auto-detect file format and load hotspots
pub fn load_hotspots_auto(
    path: &Path,
    top_n: usize,
) -> Result<(Vec<Hotspot>, u64), Box<dyn std::error::Error>> {
    let path_str = path.to_string_lossy();
    if path_str.ends_with(".pb.gz") {
        load_hotspots_from_pprof(path, top_n)
    } else if path_str.ends_with(".json.gz") {
        parse_samply_json(path, top_n)
    } else {
        Err(format!(
            "Error: Unsupported file format for '{}'. Expected .pb.gz or .json.gz",
            path.display()
        )
        .into())
    }
}
