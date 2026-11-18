//! ELF64 Program Loader, User Stack Construction (`argv`, `envp`, `auxv`),
//! and `sys_execve` / `execute_as_uuid` Credential Transition (Milestone 8).

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use aether_abi::Errno;
use aether_elf::parse_and_validate_elf64;

use crate::arch::x86_64::gdt;
use crate::arch::x86_64::idt::TrapFrame;
use crate::arch::x86_64::paging::{PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE};
use crate::fs::{self, default_stdio_fds};
use crate::mm::pmm;
use crate::mm::vmm::{self, AddressSpace};
use crate::mm::{align_down, align_up, PAGE_SIZE};
use crate::sched::{
    self, alloc_tid, current_pid, Credentials, Task, TaskState, DEFAULT_QUANTUM_TICKS,
    PRIORITY_NORMAL, SCHED, SCHED_YIELD_VECTOR,
};
use crate::security;

const USER_STACK_PAGES: u64 = 8; // 32 KiB Ring 3 stack + 4 KiB bottom guard page
const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_0000;
const USER_STACK_GUARD: u64 = USER_STACK_TOP - (USER_STACK_PAGES + 1) * (PAGE_SIZE as u64);

// Standard ELF auxiliary vector tags
const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;

pub struct LoadedElfImage {
    pub address_space: AddressSpace,
    pub entry_point: u64,
    pub user_rsp: u64,
    pub argc: u64,
    pub argv_ptr: u64,
    pub envp_ptr: u64,
    pub brk_base: u64,
    pub new_creds: Credentials,
}

/// Load a validated ELF64 binary into a fresh isolated user `AddressSpace`, populate its
/// guarded Ring 3 stack with `argv`, `envp`, and `auxv`, and return `LoadedElfImage`.
pub fn load_elf_into_new_address_space(
    elf_bytes: &[u8],
    argv: &[String],
    envp: &[String],
    new_creds: Credentials,
) -> Result<LoadedElfImage, Errno> {
    let parsed = parse_and_validate_elf64(elf_bytes).map_err(|_| Errno::Enoexec)?;

    let aspace = AddressSpace::new_user().map_err(|_| Errno::Enomem)?;

    // 1. Map all PT_LOAD segments with strict W^X page permissions
    for seg in parsed.load_segments.iter().take(parsed.load_count) {
        let seg_page_start = align_down(seg.vaddr, PAGE_SIZE as u64);
        let seg_page_end = align_up(seg.vaddr + seg.memsz, PAGE_SIZE as u64);
        let mut pte_flags = PTE_PRESENT | PTE_USER;
        if seg.is_writable() {
            pte_flags |= PTE_WRITABLE;
        }
        if !seg.is_executable() {
            pte_flags |= PTE_NO_EXECUTE;
        }

        let mut page_vaddr = seg_page_start;
        while page_vaddr < seg_page_end {
            // Allocate or reuse page if two adjacent segments share a page boundary
            let phys = match aspace.translate(page_vaddr) {
                Some((existing_phys, _)) => existing_phys & !0xFFF,
                None => {
                    let f = pmm::alloc_zeroed_frame().map_err(|_| {
                        aspace.destroy_user();
                        Errno::Enomem
                    })?;
                    if aspace.map_page(page_vaddr, f, pte_flags).is_err() {
                        let _ = pmm::free_frame(f);
                        aspace.destroy_user();
                        return Err(Errno::Enomem);
                    }
                    f
                }
            };

            // Copy overlapping file bytes `[seg.vaddr, seg.vaddr + seg.filesz)` into this 4 KiB page
            let file_vaddr_start = seg.vaddr;
            let file_vaddr_end = seg.vaddr + (seg.filesz as u64);
            let copy_start = page_vaddr.max(file_vaddr_start);
            let copy_end = (page_vaddr + (PAGE_SIZE as u64)).min(file_vaddr_end);
            if copy_end > copy_start {
                let page_off = (copy_start - page_vaddr) as usize;
                let file_off = seg.file_offset + ((copy_start - seg.vaddr) as usize);
                let len = (copy_end - copy_start) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf_bytes[file_off..file_off + len].as_ptr(),
                        pmm::phys_to_virt(phys).add(page_off),
                        len,
                    );
                }
            }

            page_vaddr += PAGE_SIZE as u64;
        }
    }

    // 2. Map guarded Ring 3 user stack (bottom 4 KiB guard page + 8 * 4 KiB RW+NX stack pages)
    if aspace.map_guard_page(USER_STACK_GUARD).is_err() {
        aspace.destroy_user();
        return Err(Errno::Enomem);
    }
    let mut top_page_phys: u64 = 0;
    for i in 1..=USER_STACK_PAGES {
        let v = USER_STACK_GUARD + i * (PAGE_SIZE as u64);
        let phys = pmm::alloc_zeroed_frame().map_err(|_| {
            aspace.destroy_user();
            Errno::Enomem
        })?;
        if aspace
            .map_page(
                v,
                phys,
                PTE_PRESENT | PTE_WRITABLE | PTE_USER | PTE_NO_EXECUTE,
            )
            .is_err()
        {
            let _ = pmm::free_frame(phys);
            aspace.destroy_user();
            return Err(Errno::Enomem);
        }
        if i == USER_STACK_PAGES {
            top_page_phys = phys;
        }
    }

    // 3. Serialize `argv`, `envp`, and `auxv` into the top 4 KiB page of the user stack
    //    (`[USER_STACK_TOP - 4096 .. USER_STACK_TOP)` backed by `top_page_phys`).
    let top_page_vaddr = USER_STACK_TOP - (PAGE_SIZE as u64);
    let top_page_buf =
        unsafe { core::slice::from_raw_parts_mut(pmm::phys_to_virt(top_page_phys), PAGE_SIZE) };
    let mut cursor: usize = PAGE_SIZE;

    let mut env_ptrs: Vec<u64> = Vec::with_capacity(envp.len());
    for e in envp.iter().take(32) {
        let b = e.as_bytes();
        let need = b.len() + 1;
        if cursor < need + 1024 {
            aspace.destroy_user();
            return Err(Errno::E2big);
        }
        cursor -= need;
        top_page_buf[cursor..cursor + b.len()].copy_from_slice(b);
        top_page_buf[cursor + b.len()] = 0;
        env_ptrs.push(top_page_vaddr + (cursor as u64));
    }

    let mut arg_ptrs: Vec<u64> = Vec::with_capacity(argv.len());
    for a in argv.iter().take(64) {
        let b = a.as_bytes();
        let need = b.len() + 1;
        if cursor < need + 512 {
            aspace.destroy_user();
            return Err(Errno::E2big);
        }
        cursor -= need;
        top_page_buf[cursor..cursor + b.len()].copy_from_slice(b);
        top_page_buf[cursor + b.len()] = 0;
        arg_ptrs.push(top_page_vaddr + (cursor as u64));
    }

    // Align string pool bottom down to 16 bytes
    cursor &= !0xF;

    // Count total qwords in stack table:
    // argc (1) + argv (arg_ptrs.len() + 1) + envp (env_ptrs.len() + 1) + auxv (9 pairs = 18)
    let auxv_pairs: [(u64, u64); 9] = [
        (AT_PAGESZ, PAGE_SIZE as u64),
        (AT_PHDR, parsed.phdr_vaddr),
        (AT_PHENT, 56),
        (AT_PHNUM, parsed.phnum as u64),
        (AT_ENTRY, parsed.entry_point),
        (AT_UID, new_creds.uid as u64),
        (AT_EUID, new_creds.euid as u64),
        (AT_GID, new_creds.gid as u64),
        (AT_EGID, new_creds.egid as u64),
    ];
    let total_qwords =
        1 + (arg_ptrs.len() + 1) + (env_ptrs.len() + 1) + ((auxv_pairs.len() + 1) * 2);
    let table_bytes = total_qwords * 8;
    // Ensure final `user_rsp` (`cursor - table_bytes`) is 16-byte aligned per System V AMD64 ABI!
    let mut table_start = (cursor - table_bytes) & !0xF;
    let user_rsp = top_page_vaddr + (table_start as u64);

    let argc = arg_ptrs.len() as u64;
    top_page_buf[table_start..table_start + 8].copy_from_slice(&argc.to_le_bytes());
    table_start += 8;

    let argv_ptr = top_page_vaddr + (table_start as u64);
    for &p in &arg_ptrs {
        top_page_buf[table_start..table_start + 8].copy_from_slice(&p.to_le_bytes());
        table_start += 8;
    }
    top_page_buf[table_start..table_start + 8].fill(0);
    table_start += 8;

    let envp_ptr = top_page_vaddr + (table_start as u64);
    for &p in &env_ptrs {
        top_page_buf[table_start..table_start + 8].copy_from_slice(&p.to_le_bytes());
        table_start += 8;
    }
    top_page_buf[table_start..table_start + 8].fill(0);
    table_start += 8;

    for (k, v) in auxv_pairs {
        top_page_buf[table_start..table_start + 8].copy_from_slice(&k.to_le_bytes());
        table_start += 8;
        top_page_buf[table_start..table_start + 8].copy_from_slice(&v.to_le_bytes());
        table_start += 8;
    }
    top_page_buf[table_start..table_start + 8].copy_from_slice(&AT_NULL.to_le_bytes());
    top_page_buf[table_start + 8..table_start + 16].fill(0);

    let brk_base = align_up(parsed.highest_vaddr_end, PAGE_SIZE as u64) + (PAGE_SIZE as u64);

    Ok(LoadedElfImage {
        address_space: aspace,
        entry_point: parsed.entry_point,
        user_rsp,
        argc,
        argv_ptr,
        envp_ptr,
        brk_base,
        new_creds,
    })
}

/// Replace the currently executing process image with an ELF64 executable at `path` (`sys_execve`).
/// Applies `execute_as_uuid` / `S_ISUID` / `S_ISGID` credential transitions and updates `frame` in-place.
pub fn execve_current_from_frame(
    frame: &mut TrapFrame,
    raw_path: &str,
    argv: &[String],
    envp: &[String],
) -> Result<(), Errno> {
    let caller_creds = sched::with_current_task(|t| t.creds.clone());
    let (elf_bytes, node, mount_flags) = fs::vfs_read_executable_and_meta(raw_path, &caller_creds)?;
    let new_creds =
        security::compute_exec_credentials(&caller_creds, &node, mount_flags, raw_path)?;

    let loaded = load_elf_into_new_address_space(&elf_bytes, argv, envp, new_creds)?;

    let basename = raw_path.rsplit('/').next().unwrap_or(raw_path);
    let old_aspace = sched::with_current_task_mut(|t| {
        let old = t.address_space;
        t.address_space = loaded.address_space;
        t.is_user = true;
        t.user_brk_base = loaded.brk_base;
        t.user_brk_current = loaded.brk_base;
        t.user_mmap_next = 0x0000_4000_0000_0000;
        t.creds = loaded.new_creds;
        t.close_cloexec_fds();
        t.set_name(basename);
        old
    });

    loaded.address_space.activate();
    old_aspace.destroy_user();

    // Reset TrapFrame to jump to the ELF64 entry point in Ring 3 with (rdi=argc, rsi=argv, rdx=envp)
    *frame = TrapFrame {
        r15: 0,
        r14: 0,
        r13: 0,
        r12: 0,
        r11: 0x202,
        r10: 0,
        r9: 0,
        r8: 0,
        rbp: 0,
        rdi: loaded.argc,
        rsi: loaded.argv_ptr,
        rdx: loaded.envp_ptr,
        rcx: loaded.entry_point,
        rbx: 0,
        rax: 0,
        vector: SCHED_YIELD_VECTOR as u64,
        error_code: 0,
        rip: loaded.entry_point,
        cs: gdt::USER_CS as u64,
        rflags: 0x202,
        rsp: loaded.user_rsp,
        ss: gdt::USER_DS as u64,
    };

    Ok(())
}

/// Spawn a new Ring 3 user process from a VFS ELF64 file path (with optional fixed PID override, e.g. PID 1 for `/sbin/init`).
pub fn spawn_elf_process_from_vfs(
    raw_path: &str,
    argv: &[String],
    envp: &[String],
    initial_creds: Credentials,
    fixed_pid_override: Option<u32>,
) -> Result<u32, Errno> {
    let (elf_bytes, node, mount_flags) =
        fs::vfs_read_executable_and_meta(raw_path, &initial_creds)?;
    let eff_creds =
        security::compute_exec_credentials(&initial_creds, &node, mount_flags, raw_path)?;
    let loaded = load_elf_into_new_address_space(&elf_bytes, argv, envp, eff_creds)?;

    let kstack_top = vmm::alloc_guarded_kernel_stack(16).map_err(|_| {
        loaded.address_space.destroy_user();
        Errno::Enomem
    })?;
    let frame_addr = (kstack_top - (size_of::<TrapFrame>() as u64)) & !0xF;
    let frame_ptr = frame_addr as *mut TrapFrame;

    unsafe {
        frame_ptr.write(TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0x202,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: loaded.argc,
            rsi: loaded.argv_ptr,
            rdx: loaded.envp_ptr,
            rcx: loaded.entry_point,
            rbx: 0,
            rax: 0,
            vector: SCHED_YIELD_VECTOR as u64,
            error_code: 0,
            rip: loaded.entry_point,
            cs: gdt::USER_CS as u64,
            rflags: 0x202,
            rsp: loaded.user_rsp,
            ss: gdt::USER_DS as u64,
        });
    }

    let tid = alloc_tid();
    let pid = fixed_pid_override.unwrap_or(tid);
    let ppid = if pid == 1 { 0 } else { current_pid() };
    let basename = raw_path.rsplit('/').next().unwrap_or(raw_path);

    let mut task = Task {
        tid,
        pid,
        ppid,
        name: [0; 32],
        state: TaskState::Ready,
        priority: PRIORITY_NORMAL,
        quantum_ticks: DEFAULT_QUANTUM_TICKS,
        remaining_ticks: DEFAULT_QUANTUM_TICKS,
        cpu_ticks: 0,
        context_switches: 0,
        is_user: true,
        address_space: loaded.address_space,
        kernel_stack_top: kstack_top,
        kernel_rsp: frame_addr,
        user_brk_base: loaded.brk_base,
        user_brk_current: loaded.brk_base,
        user_mmap_next: 0x0000_4000_0000_0000,
        cwd: String::from("/"),
        creds: loaded.new_creds,
        fds: default_stdio_fds(),
        pending_signals: 0,
    };
    task.set_name(basename);

    SCHED.lock().tasks.push(task);
    Ok(pid)
}
