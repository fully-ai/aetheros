//! Wait Queue primitive for blocking and waking threads on kernel events.

use crate::sync::IrqSpinLock;
use alloc::collections::VecDeque;

pub struct WaitQueue {
    waiters: IrqSpinLock<VecDeque<u32>>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            waiters: IrqSpinLock::new(VecDeque::new()),
        }
    }

    /// Enqueue the currently running task and block until woken.
    pub fn wait(&self) {
        let tid = crate::sched::current_tid();
        if tid == 0 {
            crate::sched::yield_now();
            return;
        }
        {
            let mut q = self.waiters.lock();
            if !q.contains(&tid) {
                q.push_back(tid);
            }
        }
        crate::sched::block_current_task();
    }

    /// Wake up to one waiting task. Returns the woken TID if any.
    pub fn wake_one(&self) -> Option<u32> {
        let tid = self.waiters.lock().pop_front()?;
        crate::sched::wake_task(tid);
        Some(tid)
    }

    /// Wake all currently queued tasks. Returns the number of tasks woken.
    pub fn wake_all(&self) -> usize {
        let mut tids = VecDeque::new();
        {
            let mut q = self.waiters.lock();
            core::mem::swap(&mut *q, &mut tids);
        }
        let count = tids.len();
        for tid in tids {
            crate::sched::wake_task(tid);
        }
        count
    }

    pub fn waiter_count(&self) -> usize {
        self.waiters.lock().len()
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}
