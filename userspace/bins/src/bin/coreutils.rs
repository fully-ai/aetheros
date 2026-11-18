#![no_std]
#![no_main]

//! AetherOS Native ELF64 Core Utilities Suite (`/bin/*`, `/sbin/useradd`).
//! Dispatches on `argv[0]` basename (`ls`, `cat`, `grep`, `ps`, `kill`, `id`, `whoami`,
//! `groups`, `stat`, `chmod`, `chown`, `mkdir`, `rmdir`, `touch`, `rm`, `cp`, `mv`,
//! `uname`, `uptime`, `free`, `df`, `dmesg`, `mount`, `umount`, `wc`, `useradd`).

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use libaether::{
    aether_main, chmod, chown, close, dmesg, eprintln, get_group_uuid, get_user_uuid, getcwd,
    getdents64, getgid, getuid, kill, mkdir, mount, open, print, println, proc_list, read,
    read_file_to_string, rename, rmdir, stat, sysinfo, umount, uname, unlink, useradd, write,
    write_string_to_file, Dirent64, ProcInfo, Uuid, O_CREAT, O_DIRECTORY, O_RDONLY, O_WRONLY,
};

fn format_mode(mode: u32) -> [u8; 10] {
    let ftype = match mode & 0o170000 {
        0o040000 => b'd',
        0o120000 => b'l',
        0o020000 => b'c',
        0o010000 => b'p',
        _ => b'-',
    };
    let r = |bit: u32, ch: u8| if (mode & bit) != 0 { ch } else { b'-' };
    [
        ftype,
        r(0o400, b'r'),
        r(0o200, b'w'),
        if (mode & 0o4000) != 0 {
            b's'
        } else {
            r(0o100, b'x')
        },
        r(0o040, b'r'),
        r(0o020, b'w'),
        if (mode & 0o2000) != 0 {
            b's'
        } else {
            r(0o010, b'x')
        },
        r(0o004, b'r'),
        r(0o002, b'w'),
        if (mode & 0o1000) != 0 {
            b't'
        } else {
            r(0o001, b'x')
        },
    ]
}

fn cmd_ls(args: &[&str]) -> i32 {
    let mut long_fmt = false;
    let mut target_dir = None;
    for &a in args.iter().skip(1) {
        if a == "-l" || a == "-la" || a == "-al" {
            long_fmt = true;
        } else if !a.starts_with('-') {
            target_dir = Some(String::from(a));
        }
    }
    let dir_path = target_dir.unwrap_or_else(|| getcwd().unwrap_or_else(|_| String::from("/")));

    let fd = match open(&dir_path, O_RDONLY | O_DIRECTORY, 0) {
        Ok(fd) => fd,
        Err(_) => {
            // Maybe a single file path
            if let Ok(st) = stat(&dir_path) {
                let m = format_mode(st.mode);
                let ms = core::str::from_utf8(&m).unwrap_or("----------");
                println!(
                    "{} {:4} {:4} {:8} {} (owner_uuid={})",
                    ms,
                    st.uid,
                    st.gid,
                    st.size,
                    dir_path,
                    Uuid::from_bytes(st.owner_user_uuid)
                );
                return 0;
            }
            eprintln!("ls: cannot access '{}'", dir_path);
            return 1;
        }
    };

    let mut entries = [Dirent64 {
        d_ino: 0,
        d_reclen: 0,
        d_type: 0,
        d_namlen: 0,
        _pad: 0,
        d_name: [0; 240],
    }; 32];

    loop {
        match getdents64(fd, &mut entries) {
            Ok(0) => break,
            Ok(n) => {
                for item in entries.iter().take(n) {
                    let nlen = (item.d_namlen as usize).min(239);
                    let name = core::str::from_utf8(&item.d_name[..nlen]).unwrap_or("?");
                    if long_fmt {
                        let full = if dir_path == "/" {
                            format!("/{}", name)
                        } else {
                            format!("{}/{}", dir_path.trim_end_matches('/'), name)
                        };
                        if let Ok(st) = stat(&full) {
                            let m = format_mode(st.mode);
                            let ms = core::str::from_utf8(&m).unwrap_or("----------");
                            let exec_as = Uuid::from_bytes(st.execute_as_uuid);
                            if !exec_as.is_nil() {
                                println!(
                                    "{} {:4} {:4} {:8} {} [exec_as={}]",
                                    ms, st.uid, st.gid, st.size, name, exec_as
                                );
                            } else {
                                println!("{} {:4} {:4} {:8} {}", ms, st.uid, st.gid, st.size, name);
                            }
                        } else {
                            println!("??????????    0    0        0 {}", name);
                        }
                    } else {
                        println!("{}", name);
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = close(fd);
    0
}

fn cmd_cat(args: &[&str]) -> i32 {
    if args.len() < 2 {
        let mut buf = [0u8; 512];
        loop {
            match read(0, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = write(1, &buf[..n]);
                }
                Err(_) => break,
            }
        }
        return 0;
    }
    for &path in args.iter().skip(1) {
        match read_file_to_string(path) {
            Ok(content) => print!("{}", content),
            Err(e) => {
                eprintln!("cat: {}: errno {}", path, e);
                return 1;
            }
        }
    }
    0
}

fn cmd_grep(args: &[&str]) -> i32 {
    let Some(&pattern) = args.get(1) else {
        eprintln!("usage: grep <pattern> [file]");
        return 2;
    };
    let content = if let Some(&file_path) = args.get(2) {
        match read_file_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("grep: {}: errno {}", file_path, e);
                return 2;
            }
        }
    } else {
        let mut data = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match read(0, &mut buf) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        String::from(core::str::from_utf8(&data).unwrap_or(""))
    };

    let mut matched = false;
    for line in content.lines() {
        if line.contains(pattern) {
            println!("{}", line);
            matched = true;
        }
    }
    if matched {
        0
    } else {
        1
    }
}

fn cmd_wc(args: &[&str]) -> i32 {
    let content = if let Some(&path) = args.get(1) {
        read_file_to_string(path).unwrap_or_default()
    } else {
        let mut data = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            match read(0, &mut buf) {
                Ok(0) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        String::from(core::str::from_utf8(&data).unwrap_or(""))
    };
    let lines = content.lines().count();
    let words = content.split_whitespace().count();
    let bytes = content.len();
    println!("{:6} {:6} {:6}", lines, words, bytes);
    0
}

fn cmd_ps() -> i32 {
    let mut list = [ProcInfo::default(); 64];
    let n = proc_list(&mut list).unwrap_or(0);
    println!("  PID  PPID   UID  STATE  PRIO    TICKS  COMMAND");
    for p in list.iter().take(n) {
        let nlen = p.name.iter().position(|&b| b == 0).unwrap_or(32);
        let name = core::str::from_utf8(&p.name[..nlen]).unwrap_or("?");
        let st = match p.state {
            1 => "READY",
            2 => "RUN  ",
            3 => "SLEEP",
            4 => "BLOCK",
            5 => "ZOMB ",
            _ => "?    ",
        };
        println!(
            "{:5} {:5} {:5}  {} {:5} {:8}  {}",
            p.pid, p.ppid, p.euid, st, p.priority, p.cpu_ticks, name
        );
    }
    0
}

fn cmd_id() -> i32 {
    let uid = getuid();
    let gid = getgid();
    let u_uuid = get_user_uuid();
    let g_uuid = get_group_uuid();
    let uname = if uid == 0 { "root" } else { "aether" };
    println!(
        "uid={}({}) gid={}({}) user_uuid={} group_uuid={}",
        uid, uname, gid, uname, u_uuid, g_uuid
    );
    0
}

fn cmd_whoami() -> i32 {
    let uid = getuid();
    if uid == 0 {
        println!("root");
    } else {
        println!("aether");
    }
    0
}

fn cmd_groups() -> i32 {
    let uid = getuid();
    if uid == 0 {
        println!("root wheel");
    } else {
        println!("aether wheel");
    }
    0
}

fn cmd_stat(args: &[&str]) -> i32 {
    let Some(&path) = args.get(1) else {
        eprintln!("usage: stat <path>");
        return 1;
    };
    match stat(path) {
        Ok(st) => {
            let m = format_mode(st.mode);
            let ms = core::str::from_utf8(&m).unwrap_or("----------");
            println!("  File: {}", path);
            println!(
                "  Size: {:<12} Blocks: {:<8} Inode: {}",
                st.size, st.blocks, st.inode_id
            );
            println!(
                "Access: ({:04o}/{})  Uid: {}  Gid: {}",
                st.mode & 0o7777,
                ms,
                st.uid,
                st.gid
            );
            println!("Object-UUID    : {}", Uuid::from_bytes(st.object_uuid));
            println!("Owner-User-UUID: {}", Uuid::from_bytes(st.owner_user_uuid));
            println!("Owner-Grp-UUID : {}", Uuid::from_bytes(st.owner_group_uuid));
            println!("Execute-As-UUID: {}", Uuid::from_bytes(st.execute_as_uuid));
            0
        }
        Err(e) => {
            eprintln!("stat: cannot stat '{}': errno {}", path, e);
            1
        }
    }
}

fn cmd_chmod(args: &[&str]) -> i32 {
    if args.len() < 3 {
        eprintln!("usage: chmod <octal_mode> <path>");
        return 1;
    }
    let mode = u32::from_str_radix(args[1], 8).unwrap_or(0o644);
    match chmod(args[2], mode) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("chmod: {}: errno {}", args[2], e);
            1
        }
    }
}

fn cmd_chown(args: &[&str]) -> i32 {
    if args.len() < 3 {
        eprintln!("usage: chown <uid[:gid]> <path>");
        return 1;
    }
    let (uid_str, gid_str) = args[1].split_once(':').unwrap_or((args[1], ""));
    let uid = uid_str.parse::<u32>().unwrap_or(0);
    let gid = if gid_str.is_empty() {
        u32::MAX
    } else {
        gid_str.parse::<u32>().unwrap_or(u32::MAX)
    };
    match chown(args[2], uid, gid) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("chown: {}: errno {}", args[2], e);
            1
        }
    }
}

fn cstr_65(arr: &[u8; 65]) -> &str {
    let end = arr.iter().position(|&b| b == 0).unwrap_or(65);
    core::str::from_utf8(&arr[..end]).unwrap_or("")
}

fn cmd_uname() -> i32 {
    if let Ok(u) = uname() {
        println!(
            "{} {} {} {} {}",
            cstr_65(&u.sysname),
            cstr_65(&u.nodename),
            cstr_65(&u.release),
            cstr_65(&u.version),
            cstr_65(&u.machine)
        );
    }
    0
}

fn cmd_uptime() -> i32 {
    if let Ok(info) = sysinfo() {
        println!(
            "up {}.{:03}s, {} processes",
            info.uptime_ms / 1000,
            info.uptime_ms % 1000,
            info.process_count
        );
    }
    0
}

fn cmd_free() -> i32 {
    if let Ok(info) = sysinfo() {
        println!("              total        used        free   kheap_used");
        let total_kb = info.total_ram_bytes / 1024;
        let free_kb = info.free_ram_bytes / 1024;
        let used_kb = total_kb.saturating_sub(free_kb);
        let kh_kb = info.kernel_heap_used / 1024;
        println!(
            "Mem:     {:10}  {:10}  {:10}   {:10} KiB",
            total_kb, used_kb, free_kb, kh_kb
        );
    }
    0
}

fn cmd_df() -> i32 {
    println!("Filesystem      1K-blocks      Used       Available  Use%  Mounted on");
    println!("aetherfs-v1  2199023255552       128   2199023255424    1%  /");
    println!("devfs                   0         0               0    0%  /dev");
    println!("procfs                  0         0               0    0%  /proc");
    println!("tmpfs               65536        16           65520    1%  /tmp");
    0
}

fn cmd_dmesg() -> i32 {
    let mut buf = vec![0u8; 16384];
    if let Ok(n) = dmesg(&mut buf) {
        let _ = write(1, &buf[..n]);
    }
    0
}

fn main(args: &[&str]) -> i32 {
    let prog = args
        .first()
        .copied()
        .unwrap_or("coreutils")
        .rsplit('/')
        .next()
        .unwrap_or("coreutils");

    let (effective_prog, effective_args): (&str, &[&str]) =
        if prog == "coreutils" && args.len() >= 2 {
            (args[1].rsplit('/').next().unwrap_or(args[1]), &args[1..])
        } else {
            (prog, args)
        };

    match effective_prog {
        "ls" => cmd_ls(effective_args),
        "cat" => cmd_cat(effective_args),
        "grep" => cmd_grep(effective_args),
        "wc" => cmd_wc(effective_args),
        "ps" => cmd_ps(),
        "id" => cmd_id(),
        "whoami" => cmd_whoami(),
        "groups" => cmd_groups(),
        "stat" => cmd_stat(effective_args),
        "chmod" => cmd_chmod(effective_args),
        "chown" => cmd_chown(effective_args),
        "uname" => cmd_uname(),
        "uptime" => cmd_uptime(),
        "free" => cmd_free(),
        "df" => cmd_df(),
        "dmesg" => cmd_dmesg(),
        "mkdir" => {
            if let Some(&p) = effective_args.get(1) {
                if let Err(e) = mkdir(p, 0o755) {
                    eprintln!("mkdir: {}: errno {}", p, e);
                    return 1;
                }
            }
            0
        }
        "rmdir" => {
            if let Some(&p) = effective_args.get(1) {
                if let Err(e) = rmdir(p) {
                    eprintln!("rmdir: {}: errno {}", p, e);
                    return 1;
                }
            }
            0
        }
        "touch" => {
            if let Some(&p) = effective_args.get(1) {
                if let Ok(fd) = open(p, O_CREAT | O_WRONLY, 0o644) {
                    let _ = close(fd);
                }
            }
            0
        }
        "rm" => {
            if let Some(&p) = effective_args.get(1) {
                if let Err(e) = unlink(p) {
                    eprintln!("rm: {}: errno {}", p, e);
                    return 1;
                }
            }
            0
        }
        "cp" => {
            if effective_args.len() >= 3 {
                match read_file_to_string(effective_args[1]) {
                    Ok(s) => {
                        let _ = write_string_to_file(effective_args[2], &s, false);
                    }
                    Err(e) => {
                        eprintln!("cp: {}: errno {}", effective_args[1], e);
                        return 1;
                    }
                }
            }
            0
        }
        "mv" => {
            if effective_args.len() >= 3 {
                if let Err(e) = rename(effective_args[1], effective_args[2]) {
                    eprintln!("mv: errno {}", e);
                    return 1;
                }
            }
            0
        }
        "kill" => {
            if let Some(pid_str) = effective_args.get(1) {
                let pid = pid_str.parse::<u32>().unwrap_or(0);
                if let Err(e) = kill(pid, 15) {
                    eprintln!("kill: ({}) - errno {}", pid, e);
                    return 1;
                }
            }
            0
        }
        "mount" => {
            if effective_args.len() < 4 {
                if let Ok(s) = read_file_to_string("/proc/mounts") {
                    print!("{}", s);
                }
                return 0;
            }
            match mount(effective_args[1], effective_args[2], effective_args[3], 0) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("mount: errno {}", e);
                    1
                }
            }
        }
        "umount" => {
            if let Some(&tgt) = effective_args.get(1) {
                if let Err(e) = umount(tgt) {
                    eprintln!("umount: {}: errno {}", tgt, e);
                    return 1;
                }
            }
            0
        }
        "useradd" => {
            if effective_args.len() < 2 {
                eprintln!("usage: useradd <username> [uid]");
                return 1;
            }
            let uname = effective_args[1];
            let uid = effective_args
                .get(2)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1001);
            match useradd(uid, uid, uname) {
                Ok(()) => {
                    println!("useradd: created user '{}' (UID={})", uname, uid);
                    0
                }
                Err(e) => {
                    eprintln!("useradd: failed: errno {}", e);
                    1
                }
            }
        }
        "chgrp" => {
            if effective_args.len() < 3 {
                eprintln!("usage: chgrp <gid> <path>");
                return 1;
            }
            let gid = effective_args[1].parse::<u32>().unwrap_or(0);
            let path = effective_args[2];
            let cur_uid = stat(path).map(|s| s.uid).unwrap_or(0);
            if let Err(e) = chown(path, cur_uid, gid) {
                eprintln!("chgrp: {}: errno {}", path, e);
                return 1;
            }
            0
        }
        "getfacl" => {
            if effective_args.len() < 2 {
                eprintln!("usage: getfacl <path>");
                return 1;
            }
            let path = effective_args[1];
            let st = match stat(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("getfacl: {}: errno {}", path, e);
                    return 1;
                }
            };
            println!("# file: {}", path);
            println!(
                "# owner: {} ({})",
                st.uid,
                Uuid::from_bytes(st.owner_user_uuid)
            );
            println!(
                "# group: {} ({})",
                st.gid,
                Uuid::from_bytes(st.owner_group_uuid)
            );
            let mut acl = [libaether::UserAclEntry {
                subject_uuid: [0; 16],
                is_group: 0,
                allow_rwx: 0,
                deny_rwx: 0,
                _pad: 0,
            }; 16];
            if let Ok(n) = libaether::get_acl(path, &mut acl) {
                for ace in acl.iter().take(n) {
                    let kind = if ace.is_group == 0 { "user" } else { "group" };
                    let r = if (ace.allow_rwx & 4) != 0 { 'r' } else { '-' };
                    let w = if (ace.allow_rwx & 2) != 0 { 'w' } else { '-' };
                    let x = if (ace.allow_rwx & 1) != 0 { 'x' } else { '-' };
                    println!(
                        "{}:{}:{}{}{}",
                        kind,
                        Uuid::from_bytes(ace.subject_uuid),
                        r,
                        w,
                        x
                    );
                }
            }
            0
        }
        "setfacl" => {
            // usage: setfacl <uuid> <rwx_octal_0_to_7> <path>
            if effective_args.len() < 4 {
                eprintln!("usage: setfacl <subject_uuid> <rwx_mask> <path>");
                return 1;
            }
            let uuid = match Uuid::parse_str(effective_args[1]) {
                Some(u) => u,
                None => {
                    eprintln!("setfacl: invalid UUID '{}'", effective_args[1]);
                    return 1;
                }
            };
            let allow = effective_args[2].parse::<u8>().unwrap_or(4) & 7;
            let path = effective_args[3];
            let entry = libaether::UserAclEntry {
                subject_uuid: *uuid.as_bytes(),
                is_group: 0,
                allow_rwx: allow,
                deny_rwx: 0,
                _pad: 0,
            };
            if let Err(e) = libaether::set_acl(path, &[entry]) {
                eprintln!("setfacl: {}: errno {}", path, e);
                return 1;
            }
            0
        }
        other => {
            eprintln!("coreutils: unknown utility '{}'", other);
            1
        }
    }
}

aether_main!(main);
