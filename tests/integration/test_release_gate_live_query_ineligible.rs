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

use hit_leiden::benchmark::hardware_profile::HardwareProfile;
use hit_leiden::benchmark::release_gate::eligible;
use hit_leiden::core::backend::GraphSource;

#[test]
fn live_query_is_ineligible_for_release_gate() {
    let profile = HardwareProfile {
        id: "pinned".into(),
        pinned: true,
    };
    let (ok, reason) = eligible(&profile, GraphSource::LiveNeo4j);
    assert!(!ok);
    assert_eq!(
        reason.as_deref(),
        Some("LIVE_QUERY_SOURCE_INELIGIBLE_FOR_RELEASE_GATE")
    );
}
