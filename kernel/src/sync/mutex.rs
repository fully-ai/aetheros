//! Sleeping Mutex backed by atomic state and a `WaitQueue`.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::sync::waitqueue::WaitQueue;

pub struct Mutex<T> {
    locked: AtomicBool,
    owner_tid: AtomicU32,
    waitq: WaitQueue,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            owner_tid: AtomicU32::new(0),
            waitq: WaitQueue::new(),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        let tid = crate::sched::current_tid();
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            if crate::sched::is_scheduler_active() && crate::arch::x86_64::are_interrupts_enabled()
            {
                self.waitq.wait();
            } else {
                core::hint::spin_loop();
            }
        }
        self.owner_tid.store(tid, Ordering::Relaxed);
        MutexGuard { mutex: self }
    }

    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.owner_tid
                .store(crate::sched::current_tid(), Ordering::Relaxed);
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.mutex.owner_tid.store(0, Ordering::Relaxed);
        self.mutex.locked.store(false, Ordering::Release);
        self.mutex.waitq.wake_one();
    }
}
