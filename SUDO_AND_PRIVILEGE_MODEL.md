# AetherOS Privilege Model, Capabilities & Audited `sudo`

## 1. Fine-Grained Kernel Capability Bitmask (`kernel/src/sched/mod.rs`)

AetherOS partitions superuser authority into explicit 64-bit capability flags carried in each task's `Credentials` (`caps_effective`, `caps_permitted`, `caps_bounding`):

| Capability Constant | Bit Value | Governed Operations |
| :--- | :--- | :--- |
| `CAP_CHOWN` | `1 << 0` | Changing file ownership (`chown`, `chown_uuid`) |
| `CAP_DAC_OVERRIDE` | `1 << 1` | Bypassing read/write/execute DAC permissions |
| `CAP_FOWNER` | `1 << 2` | Bypassing ownership checks on `chmod`, sticky-bit `unlink`, and ACL updates |
| `CAP_KILL` | `1 << 3` | Sending signals (`sys_kill`) to processes owned by other UUIDs |
| `CAP_SETUID` | `1 << 4` | Arbitrary `setuid` transitions and creating users (`SYS_USERADD`) |
| `CAP_SETGID` | `1 << 5` | Arbitrary `setgid` and `setgroups` transitions |
| `CAP_NET_BIND_SERVICE` | `1 << 6` | Binding privileged network ports (`< 1024`) |
| `CAP_NET_RAW` | `1 << 7` | Constructing raw Ethernet/IPv4/ICMP packets |
| `CAP_SYS_ADMIN` | `1 << 8` | Mounting and unmounting filesystems (`sys_mount`, `sys_umount`) |
| `CAP_SYS_BOOT` | `1 << 9` | System reboot and shutdown control |
| `CAP_SETFCAP` | `1 << 10` | Assigning or clearing `execute_as_uuid` (`SYS_SET_EXEC_AS_UUID`) |
| `CAP_AUDIT_READ` | `1 << 11` | Reading the kernel security audit log (`SYS_AUDIT_LOG`) |

Processes can inspect their effective capabilities via `SYS_CAP_GET` and drop capabilities via `SYS_CAP_DROP(mask)`.

---

## 2. Kernel-Controlled `sudo` Architecture (`/bin/sudo`, `SYS_ELEVATE_SUDO`)

### Security Policy (`/etc/sudoers`)
```text
# AetherOS sudo policy (QEMU dev mode opt-in)
%wheel ALL=(ALL) NOPASSWD_DEV: ALL
```

### Elevation Enforcement (`security::sys_elevate_sudo_current`)
When `/bin/sudo <command> [args...]` executes in Ring 3:
1. `/bin/sudo` prints an explicit development-mode warning to stderr/console:
   ```text
   [sudo] WARNING: QEMU development passwordless mode active (caller_uid=1000, command='/bin/id')
   ```
2. `/bin/sudo` invokes `SYS_ELEVATE_SUDO(command)`.
3. The kernel verifies:
   - Development passwordless sudo mode (`sudo_dev_mode_enabled`) is active.
   - The calling task does **not** have `no_new_privs` set.
   - The calling task is either `root` (`euid == 0`) or a member of the `wheel` group (`GID 10` in `creds.groups`).
4. If any check fails, the kernel logs `[AUDIT] action=SUDO_DENIED` and returns `-EPERM`.
5. If authorized, the kernel elevates the task's credentials to `root` (`UID 0`, `UUID 00000000-0000-4000-8000-000000000001`), logs `[AUDIT] action=SUDO_DEV_ELEVATE_TO_ROOT`, and `/bin/sudo` immediately executes `sys_execve(command, argv, envp)`.
