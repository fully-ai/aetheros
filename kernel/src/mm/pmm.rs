//! Physical Memory Manager (PMM) — Bitmap Frame Allocator.
//!
//! Manages 4 KiB physical page frames discovered from the bootloader memory map.
//! Features:
//! - Dynamically sized bitmap placed directly in physical RAM via HHDM (no static RAM cap).
//! - Fail-closed initialization: all frames start marked allocated; only page-aligned interiors
//!   of `Usable` regions are released.
//! - Frame `0` (`0x0000`) is permanently reserved.
//! - Contiguous multi-frame allocation for DMA and multi-page buffers.
//! - Double-free detection and fragmentation diagnostics.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::boot::{BootInfo, MemoryRegionKind};
use crate::mm::{align_down, align_up, PAGE_SIZE};
use crate::sync::IrqSpinLock;

static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Relaxed)
}

/// Convert a physical address to its Higher-Half Direct Map (HHDM) virtual address.
#[inline]
pub fn phys_to_virt(phys: u64) -> *mut u8 {
    (phys.wrapping_add(hhdm_offset())) as *mut u8
}

/// Convert an HHDM virtual address back to its physical address.
#[inline]
pub fn virt_to_phys_hhdm(virt: u64) -> Option<u64> {
    let off = hhdm_offset();
    if virt >= off {
        Some(virt - off)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmmError {
    OutOfMemory,
    UnalignedAddress,
    OutOfRange,
    ReservedFrame,
    DoubleFree,
    InvalidCount,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PmmStats {
    pub total_frames: u64,
    pub usable_frames: u64,
    pub free_frames: u64,
    pub allocated_frames: u64,
    pub reserved_bytes: u64,
    pub bitmap_bytes: u64,
    pub alloc_ops: u64,
    pub free_ops: u64,
    pub double_free_detected: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FragmentationReport {
    pub free_frames: u64,
    pub free_runs: u64,
    pub largest_free_run_frames: u64,
}

struct BitmapAllocator {
    initialized: bool,
    bitmap_ptr: *mut u8,
    bitmap_len: usize,
    bitmap_start_frame: u64,
    bitmap_frame_count: u64,
    total_frames: u64,
    usable_frames: u64,
    free_frames: u64,
    allocated_frames: u64,
    reserved_bytes: u64,
    next_hint: u64,
    alloc_ops: u64,
    free_ops: u64,
    double_free_detected: u64,
}

unsafe impl Send for BitmapAllocator {}

impl BitmapAllocator {
    const fn new() -> Self {
        Self {
            initialized: false,
            bitmap_ptr: core::ptr::null_mut(),
            bitmap_len: 0,
            bitmap_start_frame: 0,
            bitmap_frame_count: 0,
            total_frames: 0,
            usable_frames: 0,
            free_frames: 0,
            allocated_frames: 0,
            reserved_bytes: 0,
            next_hint: 1,
            alloc_ops: 0,
            free_ops: 0,
            double_free_detected: 0,
        }
    }

    #[inline]
    fn get_bit(&self, frame: u64) -> bool {
        let byte_idx = (frame / 8) as usize;
        let bit_idx = (frame % 8) as u8;
        let byte = unsafe { self.bitmap_ptr.add(byte_idx).read() };
        (byte & (1u8 << bit_idx)) != 0
    }

    #[inline]
    fn set_bit(&mut self, frame: u64) {
        let byte_idx = (frame / 8) as usize;
        let bit_idx = (frame % 8) as u8;
        unsafe {
            let p = self.bitmap_ptr.add(byte_idx);
            p.write(p.read() | (1u8 << bit_idx));
        }
    }

    #[inline]
    fn clear_bit(&mut self, frame: u64) {
        let byte_idx = (frame / 8) as usize;
        let bit_idx = (frame % 8) as u8;
        unsafe {
            let p = self.bitmap_ptr.add(byte_idx);
            p.write(p.read() & !(1u8 << bit_idx));
        }
    }

    fn init(&mut self, boot: &BootInfo) -> Result<(), &'static str> {
        HHDM_OFFSET.store(boot.hhdm_offset, Ordering::Relaxed);

        let regions = &boot.memmap[..boot.memmap_count];
        let mut max_usable_end: u64 = 0;
        let mut reserved_bytes: u64 = 0;

        for r in regions {
            if r.length == 0 {
                continue;
            }
            if r.kind == MemoryRegionKind::Usable {
                let end = r.base.saturating_add(r.length);
                if end > max_usable_end {
                    max_usable_end = end;
                }
            } else {
                reserved_bytes = reserved_bytes.saturating_add(r.length);
            }
        }

        if max_usable_end < (PAGE_SIZE as u64) * 16 {
            return Err("PMM: insufficient usable physical memory reported by bootloader");
        }

        let total_frames = align_up(max_usable_end, PAGE_SIZE as u64) / (PAGE_SIZE as u64);
        let bitmap_bytes = total_frames.div_ceil(8);
        let bitmap_aligned_bytes = align_up(bitmap_bytes, PAGE_SIZE as u64);
        let bitmap_frames = bitmap_aligned_bytes / (PAGE_SIZE as u64);

        // Locate a usable physical region large enough to hold the bitmap (never at phys 0).
        let mut bitmap_phys: Option<u64> = None;
        for r in regions {
            if r.kind != MemoryRegionKind::Usable {
                continue;
            }
            let start = align_up(r.base.max(PAGE_SIZE as u64), PAGE_SIZE as u64);
            let end = align_down(r.base.saturating_add(r.length), PAGE_SIZE as u64);
            if end > start && (end - start) >= bitmap_aligned_bytes {
                bitmap_phys = Some(start);
                break;
            }
        }

        let bitmap_phys =
            bitmap_phys.ok_or("PMM: no usable physical memory region large enough for bitmap")?;
        let bitmap_virt = phys_to_virt(bitmap_phys);

        // Fail-closed: mark ALL frames allocated (0xFF) first.
        unsafe {
            core::ptr::write_bytes(bitmap_virt, 0xFF, bitmap_aligned_bytes as usize);
        }

        self.bitmap_ptr = bitmap_virt;
        self.bitmap_len = bitmap_bytes as usize;
        self.bitmap_start_frame = bitmap_phys / (PAGE_SIZE as u64);
        self.bitmap_frame_count = bitmap_frames;
        self.total_frames = total_frames;
        self.reserved_bytes = reserved_bytes;
        self.usable_frames = 0;
        self.free_frames = 0;

        // Release strictly page-aligned interiors of Usable regions (excluding frame 0).
        for r in regions {
            if r.kind != MemoryRegionKind::Usable || r.length == 0 {
                continue;
            }
            let start = align_up(r.base.max(PAGE_SIZE as u64), PAGE_SIZE as u64);
            let end = align_down(r.base.saturating_add(r.length), PAGE_SIZE as u64);
            if end <= start {
                continue;
            }
            let first_frame = start / (PAGE_SIZE as u64);
            let last_frame = (end / (PAGE_SIZE as u64)).min(total_frames);
            for frame in first_frame..last_frame {
                if self.get_bit(frame) {
                    self.clear_bit(frame);
                    self.usable_frames += 1;
                    self.free_frames += 1;
                }
            }
        }

        // Re-reserve frame 0 and the bitmap's own physical backing frames.
        self.set_bit(0);
        for offset in 0..bitmap_frames {
            let f = self.bitmap_start_frame + offset;
            if !self.get_bit(f) {
                self.set_bit(f);
                self.free_frames = self.free_frames.saturating_sub(1);
            }
        }

        self.allocated_frames = self.usable_frames.saturating_sub(self.free_frames);
        self.next_hint = 1;
        self.initialized = true;
        Ok(())
    }

    fn alloc_frame(&mut self) -> Result<u64, PmmError> {
        if !self.initialized || self.free_frames == 0 {
            return Err(PmmError::OutOfMemory);
        }

        // Fast scan from hint to end, then wrap around from 1 to hint.
        if let Some(frame) = self.find_free_in_range(self.next_hint, self.total_frames) {
            return Ok(self.commit_alloc(frame));
        }
        if let Some(frame) = self.find_free_in_range(1, self.next_hint) {
            return Ok(self.commit_alloc(frame));
        }

        Err(PmmError::OutOfMemory)
    }

    fn find_free_in_range(&self, start_frame: u64, end_frame: u64) -> Option<u64> {
        let mut frame = start_frame;
        // Skip fully-occupied bytes quickly (8 frames at a time)
        while frame < end_frame {
            if frame.is_multiple_of(8) && frame + 8 <= end_frame {
                let byte = unsafe { self.bitmap_ptr.add((frame / 8) as usize).read() };
                if byte == 0xFF {
                    frame += 8;
                    continue;
                }
            }
            if !self.get_bit(frame) {
                return Some(frame);
            }
            frame += 1;
        }
        None
    }

    fn commit_alloc(&mut self, frame: u64) -> u64 {
        self.set_bit(frame);
        self.free_frames -= 1;
        self.allocated_frames += 1;
        self.alloc_ops += 1;
        self.next_hint = (frame + 1) % self.total_frames;
        if self.next_hint == 0 {
            self.next_hint = 1;
        }
        frame * (PAGE_SIZE as u64)
    }

    fn alloc_contiguous(&mut self, count: usize) -> Result<u64, PmmError> {
        if count == 0 {
            return Err(PmmError::InvalidCount);
        }
        if count == 1 {
            return self.alloc_frame();
        }
        let needed = count as u64;
        if !self.initialized || self.free_frames < needed {
            return Err(PmmError::OutOfMemory);
        }

        let mut run_start = 1u64;
        let mut run_len = 0u64;
        let mut f = 1u64;
        while f < self.total_frames {
            if !self.get_bit(f) {
                if run_len == 0 {
                    run_start = f;
                }
                run_len += 1;
                if run_len == needed {
                    for i in 0..needed {
                        self.set_bit(run_start + i);
                    }
                    self.free_frames -= needed;
                    self.allocated_frames += needed;
                    self.alloc_ops += 1;
                    self.next_hint = (run_start + needed).min(self.total_frames - 1);
                    return Ok(run_start * (PAGE_SIZE as u64));
                }
            } else {
                run_len = 0;
            }
            f += 1;
        }

        Err(PmmError::OutOfMemory)
    }

    fn free_frame(&mut self, phys: u64) -> Result<(), PmmError> {
        if !self.initialized {
            return Err(PmmError::OutOfRange);
        }
        if (phys & (PAGE_SIZE as u64 - 1)) != 0 {
            return Err(PmmError::UnalignedAddress);
        }
        let frame = phys / (PAGE_SIZE as u64);
        if frame == 0
            || (frame >= self.bitmap_start_frame
                && frame < self.bitmap_start_frame + self.bitmap_frame_count)
        {
            return Err(PmmError::ReservedFrame);
        }
        if frame >= self.total_frames {
            return Err(PmmError::OutOfRange);
        }
        if !self.get_bit(frame) {
            self.double_free_detected += 1;
            return Err(PmmError::DoubleFree);
        }

        self.clear_bit(frame);
        self.free_frames += 1;
        self.allocated_frames = self.allocated_frames.saturating_sub(1);
        self.free_ops += 1;
        if frame < self.next_hint {
            self.next_hint = frame;
        }
        Ok(())
    }

    fn stats(&self) -> PmmStats {
        PmmStats {
            total_frames: self.total_frames,
            usable_frames: self.usable_frames,
            free_frames: self.free_frames,
            allocated_frames: self.allocated_frames,
            reserved_bytes: self.reserved_bytes,
            bitmap_bytes: self.bitmap_len as u64,
            alloc_ops: self.alloc_ops,
            free_ops: self.free_ops,
            double_free_detected: self.double_free_detected,
        }
    }

    fn fragmentation_report(&self) -> FragmentationReport {
        let mut free_runs = 0u64;
        let mut largest = 0u64;
        let mut current = 0u64;
        for f in 1..self.total_frames {
            if !self.get_bit(f) {
                if current == 0 {
                    free_runs += 1;
                }
                current += 1;
                if current > largest {
                    largest = current;
                }
            } else {
                current = 0;
            }
        }
        FragmentationReport {
            free_frames: self.free_frames,
            free_runs,
            largest_free_run_frames: largest,
        }
    }
}

static PMM: IrqSpinLock<BitmapAllocator> = IrqSpinLock::new(BitmapAllocator::new());

const MAX_SHARED_FRAMES: usize = 4096;

#[derive(Clone, Copy)]
struct SharedFrameEntry {
    phys: u64,
    refs: u32,
}

struct SharedRefTable {
    entries: [SharedFrameEntry; MAX_SHARED_FRAMES],
}

impl SharedRefTable {
    const fn new() -> Self {
        Self {
            entries: [SharedFrameEntry { phys: 0, refs: 0 }; MAX_SHARED_FRAMES],
        }
    }

    fn get_refs(&self, phys: u64) -> u32 {
        if phys == 0 {
            return 0;
        }
        for e in &self.entries {
            if e.phys == phys {
                return e.refs;
            }
        }
        1
    }

    fn inc_ref(&mut self, phys: u64) -> u32 {
        for e in &mut self.entries {
            if e.phys == phys {
                e.refs = e.refs.saturating_add(1);
                return e.refs;
            }
        }
        for e in &mut self.entries {
            if e.phys == 0 {
                e.phys = phys;
                e.refs = 2;
                return 2;
            }
        }
        panic!("PMM: SharedRefTable capacity exhausted");
    }

    /// Decrement refcount if shared (`refs >= 2`). Returns `true` if the frame is still
    /// referenced by at least one other mapping and therefore must NOT be freed in the bitmap yet.
    fn dec_ref_should_keep(&mut self, phys: u64) -> bool {
        for e in &mut self.entries {
            if e.phys == phys {
                if e.refs > 2 {
                    e.refs -= 1;
                    return true;
                } else {
                    // Drops from 2 -> 1 (unique owner again)
                    e.phys = 0;
                    e.refs = 0;
                    return false;
                }
            }
        }
        false
    }
}

static SHARED_REFS: IrqSpinLock<SharedRefTable> = IrqSpinLock::new(SharedRefTable::new());

pub fn frame_refcount(phys: u64) -> u32 {
    SHARED_REFS.lock().get_refs(phys)
}

pub fn inc_frame_ref(phys: u64) -> u32 {
    SHARED_REFS.lock().inc_ref(phys)
}

pub fn init(boot: &BootInfo) -> Result<PmmStats, &'static str> {
    let mut alloc = PMM.lock();
    alloc.init(boot)?;
    Ok(alloc.stats())
}

pub fn alloc_frame() -> Result<u64, PmmError> {
    PMM.lock().alloc_frame()
}

pub fn alloc_zeroed_frame() -> Result<u64, PmmError> {
    let phys = alloc_frame()?;
    unsafe {
        core::ptr::write_bytes(phys_to_virt(phys), 0, PAGE_SIZE);
    }
    Ok(phys)
}

pub fn alloc_contiguous(count: usize) -> Result<u64, PmmError> {
    PMM.lock().alloc_contiguous(count)
}

pub fn free_frame(phys: u64) -> Result<(), PmmError> {
    if (phys & (PAGE_SIZE as u64 - 1)) != 0 {
        return Err(PmmError::UnalignedAddress);
    }
    if phys != 0 && SHARED_REFS.lock().dec_ref_should_keep(phys) {
        return Ok(());
    }
    PMM.lock().free_frame(phys)
}

pub fn free_contiguous(base_phys: u64, count: usize) -> Result<(), PmmError> {
    for i in 0..count {
        free_frame(base_phys + (i as u64) * (PAGE_SIZE as u64))?;
    }
    Ok(())
}

pub fn stats() -> PmmStats {
    PMM.lock().stats()
}

pub fn fragmentation_report() -> FragmentationReport {
    PMM.lock().fragmentation_report()
}
