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

pub mod compare;
pub mod dynamic_graph;
pub mod hardware_profile;
pub mod hit_leiden_incremental;
pub mod manifest;
pub mod release_gate;
pub mod runner;

#[cfg(feature = "profiling")]
pub mod charting;
#[cfg(feature = "profiling")]
pub mod igraph_baseline;
#[cfg(feature = "profiling")]
pub mod igraph_cache;
#[cfg(feature = "profiling")]
pub mod profiling;
#[cfg(feature = "profiling")]
pub mod progress;
#[cfg(feature = "profiling")]
pub mod toon_export;
