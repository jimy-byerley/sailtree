//! what to do when the destination of a transfer already has an item of the same name

/// choice of the user for one conflict
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// replace the existing item. a folder over a folder is merged with it
    Overwrite,
    /// leave both as they are
    Skip,
    /// keep both, the arriving one under a numbered name
    Rename,
}

impl Answer {
    /// from the name used by the interface
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "overwrite" => Some(Answer::Overwrite),
            "skip" => Some(Answer::Skip),
            "rename" => Some(Answer::Rename),
            _ => None,
        }
    }
}

/// an answer, and whether it also answers the next conflicts of the same transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub answer: Answer,
    pub all: bool,
}

/// name of the `number`th item wanting the same name: `photo (2).jpg`, `folder (3)`
pub fn numbered(name: &str, number: u32) -> String {
    match name.rfind('.') {
        // a leading dot is not an extension
        Some(dot) if dot > 0 => format!("{} ({number}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({number})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_names_keep_the_extension() {
        assert_eq!(numbered("photo.jpg", 2), "photo (2).jpg");
        assert_eq!(numbered("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(numbered("folder", 2), "folder (2)");
        assert_eq!(numbered(".profile", 2), ".profile (2)");
        assert_eq!(numbered("photo (2).jpg", 2), "photo (2) (2).jpg");
    }

    #[test]
    fn answers_from_the_interface() {
        assert_eq!(Answer::parse("overwrite"), Some(Answer::Overwrite));
        assert_eq!(Answer::parse("skip"), Some(Answer::Skip));
        assert_eq!(Answer::parse("rename"), Some(Answer::Rename));
        assert_eq!(Answer::parse("other"), None);
    }
}
