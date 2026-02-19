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

//! igraph Leiden baseline via Python subprocess.
//!
//! Calls the `scripts/run_igraph_leiden.py` script which uses the C-based
//! igraph library (via python-igraph + leidenalg) to run Leiden community
//! detection. This provides a comparison against the gold-standard Leiden
//! implementation used in most academic papers.

#[cfg(feature = "profiling")]
use std::io::Write;
#[cfg(feature = "profiling")]
use std::path::{Path, PathBuf};
#[cfg(feature = "profiling")]
use std::process::Command;
#[cfg(feature = "profiling")]
use std::sync::OnceLock;

/// Result from an igraph Leiden run.
#[cfg(feature = "profiling")]
#[derive(Clone, Debug)]
pub struct IgraphResult {
    pub time_ms: f64,
    pub modularity: f64,
    pub num_communities: usize,
    pub partition: Vec<usize>,
}

#[cfg(feature = "profiling")]
pub struct IgraphLeidenBaseline;

#[cfg(feature = "profiling")]
impl IgraphLeidenBaseline {
    /// Check if Python + igraph + leidenalg are available.
    /// Result is cached after first probe.
    pub fn is_available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            let python = Self::find_python();
            let output = Command::new(&python)
                .args(["-c", "import igraph; import leidenalg"])
                .output();
            match output {
                Ok(o) => o.status.success(),
                Err(_) => false,
            }
        })
    }

    /// Run igraph Leiden on the given edges via Python subprocess.
    /// Returns (time_ms, modularity, num_communities, partition).
    pub fn run(
        edges: &[(usize, usize, Option<f64>)],
        num_nodes: usize,
    ) -> Result<IgraphResult, String> {
        let python = Self::find_python();
        let script = Self::find_script()?;

        // Write edges to temp file
        let tmp_path = Self::write_edge_file(edges)?;

        // Spawn Python script
        let output = Command::new(&python)
            .args([
                script.to_str().unwrap_or("scripts/run_igraph_leiden.py"),
                "--edge-file",
                tmp_path.to_str().unwrap_or(""),
                "--num-nodes",
                &num_nodes.to_string(),
            ])
            .output()
            .map_err(|e| format!("Failed to spawn igraph Python process: {}", e))?;

        // Clean up temp file regardless of outcome
        let _ = std::fs::remove_file(&tmp_path);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "igraph Python script failed (exit {}): {}",
                output.status, stderr
            ));
        }

        // Log stderr (diagnostic messages from Python script)
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            for line in stderr.lines() {
                eprintln!("[igraph] {}", line);
            }
        }

        // Parse JSON stdout
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(&stdout).map_err(|e| {
            format!(
                "Failed to parse igraph JSON output: {} (raw: {})",
                e, stdout
            )
        })?;

        Ok(IgraphResult {
            time_ms: result["time_ms"]
                .as_f64()
                .ok_or("missing time_ms in igraph output")?,
            modularity: result["modularity"]
                .as_f64()
                .ok_or("missing modularity in igraph output")?,
            num_communities: result["num_communities"]
                .as_u64()
                .ok_or("missing num_communities in igraph output")?
                as usize,
            partition: result["partition"]
                .as_array()
                .ok_or("missing partition in igraph output")?
                .iter()
                .map(|v| v.as_u64().unwrap_or(0) as usize)
                .collect(),
        })
    }

    /// Find the Python interpreter. Prefer the project's uv-managed venv,
    /// then fall back to system python3.
    fn find_python() -> String {
        // Try venv relative to CARGO_MANIFEST_DIR (compile-time)
        let venv_python = Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python3");
        if venv_python.exists() {
            return venv_python.to_string_lossy().to_string();
        }

        // Try venv relative to current working directory (runtime)
        let cwd_venv = Path::new(".venv/bin/python3");
        if cwd_venv.exists() {
            return cwd_venv.to_string_lossy().to_string();
        }

        // Fall back to system python3
        "python3".to_string()
    }

    /// Find the igraph Python script.
    fn find_script() -> Result<PathBuf, String> {
        // Try relative to CARGO_MANIFEST_DIR (compile-time)
        let manifest_script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/run_igraph_leiden.py");
        if manifest_script.exists() {
            return Ok(manifest_script);
        }

        // Try relative to cwd (runtime)
        let cwd_script = Path::new("scripts/run_igraph_leiden.py");
        if cwd_script.exists() {
            return Ok(cwd_script.to_path_buf());
        }

        Err("Could not find scripts/run_igraph_leiden.py".to_string())
    }

    /// Write edges to a temporary TSV file for the Python script.
    fn write_edge_file(edges: &[(usize, usize, Option<f64>)]) -> Result<PathBuf, String> {
        let tmp_dir = std::env::temp_dir();
        let tmp_path = tmp_dir.join(format!("hit_leiden_igraph_{}.tsv", std::process::id()));

        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("Failed to create temp edge file: {}", e))?;

        for &(src, dst, weight) in edges {
            if let Some(w) = weight {
                writeln!(file, "{}\t{}\t{}", src, dst, w)
            } else {
                writeln!(file, "{}\t{}", src, dst)
            }
            .map_err(|e| format!("Failed to write edge: {}", e))?;
        }

        Ok(tmp_path)
    }
}
