//! Counting Semaphore and Read-Write Lock synchronization primitives.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use crate::sync::waitqueue::WaitQueue;

/// Counting semaphore supporting `acquire()` (`P`) and `release()` (`V`).
pub struct Semaphore {
    count: AtomicUsize,
    waitq: WaitQueue,
}

impl Semaphore {
    pub const fn new(initial: usize) -> Self {
        Self {
            count: AtomicUsize::new(initial),
            waitq: WaitQueue::new(),
        }
    }

    pub fn acquire(&self) {
        loop {
            let cur = self.count.load(Ordering::Acquire);
            if cur > 0 {
                if self
                    .count
                    .compare_exchange(cur, cur - 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return;
                }
            } else if crate::sched::is_scheduler_active()
                && crate::arch::x86_64::are_interrupts_enabled()
            {
                self.waitq.wait();
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_acquire(&self) -> bool {
        let mut cur = self.count.load(Ordering::Acquire);
        while cur > 0 {
            match self.count.compare_exchange_weak(
                cur,
                cur - 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => cur = actual,
            }
        }
        false
    }

    pub fn release(&self) {
        self.count.fetch_add(1, Ordering::Release);
        self.waitq.wake_one();
    }

    pub fn available(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}

/// Reader-Writer Lock (`state >= 0`: active reader count, `state == -1`: active writer).
pub struct RwLock<T> {
    state: AtomicI32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicI32::new(0),
            data: UnsafeCell::new(value),
        }
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let s = self.state.load(Ordering::Relaxed);
            if s >= 0 {
                if self
                    .state
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    return RwLockReadGuard { lock: self };
                }
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        while self
            .state
            .compare_exchange_weak(0, -1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        RwLockWriteGuard { lock: self }
    }
}

pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> Drop for RwLockReadGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for RwLockWriteGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
    }
}
