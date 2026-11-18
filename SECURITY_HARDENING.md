# AetherOS Security Hardening & Sandboxing

## 1. Hardware & Memory Protection Controls

| Control | Implementation | Verification Test |
| :--- | :--- | :--- |
| **`CR0.WP` (Supervisor Write Protect)** | Enabled in `arch::x86_64::cpu::init_and_enable_protections`; prevents Ring 0 from overwriting read-only kernel or user COW pages | `test_cpu_protections` |
| **`EFER.NXE` (No-Execute Page Bit)** | Enabled in `IA32_EFER`; enforced on all data, heap, stack, and BSS mappings (`PTE_NO_EXECUTE`) | `test_cpu_protections`, `test_vmm_page_mapping_and_wx` |
| **`CR4.SMEP` (Supervisor Mode Execution Prevention)** | Enabled in `CR4` when advertised by CPUID leaf `7`; faults if Ring 0 jumps to a `PTE_USER` page | `test_cpu_protections` |
| **Strict `W^X` Enforcement** | Both VMM `map_page_in` and ELF64 loader `parse_and_validate_elf64` reject writable+executable pages/segments | `test_vmm_page_mapping_and_wx`, `rejects_wx_segment_and_out_of_bounds_offset` |
| **Guarded User Stacks (`PTE_GUARD`)** | Unmapped 4 KiB guard page (`0x0000_7FFF_FFFE_7000`) placed below every Ring 3 process stack | `load_elf_into_new_address_space` |
| **Dedicated TSS `IST1..3` Stacks** | 16 KiB isolated interrupt stacks for `#DF` (Double Fault), `#NMI`, and `#MC` in 64-bit TSS | `test_gdt_and_tss` |
| **Heap Canary & Double-Free Detection** | `BlockHeader::magic = 0xAE74_EC01` and PMM bitmap double-free counter | `test_pmm_allocator`, `test_kernel_heap_and_alloc` |

---

## 2. Kernel Boundary & User-Space Isolation Controls

1. **Canonical User Pointer Validation (`mm::user_access`)**:
   - All syscall buffer pointers are checked against `USER_MAX_VADDR (0x0000_8000_0000_0000)` with overflow-checked arithmetic and page-table walk verification before any kernel read or write.
   - Tested against 10 adversarial boundary addresses (`0x0`, `0x0000_8000_0000_0000`, `0xffff800000000000`, `0xffffffff80000000`, `u64::MAX`) in `test_m13_security_fuzz_and_benchmarks`.
2. **Process Fault Containment (`test_ring3_fault_isolation`)**:
   - Malicious Ring 3 processes attempting to dereference higher-half kernel memory (`mov rax, [0xffffffff80000000]`) or execute privileged instructions (`cli`) trigger `#PF` / `#GP` with `from_user_mode() == true`, terminating **only** the offending PID with `exit_code = 139` (`SIGSEGV`).
3. **Irreversible `no_new_privs` & `MS_NOSUID` Enforcement**:
   - Once `SYS_SET_NO_NEW_PRIVS` is invoked or a filesystem is mounted with `MS_NOSUID`, all `execute_as_uuid`, `S_ISUID`, and `SYS_ELEVATE_SUDO` privilege transitions are blocked (`-EPERM`) and audit-logged.
4. **Fuzzing Across Parsers**:
   - `aether-elf` parser: 10,000 mutated ELF64 headers tested on every run (`fuzz_smoke_10000_mutated_elf_headers_never_panic`).
   - `net::process_inbound_ipv4`: 2,000 mutated/truncated IPv4/ICMP frames tested on every run (`test_drivers_pci_rtc_and_network_stack`).
