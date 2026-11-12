//! What Net Connections shows about this computer (`inet_class::inet_init`, `inet_wind`).

use std::net::IpAddr;
use std::sync::OnceLock;

/// `WIN_get_computer_name`: `[[NSHost currentHost] localizedName]`, the Sharing pane's
/// computer name (`scutil --get ComputerName` reads the same setting).
pub fn computer_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    })
}

/// `inet_init`'s address table: every `getifaddrs` entry that is up and running and not
/// loopback (`flags & 0x49 == 0x41`), IPv4 or IPv6 except link-local fe80::/10, at most 16;
/// then sorted (family, then address) with duplicates dropped. `inet_wind` shows the first
/// and up to three more.
pub fn local_addresses() -> Vec<IpAddr> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs fills `head` with a list we only read and then free once.
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut p = head;
        while !p.is_null() {
            let ifa = &*p;
            p = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || ifa.ifa_flags & 0x49 != 0x41 {
                continue;
            }
            let ip = match i32::from((*ifa.ifa_addr).sa_family) {
                libc::AF_INET => {
                    let a = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    IpAddr::from(u32::from_be(a.sin_addr.s_addr).to_be_bytes())
                }
                libc::AF_INET6 => {
                    let a = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    let b = a.sin6_addr.s6_addr;
                    if b[0] == 0xfe && b[1] & 0xc0 == 0x80 {
                        continue;
                    }
                    IpAddr::from(b)
                }
                _ => continue,
            };
            if out.len() < 16 {
                out.push(ip);
            }
        }
        libc::freeifaddrs(head);
    }
    out.sort_by_key(|ip| match ip {
        IpAddr::V4(a) => (0, a.octets().to_vec()),
        IpAddr::V6(a) => (1, a.octets().to_vec()),
    });
    out.dedup();
    out.truncate(4);
    out
}
