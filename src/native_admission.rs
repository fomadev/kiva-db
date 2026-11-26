// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use std::collections::VecDeque;
use std::sync::{mpsc, Mutex};

use crate::error::{KivaError, Result};
use crate::native_timing::{span, Stage};

#[derive(Clone, Copy)]
pub(crate) enum Class {
    Read,
    Data,
    Control,
    Authentication,
}

pub(crate) struct NativeAdmission {
    reads: Lane,
    data: Lane,
    control: Lane,
    auth: Lane,
}

impl NativeAdmission {
    pub(crate) fn new(threads: usize, waiting: usize) -> Self {
        let data = threads.clamp(1, 64);
        Self {
            reads: Lane::new(data, waiting),
            data: Lane::new(data, waiting),
            control: Lane::new(1, waiting),
            auth: Lane::new(data.min(4), waiting),
        }
    }

    pub(crate) fn enter(&self, class: Class) -> Result<NativePermit<'_>> {
        let lane = match class {
            Class::Read => &self.reads,
            Class::Data => &self.data,
            Class::Control => &self.control,
            Class::Authentication => &self.auth,
        };
        let _queue = span(Stage::Queue);
        lane.enter()
    }
}

struct Lane {
    max_waiting: usize,
    state: Mutex<State>,
}

struct State {
    free: usize,
    waiters: VecDeque<mpsc::SyncSender<()>>,
}

pub(crate) struct NativePermit<'a>(&'a Lane);

impl Lane {
    fn new(slots: usize, max_waiting: usize) -> Self {
        Self { max_waiting, state: Mutex::new(State { free: slots, waiters: VecDeque::new() }) }
    }

    fn enter(&self) -> Result<NativePermit<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        if state.free != 0 {
            state.free -= 1;
        } else {
            if state.waiters.len() >= self.max_waiting {
                return Err(KivaError::Other("ERR_SERVER_BUSY: native request queue is full".into()));
            }
            let (sender, receiver) = mpsc::sync_channel(1);
            state.waiters.push_back(sender);
            drop(state);
            receiver.recv().map_err(|_| KivaError::Other("native admission closed".into()))?;
        }
        Ok(NativePermit(self))
    }
}

impl Drop for NativePermit<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|err| err.into_inner());
        // Give the permit directly to the oldest waiter. New arrivals
        // cannot take that permit before the waiter is scheduled.
        while let Some(waiter) = state.waiters.pop_front() {
            if waiter.send(()).is_ok() {
                return;
            }
        }
        state.free += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn queued_work_is_fifo_bounded_and_isolated_by_class() {
        let admission = NativeAdmission::new(1, 2);
        let held = admission.enter(Class::Data).unwrap();
        std::thread::scope(|scope| {
            let (completed, order) = mpsc::channel();
            for i in 0..2 {
                let completed = completed.clone();
                let admission = &admission;
                scope.spawn(move || {
                    let _permit = admission.enter(Class::Data).unwrap();
                    completed.send(i).unwrap();
                });
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if admission.data.state.lock().unwrap().waiters.len() == i + 1 {
                        break;
                    }
                    assert!(Instant::now() < deadline, "worker did not queue");
                    std::thread::yield_now();
                }
            }
            let busy = admission.enter(Class::Data).err().expect("third waiter exceeds capacity");
            assert!(busy.to_string().starts_with("ERR_SERVER_BUSY"));
            assert!(admission.enter(Class::Read).is_ok());
            assert!(admission.enter(Class::Control).is_ok());
            drop(held);
            for i in 0..2 {
                assert_eq!(order.recv_timeout(Duration::from_secs(5)).unwrap(), i);
            }
        });
    }
}
