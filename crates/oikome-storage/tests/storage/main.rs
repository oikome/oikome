// Copyright 2025 The Oikome Authors
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

//! oikome-storage's integration tests, one module per area, built as one test
//! binary: each test binary links DuckDB, DataFusion and Lance, so one per
//! crate keeps builds smaller and faster (AGENT.md §6.2.1).
//! `cargo t --test storage -- compaction_test::` runs one area.

mod compaction_test;
mod integration_test;
mod pushdown_test;
mod tpc_h_integration;
