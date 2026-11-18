//! x86_64 CPU feature detection (CPUID), MSR access, control registers,
//! and baseline security hardening (NX, WP, PGE, SMEP, SYSCALL).

use core::arch::asm;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::fmt;

// Architectural Model-Specific Registers (MSRs)
pub const MSR_APIC_BASE: u32 = 0x0000_001B;
pub const MSR_EFER: u32 = 0xC000_0080;
pub const MSR_STAR: u32 = 0xC000_0081;
pub const MSR_LSTAR: u32 = 0xC000_0082;
pub const MSR_CSTAR: u32 = 0xC000_0083;
pub const MSR_SFMASK: u32 = 0xC000_0084;
/// Mask IF (bit 9), TF (bit 8), DF (bit 10), AC (bit 18) on SYSCALL entry.
pub const MSR_FMASK_BITS: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 18);
pub const MSR_FS_BASE: u32 = 0xC000_0100;
pub const MSR_GS_BASE: u32 = 0xC000_0101;
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

// EFER bits
pub const EFER_SCE: u64 = 1 << 0; // SYSCALL/SYSRET enable
pub const EFER_LME: u64 = 1 << 8; // Long mode enable
pub const EFER_LMA: u64 = 1 << 10; // Long mode active
pub const EFER_NXE: u64 = 1 << 11; // No-Execute enable

// CR0 bits
pub const CR0_WP: u64 = 1 << 16; // Write Protect (enforce RO in Ring 0)

// CR4 bits
pub const CR4_PGE: u64 = 1 << 7; // Page Global Enable
pub const CR4_OSFXSR: u64 = 1 << 9;
pub const CR4_OSXMMEXCPT: u64 = 1 << 10;
pub const CR4_FSGSBASE: u64 = 1 << 16;
pub const CR4_SMEP: u64 = 1 << 20; // Supervisor Mode Execution Prevention
pub const CR4_SMAP: u64 = 1 << 21; // Supervisor Mode Access Prevention

/// Read a 64-bit Model-Specific Register.
#[inline]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((high as u64) << 32) | (low as u64)
}

/// Write a 64-bit Model-Specific Register.
#[inline]
pub unsafe fn wrmsr(msr: u32, val: u64) {
    let low = val as u32;
    let high = (val >> 32) as u32;
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
}

#[inline]
pub fn read_cr0() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr0", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn write_cr0(val: u64) {
    unsafe { asm!("mov cr0, {}", in(reg) val, options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn read_cr2() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr2", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub fn read_cr3() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr3", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn write_cr3(val: u64) {
    unsafe { asm!("mov cr3, {}", in(reg) val, options(nostack, preserves_flags)) };
}

#[inline]
pub fn read_cr4() -> u64 {
    let val: u64;
    unsafe { asm!("mov {}, cr4", out(reg) val, options(nomem, nostack, preserves_flags)) };
    val
}

#[inline]
pub unsafe fn write_cr4(val: u64) {
    unsafe { asm!("mov cr4, {}", in(reg) val, options(nomem, nostack, preserves_flags)) };
}

#[inline]
pub fn rdtsc() -> u64 {
    let (low, high): (u32, u32);
    unsafe {
        asm!("rdtsc", out("eax") low, out("edx") high, options(nomem, nostack, preserves_flags));
    }
    ((high as u64) << 32) | (low as u64)
}

#[inline]
pub fn invlpg(vaddr: u64) {
    unsafe {
        asm!("invlpg [{}]", in(reg) vaddr, options(nostack, preserves_flags));
    }
}

/// Detected x86_64 processor capabilities.
#[derive(Clone, Copy)]
pub struct CpuFeatures {
    pub vendor: [u8; 12],
    pub brand: [u8; 48],
    pub max_basic_leaf: u32,
    pub max_ext_leaf: u32,
    pub fpu: bool,
    pub tsc: bool,
    pub msr: bool,
    pub pae: bool,
    pub apic: bool,
    pub x2apic: bool,
    pub pge: bool,
    pub pat: bool,
    pub fxsr: bool,
    pub sse: bool,
    pub sse2: bool,
    pub xsave: bool,
    pub rdrand: bool,
    pub nx: bool,
    pub syscall: bool,
    pub gigabyte_pages: bool,
    pub rdtscp: bool,
    pub smep: bool,
    pub smap: bool,
    pub fsgsbase: bool,
    pub la57: bool,
    pub invariant_tsc: bool,
    // Active state after early hardening
    pub nxe_enabled: bool,
    pub wp_enabled: bool,
    pub smep_enabled: bool,
    pub smap_enabled: bool,
}

impl CpuFeatures {
    pub fn vendor_str(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("Unknown")
    }

    pub fn brand_str(&self) -> &str {
        let s = core::str::from_utf8(&self.brand).unwrap_or("Unknown x86_64");
        s.trim_matches('\0').trim()
    }
}

impl fmt::Debug for CpuFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CPU({} '{}', apic={} x2apic={} nx={} syscall={} pge={} smep={} smap={} la57={} rdrand={} inv_tsc={})",
            self.vendor_str(),
            self.brand_str(),
            self.apic,
            self.x2apic,
            self.nx,
            self.syscall,
            self.pge,
            self.smep,
            self.smap,
            self.la57,
            self.rdrand,
            self.invariant_tsc,
        )
    }
}

/// Probe CPUID leaves and return detected features without assuming uniform hardware.
pub fn detect_features() -> CpuFeatures {
    let leaf0 = __cpuid(0);
    let max_basic = leaf0.eax;
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&leaf0.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&leaf0.edx.to_le_bytes());
    vendor[8..12].copy_from_slice(&leaf0.ecx.to_le_bytes());

    let mut features = CpuFeatures {
        vendor,
        brand: [0u8; 48],
        max_basic_leaf: max_basic,
        max_ext_leaf: 0,
        fpu: false,
        tsc: false,
        msr: false,
        pae: false,
        apic: false,
        x2apic: false,
        pge: false,
        pat: false,
        fxsr: false,
        sse: false,
        sse2: false,
        xsave: false,
        rdrand: false,
        nx: false,
        syscall: false,
        gigabyte_pages: false,
        rdtscp: false,
        smep: false,
        smap: false,
        fsgsbase: false,
        la57: false,
        invariant_tsc: false,
        nxe_enabled: false,
        wp_enabled: false,
        smep_enabled: false,
        smap_enabled: false,
    };

    if max_basic >= 1 {
        let l1 = __cpuid(1);
        features.fpu = (l1.edx & (1 << 0)) != 0;
        features.tsc = (l1.edx & (1 << 4)) != 0;
        features.msr = (l1.edx & (1 << 5)) != 0;
        features.pae = (l1.edx & (1 << 6)) != 0;
        features.apic = (l1.edx & (1 << 9)) != 0;
        features.pge = (l1.edx & (1 << 13)) != 0;
        features.pat = (l1.edx & (1 << 16)) != 0;
        features.fxsr = (l1.edx & (1 << 24)) != 0;
        features.sse = (l1.edx & (1 << 25)) != 0;
        features.sse2 = (l1.edx & (1 << 26)) != 0;
        features.x2apic = (l1.ecx & (1 << 21)) != 0;
        features.xsave = (l1.ecx & (1 << 26)) != 0;
        features.rdrand = (l1.ecx & (1 << 30)) != 0;
    }

    if max_basic >= 7 {
        let l7 = __cpuid_count(7, 0);
        features.fsgsbase = (l7.ebx & (1 << 0)) != 0;
        features.smep = (l7.ebx & (1 << 7)) != 0;
        features.smap = (l7.ebx & (1 << 20)) != 0;
        features.la57 = (l7.ecx & (1 << 16)) != 0;
    }

    let ext0 = __cpuid(0x8000_0000);
    features.max_ext_leaf = ext0.eax;

    if features.max_ext_leaf >= 0x8000_0001 {
        let e1 = __cpuid(0x8000_0001);
        features.syscall = (e1.edx & (1 << 11)) != 0;
        features.nx = (e1.edx & (1 << 20)) != 0;
        features.gigabyte_pages = (e1.edx & (1 << 26)) != 0;
        features.rdtscp = (e1.edx & (1 << 27)) != 0;
    }

    if features.max_ext_leaf >= 0x8000_0004 {
        for i in 0..3u32 {
            let b = __cpuid(0x8000_0002 + i);
            let off = (i as usize) * 16;
            features.brand[off..off + 4].copy_from_slice(&b.eax.to_le_bytes());
            features.brand[off + 4..off + 8].copy_from_slice(&b.ebx.to_le_bytes());
            features.brand[off + 8..off + 12].copy_from_slice(&b.ecx.to_le_bytes());
            features.brand[off + 12..off + 16].copy_from_slice(&b.edx.to_le_bytes());
        }
    }

    if features.max_ext_leaf >= 0x8000_0007 {
        let e7 = __cpuid(0x8000_0007);
        features.invariant_tsc = (e7.edx & (1 << 8)) != 0;
    }

    features
}

/// Validate mandatory x86_64 baseline features and enable hardware protections.
/// Returns `Err(&'static str)` if the processor lacks a mandatory architectural requirement.
pub fn init_early_protections(features: &mut CpuFeatures) -> Result<(), &'static str> {
    if !features.msr {
        return Err("CPU missing mandatory MSR support");
    }
    if !features.pae {
        return Err("CPU missing mandatory PAE support");
    }
    if !features.apic {
        return Err("CPU missing mandatory Local APIC support");
    }

    // 1. Enable CR0.WP (Write Protect: enforce read-only pages even in Ring 0)
    let mut cr0 = read_cr0();
    cr0 |= CR0_WP;
    unsafe { write_cr0(cr0) };
    features.wp_enabled = (read_cr0() & CR0_WP) != 0;

    // 2. Enable EFER.NXE (No-Execute) and EFER.SCE (SYSCALL/SYSRET) when supported
    let mut efer = unsafe { rdmsr(MSR_EFER) };
    if features.nx {
        efer |= EFER_NXE;
    }
    if features.syscall {
        efer |= EFER_SCE;
    }
    unsafe { wrmsr(MSR_EFER, efer) };
    features.nxe_enabled = (unsafe { rdmsr(MSR_EFER) } & EFER_NXE) != 0;

    // 3. Enable CR4.PGE (Page Global Enable) and CR4.SMEP when supported
    let mut cr4 = read_cr4();
    if features.pge {
        cr4 |= CR4_PGE;
    }
    if features.smep {
        cr4 |= CR4_SMEP;
    }
    if features.fsgsbase {
        cr4 |= CR4_FSGSBASE;
    }
    unsafe { write_cr4(cr4) };
    features.smep_enabled = (read_cr4() & CR4_SMEP) != 0;
    // Note: SMAP is intentionally enabled in Milestone 4 once `stac`/`clac`
    // copy_from_user/copy_to_user guards are active.
    features.smap_enabled = (read_cr4() & CR4_SMAP) != 0;

    Ok(())
}
