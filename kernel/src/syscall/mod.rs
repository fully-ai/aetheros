//! AetherOS Native System Call Dispatcher and Core Handlers (ABI v1).
//!
//! Conventions:
//! - `RAX`: Syscall number
//! - `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`: Arguments 1..6
//! - Return in `RAX`: `>= 0` on success, `-Errno` in `[-4095, -1]` on failure.
//! - Every user pointer is validated via `mm::user_access` before dereferencing.
//! - Unsupported/planned syscalls return `-ENOSYS` (never fake success).

use alloc::vec;
use core::sync::atomic::{AtomicU64, Ordering};

use aether_abi::caps::CAP_SETUID;
use aether_abi::clock_id::{CLOCK_MONOTONIC, CLOCK_REALTIME};
use aether_abi::mmap_flags::{MAP_ANONYMOUS, MAP_FIXED, PROT_EXEC, PROT_READ, PROT_WRITE};
use aether_abi::syscall_nr::*;
use aether_abi::{
    Errno, ProcInfo, SysInfo, SyscallStatus, Timespec, UtsName, ABI_VERSION_PACKED, USER_ADDR_MAX,
    USER_ADDR_MIN,
};
use aether_uuid::Uuid;

use crate::arch::x86_64::idt::TrapFrame;
use crate::arch::x86_64::paging::{PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE};
use crate::arch::x86_64::{cli, serial, sti, timer};
use crate::mm::user_access::{
    copy_from_user, copy_to_user, read_user_struct, validate_user_range, write_user_struct,
};
use crate::mm::{align_up, heap, pmm, PAGE_SIZE};
use crate::sched;

static TOTAL_SYSCALLS: AtomicU64 = AtomicU64::new(0);

pub fn total_syscalls() -> u64 {
    TOTAL_SYSCALLS.load(Ordering::Relaxed)
}

/// Query the explicit implementation status of any syscall number in the AetherOS ABI v1 table.
pub fn syscall_status(nr: u64) -> SyscallStatus {
    match nr {
        SYS_ABI_VERSION | SYS_READ | SYS_WRITE | SYS_MMAP | SYS_MUNMAP | SYS_MPROTECT | SYS_BRK
        | SYS_EXIT | SYS_FORK | SYS_WAIT4 | SYS_GETPID | SYS_GETPPID | SYS_KILL | SYS_UNAME
        | SYS_DMESG | SYS_SYSINFO | SYS_PROC_LIST | SYS_CLOCK_GETTIME | SYS_NANOSLEEP
        | SYS_SCHED_YIELD | SYS_SCHED_SETPARAM | SYS_SCHED_GETPARAM | SYS_GETUID | SYS_GETGID
        | SYS_SETUID | SYS_SETGID | SYS_GETRESUID | SYS_SETRESUID | SYS_GETRESGID
        | SYS_SETRESGID | SYS_GETGROUPS | SYS_SETGROUPS | SYS_CAPGET | SYS_CAPSET
        | SYS_GET_USER_UUID | SYS_GET_GROUP_UUID | SYS_NO_NEW_PRIVS => SyscallStatus::Implemented,

        SYS_OPEN | SYS_CLOSE | SYS_STAT | SYS_FSTAT | SYS_LSTAT | SYS_LSEEK | SYS_PREAD64
        | SYS_PWRITE64 | SYS_MKDIR | SYS_RMDIR | SYS_UNLINK | SYS_RENAME | SYS_LINK
        | SYS_SYMLINK | SYS_READLINK | SYS_CHMOD | SYS_CHOWN | SYS_GETDENTS64 | SYS_MOUNT
        | SYS_UMOUNT2 | SYS_GETCWD | SYS_CHDIR | SYS_DUP | SYS_DUP2 | SYS_PIPE | SYS_PIPE2
        | SYS_POLL | SYS_SOCKET | SYS_BIND | SYS_LISTEN | SYS_ACCEPT | SYS_CONNECT | SYS_SENDTO
        | SYS_RECVFROM | SYS_SHM_OPEN | SYS_EXECVE | SYS_SET_EXECUTE_AS | SYS_GET_EXECUTE_AS
        | SYS_SET_ACL | SYS_GET_ACL | SYS_IDENTITY_CTL | SYS_AUDIT_READ | SYS_ELEVATE_SUDO => {
            SyscallStatus::Planned
        }

        _ => SyscallStatus::NotImplemented,
    }
}

/// Top-level syscall entry dispatcher invoked from `syscall_entry_stub`.
pub fn dispatch(frame: &mut TrapFrame) -> *mut TrapFrame {
    TOTAL_SYSCALLS.fetch_add(1, Ordering::Relaxed);

    let nr = frame.rax;
    let a1 = frame.rdi;
    let a2 = frame.rsi;
    let a3 = frame.rdx;
    let a4 = frame.r10;
    let a5 = frame.r8;
    let a6 = frame.r9;

    // `SYS_EXIT` terminates the calling task and directly returns the next scheduled task's TrapFrame.
    if nr == SYS_EXIT {
        let code = (a1 as i32) & 0xFF;
        return sched::exit_current_from_frame(frame, code);
    }

    // Re-enable maskable interrupts during normal syscall execution so blocking operations
    // (`wait4`, `nanosleep`, `read`, `poll`) can sleep and be woken by hardware interrupts.
    sti();

    let result: Result<u64, Errno> = match nr {
        SYS_ABI_VERSION => Ok(ABI_VERSION_PACKED),
        SYS_READ => sys_read(a1, a2, a3),
        SYS_WRITE => sys_write(a1, a2, a3),
        SYS_MMAP => sys_mmap(a1, a2, a3 as u32, a4 as u32, a5 as i64, a6),
        SYS_MUNMAP => sys_munmap(a1, a2),
        SYS_MPROTECT => sys_mprotect(a1, a2, a3 as u32),
        SYS_BRK => sys_brk(a1),
        SYS_FORK => sched::fork_current_from_frame(frame).map(|pid| pid as u64),
        SYS_WAIT4 => sys_wait4(a1 as i32, a2, a3 as u32),
        SYS_GETPID => Ok(sched::current_pid() as u64),
        SYS_GETPPID => Ok(sched::with_current_task(|t| t.ppid as u64)),
        SYS_KILL => sched::kill_process(a1 as u32, a2 as u32).map(|()| 0),
        SYS_UNAME => sys_uname(a1),
        SYS_DMESG => sys_dmesg(a1, a2 as usize),
        SYS_SYSINFO => sys_sysinfo(a1),
        SYS_PROC_LIST => sys_proc_list(a1, a2 as usize),
        SYS_CLOCK_GETTIME => sys_clock_gettime(a1 as u32, a2),
        SYS_NANOSLEEP => sys_nanosleep(a1),
        SYS_SCHED_YIELD => {
            sched::yield_now();
            Ok(0)
        }
        SYS_SCHED_SETPARAM => {
            let tid = if a1 == 0 {
                sched::current_tid()
            } else {
                a1 as u32
            };
            if sched::set_priority(tid, a2 as u8) {
                Ok(0)
            } else {
                Err(Errno::Esrch)
            }
        }
        SYS_SCHED_GETPARAM => {
            let tid = if a1 == 0 {
                sched::current_tid()
            } else {
                a1 as u32
            };
            sched::get_priority(tid)
                .map(|p| p as u64)
                .ok_or(Errno::Esrch)
        }
        SYS_GETUID => Ok(sched::with_current_task(|t| t.creds.euid as u64)),
        SYS_GETGID => Ok(sched::with_current_task(|t| t.creds.egid as u64)),
        SYS_SETUID => {
            let uid = a1 as u32;
            let uuid = if uid == 0 {
                Uuid::ROOT_USER
            } else {
                Uuid::deterministic(1, uid as u64)
            };
            sched::sys_setuid_current(uid, uuid).map(|()| 0)
        }
        SYS_SETGID => {
            let gid = a1 as u32;
            let uuid = if gid == 0 {
                Uuid::ROOT_GROUP
            } else {
                Uuid::deterministic(2, gid as u64)
            };
            sched::sys_setgid_current(gid, uuid).map(|()| 0)
        }
        SYS_GETRESUID => sys_getresuid(a1, a2, a3),
        SYS_SETRESUID => sys_setresuid(a1 as u32, a2 as u32, a3 as u32),
        SYS_GETRESGID => sys_getresgid(a1, a2, a3),
        SYS_SETRESGID => sys_setresgid(a1 as u32, a2 as u32, a3 as u32),
        SYS_GETGROUPS => sys_getgroups(a1 as usize, a2),
        SYS_SETGROUPS => sys_setgroups(a1 as usize, a2),
        SYS_CAPGET => Ok(sched::with_current_task(|t| t.creds.caps_effective)),
        SYS_CAPSET => sys_capset(a1),
        SYS_GET_USER_UUID => sys_get_user_uuid(a1),
        SYS_GET_GROUP_UUID => sys_get_group_uuid(a1),
        SYS_NO_NEW_PRIVS => {
            sched::with_current_task_mut(|t| t.creds.no_new_privs = true);
            Ok(0)
        }
        _ => Err(Errno::Enosys),
    };

    // Mask interrupts before returning the TrapFrame pointer to assembly exit stub
    cli();

    frame.rax = match result {
        Ok(val) => val,
        Err(errno) => errno.as_neg_u64(),
    };

    frame as *mut TrapFrame
}

fn sys_read(fd: u64, user_buf: u64, len: u64) -> Result<u64, Errno> {
    let count = (len as usize).min(64 * 1024);
    validate_user_range(user_buf, count, true)?;
    if count == 0 {
        return Ok(0);
    }
    if fd == 0 {
        if let Some(byte) = serial::read_byte_nonblocking() {
            copy_to_user(user_buf, &[byte])?;
            return Ok(1);
        }
        return Ok(0);
    }
    Err(Errno::Ebadf)
}

fn sys_write(fd: u64, user_buf: u64, len: u64) -> Result<u64, Errno> {
    let count = (len as usize).min(64 * 1024);
    validate_user_range(user_buf, count, false)?;
    if count == 0 {
        return Ok(0);
    }
    if fd == 1 || fd == 2 {
        let mut kbuf = vec![0u8; count];
        copy_from_user(&mut kbuf, user_buf)?;
        if let Ok(s) = core::str::from_utf8(&kbuf) {
            serial::write_str(s);
        } else {
            for &b in &kbuf {
                serial::COM1.lock().write_byte(b);
            }
        }
        return Ok(count as u64);
    }
    Err(Errno::Ebadf)
}

fn prot_to_pte_flags(prot: u32) -> Result<u64, Errno> {
    if (prot & PROT_WRITE) != 0 && (prot & PROT_EXEC) != 0 {
        return Err(Errno::Einval); // W^X violation rejected!
    }
    let mut flags = PTE_USER;
    if (prot & PROT_WRITE) != 0 {
        flags |= PTE_WRITABLE;
    }
    if (prot & PROT_EXEC) == 0 {
        flags |= PTE_NO_EXECUTE;
    }
    Ok(flags)
}

fn sys_mmap(
    addr_hint: u64,
    len: u64,
    prot: u32,
    flags: u32,
    _fd: i64,
    _offset: u64,
) -> Result<u64, Errno> {
    if len == 0 || (flags & MAP_ANONYMOUS) == 0 {
        return Err(Errno::Einval);
    }
    let aligned_len = align_up(len, PAGE_SIZE as u64);
    let pages = (aligned_len / (PAGE_SIZE as u64)) as usize;
    let pte_flags = prot_to_pte_flags(prot)?;

    let (aspace, base_vaddr) = sched::with_current_task_mut(|t| {
        let vaddr = if (flags & MAP_FIXED) != 0 && addr_hint != 0 {
            addr_hint & !(PAGE_SIZE as u64 - 1)
        } else {
            let next = t.user_mmap_next;
            t.user_mmap_next = next.saturating_add(aligned_len + (PAGE_SIZE as u64));
            next
        };
        (t.address_space, vaddr)
    });

    if (base_vaddr as usize) < USER_ADDR_MIN
        || (base_vaddr.saturating_add(aligned_len) as usize) > USER_ADDR_MAX
    {
        return Err(Errno::Enomem);
    }

    for i in 0..pages {
        let v = base_vaddr + (i as u64) * (PAGE_SIZE as u64);
        if (prot & (PROT_READ | PROT_WRITE | PROT_EXEC)) == 0 {
            aspace.map_guard_page(v).map_err(|_| Errno::Enomem)?;
        } else {
            aspace
                .map_demand_page(v, pte_flags)
                .map_err(|_| Errno::Enomem)?;
        }
    }

    Ok(base_vaddr)
}

fn sys_munmap(addr: u64, len: u64) -> Result<u64, Errno> {
    if (addr & (PAGE_SIZE as u64 - 1)) != 0 || len == 0 {
        return Err(Errno::Einval);
    }
    let aligned_len = align_up(len, PAGE_SIZE as u64);
    if (addr as usize) < USER_ADDR_MIN
        || (addr.saturating_add(aligned_len) as usize) > USER_ADDR_MAX
    {
        return Err(Errno::Einval);
    }
    let pages = (aligned_len / (PAGE_SIZE as u64)) as usize;
    let aspace = sched::with_current_task(|t| t.address_space);
    for i in 0..pages {
        let v = addr + (i as u64) * (PAGE_SIZE as u64);
        let _ = aspace.unmap_and_free_page(v);
    }
    Ok(0)
}

fn sys_mprotect(addr: u64, len: u64, prot: u32) -> Result<u64, Errno> {
    if (addr & (PAGE_SIZE as u64 - 1)) != 0 || len == 0 {
        return Err(Errno::Einval);
    }
    let aligned_len = align_up(len, PAGE_SIZE as u64);
    if (addr as usize) < USER_ADDR_MIN
        || (addr.saturating_add(aligned_len) as usize) > USER_ADDR_MAX
    {
        return Err(Errno::Einval);
    }
    let pte_flags = prot_to_pte_flags(prot)? | PTE_PRESENT;
    let pages = (aligned_len / (PAGE_SIZE as u64)) as usize;
    let aspace = sched::with_current_task(|t| t.address_space);
    for i in 0..pages {
        let v = addr + (i as u64) * (PAGE_SIZE as u64);
        // Ensure any lazy demand page is materialized before changing permissions
        validate_user_range(v, 1, false)?;
        aspace
            .protect_page(v, pte_flags)
            .map_err(|_| Errno::Eacces)?;
    }
    Ok(0)
}

fn sys_brk(new_brk: u64) -> Result<u64, Errno> {
    let (aspace, base, cur) =
        sched::with_current_task(|t| (t.address_space, t.user_brk_base, t.user_brk_current));
    if new_brk == 0 || new_brk < base {
        return Ok(cur);
    }
    let max_brk = base + 64 * 1024 * 1024; // 64 MiB per-process brk limit
    if new_brk > max_brk || (new_brk as usize) >= USER_ADDR_MAX {
        return Ok(cur);
    }

    let old_page_end = align_up(cur, PAGE_SIZE as u64);
    let new_page_end = align_up(new_brk, PAGE_SIZE as u64);

    if new_page_end > old_page_end {
        let mut p = old_page_end;
        while p < new_page_end {
            aspace
                .map_demand_page(p, PTE_WRITABLE | PTE_USER | PTE_NO_EXECUTE)
                .map_err(|_| Errno::Enomem)?;
            p += PAGE_SIZE as u64;
        }
    } else if new_page_end < old_page_end {
        let mut p = new_page_end;
        while p < old_page_end {
            let _ = aspace.unmap_and_free_page(p);
            p += PAGE_SIZE as u64;
        }
    }

    sched::with_current_task_mut(|t| t.user_brk_current = new_brk);
    Ok(new_brk)
}

fn sys_wait4(target_pid: i32, status_user_ptr: u64, options: u32) -> Result<u64, Errno> {
    if status_user_ptr != 0 {
        validate_user_range(status_user_ptr, 4, true)?;
    }
    let nonblocking = (options & 1) != 0; // WNOHANG = 1
    let (reaped_pid, exit_code) = sched::wait4_child(target_pid, nonblocking)?;
    if reaped_pid != 0 && status_user_ptr != 0 {
        let encoded_status: i32 = (exit_code & 0xFF) << 8;
        write_user_struct(status_user_ptr, &encoded_status)?;
    }
    Ok(reaped_pid as u64)
}

fn copy_str_into_fixed<const N: usize>(dst: &mut [u8; N], s: &str) {
    *dst = [0u8; N];
    let bytes = s.as_bytes();
    let len = bytes.len().min(N - 1);
    dst[..len].copy_from_slice(&bytes[..len]);
}

fn sys_uname(user_ptr: u64) -> Result<u64, Errno> {
    let mut uts = UtsName::default();
    copy_str_into_fixed(&mut uts.sysname, "AetherOS");
    copy_str_into_fixed(&mut uts.nodename, "aether");
    copy_str_into_fixed(&mut uts.release, "0.1.0-alpha.1");
    copy_str_into_fixed(&mut uts.version, "AetherOS x86_64 Modular Kernel");
    copy_str_into_fixed(&mut uts.machine, "x86_64");
    write_user_struct(user_ptr, &uts)?;
    Ok(0)
}

fn sys_dmesg(user_buf: u64, max_len: usize) -> Result<u64, Errno> {
    let len = max_len.min(64 * 1024);
    validate_user_range(user_buf, len, true)?;
    let mut kbuf = vec![0u8; len];
    let copied = crate::log::read_dmesg(&mut kbuf);
    copy_to_user(user_buf, &kbuf[..copied])?;
    Ok(copied as u64)
}

fn sys_sysinfo(user_ptr: u64) -> Result<u64, Errno> {
    let pmm_st = pmm::stats();
    let heap_st = heap::stats();
    let sched_st = sched::stats();
    let info = SysInfo {
        uptime_ms: timer::uptime_ms(),
        total_ram_bytes: pmm_st.usable_frames * (PAGE_SIZE as u64),
        free_ram_bytes: pmm_st.free_frames * (PAGE_SIZE as u64),
        kernel_heap_used: heap_st.used_bytes as u64,
        kernel_heap_total: heap_st.mapped_bytes as u64,
        process_count: sched_st.total_tasks as u32,
        cpu_count: 1,
    };
    write_user_struct(user_ptr, &info)?;
    Ok(0)
}

fn sys_proc_list(user_ptr: u64, max_entries: usize) -> Result<u64, Errno> {
    let count = max_entries.min(128);
    let byte_len = count * core::mem::size_of::<ProcInfo>();
    validate_user_range(user_ptr, byte_len, true)?;
    let mut list = vec![ProcInfo::default(); count];
    let actual = sched::list_processes(&mut list);
    for (i, item) in list.iter().enumerate().take(actual) {
        let dst = user_ptr + (i * core::mem::size_of::<ProcInfo>()) as u64;
        write_user_struct(dst, item)?;
    }
    Ok(actual as u64)
}

fn sys_clock_gettime(clock: u32, user_ts_ptr: u64) -> Result<u64, Errno> {
    if clock != CLOCK_MONOTONIC && clock != CLOCK_REALTIME {
        return Err(Errno::Einval);
    }
    let ns = timer::uptime_ns();
    let ts = Timespec {
        tv_sec: (ns / 1_000_000_000) as i64,
        tv_nsec: (ns % 1_000_000_000) as i64,
    };
    write_user_struct(user_ts_ptr, &ts)?;
    Ok(0)
}

fn sys_nanosleep(user_req_ptr: u64) -> Result<u64, Errno> {
    let req: Timespec = read_user_struct(user_req_ptr)?;
    if req.tv_sec < 0 || req.tv_nsec < 0 || req.tv_nsec >= 1_000_000_000 {
        return Err(Errno::Einval);
    }
    let ms = (req.tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add((req.tv_nsec as u64).div_ceil(1_000_000));
    sched::sleep_ms(ms);
    Ok(0)
}

fn sys_getresuid(ruid_ptr: u64, euid_ptr: u64, suid_ptr: u64) -> Result<u64, Errno> {
    let (r, e, s) = sched::with_current_task(|t| (t.creds.uid, t.creds.euid, t.creds.suid));
    if ruid_ptr != 0 {
        write_user_struct(ruid_ptr, &r)?;
    }
    if euid_ptr != 0 {
        write_user_struct(euid_ptr, &e)?;
    }
    if suid_ptr != 0 {
        write_user_struct(suid_ptr, &s)?;
    }
    Ok(0)
}

fn sys_setresuid(ruid: u32, euid: u32, suid: u32) -> Result<u64, Errno> {
    sched::with_current_task_mut(|t| {
        let is_root = t.creds.euid == 0 || (t.creds.caps_effective & CAP_SETUID) != 0;
        let allowed = |id: u32| {
            id == u32::MAX
                || is_root
                || id == t.creds.uid
                || id == t.creds.euid
                || id == t.creds.suid
        };
        if !allowed(ruid) || !allowed(euid) || !allowed(suid) {
            return Err(Errno::Eperm);
        }
        if ruid != u32::MAX {
            t.creds.uid = ruid;
        }
        if euid != u32::MAX {
            t.creds.euid = euid;
            t.creds.user_uuid = if euid == 0 {
                Uuid::ROOT_USER
            } else {
                Uuid::deterministic(1, euid as u64)
            };
        }
        if suid != u32::MAX {
            t.creds.suid = suid;
        }
        if t.creds.euid != 0 {
            t.creds.caps_effective = 0;
        }
        Ok(0)
    })
}

fn sys_getresgid(rgid_ptr: u64, egid_ptr: u64, sgid_ptr: u64) -> Result<u64, Errno> {
    let (r, e, s) = sched::with_current_task(|t| (t.creds.gid, t.creds.egid, t.creds.sgid));
    if rgid_ptr != 0 {
        write_user_struct(rgid_ptr, &r)?;
    }
    if egid_ptr != 0 {
        write_user_struct(egid_ptr, &e)?;
    }
    if sgid_ptr != 0 {
        write_user_struct(sgid_ptr, &s)?;
    }
    Ok(0)
}

fn sys_setresgid(rgid: u32, egid: u32, sgid: u32) -> Result<u64, Errno> {
    sched::with_current_task_mut(|t| {
        let is_root =
            t.creds.euid == 0 || (t.creds.caps_effective & aether_abi::caps::CAP_SETGID) != 0;
        let allowed = |id: u32| {
            id == u32::MAX
                || is_root
                || id == t.creds.gid
                || id == t.creds.egid
                || id == t.creds.sgid
        };
        if !allowed(rgid) || !allowed(egid) || !allowed(sgid) {
            return Err(Errno::Eperm);
        }
        if rgid != u32::MAX {
            t.creds.gid = rgid;
        }
        if egid != u32::MAX {
            t.creds.egid = egid;
            t.creds.group_uuid = if egid == 0 {
                Uuid::ROOT_GROUP
            } else {
                Uuid::deterministic(2, egid as u64)
            };
        }
        if sgid != u32::MAX {
            t.creds.sgid = sgid;
        }
        Ok(0)
    })
}

fn sys_getgroups(max_count: usize, user_list_ptr: u64) -> Result<u64, Errno> {
    let groups = sched::with_current_task(|t| t.creds.groups.clone());
    if max_count == 0 {
        return Ok(groups.len() as u64);
    }
    if max_count < groups.len() {
        return Err(Errno::Einval);
    }
    validate_user_range(user_list_ptr, groups.len() * 4, true)?;
    for (i, &g) in groups.iter().enumerate() {
        write_user_struct(user_list_ptr + (i * 4) as u64, &g)?;
    }
    Ok(groups.len() as u64)
}

fn sys_setgroups(count: usize, user_list_ptr: u64) -> Result<u64, Errno> {
    if count > 32 {
        return Err(Errno::Einval);
    }
    let is_root = sched::with_current_task(|t| {
        t.creds.euid == 0 || (t.creds.caps_effective & aether_abi::caps::CAP_SETGID) != 0
    });
    if !is_root {
        return Err(Errno::Eperm);
    }
    validate_user_range(user_list_ptr, count * 4, false)?;
    let mut new_groups = vec![0u32; count];
    for (i, slot) in new_groups.iter_mut().enumerate() {
        *slot = read_user_struct(user_list_ptr + (i * 4) as u64)?;
    }
    sched::with_current_task_mut(|t| t.creds.groups = new_groups);
    Ok(0)
}

fn sys_capset(new_caps: u64) -> Result<u64, Errno> {
    sched::with_current_task_mut(|t| {
        // A process may only enable capabilities present in its permitted & bounding sets
        if (new_caps & !t.creds.caps_permitted) != 0 || (new_caps & !t.creds.caps_bounding) != 0 {
            return Err(Errno::Eperm);
        }
        t.creds.caps_effective = new_caps;
        t.creds.caps_permitted &= new_caps;
        Ok(0)
    })
}

fn sys_get_user_uuid(user_out_ptr: u64) -> Result<u64, Errno> {
    let bytes = sched::with_current_task(|t| *t.creds.user_uuid.as_bytes());
    copy_to_user(user_out_ptr, &bytes)?;
    Ok(0)
}

fn sys_get_group_uuid(user_out_ptr: u64) -> Result<u64, Errno> {
    let bytes = sched::with_current_task(|t| *t.creds.group_uuid.as_bytes());
    copy_to_user(user_out_ptr, &bytes)?;
    Ok(0)
}
