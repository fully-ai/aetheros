# AetherOS UUID-Based User & Group Identity Architecture

## 1. First-Class 128-Bit UUID Identity Model (`crates/aether-uuid/`, `kernel/src/security/mod.rs`)

Unlike traditional UNIX systems where identity is strictly a 32-bit integer (`uid`/`gid`), AetherOS identifies every user account, group, and filesystem inode by an RFC 4122 128-bit UUID v4 (`aether_uuid::Uuid`), while maintaining 32-bit `uid` and `gid` integer mappings for POSIX utility compatibility.

### Well-Known System Identity Table

| Principal | Kind | UID / GID | 128-Bit Canonical UUID | Membership / Role |
| :--- | :--- | :--- | :--- | :--- |
| `root` | User | `UID 0` | `00000000-0000-4000-8000-000000000001` | System superuser (`CAP_ALL`) |
| `root` | Group | `GID 0` | `00000000-0000-4000-8000-000000000000` | Primary superuser group |
| `wheel` | Group | `GID 10` | `00000000-0000-4000-8000-00000000000a` | Administrative elevation group (`root`, `aether`) |
| `aether` | User | `UID 1000` | `a37eb089-6126-44a4-94cb-02bf71a303e8` | Default unprivileged interactive user |
| `aether` | Group | `GID 1000` | `525c03f7-f7c7-4821-83fd-9d0d658a4ef3` | Primary group for user `aether` |

---

## 2. Process Credential Set (`sched::Credentials`)

Every running process carries an immutable-by-default `Credentials` structure in its kernel `Task`:
- **Real, Effective, and Saved IDs**: `uid`, `euid`, `suid` and `gid`, `egid`, `sgid`.
- **UUID Principals**: `user_uuid: Uuid` and `group_uuid: Uuid`.
- **Supplementary Groups**: `groups: Vec<u32>`.
- **Capability Masks**: `caps_effective`, `caps_permitted`, `caps_bounding`.
- **Privilege Lock**: `no_new_privs: bool` (once set via `SYS_SET_NO_NEW_PRIVS`, neither the process nor any of its `fork`/`execve` descendants can ever gain privileges via `execute_as_uuid`, `S_ISUID`, or `sudo`).

---

## 3. Persistent Identity Database (`/etc/passwd`, `/etc/group`) & `SYS_USERADD`

- `/etc/passwd` and `/etc/group` include the 128-bit UUID field alongside the standard colon-delimited fields:
  ```text
  root:x:0:0:00000000-0000-4000-8000-000000000001:System Root:/root:/bin/aethersh
  aether:x:1000:1000:a37eb089-6126-44a4-94cb-02bf71a303e8:Aether User:/home/aether:/bin/aethersh
  ```
- New user accounts created at runtime via `SYS_USERADD` (`/bin/useradd <username> [uid] [gid]`) are assigned a deterministic RFC 4122 UUID v4, registered in the kernel's `IDENTITY_DB`, appended to `/etc/passwd` and `/etc/group`, and recorded in the kernel security audit log (`[AUDIT] USERADD`).
