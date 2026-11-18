//! looks for hosts of the local networks accepting connections on a port

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// connections tried at the same time
const WORKERS: usize = 32;

/// address and prefix length of the IPv4 networks this device is on, loopback aside
pub fn networks() -> Vec<(Ipv4Addr, u8)> {
    let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
    interfaces
        .into_iter()
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() => Some((v4.ip, v4.prefixlen)),
            _ => None,
        })
        .collect()
}

/// the other hosts of the network of `address`. a network bigger than 254 hosts is only
/// looked at around the address
pub fn candidates(address: Ipv4Addr, prefix: u8) -> Vec<Ipv4Addr> {
    let prefix = u32::from(prefix.clamp(24, 32));
    if prefix >= 31 {
        return Vec::new();
    }
    let size = 1u32 << (32 - prefix);
    let own = u32::from(address);
    let network = own & !(size - 1);
    // without the network and broadcast addresses
    (network + 1..network + size - 1).filter(|host| *host != own).map(Ipv4Addr::from).collect()
}

/// the addresses accepting a connection on `port` within `timeout`. blocks
pub fn sweep(addresses: &[Ipv4Addr], port: u16, timeout: Duration) -> Vec<Ipv4Addr> {
    let next = AtomicUsize::new(0);
    let answering = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(addresses.len()) {
            scope.spawn(|| loop {
                let Some(address) = addresses.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
                if TcpStream::connect_timeout(&SocketAddr::from((*address, port)), timeout).is_ok() {
                    answering.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(*address);
                }
            });
        }
    });
    let mut answering = answering.into_inner().unwrap_or_else(|poisoned| poisoned.into_inner());
    answering.sort();
    answering
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_of_a_small_network() {
        let hosts = candidates(Ipv4Addr::new(192, 168, 1, 20), 24);
        assert_eq!(hosts.len(), 253);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(hosts[252], Ipv4Addr::new(192, 168, 1, 254));
        assert!(!hosts.contains(&Ipv4Addr::new(192, 168, 1, 20)));

        let hosts = candidates(Ipv4Addr::new(10, 0, 0, 5), 29);
        let last: Vec<u8> = hosts.iter().map(|host| host.octets()[3]).collect();
        assert_eq!(last, [1, 2, 3, 4, 6]);
    }

    #[test]
    fn big_networks_are_looked_at_around_the_address() {
        let hosts = candidates(Ipv4Addr::new(10, 7, 3, 200), 8);
        assert_eq!(hosts.len(), 253);
        assert!(hosts.iter().all(|host| host.octets()[..3] == [10, 7, 3]));
    }

    #[test]
    fn point_to_point_links_have_no_other_host() {
        assert!(candidates(Ipv4Addr::new(10, 0, 0, 1), 31).is_empty());
        assert!(candidates(Ipv4Addr::new(10, 0, 0, 1), 32).is_empty());
        assert_eq!(candidates(Ipv4Addr::new(10, 0, 0, 1), 30), [Ipv4Addr::new(10, 0, 0, 2)]);
    }

    #[test]
    fn sweep_finds_what_listens() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let addresses = [Ipv4Addr::new(127, 0, 0, 2), Ipv4Addr::LOCALHOST];
        let timeout = Duration::from_millis(300);
        // the whole loopback network answers on linux, when something listens on all of it
        let answering = sweep(&addresses, port, timeout);
        assert!(answering.contains(&Ipv4Addr::LOCALHOST));
        drop(listener);
        assert!(sweep(&addresses, port, timeout).is_empty());
        assert!(sweep(&[], port, timeout).is_empty());
    }

    #[test]
    fn own_networks_exclude_loopback() {
        assert!(networks().iter().all(|(address, prefix)| !address.is_loopback() && *prefix <= 32));
    }
}
