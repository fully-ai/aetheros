# Contributing to AetherOS

## Engineering Rules

1. **Kernel First, GUI Last**: All changes must preserve subsystem layering (`arch -> mm -> sync -> sched -> security/ipc -> fs -> drivers/net/gui -> syscall -> userspace`).
2. **Zero Fake Functionality**:
   - Never add empty modules or syscall stubs that return fake success (`0`). Unimplemented syscalls must be explicitly marked `SyscallStatus::Planned` or `SyscallStatus::NotImplemented` and return `-ENOSYS`.
   - Every new kernel or user-space feature must be accompanied by an automated host unit test (`crates/*/src/lib.rs`) or in-kernel QEMU self-test (`kernel/src/selftest.rs`).
3. **Pre-Commit Verification Gate**:
   Every pull request must pass all three commands cleanly before merging:
   ```bash
   ./scripts/format.sh --check
   ./scripts/lint.sh
   ./scripts/test.sh
   ```
