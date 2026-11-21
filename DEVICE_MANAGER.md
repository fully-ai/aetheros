# AetherOS Hardware & Device Manager (`DEVICE_MANAGER.md`)

## 1. Discovered Hardware Inventory (`/proc/pci`, `/proc/usb`, `/proc/block`, `/proc/net`)

The AetherOS Device Manager (`AppKind::Settings` & `AppKind::Disks` in `kernel/src/gui/mod.rs`, plus CLI tools `lspci`, `lsusb`, `lsblk`, `lscpu`, `lsmem`) exposes live discovered hardware:

1. **PCI Bus Devices (`kernel/src/drivers/pci.rs`, `/proc/pci`)**:
   - Host Bridge (`06:00`), ISA Bridge (`06:01`), PIIX3 IDE/SATA Storage Controller (`01:01`), VGA/GOP Compatible Display (`03:00`), Ethernet Network Adapter (`02:00`), and USB Host Controller (`0C:03`).
2. **USB Bus & Peripherals (`kernel/src/usb/mod.rs`, `/proc/usb`)**:
   - xHCI / EHCI Host Controller (`00:1d.0`).
   - Bus 001 Addr 001: `1d6b:0003` AetherOS xHCI Root Hub 3.0 (`5000 Mbps`).
   - Bus 001 Addr 002: `0627:0001` QEMU USB Tablet/Mouse (`HID Mouse`, `480 Mbps`).
   - Bus 001 Addr 003: `0951:1666` AetherMedia DataTraveler 3.0 Flash Drive (`Mass Storage BOT/SCSI`, `/dev/sdc1`, `vfat`, `5000 Mbps`).
3. **Block Storage Devices (`kernel/src/storage/mod.rs`, `/proc/block`)**:
   - `/dev/sda` & `/dev/sda1` (`SATA/IDE`, GPT, `/boot`).
   - `/dev/sdb` (`VirtIO-Blk`, 2 PiB AetherFS v1, `/mnt/aether`).
   - `/dev/sdc` & `/dev/sdc1` (`USB-BOT/SCSI`, MBR, FAT32, `/mnt/usb/AETHER_USB`).
4. **Input & Time Devices (`kernel/src/drivers/rtc.rs`)**:
   - 8042 PS/2 Keyboard (Set-1 full QWERTY with Shift), 8042 Auxiliary PS/2 Mouse (3-byte packet streaming), CMOS Real-Time Clock (UTC), and 16550A UART (`COM1`).
