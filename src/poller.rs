//! Event-driven poll loop. Parks on `wake.recv()` while sessions exist; falls
//! back to a bounded reconnect probe only when the wall is empty.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::AppMsg;
use crate::tmux;

/// Retry cadence while there is nothing to watch (tmux down / no sessions) —
/// this is a reconnect probe, not a content poll.
const RECONNECT: Duration = Duration::from_secs(2);

/// Steady-state cap: 5 iterations/s under continuous output; the first wake
/// after an idle period stays instant.
const MIN_REFRESH: Duration = Duration::from_millis(200);

/// Run the loop. `wake` carries pokes from the FIFO reader (real tmux activity)
/// and from the UI (kill / reveal). `visible` gates the expensive pane capture:
/// hidden windows still track the session list but skip previews.
pub fn run(
    tx: std::sync::mpsc::SyncSender<AppMsg>,
    ctx: egui::Context,
    wake: Receiver<()>,
    visible: Arc<AtomicBool>,
    fifo: String,
) {
    let mut piped: HashSet<String> = HashSet::new();
    let mut last_iteration = Instant::now();
    tmux::install_hooks(&fifo);

    loop {
        let sessions = tmux::list_sessions();
        let _ = tx.try_send(AppMsg::Sessions(sessions.clone()));

        // Wire pipe-pane onto any session we have not streamed yet, and forget
        // ones that died so a recycled id re-pipes cleanly.
        let alive: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
        for s in &sessions {
            if piped.insert(s.id.clone()) {
                tmux::pipe_pane(&s.id, &fifo);
            }
        }
        piped.retain(|id| alive.contains(id));

        // Previews only matter when the window is on screen.
        if visible.load(Ordering::Relaxed) {
            for s in &sessions {
                let _ = tx.try_send(AppMsg::Pane(s.name.clone(), tmux::capture_pane(&s.id)));
            }
        }

        ctx.request_repaint();

        // Park until the next real event. With live sessions this is a pure
        // `recv()` (no timer); only an empty wall falls back to the bounded
        // reconnect probe.
        wait_for_wake(&wake, !sessions.is_empty(), RECONNECT, last_iteration);
        // Stamped after the wait returns, so `elapsed` at the next call spans
        // the full wake→work→wait cycle — the quantity MIN_REFRESH caps.
        last_iteration = Instant::now();
    }
}

/// Block until the next event. Active (`has_sessions`): pure `recv()`, no
/// timer — burns zero CPU while sessions sit idle — unless the previous
/// iteration ended less than `MIN_REFRESH` ago, in which case early wakes are
/// coalesced until that deadline so a sustained output flood caps the loop at
/// 5 iterations/s. Idle: bounded `recv_timeout(reconnect)` probe so a fresh
/// tmux server is noticed. Either way, drain the backlog so a burst collapses
/// into one refresh.
fn wait_for_wake(
    wake: &Receiver<()>,
    has_sessions: bool,
    reconnect: Duration,
    last_iteration: Instant,
) {
    if has_sessions {
        if last_iteration.elapsed() >= MIN_REFRESH {
            // Outside the cooldown: first wake after a quiet period is instant.
            let _ = wake.recv();
        } else {
            // Cooldown: hold every wake until the MIN_REFRESH deadline so a
            // chatty pane can't spin the loop.
            let mut woke = false;
            loop {
                let remaining = MIN_REFRESH.saturating_sub(last_iteration.elapsed());
                if remaining.is_zero() {
                    break;
                }
                match wake.recv_timeout(remaining) {
                    Ok(()) => {
                        woke = true;
                        while wake.try_recv().is_ok() {}
                    }
                    // Deadline reached (or sender gone): stop coalescing.
                    Err(_) => break,
                }
            }
            if !woke {
                // Nothing arrived during the cooldown — the wall went quiet,
                // so fall back to the untimed sleep.
                let _ = wake.recv();
            }
        }
    } else {
        let _ = wake.recv_timeout(reconnect);
    }
    while wake.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::{MIN_REFRESH, wait_for_wake};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// Active mode is a pure `recv()`: it must NOT return on its own, even long
    /// after any plausible timer would have fired — then wake promptly on a poke.
    #[test]
    fn active_mode_blocks_until_wake() {
        let (tx, rx) = mpsc::sync_channel::<()>(8);
        let returned = Arc::new(AtomicBool::new(false));
        let flag = returned.clone();

        let handle = std::thread::spawn(move || {
            // reconnect is deliberately tiny; active mode must ignore it entirely.
            // last_iteration far in the past skips the cooldown entirely.
            wait_for_wake(
                &rx,
                true,
                Duration::from_millis(20),
                Instant::now() - Duration::from_secs(60),
            );
            flag.store(true, Ordering::SeqCst);
        });

        // 200 ms ≫ the 20 ms reconnect: if any timer were still in play the call
        // would have returned. It must not have.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !returned.load(Ordering::SeqCst),
            "active mode returned without a wake — timer polling still present"
        );

        // A real event releases it right away.
        tx.send(()).unwrap();
        handle.join().unwrap();
        assert!(
            returned.load(Ordering::SeqCst),
            "wake did not release recv()"
        );
    }

    /// Idle mode retries on a bounded timer and does NOT spin: three back-to-back
    /// probes with no wake must together consume ~3× the reconnect interval.
    #[test]
    fn idle_mode_reconnect_probe_does_not_busy_loop() {
        // `_tx` stays alive so recv_timeout yields Timeout, not Disconnected.
        let (_tx, rx) = mpsc::sync_channel::<()>(8);
        let reconnect = Duration::from_millis(60);

        let start = Instant::now();
        for _ in 0..3 {
            // last_iteration is irrelevant in idle mode; keep it old anyway.
            wait_for_wake(
                &rx,
                false,
                reconnect,
                Instant::now() - Duration::from_secs(60),
            );
        }
        let elapsed = start.elapsed();

        assert!(
            elapsed >= reconnect * 3,
            "idle probes returned in {elapsed:?} (< 3×{reconnect:?}) — busy loop"
        );
    }

    /// A queued burst returns immediately and is coalesced into one refresh,
    /// regardless of the (here huge) reconnect value.
    #[test]
    fn queued_burst_returns_immediately_and_coalesces() {
        let (tx, rx) = mpsc::sync_channel::<()>(8);
        for _ in 0..3 {
            tx.send(()).unwrap();
        }

        let start = Instant::now();
        // last_iteration far in the past: cooldown skipped, semantics unchanged.
        wait_for_wake(
            &rx,
            true,
            Duration::from_secs(30),
            Instant::now() - Duration::from_secs(60),
        );

        assert!(
            start.elapsed() < Duration::from_millis(100),
            "did not return promptly on an already-queued wake"
        );
        assert!(
            rx.try_recv().is_err(),
            "burst was not coalesced into one refresh"
        );
    }

    /// Cooldown caps the iteration rate: with a fresh `last_iteration` and a
    /// wake already queued, the call must hold until ~MIN_REFRESH (the queued
    /// wake is drained, then the loop waits out the window) and return soon
    /// after — never before.
    #[test]
    fn cooldown_caps_iteration_rate() {
        let (tx, rx) = mpsc::sync_channel::<()>(8);
        // Queued BEFORE the call: the first recv_timeout succeeds instantly,
        // so the elapsed time measures the cooldown window, not scheduling.
        tx.send(()).unwrap();

        let start = Instant::now();
        wait_for_wake(&rx, true, Duration::from_secs(30), start);
        let elapsed = start.elapsed();

        assert!(
            elapsed >= MIN_REFRESH - Duration::from_millis(20),
            "returned in {elapsed:?} (< {MIN_REFRESH:?}) — cooldown not enforced"
        );
        assert!(
            elapsed < MIN_REFRESH + Duration::from_millis(500),
            "returned in {elapsed:?} — cooldown overshot badly"
        );
    }
}
