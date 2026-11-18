//! Process, Thread, and Preemptive CPU Scheduler Subsystem.
//!
//! Features:
//! - Timer-driven preemptive priority round-robin scheduling (1000 Hz PIT tick on vector 32).
//! - Software yield/block trap on vector `0x81` (`129`) using the uniform `TrapFrame` mechanism.
//! - Kernel threads and isolated Ring 3 user processes.
//! - Per-process File Descriptor Table (`fds`) supporting VFS files, pipes, sockets, and shm.
//! - Sleeping (`sleep_ms`), blocking (`WaitQueue`), wake-up, exit codes, and `waitpid` reaping.
//! - Dedicated priority-0 idle thread (`idle/0`) ensuring the run queue is never empty.

pub mod exec;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::arch::asm;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use aether_abi::caps::{CAP_ALL, CAP_KILL, CAP_SETGID, CAP_SETUID};
use aether_abi::signal::{SIGFPE, SIGILL, SIGKILL, SIGSEGV};
use aether_abi::{Errno, ProcInfo};
use aether_uuid::Uuid;

use crate::arch::x86_64::idt::{self, TrapFrame};
use crate::arch::x86_64::paging::{PTE_NO_EXECUTE, PTE_PRESENT, PTE_USER, PTE_WRITABLE};
use crate::arch::x86_64::{gdt, hlt, percpu, sti, timer};
use crate::fs::{default_stdio_fds, FileDescriptorEntry, OpenFile};
use crate::mm::pmm;
use crate::mm::vmm::{self, AddressSpace};
use crate::mm::PAGE_SIZE;
use crate::sync::IrqSpinLock;

/// Software interrupt vector used for voluntary yield / sleep / block transitions.
pub const SCHED_YIELD_VECTOR: u8 = 0x81;

pub const PRIORITY_IDLE: u8 = 0;
pub const PRIORITY_LOW: u8 = 8;
pub const PRIORITY_NORMAL: u8 = 16;
pub const PRIORITY_HIGH: u8 = 24;
pub const PRIORITY_REALTIME: u8 = 31;

pub const DEFAULT_QUANTUM_TICKS: u32 = 5; // 5 ms default time slice at 1000 Hz
const KERNEL_STACK_PAGES: usize = 16; // 64 KiB + 4 KiB guard page

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping { wake_tick: u64 },
    Blocked,
    Zombie { exit_code: i32 },
}

impl TaskState {
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Ready => 1,
            Self::Running => 2,
            Self::Sleeping { .. } => 3,
            Self::Blocked => 4,
            Self::Zombie { .. } => 5,
        }
    }
}

#[derive(Clone)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub suid: u32,
    pub sgid: u32,
    pub user_uuid: Uuid,
    pub group_uuid: Uuid,
    pub groups: Vec<u32>,
    pub caps_effective: u64,
    pub caps_permitted: u64,
    pub caps_bounding: u64,
    pub no_new_privs: bool,
}

impl Credentials {
    pub fn root() -> Self {
        Self {
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            suid: 0,
            sgid: 0,
            user_uuid: Uuid::ROOT_USER,
            group_uuid: Uuid::ROOT_GROUP,
            groups: alloc::vec![0],
            caps_effective: CAP_ALL,
            caps_permitted: CAP_ALL,
            caps_bounding: CAP_ALL,
            no_new_privs: false,
        }
    }
}

pub struct Task {
    pub tid: u32,
    pub pid: u32,
    pub ppid: u32,
    pub name: [u8; 32],
    pub state: TaskState,
    pub priority: u8,
    pub quantum_ticks: u32,
    pub remaining_ticks: u32,
    pub cpu_ticks: u64,
    pub context_switches: u64,
    pub is_user: bool,
    pub address_space: AddressSpace,
    pub kernel_stack_top: u64,
    pub kernel_rsp: u64,
    pub user_brk_base: u64,
    pub user_brk_current: u64,
    pub user_mmap_next: u64,
    pub cwd: String,
    pub creds: Credentials,
    pub fds: Vec<Option<FileDescriptorEntry>>,
    pub pending_signals: u64,
}

pub const MAX_FDS_PER_PROCESS: usize = 64;

impl Task {
    pub fn set_name(&mut self, s: &str) {
        self.name = [0; 32];
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        self.name[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("?")
    }

    pub fn alloc_fd(&mut self, file: Arc<OpenFile>, cloexec: bool) -> Result<u32, Errno> {
        for (idx, slot) in self.fds.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(FileDescriptorEntry { file, cloexec });
                return Ok(idx as u32);
            }
        }
        if self.fds.len() >= MAX_FDS_PER_PROCESS {
            return Err(Errno::Emfile);
        }
        let idx = self.fds.len();
        self.fds.push(Some(FileDescriptorEntry { file, cloexec }));
        Ok(idx as u32)
    }

    pub fn get_fd(&self, fd: u32) -> Result<Arc<OpenFile>, Errno> {
        let idx = fd as usize;
        self.fds
            .get(idx)
            .and_then(|s| s.as_ref())
            .map(|e| Arc::clone(&e.file))
            .ok_or(Errno::Ebadf)
    }

    pub fn close_fd(&mut self, fd: u32) -> Result<(), Errno> {
        let idx = fd as usize;
        let slot = self.fds.get_mut(idx).ok_or(Errno::Ebadf)?;
        if slot.take().is_some() {
            Ok(())
        } else {
            Err(Errno::Ebadf)
        }
    }

    pub fn close_cloexec_fds(&mut self) {
        for slot in &mut self.fds {
            if let Some(entry) = slot {
                if entry.cloexec {
                    *slot = None;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SchedulerStats {
    pub total_tasks: usize,
    pub ready_tasks: usize,
    pub sleeping_tasks: usize,
    pub blocked_tasks: usize,
    pub zombie_tasks: usize,
    pub context_switches: u64,
    pub preemptions: u64,
}

struct Scheduler {
    tasks: Vec<Task>,
    current_idx: usize,
    rr_cursor: usize,
    total_context_switches: u64,
    total_preemptions: u64,
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            tasks: Vec::new(),
            current_idx: 0,
            rr_cursor: 0,
            total_context_switches: 0,
            total_preemptions: 0,
        }
    }

    fn wake_expired_sleepers(&mut self, now_tick: u64) {
        for task in &mut self.tasks {
            if let TaskState::Sleeping { wake_tick } = task.state {
                if now_tick >= wake_tick {
                    task.state = TaskState::Ready;
                    task.remaining_ticks = task.quantum_ticks;
                }
            }
        }
    }

    /// Select the next runnable task index using priority scheduling with round-robin fairness
    /// among tasks of equal priority.
    fn pick_next_runnable(&mut self) -> usize {
        let n = self.tasks.len();
        debug_assert!(n > 0, "Scheduler has no tasks");

        // Find maximum priority among all Ready (or still Running) tasks
        let mut max_prio: Option<u8> = None;
        for t in &self.tasks {
            if matches!(t.state, TaskState::Ready | TaskState::Running) {
                max_prio = Some(match max_prio {
                    Some(p) => p.max(t.priority),
                    None => t.priority,
                });
            }
        }

        let target_prio = max_prio.unwrap_or(PRIORITY_IDLE);
        let start = (self.rr_cursor + 1) % n;
        for offset in 0..n {
            let idx = (start + offset) % n;
            let t = &self.tasks[idx];
            if matches!(t.state, TaskState::Ready | TaskState::Running) && t.priority == target_prio
            {
                self.rr_cursor = idx;
                return idx;
            }
        }

        // Fallback to idle thread (index 1) or boot thread (index 0)
        0
    }

    fn schedule_from_isr(
        &mut self,
        current_frame: &mut TrapFrame,
        is_timer_tick: bool,
    ) -> *mut TrapFrame {
        if self.tasks.is_empty() {
            return current_frame as *mut TrapFrame;
        }

        let cur_idx = self.current_idx;
        self.tasks[cur_idx].kernel_rsp = current_frame as *mut TrapFrame as u64;

        let mut should_reschedule = !is_timer_tick;
        if is_timer_tick {
            self.tasks[cur_idx].cpu_ticks += 1;
            if self.tasks[cur_idx].remaining_ticks > 1 {
                self.tasks[cur_idx].remaining_ticks -= 1;
            } else {
                self.tasks[cur_idx].remaining_ticks = self.tasks[cur_idx].quantum_ticks;
                should_reschedule = true;
            }
        }

        // Always reschedule if the current task is no longer Running or if a higher-priority task woke up
        let cur_prio = self.tasks[cur_idx].priority;
        if !matches!(self.tasks[cur_idx].state, TaskState::Running) {
            should_reschedule = true;
        } else {
            for t in &self.tasks {
                if matches!(t.state, TaskState::Ready) && t.priority > cur_prio {
                    should_reschedule = true;
                    break;
                }
            }
        }

        if !should_reschedule {
            return current_frame as *mut TrapFrame;
        }

        if matches!(self.tasks[cur_idx].state, TaskState::Running) {
            self.tasks[cur_idx].state = TaskState::Ready;
        }

        let next_idx = self.pick_next_runnable();
        self.tasks[next_idx].state = TaskState::Running;
        if self.tasks[next_idx].remaining_ticks == 0 {
            self.tasks[next_idx].remaining_ticks = self.tasks[next_idx].quantum_ticks;
        }

        if next_idx != cur_idx {
            self.current_idx = next_idx;
            self.total_context_switches += 1;
            if is_timer_tick {
                self.total_preemptions += 1;
            }
            self.tasks[next_idx].context_switches += 1;
            CURRENT_TID.store(self.tasks[next_idx].tid, Ordering::Relaxed);
            CURRENT_PID.store(self.tasks[next_idx].pid, Ordering::Relaxed);

            let next_stack_top = self.tasks[next_idx].kernel_stack_top;
            let next_aspace = self.tasks[next_idx].address_space;
            if next_stack_top != 0 {
                gdt::set_kernel_stack(next_stack_top);
                percpu::set_kernel_stack(next_stack_top);
            }
            next_aspace.activate();
        }

        self.tasks[next_idx].kernel_rsp as *mut TrapFrame
    }
}

static SCHED: IrqSpinLock<Scheduler> = IrqSpinLock::new(Scheduler::new());
static SCHED_ACTIVE: AtomicBool = AtomicBool::new(false);
static NEXT_TID: AtomicU32 = AtomicU32::new(1);
static CURRENT_TID: AtomicU32 = AtomicU32::new(0);
static CURRENT_PID: AtomicU32 = AtomicU32::new(0);
static LAST_SWITCH_COUNT: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn is_scheduler_active() -> bool {
    SCHED_ACTIVE.load(Ordering::Relaxed)
}

#[inline]
pub fn current_tid() -> u32 {
    CURRENT_TID.load(Ordering::Relaxed)
}

#[inline]
pub fn current_pid() -> u32 {
    CURRENT_PID.load(Ordering::Relaxed)
}

pub fn alloc_tid() -> u32 {
    NEXT_TID.fetch_add(1, Ordering::Relaxed)
}

/// Entry trampoline for all newly spawned kernel threads.
extern "C" fn kernel_thread_trampoline(entry_ptr: usize, arg: usize) -> ! {
    let func: fn(usize) = unsafe { core::mem::transmute(entry_ptr) };
    func(arg);
    exit_current(0)
}

/// Priority-0 idle thread executed when all higher-priority tasks are sleeping or blocked.
fn idle_thread_main(_arg: usize) {
    loop {
        sti();
        hlt();
    }
}

fn on_yield_trap(frame: &mut TrapFrame) -> *mut TrapFrame {
    let mut sched = SCHED.lock();
    let out = sched.schedule_from_isr(frame, false);
    LAST_SWITCH_COUNT.store(sched.total_context_switches, Ordering::Relaxed);
    out
}

/// Timer interrupt hook called at 1000 Hz from `arch::x86_64::timer`.
pub fn on_timer_tick(frame: &mut TrapFrame, now_tick: u64) -> *mut TrapFrame {
    if !is_scheduler_active() {
        return frame as *mut TrapFrame;
    }
    let mut sched = SCHED.lock();
    sched.wake_expired_sleepers(now_tick);
    let out = sched.schedule_from_isr(frame, true);
    LAST_SWITCH_COUNT.store(sched.total_context_switches, Ordering::Relaxed);
    out
}

/// Initialize the scheduler, register the bootstrap thread (`tid=1`) and `idle/0` thread (`tid=2`),
/// and install the software yield trap handler on vector `0x81`.
pub fn init() -> Result<(), &'static str> {
    idt::register_handler(SCHED_YIELD_VECTOR, on_yield_trap);
    idt::register_user_fault_hook(on_user_fault_terminate);

    let boot_tid = alloc_tid(); // TID 1
    let mut boot_task = Task {
        tid: boot_tid,
        pid: boot_tid,
        ppid: 0,
        name: [0; 32],
        state: TaskState::Running,
        priority: PRIORITY_NORMAL,
        quantum_ticks: DEFAULT_QUANTUM_TICKS,
        remaining_ticks: DEFAULT_QUANTUM_TICKS,
        cpu_ticks: 0,
        context_switches: 0,
        is_user: false,
        address_space: AddressSpace::kernel(),
        kernel_stack_top: 0,
        kernel_rsp: 0,
        user_brk_base: 0,
        user_brk_current: 0,
        user_mmap_next: 0x0000_4000_0000_0000,
        cwd: String::from("/"),
        creds: Credentials::root(),
        fds: default_stdio_fds(),
        pending_signals: 0,
    };
    boot_task.set_name("kthreadd/boot");

    {
        let mut sched = SCHED.lock();
        sched.tasks.push(boot_task);
        sched.current_idx = 0;
    }

    CURRENT_TID.store(boot_tid, Ordering::Relaxed);
    CURRENT_PID.store(boot_tid, Ordering::Relaxed);

    // Spawn the dedicated priority-0 idle thread (`idle/0`)
    spawn_kernel_thread("idle/0", PRIORITY_IDLE, idle_thread_main, 0)?;

    SCHED_ACTIVE.store(true, Ordering::Release);
    Ok(())
}

/// Spawn a new preemptible kernel thread with a 16 KiB stack and 4 KiB guard page.
pub fn spawn_kernel_thread(
    name: &str,
    priority: u8,
    entry: fn(usize),
    arg: usize,
) -> Result<u32, &'static str> {
    let stack_top = vmm::alloc_guarded_kernel_stack(KERNEL_STACK_PAGES)
        .map_err(|_| "Scheduler: failed to allocate guarded kernel stack")?;
    let frame_addr = (stack_top - (size_of::<TrapFrame>() as u64)) & !0xF;
    let frame_ptr = frame_addr as *mut TrapFrame;

    unsafe {
        frame_ptr.write(TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: entry as usize as u64,
            rsi: arg as u64,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            vector: SCHED_YIELD_VECTOR as u64,
            error_code: 0,
            rip: kernel_thread_trampoline as *const () as usize as u64,
            cs: gdt::KERNEL_CS as u64,
            rflags: 0x202, // IF=1 (interrupts enabled)
            rsp: stack_top,
            ss: gdt::KERNEL_DS as u64,
        });
    }

    let tid = alloc_tid();
    let ppid = current_pid();
    let prio = priority.min(PRIORITY_REALTIME);
    let mut task = Task {
        tid,
        pid: tid,
        ppid,
        name: [0; 32],
        state: TaskState::Ready,
        priority: prio,
        quantum_ticks: DEFAULT_QUANTUM_TICKS,
        remaining_ticks: DEFAULT_QUANTUM_TICKS,
        cpu_ticks: 0,
        context_switches: 0,
        is_user: false,
        address_space: AddressSpace::kernel(),
        kernel_stack_top: stack_top,
        kernel_rsp: frame_addr,
        user_brk_base: 0,
        user_brk_current: 0,
        user_mmap_next: 0x0000_4000_0000_0000,
        cwd: String::from("/"),
        creds: Credentials::root(),
        fds: default_stdio_fds(),
        pending_signals: 0,
    };
    task.set_name(name);

    SCHED.lock().tasks.push(task);
    Ok(tid)
}

/// Voluntarily yield the CPU to the next runnable task.
#[inline]
pub fn yield_now() {
    if !is_scheduler_active() {
        return;
    }
    unsafe {
        asm!("int {vec}", vec = const SCHED_YIELD_VECTOR, options(nostack));
    }
}

/// Put the calling task to sleep for `duration_ms` milliseconds (woken by the 1000 Hz timer).
pub fn sleep_ms(duration_ms: u64) {
    if duration_ms == 0 || !is_scheduler_active() {
        yield_now();
        return;
    }
    let wake_tick = timer::ticks().saturating_add(duration_ms);
    {
        let mut sched = SCHED.lock();
        let cur = sched.current_idx;
        sched.tasks[cur].state = TaskState::Sleeping { wake_tick };
    }
    yield_now();
}

/// Block the currently executing task (called by `WaitQueue::wait`).
pub fn block_current_task() {
    if !is_scheduler_active() {
        return;
    }
    {
        let mut sched = SCHED.lock();
        let cur = sched.current_idx;
        sched.tasks[cur].state = TaskState::Blocked;
    }
    yield_now();
}

/// Wake a blocked or sleeping task by TID, transitioning it to `TaskState::Ready`.
pub fn wake_task(tid: u32) -> bool {
    let mut sched = SCHED.lock();
    for task in &mut sched.tasks {
        if task.tid == tid && matches!(task.state, TaskState::Blocked | TaskState::Sleeping { .. })
        {
            task.state = TaskState::Ready;
            task.remaining_ticks = task.quantum_ticks;
            return true;
        }
    }
    false
}

/// Change a task's scheduling priority (`0..=31`).
pub fn set_priority(tid: u32, new_priority: u8) -> bool {
    let prio = new_priority.min(PRIORITY_REALTIME);
    let mut sched = SCHED.lock();
    for task in &mut sched.tasks {
        if task.tid == tid {
            task.priority = prio;
            return true;
        }
    }
    false
}

pub fn get_priority(tid: u32) -> Option<u8> {
    let sched = SCHED.lock();
    sched
        .tasks
        .iter()
        .find(|t| t.tid == tid)
        .map(|t| t.priority)
}

/// Terminate the currently executing task with `exit_code` and switch to the next runnable task.
pub fn exit_current(exit_code: i32) -> ! {
    {
        let mut sched = SCHED.lock();
        let cur = sched.current_idx;
        sched.tasks[cur].fds.clear();
        sched.tasks[cur].state = TaskState::Zombie { exit_code };
    }
    yield_now();
    crate::arch::x86_64::halt_loop()
}

/// Wait for a specific child/thread `tid` to exit and reap its `Zombie` entry, returning its `exit_code`.
pub fn wait_tid(target_tid: u32) -> Result<i32, &'static str> {
    loop {
        {
            let mut sched = SCHED.lock();
            let mut found_idx = None;
            let mut exit_code = 0;
            for (idx, t) in sched.tasks.iter().enumerate() {
                if t.tid == target_tid {
                    if let TaskState::Zombie { exit_code: code } = t.state {
                        found_idx = Some(idx);
                        exit_code = code;
                    } else {
                        found_idx = Some(usize::MAX); // still alive
                    }
                    break;
                }
            }

            match found_idx {
                Some(usize::MAX) => {} // still running; sleep/yield below
                Some(idx) => {
                    // Reap zombie task (adjust current_idx if needed)
                    let removed = sched.tasks.remove(idx);
                    if removed.is_user {
                        removed.address_space.destroy_user();
                    }
                    if idx < sched.current_idx {
                        sched.current_idx -= 1;
                    }
                    return Ok(exit_code);
                }
                None => return Err("wait_tid: no such task"),
            }
        }
        sleep_ms(1);
    }
}

pub fn stats() -> SchedulerStats {
    let sched = SCHED.lock();
    let mut ready = 0;
    let mut sleeping = 0;
    let mut blocked = 0;
    let mut zombie = 0;
    for t in &sched.tasks {
        match t.state {
            TaskState::Ready | TaskState::Running => ready += 1,
            TaskState::Sleeping { .. } => sleeping += 1,
            TaskState::Blocked => blocked += 1,
            TaskState::Zombie { .. } => zombie += 1,
        }
    }
    SchedulerStats {
        total_tasks: sched.tasks.len(),
        ready_tasks: ready,
        sleeping_tasks: sleeping,
        blocked_tasks: blocked,
        zombie_tasks: zombie,
        context_switches: sched.total_context_switches,
        preemptions: sched.total_preemptions,
    }
}

/// Fatal user-mode exception hook installed in the IDT (Section 3.3).
/// Terminates ONLY the offending Ring 3 user process with exit code `128 + signal`
/// and immediately schedules the next runnable task without crashing the kernel.
fn on_user_fault_terminate(frame: &mut TrapFrame) -> *mut TrapFrame {
    let sig = match frame.vector {
        0 | 16 | 19 => SIGFPE,
        6 => SIGILL,
        _ => SIGSEGV,
    };
    let exit_code = 128 + (sig as i32);
    let mut sched = SCHED.lock();
    let cur = sched.current_idx;
    let pid = sched.tasks[cur].pid;
    crate::kwarn!(
        "Scheduler: terminating faulting user process PID={} ('{}') vec={} RIP={:#018x} exit_code={}",
        pid,
        sched.tasks[cur].name_str(),
        frame.vector,
        frame.rip,
        exit_code
    );
    sched.tasks[cur].fds.clear();
    sched.tasks[cur].state = TaskState::Zombie { exit_code };
    sched.schedule_from_isr(frame, false)
}

/// Terminate the currently executing task directly from an in-flight `TrapFrame` (e.g. `sys_exit`).
pub fn exit_current_from_frame(frame: &mut TrapFrame, exit_code: i32) -> *mut TrapFrame {
    let mut sched = SCHED.lock();
    let cur = sched.current_idx;
    sched.tasks[cur].fds.clear();
    sched.tasks[cur].state = TaskState::Zombie { exit_code };
    sched.schedule_from_isr(frame, false)
}

/// Access or mutate the currently executing `Task` under the scheduler lock.
pub fn with_current_task<R>(f: impl FnOnce(&Task) -> R) -> R {
    let sched = SCHED.lock();
    f(&sched.tasks[sched.current_idx])
}

pub fn with_current_task_mut<R>(f: impl FnOnce(&mut Task) -> R) -> R {
    let mut sched = SCHED.lock();
    let idx = sched.current_idx;
    f(&mut sched.tasks[idx])
}

/// Spawn an isolated Ring 3 user process from position-independent machine code bytes.
/// Maps:
/// - Code at `0x0000_0000_0040_0000` (`PTE_PRESENT | PTE_USER`, RX, non-writable W^X!).
/// - User stack at `0x0000_7FFF_FFFE_0000..0x0000_7FFF_FFFF_0000` (64 KiB, RW + NX + bottom guard page!).
/// - User heap `brk` base at `0x0000_0000_0080_0000`.
pub fn spawn_user_code_process(
    name: &str,
    code: &[u8],
    creds: Credentials,
) -> Result<u32, &'static str> {
    if code.is_empty() || code.len() > PAGE_SIZE * 4 {
        return Err("spawn_user_code_process: invalid code size");
    }

    let aspace = AddressSpace::new_user().map_err(|_| "failed to create user AddressSpace")?;

    // 1. Map user code page(s) at 0x0000_0000_0040_0000 as RX (W^X enforced)
    let code_base: u64 = 0x0000_0000_0040_0000;
    let code_pages = code.len().div_ceil(PAGE_SIZE);
    for i in 0..code_pages {
        let phys = pmm::alloc_zeroed_frame().map_err(|_| "out of frames for user code")?;
        let chunk_start = i * PAGE_SIZE;
        let chunk_end = (chunk_start + PAGE_SIZE).min(code.len());
        unsafe {
            core::ptr::copy_nonoverlapping(
                code[chunk_start..chunk_end].as_ptr(),
                pmm::phys_to_virt(phys),
                chunk_end - chunk_start,
            );
        }
        aspace
            .map_page(
                code_base + (i as u64) * (PAGE_SIZE as u64),
                phys,
                PTE_PRESENT | PTE_USER, // RX (not writable, executable)
            )
            .map_err(|_| "failed to map user code page")?;
    }

    // 2. Map user stack with bottom guard page + 4 pages (16 KiB) of RW+NX user stack
    let stack_guard_vaddr: u64 = 0x0000_7FFF_FFFE_B000;
    let stack_top_vaddr: u64 = 0x0000_7FFF_FFFF_0000;
    aspace
        .map_guard_page(stack_guard_vaddr)
        .map_err(|_| "failed to map user stack guard page")?;
    for i in 1..=4u64 {
        let v = stack_guard_vaddr + i * (PAGE_SIZE as u64);
        let phys = pmm::alloc_zeroed_frame().map_err(|_| "out of frames for user stack")?;
        aspace
            .map_page(
                v,
                phys,
                PTE_PRESENT | PTE_WRITABLE | PTE_USER | PTE_NO_EXECUTE,
            )
            .map_err(|_| "failed to map user stack page")?;
    }

    // 3. Allocate guarded Ring 0 kernel stack for syscalls and interrupts
    let kstack_top = vmm::alloc_guarded_kernel_stack(KERNEL_STACK_PAGES)
        .map_err(|_| "failed to alloc user process kernel stack")?;
    let frame_addr = (kstack_top - (size_of::<TrapFrame>() as u64)) & !0xF;
    let frame_ptr = frame_addr as *mut TrapFrame;

    unsafe {
        frame_ptr.write(TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0x202,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: code_base,
            rbx: 0,
            rax: 0,
            vector: SCHED_YIELD_VECTOR as u64,
            error_code: 0,
            rip: code_base,
            cs: gdt::USER_CS as u64, // 0x2B (Ring 3!)
            rflags: 0x202,           // IF=1
            rsp: stack_top_vaddr - 16,
            ss: gdt::USER_DS as u64, // 0x23 (Ring 3!)
        });
    }

    let tid = alloc_tid();
    let ppid = current_pid();
    let mut task = Task {
        tid,
        pid: tid,
        ppid,
        name: [0; 32],
        state: TaskState::Ready,
        priority: PRIORITY_NORMAL,
        quantum_ticks: DEFAULT_QUANTUM_TICKS,
        remaining_ticks: DEFAULT_QUANTUM_TICKS,
        cpu_ticks: 0,
        context_switches: 0,
        is_user: true,
        address_space: aspace,
        kernel_stack_top: kstack_top,
        kernel_rsp: frame_addr,
        user_brk_base: 0x0000_0000_0080_0000,
        user_brk_current: 0x0000_0000_0080_0000,
        user_mmap_next: 0x0000_4000_0000_0000,
        cwd: String::from("/"),
        creds,
        fds: default_stdio_fds(),
        pending_signals: 0,
    };
    task.set_name(name);

    SCHED.lock().tasks.push(task);
    Ok(tid)
}

/// Fork the currently executing user process (`sys_fork`) using Copy-on-Write (`clone_cow`).
/// Returns `Ok(child_pid)` to the parent, while the child wakes in Ring 3 with `RAX = 0`.
pub fn fork_current_from_frame(parent_frame: &TrapFrame) -> Result<u32, Errno> {
    let (
        parent_pid,
        parent_name,
        parent_prio,
        parent_aspace,
        parent_brk_base,
        parent_brk_cur,
        parent_mmap,
        parent_cwd,
        parent_creds,
        parent_fds,
    ) = {
        let sched = SCHED.lock();
        let cur = &sched.tasks[sched.current_idx];
        if !cur.is_user {
            return Err(Errno::Einval);
        }
        (
            cur.pid,
            cur.name,
            cur.priority,
            cur.address_space,
            cur.user_brk_base,
            cur.user_brk_current,
            cur.user_mmap_next,
            cur.cwd.clone(),
            cur.creds.clone(),
            cur.fds.clone(),
        )
    };

    let child_aspace = parent_aspace.clone_cow().map_err(|_| Errno::Enomem)?;
    let kstack_top =
        vmm::alloc_guarded_kernel_stack(KERNEL_STACK_PAGES).map_err(|_| Errno::Enomem)?;
    let frame_addr = (kstack_top - (size_of::<TrapFrame>() as u64)) & !0xF;
    let frame_ptr = frame_addr as *mut TrapFrame;

    let mut child_frame = *parent_frame;
    child_frame.rax = 0; // fork() returns 0 in the child!
    unsafe {
        frame_ptr.write(child_frame);
    }

    let child_tid = alloc_tid();
    let child_task = Task {
        tid: child_tid,
        pid: child_tid,
        ppid: parent_pid,
        name: parent_name,
        state: TaskState::Ready,
        priority: parent_prio,
        quantum_ticks: DEFAULT_QUANTUM_TICKS,
        remaining_ticks: DEFAULT_QUANTUM_TICKS,
        cpu_ticks: 0,
        context_switches: 0,
        is_user: true,
        address_space: child_aspace,
        kernel_stack_top: kstack_top,
        kernel_rsp: frame_addr,
        user_brk_base: parent_brk_base,
        user_brk_current: parent_brk_cur,
        user_mmap_next: parent_mmap,
        cwd: parent_cwd,
        creds: parent_creds,
        fds: parent_fds,
        pending_signals: 0,
    };

    SCHED.lock().tasks.push(child_task);
    Ok(child_tid)
}

/// Wait for a child process (`target_pid <= 0` for any child of `parent_pid`, or specific `target_pid > 0`).
/// Returns `Ok((child_pid, exit_code))` or `Err(Errno::Echild)`.
pub fn wait4_child(target_pid: i32, nonblocking: bool) -> Result<(u32, i32), Errno> {
    let parent_pid = current_pid();
    loop {
        {
            let mut sched = SCHED.lock();
            let mut has_matching_child = false;
            let mut zombie_idx_and_info: Option<(usize, u32, i32)> = None;

            for (idx, t) in sched.tasks.iter().enumerate() {
                if !t.is_user {
                    continue;
                }
                let matches = if target_pid > 0 {
                    t.pid == (target_pid as u32) && (t.ppid == parent_pid || parent_pid == 1)
                } else {
                    t.ppid == parent_pid && t.pid != parent_pid
                };
                if matches {
                    has_matching_child = true;
                    if let TaskState::Zombie { exit_code } = t.state {
                        zombie_idx_and_info = Some((idx, t.pid, exit_code));
                        break;
                    }
                }
            }

            if let Some((idx, reaped_pid, exit_code)) = zombie_idx_and_info {
                let removed = sched.tasks.remove(idx);
                if removed.is_user {
                    removed.address_space.destroy_user();
                }
                if idx < sched.current_idx {
                    sched.current_idx -= 1;
                }
                return Ok((reaped_pid, exit_code));
            }

            if !has_matching_child {
                return Err(Errno::Echild);
            }
            if nonblocking {
                return Ok((0, 0));
            }
        }
        sleep_ms(1);
    }
}

/// Send a signal to `target_pid` (`sys_kill`), enforcing UID / `CAP_KILL` authorization.
pub fn kill_process(target_pid: u32, sig: u32) -> Result<(), Errno> {
    if sig > 64 {
        return Err(Errno::Einval);
    }
    let (caller_euid, caller_caps) = with_current_task(|t| (t.creds.euid, t.creds.caps_effective));

    let mut sched = SCHED.lock();
    let target_idx = sched
        .tasks
        .iter()
        .position(|t| t.pid == target_pid)
        .ok_or(Errno::Esrch)?;

    if target_pid == 1 && sig == SIGKILL {
        return Err(Errno::Eperm); // PID 1 (init) is protected from SIGKILL
    }

    let target_uid = sched.tasks[target_idx].creds.uid;
    if caller_euid != 0 && caller_euid != target_uid && (caller_caps & CAP_KILL) == 0 {
        return Err(Errno::Eperm);
    }

    if sig == 0 {
        return Ok(()); // Existence & permission check only
    }

    sched.tasks[target_idx].pending_signals |= 1u64 << (sig - 1);
    if sig == SIGKILL || sig == aether_abi::signal::SIGTERM {
        sched.tasks[target_idx].state = TaskState::Zombie {
            exit_code: 128 + (sig as i32),
        };
    } else if matches!(
        sched.tasks[target_idx].state,
        TaskState::Blocked | TaskState::Sleeping { .. }
    ) {
        sched.tasks[target_idx].state = TaskState::Ready;
    }

    Ok(())
}

/// Populate up to `out.len()` `ProcInfo` process table snapshots.
pub fn list_processes(out: &mut [ProcInfo]) -> usize {
    let sched = SCHED.lock();
    let count = sched.tasks.len().min(out.len());
    for (slot, t) in out.iter_mut().zip(sched.tasks.iter()).take(count) {
        *slot = ProcInfo {
            pid: t.pid,
            ppid: t.ppid,
            uid: t.creds.uid,
            gid: t.creds.gid,
            euid: t.creds.euid,
            egid: t.creds.egid,
            state: t.state.as_u32(),
            priority: t.priority as u32,
            user_uuid: *t.creds.user_uuid.as_bytes(),
            cpu_ticks: t.cpu_ticks,
            name: t.name,
        };
    }
    count
}

/// Helper for `sys_setuid` enforcing POSIX + capability rules.
pub fn sys_setuid_current(new_uid: u32, new_uuid: Uuid) -> Result<(), Errno> {
    with_current_task_mut(|t| {
        let is_root = t.creds.euid == 0 || (t.creds.caps_effective & CAP_SETUID) != 0;
        if is_root {
            t.creds.uid = new_uid;
            t.creds.euid = new_uid;
            t.creds.suid = new_uid;
            t.creds.user_uuid = new_uuid;
            if new_uid != 0 {
                // Dropping root drops effective capabilities
                t.creds.caps_effective = 0;
                t.creds.caps_permitted = 0;
            }
            Ok(())
        } else if new_uid == t.creds.uid || new_uid == t.creds.suid {
            t.creds.euid = new_uid;
            t.creds.user_uuid = new_uuid;
            Ok(())
        } else {
            Err(Errno::Eperm)
        }
    })
}

/// Helper for `sys_setgid` enforcing POSIX + capability rules.
pub fn sys_setgid_current(new_gid: u32, new_group_uuid: Uuid) -> Result<(), Errno> {
    with_current_task_mut(|t| {
        let is_root = t.creds.euid == 0 || (t.creds.caps_effective & CAP_SETGID) != 0;
        if is_root {
            t.creds.gid = new_gid;
            t.creds.egid = new_gid;
            t.creds.sgid = new_gid;
            t.creds.group_uuid = new_group_uuid;
            Ok(())
        } else if new_gid == t.creds.gid || new_gid == t.creds.sgid {
            t.creds.egid = new_gid;
            t.creds.group_uuid = new_group_uuid;
            Ok(())
        } else {
            Err(Errno::Eperm)
        }
    })
}
