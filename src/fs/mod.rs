//! filesystem backends behind one common set of operations

pub mod local;

use std::fmt;
use std::path::{Path, PathBuf};

/// what an entry is, after following symbolic links
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// one item of a folder listing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// kind of the link target for a symbolic link, `File` for a broken link
    pub kind: Kind,
    /// where the symbolic link points to, `None` when the entry is not a link
    pub link_target: Option<PathBuf>,
    /// size in bytes, of the link target for a symbolic link
    pub size: u64,
    /// modification time in seconds since the unix epoch
    pub mtime: i64,
    /// owner name, or numeric id when it has no name
    pub owner: String,
    /// group name, or numeric id when it has no name
    pub group: String,
    /// permission bits only (`0o7777` mask)
    pub mode: u32,
}

impl Entry {
    /// hidden by convention: the name starts with a dot
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// failure of a filesystem operation, with a message that can be shown to the user
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound(PathBuf),
    PermissionDenied(PathBuf),
    AlreadyExists(PathBuf),
    InvalidName(String),
    Other(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FsError::NotFound(path) => write!(f, "{} does not exist", path.display()),
            FsError::PermissionDenied(path) => write!(f, "permission denied on {}", path.display()),
            FsError::AlreadyExists(path) => write!(f, "{} already exists", path.display()),
            FsError::InvalidName(name) => write!(f, "\"{name}\" is not a valid name"),
            FsError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FsError {}

impl FsError {
    /// convert an io error raised while working on `path`
    pub fn from_io(error: std::io::Error, path: &Path) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::NotFound => FsError::NotFound(path.to_owned()),
            ErrorKind::PermissionDenied => FsError::PermissionDenied(path.to_owned()),
            ErrorKind::AlreadyExists => FsError::AlreadyExists(path.to_owned()),
            _ => FsError::Other(format!("{}: {}", path.display(), error)),
        }
    }
}

pub type Result<T> = std::result::Result<T, FsError>;

/// a filesystem that can be explored and modified
#[derive(Debug, Clone)]
pub enum Fs {
    Local(local::Local),
}

impl Fs {
    /// folder to start exploring from
    pub fn home(&self) -> PathBuf {
        match self {
            Fs::Local(fs) => fs.home(),
        }
    }

    /// content of a folder, in no particular order
    pub async fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        match self {
            Fs::Local(fs) => fs.list(path).await,
        }
    }

    /// details of one item
    pub async fn stat(&self, path: &Path) -> Result<Entry> {
        match self {
            Fs::Local(fs) => fs.stat(path).await,
        }
    }

    /// rename an item inside its folder, returns its new path.
    /// fails if the new name is already taken
    pub async fn rename(&self, path: &Path, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        match self {
            Fs::Local(fs) => fs.rename(path, name).await,
        }
    }

    /// delete a file, a symbolic link (not its target), or a folder with its content
    pub async fn delete(&self, path: &Path) -> Result<()> {
        match self {
            Fs::Local(fs) => fs.delete(path).await,
        }
    }

    /// create an empty folder in `parent`, returns its path
    pub async fn mkdir(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        match self {
            Fs::Local(fs) => fs.mkdir(parent, name).await,
        }
    }

    /// create an empty file in `parent`, returns its path
    pub async fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        match self {
            Fs::Local(fs) => fs.create_file(parent, name).await,
        }
    }

    /// copy an item, a folder with its content, into `folder`. returns the path of the copy.
    /// fails if `folder` already has an item of this name
    pub async fn copy_into(&self, source: &Path, folder: &Path) -> Result<PathBuf> {
        match self {
            Fs::Local(fs) => fs.copy_into(source, folder).await,
        }
    }

    /// move an item into `folder`, returns its new path.
    /// fails if `folder` already has an item of this name
    pub async fn move_into(&self, source: &Path, folder: &Path) -> Result<PathBuf> {
        match self {
            Fs::Local(fs) => fs.move_into(source, folder).await,
        }
    }

    /// set the permission bits of an item
    pub async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        match self {
            Fs::Local(fs) => fs.chmod(path, mode & 0o7777).await,
        }
    }
}

/// check that `name` can be the name of a single file or folder
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(FsError::InvalidName(name.to_owned()));
    }
    Ok(())
}

/// permission bits as shown by `ls`, eg. `rwxr-xr-x`
pub fn mode_to_string(mode: u32) -> String {
    let mut text = String::with_capacity(9);
    for shift in [6, 3, 0] {
        let bits = mode >> shift;
        text.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        text.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        text.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_string_conversion() {
        assert_eq!(mode_to_string(0o755), "rwxr-xr-x");
        assert_eq!(mode_to_string(0o640), "rw-r-----");
        assert_eq!(mode_to_string(0o000), "---------");
        assert_eq!(mode_to_string(0o777), "rwxrwxrwx");
        // file type and special bits are ignored
        assert_eq!(mode_to_string(0o100644), "rw-r--r--");
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("file.txt").is_ok());
        assert!(validate_name(".hidden").is_ok());
        assert!(validate_name("with space").is_ok());
        assert!(validate_name("...").is_ok());
        for name in ["", ".", "..", "a/b", "/", "nul\0"] {
            assert_eq!(validate_name(name), Err(FsError::InvalidName(name.to_owned())));
        }
    }

    #[test]
    fn hidden_entries() {
        let mut entry = Entry {
            name: ".config".into(),
            kind: Kind::Folder,
            link_target: None,
            size: 0,
            mtime: 0,
            owner: "me".into(),
            group: "me".into(),
            mode: 0o755,
        };
        assert!(entry.hidden());
        entry.name = "config".into();
        assert!(!entry.hidden());
    }

    #[test]
    fn io_error_conversion() {
        use std::io::{Error, ErrorKind};
        let path = Path::new("/some/where");
        assert_eq!(
            FsError::from_io(Error::from(ErrorKind::NotFound), path),
            FsError::NotFound(path.to_owned())
        );
        assert_eq!(
            FsError::from_io(Error::from(ErrorKind::PermissionDenied), path),
            FsError::PermissionDenied(path.to_owned())
        );
        assert_eq!(
            FsError::from_io(Error::from(ErrorKind::AlreadyExists), path),
            FsError::AlreadyExists(path.to_owned())
        );
        let other = FsError::from_io(Error::new(ErrorKind::Other, "disk on fire"), path);
        assert_eq!(other.to_string(), "/some/where: disk on fire");
    }
}
