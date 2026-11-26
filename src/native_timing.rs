// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Stage {
    FrameRead,
    Decode,
    Queue,
    Authentication,
    Account,
    Collection,
    Permission,
    Layout,
    Registry,
    Execute,
    Sync,
    Encode,
    FrameWrite,
}

const STAGES: [&str; 13] = [
    "frame_read",
    "decode",
    "queue",
    "authentication",
    "account",
    "collection",
    "permission",
    "layout",
    "registry",
    "execute",
    "sync",
    "encode",
    "frame_write",
];
const KINDS: [&str; 7] = ["get", "set", "del", "exec", "api", "auth", "control"];

#[derive(Clone, Copy, Default)]
struct Counter {
    samples: u64,
    nanos: u64,
}

type Timings = [Counter; STAGES.len()];
thread_local! {
    static CURRENT: Cell<Option<Timings>> = const { Cell::new(None) };
}

pub(crate) struct Span(Stage, Option<Instant>);

pub(crate) fn span(stage: Stage) -> Span {
    Span(stage, CURRENT.with(|current| current.get().is_some()).then(Instant::now))
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.1 {
            CURRENT.with(|current| {
                if let Some(mut timings) = current.get() {
                    let counter = &mut timings[self.0 as usize];
                    counter.samples += 1;
                    counter.nanos += start.elapsed().as_nanos() as u64;
                    current.set(Some(timings));
                }
            });
        }
    }
}

#[derive(Default)]
struct Total {
    samples: AtomicU64,
    nanos: AtomicU64,
}

#[derive(Default)]
pub(crate) struct NativeMetrics {
    totals: [[Total; STAGES.len()]; KINDS.len()],
}

/// Aggregate native timing samples since the instance started. No account,
/// key, collection or address is retained.
#[derive(Debug, Clone)]
pub struct NativeTiming {
    pub operation: &'static str,
    pub stage: &'static str,
    pub samples: u64,
    pub nanos: u64,
}

impl NativeMetrics {
    pub(crate) fn begin(&self) {
        CURRENT.with(|current| current.set(Some([Counter::default(); STAGES.len()])));
    }

    pub(crate) fn finish(&self, command: u16) {
        use crate::proto::*;
        let row = match command {
            CMD_GET => 0,
            CMD_SET => 1,
            CMD_DEL => 2,
            CMD_EXEC => 3,
            CMD_API => 4,
            CMD_AUTH => 5,
            _ => 6,
        };
        if let Some(timings) = CURRENT.with(Cell::take) {
            for (total, count) in self.totals[row].iter().zip(timings) {
                if count.samples != 0 {
                    total.samples.fetch_add(count.samples, Relaxed);
                    total.nanos.fetch_add(count.nanos, Relaxed);
                }
            }
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<NativeTiming> {
        self.totals
            .iter()
            .zip(KINDS)
            .flat_map(|(row, operation)| {
                row.iter().zip(STAGES).map(move |(total, stage)| NativeTiming {
                    operation,
                    stage,
                    samples: total.samples.load(Relaxed),
                    nanos: total.nanos.load(Relaxed),
                })
            })
            .collect()
    }
}
