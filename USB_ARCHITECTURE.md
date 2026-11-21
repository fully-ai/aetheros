# AetherOS USB Subsystem Architecture (`USB_ARCHITECTURE.md`)

## 1. Host Controller & Bus Enumeration (`kernel/src/usb/mod.rs`)

AetherOS discovers USB Host Controllers via PCI Type-1 Configuration Space (`class_code == 0x0C`, `subclass == 0x03`):
- `prog_if == 0x30`: **xHCI (USB 3.0 SuperSpeed, 5000 Mbps)**
- `prog_if == 0x20`: **EHCI (USB 2.0 HighSpeed, 480 Mbps)**
- `prog_if == 0x10`: **OHCI (USB 1.1)**
- `prog_if == 0x00`: **UHCI (USB 1.1)**

## 2. Bounds-Checked USB Descriptor Parser (`parse_usb_descriptors`)

All raw USB descriptor buffers are validated before device classification:
1. **Device Descriptor (`0x01`, 18 bytes)**: Validates `bLength == 18`, `bDescriptorType == 1`, `bcdUSB`, `idVendor`, `idProduct`, and `bDeviceClass`.
2. **Configuration Descriptor (`0x02`, 9 bytes)**: Validates `bLength >= 9`, `bDescriptorType == 2`, and checks that `wTotalLength` does not exceed the received buffer (`UsbError::ConfigTotalLengthOutOfBounds`).
3. **Interface Descriptor (`0x04`, 9 bytes)** & **Endpoint Descriptors (`0x05`, 7 bytes)**: Walks descriptor TLVs checking `d_len >= 2` and `offset + d_len <= total_len` (`UsbError::InvalidDescriptorLength`), extracting Bulk IN (`0x81`) and Bulk OUT (`0x02`) endpoints.

## 3. Supported USB Device Classes
- `0x08` (`SubClass 0x06`, `Protocol 0x50`): **USB Mass Storage (Bulk-Only Transport / SCSI)**
- `0x03`: **USB HID (Keyboards & Optical Mice/Tablets)**
- `0x09`: **USB Root & External Hubs**
- `0x01`: **USB Audio**
- `0x02`: **USB CDC Ethernet**
