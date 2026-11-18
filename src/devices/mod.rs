//! devices that can be explored, their connection state, and how they are remembered

use std::path::PathBuf;

use crate::config::Config;
use crate::fs::{local::Local, Fs};

/// identifies a device for the life of the app, and in the configuration
pub type DeviceId = u32;

/// the local device always has this id
pub const LOCAL: DeviceId = 0;

/// most remote devices remembered: beyond, the least recently used are forgotten
pub const REMEMBERED: usize = 20;

/// port of the ssh service when none is given
pub const SSH_PORT: u16 = 22;

/// port of the FTP service
pub const FTP_PORT: u16 = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Local,
    /// reached through ssh
    Sftp,
    /// reached through FTP, without encryption
    Ftp,
}

impl DeviceKind {
    /// name in the configuration and for the interface
    pub fn protocol(self) -> &'static str {
        match self {
            DeviceKind::Local => "local",
            DeviceKind::Sftp => "sftp",
            DeviceKind::Ftp => "ftp",
        }
    }

    /// the kind of remote device speaking this protocol
    pub fn from_protocol(protocol: &str) -> Option<Self> {
        match protocol {
            "sftp" => Some(DeviceKind::Sftp),
            "ftp" => Some(DeviceKind::Ftp),
            _ => None,
        }
    }
}

/// connection state, which is also the category the device is listed under
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Connected,
    /// answers on the network, not connected
    Online,
    Offline,
}

impl State {
    /// name of the category in the user interface
    pub fn category(self) -> &'static str {
        match self {
            State::Connected => "connected",
            State::Online => "online",
            State::Offline => "offline",
        }
    }
}

/// how a remote device is reached
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// known for this run, or from the configuration when the user asked to save it
    pub password: Option<String>,
    /// keep the password in the configuration
    pub save_password: bool,
    /// fingerprint of the key the device must present, once the user trusted it
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub state: State,
    /// its files, while connected
    pub fs: Option<Fs>,
    /// current folder to start from. empty for a remote device never connected: its home
    /// folder is asked when connecting
    pub home: PathBuf,
    pub remote: Option<Remote>,
    /// when it was last connected, in seconds since the unix epoch
    pub used: u64,
    /// kept in the configuration. a device seen on the network is only remembered once
    /// connected or edited
    pub remembered: bool,
}

fn section(id: DeviceId) -> String {
    format!("device {id}")
}

impl Device {
    /// the device the app runs on: needs no connection, so it is connected from the start
    pub fn local() -> Self {
        let fs = Fs::Local(Local);
        Device {
            id: LOCAL,
            name: hostname().unwrap_or_else(|| "This device".to_owned()),
            kind: DeviceKind::Local,
            state: State::Connected,
            home: fs.home(),
            fs: Some(fs),
            remote: None,
            used: 0,
            remembered: false,
        }
    }

    /// a remote device reached through ssh, not connected. named after its host when `name` is empty
    pub fn remote(id: DeviceId, name: &str, remote: Remote) -> Self {
        Device {
            id,
            name: if name.trim().is_empty() { remote.host.clone() } else { name.trim().to_owned() },
            kind: DeviceKind::Sftp,
            state: State::Offline,
            fs: None,
            home: PathBuf::new(),
            remote: Some(remote),
            used: 0,
            remembered: true,
        }
    }

    /// a device seen on the network, answering at `address`: online, and not remembered yet
    pub fn discovered(id: DeviceId, name: &str, address: &str, port: u16, kind: DeviceKind) -> Self {
        let remote = Remote {
            host: address.to_owned(),
            port,
            user: String::new(),
            password: None,
            save_password: false,
            fingerprint: None,
        };
        Device { kind, state: State::Online, remembered: false, ..Device::remote(id, name, remote) }
    }

    /// write a remote device to the configuration
    pub fn save(&self, config: &mut Config) {
        let Some(remote) = self.remote.as_ref().filter(|_| self.remembered) else { return };
        let section = section(self.id);
        config.set(&section, "name", &self.name);
        config.set(&section, "kind", self.kind.protocol());
        config.set(&section, "host", &remote.host);
        config.set(&section, "port", &remote.port.to_string());
        config.set(&section, "user", &remote.user);
        config.set(&section, "folder", &self.home.to_string_lossy());
        config.set(&section, "used", &self.used.to_string());
        match (&remote.password, remote.save_password) {
            (Some(password), true) => config.set(&section, "password", password),
            _ => config.remove(&section, "password"),
        }
        match &remote.fingerprint {
            Some(fingerprint) => config.set(&section, "fingerprint", fingerprint),
            None => config.remove(&section, "fingerprint"),
        }
    }

    pub fn forget(&self, config: &mut Config) {
        config.remove_section(&section(self.id));
    }

    /// the remote devices of the configuration, most recently used first
    pub fn load(config: &Config) -> Vec<Device> {
        let mut devices: Vec<Device> = config
            .sections("device ")
            .iter()
            .filter_map(|id| {
                let id: DeviceId = id.parse().ok().filter(|id| *id != LOCAL)?;
                let section = section(id);
                let get = |key: &str| config.get(&section, key).map(str::to_owned);
                let host = get("host").filter(|host| !host.is_empty())?;
                let password = get("password");
                let remote = Remote {
                    host,
                    port: get("port").and_then(|port| port.parse().ok()).unwrap_or(SSH_PORT),
                    user: get("user").unwrap_or_default(),
                    save_password: password.is_some(),
                    password,
                    fingerprint: get("fingerprint"),
                };
                let mut device = Device::remote(id, &get("name").unwrap_or_default(), remote);
                // devices saved before FTP existed have no kind
                device.kind = get("kind").and_then(|kind| DeviceKind::from_protocol(&kind)).unwrap_or(DeviceKind::Sftp);
                device.home = get("folder").map(PathBuf::from).filter(|folder| folder.is_absolute()).unwrap_or_default();
                device.used = get("used").and_then(|used| used.parse().ok()).unwrap_or(0);
                Some(device)
            })
            .collect();
        devices.sort_by(|a, b| b.used.cmp(&a.used).then(a.id.cmp(&b.id)));
        devices
    }
}

/// seconds since the unix epoch
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// whether something listens on this port of a host. blocks for at most a second and a half
pub fn reachable(host: &str, port: u16) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Some(address) = (host, port).to_socket_addrs().ok().and_then(|mut addresses| addresses.next()) else {
        return false;
    };
    TcpStream::connect_timeout(&address, std::time::Duration::from_millis(1500)).is_ok()
}

fn hostname() -> Option<String> {
    let mut buffer = [0u8; 256];
    let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if status != 0 {
        return None;
    }
    let length = buffer.iter().position(|&byte| byte == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..length]).trim().to_owned();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nas() -> Remote {
        Remote {
            host: "nas.local".into(),
            port: 2222,
            user: "me".into(),
            password: Some("secret".into()),
            save_password: true,
            fingerprint: Some("SHA256:abc".into()),
        }
    }

    #[test]
    fn local_device_is_connected() {
        let device = Device::local();
        assert_eq!(device.id, LOCAL);
        assert_eq!(device.kind, DeviceKind::Local);
        assert_eq!(device.state, State::Connected);
        assert!(matches!(device.fs, Some(Fs::Local(_))));
        assert!(!device.name.is_empty());
        assert!(device.home.is_absolute());
        assert_eq!(device.remote, None);
    }

    #[test]
    fn local_device_is_named_after_the_host() {
        if let Some(name) = hostname() {
            assert_eq!(Device::local().name, name);
            assert!(!name.contains('\0'));
        }
    }

    #[test]
    fn categories_and_their_order() {
        assert_eq!(State::Connected.category(), "connected");
        assert_eq!(State::Online.category(), "online");
        assert_eq!(State::Offline.category(), "offline");
        assert!(State::Connected < State::Online && State::Online < State::Offline);
    }

    #[test]
    fn remote_device_starts_offline() {
        let device = Device::remote(3, "  ", nas());
        assert_eq!(device.name, "nas.local", "named after the host by default");
        assert_eq!(device.kind, DeviceKind::Sftp);
        assert_eq!(device.state, State::Offline);
        assert!(device.fs.is_none());
        assert_eq!(Device::remote(3, " NAS ", nas()).name, "NAS");
    }

    #[test]
    fn discovered_device_is_online_and_not_remembered() {
        let device = Device::discovered(4, "pi.local", "192.168.1.9", 22, DeviceKind::Sftp);
        assert_eq!((device.name.as_str(), device.state), ("pi.local", State::Online));
        let remote = device.remote.as_ref().unwrap();
        assert_eq!((remote.host.as_str(), remote.port, remote.user.as_str()), ("192.168.1.9", 22, ""));
        assert!(!device.remembered);
        let mut config = Config::default();
        device.save(&mut config);
        assert!(Device::load(&config).is_empty());
    }

    #[test]
    fn remote_devices_are_remembered() {
        let mut config = Config::default();
        let mut first = Device::remote(1, "NAS", nas());
        first.home = "/srv/files".into();
        first.used = 100;
        first.save(&mut config);
        first.kind = DeviceKind::Ftp;
        first.save(&mut config);
        let mut second = Device::remote(2, "", Remote { password: Some("typed".into()), save_password: false, fingerprint: None, ..nas() });
        second.used = 200;
        second.save(&mut config);
        Device::local().save(&mut config);

        let devices = Device::load(&config);
        // most recently used first, the local device is not one of them
        assert_eq!(devices.iter().map(|device| device.id).collect::<Vec<_>>(), [2, 1]);
        let loaded = &devices[1];
        assert_eq!(loaded.name, "NAS");
        assert_eq!((loaded.kind, devices[0].kind), (DeviceKind::Ftp, DeviceKind::Sftp));
        assert_eq!(DeviceKind::from_protocol(DeviceKind::Ftp.protocol()), Some(DeviceKind::Ftp));
        assert_eq!(DeviceKind::from_protocol("local"), None);
        assert_eq!(loaded.remote, Some(nas()));
        assert_eq!(loaded.home, PathBuf::from("/srv/files"));
        assert_eq!(loaded.used, 100);
        assert_eq!(loaded.state, State::Offline);
        // a password is only kept when asked
        let unsaved = devices[0].remote.as_ref().unwrap();
        assert_eq!(unsaved.password, None);
        assert!(!unsaved.save_password);
        assert_eq!(unsaved.fingerprint, None);
        assert_eq!(devices[0].home, PathBuf::new());
    }

    #[test]
    fn saved_password_and_key_can_be_dropped() {
        let mut config = Config::default();
        let mut device = Device::remote(1, "NAS", nas());
        device.save(&mut config);
        let remote = device.remote.as_mut().unwrap();
        remote.save_password = false;
        remote.fingerprint = None;
        device.save(&mut config);
        let loaded = Device::load(&config).remove(0).remote.unwrap();
        assert_eq!((loaded.password, loaded.fingerprint), (None, None));

        device.forget(&mut config);
        assert!(Device::load(&config).is_empty());
    }

    #[test]
    fn damaged_devices_are_skipped() {
        let mut config = Config::default();
        config.set("device 0", "host", "pretends to be local");
        config.set("device x", "host", "no number");
        config.set("device 4", "name", "no host");
        config.set("device 5", "host", "only.host");
        let devices = Device::load(&config);
        assert_eq!(devices.len(), 1);
        let remote = devices[0].remote.as_ref().unwrap();
        assert_eq!((remote.port, remote.user.as_str()), (SSH_PORT, ""));
        assert_eq!(devices[0].name, "only.host");
    }

    #[test]
    fn reachability() {
        assert!(!reachable("127.0.0.1", 1));
        assert!(!reachable("no such host.invalid", 22));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(reachable("127.0.0.1", listener.local_addr().unwrap().port()));
        assert!(now() > 1_600_000_000);
    }
}
