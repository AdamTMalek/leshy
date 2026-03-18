use std::cmp::Ordering;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

use crate::{Directory, DirectoryId, File, GraphError, RelativePath, Repository};

/// A deterministic repository scan result that can be consumed by the indexing pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryScan {
    pub repository: Repository,
    pub directories: Vec<Directory>,
    pub files: Vec<File>,
    pub skipped: Vec<SkippedPath>,
}

/// A skipped filesystem entry recorded during repository scanning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedPath {
    pub path: PathBuf,
    pub reason: SkippedPathReason,
}

/// The reason a path was skipped during repository scanning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedPathReason {
    IgnoredGitDirectory,
    Symlink,
    UnsupportedFileType,
    InvalidRelativePath,
}

/// Errors returned by repository scanning.
#[derive(Debug)]
pub enum ScanError {
    ReadPath {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    RootNotDirectory {
        path: PathBuf,
    },
    RepositoryMetadata {
        source: GraphError,
    },
    PathOutsideRoot {
        path: PathBuf,
        root: PathBuf,
    },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadPath { action, path, .. } => {
                write!(f, "failed to {action} `{}`", path.display())
            }
            Self::RootNotDirectory { path } => {
                write!(f, "repository root `{}` is not a directory", path.display())
            }
            Self::RepositoryMetadata { source } => {
                write!(f, "failed to build repository metadata: {source}")
            }
            Self::PathOutsideRoot { path, root } => {
                write!(
                    f,
                    "scanned path `{}` is outside repository root `{}`",
                    path.display(),
                    root.display()
                )
            }
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReadPath { source, .. } => Some(source),
            Self::RepositoryMetadata { source } => Some(source),
            Self::RootNotDirectory { .. } | Self::PathOutsideRoot { .. } => None,
        }
    }
}

/// Scans a repository root into deterministic filesystem-layer entities.
pub fn scan_repository(root: &Path) -> Result<RepositoryScan, ScanError> {
    let canonical_root = fs::canonicalize(root).map_err(|source| ScanError::ReadPath {
        action: "canonicalize repository root",
        path: root.to_path_buf(),
        source,
    })?;

    let metadata = fs::metadata(&canonical_root).map_err(|source| ScanError::ReadPath {
        action: "read repository root metadata",
        path: canonical_root.clone(),
        source,
    })?;

    if !metadata.is_dir() {
        return Err(ScanError::RootNotDirectory {
            path: canonical_root,
        });
    }

    let stable_key = canonical_root.to_string_lossy().into_owned();
    let display_name = canonical_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| stable_key.clone());
    let repository = Repository::new(stable_key, display_name, canonical_root.clone())
        .map_err(|source| ScanError::RepositoryMetadata { source })?;

    let root_directory = Directory::new(repository.id, None, ".")
        .map_err(|source| ScanError::RepositoryMetadata { source })?;
    let root_directory_id = root_directory.id;

    let mut scan = RepositoryScan {
        repository,
        directories: vec![root_directory],
        files: Vec::new(),
        skipped: Vec::new(),
    };

    walk_directory(
        &canonical_root,
        &canonical_root,
        root_directory_id,
        &mut scan,
    )?;

    scan.directories.sort_by(compare_directories);
    scan.files.sort_by(|left, right| {
        left.relative_path
            .as_str()
            .cmp(right.relative_path.as_str())
    });
    scan.skipped
        .sort_by(|left, right| left.path.cmp(&right.path));

    Ok(scan)
}

fn walk_directory(
    repository_root: &Path,
    current_directory: &Path,
    parent_id: DirectoryId,
    scan: &mut RepositoryScan,
) -> Result<(), ScanError> {
    let mut entries = read_dir_entries(current_directory)?;
    entries.sort_by(compare_entries);

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| ScanError::ReadPath {
            action: "read filesystem entry type",
            path: path.clone(),
            source,
        })?;

        if file_type.is_symlink() {
            scan.skipped.push(SkippedPath {
                path,
                reason: SkippedPathReason::Symlink,
            });
            continue;
        }

        let Some(relative_path) =
            normalize_relative_path(repository_root, &path, &mut scan.skipped)?
        else {
            continue;
        };

        if file_type.is_dir() && entry.file_name() == ".git" {
            scan.skipped.push(SkippedPath {
                path,
                reason: SkippedPathReason::IgnoredGitDirectory,
            });
            continue;
        }

        if file_type.is_dir() {
            let directory =
                Directory::new(scan.repository.id, Some(parent_id), relative_path.as_str())
                    .map_err(|source| ScanError::RepositoryMetadata { source })?;
            let directory_id = directory.id;
            scan.directories.push(directory);
            walk_directory(repository_root, &path, directory_id, scan)?;
            continue;
        }

        if file_type.is_file() {
            let file = File::new(scan.repository.id, parent_id, relative_path.as_str())
                .map_err(|source| ScanError::RepositoryMetadata { source })?;
            scan.files.push(file);
            continue;
        }

        scan.skipped.push(SkippedPath {
            path,
            reason: SkippedPathReason::UnsupportedFileType,
        });
    }

    Ok(())
}

fn normalize_relative_path(
    repository_root: &Path,
    path: &Path,
    skipped: &mut Vec<SkippedPath>,
) -> Result<Option<RelativePath>, ScanError> {
    let stripped = path
        .strip_prefix(repository_root)
        .map_err(|_| ScanError::PathOutsideRoot {
            path: path.to_path_buf(),
            root: repository_root.to_path_buf(),
        })?;

    match RelativePath::new(stripped) {
        Ok(relative_path) => Ok(Some(relative_path)),
        Err(_) => {
            skipped.push(SkippedPath {
                path: path.to_path_buf(),
                reason: SkippedPathReason::InvalidRelativePath,
            });
            Ok(None)
        }
    }
}

fn read_dir_entries(path: &Path) -> Result<Vec<DirEntry>, ScanError> {
    fs::read_dir(path)
        .map_err(|source| ScanError::ReadPath {
            action: "read directory",
            path: path.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ScanError::ReadPath {
            action: "read directory entry",
            path: path.to_path_buf(),
            source,
        })
}

fn compare_entries(left: &DirEntry, right: &DirEntry) -> Ordering {
    left.file_name()
        .to_string_lossy()
        .cmp(&right.file_name().to_string_lossy())
}

fn compare_directories(left: &Directory, right: &Directory) -> Ordering {
    relative_path_depth(left.relative_path.as_str())
        .cmp(&relative_path_depth(right.relative_path.as_str()))
        .then_with(|| {
            left.relative_path
                .as_str()
                .cmp(right.relative_path.as_str())
        })
}

fn relative_path_depth(path: &str) -> usize {
    if path.is_empty() {
        0
    } else {
        path.split('/').count()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{SkippedPathReason, scan_repository};
    use crate::{DirectoryId, RelativePath, RepositoryGraph};

    #[test]
    fn scans_empty_repository_with_only_root_directory() {
        let tempdir = TestDir::new();

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");

        assert_eq!(scan.directories.len(), 1);
        assert_eq!(scan.directories[0].relative_path, RelativePath::root());
        assert!(scan.files.is_empty());
        assert!(scan.skipped.is_empty());
    }

    #[test]
    fn emits_directories_parent_before_child() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join("src/nested")).expect("nested directories");

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");
        let paths = scan
            .directories
            .iter()
            .map(|directory| directory.relative_path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec![".", "src", "src/nested"]);
    }

    #[test]
    fn emits_files_in_lexicographic_relative_path_order() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join("src/zeta")).expect("zeta directory");
        fs::create_dir_all(tempdir.path().join("src/alpha")).expect("alpha directory");
        fs::write(tempdir.path().join("src/zeta/lib.rs"), "").expect("zeta file");
        fs::write(tempdir.path().join("src/alpha/main.rs"), "").expect("alpha file");

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");
        let files = scan
            .files
            .iter()
            .map(|file| file.relative_path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(files, vec!["src/alpha/main.rs", "src/zeta/lib.rs"]);
    }

    #[test]
    fn skips_git_directory_recursively() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join(".git/objects")).expect("git directory");
        fs::write(tempdir.path().join(".git/config"), "").expect("git config");
        fs::write(tempdir.path().join("Cargo.toml"), "").expect("repo file");

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");
        let files = scan
            .files
            .iter()
            .map(|file| file.relative_path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(files, vec!["Cargo.toml"]);
        assert_eq!(scan.skipped.len(), 1);
        assert_eq!(
            scan.skipped[0].reason,
            SkippedPathReason::IgnoredGitDirectory
        );
        assert!(scan.skipped[0].path.ends_with(".git"));
    }

    #[test]
    fn can_populate_repository_graph_from_scan_output() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join("src/bin")).expect("nested directories");
        fs::write(tempdir.path().join("src/lib.rs"), "").expect("lib file");
        fs::write(tempdir.path().join("src/bin/app.rs"), "").expect("bin file");

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");
        let mut graph = RepositoryGraph::new(scan.repository.clone());

        for directory in scan.directories {
            graph
                .insert_directory(directory)
                .expect("directory should insert");
        }

        for file in scan.files {
            graph.insert_file(file).expect("file should insert");
        }

        let root_id = DirectoryId::new(graph.repository().id, &RelativePath::root());
        assert!(graph.directory(root_id).is_some());
        assert_eq!(graph.directories().count(), 3);
        assert_eq!(graph.files().count(), 2);
    }

    #[test]
    fn skips_symlinks_when_the_platform_allows_creating_them() {
        let tempdir = TestDir::new();
        fs::write(tempdir.path().join("real.rs"), "").expect("real file");

        if !create_file_symlink(
            &tempdir.path().join("real.rs"),
            &tempdir.path().join("linked.rs"),
        ) {
            return;
        }

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");
        let skipped = scan
            .skipped
            .iter()
            .find(|entry| entry.path == tempdir.path().join("linked.rs"))
            .expect("symlink should be skipped");

        assert_eq!(skipped.reason, SkippedPathReason::Symlink);
    }

    fn create_file_symlink(source: &Path, target: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(source, target).is_ok()
        }

        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(source, target).is_ok()
        }
    }

    struct TestDir {
        path: std::path::PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "leshy-core-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time should be valid")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir(&path).expect("temporary directory should be created");

            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
