//! name of a host from its address, as the system knows it (hosts file, DNS)

use std::ffi::CStr;
use std::net::Ipv4Addr;

/// the name of the host at this address, if it has one. blocks while the system looks it up
pub fn name(address: Ipv4Addr) -> Option<String> {
    let mut socket: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    socket.sin_family = libc::AF_INET as libc::sa_family_t;
    socket.sin_addr = libc::in_addr { s_addr: u32::from(address).to_be() };
    let mut host = [0 as libc::c_char; 256];
    let status = unsafe {
        libc::getnameinfo(
            (&socket as *const libc::sockaddr_in).cast(),
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            host.as_mut_ptr(),
            host.len() as libc::socklen_t,
            std::ptr::null_mut(),
            0,
            // no name is a failure, rather than the address as text
            libc::NI_NAMEREQD,
        )
    };
    if status != 0 {
        return None;
    }
    let name = unsafe { CStr::from_ptr(host.as_ptr()) }.to_string_lossy().into_owned();
    Some(name).filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_gives_a_name_or_nothing() {
        // depends on the system: only the shape of the answer is checked
        if let Some(name) = name(Ipv4Addr::LOCALHOST) {
            assert!(!name.is_empty());
            assert!(name.parse::<Ipv4Addr>().is_err(), "an address is not a name");
        }
        // reserved for documentation: has no name
        assert_eq!(name(Ipv4Addr::new(192, 0, 2, 1)), None);
    }
}
