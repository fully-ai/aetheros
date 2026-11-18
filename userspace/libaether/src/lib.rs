#![no_std]
#![allow(clippy::missing_safety_doc)]

//! `libaether` — Native AetherOS User-Space Runtime and Syscall Wrapper Library.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::asm;
use core::fmt;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};

pub use aether_abi::clock_id::*;
pub use aether_abi::mmap_flags::*;
pub use aether_abi::open_flags::*;
pub use aether_abi::poll_events::*;
pub use aether_abi::seek::*;
pub use aether_abi::signal::*;
pub use aether_abi::syscall_nr::*;
pub use aether_abi::{
    AetherStat, Dirent64, Errno, PollFd, ProcInfo, SysInfo, Timespec, UtsName, ABI_VERSION_PACKED,
};
pub use aether_uuid::Uuid;

// --- Raw x86_64 SYSCALL Primitives ---

#[inline(always)]
pub unsafe fn syscall0(nr: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall1(nr: u64, a1: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a1,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a1,
            in("rsi") a2,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall6(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") nr as i64 => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            in("r9") a6,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret
}

#[inline]
fn check_ret(raw: i64) -> Result<u64, i64> {
    Errno::decode_raw(raw)
}

// --- Typed Syscall Wrappers ---

pub fn exit(code: i32) -> ! {
    unsafe {
        let _ = syscall1(SYS_EXIT, code as u64);
    }
    loop {
        core::hint::spin_loop();
    }
}

pub fn abi_version() -> u64 {
    unsafe { syscall0(SYS_ABI_VERSION) as u64 }
}

pub fn read(fd: u32, buf: &mut [u8]) -> Result<usize, i64> {
    let r = unsafe {
        syscall3(
            SYS_READ,
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    check_ret(r).map(|n| n as usize)
}

pub fn write(fd: u32, buf: &[u8]) -> Result<usize, i64> {
    let r = unsafe { syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64) };
    check_ret(r).map(|n| n as usize)
}

pub fn open(path: &str, flags: u32, mode: u32) -> Result<u32, i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall3(SYS_OPEN, cpath.as_ptr() as u64, flags as u64, mode as u64) };
    check_ret(r).map(|fd| fd as u32)
}

pub fn close(fd: u32) -> Result<(), i64> {
    let r = unsafe { syscall1(SYS_CLOSE, fd as u64) };
    check_ret(r).map(|_| ())
}

pub fn stat(path: &str) -> Result<AetherStat, i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let mut st = AetherStat::default();
    let r = unsafe {
        syscall2(
            SYS_STAT,
            cpath.as_ptr() as u64,
            (&mut st as *mut AetherStat) as u64,
        )
    };
    check_ret(r).map(|_| st)
}

pub fn fstat(fd: u32) -> Result<AetherStat, i64> {
    let mut st = AetherStat::default();
    let r = unsafe { syscall2(SYS_FSTAT, fd as u64, (&mut st as *mut AetherStat) as u64) };
    check_ret(r).map(|_| st)
}

pub fn mkdir(path: &str, mode: u32) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall2(SYS_MKDIR, cpath.as_ptr() as u64, mode as u64) };
    check_ret(r).map(|_| ())
}

pub fn rmdir(path: &str) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall1(SYS_RMDIR, cpath.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn unlink(path: &str) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall1(SYS_UNLINK, cpath.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn rename(old_path: &str, new_path: &str) -> Result<(), i64> {
    let mut cold = Vec::with_capacity(old_path.len() + 1);
    cold.extend_from_slice(old_path.as_bytes());
    cold.push(0);
    let mut cnew = Vec::with_capacity(new_path.len() + 1);
    cnew.extend_from_slice(new_path.as_bytes());
    cnew.push(0);
    let r = unsafe { syscall2(SYS_RENAME, cold.as_ptr() as u64, cnew.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn chmod(path: &str, mode: u32) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall2(SYS_CHMOD, cpath.as_ptr() as u64, mode as u64) };
    check_ret(r).map(|_| ())
}

pub fn chown(path: &str, uid: u32, gid: u32) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall3(SYS_CHOWN, cpath.as_ptr() as u64, uid as u64, gid as u64) };
    check_ret(r).map(|_| ())
}

pub fn getdents64(fd: u32, out: &mut [Dirent64]) -> Result<usize, i64> {
    let byte_len = core::mem::size_of_val(out);
    let r = unsafe {
        syscall3(
            SYS_GETDENTS64,
            fd as u64,
            out.as_mut_ptr() as u64,
            byte_len as u64,
        )
    };
    check_ret(r).map(|bytes| (bytes as usize) / core::mem::size_of::<Dirent64>())
}

pub fn mount(source: &str, target: &str, fstype: &str, flags: u32) -> Result<(), i64> {
    let mut csrc = Vec::with_capacity(source.len() + 1);
    csrc.extend_from_slice(source.as_bytes());
    csrc.push(0);
    let mut ctgt = Vec::with_capacity(target.len() + 1);
    ctgt.extend_from_slice(target.as_bytes());
    ctgt.push(0);
    let mut cfs = Vec::with_capacity(fstype.len() + 1);
    cfs.extend_from_slice(fstype.as_bytes());
    cfs.push(0);
    let r = unsafe {
        syscall4(
            SYS_MOUNT,
            csrc.as_ptr() as u64,
            ctgt.as_ptr() as u64,
            cfs.as_ptr() as u64,
            flags as u64,
        )
    };
    check_ret(r).map(|_| ())
}

pub fn umount(target: &str) -> Result<(), i64> {
    let mut ctgt = Vec::with_capacity(target.len() + 1);
    ctgt.extend_from_slice(target.as_bytes());
    ctgt.push(0);
    let r = unsafe { syscall1(SYS_UMOUNT2, ctgt.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn getcwd() -> Result<String, i64> {
    let mut buf = [0u8; 256];
    let r = unsafe { syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64) };
    check_ret(r)?;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(String::from(
        core::str::from_utf8(&buf[..end]).unwrap_or("/"),
    ))
}

pub fn chdir(path: &str) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe { syscall1(SYS_CHDIR, cpath.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn dup(oldfd: u32) -> Result<u32, i64> {
    let r = unsafe { syscall1(SYS_DUP, oldfd as u64) };
    check_ret(r).map(|fd| fd as u32)
}

pub fn dup2(oldfd: u32, newfd: u32) -> Result<u32, i64> {
    let r = unsafe { syscall2(SYS_DUP2, oldfd as u64, newfd as u64) };
    check_ret(r).map(|fd| fd as u32)
}

pub fn pipe() -> Result<(u32, u32), i64> {
    let mut fds = [0i32; 2];
    let r = unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u64) };
    check_ret(r).map(|_| (fds[0] as u32, fds[1] as u32))
}

pub fn fork() -> Result<u32, i64> {
    let r = unsafe { syscall0(SYS_FORK) };
    check_ret(r).map(|pid| pid as u32)
}

pub fn execve(path: &str, args: &[&str], envs: &[&str]) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);

    let mut cargs_storage: Vec<Vec<u8>> = Vec::with_capacity(args.len());
    let mut cargs_ptrs: Vec<u64> = Vec::with_capacity(args.len() + 1);
    for &a in args {
        let mut v = Vec::with_capacity(a.len() + 1);
        v.extend_from_slice(a.as_bytes());
        v.push(0);
        cargs_ptrs.push(v.as_ptr() as u64);
        cargs_storage.push(v);
    }
    cargs_ptrs.push(0);

    let mut cenvs_storage: Vec<Vec<u8>> = Vec::with_capacity(envs.len());
    let mut cenvs_ptrs: Vec<u64> = Vec::with_capacity(envs.len() + 1);
    for &e in envs {
        let mut v = Vec::with_capacity(e.len() + 1);
        v.extend_from_slice(e.as_bytes());
        v.push(0);
        cenvs_ptrs.push(v.as_ptr() as u64);
        cenvs_storage.push(v);
    }
    cenvs_ptrs.push(0);

    let r = unsafe {
        syscall3(
            SYS_EXECVE,
            cpath.as_ptr() as u64,
            cargs_ptrs.as_ptr() as u64,
            cenvs_ptrs.as_ptr() as u64,
        )
    };
    check_ret(r).map(|_| ())
}

pub fn wait4(pid: i32) -> Result<(u32, i32), i64> {
    let mut status: i32 = 0;
    let r = unsafe {
        syscall3(
            SYS_WAIT4,
            pid as i64 as u64,
            (&mut status as *mut i32) as u64,
            0,
        )
    };
    check_ret(r).map(|reaped| (reaped as u32, (status >> 8) & 0xFF))
}

pub fn getpid() -> u32 {
    unsafe { syscall0(SYS_GETPID) as u32 }
}

pub fn getppid() -> u32 {
    unsafe { syscall0(SYS_GETPPID) as u32 }
}

pub fn kill(pid: u32, sig: u32) -> Result<(), i64> {
    let r = unsafe { syscall2(SYS_KILL, pid as u64, sig as u64) };
    check_ret(r).map(|_| ())
}

pub fn getuid() -> u32 {
    unsafe { syscall0(SYS_GETUID) as u32 }
}

pub fn getgid() -> u32 {
    unsafe { syscall0(SYS_GETGID) as u32 }
}

pub fn setuid(uid: u32) -> Result<(), i64> {
    let r = unsafe { syscall1(SYS_SETUID, uid as u64) };
    check_ret(r).map(|_| ())
}

pub fn setgid(gid: u32) -> Result<(), i64> {
    let r = unsafe { syscall1(SYS_SETGID, gid as u64) };
    check_ret(r).map(|_| ())
}

pub fn get_user_uuid() -> Uuid {
    let mut bytes = [0u8; 16];
    let _ = unsafe { syscall1(SYS_GET_USER_UUID, bytes.as_mut_ptr() as u64) };
    Uuid::from_bytes(bytes)
}

pub fn get_group_uuid() -> Uuid {
    let mut bytes = [0u8; 16];
    let _ = unsafe { syscall1(SYS_GET_GROUP_UUID, bytes.as_mut_ptr() as u64) };
    Uuid::from_bytes(bytes)
}

pub fn set_execute_as(path: &str, target_uuid: Uuid) -> Result<(), i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let r = unsafe {
        syscall2(
            SYS_SET_EXECUTE_AS,
            cpath.as_ptr() as u64,
            target_uuid.as_bytes().as_ptr() as u64,
        )
    };
    check_ret(r).map(|_| ())
}

pub fn get_execute_as(path: &str) -> Result<Uuid, i64> {
    let mut cpath = Vec::with_capacity(path.len() + 1);
    cpath.extend_from_slice(path.as_bytes());
    cpath.push(0);
    let mut bytes = [0u8; 16];
    let r = unsafe {
        syscall2(
            SYS_GET_EXECUTE_AS,
            cpath.as_ptr() as u64,
            bytes.as_mut_ptr() as u64,
        )
    };
    check_ret(r).map(|_| Uuid::from_bytes(bytes))
}

pub fn elevate_sudo(reason: &str) -> Result<(), i64> {
    let mut creason = Vec::with_capacity(reason.len() + 1);
    creason.extend_from_slice(reason.as_bytes());
    creason.push(0);
    let r = unsafe { syscall1(SYS_ELEVATE_SUDO, creason.as_ptr() as u64) };
    check_ret(r).map(|_| ())
}

pub fn useradd(uid: u32, gid: u32, username: &str) -> Result<(), i64> {
    let mut cname = Vec::with_capacity(username.len() + 1);
    cname.extend_from_slice(username.as_bytes());
    cname.push(0);
    let r = unsafe {
        syscall4(
            SYS_IDENTITY_CTL,
            1,
            uid as u64,
            gid as u64,
            cname.as_ptr() as u64,
        )
    };
    check_ret(r).map(|_| ())
}

pub fn uname() -> Result<UtsName, i64> {
    let mut uts = UtsName::default();
    let r = unsafe { syscall1(SYS_UNAME, (&mut uts as *mut UtsName) as u64) };
    check_ret(r).map(|_| uts)
}

pub fn sysinfo() -> Result<SysInfo, i64> {
    let mut info = SysInfo::default();
    let r = unsafe { syscall1(SYS_SYSINFO, (&mut info as *mut SysInfo) as u64) };
    check_ret(r).map(|_| info)
}

pub fn proc_list(out: &mut [ProcInfo]) -> Result<usize, i64> {
    let r = unsafe { syscall2(SYS_PROC_LIST, out.as_mut_ptr() as u64, out.len() as u64) };
    check_ret(r).map(|n| n as usize)
}

pub fn dmesg(out: &mut [u8]) -> Result<usize, i64> {
    let r = unsafe { syscall2(SYS_DMESG, out.as_mut_ptr() as u64, out.len() as u64) };
    check_ret(r).map(|n| n as usize)
}

pub fn sleep_ms(ms: u64) {
    let ts = Timespec {
        tv_sec: (ms / 1000) as i64,
        tv_nsec: ((ms % 1000) * 1_000_000) as i64,
    };
    let _ = unsafe { syscall1(SYS_NANOSLEEP, (&ts as *const Timespec) as u64) };
}

pub fn read_file_to_string(path: &str) -> Result<String, i64> {
    let fd = open(path, O_RDONLY, 0)?;
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = read(fd, &mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let _ = close(fd);
    Ok(String::from(core::str::from_utf8(&data).unwrap_or("")))
}

pub fn write_string_to_file(path: &str, content: &str, append: bool) -> Result<(), i64> {
    let flags = if append {
        O_CREAT | O_WRONLY | O_APPEND
    } else {
        O_CREAT | O_WRONLY | O_TRUNC
    };
    let fd = open(path, flags, 0o644)?;
    let _ = write(fd, content.as_bytes())?;
    let _ = close(fd);
    Ok(())
}

// --- Stdout / Stderr Formatting ---

pub struct FdWriter(pub u32);

impl fmt::Write for FdWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let _ = write(self.0, s.as_bytes());
        Ok(())
    }
}

pub fn print_fmt(fd: u32, args: fmt::Arguments<'_>) {
    let mut w = FdWriter(fd);
    let _ = fmt::Write::write_fmt(&mut w, args);
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::print_fmt(1, format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {
        $crate::print_fmt(1, format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! eprintln {
    ($($arg:tt)*) => {
        $crate::print_fmt(2, format_args!("{}\n", format_args!($($arg)*)))
    };
}

// --- User-Space Heap Allocator backed by `sys_brk` ---

struct UserBrkAllocator {
    cur: AtomicUsize,
    end: AtomicUsize,
}

unsafe impl GlobalAlloc for UserBrkAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align().max(16);
        let size = (layout.size().max(16) + 15) & !15;

        loop {
            let mut cur = self.cur.load(Ordering::Relaxed);
            let mut end = self.end.load(Ordering::Relaxed);
            if cur == 0 {
                let base = unsafe { syscall1(SYS_BRK, 0) as usize };
                let initial_end = base + 64 * 1024;
                let _ = unsafe { syscall1(SYS_BRK, initial_end as u64) };
                let _ = self
                    .cur
                    .compare_exchange(0, base, Ordering::SeqCst, Ordering::Relaxed);
                let _ =
                    self.end
                        .compare_exchange(0, initial_end, Ordering::SeqCst, Ordering::Relaxed);
                cur = self.cur.load(Ordering::Relaxed);
                end = self.end.load(Ordering::Relaxed);
            }

            let aligned = (cur + align - 1) & !(align - 1);
            let next = aligned + size;
            if next > end {
                let grow = (next - end).max(64 * 1024);
                let new_end = ((end + grow + 4095) & !4095) as u64;
                let actual = unsafe { syscall1(SYS_BRK, new_end) as usize };
                if actual < next {
                    return core::ptr::null_mut();
                }
                self.end.store(actual, Ordering::SeqCst);
                continue;
            }

            if self
                .cur
                .compare_exchange(cur, next, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                return aligned as *mut u8;
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static USER_ALLOC: UserBrkAllocator = UserBrkAllocator {
    cur: AtomicUsize::new(0),
    end: AtomicUsize::new(0),
};

/// Parse raw `(argc, argv)` from `_start` registers into a `Vec<&'static str>`.
pub unsafe fn parse_args(argc: usize, argv: *const *const u8) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(argc.min(64));
    if argv.is_null() {
        return out;
    }
    for i in 0..argc.min(64) {
        let p = unsafe { *argv.add(i) };
        if p.is_null() {
            break;
        }
        let mut len = 0usize;
        while len < 512 && unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        let slice = unsafe { core::slice::from_raw_parts(p, len) };
        if let Ok(s) = core::str::from_utf8(slice) {
            out.push(s);
        }
    }
    out
}

#[panic_handler]
fn user_panic(info: &PanicInfo<'_>) -> ! {
    eprintln!("[userspace panic] {}", info.message());
    exit(101)
}

/// Declare the ELF64 `_start` entry point for a user-space binary.
#[macro_export]
macro_rules! aether_main {
    ($main_fn:ident) => {
        #[no_mangle]
        #[link_section = ".text._start"]
        pub unsafe extern "C" fn _start(argc: usize, argv: *const *const u8) -> ! {
            let args = unsafe { $crate::parse_args(argc, argv) };
            let code = $main_fn(&args);
            $crate::exit(code)
        }
    };
}
