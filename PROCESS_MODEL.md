# AetherOS Process & Thread Model

See also [PROCESS_AND_SCHEDULING.md](file:///home/ubeyt/Belgeler/AetherOS/PROCESS_AND_SCHEDULING.md).

## 1. Process Lifecycle State Machine

```mermaid
stateDiagram-v2
    [*] --> Ready: spawn_kernel_thread / spawn_elf_process_from_vfs / sys_fork
    Ready --> Running: schedule_from_isr selects highest-priority Ready task
    Running --> Ready: 1000 Hz PIT quantum expired (ticks_remaining == 0) or sys_sched_yield
    Running --> Sleeping: sys_nanosleep / sched::sleep_ms(ms)
    Sleeping --> Ready: timer IRQ0 observes now_ticks >= wake_tick
    Running --> Blocked: WaitQueue::sleep_current / sys_wait4 / blocking pipe or socket read
    Blocked --> Ready: WaitQueue::wake_one / child exit delivers SIGCHLD
    Running --> Zombie: sys_exit(code) / fatal Ring 3 exception (#PF, #GP) / SIGKILL
    Zombie --> [*]: Parent reaps via sys_wait4 (destroys user PML4 & frees 24 KiB kernel stack)
```

## 2. Address Space & File Descriptor Inheritance

- **`sys_fork` (`71`)**:
  - Clones user `PML4[0..256]` using Copy-on-Write (`clone_user_pml4_cow`), marking writable pages `PTE_COW` and incrementing physical frame reference counts in `pmm::SHARED_REFS`.
  - Duplicates all open file descriptors (`fds`), incrementing pipe reader/writer endpoint counts so multi-stage shell pipelines (`cmd1 | cmd2 | cmd3`) detect `EOF` and `EPIPE` accurately.
- **`sys_execve` (`72`)**:
  - Parses and validates the target ELF64 binary (`aether_elf::parse_and_validate_elf64`), evaluates `execute_as_uuid` / `S_ISUID` / `S_ISGID` transitions (`security::compute_exec_credentials`), builds a fresh user `AddressSpace` with a guarded 32 KiB stack (`argv`, `envp`, `auxv`), closes descriptors marked `O_CLOEXEC`, activates the new `CR3`, and destroys the previous `AddressSpace`.
