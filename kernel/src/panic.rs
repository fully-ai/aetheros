//! Kernel panic handler and frame-pointer (`RBP`) stack backtrace generator.

use core::arch::asm;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::{cli, halt_loop, qemu_debug_exit, serial};

static PANIC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static TEST_MODE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_test_mode(enabled: bool) {
    TEST_MODE_ACTIVE.store(enabled, Ordering::Relaxed);
}

#[inline]
fn is_canonical_kernel_addr(addr: u64) -> bool {
    addr >= 0xFFFF_8000_0000_0000 && (addr & 0x7) == 0
}

/// Walk the `RBP` frame-pointer chain up to 16 frames and print return addresses.
pub unsafe fn print_backtrace() {
    let mut rbp: u64;
    unsafe {
        asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack, preserves_flags));
    }
    unsafe {
        serial::emergency_write_fmt(format_args!("--- Kernel Stack Backtrace ---\n"));
    }
    for depth in 0..16usize {
        if !is_canonical_kernel_addr(rbp) {
            break;
        }
        let frame_ptr = rbp as *const u64;
        let next_rbp = unsafe { frame_ptr.read_volatile() };
        let ret_rip = unsafe { frame_ptr.add(1).read_volatile() };
        if ret_rip == 0 {
            break;
        }
        unsafe {
            serial::emergency_write_fmt(format_args!(
                "  #{:02}: RIP={:#018x} (RBP={:#018x})\n",
                depth, ret_rip, rbp
            ));
        }
        if next_rbp <= rbp || next_rbp - rbp > 0x40000 {
            break;
        }
        rbp = next_rbp;
    }
    unsafe {
        serial::emergency_write_fmt(format_args!("------------------------------\n"));
    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    cli();

    // Prevent recursive panic loops
    if PANIC_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        halt_loop();
    }

    unsafe {
        serial::emergency_write_fmt(format_args!(
            "\n============================================================\n"
        ));
        serial::emergency_write_fmt(format_args!("!!! AETHEROS KERNEL PANIC !!!\n"));
        if let Some(loc) = info.location() {
            serial::emergency_write_fmt(format_args!(
                "Location : {}:{}:{}\n",
                loc.file(),
                loc.line(),
                loc.column()
            ));
        }
        serial::emergency_write_fmt(format_args!("Message  : {}\n", info.message()));
        print_backtrace();
        serial::emergency_write_fmt(format_args!(
            "============================================================\n"
        ));
    }

    if TEST_MODE_ACTIVE.load(Ordering::Relaxed) {
        // Exit code 0x11 -> QEMU return code ((0x11 << 1) | 1) = 35 (TEST FAIL)
        qemu_debug_exit(0x11);
    }

    halt_loop()
}
