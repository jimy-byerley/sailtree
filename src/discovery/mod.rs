//! finds the devices of the local network that can be explored
//!
//! two sources are merged per address: devices announcing their ssh service (mDNS), which are
//! known at once with their name, and those answering on the ssh port during a sweep of the
//! local networks, named by the system's reverse lookup.

pub mod mdns;
pub mod resolve;
pub mod scan;

use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use crate::devices::{DeviceKind, FTP_PORT, SSH_PORT};

/// how long a host is given to accept a connection during a sweep
const SWEEP_TIMEOUT: Duration = Duration::from_millis(300);

/// a device seen on the network
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub address: IpAddr,
    pub port: u16,
    /// its name on the network, its address when it has none
    pub name: String,
    /// how its files are reached
    pub kind: DeviceKind,
}

/// announced devices first, then swept ones at other addresses. the addresses of this device
/// are left out. a device is listed once, by the first way it was found
pub fn merge(announced: Vec<Found>, swept: Vec<Found>, own: &[IpAddr]) -> Vec<Found> {
    let mut seen: HashSet<IpAddr> = own.iter().copied().collect();
    announced.into_iter().chain(swept).filter(|found| seen.insert(found.address)).collect()
}

/// leave out the devices already known, by name or by one of the addresses of their host
pub fn unknown(found: Vec<Found>, known_hosts: &[String], known_addresses: &HashSet<IpAddr>) -> Vec<Found> {
    found
        .into_iter()
        .filter(|found| {
            !known_addresses.contains(&found.address)
                && !known_hosts.iter().any(|host| host.eq_ignore_ascii_case(&found.name))
        })
        .collect()
}

/// one look at the network: the devices that are not among `known_hosts`.
/// blocks for a few seconds: to be called out of the async worker threads
pub fn discover(known_hosts: &[String]) -> Vec<Found> {
    mdns::start();
    let networks = scan::networks();
    let own: Vec<IpAddr> = networks.iter().map(|(address, _)| IpAddr::V4(*address)).collect();
    let mut swept = Vec::new();
    // ssh first: a device with both is reached through it
    for (port, kind) in [(SSH_PORT, DeviceKind::Sftp), (FTP_PORT, DeviceKind::Ftp)] {
        for (address, prefix) in &networks {
            for answering in scan::sweep(&scan::candidates(*address, *prefix), port, SWEEP_TIMEOUT) {
                let name = resolve::name(answering).unwrap_or_else(|| answering.to_string());
                swept.push(Found { address: IpAddr::V4(answering), port, name, kind });
            }
        }
    }
    let known_addresses: HashSet<IpAddr> = known_hosts
        .iter()
        .filter_map(|host| (host.as_str(), 0).to_socket_addrs().ok())
        .flatten()
        .map(|address| address.ip())
        .collect();
    unknown(merge(mdns::found(), swept, &own), known_hosts, &known_addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(address: &str, name: &str) -> Found {
        Found { address: address.parse().unwrap(), port: 22, name: name.to_owned(), kind: DeviceKind::Sftp }
    }

    #[test]
    fn sources_are_merged_per_address() {
        let announced = vec![found("192.168.1.5", "nas.local"), found("192.168.1.9", "pi.local")];
        let swept = vec![found("192.168.1.5", "192.168.1.5"), found("192.168.1.7", "printer"), found("192.168.1.2", "me")];
        let own = ["192.168.1.2".parse().unwrap()];
        let merged = merge(announced, swept, &own);
        let names: Vec<&str> = merged.iter().map(|found| found.name.as_str()).collect();
        // the announced name wins, this device is not listed
        assert_eq!(names, ["nas.local", "pi.local", "printer"]);
    }

    #[test]
    fn known_devices_are_left_out() {
        let all = vec![found("192.168.1.5", "nas.local"), found("192.168.1.7", "192.168.1.7"), found("192.168.1.9", "pi.local")];
        let hosts = ["NAS.local".to_owned(), "server.example".to_owned()];
        let addresses: HashSet<IpAddr> = ["192.168.1.7".parse().unwrap()].into();
        assert_eq!(unknown(all.clone(), &hosts, &addresses), [found("192.168.1.9", "pi.local")]);
        assert_eq!(unknown(all.clone(), &[], &HashSet::new()), all);
    }
}
