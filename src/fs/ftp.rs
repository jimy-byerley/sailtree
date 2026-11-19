//! files of a remote device reached through FTP
//!
//! the protocol does one thing at a time per connection: browsing and changes share one
//! control connection, and each file being transferred has a connection of its own, so
//! browsing goes on meanwhile. nothing is encrypted, the password included.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::sftp::ConnectError;
use super::{Entry, FsError, Kind, Result};

/// how long a connection attempt or a silent server is waited for
const TIMEOUT: Duration = Duration::from_secs(15);

/// an open session. clones share it
#[derive(Clone)]
pub struct Ftp {
    inner: Arc<Inner>,
}

struct Inner {
    control: Mutex<Control>,
    /// to open the connections of file transfers
    login: Login,
    home: PathBuf,
}

#[derive(Clone)]
struct Login {
    address: SocketAddr,
    user: String,
    password: String,
}

impl std::fmt::Debug for Ftp {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Ftp({})", self.inner.login.address)
    }
}

/// answer of the server to a command
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    /// text of the last line, and the lines before it for a multi-line reply
    pub lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> &str {
        self.lines.last().map_or("", String::as_str)
    }

    fn positive(&self) -> bool {
        (100..400).contains(&self.code)
    }
}

/// read one reply: a line `123 text`, or lines from `123-text` to `123 text`
pub fn read_reply(input: &mut impl BufRead) -> std::io::Result<Reply> {
    let mut lines = Vec::new();
    let mut first: Option<String> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the device closed the connection"));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        let code = line.get(..3).filter(|code| code.bytes().all(|byte| byte.is_ascii_digit()));
        let last = match (&first, code) {
            (None, Some(code)) if line.as_bytes().get(3) == Some(&b'-') => {
                first = Some(code.to_owned());
                false
            }
            (None, Some(_)) => true,
            (Some(first), Some(code)) => code == first && line.as_bytes().get(3) != Some(&b'-'),
            _ => false,
        };
        if last {
            let code = line[..3].parse().unwrap_or(0);
            lines.push(line.get(4..).unwrap_or("").to_owned());
            return Ok(Reply { code, lines });
        }
        // the first line of a multi-line reply has the code, the others may not
        let text = if code.is_some() { line.get(4..).unwrap_or("") } else { line.trim_start() };
        lines.push(text.to_owned());
    }
}

/// port of the data connection announced by `EPSV`: `Entering Extended Passive Mode (|||60001|)`
pub fn parse_epsv(text: &str) -> Option<u16> {
    let inside = text.split('(').nth(1)?.split(')').next()?;
    inside.trim_matches(|character: char| !character.is_ascii_digit()).parse().ok()
}

/// port of the data connection announced by `PASV`: `Entering Passive Mode (127,0,0,1,234,97)`.
/// the address is ignored: the data connection goes where the control one does
pub fn parse_pasv(text: &str) -> Option<u16> {
    let start = text.find(|character: char| character.is_ascii_digit())?;
    let numbers: Vec<u16> = text[start..]
        .split(|character: char| !character.is_ascii_digit())
        .filter(|number| !number.is_empty())
        .take(6)
        .filter_map(|number| number.parse().ok())
        .collect();
    match numbers[..] {
        [_, _, _, _, high, low] if high < 256 && low < 256 => Some(high * 256 + low),
        _ => None,
    }
}

/// folder announced by `PWD`: `"/home/me" is the current directory`
pub fn parse_pwd(text: &str) -> Option<PathBuf> {
    let start = text.find('"')? + 1;
    let end = text.rfind('"').filter(|end| *end > start)?;
    // a quote in the name is doubled
    Some(PathBuf::from(text[start..end].replace("\"\"", "\"")))
}

/// seconds since the unix epoch of a UTC date
fn epoch(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    // days from civil, by Howard Hinnant
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146097 + day_of_era - 719468) * 86400 + hour * 3600 + minute * 60 + second
}

/// `20261119003800` or `20261119003800.123` to seconds since the unix epoch
fn parse_timestamp(text: &str) -> Option<i64> {
    let digits = text.split('.').next()?;
    if digits.len() != 14 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = |from: usize, to: usize| digits[from..to].parse::<i64>().ok();
    Some(epoch(number(0, 4)?, number(4, 6)?, number(6, 8)?, number(8, 10)?, number(10, 12)?, number(12, 14)?))
}

/// one line of a machine readable listing (`MLSD`): `type=file;size=12;modify=…; name`.
/// `None` for the folder itself, its parent, and lines that cannot be understood
pub fn parse_mlsd(line: &str) -> Option<Entry> {
    let (facts, name) = line.split_once(' ')?;
    if name.is_empty() {
        return None;
    }
    let mut entry = Entry {
        name: name.to_owned(),
        kind: Kind::File,
        link_target: None,
        size: 0,
        mtime: 0,
        owner: String::new(),
        group: String::new(),
        mode: 0,
    };
    let mut known = false;
    for fact in facts.split(';') {
        let Some((key, value)) = fact.split_once('=') else { continue };
        match key.to_ascii_lowercase().as_str() {
            "type" => {
                known = true;
                match value.to_ascii_lowercase().as_str() {
                    "dir" => entry.kind = Kind::Folder,
                    "cdir" | "pdir" => return None,
                    "file" => {}
                    // links: `OS.unix=slink:target` or `OS.unix=symlink`
                    other if other.starts_with("os.unix=s") => {
                        let target = value.split_once(':').map_or("", |(_, target)| target);
                        entry.link_target = Some(PathBuf::from(target));
                    }
                    _ => {}
                }
            }
            "size" | "sizd" => entry.size = value.parse().unwrap_or(0),
            "modify" => entry.mtime = parse_timestamp(value).unwrap_or(0),
            // `0644`, or `0o644` as some servers write it
            "unix.mode" => entry.mode = u32::from_str_radix(value.trim_start_matches("0o"), 8).unwrap_or(0) & 0o7777,
            "unix.owner" | "unix.ownername" => entry.owner = value.to_owned(),
            "unix.group" | "unix.groupname" => entry.group = value.to_owned(),
            "unix.uid" if entry.owner.is_empty() => entry.owner = value.to_owned(),
            "unix.gid" if entry.group.is_empty() => entry.group = value.to_owned(),
            _ => {}
        }
    }
    if entry.kind == Kind::Folder {
        entry.size = 0;
    }
    known.then_some(entry)
}

/// permission bits of the `rwxr-xr-x` part of a listing line
fn parse_mode(text: &str) -> u32 {
    text.bytes().take(9).enumerate().fold(0, |mode, (index, byte)| match byte {
        b'-' => mode,
        _ => mode | 1 << (8 - index),
    })
}

/// one line of a `ls -l` like listing (`LIST`), for servers without `MLSD`:
/// `-rw-r--r--   1 owner group   1234 Nov 19 00:38 name`. the date is not read
pub fn parse_list(line: &str) -> Option<Entry> {
    let kind = line.bytes().next()?;
    if !matches!(kind, b'-' | b'd' | b'l') || line.len() < 10 {
        return None;
    }
    // eight fields before the name, which may hold spaces
    let mut rest = line;
    let mut fields = Vec::with_capacity(8);
    for _ in 0..8 {
        rest = rest.trim_start();
        let end = rest.find(' ')?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    let name = rest.strip_prefix(' ')?;
    let (name, target) = match (kind, name.split_once(" -> ")) {
        (b'l', Some((name, target))) => (name, Some(PathBuf::from(target))),
        _ => (name, None),
    };
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(Entry {
        name: name.to_owned(),
        kind: if kind == b'd' { Kind::Folder } else { Kind::File },
        link_target: target,
        size: if kind == b'd' { 0 } else { fields[4].parse().unwrap_or(0) },
        mtime: 0,
        owner: fields[2].to_owned(),
        group: fields[3].to_owned(),
        mode: parse_mode(&line[1..]),
    })
}

fn failure(reply: &Reply, path: &Path) -> FsError {
    let text = reply.text().to_lowercase();
    if text.contains("permission") || text.contains("denied") || text.contains("not allowed") {
        FsError::PermissionDenied(path.to_owned())
    } else if reply.code == 550 || reply.code == 450 {
        FsError::NotFound(path.to_owned())
    } else {
        FsError::Other(format!("{}: {}", path.display(), reply.text()))
    }
}

fn broken(error: std::io::Error) -> FsError {
    FsError::Other(format!("connection to the device lost: {error}"))
}

/// one control connection, logged in
struct Control {
    reader: BufReader<TcpStream>,
    address: SocketAddr,
    /// the server has machine readable listings
    mlsd: bool,
}

impl Control {
    fn open(login: &Login) -> std::result::Result<Control, ConnectError> {
        let unreachable = |error: std::io::Error| ConnectError::Unreachable(error.to_string());
        let other = |error: std::io::Error| ConnectError::Other(error.to_string());
        let stream = TcpStream::connect_timeout(&login.address, TIMEOUT).map_err(unreachable)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(other)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(other)?;
        let mut control = Control { reader: BufReader::new(stream), address: login.address, mlsd: false };
        let greeting = read_reply(&mut control.reader).map_err(other)?;
        if !greeting.positive() {
            return Err(ConnectError::Other(greeting.text().to_owned()));
        }
        let mut reply = control.command(&format!("USER {}", login.user)).map_err(other)?;
        if reply.code == 331 {
            reply = control.command(&format!("PASS {}", login.password)).map_err(other)?;
        }
        if reply.code != 230 && reply.code != 202 {
            return Err(ConnectError::Login);
        }
        // files as they are, names in utf-8, and every detail the server can give
        let _ = control.command("TYPE I").map_err(other)?;
        let _ = control.command("OPTS UTF8 ON").map_err(other)?;
        let features = control.command("FEAT").map_err(other)?;
        control.mlsd = features.lines.iter().any(|line| line.trim().to_uppercase().starts_with("MLS"));
        if control.mlsd {
            let facts = "OPTS MLST type;size;modify;UNIX.mode;UNIX.owner;UNIX.group;UNIX.uid;UNIX.gid;";
            let _ = control.command(facts).map_err(other)?;
        }
        Ok(control)
    }

    fn command(&mut self, command: &str) -> std::io::Result<Reply> {
        // a name cannot hold a line end: it would start another command
        let command = command.replace(['\r', '\n'], " ");
        self.reader.get_mut().write_all(format!("{command}\r\n").as_bytes())?;
        read_reply(&mut self.reader)
    }

    /// run a command that must succeed
    fn must(&mut self, command: &str, path: &Path) -> Result<Reply> {
        let reply = self.command(command).map_err(broken)?;
        match reply.positive() {
            true => Ok(reply),
            false => Err(failure(&reply, path)),
        }
    }

    /// open a data connection for the next transfer command
    fn data(&mut self) -> Result<TcpStream> {
        let reply = self.command("EPSV").map_err(broken)?;
        let port = match reply.code {
            229 => parse_epsv(reply.text()),
            _ => {
                let reply = self.command("PASV").map_err(broken)?;
                parse_pasv(reply.text()).filter(|_| reply.code == 227)
            }
        };
        let port = port.ok_or_else(|| FsError::Other("the device gave no way to transfer data".into()))?;
        let address = SocketAddr::new(self.address.ip(), port);
        let stream = TcpStream::connect_timeout(&address, TIMEOUT).map_err(broken)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(broken)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(broken)?;
        Ok(stream)
    }

    /// what a command sends on the data connection
    fn download(&mut self, command: &str, path: &Path) -> Result<Vec<u8>> {
        let mut data = self.data()?;
        self.must(command, path)?;
        let mut content = Vec::new();
        data.read_to_end(&mut content).map_err(broken)?;
        drop(data);
        // the transfer is confirmed after the data
        let done = read_reply(&mut self.reader).map_err(broken)?;
        match done.positive() {
            true => Ok(content),
            false => Err(failure(&done, path)),
        }
    }

    fn list(&mut self, path: &Path) -> Result<Vec<Entry>> {
        let text = path.to_string_lossy();
        let (command, parse): (String, fn(&str) -> Option<Entry>) = match self.mlsd {
            true => (format!("MLSD {text}"), parse_mlsd),
            false => (format!("LIST -a {text}"), parse_list),
        };
        let content = self.download(&command, path)?;
        Ok(String::from_utf8_lossy(&content).lines().filter_map(|line| parse(line.trim_end_matches('\r'))).collect())
    }

    fn stat(&mut self, path: &Path) -> Result<Entry> {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            // the top folder
            return Ok(Entry {
                name: String::new(),
                kind: Kind::Folder,
                link_target: None,
                size: 0,
                mtime: 0,
                owner: String::new(),
                group: String::new(),
                mode: 0o755,
            });
        };
        let name = name.to_string_lossy();
        let found = self.list(parent)?.into_iter().find(|entry| entry.name == name);
        found.ok_or_else(|| FsError::NotFound(path.to_owned()))
    }

    fn exists(&mut self, path: &Path) -> bool {
        self.stat(path).is_ok()
    }

    fn remove(&mut self, path: &Path, entry: &Entry) -> Result<()> {
        let text = path.to_string_lossy();
        if entry.kind == Kind::Folder && entry.link_target.is_none() {
            for child in self.list(path)? {
                self.remove(&path.join(&child.name), &child)?;
            }
            self.must(&format!("RMD {text}"), path)?;
        } else {
            self.must(&format!("DELE {text}"), path)?;
        }
        Ok(())
    }
}

/// open a session. blocks: to be called out of the async worker threads
pub fn connect(host: &str, port: u16, user: &str, password: &str) -> std::result::Result<Ftp, ConnectError> {
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| ConnectError::Unreachable(error.to_string()))?
        .next()
        .ok_or_else(|| ConnectError::Unreachable(format!("{host} has no address")))?;
    let login = Login { address, user: user.to_owned(), password: password.to_owned() };
    let mut control = Control::open(&login)?;
    let home = control.command("PWD").ok().and_then(|reply| parse_pwd(reply.text())).unwrap_or_else(|| PathBuf::from("/"));
    Ok(Ftp { inner: Arc::new(Inner { control: Mutex::new(control), login, home }) })
}

impl Ftp {
    pub fn home(&self) -> PathBuf {
        self.inner.home.clone()
    }

    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Control) -> Result<T> + Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut control = inner.control.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&mut control)
        })
        .await
        .unwrap_or_else(|failure| Err(FsError::Other(format!("ftp task failed: {failure}"))))
    }

    /// whether the server still answers
    pub async fn alive(&self) -> bool {
        self.run(|control| control.command("NOOP").map(|_| ()).map_err(broken)).await.is_ok()
    }

    pub async fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        let path = path.to_owned();
        self.run(move |control| control.list(&path)).await
    }

    pub async fn stat(&self, path: &Path) -> Result<Entry> {
        let path = path.to_owned();
        self.run(move |control| control.stat(&path)).await
    }

    pub async fn rename(&self, path: &Path, name: &str) -> Result<PathBuf> {
        let destination = path.with_file_name(name);
        self.rename_to(path, &destination).await?;
        Ok(destination)
    }

    pub async fn rename_to(&self, from: &Path, to: &Path) -> Result<()> {
        let (from, to) = (from.to_owned(), to.to_owned());
        self.run(move |control| {
            if to != from && control.exists(&to) {
                return Err(FsError::AlreadyExists(to));
            }
            control.must(&format!("RNFR {}", from.to_string_lossy()), &from)?;
            control.must(&format!("RNTO {}", to.to_string_lossy()), &to)?;
            Ok(())
        })
        .await
    }

    pub async fn delete(&self, path: &Path) -> Result<()> {
        let path = path.to_owned();
        self.run(move |control| {
            let entry = control.stat(&path)?;
            control.remove(&path, &entry)
        })
        .await
    }

    pub async fn mkdir(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        self.run(move |control| {
            if control.exists(&path) {
                return Err(FsError::AlreadyExists(path));
            }
            control.must(&format!("MKD {}", path.to_string_lossy()), &path)?;
            Ok(path)
        })
        .await
    }

    pub async fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        self.create(&path, 0o644).await?.finish().await?;
        Ok(path)
    }

    pub async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        let path = path.to_owned();
        self.run(move |control| {
            control.must(&format!("SITE CHMOD {mode:o} {}", path.to_string_lossy()), &path)?;
            Ok(())
        })
        .await
    }

    pub async fn open(&self, path: &Path) -> Result<FtpFile> {
        self.transfer(path, "RETR", None).await
    }

    pub async fn create(&self, path: &Path, mode: u32) -> Result<FtpFile> {
        let existing = path.to_owned();
        if self.run(move |control| Ok(control.exists(&existing))).await? {
            return Err(FsError::AlreadyExists(path.to_owned()));
        }
        self.transfer(path, "STOR", Some(mode)).await
    }

    /// start the transfer of one file on a connection of its own
    async fn transfer(&self, path: &Path, command: &'static str, mode: Option<u32>) -> Result<FtpFile> {
        let (login, path) = (self.inner.login.clone(), path.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut control = Control::open(&login).map_err(|error| FsError::Other(error.to_string()))?;
            let data = control.data()?;
            control.must(&format!("{command} {}", path.to_string_lossy()), &path)?;
            Ok(FtpFile { state: Arc::new(Mutex::new(Some((control, data)))), path, mode })
        })
        .await
        .unwrap_or_else(|failure| Err(FsError::Other(format!("ftp task failed: {failure}"))))
    }
}

/// a remote file being read or written, on its own connection
pub struct FtpFile {
    /// gone once the transfer is finished
    state: Arc<Mutex<Option<(Control, TcpStream)>>>,
    path: PathBuf,
    /// permissions to give to a written file
    mode: Option<u32>,
}

impl std::fmt::Debug for FtpFile {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "FtpFile({})", self.path.display())
    }
}

impl FtpFile {
    async fn run<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Option<(Control, TcpStream)>) -> Result<T> + Send + 'static,
    {
        let state = self.state.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            operation(&mut state)
        })
        .await
        .unwrap_or_else(|failure| Err(FsError::Other(format!("ftp task failed: {failure}"))))
    }

    /// the next bytes, at most `limit`, none at the end of the file
    pub async fn read(&mut self, limit: usize) -> Result<Vec<u8>> {
        self.run(move |state| {
            let Some((_, data)) = state else { return Ok(Vec::new()) };
            let mut content = vec![0; limit];
            let mut length = 0;
            while length < limit {
                match data.read(&mut content[length..]).map_err(broken)? {
                    0 => break,
                    count => length += count,
                }
            }
            content.truncate(length);
            Ok(content)
        })
        .await
    }

    pub async fn write(&mut self, content: Vec<u8>) -> Result<()> {
        self.run(move |state| match state {
            Some((_, data)) => data.write_all(&content).map_err(broken),
            None => Err(FsError::Other("file already closed".into())),
        })
        .await
    }

    /// end a written file: the server confirms it has everything
    pub async fn finish(self) -> Result<()> {
        let (path, mode) = (self.path.clone(), self.mode);
        self.run(move |state| {
            let Some((mut control, data)) = state.take() else { return Ok(()) };
            // closing the data connection is what ends the file
            drop(data);
            let done = read_reply(&mut control.reader).map_err(broken)?;
            if !done.positive() {
                return Err(failure(&done, &path));
            }
            if let Some(mode) = mode {
                // not every server can
                let _ = control.command(&format!("SITE CHMOD {mode:o} {}", path.to_string_lossy()));
            }
            let _ = control.command("QUIT");
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn reply(text: &str) -> Reply {
        read_reply(&mut Cursor::new(text.as_bytes())).unwrap()
    }

    #[test]
    fn single_and_multi_line_replies() {
        let greeting = reply("220 pyftpdlib ready.\r\n");
        assert_eq!((greeting.code, greeting.text()), (220, "pyftpdlib ready."));
        assert!(greeting.positive());

        let features = reply("211-Features supported:\r\n EPSV\r\n MLST type*;size*;\r\n211 End FEAT.\r\nnext");
        assert_eq!(features.code, 211);
        assert_eq!(features.lines, ["Features supported:", "EPSV", "MLST type*;size*;", "End FEAT."]);

        // lines of a long reply may repeat the code, or start with another number
        let long = reply("230-Welcome\r\n230-second line\r\n200 files today\r\n230 Logged in\r\n");
        assert_eq!(long.lines.len(), 4);
        assert_eq!(long.text(), "Logged in");

        let refused = reply("530 Login incorrect.\r\n");
        assert!(!refused.positive());
        assert_eq!(reply("200\r\n").text(), "");
        assert!(read_reply(&mut Cursor::new(b"220-never ends\r\n".as_slice())).is_err());
        assert!(read_reply(&mut Cursor::new(b"".as_slice())).is_err());
    }

    #[test]
    fn data_connection_ports() {
        assert_eq!(parse_epsv("Entering extended passive mode (|||60001|)."), Some(60001));
        assert_eq!(parse_epsv("no port here"), None);
        assert_eq!(parse_pasv("Entering passive mode (127,0,0,1,234,97)."), Some(234 * 256 + 97));
        assert_eq!(parse_pasv("=127,0,0,1,0,21"), Some(21));
        assert_eq!(parse_pasv("Entering passive mode (127,0,0,1)."), None);
        assert_eq!(parse_pasv("Entering passive mode (127,0,0,1,999,1)."), None);
    }

    #[test]
    fn current_folder() {
        assert_eq!(parse_pwd("\"/home/me\" is the current directory."), Some("/home/me".into()));
        assert_eq!(parse_pwd("\"/\""), Some("/".into()));
        assert_eq!(parse_pwd("\"/odd \"\"name\"\"\" is current"), Some("/odd \"name\"".into()));
        assert_eq!(parse_pwd("no folder"), None);
    }

    #[test]
    fn dates() {
        assert_eq!(epoch(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(epoch(2000, 3, 1, 0, 0, 0), 951868800);
        assert_eq!(parse_timestamp("20261119003800"), Some(1795048680));
        assert_eq!(parse_timestamp("20261119003800.250"), Some(1795048680));
        assert_eq!(parse_timestamp("2026"), None);
        assert_eq!(parse_timestamp("2026111900380x"), None);
    }

    #[test]
    fn machine_readable_listing() {
        let file = parse_mlsd("modify=20261119003800;perm=radfw;size=3000000;type=file;unique=26g130b;UNIX.mode=0640;UNIX.owner=me;UNIX.group=users; big file.bin").unwrap();
        assert_eq!(file.name, "big file.bin");
        assert_eq!((file.kind, file.size, file.mode), (Kind::File, 3000000, 0o640));
        assert_eq!(file.mtime, 1795048680);
        assert_eq!((file.owner.as_str(), file.group.as_str()), ("me", "users"));

        assert_eq!(parse_mlsd("type=file;unix.mode=0o600; a").unwrap().mode, 0o600);
        let folder = parse_mlsd("Type=dir;Size=60;UNIX.uid=1000;UNIX.gid=100; docs").unwrap();
        assert_eq!((folder.kind, folder.size), (Kind::Folder, 0));
        assert_eq!((folder.owner.as_str(), folder.group.as_str()), ("1000", "100"));

        let link = parse_mlsd("type=OS.unix=slink:/srv/target;size=4; shortcut").unwrap();
        assert_eq!(link.link_target, Some(PathBuf::from("/srv/target")));
        assert_eq!(link.kind, Kind::File);

        assert_eq!(parse_mlsd("type=cdir;modify=20261119003800; ."), None);
        assert_eq!(parse_mlsd("type=pdir; .."), None);
        assert_eq!(parse_mlsd("size=4; no type"), None);
        assert_eq!(parse_mlsd("garbage"), None);
        assert_eq!(parse_mlsd(""), None);
    }

    #[test]
    fn unix_listing() {
        let file = parse_list("-rw-r-----   1 jimy     users     3000000 Nov 18 23:04 big file.bin").unwrap();
        assert_eq!(file.name, "big file.bin");
        assert_eq!((file.kind, file.size, file.mode), (Kind::File, 3000000, 0o640));
        assert_eq!((file.owner.as_str(), file.group.as_str()), ("jimy", "users"));

        let folder = parse_list("drwxrwxr-x   2 jimy     jimy           60 Nov 18  2025 docs").unwrap();
        assert_eq!((folder.kind, folder.size, folder.mode), (Kind::Folder, 0, 0o775));

        let link = parse_list("lrwxrwxrwx   1 root     root            4 Nov 18 23:04 shortcut -> docs").unwrap();
        assert_eq!(link.name, "shortcut");
        assert_eq!(link.link_target, Some(PathBuf::from("docs")));

        assert_eq!(parse_list("drwxr-xr-x   2 root     root           60 Nov 18 23:04 ."), None);
        assert_eq!(parse_list("drwxr-xr-x   2 root     root           60 Nov 18 23:04 .."), None);
        assert_eq!(parse_list("total 12"), None);
        assert_eq!(parse_list(""), None);
        assert_eq!(parse_list("-rw-r--r-- 1 too short"), None);
    }

    #[test]
    fn failures_are_told_apart() {
        let path = Path::new("/a");
        let missing = Reply { code: 550, lines: vec!["No such file or directory.".into()] };
        assert_eq!(failure(&missing, path), FsError::NotFound(path.to_owned()));
        let denied = Reply { code: 550, lines: vec!["Permission denied.".into()] };
        assert_eq!(failure(&denied, path), FsError::PermissionDenied(path.to_owned()));
        let other = Reply { code: 553, lines: vec!["Disk full".into()] };
        assert_eq!(failure(&other, path), FsError::Other("/a: Disk full".into()));
    }

    /// a server saying what it is told to, for one connection
    fn fake_server(script: &'static [&'static str]) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut commands = BufReader::new(stream.try_clone().unwrap());
            stream.write_all(script[0].as_bytes()).unwrap();
            for answer in &script[1..] {
                let mut command = String::new();
                if commands.read_line(&mut command).unwrap_or(0) == 0 {
                    break;
                }
                stream.write_all(answer.as_bytes()).unwrap();
            }
        });
        port
    }

    #[test]
    fn login_sequence() {
        let port = fake_server(&[
            "220 ready\r\n",
            "331 password please\r\n",
            "230 welcome\r\n",
            "200 binary\r\n",
            "200 utf8\r\n",
            "211-Features:\r\n MLSD\r\n211 end\r\n",
            "200 facts\r\n",
            "257 \"/home/me\" is current\r\n",
        ]);
        let session = connect("127.0.0.1", port, "me", "secret").unwrap();
        assert_eq!(session.home(), PathBuf::from("/home/me"));
        assert!(session.inner.control.lock().unwrap().mlsd);
    }

    #[test]
    fn refused_login_and_unreachable_server() {
        let port = fake_server(&["220 ready\r\n", "331 password please\r\n", "530 Login incorrect\r\n"]);
        assert_eq!(connect("127.0.0.1", port, "me", "wrong").err(), Some(ConnectError::Login));
        assert!(matches!(connect("127.0.0.1", 1, "me", "x"), Err(ConnectError::Unreachable(_))));
        let port = fake_server(&["421 too many users\r\n"]);
        assert_eq!(connect("127.0.0.1", port, "me", "x").err(), Some(ConnectError::Other("too many users".into())));
    }
}
