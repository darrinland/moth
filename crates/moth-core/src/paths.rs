//! Where Moth keeps user data: the library, settings and recovery files.
//!
//! macOS and Linux use the platform's data folder. The Windows build is portable, so its
//! data sits in a `data` folder beside the executable.

use std::path::PathBuf;

/// The folder for user data. It may not exist yet; callers create it when they write.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::current_exe()
            .ok()?
            .parent()
            .map(|dir| dir.join("data"))
    }
    #[cfg(not(windows))]
    {
        directories::ProjectDirs::from("", "", "Moth").map(|dirs| dirs.data_dir().to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_named_for_moth() {
        let dir = data_dir().expect("a data folder");
        if cfg!(target_os = "macos") {
            assert!(dir.ends_with("Library/Application Support/Moth"), "{dir:?}");
        }
        if cfg!(windows) {
            assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("data"));
        }
    }
}
