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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphSource {
    File,
    Neo4jSnapshot,
    LiveNeo4j,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccelerationTarget {
    PureRust,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionMetadata {
    pub source_resolved: GraphSource,
    pub accel_resolved: AccelerationTarget,
    pub fallback_reason: Option<String>,
}
