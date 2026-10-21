//! TEMP(writer scroll trace): logs wheel packets and editor frame times to stderr when
//! `WRITER_SCROLL_TRACE=1`. Remove after the scroll hitch is found.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::Instant;

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("WRITER_SCROLL_TRACE").is_some())
}

pub(crate) fn now_ms() -> f64 {
    epoch().elapsed().as_secs_f64() * 1000.
}

thread_local! {
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub(crate) fn prepaint_started() {
    if enabled() {
        STARTED.set(Some(Instant::now()));
    }
}

pub(crate) fn painted(before: f32, after: f32, stretch: f32) {
    if !enabled() {
        return;
    }
    let took = STARTED
        .take()
        .map_or(-1., |at| at.elapsed().as_secs_f64() * 1000.);
    let note = if before != after {
        "  <-- paint moved scroll"
    } else {
        ""
    };
    eprintln!(
        "[frame] t={:.1}ms editor={:.2}ms y {:.3} -> {:.3} stretch={:.3}{note}",
        now_ms(),
        took,
        before,
        after,
        stretch
    );
}
