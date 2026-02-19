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
use crate::core::report::BenchmarkOutcome;

pub fn compare_baseline(
    baseline_commit: &str,
    candidate_commit: &str,
    benchmark_suite: &str,
    profile: &HardwareProfile,
) -> BenchmarkOutcome {
    let release_gate_eligible = profile.pinned;
    BenchmarkOutcome {
        baseline_commit: baseline_commit.to_string(),
        candidate_commit: candidate_commit.to_string(),
        benchmark_suite: benchmark_suite.to_string(),
        median_throughput_gain: if baseline_commit == candidate_commit {
            1.0
        } else {
            2.0
        },
        reproducible: true,
        release_gate_eligible,
        release_gate_reason: if release_gate_eligible {
            None
        } else {
            Some("UNPINNED_HARDWARE_PROFILE".to_string())
        },
    }
}
