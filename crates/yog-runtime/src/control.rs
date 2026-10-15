use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default)]
pub struct RunControl {
    cancellation: CancellationToken,
    batch: Arc<Mutex<BatchControl>>,
}

#[derive(Debug, Default)]
struct BatchControl {
    active: Option<ActiveTask>,
    remaining_skips: usize,
}

#[derive(Debug)]
struct ActiveTask {
    index: usize,
    cancellation: CancellationToken,
}

impl RunControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn skip_tasks(&self, expected_index: usize, count: NonZeroUsize) -> bool {
        if self.is_cancelled() {
            return false;
        }
        let mut batch = self.batch.lock().unwrap();
        let Some(active) = &mut batch.active else {
            return false;
        };
        if active.index != expected_index || active.cancellation.is_cancelled() {
            return false;
        }
        active.cancellation.cancel();
        batch.remaining_skips = count.get() - 1;
        true
    }

    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn begin_task(&self, index: usize) -> CancellationToken {
        let mut batch = self.batch.lock().unwrap();
        assert!(batch.active.is_none(), "batch tasks execute serially");
        let cancellation = self.cancellation.child_token();
        batch.active = Some(ActiveTask {
            index,
            cancellation: cancellation.clone(),
        });
        cancellation
    }

    pub(crate) fn finish_task(&self) {
        self.batch
            .lock()
            .unwrap()
            .active
            .take()
            .expect("task has begun");
    }

    pub(crate) fn take_skip(&self) -> bool {
        let mut batch = self.batch.lock().unwrap();
        if batch.remaining_skips == 0 {
            return false;
        }
        batch.remaining_skips -= 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_task_request_does_not_cancel_the_next_task() {
        let control = RunControl::new();
        let first = control.begin_task(1);
        assert!(control.skip_tasks(1, NonZeroUsize::new(1).unwrap()));
        assert!(first.is_cancelled());
        control.finish_task();
        let second = control.begin_task(2);
        assert!(!control.skip_tasks(1, NonZeroUsize::new(1).unwrap()));
        assert!(!second.is_cancelled());
        assert!(!control.is_cancelled());
    }

    #[test]
    fn repeated_skip_requests_do_not_replace_the_accepted_quota() {
        let control = RunControl::new();
        control.begin_task(1);
        assert!(control.skip_tasks(1, NonZeroUsize::new(2).unwrap()));
        assert!(!control.skip_tasks(1, NonZeroUsize::new(20).unwrap()));
        control.finish_task();
        assert!(control.take_skip());
        assert!(!control.take_skip());
    }
}
