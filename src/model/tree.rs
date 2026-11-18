//! the list model shown by the main page: devices, with their folders and files below them

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::future::Future;
use std::path::{Path, PathBuf};

use qmetaobject::{
    qt_base_class, qt_method, qt_property, qt_signal, QAbstractListModel, QByteArray, QModelIndex,
    QObject, QPointer, QString, QVariant, USER_ROLE,
};

use super::node::{Row, RowKind, Sort, Step, Tree};
use super::selection::Selection;
use crate::config::Config;
use crate::devices::{self, Device, DeviceId, DeviceKind, Remote, State as DeviceState, LOCAL};
use crate::discovery::{self, Found};
use crate::fs::ftp;
use crate::fs::sftp::{self, ConnectError};
use crate::fs::{mode_to_string, Entry, Fs, FsError};
use crate::opener;
use crate::runtime;
use crate::staging;
use crate::transfer::conflict::{Answer, Decision};
use crate::transfer::job::{self, Event, Job, Outcome};

/// data exposed for each row, in the order of their Qt role numbers
const ROLES: [&str; 19] = [
    "name",
    "depth",
    "kind",
    "expanded",
    "hasChildren",
    "selectable",
    "selected",
    "category",
    "path",
    "size",
    "date",
    "owner",
    "group",
    "permissions",
    "mode",
    "link",
    "ratio",
    "remote",
    "busy",
];

/// a role value, before its conversion for Qt
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i32),
    Number(f64),
    Bool(bool),
}

impl From<Value> for QVariant {
    fn from(value: Value) -> QVariant {
        match value {
            Value::Text(text) => QString::from(text).into(),
            Value::Int(number) => number.into(),
            Value::Number(number) => number.into(),
            Value::Bool(flag) => flag.into(),
        }
    }
}

/// everything that is not a Qt declaration
struct State {
    tree: Tree,
    devices: Vec<Device>,
    selection: Selection,
    /// items waiting for the destination of their copy or move
    pending: Option<Pending>,
    /// remote devices a connection is being opened to
    connecting: HashSet<DeviceId>,
    /// a look at the network is running
    discovering: bool,
    /// what the network shows, instead of looking at it (unit tests)
    inline_found: Option<Vec<Found>>,
    /// copies and moves waiting for the running one to end
    queue: VecDeque<Transfer>,
    running: Option<Running>,
    /// answer to the conflicts of transfers run inline (unit tests), none to skip
    inline_decision: Option<Decision>,
    /// second line with the details of each item
    show_details: bool,
    config: Config,
    /// home folder of the user, where files are made visible to the applications opening them
    home: PathBuf,
    /// run filesystem operations before returning instead of in the background (unit tests)
    inline: bool,
}

impl Default for State {
    fn default() -> Self {
        let config = Config::load(&crate::config::default_path());
        let mut local = Device::local();
        // back where the user was, if that still exists
        if let Some(folder) = config.get(LOCAL_SECTION, "folder").map(PathBuf::from) {
            if folder.is_absolute() && folder.is_dir() {
                local.home = folder;
            }
        }
        let mut devices = vec![local];
        devices.extend(Device::load(&config));
        let mut state = State::new(devices);
        state.tree.set_show_hidden(config.flag(SETTINGS, "hidden").unwrap_or(false));
        state.tree.set_sort(config.get(SETTINGS, "sort").and_then(parse_sort).unwrap_or_default());
        state.show_details = config.flag(SETTINGS, "details").unwrap_or(false);
        state.config = config;
        state
    }
}

/// configuration sections
const SETTINGS: &str = "settings";
const LOCAL_SECTION: &str = "local";

fn parse_sort(name: &str) -> Option<Sort> {
    match name {
        "name" => Some(Sort::Name),
        "date" => Some(Sort::Date),
        "size" => Some(Sort::Size),
        _ => None,
    }
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Name => "name",
        Sort::Date => "date",
        Sort::Size => "size",
    }
}

fn flag(value: bool) -> &'static str {
    match value {
        true => "true",
        false => "false",
    }
}

impl State {
    fn new(devices: Vec<Device>) -> Self {
        let mut tree = Tree::default();
        for device in &devices {
            for step in tree.plan_add_device(device.id, &device.name, &device.home) {
                tree.apply(step);
            }
        }
        State {
            tree,
            devices,
            selection: Selection::default(),
            pending: None,
            connecting: HashSet::new(),
            discovering: false,
            inline_found: None,
            queue: VecDeque::new(),
            running: None,
            inline_decision: None,
            show_details: false,
            config: Config::default(),
            home: Device::local().home,
            inline: false,
        }
    }

    fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.iter().find(|device| device.id == id)
    }

    fn device_mut(&mut self, id: DeviceId) -> Option<&mut Device> {
        self.devices.iter_mut().find(|device| device.id == id)
    }

    /// row of a selection item, if it is displayed
    fn shown(&self, device: DeviceId, path: &Path) -> Option<usize> {
        if path == device_item() {
            self.tree.find_device(device)
        } else {
            // the current folder of a device is its device row, not an item of its own
            self.tree.find(device, path).filter(|&row| self.tree.rows()[row].is_item())
        }
    }

    /// forget the selected items that are not displayed. returns whether the selection changed
    fn prune_selection(&mut self) -> bool {
        let mut selection = std::mem::take(&mut self.selection);
        let changed = selection.retain(|device, path| self.shown(device, path).is_some());
        self.selection = selection;
        changed
    }
}

struct Pending {
    moving: bool,
    items: Vec<(DeviceId, PathBuf)>,
}

/// a copy or move from one device into a folder of another, or of the same
struct Transfer {
    from: DeviceId,
    to: DeviceId,
    /// open what arrives with another application, instead of showing it in the tree
    open: bool,
    job: Job,
}

/// the transfer being done
struct Running {
    cancel: Arc<AtomicBool>,
    /// where to answer the conflict the transfer is waiting on
    reply: Option<tokio::sync::oneshot::Sender<Decision>>,
    moving: bool,
    done: u64,
    total: u64,
    /// item being transferred
    name: String,
}

/// path standing for a device itself in the selection: its current folder changes
fn device_item() -> &'static Path {
    Path::new("")
}

/// how a row is identified in the selection
fn selection_path(row: &Row) -> &Path {
    match row.kind {
        RowKind::Device => device_item(),
        _ => &row.path,
    }
}

fn selectable(row: &Row) -> bool {
    row.is_item() || row.kind == RowKind::Device
}

fn kind_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Device => "device",
        RowKind::Folder => "folder",
        RowKind::File => "file",
        RowKind::Loading => "loading",
        RowKind::Error => "error",
    }
}

#[derive(QObject, Default)]
#[allow(non_snake_case)] // names seen from QML
pub struct TreeModel {
    base: qt_base_class!(trait QAbstractListModel),

    /// whether items whose name starts with a dot are listed
    #[allow(dead_code)] // storage unused, the value lives in the tree
    showHidden: qt_property!(bool; READ show_hidden WRITE set_show_hidden NOTIFY showHiddenChanged),
    showHiddenChanged: qt_signal!(),

    /// what orders the items of a folder: "name", "date" or "size"
    #[allow(dead_code)] // storage unused, the value lives in the tree
    sort: qt_property!(QString; READ sort_name WRITE set_sort_name NOTIFY sortChanged),
    sortChanged: qt_signal!(),

    /// number of selected items
    selectionCount: qt_property!(i32; READ selection_count NOTIFY selectionChanged),
    /// number of selected files and folders, the items that can be deleted
    selectedItemCount: qt_property!(i32; READ selected_item_count NOTIFY selectionChanged),
    /// row of the selected item when there is exactly one, -1 otherwise
    selectedRow: qt_property!(i32; READ selected_row NOTIFY selectionChanged),
    /// "device", "folder" or "file" when exactly one item is selected, empty otherwise
    selectedKind: qt_property!(QString; READ selected_kind NOTIFY selectionChanged),
    selectionChanged: qt_signal!(),

    /// number of items waiting for the destination of their copy or move
    pendingCount: qt_property!(i32; READ pending_count NOTIFY pendingChanged),
    /// whether the waiting items will be moved rather than copied
    pendingMove: qt_property!(bool; READ pending_move NOTIFY pendingChanged),
    pendingChanged: qt_signal!(),
    /// whether a copy or move is being done
    busy: qt_property!(bool; READ busy NOTIFY transferChanged),
    /// part of the running transfer that is done, from 0 to 1
    transferProgress: qt_property!(f64; READ transfer_progress NOTIFY transferChanged),
    /// name of the item being transferred
    transferName: qt_property!(QString; READ transfer_name NOTIFY transferChanged),
    /// whether the running transfer is a move rather than a copy
    transferMoving: qt_property!(bool; READ transfer_moving NOTIFY transferChanged),
    /// number of transfers waiting for the running one
    transferWaiting: qt_property!(i32; READ transfer_waiting NOTIFY transferChanged),
    transferChanged: qt_signal!(),
    /// the running transfer found an item of this name where it goes, and waits for
    /// `resolveConflict`
    conflict: qt_signal!(name: QString),

    /// second line with permissions, owner, date and size under each item
    #[allow(dead_code)] // storage unused, the value lives in the state
    showDetails: qt_property!(bool; READ show_details WRITE set_show_details NOTIFY showDetailsChanged),
    showDetailsChanged: qt_signal!(),

    /// the device at `row` needs a user and password to connect: answered by `login`
    loginRequired: qt_signal!(row: i32),
    /// the device at `row` presents a key that is not trusted yet, or `changed` since it
    /// was: answered by `trustHost`, or not at all
    hostKeyUnknown: qt_signal!(row: i32, fingerprint: QString, changed: bool),

    /// an operation failed, with a message for the user
    failed: qt_signal!(message: QString),
    /// this local path should be opened with the application the system associates to it
    openRequested: qt_signal!(path: QString),
    /// this row should be brought into view, at the top of the view if `top`
    revealed: qt_signal!(row: i32, top: bool),

    /// expand or collapse a device or folder
    toggle: qt_method!(fn(&mut self, row: i32)),
    /// open the file at `row` with another application: answers with `openRequested`
    open: qt_method!(fn(&mut self, row: i32)),
    /// list an expanded device or folder again
    refresh: qt_method!(fn(&mut self, row: i32)),
    /// rename a file or folder inside its folder
    rename: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// create a folder in the folder at `row`, in the current folder of the device at `row`,
    /// or next to the file at `row`
    createFolder: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// create an empty file, same destination rules as `createFolder`
    createFile: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// set the permission bits of a file or folder
    chmod: qt_method!(fn(&mut self, row: i32, mode: i32)),
    /// name of the item at `row`, empty for an unknown row
    nameAt: qt_method!(fn(&self, row: i32) -> QString),
    /// permission bits of the item at `row`
    modeAt: qt_method!(fn(&self, row: i32) -> i32),
    /// make the folder at `row` the current folder of its device
    enter: qt_method!(fn(&mut self, row: i32)),
    /// set the current folder of the device `row` belongs to
    setCurrentFolder: qt_method!(fn(&mut self, row: i32, path: QString)),
    /// keep the selected files and folders aside to copy or move them, and clear the selection
    startTransfer: qt_method!(fn(&mut self, moving: bool)),
    /// forget the items kept aside
    cancelTransfer: qt_method!(fn(&mut self)),
    /// copy or move the items kept aside into the folder at `row`, or into the current folder
    /// of the device at `row`
    transferTo: qt_method!(fn(&mut self, row: i32)),
    /// answer the conflict the running transfer waits on with "overwrite", "skip" or "rename",
    /// for this item or for `all` the next ones too
    resolveConflict: qt_method!(fn(&mut self, answer: QString, all: bool)),
    /// stop the running transfer and forget the waiting ones
    cancelRunning: qt_method!(fn(&mut self)),
    /// remember a new remote device reached through ssh, and give its identifier, -1 if refused.
    /// an empty password is asked when connecting
    addDevice: qt_method!(
        fn(&mut self, name: QString, host: QString, port: i32, user: QString, password: QString, save: bool) -> i32
    ),
    /// reach the remote device at `row` through "sftp" or "ftp"
    setProtocol: qt_method!(fn(&mut self, row: i32, protocol: QString)),
    /// change how the remote device at `row` is reached, and its current folder
    #[allow(clippy::too_many_arguments)]
    updateDevice: qt_method!(
        fn(&mut self, row: i32, name: QString, host: QString, port: i32, user: QString, password: QString, save: bool, folder: QString)
    ),
    /// forget the remote device at `row`
    removeDevice: qt_method!(fn(&mut self, row: i32)),
    /// "name", "protocol", "host", "port", "user", "password", "fingerprint" or "folder" of the
    /// device at `row`
    deviceText: qt_method!(fn(&self, row: i32, field: QString) -> QString),
    /// identifier of the device the row belongs to, which stays when rows move. -1 for an unknown row
    deviceIdAt: qt_method!(fn(&self, row: i32) -> i32),
    /// row of the device with this identifier, -1 when it is gone
    deviceRow: qt_method!(fn(&self, id: i32) -> i32),
    deviceSavesPassword: qt_method!(fn(&self, row: i32) -> bool),
    deviceConnected: qt_method!(fn(&self, row: i32) -> bool),
    /// connect the device at `row` with this user and password, kept in the configuration if `save`
    login: qt_method!(fn(&mut self, row: i32, user: QString, password: QString, save: bool)),
    /// trust the key with this fingerprint for the device at `row`, and connect
    trustHost: qt_method!(fn(&mut self, row: i32, fingerprint: QString)),
    /// close the connection of the device at `row`
    disconnect: qt_method!(fn(&mut self, row: i32)),
    /// check which remote devices answer on the network
    probe: qt_method!(fn(&mut self)),
    /// look for devices of the local network that are not listed yet
    discover: qt_method!(fn(&mut self)),
    /// select or unselect a device, file or folder
    toggleSelected: qt_method!(fn(&mut self, row: i32)),
    clearSelection: qt_method!(fn(&mut self)),
    /// delete the selected files and folders, folders with their content
    removeSelected: qt_method!(fn(&mut self)),

    state: State,
}

impl QAbstractListModel for TreeModel {
    fn row_count(&self) -> i32 {
        self.state.tree.rows().len() as i32
    }

    fn data(&self, index: QModelIndex, role: i32) -> QVariant {
        let name = usize::try_from(role - USER_ROLE).ok().and_then(|role| ROLES.get(role));
        match name.and_then(|name| self.value(index.row(), name)) {
            Some(value) => value.into(),
            None => QVariant::default(),
        }
    }

    fn role_names(&self) -> HashMap<i32, QByteArray> {
        ROLES
            .iter()
            .enumerate()
            .map(|(index, name)| (USER_ROLE + index as i32, QByteArray::from(*name)))
            .collect()
    }
}

impl TreeModel {
    /// value of a role for a row, `None` for an unknown row or role
    pub fn value(&self, row: i32, role: &str) -> Option<Value> {
        let index = usize::try_from(row).ok()?;
        let row = self.row(row)?;
        // the size ring is only for the local device
        let local = self.state.device(row.device).is_some_and(|device| device.kind == DeviceKind::Local);
        let entry = row.entry.as_ref();
        let text = |text: &str| Value::Text(text.to_owned());
        Some(match role {
            "name" => text(&row.name),
            "depth" => Value::Int(row.depth as i32),
            "kind" => text(kind_name(row.kind)),
            "expanded" => Value::Bool(row.expanded),
            "hasChildren" => Value::Bool(row.expandable()),
            "selectable" => Value::Bool(selectable(row)),
            "selected" => Value::Bool(
                selectable(row) && self.state.selection.contains(row.device, selection_path(row)),
            ),
            "category" => text(
                self.state.device(row.device).map_or("", |device| device.state.category()),
            ),
            "path" => text(&row.path.to_string_lossy()),
            "size" => Value::Number(entry.map_or(0.0, |entry| entry.size as f64)),
            // milliseconds, as expected by javascript dates
            "date" => Value::Number(entry.map_or(0.0, |entry| entry.mtime as f64 * 1000.0)),
            "owner" => text(entry.map_or("", |entry| &entry.owner)),
            "group" => text(entry.map_or("", |entry| &entry.group)),
            "permissions" => Value::Text(entry.map_or(String::new(), |entry| mode_to_string(entry.mode))),
            "mode" => Value::Int(entry.map_or(0, |entry| entry.mode as i32)),
            "link" => Value::Text(
                entry
                    .and_then(|entry| entry.link_target.as_ref())
                    .map_or(String::new(), |target| target.to_string_lossy().into_owned()),
            ),
            // share of the files of its folder, negative when it has none
            "ratio" => Value::Number(self.state.tree.ratio(index).filter(|_| local).unwrap_or(-1.0)),
            "remote" => Value::Bool(row.kind == RowKind::Device && !local),
            // a connection is being opened
            "busy" => Value::Bool(row.kind == RowKind::Device && self.state.connecting.contains(&row.device)),
            _ => return None,
        })
    }

    fn row(&self, row: i32) -> Option<&Row> {
        self.state.tree.row(usize::try_from(row).ok()?)
    }

    /// apply planned changes, announcing each of them to the views
    fn run(&mut self, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Insert { at, ref rows } => {
                    self.begin_insert_rows(at as i32, (at + rows.len()) as i32 - 1);
                    self.state.tree.apply(step);
                    self.end_insert_rows();
                }
                Step::Remove { first, count } => {
                    self.begin_remove_rows(first as i32, (first + count) as i32 - 1);
                    self.state.tree.apply(step);
                    self.end_remove_rows();
                }
                Step::Update { row, .. } => {
                    self.state.tree.apply(step);
                    self.rows_changed(row, row);
                }
            }
        }
    }

    fn rows_changed(&mut self, first: usize, last: usize) {
        let first = self.row_index(first as i32);
        let last = self.row_index(last as i32);
        self.data_changed(first, last);
    }

    /// run `task` on the worker threads, then `done` on the Qt thread with its result
    fn spawn<T, F, D>(&mut self, task: F, done: D)
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
        D: FnOnce(&mut Self, T) + 'static,
    {
        if self.state.inline {
            let result = runtime::runtime().block_on(task);
            done(self, result);
        } else {
            let this = QPointer::from(&*self);
            runtime::spawn_to_qt(task, move |result| {
                // the model may have been destroyed meanwhile
                if let Some(this) = this.as_pinned() {
                    done(&mut this.borrow_mut(), result);
                }
            });
        }
    }

    fn fs(&self, device: DeviceId) -> Option<Fs> {
        self.state.device(device).and_then(|device| device.fs.clone())
    }

    fn report(&mut self, error: impl ToString) {
        let message = error.to_string();
        // also in the log of the app, where it stays after the notice is gone
        eprintln!("sailtree: {message}");
        self.failed(QString::from(message));
    }

    /// one message for the failures of a grouped action
    fn report_all(&mut self, errors: &[String]) {
        if let Some(first) = errors.first() {
            let message = match errors.len() {
                1 => first.clone(),
                count => format!("{first} (and {} more)", count - 1),
            };
            self.report(message);
        }
    }

    /// the files of a folder changed: the ratio of each of them depends on the others
    fn measure(&mut self, device: DeviceId, folder: &Path) {
        if let Some(row) = self.state.tree.find(device, folder) {
            let last = row + self.state.tree.descendants(row);
            self.rows_changed(row, last);
        }
    }

    fn toggle(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if !current.expandable() {
            return;
        }
        let (device, path) = (current.device, current.path.clone());
        // a device that is not connected is first connected, which expands it
        if current.kind == RowKind::Device && self.fs(device).is_none() {
            self.connect(device);
            return;
        }
        if current.expanded {
            let steps = self.state.tree.plan_collapse(row as usize);
            self.run(steps);
            // what is not displayed anymore is not kept selected
            if self.state.selection.removed_inside(device, &path) {
                self.selectionChanged();
            }
            return;
        }
        self.expand(row as usize);
    }

    fn expand(&mut self, row: usize) {
        let Some(current) = self.state.tree.row(row) else { return };
        let (device, path) = (current.device, current.path.clone());
        let steps = self.state.tree.plan_expand(row);
        if steps.is_empty() {
            return;
        }
        self.run(steps);
        self.list(device, path);
    }

    fn open(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if current.kind != RowKind::File {
            return;
        }
        let (device, path) = (current.device, current.path.clone());
        let empty = current.entry.as_ref().is_some_and(|entry| entry.size == 0);
        if device == LOCAL {
            self.open_local(path, empty);
            return;
        }
        // a remote file is first fetched where applications can read it
        let (Some(from), Some(to)) = (self.fs(device), self.fs(LOCAL)) else { return };
        let folder = match staging::folder(&self.state.home) {
            Ok(folder) => folder,
            Err(error) => return self.report(format!("cannot fetch the file: {error}")),
        };
        let job = Job { moving: false, from, sources: vec![path], to, folder, same_device: false };
        self.state.queue.push_back(Transfer { from: device, to: LOCAL, open: true, job });
        self.transferChanged();
        self.start_transfer();
    }

    /// open a file of this device with the application the system associates to it
    fn open_local(&mut self, path: PathBuf, empty: bool) {
        let home = self.state.home.clone();
        self.spawn(
            async move {
                // the system has no application for a file without content
                let work = move || match empty {
                    true => opener::open_empty(&home, &path).map(|()| None),
                    false => Ok(Some(staging::stage(&home, &path))),
                };
                tokio::task::spawn_blocking(work).await.unwrap_or_else(|error| Err(error.to_string()))
            },
            |this, result| match result {
                Ok(Some(path)) => this.openRequested(QString::from(path.to_string_lossy().as_ref())),
                Ok(None) => {}
                Err(message) => this.report(message),
            },
        );
    }

    fn device_changed(&mut self, device: DeviceId) {
        if let Some(row) = self.state.tree.find_device(device) {
            self.rows_changed(row, row);
        }
    }

    /// move a device to its place among the others: by category, last of its category
    fn place(&mut self, device: DeviceId) {
        let Some(state) = self.state.device(device).map(|device| device.state) else { return };
        let before = self
            .state
            .tree
            .rows()
            .iter()
            .filter(|row| row.kind == RowKind::Device && row.device != device)
            .map(|row| row.device)
            .find(|other| self.state.device(*other).is_some_and(|other| other.state > state));
        let steps = self.state.tree.plan_move_device(device, before);
        self.run(steps);
        self.device_changed(device);
    }

    /// open a connection to a remote device, asking for what is missing
    fn connect(&mut self, id: DeviceId) {
        let Some(device) = self.state.device(id) else { return };
        let Some(remote) = device.remote.clone() else { return };
        let kind = device.kind;
        if device.fs.is_some() || self.state.connecting.contains(&id) {
            return;
        }
        let Some(row) = self.state.tree.find_device(id) else { return };
        let Some(password) = remote.password.clone() else {
            self.loginRequired(row as i32);
            return;
        };
        self.state.connecting.insert(id);
        self.device_changed(id);
        self.spawn(
            async move {
                let work = move || match kind {
                    DeviceKind::Ftp => ftp::connect(&remote.host, remote.port, &remote.user, &password).map(Fs::Ftp),
                    _ => sftp::connect(&remote.host, remote.port, &remote.user, &password, remote.fingerprint.as_deref())
                        .map(Fs::Sftp),
                };
                tokio::task::spawn_blocking(work)
                    .await
                    .unwrap_or_else(|error| Err(ConnectError::Other(error.to_string())))
            },
            move |this, result| this.connected(id, result),
        );
    }

    fn connected(&mut self, id: DeviceId, result: Result<Fs, ConnectError>) {
        self.state.connecting.remove(&id);
        self.device_changed(id);
        let Some(row) = self.state.tree.find_device(id) else { return };
        let Some(index) = self.state.devices.iter().position(|device| device.id == id) else { return };
        match result {
            Ok(session) => {
                let device = &mut self.state.devices[index];
                if device.home.as_os_str().is_empty() {
                    device.home = session.home();
                }
                device.fs = Some(session);
                device.remembered = true;
                device.state = DeviceState::Connected;
                device.used = devices::now();
                device.save(&mut self.state.config);
                let folder = device.home.clone();
                let steps = self.state.tree.plan_set_current(id, &folder);
                self.run(steps);
                self.place(id);
                self.forget_oldest();
                if let Some(row) = self.state.tree.find_device(id) {
                    self.expand(row);
                    self.revealed(row as i32, false);
                }
            }
            Err(ConnectError::HostKey(fingerprint)) => {
                let changed = self.state.devices[index].remote.as_ref().is_some_and(|remote| remote.fingerprint.is_some());
                self.hostKeyUnknown(row as i32, QString::from(fingerprint), changed);
            }
            Err(ConnectError::Login) => {
                // ask again rather than try the same password forever
                if let Some(remote) = &mut self.state.devices[index].remote {
                    remote.password = None;
                }
                self.report(ConnectError::Login);
                self.loginRequired(row as i32);
            }
            Err(error) => {
                self.state.devices[index].state = DeviceState::Offline;
                self.place(id);
                self.report(error);
            }
        }
    }

    /// keep the number of remembered devices bounded, dropping the least recently connected
    fn forget_oldest(&mut self) {
        loop {
            let remotes = self.state.devices.iter().filter(|device| device.remote.is_some() && device.remembered);
            if remotes.clone().count() <= devices::REMEMBERED {
                return;
            }
            let oldest = remotes.filter(|device| device.fs.is_none()).min_by_key(|device| device.used).map(|device| device.id);
            match oldest {
                Some(id) => self.remove_device(id),
                None => return,
            }
        }
    }

    fn next_id(&self) -> DeviceId {
        self.state.devices.iter().map(|device| device.id).max().unwrap_or(LOCAL) + 1
    }

    fn discover(&mut self) {
        if self.state.discovering {
            return;
        }
        let known: Vec<String> = self
            .state
            .devices
            .iter()
            .filter(|device| device.remembered)
            .filter_map(|device| device.remote.as_ref().map(|remote| remote.host.clone()))
            .collect();
        if self.state.inline {
            if let Some(found) = self.state.inline_found.clone() {
                self.discovered(discovery::unknown(found, &known, &HashSet::new()));
            }
            return;
        }
        self.state.discovering = true;
        self.spawn(
            async move { tokio::task::spawn_blocking(move || discovery::discover(&known)).await.unwrap_or_default() },
            |this, found| {
                this.state.discovering = false;
                this.discovered(found);
            },
        );
    }

    /// list the devices seen on the network, and drop those that were and are not seen anymore
    fn discovered(&mut self, found: Vec<Found>) {
        let seen: HashSet<String> = found.iter().map(|found| found.address.to_string()).collect();
        let host = |device: &Device| device.remote.as_ref().map(|remote| remote.host.clone());
        let gone: Vec<DeviceId> = self
            .state
            .devices
            .iter()
            // those in use stay
            .filter(|device| !device.remembered && device.fs.is_none() && !self.state.connecting.contains(&device.id))
            .filter(|device| host(device).is_some_and(|host| !seen.contains(&host)))
            .map(|device| device.id)
            .collect();
        for id in gone {
            self.remove_device(id);
        }
        for found in found {
            let address = found.address.to_string();
            if self.state.devices.iter().any(|device| host(device).as_deref() == Some(address.as_str())) {
                continue;
            }
            let id = self.next_id();
            let device = Device::discovered(id, &found.name, &address, found.port, found.kind);
            let steps = self.state.tree.plan_add_device(id, &device.name, &device.home);
            self.state.devices.push(device);
            self.run(steps);
            self.place(id);
        }
    }

    fn remote_at(&self, row: i32) -> Option<&Device> {
        let current = self.row(row)?;
        self.state.device(current.device).filter(|device| device.remote.is_some())
    }

    #[allow(non_snake_case)]
    fn addDevice(&mut self, name: QString, host: QString, port: i32, user: QString, password: QString, save: bool) -> i32 {
        let host = host.to_string().trim().to_owned();
        let Ok(port) = u16::try_from(port) else { return -1 };
        if host.is_empty() || port == 0 {
            return -1;
        }
        let password = Some(password.to_string()).filter(|password| !password.is_empty());
        let remote = Remote { host, port, user: user.to_string(), password, save_password: save, fingerprint: None };
        let id = self.next_id();
        let device = Device::remote(id, &name.to_string(), remote);
        device.save(&mut self.state.config);
        let steps = self.state.tree.plan_add_device(id, &device.name, &device.home);
        self.state.devices.push(device);
        self.run(steps);
        self.place(id);
        self.probe();
        id as i32
    }

    #[allow(non_snake_case)]
    fn setProtocol(&mut self, row: i32, protocol: QString) {
        let Some(id) = self.remote_at(row).map(|device| device.id) else { return };
        let Some(kind) = DeviceKind::from_protocol(&protocol.to_string()) else { return };
        let Some(device) = self.state.devices.iter_mut().find(|device| device.id == id) else { return };
        if device.kind == kind {
            return;
        }
        device.kind = kind;
        device.remembered = true;
        device.save(&mut self.state.config);
        // what was open spoke the other protocol
        self.close(id);
        self.device_changed(id);
    }

    #[allow(non_snake_case, clippy::too_many_arguments)]
    fn updateDevice(&mut self, row: i32, name: QString, host: QString, port: i32, user: QString, password: QString, save: bool, folder: QString) {
        let Some(id) = self.remote_at(row).map(|device| device.id) else { return };
        let host = host.to_string().trim().to_owned();
        let Ok(port) = u16::try_from(port) else { return };
        if host.is_empty() || port == 0 {
            return;
        }
        let Some(device) = self.state.devices.iter_mut().find(|device| device.id == id) else { return };
        let Some(remote) = &mut device.remote else { return };
        let password = Some(password.to_string()).filter(|password| !password.is_empty());
        let moved = remote.host != host || remote.port != port;
        let reconnect = moved || remote.user != user.to_string() || remote.password != password;
        if moved {
            // another machine: its key has to be trusted again
            remote.fingerprint = None;
        }
        (remote.host, remote.port, remote.user) = (host, port, user.to_string());
        (remote.password, remote.save_password) = (password, save);
        // settings chosen by the user are worth keeping
        device.remembered = true;
        let name = name.to_string();
        device.name = if name.trim().is_empty() { remote.host.clone() } else { name.trim().to_owned() };
        let folder = PathBuf::from(folder.to_string());
        let entered = folder.is_absolute() && folder != device.home;
        if entered {
            device.home = folder.clone();
        }
        let (name, connected) = (device.name.clone(), device.fs.is_some());
        device.save(&mut self.state.config);
        let steps = self.state.tree.plan_name_device(id, &name);
        self.run(steps);
        if connected && reconnect {
            self.close(id);
        } else if connected && entered {
            self.set_current(id, folder);
        } else if entered {
            let steps = self.state.tree.plan_set_current(id, &folder);
            self.run(steps);
        }
    }

    #[allow(non_snake_case)]
    fn removeDevice(&mut self, row: i32) {
        if let Some(id) = self.remote_at(row).map(|device| device.id) {
            self.remove_device(id);
        }
    }

    fn remove_device(&mut self, id: DeviceId) {
        let Some(index) = self.state.devices.iter().position(|device| device.id == id && device.remote.is_some())
        else {
            return;
        };
        let steps = self.state.tree.plan_remove_device(id);
        self.run(steps);
        let device = self.state.devices.remove(index);
        device.forget(&mut self.state.config);
        self.state.connecting.remove(&id);
        self.state.prune_selection();
        self.selectionChanged();
    }

    #[allow(non_snake_case)]
    fn deviceText(&self, row: i32, field: QString) -> QString {
        let Some(device) = self.row(row).and_then(|row| self.state.device(row.device)) else {
            return QString::default();
        };
        let remote = device.remote.as_ref();
        let text = match field.to_string().as_str() {
            "name" => device.name.clone(),
            "protocol" => device.kind.protocol().to_owned(),
            "folder" => device.home.to_string_lossy().into_owned(),
            "host" => remote.map(|remote| remote.host.clone()).unwrap_or_default(),
            "port" => remote.map(|remote| remote.port.to_string()).unwrap_or_default(),
            "user" => remote.map(|remote| remote.user.clone()).unwrap_or_default(),
            "password" => remote.and_then(|remote| remote.password.clone()).unwrap_or_default(),
            "fingerprint" => remote.and_then(|remote| remote.fingerprint.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        QString::from(text)
    }

    #[allow(non_snake_case)]
    fn deviceIdAt(&self, row: i32) -> i32 {
        self.row(row).map_or(-1, |row| row.device as i32)
    }

    #[allow(non_snake_case)]
    fn deviceRow(&self, id: i32) -> i32 {
        let row = DeviceId::try_from(id).ok().and_then(|id| self.state.tree.find_device(id));
        row.map_or(-1, |row| row as i32)
    }

    #[allow(non_snake_case)]
    fn deviceSavesPassword(&self, row: i32) -> bool {
        self.remote_at(row).and_then(|device| device.remote.as_ref()).is_some_and(|remote| remote.save_password)
    }

    #[allow(non_snake_case)]
    fn deviceConnected(&self, row: i32) -> bool {
        self.row(row).is_some_and(|row| self.fs(row.device).is_some())
    }

    fn login(&mut self, row: i32, user: QString, password: QString, save: bool) {
        let Some(id) = self.remote_at(row).map(|device| device.id) else { return };
        let Some(device) = self.state.devices.iter_mut().find(|device| device.id == id) else { return };
        if let Some(remote) = &mut device.remote {
            remote.user = user.to_string();
            remote.password = Some(password.to_string());
            remote.save_password = save;
        }
        device.save(&mut self.state.config);
        self.connect(id);
    }

    #[allow(non_snake_case)]
    fn trustHost(&mut self, row: i32, fingerprint: QString) {
        let Some(id) = self.remote_at(row).map(|device| device.id) else { return };
        let Some(device) = self.state.devices.iter_mut().find(|device| device.id == id) else { return };
        if let Some(remote) = &mut device.remote {
            remote.fingerprint = Some(fingerprint.to_string());
        }
        device.save(&mut self.state.config);
        self.connect(id);
    }

    fn disconnect(&mut self, row: i32) {
        if let Some(id) = self.remote_at(row).map(|device| device.id) {
            self.close(id);
        }
    }

    /// close the connection of a remote device: it stays listed, collapsed
    fn close(&mut self, id: DeviceId) {
        let Some(device) = self.state.devices.iter_mut().find(|device| device.id == id) else { return };
        if device.remote.is_none() || device.fs.take().is_none() {
            return;
        }
        // it was answering a moment ago
        device.state = DeviceState::Online;
        let folder = device.home.clone();
        let steps = self.state.tree.plan_set_current(id, &folder);
        self.run(steps);
        self.state.prune_selection();
        self.selectionChanged();
        self.place(id);
    }

    fn probe(&mut self) {
        let waiting: Vec<(DeviceId, String, u16)> = self
            .state
            .devices
            .iter()
            .filter(|device| device.fs.is_none() && !self.state.connecting.contains(&device.id))
            .filter_map(|device| device.remote.as_ref().map(|remote| (device.id, remote.host.clone(), remote.port)))
            .collect();
        for (id, host, port) in waiting {
            self.spawn(
                async move { tokio::task::spawn_blocking(move || devices::reachable(&host, port)).await.unwrap_or(false) },
                move |this, reachable| {
                    let Some(device) = this.state.device_mut(id) else { return };
                    let state = if reachable { DeviceState::Online } else { DeviceState::Offline };
                    if device.fs.is_none() && device.state != state {
                        device.state = state;
                        this.place(id);
                    }
                },
            );
        }
    }

    fn refresh(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if current.expanded {
            self.list(current.device, current.path.clone());
        }
    }

    fn list(&mut self, device: DeviceId, path: PathBuf) {
        let Some(fs) = self.fs(device) else { return };
        let listed = path.clone();
        self.spawn(
            async move { fs.list(&listed).await.map_err(|error| error.to_string()) },
            move |this, listing| this.listed(device, &path, listing),
        );
    }

    fn listed(&mut self, device: DeviceId, path: &Path, listing: Result<Vec<Entry>, String>) {
        let steps = self.state.tree.plan_loaded(device, path, listing);
        if steps.is_empty() {
            return;
        }
        self.run(steps);
        // a full listing replaces the rows of the folder: selected items may be gone
        if self.state.prune_selection() {
            self.selectionChanged();
        }
    }

    fn enter(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if current.kind == RowKind::Folder {
            self.set_current(current.device, current.path.clone());
        }
    }

    #[allow(non_snake_case)]
    fn setCurrentFolder(&mut self, row: i32, path: QString) {
        let Some(current) = self.row(row) else { return };
        let path = PathBuf::from(path.to_string());
        if path.is_absolute() {
            self.set_current(current.device, path);
        }
    }

    /// restart the tree of a device from another folder, and show its content
    fn set_current(&mut self, device: DeviceId, path: PathBuf) {
        let steps = self.state.tree.plan_set_current(device, &path);
        self.run(steps);
        if device == LOCAL {
            self.state.config.set(LOCAL_SECTION, "folder", &path.to_string_lossy());
        } else if let Some(index) = self.state.devices.iter().position(|other| other.id == device) {
            self.state.devices[index].home = path.clone();
            self.state.devices[index].save(&mut self.state.config);
        }
        // the previous rows are gone, the device itself may stay selected
        self.state.prune_selection();
        self.selectionChanged();
        if let Some(row) = self.state.tree.find_device(device) {
            self.expand(row);
            self.revealed(row as i32, true);
        }
    }

    fn rename(&mut self, row: i32, name: QString) {
        let Some(current) = self.row(row) else { return };
        if !current.is_item() {
            return;
        }
        let (device, old) = (current.device, current.path.clone());
        let Some(fs) = self.fs(device) else { return };
        let name = name.to_string();
        let source = old.clone();
        self.spawn(
            async move {
                let new = fs.rename(&source, &name).await?;
                let entry = fs.stat(&new).await?;
                Ok::<_, FsError>((new, entry))
            },
            move |this, result| match result {
                Ok((new, entry)) => {
                    if new != old {
                        let steps = this.state.tree.plan_remove(device, &old);
                        this.run(steps);
                        this.state.selection.renamed(device, &old, &new);
                    }
                    this.upsert(device, &new, entry);
                    // the single selected row may have moved
                    this.selectionChanged();
                }
                Err(error) => this.report(error),
            },
        );
    }

    /// show a created or modified item, and bring it into view
    fn upsert(&mut self, device: DeviceId, path: &Path, entry: Entry) {
        let Some(folder) = path.parent() else { return };
        let steps = self.state.tree.plan_upsert(device, folder, entry);
        self.run(steps);
        if let Some(row) = self.state.tree.find(device, path) {
            self.revealed(row as i32, false);
        }
    }

    /// folder in which something created "at" this row goes
    fn destination(&self, row: i32) -> Option<(DeviceId, PathBuf)> {
        let current = self.row(row)?;
        match current.kind {
            RowKind::Device | RowKind::Folder => Some((current.device, current.path.clone())),
            // next to a file, or in the folder a placeholder stands for
            RowKind::File => Some((current.device, current.path.parent()?.to_owned())),
            RowKind::Loading | RowKind::Error => Some((current.device, current.path.clone())),
        }
    }

    fn create(&mut self, row: i32, name: QString, folder: bool) {
        let Some((device, parent)) = self.destination(row) else { return };
        let Some(fs) = self.fs(device) else { return };
        let name = name.to_string();
        let destination = parent.clone();
        self.spawn(
            async move {
                let path = match folder {
                    true => fs.mkdir(&destination, &name).await?,
                    false => fs.create_file(&destination, &name).await?,
                };
                let entry = fs.stat(&path).await?;
                Ok::<_, FsError>((path, entry))
            },
            move |this, result| match result {
                Ok((path, entry)) => {
                    this.upsert(device, &path, entry);
                    this.arrived(device, &parent);
                }
                Err(error) => this.report(error),
            },
        );
    }

    /// after items were added to a folder: show its content
    fn arrived(&mut self, device: DeviceId, folder: &Path) {
        match self.state.tree.find(device, folder) {
            // the listing of a collapsed folder brings the new items
            Some(row) if !self.state.tree.rows()[row].expanded => self.expand(row),
            _ => self.measure(device, folder),
        }
    }

    #[allow(non_snake_case)]
    fn startTransfer(&mut self, moving: bool) {
        let items: Vec<_> =
            self.state.selection.top_level().into_iter().filter(|(_, path)| path != device_item()).collect();
        if items.is_empty() {
            return;
        }
        self.state.pending = Some(Pending { moving, items });
        self.pendingChanged();
        self.clearSelection();
    }

    #[allow(non_snake_case)]
    fn cancelTransfer(&mut self) {
        if self.state.pending.take().is_some() {
            self.pendingChanged();
        }
    }

    #[allow(non_snake_case)]
    fn transferTo(&mut self, row: i32) {
        let Some((to, folder)) = self.destination(row) else { return };
        let Some(Pending { moving, items }) = self.state.pending.take() else { return };
        self.pendingChanged();
        // one job per device the items come from
        let mut sources: BTreeMap<DeviceId, Vec<PathBuf>> = BTreeMap::new();
        for (device, path) in items {
            sources.entry(device).or_default().push(path);
        }
        for (from, sources) in sources {
            let (Some(source), Some(destination)) = (self.fs(from), self.fs(to)) else { continue };
            let job = Job {
                moving,
                from: source,
                sources,
                to: destination,
                folder: folder.clone(),
                same_device: from == to,
            };
            self.state.queue.push_back(Transfer { from, to, open: false, job });
        }
        self.transferChanged();
        self.start_transfer();
    }

    /// run the next waiting transfer, if none is running
    fn start_transfer(&mut self) {
        if self.state.running.is_some() {
            return;
        }
        let Some(Transfer { from, to, open, job }) = self.state.queue.pop_front() else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        let (moving, folder) = (job.moving, job.folder.clone());
        self.state.running =
            Some(Running { cancel: cancel.clone(), reply: None, moving, done: 0, total: 0, name: String::new() });
        self.transferChanged();
        let events: Box<dyn Fn(Event) + Send + Sync> = if self.state.inline {
            let decision = self.state.inline_decision;
            Box::new(move |event| {
                if let (Event::Conflict { reply, .. }, Some(decision)) = (event, decision) {
                    let _ = reply.send(decision);
                }
            })
        } else {
            let this = QPointer::from(&*self);
            Box::new(qmetaobject::queued_callback(move |event: Event| {
                if let Some(this) = this.as_pinned() {
                    this.borrow_mut().transfer_event(event);
                }
            }))
        };
        self.spawn(
            async move { job::run(job, &cancel, events).await },
            move |this, outcome| this.transfer_done(from, to, &folder, open, outcome),
        );
    }

    /// news from the running transfer
    fn transfer_event(&mut self, event: Event) {
        let Some(running) = &mut self.state.running else { return };
        match event {
            Event::Progress { done, total, name } => {
                (running.done, running.total, running.name) = (done, total, name);
                self.transferChanged();
            }
            Event::Conflict { name, reply } => {
                running.reply = Some(reply);
                self.conflict(QString::from(name));
            }
        }
    }

    fn transfer_done(&mut self, from: DeviceId, to: DeviceId, folder: &Path, open: bool, outcome: Outcome) {
        self.state.running = None;
        if open {
            // a fetched file: nothing to show in the tree
            for (path, entry) in outcome.arrived {
                self.open_local(path, entry.size == 0);
            }
            self.report_all(&outcome.errors);
            self.transferChanged();
            self.start_transfer();
            return;
        }
        let mut left = Vec::new();
        for source in outcome.left {
            let steps = self.state.tree.plan_remove(from, &source);
            self.run(steps);
            left.extend(source.parent().map(Path::to_owned));
        }
        let arrived = !outcome.arrived.is_empty();
        for (path, entry) in outcome.arrived {
            self.upsert(to, &path, entry);
        }
        if arrived {
            self.arrived(to, folder);
        }
        left.sort();
        left.dedup();
        for parent in left {
            self.measure(from, &parent);
        }
        self.report_all(&outcome.errors);
        self.transferChanged();
        self.start_transfer();
    }

    #[allow(non_snake_case)]
    fn resolveConflict(&mut self, answer: QString, all: bool) {
        let reply = self.state.running.as_mut().and_then(|running| running.reply.take());
        if let Some(reply) = reply {
            let answer = Answer::parse(&answer.to_string()).unwrap_or(Answer::Skip);
            let _ = reply.send(Decision { answer, all });
        }
    }

    #[allow(non_snake_case)]
    fn cancelRunning(&mut self) {
        self.state.queue.clear();
        if let Some(running) = &mut self.state.running {
            running.cancel.store(true, Ordering::Relaxed);
            // a transfer waiting on a conflict goes on, to see it is cancelled
            running.reply = None;
        }
        self.transferChanged();
    }

    #[allow(non_snake_case)]
    fn createFolder(&mut self, row: i32, name: QString) {
        self.create(row, name, true);
    }

    #[allow(non_snake_case)]
    fn createFile(&mut self, row: i32, name: QString) {
        self.create(row, name, false);
    }

    fn chmod(&mut self, row: i32, mode: i32) {
        let Some(current) = self.row(row) else { return };
        if !current.is_item() {
            return;
        }
        let (device, path) = (current.device, current.path.clone());
        let Some(fs) = self.fs(device) else { return };
        let target = path.clone();
        self.spawn(
            async move {
                fs.chmod(&target, mode as u32).await?;
                fs.stat(&target).await
            },
            move |this, result| match result {
                Ok(entry) => this.upsert(device, &path, entry),
                Err(error) => this.report(error),
            },
        );
    }

    #[allow(non_snake_case)]
    fn nameAt(&self, row: i32) -> QString {
        self.row(row).map_or_else(QString::default, |row| QString::from(row.name.as_str()))
    }

    #[allow(non_snake_case)]
    fn modeAt(&self, row: i32) -> i32 {
        self.row(row).and_then(|row| row.entry.as_ref()).map_or(0, |entry| entry.mode as i32)
    }

    #[allow(non_snake_case)]
    fn toggleSelected(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if !selectable(current) {
            return;
        }
        let (device, path) = (current.device, selection_path(current).to_owned());
        self.state.selection.toggle(device, &path);
        self.rows_changed(row as usize, row as usize);
        self.selectionChanged();
    }

    #[allow(non_snake_case)]
    fn clearSelection(&mut self) {
        if self.state.selection.clear() {
            self.all_rows_changed();
            self.selectionChanged();
        }
    }

    fn all_rows_changed(&mut self) {
        let count = self.state.tree.rows().len();
        if count > 0 {
            self.rows_changed(0, count - 1);
        }
    }

    #[allow(non_snake_case)]
    fn removeSelected(&mut self) {
        let items: Vec<(DeviceId, PathBuf, Fs)> = self
            .state
            .selection
            .top_level()
            .into_iter()
            // devices are not deleted
            .filter(|(_, path)| path != device_item())
            .filter_map(|(device, path)| Some((device, path, self.fs(device)?)))
            .collect();
        if items.is_empty() {
            return;
        }
        self.spawn(
            async move {
                let mut results = Vec::with_capacity(items.len());
                for (device, path, fs) in items {
                    let result = fs.delete(&path).await;
                    results.push((device, path, result));
                }
                results
            },
            |this, results| {
                let mut errors = Vec::new();
                let mut left = Vec::new();
                for (device, path, result) in results {
                    match result {
                        // already gone is as good as deleted
                        Ok(()) | Err(FsError::NotFound(_)) => {
                            let steps = this.state.tree.plan_remove(device, &path);
                            this.run(steps);
                            this.state.selection.removed(device, &path);
                            left.extend(path.parent().map(|parent| (device, parent.to_owned())));
                        }
                        Err(error) => errors.push(error.to_string()),
                    }
                }
                this.selectionChanged();
                left.sort();
                left.dedup();
                for (device, parent) in left {
                    this.measure(device, &parent);
                }
                this.report_all(&errors);
            },
        );
    }

    fn show_hidden(&self) -> bool {
        self.state.tree.show_hidden()
    }

    fn set_show_hidden(&mut self, show: bool) {
        if show == self.state.tree.show_hidden() {
            return;
        }
        self.begin_reset_model();
        self.state.tree.set_show_hidden(show);
        let unselected = self.state.prune_selection();
        self.end_reset_model();
        self.state.config.set(SETTINGS, "hidden", flag(show));
        self.showHiddenChanged();
        if unselected {
            self.selectionChanged();
        }
    }

    fn selection_count(&self) -> i32 {
        self.state.selection.count() as i32
    }

    fn sort_name(&self) -> QString {
        QString::from(sort_name(self.state.tree.sort()))
    }

    fn set_sort_name(&mut self, name: QString) {
        let Some(sort) = parse_sort(&name.to_string()) else { return };
        if sort == self.state.tree.sort() {
            return;
        }
        self.begin_reset_model();
        self.state.tree.set_sort(sort);
        self.end_reset_model();
        self.state.config.set(SETTINGS, "sort", sort_name(sort));
        self.sortChanged();
        // the selected row may have moved
        self.selectionChanged();
    }

    fn pending_count(&self) -> i32 {
        self.state.pending.as_ref().map_or(0, |pending| pending.items.len() as i32)
    }

    fn pending_move(&self) -> bool {
        self.state.pending.as_ref().is_some_and(|pending| pending.moving)
    }

    fn busy(&self) -> bool {
        self.state.running.is_some()
    }

    fn transfer_progress(&self) -> f64 {
        match &self.state.running {
            Some(running) if running.total > 0 => (running.done as f64 / running.total as f64).min(1.0),
            _ => 0.0,
        }
    }

    fn transfer_name(&self) -> QString {
        QString::from(self.state.running.as_ref().map_or("", |running| running.name.as_str()))
    }

    fn transfer_moving(&self) -> bool {
        self.state.running.as_ref().is_some_and(|running| running.moving)
    }

    fn transfer_waiting(&self) -> i32 {
        self.state.queue.len() as i32
    }

    fn show_details(&self) -> bool {
        self.state.show_details
    }

    fn set_show_details(&mut self, show: bool) {
        if show != self.state.show_details {
            self.state.show_details = show;
            self.state.config.set(SETTINGS, "details", flag(show));
            self.showDetailsChanged();
        }
    }

    fn selected_item_count(&self) -> i32 {
        self.state.selection.iter().filter(|(_, path)| path != device_item()).count() as i32
    }

    fn selected_row(&self) -> i32 {
        self.state
            .selection
            .single()
            .and_then(|(device, path)| self.state.shown(*device, path))
            .map_or(-1, |row| row as i32)
    }

    fn selected_kind(&self) -> QString {
        let kind = usize::try_from(self.selected_row()).ok().and_then(|row| self.state.tree.row(row));
        QString::from(kind.map_or("", |row| kind_name(row.kind)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::local::Local;
    use crate::testing::TempDir;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// temp/ { home/user/ { docs/ { report.txt }, .config/, notes.txt }, etc/ { hosts } }
    /// with temp as current folder of the device
    fn sample() -> (TempDir, TreeModel) {
        let dir = TempDir::new();
        let root = dir.path();
        fs::create_dir_all(root.join("home/user/docs")).unwrap();
        fs::create_dir_all(root.join("home/user/.config")).unwrap();
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::write(root.join("home/user/docs/report.txt"), b"report").unwrap();
        fs::write(root.join("home/user/notes.txt"), b"12345").unwrap();
        fs::write(root.join("etc/hosts"), b"").unwrap();

        let device = Device {
            id: 0,
            name: "phone".into(),
            kind: DeviceKind::Local,
            state: DeviceState::Connected,
            fs: Some(Fs::Local(Local)),
            home: root.to_owned(),
            remote: None,
            used: 0,
            remembered: false,
        };
        let mut model = TreeModel::default();
        model.state = State::new(vec![device]);
        model.state.home = root.join("home/user");
        model.state.inline = true;
        (dir, model)
    }

    fn text(model: &TreeModel, row: i32, role: &str) -> String {
        match model.value(row, role) {
            Some(Value::Text(text)) => text,
            other => panic!("role {role} of row {row} is {other:?}"),
        }
    }

    fn names(model: &TreeModel) -> Vec<String> {
        (0..model.row_count())
            .map(|row| {
                let depth = match model.value(row, "depth") {
                    Some(Value::Int(depth)) => depth as usize,
                    other => panic!("depth is {other:?}"),
                };
                let kind = text(model, row, "kind");
                let name = match kind.as_str() {
                    "loading" | "error" => format!("<{kind}>"),
                    _ => text(model, row, "name"),
                };
                format!("{}{}", "  ".repeat(depth), name)
            })
            .collect()
    }

    /// expand the device, then down to home/user
    fn open(model: &mut TreeModel) {
        open_user(model);
        assert_eq!(
            names(model),
            ["phone", "  etc", "  home", "    user", "      docs", "      notes.txt"]
        );
    }

    fn open_user(model: &mut TreeModel) {
        model.toggle(0);
        model.toggle(row_of(model, "home"));
        model.toggle(row_of(model, "user"));
    }

    fn row_of(model: &TreeModel, name: &str) -> i32 {
        (0..model.row_count())
            .find(|&row| text(model, row, "name") == name)
            .unwrap_or_else(|| panic!("no row named {name} in {:?}", names(model)))
    }

    #[test]
    fn default_model_lists_the_local_device() {
        let model = TreeModel::default();
        assert_eq!(model.row_count(), 1);
        assert_eq!(text(&model, 0, "kind"), "device");
        assert_eq!(text(&model, 0, "category"), "connected");
        // its tree starts from the home folder
        assert_eq!(text(&model, 0, "path"), Device::local().home.to_str().unwrap());
        assert_eq!(model.value(0, "expanded"), Some(Value::Bool(false)));
        assert_eq!(model.value(0, "hasChildren"), Some(Value::Bool(true)));
        assert_eq!(model.value(0, "selectable"), Some(Value::Bool(true)));
    }

    #[test]
    fn every_role_has_a_value_and_a_qt_number() {
        let (_dir, mut model) = sample();
        model.toggle(0);
        for row in 0..model.row_count() {
            for role in ROLES {
                assert!(model.value(row, role).is_some(), "role {role} of row {row}");
            }
        }
        assert_eq!(model.value(0, "unknown"), None);
        assert_eq!(model.value(-1, "name"), None);
        assert_eq!(model.value(model.row_count(), "name"), None);

        let roles = model.role_names();
        assert_eq!(roles.len(), ROLES.len());
        assert_eq!(roles[&USER_ROLE].to_string(), "name");
        assert_eq!(roles[&(USER_ROLE + 6)].to_string(), "selected");
    }

    #[test]
    fn device_tree_starts_from_its_current_folder() {
        let (dir, mut model) = sample();
        let root = dir.path().to_str().unwrap().to_owned();
        assert_eq!(text(&model, 0, "path"), root);
        model.toggle(0);
        assert_eq!(names(&model), ["phone", "  etc", "  home"]);
        assert_eq!(text(&model, 1, "category"), "connected");
        model.toggle(0);
        assert_eq!(names(&model), ["phone"]);
    }

    #[test]
    fn entering_a_folder_makes_it_the_current_folder() {
        let (dir, mut model) = sample();
        let root = dir.path().to_str().unwrap().to_owned();
        open(&mut model);
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.toggleSelected(0);
        // only folders can be entered
        model.enter(row_of(&model, "notes.txt"));
        model.enter(0);
        model.enter(99);
        assert_eq!(text(&model, 0, "path"), root);

        model.enter(row_of(&model, "user"));
        assert_eq!(names(&model), ["phone", "  docs", "  notes.txt"]);
        assert_eq!(text(&model, 0, "path"), format!("{root}/home/user"));
        assert_eq!(text(&model, 2, "path"), format!("{root}/home/user/notes.txt"));
        // the rows were replaced: only the device is still selected
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), 0);
        assert_eq!(model.value(2, "selected"), Some(Value::Bool(false)));

        // back up through the path, from any row of the device
        model.setCurrentFolder(1, root.as_str().into());
        assert_eq!(names(&model), ["phone", "  etc", "  home"]);
        assert_eq!(text(&model, 0, "path"), root);

        // refused: not a full path, unknown row
        model.setCurrentFolder(0, "home".into());
        model.setCurrentFolder(99, "/".into());
        assert_eq!(text(&model, 0, "path"), root);
    }

    #[test]
    fn changing_the_current_folder_expands_the_device() {
        let (dir, mut model) = sample();
        model.setCurrentFolder(0, dir.path().join("etc").to_str().unwrap().into());
        assert_eq!(names(&model), ["phone", "  hosts"]);
        // a folder that cannot be listed shows why
        model.setCurrentFolder(0, dir.path().join("missing").to_str().unwrap().into());
        assert_eq!(names(&model), ["phone", "  <error>"]);
    }

    #[test]
    fn device_selection() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggleSelected(0);
        assert_eq!(model.value(0, "selected"), Some(Value::Bool(true)));
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_item_count(), 0);
        assert_eq!(model.selected_row(), 0);
        assert_eq!(model.selected_kind().to_string(), "device");

        // file actions do not apply to a device
        model.removeSelected();
        model.rename(0, "other".into());
        model.chmod(0, 0o700);
        assert!(dir.path().join("home/user/notes.txt").exists());
        assert_eq!(names(&model).len(), 6);

        // it stays selected when collapsed, and is not deleted with the rest
        model.toggleSelected(row_of(&model, "etc"));
        assert_eq!(model.selection_count(), 2);
        assert_eq!(model.selected_item_count(), 1);
        assert_eq!(model.selected_kind().to_string(), "");
        model.removeSelected();
        assert!(!dir.path().join("etc").exists());
        assert!(dir.path().exists());
        model.toggle(0);
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_kind().to_string(), "device");
    }

    #[test]
    fn kind_of_the_single_selected_item() {
        let (_dir, mut model) = sample();
        open(&mut model);
        assert_eq!(model.selected_kind().to_string(), "");
        model.toggleSelected(row_of(&model, "docs"));
        assert_eq!(model.selected_kind().to_string(), "folder");
        model.toggleSelected(row_of(&model, "notes.txt"));
        assert_eq!(model.selected_kind().to_string(), "");
        model.toggleSelected(row_of(&model, "docs"));
        assert_eq!(model.selected_kind().to_string(), "file");
        assert_eq!(model.selected_item_count(), 1);
    }

    #[test]
    fn sort_follows_the_property() {
        let (dir, mut model) = sample();
        fs::write(dir.path().join("home/user/big.txt"), [0u8; 100]).unwrap();
        fs::write(dir.path().join("home/user/a.txt"), b"1").unwrap();
        open_user(&mut model);
        assert_eq!(model.sort_name().to_string(), "name");
        assert_eq!(names(&model)[4..], ["      docs", "      a.txt", "      big.txt", "      notes.txt"]);

        model.toggleSelected(row_of(&model, "a.txt"));
        model.set_sort_name("size".into());
        assert_eq!(model.sort_name().to_string(), "size");
        assert_eq!(names(&model)[4..], ["      docs", "      big.txt", "      notes.txt", "      a.txt"]);
        assert_eq!(model.selected_row(), row_of(&model, "a.txt"));

        // created items take their place in the current order
        model.createFile(row_of(&model, "user"), "empty".into());
        assert_eq!(names(&model)[7..], ["      a.txt", "      empty"]);

        model.set_sort_name("date".into());
        assert_eq!(model.sort_name().to_string(), "date");
        model.set_sort_name("colour".into());
        assert_eq!(model.sort_name().to_string(), "date");
        model.set_sort_name("name".into());
        assert_eq!(names(&model)[5..], ["      a.txt", "      big.txt", "      empty", "      notes.txt"]);
    }

    #[test]
    fn details_of_files_and_folders() {
        let (dir, mut model) = sample();
        fs::set_permissions(dir.path().join("home/user/notes.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink("notes.txt", dir.path().join("home/user/shortcut")).unwrap();
        open_user(&mut model);

        let notes = row_of(&model, "notes.txt");
        assert_eq!(text(&model, notes, "kind"), "file");
        assert_eq!(model.value(notes, "size"), Some(Value::Number(5.0)));
        assert_eq!(text(&model, notes, "permissions"), "rw-r-----");
        assert_eq!(model.value(notes, "mode"), Some(Value::Int(0o640)));
        assert_eq!(model.value(notes, "hasChildren"), Some(Value::Bool(false)));
        assert_eq!(model.value(notes, "selectable"), Some(Value::Bool(true)));
        assert_eq!(text(&model, notes, "path"), dir.path().join("home/user/notes.txt").to_str().unwrap());
        assert!(!text(&model, notes, "owner").is_empty());
        assert!(!text(&model, notes, "group").is_empty());
        assert_eq!(text(&model, notes, "link"), "");
        match model.value(notes, "date") {
            Some(Value::Number(date)) => assert!(date > 1.6e12, "date in milliseconds: {date}"),
            other => panic!("date is {other:?}"),
        }

        let docs = row_of(&model, "docs");
        assert_eq!(text(&model, docs, "kind"), "folder");
        assert_eq!(model.value(docs, "hasChildren"), Some(Value::Bool(true)));
        assert_eq!(text(&model, row_of(&model, "shortcut"), "link"), "notes.txt");
    }

    #[test]
    fn toggle_expands_and_collapses_folders() {
        let (_dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "docs"));
        model.toggle(row_of(&model, "etc"));
        assert_eq!(
            names(&model),
            [
                "phone", "  etc", "    hosts", "  home", "    user", "      docs",
                "        report.txt", "      notes.txt"
            ]
        );
        model.toggle(row_of(&model, "home"));
        assert_eq!(names(&model), ["phone", "  etc", "    hosts", "  home"]);

        // files and out of range rows do nothing
        model.toggle(row_of(&model, "hosts"));
        model.toggle(-1);
        model.toggle(99);
        assert_eq!(names(&model).len(), 4);
    }

    #[test]
    fn unreadable_folder_shows_an_error_row() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let (dir, mut model) = sample();
        let locked = dir.path().join("etc");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        model.toggle(0);
        model.toggle(row_of(&model, "etc"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(names(&model)[1..3], ["  etc", "    <error>"]);
        assert!(text(&model, 2, "name").contains("permission denied"));
        assert_eq!(model.value(2, "selectable"), Some(Value::Bool(false)));

        // listing again once readable replaces the error
        model.refresh(1);
        assert_eq!(names(&model)[1..3], ["  etc", "    hosts"]);
    }

    #[test]
    fn refresh_shows_outside_changes() {
        let (dir, mut model) = sample();
        open(&mut model);
        fs::write(dir.path().join("home/user/added.txt"), b"").unwrap();
        fs::remove_file(dir.path().join("home/user/notes.txt")).unwrap();
        model.refresh(row_of(&model, "user"));
        assert_eq!(names(&model)[4..], ["      docs", "      added.txt"]);
        // nothing to refresh on a collapsed folder
        model.refresh(row_of(&model, "etc"));
        assert_eq!(names(&model).len(), 6);
    }

    #[test]
    fn hidden_items_follow_the_property() {
        let (_dir, mut model) = sample();
        open(&mut model);
        assert!(!model.show_hidden());
        assert!(!names(&model).contains(&"      .config".to_owned()));

        model.set_show_hidden(true);
        assert!(model.show_hidden());
        assert_eq!(names(&model)[4..], ["      .config", "      docs", "      notes.txt"]);

        model.toggleSelected(row_of(&model, ".config"));
        model.toggleSelected(row_of(&model, "docs"));
        model.set_show_hidden(false);
        assert_eq!(names(&model)[4..], ["      docs", "      notes.txt"]);
        // what is not displayed is not selected anymore
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), row_of(&model, "docs"));
    }

    #[test]
    fn selection_across_folders() {
        let (_dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "etc"));
        assert_eq!(model.selection_count(), 0);
        assert_eq!(model.selected_row(), -1);

        let notes = row_of(&model, "notes.txt");
        model.toggleSelected(notes);
        assert_eq!(model.value(notes, "selected"), Some(Value::Bool(true)));
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), notes);

        model.toggleSelected(row_of(&model, "hosts"));
        assert_eq!(model.selection_count(), 2);
        assert_eq!(model.selected_row(), -1);

        // placeholders and out of range rows are not selectable
        model.toggleSelected(99);
        model.toggleSelected(-1);
        assert_eq!(model.selection_count(), 2);

        model.toggleSelected(notes);
        assert_eq!(model.value(notes, "selected"), Some(Value::Bool(false)));
        model.clearSelection();
        assert_eq!(model.selection_count(), 0);
    }

    #[test]
    fn collapsing_unselects_what_gets_hidden() {
        let (_dir, mut model) = sample();
        open(&mut model);
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.toggleSelected(row_of(&model, "user"));
        model.toggle(row_of(&model, "user"));
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), row_of(&model, "user"));
    }

    #[test]
    fn rename_moves_the_row_and_keeps_it_selected() {
        let (dir, mut model) = sample();
        open(&mut model);
        let notes = row_of(&model, "notes.txt");
        model.toggleSelected(notes);
        model.rename(notes, "Archive.txt".into());

        assert_eq!(names(&model)[4..], ["      docs", "      Archive.txt"]);
        assert!(dir.path().join("home/user/Archive.txt").exists());
        assert!(!dir.path().join("home/user/notes.txt").exists());
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), row_of(&model, "Archive.txt"));
    }

    #[test]
    fn rename_of_a_folder() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "docs"));
        model.rename(row_of(&model, "docs"), "papers".into());
        // the renamed folder comes back collapsed
        assert_eq!(names(&model)[4..], ["      papers", "      notes.txt"]);
        assert!(dir.path().join("home/user/papers/report.txt").exists());
    }

    #[test]
    fn rename_failure_is_reported_and_changes_nothing() {
        let (dir, mut model) = sample();
        open(&mut model);
        let before = names(&model);
        model.rename(row_of(&model, "notes.txt"), "docs".into());
        model.rename(row_of(&model, "notes.txt"), "a/b".into());
        model.rename(0, "device".into());
        assert_eq!(names(&model), before);
        assert!(dir.path().join("home/user/notes.txt").exists());
    }

    #[test]
    fn create_folder_and_file_in_a_folder() {
        let (dir, mut model) = sample();
        open(&mut model);
        let user = row_of(&model, "user");
        model.createFolder(user, "music".into());
        model.createFile(user, "a.txt".into());
        assert_eq!(
            names(&model)[4..],
            ["      docs", "      music", "      a.txt", "      notes.txt"]
        );
        assert!(dir.path().join("home/user/music").is_dir());
        assert!(dir.path().join("home/user/a.txt").is_file());
    }

    #[test]
    fn create_next_to_a_file_or_in_the_current_folder() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.createFile(row_of(&model, "notes.txt"), "sibling".into());
        assert!(dir.path().join("home/user/sibling").is_file());
        assert_eq!(names(&model)[4..], ["      docs", "      notes.txt", "      sibling"]);
        // on a device: in its current folder
        model.createFolder(0, "from-device".into());
        assert!(dir.path().join("from-device").is_dir());
        assert_eq!(names(&model)[..4], ["phone", "  etc", "  from-device", "  home"]);
        model.createFolder(-1, "nowhere".into());
        assert_eq!(names(&model).len(), 8);
    }

    #[test]
    fn create_in_a_collapsed_folder_expands_it() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.createFile(row_of(&model, "etc"), "fstab".into());
        assert!(dir.path().join("etc/fstab").is_file());
        assert_eq!(names(&model)[1..4], ["  etc", "    fstab", "    hosts"]);
    }

    #[test]
    fn create_failure_changes_nothing() {
        let (_dir, mut model) = sample();
        open(&mut model);
        let before = names(&model);
        model.createFolder(row_of(&model, "user"), "docs".into());
        model.createFile(row_of(&model, "user"), "".into());
        assert_eq!(names(&model), before);
    }

    #[test]
    fn chmod_updates_the_row() {
        let (dir, mut model) = sample();
        open(&mut model);
        let notes = row_of(&model, "notes.txt");
        model.chmod(notes, 0o600);
        assert_eq!(text(&model, notes, "permissions"), "rw-------");
        let mode = fs::metadata(dir.path().join("home/user/notes.txt")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        // an expanded folder stays expanded
        let user = row_of(&model, "user");
        model.chmod(user, 0o750);
        assert_eq!(text(&model, user, "permissions"), "rwxr-x---");
        assert_eq!(model.value(user, "expanded"), Some(Value::Bool(true)));
        assert_eq!(names(&model).len(), 6);
    }

    #[test]
    fn remove_selected_items() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "etc"));
        model.toggle(row_of(&model, "docs"));
        model.toggleSelected(row_of(&model, "hosts"));
        model.toggleSelected(row_of(&model, "docs"));
        model.toggleSelected(row_of(&model, "report.txt"));
        model.removeSelected();

        assert_eq!(names(&model), ["phone", "  etc", "  home", "    user", "      notes.txt"]);
        assert!(!dir.path().join("etc/hosts").exists());
        assert!(!dir.path().join("home/user/docs").exists());
        assert_eq!(model.selection_count(), 0);

        // nothing selected: nothing happens
        model.removeSelected();
        assert_eq!(names(&model).len(), 5);
    }

    #[test]
    fn remove_failure_keeps_the_item_selected() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "etc"));
        model.toggleSelected(row_of(&model, "hosts"));
        model.toggleSelected(row_of(&model, "notes.txt"));
        fs::set_permissions(dir.path().join("etc"), fs::Permissions::from_mode(0o555)).unwrap();
        model.removeSelected();
        fs::set_permissions(dir.path().join("etc"), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(dir.path().join("etc/hosts").exists());
        assert!(!dir.path().join("home/user/notes.txt").exists());
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), row_of(&model, "hosts"));
    }

    #[test]
    fn name_and_mode_of_a_row() {
        let (dir, mut model) = sample();
        fs::set_permissions(dir.path().join("home/user/notes.txt"), fs::Permissions::from_mode(0o604)).unwrap();
        open(&mut model);
        let notes = row_of(&model, "notes.txt");
        assert_eq!(model.nameAt(notes).to_string(), "notes.txt");
        assert_eq!(model.modeAt(notes), 0o604);
        assert_eq!(model.nameAt(0).to_string(), "phone");
        assert_eq!(model.modeAt(0), 0);
        assert_eq!(model.nameAt(99).to_string(), "");
        assert_eq!(model.modeAt(-1), 0);
    }

    #[test]
    fn opening_a_file_outside_the_shared_folders_links_it() {
        let (dir, mut model) = sample();
        let links = dir.path().join("home/user/Downloads/.sailtree-open");
        open(&mut model);
        // folders and devices are not opened
        model.open(row_of(&model, "docs"));
        model.open(0);
        model.open(99);
        assert!(!links.exists());

        model.open(row_of(&model, "notes.txt"));
        let staged: Vec<_> = fs::read_dir(&links).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(staged.len(), 1);
        assert_eq!(fs::read(staged[0].join("notes.txt")).unwrap(), b"12345");
    }

    fn number(model: &TreeModel, row: i32, role: &str) -> f64 {
        match model.value(row, role) {
            Some(Value::Number(number)) => number,
            other => panic!("role {role} of row {row} is {other:?}"),
        }
    }

    #[test]
    fn opening_an_empty_file_does_not_go_through_the_system_default() {
        let (dir, mut model) = sample();
        fs::write(dir.path().join("home/user/empty.txt"), b"").unwrap();
        fs::write(dir.path().join("home/user/empty.log"), b"").unwrap();
        open_user(&mut model);
        // no launcher in the test environment: the failure is reported, the files were still
        // made visible, under a name the text viewer accepts
        model.open(row_of(&model, "empty.txt"));
        model.open(row_of(&model, "empty.log"));
        let links = dir.path().join("home/user/Downloads/.sailtree-open");
        let mut staged: Vec<_> = fs::read_dir(links)
            .unwrap()
            .flat_map(|folder| fs::read_dir(folder.unwrap().path()).unwrap())
            .map(|link| link.unwrap().file_name().into_string().unwrap())
            .collect();
        staged.sort();
        assert_eq!(staged, ["empty.log.txt", "empty.txt"]);
    }

    #[test]
    fn ratio_of_files_in_their_folder() {
        let (dir, mut model) = sample();
        fs::write(dir.path().join("home/user/big.bin"), [0u8; 15]).unwrap();
        open_user(&mut model);
        // notes.txt holds 5 bytes of the 20 in files directly in its folder
        assert_eq!(number(&model, row_of(&model, "notes.txt"), "ratio"), 0.25);
        assert_eq!(number(&model, row_of(&model, "big.bin"), "ratio"), 0.75);
        // folders and devices have none
        assert_eq!(number(&model, row_of(&model, "docs"), "ratio"), -1.0);
        assert_eq!(number(&model, 0, "ratio"), -1.0);

        model.toggleSelected(row_of(&model, "big.bin"));
        model.removeSelected();
        assert_eq!(number(&model, row_of(&model, "notes.txt"), "ratio"), 1.0);
    }

    #[test]
    fn copy_of_the_selection_into_a_folder() {
        let (dir, mut model) = sample();
        open(&mut model);
        assert_eq!(model.pending_count(), 0);
        // nothing selected, or only a device: nothing to transfer
        model.startTransfer(false);
        model.toggleSelected(0);
        model.startTransfer(false);
        assert_eq!(model.pending_count(), 0);
        model.clearSelection();

        model.toggleSelected(row_of(&model, "docs"));
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(false);
        assert_eq!(model.pending_count(), 2);
        assert!(!model.pending_move());
        assert_eq!(model.selection_count(), 0);

        // the items wait while the tree changes
        model.toggle(row_of(&model, "user"));
        model.transferTo(row_of(&model, "etc"));
        assert_eq!(model.pending_count(), 0);
        assert!(!model.busy());
        let root = dir.path();
        assert_eq!(fs::read(root.join("etc/notes.txt")).unwrap(), b"12345");
        assert_eq!(fs::read(root.join("etc/docs/report.txt")).unwrap(), b"report");
        assert!(root.join("home/user/notes.txt").exists());
        // the destination is expanded to show them
        assert_eq!(
            names(&model),
            ["phone", "  etc", "    docs", "    hosts", "    notes.txt", "  home", "    user"]
        );
        assert_eq!(number(&model, row_of(&model, "notes.txt"), "ratio"), 1.0);
    }

    #[test]
    fn move_of_the_selection_into_the_current_folder() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggle(row_of(&model, "docs"));
        model.toggleSelected(row_of(&model, "report.txt"));
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        assert!(model.pending_move());

        // on a device: into its current folder
        model.transferTo(0);
        let root = dir.path();
        assert!(root.join("report.txt").is_file());
        assert!(root.join("notes.txt").is_file());
        assert!(!root.join("home/user/notes.txt").exists());
        assert!(!root.join("home/user/docs/report.txt").exists());
        assert_eq!(
            names(&model),
            ["phone", "  etc", "  home", "    user", "      docs", "  notes.txt", "  report.txt"]
        );
        assert_eq!(number(&model, row_of(&model, "report.txt"), "ratio"), 6.0 / 11.0);
        assert_eq!(model.pending_count(), 0);
    }

    #[test]
    fn transfer_failures_keep_the_items_in_place() {
        let (dir, mut model) = sample();
        open(&mut model);
        let before = names(&model);
        // into the folder it is already in, and a folder into itself
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        model.transferTo(row_of(&model, "user"));
        model.cancelRunning();
        model.resolveConflict("overwrite".into(), true);
        model.toggleSelected(row_of(&model, "user"));
        model.startTransfer(false);
        model.transferTo(row_of(&model, "docs"));
        assert_eq!(names(&model), before);
        assert!(dir.path().join("home/user/notes.txt").exists());
        assert!(!dir.path().join("home/user/docs/user").exists());
    }

    #[test]
    fn conflicts_follow_the_answer() {
        let (dir, mut model) = sample();
        let root = dir.path();
        fs::write(root.join("etc/notes.txt"), b"old").unwrap();
        open(&mut model);
        // without answer the item is skipped
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        model.transferTo(row_of(&model, "etc"));
        assert_eq!(fs::read(root.join("etc/notes.txt")).unwrap(), b"old");
        assert!(root.join("home/user/notes.txt").exists());

        model.state.inline_decision = Some(Decision { answer: Answer::Rename, all: false });
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(false);
        model.transferTo(row_of(&model, "etc"));
        assert_eq!(fs::read(root.join("etc/notes (2).txt")).unwrap(), b"12345");

        model.state.inline_decision = Some(Decision { answer: Answer::Overwrite, all: false });
        // collapsed, so the only notes.txt shown is the one to move
        model.toggle(row_of(&model, "etc"));
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        model.transferTo(row_of(&model, "etc"));
        assert_eq!(fs::read(root.join("etc/notes.txt")).unwrap(), b"12345");
        assert!(!root.join("home/user/notes.txt").exists());
        assert_eq!(names(&model)[1..5], ["  etc", "    hosts", "    notes (2).txt", "    notes.txt"]);
        assert!(!model.busy());
        assert_eq!(model.transfer_waiting(), 0);
        assert_eq!(model.transfer_progress(), 0.0);
    }

    #[test]
    fn copy_where_the_item_is_duplicates_it() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(false);
        model.transferTo(row_of(&model, "user"));
        assert_eq!(fs::read(dir.path().join("home/user/notes (2).txt")).unwrap(), b"12345");
        assert_eq!(names(&model)[4..], ["      docs", "      notes (2).txt", "      notes.txt"]);
    }

    #[test]
    fn settings_are_remembered() {
        let dir = TempDir::new();
        let path = dir.path().join("config.ini");
        let (_files, mut model) = sample();
        model.state.config = Config::load(&path);
        assert!(!model.show_details());
        model.set_show_details(true);
        model.set_show_details(true);
        model.set_show_hidden(true);
        model.set_sort_name("date".into());
        model.toggle(0);
        model.enter(row_of(&model, "etc"));

        let config = Config::load(&path);
        assert_eq!(config.flag(SETTINGS, "details"), Some(true));
        assert_eq!(config.flag(SETTINGS, "hidden"), Some(true));
        assert_eq!(config.get(SETTINGS, "sort"), Some("date"));
        assert_eq!(config.get(LOCAL_SECTION, "folder"), Some(text(&model, 0, "path").as_str()));
        assert_eq!(parse_sort("size"), Some(Sort::Size));
        assert_eq!(parse_sort("colour"), None);
    }

    fn device_names(model: &TreeModel) -> Vec<String> {
        (0..model.row_count())
            .filter(|&row| text(model, row, "kind") == "device")
            .map(|row| format!("{} [{}]", text(model, row, "name"), text(model, row, "category")))
            .collect()
    }

    #[test]
    fn remote_devices_are_added_changed_and_removed() {
        let dir = TempDir::new();
        let path = dir.path().join("config.ini");
        let (_files, mut model) = sample();
        model.state.config = Config::load(&path);
        // refused: no host, impossible port
        assert_eq!(model.addDevice("x".into(), "  ".into(), 22, "me".into(), "".into(), false), -1);
        model.addDevice("x".into(), "host".into(), 70000, "me".into(), "".into(), false);
        model.addDevice("x".into(), "host".into(), 0, "me".into(), "".into(), false);
        assert_eq!(model.row_count(), 1);

        // nothing listens there
        assert_eq!(model.addDevice("NAS".into(), "127.0.0.1".into(), 1, "me".into(), "secret".into(), true), 1);
        model.addDevice("".into(), "127.0.0.2".into(), 1, "you".into(), "".into(), false);
        assert_eq!(device_names(&model), ["phone [connected]", "NAS [offline]", "127.0.0.2 [offline]"]);
        assert_eq!(model.value(1, "remote"), Some(Value::Bool(true)));
        assert_eq!(model.value(0, "remote"), Some(Value::Bool(false)));
        assert_eq!(model.value(1, "busy"), Some(Value::Bool(false)));
        assert_eq!(model.deviceText(1, "host".into()).to_string(), "127.0.0.1");
        assert_eq!(model.deviceText(1, "port".into()).to_string(), "1");
        assert_eq!(model.deviceText(1, "user".into()).to_string(), "me");
        assert_eq!(model.deviceText(1, "password".into()).to_string(), "secret");
        assert_eq!(model.deviceText(1, "fingerprint".into()).to_string(), "");
        assert_eq!(model.deviceText(1, "other".into()).to_string(), "");
        assert_eq!(model.deviceText(0, "host".into()).to_string(), "");
        assert_eq!(model.deviceText(0, "name".into()).to_string(), "phone");
        assert!(model.deviceSavesPassword(1) && !model.deviceSavesPassword(2) && !model.deviceSavesPassword(0));
        assert!(model.deviceConnected(0) && !model.deviceConnected(1));

        assert_eq!(model.deviceText(1, "protocol".into()).to_string(), "sftp");
        model.setProtocol(1, "ftp".into());
        model.setProtocol(1, "gopher".into());
        model.setProtocol(0, "ftp".into());
        assert_eq!(model.deviceText(1, "protocol".into()).to_string(), "ftp");
        assert_eq!(model.deviceText(0, "protocol".into()).to_string(), "local");
        model.updateDevice(1, "Storage".into(), "127.0.0.3".into(), 2, "admin".into(), "".into(), false, "/srv".into());
        // the local device has no settings
        model.updateDevice(0, "x".into(), "h".into(), 22, "u".into(), "p".into(), true, "/".into());
        assert_eq!(device_names(&model)[..2], ["phone [connected]", "Storage [offline]"]);
        assert_eq!(text(&model, 1, "path"), "/srv");

        let saved = Device::load(&Config::load(&path));
        assert_eq!(saved.len(), 2);
        let storage = saved.iter().find(|device| device.name == "Storage").unwrap();
        assert_eq!(storage.kind, DeviceKind::Ftp);
        let remote = storage.remote.as_ref().unwrap();
        assert_eq!((remote.host.as_str(), remote.port, remote.user.as_str()), ("127.0.0.3", 2, "admin"));
        assert_eq!(remote.password, None);
        assert_eq!(storage.home, PathBuf::from("/srv"));

        model.toggleSelected(1);
        model.removeDevice(1);
        model.removeDevice(0);
        assert_eq!(device_names(&model), ["phone [connected]", "127.0.0.2 [offline]"]);
        assert_eq!(model.selection_count(), 0);
        assert_eq!(Device::load(&Config::load(&path)).len(), 1);
    }

    #[test]
    fn devices_are_listed_by_category() {
        let (_files, mut model) = sample();
        open(&mut model);
        for name in ["a", "b", "c"] {
            model.addDevice(name.into(), "127.0.0.1".into(), 1, "me".into(), "".into(), false);
        }
        let state = |model: &mut TreeModel, id: DeviceId, state: DeviceState| {
            model.state.device_mut(id).unwrap().state = state;
            model.place(id);
        };
        state(&mut model, 3, DeviceState::Online);
        assert_eq!(device_names(&model), ["phone [connected]", "c [online]", "a [offline]", "b [offline]"]);
        state(&mut model, 1, DeviceState::Connected);
        state(&mut model, 2, DeviceState::Online);
        assert_eq!(device_names(&model), ["phone [connected]", "a [connected]", "c [online]", "b [online]"]);
        state(&mut model, 1, DeviceState::Offline);
        assert_eq!(device_names(&model), ["phone [connected]", "c [online]", "b [online]", "a [offline]"]);
        // the tree of the phone followed
        assert_eq!(names(&model)[..6], ["phone", "  etc", "  home", "    user", "      docs", "      notes.txt"]);
    }

    #[test]
    fn connection_asks_for_what_is_missing_and_reports_failures() {
        let (_files, mut model) = sample();
        // no password yet: nothing is tried until the login is given
        model.addDevice("a".into(), "127.0.0.1".into(), 1, "me".into(), "".into(), false);
        model.toggle(1);
        assert_eq!(names(&model), ["phone", "a"]);
        assert!(model.state.connecting.is_empty());

        // nothing listens there: the device stays offline, not connected
        model.login(1, "me".into(), "secret".into(), false);
        assert!(model.state.connecting.is_empty());
        assert!(!model.deviceConnected(1));
        assert_eq!(device_names(&model), ["phone [connected]", "a [offline]"]);
        assert_eq!(model.deviceText(1, "password".into()).to_string(), "secret");
        model.trustHost(1, "SHA256:abc".into());
        assert_eq!(model.deviceText(1, "fingerprint".into()).to_string(), "SHA256:abc");
        // nothing to close, nothing to probe successfully
        model.disconnect(1);
        model.disconnect(0);
        model.probe();
        assert_eq!(device_names(&model), ["phone [connected]", "a [offline]"]);
        // files of a device that is not connected cannot be reached
        model.createFolder(1, "x".into());
        assert_eq!(names(&model), ["phone", "a"]);
    }

    #[test]
    fn probing_finds_devices_that_answer() {
        let (_files, mut model) = sample();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port() as i32;
        model.addDevice("up".into(), "127.0.0.1".into(), port, "me".into(), "".into(), false);
        model.addDevice("down".into(), "127.0.0.1".into(), 1, "me".into(), "".into(), false);
        assert_eq!(device_names(&model), ["phone [connected]", "up [online]", "down [offline]"]);
        drop(listener);
        model.probe();
        // it goes last of its new category
        assert_eq!(device_names(&model), ["phone [connected]", "down [offline]", "up [offline]"]);
        assert_eq!(model.deviceIdAt(1), 2);
        assert_eq!(model.deviceRow(2), 1);
        assert_eq!(model.deviceRow(1), 2);
        assert_eq!(model.deviceIdAt(99), -1);
        assert_eq!(model.deviceRow(99), -1);
    }

    #[test]
    fn devices_seen_on_the_network_are_listed_online() {
        let dir = TempDir::new();
        let path = dir.path().join("config.ini");
        let (_files, mut model) = sample();
        model.state.config = Config::load(&path);
        let found = |address: &str, name: &str| Found {
            address: address.parse().unwrap(),
            port: 22,
            name: name.to_owned(),
            kind: DeviceKind::Sftp,
        };
        // nothing is known of the network in tests until told
        model.discover();
        assert_eq!(model.row_count(), 1);

        model.addDevice("NAS".into(), "nas.local".into(), 1, "me".into(), "".into(), false);
        model.state.inline_found =
            Some(vec![found("10.9.9.5", "nas.local"), found("10.9.9.7", "pi.local"), found("10.9.9.8", "10.9.9.8")]);
        model.discover();
        model.discover();
        // the remembered device is not listed twice
        assert_eq!(
            device_names(&model),
            ["phone [connected]", "pi.local [online]", "10.9.9.8 [online]", "NAS [offline]"]
        );
        assert_eq!(model.deviceText(1, "host".into()).to_string(), "10.9.9.7");
        assert_eq!(model.deviceText(1, "user".into()).to_string(), "");
        // not remembered until connected or edited
        assert_eq!(Device::load(&Config::load(&path)).len(), 1);

        // editing one keeps it, even when it is not seen anymore
        model.updateDevice(2, "Printer".into(), "10.9.9.8".into(), 22, "me".into(), "".into(), false, "".into());
        model.state.inline_found = Some(vec![]);
        model.discover();
        assert_eq!(device_names(&model), ["phone [connected]", "Printer [online]", "NAS [offline]"]);
        assert_eq!(Device::load(&Config::load(&path)).len(), 2);

        // connecting one asks for the login, as nothing is known of it
        model.state.inline_found = Some(vec![found("10.9.9.7", "pi.local")]);
        model.discover();
        let pi = model.deviceRow(model.next_id() as i32 - 1);
        model.toggle(pi);
        assert!(model.state.connecting.is_empty());
        assert!(!model.deviceConnected(pi));
    }

    #[test]
    fn transfer_can_be_cancelled() {
        let (dir, mut model) = sample();
        open(&mut model);
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        model.cancelTransfer();
        assert_eq!(model.pending_count(), 0);
        model.transferTo(row_of(&model, "etc"));
        model.cancelTransfer();
        assert!(dir.path().join("home/user/notes.txt").exists());
        assert!(!dir.path().join("etc/notes.txt").exists());
        // an unknown destination keeps the items waiting
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.startTransfer(true);
        model.transferTo(99);
        assert_eq!(model.pending_count(), 1);
    }

    #[test]
    fn values_convert_to_qt() {
        assert_eq!(QVariant::from(Value::Text("abc".into())).to_qbytearray().to_string(), "abc");
        assert_eq!(QVariant::from(Value::Int(42)).to_qbytearray().to_string(), "42");
        assert_eq!(QVariant::from(Value::Number(1.5)).to_qbytearray().to_string(), "1.5");
        assert!(QVariant::from(Value::Bool(true)).to_bool());
        assert!(!QVariant::from(Value::Bool(false)).to_bool());
    }
}
