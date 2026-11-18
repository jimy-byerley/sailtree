//! the selected items, identified by device and path so they survive row changes

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::devices::DeviceId;

pub type Item = (DeviceId, PathBuf);

#[derive(Debug, Default)]
pub struct Selection {
    items: BTreeSet<Item>,
}

impl Selection {
    /// select the item if it was not, unselect it otherwise. returns whether it is now selected
    pub fn toggle(&mut self, device: DeviceId, path: &Path) -> bool {
        let item = (device, path.to_owned());
        if self.items.remove(&item) {
            false
        } else {
            self.items.insert(item);
            true
        }
    }

    pub fn contains(&self, device: DeviceId, path: &Path) -> bool {
        // avoids allocating for the common empty case, this is called for every displayed row
        !self.items.is_empty() && self.items.contains(&(device, path.to_owned()))
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// returns whether something was selected
    pub fn clear(&mut self) -> bool {
        let changed = !self.items.is_empty();
        self.items.clear();
        changed
    }

    /// the selected item when there is exactly one
    pub fn single(&self) -> Option<&Item> {
        if self.items.len() == 1 {
            self.items.iter().next()
        } else {
            None
        }
    }

    /// selected items without those inside another selected folder:
    /// acting on the folder already covers them
    pub fn top_level(&self) -> Vec<Item> {
        self.items
            .iter()
            .filter(|(device, path)| {
                !path
                    .ancestors()
                    .skip(1)
                    .any(|ancestor| self.items.contains(&(*device, ancestor.to_owned())))
            })
            .cloned()
            .collect()
    }

    /// follow an item that was renamed or moved, with everything selected inside it
    pub fn renamed(&mut self, device: DeviceId, old: &Path, new: &Path) {
        let moved: Vec<Item> = self
            .items
            .iter()
            .filter(|(item_device, path)| *item_device == device && path.starts_with(old))
            .cloned()
            .collect();
        for item in moved {
            let suffix = item.1.strip_prefix(old).expect("filtered on this prefix").to_owned();
            self.items.remove(&item);
            let path = if suffix.as_os_str().is_empty() { new.to_owned() } else { new.join(suffix) };
            self.items.insert((device, path));
        }
    }

    /// forget an item and everything selected inside it. returns whether the selection changed
    pub fn removed(&mut self, device: DeviceId, path: &Path) -> bool {
        self.retain(|item_device, item_path| !(item_device == device && item_path.starts_with(path)))
    }

    /// forget what is selected strictly inside a folder. returns whether the selection changed
    pub fn removed_inside(&mut self, device: DeviceId, folder: &Path) -> bool {
        self.retain(|item_device, item_path| {
            !(item_device == device && item_path != folder && item_path.starts_with(folder))
        })
    }

    /// keep only the items accepted by `keep`. returns whether the selection changed
    pub fn retain(&mut self, mut keep: impl FnMut(DeviceId, &Path) -> bool) -> bool {
        let before = self.items.len();
        self.items.retain(|(device, path)| keep(*device, path));
        self.items.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[Item]) -> Vec<(DeviceId, &str)> {
        items.iter().map(|(device, path)| (*device, path.to_str().unwrap())).collect()
    }

    #[test]
    fn toggle_selects_and_unselects() {
        let mut selection = Selection::default();
        assert!(!selection.contains(0, Path::new("/a")));
        assert!(selection.toggle(0, Path::new("/a")));
        assert!(selection.contains(0, Path::new("/a")));
        assert_eq!(selection.count(), 1);
        assert!(!selection.toggle(0, Path::new("/a")));
        assert!(!selection.contains(0, Path::new("/a")));
        assert_eq!(selection.count(), 0);
    }

    #[test]
    fn selection_spans_folders_and_devices() {
        let mut selection = Selection::default();
        selection.toggle(0, Path::new("/home/a"));
        selection.toggle(0, Path::new("/etc/b"));
        selection.toggle(1, Path::new("/home/a"));
        assert_eq!(selection.count(), 3);
        assert!(selection.contains(1, Path::new("/home/a")));
        assert!(!selection.contains(1, Path::new("/etc/b")));
        assert_eq!(selection.single(), None);
    }

    #[test]
    fn single_item() {
        let mut selection = Selection::default();
        assert_eq!(selection.single(), None);
        selection.toggle(2, Path::new("/a"));
        assert_eq!(selection.single(), Some(&(2, PathBuf::from("/a"))));
    }

    #[test]
    fn clear_reports_changes() {
        let mut selection = Selection::default();
        assert!(!selection.clear());
        selection.toggle(0, Path::new("/a"));
        assert!(selection.clear());
        assert_eq!(selection.count(), 0);
    }

    #[test]
    fn top_level_drops_nested_items() {
        let mut selection = Selection::default();
        selection.toggle(0, Path::new("/a"));
        selection.toggle(0, Path::new("/a/b"));
        selection.toggle(0, Path::new("/a/b/c"));
        selection.toggle(0, Path::new("/ab"));
        selection.toggle(1, Path::new("/a/b"));
        assert_eq!(paths(&selection.top_level()), [(0, "/a"), (0, "/ab"), (1, "/a/b")]);
    }

    #[test]
    fn rename_is_followed() {
        let mut selection = Selection::default();
        selection.toggle(0, Path::new("/a/old"));
        selection.toggle(0, Path::new("/a/old/inside/deep"));
        selection.toggle(0, Path::new("/a/older"));
        selection.toggle(1, Path::new("/a/old"));
        selection.renamed(0, Path::new("/a/old"), Path::new("/a/new"));

        assert!(selection.contains(0, Path::new("/a/new")));
        assert!(selection.contains(0, Path::new("/a/new/inside/deep")));
        assert!(selection.contains(0, Path::new("/a/older")));
        assert!(selection.contains(1, Path::new("/a/old")));
        assert!(!selection.contains(0, Path::new("/a/old")));
        assert_eq!(selection.count(), 4);
    }

    #[test]
    fn removal_forgets_the_item_and_its_content() {
        let mut selection = Selection::default();
        selection.toggle(0, Path::new("/a"));
        selection.toggle(0, Path::new("/a/b"));
        selection.toggle(0, Path::new("/ab"));
        selection.toggle(1, Path::new("/a"));

        assert!(selection.removed(0, Path::new("/a")));
        assert!(!selection.removed(0, Path::new("/a")));
        assert_eq!(paths(&selection.top_level()), [(0, "/ab"), (1, "/a")]);
    }

    #[test]
    fn removal_inside_keeps_the_folder() {
        let mut selection = Selection::default();
        selection.toggle(0, Path::new("/a"));
        selection.toggle(0, Path::new("/a/b"));
        selection.toggle(0, Path::new("/a/c/d"));

        assert!(selection.removed_inside(0, Path::new("/a")));
        assert!(!selection.removed_inside(0, Path::new("/a")));
        assert_eq!(paths(&selection.top_level()), [(0, "/a")]);
    }
}
