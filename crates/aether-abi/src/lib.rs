#![cfg_attr(not(feature = "std"), no_std)]

//! AetherOS Native System Call ABI (v1).
//!
//! Register convention (`x86_64` `SYSCALL`/`SYSRET`):
//! - Syscall number: `RAX`
//! - Arguments 1..6: `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`
//! - Return value:   `RAX` (`>= 0` on success, `-Errno` in `[-4095, -1]` on error)
//! - Clobbered:      `RCX` (user RIP), `R11` (user RFLAGS)
//!
//! AetherOS does NOT claim Linux binary compatibility for its native ABI.

/// ABI major version (incremented on breaking binary layout changes).
pub const ABI_VERSION_MAJOR: u16 = 1;
/// ABI minor version (incremented on backward-compatible syscall additions).
pub const ABI_VERSION_MINOR: u16 = 0;
/// Packed ABI version returned by `SYS_ABI_VERSION`: `(major << 16) | minor`.
pub const ABI_VERSION_PACKED: u64 = ((ABI_VERSION_MAJOR as u64) << 16) | (ABI_VERSION_MINOR as u64);

/// Upper exclusive limit for canonical Ring 3 user virtual addresses.
/// Any user pointer `>= USER_ADDR_MAX` must be rejected with `EFAULT`.
pub const USER_ADDR_MAX: usize = 0x0000_8000_0000_0000;

/// Lower inclusive limit for valid Ring 3 user virtual addresses (null guard region is 64 KiB).
pub const USER_ADDR_MIN: usize = 0x0000_0000_0001_0000;

/// Standard AetherOS error numbers (POSIX-inspired numbering, native ABI).
#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Errno {
    Eperm = 1,
    Enoent = 2,
    Esrch = 3,
    Eintr = 4,
    Eio = 5,
    Enxio = 6,
    E2big = 7,
    Enoexec = 8,
    Ebadf = 9,
    Echild = 10,
    Eagain = 11,
    Enomem = 12,
    Eacces = 13,
    Efault = 14,
    Ebusy = 16,
    Eexist = 17,
    Exdev = 18,
    Enodev = 19,
    Enotdir = 20,
    Eisdir = 21,
    Einval = 22,
    Enfile = 23,
    Emfile = 24,
    Enotty = 25,
    Efbig = 27,
    Enospc = 28,
    Espipe = 29,
    Erofs = 30,
    Emlink = 31,
    Epipe = 32,
    Erange = 34,
    Enametoolong = 36,
    Enosys = 38,
    Enotempty = 39,
    Eloop = 40,
    Enotconn = 107,
    Econnrefused = 111,
    Ealready = 114,
    Einprogress = 115,
    Eafnosupport = 97,
    Eaddrinuse = 98,
    Eprototype = 91,
    Eopnotsupp = 95,
    Etimedout = 110,
}

impl Errno {
    /// Convert an error code into the negative `isize`/`u64` syscall return encoding.
    #[inline]
    pub const fn as_neg_i64(self) -> i64 {
        -(self as i64)
    }

    #[inline]
    pub const fn as_neg_u64(self) -> u64 {
        self.as_neg_i64() as u64
    }

    /// Decode a raw syscall return value into `Ok(val)` or `Err(errno_code)`.
    #[inline]
    pub const fn decode_raw(raw: i64) -> Result<u64, i64> {
        if raw < 0 && raw >= -4095 {
            Err(-raw)
        } else {
            Ok(raw as u64)
        }
    }
}

/// Implementation status tracked explicitly in the kernel syscall table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallStatus {
    Implemented,
    PartiallyImplemented,
    NotImplemented,
    Planned,
}

/// Native AetherOS syscall numbers.
pub mod syscall_nr {
    // --- Meta / ABI ---
    pub const SYS_ABI_VERSION: u64 = 0;

    // --- Filesystem & I/O (1..49) ---
    pub const SYS_READ: u64 = 1;
    pub const SYS_WRITE: u64 = 2;
    pub const SYS_OPEN: u64 = 3;
    pub const SYS_CLOSE: u64 = 4;
    pub const SYS_STAT: u64 = 5;
    pub const SYS_FSTAT: u64 = 6;
    pub const SYS_LSTAT: u64 = 7;
    pub const SYS_LSEEK: u64 = 8;
    pub const SYS_PREAD64: u64 = 9;
    pub const SYS_PWRITE64: u64 = 10;
    pub const SYS_MKDIR: u64 = 11;
    pub const SYS_RMDIR: u64 = 12;
    pub const SYS_UNLINK: u64 = 13;
    pub const SYS_RENAME: u64 = 14;
    pub const SYS_LINK: u64 = 15;
    pub const SYS_SYMLINK: u64 = 16;
    pub const SYS_READLINK: u64 = 17;
    pub const SYS_CHMOD: u64 = 18;
    pub const SYS_CHOWN: u64 = 19;
    pub const SYS_GETDENTS64: u64 = 20;
    pub const SYS_MOUNT: u64 = 21;
    pub const SYS_UMOUNT2: u64 = 22;
    pub const SYS_GETCWD: u64 = 23;
    pub const SYS_CHDIR: u64 = 24;
    pub const SYS_DUP: u64 = 25;
    pub const SYS_DUP2: u64 = 26;
    pub const SYS_FCNTL: u64 = 27;
    pub const SYS_SYNC: u64 = 28;
    pub const SYS_STATFS: u64 = 29;
    pub const SYS_SET_EXECUTE_AS: u64 = 30;
    pub const SYS_GET_EXECUTE_AS: u64 = 31;
    pub const SYS_SET_ACL: u64 = 32;
    pub const SYS_GET_ACL: u64 = 33;

    // --- Memory (50..69) ---
    pub const SYS_MMAP: u64 = 50;
    pub const SYS_MUNMAP: u64 = 51;
    pub const SYS_MPROTECT: u64 = 52;
    pub const SYS_BRK: u64 = 53;

    // --- Process & Thread Lifecycle (70..99) ---
    pub const SYS_EXIT: u64 = 70;
    pub const SYS_FORK: u64 = 71;
    pub const SYS_EXECVE: u64 = 72;
    pub const SYS_WAIT4: u64 = 73;
    pub const SYS_GETPID: u64 = 74;
    pub const SYS_GETPPID: u64 = 75;
    pub const SYS_KILL: u64 = 76;
    pub const SYS_UNAME: u64 = 77;
    pub const SYS_DMESG: u64 = 78;
    pub const SYS_SYSINFO: u64 = 79;
    pub const SYS_PROC_LIST: u64 = 80;
    pub const SYS_REBOOT: u64 = 81;

    // --- IPC & Events (100..129) ---
    pub const SYS_PIPE: u64 = 100;
    pub const SYS_PIPE2: u64 = 101;
    pub const SYS_POLL: u64 = 102;
    pub const SYS_SOCKET: u64 = 103;
    pub const SYS_BIND: u64 = 104;
    pub const SYS_LISTEN: u64 = 105;
    pub const SYS_ACCEPT: u64 = 106;
    pub const SYS_CONNECT: u64 = 107;
    pub const SYS_SENDTO: u64 = 108;
    pub const SYS_RECVFROM: u64 = 109;
    pub const SYS_SENDMSG: u64 = 110;
    pub const SYS_RECVMSG: u64 = 111;
    pub const SYS_SHM_OPEN: u64 = 112;
    pub const SYS_SHM_UNLINK: u64 = 113;
    pub const SYS_SIGACTION: u64 = 114;
    pub const SYS_SIGRETURN: u64 = 115;

    // --- Time & Scheduling (130..149) ---
    pub const SYS_CLOCK_GETTIME: u64 = 130;
    pub const SYS_NANOSLEEP: u64 = 131;
    pub const SYS_SCHED_YIELD: u64 = 132;
    pub const SYS_SCHED_SETPARAM: u64 = 133;
    pub const SYS_SCHED_GETPARAM: u64 = 134;

    // --- Identity, Multi-User & Security (150..189) ---
    pub const SYS_GETUID: u64 = 150;
    pub const SYS_GETGID: u64 = 151;
    pub const SYS_SETUID: u64 = 152;
    pub const SYS_SETGID: u64 = 153;
    pub const SYS_GETRESUID: u64 = 154;
    pub const SYS_SETRESUID: u64 = 155;
    pub const SYS_GETRESGID: u64 = 156;
    pub const SYS_SETRESGID: u64 = 157;
    pub const SYS_GETGROUPS: u64 = 158;
    pub const SYS_SETGROUPS: u64 = 159;
    pub const SYS_CAPGET: u64 = 160;
    pub const SYS_CAPSET: u64 = 161;
    pub const SYS_GET_USER_UUID: u64 = 162;
    pub const SYS_GET_GROUP_UUID: u64 = 163;
    pub const SYS_NO_NEW_PRIVS: u64 = 164;
    pub const SYS_IDENTITY_CTL: u64 = 165;
    pub const SYS_AUDIT_READ: u64 = 166;
    pub const SYS_ELEVATE_SUDO: u64 = 167;

    pub const SYSCALL_MAX: usize = 192;
}

/// File open flags.
pub mod open_flags {
    pub const O_RDONLY: u32 = 0;
    pub const O_WRONLY: u32 = 1;
    pub const O_RDWR: u32 = 2;
    pub const O_ACCMODE: u32 = 3;
    pub const O_CREAT: u32 = 1 << 6;
    pub const O_EXCL: u32 = 1 << 7;
    pub const O_TRUNC: u32 = 1 << 9;
    pub const O_APPEND: u32 = 1 << 10;
    pub const O_NONBLOCK: u32 = 1 << 11;
    pub const O_DIRECTORY: u32 = 1 << 16;
    pub const O_CLOEXEC: u32 = 1 << 19;
}

/// `lseek` whence values.
pub mod seek {
    pub const SEEK_SET: u32 = 0;
    pub const SEEK_CUR: u32 = 1;
    pub const SEEK_END: u32 = 2;
}

/// `mmap` protection & mapping flags.
pub mod mmap_flags {
    pub const PROT_NONE: u32 = 0;
    pub const PROT_READ: u32 = 1;
    pub const PROT_WRITE: u32 = 2;
    pub const PROT_EXEC: u32 = 4;

    pub const MAP_SHARED: u32 = 0x01;
    pub const MAP_PRIVATE: u32 = 0x02;
    pub const MAP_FIXED: u32 = 0x10;
    pub const MAP_ANONYMOUS: u32 = 0x20;
}

/// Signal numbers.
pub mod signal {
    pub const SIGHUP: u32 = 1;
    pub const SIGINT: u32 = 2;
    pub const SIGQUIT: u32 = 3;
    pub const SIGILL: u32 = 4;
    pub const SIGTRAP: u32 = 5;
    pub const SIGABRT: u32 = 6;
    pub const SIGBUS: u32 = 7;
    pub const SIGFPE: u32 = 8;
    pub const SIGKILL: u32 = 9;
    pub const SIGUSR1: u32 = 10;
    pub const SIGSEGV: u32 = 11;
    pub const SIGUSR2: u32 = 12;
    pub const SIGPIPE: u32 = 13;
    pub const SIGALRM: u32 = 14;
    pub const SIGTERM: u32 = 15;
    pub const SIGCHLD: u32 = 17;
    pub const SIGCONT: u32 = 18;
    pub const SIGSTOP: u32 = 19;
}

/// `stat` structure returned by `sys_stat` / `sys_fstat` / `sys_lstat`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct AetherStat {
    pub inode_id: u64,
    pub object_uuid: [u8; 16],
    pub owner_user_uuid: [u8; 16],
    pub owner_group_uuid: [u8; 16],
    pub execute_as_uuid: [u8; 16],
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub flags: u32,
    pub nlink: u32,
    pub _pad: u32,
    pub size: u64,
    pub blocks: u64,
    pub blksize: u32,
    pub dev: u32,
    pub ctime_ns: u64,
    pub mtime_ns: u64,
    pub atime_ns: u64,
}

/// Clock IDs for `sys_clock_gettime`.
pub mod clock_id {
    pub const CLOCK_REALTIME: u32 = 0;
    pub const CLOCK_MONOTONIC: u32 = 1;
}

/// Timespec passed to/from `sys_clock_gettime` and `sys_nanosleep`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// Fixed-size `utsname` returned by `sys_uname`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UtsName {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
}

impl Default for UtsName {
    fn default() -> Self {
        Self {
            sysname: [0; 65],
            nodename: [0; 65],
            release: [0; 65],
            version: [0; 65],
            machine: [0; 65],
        }
    }
}

/// System memory & uptime summary returned by `sys_sysinfo`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SysInfo {
    pub uptime_ms: u64,
    pub total_ram_bytes: u64,
    pub free_ram_bytes: u64,
    pub kernel_heap_used: u64,
    pub kernel_heap_total: u64,
    pub process_count: u32,
    pub cpu_count: u32,
}

/// Process summary entry returned by `sys_proc_list`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub state: u32,
    pub priority: u32,
    pub user_uuid: [u8; 16],
    pub cpu_ticks: u64,
    pub name: [u8; 32],
}

/// Poll file descriptor entry.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

pub mod poll_events {
    pub const POLLIN: i16 = 0x0001;
    pub const POLLOUT: i16 = 0x0004;
    pub const POLLERR: i16 = 0x0008;
    pub const POLLHUP: i16 = 0x0010;
    pub const POLLNVAL: i16 = 0x0020;
}

/// Directory entry header returned by `sys_getdents64`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Dirent64 {
    pub d_ino: u64,
    pub d_reclen: u16,
    pub d_type: u8,
    pub d_namlen: u8,
    pub _pad: u32,
    pub d_name: [u8; 240],
}

/// Capability bitmask constants.
pub mod caps {
    pub const CAP_CHOWN: u64 = 1 << 0;
    pub const CAP_DAC_OVERRIDE: u64 = 1 << 1;
    pub const CAP_FOWNER: u64 = 1 << 2;
    pub const CAP_KILL: u64 = 1 << 3;
    pub const CAP_SETUID: u64 = 1 << 4;
    pub const CAP_SETGID: u64 = 1 << 5;
    pub const CAP_SET_EXECUTE_AS: u64 = 1 << 6;
    pub const CAP_MOUNT: u64 = 1 << 7;
    pub const CAP_SYS_ADMIN: u64 = 1 << 8;
    pub const CAP_NET_ADMIN: u64 = 1 << 9;
    pub const CAP_NET_BIND_SERVICE: u64 = 1 << 10;
    pub const CAP_AUDIT_READ: u64 = 1 << 11;
    pub const CAP_SYS_BOOT: u64 = 1 << 12;
    pub const CAP_ALL: u64 = (1 << 13) - 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_roundtrip() {
        let raw = Errno::Enosys.as_neg_i64();
        assert_eq!(raw, -38);
        assert_eq!(Errno::decode_raw(raw), Err(38));
        assert_eq!(Errno::decode_raw(0), Ok(0));
        assert_eq!(Errno::decode_raw(42), Ok(42));
    }

    #[test]
    fn abi_version_packed_layout() {
        assert_eq!(ABI_VERSION_PACKED >> 16, ABI_VERSION_MAJOR as u64);
        assert_eq!(ABI_VERSION_PACKED & 0xFFFF, ABI_VERSION_MINOR as u64);
    }
}
