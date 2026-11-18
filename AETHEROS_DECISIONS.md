# AetherOS Architectural Decisions Summary

Detailed Architecture Decision Records are maintained under `docs/adr/`:

1. **[ADR-0001: Bootloader Selection — Limine Boot Protocol v3](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0001-bootloader-limine.md)**:
   Selected Limine v9 (`BaseRevision::new(3)`) for deterministic 64-bit Long Mode handoff, HHDM (`0xffff800000000000`), higher-half kernel placement (`0xffffffff80000000`), and single-image hybrid UEFI (`BOOTX64.EFI`) + Legacy BIOS (`limine-bios.sys`) boot support.
2. **[ADR-0002: Language & Toolchain — `no_std` Rust + `x86_64` Assembly](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0002-language-and-toolchain.md)**:
   Selected Rust (`x86_64-unknown-none`) for memory-safe kernel subsystems and Ring 3 user-space utilities, isolating hardware context switching (`idt.rs`, `syscall_entry.rs`) in audited inline assembly.
3. **[ADR-0003: Virtual Memory Layout & `W^X` Policy](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0003-memory-layout.md)**:
   Partitioned 48-bit canonical virtual memory into isolated lower-half Ring 3 user spaces (`PML4[0..256]`) and shared higher-half Ring 0 kernel mappings (`PML4[256..512]`) with strict `W^X`, Demand Paging (`PTE_DEMAND`), Copy-on-Write (`PTE_COW`), and Guard Pages (`PTE_GUARD`).
4. **[ADR-0004: Native Syscall ABI v1 (`SYSCALL`/`SYSRET`)](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0004-syscall-abi.md)**:
   Defined a versioned native syscall ABI (`crates/aether-abi`) with mandatory user pointer validation (`mm::user_access`) and an explicit `SyscallStatus` table returning `-ENOSYS` for unimplemented calls.
5. **[ADR-0005: AetherFS v1 (`>= 1 PiB` 64-Bit Extent Journaling Filesystem)](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0005-aetherfs-v1.md)**:
   Designed an independent 64-bit extent filesystem (`crates/aether-fs-core`) supporting `>= 1 PiB` volumes, Primary (`LBA 1`) + Backup (`LBA 2`) superblocks, Castagnoli CRC32C checksums on superblocks/inodes/journal, and a Write-Ahead Journal (`AETHJRNL`).
6. **[ADR-0006: 128-Bit UUID Identity, ACLs & `execute_as_uuid` Security Model](file:///home/ubeyt/Belgeler/AetherOS/docs/adr/0006-uuid-security-model.md)**:
   Integrated RFC 4122 128-bit UUIDs into process credentials and AetherFS/VFS inodes alongside POSIX `uid`/`gid` mappings, per-inode UUID ACLs, and root-controlled `execute_as_uuid` execution transitions.
