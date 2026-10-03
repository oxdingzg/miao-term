//! Low-cost render counters for the application's resource recorder.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static FRAMES: AtomicU64 = AtomicU64::new(0);
static RENDER_US: AtomicU64 = AtomicU64::new(0);

pub struct RenderTimer(Instant);

impl RenderTimer {
    pub(crate) fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for RenderTimer {
    fn drop(&mut self) {
        RENDER_US.fetch_add(self.0.elapsed().as_micros() as u64, Ordering::Relaxed);
        FRAMES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Cumulative render calls and wall time (including early-return frames).
pub fn snapshot() -> (u64, u64) {
    (
        FRAMES.load(Ordering::Relaxed),
        RENDER_US.load(Ordering::Relaxed),
    )
}
