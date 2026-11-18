# AetherOS Executable Loading & Ring 3 Execution Model

See also [EXECUTABLE_AND_ELF_LOADER.md](file:///home/ubeyt/Belgeler/AetherOS/EXECUTABLE_AND_ELF_LOADER.md) and [INIT_AND_SERVICES.md](file:///home/ubeyt/Belgeler/AetherOS/INIT_AND_SERVICES.md).

## 1. ELF64 Loading & Ring 3 Entry Pipeline

```mermaid
flowchart TD
    VFS["fs::vfs_read_executable_and_meta(path, caller_creds): verify X permission & !MS_NOEXEC"]
    CRED["security::compute_exec_credentials: evaluate execute_as_uuid / S_ISUID / MS_NOSUID / no_new_privs"]
    ELF["aether_elf::parse_and_validate_elf64: verify magic, EM_X86_64, PT_LOAD bounds, W^X, user address range"]
    ASP["AddressSpace::new_user(): create isolated 4-level PML4 copying higher-half kernel entries 256..512"]
    SEG["Map PT_LOAD segments (4 KiB frames, copy file bytes, zero .bss, apply PTE_USER + W^X flags)"]
    STK["Map 4 KiB PTE_GUARD (0x7ffffffe7000) + 32 KiB RW+NX user stack; serialize argv, envp, auxv"]
    IRET["Activate new CR3, destroy old user PML4, set TrapFrame (CS=0x2B, SS=0x23, RIP=e_entry, RSP=user_rsp)"]

    VFS --> CRED --> ELF --> ASP --> SEG --> STK --> IRET
```

## 2. User-Space Runtime (`userspace/libaether/src/lib.rs`)
- Entry symbol `_start(argc, argv, envp)` receives `(RDI, RSI, RDX)`, converts `argv` into `&[&str]`, invokes the binary's `main(args) -> i32`, and terminates cleanly via `SYS_EXIT(code)`.
- Provides a thread-safe 1 MiB `.bss` static bump `#[global_allocator]` so Ring 3 ELF64 binaries freely use `alloc::vec::Vec`, `alloc::string::String`, and `format!`.
