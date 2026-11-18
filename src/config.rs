//! what the app remembers between runs: settings and devices, in `~/.config/sailtree/config.ini`
//!
//! the file is made of `[section]` lines followed by `key=value` lines.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct Config {
    /// file the values are saved to, none to keep them in memory only
    path: Option<PathBuf>,
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

/// where the configuration of the user is
pub fn default_path() -> PathBuf {
    let folder = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    folder.join("sailtree/config.ini")
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some('n') => result.push('\n'),
                Some(other) => result.push(other),
                None => {}
            },
            other => result.push(other),
        }
    }
    result
}

impl Config {
    /// read the file, starting empty when it is missing or unreadable
    pub fn load(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Config { path: Some(path.to_owned()), sections: parse(&text) }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections.get(section)?.get(key).map(String::as_str)
    }

    pub fn flag(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key)? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    #[allow(dead_code)] // used with remote devices
    /// names of the sections starting with `prefix`, without it
    pub fn sections(&self, prefix: &str) -> Vec<String> {
        self.sections.keys().filter_map(|name| name.strip_prefix(prefix)).map(str::to_owned).collect()
    }

    /// set a value and save. failures to save are ignored: the app works without memory
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let values = self.sections.entry(section.to_owned()).or_default();
        if values.get(key).map(String::as_str) != Some(value) {
            values.insert(key.to_owned(), value.to_owned());
            self.save();
        }
    }

    #[allow(dead_code)] // used with remote devices
    pub fn remove(&mut self, section: &str, key: &str) {
        if self.sections.get_mut(section).and_then(|values| values.remove(key)).is_some() {
            self.save();
        }
    }

    #[allow(dead_code)] // used with remote devices
    pub fn remove_section(&mut self, section: &str) {
        if self.sections.remove(section).is_some() {
            self.save();
        }
    }

    fn text(&self) -> String {
        let mut text = String::new();
        for (name, values) in &self.sections {
            text.push_str(&format!("[{name}]\n"));
            for (key, value) in values {
                text.push_str(&format!("{key}={}\n", escape(value)));
            }
            text.push('\n');
        }
        text
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(folder) = path.parent() {
                fs::create_dir_all(folder)?;
            }
            // written aside then renamed, so a crash never leaves half a file.
            // private: it can hold passwords
            let aside = path.with_extension("ini.new");
            let _ = fs::remove_file(&aside);
            let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&aside)?;
            file.write_all(self.text().as_bytes())?;
            fs::rename(&aside, path)
        };
        let _ = write();
    }
}

fn parse(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            sections.entry(name.to_owned()).or_default();
            current = Some(name.to_owned());
        } else if let (Some(section), Some((key, value))) = (&current, line.split_once('=')) {
            sections.entry(section.clone()).or_default().insert(key.to_owned(), unescape(value));
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn values_survive_a_reload() {
        let dir = TempDir::new();
        let path = dir.path().join("deep/config.ini");
        let mut config = Config::load(&path);
        assert_eq!(config.get("settings", "sort"), None);
        config.set("settings", "sort", "date");
        config.set("settings", "hidden", "true");
        config.set("device nas", "folder", "/srv/my = files\nwith \\ odd name");
        config.set("device nas", "port", "22");

        let config = Config::load(&path);
        assert_eq!(config.get("settings", "sort"), Some("date"));
        assert_eq!(config.flag("settings", "hidden"), Some(true));
        assert_eq!(config.flag("settings", "sort"), None);
        assert_eq!(config.flag("settings", "missing"), None);
        assert_eq!(config.get("device nas", "folder"), Some("/srv/my = files\nwith \\ odd name"));
        assert_eq!(config.sections("device "), ["nas"]);
        assert_eq!(config.get("nowhere", "port"), None);
        // private file
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn removals_are_saved() {
        let dir = TempDir::new();
        let path = dir.path().join("config.ini");
        let mut config = Config::load(&path);
        config.set("device a", "port", "22");
        config.set("device a", "user", "me");
        config.set("device b", "port", "21");
        config.remove("device a", "user");
        config.remove("device a", "nothing");
        config.remove_section("device b");
        config.remove_section("device c");
        let config = Config::load(&path);
        assert_eq!(config.get("device a", "port"), Some("22"));
        assert_eq!(config.get("device a", "user"), None);
        assert_eq!(config.sections("device "), ["a"]);
    }

    #[test]
    fn memory_only_and_damaged_files() {
        let mut config = Config::default();
        config.set("settings", "sort", "size");
        assert_eq!(config.get("settings", "sort"), Some("size"));

        let dir = TempDir::new();
        let path = dir.path().join("config.ini");
        fs::write(&path, "stray=1\n[settings]\nno separator\nsort=name\n=empty key\n[open\n").unwrap();
        let config = Config::load(&path);
        assert_eq!(config.get("settings", "sort"), Some("name"));
        assert_eq!(config.get("settings", ""), Some("empty key"));
        assert_eq!(config.sections(""), ["settings"]);
    }

    #[test]
    fn default_path_is_in_the_user_configuration() {
        assert!(default_path().ends_with("sailtree/config.ini"));
        assert!(default_path().is_absolute());
    }
}
