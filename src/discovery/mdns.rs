//! devices announcing their ssh service on the local network (mDNS service discovery)

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};

use mdns_sd::{ServiceDaemon, ServiceEvent};

use super::Found;
use crate::devices::DeviceKind;

/// service types of devices giving access to their files, ssh first: a device announcing
/// several is reached through the first
const SERVICES: [(&str, DeviceKind); 3] = [
    ("_sftp-ssh._tcp.local.", DeviceKind::Sftp),
    ("_ssh._tcp.local.", DeviceKind::Sftp),
    ("_ftp._tcp.local.", DeviceKind::Ftp),
];

/// announced devices by the full name of their service
static ANNOUNCED: Mutex<Option<HashMap<String, Found>>> = Mutex::new(None);
static DAEMON: OnceLock<Option<Mutex<ServiceDaemon>>> = OnceLock::new();

fn announced<T>(change: impl FnOnce(&mut HashMap<String, Found>) -> T) -> T {
    let mut guard = ANNOUNCED.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    change(guard.get_or_insert_with(HashMap::new))
}

/// the address to reach a device at: IPv4 when it has one
fn preferred(addresses: impl Iterator<Item = IpAddr>) -> Option<IpAddr> {
    let mut addresses: Vec<IpAddr> = addresses.collect();
    addresses.sort_by_key(|address| !address.is_ipv4());
    addresses.into_iter().next()
}

/// start listening to announcements, once. they are then collected in the background
pub fn start() {
    DAEMON.get_or_init(|| {
        let daemon = ServiceDaemon::new().ok()?;
        for (service, kind) in SERVICES {
            let Ok(events) = daemon.browse(service) else { continue };
            std::thread::spawn(move || {
                while let Ok(event) = events.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(service) => {
                            let addresses = service.addresses.iter().map(|address| address.to_ip_addr());
                            if let Some(address) = preferred(addresses) {
                                let name = service.host.trim_end_matches('.').to_owned();
                                let found = Found { address, port: service.port, name, kind };
                                announced(|devices| devices.insert(service.fullname.clone(), found));
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            announced(|devices| devices.remove(&fullname));
                        }
                        _ => {}
                    }
                }
            });
        }
        Some(Mutex::new(daemon))
    });
}

/// the devices announced so far
pub fn found() -> Vec<Found> {
    let mut found: Vec<Found> = announced(|devices| devices.values().cloned().collect());
    // ssh before FTP, then by name
    found.sort_by(|a, b| {
        (a.kind == DeviceKind::Ftp).cmp(&(b.kind == DeviceKind::Ftp)).then(a.name.cmp(&b.name)).then(a.address.cmp(&b.address))
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_is_preferred() {
        let v4: IpAddr = "192.168.1.5".parse().unwrap();
        let v6: IpAddr = "fe80::1".parse().unwrap();
        assert_eq!(preferred([v6, v4].into_iter()), Some(v4));
        assert_eq!(preferred([v6].into_iter()), Some(v6));
        assert_eq!(preferred(std::iter::empty()), None);
    }

    #[test]
    fn announced_devices_are_listed_by_name() {
        let device = |name: &str| Found {
            address: "192.168.1.5".parse().unwrap(),
            port: 22,
            name: name.to_owned(),
            kind: DeviceKind::Sftp,
        };
        announced(|devices| {
            devices.insert("b._ssh._tcp.local.".into(), device("b.local"));
            devices.insert("a._ssh._tcp.local.".into(), device("a.local"));
        });
        let names: Vec<String> = found().into_iter().map(|found| found.name).collect();
        assert_eq!(names, ["a.local", "b.local"]);
        announced(|devices| devices.clear());
        assert!(found().is_empty());
    }
}
