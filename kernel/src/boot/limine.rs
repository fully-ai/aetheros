//! Limine Boot Protocol (Base Revision 3) request and response definitions.
//! Derived from the 0BSD-licensed `limine.h` specification (`v9.x-binary`).

use core::ffi::c_char;

pub const LIMINE_COMMON_MAGIC: [u64; 2] = [0xc7b1_dd30_df4c_8b88, 0x0a82_e883_a194_f07b];

pub const LIMINE_MEMMAP_USABLE: u64 = 0;
pub const LIMINE_MEMMAP_RESERVED: u64 = 1;
pub const LIMINE_MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
pub const LIMINE_MEMMAP_ACPI_NVS: u64 = 3;
pub const LIMINE_MEMMAP_BAD_MEMORY: u64 = 4;
pub const LIMINE_MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
pub const LIMINE_MEMMAP_EXECUTABLE_AND_MODULES: u64 = 6;
pub const LIMINE_MEMMAP_FRAMEBUFFER: u64 = 7;

pub const LIMINE_FIRMWARE_TYPE_X86BIOS: u64 = 0;
pub const LIMINE_FIRMWARE_TYPE_UEFI32: u64 = 1;
pub const LIMINE_FIRMWARE_TYPE_UEFI64: u64 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LimineUuid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

#[repr(C)]
pub struct LimineFile {
    pub revision: u64,
    pub address: *const u8,
    pub size: u64,
    pub path: *const c_char,
    pub string: *const c_char,
    pub media_type: u32,
    pub unused: u32,
    pub tftp_ip: u32,
    pub tftp_port: u32,
    pub partition_index: u32,
    pub mbr_disk_id: u32,
    pub gpt_disk_uuid: LimineUuid,
    pub gpt_part_uuid: LimineUuid,
    pub part_uuid: LimineUuid,
}

#[repr(C)]
pub struct LimineBootloaderInfoResponse {
    pub revision: u64,
    pub name: *const c_char,
    pub version: *const c_char,
}

#[repr(C)]
pub struct LimineBootloaderInfoRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineBootloaderInfoResponse,
}
unsafe impl Sync for LimineBootloaderInfoRequest {}

#[repr(C)]
pub struct LimineFirmwareTypeResponse {
    pub revision: u64,
    pub firmware_type: u64,
}

#[repr(C)]
pub struct LimineFirmwareTypeRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineFirmwareTypeResponse,
}
unsafe impl Sync for LimineFirmwareTypeRequest {}

#[repr(C)]
pub struct LimineStackSizeResponse {
    pub revision: u64,
}

#[repr(C)]
pub struct LimineStackSizeRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineStackSizeResponse,
    pub stack_size: u64,
}
unsafe impl Sync for LimineStackSizeRequest {}

#[repr(C)]
pub struct LimineHhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct LimineHhdmRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineHhdmResponse,
}
unsafe impl Sync for LimineHhdmRequest {}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LimineMemmapEntry {
    pub base: u64,
    pub length: u64,
    pub entry_type: u64,
}

#[repr(C)]
pub struct LimineMemmapResponse {
    pub revision: u64,
    pub entry_count: u64,
    pub entries: *const *const LimineMemmapEntry,
}

#[repr(C)]
pub struct LimineMemmapRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineMemmapResponse,
}
unsafe impl Sync for LimineMemmapRequest {}

#[repr(C)]
pub struct LimineExecutableAddressResponse {
    pub revision: u64,
    pub physical_base: u64,
    pub virtual_base: u64,
}

#[repr(C)]
pub struct LimineExecutableAddressRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineExecutableAddressResponse,
}
unsafe impl Sync for LimineExecutableAddressRequest {}

#[repr(C)]
pub struct LimineExecutableCmdlineResponse {
    pub revision: u64,
    pub cmdline: *const c_char,
}

#[repr(C)]
pub struct LimineExecutableCmdlineRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineExecutableCmdlineResponse,
}
unsafe impl Sync for LimineExecutableCmdlineRequest {}

#[repr(C)]
pub struct LimineFramebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub unused: [u8; 7],
    pub edid_size: u64,
    pub edid: *const u8,
    pub mode_count: u64,
    pub modes: *const *const u8,
}

#[repr(C)]
pub struct LimineFramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *const *const LimineFramebuffer,
}

#[repr(C)]
pub struct LimineFramebufferRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineFramebufferResponse,
}
unsafe impl Sync for LimineFramebufferRequest {}

#[repr(C)]
pub struct LimineModuleResponse {
    pub revision: u64,
    pub module_count: u64,
    pub modules: *const *const LimineFile,
}

#[repr(C)]
pub struct LimineModuleRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineModuleResponse,
    pub internal_module_count: u64,
    pub internal_modules: *const *const u8,
}
unsafe impl Sync for LimineModuleRequest {}

#[repr(C)]
pub struct LimineRsdpResponse {
    pub revision: u64,
    pub address: u64,
}

#[repr(C)]
pub struct LimineRsdpRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineRsdpResponse,
}
unsafe impl Sync for LimineRsdpRequest {}

#[repr(C)]
pub struct LimineDateAtBootResponse {
    pub revision: u64,
    pub timestamp: i64,
}

#[repr(C)]
pub struct LimineDateAtBootRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineDateAtBootResponse,
}
unsafe impl Sync for LimineDateAtBootRequest {}

#[repr(C)]
pub struct LimineMpInfo {
    pub processor_id: u32,
    pub lapic_id: u32,
    pub reserved: u64,
    pub goto_address: usize,
    pub extra_argument: u64,
}

#[repr(C)]
pub struct LimineMpResponse {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    pub cpu_count: u64,
    pub cpus: *const *const LimineMpInfo,
}

#[repr(C)]
pub struct LimineMpRequest {
    pub id: [u64; 4],
    pub revision: u64,
    pub response: *const LimineMpResponse,
    pub flags: u64,
}
unsafe impl Sync for LimineMpRequest {}

// --- Request section markers and active requests ---

#[used]
#[link_section = ".limine_requests_start"]
pub static LIMINE_REQUESTS_START_MARKER: [u64; 4] = [
    0xf6b8_f4b3_9de7_d1ae,
    0xfab9_1a69_40fc_b9cf,
    0x785c_6ed0_15d3_e316,
    0x181e_920a_7852_b9d9,
];

#[used]
#[link_section = ".limine_requests"]
pub static mut LIMINE_BASE_REVISION: [u64; 3] = [0xf956_2b2d_5c95_a6c8, 0x6a7b_3849_4453_6bdc, 3];

#[used]
#[link_section = ".limine_requests"]
pub static BOOTLOADER_INFO_REQUEST: LimineBootloaderInfoRequest = LimineBootloaderInfoRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0xf550_38d8_e2a1_202f,
        0x2794_26fc_f5f5_9740,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static FIRMWARE_TYPE_REQUEST: LimineFirmwareTypeRequest = LimineFirmwareTypeRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x8c2f_75d9_0bef_28a8,
        0x7045_a468_8eac_00c3,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static STACK_SIZE_REQUEST: LimineStackSizeRequest = LimineStackSizeRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x224e_f046_0a8e_8926,
        0xe1cb_0fc2_5f46_ea3d,
    ],
    revision: 0,
    response: core::ptr::null(),
    stack_size: 256 * 1024, // 256 KiB initial kernel stack
};

#[used]
#[link_section = ".limine_requests"]
pub static HHDM_REQUEST: LimineHhdmRequest = LimineHhdmRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x48dc_f1cb_8ad2_b852,
        0x6398_4e95_9a98_244b,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static MEMMAP_REQUEST: LimineMemmapRequest = LimineMemmapRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x67cf_3d9d_378a_806f,
        0xe304_acdf_c50c_3c62,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static EXECUTABLE_ADDRESS_REQUEST: LimineExecutableAddressRequest =
    LimineExecutableAddressRequest {
        id: [
            LIMINE_COMMON_MAGIC[0],
            LIMINE_COMMON_MAGIC[1],
            0x71ba_7686_3cc5_5f63,
            0xb264_4a48_c516_a487,
        ],
        revision: 0,
        response: core::ptr::null(),
    };

#[used]
#[link_section = ".limine_requests"]
pub static EXECUTABLE_CMDLINE_REQUEST: LimineExecutableCmdlineRequest =
    LimineExecutableCmdlineRequest {
        id: [
            LIMINE_COMMON_MAGIC[0],
            LIMINE_COMMON_MAGIC[1],
            0x4b16_1536_e598_651e,
            0xb390_ad4a_2f1f_303a,
        ],
        revision: 0,
        response: core::ptr::null(),
    };

#[used]
#[link_section = ".limine_requests"]
pub static FRAMEBUFFER_REQUEST: LimineFramebufferRequest = LimineFramebufferRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x9d58_27dc_d881_dd75,
        0xa314_8604_f6fa_b11b,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static MODULE_REQUEST: LimineModuleRequest = LimineModuleRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x3e7e_2797_02be_32af,
        0xca1c_4f3b_d128_0cee,
    ],
    revision: 0,
    response: core::ptr::null(),
    internal_module_count: 0,
    internal_modules: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static RSDP_REQUEST: LimineRsdpRequest = LimineRsdpRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0xc5e7_7b6b_397e_7b43,
        0x2763_7845_accd_cf3c,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static DATE_AT_BOOT_REQUEST: LimineDateAtBootRequest = LimineDateAtBootRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x5027_46e1_84c0_88aa,
        0xfbc5_ec83_e632_7893,
    ],
    revision: 0,
    response: core::ptr::null(),
};

#[used]
#[link_section = ".limine_requests"]
pub static MP_REQUEST: LimineMpRequest = LimineMpRequest {
    id: [
        LIMINE_COMMON_MAGIC[0],
        LIMINE_COMMON_MAGIC[1],
        0x95a6_7b81_9a1b_857e,
        0xa0b6_1b72_3b6a_73e0,
    ],
    revision: 0,
    response: core::ptr::null(),
    flags: 0,
};

#[used]
#[link_section = ".limine_requests_end"]
pub static LIMINE_REQUESTS_END_MARKER: [u64; 2] = [0xadc0_e053_1bb1_0d03, 0x9572_709f_3176_4c62];
