//! Bootloader handoff parser and architecture-independent `BootInfo` summary.

pub mod limine;

use core::ffi::CStr;
use core::ptr::read_volatile;
use limine::*;

pub const MAX_MEMMAP_REGIONS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    KernelAndModules,
    Framebuffer,
    Unknown(u64),
}

impl MemoryRegionKind {
    pub const fn from_limine(kind: u64) -> Self {
        match kind {
            LIMINE_MEMMAP_USABLE => Self::Usable,
            LIMINE_MEMMAP_RESERVED => Self::Reserved,
            LIMINE_MEMMAP_ACPI_RECLAIMABLE => Self::AcpiReclaimable,
            LIMINE_MEMMAP_ACPI_NVS => Self::AcpiNvs,
            LIMINE_MEMMAP_BAD_MEMORY => Self::BadMemory,
            LIMINE_MEMMAP_BOOTLOADER_RECLAIMABLE => Self::BootloaderReclaimable,
            LIMINE_MEMMAP_EXECUTABLE_AND_MODULES => Self::KernelAndModules,
            LIMINE_MEMMAP_FRAMEBUFFER => Self::Framebuffer,
            other => Self::Unknown(other),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usable => "Usable",
            Self::Reserved => "Reserved",
            Self::AcpiReclaimable => "AcpiReclaimable",
            Self::AcpiNvs => "AcpiNvs",
            Self::BadMemory => "BadMemory",
            Self::BootloaderReclaimable => "BootloaderReclaimable",
            Self::KernelAndModules => "KernelAndModules",
            Self::Framebuffer => "Framebuffer",
            Self::Unknown(_) => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryRegionKind,
}

#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub addr: *mut u8,
    pub width: usize,
    pub height: usize,
    pub pitch: usize,
    pub bpp: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

unsafe impl Send for FramebufferInfo {}
unsafe impl Sync for FramebufferInfo {}

#[derive(Clone, Copy)]
pub struct BootInfo {
    pub bootloader_name: &'static str,
    pub bootloader_version: &'static str,
    pub firmware_type: &'static str,
    pub cmdline: &'static str,
    pub hhdm_offset: u64,
    pub kernel_phys_base: u64,
    pub kernel_virt_base: u64,
    pub rsdp_addr: Option<u64>,
    pub boot_timestamp: i64,
    pub cpu_count: u32,
    pub bsp_lapic_id: u32,
    pub framebuffer: Option<FramebufferInfo>,
    pub initramfs: Option<&'static [u8]>,
    pub memmap_count: usize,
    pub memmap: [MemoryRegion; MAX_MEMMAP_REGIONS],
}

unsafe fn cstr_to_static_str(
    ptr: *const core::ffi::c_char,
    fallback: &'static str,
) -> &'static str {
    if ptr.is_null() {
        return fallback;
    }
    unsafe { CStr::from_ptr(ptr).to_str().unwrap_or(fallback) }
}

/// Validate and parse all Limine Boot Protocol v3 responses into `BootInfo`.
pub fn parse_boot_info() -> Result<BootInfo, &'static str> {
    // 1. Verify Limine base revision support (base_revision[2] must be cleared to 0 by bootloader)
    let rev_status = unsafe { read_volatile(&raw const LIMINE_BASE_REVISION[2]) };
    if rev_status != 0 {
        return Err("Limine Base Revision 3 not supported by bootloader");
    }

    // 2. Bootloader info
    let mut bootloader_name = "Limine";
    let mut bootloader_version = "unknown";
    let bl_resp = unsafe { read_volatile(&BOOTLOADER_INFO_REQUEST.response) };
    if !bl_resp.is_null() {
        let resp = unsafe { &*bl_resp };
        bootloader_name = unsafe { cstr_to_static_str(resp.name, "Limine") };
        bootloader_version = unsafe { cstr_to_static_str(resp.version, "unknown") };
    }

    // 3. Firmware type
    let mut firmware_type = "Unknown";
    let fw_resp = unsafe { read_volatile(&FIRMWARE_TYPE_REQUEST.response) };
    if !fw_resp.is_null() {
        firmware_type = match unsafe { (*fw_resp).firmware_type } {
            LIMINE_FIRMWARE_TYPE_X86BIOS => "x86-BIOS",
            LIMINE_FIRMWARE_TYPE_UEFI32 => "UEFI-32",
            LIMINE_FIRMWARE_TYPE_UEFI64 => "UEFI-64",
            _ => "Other",
        };
    }

    // 4. Command line
    let mut cmdline = "";
    let cmd_resp = unsafe { read_volatile(&EXECUTABLE_CMDLINE_REQUEST.response) };
    if !cmd_resp.is_null() {
        cmdline = unsafe { cstr_to_static_str((*cmd_resp).cmdline, "") };
    }

    // 5. Higher-Half Direct Map (HHDM)
    let hhdm_resp = unsafe { read_volatile(&HHDM_REQUEST.response) };
    if hhdm_resp.is_null() {
        return Err("Missing Limine HHDM response");
    }
    let hhdm_offset = unsafe { (*hhdm_resp).offset };

    // 6. Kernel physical/virtual base addresses
    let kaddr_resp = unsafe { read_volatile(&EXECUTABLE_ADDRESS_REQUEST.response) };
    if kaddr_resp.is_null() {
        return Err("Missing Limine Executable Address response");
    }
    let kernel_phys_base = unsafe { (*kaddr_resp).physical_base };
    let kernel_virt_base = unsafe { (*kaddr_resp).virtual_base };

    // 7. Physical memory map
    let mmap_resp = unsafe { read_volatile(&MEMMAP_REQUEST.response) };
    if mmap_resp.is_null() {
        return Err("Missing Limine Memory Map response");
    }
    let entry_count = unsafe { (*mmap_resp).entry_count as usize };
    let entries_ptr = unsafe { (*mmap_resp).entries };
    if entries_ptr.is_null() || entry_count == 0 {
        return Err("Empty Limine Memory Map");
    }

    let mut memmap = [MemoryRegion {
        base: 0,
        length: 0,
        kind: MemoryRegionKind::Reserved,
    }; MAX_MEMMAP_REGIONS];
    let count = entry_count.min(MAX_MEMMAP_REGIONS);
    for (i, slot) in memmap.iter_mut().enumerate().take(count) {
        let entry_ptr = unsafe { *entries_ptr.add(i) };
        if !entry_ptr.is_null() {
            let entry = unsafe { &*entry_ptr };
            *slot = MemoryRegion {
                base: entry.base,
                length: entry.length,
                kind: MemoryRegionKind::from_limine(entry.entry_type),
            };
        }
    }

    // 8. Framebuffer (optional)
    let mut framebuffer = None;
    let fb_resp = unsafe { read_volatile(&FRAMEBUFFER_REQUEST.response) };
    if !fb_resp.is_null() {
        let resp = unsafe { &*fb_resp };
        if resp.framebuffer_count > 0 && !resp.framebuffers.is_null() {
            let fb_ptr = unsafe { *resp.framebuffers };
            if !fb_ptr.is_null() {
                let fb = unsafe { &*fb_ptr };
                if !fb.address.is_null() && fb.bpp == 32 {
                    framebuffer = Some(FramebufferInfo {
                        addr: fb.address,
                        width: fb.width as usize,
                        height: fb.height as usize,
                        pitch: fb.pitch as usize,
                        bpp: fb.bpp,
                        red_shift: fb.red_mask_shift,
                        green_shift: fb.green_mask_shift,
                        blue_shift: fb.blue_mask_shift,
                    });
                }
            }
        }
    }

    // 9. Boot module (initramfs.tar, optional in early milestones)
    let mut initramfs = None;
    let mod_resp = unsafe { read_volatile(&MODULE_REQUEST.response) };
    if !mod_resp.is_null() {
        let resp = unsafe { &*mod_resp };
        if resp.module_count > 0 && !resp.modules.is_null() {
            let m0 = unsafe { *resp.modules };
            if !m0.is_null() {
                let file = unsafe { &*m0 };
                if !file.address.is_null() && file.size > 0 {
                    initramfs = Some(unsafe {
                        core::slice::from_raw_parts(file.address, file.size as usize)
                    });
                }
            }
        }
    }

    // 10. ACPI RSDP
    let mut rsdp_addr = None;
    let rsdp_resp = unsafe { read_volatile(&RSDP_REQUEST.response) };
    if !rsdp_resp.is_null() {
        let addr = unsafe { (*rsdp_resp).address };
        if addr != 0 {
            rsdp_addr = Some(addr);
        }
    }

    // 11. Date at boot
    let mut boot_timestamp = 0i64;
    let date_resp = unsafe { read_volatile(&DATE_AT_BOOT_REQUEST.response) };
    if !date_resp.is_null() {
        boot_timestamp = unsafe { (*date_resp).timestamp };
    }

    // 12. Multiprocessor (MP) summary
    let mut cpu_count = 1u32;
    let mut bsp_lapic_id = 0u32;
    let mp_resp = unsafe { read_volatile(&MP_REQUEST.response) };
    if !mp_resp.is_null() {
        let resp = unsafe { &*mp_resp };
        cpu_count = (resp.cpu_count as u32).max(1);
        bsp_lapic_id = resp.bsp_lapic_id;
    }

    Ok(BootInfo {
        bootloader_name,
        bootloader_version,
        firmware_type,
        cmdline,
        hhdm_offset,
        kernel_phys_base,
        kernel_virt_base,
        rsdp_addr,
        boot_timestamp,
        cpu_count,
        bsp_lapic_id,
        framebuffer,
        initramfs,
        memmap_count: count,
        memmap,
    })
}
