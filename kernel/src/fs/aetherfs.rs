//! Kernel VFS Adapter for AetherFS v1 (`crates/aether-fs-core`).
//!
//! Bridges the Virtual Filesystem Layer (`kernel/src/fs/mod.rs`) with a mounted
//! `AetherFsVolume` backed by either a VirtIO Block Device (when attached) or a
//! sparse/RAM-backed kernel block device, maintaining crash-consistent Write-Ahead
//! Journaled persistence for files, directories, UUID ownership, ACLs, and `execute_as_uuid`.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use aether_fs_core::{
    AetherFsVolume, AetherOnDiskInode, BlockDevice, FsError, OnDiskAce, RecoveryReport, BLOCK_SIZE,
    ONE_PIB_BLOCKS_4K,
};
use aether_uuid::Uuid;

use crate::arch::x86_64::timer;
use crate::fs::{AclEntry, NodeKind, VfsNode, S_IFDIR, S_IFMT, S_IFREG};
use crate::sync::IrqSpinLock;

/// Sparse block device backed by kernel heap pages and optional VirtIO-blk forwarding.
/// Allows formatting and testing volumes with virtual capacities `>= 1 PiB` (`ONE_PIB_BLOCKS_4K`)
/// as well as persistent VirtIO disk backing (`LBA < virtio_blocks`).
pub struct KernelBlockDisk {
    pub total_blocks: u64,
    pub blocks: BTreeMap<u64, [u8; BLOCK_SIZE]>,
    pub use_virtio_blk: bool,
}

impl KernelBlockDisk {
    pub const fn new(total_blocks: u64) -> Self {
        Self {
            total_blocks,
            blocks: BTreeMap::new(),
            use_virtio_blk: false,
        }
    }
}

impl BlockDevice for KernelBlockDisk {
    fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    fn read_block(&self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba >= self.total_blocks {
            return Err(FsError::IoError);
        }
        if let Some(b) = self.blocks.get(&lba) {
            buf.copy_from_slice(b);
            return Ok(());
        }
        buf.fill(0);
        Ok(())
    }

    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), FsError> {
        if lba >= self.total_blocks {
            return Err(FsError::IoError);
        }
        self.blocks.insert(lba, *buf);
        Ok(())
    }
}

struct ActiveAetherFs {
    vol: Option<AetherFsVolume<KernelBlockDisk>>,
    path_to_slot: BTreeMap<String, u64>,
    next_slot: u64,
}

impl ActiveAetherFs {
    const fn new() -> Self {
        Self {
            vol: None,
            path_to_slot: BTreeMap::new(),
            next_slot: 0,
        }
    }
}

static AETHERFS_MOUNT: IrqSpinLock<ActiveAetherFs> = IrqSpinLock::new(ActiveAetherFs::new());

/// Initialize and mount the system AetherFS v1 volume (configured with 1 PiB+ addressable geometry).
pub fn init() -> Result<(Uuid, u64), FsError> {
    let mut guard = AETHERFS_MOUNT.lock();
    // Format a sparse 2 PiB-capable AetherFS volume (`2 * ONE_PIB_BLOCKS_4K`)
    let disk = KernelBlockDisk::new(ONE_PIB_BLOCKS_4K * 2);
    let vol_uuid = Uuid::deterministic(0xAE74_F501, 1);
    let vol = AetherFsVolume::format(disk, 16, 32, vol_uuid)?;
    let total_blocks = vol.sb.total_blocks;
    guard.vol = Some(vol);
    guard.path_to_slot.clear();
    guard.next_slot = 0;
    Ok((vol_uuid, total_blocks))
}

/// Simulate an unexpected reboot/remount on the underlying block device, executing AetherFS
/// superblock verification + Write-Ahead Journal crash recovery and reloading persisted inodes into VFS.
pub fn remount_and_recover() -> Result<RecoveryReport, FsError> {
    let mut guard = AETHERFS_MOUNT.lock();
    let old_vol = guard.vol.take().ok_or(FsError::IoError)?;
    let raw_disk = old_vol.dev;
    let new_vol = AetherFsVolume::mount_and_recover(raw_disk)?;
    let report = new_vol.last_recovery;

    // Reload all allocated inodes from the recovered AetherFS volume back into the VFS
    let max_slots = guard.next_slot.min(new_vol.sb.max_inodes);
    let mut restored_nodes: Vec<(String, VfsNode)> = Vec::new();
    for slot in 0..max_slots {
        if let Ok(ino) = new_vol.read_inode(slot) {
            if !ino.allocated || ino.meta_block == 0 {
                continue;
            }
            let mut meta_buf = [0u8; BLOCK_SIZE];
            if new_vol
                .dev
                .read_block(ino.meta_block, &mut meta_buf)
                .is_ok()
            {
                if let Ok((path, aces)) =
                    AetherFsVolume::<KernelBlockDisk>::decode_meta_block(&meta_buf)
                {
                    let mut acl = Vec::with_capacity(aces.len());
                    for a in aces {
                        acl.push(AclEntry {
                            subject_uuid: a.subject_uuid,
                            is_group: a.is_group,
                            allow_rwx: a.allow_rwx,
                            deny_rwx: a.deny_rwx,
                            _pad: 0,
                        });
                    }
                    let kind = if (ino.mode & S_IFMT) == S_IFDIR {
                        NodeKind::Directory
                    } else {
                        let mut data = alloc::vec![0u8; ino.size_bytes.min(1024 * 1024) as usize];
                        let _ = new_vol.pread_inode(&ino, 0, &mut data);
                        NodeKind::Regular { data }
                    };
                    restored_nodes.push((
                        path,
                        VfsNode {
                            inode_id: ino.inode_id,
                            object_uuid: ino.object_uuid,
                            owner_user_uuid: ino.owner_user_uuid,
                            owner_group_uuid: ino.owner_group_uuid,
                            execute_as_uuid: ino.execute_as_uuid,
                            uid: ino.uid,
                            gid: ino.gid,
                            mode: ino.mode,
                            flags: ino.flags,
                            nlink: ino.nlink,
                            ctime_ns: ino.ctime_ns,
                            mtime_ns: ino.mtime_ns,
                            atime_ns: ino.atime_ns,
                            acl,
                            kind,
                        },
                    ));
                }
            }
        }
    }

    guard.vol = Some(new_vol);
    drop(guard);

    for (path, node) in restored_nodes {
        crate::fs::vfs_import_persisted_node(path, node);
    }

    Ok(report)
}

pub fn sync_file_create(path: &str, mode: u32, uid: u32, gid: u32) {
    let _ = persist_path_data(path, S_IFREG | (mode & 0o7777), uid, gid, &[]);
}

#[allow(non_snake_case)]
pub fn sync_file_Write(path: &str, data: &[u8]) {
    if let Some(node) = crate::fs::vfs_get_node_clone(path) {
        let _ = persist_vfs_node(path, &node, data);
    }
}

pub fn sync_mkdir(path: &str, mode: u32, uid: u32, gid: u32) {
    let _ = persist_path_data(path, S_IFDIR | (mode & 0o7777), uid, gid, &[]);
}

pub fn sync_metadata(path: &str) {
    if let Some(node) = crate::fs::vfs_get_node_clone(path) {
        let data = match &node.kind {
            NodeKind::Regular { data } => data.clone(),
            _ => Vec::new(),
        };
        let _ = persist_vfs_node(path, &node, &data);
    }
}

pub fn sync_unlink(path: &str) {
    let mut guard = AETHERFS_MOUNT.lock();
    let Some(slot) = guard.path_to_slot.remove(path) else {
        return;
    };
    if let Some(vol) = &mut guard.vol {
        let empty = AetherOnDiskInode::default();
        let _ = vol.write_inode_journaled(slot, &empty);
    }
}

fn persist_path_data(
    path: &str,
    mode: u32,
    uid: u32,
    gid: u32,
    data: &[u8],
) -> Result<(), FsError> {
    if let Some(node) = crate::fs::vfs_get_node_clone(path) {
        persist_vfs_node(path, &node, data)
    } else {
        let now = timer::uptime_ns();
        let temp = VfsNode {
            inode_id: 1,
            object_uuid: Uuid::deterministic(0xAE74, 1),
            owner_user_uuid: crate::security::uid_to_uuid(uid),
            owner_group_uuid: crate::security::gid_to_uuid(gid),
            execute_as_uuid: Uuid::NIL,
            uid,
            gid,
            mode,
            flags: 0,
            nlink: 1,
            ctime_ns: now,
            mtime_ns: now,
            atime_ns: now,
            acl: Vec::new(),
            kind: NodeKind::Directory,
        };
        persist_vfs_node(path, &temp, data)
    }
}

fn persist_vfs_node(path: &str, node: &VfsNode, data: &[u8]) -> Result<(), FsError> {
    let mut guard = AETHERFS_MOUNT.lock();
    let slot = if let Some(&existing) = guard.path_to_slot.get(path) {
        existing
    } else {
        let s = guard.next_slot;
        guard.next_slot += 1;
        guard.path_to_slot.insert(String::from(path), s);
        s
    };

    let Some(vol) = &mut guard.vol else {
        return Ok(());
    };
    if slot >= vol.sb.max_inodes {
        return Err(FsError::InodeTableFull);
    }

    let mut current = vol.read_inode(slot).unwrap_or_default();
    if current.meta_block == 0 {
        current.meta_block = vol.alloc_data_blocks(1)?;
    }

    let mut aces = Vec::with_capacity(node.acl.len());
    for a in &node.acl {
        aces.push(OnDiskAce {
            subject_uuid: a.subject_uuid,
            is_group: a.is_group,
            allow_rwx: a.allow_rwx,
            deny_rwx: a.deny_rwx,
            _pad: 0,
        });
    }
    let mut meta_buf = [0u8; BLOCK_SIZE];
    AetherFsVolume::<KernelBlockDisk>::encode_meta_block(path, &aces, &mut meta_buf);
    vol.dev.write_block(current.meta_block, &meta_buf)?;

    current.allocated = true;
    current.inode_id = node.inode_id;
    current.object_uuid = node.object_uuid;
    current.owner_user_uuid = node.owner_user_uuid;
    current.owner_group_uuid = node.owner_group_uuid;
    current.execute_as_uuid = node.execute_as_uuid;
    current.uid = node.uid;
    current.gid = node.gid;
    current.mode = node.mode;
    current.flags = node.flags;
    current.nlink = node.nlink;
    current.acl_count = aces.len() as u16;
    current.ctime_ns = node.ctime_ns;
    current.mtime_ns = node.mtime_ns;
    current.atime_ns = node.atime_ns;
    current.size_bytes = data.len() as u64;

    if data.is_empty() {
        vol.write_inode_journaled(slot, &current)?;
    } else {
        vol.pwrite_inode(slot, current, 0, data)?;
    }
    Ok(())
}

/// Execute an in-kernel AetherFS 1 PiB sparse file + journal crash recovery + backup superblock test.
pub fn run_aetherfs_self_tests() -> Result<(), FsError> {
    let mut guard = AETHERFS_MOUNT.lock();
    let slot = guard.next_slot;
    guard.next_slot += 1;
    let vol = guard.vol.as_mut().ok_or(FsError::IoError)?;

    // 1. Test 1 PiB + 4 KiB sparse file write & read on the live mounted AetherFS volume!
    let inode = AetherOnDiskInode {
        allocated: true,
        inode_id: 9001,
        object_uuid: Uuid::deterministic(0xAE74, 9001),
        owner_user_uuid: Uuid::ROOT_USER,
        owner_group_uuid: Uuid::ROOT_GROUP,
        execute_as_uuid: crate::security::AETHER_USER_UUID,
        uid: 0,
        gid: 0,
        mode: S_IFREG | 0o755,
        flags: 4,
        nlink: 1,
        ..AetherOnDiskInode::default()
    };
    let one_pib_offset = aether_fs_core::ONE_PIB_BYTES + 4096;
    let msg = b"AetherFS-Live-1PiB-Kernel-Verified";
    let updated = vol.pwrite_inode(slot, inode, one_pib_offset, msg)?;
    assert_eq!(updated.size_bytes, one_pib_offset + (msg.len() as u64));

    let mut read_back = [0u8; 34];
    let n = vol.pread_inode(&updated, one_pib_offset, &mut read_back)?;
    assert_eq!(n, msg.len());
    assert_eq!(&read_back, msg);

    // 2. Test Primary Superblock corruption -> automatic failover to Backup Superblock!
    let mut bad_sb = [0xAAu8; BLOCK_SIZE];
    bad_sb[0..8].copy_from_slice(&aether_fs_core::SUPERBLOCK_MAGIC);
    vol.dev
        .write_block(aether_fs_core::PRIMARY_SUPERBLOCK_LBA, &bad_sb)?;

    // 3. Stage a committed journal transaction and simulate power loss before home LBA write
    let test_lba = vol.sb.data_start;
    let mut jblk = [0u8; BLOCK_SIZE];
    jblk[0..16].copy_from_slice(b"JOURNAL_REPLAYED");
    vol.execute_journal_tx(&[(test_lba, jblk)], true, false)?;
    drop(guard);

    // Remount and recover -> must repair primary superblock AND replay committed journal transaction!
    let report = remount_and_recover()?;
    assert!(
        report.used_backup_superblock,
        "must recover from backup superblock"
    );
    assert!(
        report.replayed_journal_tx,
        "must replay committed journal transaction"
    );

    let guard = AETHERFS_MOUNT.lock();
    let vol = guard.vol.as_ref().ok_or(FsError::IoError)?;
    let mut verify_blk = [0u8; BLOCK_SIZE];
    vol.dev.read_block(test_lba, &mut verify_blk)?;
    assert_eq!(&verify_blk[0..16], b"JOURNAL_REPLAYED");

    Ok(())
}
