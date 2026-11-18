//! Virtual Memory Manager (VMM) — Address Spaces, Dynamic Kernel Mappings,
//! Demand Paging, Copy-on-Write (`COW`), and Guard Page `#PF` Resolution.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::cpu::write_cr3;
use crate::arch::x86_64::idt::{self, TrapFrame};
use crate::arch::x86_64::paging::{
    self, active_pml4_phys, PageTable, PagingError, PTE_ADDR_MASK, PTE_COW, PTE_DEMAND,
    PTE_FLAGS_MASK, PTE_GUARD, PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE,
};
use crate::mm::pmm::{self, phys_to_virt};
use crate::mm::PAGE_SIZE;

/// Base virtual address for the kernel heap (`0xFFFF_9000_0000_0000`, PML4 index 288).
pub const KERNEL_HEAP_BASE: u64 = 0xFFFF_9000_0000_0000;
/// Maximum virtual size of the kernel heap (64 MiB).
pub const KERNEL_HEAP_MAX_SIZE: u64 = 64 * 1024 * 1024;

/// Base virtual address for dynamic kernel virtual allocations / stacks / MMIO (`PML4` index 290).
pub const KERNEL_VMALLOC_BASE: u64 = 0xFFFF_9100_0000_0000;

static KERNEL_PML4_PHYS: AtomicU64 = AtomicU64::new(0);
static NEXT_VMALLOC_VADDR: AtomicU64 = AtomicU64::new(KERNEL_VMALLOC_BASE);

static DEMAND_FAULTS_RESOLVED: AtomicU64 = AtomicU64::new(0);
static COW_FAULTS_RESOLVED: AtomicU64 = AtomicU64::new(0);
static GUARD_FAULTS_CAUGHT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub struct VmmStats {
    pub kernel_pml4_phys: u64,
    pub active_pml4_phys: u64,
    pub demand_faults_resolved: u64,
    pub cow_faults_resolved: u64,
    pub guard_faults_caught: u64,
}

/// Represents an isolated 4-level x86_64 virtual address space (backed by a physical PML4 frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpace {
    pub pml4_phys: u64,
}

impl AddressSpace {
    #[inline]
    pub const fn from_raw(pml4_phys: u64) -> Self {
        Self { pml4_phys }
    }

    /// Return the master kernel `AddressSpace`.
    #[inline]
    pub fn kernel() -> Self {
        Self {
            pml4_phys: KERNEL_PML4_PHYS.load(Ordering::Relaxed),
        }
    }

    /// Return the currently active `AddressSpace` (`CR3`).
    #[inline]
    pub fn active() -> Self {
        Self {
            pml4_phys: active_pml4_phys(),
        }
    }

    /// Create a fresh, isolated user `AddressSpace` sharing kernel higher-half mappings (`256..512`).
    pub fn new_user() -> Result<Self, PagingError> {
        let pml4_phys = paging::create_isolated_user_pml4(Self::kernel().pml4_phys)?;
        Ok(Self { pml4_phys })
    }

    /// Clone this user `AddressSpace` using Copy-on-Write (`PTE_COW`) for writable user pages.
    pub fn clone_cow(&self) -> Result<Self, PagingError> {
        let pml4_phys = paging::clone_user_pml4_cow(self.pml4_phys)?;
        Ok(Self { pml4_phys })
    }

    /// Switch the current CPU's `CR3` to this `AddressSpace`.
    pub fn activate(&self) {
        if active_pml4_phys() != self.pml4_phys {
            unsafe {
                write_cr3(self.pml4_phys);
            }
        }
    }

    pub fn map_page(&self, vaddr: u64, paddr: u64, flags: u64) -> Result<(), PagingError> {
        paging::map_page_in(self.pml4_phys, vaddr, paddr, flags)
    }

    pub fn remap_page(&self, vaddr: u64, paddr: u64, flags: u64) -> Result<u64, PagingError> {
        paging::remap_page_in(self.pml4_phys, vaddr, paddr, flags)
    }

    pub fn unmap_page(&self, vaddr: u64) -> Result<(u64, u64), PagingError> {
        paging::unmap_page_in(self.pml4_phys, vaddr)
    }

    /// Unmap a page and immediately release its backing physical frame to the PMM.
    pub fn unmap_and_free_page(&self, vaddr: u64) -> Result<(), PagingError> {
        let (phys, flags) = self.unmap_page(vaddr)?;
        if (flags & PTE_PRESENT) != 0 && phys != 0 {
            let _ = pmm::free_frame(phys);
        }
        Ok(())
    }

    pub fn protect_page(&self, vaddr: u64, new_flags: u64) -> Result<(), PagingError> {
        paging::protect_page_in(self.pml4_phys, vaddr, new_flags)
    }

    pub fn map_demand_page(&self, vaddr: u64, flags: u64) -> Result<(), PagingError> {
        paging::mark_demand_page_in(self.pml4_phys, vaddr, flags)
    }

    pub fn map_guard_page(&self, vaddr: u64) -> Result<(), PagingError> {
        paging::mark_guard_page_in(self.pml4_phys, vaddr)
    }

    pub fn translate(&self, vaddr: u64) -> Option<(u64, u64)> {
        paging::translate_in(self.pml4_phys, vaddr)
    }

    /// Tear down and free all user-space mappings (`0..256`) and the PML4 itself.
    pub fn destroy_user(self) {
        if self.pml4_phys != Self::kernel().pml4_phys {
            paging::destroy_user_pml4(self.pml4_phys);
        }
    }
}

/// Initialize the Virtual Memory Manager, pre-populate shared higher-half kernel PML4 slots,
/// and register the `#PF` demand/COW/guard page-fault handler with the IDT.
pub fn init() -> Result<VmmStats, &'static str> {
    let pml4_phys = active_pml4_phys();
    if pml4_phys == 0 {
        return Err("VMM: CR3 physical address is zero");
    }
    KERNEL_PML4_PHYS.store(pml4_phys, Ordering::Relaxed);

    // Pre-populate higher-half PML4 slots 288..304 (covering KERNEL_HEAP_BASE and KERNEL_VMALLOC_BASE)
    // so that every user PML4 cloned later automatically shares those kernel PDPT tables.
    let pml4 = unsafe { &mut *(phys_to_virt(pml4_phys) as *mut PageTable) };
    for entry in pml4.entries.iter_mut().take(304).skip(288) {
        if (*entry & PTE_PRESENT) == 0 {
            let pdpt_phys = pmm::alloc_zeroed_frame()
                .map_err(|_| "VMM: out of physical frames while pre-allocating kernel PDPTs")?;
            *entry = (pdpt_phys & PTE_ADDR_MASK) | PTE_PRESENT | PTE_WRITABLE;
        }
    }

    idt::register_page_fault_hook(handle_page_fault);

    Ok(stats())
}

/// Reserve a contiguous range of `page_count` virtual pages in the kernel `vmalloc` region.
pub fn alloc_kernel_virt_range(page_count: usize) -> u64 {
    let bytes = (page_count as u64) * (PAGE_SIZE as u64);
    NEXT_VMALLOC_VADDR.fetch_add(bytes, Ordering::Relaxed)
}

/// Allocate a kernel stack of `stack_pages` usable 4 KiB pages preceded by an unmapped
/// 4 KiB Guard Page (`PTE_GUARD`). Returns the top-of-stack virtual address (16-byte aligned).
pub fn alloc_guarded_kernel_stack(stack_pages: usize) -> Result<u64, PagingError> {
    let total_pages = stack_pages + 1;
    let base_vaddr = alloc_kernel_virt_range(total_pages);
    let aspace = AddressSpace::kernel();

    // Bottom page is an unmapped guard page
    aspace.map_guard_page(base_vaddr)?;

    // Remaining pages are RW + NX kernel stack pages
    for i in 1..total_pages {
        let vaddr = base_vaddr + (i as u64) * (PAGE_SIZE as u64);
        let phys = pmm::alloc_zeroed_frame()?;
        aspace.map_page(vaddr, phys, PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE)?;
    }

    let top = base_vaddr + (total_pages as u64) * (PAGE_SIZE as u64);
    Ok(top)
}

pub fn stats() -> VmmStats {
    VmmStats {
        kernel_pml4_phys: KERNEL_PML4_PHYS.load(Ordering::Relaxed),
        active_pml4_phys: active_pml4_phys(),
        demand_faults_resolved: DEMAND_FAULTS_RESOLVED.load(Ordering::Relaxed),
        cow_faults_resolved: COW_FAULTS_RESOLVED.load(Ordering::Relaxed),
        guard_faults_caught: GUARD_FAULTS_CAUGHT.load(Ordering::Relaxed),
    }
}

/// IDT `#PF` (vector 14) page-fault callback.
/// Returns `Some(frame)` if the page fault was transparently resolved (Demand Paging or COW),
/// or `None` if it is a genuine invalid memory access / guard-page violation.
fn handle_page_fault(frame: &mut TrapFrame, fault_addr: u64) -> Option<*mut TrapFrame> {
    if !paging::is_canonical(fault_addr) {
        return None;
    }

    let pml4_phys = active_pml4_phys();
    let page_vaddr = fault_addr & !(PAGE_SIZE as u64 - 1);
    let raw_pte = paging::read_raw_pte_in(pml4_phys, page_vaddr)?;

    let is_present = (raw_pte & PTE_PRESENT) != 0;
    let is_write_fault = (frame.error_code & (1 << 1)) != 0;
    let is_user_fault = (frame.error_code & (1 << 2)) != 0;
    let is_instr_fetch = (frame.error_code & (1 << 4)) != 0;

    // Case 1: Guard Page violation (`PTE_PRESENT == 0` and `PTE_GUARD != 0`)
    if !is_present && (raw_pte & PTE_GUARD) != 0 {
        GUARD_FAULTS_CAUGHT.fetch_add(1, Ordering::Relaxed);
        crate::kwarn!(
            "VMM: Guard page hit at fault_addr={:#018x} (page={:#018x}, RIP={:#018x})",
            fault_addr,
            page_vaddr,
            frame.rip
        );
        return None;
    }

    // Case 2: Lazy Demand-Zero page (`PTE_PRESENT == 0` and `PTE_DEMAND != 0`)
    if !is_present && (raw_pte & PTE_DEMAND) != 0 {
        // Verify privilege & W^X rules before resolving
        if is_user_fault && (raw_pte & PTE_USER) == 0 {
            return None;
        }
        if is_instr_fetch && (raw_pte & PTE_NO_EXECUTE) != 0 {
            return None;
        }
        let phys = pmm::alloc_zeroed_frame().ok()?;
        let new_flags = (raw_pte & PTE_FLAGS_MASK & !PTE_DEMAND) | PTE_PRESENT;
        paging::remap_page_in(pml4_phys, page_vaddr, phys, new_flags).ok()?;
        DEMAND_FAULTS_RESOLVED.fetch_add(1, Ordering::Relaxed);
        return Some(frame as *mut TrapFrame);
    }

    // Case 3: Copy-on-Write (`PTE_PRESENT == 1`, `PTE_COW != 0`, write fault)
    if is_present && is_write_fault && (raw_pte & PTE_COW) != 0 {
        if is_user_fault && (raw_pte & PTE_USER) == 0 {
            return None;
        }
        let old_phys = raw_pte & PTE_ADDR_MASK;
        let new_flags = ((raw_pte & PTE_FLAGS_MASK) & !PTE_COW) | PTE_WRITABLE | PTE_PRESENT;

        if pmm::frame_refcount(old_phys) <= 1 {
            // Sole remaining owner: simply restore PTE_WRITABLE and clear PTE_COW in-place
            paging::remap_page_in(pml4_phys, page_vaddr, old_phys, new_flags).ok()?;
        } else {
            // Shared with another process: allocate private copy, copy 4 KiB, drop old ref
            let new_phys = pmm::alloc_frame().ok()?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    phys_to_virt(old_phys),
                    phys_to_virt(new_phys),
                    PAGE_SIZE,
                );
            }
            paging::remap_page_in(pml4_phys, page_vaddr, new_phys, new_flags).ok()?;
            let _ = pmm::free_frame(old_phys);
        }
        COW_FAULTS_RESOLVED.fetch_add(1, Ordering::Relaxed);
        return Some(frame as *mut TrapFrame);
    }

    None
}
