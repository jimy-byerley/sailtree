//! makes files readable by the application opening them.
//!
//! viewers run in a sandbox that only shows them the common user folders. a file elsewhere gets
//! a temporary hard link in a folder they can see, and the link is opened instead.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// folders of the home folder that sandboxed applications can see
const VISIBLE: [&str; 6] = ["Documents", "Downloads", "Music", "Pictures", "Videos", "Public"];
/// where removable media are mounted, also visible
const MEDIA: &str = "/run/media";
/// folder of the links, relative to the home folder. hidden, so the media indexer ignores it
const FOLDER: &str = "Downloads/.sailtree-open";

static NEXT: AtomicU32 = AtomicU32::new(0);

/// whether sandboxed applications can read this path as it is
pub fn visible(home: &Path, path: &Path) -> bool {
    path.starts_with(MEDIA) || VISIBLE.iter().any(|name| path.starts_with(home.join(name)))
}

/// path to give to the application opening `path`: the file itself when it is visible,
/// otherwise a new hard link to it. when no link can be made (other filesystem, file of another
/// user) the original path is returned, for the application to report what it cannot read
pub fn stage(home: &Path, path: &Path) -> PathBuf {
    if visible(home, path) {
        return path.to_owned();
    }
    match path.file_name() {
        Some(name) => stage_as(home, path, name).unwrap_or_else(|_| path.to_owned()),
        None => path.to_owned(),
    }
}

/// a new empty folder applications can see, for a file fetched from another device
pub fn folder(home: &Path) -> io::Result<PathBuf> {
    let folder = home.join(FOLDER).join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

/// a new hard link to `path` under another name, for applications that judge a file by its name
pub fn stage_as(home: &Path, path: &Path, name: impl AsRef<Path>) -> io::Result<PathBuf> {
    // one folder per link: the file keeps its name, which viewers show and guess the type from
    let folder = home.join(FOLDER).join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
    fs::create_dir_all(&folder)?;
    let staged = folder.join(name);
    match fs::hard_link(path, &staged) {
        Ok(()) => Ok(staged),
        Err(error) => {
            let _ = fs::remove_dir(&folder);
            Err(error)
        }
    }
}

/// remove all the links, when the app starts and exits. the files they point to are not touched
pub fn clean(home: &Path) {
    let _ = fs::remove_dir_all(home.join(FOLDER));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn user_folders_and_media_are_visible() {
        let home = Path::new("/home/user");
        assert!(visible(home, Path::new("/home/user/Pictures/a.png")));
        assert!(visible(home, Path::new("/home/user/Documents/deep/er/a.txt")));
        assert!(visible(home, Path::new("/home/user/Downloads/.sailtree-open/0/a.txt")));
        assert!(visible(home, Path::new("/run/media/user/card/a.txt")));
        assert!(!visible(home, Path::new("/home/user/a.txt")));
        assert!(!visible(home, Path::new("/home/user/Projects/a.txt")));
        assert!(!visible(home, Path::new("/home/user/PicturesOld/a.png")));
        assert!(!visible(home, Path::new("/home/other/Pictures/a.png")));
        assert!(!visible(home, Path::new("/etc/hosts")));
    }

    #[test]
    fn visible_files_are_opened_in_place() {
        let dir = TempDir::new();
        let home = dir.path();
        fs::create_dir(home.join("Pictures")).unwrap();
        let picture = home.join("Pictures/a.png");
        fs::write(&picture, b"png").unwrap();
        assert_eq!(stage(home, &picture), picture);
        assert!(!home.join(FOLDER).exists());
    }

    #[test]
    fn other_files_get_a_hard_link() {
        let dir = TempDir::new();
        let home = dir.path();
        let original = home.join("notes.txt");
        fs::write(&original, b"notes").unwrap();

        let staged = stage(home, &original);
        assert_ne!(staged, original);
        assert!(staged.starts_with(home.join(FOLDER)));
        assert!(visible(home, &staged));
        assert_eq!(staged.file_name(), original.file_name());
        // the same file, not a copy
        assert_eq!(fs::metadata(&staged).unwrap().ino(), fs::metadata(&original).unwrap().ino());

        // opening again, or another file with the same name, does not collide
        fs::create_dir(home.join("other")).unwrap();
        fs::write(home.join("other/notes.txt"), b"other").unwrap();
        let again = stage(home, &original);
        let other = stage(home, &home.join("other/notes.txt"));
        assert_ne!(again, staged);
        assert_eq!(fs::read(&other).unwrap(), b"other");
        assert_eq!(fs::read(&again).unwrap(), b"notes");
    }

    #[test]
    fn folders_for_fetched_files() {
        let dir = TempDir::new();
        let home = dir.path();
        let (first, second) = (folder(home).unwrap(), folder(home).unwrap());
        assert_ne!(first, second);
        assert!(first.is_dir() && visible(home, &first));
        clean(home);
        assert!(!first.exists());
    }

    #[test]
    fn link_under_another_name() {
        let dir = TempDir::new();
        let home = dir.path();
        fs::create_dir(home.join("Documents")).unwrap();
        let original = home.join("Documents/server.log");
        fs::write(&original, b"").unwrap();
        // also for a file that is visible as it is
        let staged = stage_as(home, &original, "server.log.txt").unwrap();
        assert!(staged.ends_with("server.log.txt"));
        assert!(visible(home, &staged));
        assert_eq!(fs::metadata(&staged).unwrap().ino(), fs::metadata(&original).unwrap().ino());
        assert!(stage_as(home, &home.join("missing"), "missing.txt").is_err());
    }

    #[test]
    fn file_that_cannot_be_linked_is_opened_in_place() {
        let dir = TempDir::new();
        let home = dir.path();
        // folders cannot be hard linked, nor missing files
        fs::create_dir(home.join("folder")).unwrap();
        assert_eq!(stage(home, &home.join("folder")), home.join("folder"));
        assert_eq!(stage(home, &home.join("missing")), home.join("missing"));
        assert_eq!(stage(home, Path::new("/")), Path::new("/"));
        // nothing is left behind
        assert_eq!(fs::read_dir(home.join(FOLDER)).unwrap().count(), 0);
    }

    #[test]
    fn clean_removes_the_links_and_keeps_the_files() {
        let dir = TempDir::new();
        let home = dir.path();
        let original = home.join("notes.txt");
        fs::write(&original, b"notes").unwrap();
        let staged = stage(home, &original);
        assert!(staged.exists());

        clean(home);
        assert!(!staged.exists());
        assert!(!home.join(FOLDER).exists());
        assert!(home.join("Downloads").exists());
        assert_eq!(fs::read(&original).unwrap(), b"notes");
        // nothing to clean is fine
        clean(home);
    }
}
