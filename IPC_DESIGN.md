# AetherOS Inter-Process Communication (IPC) Design

## 1. Anonymous Pipes & Named FIFOs (`kernel/src/ipc/mod.rs`)

### Architecture
- Backed by `PipeChannel`, an interrupt-safe ring buffer (`PIPE_BUF_SIZE = 16384` bytes) with atomic reader and writer endpoint reference counts (`readers: usize`, `writers: usize`).
- **Lifecycle & Fork Semantics**:
  - `sys_pipe` / `sys_pipe2` allocate a `PipeChannel` and install two file descriptors (`fd[0]` read endpoint, `fd[1]` write endpoint).
  - `fork()` and `dup()`/`dup2()` call `clone_fd_resource`, incrementing `readers` or `writers` on the underlying `Arc<PipeChannel>`.
  - `close()` and `exit()` call `close_fd_resource`, decrementing `readers` or `writers`.
- **Blocking, EOF, and Broken Pipe (`EPIPE` / `SIGPIPE`)**:
  - **Read**: If the buffer is empty and `writers == 0`, `read()` immediately returns `0` (`EOF`). If `writers > 0` and `!nonblock`, the caller yields (`sleep_ms(1)`) until bytes arrive or all writers close.
  - **Write**: If `readers == 0`, `write()` delivers `SIGPIPE` to the calling task and returns `Err(Errno::Epipe)`.

---

## 2. Unix-Domain Local Sockets (`AF_UNIX`, `SOCK_STREAM` & `SOCK_DGRAM`)

- Created via `sys_socket(AF_UNIX, SOCK_STREAM | SOCK_DGRAM, 0)`.
- **Path Binding & Connection**:
  - `sys_bind(fd, path, len)` registers the socket endpoint in the kernel's `UNIX_REGISTRY` (`BTreeMap<String, Arc<UnixSocket>>`).
  - `sys_listen(fd, backlog)` transitions a bound stream socket into `UnixSocketState::Listening`.
  - `sys_connect(client_fd, path, len)` pairs `client_fd` with the listening server by allocating a full-duplex pair of `PipeChannel` rings (`c2s` client-to-server and `s2c` server-to-client) and queuing the server peer for `sys_accept`.
  - For `SOCK_DGRAM` sockets, `sys_send` and `sys_recv` exchange discrete length-delimited datagrams over the bound endpoint queue.

---

## 3. Shared Memory Objects (`shm_open` / `shm_unlink` + `mmap`)

- `sys_shm_open(name, flags, mode)` creates or opens a named `ShmObject` in `SHM_REGISTRY`, backed by physical 4 KiB frames allocated from the PMM and reference-counted via `pmm::inc_frame_ref`.
- When a process calls `sys_mmap(0, len, prot, MAP_SHARED, shm_fd, 0)`, the kernel maps the `ShmObject`'s physical frames directly into the caller's user `AddressSpace` (`0x0000_4000_0000_0000+`), allowing zero-copy shared memory communication across processes.

---

## 4. Signals & I/O Multiplexing (`poll`)

- **Supported Signals**: `SIGINT (2)`, `SIGKILL (9)`, `SIGSEGV (11)`, `SIGPIPE (13)`, `SIGTERM (15)`, `SIGCHLD (17)`.
- `sys_kill(pid, sig)` sets `(1 << sig)` in the target task's `pending_signals` bitmask (or immediately transitions the target to `TaskState::Zombie` with `exit_code = 128 + sig` for `SIGKILL`/`SIGTERM`).
- `sys_poll(fds_ptr, nfds, timeout_ms)` inspects readiness (`POLLIN`, `POLLOUT`, `POLLHUP`, `POLLERR`) across pipes, sockets, character devices, and regular files.
