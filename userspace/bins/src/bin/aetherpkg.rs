#![no_std]
#![no_main]

//! `/bin/aetherpkg` — Native AetherOS Package Manager (Section 17).
//!
//! Capabilities:
//! - Package repository catalog & metadata (`list`, `info <pkg>`).
//! - Dependency resolution (`install <pkg>` resolves dependencies before installing).
//! - Integrity verification using Castagnoli CRC32C checksums over package payload.
//! - File ownership tracking in `/var/lib/aetherpkg/installed.db`.
//! - Clean removal (`remove <pkg>`) and integrity audit (`verify`).

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libaether::{
    aether_main, chmod, eprintln, getuid, mkdir, println, read_file_to_string, stat, unlink,
    write_string_to_file,
};

struct RepoPkg {
    name: &'static str,
    version: &'static str,
    deps: &'static [&'static str],
    description: &'static str,
    install_path: &'static str,
    payload: &'static str,
}

const REPO_CATALOG: &[RepoPkg] = &[
    RepoPkg {
        name: "aether-base",
        version: "0.1.0",
        deps: &[],
        description: "AetherOS base system metadata and release profile",
        install_path: "/etc/aether-base.manifest",
        payload: "PACKAGE=aether-base\nVERSION=0.1.0\nARCH=x86_64\n",
    },
    RepoPkg {
        name: "aether-devtools",
        version: "0.1.0",
        deps: &["aether-base"],
        description: "AetherOS developer diagnostic profile and sysctl presets",
        install_path: "/etc/aether-devtools.conf",
        payload: "DEVTOOLS_ENABLED=1\nAUDIT_LEVEL=strict\nDependency=aether-base\n",
    },
    RepoPkg {
        name: "aether-netutils",
        version: "0.1.0",
        deps: &["aether-base"],
        description: "AetherOS network stack configuration and hosts table",
        install_path: "/etc/hosts",
        payload: "127.0.0.1\tlocalhost\n10.0.2.15\taether.local aether\n",
    },
];

const DB_PATH: &str = "/var/lib/aetherpkg/installed.db";

fn crc32c_str(s: &str) -> u32 {
    let mut crc = !0u32;
    for &byte in s.as_bytes() {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

fn ensure_db() {
    let _ = mkdir("/var", 0o755);
    let _ = mkdir("/var/lib", 0o755);
    let _ = mkdir("/var/lib/aetherpkg", 0o755);
    if stat(DB_PATH).is_err() {
        let pkg = &REPO_CATALOG[0];
        let csum = crc32c_str(pkg.payload);
        let _ = write_string_to_file(pkg.install_path, pkg.payload, false);
        let line = format!(
            "{}|{}|{}|{:08x}\n",
            pkg.name, pkg.version, pkg.install_path, csum
        );
        let _ = write_string_to_file(DB_PATH, &line, false);
    }
}

fn is_installed(pkg_name: &str) -> bool {
    let db = read_file_to_string(DB_PATH).unwrap_or_default();
    db.lines().any(|l| l.starts_with(&format!("{}|", pkg_name)))
}

fn install_single(pkg_name: &str) -> Result<(), &'static str> {
    if is_installed(pkg_name) {
        println!("[aetherpkg] '{}' is already installed.", pkg_name);
        return Ok(());
    }
    let Some(pkg) = REPO_CATALOG.iter().find(|p| p.name == pkg_name) else {
        return Err("package not found in repository catalog");
    };

    // Resolve and install dependencies first
    for &dep in pkg.deps {
        if !is_installed(dep) {
            println!(
                "[aetherpkg] Resolving dependency '{}' for '{}'...",
                dep, pkg.name
            );
            install_single(dep)?;
        }
    }

    let csum = crc32c_str(pkg.payload);
    write_string_to_file(pkg.install_path, pkg.payload, false)
        .map_err(|_| "failed to write package payload")?;
    let _ = chmod(pkg.install_path, 0o644);

    let record = format!(
        "{}|{}|{}|{:08x}\n",
        pkg.name, pkg.version, pkg.install_path, csum
    );
    write_string_to_file(DB_PATH, &record, true).map_err(|_| "failed to update installed.db")?;
    println!(
        "[aetherpkg] Installed {}-{} -> {} (CRC32C={:08x})",
        pkg.name, pkg.version, pkg.install_path, csum
    );
    Ok(())
}

fn main(args: &[&str]) -> i32 {
    ensure_db();
    let subcmd = args.get(1).copied().unwrap_or("list");

    match subcmd {
        "list" => {
            println!("PACKAGE           VERSION   STATUS      DESCRIPTION");
            for p in REPO_CATALOG {
                let status = if is_installed(p.name) {
                    "installed"
                } else {
                    "available"
                };
                println!(
                    "{:<16}  {:<8}  {:<10}  {}",
                    p.name, p.version, status, p.description
                );
            }
            0
        }
        "info" => {
            let Some(&name) = args.get(2) else {
                eprintln!("usage: aetherpkg info <package>");
                return 1;
            };
            let Some(p) = REPO_CATALOG.iter().find(|x| x.name == name) else {
                eprintln!("aetherpkg: unknown package '{}'", name);
                return 1;
            };
            println!("Name       : {}", p.name);
            println!("Version    : {}", p.version);
            println!("Description: {}", p.description);
            println!("File       : {}", p.install_path);
            println!("CRC32C     : {:08x}", crc32c_str(p.payload));
            0
        }
        "install" => {
            if getuid() != 0 {
                eprintln!("aetherpkg: root privileges required to install packages");
                return 1;
            }
            let Some(&name) = args.get(2) else {
                eprintln!("usage: aetherpkg install <package>");
                return 1;
            };
            match install_single(name) {
                Ok(()) => 0,
                Err(msg) => {
                    eprintln!("aetherpkg: install failed: {}", msg);
                    1
                }
            }
        }
        "remove" => {
            if getuid() != 0 {
                eprintln!("aetherpkg: root privileges required to remove packages");
                return 1;
            }
            let Some(&name) = args.get(2) else {
                eprintln!("usage: aetherpkg remove <package>");
                return 1;
            };
            let db = read_file_to_string(DB_PATH).unwrap_or_default();
            let mut kept = String::new();
            let mut removed_file = None;
            for line in db.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.first().copied() == Some(name) {
                    if let Some(&f) = parts.get(2) {
                        removed_file = Some(String::from(f));
                    }
                } else if !line.is_empty() {
                    kept.push_str(line);
                    kept.push('\n');
                }
            }
            if let Some(f) = removed_file {
                let _ = unlink(&f);
                let _ = write_string_to_file(DB_PATH, &kept, false);
                println!("[aetherpkg] Removed package '{}' and file {}", name, f);
                0
            } else {
                eprintln!("aetherpkg: package '{}' is not installed", name);
                1
            }
        }
        "verify" => {
            let db = read_file_to_string(DB_PATH).unwrap_or_default();
            let mut ok_count = 0usize;
            for line in db.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 4 {
                    let pkg = parts[0];
                    let path = parts[2];
                    let expected_hex = parts[3];
                    match read_file_to_string(path) {
                        Ok(data) => {
                            let actual_hex = format!("{:08x}", crc32c_str(&data));
                            if actual_hex == expected_hex {
                                println!("[OK] {} ({}) CRC32C={}", pkg, path, actual_hex);
                                ok_count += 1;
                            } else {
                                eprintln!(
                                    "[FAIL] {} ({}) expected={} actual={}",
                                    pkg, path, expected_hex, actual_hex
                                );
                                return 1;
                            }
                        }
                        Err(_) => {
                            eprintln!("[MISSING] {} ({})", pkg, path);
                            return 1;
                        }
                    }
                }
            }
            println!("[aetherpkg] Verified {} installed package(s).", ok_count);
            0
        }
        _ => {
            eprintln!("usage: aetherpkg <list|info|install|remove|verify> [pkg]");
            1
        }
    }
}

aether_main!(main);
