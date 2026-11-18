//! Kernel spinlock primitives (interrupt-safe and raw ticket spinlocks).

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

/// Fair ticket spinlock (does NOT mask interrupts).
/// Prefer `IrqSpinLock` for any data touched by interrupt handlers.
pub struct SpinLock<T> {
    next_ticket: AtomicU32,
    now_serving: AtomicU32,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            next_ticket: AtomicU32::new(0),
            now_serving: AtomicU32::new(0),
            data: UnsafeCell::new(value),
        }
    }

    #[inline]
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        while self.now_serving.load(Ordering::Acquire) != ticket {
            spin_loop();
        }
        SpinLockGuard { lock: self }
    }

    #[inline]
    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let current = self.now_serving.load(Ordering::Acquire);
        if self
            .next_ticket
            .compare_exchange(
                current,
                current.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            Some(SpinLockGuard { lock: self })
        } else {
            None
        }
    }

    /// Force-unlock (ONLY for fatal panic paths to avoid deadlocking the console).
    ///
    /// # Safety
    /// Caller must be in a single-CPU crash/panic context with interrupts disabled.
    pub unsafe fn force_unlock(&self) {
        let next = self.next_ticket.load(Ordering::Relaxed);
        self.now_serving.store(next, Ordering::Release);
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinLockGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.now_serving.fetch_add(1, Ordering::Release);
    }
}

/// Interrupt-safe ticket spinlock: disables local CPU interrupts (`cli`) while held
/// and restores the caller's previous `RFLAGS.IF` state on drop.
pub struct IrqSpinLock<T> {
    inner: SpinLock<T>,
}

unsafe impl<T: Send> Sync for IrqSpinLock<T> {}
unsafe impl<T: Send> Send for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }

    #[inline]
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        let irq_was_enabled = crate::arch::x86_64::are_interrupts_enabled();
        crate::arch::x86_64::cli();
        let guard = core::mem::ManuallyDrop::new(self.inner.lock());
        IrqSpinLockGuard {
            guard,
            irq_was_enabled,
        }
    }

    #[inline]
    pub fn try_lock(&self) -> Option<IrqSpinLockGuard<'_, T>> {
        let irq_was_enabled = crate::arch::x86_64::are_interrupts_enabled();
        crate::arch::x86_64::cli();
        match self.inner.try_lock() {
            Some(guard) => Some(IrqSpinLockGuard {
                guard: core::mem::ManuallyDrop::new(guard),
                irq_was_enabled,
            }),
            None => {
                if irq_was_enabled {
                    crate::arch::x86_64::sti();
                }
                None
            }
        }
    }

    /// Force-unlock for fatal panic reporting only.
    ///
    /// # Safety
    /// Must only be invoked when halting after a fatal kernel panic.
    pub unsafe fn force_unlock(&self) {
        unsafe { self.inner.force_unlock() };
    }
}

pub struct IrqSpinLockGuard<'a, T> {
    guard: core::mem::ManuallyDrop<SpinLockGuard<'a, T>>,
    irq_was_enabled: bool,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `self.guard` is never accessed again after this point.
        // We MUST release the underlying spinlock BEFORE restoring interrupts (`sti`)
        // to prevent an IRQ arriving while the lock is still held.
        unsafe {
            core::mem::ManuallyDrop::drop(&mut self.guard);
        }
        if self.irq_was_enabled {
            crate::arch::x86_64::sti();
        }
    }
}
