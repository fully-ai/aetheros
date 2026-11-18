//! In-kernel verification self-test suite (executed in QEMU when `selftest=...` is on cmdline).

use core::arch::asm;

use crate::arch::x86_64::{cpu::CpuFeatures, gdt, idt, qemu_debug_exit};
use crate::boot::BootInfo;
use crate::mm::pmm::{self, PmmError};
use crate::mm::PAGE_SIZE;

pub fn maybe_run(boot: &BootInfo, cpu_features: &CpuFeatures) {
    if !boot.cmdline.contains("selftest=") {
        return;
    }

    crate::panic::set_test_mode(true);
    crate::kinfo!("=== AetherOS Kernel Self-Test Suite (Milestone 1) ===");

    test_boot_handoff(boot);
    test_cpu_protections(cpu_features);
    test_gdt_and_tss();
    test_idt_breakpoint_exception();
    test_pmm_allocator();

    crate::kinfo!("[AETHEROS-TEST] ALL MILESTONE 1 KERNEL TESTS PASSED");
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
    // Execute a real software breakpoint exception (#BP, vector 3) and verify clean return!
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

    // 1. Allocate 32 distinct frames, write unique patterns via HHDM, verify zero overlap
    let mut frames = [0u64; 32];
    for (i, slot) in frames.iter_mut().enumerate() {
        let phys = pmm::alloc_zeroed_frame().expect("alloc_zeroed_frame");
        assert_eq!(phys % (PAGE_SIZE as u64), 0, "frame must be 4 KiB aligned");
        assert_ne!(phys, 0, "frame 0 must never be allocated");
        // Verify zeroing
        let ptr = pmm::phys_to_virt(phys) as *mut u64;
        for w in 0..(PAGE_SIZE / 8) {
            assert_eq!(unsafe { ptr.add(w).read() }, 0, "zeroed frame must be zero");
        }
        // Write unique sentinel
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

    // 2. Free all 32 frames and verify double-free detection
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

    // 3. Contiguous allocation test (16 pages = 64 KiB)
    let contig = pmm::alloc_contiguous(16).expect("alloc_contiguous(16)");
    assert_eq!(contig % (PAGE_SIZE as u64), 0);
    pmm::free_contiguous(contig, 16).expect("free_contiguous(16)");

    let final_stats = pmm::stats();
    assert_eq!(
        final_stats.free_frames, initial.free_frames,
        "PMM leaked frames during self-test"
    );
    assert!(
        final_stats.double_free_detected >= 1,
        "double_free_detected counter must increment"
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
