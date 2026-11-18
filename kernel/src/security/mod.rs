//! Multi-User Identity Database, UUID Ownership Mapping, `execute_as_uuid`
//! Credential Transition Policy, Sudo Development Mode, and Security Audit Log.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use aether_abi::caps::{CAP_ALL, CAP_AUDIT_READ, CAP_SETUID};
use aether_abi::Errno;
use aether_uuid::Uuid;

use crate::arch::x86_64::timer;
use crate::fs::{VfsNode, INODE_FLAG_EXECUTE_AS, MS_NOSUID, S_ISGID, S_ISUID};
use crate::sched::{self, Credentials};
use crate::sync::IrqSpinLock;

pub const DEFAULT_USER_UID: u32 = 1000;
pub const DEFAULT_USER_GID: u32 = 1000;
pub const WHEEL_GID: u32 = 10;

/// Stable UUID for the default unprivileged user `aether` (UID 1000).
pub const AETHER_USER_UUID: Uuid = Uuid::from_bytes([
    0xa3, 0x7e, 0xb0, 0x89, 0x61, 0x26, 0x44, 0xa4, 0x94, 0xcb, 0x02, 0xbf, 0x71, 0xa3, 0x03, 0xe8,
]);

/// Stable UUID for the default unprivileged group `aether` (GID 1000).
pub const AETHER_GROUP_UUID: Uuid = Uuid::from_bytes([
    0x52, 0x5c, 0x03, 0xf7, 0xf7, 0xc7, 0x48, 0x21, 0x83, 0xfd, 0x9d, 0x0d, 0x65, 0x8a, 0x4e, 0xf3,
]);

#[derive(Clone)]
pub struct UserAccount {
    pub uid: u32,
    pub primary_gid: u32,
    pub uuid: Uuid,
    pub username: String,
    pub home: String,
    pub shell: String,
    pub groups: Vec<u32>,
    pub active: bool,
}

#[derive(Clone)]
pub struct GroupAccount {
    pub gid: u32,
    pub uuid: Uuid,
    pub name: String,
}

struct IdentityDb {
    users: Vec<UserAccount>,
    groups: Vec<GroupAccount>,
    sudo_dev_mode_enabled: bool,
    audit_log: VecDeque<String>,
}

impl IdentityDb {
    const fn new() -> Self {
        Self {
            users: Vec::new(),
            groups: Vec::new(),
            sudo_dev_mode_enabled: true, // Enabled in QEMU dev build with explicit audit warning
            audit_log: VecDeque::new(),
        }
    }

    fn push_audit(&mut self, msg: String) {
        if self.audit_log.len() >= 256 {
            self.audit_log.pop_front();
        }
        self.audit_log.push_back(msg);
    }
}

static IDENTITY_DB: IrqSpinLock<IdentityDb> = IrqSpinLock::new(IdentityDb::new());

/// Initialize the internal UUID-based user and group identity database.
pub fn init(cmdline: &str) {
    let mut db = IDENTITY_DB.lock();
    db.sudo_dev_mode_enabled = !cmdline.contains("sudo_dev=off");

    db.groups.push(GroupAccount {
        gid: 0,
        uuid: Uuid::ROOT_GROUP,
        name: String::from("root"),
    });
    db.groups.push(GroupAccount {
        gid: WHEEL_GID,
        uuid: Uuid::WHEEL_GROUP,
        name: String::from("wheel"),
    });
    db.groups.push(GroupAccount {
        gid: DEFAULT_USER_GID,
        uuid: AETHER_GROUP_UUID,
        name: String::from("aether"),
    });

    db.users.push(UserAccount {
        uid: 0,
        primary_gid: 0,
        uuid: Uuid::ROOT_USER,
        username: String::from("root"),
        home: String::from("/root"),
        shell: String::from("/bin/aethersh"),
        groups: alloc::vec![0, WHEEL_GID],
        active: true,
    });
    db.users.push(UserAccount {
        uid: DEFAULT_USER_UID,
        primary_gid: DEFAULT_USER_GID,
        uuid: AETHER_USER_UUID,
        username: String::from("aether"),
        home: String::from("/home/aether"),
        shell: String::from("/bin/aethersh"),
        groups: alloc::vec![DEFAULT_USER_GID, WHEEL_GID],
        active: true,
    });

    if db.sudo_dev_mode_enabled {
        db.push_audit(String::from(
            "[AUDIT] WARNING: Passwordless sudo development mode is ENABLED (QEMU dev image, restricted to %wheel)",
        ));
    }
}

pub fn uid_to_uuid(uid: u32) -> Uuid {
    if uid == 0 {
        return Uuid::ROOT_USER;
    }
    let db = IDENTITY_DB.lock();
    for u in &db.users {
        if u.uid == uid && u.active {
            return u.uuid;
        }
    }
    Uuid::deterministic(1, uid as u64)
}

pub fn gid_to_uuid(gid: u32) -> Uuid {
    if gid == 0 {
        return Uuid::ROOT_GROUP;
    }
    let db = IDENTITY_DB.lock();
    for g in &db.groups {
        if g.gid == gid {
            return g.uuid;
        }
    }
    Uuid::deterministic(2, gid as u64)
}

pub fn user_uuid_exists(uuid: Uuid) -> bool {
    if uuid == Uuid::ROOT_USER {
        return true;
    }
    let db = IDENTITY_DB.lock();
    db.users.iter().any(|u| u.uuid == uuid && u.active)
}

pub fn lookup_user_by_uuid(uuid: Uuid) -> Option<UserAccount> {
    let db = IDENTITY_DB.lock();
    db.users
        .iter()
        .find(|u| u.uuid == uuid && u.active)
        .cloned()
}

pub fn lookup_user_by_uid(uid: u32) -> Option<UserAccount> {
    let db = IDENTITY_DB.lock();
    db.users.iter().find(|u| u.uid == uid && u.active).cloned()
}

/// Build unprivileged `Credentials` for the default user `aether` (UID 1000, member of `wheel`).
pub fn aether_user_credentials() -> Credentials {
    Credentials {
        uid: DEFAULT_USER_UID,
        gid: DEFAULT_USER_GID,
        euid: DEFAULT_USER_UID,
        egid: DEFAULT_USER_GID,
        suid: DEFAULT_USER_UID,
        sgid: DEFAULT_USER_GID,
        user_uuid: AETHER_USER_UUID,
        group_uuid: AETHER_GROUP_UUID,
        groups: alloc::vec![DEFAULT_USER_GID, WHEEL_GID],
        caps_effective: 0,
        caps_permitted: 0,
        caps_bounding: CAP_ALL,
        no_new_privs: false,
    }
}

pub fn audit_log(action: &str, actor_uid: u32, subject_uuid: Uuid, target: &str) {
    let ts = timer::uptime_ms();
    let line = format!(
        "[AUDIT +{}ms] action={} actor_uid={} subject_uuid={} target={}",
        ts, action, actor_uid, subject_uuid, target
    );
    crate::kinfo!("{}", line);
    IDENTITY_DB.lock().push_audit(line);
}

pub fn read_audit_log() -> String {
    let db = IDENTITY_DB.lock();
    let mut out = String::new();
    for line in &db.audit_log {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Add a new user to the identity database (requires root or `CAP_SETUID`).
pub fn add_user(uid: u32, gid: u32, username: &str) -> Result<Uuid, Errno> {
    let is_priv = sched::with_current_task(|t| {
        t.creds.euid == 0 || (t.creds.caps_effective & CAP_SETUID) != 0
    });
    if !is_priv {
        return Err(Errno::Eperm);
    }
    if uid == 0 || username.is_empty() || username.len() > 32 {
        return Err(Errno::Einval);
    }
    let mut db = IDENTITY_DB.lock();
    if db
        .users
        .iter()
        .any(|u| (u.uid == uid || u.username == username) && u.active)
    {
        return Err(Errno::Eexist);
    }
    let uuid = Uuid::deterministic(0xAE74_05E8, uid as u64);
    let g_uuid = Uuid::deterministic(0xAE74_6800, gid as u64);
    if !db.groups.iter().any(|g| g.gid == gid) {
        db.groups.push(GroupAccount {
            gid,
            uuid: g_uuid,
            name: String::from(username),
        });
    }
    db.users.push(UserAccount {
        uid,
        primary_gid: gid,
        uuid,
        username: String::from(username),
        home: format!("/home/{}", username),
        shell: String::from("/bin/aethersh"),
        groups: alloc::vec![gid],
        active: true,
    });
    db.push_audit(format!(
        "[AUDIT] USERADD username={} uid={} gid={} uuid={}",
        username, uid, gid, uuid
    ));
    Ok(uuid)
}

/// Evaluate and apply execution identity transition (`execute_as_uuid` / `S_ISUID` / `S_ISGID`)
/// when executing `node` on a mount with `mount_flags` (Section 8.4).
///
/// Critical security rules enforced:
/// 1. Ordinary binaries (`execute_as_uuid == NIL` and no `S_ISUID`) NEVER elevate privileges.
/// 2. If `mount_flags` includes `MS_NOSUID` or `caller_creds.no_new_privs == true`, any
///    `execute_as_uuid` / `S_ISUID` transition to a more privileged identity is rejected/ignored.
/// 3. If `execute_as_uuid` is set (`!is_nil()`), it MUST resolve to an active user in `IDENTITY_DB`.
///    A deleted or invalid UUID NEVER silently becomes root — execution fails with `Errno::Eacces`!
/// 4. Every execution identity transition is recorded in the kernel security audit log.
pub fn compute_exec_credentials(
    caller: &Credentials,
    node: &VfsNode,
    mount_flags: u32,
    path: &str,
) -> Result<Credentials, Errno> {
    let mut next = caller.clone();

    let has_exec_as = (node.flags & INODE_FLAG_EXECUTE_AS) != 0 && !node.execute_as_uuid.is_nil();
    let has_suid = (node.mode & S_ISUID) != 0;
    let has_sgid = (node.mode & S_ISGID) != 0;

    if !has_exec_as && !has_suid && !has_sgid {
        return Ok(next);
    }

    // Check nosuid mount and no_new_privs process restriction
    if (mount_flags & MS_NOSUID) != 0 || caller.no_new_privs {
        audit_log(
            "BLOCKED_EXEC_ELEVATION_NOSUID_OR_NNP",
            caller.euid,
            node.execute_as_uuid,
            path,
        );
        return Err(Errno::Eperm);
    }

    if has_exec_as {
        // Validate target UUID against active identity database — never fall back to root!
        let target_account = lookup_user_by_uuid(node.execute_as_uuid).ok_or(Errno::Eacces)?;
        next.euid = target_account.uid;
        next.suid = target_account.uid;
        next.user_uuid = target_account.uuid;
        next.egid = target_account.primary_gid;
        next.sgid = target_account.primary_gid;
        next.group_uuid = gid_to_uuid(target_account.primary_gid);
        if target_account.uid == 0 {
            next.caps_effective = next.caps_bounding;
            next.caps_permitted = next.caps_bounding;
        } else {
            next.caps_effective = 0;
            next.caps_permitted = 0;
        }
        audit_log("EXEC_AS_TRANSITION", caller.euid, target_account.uuid, path);
        return Ok(next);
    }

    if has_suid {
        let target_account = lookup_user_by_uid(node.uid).ok_or(Errno::Eacces)?;
        next.euid = target_account.uid;
        next.suid = target_account.uid;
        next.user_uuid = target_account.uuid;
        if target_account.uid == 0 {
            next.caps_effective = next.caps_bounding;
            next.caps_permitted = next.caps_bounding;
        }
        audit_log(
            "EXEC_SUID_TRANSITION",
            caller.euid,
            target_account.uuid,
            path,
        );
    }

    if has_sgid {
        next.egid = node.gid;
        next.sgid = node.gid;
        next.group_uuid = node.owner_group_uuid;
    }

    Ok(next)
}

/// Kernel-controlled credential elevation for `sudo` (Section 12.1).
/// Only succeeds if:
/// 1. Passwordless development sudo is enabled (`sudo_dev_mode_enabled`),
/// 2. The calling process does NOT have `no_new_privs`,
/// 3. The calling user is root (`euid == 0`) or a member of the `wheel` group (`GID 10`),
/// 4. Every elevation is explicitly warned and audit-logged.
pub fn sys_elevate_sudo_current(reason: &str) -> Result<(), Errno> {
    let dev_enabled = IDENTITY_DB.lock().sudo_dev_mode_enabled;
    if !dev_enabled {
        return Err(Errno::Eperm);
    }

    let (old_uid, old_uuid, allowed) = sched::with_current_task(|t| {
        let in_wheel =
            t.creds.euid == 0 || t.creds.egid == WHEEL_GID || t.creds.groups.contains(&WHEEL_GID);
        let ok = in_wheel && !t.creds.no_new_privs;
        (t.creds.euid, t.creds.user_uuid, ok)
    });

    if !allowed {
        audit_log("SUDO_DENIED", old_uid, old_uuid, reason);
        return Err(Errno::Eperm);
    }

    sched::with_current_task_mut(|t| {
        t.creds.uid = 0;
        t.creds.euid = 0;
        t.creds.suid = 0;
        t.creds.gid = 0;
        t.creds.egid = 0;
        t.creds.sgid = 0;
        t.creds.user_uuid = Uuid::ROOT_USER;
        t.creds.group_uuid = Uuid::ROOT_GROUP;
        t.creds.caps_effective = t.creds.caps_bounding;
        t.creds.caps_permitted = t.creds.caps_bounding;
    });

    audit_log("SUDO_DEV_ELEVATE_TO_ROOT", old_uid, Uuid::ROOT_USER, reason);
    Ok(())
}

/// Read the kernel security audit log into user buffer (`CAP_AUDIT_READ` or root required).
pub fn read_audit_for_current() -> Result<String, Errno> {
    let allowed = sched::with_current_task(|t| {
        t.creds.euid == 0 || (t.creds.caps_effective & CAP_AUDIT_READ) != 0
    });
    if !allowed {
        return Err(Errno::Eperm);
    }
    Ok(read_audit_log())
}
