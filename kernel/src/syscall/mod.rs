//! AetherOS Native System Call Dispatcher and Full Handler Suite (ABI v1).
//!
//! Conventions:
//! - `RAX`: Syscall number
//! - `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`: Arguments 1..6
//! - Return in `RAX`: `>= 0` on success, `-Errno` in `[-4095, -1]` on failure.
//! - Every user pointer is validated via `mm::user_access` before dereferencing.
//! - Unsupported/planned syscalls return `-ENOSYS` (never fake success).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

use aether_abi::caps::CAP_SETUID;
use aether_abi::clock_id::{CLOCK_MONOTONIC, CLOCK_REALTIME};
use aether_abi::mmap_flags::{
    MAP_ANONYMOUS, MAP_FIXED, MAP_SHARED, PROT_EXEC, PROT_READ, PROT_WRITE,
};
use aether_abi::open_flags::{O_CLOEXEC, O_NONBLOCK, O_RDONLY, O_RDWR, O_WRONLY};
use aether_abi::syscall_nr::*;
use aether_abi::{
    Dirent64, Errno, PollFd, ProcInfo, SysInfo, SyscallStatus, Timespec, UtsName,
    ABI_VERSION_PACKED, USER_ADDR_MAX, USER_ADDR_MIN,
};
use aether_uuid::Uuid;

use crate::arch::x86_64::idt::TrapFrame;
use crate::arch::x86_64::paging::{PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE};
use crate::arch::x86_64::{cli, sti, timer};
use crate::fs::{self, AclEntry, FdResource, FileDescriptorEntry, OpenFile};
use crate::ipc::{self, Pipe, UnixSocket};
use crate::mm::user_access::{
    copy_from_user, copy_to_user, copy_user_cstr, read_user_struct, validate_user_range,
    write_user_struct,
};
use crate::mm::{align_up, heap, pmm, PAGE_SIZE};
use crate::sched;
use crate::security;

static TOTAL_SYSCALLS: AtomicU64 = AtomicU64::new(0);

pub fn total_syscalls() -> u64 {
    TOTAL_SYSCALLS.load(Ordering::Relaxed)
}

/// Query the explicit implementation status of any syscall number in the AetherOS ABI v1 table.
pub fn syscall_status(nr: u64) -> SyscallStatus {
    match nr {
        SYS_ABI_VERSION | SYS_READ | SYS_WRITE | SYS_OPEN | SYS_CLOSE | SYS_STAT | SYS_FSTAT
        | SYS_LSTAT | SYS_LSEEK | SYS_PREAD64 | SYS_PWRITE64 | SYS_MKDIR | SYS_RMDIR
        | SYS_UNLINK | SYS_RENAME | SYS_LINK | SYS_SYMLINK | SYS_READLINK | SYS_CHMOD
        | SYS_CHOWN | SYS_GETDENTS64 | SYS_MOUNT | SYS_UMOUNT2 | SYS_GETCWD | SYS_CHDIR
        | SYS_DUP | SYS_DUP2 | SYS_FCNTL | SYS_SYNC | SYS_SET_EXECUTE_AS | SYS_GET_EXECUTE_AS
        | SYS_SET_ACL | SYS_GET_ACL | SYS_MMAP | SYS_MUNMAP | SYS_MPROTECT | SYS_BRK | SYS_EXIT
        | SYS_FORK | SYS_EXECVE | SYS_WAIT4 | SYS_GETPID | SYS_GETPPID | SYS_KILL | SYS_UNAME
        | SYS_DMESG | SYS_SYSINFO | SYS_PROC_LIST | SYS_PIPE | SYS_PIPE2 | SYS_POLL
        | SYS_SOCKET | SYS_BIND | SYS_LISTEN | SYS_ACCEPT | SYS_CONNECT | SYS_SENDTO
        | SYS_RECVFROM | SYS_SHM_OPEN | SYS_SHM_UNLINK | SYS_CLOCK_GETTIME | SYS_NANOSLEEP
        | SYS_SCHED_YIELD | SYS_SCHED_SETPARAM | SYS_SCHED_GETPARAM | SYS_GETUID | SYS_GETGID
        | SYS_SETUID | SYS_SETGID | SYS_GETRESUID | SYS_SETRESUID | SYS_GETRESGID
        | SYS_SETRESGID | SYS_GETGROUPS | SYS_SETGROUPS | SYS_CAPGET | SYS_CAPSET
        | SYS_GET_USER_UUID | SYS_GET_GROUP_UUID | SYS_NO_NEW_PRIVS | SYS_IDENTITY_CTL
        | SYS_AUDIT_READ | SYS_ELEVATE_SUDO => SyscallStatus::Implemented,

        SYS_STATFS | SYS_REBOOT | SYS_SENDMSG | SYS_RECVMSG | SYS_SIGACTION | SYS_SIGRETURN => {
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
    // (`wait4`, `nanosleep`, `read`, `poll`, `accept`) can sleep and be woken by hardware interrupts.
    sti();

    // `SYS_EXECVE` replaces `frame` directly on success!
    if nr == SYS_EXECVE {
        match sys_execve(frame, a1, a2, a3) {
            Ok(()) => {
                cli();
                return frame as *mut TrapFrame;
            }
            Err(errno) => {
                cli();
                frame.rax = errno.as_neg_u64();
                return frame as *mut TrapFrame;
            }
        }
    }

    let result: Result<u64, Errno> = match nr {
        SYS_ABI_VERSION => Ok(ABI_VERSION_PACKED),
        SYS_READ => sys_read(a1 as u32, a2, a3 as usize, None),
        SYS_WRITE => sys_write(a1 as u32, a2, a3 as usize, None),
        SYS_PREAD64 => sys_read(a1 as u32, a2, a3 as usize, Some(a4)),
        SYS_PWRITE64 => sys_write(a1 as u32, a2, a3 as usize, Some(a4)),
        SYS_OPEN => sys_open(a1, a2 as u32, a3 as u32),
        SYS_CLOSE => sched::with_current_task_mut(|t| t.close_fd(a1 as u32)).map(|()| 0),
        SYS_STAT => sys_stat(a1, a2, true),
        SYS_LSTAT => sys_stat(a1, a2, false),
        SYS_FSTAT => sys_fstat(a1 as u32, a2),
        SYS_LSEEK => sys_lseek(a1 as u32, a2 as i64, a3 as u32),
        SYS_MKDIR => sys_mkdir(a1, a2 as u32),
        SYS_RMDIR => sys_rmdir(a1),
        SYS_UNLINK => sys_unlink(a1),
        SYS_RENAME => sys_rename(a1, a2),
        SYS_LINK => sys_link(a1, a2),
        SYS_SYMLINK => sys_symlink(a1, a2),
        SYS_READLINK => sys_readlink(a1, a2, a3 as usize),
        SYS_CHMOD => sys_chmod(a1, a2 as u32),
        SYS_CHOWN => sys_chown(a1, a2 as u32, a3 as u32),
        SYS_GETDENTS64 => sys_getdents64(a1 as u32, a2, a3 as usize),
        SYS_MOUNT => sys_mount(a1, a2, a3, a4 as u32),
        SYS_UMOUNT2 => sys_umount2(a1),
        SYS_GETCWD => sys_getcwd(a1, a2 as usize),
        SYS_CHDIR => sys_chdir(a1),
        SYS_DUP => sys_dup(a1 as u32),
        SYS_DUP2 => sys_dup2(a1 as u32, a2 as u32),
        SYS_FCNTL => Ok(0),
        SYS_SYNC => Ok(0),
        SYS_SET_EXECUTE_AS => sys_set_execute_as(a1, a2),
        SYS_GET_EXECUTE_AS => sys_get_execute_as(a1, a2),
        SYS_SET_ACL => sys_set_acl(a1, a2, a3 as usize),
        SYS_GET_ACL => sys_get_acl(a1, a2, a3 as usize),
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
        SYS_PIPE => sys_pipe2(a1, 0),
        SYS_PIPE2 => sys_pipe2(a1, a2 as u32),
        SYS_POLL => sys_poll(a1, a2 as usize, a3 as i64),
        SYS_SOCKET => sys_socket(a1 as u32, a2 as u32),
        SYS_BIND => sys_bind(a1 as u32, a2),
        SYS_LISTEN => sys_listen(a1 as u32, a2 as usize),
        SYS_ACCEPT => sys_accept(a1 as u32),
        SYS_CONNECT => sys_connect(a1 as u32, a2),
        SYS_SENDTO => sys_write(a1 as u32, a2, a3 as usize, None),
        SYS_RECVFROM => sys_read(a1 as u32, a2, a3 as usize, None),
        SYS_SHM_OPEN => sys_shm_open(a1, a2 as u32, a3 as u32),
        SYS_SHM_UNLINK => sys_shm_unlink(a1),
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
            let uuid = security::uid_to_uuid(uid);
            sched::sys_setuid_current(uid, uuid).map(|()| 0)
        }
        SYS_SETGID => {
            let gid = a1 as u32;
            let uuid = security::gid_to_uuid(gid);
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
        SYS_IDENTITY_CTL => sys_identity_ctl(a1 as u32, a2 as u32, a3, a4),
        SYS_AUDIT_READ => sys_audit_read(a1, a2 as usize),
        SYS_ELEVATE_SUDO => sys_elevate_sudo(a1),
        _ => Err(Errno::Enosys),
    };

    cli();

    frame.rax = match result {
        Ok(val) => val,
        Err(errno) => errno.as_neg_u64(),
    };

    frame as *mut TrapFrame
}

fn sys_open(path_ptr: u64, flags: u32, mode: u32) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let file = fs::vfs_open(&path, flags, mode)?;
    let cloexec = (flags & O_CLOEXEC) != 0;
    let fd = sched::with_current_task_mut(|t| t.alloc_fd(file, cloexec))?;
    Ok(fd as u64)
}

fn sys_read(
    fd: u32,
    user_buf: u64,
    len: usize,
    explicit_offset: Option<u64>,
) -> Result<u64, Errno> {
    let count = len.min(64 * 1024);
    validate_user_range(user_buf, count, true)?;
    if count == 0 {
        return Ok(0);
    }
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    let mut kbuf = vec![0u8; count];
    let n = fs::vfs_read_file(&file, &mut kbuf, explicit_offset)?;
    copy_to_user(user_buf, &kbuf[..n])?;
    Ok(n as u64)
}

fn sys_write(
    fd: u32,
    user_buf: u64,
    len: usize,
    explicit_offset: Option<u64>,
) -> Result<u64, Errno> {
    let count = len.min(64 * 1024);
    validate_user_range(user_buf, count, false)?;
    if count == 0 {
        return Ok(0);
    }
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    let mut kbuf = vec![0u8; count];
    copy_from_user(&mut kbuf, user_buf)?;
    let n = fs::vfs_write_file(&file, &kbuf, explicit_offset)?;
    Ok(n as u64)
}

fn sys_stat(path_ptr: u64, stat_out_ptr: u64, follow_symlinks: bool) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let st = fs::vfs_stat_path(&path, follow_symlinks)?;
    write_user_struct(stat_out_ptr, &st)?;
    Ok(0)
}

fn sys_fstat(fd: u32, stat_out_ptr: u64) -> Result<u64, Errno> {
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    let st = fs::vfs_fstat(&file)?;
    write_user_struct(stat_out_ptr, &st)?;
    Ok(0)
}

fn sys_lseek(fd: u32, offset: i64, whence: u32) -> Result<u64, Errno> {
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    fs::vfs_lseek(&file, offset, whence)
}

fn sys_mkdir(path_ptr: u64, mode: u32) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_mkdir(&path, mode)?;
    Ok(0)
}

fn sys_rmdir(path_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_rmdir(&path)?;
    Ok(0)
}

fn sys_unlink(path_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_unlink(&path)?;
    Ok(0)
}

fn sys_rename(old_ptr: u64, new_ptr: u64) -> Result<u64, Errno> {
    let old_path = copy_user_cstr(old_ptr, 512)?;
    let new_path = copy_user_cstr(new_ptr, 512)?;
    fs::vfs_rename(&old_path, &new_path)?;
    Ok(0)
}

fn sys_link(old_ptr: u64, new_ptr: u64) -> Result<u64, Errno> {
    let old_path = copy_user_cstr(old_ptr, 512)?;
    let new_path = copy_user_cstr(new_ptr, 512)?;
    fs::vfs_link(&old_path, &new_path)?;
    Ok(0)
}

fn sys_symlink(target_ptr: u64, linkpath_ptr: u64) -> Result<u64, Errno> {
    let target = copy_user_cstr(target_ptr, 512)?;
    let linkpath = copy_user_cstr(linkpath_ptr, 512)?;
    fs::vfs_symlink(&target, &linkpath)?;
    Ok(0)
}

fn sys_readlink(path_ptr: u64, user_buf: u64, max_len: usize) -> Result<u64, Errno> {
    if max_len == 0 {
        return Err(Errno::Einval);
    }
    let path = copy_user_cstr(path_ptr, 512)?;
    let target = fs::vfs_readlink(&path)?;
    let bytes = target.as_bytes();
    let copy_len = bytes.len().min(max_len);
    copy_to_user(user_buf, &bytes[..copy_len])?;
    Ok(copy_len as u64)
}

fn sys_chmod(path_ptr: u64, mode: u32) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_chmod(&path, mode)?;
    Ok(0)
}

fn sys_chown(path_ptr: u64, uid: u32, gid: u32) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_chown(&path, uid, gid)?;
    Ok(0)
}

fn sys_getdents64(fd: u32, user_buf: u64, byte_len: usize) -> Result<u64, Errno> {
    let entry_size = size_of::<Dirent64>();
    let max_entries = (byte_len / entry_size).min(64);
    if max_entries == 0 {
        return Err(Errno::Einval);
    }
    validate_user_range(user_buf, max_entries * entry_size, true)?;
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    let mut entries = vec![
        Dirent64 {
            d_ino: 0,
            d_reclen: 0,
            d_type: 0,
            d_namlen: 0,
            _pad: 0,
            d_name: [0; 240],
        };
        max_entries
    ];
    let count = fs::vfs_getdents64(&file, &mut entries)?;
    for (i, item) in entries.iter().enumerate().take(count) {
        write_user_struct(user_buf + (i * entry_size) as u64, item)?;
    }
    Ok((count * entry_size) as u64)
}

fn sys_mount(src_ptr: u64, target_ptr: u64, fstype_ptr: u64, flags: u32) -> Result<u64, Errno> {
    let src = copy_user_cstr(src_ptr, 256)?;
    let target = copy_user_cstr(target_ptr, 256)?;
    let fstype = copy_user_cstr(fstype_ptr, 64)?;
    fs::vfs_mount(&src, &target, &fstype, flags)?;
    Ok(0)
}

fn sys_umount2(target_ptr: u64) -> Result<u64, Errno> {
    let target = copy_user_cstr(target_ptr, 256)?;
    fs::vfs_umount(&target)?;
    Ok(0)
}

fn sys_getcwd(user_buf: u64, size: usize) -> Result<u64, Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let b = cwd.as_bytes();
    if b.len() + 1 > size {
        return Err(Errno::Erange);
    }
    validate_user_range(user_buf, b.len() + 1, true)?;
    copy_to_user(user_buf, b)?;
    copy_to_user(user_buf + (b.len() as u64), &[0u8])?;
    Ok(user_buf)
}

fn sys_chdir(path_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    fs::vfs_chdir(&path)?;
    Ok(0)
}

fn sys_dup(oldfd: u32) -> Result<u64, Errno> {
    let file = sched::with_current_task(|t| t.get_fd(oldfd))?;
    let newfd = sched::with_current_task_mut(|t| t.alloc_fd(file, false))?;
    Ok(newfd as u64)
}

fn sys_dup2(oldfd: u32, newfd: u32) -> Result<u64, Errno> {
    if (newfd as usize) >= sched::MAX_FDS_PER_PROCESS {
        return Err(Errno::Ebadf);
    }
    let file = sched::with_current_task(|t| t.get_fd(oldfd))?;
    if oldfd == newfd {
        return Ok(newfd as u64);
    }
    sched::with_current_task_mut(|t| {
        while t.fds.len() <= (newfd as usize) {
            t.fds.push(None);
        }
        t.fds[newfd as usize] = Some(FileDescriptorEntry {
            file,
            cloexec: false,
        });
    });
    Ok(newfd as u64)
}

fn sys_set_execute_as(path_ptr: u64, uuid_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let mut uuid_bytes = [0u8; 16];
    copy_from_user(&mut uuid_bytes, uuid_ptr)?;
    fs::vfs_set_execute_as(&path, Uuid::from_bytes(uuid_bytes))?;
    Ok(0)
}

fn sys_get_execute_as(path_ptr: u64, uuid_out_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let uuid = fs::vfs_get_execute_as(&path)?;
    copy_to_user(uuid_out_ptr, uuid.as_bytes())?;
    Ok(0)
}

fn sys_set_acl(path_ptr: u64, acl_ptr: u64, count: usize) -> Result<u64, Errno> {
    if count > 32 {
        return Err(Errno::E2big);
    }
    let path = copy_user_cstr(path_ptr, 512)?;
    validate_user_range(acl_ptr, count * size_of::<AclEntry>(), false)?;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let ace: AclEntry = read_user_struct(acl_ptr + (i * size_of::<AclEntry>()) as u64)?;
        entries.push(ace);
    }
    fs::vfs_set_acl(&path, entries)?;
    Ok(0)
}

fn sys_get_acl(path_ptr: u64, acl_out_ptr: u64, max_count: usize) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let entries = fs::vfs_get_acl(&path)?;
    if max_count == 0 {
        return Ok(entries.len() as u64);
    }
    let n = entries.len().min(max_count);
    validate_user_range(acl_out_ptr, n * size_of::<AclEntry>(), true)?;
    for (i, ace) in entries.iter().enumerate().take(n) {
        write_user_struct(acl_out_ptr + (i * size_of::<AclEntry>()) as u64, ace)?;
    }
    Ok(n as u64)
}

fn sys_pipe2(fds_user_ptr: u64, flags: u32) -> Result<u64, Errno> {
    validate_user_range(fds_user_ptr, 8, true)?;
    let (reader, writer) = Pipe::new_pair();
    let nonblock = flags & O_NONBLOCK;
    let cloexec = (flags & O_CLOEXEC) != 0;
    let r_file = OpenFile::new(FdResource::PipeRead(reader), O_RDONLY | nonblock);
    let w_file = OpenFile::new(FdResource::PipeWrite(writer), O_WRONLY | nonblock);

    let (r_fd, w_fd) = sched::with_current_task_mut(|t| -> Result<(u32, u32), Errno> {
        let r = t.alloc_fd(r_file, cloexec)?;
        match t.alloc_fd(w_file, cloexec) {
            Ok(w) => Ok((r, w)),
            Err(e) => {
                let _ = t.close_fd(r);
                Err(e)
            }
        }
    })?;

    let pair: [i32; 2] = [r_fd as i32, w_fd as i32];
    write_user_struct(fds_user_ptr, &pair)?;
    Ok(0)
}

fn sys_poll(fds_ptr: u64, nfds: usize, timeout_ms: i64) -> Result<u64, Errno> {
    if nfds > 64 {
        return Err(Errno::Einval);
    }
    validate_user_range(fds_ptr, nfds * size_of::<PollFd>(), true)?;
    let start_ms = timer::uptime_ms();

    loop {
        let mut ready_count = 0u64;
        for i in 0..nfds {
            let entry_addr = fds_ptr + (i * size_of::<PollFd>()) as u64;
            let mut pfd: PollFd = read_user_struct(entry_addr)?;
            pfd.revents = 0;
            if pfd.fd >= 0 {
                match sched::with_current_task(|t| t.get_fd(pfd.fd as u32)) {
                    Ok(file) => {
                        let avail = fs::vfs_poll_file(&file);
                        let matched = avail
                            & (pfd.events
                                | aether_abi::poll_events::POLLHUP
                                | aether_abi::poll_events::POLLERR);
                        if matched != 0 {
                            pfd.revents = matched;
                            ready_count += 1;
                        }
                    }
                    Err(_) => {
                        pfd.revents = aether_abi::poll_events::POLLNVAL;
                        ready_count += 1;
                    }
                }
            }
            write_user_struct(entry_addr, &pfd)?;
        }

        if ready_count > 0 || timeout_ms == 0 {
            return Ok(ready_count);
        }
        if timeout_ms > 0 && (timer::uptime_ms().saturating_sub(start_ms) as i64) >= timeout_ms {
            return Ok(0);
        }
        sched::sleep_ms(1);
    }
}

fn sys_socket(domain: u32, sock_type: u32) -> Result<u64, Errno> {
    if domain != ipc::AF_UNIX {
        return Err(Errno::Eafnosupport);
    }
    let clean_type = sock_type & 0xF;
    let nonblock = if (sock_type & O_NONBLOCK) != 0 {
        O_NONBLOCK
    } else {
        0
    };
    let cloexec = (sock_type & O_CLOEXEC) != 0;
    let sock = UnixSocket::new(clean_type)?;
    let file = OpenFile::new(FdResource::UnixSocket(sock), O_RDWR | nonblock);
    let fd = sched::with_current_task_mut(|t| t.alloc_fd(file, cloexec))?;
    Ok(fd as u64)
}

fn get_unix_socket(fd: u32) -> Result<(alloc::sync::Arc<UnixSocket>, bool), Errno> {
    let file = sched::with_current_task(|t| t.get_fd(fd))?;
    match &file.resource {
        FdResource::UnixSocket(s) => Ok((alloc::sync::Arc::clone(s), file.is_nonblocking())),
        _ => Err(Errno::Einval),
    }
}

fn sys_bind(fd: u32, path_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 108)?;
    let (sock, _) = get_unix_socket(fd)?;
    sock.bind(path)?;
    Ok(0)
}

fn sys_listen(fd: u32, backlog: usize) -> Result<u64, Errno> {
    let (sock, _) = get_unix_socket(fd)?;
    sock.listen(backlog)?;
    Ok(0)
}

fn sys_accept(fd: u32) -> Result<u64, Errno> {
    let (sock, nonblock) = get_unix_socket(fd)?;
    let accepted = sock.accept(nonblock)?;
    let file = OpenFile::new(FdResource::UnixSocket(accepted), O_RDWR);
    let new_fd = sched::with_current_task_mut(|t| t.alloc_fd(file, false))?;
    Ok(new_fd as u64)
}

fn sys_connect(fd: u32, path_ptr: u64) -> Result<u64, Errno> {
    let path = copy_user_cstr(path_ptr, 108)?;
    let (sock, _) = get_unix_socket(fd)?;
    sock.connect(&path)?;
    Ok(0)
}

fn sys_shm_open(name_ptr: u64, flags: u32, mode: u32) -> Result<u64, Errno> {
    let name = copy_user_cstr(name_ptr, 64)?;
    let shm = ipc::shm_open_object(&name, flags, mode)?;
    let file = OpenFile::new(FdResource::Shm(shm), flags);
    let fd = sched::with_current_task_mut(|t| t.alloc_fd(file, (flags & O_CLOEXEC) != 0))?;
    Ok(fd as u64)
}

fn sys_shm_unlink(name_ptr: u64) -> Result<u64, Errno> {
    let name = copy_user_cstr(name_ptr, 64)?;
    ipc::shm_unlink_object(&name)?;
    Ok(0)
}

fn sys_execve(
    frame: &mut TrapFrame,
    path_ptr: u64,
    argv_ptr: u64,
    envp_ptr: u64,
) -> Result<(), Errno> {
    let path = copy_user_cstr(path_ptr, 512)?;
    let mut argv = Vec::new();
    if argv_ptr != 0 {
        for i in 0..64usize {
            let elem_ptr: u64 = read_user_struct(argv_ptr + (i * 8) as u64)?;
            if elem_ptr == 0 {
                break;
            }
            argv.push(copy_user_cstr(elem_ptr, 512)?);
        }
    }
    if argv.is_empty() {
        argv.push(path.clone());
    }

    let mut envp = Vec::new();
    if envp_ptr != 0 {
        for i in 0..32usize {
            let elem_ptr: u64 = read_user_struct(envp_ptr + (i * 8) as u64)?;
            if elem_ptr == 0 {
                break;
            }
            envp.push(copy_user_cstr(elem_ptr, 512)?);
        }
    }

    sched::exec::execve_current_from_frame(frame, &path, &argv, &envp)
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
    fd: i64,
    _offset: u64,
) -> Result<u64, Errno> {
    if len == 0 {
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

    // Shared memory object mapping (`MAP_SHARED` with a valid `ShmObject` fd)
    if (flags & MAP_SHARED) != 0 && (flags & MAP_ANONYMOUS) == 0 && fd >= 0 {
        let file = sched::with_current_task(|t| t.get_fd(fd as u32))?;
        if let FdResource::Shm(shm) = &file.resource {
            let shm_frames = shm.ensure_pages(pages)?;
            for (i, &phys) in shm_frames.iter().enumerate() {
                let v = base_vaddr + (i as u64) * (PAGE_SIZE as u64);
                pmm::inc_frame_ref(phys);
                aspace
                    .map_page(v, phys, pte_flags | PTE_PRESENT)
                    .map_err(|_| Errno::Enomem)?;
            }
            return Ok(base_vaddr);
        }
        return Err(Errno::Einval);
    }

    if (flags & MAP_ANONYMOUS) == 0 {
        return Err(Errno::Einval);
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
    let max_brk = base + 64 * 1024 * 1024;
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
    let nonblocking = (options & 1) != 0;
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
    let byte_len = count * size_of::<ProcInfo>();
    validate_user_range(user_ptr, byte_len, true)?;
    let mut list = vec![ProcInfo::default(); count];
    let actual = sched::list_processes(&mut list);
    for (i, item) in list.iter().enumerate().take(actual) {
        let dst = user_ptr + (i * size_of::<ProcInfo>()) as u64;
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
            t.creds.user_uuid = security::uid_to_uuid(euid);
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
            t.creds.group_uuid = security::gid_to_uuid(egid);
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

fn sys_identity_ctl(op: u32, uid: u32, gid_or_arg: u64, name_ptr: u64) -> Result<u64, Errno> {
    match op {
        // op 1: useradd(uid, gid, username_cstr)
        1 => {
            let username = copy_user_cstr(name_ptr, 32)?;
            let _uuid = security::add_user(uid, gid_or_arg as u32, &username)?;
            Ok(0)
        }
        _ => Err(Errno::Einval),
    }
}

fn sys_audit_read(user_buf: u64, max_len: usize) -> Result<u64, Errno> {
    let log = security::read_audit_for_current()?;
    let bytes = log.as_bytes();
    let n = bytes.len().min(max_len);
    validate_user_range(user_buf, n, true)?;
    copy_to_user(user_buf, &bytes[..n])?;
    Ok(n as u64)
}

fn sys_elevate_sudo(reason_ptr: u64) -> Result<u64, Errno> {
    let reason = if reason_ptr != 0 {
        copy_user_cstr(reason_ptr, 128).unwrap_or_else(|_| String::from("sudo"))
    } else {
        String::from("sudo")
    };
    security::sys_elevate_sudo_current(&reason)?;
    Ok(0)
}
