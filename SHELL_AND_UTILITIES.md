# AetherOS Native Shell (`aethersh`) & Core User-Space Utilities

## 1. `/bin/aethersh` Architecture (`userspace/bins/src/bin/aethersh.rs`)

`/bin/aethersh` is AetherOS's native Ring 3 command shell compiled as a freestanding ELF64 binary linking against `libaether`.

### Supported Shell Grammar & Execution Features
1. **Multi-Stage Pipelines (`cmd1 | cmd2 | cmd3`)**:
   - `execute_line` splits command lines on unquoted `|` into `N` pipeline stages.
   - Allocates `N - 1` kernel anonymous pipes (`sys_pipe`), forks `N` child processes, wires `dup2(prev_read_fd, 0)` and `dup2(next_write_fd, 1)` inside each child, closes unused pipe descriptors in both children and parent, and waits for all `N` pipeline stages in order (`wait4`).
2. **Input & Output Redirection (`>`, `>>`, `<`)**:
   - `>` opens target file with `O_CREAT | O_WRONLY | O_TRUNC` (mode `0644`) and redirects `stdout` (`fd 1`).
   - `>>` opens target file with `O_CREAT | O_WRONLY | O_APPEND` (mode `0644`) and redirects `stdout` (`fd 1`).
   - `<` opens source file with `O_RDONLY` and redirects `stdin` (`fd 0`).
3. **Quoting, Escaping & Environment Variable Expansion (`$VAR`)**:
   - Single quotes (`'...'`) preserve literal characters.
   - Double quotes (`"..."`) and unquoted tokens expand `$VAR` (including `$?` for the last command exit code) from the shell's environment table (`PATH`, `HOME`, `USER`, `TERM`, `SHELL`).
4. **Built-in Commands**:
   - `cd [dir]`, `pwd`, `echo [-n] [args...]`, `export KEY=VALUE`, `unset KEY`, `env`, `history`, `help`, `clear`, `exit [code]`.
5. **PATH Lookup**:
   - Commands without a `/` are resolved across `PATH` (`/bin:/sbin:/usr/bin`) via `sys_stat` before invoking `execve`.

---

## 2. Core User-Space Utilities (`userspace/bins/src/bin/coreutils.rs`)

All 28 standard utilities are real compiled Ring 3 ELF64 binaries installed in `/bin/` inside `initramfs.tar` and dispatched via `argv[0]`:

| Utility | Path | Functionality |
| :--- | :--- | :--- |
| `ls` | `/bin/ls` | Lists directory entries via `SYS_GETDENTS` & `SYS_STAT` (`ls -l` shows mode string, UID, GID, size, name) |
| `cat` | `/bin/cat` | Concatenates files or `stdin` to `stdout` |
| `grep` | `/bin/grep` | Filters lines matching pattern (supports `-v` invert match) from files or `stdin` pipeline |
| `wc` | `/bin/wc` | Counts lines (`-l`), words (`-w`), and bytes (`-c`) from files or `stdin` pipeline |
| `ps` | `/bin/ps` | Queries kernel process table via `SYS_GETPROCS` (`PID`, `PPID`, `UID`, `STATE`, `MEM_KB`, `TICKS`, `NAME`) |
| `kill` | `/bin/kill` | Sends signal (`kill [-SIGNAL] <pid>`) via `SYS_KILL` |
| `id` | `/bin/id` | Prints real/effective `uid`, `gid`, 128-bit `user_uuid`, and 128-bit `group_uuid` |
| `whoami` | `/bin/whoami` | Prints current username (`root`, `aether`, or `uid-<N>`) |
| `groups` | `/bin/groups` | Lists supplementary group memberships (`SYS_GETGROUPS`) |
| `stat` | `/bin/stat` | Displays full inode metadata (`Inode`, `Mode`, `UID/GID`, `Size`, `Links`, `ObjectUUID`, `OwnerUUID`, `GroupUUID`, `ExecAsUUID`) |
| `chmod` | `/bin/chmod` | Updates octal file permissions (`chmod <octal> <path>`) via `SYS_CHMOD` |
| `chown` | `/bin/chown` | Updates file ownership (`chown <uid>[:<gid>] <path>`) via `SYS_CHOWN` |
| `mkdir` | `/bin/mkdir` | Creates directories via `SYS_MKDIR` |
| `rmdir` | `/bin/rmdir` | Removes empty directories via `SYS_RMDIR` |
| `touch` | `/bin/touch` | Creates empty files if absent (`O_CREAT | O_WRONLY`) |
| `rm` | `/bin/rm` | Unlinks files via `SYS_UNLINK` |
| `cp` | `/bin/cp` | Copies file contents and mode bits (`cp <src> <dst>`) |
| `mv` | `/bin/mv` | Renames/moves filesystem objects via `SYS_RENAME` |
| `uname` | `/bin/uname` | Queries `SYS_UNAME` (`uname -a` prints sysname, nodename, release, version, machine) |
| `uptime` | `/bin/uptime` | Queries `SYS_SYSINFO` and prints uptime, process count, and free RAM |
| `free` | `/bin/free` | Displays physical RAM and kernel heap utilization in KiB (`SYS_SYSINFO`) |
| `df` | `/bin/df` | Queries `SYS_STATFS` across `/` (`ramfs`) and `/mnt/aether` (`aetherfs`) |
| `dmesg` | `/bin/dmesg` | Reads structured kernel ring buffer logs via `SYS_DMESG` |
| `mount` | `/bin/mount` | Lists `/proc/mounts` or mounts a filesystem via `SYS_MOUNT` |
| `umount` | `/bin/umount` | Unmounts a filesystem via `SYS_UMOUNT` |
| `useradd` | `/bin/useradd` | Creates a new user account with a 128-bit UUID via `SYS_USERADD` |
| `true` / `false` | `/bin/true`, `/bin/false` | Exits with status `0` or `1` |
