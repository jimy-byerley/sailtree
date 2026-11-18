//! the filesystem of the device the app runs on

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{Entry, FsError, Kind, Result};

#[derive(Debug, Clone, Default)]
pub struct Local;

impl Local {
    pub fn home(&self) -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    pub async fn list(&self, path: &Path) -> Result<Vec<Entry>> {
        let path = path.to_owned();
        blocking(move || {
            let mut names = Names::default();
            let mut entries = Vec::new();
            for item in fs::read_dir(&path).map_err(|error| FsError::from_io(error, &path))? {
                let item = item.map_err(|error| FsError::from_io(error, &path))?;
                // an item deleted between the listing and its stat is simply skipped
                if let Ok(entry) = stat(&item.path(), &mut names) {
                    entries.push(entry);
                }
            }
            Ok(entries)
        })
        .await
    }

    pub async fn stat(&self, path: &Path) -> Result<Entry> {
        let path = path.to_owned();
        blocking(move || stat(&path, &mut Names::default())).await
    }

    pub async fn rename(&self, path: &Path, name: &str) -> Result<PathBuf> {
        let source = path.to_owned();
        let destination = path.with_file_name(name);
        blocking(move || {
            fs::symlink_metadata(&source).map_err(|error| FsError::from_io(error, &source))?;
            // a plain rename would silently replace an existing file
            if destination != source && fs::symlink_metadata(&destination).is_ok() {
                return Err(FsError::AlreadyExists(destination));
            }
            fs::rename(&source, &destination).map_err(|error| FsError::from_io(error, &source))?;
            Ok(destination)
        })
        .await
    }

    pub async fn delete(&self, path: &Path) -> Result<()> {
        let path = path.to_owned();
        blocking(move || {
            let metadata = fs::symlink_metadata(&path).map_err(|error| FsError::from_io(error, &path))?;
            if metadata.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
            .map_err(|error| FsError::from_io(error, &path))
        })
        .await
    }

    pub async fn mkdir(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        blocking(move || {
            fs::create_dir(&path).map_err(|error| FsError::from_io(error, &path))?;
            Ok(path)
        })
        .await
    }

    pub async fn create_file(&self, parent: &Path, name: &str) -> Result<PathBuf> {
        let path = parent.join(name);
        blocking(move || {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|error| FsError::from_io(error, &path))?;
            Ok(path)
        })
        .await
    }

    pub async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        let path = path.to_owned();
        blocking(move || {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .map_err(|error| FsError::from_io(error, &path))
        })
        .await
    }
}

/// run blocking filesystem calls out of the async worker threads
async fn blocking<T, F>(operation: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .unwrap_or_else(|error| Err(FsError::Other(format!("filesystem task failed: {error}"))))
}

fn stat(path: &Path, names: &mut Names) -> Result<Entry> {
    let own = fs::symlink_metadata(path).map_err(|error| FsError::from_io(error, path))?;
    let link_target = if own.file_type().is_symlink() {
        Some(fs::read_link(path).map_err(|error| FsError::from_io(error, path))?)
    } else {
        None
    };
    // size and kind are those of the link target, when it can be reached
    let target = match link_target {
        Some(_) => fs::metadata(path).ok(),
        None => None,
    };
    let shown = target.as_ref().unwrap_or(&own);
    let name = match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    };
    Ok(Entry {
        name,
        kind: if shown.is_dir() { Kind::Folder } else { Kind::File },
        link_target,
        size: shown.len(),
        mtime: shown.mtime(),
        owner: names.user(shown.uid()),
        group: names.group(shown.gid()),
        mode: shown.mode() & 0o7777,
    })
}

/// cache of user and group names, a folder usually has the same owner for all its items
#[derive(Default)]
struct Names {
    users: HashMap<u32, String>,
    groups: HashMap<u32, String>,
}

impl Names {
    fn user(&mut self, uid: u32) -> String {
        self.users
            .entry(uid)
            .or_insert_with(|| user_name(uid).unwrap_or_else(|| uid.to_string()))
            .clone()
    }

    fn group(&mut self, gid: u32) -> String {
        self.groups
            .entry(gid)
            .or_insert_with(|| group_name(gid).unwrap_or_else(|| gid.to_string()))
            .clone()
    }
}

fn user_name(uid: u32) -> Option<String> {
    let mut buffer = vec![0; 1024];
    loop {
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(uid, &mut record, buffer.as_mut_ptr(), buffer.len(), &mut found)
        };
        if status == libc::ERANGE && buffer.len() < 1 << 20 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(record.pw_name) };
        return Some(name.to_string_lossy().into_owned());
    }
}

fn group_name(gid: u32) -> Option<String> {
    let mut buffer = vec![0; 1024];
    loop {
        let mut record: libc::group = unsafe { std::mem::zeroed() };
        let mut found = std::ptr::null_mut();
        let status = unsafe {
            libc::getgrgid_r(gid, &mut record, buffer.as_mut_ptr(), buffer.len(), &mut found)
        };
        if status == libc::ERANGE && buffer.len() < 1 << 20 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(record.gr_name) };
        return Some(name.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::Fs;
    use crate::testing::TempDir;
    use std::os::unix::fs::symlink;

    fn local() -> Fs {
        Fs::Local(Local)
    }

    fn sorted_names(entries: &[Entry]) -> Vec<&str> {
        let mut names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        names.sort();
        names
    }

    fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries.iter().find(|entry| entry.name == name).expect(name)
    }

    fn is_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    #[tokio::test]
    async fn list_reports_names_kinds_and_sizes() {
        let dir = TempDir::new();
        fs::write(dir.path().join("file.txt"), b"hello").unwrap();
        fs::write(dir.path().join(".hidden"), b"").unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();

        let entries = local().list(dir.path()).await.unwrap();
        assert_eq!(sorted_names(&entries), [".hidden", "file.txt", "folder"]);

        let file = find(&entries, "file.txt");
        assert_eq!(file.kind, Kind::File);
        assert_eq!(file.size, 5);
        assert_eq!(file.link_target, None);
        assert!(!file.hidden());
        assert_eq!(find(&entries, "folder").kind, Kind::Folder);
        assert!(find(&entries, ".hidden").hidden());
    }

    #[tokio::test]
    async fn list_reports_owner_date_and_permissions() {
        let dir = TempDir::new();
        let path = dir.path().join("file");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let entries = local().list(dir.path()).await.unwrap();
        let file = find(&entries, "file");
        assert_eq!(file.mode, 0o640);
        let uid = unsafe { libc::geteuid() };
        assert_eq!(file.owner, user_name(uid).unwrap_or_else(|| uid.to_string()));
        assert!(!file.group.is_empty());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!((now - file.mtime).abs() < 60, "mtime {} vs now {}", file.mtime, now);
    }

    #[tokio::test]
    async fn list_of_an_empty_folder() {
        let dir = TempDir::new();
        assert_eq!(local().list(dir.path()).await.unwrap(), []);
    }

    #[tokio::test]
    async fn symlinks_report_their_target() {
        let dir = TempDir::new();
        fs::write(dir.path().join("file"), b"12345678").unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        symlink("file", dir.path().join("to-file")).unwrap();
        symlink(dir.path().join("folder"), dir.path().join("to-folder")).unwrap();
        symlink("missing", dir.path().join("broken")).unwrap();

        let entries = local().list(dir.path()).await.unwrap();
        let to_file = find(&entries, "to-file");
        assert_eq!(to_file.kind, Kind::File);
        assert_eq!(to_file.size, 8);
        assert_eq!(to_file.link_target.as_deref(), Some(Path::new("file")));

        let to_folder = find(&entries, "to-folder");
        assert_eq!(to_folder.kind, Kind::Folder);
        assert_eq!(to_folder.link_target, Some(dir.path().join("folder")));

        let broken = find(&entries, "broken");
        assert_eq!(broken.kind, Kind::File);
        assert_eq!(broken.link_target.as_deref(), Some(Path::new("missing")));
    }

    #[tokio::test]
    async fn list_through_a_symlink_to_a_folder() {
        let dir = TempDir::new();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/inside"), b"").unwrap();
        symlink("folder", dir.path().join("link")).unwrap();

        let entries = local().list(&dir.path().join("link")).await.unwrap();
        assert_eq!(sorted_names(&entries), ["inside"]);
    }

    #[tokio::test]
    async fn list_errors() {
        let dir = TempDir::new();
        let missing = dir.path().join("missing");
        assert_eq!(local().list(&missing).await, Err(FsError::NotFound(missing)));

        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        assert!(matches!(local().list(&file).await, Err(FsError::Other(_))));
    }

    #[tokio::test]
    async fn list_without_permission() {
        if is_root() {
            return; // root can read everything
        }
        let dir = TempDir::new();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        let result = local().list(&locked).await;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result, Err(FsError::PermissionDenied(locked)));
    }

    #[tokio::test]
    async fn stat_of_one_item() {
        let dir = TempDir::new();
        let path = dir.path().join("file.txt");
        fs::write(&path, b"abc").unwrap();

        let entry = local().stat(&path).await.unwrap();
        assert_eq!(entry.name, "file.txt");
        assert_eq!(entry.kind, Kind::File);
        assert_eq!(entry.size, 3);

        let missing = dir.path().join("missing");
        assert_eq!(local().stat(&missing).await, Err(FsError::NotFound(missing)));
        assert_eq!(local().stat(Path::new("/")).await.unwrap().kind, Kind::Folder);
    }

    #[tokio::test]
    async fn rename_keeps_content() {
        let dir = TempDir::new();
        let old = dir.path().join("old");
        fs::write(&old, b"content").unwrap();

        let new = local().rename(&old, "new").await.unwrap();
        assert_eq!(new, dir.path().join("new"));
        assert!(!old.exists());
        assert_eq!(fs::read(&new).unwrap(), b"content");
    }

    #[tokio::test]
    async fn rename_of_a_folder_keeps_its_content() {
        let dir = TempDir::new();
        fs::create_dir(dir.path().join("old")).unwrap();
        fs::write(dir.path().join("old/inside"), b"").unwrap();

        local().rename(&dir.path().join("old"), "new").await.unwrap();
        assert!(dir.path().join("new/inside").exists());
    }

    #[tokio::test]
    async fn rename_refuses_to_overwrite() {
        let dir = TempDir::new();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::write(&first, b"1").unwrap();
        fs::write(&second, b"2").unwrap();

        assert_eq!(
            local().rename(&first, "second").await,
            Err(FsError::AlreadyExists(second.clone()))
        );
        assert_eq!(fs::read(&first).unwrap(), b"1");
        assert_eq!(fs::read(&second).unwrap(), b"2");
        // renaming to its own name is not a conflict
        assert_eq!(local().rename(&first, "first").await, Ok(first));
    }

    #[tokio::test]
    async fn rename_errors() {
        let dir = TempDir::new();
        let missing = dir.path().join("missing");
        assert_eq!(
            local().rename(&missing, "new").await,
            Err(FsError::NotFound(missing))
        );
        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        assert_eq!(
            local().rename(&file, "a/b").await,
            Err(FsError::InvalidName("a/b".into()))
        );
        assert!(file.exists());
    }

    #[tokio::test]
    async fn delete_file_and_non_empty_folder() {
        let dir = TempDir::new();
        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        let folder = dir.path().join("folder");
        fs::create_dir_all(folder.join("nested/deeper")).unwrap();
        fs::write(folder.join("nested/file"), b"").unwrap();

        local().delete(&file).await.unwrap();
        local().delete(&folder).await.unwrap();
        assert!(!file.exists());
        assert!(!folder.exists());
        assert_eq!(local().delete(&file).await, Err(FsError::NotFound(file)));
    }

    #[tokio::test]
    async fn delete_of_a_symlink_keeps_its_target() {
        let dir = TempDir::new();
        let folder = dir.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("inside"), b"").unwrap();
        let link = dir.path().join("link");
        symlink(&folder, &link).unwrap();

        local().delete(&link).await.unwrap();
        assert!(fs::symlink_metadata(&link).is_err());
        assert!(folder.join("inside").exists());
    }

    #[tokio::test]
    async fn mkdir_and_create_file() {
        let dir = TempDir::new();
        let folder = local().mkdir(dir.path(), "folder").await.unwrap();
        assert_eq!(folder, dir.path().join("folder"));
        assert!(folder.is_dir());

        let file = local().create_file(&folder, "file.txt").await.unwrap();
        assert_eq!(file, folder.join("file.txt"));
        assert_eq!(fs::read(&file).unwrap(), b"");
    }

    #[tokio::test]
    async fn creation_errors() {
        let dir = TempDir::new();
        fs::write(dir.path().join("taken"), b"data").unwrap();

        let taken = dir.path().join("taken");
        assert_eq!(
            local().mkdir(dir.path(), "taken").await,
            Err(FsError::AlreadyExists(taken.clone()))
        );
        assert_eq!(
            local().create_file(dir.path(), "taken").await,
            Err(FsError::AlreadyExists(taken.clone()))
        );
        assert_eq!(fs::read(&taken).unwrap(), b"data");

        assert_eq!(
            local().mkdir(dir.path(), "").await,
            Err(FsError::InvalidName("".into()))
        );
        assert_eq!(
            local().create_file(dir.path(), "..").await,
            Err(FsError::InvalidName("..".into()))
        );
        let nowhere = dir.path().join("missing");
        assert_eq!(
            local().mkdir(&nowhere, "folder").await,
            Err(FsError::NotFound(nowhere.join("folder")))
        );
    }

    #[tokio::test]
    async fn creation_without_permission() {
        if is_root() {
            return;
        }
        let dir = TempDir::new();
        let readonly = dir.path().join("readonly");
        fs::create_dir(&readonly).unwrap();
        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o500)).unwrap();

        let result = local().create_file(&readonly, "file").await;
        fs::set_permissions(&readonly, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result, Err(FsError::PermissionDenied(readonly.join("file"))));
    }

    #[tokio::test]
    async fn chmod_changes_permission_bits() {
        let dir = TempDir::new();
        let path = dir.path().join("file");
        fs::write(&path, b"").unwrap();

        local().chmod(&path, 0o600).await.unwrap();
        assert_eq!(local().stat(&path).await.unwrap().mode, 0o600);
        local().chmod(&path, 0o754).await.unwrap();
        assert_eq!(local().stat(&path).await.unwrap().mode, 0o754);

        let missing = dir.path().join("missing");
        assert_eq!(
            local().chmod(&missing, 0o600).await,
            Err(FsError::NotFound(missing))
        );
    }

    #[test]
    fn home_is_absolute() {
        assert!(Local.home().is_absolute());
    }

    #[test]
    fn unknown_ids_fall_back_to_numbers() {
        let mut names = Names::default();
        // an id nobody uses
        assert_eq!(names.user(4_000_000_123), "4000000123");
        assert_eq!(names.group(4_000_000_123), "4000000123");
        assert_eq!(user_name(0).as_deref(), Some("root"));
        assert_eq!(group_name(0).as_deref(), Some("root"));
    }
}
