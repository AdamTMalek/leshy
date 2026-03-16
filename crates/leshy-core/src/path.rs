use std::fmt::{Display, Formatter};
use std::path::Path;

use crate::GraphError;

/// A normalized repository-relative path that always uses `/` separators.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelativePath(String);

impl RelativePath {
    /// Returns the normalized repository root path.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parses and normalizes a repository-relative path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        let raw = path.as_ref();
        let raw_display = raw.display().to_string();
        let raw_string = raw
            .to_str()
            .ok_or_else(|| GraphError::InvalidRelativePath {
                path: raw_display.clone(),
                reason: "path must be valid UTF-8",
            })?;
        let normalized_input = raw_string.replace('\\', "/");

        if raw.is_absolute() || looks_like_absolute_path(&normalized_input) {
            return Err(GraphError::InvalidRelativePath {
                path: raw_display,
                reason: "path must be relative to the repository root",
            });
        }

        let mut segments = Vec::new();

        for segment in normalized_input.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    return Err(GraphError::InvalidRelativePath {
                        path: raw_display,
                        reason: "path must not escape the repository root",
                    });
                }
                _ => segments.push(segment),
            }
        }

        Ok(Self(segments.join("/")))
    }

    /// Returns the underlying normalized path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether the path refers to the repository root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the normalized parent path, if one exists.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }

        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_string())),
            None => Some(Self::root()),
        }
    }
}

impl Display for RelativePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_root() {
            write!(f, ".")
        } else {
            f.write_str(&self.0)
        }
    }
}

impl TryFrom<&str> for RelativePath {
    type Error = GraphError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&Path> for RelativePath {
    type Error = GraphError;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

fn looks_like_absolute_path(path: &str) -> bool {
    path.starts_with('/') || has_windows_drive_prefix(path)
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();

    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(test)]
mod tests {
    use super::RelativePath;

    #[test]
    fn normalizes_current_directory_to_root() {
        let path = RelativePath::new(".").expect("current directory should normalize");
        assert!(path.is_root());
        assert_eq!(path.to_string(), ".");
    }

    #[test]
    fn normalizes_redundant_separators() {
        let path = RelativePath::new("src//graph/./mod.rs").expect("path should normalize");
        assert_eq!(path.as_str(), "src/graph/mod.rs");
    }

    #[test]
    fn normalizes_backslashes_to_forward_slashes() {
        let path = RelativePath::new(r"src\graph\mod.rs").expect("path should normalize");
        assert_eq!(path.as_str(), "src/graph/mod.rs");
    }

    #[test]
    fn rejects_parent_traversal() {
        let error = RelativePath::new("../Cargo.toml").expect_err("parent traversal must fail");
        assert!(
            error
                .to_string()
                .contains("path must not escape the repository root")
        );
    }

    #[test]
    fn rejects_windows_absolute_paths() {
        let error = RelativePath::new(r"C:\repo\src\lib.rs").expect_err("absolute path must fail");
        assert!(
            error
                .to_string()
                .contains("path must be relative to the repository root")
        );
    }
}
