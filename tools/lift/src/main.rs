//! lift <binary> <functions.tsv> <out_dir> [--prefix f_] [--only addr,addr,...] [--module name] [--patch file]
//!
//! `--patch file`: lines `<address hex><TAB><old bytes hex><TAB><new bytes hex>` (`#` starts a comment) are applied to
//! the image before it is translated. Every old byte string is checked; any mismatch skips the whole file.
//!
//! Writes `<out_dir>/lifted_NNN.rs` (the functions), `<out_dir>/dispatch.rs`
//! (address -> function) and `<out_dir>/unsupported.txt`.

use lift::{cfg, eh, fn_name, header, lift_function, pe};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "cases").unwrap_or(false) {
        lift::cases::generate(&args[2..], &args[1]);
        return;
    }
    if args.len() < 3 {
        eprintln!("usage: lift <binary> <functions.tsv> <out_dir> [--prefix f_] [--only a,b]");
        std::process::exit(2);
    }
    let (binary, tsv, out_dir) = (&args[0], &args[1], &args[2]);
    let mut prefix = "f_".to_string();
    let mut only: Option<BTreeSet<u32>> = None;
    let mut patch: Option<String> = None;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--prefix" => {
                prefix = args[i + 1].clone();
                i += 2;
            }
            "--patch" => {
                patch = Some(args[i + 1].clone());
                i += 2;
            }
            "--only" => {
                only = Some(args[i + 1].split(',').map(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).unwrap()).collect());
                i += 2;
            }
            a => panic!("unknown argument {a}"),
        }
    }
    let mut img = pe::Image::load(binary);
    if let Some(file) = &patch {
        apply_patches(&mut img, file);
    }
    let mut entries = BTreeSet::new();
    let mut catch_funclets: BTreeSet<u32> = BTreeSet::new();
    let have_list = std::fs::metadata(tsv).is_ok();
    if let Ok(text) = std::fs::read_to_string(tsv) {
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            let a = u32::from_str_radix(f[0], 16).unwrap();
            entries.insert(a);
            // The Ghidra export has the name in column 3; the committed lists have a kind in column 2.
            if f.get(2).or(f.get(1)).map(|n| n.starts_with("Catch@")).unwrap_or(false) {
                catch_funclets.insert(a);
            }
        }
    }
    // Without Ghidra's list the exports and the entry point seed the search.
    for (_, a) in &img.exports {
        if img.is_code(*a) {
            entries.insert(*a);
        }
    }
    if img.entry != 0 && img.is_code(img.entry) {
        entries.insert(img.entry);
    }
    // The exception-handling tables name funclets and handler stubs that nothing calls directly.
    let ehi = eh::scan(&img);
    for &a in ehi.unwind_actions.iter().chain(&ehi.catch_handlers).chain(&ehi.stubs) {
        entries.insert(a);
    }
    if !have_list {
        catch_funclets.extend(ehi.catch_handlers.iter().copied());
    }
    // Entry points: the function list's functions, plus what they call directly and every code
    // address the image's relocations point at, that no known function contains.
    let mut base_entries = entries.clone();
    let mut hard: BTreeSet<u32> = entries.clone();
    let mut entries = entries;
    let mut covered: BTreeSet<u32> = BTreeSet::new();
    let mut table_targets: BTreeSet<u32> = BTreeSet::new();
    let mut seh_entries: BTreeSet<u32> = BTreeSet::new();
    let mut immediates: BTreeSet<u32> = BTreeSet::new();
    // Code addresses used as immediates inside catch handlers: where the handler resumes.
    let mut resume_points: BTreeSet<u32> = BTreeSet::new();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut pending: Vec<u32> = entries.iter().copied().collect();
    let mut candidates: BTreeSet<u32> = img.relocs.iter().filter_map(|&r| img.read32(r)).filter(|&t| img.is_code(t)).collect();
    if img.relocs.is_empty() {
        // No relocation table (the program): every aligned dword in the data sections that points into code.
        for sec in img.sections.iter().filter(|s| s.flags & 0x2000_0000 == 0) {
            let at = img.base + sec.rva;
            for off in (0..sec.data.len().saturating_sub(3)).step_by(4) {
                let v = u32::from_le_bytes(sec.data[off..off + 4].try_into().unwrap());
                if img.is_code(v) {
                    candidates.insert(v);
                }
            }
            let _ = at;
        }
    }
    // Instruction-pointer-to-state maps hold addresses inside functions.
    candidates.retain(|a| !ehi.ip_map.contains(a));
    let mut cand_pending: Vec<u32> = candidates.iter().copied().collect();
    hard.extend(candidates.iter().copied());
    loop {
        while let Some(a) = pending.pop() {
            if !seen.insert(a) {
                continue;
            }
            let f = cfg::discover(&img, a, &entries);
            if f.has_seh() {
                seh_entries.insert(a);
            }
            for ts in f.tables.values() {
                table_targets.extend(ts.iter().copied());
            }
            for (&ia, ins) in &f.insns {
                covered.insert(ia);
                for k in 0..ins.op_count() {
                    if ins.op_kind(k) == iced_x86::OpKind::Immediate32 {
                        let v = ins.immediate(k) as u32;
                        if img.is_code(v) {
                            cand_pending.push(v);
                            immediates.insert(v);
                            if catch_funclets.contains(&a) {
                                resume_points.insert(v);
                            }
                        }
                    }
                }
                if ins.mnemonic() == iced_x86::Mnemonic::Call && ins.near_branch_target() as u32 != ins.next_ip() as u32 && ins.op0_kind() != iced_x86::OpKind::Memory && ins.op0_kind() != iced_x86::OpKind::Register {
                    let t = ins.near_branch_target() as u32;
                    if img.is_code(t) {
                        hard.insert(t);
                    }
                    if img.is_code(t) && entries.insert(t) {
                        pending.push(t);
                    }
                }
            }
        }
        // Relocated code pointers that are not inside any function found so far.
        let mut added = false;
        while let Some(t) = cand_pending.pop() {
            if !table_targets.contains(&t) && !entries.contains(&t) {
                entries.insert(t);
                pending.push(t);
                added = true;
            }
        }
        if !added && pending.is_empty() {
            break;
        }
    }
    // Without a function list the boundaries are what the image itself shows: exports, direct
    // calls, code pointers in data (minus jump-table case labels) and the exception tables' funclets.
    if !have_list {
        base_entries = hard.difference(&table_targets).copied().collect();
    }
    // A code address used as an immediate inside the range of a function with an exception
    // frame is a catch continuation: a block of that function, not a function.
    let mut extras: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    // With a function list any such immediate inside a function with an exception frame is taken as a
    // continuation (the list tells where functions start); without one only the resume points of
    // catch handlers are, which the exception tables identify exactly.
    let continuation_candidates = if have_list { immediates.clone() } else { resume_points.clone() };
    for &t in &continuation_candidates {
        // Function starts follow alignment padding; continuations follow code.
        if matches!(img.read8(t.wrapping_sub(1)), Some(0x90) | Some(0xcc)) {
            continue;
        }
        // The parent: the nearest earlier function that is not a catch funclet; the
        // address must lie before the next ordinary function.
        let mut it = base_entries.range(..=t).rev().filter(|e| !catch_funclets.contains(e));
        if let Some(&p) = it.next() {
            let next = base_entries.range(t + 1..).find(|e| !catch_funclets.contains(e)).copied().unwrap_or(u32::MAX);
            if p != t && seh_entries.contains(&p) && t < next {
                extras.entry(p).or_default().push(t);
            }
        }
    }
    for list in extras.values() {
        for t in list {
            entries.remove(t);
        }
    }
    let targets: Vec<u32> = entries.iter().copied().filter(|a| only.as_ref().map(|o| o.contains(a)).unwrap_or(true)).collect();
    let all: BTreeSet<u32> = targets.iter().copied().collect();
    // Pass 1: which functions lift at all.
    let mut funs = BTreeMap::new();
    let mut failed: BTreeMap<u32, String> = BTreeMap::new();
    let mut gaps: Vec<String> = Vec::new();
    for &a in &targets {
        if std::env::var("LIFT_TRACE").is_ok() {
            eprintln!("{a:08x}");
        }
        let f = cfg::discover_extra(&img, a, &entries, extras.get(&a).map(|v| v.as_slice()).unwrap_or(&[]));
        if std::env::var("LIFT_TRACE").is_ok() {
            eprintln!("discovered {} insns", f.insns.len());
        }
        match lift_function(&img, &f, &all, &prefix) {
            Ok((_, g)) => {
                for line in g {
                    gaps.push(format!("{a:08x}\t{line}"));
                }
                funs.insert(a, f);
            }
            Err(e) => {
                failed.insert(a, e.0);
            }
        }
    }
    let ok: BTreeSet<u32> = funs.keys().copied().collect();
    // Pass 2: with direct calls only to what exists.
    std::fs::create_dir_all(out_dir).unwrap();
    let mut chunk = String::new();
    let mut files = 0;
    let mut dispatch = String::new();
    let mut count = 0;
    // Block starts of lifted functions that are not function entries: the original calls some of them
    // as functions (a decompiler's list knows them; the image alone does not), so they dispatch into
    // the function that contains them.
    let mut block_entries: BTreeSet<u32> = BTreeSet::new();
    for (&a, f) in &funs {
        chunk.push_str(&lift_function(&img, f, &ok, &prefix).unwrap().0);
        writeln!(dispatch, "        {a:#x} => {{ {}(c); true }}", fn_name(&prefix, a)).unwrap();
        // Only blocks that start right after a return or an unconditional jump (or padding) look like
        // function starts.
        let after_exit: BTreeSet<u32> = f
            .insns
            .values()
            .filter(|i| matches!(i.flow_control(), iced_x86::FlowControl::Return | iced_x86::FlowControl::UnconditionalBranch | iced_x86::FlowControl::IndirectBranch | iced_x86::FlowControl::Exception))
            .map(|i| i.next_ip() as u32)
            .collect();
        for &l in &f.leaders {
            let function_like = after_exit.contains(&l) || matches!(img.read8(l.wrapping_sub(1)), Some(0x90) | Some(0xcc));
            if l != a && function_like && !ok.contains(&l) && f.insns.contains_key(&l) && block_entries.insert(l) {
                writeln!(dispatch, "        {l:#x} => {{ {}_at(c, {l:#x}); true }}", fn_name(&prefix, a)).unwrap();
            }
        }
        count += 1;
        if count % 100 == 0 {
            flush(out_dir, &mut files, &mut chunk);
        }
    }
    flush(out_dir, &mut files, &mut chunk);
    let mut lib = String::new();
    lib.push_str("// Generated by tools/lift; do not edit.\n");
    for n in 0..files {
        writeln!(lib, "mod lifted_{n:03};\npub use lifted_{n:03}::*;").unwrap();
    }
    writeln!(lib, "\n/// Calls the lifted function at `addr`, if there is one.\npub fn dispatch(c: &mut s4_x86::Cpu, addr: u32) -> bool {{\n    match addr {{\n{dispatch}        _ => false,\n    }}\n}}").unwrap();
    std::fs::write(format!("{out_dir}/lib.rs"), lib).unwrap();
    let mut report = String::new();
    for (a, why) in &failed {
        writeln!(report, "{a:08x}\t{why}").unwrap();
    }
    std::fs::write(format!("{out_dir}/unsupported.txt"), report).unwrap();
    std::fs::write(format!("{out_dir}/gaps.txt"), gaps.join("\n")).unwrap();
    eprintln!("lifted {} of {} functions ({} unsupported)", ok.len(), targets.len(), failed.len());
}

fn flush(out_dir: &str, files: &mut usize, chunk: &mut String) {
    if chunk.is_empty() {
        return;
    }
    let text = format!("// Generated by tools/lift; do not edit.\n{}{}", header(), chunk);
    std::fs::write(format!("{out_dir}/lifted_{files:03}.rs"), text).unwrap();
    *files += 1;
    chunk.clear();
}

/// Applies a patch file (see the header). All or nothing: the original image is kept on any error.
fn apply_patches(img: &mut pe::Image, file: &str) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
    let hex = |s: &str| -> Vec<u8> { (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap_or_else(|_| panic!("{file}: bad hex {s:?}"))).collect() };
    let mut list = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() >= 3, "{file}: a patch line needs address, old bytes, new bytes");
        list.push((u32::from_str_radix(f[0].trim_start_matches("0x"), 16).unwrap(), hex(f[1].trim()), hex(f[2].trim())));
    }
    let mut done: Vec<&(u32, Vec<u8>, Vec<u8>)> = Vec::new();
    for p in &list {
        match img.patch(p.0, &p.1, &p.2) {
            Ok(()) => done.push(p),
            Err(e) => {
                // Undo what was applied: the image stays as loaded.
                for (a, old, new) in done.iter().rev().map(|p| (p.0, &p.1, &p.2)) {
                    let _ = img.patch(a, new, old);
                }
                eprintln!("patch file {file} not applied: {e}");
                return;
            }
        }
    }
    eprintln!("applied {} patches from {file}", done.len());
}
