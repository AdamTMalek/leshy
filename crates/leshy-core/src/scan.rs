use std::cmp::Ordering;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

use crate::{Directory, DirectoryId, File, GraphError, RelativePath, Repository};

/// A deterministic repository scan result that can be consumed by the indexing pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryScan {
    pub identity_source: RepositoryIdentitySource,
    pub repository: Repository,
    pub directories: Vec<Directory>,
    pub files: Vec<File>,
    pub skipped: Vec<SkippedPath>,
}

/// The source used to derive repository identity for stable IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryIdentitySource {
    GitOrigin,
    PathFallback,
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

    let (stable_key, identity_source) = repository_identity(&canonical_root)?;
    let display_name = canonical_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical_root.to_string_lossy().into_owned());
    let repository = Repository::new(stable_key, display_name, canonical_root.clone())
        .map_err(|source| ScanError::RepositoryMetadata { source })?;

    let root_directory = Directory::new(repository.id, None, ".")
        .map_err(|source| ScanError::RepositoryMetadata { source })?;
    let root_directory_id = root_directory.id;

    let mut scan = RepositoryScan {
        identity_source,
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

fn repository_identity(root: &Path) -> Result<(String, RepositoryIdentitySource), ScanError> {
    match git_origin_url(root)? {
        Some(origin) => Ok((
            normalize_git_origin(&origin),
            RepositoryIdentitySource::GitOrigin,
        )),
        None => Ok((
            root.to_string_lossy().into_owned(),
            RepositoryIdentitySource::PathFallback,
        )),
    }
}

fn git_origin_url(root: &Path) -> Result<Option<String>, ScanError> {
    let Some(git_dir) = resolve_git_dir(root)? else {
        return Ok(None);
    };

    let Some(config_path) = resolve_git_config_path(&git_dir)? else {
        return Ok(None);
    };
    let config = fs::read_to_string(&config_path).map_err(|source| ScanError::ReadPath {
        action: "read git config",
        path: config_path.clone(),
        source,
    })?;

    Ok(parse_origin_url(&config))
}

fn resolve_git_dir(root: &Path) -> Result<Option<PathBuf>, ScanError> {
    let git_path = root.join(".git");
    let metadata = match fs::metadata(&git_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ScanError::ReadPath {
                action: "read git metadata",
                path: git_path,
                source,
            });
        }
    };

    if metadata.is_dir() {
        return Ok(Some(git_path));
    }

    if metadata.is_file() {
        let contents = fs::read_to_string(&git_path).map_err(|source| ScanError::ReadPath {
            action: "read git indirection file",
            path: git_path.clone(),
            source,
        })?;
        let git_dir = parse_git_dir_reference(&contents).ok_or_else(|| ScanError::ReadPath {
            action: "parse git indirection file",
            path: git_path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidData, "missing `gitdir:` prefix"),
        })?;

        let resolved = if git_dir.is_absolute() {
            git_dir
        } else {
            root.join(git_dir)
        };

        return Ok(Some(resolved));
    }

    Ok(None)
}

fn parse_git_dir_reference(contents: &str) -> Option<PathBuf> {
    let line = contents.lines().next()?.trim();
    let git_dir = line.strip_prefix("gitdir:")?.trim();

    if git_dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(git_dir))
    }
}

fn resolve_git_config_path(git_dir: &Path) -> Result<Option<PathBuf>, ScanError> {
    let local_config = git_dir.join("config");
    if local_config.is_file() {
        return Ok(Some(local_config));
    }

    // Git worktrees often point `.git` at a per-worktree directory without its own `config`.
    // In that layout, `commondir` points back to the shared git dir that owns repository config.
    let common_dir_path = git_dir.join("commondir");
    let common_dir = match fs::read_to_string(&common_dir_path) {
        Ok(contents) => parse_common_dir_reference(&contents).map(|path| {
            if path.is_absolute() {
                path
            } else {
                git_dir.join(path)
            }
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ScanError::ReadPath {
                action: "read git common dir",
                path: common_dir_path,
                source,
            });
        }
    };

    Ok(common_dir
        .map(|path| path.join("config"))
        .filter(|path| path.is_file()))
}

fn parse_common_dir_reference(contents: &str) -> Option<PathBuf> {
    let common_dir = contents.lines().next()?.trim();

    if common_dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(common_dir))
    }
}

fn parse_origin_url(config: &str) -> Option<String> {
    let mut in_origin_section = false;

    for line in config.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_origin_section = trimmed[1..trimmed.len() - 1].trim() == r#"remote "origin""#;
            continue;
        }

        if !in_origin_section {
            continue;
        }

        let (key, value) = trimmed.split_once('=')?;
        if key.trim() == "url" {
            let url = value.trim();
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }

    None
}

fn normalize_git_origin(origin: &str) -> String {
    let trimmed = trim_git_origin_suffixes(origin);

    normalize_git_transport(trimmed).unwrap_or_else(|| trimmed.to_string())
}

fn trim_git_origin_suffixes(origin: &str) -> &str {
    origin.trim().trim_end_matches('/').trim_end_matches(".git")
}

fn normalize_git_transport(origin: &str) -> Option<String> {
    if let Some(rest) = origin.strip_prefix("git@") {
        return normalize_scp_like_origin(rest);
    }

    if let Some(rest) = strip_supported_scheme(origin) {
        return normalize_scheme_origin(rest);
    }

    None
}

fn strip_supported_scheme(origin: &str) -> Option<&str> {
    ["ssh://", "https://", "http://"]
        .into_iter()
        .find_map(|scheme| origin.strip_prefix(scheme))
}

fn normalize_scp_like_origin(origin: &str) -> Option<String> {
    let (host, path) = origin.split_once(':')?;

    normalize_host_and_path(host, path)
}

fn normalize_scheme_origin(origin: &str) -> Option<String> {
    let without_user = origin
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let (host, path) = without_user.split_once('/')?;

    normalize_host_and_path(host, path)
}

fn normalize_host_and_path(host: &str, path: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('/').trim_end_matches(':');
    let path = path.trim().trim_matches('/');

    if host.is_empty() || path.is_empty() {
        return None;
    }

    Some(format!("{}/{}", host.to_ascii_lowercase(), path))
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

    use super::{RepositoryIdentitySource, SkippedPathReason, scan_repository};
    use crate::{DirectoryId, RelativePath, RepositoryGraph};

    #[test]
    fn scans_empty_repository_with_only_root_directory() {
        let tempdir = TestDir::new();

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");

        assert_eq!(scan.identity_source, RepositoryIdentitySource::PathFallback);
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
    fn uses_git_origin_for_repository_identity() {
        let tempdir = TestDir::new();
        tempdir.write_git_config(
            r#"[remote "origin"]
    url = https://github.com/AdamTMalek/leshy.git
"#,
        );

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");

        assert_eq!(scan.identity_source, RepositoryIdentitySource::GitOrigin);
        assert_eq!(scan.repository.stable_key, "github.com/AdamTMalek/leshy");
    }

    #[test]
    fn git_origin_ids_are_stable_across_repository_moves() {
        let left = TestDir::new();
        let right = TestDir::new();

        left.write_git_config(
            r#"[remote "origin"]
    url = git@github.com:AdamTMalek/leshy.git
"#,
        );
        right.write_git_config(
            r#"[remote "origin"]
    url = git@github.com:AdamTMalek/leshy.git
"#,
        );
        left.write_file("src/lib.rs", "");
        right.write_file("src/lib.rs", "");

        let left_scan = scan_repository(left.path()).expect("left scan should succeed");
        let right_scan = scan_repository(right.path()).expect("right scan should succeed");

        assert_eq!(
            left_scan.identity_source,
            RepositoryIdentitySource::GitOrigin
        );
        assert_eq!(
            right_scan.identity_source,
            RepositoryIdentitySource::GitOrigin
        );
        assert_eq!(left_scan.repository.id, right_scan.repository.id);
        assert_eq!(left_scan.files[0].id, right_scan.files[0].id);
    }

    #[test]
    fn normalizes_https_and_ssh_git_origins_to_the_same_identity() {
        let https = TestDir::new();
        let ssh = TestDir::new();
        let ssh_scheme = TestDir::new();

        https.write_git_config(
            r#"[remote "origin"]
    url = https://github.com/AdamTMalek/leshy.git
"#,
        );
        ssh.write_git_config(
            r#"[remote "origin"]
    url = git@github.com:AdamTMalek/leshy.git
"#,
        );
        ssh_scheme.write_git_config(
            r#"[remote "origin"]
    url = ssh://git@github.com/AdamTMalek/leshy.git
"#,
        );

        let https_scan = scan_repository(https.path()).expect("https scan should succeed");
        let ssh_scan = scan_repository(ssh.path()).expect("ssh scan should succeed");
        let ssh_scheme_scan =
            scan_repository(ssh_scheme.path()).expect("ssh scheme scan should succeed");

        assert_eq!(https_scan.repository.stable_key, "github.com/AdamTMalek/leshy");
        assert_eq!(
            https_scan.repository.stable_key,
            ssh_scan.repository.stable_key
        );
        assert_eq!(
            https_scan.repository.stable_key,
            ssh_scheme_scan.repository.stable_key
        );
        assert_eq!(https_scan.repository.id, ssh_scan.repository.id);
        assert_eq!(https_scan.repository.id, ssh_scheme_scan.repository.id);
    }

    #[test]
    fn git_repositories_without_origin_use_path_fallback() {
        let tempdir = TestDir::new();
        tempdir.write_git_config(
            r#"[core]
    repositoryformatversion = 0
"#,
        );

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");

        assert_eq!(scan.identity_source, RepositoryIdentitySource::PathFallback);
    }

    #[test]
    fn resolves_git_origin_from_worktree_common_dir() {
        let tempdir = TestDir::new();
        tempdir.write_file(".git", "gitdir: .git-main/worktrees/current-worktree\n");
        tempdir.write_file(".git-main/worktrees/current-worktree/commondir", "../..\n");
        tempdir.write_file(
            ".git-main/config",
            r#"[remote "origin"]
    url = https://github.com/AdamTMalek/leshy.git
"#,
        );

        let scan = scan_repository(tempdir.path()).expect("scan should succeed");

        assert_eq!(scan.identity_source, RepositoryIdentitySource::GitOrigin);
        assert_eq!(scan.repository.stable_key, "github.com/AdamTMalek/leshy");
    }

    #[test]
    fn path_fallback_ids_change_across_repository_moves() {
        let left = TestDir::new();
        let right = TestDir::new();
        left.write_file("src/lib.rs", "");
        right.write_file("src/lib.rs", "");

        let left_scan = scan_repository(left.path()).expect("left scan should succeed");
        let right_scan = scan_repository(right.path()).expect("right scan should succeed");

        assert_eq!(
            left_scan.identity_source,
            RepositoryIdentitySource::PathFallback
        );
        assert_eq!(
            right_scan.identity_source,
            RepositoryIdentitySource::PathFallback
        );
        assert_ne!(left_scan.repository.id, right_scan.repository.id);
        assert_ne!(left_scan.files[0].id, right_scan.files[0].id);
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

        fn write_file(&self, relative_path: &str, contents: &str) {
            let file_path = self.path.join(relative_path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).expect("parent directories should be created");
            }
            fs::write(file_path, contents).expect("file should be written");
        }

        fn write_git_config(&self, contents: &str) {
            fs::create_dir_all(self.path.join(".git")).expect("git directory should be created");
            fs::write(self.path.join(".git/config"), contents)
                .expect("git config should be written");
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
