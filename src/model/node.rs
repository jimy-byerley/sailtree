//! the tree of devices, folders and files, kept as the flat list of its visible rows.
//!
//! a row is followed by its visible descendants, which have a greater depth.
//! changes are first planned as a list of [Step]s, then applied one by one, so the Qt model
//! can announce each of them to the views.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::devices::DeviceId;
use crate::fs::{Entry, Kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Device,
    Folder,
    File,
    /// placeholder shown while the content of a folder is being listed
    Loading,
    /// placeholder shown when the content of a folder could not be listed
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub device: DeviceId,
    /// path of the item on its device. a device has the path of its current folder,
    /// placeholders the path of the folder they stand in
    pub path: PathBuf,
    /// 0 for devices, parent depth + 1 otherwise
    pub depth: u32,
    pub kind: RowKind,
    /// device name, file or folder name, or error message
    pub name: String,
    /// details of a file or folder
    pub entry: Option<Entry>,
    pub expanded: bool,
    /// hidden items of an expanded folder that are not shown
    stash: Vec<Entry>,
}

/// what an item weighs in its folder: files have their size, the content of folders is not counted
fn weight(entry: &Entry) -> u64 {
    match entry.kind {
        Kind::File => entry.size,
        Kind::Folder => 0,
    }
}

impl Row {
    pub fn device(device: DeviceId, name: &str, current: &Path) -> Self {
        Row {
            device,
            path: current.to_owned(),
            depth: 0,
            kind: RowKind::Device,
            name: name.to_owned(),
            entry: None,
            expanded: false,
            stash: Vec::new(),
        }
    }

    fn child(parent: &Row, entry: Entry) -> Self {
        Row {
            device: parent.device,
            path: parent.path.join(&entry.name),
            depth: parent.depth + 1,
            kind: match entry.kind {
                Kind::Folder => RowKind::Folder,
                Kind::File => RowKind::File,
            },
            name: entry.name.clone(),
            entry: Some(entry),
            expanded: false,
            stash: Vec::new(),
        }
    }

    fn placeholder(parent: &Row, kind: RowKind, name: &str) -> Self {
        Row {
            device: parent.device,
            path: parent.path.clone(),
            depth: parent.depth + 1,
            kind,
            name: name.to_owned(),
            entry: None,
            expanded: false,
            stash: Vec::new(),
        }
    }

    /// can have children
    pub fn expandable(&self) -> bool {
        matches!(self.kind, RowKind::Device | RowKind::Folder)
    }

    /// a file or folder, on which file operations make sense
    pub fn is_item(&self) -> bool {
        matches!(self.kind, RowKind::Folder | RowKind::File)
    }

    pub fn is_placeholder(&self) -> bool {
        matches!(self.kind, RowKind::Loading | RowKind::Error)
    }
}

/// one change of the row list. indices refer to the list as it is when the step gets applied
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Insert { at: usize, rows: Vec<Row> },
    Remove { first: usize, count: usize },
    Update { row: usize, new: Row },
}

/// what orders the items of a folder
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    /// most recently modified first
    Date,
    /// biggest files first. folders have no size of their own and stay ordered by name
    Size,
}

/// display order inside a folder: folders first, then by the sort criterion,
/// then by name ignoring case
pub fn order(sort: Sort, a: &Entry, b: &Entry) -> Ordering {
    let criterion = match sort {
        Sort::Name => Ordering::Equal,
        Sort::Date => b.mtime.cmp(&a.mtime),
        Sort::Size if a.kind == Kind::Folder => Ordering::Equal,
        Sort::Size => b.size.cmp(&a.size),
    };
    (a.kind != Kind::Folder)
        .cmp(&(b.kind != Kind::Folder))
        .then(criterion)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

/// a run of sibling rows, each followed by its descendants, with every level put in order
fn reordered(rows: &[Row], sort: Sort) -> Vec<Row> {
    let Some(first) = rows.first() else { return Vec::new() };
    let depth = first.depth;
    let mut blocks: Vec<&[Row]> = Vec::new();
    let mut start = 0;
    for index in 1..=rows.len() {
        if index == rows.len() || rows[index].depth == depth {
            blocks.push(&rows[start..index]);
            start = index;
        }
    }
    // devices keep their order, placeholders are alone in their folder
    blocks.sort_by(|a, b| match (&a[0].entry, &b[0].entry) {
        (Some(a), Some(b)) => order(sort, a, b),
        _ => Ordering::Equal,
    });
    let mut result = Vec::with_capacity(rows.len());
    for block in blocks {
        result.push(block[0].clone());
        result.extend(reordered(&block[1..], sort));
    }
    result
}

#[derive(Debug, Default)]
pub struct Tree {
    rows: Vec<Row>,
    show_hidden: bool,
    sort: Sort,
}

impl Tree {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// index of the row of a device
    pub fn find_device(&self, device: DeviceId) -> Option<usize> {
        self.rows.iter().position(|row| row.device == device && row.kind == RowKind::Device)
    }

    /// index of the folder or file at this path if it is visible,
    /// or of the device having this path as current folder
    pub fn find(&self, device: DeviceId, path: &Path) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row.device == device && !row.is_placeholder() && row.path == path)
    }

    /// number of visible rows below `row` that belong to it
    pub fn descendants(&self, row: usize) -> usize {
        let depth = self.rows[row].depth;
        self.rows[row + 1..].iter().take_while(|other| other.depth > depth).count()
    }

    /// indices of the direct children of `row`
    fn children(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        let depth = self.rows[row].depth + 1;
        (row + 1..row + 1 + self.descendants(row)).filter(move |&index| self.rows[index].depth == depth)
    }

    /// where a new child of `parent` goes to keep the display order
    fn child_position(&self, parent: usize, entry: &Entry) -> usize {
        self.children(parent)
            .find(|&index| match &self.rows[index].entry {
                Some(other) => order(self.sort, entry, other) == Ordering::Less,
                None => false,
            })
            .unwrap_or(parent + 1 + self.descendants(parent))
    }

    pub fn apply(&mut self, step: Step) {
        match step {
            Step::Insert { at, rows } => {
                self.rows.splice(at..at, rows);
            }
            Step::Remove { first, count } => {
                self.rows.drain(first..first + count);
            }
            Step::Update { row, new } => self.rows[row] = new,
        }
    }

    /// add a device after the existing ones, with the folder its tree starts from
    pub fn plan_add_device(&self, device: DeviceId, name: &str, current: &Path) -> Vec<Step> {
        vec![Step::Insert { at: self.rows.len(), rows: vec![Row::device(device, name, current)] }]
    }

    /// change the folder the tree of a device starts from. the device ends collapsed
    pub fn plan_set_current(&self, device: DeviceId, current: &Path) -> Vec<Step> {
        let Some(row) = self.find_device(device) else { return Vec::new() };
        let mut steps = Vec::new();
        let count = self.descendants(row);
        if count > 0 {
            steps.push(Step::Remove { first: row + 1, count });
        }
        let mut new = self.rows[row].clone();
        new.path = current.to_owned();
        new.expanded = false;
        new.stash.clear();
        steps.push(Step::Update { row, new });
        steps
    }

    /// mark a device or folder as expanded, with a placeholder until [Tree::plan_loaded]
    pub fn plan_expand(&self, row: usize) -> Vec<Step> {
        let Some(current) = self.rows.get(row) else { return Vec::new() };
        if !current.expandable() || current.expanded {
            return Vec::new();
        }
        let mut new = current.clone();
        new.expanded = true;
        vec![
            Step::Update { row, new },
            Step::Insert { at: row + 1, rows: vec![Row::placeholder(current, RowKind::Loading, "")] },
        ]
    }

    /// replace what is under an expanded folder by the result of its listing.
    /// nothing happens if the folder was collapsed or removed meanwhile
    pub fn plan_loaded(
        &self,
        device: DeviceId,
        path: &Path,
        listing: Result<Vec<Entry>, String>,
    ) -> Vec<Step> {
        let Some(row) = self.find(device, path) else { return Vec::new() };
        let parent = &self.rows[row];
        if !parent.expanded {
            return Vec::new();
        }
        let mut steps = Vec::new();
        let previous = self.descendants(row);
        if previous > 0 {
            steps.push(Step::Remove { first: row + 1, count: previous });
        }
        let mut new = parent.clone();
        new.stash.clear();
        let rows = match listing {
            Err(message) => vec![Row::placeholder(parent, RowKind::Error, &message)],
            Ok(mut entries) => {
                entries.sort_by(|a, b| order(self.sort, a, b));
                let (hidden, shown): (Vec<Entry>, Vec<Entry>) =
                    entries.into_iter().partition(|entry| entry.hidden() && !self.show_hidden);
                new.stash = hidden;
                shown.into_iter().map(|entry| Row::child(parent, entry)).collect()
            }
        };
        steps.push(Step::Update { row, new });
        if !rows.is_empty() {
            steps.push(Step::Insert { at: row + 1, rows });
        }
        steps
    }

    /// hide what is under an expanded device or folder
    pub fn plan_collapse(&self, row: usize) -> Vec<Step> {
        let Some(current) = self.rows.get(row) else { return Vec::new() };
        if !current.expanded {
            return Vec::new();
        }
        let mut steps = Vec::new();
        let count = self.descendants(row);
        if count > 0 {
            steps.push(Step::Remove { first: row + 1, count });
        }
        let mut new = current.clone();
        new.expanded = false;
        new.stash.clear();
        steps.push(Step::Update { row, new });
        steps
    }

    /// share of the files directly in its folder taken by a file, between 0 and 1.
    /// hidden files are part of the folder even when they are not shown. folders have none:
    /// their content is not counted
    pub fn ratio(&self, row: usize) -> Option<f64> {
        let entry = self.rows.get(row)?.entry.as_ref()?;
        if entry.kind != Kind::File {
            return None;
        }
        let parent = self.parent(row)?;
        let shown = self.children(parent).filter_map(|index| self.rows[index].entry.as_ref());
        let total: u64 = shown.chain(&self.rows[parent].stash).map(weight).sum();
        Some(if total == 0 { 0.0 } else { entry.size as f64 / total as f64 })
    }

    /// forget a file or folder that does not exist anymore
    pub fn plan_remove(&self, device: DeviceId, path: &Path) -> Vec<Step> {
        if let Some(row) = self.find(device, path) {
            if self.rows[row].kind == RowKind::Device {
                return Vec::new();
            }
            return vec![Step::Remove { first: row, count: 1 + self.descendants(row) }];
        }
        // it may be a hidden item that is not shown
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return Vec::new() };
        let Some(row) = self.find(device, parent) else { return Vec::new() };
        let current = &self.rows[row];
        if !current.stash.iter().any(|entry| *entry.name == *name) {
            return Vec::new();
        }
        let mut new = current.clone();
        new.stash.retain(|entry| *entry.name != *name);
        vec![Step::Update { row, new }]
    }

    /// show a file or folder of `folder` that was created or changed.
    /// nothing happens if the folder content is not displayed
    pub fn plan_upsert(&self, device: DeviceId, folder: &Path, entry: Entry) -> Vec<Step> {
        let mut steps = Vec::new();
        // number of rows removed before the insertion point, if the item has to be replaced
        let mut removed = (0, 0);
        if let Some(row) = self.find(device, &folder.join(&entry.name)) {
            let current = &self.rows[row];
            let same_kind = current.entry.as_ref().map(|old| old.kind) == Some(entry.kind);
            if same_kind {
                let mut new = current.clone();
                new.entry = Some(entry);
                return vec![Step::Update { row, new }];
            }
            // a folder replaced by a file or the reverse: start over
            removed = (row, 1 + self.descendants(row));
            steps.push(Step::Remove { first: removed.0, count: removed.1 });
        }
        let Some(parent) = self.find(device, folder) else { return steps };
        let current = &self.rows[parent];
        if !current.expanded || self.children(parent).any(|index| self.rows[index].is_placeholder()) {
            return steps;
        }
        if entry.hidden() && !self.show_hidden {
            let mut new = current.clone();
            new.stash.retain(|other| other.name != entry.name);
            new.stash.push(entry);
            steps.push(Step::Update { row: parent, new });
            return steps;
        }
        let mut at = self.child_position(parent, &entry);
        if at > removed.0 {
            at -= removed.1;
        }
        steps.push(Step::Insert { at, rows: vec![Row::child(current, entry)] });
        steps
    }

    /// index of the device or folder containing `row`
    pub fn parent(&self, row: usize) -> Option<usize> {
        let depth = self.rows[row].depth;
        (0..row).rev().find(|&index| self.rows[index].depth < depth)
    }

    /// change the order of the items in all expanded folders.
    /// this moves rows all over the list: views must be reset around it
    pub fn set_sort(&mut self, sort: Sort) {
        if sort != self.sort {
            self.sort = sort;
            self.rows = reordered(&self.rows, sort);
        }
    }

    /// show or hide the hidden items of all expanded folders.
    /// this changes rows all over the list: views must be reset around it
    pub fn set_show_hidden(&mut self, show: bool) {
        if show == self.show_hidden {
            return;
        }
        self.show_hidden = show;
        let mut index = 0;
        while index < self.rows.len() {
            if show {
                for entry in std::mem::take(&mut self.rows[index].stash) {
                    let at = self.child_position(index, &entry);
                    let row = Row::child(&self.rows[index], entry);
                    self.rows.insert(at, row);
                }
            } else if self.rows[index].entry.as_ref().is_some_and(Entry::hidden) {
                let parent = self.parent(index).expect("files and folders have a parent");
                let count = 1 + self.descendants(index);
                let removed = self.rows.drain(index..index + count).next().expect("at least the row");
                self.rows[parent].stash.extend(removed.entry);
                continue;
            }
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: DeviceId = 0;

    fn entry(name: &str, kind: Kind) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            link_target: None,
            size: 0,
            mtime: 0,
            owner: "user".into(),
            group: "user".into(),
            mode: 0o644,
        }
    }

    fn file(name: &str) -> Entry {
        entry(name, Kind::File)
    }

    fn folder(name: &str) -> Entry {
        entry(name, Kind::Folder)
    }

    fn run(tree: &mut Tree, steps: Vec<Step>) {
        for step in steps {
            tree.apply(step);
            check(tree);
        }
    }

    /// structural rules every row list must follow
    fn check(tree: &Tree) {
        let mut previous: Option<&Row> = None;
        for row in tree.rows() {
            match previous {
                None => assert_eq!(row.depth, 0),
                Some(previous) => {
                    assert!(row.depth <= previous.depth + 1, "depth jump at {row:?}");
                    if row.depth > previous.depth {
                        assert!(previous.expanded, "child of a collapsed row: {row:?}");
                    }
                }
            }
            assert_eq!(row.depth == 0, row.kind == RowKind::Device);
            previous = Some(row);
        }
    }

    /// compact picture of the tree: one `depth:name` per row, `+` marks expanded rows
    fn picture(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|row| {
                let name = match row.kind {
                    RowKind::Loading => "<loading>".to_owned(),
                    RowKind::Error => format!("<error {}>", row.name),
                    _ => row.name.clone(),
                };
                format!("{}:{}{}", row.depth, name, if row.expanded { "+" } else { "" })
            })
            .collect()
    }

    fn tree_with_device() -> Tree {
        let mut tree = Tree::default();
        let steps = tree.plan_add_device(DEVICE, "phone", Path::new("/"));
        run(&mut tree, steps);
        tree
    }

    fn expand(tree: &mut Tree, path: &str, entries: Vec<Entry>) {
        let row = tree.find(DEVICE, Path::new(path)).expect(path);
        let steps = tree.plan_expand(row);
        run(tree, steps);
        let steps = tree.plan_loaded(DEVICE, Path::new(path), Ok(entries));
        run(tree, steps);
    }

    /// phone / { etc { hosts }, home { user { .config, notes.txt } }, readme }
    fn sample() -> Tree {
        let mut tree = tree_with_device();
        expand(&mut tree, "/", vec![file("readme"), folder("home"), folder("etc")]);
        expand(&mut tree, "/home", vec![folder("user")]);
        expand(&mut tree, "/home/user", vec![file("notes.txt"), folder(".config")]);
        expand(&mut tree, "/etc", vec![file("hosts")]);
        tree
    }

    #[test]
    fn devices_are_top_level_rows() {
        let mut tree = tree_with_device();
        let steps = tree.plan_add_device(1, "server", Path::new("/"));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone", "0:server"]);
        assert_eq!(tree.rows()[1].device, 1);
        assert!(tree.rows()[0].expandable());
        assert!(!tree.rows()[0].is_item());
    }

    #[test]
    fn expand_shows_a_placeholder_then_the_content() {
        let mut tree = tree_with_device();
        let steps = tree.plan_expand(0);
        assert!(matches!(steps[..], [Step::Update { row: 0, .. }, Step::Insert { at: 1, .. }]));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:<loading>"]);
        assert!(tree.rows()[1].is_placeholder());

        let listing = Ok(vec![file("b"), folder("a")]);
        let steps = tree.plan_loaded(DEVICE, Path::new("/"), listing);
        assert!(matches!(
            steps[..],
            [Step::Remove { first: 1, count: 1 }, Step::Update { row: 0, .. }, Step::Insert { at: 1, .. }]
        ));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:a", "1:b"]);
        assert_eq!(tree.rows()[1].path, Path::new("/a"));
        assert_eq!(tree.rows()[1].kind, RowKind::Folder);
        assert_eq!(tree.rows()[2].kind, RowKind::File);
        assert!(tree.rows()[2].is_item());
    }

    #[test]
    fn expand_is_refused_for_files_placeholders_and_expanded_rows() {
        let mut tree = sample();
        let readme = tree.find(DEVICE, Path::new("/readme")).unwrap();
        assert_eq!(tree.plan_expand(readme), []);
        assert_eq!(tree.plan_expand(0), []);
        assert_eq!(tree.plan_expand(999), []);

        let steps = tree.plan_collapse(0);
        run(&mut tree, steps);
        let steps = tree.plan_expand(0);
        run(&mut tree, steps);
        assert_eq!(tree.plan_expand(1), []);
    }

    #[test]
    fn nested_expansion_keeps_rows_in_order() {
        let tree = sample();
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:etc+", "2:hosts", "1:home+", "2:user+", "3:notes.txt", "1:readme"]
        );
        assert_eq!(tree.rows()[5].path, Path::new("/home/user/notes.txt"));
    }

    #[test]
    fn empty_folder_expands_to_nothing() {
        let mut tree = sample();
        let steps = tree.plan_collapse(tree.find(DEVICE, Path::new("/etc")).unwrap());
        run(&mut tree, steps);
        expand(&mut tree, "/etc", vec![]);
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:etc+", "1:home+", "2:user+", "3:notes.txt", "1:readme"]
        );
    }

    #[test]
    fn failed_listing_shows_an_error_row() {
        let mut tree = sample();
        let steps = tree.plan_collapse(tree.find(DEVICE, Path::new("/etc")).unwrap());
        run(&mut tree, steps);
        let etc = tree.find(DEVICE, Path::new("/etc")).unwrap();
        let steps = tree.plan_expand(etc);
        run(&mut tree, steps);
        let steps = tree.plan_loaded(DEVICE, Path::new("/etc"), Err("permission denied".into()));
        run(&mut tree, steps);
        assert_eq!(picture(&tree)[1..3], ["1:etc+", "2:<error permission denied>"]);
        // the placeholder is not found as the folder itself
        assert_eq!(tree.find(DEVICE, Path::new("/etc")), Some(etc));
        assert_eq!(tree.descendants(etc), 1);
    }

    #[test]
    fn listing_of_a_collapsed_or_unknown_folder_is_ignored() {
        let mut tree = tree_with_device();
        let steps = tree.plan_expand(0);
        run(&mut tree, steps);
        let steps = tree.plan_collapse(0);
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone"]);
        assert_eq!(tree.plan_loaded(DEVICE, Path::new("/"), Ok(vec![file("a")])), []);
        assert_eq!(tree.plan_loaded(DEVICE, Path::new("/nowhere"), Ok(vec![file("a")])), []);
        assert_eq!(tree.plan_loaded(7, Path::new("/"), Ok(vec![file("a")])), []);
    }

    #[test]
    fn collapse_removes_all_descendants() {
        let mut tree = sample();
        let home = tree.find(DEVICE, Path::new("/home")).unwrap();
        assert_eq!(tree.descendants(home), 2);
        let steps = tree.plan_collapse(home);
        assert!(matches!(steps[..], [Step::Remove { count: 2, .. }, Step::Update { .. }]));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:etc+", "2:hosts", "1:home", "1:readme"]);

        let steps = tree.plan_collapse(0);
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone"]);
        assert_eq!(tree.plan_collapse(0), []);
    }

    #[test]
    fn rows_map_to_the_right_items_after_many_changes() {
        let mut tree = sample();
        let steps = tree.plan_collapse(tree.find(DEVICE, Path::new("/etc")).unwrap());
        run(&mut tree, steps);
        let steps = tree.plan_collapse(tree.find(DEVICE, Path::new("/home/user")).unwrap());
        run(&mut tree, steps);
        expand(&mut tree, "/home/user", vec![file("z"), file("a"), folder("m")]);
        expand(&mut tree, "/home/user/m", vec![file("inside")]);
        expand(&mut tree, "/etc", vec![file("hosts"), file("fstab")]);

        assert_eq!(
            picture(&tree),
            [
                "0:phone+", "1:etc+", "2:fstab", "2:hosts", "1:home+", "2:user+", "3:m+",
                "4:inside", "3:a", "3:z", "1:readme"
            ]
        );
        for (index, row) in tree.rows().iter().enumerate() {
            assert_eq!(tree.find(DEVICE, &row.path), Some(index));
        }
        assert_eq!(tree.rows()[7].path, Path::new("/home/user/m/inside"));
        assert_eq!(tree.parent(7), Some(6));
        assert_eq!(tree.parent(6), Some(5));
        assert_eq!(tree.parent(10), Some(0));
        assert_eq!(tree.parent(0), None);
    }

    #[test]
    fn reloading_replaces_the_content() {
        let mut tree = sample();
        let steps = tree.plan_loaded(DEVICE, Path::new("/home"), Ok(vec![folder("user"), folder("guest")]));
        run(&mut tree, steps);
        // the nested expansion is gone with the old rows
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:etc+", "2:hosts", "1:home+", "2:guest", "2:user", "1:readme"]
        );
    }

    #[test]
    fn display_order() {
        let mut entries = vec![
            file("b.txt"),
            folder("Zeta"),
            file("A.txt"),
            folder("alpha"),
            file("a.txt"),
            file(".profile"),
            folder(".cache"),
        ];
        entries.sort_by(|a, b| order(Sort::Name, a, b));
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, [".cache", "alpha", "Zeta", ".profile", "A.txt", "a.txt", "b.txt"]);
    }

    #[test]
    fn hidden_items_are_filtered_by_default() {
        let tree = sample();
        assert!(!tree.show_hidden());
        assert_eq!(tree.find(DEVICE, Path::new("/home/user/.config")), None);
    }

    #[test]
    fn hidden_items_toggle() {
        let mut tree = sample();
        let before = picture(&tree);

        tree.set_show_hidden(true);
        check(&tree);
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:etc+", "2:hosts", "1:home+", "2:user+", "3:.config", "3:notes.txt", "1:readme"]
        );
        expand(&mut tree, "/home/user/.config", vec![file("settings"), file(".secret")]);
        assert_eq!(picture(&tree)[5..8], ["3:.config+", "4:.secret", "4:settings"]);

        tree.set_show_hidden(false);
        check(&tree);
        assert_eq!(picture(&tree), before);

        // shown again, but collapsed: its rows were dropped
        tree.set_show_hidden(true);
        check(&tree);
        assert_eq!(picture(&tree)[5..7], ["3:.config", "3:notes.txt"]);
        tree.set_show_hidden(true);
        assert_eq!(picture(&tree).len(), 8);
    }

    #[test]
    fn listing_while_hidden_items_are_shown() {
        let mut tree = tree_with_device();
        tree.set_show_hidden(true);
        expand(&mut tree, "/", vec![file(".b"), file("a")]);
        assert_eq!(picture(&tree), ["0:phone+", "1:.b", "1:a"]);
    }

    #[test]
    fn remove_takes_the_descendants_away() {
        let mut tree = sample();
        let steps = tree.plan_remove(DEVICE, Path::new("/home"));
        assert_eq!(steps, [Step::Remove { first: 3, count: 3 }]);
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:etc+", "2:hosts", "1:readme"]);

        assert_eq!(tree.plan_remove(DEVICE, Path::new("/home")), []);
        assert_eq!(tree.plan_remove(DEVICE, Path::new("/")), [], "devices are not removed this way");
    }

    #[test]
    fn remove_of_a_hidden_item_updates_the_stash() {
        let mut tree = sample();
        let steps = tree.plan_remove(DEVICE, Path::new("/home/user/.config"));
        assert!(matches!(steps[..], [Step::Update { .. }]));
        run(&mut tree, steps);
        tree.set_show_hidden(true);
        assert_eq!(tree.find(DEVICE, Path::new("/home/user/.config")), None);
    }

    #[test]
    fn upsert_inserts_at_the_sorted_position() {
        let mut tree = sample();
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), folder("zz"));
        run(&mut tree, steps);
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), file("aaa"));
        run(&mut tree, steps);
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), file("zzz"));
        run(&mut tree, steps);
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), folder("aa"));
        run(&mut tree, steps);
        let steps = tree.plan_upsert(DEVICE, Path::new("/home/user"), folder("docs"));
        run(&mut tree, steps);
        assert_eq!(
            picture(&tree),
            [
                "0:phone+", "1:aa", "1:etc+", "2:hosts", "1:home+", "2:user+", "3:docs",
                "3:notes.txt", "1:zz", "1:aaa", "1:readme", "1:zzz"
            ]
        );
        assert_eq!(tree.rows()[6].path, Path::new("/home/user/docs"));
        assert_eq!(tree.rows()[6].depth, 3);
    }

    #[test]
    fn upsert_into_an_empty_expanded_folder() {
        let mut tree = tree_with_device();
        expand(&mut tree, "/", vec![folder("empty"), file("after")]);
        expand(&mut tree, "/empty", vec![]);
        let steps = tree.plan_upsert(DEVICE, Path::new("/empty"), file("new"));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:empty+", "2:new", "1:after"]);
    }

    #[test]
    fn upsert_updates_an_existing_item_in_place() {
        let mut tree = sample();
        let mut changed = folder("home");
        changed.mode = 0o700;
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), changed);
        assert!(matches!(steps[..], [Step::Update { row: 3, .. }]));
        run(&mut tree, steps);
        assert_eq!(tree.rows()[3].entry.as_ref().unwrap().mode, 0o700);
        // still expanded with its content
        assert_eq!(picture(&tree)[3..6], ["1:home+", "2:user+", "3:notes.txt"]);
    }

    #[test]
    fn upsert_replaces_an_item_that_changed_kind() {
        let mut tree = sample();
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), file("etc"));
        run(&mut tree, steps);
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:home+", "2:user+", "3:notes.txt", "1:etc", "1:readme"]
        );
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), folder("readme"));
        run(&mut tree, steps);
        assert_eq!(
            picture(&tree),
            ["0:phone+", "1:home+", "2:user+", "3:notes.txt", "1:readme", "1:etc"]
        );
    }

    #[test]
    fn upsert_is_ignored_when_the_folder_is_not_displayed() {
        let mut tree = sample();
        assert_eq!(tree.plan_upsert(DEVICE, Path::new("/var"), file("log")), []);
        let steps = tree.plan_collapse(tree.find(DEVICE, Path::new("/etc")).unwrap());
        run(&mut tree, steps);
        assert_eq!(tree.plan_upsert(DEVICE, Path::new("/etc"), file("fstab")), []);

        // still loading: the listing will bring the new item
        let steps = tree.plan_expand(tree.find(DEVICE, Path::new("/etc")).unwrap());
        run(&mut tree, steps);
        assert_eq!(tree.plan_upsert(DEVICE, Path::new("/etc"), file("fstab")), []);
    }

    #[test]
    fn upsert_of_a_hidden_item_goes_to_the_stash() {
        let mut tree = sample();
        let before = picture(&tree);
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), file(".hidden"));
        assert!(matches!(steps[..], [Step::Update { row: 0, .. }]));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), before);
        // twice the same name does not duplicate it
        let steps = tree.plan_upsert(DEVICE, Path::new("/"), file(".hidden"));
        run(&mut tree, steps);

        tree.set_show_hidden(true);
        let names = picture(&tree);
        assert_eq!(names.iter().filter(|name| *name == "1:.hidden").count(), 1);
        assert_eq!(names[names.len() - 2..], ["1:.hidden", "1:readme"]);
    }

    fn sized(name: &str, kind: Kind, size: u64, mtime: i64) -> Entry {
        Entry { size, mtime, ..entry(name, kind) }
    }

    fn sorted_names(sort: Sort) -> Vec<String> {
        let mut entries = vec![
            sized("old-big", Kind::File, 900, 10),
            sized("new-small", Kind::File, 1, 30),
            sized("middle", Kind::File, 50, 20),
            sized("twin", Kind::File, 50, 20),
            sized("b-folder", Kind::Folder, 4096, 5),
            sized("a-folder", Kind::Folder, 10, 40),
        ];
        entries.sort_by(|a, b| order(sort, a, b));
        entries.into_iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn sort_criteria() {
        assert_eq!(
            sorted_names(Sort::Name),
            ["a-folder", "b-folder", "middle", "new-small", "old-big", "twin"]
        );
        // newest first, folders still before files
        assert_eq!(
            sorted_names(Sort::Date),
            ["a-folder", "b-folder", "new-small", "middle", "twin", "old-big"]
        );
        // biggest first, folders by name
        assert_eq!(
            sorted_names(Sort::Size),
            ["a-folder", "b-folder", "old-big", "middle", "twin", "new-small"]
        );
    }

    #[test]
    fn changing_the_sort_reorders_every_level() {
        let mut tree = tree_with_device();
        let steps = tree.plan_add_device(1, "server", Path::new("/"));
        run(&mut tree, steps);
        expand(
            &mut tree,
            "/",
            vec![sized("a", Kind::File, 1, 1), sized("b", Kind::File, 9, 2), folder("docs")],
        );
        expand(
            &mut tree,
            "/docs",
            vec![sized("x", Kind::File, 5, 9), sized("y", Kind::File, 7, 3), sized(".h", Kind::File, 99, 99)],
        );
        assert_eq!(tree.sort(), Sort::Name);
        assert_eq!(picture(&tree), ["0:phone+", "1:docs+", "2:x", "2:y", "1:a", "1:b", "0:server"]);

        tree.set_sort(Sort::Size);
        check(&tree);
        assert_eq!(picture(&tree), ["0:phone+", "1:docs+", "2:y", "2:x", "1:b", "1:a", "0:server"]);
        assert_eq!(tree.rows()[2].path, Path::new("/docs/y"));

        tree.set_sort(Sort::Date);
        check(&tree);
        assert_eq!(picture(&tree), ["0:phone+", "1:docs+", "2:x", "2:y", "1:b", "1:a", "0:server"]);

        // new and revealed items follow the current sort
        let steps = tree.plan_upsert(DEVICE, Path::new("/docs"), sized("z", Kind::File, 0, 5));
        run(&mut tree, steps);
        tree.set_show_hidden(true);
        assert_eq!(picture(&tree)[1..6], ["1:docs+", "2:.h", "2:x", "2:z", "2:y"]);
        let steps = tree.plan_loaded(
            DEVICE,
            Path::new("/"),
            Ok(vec![sized("old", Kind::File, 0, 1), sized("new", Kind::File, 0, 2)]),
        );
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone+", "1:new", "1:old", "0:server"]);
    }

    #[test]
    fn ratio_of_a_file_among_the_files_of_its_folder() {
        let mut tree = tree_with_device();
        expand(
            &mut tree,
            "/",
            vec![
                sized("a", Kind::File, 30, 0),
                sized("b", Kind::File, 50, 0),
                sized("docs", Kind::Folder, 4096, 0),
                sized(".rc", Kind::File, 20, 0),
            ],
        );
        assert_eq!(picture(&tree), ["0:phone+", "1:docs", "1:a", "1:b"]);
        // the hidden file is part of the total, the folder is not
        assert_eq!(tree.ratio(2), Some(0.3));
        assert_eq!(tree.ratio(3), Some(0.5));
        // devices, folders and unknown rows have none
        assert_eq!(tree.ratio(0), None);
        assert_eq!(tree.ratio(1), None);
        assert_eq!(tree.ratio(99), None);

        tree.set_show_hidden(true);
        assert_eq!(tree.ratio(2), Some(0.2));
        assert_eq!(tree.ratio(3), Some(0.3));
    }

    #[test]
    fn ratios_are_per_folder_and_follow_changes() {
        let mut tree = tree_with_device();
        expand(&mut tree, "/", vec![folder("docs"), sized("a", Kind::File, 60, 0)]);
        expand(&mut tree, "/docs", vec![sized("x", Kind::File, 30, 0), sized("y", Kind::File, 10, 0)]);
        assert_eq!(picture(&tree), ["0:phone+", "1:docs+", "2:x", "2:y", "1:a"]);
        assert_eq!(tree.ratio(2), Some(0.75));
        assert_eq!(tree.ratio(3), Some(0.25));
        // alone among the files of its folder, whatever the folders next to it hold
        assert_eq!(tree.ratio(4), Some(1.0));

        let steps = tree.plan_upsert(DEVICE, Path::new("/docs"), sized("x", Kind::File, 90, 0));
        run(&mut tree, steps);
        assert_eq!(tree.ratio(2), Some(0.9));
        let steps = tree.plan_remove(DEVICE, Path::new("/docs/x"));
        run(&mut tree, steps);
        assert_eq!(tree.ratio(2), Some(1.0));
    }

    #[test]
    fn empty_files_have_null_ratios() {
        let mut tree = tree_with_device();
        expand(&mut tree, "/", vec![sized("empty", Kind::File, 0, 0)]);
        assert_eq!(tree.ratio(1), Some(0.0));
    }

    #[test]
    fn current_folder_of_a_device() {
        let mut tree = sample();
        let steps = tree.plan_add_device(1, "server", Path::new("/srv"));
        run(&mut tree, steps);
        assert_eq!(tree.find_device(1), Some(tree.rows().len() - 1));
        assert_eq!(tree.find_device(7), None);

        let steps = tree.plan_set_current(DEVICE, Path::new("/home/user"));
        assert!(matches!(steps[..], [Step::Remove { first: 1, count: 6 }, Step::Update { row: 0, .. }]));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone", "0:server"]);
        assert_eq!(tree.rows()[0].path, Path::new("/home/user"));
        assert_eq!(tree.rows()[1].path, Path::new("/srv"));

        // the tree of the device now starts from that folder
        expand(&mut tree, "/home/user", vec![file("notes.txt"), file(".profile")]);
        assert_eq!(picture(&tree), ["0:phone+", "1:notes.txt", "0:server"]);
        assert_eq!(tree.rows()[1].path, Path::new("/home/user/notes.txt"));

        // on a collapsed device only the path changes
        let steps = tree.plan_collapse(0);
        run(&mut tree, steps);
        let steps = tree.plan_set_current(DEVICE, Path::new("/"));
        assert!(matches!(steps[..], [Step::Update { row: 0, .. }]));
        run(&mut tree, steps);
        assert_eq!(picture(&tree), ["0:phone", "0:server"]);
        assert_eq!(tree.plan_set_current(7, Path::new("/")), []);
    }
}
