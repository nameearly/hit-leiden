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
use crate::core::backend::GraphSource;

pub fn eligible(profile: &HardwareProfile, source: GraphSource) -> (bool, Option<String>) {
    if !profile.pinned {
        return (false, Some("UNPINNED_HARDWARE_PROFILE".to_string()));
    }
    if source == GraphSource::LiveNeo4j {
        return (
            false,
            Some("LIVE_QUERY_SOURCE_INELIGIBLE_FOR_RELEASE_GATE".to_string()),
        );
    }
    (true, None)
}
