# AetherOS Automated Test Execution Report (`AETHEROS_TEST_REPORT.md`)

- **Command**: `./scripts/test.sh`
- **Host Unit & Fuzz Tests**: `14 passed; 0 failed`
- **QEMU UEFI (`edk2-x86_64-code.fd`) Self-Test**: `21 suites passed; exit=33 (PASS)`
- **QEMU Legacy BIOS Self-Test**: `21 suites passed; exit=33 (PASS)`

## Verified QEMU Self-Test Transcript (`v0.2.0-beta.1`)
```text
[AetherOS INFO ] [TEST PASS] boot_handoff (HHDM=0xffff800000000000, memmap_entries=38)
[AetherOS INFO ] [TEST PASS] cpu_protections (WP=true NXE=true SMEP=true)
[AetherOS INFO ] [TEST PASS] gdt_and_tss (CS=0x08, TR=0x30)
[AetherOS INFO ] [TEST PASS] idt_breakpoint_exception (before=0, after=1)
[AetherOS INFO ] [TEST PASS] pmm_allocator (usable=447 MiB, free=435 MiB, free_runs=6, max_run=384684 KiB)
[AetherOS INFO ] [TEST PASS] vmm_page_mapping_and_wx (vaddr=0xffff910000011000, phys=0x00c8d000)
[AetherOS INFO ] [TEST PASS] vmm_demand_paging (faults_resolved=4)
[AetherOS INFO ] [TEST PASS] vmm_address_space_isolation_and_cow (cow_faults=1)
[AetherOS INFO ] [TEST PASS] kernel_heap_and_alloc (mapped=8192 KiB, peak=1982 KiB, allocs=321, free_blocks=9)
[AetherOS INFO ] [TEST PASS] timer_and_sleep (elapsed=13 ms, switches=2)
[AetherOS INFO ] [TEST PASS] preemptive_multitasking_and_sync (preemptions=5, switches=12, mutex_val=250)
[AetherOS INFO ] [TEST PASS] ring3_syscalls_fork_and_pointer_validation (pid=6, exit_code=77, syscalls=10)
[AetherOS INFO ] [TEST PASS] ring3_fault_isolation (kread_exit=139, priv_cli_exit=139)
[AetherOS INFO ] [TEST PASS] ipc_pipes_unix_sockets_and_shm
[AetherOS INFO ] [TEST PASS] vfs_hierarchy_devices_and_proc
[AetherOS INFO ] [TEST PASS] aetherfs_1pib_and_crash_recovery (1 PiB sparse + SB failover + WAL replay)
[AetherOS INFO ] [TEST PASS] elf64_loader_and_ring3_execution (pid=10, argc=3, exit_code=55)
[AetherOS INFO ] [TEST PASS] multiuser_permissions_acl_and_execute_as (acl_entries=1, recovery_replayed=false)
[AetherOS INFO ] [TEST PASS] userspace_init_aethersh_pipelines_and_aetherpkg (init_pid=11, log_bytes=424)
[AetherOS INFO ] [TEST PASS] drivers_pci_rtc_and_network_stack (pci_devs=6, rtc=2026-11-21, icmp_replies=7)
[AetherOS INFO ] [TEST PASS] usb_mass_storage_partitions_fat32_and_eject (usb_devs=3, blk_devs=5, mount=/mnt/usb/AETHER_USB)
[AetherOS INFO ] [TEST PASS] dhcp_dns_tcp_http_and_tls_security (ip=10.0.2.15, tcp_conns=3, tls=verified)
[AetherOS INFO ] [TEST PASS] m13_security_fuzz_and_benchmarks (pmm=1714 cyc/op, heap=2010 cyc/op, vfs=21221 cyc/op)
[AetherOS INFO ] [TEST PASS] m14_framebuffer_compositor_and_gui (res=1280x800, windows=8, frame_crc=0x6bd960e6)
[AetherOS INFO ] [AETHEROS-TEST] ALL MILESTONE 1 - 14 KERNEL, USERSPACE & GUI TESTS PASSED
[PASS] QEMU self-test (uefi) succeeded (exit=33)
[PASS] QEMU self-test (bios) succeeded (exit=33)
=== ALL AETHEROS TESTS PASSED ===
```
