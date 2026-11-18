# AetherOS Filesystem Permissions, UUID ACLs & Execution Identity Policy

## 1. Permission Evaluation Order (`fs::check_node_permission`)

Every VFS and AetherFS operation (`open`, `read`, `write`, `execve`, `mkdir`, `unlink`, `chmod`, `chown`) evaluates access permissions in strict order:

1. **Mount & Immutable Guards**:
   - Write access (`want_w`) on a read-only mount (`MS_RDONLY`) returns `-EROFS`.
   - Write or delete access on an inode marked `INODE_FLAG_IMMUTABLE` returns `-EPERM` (even for `root`).
   - Execute access (`want_x`) on a regular file under a `MS_NOEXEC` mount returns `-EACCES`.
2. **Superuser / `CAP_DAC_OVERRIDE`**:
   - If `creds.euid == 0` or `CAP_DAC_OVERRIDE` is present in `creds.caps_effective`, read/write DAC checks succeed; execute checks succeed for directories or files with at least one execute bit (`mode & 0o111 != 0`).
3. **Explicit UUID-Based Access Control Lists (`node.acl`)**:
   - Each `AclEntry` (`24 bytes`) specifies `subject_uuid: [u8; 16]`, `is_group: u8`, `allow_rwx: u8`, and `deny_rwx: u8`.
   - **Deny Precedence**: All matching user and group ACL entries are accumulated. If `(acl_deny & want_rwx) != 0`, access is immediately denied with `-EACCES`.
   - If an explicit ACL entry matches the caller's `user_uuid` or `group_uuid` and `(acl_allow & want_rwx) == want_rwx`, access is granted.
4. **Owner UUID / Group UUID / Other Mode Bits**:
   - If `creds.user_uuid == node.owner_user_uuid` (or `creds.euid == node.uid`), bits `(mode >> 6) & 7` apply.
   - Else if `creds.group_uuid == node.owner_group_uuid` (or `creds.egid == node.gid` or `creds.groups.contains(&node.gid)`), bits `(mode >> 3) & 7` apply.
   - Else world/other bits `mode & 7` apply.

---

## 2. Sticky Bit Enforcement (`/tmp`, Mode `01777`)

When a directory has `S_ISVTX` (`0o1000`, e.g., `/tmp` with mode `01777`), `vfs_unlink` and `vfs_rmdir` enforce that an unprivileged user can only delete or rename an entry inside that directory if:
- The caller is `root` (`euid == 0`), OR
- The caller owns the target file (`creds.user_uuid == target.owner_user_uuid`), OR
- The caller owns the parent directory.

Attempts by UID `1000` (`aether`) to delete root-owned files in `/tmp` fail with `-EPERM` (verified in `test_multiuser_permissions_acl_and_execute_as`).

---

## 3. Root-Controlled `execute_as_uuid` Execution Identity Policy

AetherOS replaces ad-hoc setuid pitfalls with a strictly validated `execute_as_uuid` inode metadata policy (`security::compute_exec_credentials`):

1. **Assignment Restriction**: Only `root` (`euid == 0` or `CAP_SETFCAP`) can set or clear `execute_as_uuid` via `SYS_SET_EXEC_AS_UUID`. Any attempt by an unprivileged user is rejected with `-EPERM` and logged (`DENIED_SET_EXECUTE_AS`).
2. **Modification Clearing**: Whenever a file's contents are modified (`O_TRUNC` or `vfs_write_file`) or its ownership changes (`chown` / `chown_uuid`), the kernel automatically clears `INODE_FLAG_EXECUTE_AS`, resets `execute_as_uuid = Uuid::NIL`, and strips `S_ISUID` / `S_ISGID`.
3. **Mount & Process Restrictions**: If the filesystem is mounted `MS_NOSUID` or the calling process has `no_new_privs == true`, any `execute_as_uuid` or `S_ISUID` transition is blocked (`-EPERM`, audit action `BLOCKED_EXEC_ELEVATION_NOSUID_OR_NNP`).
4. **No Silent Root Fallback**: On `sys_execve`, `execute_as_uuid` is looked up in `IDENTITY_DB` (`lookup_user_by_uuid`). If the target UUID does not exist or has been deactivated, execution fails closed with `-EACCES`.
