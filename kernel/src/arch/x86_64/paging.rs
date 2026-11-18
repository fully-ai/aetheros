//! x86_64 4-Level Paging Engine (PML4 -> PDPT -> PD -> PT).
//!
//! Enforces:
//! - W^X (Write XOR Execute): rejects mappings that are both writable and executable.
//! - NX (No-Execute) bit (`1 << 63`).
//! - Software PTE flags in bits 9..11:
//!   - `PTE_COW`    (`1 << 9`):  Copy-on-Write shared read-only page.
//!   - `PTE_DEMAND` (`1 << 10`): Lazy demand-zero page (`PRESENT=0`).
//!   - `PTE_GUARD`  (`1 << 11`): Unmapped stack/buffer guard page (`PRESENT=0`).

use crate::arch::x86_64::cpu::{invlpg, read_cr3};
use crate::mm::pmm::{self, phys_to_virt, PmmError};
use crate::mm::PAGE_SIZE;

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITABLE: u64 = 1 << 1;
pub const PTE_USER: u64 = 1 << 2;
pub const PTE_WRITE_THROUGH: u64 = 1 << 3;
pub const PTE_CACHE_DISABLE: u64 = 1 << 4;
pub const PTE_ACCESSED: u64 = 1 << 5;
pub const PTE_DIRTY: u64 = 1 << 6;
pub const PTE_HUGE_PAGE: u64 = 1 << 7;
pub const PTE_GLOBAL: u64 = 1 << 8;

/// Software-defined PTE bit 9: Copy-on-Write page.
pub const PTE_COW: u64 = 1 << 9;
/// Software-defined PTE bit 10: Lazy Demand-Zero page (`PTE_PRESENT == 0`).
pub const PTE_DEMAND: u64 = 1 << 10;
/// Software-defined PTE bit 11: Guard page (`PTE_PRESENT == 0`).
pub const PTE_GUARD: u64 = 1 << 11;

/// Hardware No-Execute bit (bit 63, requires `EFER.NXE=1`).
pub const PTE_NO_EXECUTE: u64 = 1 << 63;

/// Mask extracting the 4 KiB-aligned physical frame address from a 64-bit PTE (bits 12..51).
pub const PTE_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Mask extracting permission/software flags from a 64-bit PTE.
pub const PTE_FLAGS_MASK: u64 = !PTE_ADDR_MASK;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingError {
    OutOfMemory,
    UnalignedAddress,
    NonCanonicalAddress,
    WxViolation,
    AlreadyMapped,
    NotMapped,
    HugePageConflict,
}

impl From<PmmError> for PagingError {
    fn from(_: PmmError) -> Self {
        Self::OutOfMemory
    }
}

#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [u64; 512],
}

#[inline]
pub const fn is_canonical(vaddr: u64) -> bool {
    let sign_ext = ((vaddr as i64) << 16) >> 16;
    (sign_ext as u64) == vaddr
}

#[inline]
pub const fn pml4_index(vaddr: u64) -> usize {
    ((vaddr >> 39) & 0x1FF) as usize
}

#[inline]
pub const fn pdpt_index(vaddr: u64) -> usize {
    ((vaddr >> 30) & 0x1FF) as usize
}

#[inline]
pub const fn pd_index(vaddr: u64) -> usize {
    ((vaddr >> 21) & 0x1FF) as usize
}

#[inline]
pub const fn pt_index(vaddr: u64) -> usize {
    ((vaddr >> 12) & 0x1FF) as usize
}

#[inline]
pub fn active_pml4_phys() -> u64 {
    read_cr3() & PTE_ADDR_MASK
}

#[inline]
pub fn validate_wx(flags: u64) -> Result<(), PagingError> {
    let writable = (flags & PTE_WRITABLE) != 0;
    let executable = (flags & PTE_NO_EXECUTE) == 0;
    if writable && executable {
        return Err(PagingError::WxViolation);
    }
    Ok(())
}

#[inline]
unsafe fn table_at_mut(table_phys: u64) -> &'static mut PageTable {
    unsafe { &mut *(phys_to_virt(table_phys) as *mut PageTable) }
}

#[inline]
unsafe fn table_at(table_phys: u64) -> &'static PageTable {
    unsafe { &*(phys_to_virt(table_phys) as *const PageTable) }
}

/// Walk from `pml4_phys` down to the 4 KiB Page Table (`PT`) slot for `vaddr`,
/// allocating intermediate page tables if `create == true`.
fn walk_to_pte_slot(
    pml4_phys: u64,
    vaddr: u64,
    user_accessible: bool,
    create: bool,
) -> Result<*mut u64, PagingError> {
    if !is_canonical(vaddr) {
        return Err(PagingError::NonCanonicalAddress);
    }
    let indices = [pml4_index(vaddr), pdpt_index(vaddr), pd_index(vaddr)];
    let parent_flags = PTE_PRESENT | PTE_WRITABLE | if user_accessible { PTE_USER } else { 0 };

    let mut current_table_phys = pml4_phys;
    for &idx in &indices {
        let table = unsafe { table_at_mut(current_table_phys) };
        let entry = table.entries[idx];
        if (entry & PTE_PRESENT) != 0 {
            if (entry & PTE_HUGE_PAGE) != 0 {
                return Err(PagingError::HugePageConflict);
            }
            // Upgrade parent entry to allow user traversal if mapping a user page
            if user_accessible && (entry & PTE_USER) == 0 {
                table.entries[idx] = entry | PTE_USER | PTE_WRITABLE;
            }
            current_table_phys = entry & PTE_ADDR_MASK;
        } else {
            if !create {
                return Err(PagingError::NotMapped);
            }
            let new_table_phys = pmm::alloc_zeroed_frame()?;
            table.entries[idx] = (new_table_phys & PTE_ADDR_MASK) | parent_flags;
            current_table_phys = new_table_phys;
        }
    }

    let pt = unsafe { table_at_mut(current_table_phys) };
    Ok(&mut pt.entries[pt_index(vaddr)] as *mut u64)
}

/// Map a single 4 KiB virtual page `vaddr` to physical frame `paddr` with `flags` (W^X enforced).
pub fn map_page_in(pml4_phys: u64, vaddr: u64, paddr: u64, flags: u64) -> Result<(), PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 || (paddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    validate_wx(flags)?;

    let user = (flags & PTE_USER) != 0;
    let slot = walk_to_pte_slot(pml4_phys, vaddr, user, true)?;
    let old = unsafe { slot.read() };
    if (old & PTE_PRESENT) != 0 {
        return Err(PagingError::AlreadyMapped);
    }

    let new_pte = (paddr & PTE_ADDR_MASK) | (flags & PTE_FLAGS_MASK) | PTE_PRESENT;
    unsafe { slot.write(new_pte) };

    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok(())
}

/// Replace or create a 4 KiB mapping at `vaddr` (used by demand paging and COW resolution).
pub fn remap_page_in(
    pml4_phys: u64,
    vaddr: u64,
    paddr: u64,
    flags: u64,
) -> Result<u64, PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 || (paddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    validate_wx(flags)?;
    let user = (flags & PTE_USER) != 0;
    let slot = walk_to_pte_slot(pml4_phys, vaddr, user, true)?;
    let old = unsafe { slot.read() };
    let new_pte = (paddr & PTE_ADDR_MASK) | (flags & PTE_FLAGS_MASK) | PTE_PRESENT;
    unsafe { slot.write(new_pte) };
    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok(old)
}

/// Unmap a 4 KiB virtual page `vaddr`, returning `(old_phys_frame, old_flags)`.
/// Does NOT free the physical frame automatically (caller decides whether to free).
pub fn unmap_page_in(pml4_phys: u64, vaddr: u64) -> Result<(u64, u64), PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    let slot = walk_to_pte_slot(pml4_phys, vaddr, false, false)?;
    let old = unsafe { slot.read() };
    if (old & (PTE_PRESENT | PTE_DEMAND | PTE_GUARD)) == 0 {
        return Err(PagingError::NotMapped);
    }
    unsafe { slot.write(0) };
    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok((old & PTE_ADDR_MASK, old & PTE_FLAGS_MASK))
}

/// Update permission flags on an existing mapped 4 KiB page (W^X enforced).
pub fn protect_page_in(pml4_phys: u64, vaddr: u64, new_flags: u64) -> Result<(), PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    validate_wx(new_flags)?;
    let slot = walk_to_pte_slot(pml4_phys, vaddr, (new_flags & PTE_USER) != 0, false)?;
    let old = unsafe { slot.read() };
    if (old & PTE_PRESENT) == 0 {
        return Err(PagingError::NotMapped);
    }
    let phys = old & PTE_ADDR_MASK;
    unsafe {
        slot.write(phys | (new_flags & PTE_FLAGS_MASK) | PTE_PRESENT);
    }
    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok(())
}

/// Mark a 4 KiB virtual page as a lazy Demand-Zero page (`PTE_PRESENT=0`, `PTE_DEMAND=1`).
/// When first accessed, the `#PF` handler allocates a zeroed frame and maps it with `flags`.
pub fn mark_demand_page_in(pml4_phys: u64, vaddr: u64, flags: u64) -> Result<(), PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    validate_wx(flags)?;
    let user = (flags & PTE_USER) != 0;
    let slot = walk_to_pte_slot(pml4_phys, vaddr, user, true)?;
    let old = unsafe { slot.read() };
    if (old & PTE_PRESENT) != 0 {
        return Err(PagingError::AlreadyMapped);
    }
    let pte = ((flags & PTE_FLAGS_MASK) | PTE_DEMAND) & !PTE_PRESENT;
    unsafe { slot.write(pte) };
    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok(())
}

/// Mark a 4 KiB virtual page as an unmapped Guard Page (`PTE_PRESENT=0`, `PTE_GUARD=1`).
pub fn mark_guard_page_in(pml4_phys: u64, vaddr: u64) -> Result<(), PagingError> {
    if (vaddr & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PagingError::UnalignedAddress);
    }
    let slot = walk_to_pte_slot(pml4_phys, vaddr, false, true)?;
    unsafe { slot.write(PTE_GUARD) };
    if pml4_phys == active_pml4_phys() {
        invlpg(vaddr);
    }
    Ok(())
}

/// Inspect the raw 4 KiB leaf PTE value for `vaddr` (even if `PTE_PRESENT == 0`).
pub fn read_raw_pte_in(pml4_phys: u64, vaddr: u64) -> Option<u64> {
    let page_vaddr = vaddr & !(PAGE_SIZE as u64 - 1);
    let slot = walk_to_pte_slot(pml4_phys, page_vaddr, false, false).ok()?;
    Some(unsafe { slot.read() })
}

/// Translate virtual address `vaddr` in `pml4_phys` to `(phys_addr, effective_flags)`.
/// Supports 4 KiB pages, 2 MiB huge pages, and 1 GiB huge pages.
pub fn translate_in(pml4_phys: u64, vaddr: u64) -> Option<(u64, u64)> {
    if !is_canonical(vaddr) {
        return None;
    }
    let pml4 = unsafe { table_at(pml4_phys) };
    let e4 = pml4.entries[pml4_index(vaddr)];
    if (e4 & PTE_PRESENT) == 0 {
        return None;
    }

    let pdpt = unsafe { table_at(e4 & PTE_ADDR_MASK) };
    let e3 = pdpt.entries[pdpt_index(vaddr)];
    if (e3 & PTE_PRESENT) == 0 {
        return None;
    }
    if (e3 & PTE_HUGE_PAGE) != 0 {
        // 1 GiB huge page
        let base = e3 & 0x000F_FFFF_C000_0000;
        let offset = vaddr & 0x3FFF_FFFF;
        return Some((base + offset, e3 & PTE_FLAGS_MASK));
    }

    let pd = unsafe { table_at(e3 & PTE_ADDR_MASK) };
    let e2 = pd.entries[pd_index(vaddr)];
    if (e2 & PTE_PRESENT) == 0 {
        return None;
    }
    if (e2 & PTE_HUGE_PAGE) != 0 {
        // 2 MiB huge page
        let base = e2 & 0x000F_FFFF_FFE0_0000;
        let offset = vaddr & 0x001F_FFFF;
        return Some((base + offset, e2 & PTE_FLAGS_MASK));
    }

    let pt = unsafe { table_at(e2 & PTE_ADDR_MASK) };
    let e1 = pt.entries[pt_index(vaddr)];
    if (e1 & PTE_PRESENT) == 0 {
        return None;
    }
    let base = e1 & PTE_ADDR_MASK;
    let offset = vaddr & 0xFFF;
    Some((base + offset, e1 & PTE_FLAGS_MASK))
}

/// Create a fresh user PML4 table that inherits all higher-half kernel mappings (`256..512`)
/// and has an empty, isolated lower half (`0..256`).
pub fn create_isolated_user_pml4(kernel_pml4_phys: u64) -> Result<u64, PagingError> {
    let new_pml4_phys = pmm::alloc_zeroed_frame()?;
    let src = unsafe { table_at(kernel_pml4_phys) };
    let dst = unsafe { table_at_mut(new_pml4_phys) };
    dst.entries[256..512].copy_from_slice(&src.entries[256..512]);
    Ok(new_pml4_phys)
}

/// Clone a user address space (`0..256` PML4 entries) using Copy-on-Write (`PTE_COW`).
/// Writable user pages in both parent (`src_pml4_phys`) and child (`dst_pml4_phys`) are
/// demoted to read-only with `PTE_COW` set, and their physical frame refcount is incremented.
pub fn clone_user_pml4_cow(src_pml4_phys: u64) -> Result<u64, PagingError> {
    let dst_pml4_phys = create_isolated_user_pml4(src_pml4_phys)?;
    let is_active = src_pml4_phys == active_pml4_phys();

    for i4 in 0..256usize {
        let src_pml4 = unsafe { table_at_mut(src_pml4_phys) };
        let e4 = src_pml4.entries[i4];
        if (e4 & PTE_PRESENT) == 0 {
            continue;
        }
        let dst_pdpt_phys = pmm::alloc_zeroed_frame()?;
        let dst_pml4 = unsafe { table_at_mut(dst_pml4_phys) };
        dst_pml4.entries[i4] = (dst_pdpt_phys & PTE_ADDR_MASK) | (e4 & PTE_FLAGS_MASK);

        let src_pdpt_phys = e4 & PTE_ADDR_MASK;
        for i3 in 0..512usize {
            let src_pdpt = unsafe { table_at_mut(src_pdpt_phys) };
            let e3 = src_pdpt.entries[i3];
            if (e3 & PTE_PRESENT) == 0 || (e3 & PTE_HUGE_PAGE) != 0 {
                continue;
            }
            let dst_pd_phys = pmm::alloc_zeroed_frame()?;
            let dst_pdpt = unsafe { table_at_mut(dst_pdpt_phys) };
            dst_pdpt.entries[i3] = (dst_pd_phys & PTE_ADDR_MASK) | (e3 & PTE_FLAGS_MASK);

            let src_pd_phys = e3 & PTE_ADDR_MASK;
            for i2 in 0..512usize {
                let src_pd = unsafe { table_at_mut(src_pd_phys) };
                let e2 = src_pd.entries[i2];
                if (e2 & PTE_PRESENT) == 0 || (e2 & PTE_HUGE_PAGE) != 0 {
                    continue;
                }
                let dst_pt_phys = pmm::alloc_zeroed_frame()?;
                let dst_pd = unsafe { table_at_mut(dst_pd_phys) };
                dst_pd.entries[i2] = (dst_pt_phys & PTE_ADDR_MASK) | (e2 & PTE_FLAGS_MASK);

                let src_pt_phys = e2 & PTE_ADDR_MASK;
                let src_pt = unsafe { table_at_mut(src_pt_phys) };
                let dst_pt = unsafe { table_at_mut(dst_pt_phys) };

                for i1 in 0..512usize {
                    let pte = src_pt.entries[i1];
                    if (pte & PTE_PRESENT) != 0 {
                        let frame_phys = pte & PTE_ADDR_MASK;
                        let mut new_pte = pte;
                        if (pte & PTE_WRITABLE) != 0 || (pte & PTE_COW) != 0 {
                            new_pte = (pte & !PTE_WRITABLE) | PTE_COW;
                            src_pt.entries[i1] = new_pte;
                            if is_active {
                                let vaddr = ((i4 as u64) << 39)
                                    | ((i3 as u64) << 30)
                                    | ((i2 as u64) << 21)
                                    | ((i1 as u64) << 12);
                                invlpg(vaddr);
                            }
                        }
                        pmm::inc_frame_ref(frame_phys);
                        dst_pt.entries[i1] = new_pte;
                    } else if (pte & (PTE_DEMAND | PTE_GUARD)) != 0 {
                        dst_pt.entries[i1] = pte;
                    }
                }
            }
        }
    }

    Ok(dst_pml4_phys)
}

/// Free all user-half (`0..256`) mapped frames and intermediate page tables, then free `pml4_phys`.
pub fn destroy_user_pml4(pml4_phys: u64) {
    if pml4_phys == 0 || pml4_phys == active_pml4_phys() {
        return;
    }
    let pml4 = unsafe { table_at(pml4_phys) };
    for &e4 in pml4.entries.iter().take(256) {
        if (e4 & PTE_PRESENT) == 0 {
            continue;
        }
        let pdpt_phys = e4 & PTE_ADDR_MASK;
        let pdpt = unsafe { table_at(pdpt_phys) };
        for &e3 in &pdpt.entries {
            if (e3 & PTE_PRESENT) == 0 || (e3 & PTE_HUGE_PAGE) != 0 {
                continue;
            }
            let pd_phys = e3 & PTE_ADDR_MASK;
            let pd = unsafe { table_at(pd_phys) };
            for &e2 in &pd.entries {
                if (e2 & PTE_PRESENT) == 0 || (e2 & PTE_HUGE_PAGE) != 0 {
                    continue;
                }
                let pt_phys = e2 & PTE_ADDR_MASK;
                let pt = unsafe { table_at(pt_phys) };
                for &e1 in &pt.entries {
                    if (e1 & PTE_PRESENT) != 0 {
                        let _ = pmm::free_frame(e1 & PTE_ADDR_MASK);
                    }
                }
                let _ = pmm::free_frame(pt_phys);
            }
            let _ = pmm::free_frame(pd_phys);
        }
        let _ = pmm::free_frame(pdpt_phys);
    }
    let _ = pmm::free_frame(pml4_phys);
}
