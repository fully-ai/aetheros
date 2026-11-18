# AetherOS Device Driver Model

See also [DRIVERS_AND_DEVICES.md](file:///home/ubeyt/Belgeler/AetherOS/DRIVERS_AND_DEVICES.md).

## 1. Driver Isolation & Hardware Access Layers

All raw hardware I/O port (`inb`, `outb`, `inl`, `outl`) and memory-mapped I/O (`MMIO`) operations are encapsulated inside dedicated driver modules under `kernel/src/arch/x86_64/` and `kernel/src/drivers/` behind `IrqSpinLock` synchronization:

- **`arch::x86_64::serial`**: Dual 16550A UART controller (`COM1 = 0x3F8`, `COM2 = 0x2F8`).
- **`arch::x86_64::pic` & `timer`**: 8259 dual PIC (`0x20/0xA0`, vectors `32..47`), 1000 Hz 8254 PIT (`IRQ0 -> vector 32`), and Local APIC (`IA32_APIC_BASE`) detection.
- **`drivers::fbcon`**: Linear 32-bit RGB/BGR framebuffer console + full-frame backbuffer blitter for the Milestone 14 desktop compositor (`kernel/src/gui/mod.rs`).
- **`drivers::pci`**: PCI Type-1 configuration space bus/slot/function enumerator (`0xCF8`/`0xCFC`), identifying host bridge, ISA bridge, PIIX3 IDE storage, VGA display, E1000 network, and VirtIO controllers.
- **`drivers::rtc`**: CMOS RTC date/time reader (`0x70`/`0x71` with UIP synchronization and BCD decoding) + PS/2 controller (`0x64`/`0x60`) keyboard scancode polling.
