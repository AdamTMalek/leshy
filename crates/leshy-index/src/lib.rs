use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use leshy_core::{
    DirectoryId, ExtractedSymbol, FileId, GraphError, RepositoryGraph, RepositoryScan, ScanError,
    scan_repository,
};
use leshy_parser::{
    LanguageRegistry, ParseError, ParsedFile, extract_symbols, parse_repository_scan,
};

/// The end-to-end indexing result for a repository root.
#[derive(Debug)]
pub struct RepositoryIndex {
    pub scan: RepositoryScan,
    pub parsed_files: Vec<ParsedFile>,
    pub symbols: Vec<ExtractedSymbol>,
    pub graph: RepositoryGraph,
}

/// Errors returned by the indexing orchestration pipeline.
#[derive(Debug)]
pub enum IndexError {
    Scan {
        source: ScanError,
    },
    Parse {
        source: ParseError,
    },
    InsertDirectory {
        directory_id: DirectoryId,
        source: GraphError,
    },
    InsertFile {
        file_id: FileId,
        source: GraphError,
    },
}

impl Display for IndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scan { source } => write!(f, "failed to scan repository: {source}"),
            Self::Parse { source } => write!(f, "failed to parse repository: {source}"),
            Self::InsertDirectory {
                directory_id,
                source,
            } => {
                write!(f, "failed to populate directory `{directory_id}`: {source}")
            }
            Self::InsertFile { file_id, source } => {
                write!(f, "failed to populate file `{file_id}`: {source}")
            }
        }
    }
}

impl Error for IndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scan { source } => Some(source),
            Self::Parse { source } => Some(source),
            Self::InsertDirectory { source, .. } => Some(source),
            Self::InsertFile { source, .. } => Some(source),
        }
    }
}

/// Scans a repository root, parses supported source files, and populates a repository graph.
pub fn index_repository(
    root: &Path,
    registry: &LanguageRegistry,
) -> Result<RepositoryIndex, IndexError> {
    let scan = scan_repository(root).map_err(|source| IndexError::Scan { source })?;
    let parsed_files = parse_repository_scan(root, &scan, registry)
        .map_err(|source| IndexError::Parse { source })?;
    let symbols = extract_symbols(&parsed_files, registry);
    let graph = build_graph_from_scan(&scan)?;

    Ok(RepositoryIndex {
        scan,
        parsed_files,
        symbols,
        graph,
    })
}

fn build_graph_from_scan(scan: &RepositoryScan) -> Result<RepositoryGraph, IndexError> {
    let mut graph = RepositoryGraph::new(scan.repository.clone());

    for directory in &scan.directories {
        graph
            .insert_directory(directory.clone())
            .map_err(|source| IndexError::InsertDirectory {
                directory_id: directory.id,
                source,
            })?;
    }

    for file in &scan.files {
        graph
            .insert_file(file.clone())
            .map_err(|source| IndexError::InsertFile {
                file_id: file.id,
                source,
            })?;
    }

    Ok(graph)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use leshy_lang_rust::RUST_LANGUAGE_PLUGIN;
    use leshy_parser::{LanguageId, LanguageRegistry, ParseError};

    use super::{IndexError, build_graph_from_scan, index_repository};
    use leshy_core::{DirectoryId, RelativePath, ScanError};

    #[test]
    fn indexes_repository_end_to_end() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn library() {}\n");
        tempdir.write_file("src/bin/app.rs", "");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");

        assert_eq!(index.scan.directories.len(), 3);
        assert_eq!(index.scan.files.len(), 2);
        assert_eq!(index.parsed_files.len(), 2);
        assert_eq!(index.symbols.len(), 1);
        assert_eq!(index.parsed_files[0].language, LanguageId::new("rust"));
        assert_eq!(index.symbols[0].display_name, "library");
        assert_eq!(index.graph.directories().count(), 3);
        assert_eq!(index.graph.files().count(), 2);
        assert_eq!(index.graph.relationships().count(), 5);
        assert_eq!(index.graph.repository().id, index.scan.repository.id);
    }

    #[test]
    fn wraps_scan_failures() {
        let missing_path = unique_temp_path("missing");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);
        let error = index_repository(&missing_path, &registry).expect_err("indexing should fail");

        assert!(matches!(
            error,
            IndexError::Scan {
                source: ScanError::ReadPath { .. }
            }
        ));
    }

    #[test]
    fn wraps_parse_failures() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "fn broken( {\n");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let error = index_repository(tempdir.path(), &registry).expect_err("indexing should fail");

        assert!(matches!(
            error,
            IndexError::Parse {
                source: ParseError::SyntaxErrors { .. }
            }
        ));
        assert!(error.to_string().contains("failed to parse repository"));
        assert!(error.to_string().contains("src/lib.rs"));
    }

    #[test]
    fn reports_directory_population_stage_failures() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join("src/nested")).expect("nested directories");

        let mut scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        scan.directories[1].parent_id = None;
        let failing_directory_id = scan.directories[1].id;

        let error = build_graph_from_scan(&scan).expect_err("graph population should fail");

        assert!(matches!(
            error,
            IndexError::InsertDirectory { directory_id, .. } if directory_id == failing_directory_id
        ));
    }

    #[test]
    fn reports_file_population_stage_failures() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "");

        let mut scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        scan.files[0].parent_id = DirectoryId::new(
            scan.repository.id,
            &RelativePath::new("missing").expect("relative path should build"),
        );
        let failing_file_id = scan.files[0].id;

        let error = build_graph_from_scan(&scan).expect_err("graph population should fail");

        assert!(matches!(
            error,
            IndexError::InsertFile { file_id, .. } if file_id == failing_file_id
        ));
    }

    #[test]
    fn indexes_without_parsed_files_when_no_plugins_are_registered() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn library() {}\n");

        let index = index_repository(tempdir.path(), &LanguageRegistry::new())
            .expect("indexing should succeed");

        assert!(index.parsed_files.is_empty());
        assert!(index.symbols.is_empty());
        assert_eq!(index.scan.files.len(), 1);
        assert_eq!(index.graph.files().count(), 1);
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

            let unique = format!(
                "leshy-index-test-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time should be valid")
                    .as_nanos(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn unique_temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "leshy-index-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos()
        ))
    }
}
