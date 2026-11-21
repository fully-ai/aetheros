# AetherOS Mount System & Removable Media Security Policy (`MOUNT_SYSTEM.md`)

## 1. Mount Table & Security Flags (`kernel/src/fs/mod.rs`, `kernel/src/storage/mod.rs`)

AetherOS maintains an active mount table (`vfs.mounts`, exposed via `/proc/mounts` and `findmnt`) enforcing four kernel-level mount flags:
- `MS_RDONLY (1 << 0)`: Rejects all file creation, modification, truncation, and deletion with `-EROFS`.
- `MS_NOSUID (1 << 1)`: Blocks `execute_as_uuid` and `S_ISUID` / `S_ISGID` privilege transitions (`-EPERM` + `[AUDIT] BLOCKED_EXEC_ELEVATION_NOSUID_OR_NNP`).
- `MS_NOEXEC (1 << 2)`: Blocks direct `sys_execve` execution of any binary residing on the mount (`-EACCES`).
- `MS_NODEV (1 << 3)`: Restricts untrusted device node interpretation on removable volumes.

## 2. Automatic Removable USB Mounting Policy
When a USB Mass Storage device (`/dev/sdc1`) is detected:
1. Its volume label is sanitized (`storage::sanitize_volume_label`) to strip path separators (`/`, `\`, `..`) and resolve duplicate label collisions (`AETHER_USB` -> `AETHER_USB_2`).
2. It is mounted at `/mnt/usb/<SANITIZED_LABEL>` with **`MS_NOSUID | MS_NODEV | MS_NOEXEC`**.
3. No script or binary on the USB drive is ever auto-executed, and attempts to `execve` a file on `/mnt/usb/...` return `-EACCES`.
4. Before `safely_eject_usb_drive()` or `/bin/eject` unmounts the volume, dirty VFS files are synchronized to the FAT32 cluster chain and flushed via SCSI `SYNCHRONIZE CACHE(10)`.
