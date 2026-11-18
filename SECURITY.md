# AetherOS Security Policy & Vulnerability Reporting

See [SECURITY_MODEL.md](file:///home/ubeyt/Belgeler/AetherOS/SECURITY_MODEL.md) and [SECURITY_HARDENING.md](file:///home/ubeyt/Belgeler/AetherOS/SECURITY_HARDENING.md) for the complete architecture specification.

## 1. Security Invariants Enforced by Default
- **Hardware Page & Execution Protections**: `CR0.WP`, `EFER.NXE`, `CR4.SMEP`, strict `W^X` in both VMM and ELF64 loading, and unmapped `PTE_GUARD` user stack guard pages.
- **User Pointer Isolation**: All user pointers crossing the `SYSCALL` ABI v1 boundary are checked against `0x0000_8000_0000_0000` and validated against the active user `PML4` before dereferencing.
- **UUID Identity & `execute_as_uuid` Safeguards**:
  - `execute_as_uuid` is modifiable only by `root` (`SYS_SET_EXECUTE_AS`), cleared on file write/truncation/chown, blocked on `MS_NOSUID` mounts and `no_new_privs` tasks, and never falls back to `root` if a target UUID is missing or inactive.
- **Development-Mode `sudo` Warning**:
  - Passwordless `sudo` for `%wheel` is strictly an opt-in QEMU development mode (`sudo_dev_mode_enabled`), emits an explicit console warning, and records every elevation in the kernel security audit log (`[AUDIT]`).

## 2. Reporting Security Issues
Report kernel isolation, `mm::user_access`, ELF64 parser, or AetherFS metadata vulnerabilities via GitHub Issues on `https://github.com/fully-ai/aetheros` with a reproducible QEMU or host unit test case.
