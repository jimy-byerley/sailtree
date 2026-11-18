//! the list model shown by the main page: devices, with their folders and files below them

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use qmetaobject::{
    qt_base_class, qt_method, qt_property, qt_signal, QAbstractListModel, QByteArray, QModelIndex,
    QObject, QPointer, QString, QVariant, USER_ROLE,
};

use super::node::{next_towards, Row, RowKind, Step, Tree};
use super::selection::Selection;
use crate::devices::{Device, DeviceId};
use crate::fs::{mode_to_string, Entry, Fs, FsError};
use crate::runtime;

/// data exposed for each row, in the order of their Qt role numbers
const ROLES: [&str; 16] = [
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
    /// folder to expand down to, on the first expansion of a device
    reveal: Option<(DeviceId, PathBuf)>,
    /// devices that already had their first expansion
    opened: HashSet<DeviceId>,
    /// run filesystem operations before returning instead of in the background (unit tests)
    inline: bool,
}

impl Default for State {
    fn default() -> Self {
        State::new(vec![Device::local()])
    }
}

impl State {
    fn new(devices: Vec<Device>) -> Self {
        let mut tree = Tree::default();
        for device in &devices {
            for step in tree.plan_add_device(device.id, &device.name, &device.root) {
                tree.apply(step);
            }
        }
        State {
            tree,
            devices,
            selection: Selection::default(),
            reveal: None,
            opened: HashSet::new(),
            inline: false,
        }
    }

    fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.iter().find(|device| device.id == id)
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

    /// number of selected items
    selectionCount: qt_property!(i32; READ selection_count NOTIFY selectionChanged),
    /// row of the selected item when there is exactly one, -1 otherwise
    selectedRow: qt_property!(i32; READ selected_row NOTIFY selectionChanged),
    selectionChanged: qt_signal!(),

    /// an operation failed, with a message for the user
    failed: qt_signal!(message: QString),
    /// this row should be brought into view, at the top of the view if `top`
    revealed: qt_signal!(row: i32, top: bool),

    /// expand or collapse a device or folder
    toggle: qt_method!(fn(&mut self, row: i32)),
    /// list an expanded device or folder again
    refresh: qt_method!(fn(&mut self, row: i32)),
    /// rename a file or folder inside its folder
    rename: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// create a folder in the device or folder at `row`, or next to the file at `row`.
    /// a negative row means the home folder of the first device
    createFolder: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// create an empty file, same destination rules as `createFolder`
    createFile: qt_method!(fn(&mut self, row: i32, name: QString)),
    /// set the permission bits of a file or folder
    chmod: qt_method!(fn(&mut self, row: i32, mode: i32)),
    /// name of the item at `row`, empty for an unknown row
    nameAt: qt_method!(fn(&self, row: i32) -> QString),
    /// permission bits of the item at `row`
    modeAt: qt_method!(fn(&self, row: i32) -> i32),
    /// select or unselect a file or folder
    toggleSelected: qt_method!(fn(&mut self, row: i32)),
    clearSelection: qt_method!(fn(&mut self)),
    /// delete the selected items, folders with their content
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
        let row = self.row(row)?;
        let entry = row.entry.as_ref();
        let text = |text: &str| Value::Text(text.to_owned());
        Some(match role {
            "name" => text(&row.name),
            "depth" => Value::Int(row.depth as i32),
            "kind" => text(match row.kind {
                RowKind::Device => "device",
                RowKind::Folder => "folder",
                RowKind::File => "file",
                RowKind::Loading => "loading",
                RowKind::Error => "error",
            }),
            "expanded" => Value::Bool(row.expanded),
            "hasChildren" => Value::Bool(row.expandable()),
            "selectable" => Value::Bool(row.is_item()),
            "selected" => Value::Bool(row.is_item() && self.state.selection.contains(row.device, &row.path)),
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
        self.state.device(device).map(|device| device.fs.clone())
    }

    fn report(&mut self, error: impl ToString) {
        self.failed(QString::from(error.to_string()));
    }

    fn toggle(&mut self, row: i32) {
        let Some(current) = self.row(row) else { return };
        if !current.expandable() {
            return;
        }
        let (device, path, kind) = (current.device, current.path.clone(), current.kind);
        if current.expanded {
            let steps = self.state.tree.plan_collapse(row as usize);
            self.run(steps);
            // what is not displayed anymore is not kept selected
            if self.state.selection.removed_inside(device, &path) {
                self.selectionChanged();
            }
            return;
        }
        if kind == RowKind::Device && self.state.opened.insert(device) {
            if let Some(home) = self.state.device(device).map(|device| device.home.clone()) {
                self.state.reveal = Some((device, home));
            }
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
        let tree = &self.state.tree;
        if self.state.selection.retain(|device, path| tree.find(device, path).is_some()) {
            self.selectionChanged();
        }
        self.continue_reveal(device, path);
    }

    /// after the listing of `path`: expand the next folder towards the one to reveal
    fn continue_reveal(&mut self, device: DeviceId, path: &Path) {
        let Some((reveal_device, target)) = self.state.reveal.clone() else { return };
        if reveal_device != device || !target.starts_with(path) {
            return;
        }
        let next = next_towards(path, &target).and_then(|next| self.state.tree.find(device, &next));
        match next {
            Some(row) if self.state.tree.rows()[row].expandable() => self.expand(row),
            // arrived, or the way is not visible: show how far we went
            _ => {
                self.state.reveal = None;
                if let Some(row) = self.state.tree.find(device, path) {
                    self.revealed(row as i32, true);
                }
            }
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
        if row < 0 {
            let device = self.state.devices.first()?;
            return Some((device.id, device.home.clone()));
        }
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
                    // make the new item visible when its folder was collapsed
                    if let Some(row) = this.state.tree.find(device, &parent) {
                        this.expand(row);
                    }
                }
                Err(error) => this.report(error),
            },
        );
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
        if !current.is_item() {
            return;
        }
        let (device, path) = (current.device, current.path.clone());
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
                for (device, path, result) in results {
                    match result {
                        // already gone is as good as deleted
                        Ok(()) | Err(FsError::NotFound(_)) => {
                            let steps = this.state.tree.plan_remove(device, &path);
                            this.run(steps);
                            this.state.selection.removed(device, &path);
                        }
                        Err(error) => errors.push(error.to_string()),
                    }
                }
                this.selectionChanged();
                if let Some(first) = errors.first() {
                    let message = match errors.len() {
                        1 => first.clone(),
                        count => format!("{first} (and {} more)", count - 1),
                    };
                    this.report(message);
                }
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
        let tree = &self.state.tree;
        let unselected = self.state.selection.retain(|device, path| tree.find(device, path).is_some());
        self.end_reset_model();
        self.showHiddenChanged();
        if unselected {
            self.selectionChanged();
        }
    }

    fn selection_count(&self) -> i32 {
        self.state.selection.count() as i32
    }

    fn selected_row(&self) -> i32 {
        self.state
            .selection
            .single()
            .and_then(|(device, path)| self.state.tree.find(*device, path))
            .map_or(-1, |row| row as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::{DeviceKind, State as DeviceState};
    use crate::fs::local::Local;
    use crate::testing::TempDir;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// temp/ { home/user/ { docs/ { report.txt }, .config/, notes.txt }, etc/ { hosts } }
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
            fs: Fs::Local(Local),
            root: root.to_owned(),
            home: root.join("home/user"),
        };
        let mut model = TreeModel::default();
        model.state = State::new(vec![device]);
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
        assert_eq!(text(&model, 0, "path"), "/");
        assert_eq!(model.value(0, "expanded"), Some(Value::Bool(false)));
        assert_eq!(model.value(0, "hasChildren"), Some(Value::Bool(true)));
        assert_eq!(model.value(0, "selectable"), Some(Value::Bool(false)));
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
    fn first_expansion_of_a_device_goes_down_to_home() {
        let (_dir, mut model) = sample();
        model.toggle(0);
        assert_eq!(
            names(&model),
            ["phone", "  etc", "  home", "    user", "      docs", "      notes.txt"]
        );
        assert_eq!(model.value(3, "expanded"), Some(Value::Bool(true)));
        assert_eq!(model.value(1, "expanded"), Some(Value::Bool(false)));
        assert_eq!(text(&model, 4, "category"), "connected");

        // only the first time
        model.toggle(0);
        assert_eq!(names(&model), ["phone"]);
        model.toggle(0);
        assert_eq!(names(&model), ["phone", "  etc", "  home"]);
    }

    #[test]
    fn details_of_files_and_folders() {
        let (dir, mut model) = sample();
        fs::set_permissions(dir.path().join("home/user/notes.txt"), fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink("notes.txt", dir.path().join("home/user/shortcut")).unwrap();
        model.toggle(0);

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
        model.toggle(0);
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
        model.toggle(0);
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
        model.toggle(0);
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
        model.toggle(0);
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

        // devices and out of range rows are not selectable
        model.toggleSelected(0);
        model.toggleSelected(99);
        assert_eq!(model.selection_count(), 2);

        model.toggleSelected(notes);
        assert_eq!(model.value(notes, "selected"), Some(Value::Bool(false)));
        model.clearSelection();
        assert_eq!(model.selection_count(), 0);
    }

    #[test]
    fn collapsing_unselects_what_gets_hidden() {
        let (_dir, mut model) = sample();
        model.toggle(0);
        model.toggleSelected(row_of(&model, "notes.txt"));
        model.toggleSelected(row_of(&model, "user"));
        model.toggle(row_of(&model, "user"));
        assert_eq!(model.selection_count(), 1);
        assert_eq!(model.selected_row(), row_of(&model, "user"));
    }

    #[test]
    fn rename_moves_the_row_and_keeps_it_selected() {
        let (dir, mut model) = sample();
        model.toggle(0);
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
        model.toggle(0);
        model.toggle(row_of(&model, "docs"));
        model.rename(row_of(&model, "docs"), "papers".into());
        // the renamed folder comes back collapsed
        assert_eq!(names(&model)[4..], ["      papers", "      notes.txt"]);
        assert!(dir.path().join("home/user/papers/report.txt").exists());
    }

    #[test]
    fn rename_failure_is_reported_and_changes_nothing() {
        let (dir, mut model) = sample();
        model.toggle(0);
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
        model.toggle(0);
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
    fn create_next_to_a_file_or_in_home() {
        let (dir, mut model) = sample();
        model.toggle(0);
        model.createFile(row_of(&model, "notes.txt"), "sibling".into());
        assert!(dir.path().join("home/user/sibling").is_file());
        model.createFolder(-1, "from-top-bar".into());
        assert!(dir.path().join("home/user/from-top-bar").is_dir());
        assert_eq!(
            names(&model)[4..],
            ["      docs", "      from-top-bar", "      notes.txt", "      sibling"]
        );
    }

    #[test]
    fn create_in_a_collapsed_folder_expands_it() {
        let (dir, mut model) = sample();
        model.toggle(0);
        model.createFile(row_of(&model, "etc"), "fstab".into());
        assert!(dir.path().join("etc/fstab").is_file());
        assert_eq!(names(&model)[1..4], ["  etc", "    fstab", "    hosts"]);
    }

    #[test]
    fn create_failure_changes_nothing() {
        let (_dir, mut model) = sample();
        model.toggle(0);
        let before = names(&model);
        model.createFolder(row_of(&model, "user"), "docs".into());
        model.createFile(row_of(&model, "user"), "".into());
        assert_eq!(names(&model), before);
    }

    #[test]
    fn chmod_updates_the_row() {
        let (dir, mut model) = sample();
        model.toggle(0);
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
        model.toggle(0);
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
        model.toggle(0);
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
        model.toggle(0);
        let notes = row_of(&model, "notes.txt");
        assert_eq!(model.nameAt(notes).to_string(), "notes.txt");
        assert_eq!(model.modeAt(notes), 0o604);
        assert_eq!(model.nameAt(0).to_string(), "phone");
        assert_eq!(model.modeAt(0), 0);
        assert_eq!(model.nameAt(99).to_string(), "");
        assert_eq!(model.modeAt(-1), 0);
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
