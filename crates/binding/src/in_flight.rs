//! Which components the checker is still answering, so a host that showed a component's structural props learns when
//! the checker's answer is kept and a request for it gets the checker's props (ADR 0022).
//!
//! A session asks its checker through [`Tracked`], which counts the requests in flight per component key and wakes
//! [`InFlight::settle`] when the last one is delivered. The delivery runs first: by the time a waiter wakes, the session
//! has kept the checker's answer (or refused it, for an edit made since the request).

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use caliper_docgen_core::types::semantic::{
    Delivery, SemanticOutcome, SemanticProvider, SemanticTarget, SharedPriority, SourceChange,
};

/// The checker's requests in flight, counted per component key.
#[derive(Default)]
pub(crate) struct InFlight {
    counts: Mutex<BTreeMap<String, usize>>,
    delivered: Condvar,
}

impl InFlight {
    fn begin(&self, keys: &[String]) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        for key in keys {
            *counts.entry(key.clone()).or_default() += 1;
        }
    }

    fn end(&self, keys: &[String]) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        for key in keys {
            if let Some(count) = counts.get_mut(key) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(key);
                }
            }
        }
        self.delivered.notify_all();
    }

    /// Waits up to `wait` until no request for any of `keys` is in flight: true when none is, false when one still was
    /// at the deadline. A key never asked for is settled at once.
    pub(crate) fn settle(&self, keys: &[String], wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if !keys.iter().any(|key| counts.contains_key(key)) {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) else {
                return false;
            };
            counts = self.delivered.wait_timeout(counts, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}

/// A checker whose requests made with [`SemanticProvider::request_at`] (every request a session makes) are counted in
/// an [`InFlight`] until delivered.
pub(crate) struct Tracked {
    inner: Arc<dyn SemanticProvider>,
    in_flight: Arc<InFlight>,
}

impl Tracked {
    pub(crate) fn new(inner: Arc<dyn SemanticProvider>, in_flight: Arc<InFlight>) -> Self {
        Self { inner, in_flight }
    }
}

impl SemanticProvider for Tracked {
    fn request(&self, targets: Vec<SemanticTarget>) -> std::sync::mpsc::Receiver<SemanticOutcome> {
        self.inner.request(targets)
    }

    fn request_at(&self, targets: Vec<SemanticTarget>, priority: SharedPriority, delivery: Delivery) {
        let keys: Vec<String> = targets.iter().map(|target| target.key.clone()).collect();
        self.in_flight.begin(&keys);
        let in_flight = self.in_flight.clone();
        // A delivery dropped unused still runs this, with `None`, so a request that is lost ends too.
        let counted = Delivery::new(move |outcome| {
            delivery.deliver(outcome);
            in_flight.end(&keys);
        });
        self.inner.request_at(targets, priority, counted);
    }

    fn changed(&self, change: SourceChange) {
        self.inner.changed(change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_settles_when_its_last_request_ends_and_not_before() {
        let in_flight = Arc::new(InFlight::default());
        let keys = vec!["Button".to_owned()];
        assert!(in_flight.settle(&keys, Duration::ZERO), "never asked: settled at once");

        in_flight.begin(&keys);
        in_flight.begin(&keys);
        assert!(!in_flight.settle(&keys, Duration::from_millis(5)));
        in_flight.end(&keys);
        assert!(!in_flight.settle(&keys, Duration::from_millis(5)), "one request is still in flight");

        let ending = in_flight.clone();
        let ender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            ending.end(&["Button".to_owned()]);
        });
        assert!(in_flight.settle(&keys, Duration::from_secs(10)), "woken by the last delivery");
        ender.join().unwrap();
        assert!(in_flight.settle(&["Card".to_owned()], Duration::ZERO));
    }
}
