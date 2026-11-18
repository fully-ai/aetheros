# AetherOS Virtual Filesystem (VFS) Design

## 1. Unified VFS Architecture (`kernel/src/fs/mod.rs`)

The AetherOS Virtual Filesystem provides a single hierarchical namespace with canonical path normalization (`normalize_path`), symlink traversal (`MAX_SYMLINK_HOPS = 8`), per-mount security flags (`MS_RDONLY`, `MS_NOSUID`, `MS_NOEXEC`), UUID-based ownership and ACL metadata on every node (`VfsNode`), and transparent routing to underlying filesystem engines (`ramfs`, `devfs`, `procfs`, `tmpfs`, and **AetherFS v1** on `/mnt/aether`).

---

## 2. Standard Linux-Like Directory Hierarchy

During Stage 10 boot initialization, the VFS populates the full filesystem hierarchy and unpacks `/boot/initramfs.tar`:

```text
/
├── bin/            # Compiled Ring 3 ELF64 core utilities & /bin/aethersh, /bin/sudo, /bin/aetherpkg
├── sbin/           # System administration & /sbin/init (PID 1 service supervisor)
├── boot/           # Kernel image & initramfs metadata
├── dev/            # Character & system device nodes (null, zero, random, urandom, console, tty, serial)
├── etc/            # System configuration (passwd, group, sudoers, hostname, os-release, services.conf)
├── home/
│   └── aether/     # Default unprivileged user home directory (UID 1000, mode 0750)
├── lib/            # Shared system libraries
├── mnt/
│   └── aether/     # Mounted AetherFS v1 persistent journaling volume
├── opt/            # Optional add-on packages
├── proc/           # Live kernel & process diagnostic virtual filesystem (procfs)
├── root/           # Root home directory (UID 0, mode 0700)
├── run/            # Runtime service state (tmpfs, nosuid)
├── tmp/            # Temporary files with sticky bit enforced (mode 01777, tmpfs, nosuid)
├── usr/
│   ├── bin/
│   ├── lib/
│   ├── include/
│   └── share/
└── var/
    ├── log/        # Service logs (/var/log/init.log)
    ├── cache/
    └── lib/
        └── aetherpkg/ # Package manager installed database
```

---

## 3. Character Devices (`/dev`) and Diagnostic Virtual Files (`/proc`)

### `/dev` Nodes
- `/dev/null` (`0666`): Discards all writes; reads return `0` (`EOF`).
- `/dev/zero` (`0666`): Discards all writes; reads fill destination buffer with `0x00`.
- `/dev/random` & `/dev/urandom` (`0666`): CSPRNG-seeded SplitMix64 + `RDTSC` entropy stream.
- `/dev/console`, `/dev/tty`, `/dev/serial`: Interactive 16550 UART + linear framebuffer console I/O.

### `/proc` Virtual Files
- `/proc/version`: Kernel version, target triple, compiler, and bootloader protocol.
- `/proc/uptime`: Millisecond uptime and PIT tick counter.
- `/proc/meminfo`: Total usable physical memory, free physical memory, and kernel heap statistics.
- `/proc/cpuinfo`: Processor architecture and Long Mode privilege rings.
- `/proc/mounts`: Active mount table (`ramfs`, `devfs`, `procfs`, `tmpfs`, `aetherfs`) and mount flags.
- `/proc/syscalls`: Total system calls dispatched, context switches, and timer preemptions.
- `/proc/gui`: Milestone 14 desktop compositor resolution, window z-order table, and frame CRC.
