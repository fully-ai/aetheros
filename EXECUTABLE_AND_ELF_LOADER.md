# AetherOS Executable Format & ELF64 Loader

## 1. ELF64 Parser & Validator (`crates/aether-elf/src/lib.rs`)

AetherOS executes standard 64-bit `x86_64` ELF (`ET_EXEC` and fixed-base `ET_DYN`) binaries compiled for the `x86_64-unknown-none` target. Before mapping any bytes into memory, `parse_and_validate_elf64(bytes)` enforces 10 structural and security invariants:

1. **Header Bounds & Magic**: Image must be at least `64` bytes (`sizeof(Elf64Ehdr)`) and start with `\x7FELF`.
2. **Class, Endianness & Machine**: Must be 64-bit (`ELFCLASS64 = 2`), little-endian (`ELFDATA2LSB = 1`), and `EM_X86_64` (`62`).
3. **Program Header Table Bounds**: `e_phoff + e_phnum * e_phentsize` is checked with saturating/checked arithmetic (`e_phentsize == 56`, `1 <= e_phnum <= 32`) against `bytes.len()`.
4. **Canonical User Address Range**: Every `PT_LOAD` segment must satisfy `USER_MIN_VADDR (0x1000) <= p_vaddr` and `p_vaddr + p_memsz <= USER_MAX_VADDR (0x0000_7FFF_FFFF_0000)`. Any segment targeting null (`< 0x1000`) or higher-half kernel memory (`>= 0xffff800000000000`) is rejected with `ElfError::KernelOrNullAddressTarget`.
5. **File Offset & BSS Invariant**: `p_filesz <= p_memsz` and `p_offset + p_filesz <= bytes.len()`.
6. **Strict `W^X` Segment Policy**: No `PT_LOAD` segment may have both `PF_W (0x2)` and `PF_X (0x1)` set (`ElfError::WritableAndExecutableSegment`).
7. **Entry Point Verification**: `e_entry` must fall inside at least one executable (`PF_X`) `PT_LOAD` segment (`ElfError::EntryPointNotExecutable`).
8. **10,000-Iteration Fuzz Verification**: `tests::fuzz_smoke_10000_mutated_elf_headers_never_panic` mutates valid and malformed ELF64 headers across 10,000 iterations to guarantee zero panics or out-of-bounds reads.

---

## 2. Segment Mapping & Initial Ring 3 User Stack (`kernel/src/sched/exec.rs`)

When `load_elf_into_new_address_space` loads a validated ELF64 image:
1. **Isolated User `AddressSpace`**: Creates a fresh 4-level `PML4` copying only the kernel higher-half mappings (`PML4[256..512]`).
2. **`PT_LOAD` Segment Pages**: Allocates zeroed 4 KiB physical frames for each segment page `[align_down(vaddr), align_up(vaddr + memsz))`, copies `[file_offset .. file_offset + filesz)` into the page, leaves trailing `.bss` bytes zeroed, and maps the page with `PTE_PRESENT | PTE_USER`, adding `PTE_WRITABLE` if `PF_W` and `PTE_NO_EXECUTE` if `!PF_X`.
3. **Guarded 32 KiB Ring 3 Stack**:
   - Maps an unmapped guard page (`PTE_GUARD`) at `0x0000_7FFF_FFFE_7000`.
   - Maps eight 4 KiB RW+NX user stack pages spanning `0x0000_7FFF_FFFE_8000 .. 0x0000_7FFF_FFFF_0000`.
4. **System V AMD64 Initial Stack & Register Layout**:
   - Copies `envp` and `argv` NUL-terminated strings to the top of `0x0000_7FFF_FFFE_F000`.
   - Places the 16-byte aligned stack table at `user_rsp`:
     ```text
     [user_rsp + 0x00] : argc (u64)
     [user_rsp + 0x08] : argv[0] pointer ... argv[argc-1] pointer, NULL (0)
     [user_rsp + ...]  : envp[0] pointer ... envp[envc-1] pointer, NULL (0)
     [user_rsp + ...]  : ELF Auxiliary Vector (AT_PAGESZ, AT_PHDR, AT_PHENT, AT_PHNUM, AT_ENTRY, AT_UID, AT_EUID, AT_GID, AT_EGID, AT_NULL)
     ```
   - Passes `RDI = argc`, `RSI = argv_ptr`, `RDX = envp_ptr`, `RIP = entry_point`, `RSP = user_rsp`, `CS = 0x2B`, `SS = 0x23`, `RFLAGS = 0x202`.
