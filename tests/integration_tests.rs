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

#[path = "integration/test_benchmark_reproducibility.rs"]
mod test_benchmark_reproducibility;
#[path = "integration/test_connected_graph_not_all_singletons.rs"]
mod test_connected_graph_not_all_singletons;
#[path = "integration/test_default_config_minimal_args.rs"]
mod test_default_config_minimal_args;
#[path = "integration/test_deterministic_identity.rs"]
mod test_deterministic_identity;
#[path = "integration/test_neo4j_snapshot_parity.rs"]
mod test_neo4j_snapshot_parity;
#[path = "integration/test_release_gate_live_query_ineligible.rs"]
mod test_release_gate_live_query_ineligible;
#[path = "integration/test_throughput_equivalence.rs"]
mod test_throughput_equivalence;
