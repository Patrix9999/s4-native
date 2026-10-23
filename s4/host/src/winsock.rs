//! Winsock 1.1/2 on the host's BSD sockets. Addresses (`sockaddr_in`) have the
//! same layout; constants and error numbers are translated.

use crate::Shim;
use s4_x86::Cpu;
use std::cell::Cell;

macro_rules! sh {
    ($name:expr, $args:expr, $f:expr) => {
        Shim { dll: "ws2_32.dll", name: $name, args: $args, cdecl: false, f: $f }
    };
}

thread_local! {
    /// UDP sockets the game bound to a specific local address: fd -> that address.
    static BOUND_IP: std::cell::RefCell<std::collections::HashMap<i32, [u8; 4]>> = Default::default();

    static WSA_ERROR: Cell<u32> = const { Cell::new(0) };
}

#[cfg(unix)]
mod sys {
    pub type RawSocket = i32;
    pub type SockLen = libc::socklen_t;

    pub const MSG_NOSIGNAL: i32 = libc::MSG_NOSIGNAL;
    pub const SOL_SOCKET: i32 = libc::SOL_SOCKET;
    pub const SO_TYPE: i32 = libc::SO_TYPE;
    pub const SOCK_DGRAM: i32 = libc::SOCK_DGRAM;

    pub unsafe fn socket(af: i32, ty: i32, proto: i32) -> RawSocket {
        unsafe { libc::socket(af, ty, proto) }
    }

    pub unsafe fn closesocket(s: RawSocket) -> i32 {
        unsafe { libc::close(s) }
    }

    pub unsafe fn bind(s: RawSocket, name: *const u8, namelen: SockLen) -> i32 {
        unsafe { libc::bind(s, name.cast(), namelen) }
    }

    pub unsafe fn connect(s: RawSocket, name: *const u8, namelen: SockLen) -> i32 {
        unsafe { libc::connect(s, name.cast(), namelen) }
    }

    pub unsafe fn listen(s: RawSocket, backlog: i32) -> i32 {
        unsafe { libc::listen(s, backlog) }
    }

    pub unsafe fn accept(s: RawSocket, addr: *mut u8, addrlen: *mut SockLen) -> RawSocket {
        unsafe { libc::accept(s, addr.cast(), addrlen) }
    }

    pub unsafe fn send(s: RawSocket, buf: *const u8, len: usize, flags: i32) -> i32 {
        unsafe { libc::send(s, buf.cast(), len, flags) as i32 }
    }

    pub unsafe fn sendto(s: RawSocket, buf: *const u8, len: usize, flags: i32, to: *const u8, tolen: SockLen) -> i32 {
        unsafe { libc::sendto(s, buf.cast(), len, flags, to.cast(), tolen) as i32 }
    }

    pub unsafe fn recv(s: RawSocket, buf: *mut u8, len: usize, flags: i32) -> i32 {
        unsafe { libc::recv(s, buf.cast(), len, flags) as i32 }
    }

    pub unsafe fn recvfrom(s: RawSocket, buf: *mut u8, len: usize, flags: i32, from: *mut u8, fromlen: *mut SockLen) -> i32 {
        unsafe { libc::recvfrom(s, buf.cast(), len, flags, from.cast(), fromlen) as i32 }
    }

    pub unsafe fn shutdown(s: RawSocket, how: i32) -> i32 {
        unsafe { libc::shutdown(s, how) }
    }

    pub unsafe fn getsockname(s: RawSocket, name: *mut u8, namelen: *mut SockLen) -> i32 {
        unsafe { libc::getsockname(s, name.cast(), namelen) }
    }

    pub unsafe fn getpeername(s: RawSocket, name: *mut u8, namelen: *mut SockLen) -> i32 {
        unsafe { libc::getpeername(s, name.cast(), namelen) }
    }

    pub unsafe fn setsockopt(s: RawSocket, level: i32, name: i32, val: *const u8, len: SockLen) -> i32 {
        unsafe { libc::setsockopt(s, level, name, val.cast(), len) }
    }

    pub unsafe fn getsockopt(s: RawSocket, level: i32, name: i32, val: *mut u8, len: *mut SockLen) -> i32 {
        unsafe { libc::getsockopt(s, level, name, val.cast(), len) }
    }

    pub unsafe fn set_nonblocking(s: RawSocket, on: bool) -> i32 {
        unsafe {
            let fl = libc::fcntl(s, libc::F_GETFL);
            let nf = if on { fl | libc::O_NONBLOCK } else { fl & !libc::O_NONBLOCK };
            libc::fcntl(s, libc::F_SETFL, nf)
        }
    }

    pub unsafe fn fionread(s: RawSocket, out: *mut i32) -> i32 {
        unsafe { libc::ioctl(s, libc::FIONREAD, out) }
    }

    pub unsafe fn select(
        r: &[i32],
        w: &[i32],
        e: &[i32],
        has_r: bool,
        has_w: bool,
        has_e: bool,
        timeout: Option<(u32, u32)>,
    ) -> (i32, Vec<i32>, Vec<i32>, Vec<i32>) {
        let set = |v: &[i32]| -> (libc::fd_set, i32) {
            let mut s: libc::fd_set = unsafe { std::mem::zeroed() };
            unsafe { libc::FD_ZERO(&mut s) };
            let mut m = -1;
            for &f in v {
                unsafe { libc::FD_SET(f, &mut s) };
                m = m.max(f);
            }
            (s, m)
        };
        let ((mut rs, rm), (mut ws, wm), (mut es, em)) = (set(r), set(w), set(e));
        let mut tv;
        let tvp = if let Some((sec, usec)) = timeout {
            tv = libc::timeval { tv_sec: sec as _, tv_usec: usec as _ };
            &mut tv as *mut libc::timeval
        } else {
            std::ptr::null_mut()
        };
        let max_fd = rm.max(wm).max(em);
        let n = unsafe {
            libc::select(
                max_fd + 1,
                if has_r { &mut rs } else { std::ptr::null_mut() },
                if has_w { &mut ws } else { std::ptr::null_mut() },
                if has_e { &mut es } else { std::ptr::null_mut() },
                tvp,
            )
        };
        let check_ready = |v: &[i32], s: &libc::fd_set| -> Vec<i32> {
            v.iter().copied().filter(|&f| unsafe { libc::FD_ISSET(f, s) }).collect()
        };
        (n, check_ready(r, &rs), check_ready(w, &ws), check_ready(e, &es))
    }
}

#[cfg(windows)]
#[allow(non_snake_case)]
mod sys {
    pub type RawSocket = usize;
    pub type SockLen = i32;

    pub const MSG_NOSIGNAL: i32 = 0;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_TYPE: i32 = 0x1008;
    pub const SOCK_DGRAM: i32 = 2;

    #[repr(C)]
    struct WinTimeVal {
        tv_sec: i32,
        tv_usec: i32,
    }

    #[repr(C)]
    struct WinFdSet {
        count: u32,
        array: [usize; 64],
    }

    mod ffi {
        use super::{WinFdSet, WinTimeVal};

        #[link(name = "ws2_32")]
        unsafe extern "system" {
            pub fn WSAStartup(wVersionRequired: u16, lpWSAData: *mut u8) -> i32;
            pub fn WSACleanup() -> i32;
            pub fn WSAGetLastError() -> i32;
            pub fn socket(af: i32, socket_type: i32, protocol: i32) -> usize;
            pub fn closesocket(s: usize) -> i32;
            pub fn bind(s: usize, name: *const u8, namelen: i32) -> i32;
            pub fn connect(s: usize, name: *const u8, namelen: i32) -> i32;
            pub fn listen(s: usize, backlog: i32) -> i32;
            pub fn accept(s: usize, addr: *mut u8, addrlen: *mut i32) -> usize;
            pub fn send(s: usize, buf: *const u8, len: i32, flags: i32) -> i32;
            pub fn sendto(s: usize, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32;
            pub fn recv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
            pub fn recvfrom(s: usize, buf: *mut u8, len: i32, flags: i32, from: *mut u8, fromlen: *mut i32) -> i32;
            pub fn shutdown(s: usize, how: i32) -> i32;
            pub fn getsockname(s: usize, name: *mut u8, namelen: *mut i32) -> i32;
            pub fn getpeername(s: usize, name: *mut u8, namelen: *mut i32) -> i32;
            pub fn setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
            pub fn getsockopt(s: usize, level: i32, optname: i32, optval: *mut u8, optlen: *mut i32) -> i32;
            pub fn ioctlsocket(s: usize, cmd: i32, argp: *mut u32) -> i32;
            pub fn select(nfds: i32, readfds: *mut WinFdSet, writefds: *mut WinFdSet, exceptfds: *mut WinFdSet, timeout: *const WinTimeVal) -> i32;
            pub fn gethostname(name: *mut u8, namelen: i32) -> i32;
        }
    }

    pub unsafe fn WSAStartup(wVersionRequired: u16, lpWSAData: *mut u8) -> i32 {
        unsafe { ffi::WSAStartup(wVersionRequired, lpWSAData) }
    }

    pub unsafe fn WSACleanup() -> i32 {
        unsafe { ffi::WSACleanup() }
    }

    pub unsafe fn WSAGetLastError() -> i32 {
        unsafe { ffi::WSAGetLastError() }
    }

    pub unsafe fn gethostname(name: *mut u8, namelen: i32) -> i32 {
        unsafe { ffi::gethostname(name, namelen) }
    }

    pub unsafe fn socket(af: i32, ty: i32, proto: i32) -> RawSocket {
        unsafe { ffi::socket(af, ty, proto) }
    }

    pub unsafe fn closesocket(s: RawSocket) -> i32 {
        unsafe { ffi::closesocket(s) }
    }

    pub unsafe fn bind(s: RawSocket, name: *const u8, namelen: SockLen) -> i32 {
        unsafe { ffi::bind(s, name, namelen) }
    }

    pub unsafe fn connect(s: RawSocket, name: *const u8, namelen: SockLen) -> i32 {
        unsafe { ffi::connect(s, name, namelen) }
    }

    pub unsafe fn listen(s: RawSocket, backlog: i32) -> i32 {
        unsafe { ffi::listen(s, backlog) }
    }

    pub unsafe fn accept(s: RawSocket, addr: *mut u8, addrlen: *mut SockLen) -> RawSocket {
        unsafe { ffi::accept(s, addr, addrlen) }
    }

    pub unsafe fn send(s: RawSocket, buf: *const u8, len: usize, flags: i32) -> i32 {
        unsafe { ffi::send(s, buf, len as i32, flags) }
    }

    pub unsafe fn sendto(s: RawSocket, buf: *const u8, len: usize, flags: i32, to: *const u8, tolen: SockLen) -> i32 {
        unsafe { ffi::sendto(s, buf, len as i32, flags, to, tolen) }
    }

    pub unsafe fn recv(s: RawSocket, buf: *mut u8, len: usize, flags: i32) -> i32 {
        unsafe { ffi::recv(s, buf, len as i32, flags) }
    }

    pub unsafe fn recvfrom(s: RawSocket, buf: *mut u8, len: usize, flags: i32, from: *mut u8, fromlen: *mut SockLen) -> i32 {
        unsafe { ffi::recvfrom(s, buf, len as i32, flags, from, fromlen) }
    }

    pub unsafe fn shutdown(s: RawSocket, how: i32) -> i32 {
        unsafe { ffi::shutdown(s, how) }
    }

    pub unsafe fn getsockname(s: RawSocket, name: *mut u8, namelen: *mut SockLen) -> i32 {
        unsafe { ffi::getsockname(s, name, namelen) }
    }

    pub unsafe fn getpeername(s: RawSocket, name: *mut u8, namelen: *mut SockLen) -> i32 {
        unsafe { ffi::getpeername(s, name, namelen) }
    }

    pub unsafe fn setsockopt(s: RawSocket, level: i32, name: i32, val: *const u8, len: SockLen) -> i32 {
        unsafe { ffi::setsockopt(s, level, name, val, len) }
    }

    pub unsafe fn getsockopt(s: RawSocket, level: i32, name: i32, val: *mut u8, len: *mut SockLen) -> i32 {
        unsafe { ffi::getsockopt(s, level, name, val, len) }
    }

    pub unsafe fn set_nonblocking(s: RawSocket, on: bool) -> i32 {
        let mut mode: u32 = if on { 1 } else { 0 };
        unsafe { ffi::ioctlsocket(s, 0x8004_667e_u32 as i32, &mut mode) }
    }

    pub unsafe fn fionread(s: RawSocket, out: *mut i32) -> i32 {
        let mut n: u32 = 0;
        let r = unsafe { ffi::ioctlsocket(s, 0x4004_667f_u32 as i32, &mut n) };
        unsafe { *out = n as i32 };
        r
    }

    pub unsafe fn select(
        r: &[i32],
        w: &[i32],
        e: &[i32],
        has_r: bool,
        has_w: bool,
        has_e: bool,
        timeout: Option<(u32, u32)>,
    ) -> (i32, Vec<i32>, Vec<i32>, Vec<i32>) {
        let make_set = |v: &[i32]| -> WinFdSet {
            let mut s = WinFdSet { count: 0, array: [0; 64] };
            let count = v.len().min(64);
            s.count = count as u32;
            for i in 0..count {
                s.array[i] = v[i] as usize;
            }
            s
        };
        let mut rs = make_set(r);
        let mut ws = make_set(w);
        let mut es = make_set(e);
        let tv = timeout.map(|(sec, usec)| WinTimeVal { tv_sec: sec as i32, tv_usec: usec as i32 });
        let tvp = match &tv {
            Some(t) => t as *const WinTimeVal,
            None => std::ptr::null(),
        };
        let n = unsafe {
            ffi::select(
                0,
                if has_r { &mut rs } else { std::ptr::null_mut() },
                if has_w { &mut ws } else { std::ptr::null_mut() },
                if has_e { &mut es } else { std::ptr::null_mut() },
                tvp,
            )
        };
        let to_ready = |s: &WinFdSet| -> Vec<i32> {
            (0..s.count as usize).map(|i| s.array[i] as i32).collect()
        };
        (n, to_ready(&rs), to_ready(&ws), to_ready(&es))
    }
}

fn wsa_error() -> u32 {
    #[cfg(unix)]
    let w = {
        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        match e {
            libc::EWOULDBLOCK => 10035,
            libc::EINPROGRESS => 10036,
            libc::EALREADY => 10037,
            libc::ENOTSOCK => 10038,
            libc::EMSGSIZE => 10040,
            libc::EPROTOTYPE => 10041,
            libc::ENOPROTOOPT => 10042,
            libc::EPROTONOSUPPORT => 10043,
            libc::EOPNOTSUPP => 10045,
            libc::EAFNOSUPPORT => 10047,
            libc::EADDRINUSE => 10048,
            libc::EADDRNOTAVAIL => 10049,
            libc::ENETUNREACH => 10051,
            libc::ECONNABORTED => 10053,
            libc::ECONNRESET => 10054,
            libc::ENOBUFS => 10055,
            libc::EISCONN => 10056,
            libc::ENOTCONN => 10057,
            libc::ETIMEDOUT => 10060,
            libc::ECONNREFUSED => 10061,
            libc::EHOSTUNREACH => 10065,
            libc::EINVAL => 10022,
            libc::EBADF => 10009,
            libc::EACCES => 10013,
            libc::EFAULT => 10014,
            _ => 10000 + e as u32,
        }
    };
    #[cfg(windows)]
    let w = {
        let err = unsafe { sys::WSAGetLastError() };
        if err != 0 {
            err as u32
        } else {
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32
        }
    };
    #[cfg(not(any(unix, windows)))]
    let w = 0;

    WSA_ERROR.with(|c| c.set(w));
    w
}

fn check(r: i32) -> u32 {
    if r < 0 {
        wsa_error();
        0xffff_ffff
    } else {
        r as u32
    }
}

fn check_socket(s: sys::RawSocket) -> u32 {
    #[cfg(unix)]
    {
        check(s)
    }
    #[cfg(windows)]
    {
        if s == !0 || (s as isize) < 0 {
            wsa_error();
            0xffff_ffff
        } else {
            s as u32
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        check(s as i32)
    }
}

fn sockaddr(c: &Cpu, p: u32, len: u32) -> Vec<u8> {
    c.mem.read_bytes(p, len as usize)
}

fn opt_level(level: u32, name: u32) -> (i32, i32) {
    #[cfg(unix)]
    {
        if level == 0xffff {
            let n = match name {
                4 => libc::SO_REUSEADDR,
                8 => libc::SO_KEEPALIVE,
                0x20 => libc::SO_BROADCAST,
                0x80 => libc::SO_LINGER,
                0x1001 => libc::SO_SNDBUF,
                0x1002 => libc::SO_RCVBUF,
                0x1007 => libc::SO_ERROR,
                0x1008 => libc::SO_TYPE,
                _ => name as i32,
            };
            (libc::SOL_SOCKET, n)
        } else if level == 6 {
            (libc::IPPROTO_TCP, name as i32)
        } else {
            (level as i32, name as i32)
        }
    }
    #[cfg(not(unix))]
    {
        (level as i32, name as i32)
    }
}

fn host_name() -> String {
    let mut buf = [0u8; 256];
    #[cfg(unix)]
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    #[cfg(windows)]
    let rc = unsafe { sys::gethostname(buf.as_mut_ptr(), buf.len() as i32) };
    #[cfg(not(any(unix, windows)))]
    let rc = -1;

    if rc != 0 {
        #[cfg(windows)]
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            if !name.is_empty() {
                return name;
            }
        }
        return "localhost".into();
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The address of the interface that routes to the LAN/internet (no packet is sent), else loopback.
fn local_ip() -> std::net::Ipv4Addr {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok();
    match sock.as_ref().and_then(|s| s.connect("192.0.2.1:9").ok().and_then(|_| s.local_addr().ok())) {
        Some(std::net::SocketAddr::V4(v)) => *v.ip(),
        _ => std::net::Ipv4Addr::LOCALHOST,
    }
}

pub fn all() -> Vec<Shim> {
    vec![
        sh!("WSAStartup", 2, |c, a| {
            #[cfg(windows)]
            unsafe {
                let mut data = [0u8; 512];
                sys::WSAStartup(0x0202, data.as_mut_ptr());
            }
            // WSADATA: version 2.2, high version 2.2, description.
            c.mem.fill(a[1], 0, 400);
            c.mem.w16(a[1], 0x0202);
            c.mem.w16(a[1] + 2, 0x0202);
            0
        }),
        sh!("WSACleanup", 0, |_, _| {
            #[cfg(windows)]
            unsafe {
                sys::WSACleanup();
            }
            0
        }),
        sh!("WSAGetLastError", 0, |_, _| WSA_ERROR.with(|c| c.get())),
        sh!("socket", 3, |_, a| check_socket(unsafe { sys::socket(a[0] as i32, a[1] as i32, a[2] as i32) })),
        sh!("closesocket", 1, |_, a| {
            BOUND_IP.with(|m| m.borrow_mut().remove(&(a[0] as i32)));
            check(unsafe { sys::closesocket(a[0] as sys::RawSocket) })
        }),
        sh!("bind", 3, |c, a| {
            let sa = sockaddr(c, a[1], a[2]);
            if std::env::var_os("S4_HOST_TRACE").is_some() {
                eprintln!("  bind {:02x?}", &sa[..sa.len().min(8)]);
            }
            let mut sa = sa;
            // A Windows UDP socket bound to a specific local address still receives broadcasts to its
            // port; Linux's does not. Bind to the wildcard and remember the address for getsockname.
            if sa.len() >= 8 && sa[0] == 2 && sa[4] != 0 && sa[4] != 127 {
                let mut ty = 0i32;
                let mut l = 4 as sys::SockLen;
                // SAFETY: valid out-parameters.
                unsafe { sys::getsockopt(a[0] as sys::RawSocket, sys::SOL_SOCKET, sys::SO_TYPE, (&mut ty as *mut i32).cast(), &mut l) };
                if ty == sys::SOCK_DGRAM {
                    BOUND_IP.with(|m| m.borrow_mut().insert(a[0] as i32, [sa[4], sa[5], sa[6], sa[7]]));
                    sa[4..8].fill(0);
                }
            }
            check(unsafe { sys::bind(a[0] as sys::RawSocket, sa.as_ptr(), a[2] as sys::SockLen) })
        }),
        sh!("connect", 3, |c, a| {
            let sa = sockaddr(c, a[1], a[2]);
            check(unsafe { sys::connect(a[0] as sys::RawSocket, sa.as_ptr(), a[2] as sys::SockLen) })
        }),
        sh!("listen", 2, |_, a| check(unsafe { sys::listen(a[0] as sys::RawSocket, a[1] as i32) })),
        sh!("accept", 3, |c, a| {
            let mut buf = [0u8; 128];
            let mut len: sys::SockLen = if a[2] != 0 { c.mem.r32(a[2]) as sys::SockLen } else { 16 };
            let r = unsafe { sys::accept(a[0] as sys::RawSocket, buf.as_mut_ptr(), &mut len) };
            let res = check_socket(r);
            if res != 0xffff_ffff && a[1] != 0 {
                c.mem.write_bytes(a[1], &buf[..len as usize]);
                c.mem.w32(a[2], len as u32);
            }
            res
        }),
        sh!("send", 4, |c, a| {
            let data = c.mem.read_bytes(a[1], a[2] as usize);
            check(unsafe { sys::send(a[0] as sys::RawSocket, data.as_ptr(), data.len(), a[3] as i32 | sys::MSG_NOSIGNAL) })
        }),
        sh!("sendto", 6, |c, a| {
            let data = c.mem.read_bytes(a[1], a[2] as usize);
            let sa = sockaddr(c, a[4], a[5]);
            if std::env::var_os("S4_HOST_TRACE").is_some() {
                eprintln!("  sendto {} bytes to {:?}: {:02x?}", data.len(), &sa[..sa.len().min(8)], &data[..data.len().min(16)]);
            }
            check(unsafe { sys::sendto(a[0] as sys::RawSocket, data.as_ptr(), data.len(), a[3] as i32 | sys::MSG_NOSIGNAL, sa.as_ptr(), a[5] as sys::SockLen) })
        }),
        sh!("recv", 4, |c, a| {
            let mut buf = vec![0u8; a[2] as usize];
            let r = unsafe { sys::recv(a[0] as sys::RawSocket, buf.as_mut_ptr(), buf.len(), a[3] as i32) };
            if r > 0 {
                c.mem.write_bytes(a[1], &buf[..r as usize]);
            }
            check(r)
        }),
        sh!("recvfrom", 6, |c, a| {
            let mut buf = vec![0u8; a[2] as usize];
            let mut sa = [0u8; 128];
            let mut len: sys::SockLen = if a[5] != 0 { c.mem.r32(a[5]) as sys::SockLen } else { 16 };
            let r = unsafe { sys::recvfrom(a[0] as sys::RawSocket, buf.as_mut_ptr(), buf.len(), a[3] as i32, sa.as_mut_ptr(), &mut len) };
            if r >= 0 {
                c.mem.write_bytes(a[1], &buf[..r as usize]);
                if a[4] != 0 {
                    c.mem.write_bytes(a[4], &sa[..len as usize]);
                    c.mem.w32(a[5], len as u32);
                }
            }
            check(r)
        }),
        sh!("shutdown", 2, |_, a| check(unsafe { sys::shutdown(a[0] as sys::RawSocket, a[1] as i32) })),
        sh!("getsockname", 3, |c, a| {
            let mut buf = [0u8; 128];
            let mut len: sys::SockLen = c.mem.r32(a[2]) as sys::SockLen;
            let r = unsafe { sys::getsockname(a[0] as sys::RawSocket, buf.as_mut_ptr(), &mut len) };
            if r == 0 {
                if len >= 8 && buf[0] == 2 && buf[4..8] == [0, 0, 0, 0] {
                    if let Some(ip) = BOUND_IP.with(|m| m.borrow().get(&(a[0] as i32)).copied()) {
                        buf[4..8].copy_from_slice(&ip);
                    }
                }
                c.mem.write_bytes(a[1], &buf[..len as usize]);
                c.mem.w32(a[2], len as u32);
            }
            check(r)
        }),
        sh!("getpeername", 3, |c, a| {
            let mut buf = [0u8; 128];
            let mut len: sys::SockLen = c.mem.r32(a[2]) as sys::SockLen;
            let r = unsafe { sys::getpeername(a[0] as sys::RawSocket, buf.as_mut_ptr(), &mut len) };
            if r == 0 {
                c.mem.write_bytes(a[1], &buf[..len as usize]);
                c.mem.w32(a[2], len as u32);
            }
            check(r)
        }),
        sh!("setsockopt", 5, |c, a| {
            let (l, n) = opt_level(a[1], a[2]);
            let v = c.mem.read_bytes(a[3], a[4] as usize);
            check(unsafe { sys::setsockopt(a[0] as sys::RawSocket, l, n, v.as_ptr(), a[4] as sys::SockLen) })
        }),
        sh!("getsockopt", 5, |c, a| {
            let (l, n) = opt_level(a[1], a[2]);
            let mut buf = [0u8; 64];
            let mut len: sys::SockLen = c.mem.r32(a[4]) as sys::SockLen;
            let r = unsafe { sys::getsockopt(a[0] as sys::RawSocket, l, n, buf.as_mut_ptr(), &mut len) };
            if r == 0 {
                c.mem.write_bytes(a[3], &buf[..len as usize]);
                c.mem.w32(a[4], len as u32);
            }
            check(r)
        }),
        sh!("ioctlsocket", 3, |c, a| {
            // FIONBIO = 0x8004667e
            if a[1] == 0x8004_667e {
                let on = c.mem.r32(a[2]) != 0;
                return check(unsafe { sys::set_nonblocking(a[0] as sys::RawSocket, on) });
            }
            if a[1] == 0x4004_667f {
                // FIONREAD
                let mut n: i32 = 0;
                let r = unsafe { sys::fionread(a[0] as sys::RawSocket, &mut n) };
                c.mem.w32(a[2], n as u32);
                return check(r);
            }
            0
        }),
        sh!("select", 5, |c, a| {
            let read = |p: u32| -> Vec<i32> {
                if p == 0 {
                    return Vec::new();
                }
                (0..c.mem.r32(p)).map(|i| c.mem.r32(p + 4 + 4 * i) as i32).collect()
            };
            let (r, w, e) = (read(a[1]), read(a[2]), read(a[3]));
            let timeout = if a[4] != 0 {
                Some((c.mem.r32(a[4]), c.mem.r32(a[4] + 4)))
            } else {
                None
            };
            let (n, ready_r, ready_w, ready_e) = unsafe {
                sys::select(&r, &w, &e, a[1] != 0, a[2] != 0, a[3] != 0, timeout)
            };
            let write_back = |p: u32, ready: &[i32]| {
                if p != 0 {
                    c.mem.w32(p, ready.len() as u32);
                    for (i, &f) in ready.iter().enumerate() {
                        c.mem.w32(p + 4 + 4 * i as u32, f as u32);
                    }
                }
            };
            if n >= 0 {
                write_back(a[1], &ready_r);
                write_back(a[2], &ready_w);
                write_back(a[3], &ready_e);
            }
            check(n)
        }),
        sh!("htons", 1, |_, a| (a[0] as u16).swap_bytes() as u32),
        sh!("ntohs", 1, |_, a| (a[0] as u16).swap_bytes() as u32),
        sh!("htonl", 1, |_, a| a[0].swap_bytes()),
        sh!("ntohl", 1, |_, a| a[0].swap_bytes()),
        sh!("inet_addr", 1, |c, a| {
            let s: String = c.mem.cstr(a[0]).iter().map(|&b| b as char).collect();
            match s.parse::<std::net::Ipv4Addr>() {
                Ok(ip) => u32::from_le_bytes(ip.octets()),
                Err(_) => 0xffff_ffff,
            }
        }),
        sh!("inet_ntoa", 1, |c, a| {
            let ip = a[0].to_le_bytes();
            let s = format!("{}.{}.{}.{}\0", ip[0], ip[1], ip[2], ip[3]);
            let p = crate::heap::alloc(c, 20);
            c.mem.write_bytes(p, s.as_bytes());
            p
        }),
        sh!("gethostname", 2, |c, a| {
            let mut name = host_name().into_bytes();
            name.push(0);
            if a[1] < name.len() as u32 {
                return 0xffff_ffff;
            }
            c.mem.write_bytes(a[0], &name);
            0
        }),
        sh!("gethostbyname", 1, |c, a| {
            let name: String = c.mem.cstr(a[0]).iter().map(|&b| b as char).collect();
            use std::net::ToSocketAddrs;
            let ip = match name.parse::<std::net::Ipv4Addr>() {
                Ok(ip) => Some(ip),
                // The machine's own name: its LAN address (what the game advertises to other players).
                Err(_) if name.eq_ignore_ascii_case(&host_name()) => Some(local_ip()),
                Err(_) => (name.as_str(), 0).to_socket_addrs().ok().and_then(|mut it| it.find_map(|s| if let std::net::SocketAddr::V4(v) = s { Some(*v.ip()) } else { None })),
            };
            let Some(ip) = ip else {
                WSA_ERROR.with(|c| c.set(11001));
                return 0;
            };
            // hostent: name, aliases, addrtype, length, addr_list
            let h = crate::heap::alloc_zeroed(c, 64);
            let addr = h + 32;
            c.mem.write_bytes(addr, &ip.octets());
            let list = h + 40;
            c.mem.w32(list, addr);
            c.mem.w32(list + 4, 0);
            c.mem.w32(h + 8, 2);
            c.mem.w16(h + 8, 2);
            c.mem.w16(h + 10, 4);
            c.mem.w32(h + 12, list);
            let nm = h + 48;
            c.mem.write_bytes(nm, b"host\0");
            c.mem.w32(h, nm);
            c.mem.w32(h + 4, h + 56);
            h
        }),
    ]
}
