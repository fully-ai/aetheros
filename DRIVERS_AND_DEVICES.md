# AetherOS Device Drivers & Hardware Abstractions

## 1. Driver Subsystem Overview (`kernel/src/drivers/`, `kernel/src/arch/x86_64/`)

All hardware I/O port and MMIO accesses are isolated inside `kernel/src/arch/x86_64/` and `kernel/src/drivers/`.

| Driver | Location | Hardware Interface & Features |
| :--- | :--- | :--- |
| **16550 UART Serial** | `arch/x86_64/serial.rs` | `COM1` (`0x3F8`) & `COM2` (`0x2F8`), 115200 baud 8N1, FIFO enabled, non-blocking receive (`try_read_byte`) and transmit polling |
| **8259 Dual PIC** | `arch/x86_64/pic.rs` | Master (`0x20/0x21`) and Slave (`0xA0/0xA1`) remapped to vectors `32..47` (`0x20..0x2F`), cascade IRQ2 enabled, per-line masking and EOI |
| **8254 PIT & LAPIC Timer** | `arch/x86_64/timer.rs` | Channel 0 (`0x40/0x43`) programmed to 1000 Hz (`divisor = 1193`), driving `IRQ0 -> vector 32` monotonic millisecond/nanosecond clocks; detects Local APIC physical base from `IA32_APIC_BASE` (`0x1B`) |
| **Linear Framebuffer (`fbcon`)** | `drivers/fbcon.rs` | Limine 32-bit RGB/BGR linear framebuffer console (`8x8` embedded bitmap font) + full-frame RGB backbuffer blit (`blit_rgb_buffer`) for the Milestone 14 desktop compositor |
| **PCI Bus Type-1 Scanner** | `drivers/pci.rs` | Enumerates PCI Configuration Space via ports `0xCF8` (`CONFIG_ADDRESS`) and `0xCFC` (`CONFIG_DATA`), recording bus/slot/function, `vendor_id`, `device_id`, class/subclass/prog-IF, IRQ line, and `BAR0`, and classifying VirtIO/Storage/Network/Display controllers |
| **CMOS RTC & PS/2 Keyboard** | `drivers/rtc.rs` | Reads real-time UTC date/time (`2026-11-18 HH:MM:SS UTC`) from CMOS ports `0x70/0x71` with UIP (Update-In-Progress) spin guard and BCD-to-binary conversion; polls PS/2 controller (`0x64/0x60`) with Set-1 scancode-to-ASCII translation |
| **QEMU `isa-debug-exit`** | `arch/x86_64/mod.rs` | Port `0xF4` deterministic automated test termination (`0x10 -> exit status 33 PASS`, `0x11 -> exit status 35 FAIL`) |
