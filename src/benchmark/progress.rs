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

use crate::core::types::ProgressEvent;

/// Trait for reporting benchmark progress
pub trait ProgressReporter {
    fn report(&mut self, event: &ProgressEvent);
}

/// Writes structured progress lines to stderr
pub struct StderrProgressReporter;

impl ProgressReporter for StderrProgressReporter {
    fn report(&mut self, event: &ProgressEvent) {
        let elapsed_secs = event.elapsed.as_secs_f64();

        let batch_info = match (event.batch_index, event.batch_total) {
            (Some(idx), Some(total)) => format!(" {}/{}", idx, total),
            _ => String::new(),
        };

        let metric_info = match (&event.metric_label, event.metric_value) {
            (Some(label), Some(value)) => format!(" {}={:.2}x", label, value),
            _ => String::new(),
        };

        eprintln!(
            "[{:.1}s] {}{}{}",
            elapsed_secs, event.phase, batch_info, metric_info
        );
    }
}
