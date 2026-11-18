#![no_std]
#![no_main]
#![allow(clippy::missing_safety_doc)]

//! AetherOS x86_64 Kernel Entry Point and Staged Initialization.

extern crate alloc;

pub mod arch;
pub mod boot;
pub mod drivers;
pub mod log;
pub mod mm;
pub mod panic;
pub mod sched;
pub mod selftest;
pub mod sync;

use aether_abi::{ABI_VERSION_MAJOR, ABI_VERSION_MINOR};
use aether_uuid::Uuid;

extern "C" {
    static __kernel_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __bss_end: u8;
    static __kernel_end: u8;
}

/// Higher-half kernel entry point invoked by Limine in 64-bit Long Mode.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Stage 0: Ensure interrupts are masked while bringing up core CPU tables
    arch::x86_64::cli();

    // Stage 1: Bring up early COM1 serial console for immediate diagnostics
    let serial_ok = arch::x86_64::serial::init();
    if serial_ok {
        arch::x86_64::serial::write_str(
            "\n============================================================\n",
        );
        arch::x86_64::serial::write_str("  AetherOS x86_64 Kernel v0.1.0-alpha.1 (Milestone 1)\n");
        arch::x86_64::serial::write_str(
            "============================================================\n",
        );
    }

    // Stage 2: Parse and validate Limine Boot Protocol v3 handoff structures
    let boot_info = match boot::parse_boot_info() {
        Ok(info) => info,
        Err(reason) => panic!("Bootloader handoff validation failed: {}", reason),
    };

    // Stage 3: Initialize optional framebuffer debug text console (secondary to serial)
    if let Some(fb) = boot_info.framebuffer {
        drivers::fbcon::init(fb);
    }

    kinfo!(
        "Bootloader: {} {} | Firmware: {} | CPUs: {} (BSP LAPIC={})",
        boot_info.bootloader_name,
        boot_info.bootloader_version,
        boot_info.firmware_type,
        boot_info.cpu_count,
        boot_info.bsp_lapic_id
    );
    kinfo!(
        "ABI v{}.{} | Root UUID: {} | Cmdline: '{}'",
        ABI_VERSION_MAJOR,
        ABI_VERSION_MINOR,
        Uuid::ROOT_USER,
        boot_info.cmdline
    );
    kinfo!(
        "Kernel ELF: virt={:#018x}..{:#018x} phys={:#018x} HHDM={:#018x}",
        &raw const __kernel_start as u64,
        &raw const __kernel_end as u64,
        boot_info.kernel_phys_base,
        boot_info.hhdm_offset
    );
    kdebug!(
        "Sections: .text={:#018x}..{:#018x} .rodata={:#018x}..{:#018x} .data/.bss={:#018x}..{:#018x}",
        &raw const __text_start as u64,
        &raw const __text_end as u64,
        &raw const __rodata_start as u64,
        &raw const __rodata_end as u64,
        &raw const __data_start as u64,
        &raw const __bss_end as u64
    );

    // Stage 4: Probe CPUID features and enable baseline x86_64 hardware protections
    let mut cpu_features = arch::x86_64::cpu::detect_features();
    if let Err(err) = arch::x86_64::cpu::init_early_protections(&mut cpu_features) {
        panic!("Unsupported x86_64 CPU configuration: {}", err);
    }
    kinfo!("{:?}", cpu_features);
    kinfo!(
        "CPU protections active: CR0.WP={} EFER.NXE={} CR4.SMEP={} CR4.SMAP={}",
        cpu_features.wp_enabled,
        cpu_features.nxe_enabled,
        cpu_features.smep_enabled,
        cpu_features.smap_enabled
    );

    // Stage 5: Initialize GDT, TSS (with IST1 #DF / IST2 #NMI / IST3 #MC stacks)
    arch::x86_64::gdt::init();
    kinfo!(
        "GDT & TSS initialized (CS={:#04x}, TR={:#04x})",
        arch::x86_64::gdt::read_cs(),
        arch::x86_64::gdt::read_tr()
    );

    // Stage 6: Remap legacy 8259 PIC (vectors 32..47, all masked) & load 256-entry IDT
    arch::x86_64::pic::init_and_mask_all();
    arch::x86_64::idt::init();
    kinfo!("IDT (256 vectors) loaded and legacy 8259 PIC remapped (32..47, masked)");

    // Stage 7: Initialize Physical Memory Manager (bitmap allocator over Limine memmap)
    let pmm_stats = match mm::pmm::init(&boot_info) {
        Ok(stats) => stats,
        Err(err) => panic!("Physical Memory Manager init failed: {}", err),
    };
    kinfo!(
        "PMM ready: usable={} KiB ({} frames) | free={} KiB | bitmap={} B | reserved={} KiB",
        pmm_stats.usable_frames * 4,
        pmm_stats.usable_frames,
        pmm_stats.free_frames * 4,
        pmm_stats.bitmap_bytes,
        pmm_stats.reserved_bytes / 1024
    );
    if let Some(initramfs) = boot_info.initramfs {
        kinfo!(
            "Boot module detected: initramfs ({} bytes)",
            initramfs.len()
        );
    }
    if let Some(rsdp) = boot_info.rsdp_addr {
        kinfo!("ACPI RSDP handoff address: {:#018x}", rsdp);
    }

    // Stage 8: Initialize Virtual Memory Manager (4-level paging, #PF demand/COW/guard handler)
    let vmm_stats = match mm::vmm::init() {
        Ok(stats) => stats,
        Err(err) => panic!("Virtual Memory Manager init failed: {}", err),
    };
    kinfo!(
        "VMM ready: kernel_pml4={:#010x} (4-level paging, W^X, Demand Paging, COW, Guard Pages)",
        vmm_stats.kernel_pml4_phys
    );

    // Stage 9: Initialize Kernel Heap Allocator (`#[global_allocator]`, 2 MiB initial, up to 64 MiB)
    let heap_stats = match mm::heap::init() {
        Ok(stats) => stats,
        Err(err) => panic!("Kernel Heap Allocator init failed: {}", err),
    };
    kinfo!(
        "Kernel Heap ready: base={:#018x} mapped={} KiB free={} KiB",
        mm::vmm::KERNEL_HEAP_BASE,
        heap_stats.mapped_bytes / 1024,
        heap_stats.free_bytes / 1024
    );

    // Stage 10: Initialize Preemptive Scheduler (kthreadd/boot + idle/0) & 1000 Hz PIT Timer IRQ0
    if let Err(err) = sched::init() {
        panic!("Preemptive Scheduler init failed: {}", err);
    }
    arch::x86_64::timer::init();
    arch::x86_64::sti();
    kinfo!(
        "Scheduler & Timer active: 1000 Hz PIT (IRQ0 -> vec {}), LAPIC={:#010x}, tasks={}",
        arch::x86_64::timer::TIMER_VECTOR,
        arch::x86_64::timer::lapic_phys_base(),
        sched::stats().total_tasks
    );

    // Stage 11: Run automated kernel self-tests when requested via kernel command line
    selftest::maybe_run(&boot_info, &cpu_features);

    kinfo!("Milestone 3 kernel initialization complete. Entering idle halt loop.");
    arch::x86_64::halt_loop()
}
