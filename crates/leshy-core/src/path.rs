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

        if raw.is_absolute() {
            return Err(GraphError::InvalidRelativePath {
                path: raw_display,
                reason: "path must be relative to the repository root",
            });
        }

        let mut segments = Vec::new();

        for component in raw.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::Normal(segment) => {
                    let segment =
                        segment
                            .to_str()
                            .ok_or_else(|| GraphError::InvalidRelativePath {
                                path: raw_display.clone(),
                                reason: "path must be valid UTF-8",
                            })?;
                    segments.push(segment);
                }
                std::path::Component::ParentDir => {
                    return Err(GraphError::InvalidRelativePath {
                        path: raw_display,
                        reason: "path must not escape the repository root",
                    });
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(GraphError::InvalidRelativePath {
                        path: raw_display,
                        reason: "path must be relative to the repository root",
                    });
                }
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
    fn rejects_parent_traversal() {
        let error = RelativePath::new("../Cargo.toml").expect_err("parent traversal must fail");
        assert!(
            error
                .to_string()
                .contains("path must not escape the repository root")
        );
    }
}
