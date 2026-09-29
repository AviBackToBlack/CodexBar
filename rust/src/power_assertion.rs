//! Stay Awake: hold one idle-sleep prevention while a local agent session is live.
//!
//! Port of upstream CodexBar 0.67.0 `AgentSessionPowerAssertion` and the
//! `updatePowerAssertion` policy in `AgentSessionsStore`. [`PowerAssertion`] is
//! the platform seam (injectable for tests); [`StayAwake`] is the pure policy
//! that decides when to acquire and release it.
//!
//! Only the system is kept awake, never the display: no display-required flag
//! is set, and lid close or an explicit sleep still sleeps the machine.

/// Platform seam for one system idle-sleep prevention.
pub trait PowerAssertion: Send {
    /// Start preventing idle system sleep. Returns `false` when the OS refused.
    fn acquire(&mut self) -> bool;
    /// Stop preventing idle system sleep. Safe to call when nothing is held.
    fn release(&mut self);
}

/// Stay Awake policy: at most one assertion, held only while enabled and a
/// live local agent session exists.
///
/// Every method returns whether the held state changed so callers can refresh
/// UI that mirrors it. Scan results arrive asynchronously, so [`observe`] is
/// ignored once the feature is disabled or shut down: a stale scan completion
/// can never acquire after the toggle was turned off.
///
/// [`observe`]: StayAwake::observe
pub struct StayAwake<A: PowerAssertion> {
    assertion: A,
    enabled: bool,
    shut_down: bool,
    held: bool,
}

impl<A: PowerAssertion> StayAwake<A> {
    pub fn new(assertion: A) -> Self {
        Self {
            assertion,
            enabled: false,
            shut_down: false,
            held: false,
        }
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Apply the user setting. Turning it off releases immediately; turning it
    /// on acquires nothing until a scan reports a live session.
    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        self.enabled = enabled;
        if enabled { false } else { self.release() }
    }

    /// Record one scan outcome. A failed acquire leaves nothing held, so the
    /// next scan that still sees a live session retries.
    pub fn observe(&mut self, live_local_session: bool) -> bool {
        if !self.enabled || self.shut_down {
            return false;
        }
        match (live_local_session, self.held) {
            (true, false) => {
                self.held = self.assertion.acquire();
                self.held
            }
            (false, true) => self.release(),
            _ => false,
        }
    }

    /// Release for good on app exit; later scans and toggles are ignored.
    pub fn shutdown(&mut self) -> bool {
        self.shut_down = true;
        self.enabled = false;
        self.release()
    }

    fn release(&mut self) -> bool {
        if !self.held {
            return false;
        }
        self.assertion.release();
        self.held = false;
        true
    }
}

#[cfg(windows)]
pub use windows_impl::SystemPowerAssertion;

/// Holds `ES_SYSTEM_REQUIRED` for the process.
///
/// `SetThreadExecutionState(ES_CONTINUOUS | ...)` is scoped to the calling
/// thread and lapses when that thread ends, so a dedicated long-lived thread
/// owns it. Calling it from a tokio worker would drop the request whenever the
/// runtime retires or reuses that thread.
#[cfg(windows)]
mod windows_impl {
    use super::PowerAssertion;
    use std::sync::mpsc::{self, Sender};
    use std::thread::JoinHandle;
    use std::time::Duration;
    use windows::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, EXECUTION_STATE, SetThreadExecutionState,
    };

    const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);

    enum Request {
        Hold(Sender<bool>),
        Clear,
    }

    struct Worker {
        requests: Sender<Request>,
        thread: JoinHandle<()>,
    }

    #[derive(Default)]
    pub struct SystemPowerAssertion {
        worker: Option<Worker>,
    }

    impl SystemPowerAssertion {
        pub fn new() -> Self {
            Self::default()
        }

        fn worker(&mut self) -> Option<&Worker> {
            if self.worker.is_none() {
                let (requests, inbox) = mpsc::channel();
                let thread = std::thread::Builder::new()
                    .name("codexbar-stay-awake".to_string())
                    .spawn(move || run(inbox))
                    .map_err(|error| tracing::warn!("Stay Awake thread failed to start: {error}"))
                    .ok()?;
                self.worker = Some(Worker { requests, thread });
            }
            self.worker.as_ref()
        }
    }

    impl PowerAssertion for SystemPowerAssertion {
        fn acquire(&mut self) -> bool {
            let (reply, answer) = mpsc::channel();
            let Some(worker) = self.worker() else {
                return false;
            };
            if worker.requests.send(Request::Hold(reply)).is_err() {
                // The thread is gone; the next attempt starts a fresh one.
                self.worker = None;
                return false;
            }
            answer.recv_timeout(ACQUIRE_TIMEOUT).unwrap_or(false)
        }

        fn release(&mut self) {
            if let Some(worker) = &self.worker
                && worker.requests.send(Request::Clear).is_err()
            {
                self.worker = None;
            }
        }
    }

    impl Drop for SystemPowerAssertion {
        fn drop(&mut self) {
            if let Some(Worker { requests, thread }) = self.worker.take() {
                drop(requests);
                thread.join().ok();
            }
        }
    }

    fn run(inbox: mpsc::Receiver<Request>) {
        while let Ok(request) = inbox.recv() {
            match request {
                Request::Hold(reply) => {
                    reply
                        .send(set_state(ES_CONTINUOUS | ES_SYSTEM_REQUIRED))
                        .ok();
                }
                Request::Clear => {
                    set_state(ES_CONTINUOUS);
                }
            }
        }
        // Channel closed: clear explicitly instead of relying on thread exit.
        set_state(ES_CONTINUOUS);
    }

    /// `SetThreadExecutionState` returns the previous state, or 0 on failure.
    fn set_state(flags: EXECUTION_STATE) -> bool {
        // SAFETY: plain Win32 call with valid flags; it only touches the
        // calling thread's execution state.
        unsafe { SetThreadExecutionState(flags) }.0 != 0
    }
}

#[cfg(not(windows))]
pub use fallback::SystemPowerAssertion;

/// Other platforms have no implementation; nothing is ever held.
#[cfg(not(windows))]
mod fallback {
    use super::PowerAssertion;

    #[derive(Default)]
    pub struct SystemPowerAssertion;

    impl SystemPowerAssertion {
        pub fn new() -> Self {
            Self
        }
    }

    impl PowerAssertion for SystemPowerAssertion {
        fn acquire(&mut self) -> bool {
            false
        }

        fn release(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Counts {
        acquires: u32,
        releases: u32,
        fail_next_acquires: u32,
    }

    #[derive(Clone, Default)]
    struct FakeAssertion(Arc<Mutex<Counts>>);

    impl FakeAssertion {
        fn counts(&self) -> (u32, u32) {
            let counts = self.0.lock().unwrap();
            (counts.acquires, counts.releases)
        }

        fn fail_next(&self, times: u32) {
            self.0.lock().unwrap().fail_next_acquires = times;
        }
    }

    impl PowerAssertion for FakeAssertion {
        fn acquire(&mut self) -> bool {
            let mut counts = self.0.lock().unwrap();
            counts.acquires += 1;
            if counts.fail_next_acquires > 0 {
                counts.fail_next_acquires -= 1;
                return false;
            }
            true
        }

        fn release(&mut self) {
            self.0.lock().unwrap().releases += 1;
        }
    }

    fn enabled() -> (StayAwake<FakeAssertion>, FakeAssertion) {
        let fake = FakeAssertion::default();
        let mut stay_awake = StayAwake::new(fake.clone());
        stay_awake.set_enabled(true);
        (stay_awake, fake)
    }

    #[test]
    fn disabled_never_acquires() {
        let fake = FakeAssertion::default();
        let mut stay_awake = StayAwake::new(fake.clone());
        assert!(!stay_awake.observe(true));
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (0, 0));
    }

    #[test]
    fn enabling_alone_acquires_nothing() {
        let (stay_awake, fake) = enabled();
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (0, 0));
    }

    #[test]
    fn live_session_holds_exactly_one_assertion_until_it_disappears() {
        let (mut stay_awake, fake) = enabled();
        assert!(stay_awake.observe(true), "appearing session acquires");
        assert!(!stay_awake.observe(true), "idle session keeps the hold");
        assert!(!stay_awake.observe(true));
        assert!(stay_awake.is_held());
        assert_eq!(fake.counts(), (1, 0));

        assert!(stay_awake.observe(false), "disappearing session releases");
        assert!(!stay_awake.observe(false));
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (1, 1));

        assert!(stay_awake.observe(true), "a later session holds again");
        assert_eq!(fake.counts(), (2, 1));
    }

    #[test]
    fn toggling_off_releases_immediately() {
        let (mut stay_awake, fake) = enabled();
        stay_awake.observe(true);
        assert!(stay_awake.set_enabled(false));
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (1, 1));
        assert!(!stay_awake.set_enabled(false), "already released");
        assert_eq!(fake.counts(), (1, 1));
    }

    #[test]
    fn stale_scan_after_disable_does_not_acquire() {
        let (mut stay_awake, fake) = enabled();
        stay_awake.set_enabled(false);
        assert!(!stay_awake.observe(true));
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (0, 0));
    }

    #[test]
    fn shutdown_releases_and_blocks_later_acquires() {
        let (mut stay_awake, fake) = enabled();
        stay_awake.observe(true);
        assert!(stay_awake.shutdown());
        assert_eq!(fake.counts(), (1, 1));

        assert!(!stay_awake.observe(true));
        stay_awake.set_enabled(true);
        assert!(!stay_awake.observe(true));
        assert!(!stay_awake.is_held());
        assert_eq!(fake.counts(), (1, 1));
    }

    #[test]
    fn failed_acquire_retries_on_the_next_scan() {
        let (mut stay_awake, fake) = enabled();
        fake.fail_next(1);
        assert!(!stay_awake.observe(true), "refused acquire is not held");
        assert!(!stay_awake.is_held());
        assert!(stay_awake.observe(true), "next scan retries");
        assert!(stay_awake.is_held());
        assert_eq!(fake.counts(), (2, 0));
    }

    #[test]
    fn failed_acquire_then_no_session_never_releases() {
        let (mut stay_awake, fake) = enabled();
        fake.fail_next(1);
        stay_awake.observe(true);
        assert!(!stay_awake.observe(false));
        assert_eq!(fake.counts(), (1, 0));
    }

    #[cfg(windows)]
    #[test]
    fn system_assertion_acquires_and_releases_on_its_own_thread() {
        let mut assertion = SystemPowerAssertion::new();
        assert!(assertion.acquire());
        assertion.release();
        assert!(assertion.acquire(), "can be re-acquired after release");
        drop(assertion);
    }
}
