//! files of a remote device reached through ssh (sftp subsystem)
//!
//! libssh2 calls block: each operation runs out of the async worker threads, one at a time on
//! the session. transfers go by pieces, so browsing gets its turn between two of them.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ssh2::{ErrorCode, FileStat, HashType, OpenFlags, OpenType, Session};

use super::{Entry, FsError, Kind, Result};

/// how long a connection attempt or a silent server is waited for
const TIMEOUT: Duration = Duration::from_secs(15);

/// how long the goodbye to a server is waited for when a session is closed
const CLOSING: Duration = Duration::from_secs(2);

/// sftp status codes
const NO_SUCH_FILE: i32 = 2;
const PERMISSION_DENIED: i32 = 3;
const FAILURE: i32 = 4;
const OP_UNSUPPORTED: i32 = 8;

/// file type bits of the permissions
const TYPE_MASK: u32 = 0o170000;
const TYPE_LINK: u32 = 0o120000;
const TYPE_FOLDER: u32 = 0o040000;

/// an open session. clones share it
#[derive(Clone)]
pub struct Sftp {
    inner: Arc<Inner>,
}

struct Inner {
    sftp: ssh2::Sftp,
    /// kept for as long as the sftp channel is used
    _session: Session,
    home: PathBuf,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // closing tells the server, and waits for its answer: not for long, it may be gone.
        // this still blocks, sessions are to be dropped out of the Qt thread
        self._session.set_timeout(CLOSING.as_millis() as u32);
    }
}

impl std::fmt::Debug for Sftp {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Sftp({})", self.inner.home.display())
    }
}

/// why a connection could not be opened
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// the server cannot be reached
    Unreachable(String),
    /// the key of the server is not the trusted one, or none is trusted yet.
    /// holds the fingerprint it presents
    HostKey(String),
    /// user or password refused
    Login,
    Other(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(message) => write!(f, "cannot reach the device: {message}"),
            ConnectError::HostKey(_) => f.write_str("the key of the device is not trusted"),
            ConnectError::Login => f.write_str("user or password refused"),
            ConnectError::Other(message) => f.write_str(message),
        }
    }
}

/// fingerprint of a host key as shown by ssh tools: `SHA256:` then unpadded base64
pub fn fingerprint(hash: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::from("SHA256:");
    for chunk in hash.chunks(3) {
        let bits = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for index in 0..=chunk.len() {
            text.push(ALPHABET[(bits >> (18 - 6 * index) & 63) as usize] as char);
        }
    }
    text
}

fn other(error: ssh2::Error) -> ConnectError {
    ConnectError::Other(error.message().to_owned())
}

/// open a session. `trusted` is the fingerprint the server must present.
/// blocks: to be called out of the async worker threads
pub fn connect(host: &str, port: u16, user: &str, password: &str, trusted: Option<&str>) -> std::result::Result<Sftp, ConnectError> {
    let unreachable = |error: std::io::Error| ConnectError::Unreachable(error.to_string());
    let address = (host, port)
        .to_socket_addrs()
        .map_err(unreachable)?
        .next()
        .ok_or_else(|| ConnectError::Unreachable(format!("{host} has no address")))?;
    let stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(unreachable)?;
    let mut session = Session::new().map_err(other)?;
    session.set_timeout(TIMEOUT.as_millis() as u32);
    session.set_tcp_stream(stream);
    session.handshake().map_err(other)?;

    let presented = session
        .host_key_hash(HashType::Sha256)
        .map(fingerprint)
        .ok_or_else(|| ConnectError::Other("the device presents no key".into()))?;
    if trusted != Some(presented.as_str()) {
        return Err(ConnectError::HostKey(presented));
    }
    login(&session, user, password)?;
    let sftp = session
        .sftp()
        .map_err(|failure| ConnectError::Other(format!("the device does not give access to its files: {}", failure.message())))?;
    let home = sftp.realpath(Path::new(".")).unwrap_or_else(|_| PathBuf::from("/"));
    Ok(Sftp { inner: Arc::new(Inner { sftp, _session: session, home }) })
}

/// answers every question of the server with the password
struct Answers<'a>(&'a str);

impl ssh2::KeyboardInteractivePrompt for Answers<'_> {
    fn prompt<'a>(&mut self, _user: &str, _instructions: &str, prompts: &[ssh2::Prompt<'a>]) -> Vec<String> {
        prompts.iter().map(|_| self.0.to_owned()).collect()
    }
}

/// whether a password can be given, from the comma separated methods a server offers
pub fn accepts_password(methods: &str) -> bool {
    methods.split(',').any(|method| matches!(method.trim(), "password" | "keyboard-interactive"))
}

fn login(session: &Session, user: &str, password: &str) -> std::result::Result<(), ConnectError> {
    // asking the methods also logs in when the server wants nothing
    let methods = session.auth_methods(user).unwrap_or("").to_owned();
    if session.authenticated() {
        return Ok(());
    }
    if !accepts_password(&methods) {
        return Err(ConnectError::Other(format!("the device does not accept passwords, only: {methods}")));
    }
    if methods.split(',').any(|method| method == "password") {
        let _ = session.userauth_password(user, password);
    }
    if !session.authenticated() && methods.split(',').any(|method| method == "keyboard-interactive") {
        let _ = session.userauth_keyboard_interactive(user, &mut Answers(password));
    }
    match session.authenticated() {
        true => Ok(()),
        false => Err(ConnectError::Login),
    }
}

fn error(error: ssh2::Error, path: &Path) -> FsError {
    match error.code() {
        ErrorCode::SFTP(NO_SUCH_FILE) => FsError::NotFound(path.to_owned()),
        ErrorCode::SFTP(PERMISSION_DENIED) => FsError::PermissionDenied(path.to_owned()),
        _ => FsError::Other(format!("{}: {}", path.display(), error.message())),
    }
}

/// details of an item from what the server tells about it. `own` describes the item itself,
/// `target` what it points to when it is a link that can be followed
pub fn entry(name: &str, own: &FileStat, link_target: Option<PathBuf>, target: Option<&FileStat>) -> Entry {
    let shown = target.unwrap_or(own);
    let permissions = shown.perm.unwrap_or(0);
    let is_link = own.perm.unwrap_or(0) & TYPE_MASK == TYPE_LINK;
    let kind = match permissions & TYPE_MASK {
        TYPE_FOLDER => Kind::Folder,
        _ => Kind::File,
    };
    Entry {
        name: name.to_owned(),
        // a broken link is shown as a file
        kind: if is_link && target.is_none() { Kind::File } else { kind },
        link_target,
        size: shown.size.unwrap_or(0),
        mtime: shown.mtime.unwrap_or(0) as i64,
        // the protocol only gives numbers
        owner: shown.uid.map(|uid| uid.to_string()).unwrap_or_default(),
        group: shown.gid.map(|gid| gid.to_string()).unwrap_or_default(),
        mode: permissions & 0o7777,
    }
}

impl Inner {
    fn describe(&self, path: &Path, own: &FileStat) -> Entry {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        if own.perm.unwrap_or(0) & TYPE_MASK == TYPE_LINK {
            let link_target = self.sftp.readlink(path).ok();
            let target = self.sftp.stat(path).ok();
            entry(&name, own, link_target.or_else(|| Some(PathBuf::new())), target.as_ref())
        } else {
            entry(&name, own, None, None)
        }
    }

    fn exists(&self, path: &Path) -> bool {
        self.sftp.lstat(path).is_ok()
    }

    fn remove(&self, path: &Path) -> Result<()> {
        let own = self.sftp.lstat(path).map_err(|failure| error(failure, path))?;
        if own.perm.unwrap_or(0) & TYPE_MASK == TYPE_FOLDER {
            for (child, _) in self.sftp.readdir(path).map_err(|failure| error(failure, path))? {
                self.remove(&child)?;
            }
            self.sftp.rmdir(path)
        } else {
            self.sftp.unlink(path)
        }
        .map_err(|failure| error(failure, path))
    }
}

impl Sftp {
    pub fn home(&self) -> PathBuf {
        self.inner.home.clone()
    }

    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> Result<T> + Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || operation(&inner))
            .await
            .unwrap_or_else(|failure| Err(FsError::Other(format!("sftp task failed: {failure}"))))
    }

    /// whether the server still answers. an answer that is a refusal counts
    pub async fn alive(&self) -> bool {
        self.run(|inner| match inner.sftp.realpath(Path::new(".")) {
            Err(failure) if matches!(failure.code(), ErrorCode::Session(_)) => Err(FsError::Other(failure.message().to_owned())),
            _ => Ok(()),
        })
        .await
        .is_ok()
    }

    pub async fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        let path = path.to_owned();
        self.run(move |inner| {
            let items = inner.sftp.readdir(&path).map_err(|failure| error(failure, &path))?;
            Ok(items.iter().map(|(child, own)| inner.describe(child, own)).collect())
        })
        .await
    }

    pub async fn stat(&self, path: &Path) -> Result<Entry> {
        let path = path.to_owned();
        self.run(move |inner| {
            let own = inner.sftp.lstat(&path).map_err(|failure| error(failure, &path))?;
            Ok(inner.describe(&path, &own))
        })
        .await
    }

    pub async fn rename(&self, path: &Path, name: &str) -> Result<PathBuf> {
        let source = path.to_owned();
        let destination = path.with_file_name(name);
        self.run(move |inner| {
            if destination != source && inner.exists(&destination) {
                return Err(FsError::AlreadyExists(destination));
            }
            inner.sftp.rename(&source, &destination, None).map_err(|failure| error(failure, &source))?;
            Ok(destination)
        })
        .await
    }

    pub async fn rename_to(&self, from: &Path, to: &Path) -> Result<()> {
        let (from, to) = (from.to_owned(), to.to_owned());
        self.run(move |inner| {
            inner.sftp.rename(&from, &to, None).map_err(|failure| match failure.code() {
                // servers do not tell why: it may be another filesystem, which a copy solves
                ErrorCode::SFTP(FAILURE) | ErrorCode::SFTP(OP_UNSUPPORTED) => FsError::CrossDevice,
                _ => error(failure, &from),
            })
        })
        .await
    }

    pub async fn delete(&self, path: &Path) -> Result<()> {
        let path = path.to_owned();
        self.run(move |inner| inner.remove(&path)).await
    }

    pub async fn mkdir(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        self.run(move |inner| {
            if inner.exists(&path) {
                return Err(FsError::AlreadyExists(path));
            }
            inner.sftp.mkdir(&path, 0o755).map_err(|failure| error(failure, &path))?;
            Ok(path)
        })
        .await
    }

    pub async fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        self.create(&path, 0o644).await?;
        Ok(path)
    }

    pub async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        let path = path.to_owned();
        self.run(move |inner| {
            let change = FileStat { size: None, uid: None, gid: None, perm: Some(mode), atime: None, mtime: None };
            inner.sftp.setstat(&path, change).map_err(|failure| error(failure, &path))
        })
        .await
    }

    pub async fn symlink(&self, target: &Path, path: &Path) -> Result<()> {
        let (target, path) = (target.to_owned(), path.to_owned());
        self.run(move |inner| inner.sftp.symlink(&path, &target).map_err(|failure| error(failure, &path))).await
    }

    pub async fn open(&self, path: &Path) -> Result<SftpFile> {
        let path = path.to_owned();
        self.run(move |inner| {
            let file = inner.sftp.open(&path).map_err(|failure| error(failure, &path))?;
            Ok(SftpFile { file: Arc::new(Mutex::new(file)), path })
        })
        .await
    }

    pub async fn create(&self, path: &Path, mode: u32) -> Result<SftpFile> {
        let path = path.to_owned();
        self.run(move |inner| {
            if inner.exists(&path) {
                return Err(FsError::AlreadyExists(path));
            }
            let flags = OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUSIVE;
            let file = inner
                .sftp
                .open_mode(&path, flags, mode as i32, OpenType::File)
                .map_err(|failure| error(failure, &path))?;
            Ok(SftpFile { file: Arc::new(Mutex::new(file)), path })
        })
        .await
    }
}

/// a remote file open for reading or writing by pieces
pub struct SftpFile {
    file: Arc<Mutex<ssh2::File>>,
    path: PathBuf,
}

impl std::fmt::Debug for SftpFile {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "SftpFile({})", self.path.display())
    }
}

impl SftpFile {
    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut ssh2::File) -> std::io::Result<T> + Send + 'static,
    {
        let (file, path) = (self.file.clone(), self.path.clone());
        tokio::task::spawn_blocking(move || {
            let mut file = file.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&mut file).map_err(|failure| FsError::from_io(failure, &path))
        })
        .await
        .unwrap_or_else(|failure| Err(FsError::Other(format!("sftp task failed: {failure}"))))
    }

    /// the next bytes, at most `limit`, none at the end of the file
    pub async fn read(&mut self, limit: usize) -> Result<Vec<u8>> {
        self.run(move |file| {
            let mut data = vec![0; limit];
            let mut length = 0;
            // the server answers by small packets: fill the piece
            while length < limit {
                match file.read(&mut data[length..])? {
                    0 => break,
                    count => length += count,
                }
            }
            data.truncate(length);
            Ok(data)
        })
        .await
    }

    pub async fn write(&mut self, data: Vec<u8>) -> Result<()> {
        self.run(move |file| file.write_all(&data)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(perm: u32, size: u64) -> FileStat {
        FileStat { size: Some(size), uid: Some(1000), gid: Some(100), perm: Some(perm), atime: None, mtime: Some(1_700_000_000) }
    }

    #[test]
    fn fingerprints_look_like_those_of_ssh() {
        // sha256 of the empty string
        let hash = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9, 0x24, 0x27,
            0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(fingerprint(&hash), "SHA256:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU");
        assert_eq!(fingerprint(b"abc"), "SHA256:YWJj");
        assert_eq!(fingerprint(b"ab"), "SHA256:YWI");
        assert_eq!(fingerprint(b"a"), "SHA256:YQ");
        assert_eq!(fingerprint(b""), "SHA256:");
    }

    #[test]
    fn methods_taking_a_password() {
        assert!(accepts_password("publickey,password"));
        assert!(accepts_password("keyboard-interactive"));
        assert!(accepts_password("publickey, keyboard-interactive"));
        assert!(!accepts_password("publickey"));
        assert!(!accepts_password("publickey,gssapi-with-mic"));
        assert!(!accepts_password(""));
    }

    #[test]
    fn files_and_folders() {
        let file = entry("notes.txt", &stat(0o100640, 12), None, None);
        assert_eq!(file.kind, Kind::File);
        assert_eq!(file.size, 12);
        assert_eq!(file.mode, 0o640);
        assert_eq!(file.mtime, 1_700_000_000);
        assert_eq!((file.owner.as_str(), file.group.as_str()), ("1000", "100"));
        assert_eq!(file.link_target, None);

        let folder = entry("docs", &stat(0o040755, 4096), None, None);
        assert_eq!(folder.kind, Kind::Folder);
        assert_eq!(folder.mode, 0o755);
    }

    #[test]
    fn links_show_what_they_point_to() {
        let link = stat(0o120777, 5);
        let to_folder = entry("shortcut", &link, Some("docs".into()), Some(&stat(0o040700, 4096)));
        assert_eq!(to_folder.kind, Kind::Folder);
        assert_eq!(to_folder.mode, 0o700);
        assert_eq!(to_folder.link_target, Some(PathBuf::from("docs")));

        let to_file = entry("shortcut", &link, Some("a.txt".into()), Some(&stat(0o100644, 99)));
        assert_eq!((to_file.kind, to_file.size), (Kind::File, 99));

        let broken = entry("shortcut", &link, Some("gone".into()), None);
        assert_eq!(broken.kind, Kind::File);
        assert_eq!(broken.link_target, Some(PathBuf::from("gone")));
    }

    #[test]
    fn missing_details_do_not_fail() {
        let empty = FileStat { size: None, uid: None, gid: None, perm: None, atime: None, mtime: None };
        let unknown = entry("x", &empty, None, None);
        assert_eq!((unknown.kind, unknown.size, unknown.mode), (Kind::File, 0, 0));
        assert_eq!(unknown.owner, "");
    }

    #[test]
    fn unreachable_devices_are_reported() {
        // nothing listens on this port of the local machine
        match connect("127.0.0.1", 1, "user", "password", None) {
            Err(ConnectError::Unreachable(_)) => {}
            other => panic!("expected an unreachable device, got {other:?}"),
        }
        assert!(matches!(
            connect("no such host.invalid", 22, "user", "password", None),
            Err(ConnectError::Unreachable(_))
        ));
        assert_eq!(ConnectError::Login.to_string(), "user or password refused");
    }
}
