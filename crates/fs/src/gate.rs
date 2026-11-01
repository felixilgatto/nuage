//! Whether the backing folder (the hidden rclone mount) is usable.
//!
//! While rclone remounts, the backing folder is an empty directory. Rather
//! than showing an empty folder, operations wait for it to come back, and
//! fail with `EIO` if it takes too long.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Debug)]
pub struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    timeout: Duration,
    /// The provider cannot be reached (v1.1): nothing waits for the
    /// backing folder, requests that need it fail at once.
    offline: AtomicBool,
}

impl Gate {
    pub fn new(open: bool, timeout: Duration) -> Self {
        Self {
            open: Mutex::new(open),
            changed: Condvar::new(),
            timeout,
            offline: AtomicBool::new(false),
        }
    }

    pub fn set(&self, open: bool) {
        *self.open.lock().expect("gate lock") = open;
        self.changed.notify_all();
    }

    pub fn set_offline(&self, offline: bool) {
        // Under the lock: a waiter checks the flag before sleeping.
        let _guard = self.open.lock().expect("gate lock");
        self.offline.store(offline, Ordering::SeqCst);
        self.changed.notify_all();
    }

    pub fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Waits until the gate is open; `false` if it stayed closed or the
    /// provider is offline.
    pub fn wait(&self) -> bool {
        let guard = self.open.lock().expect("gate lock");
        let (guard, _) = self
            .changed
            .wait_timeout_while(guard, self.timeout, |open| {
                !*open && !self.offline.load(Ordering::SeqCst)
            })
            .expect("gate lock");
        *guard && !self.is_offline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn waits_then_gives_up() {
        let gate = Gate::new(false, Duration::from_millis(50));
        assert!(!gate.wait());
        gate.set(true);
        assert!(gate.wait());
    }

    #[test]
    fn wakes_up_when_opened() {
        let gate = Arc::new(Gate::new(false, Duration::from_secs(10)));
        let g = Arc::clone(&gate);
        let opener = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            g.set(true);
        });
        let start = std::time::Instant::now();
        assert!(gate.wait());
        assert!(start.elapsed() < Duration::from_secs(5));
        opener.join().unwrap();
    }

    #[test]
    fn offline_does_not_wait() {
        let gate = Arc::new(Gate::new(false, Duration::from_secs(10)));
        let g = Arc::clone(&gate);
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            g.set_offline(true);
        });
        let start = std::time::Instant::now();
        assert!(!gate.wait());
        assert!(start.elapsed() < Duration::from_secs(5));
        setter.join().unwrap();
        gate.set(true);
        assert!(!gate.wait(), "open but offline");
        gate.set_offline(false);
        assert!(gate.wait());
    }
}
