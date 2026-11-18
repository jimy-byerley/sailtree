//! one copy or move of several items into a folder, done piece by piece so it can report its
//! progress, be cancelled, and go from one device to another

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::oneshot;

use super::conflict::{numbered, Answer, Decision};
use crate::fs::{Entry, Fs, FsError, Kind};

pub struct Job {
    /// take the items away from where they are, instead of copying them
    pub moving: bool,
    /// where the items are
    pub from: Fs,
    pub sources: Vec<PathBuf>,
    /// where they go
    pub to: Fs,
    pub folder: PathBuf,
    /// `from` and `to` are the same device: items can be renamed instead of travelling
    pub same_device: bool,
}

/// what a running job tells
pub enum Event {
    /// bytes handled out of the total, and the item being transferred
    Progress { done: u64, total: u64, name: String },
    /// the destination has an item of this name: the job waits for the reply.
    /// dropping the reply skips the item
    Conflict { name: String, reply: oneshot::Sender<Decision> },
}

#[derive(Debug, Default)]
pub struct Outcome {
    /// items now in the destination folder, with their details
    pub arrived: Vec<(PathBuf, Entry)>,
    /// sources that are gone after a move
    pub left: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

/// why the transfer of an item stopped
enum Stop {
    Cancelled,
    Failed(String),
}

impl From<FsError> for Stop {
    fn from(error: FsError) -> Self {
        Stop::Failed(error.to_string())
    }
}

/// how the transfer of an item ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// copied entirely
    Copied,
    /// renamed: it is not at its source anymore
    Moved,
    /// skipped, or something inside it was: the source of a move must stay
    Partial,
}

type Step<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

struct Run<'a, E> {
    job: &'a Job,
    cancel: &'a AtomicBool,
    events: &'a E,
    done: u64,
    total: u64,
    /// progress last told
    told: u64,
    /// answer given for all the conflicts
    sticky: Option<Answer>,
}

/// do the job. `events` is called from the task running it
pub async fn run<E>(job: Job, cancel: &AtomicBool, events: E) -> Outcome
where
    E: Fn(Event) + Send + Sync,
{
    let mut outcome = Outcome::default();
    let mut run = Run { job: &job, cancel, events: &events, done: 0, total: 0, told: 0, sticky: None };
    for source in &job.sources {
        run.total += size(&job.from, source).await;
    }
    for source in &job.sources {
        match run.top(source).await {
            Ok(Some((arrived, status))) => {
                if let Ok(entry) = job.to.stat(&arrived).await {
                    outcome.arrived.push((arrived, entry));
                }
                if job.moving && status != Status::Partial {
                    outcome.left.push(source.clone());
                }
            }
            Ok(None) => {}
            Err(Stop::Cancelled) => {
                outcome.cancelled = true;
                break;
            }
            Err(Stop::Failed(message)) => outcome.errors.push(message),
        }
    }
    outcome
}

/// bytes of a file, or of the files under a folder. links are not followed
fn size<'a>(fs: &'a Fs, path: &'a Path) -> Step<'a, u64> {
    Box::pin(async move {
        match fs.stat(path).await {
            Ok(entry) => entry_size(fs, path, &entry).await,
            Err(_) => 0,
        }
    })
}

fn entry_size<'a>(fs: &'a Fs, path: &'a Path, entry: &'a Entry) -> Step<'a, u64> {
    Box::pin(async move {
        if entry.link_target.is_some() {
            return 0;
        }
        if entry.kind == Kind::File {
            return entry.size;
        }
        let mut total = 0;
        for child in fs.list(path).await.unwrap_or_default() {
            total += entry_size(fs, &path.join(&child.name), &child).await;
        }
        total
    })
}

fn is_folder(entry: &Entry) -> bool {
    entry.kind == Kind::Folder && entry.link_target.is_none()
}

impl<E: Fn(Event) + Send + Sync> Run<'_, E> {
    fn check(&self) -> Result<(), Stop> {
        match self.cancel.load(Ordering::Relaxed) {
            true => Err(Stop::Cancelled),
            false => Ok(()),
        }
    }

    fn advance(&mut self, bytes: u64, name: &str) {
        self.done += bytes;
        // not more often than every percent
        if self.done == bytes || self.done - self.told >= self.total / 100 {
            self.told = self.done;
            (self.events)(Event::Progress { done: self.done, total: self.total, name: name.to_owned() });
        }
    }

    async fn ask(&mut self, name: &str) -> Answer {
        if let Some(answer) = self.sticky {
            return answer;
        }
        let (reply, answer) = oneshot::channel();
        (self.events)(Event::Conflict { name: name.to_owned(), reply });
        match answer.await {
            Ok(decision) => {
                if decision.all {
                    self.sticky = Some(decision.answer);
                }
                decision.answer
            }
            Err(_) => Answer::Skip,
        }
    }

    /// a free numbered name next to `taken`
    async fn free(&self, taken: &Path) -> PathBuf {
        let name = taken.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let mut number = 2;
        loop {
            let candidate = taken.with_file_name(numbered(&name, number));
            if self.job.to.stat(&candidate).await.is_err() {
                return candidate;
            }
            number += 1;
        }
    }

    /// transfer one of the items of the job. `None` when it was skipped
    async fn top(&mut self, source: &Path) -> Result<Option<(PathBuf, Status)>, Stop> {
        self.check()?;
        let Some(name) = source.file_name() else {
            return Err(Stop::Failed(format!("{} cannot be transferred", source.display())));
        };
        let entry = self.job.from.stat(source).await?;
        if self.job.same_device && is_folder(&entry) && self.job.folder.starts_with(source) {
            return Err(Stop::Failed(format!("{} cannot go inside itself", source.display())));
        }
        let mut destination = self.job.folder.join(name);
        if self.job.same_device && destination == source {
            // already there: nothing to move, and a copy is a duplicate
            if self.job.moving {
                return Ok(None);
            }
            destination = self.free(&destination).await;
        }
        let Some((arrived, status)) = self.item(source, &entry, destination).await? else {
            return Ok(None);
        };
        if self.job.moving && status == Status::Copied {
            self.job.from.delete(source).await?;
        }
        Ok(Some((arrived, status)))
    }

    /// transfer a file, link or folder to `destination`, or next to it if the user says so.
    /// `None` when it was skipped
    fn item<'a>(
        &'a mut self,
        source: &'a Path,
        entry: &'a Entry,
        mut destination: PathBuf,
    ) -> Step<'a, Result<Option<(PathBuf, Status)>, Stop>> {
        Box::pin(async move {
            self.check()?;
            let job = self.job;
            let mut merge = false;
            if let Ok(existing) = job.to.stat(&destination).await {
                match self.ask(&entry.name).await {
                    Answer::Skip => {
                        let skipped = entry_size(&job.from, source, entry).await;
                        self.advance(skipped, &entry.name);
                        return Ok(None);
                    }
                    Answer::Rename => destination = self.free(&destination).await,
                    Answer::Overwrite if is_folder(entry) && is_folder(&existing) => merge = true,
                    Answer::Overwrite => job.to.delete(&destination).await?,
                }
                self.check()?;
            }
            if job.moving && job.same_device && !merge {
                match job.from.rename_to(source, &destination).await {
                    Ok(()) => {
                        let moved = entry_size(&job.to, &destination, entry).await;
                        self.advance(moved, &entry.name);
                        return Ok(Some((destination, Status::Moved)));
                    }
                    // another filesystem of the device: the content has to travel
                    Err(FsError::CrossDevice) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            let status = if let Some(target) = &entry.link_target {
                job.to.symlink(target, &destination).await?;
                Status::Copied
            } else if entry.kind == Kind::Folder {
                self.folder(source, entry, &destination, merge).await?
            } else {
                self.file(source, entry, &destination).await?;
                Status::Copied
            };
            Ok(Some((destination, status)))
        })
    }

    async fn folder(&mut self, source: &Path, entry: &Entry, destination: &Path, merge: bool) -> Result<Status, Stop> {
        let job = self.job;
        if !merge {
            let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
                return Err(Stop::Failed(format!("{} cannot be created", destination.display())));
            };
            job.to.mkdir(parent, &name.to_string_lossy()).await?;
        }
        let mut status = Status::Copied;
        for child in job.from.list(source).await? {
            let from = source.join(&child.name);
            match self.item(&from, &child, destination.join(&child.name)).await? {
                Some((_, Status::Copied)) => {
                    // inside a merged folder being moved, what was copied can go
                    if job.moving && merge {
                        job.from.delete(&from).await?;
                    }
                }
                Some((_, Status::Moved)) => {}
                Some((_, Status::Partial)) | None => status = Status::Partial,
            }
        }
        if !merge {
            // last, so the content of a read-only folder can be written
            job.to.chmod(destination, entry.mode).await?;
        }
        Ok(status)
    }

    async fn file(&mut self, source: &Path, entry: &Entry, destination: &Path) -> Result<(), Stop> {
        let job = self.job;
        let mut reader = job.from.open(source).await?;
        let mut writer = job.to.create(destination, entry.mode).await?;
        let copied: Result<(), Stop> = async {
            loop {
                self.check()?;
                let data = reader.read().await?;
                if data.is_empty() {
                    break;
                }
                let length = data.len() as u64;
                writer.write(data).await?;
                self.advance(length, &entry.name);
            }
            writer.finish().await?;
            Ok(())
        }
        .await;
        if copied.is_err() {
            // no half written file left behind
            let _ = job.to.delete(destination).await;
        }
        copied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::local::Local;
    use crate::testing::TempDir;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    /// temp/ { album/ { a.jpg, deep/ { b.jpg }, link -> a.jpg }, target/, note.txt }
    fn sample() -> TempDir {
        let dir = TempDir::new();
        let root = dir.path();
        fs::create_dir_all(root.join("album/deep")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::write(root.join("album/a.jpg"), b"aaaa").unwrap();
        fs::write(root.join("album/deep/b.jpg"), b"bb").unwrap();
        std::os::unix::fs::symlink("a.jpg", root.join("album/link")).unwrap();
        fs::write(root.join("note.txt"), b"note").unwrap();
        dir
    }

    fn job(dir: &TempDir, moving: bool, sources: &[&str], folder: &str) -> Job {
        Job {
            moving,
            from: Fs::Local(Local),
            sources: sources.iter().map(|source| dir.path().join(source)).collect(),
            to: Fs::Local(Local),
            folder: dir.path().join(folder),
            same_device: true,
        }
    }

    /// run a job answering every conflict the same way. returns the outcome, the conflicts
    /// asked and the last progress told
    async fn run_with(job: Job, decision: Option<Decision>) -> (Outcome, Vec<String>, (u64, u64)) {
        let asked = Mutex::new(Vec::new());
        let progress = Mutex::new((0, 0));
        let cancel = AtomicBool::new(false);
        let outcome = run(job, &cancel, |event| match event {
            Event::Progress { done, total, .. } => *progress.lock().unwrap() = (done, total),
            Event::Conflict { name, reply } => {
                asked.lock().unwrap().push(name);
                if let Some(decision) = decision {
                    let _ = reply.send(decision);
                }
            }
        })
        .await;
        let progress = *progress.lock().unwrap();
        (outcome, asked.into_inner().unwrap(), progress)
    }

    fn names(outcome: &Outcome) -> Vec<String> {
        outcome.arrived.iter().map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    const OVERWRITE: Option<Decision> = Some(Decision { answer: Answer::Overwrite, all: false });
    const RENAME: Option<Decision> = Some(Decision { answer: Answer::Rename, all: true });
    const SKIP: Option<Decision> = Some(Decision { answer: Answer::Skip, all: false });

    #[tokio::test]
    async fn copy_of_files_and_folders() {
        let dir = sample();
        let root = dir.path();
        fs::set_permissions(root.join("note.txt"), fs::Permissions::from_mode(0o600)).unwrap();
        let (outcome, asked, progress) = run_with(job(&dir, false, &["note.txt", "album"], "target"), None).await;
        assert_eq!(names(&outcome), ["note.txt", "album"]);
        assert!(outcome.errors.is_empty() && outcome.left.is_empty() && !outcome.cancelled);
        assert!(asked.is_empty());
        assert_eq!(progress, (10, 10));

        assert_eq!(fs::read(root.join("target/note.txt")).unwrap(), b"note");
        let mode = fs::metadata(root.join("target/note.txt")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(fs::read(root.join("target/album/deep/b.jpg")).unwrap(), b"bb");
        // links stay links
        assert_eq!(fs::read_link(root.join("target/album/link")).unwrap(), Path::new("a.jpg"));
        assert!(root.join("note.txt").exists());
        assert!(root.join("album/deep/b.jpg").exists());
    }

    #[tokio::test]
    async fn copy_of_a_read_only_folder() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = sample();
        let root = dir.path();
        fs::set_permissions(root.join("album/deep"), fs::Permissions::from_mode(0o555)).unwrap();
        let (outcome, ..) = run_with(job(&dir, false, &["album"], "target"), None).await;
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(root.join("target/album/deep/b.jpg").exists());
        let mode = fs::metadata(root.join("target/album/deep")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o555);
        for path in [root.join("album/deep"), root.join("target/album/deep")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[tokio::test]
    async fn move_takes_the_items_away() {
        let dir = sample();
        let root = dir.path();
        let (outcome, ..) = run_with(job(&dir, true, &["album", "note.txt"], "target"), None).await;
        assert_eq!(names(&outcome), ["album", "note.txt"]);
        assert_eq!(outcome.left, [root.join("album"), root.join("note.txt")]);
        assert!(!root.join("album").exists());
        assert!(!root.join("note.txt").exists());
        assert_eq!(fs::read(root.join("target/album/deep/b.jpg")).unwrap(), b"bb");
        assert_eq!(fs::read(root.join("target/note.txt")).unwrap(), b"note");
    }

    #[tokio::test]
    async fn move_between_devices_copies_then_deletes() {
        let dir = sample();
        let root = dir.path();
        let mut job = job(&dir, true, &["album", "note.txt"], "target");
        job.same_device = false;
        let (outcome, _, progress) = run_with(job, None).await;
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(outcome.left.len(), 2);
        assert_eq!(progress, (10, 10));
        assert!(!root.join("album").exists());
        assert!(!root.join("note.txt").exists());
        assert_eq!(fs::read(root.join("target/album/a.jpg")).unwrap(), b"aaaa");
        assert_eq!(fs::read_link(root.join("target/album/link")).unwrap(), Path::new("a.jpg"));
    }

    #[tokio::test]
    async fn conflict_skipped() {
        let dir = sample();
        let root = dir.path();
        fs::write(root.join("target/note.txt"), b"other").unwrap();
        for decision in [SKIP, None] {
            let (outcome, asked, progress) = run_with(job(&dir, true, &["note.txt", "album"], "target"), decision).await;
            assert_eq!(asked, ["note.txt"]);
            assert_eq!(names(&outcome), ["album"]);
            assert_eq!(progress, (10, 10), "skipped bytes count as handled");
            assert_eq!(fs::read(root.join("target/note.txt")).unwrap(), b"other");
            assert_eq!(fs::read(root.join("note.txt")).unwrap(), b"note");
            // back for the second round
            fs::rename(root.join("target/album"), root.join("album")).unwrap();
        }
    }

    #[tokio::test]
    async fn conflict_overwritten() {
        let dir = sample();
        let root = dir.path();
        fs::write(root.join("target/note.txt"), b"other").unwrap();
        let (outcome, asked, _) = run_with(job(&dir, false, &["note.txt"], "target"), OVERWRITE).await;
        assert_eq!(asked, ["note.txt"]);
        assert_eq!(names(&outcome), ["note.txt"]);
        assert_eq!(fs::read(root.join("target/note.txt")).unwrap(), b"note");
    }

    #[tokio::test]
    async fn conflict_kept_under_a_numbered_name() {
        let dir = sample();
        let root = dir.path();
        fs::write(root.join("target/note.txt"), b"other").unwrap();
        fs::write(root.join("target/note (2).txt"), b"second").unwrap();
        fs::create_dir(root.join("target/album")).unwrap();
        let (outcome, asked, _) = run_with(job(&dir, false, &["note.txt", "album"], "target"), RENAME).await;
        // the answer was for all the conflicts
        assert_eq!(asked, ["note.txt"]);
        assert_eq!(names(&outcome), ["note (3).txt", "album (2)"]);
        assert_eq!(fs::read(root.join("target/note.txt")).unwrap(), b"other");
        assert_eq!(fs::read(root.join("target/note (3).txt")).unwrap(), b"note");
        assert!(root.join("target/album (2)/deep/b.jpg").exists());
    }

    #[tokio::test]
    async fn folders_are_merged_when_overwritten() {
        let dir = sample();
        let root = dir.path();
        fs::create_dir(root.join("target/album")).unwrap();
        fs::write(root.join("target/album/a.jpg"), b"old").unwrap();
        fs::write(root.join("target/album/kept.jpg"), b"kept").unwrap();
        let (outcome, asked, _) = run_with(job(&dir, true, &["album"], "target"), OVERWRITE).await;
        assert_eq!(asked, ["album", "a.jpg"]);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(fs::read(root.join("target/album/a.jpg")).unwrap(), b"aaaa");
        assert_eq!(fs::read(root.join("target/album/kept.jpg")).unwrap(), b"kept");
        assert_eq!(fs::read(root.join("target/album/deep/b.jpg")).unwrap(), b"bb");
        // everything went: the source is gone
        assert_eq!(outcome.left, [root.join("album")]);
        assert!(!root.join("album").exists());
    }

    #[tokio::test]
    async fn move_with_a_skip_inside_keeps_the_rest_at_the_source() {
        let dir = sample();
        let root = dir.path();
        fs::create_dir(root.join("target/album")).unwrap();
        fs::write(root.join("target/album/a.jpg"), b"old").unwrap();
        let asked = Mutex::new(0);
        let cancel = AtomicBool::new(false);
        // merge the folder, skip the file
        let outcome = run(job(&dir, true, &["album"], "target"), &cancel, |event| {
            if let Event::Conflict { reply, .. } = event {
                let mut asked = asked.lock().unwrap();
                *asked += 1;
                let answer = if *asked == 1 { Answer::Overwrite } else { Answer::Skip };
                let _ = reply.send(Decision { answer, all: false });
            }
        })
        .await;
        assert!(outcome.left.is_empty());
        assert_eq!(fs::read(root.join("target/album/a.jpg")).unwrap(), b"old");
        assert_eq!(fs::read(root.join("album/a.jpg")).unwrap(), b"aaaa");
        assert!(root.join("target/album/deep/b.jpg").exists());
        assert!(!root.join("album/deep").exists());
    }

    #[tokio::test]
    async fn same_place_and_inside_itself() {
        let dir = sample();
        let root = dir.path();
        // a copy where the item already is makes a duplicate, a move does nothing
        let (outcome, asked, _) = run_with(job(&dir, false, &["note.txt"], ""), None).await;
        assert!(asked.is_empty());
        assert_eq!(names(&outcome), ["note (2).txt"]);
        let (outcome, ..) = run_with(job(&dir, true, &["note.txt"], ""), None).await;
        assert!(outcome.arrived.is_empty() && outcome.left.is_empty() && outcome.errors.is_empty());
        assert!(root.join("note.txt").exists());

        for folder in ["album", "album/deep"] {
            let (outcome, ..) = run_with(job(&dir, false, &["album", "note.txt"], folder), None).await;
            assert_eq!(outcome.errors.len(), 1);
            // the other items still go
            assert_eq!(names(&outcome), ["note.txt"]);
        }
        assert!(!root.join("album/album").exists());
    }

    #[tokio::test]
    async fn failures_are_reported_per_item() {
        let dir = sample();
        let (outcome, ..) = run_with(job(&dir, false, &["missing", "note.txt"], "target"), None).await;
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(names(&outcome), ["note.txt"]);
        let (outcome, ..) = run_with(job(&dir, false, &["note.txt"], "nowhere"), None).await;
        assert_eq!(outcome.errors.len(), 1);
        assert!(outcome.arrived.is_empty());
    }

    #[tokio::test]
    async fn cancelled_transfer_leaves_no_partial_file() {
        let dir = sample();
        let root = dir.path();
        // bigger than a piece, so the copy is interrupted between two of them
        fs::write(root.join("big.bin"), vec![7u8; crate::fs::PIECE * 3]).unwrap();
        let cancel = AtomicBool::new(false);
        let outcome = run(job(&dir, false, &["note.txt", "big.bin", "album"], "target"), &cancel, |event| {
            if let Event::Progress { name, .. } = event {
                if name == "big.bin" {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        })
        .await;
        assert!(outcome.cancelled);
        assert_eq!(names(&outcome), ["note.txt"]);
        assert!(!root.join("target/big.bin").exists());
        assert!(!root.join("target/album").exists());
        assert!(root.join("big.bin").exists());
    }
}
