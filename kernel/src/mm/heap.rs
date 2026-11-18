//! Kernel Heap Allocator (`#[global_allocator]`).
//!
//! Design:
//! - Maps physical frames dynamically into `[KERNEL_HEAP_BASE, KERNEL_HEAP_BASE + KERNEL_HEAP_MAX_SIZE)`
//!   with `PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE` (strictly non-executable W^X!).
//! - Coalescing block allocator with 16-byte minimum alignment, arbitrary power-of-two
//!   alignment support, header magic canaries (`MAGIC_FREE` / `MAGIC_ALLOC`), double-free
//!   detection, and live fragmentation/leak accounting.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::size_of;
use core::ptr::null_mut;

use crate::arch::x86_64::paging::{PTE_NO_EXECUTE, PTE_PRESENT, PTE_WRITABLE};
use crate::mm::pmm;
use crate::mm::vmm::{AddressSpace, KERNEL_HEAP_BASE, KERNEL_HEAP_MAX_SIZE};
use crate::mm::{align_up, PAGE_SIZE};
use crate::sync::IrqSpinLock;

const INITIAL_HEAP_SIZE: usize = 2 * 1024 * 1024; // 2 MiB initial mapped heap
const HEAP_GROW_CHUNK: usize = 1024 * 1024; // 1 MiB growth step
const MIN_SPLIT_REMAINDER: usize = size_of::<BlockHeader>() + 32;

const MAGIC_FREE: u64 = 0xAE74_E005_F8EE_B10C;
const MAGIC_ALLOC: u64 = 0xAE74_E005_A110_CA7E;

#[repr(C, align(16))]
struct BlockHeader {
    magic: u64,
    /// Total byte size of this block INCLUDING `size_of::<BlockHeader>()`.
    total_size: usize,
    is_free: bool,
    prev: *mut BlockHeader,
    next: *mut BlockHeader,
}

const _: () = assert!(size_of::<BlockHeader>().is_multiple_of(16));

#[derive(Debug, Clone, Copy, Default)]
pub struct HeapStats {
    pub mapped_bytes: usize,
    pub used_bytes: usize,
    pub free_bytes: usize,
    pub peak_used_bytes: usize,
    pub active_allocations: usize,
    pub alloc_count: u64,
    pub free_count: u64,
    pub free_blocks: usize,
    pub largest_free_block: usize,
    pub double_free_detected: u64,
}

struct CoalescingHeap {
    initialized: bool,
    base_vaddr: u64,
    mapped_bytes: usize,
    head: *mut BlockHeader,
    tail: *mut BlockHeader,
    used_bytes: usize,
    peak_used_bytes: usize,
    active_allocations: usize,
    alloc_count: u64,
    free_count: u64,
    double_free_detected: u64,
}

unsafe impl Send for CoalescingHeap {}

impl CoalescingHeap {
    const fn new() -> Self {
        Self {
            initialized: false,
            base_vaddr: KERNEL_HEAP_BASE,
            mapped_bytes: 0,
            head: null_mut(),
            tail: null_mut(),
            used_bytes: 0,
            peak_used_bytes: 0,
            active_allocations: 0,
            alloc_count: 0,
            free_count: 0,
            double_free_detected: 0,
        }
    }

    fn init(&mut self) -> Result<HeapStats, &'static str> {
        self.map_additional_bytes(INITIAL_HEAP_SIZE)?;
        let first = self.base_vaddr as *mut BlockHeader;
        unsafe {
            first.write(BlockHeader {
                magic: MAGIC_FREE,
                total_size: INITIAL_HEAP_SIZE,
                is_free: true,
                prev: null_mut(),
                next: null_mut(),
            });
        }
        self.head = first;
        self.tail = first;
        self.initialized = true;
        Ok(self.stats())
    }

    fn map_additional_bytes(&mut self, add_bytes: usize) -> Result<(), &'static str> {
        let aligned = (add_bytes + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if (self.mapped_bytes + aligned) as u64 > KERNEL_HEAP_MAX_SIZE {
            return Err("Kernel heap exhausted KERNEL_HEAP_MAX_SIZE");
        }
        let start_vaddr = self.base_vaddr + (self.mapped_bytes as u64);
        let pages = aligned / PAGE_SIZE;
        let aspace = AddressSpace::kernel();
        let flags = PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE;

        for i in 0..pages {
            let vaddr = start_vaddr + (i as u64) * (PAGE_SIZE as u64);
            let phys = pmm::alloc_zeroed_frame()
                .map_err(|_| "Kernel heap: out of physical memory frames")?;
            aspace
                .map_page(vaddr, phys, flags)
                .map_err(|_| "Kernel heap: VMM page table mapping failed")?;
        }
        self.mapped_bytes += aligned;
        Ok(())
    }

    fn grow(&mut self, min_extra: usize) -> bool {
        let chunk = min_extra.max(HEAP_GROW_CHUNK);
        let aligned = (chunk + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let new_block_vaddr = self.base_vaddr + (self.mapped_bytes as u64);
        if self.map_additional_bytes(aligned).is_err() {
            return false;
        }

        let new_hdr = new_block_vaddr as *mut BlockHeader;
        unsafe {
            new_hdr.write(BlockHeader {
                magic: MAGIC_FREE,
                total_size: aligned,
                is_free: true,
                prev: self.tail,
                next: null_mut(),
            });
            if !self.tail.is_null() {
                (*self.tail).next = new_hdr;
            }
            self.tail = new_hdr;
            // Coalesce with previous tail if it was free
            if !(*new_hdr).prev.is_null() && (*(*new_hdr).prev).is_free {
                let prev = (*new_hdr).prev;
                (*prev).total_size += (*new_hdr).total_size;
                (*prev).next = null_mut();
                self.tail = prev;
            }
        }
        true
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized {
            return null_mut();
        }
        let align = layout.align().max(16);
        let size = align_up(layout.size().max(16) as u64, 16) as usize;

        if let Some(ptr) = self.try_alloc_from_list(size, align) {
            return ptr;
        }

        // Grow heap and retry once
        if self.grow(size + align + size_of::<BlockHeader>() * 2) {
            if let Some(ptr) = self.try_alloc_from_list(size, align) {
                return ptr;
            }
        }
        null_mut()
    }

    fn try_alloc_from_list(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let mut curr = self.head;
        while !curr.is_null() {
            let hdr = unsafe { &mut *curr };
            assert!(
                hdr.magic == MAGIC_FREE || hdr.magic == MAGIC_ALLOC,
                "Kernel heap corruption detected at block {:p} (magic={:#x})",
                curr,
                hdr.magic
            );

            if hdr.is_free {
                let block_start = curr as usize;
                let block_end = block_start + hdr.total_size;
                let payload_unaligned = block_start + size_of::<BlockHeader>();
                let payload_aligned = align_up(payload_unaligned as u64, align as u64) as usize;
                let front_gap = payload_aligned - payload_unaligned;

                // If alignment requires a front gap, ensure the front gap can form a valid free block
                let effective_payload = if front_gap == 0 || front_gap >= MIN_SPLIT_REMAINDER {
                    payload_aligned
                } else {
                    align_up(
                        (payload_unaligned + MIN_SPLIT_REMAINDER) as u64,
                        align as u64,
                    ) as usize
                };

                if effective_payload + size <= block_end {
                    let actual_front_gap = effective_payload - payload_unaligned;
                    let target_hdr_ptr = if actual_front_gap >= MIN_SPLIT_REMAINDER {
                        // Split front alignment padding into its own free block
                        let split_hdr =
                            (effective_payload - size_of::<BlockHeader>()) as *mut BlockHeader;
                        let orig_next = hdr.next;
                        let orig_total = hdr.total_size;
                        hdr.total_size = actual_front_gap;
                        unsafe {
                            split_hdr.write(BlockHeader {
                                magic: MAGIC_FREE,
                                total_size: orig_total - actual_front_gap,
                                is_free: true,
                                prev: curr,
                                next: orig_next,
                            });
                            if !orig_next.is_null() {
                                (*orig_next).prev = split_hdr;
                            } else {
                                self.tail = split_hdr;
                            }
                        }
                        hdr.next = split_hdr;
                        split_hdr
                    } else {
                        curr
                    };

                    let target = unsafe { &mut *target_hdr_ptr };
                    let needed_total = size_of::<BlockHeader>() + size;

                    // Split trailing remainder if large enough
                    if target.total_size >= needed_total + MIN_SPLIT_REMAINDER {
                        let rem_ptr =
                            ((target_hdr_ptr as usize) + needed_total) as *mut BlockHeader;
                        let rem_size = target.total_size - needed_total;
                        let next_ptr = target.next;
                        unsafe {
                            rem_ptr.write(BlockHeader {
                                magic: MAGIC_FREE,
                                total_size: rem_size,
                                is_free: true,
                                prev: target_hdr_ptr,
                                next: next_ptr,
                            });
                            if !next_ptr.is_null() {
                                (*next_ptr).prev = rem_ptr;
                            } else {
                                self.tail = rem_ptr;
                            }
                        }
                        target.total_size = needed_total;
                        target.next = rem_ptr;
                    }

                    target.is_free = false;
                    target.magic = MAGIC_ALLOC;
                    self.used_bytes += target.total_size;
                    if self.used_bytes > self.peak_used_bytes {
                        self.peak_used_bytes = self.used_bytes;
                    }
                    self.active_allocations += 1;
                    self.alloc_count += 1;

                    let user_ptr =
                        ((target_hdr_ptr as usize) + size_of::<BlockHeader>()) as *mut u8;
                    return Some(user_ptr);
                }
            }
            curr = hdr.next;
        }
        None
    }

    fn dealloc(&mut self, ptr: *mut u8) -> Result<(), &'static str> {
        if ptr.is_null() {
            return Ok(());
        }
        let addr = ptr as usize;
        let heap_start = (self.base_vaddr as usize) + size_of::<BlockHeader>();
        let heap_end = (self.base_vaddr as usize) + self.mapped_bytes;
        if addr < heap_start || addr >= heap_end || !addr.is_multiple_of(16) {
            return Err("Kernel heap: pointer out of heap bounds or unaligned");
        }

        let hdr_ptr = (addr - size_of::<BlockHeader>()) as *mut BlockHeader;
        let hdr = unsafe { &mut *hdr_ptr };

        if hdr.magic == MAGIC_FREE || hdr.is_free {
            self.double_free_detected += 1;
            return Err("Kernel heap: double-free detected");
        }
        if hdr.magic != MAGIC_ALLOC {
            return Err("Kernel heap: corrupted block header magic on free");
        }

        hdr.is_free = true;
        hdr.magic = MAGIC_FREE;
        self.used_bytes = self.used_bytes.saturating_sub(hdr.total_size);
        self.active_allocations = self.active_allocations.saturating_sub(1);
        self.free_count += 1;

        // Poison freed payload in debug/alpha builds to catch use-after-free immediately
        let payload_len = hdr.total_size - size_of::<BlockHeader>();
        unsafe {
            core::ptr::write_bytes(ptr, 0xDF, payload_len.min(256));
        }

        // Coalesce forward if `next` is adjacent and free
        unsafe {
            let next = (*hdr_ptr).next;
            if !next.is_null()
                && (*next).is_free
                && (hdr_ptr as usize) + (*hdr_ptr).total_size == (next as usize)
            {
                assert_eq!((*next).magic, MAGIC_FREE, "Heap corruption in next block");
                (*hdr_ptr).total_size += (*next).total_size;
                let next_next = (*next).next;
                (*hdr_ptr).next = next_next;
                if !next_next.is_null() {
                    (*next_next).prev = hdr_ptr;
                } else {
                    self.tail = hdr_ptr;
                }
            }

            // Coalesce backward if `prev` is adjacent and free
            let prev = (*hdr_ptr).prev;
            if !prev.is_null()
                && (*prev).is_free
                && (prev as usize) + (*prev).total_size == (hdr_ptr as usize)
            {
                assert_eq!((*prev).magic, MAGIC_FREE, "Heap corruption in prev block");
                (*prev).total_size += (*hdr_ptr).total_size;
                let next_after = (*hdr_ptr).next;
                (*prev).next = next_after;
                if !next_after.is_null() {
                    (*next_after).prev = prev;
                } else {
                    self.tail = prev;
                }
            }
        }

        Ok(())
    }

    fn stats(&self) -> HeapStats {
        let mut free_blocks = 0usize;
        let mut largest_free = 0usize;
        let mut free_bytes = 0usize;
        let mut curr = self.head;
        while !curr.is_null() {
            let hdr = unsafe { &*curr };
            if hdr.is_free {
                free_blocks += 1;
                free_bytes += hdr.total_size;
                if hdr.total_size > largest_free {
                    largest_free = hdr.total_size;
                }
            }
            curr = hdr.next;
        }
        HeapStats {
            mapped_bytes: self.mapped_bytes,
            used_bytes: self.used_bytes,
            free_bytes,
            peak_used_bytes: self.peak_used_bytes,
            active_allocations: self.active_allocations,
            alloc_count: self.alloc_count,
            free_count: self.free_count,
            free_blocks,
            largest_free_block: largest_free,
            double_free_detected: self.double_free_detected,
        }
    }
}

pub struct KernelHeapAllocator {
    inner: IrqSpinLock<CoalescingHeap>,
}

impl KernelHeapAllocator {
    pub const fn new() -> Self {
        Self {
            inner: IrqSpinLock::new(CoalescingHeap::new()),
        }
    }
}

impl Default for KernelHeapAllocator {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl GlobalAlloc for KernelHeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.inner.lock().alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if let Err(err) = self.inner.lock().dealloc(ptr) {
            panic!("{}", err);
        }
    }
}

#[global_allocator]
static GLOBAL_KERNEL_HEAP: KernelHeapAllocator = KernelHeapAllocator::new();

pub fn init() -> Result<HeapStats, &'static str> {
    GLOBAL_KERNEL_HEAP.inner.lock().init()
}

pub fn stats() -> HeapStats {
    GLOBAL_KERNEL_HEAP.inner.lock().stats()
}

/// Safe non-panicking free validation wrapper (used by self-tests to verify double-free rejection).
pub fn try_checked_free(ptr: *mut u8) -> Result<(), &'static str> {
    GLOBAL_KERNEL_HEAP.inner.lock().dealloc(ptr)
}
