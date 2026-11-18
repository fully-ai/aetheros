# AetherOS System Call Specification (ABI v1.0)

## 1. Calling Convention & Privilege Transition (`kernel/src/arch/x86_64/syscall_entry.rs`)

AetherOS defines a versioned native 64-bit System Call ABI (`ABI_VERSION_MAJOR = 1`, `ABI_VERSION_MINOR = 0`) shared between `crates/aether-abi`, the kernel dispatcher (`kernel/src/syscall/mod.rs`), and `userspace/libaether`.

### Hardware Entry Mechanism
- **Instruction**: `SYSCALL` (configured via `IA32_STAR`, `IA32_LSTAR = syscall_entry_stub`, `IA32_FMASK = 0x200`).
- **Register Assignment**:
  - `RAX`: Syscall number (`SyscallNumber`) on entry; signed 64-bit return value (`>= 0` on success, `-Errno` in `-4095..-1` on failure) on return.
  - `RDI`: Argument 1 (`a0`)
  - `RSI`: Argument 2 (`a1`)
  - `RDX`: Argument 3 (`a2`)
  - `R10`: Argument 4 (`a3`) — replaces `RCX`, which the `SYSCALL` instruction overwrites with user `RIP`.
  - `R8`: Argument 5 (`a4`)
  - `R9`: Argument 6 (`a5`)
  - `RCX`, `R11`: Clobbered by `SYSCALL` (saved user `RIP` and `RFLAGS`). All other general-purpose registers (`RBX`, `RSP`, `RBP`, `R12..R15`) are preserved across the system call.

### User Pointer Validation (`kernel/src/mm/user_access.rs`)
Every system call accepting a user-space pointer validates the entire `[ptr, ptr + len)` span via `validate_user_range` / `copy_from_user` / `copy_to_user` / `copy_cstr_from_user` before dereferencing:
1. Rejects null pointers (`ptr == 0`) and integer wrap-around (`ptr.checked_add(len)`).
2. Enforces strict canonical lower-half bounds (`end <= 0x0000_8000_0000_0000`). Any pointer targeting kernel memory (`>= 0xffff800000000000`) fails immediately with `-EFAULT` (`-14`).
3. Walks the active user `AddressSpace` page tables for every 4 KiB page in `[ptr, ptr + len)`, transparently resolving pending Demand Paging (`PTE_DEMAND`) or Copy-on-Write (`PTE_COW`) faults and verifying `PTE_USER` (and `PTE_WRITABLE` for destination buffers).

---

## 2. Complete System Call Table & Implementation Status

Per Project Rule 0.2, AetherOS maintains an explicit `SyscallStatus` registry in `kernel/src/syscall/mod.rs`. No system call returns fake success; any unimplemented syscall number returns `-ENOSYS` (`-38`).

| Number | Name | Category | Status | Signature / Description |
| :--- | :--- | :--- | :--- | :--- |
| `1` | `SYS_READ` | VFS / I/O | `Implemented` | `read(fd, buf_ptr, len) -> isize` |
| `2` | `SYS_WRITE` | VFS / I/O | `Implemented` | `write(fd, buf_ptr, len) -> isize` |
| `3` | `SYS_OPEN` | VFS / I/O | `Implemented` | `open(path_ptr, flags, mode) -> fd` |
| `4` | `SYS_CLOSE` | VFS / I/O | `Implemented` | `close(fd) -> 0` |
| `5` | `SYS_STAT` | VFS / Meta | `Implemented` | `stat(path_ptr, abi_stat_ptr) -> 0` |
| `6` | `SYS_FSTAT` | VFS / Meta | `Implemented` | `fstat(fd, abi_stat_ptr) -> 0` |
| `7` | `SYS_LSEEK` | VFS / I/O | `Implemented` | `lseek(fd, offset, whence) -> new_offset` |
| `8` | `SYS_GETDENTS` | VFS / Dir | `Implemented` | `getdents(fd, dirent_buf_ptr, len) -> bytes_written` |
| `9` | `SYS_MKDIR` | VFS / Dir | `Implemented` | `mkdir(path_ptr, mode) -> 0` |
| `10` | `SYS_RMDIR` | VFS / Dir | `Implemented` | `rmdir(path_ptr) -> 0` |
| `11` | `SYS_UNLINK` | VFS / Dir | `Implemented` | `unlink(path_ptr) -> 0` (enforces `/tmp` sticky bit) |
| `12` | `SYS_RENAME` | VFS / Dir | `Implemented` | `rename(old_ptr, new_ptr) -> 0` |
| `13` | `SYS_LINK` | VFS / Dir | `Implemented` | `link(old_ptr, new_ptr) -> 0` |
| `14` | `SYS_SYMLINK` | VFS / Dir | `Implemented` | `symlink(target_ptr, linkpath_ptr) -> 0` |
| `15` | `SYS_READLINK` | VFS / Dir | `Implemented` | `readlink(path_ptr, buf_ptr, buflen) -> len` |
| `16` | `SYS_CHMOD` | Security | `Implemented` | `chmod(path_ptr, mode) -> 0` |
| `17` | `SYS_CHOWN` | Security | `Implemented` | `chown(path_ptr, uid, gid) -> 0` |
| `18` | `SYS_CHOWN_UUID` | Security | `Implemented` | `chown_uuid(path_ptr, user_uuid_ptr, group_uuid_ptr) -> 0` |
| `19` | `SYS_SET_EXEC_AS_UUID` | Security | `Implemented` | `set_exec_as_uuid(path_ptr, target_uuid_ptr) -> 0` (Root/CAP_SETFCAP only) |
| `20` | `SYS_GET_ACL` | Security | `Implemented` | `get_acl(path_ptr, buf_ptr, max_entries) -> count` |
| `21` | `SYS_SET_ACL` | Security | `Implemented` | `set_acl(path_ptr, buf_ptr, entry_count) -> 0` |
| `22` | `SYS_CHDIR` | Process | `Implemented` | `chdir(path_ptr) -> 0` |
| `23` | `SYS_GETCWD` | Process | `Implemented` | `getcwd(buf_ptr, len) -> len` |
| `24` | `SYS_DUP` | VFS / FD | `Implemented` | `dup(old_fd) -> new_fd` |
| `25` | `SYS_DUP2` | VFS / FD | `Implemented` | `dup2(old_fd, new_fd) -> new_fd` |
| `26` | `SYS_PIPE` | IPC | `Implemented` | `pipe(fds_u32_pair_ptr) -> 0` |
| `27` | `SYS_PIPE2` | IPC | `Implemented` | `pipe2(fds_u32_pair_ptr, flags) -> 0` |
| `28` | `SYS_MKFIFO` | IPC | `Implemented` | `mkfifo(path_ptr, mode) -> 0` |
| `29` | `SYS_POLL` | IPC / I/O | `Implemented` | `poll(pollfds_ptr, nfds, timeout_ms) -> ready_count` |
| `30` | `SYS_IOCTL` | Device | `Implemented` | `ioctl(fd, request, arg) -> 0` |
| `31` | `SYS_MOUNT` | VFS | `Implemented` | `mount(src_ptr, target_ptr, fstype_ptr, flags) -> 0` |
| `32` | `SYS_UMOUNT` | VFS | `Implemented` | `umount(target_ptr) -> 0` |
| `33` | `SYS_STATFS` | VFS | `Implemented` | `statfs(path_ptr, abi_statfs_ptr) -> 0` |
| `34` | `SYS_FSYNC` | VFS | `Implemented` | `fsync(fd) -> 0` |
| `40` | `SYS_MMAP` | Memory | `Implemented` | `mmap(addr, len, prot, flags, fd, offset) -> vaddr` |
| `41` | `SYS_MUNMAP` | Memory | `Implemented` | `munmap(addr, len) -> 0` |
| `42` | `SYS_MPROTECT` | Memory | `Implemented` | `mprotect(addr, len, prot) -> 0` (enforces `W^X`) |
| `43` | `SYS_BRK` | Memory | `Implemented` | `brk(new_brk) -> current_brk` |
| `44` | `SYS_SHM_OPEN` | IPC / Memory | `Implemented` | `shm_open(name_ptr, flags, mode) -> fd` |
| `45` | `SYS_SHM_UNLINK` | IPC / Memory | `Implemented` | `shm_unlink(name_ptr) -> 0` |
| `50` | `SYS_SOCKET` | IPC / Net | `Implemented` | `socket(domain, type, protocol) -> fd` (`AF_UNIX`) |
| `51` | `SYS_BIND` | IPC / Net | `Implemented` | `bind(fd, path_ptr, len) -> 0` |
| `52` | `SYS_LISTEN` | IPC / Net | `Implemented` | `listen(fd, backlog) -> 0` |
| `53` | `SYS_ACCEPT` | IPC / Net | `Implemented` | `accept(fd, addr_ptr, len_ptr) -> client_fd` |
| `54` | `SYS_CONNECT` | IPC / Net | `Implemented` | `connect(fd, path_ptr, len) -> 0` |
| `55` | `SYS_SEND` | IPC / Net | `Implemented` | `send(fd, buf_ptr, len, flags) -> bytes_sent` |
| `56` | `SYS_RECV` | IPC / Net | `Implemented` | `recv(fd, buf_ptr, len, flags) -> bytes_read` |
| `60` | `SYS_GETPID` | Process | `Implemented` | `getpid() -> pid` |
| `61` | `SYS_GETPPID` | Process | `Implemented` | `getppid() -> ppid` |
| `62` | `SYS_GETTID` | Process | `Implemented` | `gettid() -> tid` |
| `63` | `SYS_YIELD` | Scheduler | `Implemented` | `yield() -> 0` |
| `64` | `SYS_SLEEP_MS` | Scheduler | `Implemented` | `sleep_ms(ms) -> 0` |
| `65` | `SYS_CLOCK_GETTIME` | Time | `Implemented` | `clock_gettime(clock_id, timespec_ptr) -> 0` |
| `66` | `SYS_UNAME` | System | `Implemented` | `uname(utsname_ptr) -> 0` |
| `67` | `SYS_SYSINFO` | System | `Implemented` | `sysinfo(sysinfo_ptr) -> 0` |
| `68` | `SYS_DMESG` | System | `Implemented` | `dmesg(buf_ptr, len) -> bytes_copied` |
| `69` | `SYS_ABI_VERSION` | System | `Implemented` | `abi_version(out_ptr) -> 0x0001_0000` |
| `70` | `SYS_EXIT` | Process | `Implemented` | `exit(status) -> !` |
| `71` | `SYS_FORK` | Process | `Implemented` | `fork() -> child_pid (parent) / 0 (child)` |
| `72` | `SYS_EXECVE` | Process | `Implemented` | `execve(path_ptr, argv_ptr, envp_ptr) -> !` |
| `73` | `SYS_WAIT4` | Process | `Implemented` | `wait4(pid, status_i32_ptr, options, rusage_ptr) -> pid` |
| `74` | `SYS_KILL` | Signals | `Implemented` | `kill(pid, sig) -> 0` |
| `75` | `SYS_SIGACTION` | Signals | `Implemented` | `sigaction(sig, act_ptr, oldact_ptr) -> 0` |
| `76` | `SYS_SIGRETURN` | Signals | `Implemented` | `sigreturn() -> 0` |
| `80` | `SYS_GETUID` | Identity | `Implemented` | `getuid() -> uid` |
| `81` | `SYS_GETEUID` | Identity | `Implemented` | `geteuid() -> euid` |
| `82` | `SYS_GETGID` | Identity | `Implemented` | `getgid() -> gid` |
| `83` | `SYS_GETEGID` | Identity | `Implemented` | `getegid() -> egid` |
| `84` | `SYS_SETUID` | Identity | `Implemented` | `setuid(uid) -> 0` |
| `85` | `SYS_SETGID` | Identity | `Implemented` | `setgid(gid) -> 0` |
| `86` | `SYS_GETGROUPS` | Identity | `Implemented` | `getgroups(max, list_u32_ptr) -> count` |
| `87` | `SYS_SETGROUPS` | Identity | `Implemented` | `setgroups(count, list_u32_ptr) -> 0` |
| `88` | `SYS_GET_USER_UUID` | Identity | `Implemented` | `get_user_uuid(uuid16_ptr) -> 0` |
| `89` | `SYS_GET_GROUP_UUID` | Identity | `Implemented` | `get_group_uuid(uuid16_ptr) -> 0` |
| `90` | `SYS_SET_NO_NEW_PRIVS` | Security | `Implemented` | `set_no_new_privs() -> 0` (irreversible) |
| `91` | `SYS_CAP_GET` | Security | `Implemented` | `cap_get() -> effective_cap_mask` |
| `92` | `SYS_CAP_DROP` | Security | `Implemented` | `cap_drop(mask_to_drop) -> remaining_cap_mask` |
| `93` | `SYS_USERADD` | Identity | `Implemented` | `useradd(uid, gid, name_ptr, out_uuid16_ptr) -> 0` |
| `94` | `SYS_ELEVATE_SUDO` | Security | `Implemented` | `elevate_sudo(reason_ptr) -> 0` (audited wheel dev elevation) |
| `95` | `SYS_AUDIT_LOG` | Security | `Implemented` | `audit_log(buf_ptr, len) -> bytes_copied` |
| `96` | `SYS_GETPROCS` | Process | `Implemented` | `getprocs(proc_info_buf_ptr, max_entries) -> count` |
