//! Virtual Filesystem Layer (VFS), Mount Table, `/dev`, `/proc`, `/tmp`, `/run`,
//! USTAR `initramfs.tar` Loader, and Unified File Descriptor Table (Milestone 6).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use aether_abi::caps::{CAP_CHOWN, CAP_DAC_OVERRIDE, CAP_FOWNER, CAP_MOUNT, CAP_SET_EXECUTE_AS};
use aether_abi::open_flags::*;
use aether_abi::poll_events::{POLLIN, POLLOUT};
use aether_abi::seek::{SEEK_CUR, SEEK_END, SEEK_SET};
use aether_abi::{AetherStat, Dirent64, Errno};
use aether_uuid::Uuid;

use crate::arch::x86_64::{cpu, serial, timer};
use crate::ipc::{PipeReader, PipeWriter, ShmObject, UnixSocket};
use crate::mm::{heap, pmm, PAGE_SIZE};
use crate::sched::{self, Credentials};
use crate::sync::IrqSpinLock;

// Inode mode type bits
pub const S_IFMT: u32 = 0o170000;
pub const S_IFIFO: u32 = 0o010000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;

pub const S_ISUID: u32 = 0o4000;
pub const S_ISGID: u32 = 0o2000;
pub const S_ISVTX: u32 = 0o1000; // Sticky bit

// Inode protection flags (Section 8.2 & 8.5)
pub const INODE_FLAG_IMMUTABLE: u32 = 1 << 0;
pub const INODE_FLAG_APPEND_ONLY: u32 = 1 << 1;
pub const INODE_FLAG_EXECUTE_AS: u32 = 1 << 2;

// Mount flags
pub const MS_RDONLY: u32 = 1 << 0;
pub const MS_NOSUID: u32 = 1 << 1;
pub const MS_NOEXEC: u32 = 1 << 2;

// Dirent file types
pub const DT_FIFO: u8 = 1;
pub const DT_CHR: u8 = 2;
pub const DT_DIR: u8 = 4;
pub const DT_REG: u8 = 8;
pub const DT_LNK: u8 = 10;

/// UUID-based Access Control Entry (Section 8.3 & 8.5).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AclEntry {
    pub subject_uuid: [u8; 16],
    pub is_group: u8,
    pub allow_rwx: u8, // bit 2 = R, bit 1 = W, bit 0 = X
    pub deny_rwx: u8,
    pub _pad: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharDevKind {
    Null,
    Zero,
    Random,
    Urandom,
    Console,
    Tty,
    Serial,
}

#[derive(Clone)]
pub enum NodeKind {
    Regular {
        data: Vec<u8>,
    },
    Directory,
    Symlink {
        target: String,
    },
    CharDevice(CharDevKind),
    Fifo {
        reader: PipeReader,
        writer: PipeWriter,
    },
    ProcVirtual,
}

#[derive(Clone)]
pub struct VfsNode {
    pub inode_id: u64,
    pub object_uuid: Uuid,
    pub owner_user_uuid: Uuid,
    pub owner_group_uuid: Uuid,
    pub execute_as_uuid: Uuid,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub flags: u32,
    pub nlink: u32,
    pub ctime_ns: u64,
    pub mtime_ns: u64,
    pub atime_ns: u64,
    pub acl: Vec<AclEntry>,
    pub kind: NodeKind,
}

impl VfsNode {
    pub fn file_size(&self) -> u64 {
        match &self.kind {
            NodeKind::Regular { data } => data.len() as u64,
            NodeKind::Symlink { target } => target.len() as u64,
            _ => 0,
        }
    }

    pub fn dirent_type(&self) -> u8 {
        match &self.kind {
            NodeKind::Regular { .. } | NodeKind::ProcVirtual => DT_REG,
            NodeKind::Directory => DT_DIR,
            NodeKind::Symlink { .. } => DT_LNK,
            NodeKind::CharDevice(_) => DT_CHR,
            NodeKind::Fifo { .. } => DT_FIFO,
        }
    }

    pub fn to_stat(&self) -> AetherStat {
        let size = self.file_size();
        AetherStat {
            inode_id: self.inode_id,
            object_uuid: *self.object_uuid.as_bytes(),
            owner_user_uuid: *self.owner_user_uuid.as_bytes(),
            owner_group_uuid: *self.owner_group_uuid.as_bytes(),
            execute_as_uuid: *self.execute_as_uuid.as_bytes(),
            uid: self.uid,
            gid: self.gid,
            mode: self.mode,
            flags: self.flags,
            nlink: self.nlink,
            _pad: 0,
            size,
            blocks: size.div_ceil(512),
            blksize: 4096,
            dev: 1,
            ctime_ns: self.ctime_ns,
            mtime_ns: self.mtime_ns,
            atime_ns: self.atime_ns,
        }
    }
}

#[derive(Clone)]
pub struct MountEntry {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub flags: u32,
}

/// Resource backing an open file description.
#[derive(Clone)]
pub enum FdResource {
    Console {
        readable: bool,
        writable: bool,
    },
    VfsFile {
        path: String,
        snapshot: Option<Vec<u8>>,
    },
    PipeRead(PipeReader),
    PipeWrite(PipeWriter),
    UnixSocket(Arc<UnixSocket>),
    Shm(Arc<ShmObject>),
}

pub struct OpenFile {
    pub resource: FdResource,
    pub flags: u32,
    pub offset: IrqSpinLock<u64>,
}

impl OpenFile {
    pub fn new(resource: FdResource, flags: u32) -> Arc<Self> {
        Arc::new(Self {
            resource,
            flags,
            offset: IrqSpinLock::new(0),
        })
    }

    #[inline]
    pub fn is_nonblocking(&self) -> bool {
        (self.flags & O_NONBLOCK) != 0
    }

    #[inline]
    pub fn can_read(&self) -> bool {
        let acc = self.flags & O_ACCMODE;
        acc == O_RDONLY || acc == O_RDWR
    }

    #[inline]
    pub fn can_write(&self) -> bool {
        let acc = self.flags & O_ACCMODE;
        acc == O_WRONLY || acc == O_RDWR
    }
}

#[derive(Clone)]
pub struct FileDescriptorEntry {
    pub file: Arc<OpenFile>,
    pub cloexec: bool,
}

/// Build the initial `[stdin(0), stdout(1), stderr(2)]` file descriptor vector for a process.
pub fn default_stdio_fds() -> Vec<Option<FileDescriptorEntry>> {
    let stdin = FileDescriptorEntry {
        file: OpenFile::new(
            FdResource::Console {
                readable: true,
                writable: false,
            },
            O_RDONLY,
        ),
        cloexec: false,
    };
    let stdout = FileDescriptorEntry {
        file: OpenFile::new(
            FdResource::Console {
                readable: false,
                writable: true,
            },
            O_WRONLY,
        ),
        cloexec: false,
    };
    let stderr = FileDescriptorEntry {
        file: OpenFile::new(
            FdResource::Console {
                readable: false,
                writable: true,
            },
            O_WRONLY,
        ),
        cloexec: false,
    };
    alloc::vec![Some(stdin), Some(stdout), Some(stderr)]
}

struct VfsState {
    nodes: BTreeMap<String, VfsNode>,
    mounts: Vec<MountEntry>,
    next_inode: u64,
    rng_state: u64,
}

impl VfsState {
    const fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            mounts: Vec::new(),
            next_inode: 1,
            rng_state: 0x853c_49e6_748f_ea9b,
        }
    }

    fn alloc_inode_and_uuid(&mut self) -> (u64, Uuid) {
        let ino = self.next_inode;
        self.next_inode += 1;
        let uuid = Uuid::deterministic(0xAE74_F500, ino);
        (ino, uuid)
    }

    fn next_random_u64(&mut self) -> u64 {
        self.rng_state ^= cpu::rdtsc();
        self.rng_state = self.rng_state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng_state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn mount_flags_for_path(&self, path: &str) -> u32 {
        let mut best_len = 0usize;
        let mut flags = 0u32;
        for m in &self.mounts {
            let matches_mount = path == m.target
                || (m.target == "/" && path.starts_with('/'))
                || (path.starts_with(&m.target)
                    && path.as_bytes().get(m.target.len()) == Some(&b'/'));
            if matches_mount && m.target.len() >= best_len {
                best_len = m.target.len();
                flags = m.flags;
            }
        }
        flags
    }

    fn insert_dir(&mut self, path: &str, mode: u32, uid: u32, gid: u32) {
        let (ino, obj_uuid) = self.alloc_inode_and_uuid();
        let owner_user_uuid = if uid == 0 {
            Uuid::ROOT_USER
        } else {
            Uuid::deterministic(1, uid as u64)
        };
        let owner_group_uuid = if gid == 0 {
            Uuid::ROOT_GROUP
        } else {
            Uuid::deterministic(2, gid as u64)
        };
        let now = timer::uptime_ns();
        self.nodes.insert(
            String::from(path),
            VfsNode {
                inode_id: ino,
                object_uuid: obj_uuid,
                owner_user_uuid,
                owner_group_uuid,
                execute_as_uuid: Uuid::NIL,
                uid,
                gid,
                mode: S_IFDIR | (mode & 0o7777),
                flags: 0,
                nlink: 2,
                ctime_ns: now,
                mtime_ns: now,
                atime_ns: now,
                acl: Vec::new(),
                kind: NodeKind::Directory,
            },
        );
    }

    fn insert_file(&mut self, path: &str, mode: u32, uid: u32, gid: u32, data: Vec<u8>) {
        let (ino, obj_uuid) = self.alloc_inode_and_uuid();
        let owner_user_uuid = if uid == 0 {
            Uuid::ROOT_USER
        } else {
            Uuid::deterministic(1, uid as u64)
        };
        let owner_group_uuid = if gid == 0 {
            Uuid::ROOT_GROUP
        } else {
            Uuid::deterministic(2, gid as u64)
        };
        let now = timer::uptime_ns();
        self.nodes.insert(
            String::from(path),
            VfsNode {
                inode_id: ino,
                object_uuid: obj_uuid,
                owner_user_uuid,
                owner_group_uuid,
                execute_as_uuid: Uuid::NIL,
                uid,
                gid,
                mode: S_IFREG | (mode & 0o7777),
                flags: 0,
                nlink: 1,
                ctime_ns: now,
                mtime_ns: now,
                atime_ns: now,
                acl: Vec::new(),
                kind: NodeKind::Regular { data },
            },
        );
    }

    fn insert_chardev(&mut self, path: &str, mode: u32, dev: CharDevKind) {
        let (ino, obj_uuid) = self.alloc_inode_and_uuid();
        let now = timer::uptime_ns();
        self.nodes.insert(
            String::from(path),
            VfsNode {
                inode_id: ino,
                object_uuid: obj_uuid,
                owner_user_uuid: Uuid::ROOT_USER,
                owner_group_uuid: Uuid::ROOT_GROUP,
                execute_as_uuid: Uuid::NIL,
                uid: 0,
                gid: 0,
                mode: S_IFCHR | (mode & 0o777),
                flags: 0,
                nlink: 1,
                ctime_ns: now,
                mtime_ns: now,
                atime_ns: now,
                acl: Vec::new(),
                kind: NodeKind::CharDevice(dev),
            },
        );
    }

    fn insert_proc(&mut self, path: &str, mode: u32) {
        let (ino, obj_uuid) = self.alloc_inode_and_uuid();
        let now = timer::uptime_ns();
        self.nodes.insert(
            String::from(path),
            VfsNode {
                inode_id: ino,
                object_uuid: obj_uuid,
                owner_user_uuid: Uuid::ROOT_USER,
                owner_group_uuid: Uuid::ROOT_GROUP,
                execute_as_uuid: Uuid::NIL,
                uid: 0,
                gid: 0,
                mode: S_IFREG | (mode & 0o777),
                flags: 0,
                nlink: 1,
                ctime_ns: now,
                mtime_ns: now,
                atime_ns: now,
                acl: Vec::new(),
                kind: NodeKind::ProcVirtual,
            },
        );
    }
}

static VFS: IrqSpinLock<VfsState> = IrqSpinLock::new(VfsState::new());
static VFS_OPS_COUNT: AtomicU64 = AtomicU64::new(0);

/// Normalize an arbitrary path against `cwd`, resolving `.` and `..` cleanly.
pub fn normalize_path(cwd: &str, raw_path: &str) -> Result<String, Errno> {
    if raw_path.is_empty() {
        return Err(Errno::Enoent);
    }
    let mut parts: Vec<&str> = Vec::new();
    if !raw_path.starts_with('/') {
        for seg in cwd.split('/') {
            if !seg.is_empty() && seg != "." {
                parts.push(seg);
            }
        }
    }
    for seg in raw_path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => {
                if other.len() > 239 {
                    return Err(Errno::Enametoolong);
                }
                parts.push(other);
            }
        }
    }
    if parts.is_empty() {
        return Ok(String::from("/"));
    }
    let mut out = String::new();
    for seg in parts {
        out.push('/');
        out.push_str(seg);
    }
    Ok(out)
}

pub fn parent_and_name(norm_path: &str) -> Option<(String, String)> {
    if norm_path == "/" {
        return None;
    }
    match norm_path.rfind('/') {
        Some(0) => Some((String::from("/"), String::from(&norm_path[1..]))),
        Some(idx) => Some((
            String::from(&norm_path[..idx]),
            String::from(&norm_path[idx + 1..]),
        )),
        None => None,
    }
}

/// Evaluate permission & UUID-based ACL rules on `node` for `creds`.
/// Wanted mask bits: `4 = read`, `2 = write`, `1 = execute`.
pub fn check_node_permission(
    node: &VfsNode,
    creds: &Credentials,
    want_rwx: u8,
    mount_flags: u32,
) -> Result<(), Errno> {
    let want_w = (want_rwx & 2) != 0;
    let want_x = (want_rwx & 1) != 0;

    // 1. Read-only mount or immutable flag blocks all writes (even by root!)
    if want_w {
        if (mount_flags & MS_RDONLY) != 0 {
            return Err(Errno::Erofs);
        }
        if (node.flags & INODE_FLAG_IMMUTABLE) != 0 {
            return Err(Errno::Eperm);
        }
    }

    // 2. No-exec mount blocks file execution
    if want_x && (node.mode & S_IFMT) != S_IFDIR && (mount_flags & MS_NOEXEC) != 0 {
        return Err(Errno::Eacces);
    }

    // 3. Root / CAP_DAC_OVERRIDE bypasses read/write DAC, and bypasses execute if directory or any x bit set
    if creds.euid == 0 || (creds.caps_effective & CAP_DAC_OVERRIDE) != 0 {
        if want_x && (node.mode & S_IFMT) != S_IFDIR && (node.mode & 0o111) == 0 {
            return Err(Errno::Eacces);
        }
        return Ok(());
    }

    // 4. Check explicit UUID-based ACL entries first (deny takes precedence over allow)
    let mut acl_matched = false;
    let mut acl_allow = 0u8;
    let mut acl_deny = 0u8;
    for ace in &node.acl {
        let matches = if ace.is_group == 0 {
            ace.subject_uuid == *creds.user_uuid.as_bytes()
        } else {
            ace.subject_uuid == *creds.group_uuid.as_bytes()
        };
        if matches {
            acl_matched = true;
            acl_allow |= ace.allow_rwx;
            acl_deny |= ace.deny_rwx;
        }
    }
    if (acl_deny & want_rwx) != 0 {
        return Err(Errno::Eacces);
    }
    if acl_matched && (acl_allow & want_rwx) == want_rwx {
        return Ok(());
    }

    // 5. Standard Owner UUID/UID, Group UUID/GID, Other evaluation
    let mode_bits = if creds.user_uuid == node.owner_user_uuid || creds.euid == node.uid {
        ((node.mode >> 6) & 7) as u8
    } else if creds.group_uuid == node.owner_group_uuid
        || creds.egid == node.gid
        || creds.groups.contains(&node.gid)
    {
        ((node.mode >> 3) & 7) as u8
    } else {
        (node.mode & 7) as u8
    };

    if (mode_bits & want_rwx) == want_rwx {
        Ok(())
    } else {
        Err(Errno::Eacces)
    }
}

/// Resolve a normalized path inside `vfs`, following symlinks up to 8 hops when `follow_final == true`.
fn resolve_path_locked(
    vfs: &VfsState,
    start_norm: &str,
    follow_final: bool,
) -> Result<String, Errno> {
    let mut cur = String::from(start_norm);
    for _ in 0..8 {
        if !follow_final {
            return Ok(cur);
        }
        let Some(node) = vfs.nodes.get(&cur) else {
            return Ok(cur);
        };
        if let NodeKind::Symlink { target } = &node.kind {
            let parent = parent_and_name(&cur)
                .map(|(p, _)| p)
                .unwrap_or_else(|| String::from("/"));
            cur = normalize_path(&parent, target)?;
        } else {
            return Ok(cur);
        }
    }
    Err(Errno::Eloop)
}

/// Initialize the VFS hierarchy (Section 9), device nodes, `/proc` nodes, default `/etc` files,
/// and unpack any files present in `initramfs.tar`.
pub fn init(initramfs: Option<&[u8]>) {
    let mut vfs = VFS.lock();

    vfs.mounts.push(MountEntry {
        source: String::from("rootfs"),
        target: String::from("/"),
        fstype: String::from("tmpfs"),
        flags: 0,
    });
    vfs.mounts.push(MountEntry {
        source: String::from("devfs"),
        target: String::from("/dev"),
        fstype: String::from("devfs"),
        flags: MS_NOSUID,
    });
    vfs.mounts.push(MountEntry {
        source: String::from("procfs"),
        target: String::from("/proc"),
        fstype: String::from("procfs"),
        flags: MS_RDONLY | MS_NOSUID | MS_NOEXEC,
    });
    vfs.mounts.push(MountEntry {
        source: String::from("tmpfs"),
        target: String::from("/tmp"),
        fstype: String::from("tmpfs"),
        flags: MS_NOSUID,
    });
    vfs.mounts.push(MountEntry {
        source: String::from("tmpfs"),
        target: String::from("/run"),
        fstype: String::from("tmpfs"),
        flags: MS_NOSUID,
    });

    // Section 9 directory structure
    for dir in [
        "/",
        "/bin",
        "/sbin",
        "/boot",
        "/dev",
        "/etc",
        "/home",
        "/home/aether",
        "/lib",
        "/mnt",
        "/opt",
        "/proc",
        "/root",
        "/run",
        "/usr",
        "/usr/bin",
        "/usr/lib",
        "/usr/include",
        "/usr/share",
        "/var",
        "/var/log",
        "/var/cache",
        "/var/lib",
    ] {
        let (uid, gid, mode) = match dir {
            "/root" => (0, 0, 0o700),
            "/home/aether" => (1000, 1000, 0o750),
            _ => (0, 0, 0o755),
        };
        vfs.insert_dir(dir, mode, uid, gid);
    }
    // /tmp has sticky bit 0o1777
    vfs.insert_dir("/tmp", 0o1777, 0, 0);

    // /dev devices (Section 9)
    vfs.insert_chardev("/dev/null", 0o666, CharDevKind::Null);
    vfs.insert_chardev("/dev/zero", 0o666, CharDevKind::Zero);
    vfs.insert_chardev("/dev/random", 0o666, CharDevKind::Random);
    vfs.insert_chardev("/dev/urandom", 0o666, CharDevKind::Urandom);
    vfs.insert_chardev("/dev/console", 0o600, CharDevKind::Console);
    vfs.insert_chardev("/dev/tty", 0o666, CharDevKind::Tty);
    vfs.insert_chardev("/dev/serial", 0o660, CharDevKind::Serial);

    // /proc virtual files
    vfs.insert_proc("/proc/version", 0o444);
    vfs.insert_proc("/proc/uptime", 0o444);
    vfs.insert_proc("/proc/meminfo", 0o444);
    vfs.insert_proc("/proc/cpuinfo", 0o444);
    vfs.insert_proc("/proc/mounts", 0o444);
    vfs.insert_proc("/proc/syscalls", 0o444);
    vfs.insert_proc("/proc/gui", 0o444);

    // Core /etc identity & configuration files
    vfs.insert_file("/etc/hostname", 0o644, 0, 0, Vec::from(&b"aether\n"[..]));
    vfs.insert_file(
        "/etc/passwd",
        0o644,
        0,
        0,
        Vec::from(
            &b"root:x:0:0:00000000-0000-4000-8000-000000000001:System Root:/root:/bin/aethersh\naether:x:1000:1000:a37eb089-6126-44a4-94cb-02bf71a303e8:Aether User:/home/aether:/bin/aethersh\n"[..],
        ),
    );
    vfs.insert_file(
        "/etc/group",
        0o644,
        0,
        0,
        Vec::from(
            &b"root:x:0:00000000-0000-4000-8000-000000000000:root\nwheel:x:10:00000000-0000-4000-8000-00000000000a:root,aether\naether:x:1000:525c03f7-f7c7-4821-83fd-9d0d658a4ef3:aether\n"[..],
        ),
    );
    vfs.insert_file(
        "/etc/sudoers",
        0o440,
        0,
        0,
        Vec::from(
            &b"# AetherOS sudo policy (QEMU dev mode opt-in)\n%wheel ALL=(ALL) NOPASSWD_DEV: ALL\n"
                [..],
        ),
    );

    if let Some(tar_bytes) = initramfs {
        unpack_ustar_locked(&mut vfs, tar_bytes);
    }
}

fn parse_octal(field: &[u8]) -> u64 {
    let mut val = 0u64;
    for &b in field {
        if (b'0'..=b'7').contains(&b) {
            val = (val << 3) | ((b - b'0') as u64);
        } else if b == 0 {
            break;
        }
    }
    val
}

fn unpack_ustar_locked(vfs: &mut VfsState, tar: &[u8]) {
    let mut offset = 0usize;
    while offset + 512 <= tar.len() {
        let hdr = &tar[offset..offset + 512];
        if hdr[0] == 0 {
            break;
        }
        let name_end = hdr[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let raw_name = core::str::from_utf8(&hdr[..name_end]).unwrap_or("");
        let size = parse_octal(&hdr[124..136]) as usize;
        let mode = parse_octal(&hdr[100..108]) as u32;
        let typeflag = hdr[156];

        let data_start = offset + 512;
        let data_end = (data_start + size).min(tar.len());

        if let Ok(norm) = normalize_path("/", raw_name) {
            if norm != "/" {
                match typeflag {
                    b'5' => {
                        if !vfs.nodes.contains_key(&norm) {
                            vfs.insert_dir(&norm, if mode != 0 { mode } else { 0o755 }, 0, 0);
                        }
                    }
                    b'0' | 0 => {
                        let content = tar[data_start..data_end].to_vec();
                        let eff_mode = if mode != 0 {
                            mode
                        } else if norm.starts_with("/bin/") || norm.starts_with("/sbin/") {
                            0o755
                        } else {
                            0o644
                        };
                        vfs.insert_file(&norm, eff_mode, 0, 0, content);
                    }
                    _ => {}
                }
            }
        }

        let blocks = size.div_ceil(512);
        offset = data_start + blocks * 512;
    }
}

fn synthesize_proc_content(vfs: &VfsState, path: &str) -> Option<Vec<u8>> {
    let pmm_st = pmm::stats();
    let heap_st = heap::stats();
    let sched_st = sched::stats();
    let s = match path {
        "/proc/version" => String::from(
            "AetherOS version 0.1.0-alpha.1 (x86_64-unknown-none) (rustc 1.100.0, Limine Boot Protocol v3)\n",
        ),
        "/proc/uptime" => {
            let ms = timer::uptime_ms();
            format!("{}.{:03} ms ({} ticks)\n", ms / 1000, ms % 1000, ms)
        }
        "/proc/meminfo" => format!(
            "MemTotal:       {:8} kB\nMemFree:        {:8} kB\nKernelHeapUsed: {:8} kB\nKernelHeapMap:  {:8} kB\n",
            pmm_st.usable_frames * 4,
            pmm_st.free_frames * 4,
            heap_st.used_bytes / 1024,
            heap_st.mapped_bytes / 1024
        ),
        "/proc/cpuinfo" => String::from("processor\t: 0\narch\t\t: x86_64\nmode\t\t: 64-bit Long Mode (Ring 0 / Ring 3)\n"),
        "/proc/mounts" => {
            let mut out = String::new();
            for m in &vfs.mounts {
                let rw = if (m.flags & MS_RDONLY) != 0 { "ro" } else { "rw" };
                let suid = if (m.flags & MS_NOSUID) != 0 {
                    ",nosuid"
                } else {
                    ""
                };
                out.push_str(&format!("{} {} {} {}{}\n", m.source, m.target, m.fstype, rw, suid));
            }
            out
        }
        "/proc/syscalls" => format!(
            "total_syscalls: {}\ncontext_switches: {}\npreemptions: {}\n",
            crate::syscall::total_syscalls(),
            sched_st.context_switches,
            sched_st.preemptions
        ),
        "/proc/gui" => crate::gui::format_proc_gui(),
        _ => return None,
    };
    Some(s.into_bytes())
}

// --- High-level VFS Operations invoked by Syscalls and Kernel Subsystems ---

pub fn vfs_open(raw_path: &str, flags: u32, create_mode: u32) -> Result<Arc<OpenFile>, Errno> {
    VFS_OPS_COUNT.fetch_add(1, Ordering::Relaxed);
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;

    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let mount_flags = vfs.mount_flags_for_path(&resolved);

    let acc = flags & O_ACCMODE;
    let want_r = acc == O_RDONLY || acc == O_RDWR;
    let want_w = acc == O_WRONLY || acc == O_RDWR || (flags & O_TRUNC) != 0;
    let want_mask = (if want_r { 4 } else { 0 }) | (if want_w { 2 } else { 0 });

    if let Some(existing) = vfs.nodes.get(&resolved).cloned() {
        if (flags & O_CREAT) != 0 && (flags & O_EXCL) != 0 {
            return Err(Errno::Eexist);
        }
        if (flags & O_DIRECTORY) != 0 && !matches!(existing.kind, NodeKind::Directory) {
            return Err(Errno::Enotdir);
        }
        if want_w && matches!(existing.kind, NodeKind::Directory) {
            return Err(Errno::Eisdir);
        }
        check_node_permission(&existing, &creds, want_mask, mount_flags)?;

        match &existing.kind {
            NodeKind::Fifo { reader, writer } => {
                if acc == O_WRONLY {
                    return Ok(OpenFile::new(FdResource::PipeWrite(writer.clone()), flags));
                } else {
                    return Ok(OpenFile::new(FdResource::PipeRead(reader.clone()), flags));
                }
            }
            NodeKind::ProcVirtual => {
                let snap = synthesize_proc_content(&vfs, &resolved);
                return Ok(OpenFile::new(
                    FdResource::VfsFile {
                        path: resolved,
                        snapshot: snap,
                    },
                    flags,
                ));
            }
            NodeKind::Regular { .. } => {
                if (flags & O_TRUNC) != 0 && want_w {
                    if (existing.flags & INODE_FLAG_APPEND_ONLY) != 0 {
                        return Err(Errno::Eperm);
                    }
                    if let Some(node_mut) = vfs.nodes.get_mut(&resolved) {
                        if let NodeKind::Regular { data } = &mut node_mut.kind {
                            data.clear();
                        }
                        node_mut.mtime_ns = timer::uptime_ns();
                    }
                    drop(vfs);
                    crate::fs::aetherfs::sync_file_Write(&resolved, &[]);
                }
                return Ok(OpenFile::new(
                    FdResource::VfsFile {
                        path: resolved,
                        snapshot: None,
                    },
                    flags,
                ));
            }
            _ => {
                return Ok(OpenFile::new(
                    FdResource::VfsFile {
                        path: resolved,
                        snapshot: None,
                    },
                    flags,
                ));
            }
        }
    }

    // File does not exist: check O_CREAT
    if (flags & O_CREAT) == 0 {
        return Err(Errno::Enoent);
    }

    let (parent_path, _name) = parent_and_name(&resolved).ok_or(Errno::Einval)?;
    let parent_node = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    if !matches!(parent_node.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    // Creating a file requires write + execute (search) on parent directory
    check_node_permission(parent_node, &creds, 2 | 1, mount_flags)?;

    // Check setgid directory inheritance
    let inherited_gid = if (parent_node.mode & S_ISGID) != 0 {
        parent_node.gid
    } else {
        creds.egid
    };

    vfs.insert_file(
        &resolved,
        create_mode & 0o777,
        creds.euid,
        inherited_gid,
        Vec::new(),
    );
    drop(vfs);
    crate::fs::aetherfs::sync_file_create(
        &resolved,
        create_mode & 0o777,
        creds.euid,
        inherited_gid,
    );

    Ok(OpenFile::new(
        FdResource::VfsFile {
            path: resolved,
            snapshot: None,
        },
        flags,
    ))
}

pub fn vfs_read_file(
    file: &OpenFile,
    dst: &mut [u8],
    explicit_offset: Option<u64>,
) -> Result<usize, Errno> {
    if !file.can_read() {
        return Err(Errno::Ebadf);
    }
    match &file.resource {
        FdResource::Console { readable, .. } => {
            if !*readable {
                return Err(Errno::Ebadf);
            }
            if dst.is_empty() {
                return Ok(0);
            }
            if let Some(b) = serial::read_byte_nonblocking() {
                dst[0] = b;
                return Ok(1);
            }
            Ok(0)
        }
        FdResource::PipeRead(reader) => reader.inner.read(dst, file.is_nonblocking()),
        FdResource::PipeWrite(_) => Err(Errno::Ebadf),
        FdResource::UnixSocket(sock) => sock.read(dst, file.is_nonblocking()),
        FdResource::Shm(shm) => {
            let frames = shm.frames.lock();
            let total_size = frames.len() * PAGE_SIZE;
            let mut off_guard = file.offset.lock();
            let off = explicit_offset.unwrap_or(*off_guard) as usize;
            if off >= total_size {
                return Ok(0);
            }
            let to_read = dst.len().min(total_size - off);
            for (i, slot) in dst.iter_mut().enumerate().take(to_read) {
                let pos = off + i;
                let frame_phys = frames[pos / PAGE_SIZE];
                let byte_off = pos % PAGE_SIZE;
                *slot = unsafe { *pmm::phys_to_virt(frame_phys).add(byte_off) };
            }
            if explicit_offset.is_none() {
                *off_guard = (off + to_read) as u64;
            }
            Ok(to_read)
        }
        FdResource::VfsFile { path, snapshot } => {
            if let Some(snap) = snapshot {
                let mut off_guard = file.offset.lock();
                let off = explicit_offset.unwrap_or(*off_guard) as usize;
                if off >= snap.len() {
                    return Ok(0);
                }
                let n = dst.len().min(snap.len() - off);
                dst[..n].copy_from_slice(&snap[off..off + n]);
                if explicit_offset.is_none() {
                    *off_guard = (off + n) as u64;
                }
                return Ok(n);
            }

            let mut vfs = VFS.lock();
            let node = vfs.nodes.get(path).cloned().ok_or(Errno::Enoent)?;
            match node.kind {
                NodeKind::Directory => Err(Errno::Eisdir),
                NodeKind::CharDevice(dev) => match dev {
                    CharDevKind::Null => Ok(0),
                    CharDevKind::Zero => {
                        dst.fill(0);
                        Ok(dst.len())
                    }
                    CharDevKind::Random | CharDevKind::Urandom => {
                        for chunk in dst.chunks_mut(8) {
                            let r = vfs.next_random_u64().to_le_bytes();
                            chunk.copy_from_slice(&r[..chunk.len()]);
                        }
                        Ok(dst.len())
                    }
                    CharDevKind::Console | CharDevKind::Tty | CharDevKind::Serial => {
                        if dst.is_empty() {
                            return Ok(0);
                        }
                        if let Some(b) = serial::read_byte_nonblocking() {
                            dst[0] = b;
                            Ok(1)
                        } else {
                            Ok(0)
                        }
                    }
                },
                NodeKind::Regular { data } => {
                    let mut off_guard = file.offset.lock();
                    let off = explicit_offset.unwrap_or(*off_guard) as usize;
                    if off >= data.len() {
                        return Ok(0);
                    }
                    let n = dst.len().min(data.len() - off);
                    dst[..n].copy_from_slice(&data[off..off + n]);
                    if explicit_offset.is_none() {
                        *off_guard = (off + n) as u64;
                    }
                    Ok(n)
                }
                _ => Err(Errno::Einval),
            }
        }
    }
}

pub fn vfs_write_file(
    file: &OpenFile,
    src: &[u8],
    explicit_offset: Option<u64>,
) -> Result<usize, Errno> {
    if !file.can_write() {
        return Err(Errno::Ebadf);
    }
    match &file.resource {
        FdResource::Console { writable, .. } => {
            if !*writable {
                return Err(Errno::Ebadf);
            }
            if let Ok(s) = core::str::from_utf8(src) {
                serial::write_str(s);
            } else {
                for &b in src {
                    serial::COM1.lock().write_byte(b);
                }
            }
            Ok(src.len())
        }
        FdResource::PipeWrite(writer) => writer.inner.write(src, file.is_nonblocking()),
        FdResource::PipeRead(_) => Err(Errno::Ebadf),
        FdResource::UnixSocket(sock) => sock.write(src, file.is_nonblocking()),
        FdResource::Shm(shm) => {
            let mut off_guard = file.offset.lock();
            let off = explicit_offset.unwrap_or(*off_guard) as usize;
            let end = off.checked_add(src.len()).ok_or(Errno::Efbig)?;
            let needed_pages = end.div_ceil(PAGE_SIZE);
            let frames = shm.ensure_pages(needed_pages)?;
            for (i, &b) in src.iter().enumerate() {
                let pos = off + i;
                let frame_phys = frames[pos / PAGE_SIZE];
                let byte_off = pos % PAGE_SIZE;
                unsafe {
                    *pmm::phys_to_virt(frame_phys).add(byte_off) = b;
                }
            }
            if explicit_offset.is_none() {
                *off_guard = end as u64;
            }
            Ok(src.len())
        }
        FdResource::VfsFile { path, .. } => {
            let mut vfs = VFS.lock();
            let mount_flags = vfs.mount_flags_for_path(path);
            if (mount_flags & MS_RDONLY) != 0 {
                return Err(Errno::Erofs);
            }
            let node = vfs.nodes.get_mut(path).ok_or(Errno::Enoent)?;
            if (node.flags & INODE_FLAG_IMMUTABLE) != 0 {
                return Err(Errno::Eperm);
            }
            match &mut node.kind {
                NodeKind::Directory => Err(Errno::Eisdir),
                NodeKind::CharDevice(dev) => match dev {
                    CharDevKind::Null
                    | CharDevKind::Zero
                    | CharDevKind::Random
                    | CharDevKind::Urandom => Ok(src.len()),
                    CharDevKind::Console | CharDevKind::Tty | CharDevKind::Serial => {
                        if let Ok(s) = core::str::from_utf8(src) {
                            serial::write_str(s);
                        } else {
                            for &b in src {
                                serial::COM1.lock().write_byte(b);
                            }
                        }
                        Ok(src.len())
                    }
                },
                NodeKind::Regular { data } => {
                    let mut off_guard = file.offset.lock();
                    let append =
                        (file.flags & O_APPEND) != 0 || (node.flags & INODE_FLAG_APPEND_ONLY) != 0;
                    if (node.flags & INODE_FLAG_APPEND_ONLY) != 0 && explicit_offset.is_some() {
                        return Err(Errno::Eperm);
                    }
                    let off = if append {
                        data.len()
                    } else {
                        explicit_offset.unwrap_or(*off_guard) as usize
                    };
                    let end = off.checked_add(src.len()).ok_or(Errno::Efbig)?;
                    if end > 16 * 1024 * 1024 {
                        return Err(Errno::Enospc);
                    }
                    if end > data.len() {
                        data.resize(end, 0);
                    }
                    data[off..end].copy_from_slice(src);
                    node.mtime_ns = timer::uptime_ns();
                    if explicit_offset.is_none() {
                        *off_guard = end as u64;
                    }
                    let snapshot_copy = data.clone();
                    drop(off_guard);
                    drop(vfs);
                    crate::fs::aetherfs::sync_file_Write(path, &snapshot_copy);
                    Ok(src.len())
                }
                _ => Err(Errno::Einval),
            }
        }
    }
}

pub fn vfs_lseek(file: &OpenFile, offset: i64, whence: u32) -> Result<u64, Errno> {
    let size = match &file.resource {
        FdResource::VfsFile { path, snapshot } => {
            if let Some(snap) = snapshot {
                snap.len() as i64
            } else {
                let vfs = VFS.lock();
                let node = vfs.nodes.get(path).ok_or(Errno::Enoent)?;
                node.file_size() as i64
            }
        }
        FdResource::Shm(shm) => (shm.frames.lock().len() * PAGE_SIZE) as i64,
        _ => return Err(Errno::Espipe),
    };

    let mut cur = file.offset.lock();
    let new_off = match whence {
        SEEK_SET => offset,
        SEEK_CUR => (*cur as i64).checked_add(offset).ok_or(Errno::Einval)?,
        SEEK_END => size.checked_add(offset).ok_or(Errno::Einval)?,
        _ => return Err(Errno::Einval),
    };
    if new_off < 0 {
        return Err(Errno::Einval);
    }
    *cur = new_off as u64;
    Ok(*cur)
}

pub fn vfs_stat_path(raw_path: &str, follow_symlinks: bool) -> Result<AetherStat, Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, follow_symlinks)?;
    let node = vfs.nodes.get(&resolved).ok_or(Errno::Enoent)?;
    let mut st = node.to_stat();
    if matches!(node.kind, NodeKind::ProcVirtual) {
        if let Some(snap) = synthesize_proc_content(&vfs, &resolved) {
            st.size = snap.len() as u64;
        }
    }
    Ok(st)
}

pub fn vfs_fstat(file: &OpenFile) -> Result<AetherStat, Errno> {
    match &file.resource {
        FdResource::VfsFile { path, snapshot } => {
            let vfs = VFS.lock();
            let node = vfs.nodes.get(path).ok_or(Errno::Enoent)?;
            let mut st = node.to_stat();
            if let Some(snap) = snapshot {
                st.size = snap.len() as u64;
            }
            Ok(st)
        }
        FdResource::Console { .. } => Ok(AetherStat {
            mode: S_IFCHR | 0o666,
            blksize: 4096,
            ..AetherStat::default()
        }),
        FdResource::PipeRead(_) | FdResource::PipeWrite(_) => Ok(AetherStat {
            mode: S_IFIFO | 0o600,
            blksize: 4096,
            ..AetherStat::default()
        }),
        FdResource::Shm(shm) => Ok(AetherStat {
            uid: shm.owner_uid,
            gid: shm.owner_gid,
            owner_user_uuid: *shm.owner_uuid.as_bytes(),
            mode: S_IFREG | shm.mode,
            size: (shm.frames.lock().len() * PAGE_SIZE) as u64,
            blksize: 4096,
            ..AetherStat::default()
        }),
        FdResource::UnixSocket(_) => Ok(AetherStat {
            mode: 0o140666,
            blksize: 4096,
            ..AetherStat::default()
        }),
    }
}

pub fn vfs_mkdir(raw_path: &str, mode: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    if vfs.nodes.contains_key(&norm) {
        return Err(Errno::Eexist);
    }
    let mount_flags = vfs.mount_flags_for_path(&norm);
    let (parent_path, _) = parent_and_name(&norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    if !matches!(parent.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    let gid = if (parent.mode & S_ISGID) != 0 {
        parent.gid
    } else {
        creds.egid
    };
    let dir_mode = (mode & 0o777) | (parent.mode & S_ISGID);
    vfs.insert_dir(&norm, dir_mode, creds.euid, gid);
    drop(vfs);
    crate::fs::aetherfs::sync_mkdir(&norm, dir_mode, creds.euid, gid);
    Ok(())
}

pub fn vfs_mkfifo(raw_path: &str, mode: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    if vfs.nodes.contains_key(&norm) {
        return Err(Errno::Eexist);
    }
    let mount_flags = vfs.mount_flags_for_path(&norm);
    let (parent_path, _) = parent_and_name(&norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    let (ino, obj_uuid) = vfs.alloc_inode_and_uuid();
    let (reader, writer) = crate::ipc::Pipe::new_pair();
    let now = timer::uptime_ns();
    vfs.nodes.insert(
        norm,
        VfsNode {
            inode_id: ino,
            object_uuid: obj_uuid,
            owner_user_uuid: creds.user_uuid,
            owner_group_uuid: creds.group_uuid,
            execute_as_uuid: Uuid::NIL,
            uid: creds.euid,
            gid: creds.egid,
            mode: S_IFIFO | (mode & 0o777),
            flags: 0,
            nlink: 1,
            ctime_ns: now,
            mtime_ns: now,
            atime_ns: now,
            acl: Vec::new(),
            kind: NodeKind::Fifo { reader, writer },
        },
    );
    Ok(())
}

pub fn vfs_rmdir(raw_path: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    if norm == "/" || norm == "/dev" || norm == "/proc" || norm == "/tmp" {
        return Err(Errno::Ebusy);
    }
    let mut vfs = VFS.lock();
    let mount_flags = vfs.mount_flags_for_path(&norm);
    let node = vfs.nodes.get(&norm).ok_or(Errno::Enoent)?;
    if !matches!(node.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    // Check directory is empty
    let prefix = format!("{}/", norm);
    if vfs.nodes.keys().any(|k| k.starts_with(&prefix)) {
        return Err(Errno::Enotempty);
    }
    let (parent_path, _) = parent_and_name(&norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    check_sticky_delete_rule(parent, node, &creds)?;
    vfs.nodes.remove(&norm);
    drop(vfs);
    crate::fs::aetherfs::sync_unlink(&norm);
    Ok(())
}

fn check_sticky_delete_rule(
    parent: &VfsNode,
    target: &VfsNode,
    creds: &Credentials,
) -> Result<(), Errno> {
    if (parent.mode & S_ISVTX) != 0
        && creds.euid != 0
        && (creds.caps_effective & CAP_FOWNER) == 0
        && creds.euid != parent.uid
        && creds.euid != target.uid
        && creds.user_uuid != target.owner_user_uuid
    {
        return Err(Errno::Eperm);
    }
    Ok(())
}

pub fn vfs_unlink(raw_path: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let mount_flags = vfs.mount_flags_for_path(&norm);
    let node = vfs.nodes.get(&norm).ok_or(Errno::Enoent)?;
    if matches!(node.kind, NodeKind::Directory) {
        return Err(Errno::Eisdir);
    }
    if (node.flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND_ONLY)) != 0 {
        return Err(Errno::Eperm);
    }
    let (parent_path, _) = parent_and_name(&norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    check_sticky_delete_rule(parent, node, &creds)?;
    vfs.nodes.remove(&norm);
    drop(vfs);
    crate::fs::aetherfs::sync_unlink(&norm);
    Ok(())
}

pub fn vfs_rename(old_raw: &str, new_raw: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let old_norm = normalize_path(&cwd, old_raw)?;
    let new_norm = normalize_path(&cwd, new_raw)?;
    if old_norm == "/" || new_norm == "/" {
        return Err(Errno::Ebusy);
    }
    let mut vfs = VFS.lock();
    let mount_flags = vfs.mount_flags_for_path(&old_norm);
    let node = vfs.nodes.get(&old_norm).cloned().ok_or(Errno::Enoent)?;
    if (node.flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND_ONLY)) != 0 {
        return Err(Errno::Eperm);
    }
    let (old_parent_path, _) = parent_and_name(&old_norm).ok_or(Errno::Einval)?;
    let old_parent = vfs.nodes.get(&old_parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(old_parent, &creds, 2 | 1, mount_flags)?;
    check_sticky_delete_rule(old_parent, &node, &creds)?;

    let (new_parent_path, _) = parent_and_name(&new_norm).ok_or(Errno::Einval)?;
    let new_parent = vfs.nodes.get(&new_parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(new_parent, &creds, 2 | 1, mount_flags)?;

    vfs.nodes.remove(&old_norm);
    vfs.nodes.insert(new_norm.clone(), node);
    drop(vfs);
    crate::fs::aetherfs::sync_unlink(&old_norm);
    if let Ok(st_data) = vfs_read_all(&new_norm) {
        crate::fs::aetherfs::sync_file_Write(&new_norm, &st_data);
    }
    Ok(())
}

pub fn vfs_symlink(target_raw: &str, linkpath_raw: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let link_norm = normalize_path(&cwd, linkpath_raw)?;
    let mut vfs = VFS.lock();
    if vfs.nodes.contains_key(&link_norm) {
        return Err(Errno::Eexist);
    }
    let mount_flags = vfs.mount_flags_for_path(&link_norm);
    let (parent_path, _) = parent_and_name(&link_norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    let (ino, obj_uuid) = vfs.alloc_inode_and_uuid();
    let now = timer::uptime_ns();
    vfs.nodes.insert(
        link_norm,
        VfsNode {
            inode_id: ino,
            object_uuid: obj_uuid,
            owner_user_uuid: creds.user_uuid,
            owner_group_uuid: creds.group_uuid,
            execute_as_uuid: Uuid::NIL,
            uid: creds.euid,
            gid: creds.egid,
            mode: S_IFLNK | 0o777,
            flags: 0,
            nlink: 1,
            ctime_ns: now,
            mtime_ns: now,
            atime_ns: now,
            acl: Vec::new(),
            kind: NodeKind::Symlink {
                target: String::from(target_raw),
            },
        },
    );
    Ok(())
}

pub fn vfs_readlink(raw_path: &str) -> Result<String, Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let node = vfs.nodes.get(&norm).ok_or(Errno::Enoent)?;
    if let NodeKind::Symlink { target } = &node.kind {
        Ok(target.clone())
    } else {
        Err(Errno::Einval)
    }
}

pub fn vfs_link(old_raw: &str, new_raw: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let old_norm = normalize_path(&cwd, old_raw)?;
    let new_norm = normalize_path(&cwd, new_raw)?;
    let mut vfs = VFS.lock();
    if vfs.nodes.contains_key(&new_norm) {
        return Err(Errno::Eexist);
    }
    let mount_flags = vfs.mount_flags_for_path(&new_norm);
    let (parent_path, _) = parent_and_name(&new_norm).ok_or(Errno::Einval)?;
    let parent = vfs.nodes.get(&parent_path).ok_or(Errno::Enoent)?;
    check_node_permission(parent, &creds, 2 | 1, mount_flags)?;
    let existing = vfs.nodes.get_mut(&old_norm).ok_or(Errno::Enoent)?;
    if matches!(existing.kind, NodeKind::Directory) {
        return Err(Errno::Eperm);
    }
    existing.nlink += 1;
    let clone_node = existing.clone();
    vfs.nodes.insert(new_norm, clone_node);
    Ok(())
}

pub fn vfs_chmod(raw_path: &str, new_mode: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let mount_flags = vfs.mount_flags_for_path(&resolved);
    if (mount_flags & MS_RDONLY) != 0 {
        return Err(Errno::Erofs);
    }
    let node = vfs.nodes.get_mut(&resolved).ok_or(Errno::Enoent)?;
    if (node.flags & INODE_FLAG_IMMUTABLE) != 0 {
        return Err(Errno::Eperm);
    }
    if creds.euid != 0
        && (creds.caps_effective & CAP_FOWNER) == 0
        && creds.euid != node.uid
        && creds.user_uuid != node.owner_user_uuid
    {
        return Err(Errno::Eperm);
    }
    let type_bits = node.mode & S_IFMT;
    let mut perm_bits = new_mode & 0o7777;
    // Non-root cannot set SUID to root or retain SGID on unowned group
    if creds.euid != 0 && creds.egid != node.gid && !creds.groups.contains(&node.gid) {
        perm_bits &= !S_ISGID;
    }
    node.mode = type_bits | perm_bits;
    node.ctime_ns = timer::uptime_ns();
    drop(vfs);
    crate::fs::aetherfs::sync_metadata(&resolved);
    Ok(())
}

pub fn vfs_chown(raw_path: &str, new_uid: u32, new_gid: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let mount_flags = vfs.mount_flags_for_path(&resolved);
    if (mount_flags & MS_RDONLY) != 0 {
        return Err(Errno::Erofs);
    }
    let node = vfs.nodes.get_mut(&resolved).ok_or(Errno::Enoent)?;
    if (node.flags & INODE_FLAG_IMMUTABLE) != 0 {
        return Err(Errno::Eperm);
    }
    let is_priv = creds.euid == 0 || (creds.caps_effective & CAP_CHOWN) != 0;

    if new_uid != u32::MAX && new_uid != node.uid {
        if !is_priv {
            return Err(Errno::Eperm);
        }
        node.uid = new_uid;
        node.owner_user_uuid = crate::security::uid_to_uuid(new_uid);
    }
    if new_gid != u32::MAX && new_gid != node.gid {
        if !is_priv
            && (creds.euid != node.uid
                || (creds.egid != new_gid && !creds.groups.contains(&new_gid)))
        {
            return Err(Errno::Eperm);
        }
        node.gid = new_gid;
        node.owner_group_uuid = crate::security::gid_to_uuid(new_gid);
    }

    // Security rule: chown clears SUID/SGID bits on regular executables when modified by non-root
    if !is_priv {
        node.mode &= !(S_ISUID | S_ISGID);
    }
    node.ctime_ns = timer::uptime_ns();
    drop(vfs);
    crate::fs::aetherfs::sync_metadata(&resolved);
    Ok(())
}

/// Set or clear the protected `execute_as_uuid` field on an executable inode (Section 8.4).
/// Strictly requires root (`euid == 0`) or `CAP_SET_EXECUTE_AS`.
pub fn vfs_set_execute_as(raw_path: &str, target_uuid: Uuid) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    if creds.euid != 0 && (creds.caps_effective & CAP_SET_EXECUTE_AS) == 0 {
        crate::security::audit_log(
            "DENIED_SET_EXECUTE_AS",
            creds.euid,
            creds.user_uuid,
            raw_path,
        );
        return Err(Errno::Eperm);
    }
    // Validate that target_uuid is either NIL (clearing) or an existing active user UUID in Identity DB!
    if !target_uuid.is_nil() && !crate::security::user_uuid_exists(target_uuid) {
        return Err(Errno::Einval);
    }

    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get_mut(&resolved).ok_or(Errno::Enoent)?;
    if (node.flags & INODE_FLAG_IMMUTABLE) != 0 {
        return Err(Errno::Eperm);
    }
    node.execute_as_uuid = target_uuid;
    if target_uuid.is_nil() {
        node.flags &= !INODE_FLAG_EXECUTE_AS;
    } else {
        node.flags |= INODE_FLAG_EXECUTE_AS;
    }
    node.ctime_ns = timer::uptime_ns();
    drop(vfs);

    crate::security::audit_log("SET_EXECUTE_AS", creds.euid, target_uuid, &resolved);
    crate::fs::aetherfs::sync_metadata(&resolved);
    Ok(())
}

pub fn vfs_get_execute_as(raw_path: &str) -> Result<Uuid, Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get(&resolved).ok_or(Errno::Enoent)?;
    Ok(node.execute_as_uuid)
}

pub fn vfs_set_acl(raw_path: &str, entries: Vec<AclEntry>) -> Result<(), Errno> {
    if entries.len() > 32 {
        return Err(Errno::E2big);
    }
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get_mut(&resolved).ok_or(Errno::Enoent)?;
    if creds.euid != 0
        && (creds.caps_effective & CAP_FOWNER) == 0
        && creds.euid != node.uid
        && creds.user_uuid != node.owner_user_uuid
    {
        return Err(Errno::Eperm);
    }
    node.acl = entries;
    node.ctime_ns = timer::uptime_ns();
    drop(vfs);
    crate::fs::aetherfs::sync_metadata(&resolved);
    Ok(())
}

pub fn vfs_get_acl(raw_path: &str) -> Result<Vec<AclEntry>, Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get(&resolved).ok_or(Errno::Enoent)?;
    Ok(node.acl.clone())
}

pub fn vfs_set_inode_flags(raw_path: &str, new_flags: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    if creds.euid != 0 {
        return Err(Errno::Eperm);
    }
    let norm = normalize_path(&cwd, raw_path)?;
    let mut vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get_mut(&resolved).ok_or(Errno::Enoent)?;
    node.flags = (node.flags & INODE_FLAG_EXECUTE_AS)
        | (new_flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND_ONLY));
    drop(vfs);
    crate::fs::aetherfs::sync_metadata(&resolved);
    Ok(())
}

/// List immediate directory children for `sys_getdents64`.
pub fn vfs_getdents64(file: &OpenFile, out: &mut [Dirent64]) -> Result<usize, Errno> {
    let dir_path = match &file.resource {
        FdResource::VfsFile { path, .. } => path.clone(),
        _ => return Err(Errno::Enotdir),
    };
    let creds = sched::with_current_task(|t| t.creds.clone());
    let vfs = VFS.lock();
    let dir_node = vfs.nodes.get(&dir_path).ok_or(Errno::Enoent)?;
    if !matches!(dir_node.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    let mount_flags = vfs.mount_flags_for_path(&dir_path);
    check_node_permission(dir_node, &creds, 4, mount_flags)?;

    let mut entries: Vec<(u64, u8, String)> = Vec::new();
    let prefix = if dir_path == "/" {
        String::from("/")
    } else {
        format!("{}/", dir_path)
    };

    for (k, node) in &vfs.nodes {
        if k == &dir_path {
            continue;
        }
        if let Some(rest) = k.strip_prefix(&prefix) {
            if !rest.is_empty() && !rest.contains('/') {
                entries.push((node.inode_id, node.dirent_type(), String::from(rest)));
            }
        }
    }

    let mut off_guard = file.offset.lock();
    let start_idx = *off_guard as usize;
    if start_idx >= entries.len() {
        return Ok(0);
    }
    let count = (entries.len() - start_idx).min(out.len());
    for (i, slot) in out.iter_mut().enumerate().take(count) {
        let (ino, dtype, ref name) = entries[start_idx + i];
        let mut d_name = [0u8; 240];
        let nb = name.as_bytes();
        let nlen = nb.len().min(239);
        d_name[..nlen].copy_from_slice(&nb[..nlen]);
        *slot = Dirent64 {
            d_ino: ino,
            d_reclen: core::mem::size_of::<Dirent64>() as u16,
            d_type: dtype,
            d_namlen: nlen as u8,
            _pad: 0,
            d_name,
        };
    }
    *off_guard = (start_idx + count) as u64;
    Ok(count)
}

pub fn vfs_chdir(raw_path: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let node = vfs.nodes.get(&resolved).ok_or(Errno::Enoent)?;
    if !matches!(node.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    let mount_flags = vfs.mount_flags_for_path(&resolved);
    check_node_permission(node, &creds, 1, mount_flags)?;
    drop(vfs);
    sched::with_current_task_mut(|t| t.cwd = resolved);
    Ok(())
}

pub fn vfs_mount(source: &str, target_raw: &str, fstype: &str, flags: u32) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    if creds.euid != 0 && (creds.caps_effective & CAP_MOUNT) == 0 {
        return Err(Errno::Eperm);
    }
    let target = normalize_path(&cwd, target_raw)?;
    let mut vfs = VFS.lock();
    let node = vfs.nodes.get(&target).ok_or(Errno::Enoent)?;
    if !matches!(node.kind, NodeKind::Directory) {
        return Err(Errno::Enotdir);
    }
    if let Some(existing) = vfs.mounts.iter_mut().find(|m| m.target == target) {
        existing.flags = flags;
        existing.source = String::from(source);
        existing.fstype = String::from(fstype);
    } else {
        vfs.mounts.push(MountEntry {
            source: String::from(source),
            target,
            fstype: String::from(fstype),
            flags,
        });
    }
    Ok(())
}

pub fn vfs_umount(target_raw: &str) -> Result<(), Errno> {
    let (cwd, creds) = sched::with_current_task(|t| (t.cwd.clone(), t.creds.clone()));
    if creds.euid != 0 && (creds.caps_effective & CAP_MOUNT) == 0 {
        return Err(Errno::Eperm);
    }
    let target = normalize_path(&cwd, target_raw)?;
    if target == "/" {
        return Err(Errno::Ebusy);
    }
    let mut vfs = VFS.lock();
    let idx = vfs
        .mounts
        .iter()
        .position(|m| m.target == target)
        .ok_or(Errno::Einval)?;
    vfs.mounts.remove(idx);
    Ok(())
}

/// Read an entire file into a kernel `Vec<u8>`, checking `want_rwx` (e.g. `4` for read or `1` for exec).
/// Also returns `(VfsNode, mount_flags)` so `execve` can inspect `execute_as_uuid`, `S_ISUID`, and `MS_NOSUID`.
pub fn vfs_read_executable_and_meta(
    raw_path: &str,
    creds: &Credentials,
) -> Result<(Vec<u8>, VfsNode, u32), Errno> {
    let cwd = sched::with_current_task(|t| t.cwd.clone());
    let norm = normalize_path(&cwd, raw_path)?;
    let vfs = VFS.lock();
    let resolved = resolve_path_locked(&vfs, &norm, true)?;
    let mount_flags = vfs.mount_flags_for_path(&resolved);
    let node = vfs.nodes.get(&resolved).cloned().ok_or(Errno::Enoent)?;
    if matches!(node.kind, NodeKind::Directory) {
        return Err(Errno::Eisdir);
    }
    check_node_permission(&node, creds, 1, mount_flags)?;
    match &node.kind {
        NodeKind::Regular { data } => Ok((data.clone(), node, mount_flags)),
        _ => Err(Errno::Eacces),
    }
}

pub fn vfs_read_all(raw_path: &str) -> Result<Vec<u8>, Errno> {
    let vfs = VFS.lock();
    let node = vfs.nodes.get(raw_path).ok_or(Errno::Enoent)?;
    match &node.kind {
        NodeKind::Regular { data } => Ok(data.clone()),
        _ => Err(Errno::Einval),
    }
}

pub fn vfs_get_node_clone(norm_path: &str) -> Option<VfsNode> {
    VFS.lock().nodes.get(norm_path).cloned()
}

pub fn vfs_import_persisted_node(path: String, node: VfsNode) {
    let mut vfs = VFS.lock();
    if node.inode_id >= vfs.next_inode {
        vfs.next_inode = node.inode_id + 1;
    }
    vfs.nodes.insert(path, node);
}

pub fn vfs_poll_file(file: &OpenFile) -> i16 {
    match &file.resource {
        FdResource::Console { readable, writable } => {
            let mut ev = 0i16;
            if *readable && serial::COM1.lock().has_data() {
                ev |= POLLIN;
            }
            if *writable {
                ev |= POLLOUT;
            }
            ev
        }
        FdResource::PipeRead(r) => r.inner.poll_read(),
        FdResource::PipeWrite(w) => w.inner.poll_write(),
        FdResource::UnixSocket(s) => s.poll(),
        FdResource::VfsFile { .. } | FdResource::Shm(_) => POLLIN | POLLOUT,
    }
}

pub mod aetherfs;
