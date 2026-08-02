//! Generic single-shot background operation.
//!
//! Replaces the hand-rolled `start_X` / `process_X_result` / `X_ing` /
//! `X_rx` quad that used to be copy-pasted per operation. One `AsyncOp<T>`
//! field replaces the `bool` + `Option<Receiver<T>>` pair, and [`AsyncOp::poll`]
//! guarantees the op is reset to idle on both completion and worker death —
//! the Disconnected-reset that was previously optional per copy.

use std::sync::mpsc;

/// A single in-flight background operation that produces one `T`.
///
/// "In flight" is derived from the presence of the receiver, so the flag and
/// the channel can never drift apart.
pub(crate) struct AsyncOp<T> {
    rx: Option<mpsc::Receiver<T>>,
}

/// Outcome of one [`AsyncOp::poll`] call.
pub(crate) enum AsyncPoll<T> {
    /// No operation is running.
    Idle,
    /// The operation is still running; check again next frame.
    Pending,
    /// The operation finished; the op is now idle again.
    Ready(T),
    /// The worker died without sending a result; the op is now idle again.
    Failed,
}

impl<T> Default for AsyncOp<T> {
    fn default() -> Self {
        Self { rx: None }
    }
}

impl<T: Send + 'static> AsyncOp<T> {
    /// Whether an operation is currently running.
    pub(crate) fn in_flight(&self) -> bool {
        self.rx.is_some()
    }

    /// Run `job` on a background thread unless an operation is already in
    /// flight. Returns `false` (and does nothing) when busy.
    pub(crate) fn spawn(&mut self, job: impl FnOnce() -> T + Send + 'static) -> bool {
        match self.arm() {
            Some(tx) => {
                std::thread::spawn(move || {
                    let _ = tx.send(job());
                });
                true
            }
            None => false,
        }
    }

    /// Like [`spawn`](Self::spawn), but runs `after` on the worker thread once
    /// the result has been sent (e.g. to request a UI repaint).
    pub(crate) fn spawn_then(
        &mut self,
        job: impl FnOnce() -> T + Send + 'static,
        after: impl FnOnce() + Send + 'static,
    ) -> bool {
        match self.arm() {
            Some(tx) => {
                std::thread::spawn(move || {
                    let _ = tx.send(job());
                    after();
                });
                true
            }
            None => false,
        }
    }

    /// Adopt a receiver created elsewhere (e.g. by a worker-spawning helper
    /// that builds its own channel). Returns `false` when already in flight.
    pub(crate) fn adopt(&mut self, rx: mpsc::Receiver<T>) -> bool {
        if self.rx.is_some() {
            return false;
        }
        self.rx = Some(rx);
        true
    }

    /// Mark the op as in flight and hand back the sender for a caller-managed
    /// worker. Returns `None` when an operation is already in flight.
    pub(crate) fn arm(&mut self) -> Option<mpsc::Sender<T>> {
        if self.rx.is_some() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        Some(tx)
    }

    /// Check the operation without blocking. On [`AsyncPoll::Ready`] and
    /// [`AsyncPoll::Failed`] the op is reset to idle, so a dead worker can
    /// never leave the op stuck "in flight".
    pub(crate) fn poll(&mut self) -> AsyncPoll<T> {
        let rx = match &self.rx {
            Some(rx) => rx,
            None => return AsyncPoll::Idle,
        };
        match rx.try_recv() {
            Ok(value) => {
                self.rx = None;
                AsyncPoll::Ready(value)
            }
            Err(mpsc::TryRecvError::Empty) => AsyncPoll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                AsyncPoll::Failed
            }
        }
    }

    /// Drop any in-flight operation; a late result is discarded.
    pub(crate) fn cancel(&mut self) {
        self.rx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_by_default() {
        let mut op: AsyncOp<i32> = AsyncOp::default();
        assert!(!op.in_flight());
        assert!(matches!(op.poll(), AsyncPoll::Idle));
    }

    #[test]
    fn spawn_delivers_result_and_resets() {
        let mut op: AsyncOp<i32> = AsyncOp::default();
        assert!(op.spawn(|| 42));
        assert!(op.in_flight());
        loop {
            match op.poll() {
                AsyncPoll::Pending => std::thread::yield_now(),
                AsyncPoll::Ready(v) => {
                    assert_eq!(v, 42);
                    break;
                }
                _ => panic!("unexpected poll state"),
            }
        }
        assert!(!op.in_flight());
        assert!(matches!(op.poll(), AsyncPoll::Idle));
    }

    #[test]
    fn spawn_refuses_while_in_flight() {
        let mut op: AsyncOp<i32> = AsyncOp::default();
        let tx = op.arm().expect("armed");
        assert!(!op.spawn(|| 1));
        assert!(op.arm().is_none());
        drop(tx);
    }

    #[test]
    fn dead_worker_resets_to_idle() {
        let mut op: AsyncOp<i32> = AsyncOp::default();
        let tx = op.arm().expect("armed");
        drop(tx); // worker died without sending
        assert!(matches!(op.poll(), AsyncPoll::Failed));
        assert!(!op.in_flight());
        assert!(matches!(op.poll(), AsyncPoll::Idle));
    }

    #[test]
    fn cancel_discards_late_result() {
        let mut op: AsyncOp<i32> = AsyncOp::default();
        let tx = op.arm().expect("armed");
        op.cancel();
        assert!(!op.in_flight());
        let _ = tx.send(7);
        assert!(matches!(op.poll(), AsyncPoll::Idle));
    }
}
