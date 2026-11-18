# AetherOS Memory Management

## 1. Physical Memory Manager (`kernel/src/mm/pmm.rs`)

### Fail-Closed Bitmap Architecture
1. **Initialization**: `BitmapAllocator::init` scans the bootloader memory map to determine the highest physical address (`total_frames`) and places the allocation bitmap (1 bit per 4 KiB physical frame) inside the first usable region large enough to hold it.
2. **Fail-Closed Default**: Every bit in the bitmap is initialized to `1` (`0xFF`, allocated/reserved). Only the strictly page-aligned interiors of `MemoryRegionKind::Usable` regions are cleared to `0` (`free`), and frame `0` (`0x0000`) plus the bitmap's own physical pages are immediately re-marked `1`.
3. **Allocation & Fragmentation Diagnostics**:
   - `alloc_frame()` / `alloc_zeroed_frame()` use an 8-frame byte-skip fast path (`byte == 0xFF`) starting from a rolling `next_hint` cursor.
   - `alloc_contiguous(count)` locates contiguous physical frame runs for DMA and multi-page structures.
   - `free_frame(phys)` validates 4 KiB alignment, rejects reserved/bitmap frames, and detects double-frees (`PmmError::DoubleFree`).
   - `fragmentation_report()` computes `free_frames`, `free_runs`, and `largest_free_run_frames`.

### Copy-on-Write Physical Frame Reference Counting (`SharedRefTable`)
- Newly allocated frames implicitly have a reference count of `1`.
- When `clone_user_pml4_cow` (`sys_fork`) shares a user physical page between parent and child, `pmm::inc_frame_ref(phys)` increments its reference count in `SHARED_REFS` (`refs >= 2`).
- When `pmm::free_frame(phys)` is called (either during a COW write fault split or when a process exits/execs via `destroy_user_pml4`), `dec_ref_should_keep(phys)` decrements the reference count and **keeps the frame allocated in the bitmap** whenever at least one owner remains (`2 -> 1` or `N -> N-1`). The physical frame is returned to the bitmap only when the final owner frees it (`1 -> 0`).

---

## 2. Virtual Memory Manager (`kernel/src/arch/x86_64/paging.rs`, `kernel/src/mm/vmm.rs`)

### 4-Level x86_64 Page Tables (`PML4 -> PDPT -> PD -> PT`)
- All physical page table frames are accessed in O(1) time via the Higher-Half Direct Map (`phys_to_virt(phys) = hhdm_offset + phys`).
- **Strict `W^X` Enforcement**: `map_page_in` rejects any mapping request where both `PTE_WRITABLE` (bit 1) and executable permission (`PTE_NO_EXECUTE == 0`, bit 63 clear) are requested simultaneously (`PagingError::WxViolation`).

### Custom OS Software PTE Bits (`bits 9..11`)
| Constant | Bit | Meaning |
| :--- | :--- | :--- |
| `PTE_COW` | Bit 9 (`1 << 9`) | Copy-on-Write shared page (`PTE_PRESENT = 1`, `PTE_WRITABLE = 0`). Write fault allocates a private 4 KiB copy (or promotes in-place if `frame_refcount == 1`). |
| `PTE_DEMAND` | Bit 10 (`1 << 10`) | Demand-paged anonymous virtual page (`PTE_PRESENT = 0`). First read or write fault allocates a zeroed 4 KiB frame on demand and maps it with the saved permissions. |
| `PTE_GUARD` | Bit 11 (`1 << 11`) | Stack/heap guard page (`PTE_PRESENT = 0`). Any access traps into `#PF` (`handle_page_fault` returns `None`), terminating a faulting Ring 3 process or panicking on kernel stack overflow. |

---

## 3. Coalescing Kernel Heap Allocator (`kernel/src/mm/heap.rs`)

- **Virtual Arena**: Mapped at `0xffffc00000000000` (`HEAP_START_VIRT`), initially backed by 512 physical frames (**2 MiB** of contiguous virtual memory mapped RW+NX).
- **Block Header & Integrity Canary**:
  - Every heap block starts with a 32-byte aligned `BlockHeader` containing `magic: 0xAE74_EC01`, `size`, `is_free`, and `next`.
  - Freeing a block verifies `magic == BLOCK_MAGIC` and `is_free == false`; any corruption or double-free is caught immediately.
- **Splitting & Immediate Coalescing**:
  - `alloc` aligns the requested payload and splits oversized free blocks when the remainder can hold a `BlockHeader + 32` bytes.
  - `dealloc` marks the block free and performs an immediate linear coalesce pass (`coalesce_adjacent`) merging contiguous free neighbors to prevent fragmentation.
