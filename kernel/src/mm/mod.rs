pub mod heap;
pub mod pmm;
pub mod vmm;

pub const PAGE_SIZE: usize = 4096;

#[inline]
pub const fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}

#[inline]
pub const fn align_down(addr: u64, align: u64) -> u64 {
    addr & !(align - 1)
}
