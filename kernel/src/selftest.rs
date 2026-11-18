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

    // Milestone 5 tests (IPC: Pipes, Unix-Domain Sockets, Shared Memory)
    test_ipc_pipes_unix_sockets_and_shm();

    // Milestone 6 tests (VFS, /dev, /proc, /tmp sticky bit, symlinks, initramfs)
    test_vfs_hierarchy_devices_and_proc();

    // Milestone 7 tests (AetherFS: 1 PiB sparse file, Superblock failover, WAL Journal recovery)
    test_aetherfs_1pib_and_crash_recovery();

    // Milestone 8 & 10 tests (ELF64 Loader, Multi-User Security, UUID ACLs, execute_as_uuid, Sudo)
    test_elf64_loader_and_ring3_execution();
    test_multiuser_permissions_acl_and_execute_as();

    // Milestone 9 & 11 tests (User-Space /sbin/init, /bin/aethersh pipelines/redirection, coreutils, aetherpkg, sudo)
    test_userspace_init_aethersh_pipelines_and_aetherpkg();

    // Milestone 12 tests (PCI enumeration, CMOS RTC, Networking Stack Ethernet/IPv4/ICMP/UDP & packet fuzzing)
    test_drivers_pci_rtc_and_network_stack();

    crate::kinfo!("[AETHEROS-TEST] ALL MILESTONE 1 - 12 KERNEL & USERSPACE TESTS PASSED");
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

fn test_ipc_pipes_unix_sockets_and_shm() {
    use crate::ipc::{self, Pipe, UnixSocket, SOCK_STREAM};
    use aether_abi::open_flags::{O_CREAT, O_RDWR};
    use aether_abi::Errno;

    // 1. Anonymous pipe read/write, EOF on writer drop, EPIPE on reader drop
    let (r, w) = Pipe::new_pair();
    let msg = b"AetherOS-IPC-Pipe-Message";
    assert_eq!(w.inner.write(msg, false), Ok(msg.len()));
    let mut rbuf = [0u8; 32];
    assert_eq!(r.inner.read(&mut rbuf, false), Ok(msg.len()));
    assert_eq!(&rbuf[..msg.len()], msg);

    drop(w);
    // Reading after all writers closed must return Ok(0) (EOF)
    assert_eq!(r.inner.read(&mut rbuf, false), Ok(0));

    let (r2, w2) = Pipe::new_pair();
    drop(r2);
    // Writing after all readers closed must return Err(Errno::Epipe)
    assert_eq!(w2.inner.write(b"broken", true), Err(Errno::Epipe));

    // 2. Unix-domain stream socket bind/listen/connect/accept/read/write
    let server_sock = UnixSocket::new(SOCK_STREAM).expect("server UnixSocket");
    server_sock
        .bind(String::from("/run/test_ipc.sock"))
        .expect("bind");
    server_sock.listen(8).expect("listen");

    let client_sock = UnixSocket::new(SOCK_STREAM).expect("client UnixSocket");
    client_sock.connect("/run/test_ipc.sock").expect("connect");
    let accepted_sock = server_sock.accept(true).expect("accept");

    assert_eq!(client_sock.write(b"ping-unix", true), Ok(9));
    let mut sbuf = [0u8; 16];
    assert_eq!(accepted_sock.read(&mut sbuf, true), Ok(9));
    assert_eq!(&sbuf[..9], b"ping-unix");

    assert_eq!(accepted_sock.write(b"pong-unix", true), Ok(9));
    assert_eq!(client_sock.read(&mut sbuf, true), Ok(9));
    assert_eq!(&sbuf[..9], b"pong-unix");

    // 3. Shared memory object creation, page backing, and unlink
    let shm =
        ipc::shm_open_object("/aether_test_shm", O_CREAT | O_RDWR, 0o600).expect("shm_open_object");
    let frames = shm.ensure_pages(2).expect("shm ensure_pages(2)");
    assert_eq!(frames.len(), 2);
    unsafe {
        *pmm::phys_to_virt(frames[0]) = 0x7E;
    }
    let shm_reopen =
        ipc::shm_open_object("/aether_test_shm", O_RDWR, 0).expect("reopen existing shm");
    let frames2 = shm_reopen.ensure_pages(2).unwrap();
    assert_eq!(frames2[0], frames[0]);
    ipc::shm_unlink_object("/aether_test_shm").expect("shm_unlink_object");

    crate::kinfo!("[TEST PASS] ipc_pipes_unix_sockets_and_shm");
}

fn test_vfs_hierarchy_devices_and_proc() {
    use crate::fs;
    use aether_abi::open_flags::{O_CREAT, O_RDONLY, O_RDWR, O_TRUNC};

    // 1. Verify initramfs /etc/os-release was unpacked into VFS
    let os_rel = fs::vfs_open("/etc/os-release", O_RDONLY, 0).expect("open /etc/os-release");
    let mut buf = [0u8; 256];
    let n = fs::vfs_read_file(&os_rel, &mut buf, None).expect("read /etc/os-release");
    let os_str = core::str::from_utf8(&buf[..n]).unwrap();
    assert!(
        os_str.contains("NAME=\"AetherOS\""),
        "/etc/os-release missing expected banner"
    );

    // 2. Verify /dev/null, /dev/zero, /dev/urandom
    let dev_null = fs::vfs_open("/dev/null", O_RDWR, 0).expect("open /dev/null");
    assert_eq!(fs::vfs_write_file(&dev_null, b"discarded", None), Ok(9));
    assert_eq!(fs::vfs_read_file(&dev_null, &mut buf[..16], None), Ok(0));

    let dev_zero = fs::vfs_open("/dev/zero", O_RDONLY, 0).expect("open /dev/zero");
    let mut zbuf = [0xAAu8; 32];
    assert_eq!(fs::vfs_read_file(&dev_zero, &mut zbuf, None), Ok(32));
    assert_eq!(zbuf, [0u8; 32]);

    let dev_urandom = fs::vfs_open("/dev/urandom", O_RDONLY, 0).expect("open /dev/urandom");
    let mut r1 = [0u8; 16];
    let mut r2 = [0u8; 16];
    assert_eq!(fs::vfs_read_file(&dev_urandom, &mut r1, None), Ok(16));
    assert_eq!(fs::vfs_read_file(&dev_urandom, &mut r2, None), Ok(16));
    assert_ne!(r1, r2, "/dev/urandom must produce non-repeating entropy");

    // 3. Verify /proc/version and /proc/meminfo
    let proc_ver = fs::vfs_open("/proc/version", O_RDONLY, 0).expect("open /proc/version");
    let nv = fs::vfs_read_file(&proc_ver, &mut buf, None).expect("read /proc/version");
    assert!(core::str::from_utf8(&buf[..nv])
        .unwrap()
        .contains("AetherOS"));

    // 4. Create file, write, symlink, read through symlink, unlink
    let f = fs::vfs_open("/tmp/vfs_test.txt", O_CREAT | O_RDWR | O_TRUNC, 0o644)
        .expect("create /tmp/vfs_test.txt");
    assert_eq!(fs::vfs_write_file(&f, b"VFS-Content-OK", None), Ok(14));
    fs::vfs_symlink("/tmp/vfs_test.txt", "/tmp/vfs_link.txt").expect("vfs_symlink");
    assert_eq!(
        fs::vfs_readlink("/tmp/vfs_link.txt").as_deref(),
        Ok("/tmp/vfs_test.txt")
    );
    let lf = fs::vfs_open("/tmp/vfs_link.txt", O_RDONLY, 0).expect("open symlink");
    let nl = fs::vfs_read_file(&lf, &mut buf, None).expect("read via symlink");
    assert_eq!(&buf[..nl], b"VFS-Content-OK");

    fs::vfs_unlink("/tmp/vfs_link.txt").expect("unlink symlink");
    fs::vfs_unlink("/tmp/vfs_test.txt").expect("unlink file");

    crate::kinfo!("[TEST PASS] vfs_hierarchy_devices_and_proc");
}

fn test_aetherfs_1pib_and_crash_recovery() {
    crate::fs::aetherfs::run_aetherfs_self_tests()
        .expect("AetherFS 1 PiB sparse + superblock failover + WAL journal crash recovery test");
    crate::kinfo!(
        "[TEST PASS] aetherfs_1pib_and_crash_recovery (1 PiB sparse + SB failover + WAL replay)"
    );
}

fn test_elf64_loader_and_ring3_execution() {
    use crate::fs;
    use aether_abi::open_flags::{O_CREAT, O_RDWR, O_TRUNC};
    use aether_elf::{build_minimal_elf64_image, parse_and_validate_elf64, ElfError};

    // Build a real x86_64 ELF64 binary whose `_start` checks `argc == 3` (`rdi == 3`),
    // and exits with `sys_exit(55)` (`rax=70, rdi=55`) if `argc == 3`, else `sys_exit(1)`.
    // Machine code:
    //   48 83 ff 03             cmp rdi, 3
    //   75 09                   jne 1f
    //   b8 46 00 00 00          mov eax, 70
    //   bf 37 00 00 00          mov edi, 55
    //   0f 05                   syscall
    // 1:
    //   b8 46 00 00 00          mov eax, 70
    //   bf 01 00 00 00          mov edi, 1
    //   0f 05                   syscall
    let elf_code: [u8; 28] = [
        0x48, 0x83, 0xFF, 0x03, 0x75, 0x0C, 0xB8, 0x46, 0x00, 0x00, 0x00, 0xBF, 0x37, 0x00, 0x00,
        0x00, 0x0F, 0x05, 0xB8, 0x46, 0x00, 0x00, 0x00, 0xBF, 0x01, 0x00, 0x00, 0x0F,
    ];
    let mut full_code = Vec::from(&elf_code[..]);
    full_code.push(0x05);

    let mut elf_buf = alloc::vec![0u8; 4096 + 64];
    let elf_len = build_minimal_elf64_image(&full_code, &mut elf_buf);

    // Verify malformed ELF targeting kernel memory is rejected
    let mut bad_elf = elf_buf[..elf_len].to_vec();
    bad_elf[24..32].copy_from_slice(&0xFFFF_FFFF_8000_0000u64.to_le_bytes());
    bad_elf[64 + 16..64 + 24].copy_from_slice(&0xFFFF_FFFF_8000_0000u64.to_le_bytes());
    assert_eq!(
        parse_and_validate_elf64(&bad_elf),
        Err(ElfError::KernelOrNullAddressTarget)
    );

    // Write valid ELF64 executable into `/bin/test_elf` in VFS with mode 0o755
    let f = fs::vfs_open("/bin/test_elf", O_CREAT | O_RDWR | O_TRUNC, 0o755)
        .expect("create /bin/test_elf");
    fs::vfs_write_file(&f, &elf_buf[..elf_len], None).expect("write /bin/test_elf");

    let argv = alloc::vec![
        String::from("/bin/test_elf"),
        String::from("--flag"),
        String::from("aether"),
    ];
    let envp = alloc::vec![String::from("PATH=/bin:/sbin"), String::from("USER=root")];

    let pid = sched::exec::spawn_elf_process_from_vfs(
        "/bin/test_elf",
        &argv,
        &envp,
        Credentials::root(),
        None,
    )
    .expect("spawn_elf_process_from_vfs");
    let exit_code = sched::wait_tid(pid).expect("wait_tid(/bin/test_elf)");
    assert_eq!(
        exit_code, 55,
        "ELF64 Ring 3 binary must verify argc==3 and exit with 55"
    );

    crate::kinfo!(
        "[TEST PASS] elf64_loader_and_ring3_execution (pid={}, argc=3, exit_code={})",
        pid,
        exit_code
    );
}

fn test_multiuser_permissions_acl_and_execute_as() {
    use crate::fs::{self, AclEntry, MS_NOSUID};
    use crate::security;
    use aether_abi::open_flags::{O_CREAT, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY};
    use aether_abi::Errno;
    use aether_uuid::Uuid;

    // 1. Root creates a protected file `/etc/secret.conf` (mode 0o600) and a sticky `/tmp/root_note`
    let f = fs::vfs_open("/etc/secret.conf", O_CREAT | O_RDWR | O_TRUNC, 0o600)
        .expect("create /etc/secret.conf");
    fs::vfs_write_file(&f, b"root-only-secret", None).unwrap();

    let tf = fs::vfs_open("/tmp/root_sticky_file", O_CREAT | O_RDWR | O_TRUNC, 0o644)
        .expect("create /tmp/root_sticky_file");
    fs::vfs_write_file(&tf, b"sticky-protected", None).unwrap();

    // 2. Root sets `execute_as_uuid = ROOT_USER` on `/bin/test_elf`
    fs::vfs_set_execute_as("/bin/test_elf", Uuid::ROOT_USER).expect("root set execute_as_uuid");
    assert_eq!(fs::vfs_get_execute_as("/bin/test_elf"), Ok(Uuid::ROOT_USER));
    // Setting a bogus/deleted UUID must be rejected with Errno::Einval!
    let bogus_uuid = Uuid::deterministic(0xDEAD, 0xBEEF);
    assert_eq!(
        fs::vfs_set_execute_as("/bin/test_elf", bogus_uuid),
        Err(Errno::Einval),
        "bogus execute_as_uuid must be rejected"
    );

    // 3. Temporarily switch current task credentials to unprivileged user `aether` (UID 1000)
    let saved_root_creds = sched::with_current_task(|t| t.creds.clone());
    let unpriv_creds = security::aether_user_credentials();
    sched::with_current_task_mut(|t| t.creds = unpriv_creds.clone());

    // 3a. Unprivileged user MUST be denied reading/writing `/etc/secret.conf` (0o600)
    assert_eq!(
        fs::vfs_open("/etc/secret.conf", O_RDONLY, 0).err(),
        Some(Errno::Eacces)
    );
    assert_eq!(
        fs::vfs_open("/etc/secret.conf", O_WRONLY, 0).err(),
        Some(Errno::Eacces)
    );

    // 3b. Unprivileged user MUST be denied deleting root's file in sticky `/tmp` (0o1777)
    assert_eq!(
        fs::vfs_unlink("/tmp/root_sticky_file"),
        Err(Errno::Eperm),
        "sticky bit on /tmp must block non-owner unlink"
    );

    // 3c. Unprivileged user MUST be denied modifying `execute_as_uuid`!
    assert_eq!(
        fs::vfs_set_execute_as("/bin/test_elf", Uuid::ROOT_USER),
        Err(Errno::Eperm),
        "unprivileged user must never be allowed to set execute_as_uuid"
    );

    // 3d. Verify `execute_as_uuid` credential transition when `aether` (UID 1000) runs `/bin/test_elf`:
    let (_, elf_node, mount_flags) =
        fs::vfs_read_executable_and_meta("/bin/test_elf", &unpriv_creds).unwrap();
    let transitioned =
        security::compute_exec_credentials(&unpriv_creds, &elf_node, mount_flags, "/bin/test_elf")
            .expect("compute_exec_credentials");
    assert_eq!(
        transitioned.euid, 0,
        "execute_as_uuid must transition euid to 0"
    );
    assert_eq!(transitioned.user_uuid, Uuid::ROOT_USER);

    // 3e. Verify `MS_NOSUID` and `no_new_privs` block `execute_as_uuid` elevation!
    assert_eq!(
        security::compute_exec_credentials(&unpriv_creds, &elf_node, MS_NOSUID, "/bin/test_elf")
            .err(),
        Some(Errno::Eperm)
    );
    let mut nnp_creds = unpriv_creds.clone();
    nnp_creds.no_new_privs = true;
    assert_eq!(
        security::compute_exec_credentials(&nnp_creds, &elf_node, 0, "/bin/test_elf").err(),
        Some(Errno::Eperm)
    );

    // 4. Restore root credentials, grant `aether` read access to `/etc/secret.conf` via UUID ACL!
    sched::with_current_task_mut(|t| t.creds = saved_root_creds.clone());
    let acl = alloc::vec![AclEntry {
        subject_uuid: *security::AETHER_USER_UUID.as_bytes(),
        is_group: 0,
        allow_rwx: 4, // Read allowed for AETHER_USER_UUID
        deny_rwx: 0,
        _pad: 0,
    }];
    fs::vfs_set_acl("/etc/secret.conf", acl).expect("vfs_set_acl");

    // Switch back to `aether` (UID 1000): read MUST now succeed via UUID ACL, while write MUST still fail!
    sched::with_current_task_mut(|t| t.creds = unpriv_creds);
    let rf = fs::vfs_open("/etc/secret.conf", O_RDONLY, 0)
        .expect("UUID ACL must grant read access to aether");
    let mut sbuf = [0u8; 32];
    let nr = fs::vfs_read_file(&rf, &mut sbuf, None).unwrap();
    assert_eq!(&sbuf[..nr], b"root-only-secret");
    assert_eq!(
        fs::vfs_open("/etc/secret.conf", O_WRONLY, 0).err(),
        Some(Errno::Eacces),
        "UUID ACL granted read only; write must still be denied"
    );

    // 5. Test kernel-controlled sudo development mode elevation for `%wheel` member `aether`
    security::sys_elevate_sudo_current("selftest-sudo-check")
        .expect("wheel member sudo dev elevation");
    assert_eq!(sched::with_current_task(|t| t.creds.euid), 0);

    // Restore root credentials and clean up test files
    sched::with_current_task_mut(|t| t.creds = saved_root_creds);
    fs::vfs_set_execute_as("/bin/test_elf", Uuid::NIL).unwrap();
    fs::vfs_unlink("/tmp/root_sticky_file").unwrap();

    // 6. Test AetherFS persistence across simulated reboot (`remount_and_recover`)
    let report = fs::aetherfs::remount_and_recover().expect("remount_and_recover");
    let persisted_acl = fs::vfs_get_acl("/etc/secret.conf").expect("persisted ACL");
    assert_eq!(persisted_acl.len(), 1);
    assert_eq!(
        persisted_acl[0].subject_uuid,
        *security::AETHER_USER_UUID.as_bytes()
    );

    crate::kinfo!(
        "[TEST PASS] multiuser_permissions_acl_and_execute_as (acl_entries={}, recovery_replayed={})",
        persisted_acl.len(),
        report.replayed_journal_tx
    );
}

fn test_userspace_init_aethersh_pipelines_and_aetherpkg() {
    use crate::fs;
    use crate::security;

    // 1. Spawn `/sbin/init --verify-once` in Ring 3!
    //    `/sbin/init` reads `/etc/services.conf`, runs `/bin/uname -a` and `/bin/aethersh --boot-verify`
    //    (which executes `echo hello > /tmp/sh_test.txt`, `echo world >> /tmp/sh_test.txt`,
    //     `cat /tmp/sh_test.txt`, `ps | grep init`, `id`, `ls -l /etc`, `aetherpkg list`).
    let init_argv = alloc::vec![String::from("/sbin/init"), String::from("--verify-once"),];
    let init_envp = alloc::vec![
        String::from("PATH=/bin:/sbin:/usr/bin"),
        String::from("HOME=/root"),
    ];
    let init_pid = sched::exec::spawn_elf_process_from_vfs(
        "/sbin/init",
        &init_argv,
        &init_envp,
        Credentials::root(),
        None,
    )
    .expect("spawn /sbin/init --verify-once");
    let init_exit = sched::wait_tid(init_pid).expect("wait_tid(/sbin/init)");
    assert_eq!(
        init_exit, 0,
        "/sbin/init --verify-once exited with {}",
        init_exit
    );

    // 2. Verify that `aethersh` redirection created `/tmp/sh_test.txt` with `"hello\nworld\n"`!
    let sh_out = fs::vfs_read_all("/tmp/sh_test.txt").expect("read /tmp/sh_test.txt");
    assert_eq!(
        core::str::from_utf8(&sh_out).unwrap(),
        "hello\nworld\n",
        "aethersh '>' and '>>' redirection output mismatch"
    );

    // 3. Verify that `/sbin/init` wrote structured service supervision logs to `/var/log/init.log`
    let init_log = fs::vfs_read_all("/var/log/init.log").expect("read /var/log/init.log");
    let init_log_str = core::str::from_utf8(&init_log).unwrap();
    assert!(init_log_str.contains("Starting service 'aethersh'"));

    // 4. Test `aetherpkg install aether-devtools` (resolves dependency `aether-base`) and `aetherpkg verify`
    let pkg_argv = alloc::vec![
        String::from("/bin/aetherpkg"),
        String::from("install"),
        String::from("aether-devtools"),
    ];
    let pkg_pid = sched::exec::spawn_elf_process_from_vfs(
        "/bin/aetherpkg",
        &pkg_argv,
        &init_envp,
        Credentials::root(),
        None,
    )
    .expect("spawn /bin/aetherpkg install");
    assert_eq!(sched::wait_tid(pkg_pid), Ok(0));

    let verify_argv = alloc::vec![String::from("/bin/aetherpkg"), String::from("verify")];
    let ver_pid = sched::exec::spawn_elf_process_from_vfs(
        "/bin/aetherpkg",
        &verify_argv,
        &init_envp,
        Credentials::root(),
        None,
    )
    .expect("spawn /bin/aetherpkg verify");
    assert_eq!(sched::wait_tid(ver_pid), Ok(0));

    // 5. Test `/bin/sudo /bin/id` launched as unprivileged user `aether` (UID 1000, in %wheel)
    let sudo_argv = alloc::vec![String::from("/bin/sudo"), String::from("/bin/id")];
    let sudo_pid = sched::exec::spawn_elf_process_from_vfs(
        "/bin/sudo",
        &sudo_argv,
        &init_envp,
        security::aether_user_credentials(),
        None,
    )
    .expect("spawn /bin/sudo /bin/id as UID 1000");
    assert_eq!(sched::wait_tid(sudo_pid), Ok(0));

    crate::kinfo!(
        "[TEST PASS] userspace_init_aethersh_pipelines_and_aetherpkg (init_pid={}, log_bytes={})",
        init_pid,
        init_log.len()
    );
}

fn test_drivers_pci_rtc_and_network_stack() {
    use crate::drivers::{pci, rtc};
    use crate::net::{
        self, build_ipv4_icmp_echo, build_ipv4_udp_packet, parse_ipv4_packet, ICMP_ECHO_REPLY,
        ICMP_ECHO_REQUEST,
    };

    // 1. PCI Enumeration
    let devs = pci::devices();
    assert!(
        devs.len() >= 3,
        "expected >= 3 PCI devices in QEMU, found {}",
        devs.len()
    );

    // 2. CMOS RTC
    let dt = rtc::read_rtc_datetime();
    assert!(dt.year >= 2024 && (1..=12).contains(&dt.month) && (1..=31).contains(&dt.day));

    // 3. Networking Stack: IPv4 + ICMP Echo Request -> Echo Reply with verified RFC 1071 checksum
    let ping_pkt = build_ipv4_icmp_echo(
        [10, 0, 2, 2],
        [10, 0, 2, 15],
        ICMP_ECHO_REQUEST,
        0xAE74,
        1,
        b"AetherOS-Ping-Payload",
    );
    let reply = net::process_inbound_ipv4(&ping_pkt)
        .expect("process ICMP Echo Request")
        .expect("expected ICMP Echo Reply");
    let (rep_hdr, rep_icmp) = parse_ipv4_packet(&reply).expect("parse ICMP Echo Reply");
    assert_eq!(rep_hdr.src_ip, [10, 0, 2, 15]);
    assert_eq!(rep_hdr.dst_ip, [10, 0, 2, 2]);
    assert_eq!(rep_icmp[0], ICMP_ECHO_REPLY);
    assert_eq!(&rep_icmp[8..], b"AetherOS-Ping-Payload");

    // 4. UDP loopback delivery
    let udp_pkt = build_ipv4_udp_packet([127, 0, 0, 1], [127, 0, 0, 1], 40000, 8080, b"UDP-OK");
    assert_eq!(net::process_inbound_ipv4(&udp_pkt), Ok(None));
    let (src_ip, src_port, payload) =
        net::recv_udp_datagram(8080).expect("recv_udp_datagram(8080)");
    assert_eq!(src_ip, [127, 0, 0, 1]);
    assert_eq!(src_port, 40000);
    assert_eq!(&payload, b"UDP-OK");

    // 5. Fuzz / bounds-check 2,000 mutated/truncated IPv4 packets -> must never panic
    let mut rng = 0xCAFE_F00D_1234_5678u64;
    for i in 0..2000u64 {
        let mut mutated = ping_pkt.clone();
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let idx = (rng as usize) % mutated.len();
        mutated[idx] ^= (rng >> 8) as u8;
        let len = if i % 5 == 0 {
            (rng as usize) % mutated.len()
        } else {
            mutated.len()
        };
        let _ = net::process_inbound_ipv4(&mutated[..len]);
    }

    let (rx, tx, icmp_replies) = net::stats();
    assert!(rx >= 2 && tx >= 1 && icmp_replies >= 1);
    crate::kinfo!(
        "[TEST PASS] drivers_pci_rtc_and_network_stack (pci_devs={}, rtc={:04}-{:02}-{:02}, icmp_replies={})",
        devs.len(),
        dt.year,
        dt.month,
        dt.day,
        icmp_replies
    );
}
