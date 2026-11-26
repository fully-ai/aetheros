# AetherOS User & Group Identity Specification

See also [USER_GROUP_UUID_SECURITY.md](USER_GROUP_UUID_SECURITY.md).

## 1. Dual UUID + POSIX Identity Mapping

Every security principal in AetherOS is uniquely identified by an RFC 4122 128-bit UUID (`aether_uuid::Uuid`) stored inside the kernel's `IDENTITY_DB` (`kernel/src/security/mod.rs`) and persisted in `/etc/passwd`, `/etc/group`, and AetherFS 256-byte inodes (`owner_user_uuid`, `owner_group_uuid`, `execute_as_uuid`).

| Principal | UID / GID | 128-Bit UUID | Home / Shell |
| :--- | :--- | :--- | :--- |
| `root` (user) | `0` | `00000000-0000-4000-8000-000000000001` | `/root` (`/bin/aethersh`) |
| `root` (group) | `0` | `00000000-0000-4000-8000-000000000000` | Superuser group |
| `wheel` (group) | `10` | `00000000-0000-4000-8000-00000000000a` | Administrative elevation group |
| `aether` (user) | `1000` | `a37eb089-6126-44a4-94cb-02bf71a303e8` | `/home/aether` (`/bin/aethersh`) |
| `aether` (group) | `1000` | `525c03f7-f7c7-4821-83fd-9d0d658a4ef3` | Default user group |

## 2. Identity Syscalls & User-Space Tools
- **Syscalls**: `SYS_GETUID (150)`, `SYS_GETGID (151)`, `SYS_SETUID (152)`, `SYS_SETGID (153)`, `SYS_GETRESUID (154)`, `SYS_SETRESUID (155)`, `SYS_GETRESGID (156)`, `SYS_SETRESGID (157)`, `SYS_GETGROUPS (158)`, `SYS_SETGROUPS (159)`, `SYS_GET_USER_UUID (162)`, `SYS_GET_GROUP_UUID (163)`, `SYS_IDENTITY_CTL (165)`.
- **Utilities**: `/bin/id`, `/bin/whoami`, `/bin/groups`, `/bin/useradd`, `/bin/chown`, `/bin/chgrp`, `/bin/getfacl`, `/bin/setfacl`, `/bin/sudo`.
