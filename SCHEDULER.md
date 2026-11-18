# AetherOS Preemptive CPU Scheduler

See also [PROCESS_AND_SCHEDULING.md](file:///home/ubeyt/Belgeler/AetherOS/PROCESS_AND_SCHEDULING.md).

## 1. Preemptive Scheduling Flow (`kernel/src/sched/mod.rs`)

```mermaid
flowchart TD
    IRQ["1000 Hz PIT Timer IRQ0 (Vector 32) or Voluntary Yield Trap (Vector 0x81)"]
    SAVE["isr_common_stub saves RAX..R15 into TrapFrame on current kernel stack"]
    WAKE["Scan tasks: wake Sleeping(wake_tick <= now_ticks) -> Ready"]
    QDEC["Decrement running task ticks_remaining (10 ms default quantum)"]
    PICK["Select highest-priority Ready task (PRIORITY_HIGH > NORMAL > LOW > IDLE) round-robin"]
    TSS["Update TSS.rsp0 & Per-CPU gs:[0] to next_task.kernel_stack_top"]
    CR3["If next_task.address_space != current: reload CR3 (TLB context switch)"]
    IRET["isr_restore_trapframe: restore R15..RAX, conditional swapgs (if CS & 3 == 3), iretq"]

    IRQ --> SAVE --> WAKE --> QDEC --> PICK --> TSS --> CR3 --> IRET
```

## 2. Priority Levels & Fairness
- `PRIORITY_HIGH (3)`: Interactive and high-priority system tasks.
- `PRIORITY_NORMAL (2)`: Default user-space processes (`/sbin/init`, `/bin/aethersh`, utilities) and worker kernel threads.
- `PRIORITY_LOW (1)`: Background batch tasks.
- `PRIORITY_IDLE (0)`: `idle/0` `hlt` loop (selected only when no higher-priority task is `Ready`).
- **Preemption Verification**: `test_preemptive_multitasking_and_sync` spawns two competing CPU+Mutex threads without voluntary yields in their compute loops and verifies hardware timer preemptions (`preemptions=5, switches=12, mutex_val=250`).
