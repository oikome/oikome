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

//! How far a copy, backup, check or restore has got, in bytes, for whoever
//! shows it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rustic_core::{Progress, ProgressBars, ProgressType, RusticProgress};

/// Bytes done of a step and the step's whole, as the step goes.
#[derive(Debug, Default)]
pub struct Meter {
    done: AtomicU64,
    total: AtomicU64,
}

impl Meter {
    /// (done, total) of the step under way.
    pub fn read(&self) -> (u64, u64) {
        (
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }

    /// A new step of `total` bytes.
    pub fn start(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
    }

    pub fn add(&self, bytes: u64) {
        self.done.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// rustic's progress, its byte counts into a meter (when there is one).
#[derive(Debug, Clone, Default)]
pub(crate) struct Bars(pub(crate) Option<Arc<Meter>>);

impl ProgressBars for Bars {
    fn progress(&self, kind: ProgressType, _prefix: &str) -> Progress {
        match (kind, &self.0) {
            (ProgressType::Bytes, Some(meter)) => Progress::new(Bytes(meter.clone())),
            _ => Progress::hidden(),
        }
    }
}

#[derive(Debug)]
struct Bytes(Arc<Meter>);

impl RusticProgress for Bytes {
    fn is_hidden(&self) -> bool {
        false
    }
    fn set_length(&self, len: u64) {
        self.0.start(len);
    }
    fn set_title(&self, _title: &str) {}
    fn inc(&self, inc: u64) {
        self.0.add(inc);
    }
    fn finish(&self) {}
}
