//! PCI Bus Enumeration Driver (Type 1 Configuration Space I/O Ports `0xCF8` / `0xCFC`).
//! Discovers host bridges, storage controllers (IDE/AHCI/VirtIO-blk), network adapters
//! (VirtIO-net/e1000), and display adapters.

use crate::arch::x86_64::{inl, outl};
use crate::sync::IrqSpinLock;
use alloc::vec::Vec;

const PCI_CONFIG_ADDR: u16 = 0x0CF8;
const PCI_CONFIG_DATA: u16 = 0x0CFC;

pub const VENDOR_VIRTIO: u16 = 0x1AF4;
pub const VENDOR_INTEL: u16 = 0x8086;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciDeviceInfo {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub irq_line: u8,
    pub bar0: u32,
}

impl PciDeviceInfo {
    pub const fn class_name(&self) -> &'static str {
        match (self.class_code, self.subclass) {
            (0x01, 0x00) => "SCSI Storage",
            (0x01, 0x01) => "IDE Storage",
            (0x01, 0x06) => "SATA AHCI",
            (0x01, 0x08) => "NVMe Storage",
            (0x02, 0x00) => "Ethernet Network",
            (0x03, 0x00) => "VGA Compatible Display",
            (0x06, 0x00) => "Host Bridge",
            (0x06, 0x01) => "ISA Bridge",
            (0x06, 0x80) => "System Bridge",
            _ => "PCI Device",
        }
    }

    pub const fn is_virtio_block(&self) -> bool {
        self.vendor_id == VENDOR_VIRTIO && (self.device_id == 0x1001 || self.device_id == 0x1042)
    }

    pub const fn is_virtio_net(&self) -> bool {
        self.vendor_id == VENDOR_VIRTIO && (self.device_id == 0x1000 || self.device_id == 0x1041)
    }
}

#[inline]
pub fn pci_read_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let addr: u32 = 0x8000_0000
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((function & 0x07) as u32) << 8)
        | ((offset & 0xFC) as u32);
    unsafe {
        outl(PCI_CONFIG_ADDR, addr);
        inl(PCI_CONFIG_DATA)
    }
}

static PCI_DEVICES: IrqSpinLock<Vec<PciDeviceInfo>> = IrqSpinLock::new(Vec::new());

/// Scan PCI buses `0..=1` and record all populated devices.
pub fn init_and_enumerate() -> usize {
    let mut found = Vec::new();
    for bus in 0..=1u8 {
        for dev in 0..32u8 {
            let id0 = pci_read_u32(bus, dev, 0, 0x00);
            let vendor0 = (id0 & 0xFFFF) as u16;
            if vendor0 == 0xFFFF || vendor0 == 0x0000 {
                continue;
            }
            let hdr_bist0 = pci_read_u32(bus, dev, 0, 0x0C);
            let hdr_type0 = ((hdr_bist0 >> 16) & 0xFF) as u8;
            let max_funcs = if (hdr_type0 & 0x80) != 0 { 8 } else { 1 };

            for func in 0..max_funcs {
                let id_reg = pci_read_u32(bus, dev, func, 0x00);
                let vendor_id = (id_reg & 0xFFFF) as u16;
                if vendor_id == 0xFFFF || vendor_id == 0x0000 {
                    continue;
                }
                let device_id = ((id_reg >> 16) & 0xFFFF) as u16;
                let class_reg = pci_read_u32(bus, dev, func, 0x08);
                let revision = (class_reg & 0xFF) as u8;
                let prog_if = ((class_reg >> 8) & 0xFF) as u8;
                let subclass = ((class_reg >> 16) & 0xFF) as u8;
                let class_code = ((class_reg >> 24) & 0xFF) as u8;
                let hdr_reg = pci_read_u32(bus, dev, func, 0x0C);
                let header_type = ((hdr_reg >> 16) & 0x7F) as u8;
                let bar0 = pci_read_u32(bus, dev, func, 0x10);
                let irq_reg = pci_read_u32(bus, dev, func, 0x3C);
                let irq_line = (irq_reg & 0xFF) as u8;

                found.push(PciDeviceInfo {
                    bus,
                    device: dev,
                    function: func,
                    vendor_id,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                    revision,
                    header_type,
                    irq_line,
                    bar0,
                });
            }
        }
    }
    let count = found.len();
    *PCI_DEVICES.lock() = found;
    count
}

pub fn devices() -> Vec<PciDeviceInfo> {
    PCI_DEVICES.lock().clone()
}
