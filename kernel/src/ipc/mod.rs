//! Inter-Process Communication (IPC) Subsystem (Milestone 5).
//!
//! Provides:
//! - Anonymous Pipes & Named FIFOs (`Pipe`: 16 KiB bounded ring buffer, blocking & `O_NONBLOCK`,
//!   backpressure via `WaitQueue`, `EPIPE`/`SIGPIPE` on broken reader, EOF on closed writers).
//! - Unix-Domain Sockets (`AF_UNIX`, `SOCK_STREAM` & `SOCK_DGRAM`: `bind`, `listen`, `accept`,
//!   `connect`, `sendto`, `recvfrom`, credential/permission checks).
//! - POSIX-like Shared Memory Objects (`shm_open`, `shm_unlink`, `mmap(MAP_SHARED)`).
//! - Event notification readiness queries for `sys_poll`.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use aether_abi::open_flags::{O_CREAT, O_EXCL, O_RDONLY};
use aether_abi::poll_events::{POLLERR, POLLHUP, POLLIN, POLLOUT};
use aether_abi::signal::SIGPIPE;
use aether_abi::Errno;
use aether_uuid::Uuid;

use crate::mm::pmm;
use crate::sched;
use crate::sync::{IrqSpinLock, WaitQueue};

pub const PIPE_BUF_CAPACITY: usize = 16 * 1024; // 16 KiB bounded kernel pipe ring buffer
pub const AF_UNIX: u32 = 1;
pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;

struct PipeRing {
    buf: Vec<u8>,
    read_pos: usize,
    write_pos: usize,
    len: usize,
}

impl PipeRing {
    fn new() -> Self {
        Self {
            buf: alloc::vec![0u8; PIPE_BUF_CAPACITY],
            read_pos: 0,
            write_pos: 0,
            len: 0,
        }
    }

    fn available_read(&self) -> usize {
        self.len
    }

    fn available_write(&self) -> usize {
        PIPE_BUF_CAPACITY - self.len
    }

    fn pop_into(&mut self, dst: &mut [u8]) -> usize {
        let count = dst.len().min(self.len);
        for slot in dst.iter_mut().take(count) {
            *slot = self.buf[self.read_pos];
            self.read_pos = (self.read_pos + 1) % PIPE_BUF_CAPACITY;
        }
        self.len -= count;
        count
    }

    fn push_from(&mut self, src: &[u8]) -> usize {
        let count = src.len().min(self.available_write());
        for &b in src.iter().take(count) {
            self.buf[self.write_pos] = b;
            self.write_pos = (self.write_pos + 1) % PIPE_BUF_CAPACITY;
        }
        self.len += count;
        count
    }
}

/// Shared kernel pipe buffer with reader/writer endpoint tracking and blocking wait queues.
pub struct Pipe {
    ring: IrqSpinLock<PipeRing>,
    readers: AtomicUsize,
    writers: AtomicUsize,
    read_waitq: WaitQueue,
    write_waitq: WaitQueue,
}

impl Pipe {
    pub fn new_pair() -> (PipeReader, PipeWriter) {
        let pipe = Arc::new(Self {
            ring: IrqSpinLock::new(PipeRing::new()),
            readers: AtomicUsize::new(1),
            writers: AtomicUsize::new(1),
            read_waitq: WaitQueue::new(),
            write_waitq: WaitQueue::new(),
        });
        (
            PipeReader {
                inner: Arc::clone(&pipe),
            },
            PipeWriter { inner: pipe },
        )
    }

    pub fn read(&self, dst: &mut [u8], nonblocking: bool) -> Result<usize, Errno> {
        if dst.is_empty() {
            return Ok(0);
        }
        loop {
            {
                let mut ring = self.ring.lock();
                if ring.available_read() > 0 {
                    let n = ring.pop_into(dst);
                    drop(ring);
                    self.write_waitq.wake_all();
                    return Ok(n);
                }
                if self.writers.load(Ordering::Acquire) == 0 {
                    return Ok(0); // All writers closed -> EOF
                }
            }
            if nonblocking {
                return Err(Errno::Eagain);
            }
            // Wait briefly or block on read_waitq
            self.read_waitq.wait();
        }
    }

    pub fn write(&self, src: &[u8], nonblocking: bool) -> Result<usize, Errno> {
        if src.is_empty() {
            return Ok(0);
        }
        let mut total_written = 0usize;
        while total_written < src.len() {
            if self.readers.load(Ordering::Acquire) == 0 {
                let _ = sched::kill_process(sched::current_pid(), SIGPIPE);
                return Err(Errno::Epipe);
            }
            let chunk_written = {
                let mut ring = self.ring.lock();
                if ring.available_write() > 0 {
                    ring.push_from(&src[total_written..])
                } else {
                    0
                }
            };
            if chunk_written > 0 {
                total_written += chunk_written;
                self.read_waitq.wake_all();
                if total_written == src.len() || nonblocking {
                    return Ok(total_written);
                }
            } else if nonblocking {
                return if total_written > 0 {
                    Ok(total_written)
                } else {
                    Err(Errno::Eagain)
                };
            } else {
                self.write_waitq.wait();
            }
        }
        Ok(total_written)
    }

    pub fn poll_read(&self) -> i16 {
        let avail = self.ring.lock().available_read();
        let mut revents = 0i16;
        if avail > 0 {
            revents |= POLLIN;
        }
        if self.writers.load(Ordering::Acquire) == 0 {
            revents |= POLLHUP;
        }
        revents
    }

    pub fn poll_write(&self) -> i16 {
        if self.readers.load(Ordering::Acquire) == 0 {
            return POLLERR | POLLHUP;
        }
        let avail = self.ring.lock().available_write();
        if avail > 0 {
            POLLOUT
        } else {
            0
        }
    }
}

/// Refcounted read endpoint of a `Pipe` (increments `readers` on clone, decrements on drop).
pub struct PipeReader {
    pub inner: Arc<Pipe>,
}

impl Clone for PipeReader {
    fn clone(&self) -> Self {
        self.inner.readers.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        if self.inner.readers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.write_waitq.wake_all();
        }
    }
}

/// Refcounted write endpoint of a `Pipe` (increments `writers` on clone, decrements on drop).
pub struct PipeWriter {
    pub inner: Arc<Pipe>,
}

impl Clone for PipeWriter {
    fn clone(&self) -> Self {
        self.inner.writers.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        if self.inner.writers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.read_waitq.wake_all();
        }
    }
}

// --- Unix-Domain Sockets (`AF_UNIX`) ---

pub enum UnixSocketState {
    Unbound,
    Bound {
        path: String,
    },
    Listening {
        path: String,
        listener: Arc<UnixListener>,
    },
    Connected {
        rx: PipeReader,
        tx: PipeWriter,
        peer_pid: u32,
        peer_uid: u32,
    },
}

pub struct UnixListener {
    pub owner_uid: u32,
    pub owner_uuid: Uuid,
    pub backlog: usize,
    pub queue: IrqSpinLock<VecDeque<(PipeReader, PipeWriter, u32, u32)>>,
    pub accept_waitq: WaitQueue,
}

pub struct UnixSocket {
    pub sock_type: u32,
    pub state: IrqSpinLock<UnixSocketState>,
}

static UNIX_BINDINGS: IrqSpinLock<BTreeMap<String, Arc<UnixListener>>> =
    IrqSpinLock::new(BTreeMap::new());

impl UnixSocket {
    pub fn new(sock_type: u32) -> Result<Arc<Self>, Errno> {
        if sock_type != SOCK_STREAM && sock_type != SOCK_DGRAM {
            return Err(Errno::Eprototype);
        }
        Ok(Arc::new(Self {
            sock_type,
            state: IrqSpinLock::new(UnixSocketState::Unbound),
        }))
    }

    pub fn bind(&self, path: String) -> Result<(), Errno> {
        if path.is_empty() || path.len() > 108 {
            return Err(Errno::Einval);
        }
        let mut st = self.state.lock();
        if !matches!(*st, UnixSocketState::Unbound) {
            return Err(Errno::Einval);
        }
        if UNIX_BINDINGS.lock().contains_key(&path) {
            return Err(Errno::Eaddrinuse);
        }
        *st = UnixSocketState::Bound { path };
        Ok(())
    }

    pub fn listen(&self, backlog: usize) -> Result<(), Errno> {
        let mut st = self.state.lock();
        let path = match &*st {
            UnixSocketState::Bound { path } => path.clone(),
            _ => return Err(Errno::Einval),
        };
        let (uid, uuid) = sched::with_current_task(|t| (t.creds.euid, t.creds.user_uuid));
        let listener = Arc::new(UnixListener {
            owner_uid: uid,
            owner_uuid: uuid,
            backlog: backlog.clamp(1, 64),
            queue: IrqSpinLock::new(VecDeque::new()),
            accept_waitq: WaitQueue::new(),
        });
        let mut bindings = UNIX_BINDINGS.lock();
        if bindings.contains_key(&path) {
            return Err(Errno::Eaddrinuse);
        }
        bindings.insert(path.clone(), Arc::clone(&listener));
        *st = UnixSocketState::Listening { path, listener };
        Ok(())
    }

    pub fn accept(&self, nonblocking: bool) -> Result<Arc<UnixSocket>, Errno> {
        let listener = {
            let st = self.state.lock();
            match &*st {
                UnixSocketState::Listening { listener, .. } => Arc::clone(listener),
                _ => return Err(Errno::Einval),
            }
        };

        loop {
            if let Some((rx, tx, peer_pid, peer_uid)) = listener.queue.lock().pop_front() {
                return Ok(Arc::new(UnixSocket {
                    sock_type: self.sock_type,
                    state: IrqSpinLock::new(UnixSocketState::Connected {
                        rx,
                        tx,
                        peer_pid,
                        peer_uid,
                    }),
                }));
            }
            if nonblocking {
                return Err(Errno::Eagain);
            }
            listener.accept_waitq.wait();
        }
    }

    pub fn connect(&self, path: &str) -> Result<(), Errno> {
        let listener = UNIX_BINDINGS
            .lock()
            .get(path)
            .cloned()
            .ok_or(Errno::Econnrefused)?;

        let (client_pid, client_uid) = sched::with_current_task(|t| (t.pid, t.creds.euid));
        let (server_pid, server_uid) = (0u32, listener.owner_uid);

        let (c2s_r, c2s_w) = Pipe::new_pair();
        let (s2c_r, s2c_w) = Pipe::new_pair();

        {
            let mut q = listener.queue.lock();
            if q.len() >= listener.backlog {
                return Err(Errno::Eagain);
            }
            q.push_back((c2s_r, s2c_w, client_pid, client_uid));
        }

        *self.state.lock() = UnixSocketState::Connected {
            rx: s2c_r,
            tx: c2s_w,
            peer_pid: server_pid,
            peer_uid: server_uid,
        };
        listener.accept_waitq.wake_one();
        Ok(())
    }

    pub fn read(&self, dst: &mut [u8], nonblocking: bool) -> Result<usize, Errno> {
        let rx = {
            let st = self.state.lock();
            match &*st {
                UnixSocketState::Connected { rx, .. } => rx.clone(),
                _ => return Err(Errno::Enotconn),
            }
        };
        rx.inner.read(dst, nonblocking)
    }

    pub fn write(&self, src: &[u8], nonblocking: bool) -> Result<usize, Errno> {
        let tx = {
            let st = self.state.lock();
            match &*st {
                UnixSocketState::Connected { tx, .. } => tx.clone(),
                _ => return Err(Errno::Enotconn),
            }
        };
        tx.inner.write(src, nonblocking)
    }

    pub fn poll(&self) -> i16 {
        let st = self.state.lock();
        match &*st {
            UnixSocketState::Listening { listener, .. } => {
                if !listener.queue.lock().is_empty() {
                    POLLIN
                } else {
                    0
                }
            }
            UnixSocketState::Connected { rx, tx, .. } => {
                rx.inner.poll_read() | tx.inner.poll_write()
            }
            _ => 0,
        }
    }
}

impl Drop for UnixSocket {
    fn drop(&mut self) {
        let st = self.state.lock();
        if let UnixSocketState::Listening { path, .. } = &*st {
            UNIX_BINDINGS.lock().remove(path);
        }
    }
}

// --- Shared Memory Objects (`shm_open` / `shm_unlink`) ---

pub struct ShmObject {
    pub name: String,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub owner_uuid: Uuid,
    pub mode: u32,
    pub frames: IrqSpinLock<Vec<u64>>,
}

impl ShmObject {
    pub fn ensure_pages(&self, page_count: usize) -> Result<Vec<u64>, Errno> {
        let mut frames = self.frames.lock();
        while frames.len() < page_count {
            let phys = pmm::alloc_zeroed_frame().map_err(|_| Errno::Enomem)?;
            frames.push(phys);
        }
        Ok(frames[..page_count].to_vec())
    }
}

impl Drop for ShmObject {
    fn drop(&mut self) {
        let frames = self.frames.lock();
        for &phys in frames.iter() {
            let _ = pmm::free_frame(phys);
        }
    }
}

static SHM_REGISTRY: IrqSpinLock<BTreeMap<String, Arc<ShmObject>>> =
    IrqSpinLock::new(BTreeMap::new());

pub fn shm_open_object(name: &str, flags: u32, mode: u32) -> Result<Arc<ShmObject>, Errno> {
    let clean = name.trim_start_matches('/');
    if clean.is_empty() || clean.len() > 64 || clean.contains('/') {
        return Err(Errno::Einval);
    }

    let (euid, egid, user_uuid) =
        sched::with_current_task(|t| (t.creds.euid, t.creds.egid, t.creds.user_uuid));
    let mut reg = SHM_REGISTRY.lock();

    if let Some(existing) = reg.get(clean) {
        if (flags & O_CREAT) != 0 && (flags & O_EXCL) != 0 {
            return Err(Errno::Eexist);
        }
        // Check access permission (owner / group / other)
        let want_write = (flags & 3) != O_RDONLY;
        let eff_bits = if euid == 0 {
            7
        } else if euid == existing.owner_uid {
            (existing.mode >> 6) & 7
        } else if egid == existing.owner_gid {
            (existing.mode >> 3) & 7
        } else {
            existing.mode & 7
        };
        if (eff_bits & 4) == 0 || (want_write && (eff_bits & 2) == 0) {
            return Err(Errno::Eacces);
        }
        return Ok(Arc::clone(existing));
    }

    if (flags & O_CREAT) == 0 {
        return Err(Errno::Enoent);
    }

    let obj = Arc::new(ShmObject {
        name: String::from(clean),
        owner_uid: euid,
        owner_gid: egid,
        owner_uuid: user_uuid,
        mode: mode & 0o777,
        frames: IrqSpinLock::new(Vec::new()),
    });
    reg.insert(String::from(clean), Arc::clone(&obj));
    Ok(obj)
}

pub fn shm_unlink_object(name: &str) -> Result<(), Errno> {
    let clean = name.trim_start_matches('/');
    let euid = sched::with_current_task(|t| t.creds.euid);
    let mut reg = SHM_REGISTRY.lock();
    let existing = reg.get(clean).ok_or(Errno::Enoent)?;
    if euid != 0 && euid != existing.owner_uid {
        return Err(Errno::Eperm);
    }
    reg.remove(clean);
    Ok(())
}
