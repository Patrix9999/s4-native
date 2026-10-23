//! The guest heap (`malloc`, `operator new`, `HeapAlloc`, `VirtualAlloc`):
//! 16-byte aligned blocks with an 8-byte size header, free lists by size class.

use s4_x86::Cpu;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub const HEAP_START: u32 = 0x4000_0000;
pub const HEAP_END: u32 = 0x6000_0000;
/// Page-granular allocations (`VirtualAlloc`, thread stacks).
pub const VM_START: u32 = 0x6000_0000;
pub const VM_END: u32 = 0x7800_0000;
const VM_EXT_START: u32 = 0x8000_0000;
const VM_EXT_END: u32 = 0xe000_0000;

struct Heap {
    next: u32,
    free: BTreeMap<u32, Vec<u32>>,
    vm_next: u32,
    vm_allocs: BTreeMap<u32, u32>,
    vm_free: BTreeMap<u32, u32>,
}

static HEAP: Mutex<Heap> = Mutex::new(Heap {
    next: HEAP_START,
    free: BTreeMap::new(),
    vm_next: VM_START,
    vm_allocs: BTreeMap::new(),
    vm_free: BTreeMap::new(),
});

pub fn init() {
    *HEAP.lock().unwrap() = Heap {
        next: HEAP_START,
        free: BTreeMap::new(),
        vm_next: VM_START,
        vm_allocs: BTreeMap::new(),
        vm_free: BTreeMap::new(),
    };
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
    let sz = (size.max(1) + 0xfff) & !0xfff;
    let mut h = HEAP.lock().unwrap();

    // 1. Try to find a fitting free block (first fit)
    let mut found = None;
    for (&addr, &blk_sz) in &h.vm_free {
        if blk_sz >= sz {
            found = Some((addr, blk_sz));
            break;
        }
    }

    let p = if let Some((addr, blk_sz)) = found {
        h.vm_free.remove(&addr);
        if blk_sz > sz {
            h.vm_free.insert(addr + sz, blk_sz - sz);
        }
        addr
    } else {
        if h.vm_next + sz > VM_END && h.vm_next < VM_EXT_START {
            h.vm_next = VM_EXT_START;
        }
        let p = h.vm_next;
        h.vm_next += sz;
        let limit = if p >= VM_EXT_START { VM_EXT_END } else { VM_END };
        assert!(h.vm_next <= limit, "guest virtual memory exhausted");
        p
    };

    h.vm_allocs.insert(p, sz);
    c.mem.fill(p, 0, sz as usize);
    p
}

/// Frees page-aligned memory allocated with `vm_alloc` (`VirtualFree`, `UnmapViewOfFile`).
pub fn vm_free(c: &mut Cpu, p: u32, size: u32) {
    let _ = c;
    if p == 0 {
        return;
    }
    let mut h = HEAP.lock().unwrap();
    let sz = if let Some(allocated_sz) = h.vm_allocs.remove(&p) {
        allocated_sz
    } else if let Some((&base, &base_sz)) = h.vm_allocs.iter().find(|&(base, base_sz)| p >= *base && p < *base + *base_sz) {
        if p == base {
            let s = base_sz;
            h.vm_allocs.remove(&base);
            s
        } else {
            return;
        }
    } else if size > 0 {
        (size + 0xfff) & !0xfff
    } else {
        return;
    };

    if p + sz == h.vm_next {
        h.vm_next = p;
        while let Some((&prev_addr, &prev_sz)) = h.vm_free.iter().next_back() {
            if prev_addr + prev_sz == h.vm_next {
                h.vm_next = prev_addr;
                h.vm_free.remove(&prev_addr);
            } else {
                break;
            }
        }
        return;
    }

    let mut new_addr = p;
    let mut new_sz = sz;

    let prev = h.vm_free.range(..new_addr).next_back().map(|(&a, &s)| (a, s));
    if let Some((prev_addr, prev_sz)) = prev {
        if prev_addr + prev_sz == new_addr {
            h.vm_free.remove(&prev_addr);
            new_addr = prev_addr;
            new_sz += prev_sz;
        }
    }

    if let Some(next_sz) = h.vm_free.remove(&(new_addr + new_sz)) {
        new_sz += next_sz;
    }

    if new_addr + new_sz == h.vm_next {
        h.vm_next = new_addr;
    } else {
        h.vm_free.insert(new_addr, new_sz);
    }
}
