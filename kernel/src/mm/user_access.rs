//! User-Space Pointer Validation and Safe Kernel<->User Memory Copy Primitives.
//!
//! Security Rules (Section 6 & 16.1):
//! 1. Never trust any pointer or buffer length originating from Ring 3.
//! 2. Reject null/near-null pointers (`< USER_ADDR_MIN`) and any pointer whose end overflows
//!    or reaches `USER_ADDR_MAX` (`0x0000_8000_0000_0000`) with `Errno::Efault`.
//! 3. Walk the active user page tables for every 4 KiB page spanned by `[ptr, ptr + len)`
//!    (transparently resolving lazy `PTE_DEMAND` or write-target `PTE_COW` pages) and verify
//!    `PTE_PRESENT | PTE_USER` (plus `PTE_WRITABLE` for destination buffers) BEFORE copying.

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use aether_abi::{Errno, USER_ADDR_MAX, USER_ADDR_MIN};

use crate::arch::x86_64::paging::{
    self, active_pml4_phys, PTE_ADDR_MASK, PTE_COW, PTE_DEMAND, PTE_FLAGS_MASK, PTE_PRESENT,
    PTE_USER, PTE_WRITABLE,
};
use crate::mm::pmm::{self, phys_to_virt};
use crate::mm::PAGE_SIZE;

/// Validate that `[user_ptr, user_ptr + len)` lies strictly within canonical user virtual space
/// and that every spanned 4 KiB page is mapped with `PTE_USER` (and `PTE_WRITABLE` if `writable`).
pub fn validate_user_range(user_ptr: u64, len: usize, writable: bool) -> Result<(), Errno> {
    if len == 0 {
        return Ok(());
    }
    let start = user_ptr as usize;
    if start < USER_ADDR_MIN {
        return Err(Errno::Efault);
    }
    let end = start.checked_add(len).ok_or(Errno::Efault)?;
    if end > USER_ADDR_MAX {
        return Err(Errno::Efault);
    }

    let pml4_phys = active_pml4_phys();
    let first_page = (start & !(PAGE_SIZE - 1)) as u64;
    let last_page = ((end - 1) & !(PAGE_SIZE - 1)) as u64;

    let mut page = first_page;
    while page <= last_page {
        ensure_user_page_accessible(pml4_phys, page, writable)?;
        page += PAGE_SIZE as u64;
    }

    Ok(())
}

fn ensure_user_page_accessible(
    pml4_phys: u64,
    page_vaddr: u64,
    writable: bool,
) -> Result<(), Errno> {
    let raw_pte = paging::read_raw_pte_in(pml4_phys, page_vaddr).ok_or(Errno::Efault)?;

    // Resolve lazy demand-zero user page if present=0 and demand=1
    if (raw_pte & PTE_PRESENT) == 0 {
        if (raw_pte & PTE_DEMAND) != 0 && (raw_pte & PTE_USER) != 0 {
            if writable && (raw_pte & PTE_WRITABLE) == 0 {
                return Err(Errno::Efault);
            }
            let phys = pmm::alloc_zeroed_frame().map_err(|_| Errno::Enomem)?;
            let new_flags = (raw_pte & PTE_FLAGS_MASK & !PTE_DEMAND) | PTE_PRESENT;
            paging::remap_page_in(pml4_phys, page_vaddr, phys, new_flags)
                .map_err(|_| Errno::Efault)?;
            return Ok(());
        }
        return Err(Errno::Efault);
    }

    if (raw_pte & PTE_USER) == 0 {
        return Err(Errno::Efault);
    }

    if writable && (raw_pte & PTE_WRITABLE) == 0 {
        // Resolve Copy-on-Write user page if needed
        if (raw_pte & PTE_COW) != 0 {
            let old_phys = raw_pte & PTE_ADDR_MASK;
            let new_flags = ((raw_pte & PTE_FLAGS_MASK) & !PTE_COW) | PTE_WRITABLE | PTE_PRESENT;
            if pmm::frame_refcount(old_phys) <= 1 {
                paging::remap_page_in(pml4_phys, page_vaddr, old_phys, new_flags)
                    .map_err(|_| Errno::Efault)?;
            } else {
                let new_phys = pmm::alloc_frame().map_err(|_| Errno::Enomem)?;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        phys_to_virt(old_phys),
                        phys_to_virt(new_phys),
                        PAGE_SIZE,
                    );
                }
                paging::remap_page_in(pml4_phys, page_vaddr, new_phys, new_flags)
                    .map_err(|_| Errno::Efault)?;
                let _ = pmm::free_frame(old_phys);
            }
            return Ok(());
        }
        return Err(Errno::Efault);
    }

    Ok(())
}

/// Copy `dst.len()` bytes from validated user pointer `user_src` into kernel buffer `dst`.
pub fn copy_from_user(dst: &mut [u8], user_src: u64) -> Result<(), Errno> {
    if dst.is_empty() {
        return Ok(());
    }
    validate_user_range(user_src, dst.len(), false)?;
    unsafe {
        core::ptr::copy_nonoverlapping(user_src as *const u8, dst.as_mut_ptr(), dst.len());
    }
    Ok(())
}

/// Copy `src.len()` bytes from kernel slice `src` into validated user buffer `user_dst`.
pub fn copy_to_user(user_dst: u64, src: &[u8]) -> Result<(), Errno> {
    if src.is_empty() {
        return Ok(());
    }
    validate_user_range(user_dst, src.len(), true)?;
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), user_dst as *mut u8, src.len());
    }
    Ok(())
}

/// Read a `Copy` struct `T` from validated user pointer `user_src`.
pub fn read_user_struct<T: Copy>(user_src: u64) -> Result<T, Errno> {
    validate_user_range(user_src, size_of::<T>(), false)?;
    Ok(unsafe { (user_src as *const T).read_unaligned() })
}

/// Write a `Copy` struct `T` to validated user pointer `user_dst`.
pub fn write_user_struct<T: Copy>(user_dst: u64, val: &T) -> Result<(), Errno> {
    validate_user_range(user_dst, size_of::<T>(), true)?;
    unsafe {
        (user_dst as *mut T).write_unaligned(*val);
    }
    Ok(())
}

/// Copy a NUL-terminated UTF-8 string of up to `max_len` bytes from user space.
pub fn copy_user_cstr(user_ptr: u64, max_len: usize) -> Result<String, Errno> {
    let start = user_ptr as usize;
    if !(USER_ADDR_MIN..USER_ADDR_MAX).contains(&start) {
        return Err(Errno::Efault);
    }
    let pml4_phys = active_pml4_phys();
    let mut bytes = Vec::new();
    let mut cur_page = u64::MAX;

    for offset in 0..max_len {
        let addr = start.checked_add(offset).ok_or(Errno::Efault)?;
        if addr >= USER_ADDR_MAX {
            return Err(Errno::Efault);
        }
        let page = (addr & !(PAGE_SIZE - 1)) as u64;
        if page != cur_page {
            ensure_user_page_accessible(pml4_phys, page, false)?;
            cur_page = page;
        }
        let b = unsafe { *(addr as *const u8) };
        if b == 0 {
            return String::from_utf8(bytes).map_err(|_| Errno::Einval);
        }
        bytes.push(b);
    }

    Err(Errno::Enametoolong)
}
