// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Crash points: places of the commit and of the recovery where a test stops
//! the process at once, to check what a restart finds
//! (docs/testing/transaction-crash-audit.md).
//!
//! They exist only in a build with the feature `crash-points`
//! (`cargo test --features crash-points --test crash`). Without it,
//! [`crash_point!`] expands to nothing: a release holds no crash point, and
//! no check of any kind on the paths it marks.
//!
//! With it, a point fires only when the process was started with
//! `KIVADB_CRASH_ARMED=1` and `KIVADB_CRASH_POINT=<its name>`, at its
//! `KIVADB_CRASH_HIT`-th passage (1 by default), so that a test can aim at
//! one transaction among several. It then writes `KIVADB-CRASH-POINT <name>`
//! on stderr and calls [`std::process::abort`]: no unwinding, no `Drop`, no
//! rollback, no flush of buffers. What reached the operating system stays
//! (a process crash); nothing simulates a power cut.

/// Stops the process here, when a test asks for this point by its name.
#[macro_export]
macro_rules! crash_point {
    ($name:expr) => {
        #[cfg(feature = "crash-points")]
        $crate::crash::reached($name);
    };
}

#[cfg(feature = "crash-points")]
mod armed {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::OnceLock;

    struct Aim {
        point: String,
        hit: u64,
    }

    fn aim() -> Option<&'static Aim> {
        static AIM: OnceLock<Option<Aim>> = OnceLock::new();
        AIM.get_or_init(|| {
            if std::env::var("KIVADB_CRASH_ARMED").ok().as_deref() != Some("1") {
                return None;
            }
            let point = std::env::var("KIVADB_CRASH_POINT").ok().filter(|p| !p.is_empty())?;
            let hit = std::env::var("KIVADB_CRASH_HIT").ok().and_then(|h| h.parse().ok()).unwrap_or(1).max(1);
            Some(Aim { point, hit })
        })
        .as_ref()
    }

    static PASSED: AtomicU64 = AtomicU64::new(0);

    pub fn reached(name: &str) {
        let Some(aim) = aim() else { return };
        if aim.point != name {
            return;
        }
        if PASSED.fetch_add(1, Ordering::SeqCst) + 1 == aim.hit {
            use std::io::Write;
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "KIVADB-CRASH-POINT {name}");
            let _ = err.flush();
            std::process::abort();
        }
    }

    /// Whether `name` is the point asked for, at the passage asked for,
    /// without counting a passage: for a point that must do something of its
    /// own first (write half a batch, say) before [`reached`] stops it.
    pub fn aimed_at(name: &str) -> bool {
        aim().is_some_and(|a| a.point == name && PASSED.load(Ordering::SeqCst) + 1 == a.hit)
    }
}

#[cfg(feature = "crash-points")]
pub use armed::{aimed_at, reached};

/// Whether a test aims at `name` now. Always false without the feature.
#[cfg(not(feature = "crash-points"))]
#[inline(always)]
pub fn aimed_at(_name: &str) -> bool {
    false
}
