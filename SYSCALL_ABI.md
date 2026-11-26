# AetherOS Native System Call ABI (v1.0)

See also [SYSCALL_SPEC.md](SYSCALL_SPEC.md) for the complete system call status table.

## 1. Syscall Flow Diagram (`Ring 3 -> Ring 0 -> Ring 3`)

```mermaid
sequenceDiagram
    participant U as Ring 3 Process (libaether)
    participant STUB as syscall_entry_stub (IA32_LSTAR)
    participant DISP as syscall::dispatch (kernel/src/syscall)
    participant UACC as mm::user_access (Pointer Validator)
    participant SUB as VFS / Sched / MM / IPC / Security

    U->>STUB: Load RAX=nr, RDI,RSI,RDX,R10,R8,R9; execute `syscall`
    STUB->>STUB: `swapgs`, save user RSP to gs:[8], load kernel RSP from gs:[0], build TrapFrame
    STUB->>DISP: Call `syscall_dispatch_from_trapframe(&mut TrapFrame)`
    DISP->>UACC: Validate user buffer pointers [ptr, ptr+len) < 0x0000800000000000 & walk page tables
    UACC-->>DISP: Ok(()) or Err(Errno::Efault)
    DISP->>SUB: Execute kernel subsystem operation
    SUB-->>DISP: Result<u64, Errno>
    DISP-->>STUB: Write return value (`val` or `-errno`) into `frame.rax`
    STUB->>U: Jump to `isr_restore_trapframe`: restore registers, `swapgs`, `iretq` to Ring 3
```

## 2. ABI Compatibility Rules
- **Version Query**: `SYS_ABI_VERSION (0)` returns `(ABI_VERSION_MAJOR << 16) | ABI_VERSION_MINOR` (`0x0001_0000`).
- **Explicit Status**: `syscall::syscall_status(nr)` classifies every number as `Implemented`, `Planned`, or `NotImplemented`. Unimplemented or planned numbers deterministically return `-ENOSYS` (`-38`).
- **Linux Separation**: AetherOS defines its own native `aether-abi` numbering and UUID-aware structures (`AetherStat` includes 128-bit `object_uuid`, `owner_user_uuid`, `owner_group_uuid`, and `execute_as_uuid`), rather than claiming fake binary compatibility with Linux `x86_64`.
