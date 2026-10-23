//! The guest heap (`malloc`, `operator new`, `HeapAlloc`, `VirtualAlloc`):
//! 16-byte aligned blocks with an 8-byte size header, free lists by size class.

use s4_x86::Cpu;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const HEAP_START: u32 = 0x4000_0000;
pub const HEAP_END: u32 = 0x6000_0000;
/// Page-granular allocations (`VirtualAlloc`, thread stacks).
pub const VM_START: u32 = 0x6000_0000;
pub const VM_END: u32 = 0x7000_0000;

struct Heap {
    next: u32,
    free: BTreeMap<u32, Vec<u32>>,
    vm_next: u32,
}

static HEAP: Mutex<Heap> = Mutex::new(Heap { next: HEAP_START, free: BTreeMap::new(), vm_next: VM_START });

pub fn init() {
    *HEAP.lock().unwrap() = Heap { next: HEAP_START, free: BTreeMap::new(), vm_next: VM_START };
}

fn class(size: u32) -> u32 {
    (size.max(16) + 15) & !15
}

/// Allocates `size` bytes (contents unspecified unless fresh).
pub fn alloc(c: &mut Cpu, size: u32) -> u32 {
    let sz = class(size);
    let mut h = HEAP.lock().unwrap();
    if let Some(list) = h.free.get_mut(&sz) {
        if let Some(p) = list.pop() {
            if std::env::var_os("S4_ZERO_HEAP").is_some() {
                c.mem.fill(p, 0, sz as usize);
            }
            return p;
        }
    }
    let p = h.next + 16;
    h.next += 16 + sz;
    assert!(h.next < HEAP_END, "guest heap exhausted");
    c.mem.w32(p - 8, sz);
    p
}

pub fn alloc_zeroed(c: &mut Cpu, size: u32) -> u32 {
    let p = alloc(c, size);
    c.mem.fill(p, 0, size as usize);
    p
}

pub fn free(c: &mut Cpu, p: u32) {
    if p == 0 {
        return;
    }
    let sz = c.mem.r32(p - 8);
    HEAP.lock().unwrap().free.entry(sz).or_default().push(p);
}

pub fn size(c: &Cpu, p: u32) -> u32 {
    c.mem.r32(p - 8)
}

pub fn realloc(c: &mut Cpu, p: u32, size: u32) -> u32 {
    if p == 0 {
        return alloc(c, size);
    }
    let old = self::size(c, p);
    if class(size) == old {
        return p;
    }
    let n = alloc(c, size);
    let bytes = c.mem.read_bytes(p, old.min(size) as usize);
    c.mem.write_bytes(n, &bytes);
    free(c, p);
    n
}

/// Page-aligned zeroed memory (`VirtualAlloc`).
pub fn vm_alloc(c: &mut Cpu, size: u32) -> u32 {
    let sz = (size + 0xfff) & !0xfff;
    let mut h = HEAP.lock().unwrap();
    let p = h.vm_next;
    h.vm_next += sz;
    assert!(h.vm_next <= VM_END, "guest virtual memory exhausted");
    c.mem.fill(p, 0, sz as usize);
    p
}
