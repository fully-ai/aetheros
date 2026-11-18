//! In-kernel verification self-test suite (executed in QEMU when `selftest=...` is on cmdline).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::arch::asm;

use crate::arch::x86_64::paging::{
    PagingError, PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE,
};
use crate::arch::x86_64::{cpu::CpuFeatures, gdt, idt, qemu_debug_exit};
use crate::boot::BootInfo;
use crate::mm::heap;
use crate::mm::pmm::{self, PmmError};
use crate::mm::vmm::{self, AddressSpace};
use crate::mm::PAGE_SIZE;

pub fn maybe_run(boot: &BootInfo, cpu_features: &CpuFeatures) {
    if !boot.cmdline.contains("selftest=") {
        return;
    }

    crate::panic::set_test_mode(true);
    crate::kinfo!("=== AetherOS Kernel Self-Test Suite (Milestones 1 & 2) ===");

    // Milestone 1 tests
    test_boot_handoff(boot);
    test_cpu_protections(cpu_features);
    test_gdt_and_tss();
    test_idt_breakpoint_exception();
    test_pmm_allocator();

    // Milestone 2 tests
    test_vmm_page_mapping_and_wx();
    test_vmm_demand_paging();
    test_vmm_address_space_isolation_and_cow();
    test_kernel_heap_and_alloc();

    crate::kinfo!("[AETHEROS-TEST] ALL MILESTONE 1 & 2 KERNEL TESTS PASSED");
    // Exit code 0x10 -> QEMU exit status ((0x10 << 1) | 1) = 33 (PASS)
    qemu_debug_exit(0x10);
}

fn test_boot_handoff(boot: &BootInfo) {
    assert!(
        boot.hhdm_offset >= 0xFFFF_8000_0000_0000,
        "HHDM must be in higher half"
    );
    assert_eq!(
        boot.kernel_virt_base, 0xFFFF_FFFF_8000_0000,
        "kernel must be linked at -2 GiB"
    );
    assert!(boot.memmap_count > 0, "memory map must not be empty");
    crate::kinfo!(
        "[TEST PASS] boot_handoff (HHDM={:#018x}, memmap_entries={})",
        boot.hhdm_offset,
        boot.memmap_count
    );
}

fn test_cpu_protections(cpu: &CpuFeatures) {
    assert!(cpu.wp_enabled, "CR0.WP must be enabled");
    assert!(cpu.nxe_enabled, "EFER.NXE must be enabled on x86_64");
    assert!(cpu.apic, "Local APIC must be present");
    crate::kinfo!(
        "[TEST PASS] cpu_protections (WP={} NXE={} SMEP={})",
        cpu.wp_enabled,
        cpu.nxe_enabled,
        cpu.smep_enabled
    );
}

fn test_gdt_and_tss() {
    let cs = gdt::read_cs();
    let tr = gdt::read_tr();
    assert_eq!(cs, gdt::KERNEL_CS, "CS must match KERNEL_CS (0x08)");
    assert_eq!(tr, gdt::TSS_SELECTOR, "TR must match TSS_SELECTOR (0x30)");
    crate::kinfo!("[TEST PASS] gdt_and_tss (CS={:#04x}, TR={:#04x})", cs, tr);
}

fn test_idt_breakpoint_exception() {
    let before = idt::breakpoint_count();
    unsafe {
        asm!("int3", options(nomem, nostack));
    }
    let after = idt::breakpoint_count();
    assert_eq!(
        after,
        before + 1,
        "IDT #BP handler must fire exactly once on int3"
    );
    crate::kinfo!(
        "[TEST PASS] idt_breakpoint_exception (before={}, after={})",
        before,
        after
    );
}

fn test_pmm_allocator() {
    let initial = pmm::stats();
    assert!(
        initial.free_frames > 1024,
        "expected >4 MiB free physical memory"
    );

    let mut frames = [0u64; 32];
    for (i, slot) in frames.iter_mut().enumerate() {
        let phys = pmm::alloc_zeroed_frame().expect("alloc_zeroed_frame");
        assert_eq!(phys % (PAGE_SIZE as u64), 0, "frame must be 4 KiB aligned");
        assert_ne!(phys, 0, "frame 0 must never be allocated");
        let ptr = pmm::phys_to_virt(phys) as *mut u64;
        for w in 0..(PAGE_SIZE / 8) {
            assert_eq!(unsafe { ptr.add(w).read() }, 0, "zeroed frame must be zero");
        }
        unsafe { ptr.write(0xA37E_0000_0000_0000u64 | (i as u64)) };
        *slot = phys;
    }

    for (i, &phys) in frames.iter().enumerate() {
        let ptr = pmm::phys_to_virt(phys) as *const u64;
        assert_eq!(
            unsafe { ptr.read() },
            0xA37E_0000_0000_0000u64 | (i as u64),
            "frame contents corrupted / overlapped"
        );
    }

    for &phys in &frames {
        pmm::free_frame(phys).expect("free_frame");
    }
    assert_eq!(
        pmm::free_frame(frames[0]),
        Err(PmmError::DoubleFree),
        "double free must be detected and rejected"
    );
    assert_eq!(
        pmm::free_frame(0),
        Err(PmmError::ReservedFrame),
        "freeing frame 0 must be rejected"
    );
    assert_eq!(
        pmm::free_frame(0x1234),
        Err(PmmError::UnalignedAddress),
        "unaligned free must be rejected"
    );

    let contig = pmm::alloc_contiguous(16).expect("alloc_contiguous(16)");
    assert_eq!(contig % (PAGE_SIZE as u64), 0);
    pmm::free_contiguous(contig, 16).expect("free_contiguous(16)");

    let final_stats = pmm::stats();
    assert_eq!(
        final_stats.free_frames, initial.free_frames,
        "PMM leaked frames during self-test"
    );
    let frag = pmm::fragmentation_report();
    assert!(frag.largest_free_run_frames >= 16);
    crate::kinfo!(
        "[TEST PASS] pmm_allocator (usable={} MiB, free={} MiB, free_runs={}, max_run={} KiB)",
        (final_stats.usable_frames * 4) / 1024,
        (final_stats.free_frames * 4) / 1024,
        frag.free_runs,
        frag.largest_free_run_frames * 4
    );
}

fn test_vmm_page_mapping_and_wx() {
    let aspace = AddressSpace::kernel();
    let vaddr = vmm::alloc_kernel_virt_range(1);
    let phys = pmm::alloc_zeroed_frame().expect("alloc_zeroed_frame");

    // 1. W^X check: Writable + Executable (without PTE_NO_EXECUTE) MUST be rejected!
    assert_eq!(
        aspace.map_page(vaddr, phys, PTE_PRESENT | PTE_WRITABLE),
        Err(PagingError::WxViolation),
        "W^X violation must be rejected"
    );

    // 2. Valid RW + NX mapping
    aspace
        .map_page(vaddr, phys, PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE)
        .expect("map_page RW+NX");

    // 3. Duplicate mapping to same virtual page must fail
    assert_eq!(
        aspace.map_page(vaddr, phys, PTE_PRESENT | PTE_WRITABLE | PTE_NO_EXECUTE),
        Err(PagingError::AlreadyMapped)
    );

    // 4. Write through virtual mapping and verify physical translation + memory content
    let ptr = vaddr as *mut u64;
    unsafe { ptr.write_volatile(0x55AA_F00D_CAFE_BEEFu64) };
    let (translated_phys, flags) = aspace.translate(vaddr + 0x128).expect("translate");
    assert_eq!(translated_phys, phys + 0x128);
    assert_ne!(flags & PTE_NO_EXECUTE, 0);
    assert_eq!(
        unsafe { *(pmm::phys_to_virt(phys) as *const u64) },
        0x55AA_F00D_CAFE_BEEFu64
    );

    // 5. Demote to read-only + NX via protect_page and read back
    aspace
        .protect_page(vaddr, PTE_PRESENT | PTE_NO_EXECUTE)
        .expect("protect_page RO+NX");
    assert_eq!(unsafe { ptr.read_volatile() }, 0x55AA_F00D_CAFE_BEEFu64);

    // 6. Unmap and free
    aspace
        .unmap_and_free_page(vaddr)
        .expect("unmap_and_free_page");
    assert!(
        aspace.translate(vaddr).is_none(),
        "unmapped page must not translate"
    );

    crate::kinfo!(
        "[TEST PASS] vmm_page_mapping_and_wx (vaddr={:#018x}, phys={:#010x})",
        vaddr,
        phys
    );
}

fn test_vmm_demand_paging() {
    let aspace = AddressSpace::kernel();
    let base_vaddr = vmm::alloc_kernel_virt_range(4);
    let before_faults = vmm::stats().demand_faults_resolved;

    for i in 0..4u64 {
        let v = base_vaddr + i * (PAGE_SIZE as u64);
        aspace
            .map_demand_page(v, PTE_WRITABLE | PTE_NO_EXECUTE)
            .expect("map_demand_page");
        assert!(
            aspace.translate(v).is_none(),
            "demand page must not be backed before first fault"
        );
    }

    // Touch all 4 lazy pages -> triggers 4 hardware #PF faults resolved transparently by VMM!
    for i in 0..4u64 {
        let v = base_vaddr + i * (PAGE_SIZE as u64);
        let p = v as *mut u64;
        assert_eq!(
            unsafe { p.read_volatile() },
            0,
            "demand page must be zero-initialized"
        );
        unsafe { p.write_volatile(0xD34A_0000u64 + i) };
    }

    let after_faults = vmm::stats().demand_faults_resolved;
    assert_eq!(
        after_faults,
        before_faults + 4,
        "expected exactly 4 demand-page faults resolved"
    );

    for i in 0..4u64 {
        let v = base_vaddr + i * (PAGE_SIZE as u64);
        let p = v as *const u64;
        assert_eq!(unsafe { p.read_volatile() }, 0xD34A_0000u64 + i);
        aspace
            .unmap_and_free_page(v)
            .expect("unmap_and_free_page demand");
    }

    crate::kinfo!(
        "[TEST PASS] vmm_demand_paging (faults_resolved={})",
        after_faults - before_faults
    );
}

fn test_vmm_address_space_isolation_and_cow() {
    let kernel_aspace = AddressSpace::kernel();
    let user_test_vaddr: u64 = 0x0000_0000_0080_0000;

    let user_a = AddressSpace::new_user().expect("AddressSpace::new_user A");
    let frame_a = pmm::alloc_zeroed_frame().expect("alloc frame_a");
    user_a
        .map_page(
            user_test_vaddr,
            frame_a,
            PTE_PRESENT | PTE_WRITABLE | PTE_USER | PTE_NO_EXECUTE,
        )
        .expect("map user_a page");

    // Write initial value in user_a
    user_a.activate();
    let user_ptr = user_test_vaddr as *mut u64;
    unsafe { user_ptr.write_volatile(0xCAFE_BABE_1111_2222u64) };

    // Clone user_b via Copy-on-Write (`clone_cow`)
    let user_b = user_a.clone_cow().expect("user_a.clone_cow()");
    assert_eq!(
        pmm::frame_refcount(frame_a),
        2,
        "COW frame refcount must be 2 after clone_cow"
    );

    let cow_before = vmm::stats().cow_faults_resolved;

    // Activate user_b, verify it reads the shared value, then write to trigger a #PF COW split!
    user_b.activate();
    assert_eq!(
        unsafe { user_ptr.read_volatile() },
        0xCAFE_BABE_1111_2222u64
    );
    unsafe { user_ptr.write_volatile(0xDEAD_BEEF_9999_8888u64) };
    assert_eq!(
        unsafe { user_ptr.read_volatile() },
        0xDEAD_BEEF_9999_8888u64
    );

    let cow_after = vmm::stats().cow_faults_resolved;
    assert_eq!(
        cow_after,
        cow_before + 1,
        "writing to COW page in user_b must resolve 1 COW fault"
    );
    assert_eq!(
        pmm::frame_refcount(frame_a),
        1,
        "original frame refcount must drop back to 1 after COW split"
    );

    // Switch back to user_a and confirm its private value was completely isolated from user_b's write!
    user_a.activate();
    assert_eq!(
        unsafe { user_ptr.read_volatile() },
        0xCAFE_BABE_1111_2222u64,
        "user_a memory must remain isolated from user_b mutation"
    );

    // Restore kernel CR3 and tear down both isolated user address spaces
    kernel_aspace.activate();
    user_a.destroy_user();
    user_b.destroy_user();

    crate::kinfo!(
        "[TEST PASS] vmm_address_space_isolation_and_cow (cow_faults={})",
        cow_after
    );
}

fn test_kernel_heap_and_alloc() {
    let initial = heap::stats();
    assert_eq!(initial.active_allocations, 0);
    assert_eq!(initial.used_bytes, 0);

    {
        // 1. Box, Vec, String, BTreeMap exercise
        let b = Box::new(0x1337_C0DE_CAFE_BABEu64);
        assert_eq!(*b, 0x1337_C0DE_CAFE_BABEu64);

        let mut vec_vals = Vec::with_capacity(256);
        for i in 0..256u64 {
            vec_vals.push(i * 7);
        }
        assert_eq!(vec_vals[255], 255 * 7);

        let mut s = String::from("AetherOS-");
        s.push_str("KernelHeap-OK");
        assert_eq!(s, "AetherOS-KernelHeap-OK");

        let mut map = BTreeMap::new();
        for k in 0..64u32 {
            map.insert(k, k * 10);
        }
        assert_eq!(map.get(&42), Some(&420));

        // 2. Large allocation (128 KiB)
        let large: Vec<u8> = alloc::vec![0xAB; 128 * 1024];
        assert_eq!(large[0], 0xAB);
        assert_eq!(large[128 * 1024 - 1], 0xAB);

        // 3. Over-aligned 4 KiB page-aligned allocation via GlobalAlloc
        let aligned_layout = Layout::from_size_align(512, 4096).unwrap();
        let aligned_ptr = unsafe { alloc::alloc::alloc(aligned_layout) };
        assert!(!aligned_ptr.is_null(), "4096-aligned heap alloc failed");
        assert_eq!(
            (aligned_ptr as usize) % 4096,
            0,
            "heap returned insufficiently aligned pointer"
        );
        unsafe { aligned_ptr.write_bytes(0x5A, 512) };
        heap::try_checked_free(aligned_ptr).expect("first free of aligned_ptr");

        // 4. Verify double-free detection catches freeing `aligned_ptr` a second time!
        assert!(
            heap::try_checked_free(aligned_ptr).is_err(),
            "heap double-free must be detected and rejected"
        );
    }

    let after = heap::stats();
    assert_eq!(
        after.active_allocations, 0,
        "all test heap allocations must be freed"
    );
    assert_eq!(after.used_bytes, 0, "used_bytes must return to 0");
    assert_eq!(
        after.free_blocks, 1,
        "free blocks must coalesce back into 1 contiguous free block"
    );
    assert!(after.double_free_detected >= 1);

    crate::kinfo!(
        "[TEST PASS] kernel_heap_and_alloc (mapped={} KiB, peak={} KiB, allocs={}, free_blocks={})",
        after.mapped_bytes / 1024,
        after.peak_used_bytes / 1024,
        after.alloc_count,
        after.free_blocks
    );
}
