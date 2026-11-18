# AetherOS Next Steps & Post-v0.1.0 Roadmap

All `v0.1.0-alpha.1` requirements (Milestones 0 through 14, automated host & QEMU UEFI/BIOS verification, and full documentation) are complete and published to `https://github.com/fully-ai/aetheros`.

## Planned Enhancements for `v0.2.0`
1. **VirtIO-Blk & VirtIO-Net PCI DMA Drivers**: Bind the PCI scanner's detected VirtIO devices (`0x1AF4`) to Virtqueue DMA ring buffers.
2. **SMP Application Processor (AP) Scheduling**: Wake secondary CPUs via Limine MP request and enable per-CPU runqueues and IPI TLB shootdowns.
3. **Dynamic ELF64 Linker (`ld-aether.so`)**: Support `PT_INTERP` and shared libraries under `/lib`.
4. **Ring 3 GUI Client IPC Protocol**: Expose the Milestone 14 desktop compositor (`kernel/src/gui/mod.rs`) via `/run/compositor.sock` (`AF_UNIX`) + `shm_open` shared memory surfaces.
