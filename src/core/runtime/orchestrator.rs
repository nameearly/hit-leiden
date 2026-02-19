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

use crate::core::config::RunConfig;
use crate::core::runtime::resolver;

pub fn resolve_with_fallback(
    config: &RunConfig,
    available: bool,
) -> crate::core::backend::ResolutionMetadata {
    let mut r = resolver::resolve(config);
    if !available {
        r.fallback_reason = Some("ACCEL_UNAVAILABLE".to_string());
    }
    r
}
