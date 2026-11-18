//! devices that can be explored, and their connection state

use std::path::PathBuf;

use crate::fs::{local::Local, Fs};

/// identifies a device for the life of the app
pub type DeviceId = u32;

/// the local device always has this id
pub const LOCAL: DeviceId = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Local,
}

/// connection state, which is also the category the device is listed under
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // online and offline come with remote devices
pub enum State {
    Connected,
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

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    #[allow(dead_code)] // distinguishes remote devices later
    pub kind: DeviceKind,
    pub state: State,
    pub fs: Fs,
    /// top folder shown under the device
    pub root: PathBuf,
    /// folder the exploration starts from, inside `root`
    pub home: PathBuf,
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
            root: PathBuf::from("/"),
            home: fs.home(),
            fs,
        }
    }
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

    #[test]
    fn local_device_is_connected() {
        let device = Device::local();
        assert_eq!(device.id, LOCAL);
        assert_eq!(device.kind, DeviceKind::Local);
        assert_eq!(device.state, State::Connected);
        assert_eq!(device.state.category(), "connected");
        assert!(matches!(device.fs, Fs::Local(_)));
        assert!(!device.name.is_empty());
        assert_eq!(device.root, PathBuf::from("/"));
        assert!(device.home.starts_with(&device.root));
    }

    #[test]
    fn local_device_is_named_after_the_host() {
        if let Some(name) = hostname() {
            assert_eq!(Device::local().name, name);
            assert!(!name.contains('\0'));
        }
    }

    #[test]
    fn categories() {
        assert_eq!(State::Connected.category(), "connected");
        assert_eq!(State::Online.category(), "online");
        assert_eq!(State::Offline.category(), "offline");
    }
}
