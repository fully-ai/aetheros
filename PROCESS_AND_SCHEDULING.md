# AetherOS Process & Preemptive Scheduling Architecture

## 1. Process and Thread Model (`kernel/src/sched/mod.rs`)

Each schedulable execution context in AetherOS is represented by a `Task` structure managed by the global `Scheduler` (`SCHED: IrqSpinLock<Scheduler>`):

| Field | Type | Description |
| :--- | :--- | :--- |
| `tid` / `pid` / `ppid` | `u64` | Thread ID, Process ID, and Parent Process ID |
| `name` | `[u8; 24]` | Fixed-size task name (`kthreadd/boot`, `idle/0`, `init`, `aethersh`, etc.) |
| `state` | `TaskState` | `Ready`, `Running`, `Sleeping(wake_tick)`, `Blocked`, `Zombie` |
| `priority` | `u8` | `PRIORITY_IDLE (0)`, `PRIORITY_LOW (1)`, `PRIORITY_NORMAL (2)`, `PRIORITY_HIGH (3)` |
| `ticks_remaining` | `u64` | Remaining time quantum in 1 ms PIT ticks (`DEFAULT_QUANTUM_TICKS = 10` ms) |
| `total_ticks` | `u64` | Cumulative CPU ticks consumed |
| `kernel_rsp` | `u64` | Saved Ring 0 stack pointer pointing to the task's `TrapFrame` |
| `kernel_stack_top` | `u64` | Top of the task's 24 KiB (`6 * 4 KiB`) dedicated kernel stack |
| `address_space` | `AddressSpace` | Per-process 4-level `CR3` page table (`PML4` physical address) |
| `is_user` | `bool` | `true` for Ring 3 processes (`CS=0x2B`), `false` for Ring 0 kernel threads (`CS=0x08`) |
| `user_brk_base` / `user_brk_current` | `u64` | User process `sys_brk` heap bounds |
| `user_mmap_next` | `u64` | Rolling virtual cursor (`0x0000_4000_0000_0000+`) for `sys_mmap` |
| `creds` | `Credentials` | Full UUID + POSIX credential set (`uid`, `gid`, `euid`, `egid`, `suid`, `sgid`, `user_uuid`, `group_uuid`, supplementary `groups`, capabilities, `no_new_privs`) |
| `cwd` / `umask` | `String` / `u32` | Process working directory and file creation mask (`0o022`) |
| `fds` | `Vec<Option<FileDescriptorEntry>>` | Per-process file descriptor table (`0=stdin`, `1=stdout`, `2=stderr`, pipes, files, sockets) |
| `pending_signals` | `u64` | Bitmask of pending signals (`SIGTERM`, `SIGKILL`, `SIGINT`, `SIGCHLD`, `SIGSEGV`, `SIGPIPE`) |

---

## 2. Preemptive Priority Round-Robin Scheduler

### Timer Interrupt (`IRQ0`, Vector 32, 1000 Hz) & Voluntary Trap (`int 0x81`)
1. Every 1 ms, the 8254 PIT triggers vector `32`, entering `isr_common_stub`, which saves all 15 general-purpose registers into a `TrapFrame` on the current task's kernel stack and calls `sched::on_timer_tick(frame)`.
2. `schedule_from_isr(current_frame, is_timer_tick)`:
   - Saves `current_frame as u64` into `tasks[cur_idx].kernel_rsp`.
   - Wakes any `TaskState::Sleeping(wake_tick)` task whose `wake_tick <= now_ticks`.
   - Decrements `ticks_remaining` on the active task; if `ticks_remaining > 0` and the task is still `Running` on a timer tick, it continues without switch overhead.
   - Otherwise selects the highest-priority `TaskState::Ready` task using round-robin rotation starting after `cur_idx` (falling back to `idle/0` if all tasks are blocked or sleeping).
3. **Context Switch**:
   - Updates `gdt::set_kernel_stack(next_stack_top)` (`TSS.rsp0`) and `percpu::set_syscall_kernel_rsp(next_stack_top)` (`gs:[0]`).
   - Switches `CR3` via `next_aspace.activate()` if `next_aspace != cur_aspace`.
   - Returns `next_rsp as *mut TrapFrame` to `isr_restore_trapframe`, which executes `swapgs` (if returning to Ring 3) and `iretq`.

---

## 3. Process Lifecycle Operations (`fork`, `execve`, `wait4`, `exit`)

- **`sys_fork` (`fork_current_from_frame`)**:
  Clones the parent's `AddressSpace` with Copy-on-Write (`AddressSpace::clone_cow`), allocates a new 24 KiB kernel stack, duplicates the parent's `TrapFrame` at the top of the child's kernel stack with `child.rax = 0`, clones the file descriptor table (`Arc<OpenFile>` refcounts and pipe reader/writer counts), and returns `child_pid` to the parent.
- **`sys_execve` (`sched::exec::execve_current_from_frame`)**:
  Validates the target ELF64 binary via `aether_elf::parse_and_validate_elf64`, computes credential transitions (`security::compute_exec_credentials` for `execute_as_uuid` / `S_ISUID` / `S_ISGID`), maps `PT_LOAD` segments and a guarded 32 KiB Ring 3 stack (`argv`, `envp`, `auxv`) into a fresh `AddressSpace`, closes `O_CLOEXEC` file descriptors, destroys the old `AddressSpace`, and resets the `TrapFrame` to enter `_start` in Ring 3.
- **`sys_exit` & `sys_wait4`**:
  `exit_current_from_frame` closes all open file descriptors (decrementing pipe reader/writer counts so pipe peers observe `EOF`/`EPIPE`), transitions the task to `TaskState::Zombie` with `exit_code`, and delivers `SIGCHLD` to `ppid`. `wait4_child` reaps matching `Zombie` children, frees their user `AddressSpace` and kernel stack, and returns `(reaped_pid, exit_code)`.
