//! opens empty files with another application.
//!
//! the system finds the application for a file from its content, and has none for a file
//! without content. such a file is opened by the application matching its name instead, or the
//! one for plain text: an empty file is a valid text. the text viewer also judges an empty file
//! by its name, so it is given a link ending with `.txt`.

use std::path::Path;
use std::process::Command;

use crate::staging;

/// file name patterns of the system, as `weight:type:pattern` lines
const GLOBS: &str = "/usr/share/mime/globs2";
/// launcher of the applications registered for a type
const LAUNCHER: &str = "lca-tool";

/// content type announced by a file name, from the system's patterns
pub fn mime_by_name(globs: &str, name: &str) -> Option<String> {
    let name = name.to_lowercase();
    globs
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split(':');
            let weight: u32 = fields.next()?.parse().ok()?;
            let (mime, pattern) = (fields.next()?, fields.next()?.to_lowercase());
            let matches = match pattern.strip_prefix('*') {
                // only the common `*.extension` patterns and plain names are understood
                Some(end) => !end.contains(['*', '?', '[']) && name.len() > end.len() && name.ends_with(end),
                None => name == pattern,
            };
            matches.then_some((weight, pattern.len(), mime))
        })
        // the heaviest pattern wins, then the most specific one (`.tar.gz` before `.gz`)
        .max_by_key(|(weight, length, _)| (*weight, *length))
        .map(|(_, _, mime)| mime.to_owned())
}

/// `file://` url of a local path
pub fn file_url(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => url.push(byte as char),
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

/// name of the application registered first for a content type
fn application(mime: &str) -> Option<String> {
    // unit tests run under the build sandbox, where starting a process from a threaded
    // program can freeze: no application is known there
    if cfg!(test) {
        return None;
    }
    let output = Command::new(LAUNCHER).args(["--actionsformime", mime]).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(str::to_owned)
}

/// name under which the text viewer accepts an empty file
pub fn text_name(name: &str) -> String {
    match name.to_lowercase().ends_with(".txt") {
        true => name.to_owned(),
        false => format!("{name}.txt"),
    }
}

/// open a file without content, made visible to the application from the `home` folder.
/// blocks until the application is asked to start
pub fn open_empty(home: &Path, path: &Path) -> Result<(), String> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let named = std::fs::read_to_string(GLOBS).ok().and_then(|globs| mime_by_name(&globs, &name));
    let text = application("text/plain");
    let (application, staged) = match named.as_deref().and_then(application) {
        // an application of its own for this kind of file
        Some(named) if Some(&named) != text.as_ref() => (Some(named), staging::stage(home, path)),
        _ if text_name(&name) == name => (text, staging::stage(home, path)),
        _ => {
            let staged = staging::stage_as(home, path, text_name(&name))
                .map_err(|error| format!("{name} cannot be opened: {error}"))?;
            (text, staged)
        }
    };
    let Some(application) = application else {
        return Err(format!("no application to open {name}"));
    };
    let started = Command::new(LAUNCHER)
        .args(["--triggerdesktop", &format!("{application}.desktop"), &file_url(&staged)])
        .status();
    match started {
        Ok(status) if status.success() => Ok(()),
        _ => Err(format!("{application} could not open {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment line\n\
        50:text/plain:*.txt\n\
        50:image/png:*.png\n\
        40:image/apng:*.png\n\
        50:application/gzip:*.gz\n\
        50:application/x-compressed-tar:*.tar.gz\n\
        50:text/x-makefile:makefile\n\
        50:text/x-readme:readme*\n\
        50:video/x-anim:*.anim[1-9j]\n\
        broken line\n\
        60:text/x-c++src:*.C:cs\n";

    #[test]
    fn type_from_the_extension() {
        assert_eq!(mime_by_name(SAMPLE, "notes.txt").as_deref(), Some("text/plain"));
        assert_eq!(mime_by_name(SAMPLE, "PHOTO.TXT").as_deref(), Some("text/plain"));
        assert_eq!(mime_by_name(SAMPLE, "a.b.txt").as_deref(), Some("text/plain"));
        assert_eq!(mime_by_name(SAMPLE, "Makefile").as_deref(), Some("text/x-makefile"));
        assert_eq!(mime_by_name(SAMPLE, "notes"), None);
        assert_eq!(mime_by_name(SAMPLE, "notes.unknown"), None);
        assert_eq!(mime_by_name(SAMPLE, ".txt"), None, "a hidden file named txt has no extension");
        assert_eq!(mime_by_name(SAMPLE, ""), None);
        assert_eq!(mime_by_name("", "notes.txt"), None);
    }

    #[test]
    fn heaviest_then_longest_pattern_wins() {
        assert_eq!(mime_by_name(SAMPLE, "shot.png").as_deref(), Some("image/png"));
        assert_eq!(mime_by_name(SAMPLE, "backup.tar.gz").as_deref(), Some("application/x-compressed-tar"));
        assert_eq!(mime_by_name(SAMPLE, "backup.gz").as_deref(), Some("application/gzip"));
        assert_eq!(mime_by_name(SAMPLE, "main.c").as_deref(), Some("text/x-c++src"));
    }

    #[test]
    fn other_patterns_are_ignored() {
        assert_eq!(mime_by_name(SAMPLE, "readme.first"), None);
        assert_eq!(mime_by_name(SAMPLE, "film.anim1"), None);
    }

    #[test]
    fn names_for_the_text_viewer() {
        assert_eq!(text_name("notes.txt"), "notes.txt");
        assert_eq!(text_name("NOTES.TXT"), "NOTES.TXT");
        assert_eq!(text_name("server.log"), "server.log.txt");
        assert_eq!(text_name("Makefile"), "Makefile.txt");
        assert_eq!(text_name(".profile"), ".profile.txt");
    }

    #[test]
    fn urls_escape_special_characters() {
        assert_eq!(file_url(Path::new("/home/user/notes.txt")), "file:///home/user/notes.txt");
        assert_eq!(
            file_url(Path::new("/home/user/my file #1?é.txt")),
            "file:///home/user/my%20file%20%231%3F%C3%A9.txt"
        );
    }
}
