//! In-kernel verification self-test suite (executed in QEMU when `selftest=...` is on cmdline).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::arch::{asm, global_asm};
use core::ptr::addr_of;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::x86_64::paging::{
    PagingError, PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE,
};
use crate::arch::x86_64::{cpu::CpuFeatures, gdt, idt, qemu_debug_exit, timer};
use crate::boot::BootInfo;
use crate::mm::heap;
use crate::mm::pmm::{self, PmmError};
use crate::mm::vmm::{self, AddressSpace};
use crate::mm::PAGE_SIZE;
use crate::sched::{self, Credentials, PRIORITY_HIGH, PRIORITY_NORMAL};
use crate::sync::{Mutex, Semaphore, WaitQueue};
use crate::syscall;

pub fn maybe_run(boot: &BootInfo, cpu_features: &CpuFeatures) {
    if !boot.cmdline.contains("selftest=") {
        return;
    }

    crate::panic::set_test_mode(true);
    crate::kinfo!("=== AetherOS Kernel Self-Test Suite (Milestones 1 - 4) ===");

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

    // Milestone 3 tests
    test_timer_and_sleep();
    test_preemptive_multitasking_and_sync();

    // Milestone 4 tests (Ring 3 User Mode, Syscalls, Pointer Validation, Fork/Wait4, Fault Isolation)
    test_ring3_syscalls_fork_and_pointer_validation();
    test_ring3_fault_isolation();

    crate::kinfo!("[AETHEROS-TEST] ALL MILESTONE 1 - 4 KERNEL TESTS PASSED");
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
        after.active_allocations, initial.active_allocations,
        "all test heap allocations must be freed"
    );
    assert_eq!(
        after.used_bytes, initial.used_bytes,
        "used_bytes must return to pre-test baseline"
    );
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

static SHARED_MUTEX_COUNTER: Mutex<u64> = Mutex::new(0);
static CPU_WORKER_A_TICKS: AtomicU64 = AtomicU64::new(0);
static CPU_WORKER_B_TICKS: AtomicU64 = AtomicU64::new(0);
static EVENT_READY: AtomicBool = AtomicBool::new(false);
static EVENT_WAITQ: WaitQueue = WaitQueue::new();
static DONE_SEM: Semaphore = Semaphore::new(0);

fn cpu_bound_worker(id: usize) {
    // Pure CPU-bound loop WITHOUT voluntary yield_now(): relies on real 1000 Hz timer preemption!
    let start_tick = timer::ticks();
    while timer::ticks().saturating_sub(start_tick) < 12 {
        if id == 1 {
            CPU_WORKER_A_TICKS.fetch_add(1, Ordering::Relaxed);
        } else {
            CPU_WORKER_B_TICKS.fetch_add(1, Ordering::Relaxed);
        }
        core::hint::spin_loop();
    }

    for _ in 0..100 {
        let mut guard = SHARED_MUTEX_COUNTER.lock();
        *guard += 1;
    }
    DONE_SEM.release();
}

fn event_producer_worker(_arg: usize) {
    sched::sleep_ms(6);
    {
        let mut guard = SHARED_MUTEX_COUNTER.lock();
        *guard += 50;
    }
    EVENT_READY.store(true, Ordering::Release);
    EVENT_WAITQ.wake_all();
    DONE_SEM.release();
}

fn test_timer_and_sleep() {
    let t0 = timer::ticks();
    let sw0 = sched::stats().context_switches;
    sched::sleep_ms(12);
    let t1 = timer::ticks();
    let sw1 = sched::stats().context_switches;

    assert!(
        t1 >= t0 + 12,
        "sleep_ms(12) woke too early (t0={}, t1={})",
        t0,
        t1
    );
    assert!(
        sw1 >= sw0 + 2,
        "sleep_ms must context-switch to idle thread and back"
    );
    crate::kinfo!(
        "[TEST PASS] timer_and_sleep (elapsed={} ms, switches={})",
        t1 - t0,
        sw1 - sw0
    );
}

fn test_preemptive_multitasking_and_sync() {
    *SHARED_MUTEX_COUNTER.lock() = 0;
    CPU_WORKER_A_TICKS.store(0, Ordering::Relaxed);
    CPU_WORKER_B_TICKS.store(0, Ordering::Relaxed);
    EVENT_READY.store(false, Ordering::Relaxed);

    let tid_a = sched::spawn_kernel_thread("cpu_worker_a", PRIORITY_NORMAL, cpu_bound_worker, 1)
        .expect("spawn cpu_worker_a");
    let tid_b = sched::spawn_kernel_thread("cpu_worker_b", PRIORITY_NORMAL, cpu_bound_worker, 2)
        .expect("spawn cpu_worker_b");
    let tid_p = sched::spawn_kernel_thread("evt_producer", PRIORITY_HIGH, event_producer_worker, 0)
        .expect("spawn evt_producer");

    // Verify dynamic priority adjustment
    assert_eq!(sched::get_priority(tid_p), Some(PRIORITY_HIGH));
    assert!(sched::set_priority(tid_b, PRIORITY_NORMAL + 1));
    assert_eq!(sched::get_priority(tid_b), Some(PRIORITY_NORMAL + 1));
    assert!(sched::set_priority(tid_b, PRIORITY_NORMAL));

    // Wait on WaitQueue until producer signals EVENT_READY
    while !EVENT_READY.load(Ordering::Acquire) {
        EVENT_WAITQ.wait();
    }

    // Acquire semaphore 3 times (once per worker completion)
    DONE_SEM.acquire();
    DONE_SEM.acquire();
    DONE_SEM.acquire();

    // Reap all 3 exited kernel threads via wait_tid
    assert_eq!(sched::wait_tid(tid_a), Ok(0));
    assert_eq!(sched::wait_tid(tid_b), Ok(0));
    assert_eq!(sched::wait_tid(tid_p), Ok(0));

    let a_work = CPU_WORKER_A_TICKS.load(Ordering::Relaxed);
    let b_work = CPU_WORKER_B_TICKS.load(Ordering::Relaxed);
    let final_counter = *SHARED_MUTEX_COUNTER.lock();
    let st = sched::stats();

    assert!(
        a_work > 0 && b_work > 0,
        "both CPU-bound workers must make progress via timer preemption"
    );
    assert_eq!(
        final_counter, 250,
        "mutex-protected counter must equal 100 + 100 + 50"
    );
    assert!(
        st.preemptions > 0,
        "timer interrupt must trigger real preemptions"
    );

    crate::kinfo!(
        "[TEST PASS] preemptive_multitasking_and_sync (preemptions={}, switches={}, mutex_val={}, work_a={}, work_b={})",
        st.preemptions,
        st.context_switches,
        final_counter,
        a_work,
        b_work
    );
}

extern "C" {
    static ring3_test_prog_start: u8;
    static ring3_test_prog_end: u8;
    static ring3_bad_kread_start: u8;
    static ring3_bad_kread_end: u8;
    static ring3_bad_priv_start: u8;
    static ring3_bad_priv_end: u8;
}

global_asm!(
    r#"
.section .rodata

// Position-independent Ring 3 user program testing AetherOS native Syscall ABI v1:
// 1. SYS_ABI_VERSION (0) == 0x10000
// 2. SYS_GETPID (74) >= 3
// 3. SYS_BRK (53) -> expand user heap to 0x00801000 (demand-paged in Ring 3)
// 4. Write "[Ring3] Hello from user mode!\n" (30 bytes) and SYS_WRITE(1, 0x00800000, 30)
// 5. Malicious SYS_WRITE(1, 0xffffffff80000000, 16) -> must return -14 (-EFAULT)
// 6. Unsupported syscall 999 -> must return -38 (-ENOSYS)
// 7. SYS_FORK (71) -> child mutates [0x00800000] (COW fault in Ring 3!) and exits(42);
//    parent calls SYS_WAIT4(-1, 0x00800100, 0), verifies child status == (42 << 8) and
//    verifies parent's own [0x00800000] is untouched, then exits(77).
.global ring3_test_prog_start
ring3_test_prog_start:
    // 1. SYS_ABI_VERSION
    xor eax, eax
    syscall
    cmp rax, 0x10000
    jne 9f

    // 2. SYS_GETPID
    mov eax, 74
    syscall
    cmp rax, 3
    jb 9f

    // 3. SYS_BRK(0x00801000)
    mov eax, 53
    mov edi, 0x00801000
    syscall
    cmp rax, 0x00801000
    jne 9f

    // 4. Store "Ring3-OK\n" at 0x00800000 (demand page fault in Ring 3!)
    mov ebx, 0x00800000
    movabs rax, 0x4b4f2d33676e6952
    mov [rbx], rax
    mov byte ptr [rbx + 8], 0x0a

    // SYS_WRITE(1, 0x00800000, 9)
    mov eax, 2
    mov edi, 1
    mov esi, 0x00800000
    mov edx, 9
    syscall
    cmp rax, 9
    jne 9f

    // 5. Malicious SYS_WRITE with higher-half kernel address -> must return -14 (-EFAULT)
    mov eax, 2
    mov edi, 1
    mov rsi, -0x80000000
    mov edx, 16
    syscall
    cmp rax, -14
    jne 9f

    // 6. Unimplemented syscall 999 -> must return -38 (-ENOSYS)
    mov eax, 999
    syscall
    cmp rax, -38
    jne 9f

    // 7. SYS_FORK()
    mov eax, 71
    syscall
    test rax, rax
    js 9f
    jnz 1f

    // --- Child process (RAX == 0) ---
    // Mutate [0x00800000] = 'C' (triggers Ring 3 Copy-on-Write #PF!)
    mov byte ptr [rbx], 0x43
    mov eax, 70
    mov edi, 42
    syscall
    ud2

1:
    // --- Parent process (RAX == child_pid) ---
    mov r12, rax
    // SYS_WAIT4(-1, 0x00800100, 0)
    mov eax, 73
    mov rdi, -1
    mov esi, 0x00800100
    xor edx, edx
    syscall
    cmp rax, r12
    jne 9f

    // Verify child exit status == (42 << 8) == 0x2a00
    mov eax, dword ptr [rbx + 0x100]
    cmp eax, 0x2a00
    jne 9f

    // Verify parent's [0x00800000] is still 'R' (0x52), isolated from child's COW write
    cmp byte ptr [rbx], 0x52
    jne 9f

    // Success: SYS_EXIT(77)
    mov eax, 70
    mov edi, 77
    syscall
    ud2

9:
    // Failure: SYS_EXIT(99)
    mov eax, 70
    mov edi, 99
    syscall
    ud2
.global ring3_test_prog_end
ring3_test_prog_end:

// Malicious Ring 3 program #1: attempts direct load from higher-half kernel memory
.global ring3_bad_kread_start
ring3_bad_kread_start:
    mov rax, -0x80000000
    mov rax, [rax]
    mov eax, 70
    xor edi, edi
    syscall
.global ring3_bad_kread_end
ring3_bad_kread_end:

// Malicious Ring 3 program #2: attempts privileged `cli` instruction in Ring 3
.global ring3_bad_priv_start
ring3_bad_priv_start:
    cli
    mov eax, 70
    xor edi, edi
    syscall
.global ring3_bad_priv_end
ring3_bad_priv_end:
"#
);

fn code_slice(start: *const u8, end: *const u8) -> &'static [u8] {
    let len = (end as usize) - (start as usize);
    unsafe { core::slice::from_raw_parts(start, len) }
}

fn test_ring3_syscalls_fork_and_pointer_validation() {
    let code = code_slice(
        addr_of!(ring3_test_prog_start),
        addr_of!(ring3_test_prog_end),
    );
    let syscalls_before = syscall::total_syscalls();
    let pid = sched::spawn_user_code_process("user_init_test", code, Credentials::root())
        .expect("spawn_user_code_process");

    let exit_code = sched::wait_tid(pid).expect("wait_tid(user_init_test)");
    let syscalls_after = syscall::total_syscalls();

    assert_eq!(
        exit_code, 77,
        "Ring 3 test process exited with {} (expected 77)",
        exit_code
    );
    assert!(
        syscalls_after >= syscalls_before + 9,
        "expected >= 9 syscalls executed from Ring 3"
    );

    crate::kinfo!(
        "[TEST PASS] ring3_syscalls_fork_and_pointer_validation (pid={}, exit_code={}, syscalls={})",
        pid,
        exit_code,
        syscalls_after - syscalls_before
    );
}

fn test_ring3_fault_isolation() {
    // 1. Direct kernel memory read from Ring 3 must fault (#PF) and terminate process with 139 (128+SIGSEGV)
    let kread_code = code_slice(
        addr_of!(ring3_bad_kread_start),
        addr_of!(ring3_bad_kread_end),
    );
    let pid1 = sched::spawn_user_code_process("malicious_kread", kread_code, Credentials::root())
        .expect("spawn malicious_kread");
    let code1 = sched::wait_tid(pid1).expect("wait malicious_kread");
    assert_eq!(
        code1, 139,
        "Ring 3 kernel-read attempt must be killed with SIGSEGV (139)"
    );

    // 2. Privileged instruction (`cli`) in Ring 3 must fault (#GP) and terminate process with 139
    let priv_code = code_slice(addr_of!(ring3_bad_priv_start), addr_of!(ring3_bad_priv_end));
    let pid2 = sched::spawn_user_code_process("malicious_cli", priv_code, Credentials::root())
        .expect("spawn malicious_cli");
    let code2 = sched::wait_tid(pid2).expect("wait malicious_cli");
    assert_eq!(
        code2, 139,
        "Ring 3 privileged instruction must be killed with SIGSEGV (139)"
    );

    crate::kinfo!(
        "[TEST PASS] ring3_fault_isolation (kread_exit={}, priv_cli_exit={})",
        code1,
        code2
    );
}
