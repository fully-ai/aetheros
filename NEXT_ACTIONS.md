# AetherOS Next Engineering Actions (`v0.2.0` Backlog)

All `v0.1.0-alpha.1` milestones (Milestones 0 through 14) and documentation deliverables are complete and verified. Next engineering iterations for `v0.2.0`:

1. **VirtIO-Blk PCI Driver (`0x1AF4:0x1001`)**: Attach `/mnt/aether` (`AetherFS v1`) directly to a persistent VirtIO block device in QEMU so journaled writes survive across VM reboots.
2. **VirtIO-Net PCI Driver (`0x1AF4:0x1000`) & TCP State Machine**: Connect `kernel/src/net/mod.rs` (`Ethernet/ARP/IPv4/ICMP/UDP`) to QEMU SLIRP external networking and add an RFC 9293 TCP state machine.
3. **SMP Multi-Core AP Runqueues**: Wake secondary AP cores via Limine MP request and distribute `TaskState::Ready` tasks across per-CPU runqueues.
4. **Ring 3 GUI Client Protocol**: Expose the Milestone 14 desktop compositor (`kernel/src/gui/mod.rs`) over `/run/compositor.sock` (`AF_UNIX`) + `shm_open` shared memory surfaces.
