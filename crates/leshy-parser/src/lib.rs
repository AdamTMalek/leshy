use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::Path;

use leshy_core::{FileId, RelativePath, RepositoryScan};
use tree_sitter::Tree;

/// Stable identifier for a source language handled by parser plugins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanguageId(&'static str);

impl LanguageId {
    /// Creates a new stable language identifier.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// Returns the underlying stable language identifier string.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl Display for LanguageId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// Parsed syntax data for a repository file.
#[derive(Debug)]
pub struct ParsedFile {
    pub file_id: FileId,
    pub relative_path: RelativePath,
    pub language: LanguageId,
    pub source_text: String,
    pub tree: Tree,
}

/// Errors returned by repository parsing.
#[derive(Debug)]
pub enum ParseError {
    ConfigureParser {
        path: RelativePath,
        language: LanguageId,
        source: tree_sitter::LanguageError,
    },
    ReadSource {
        path: RelativePath,
        source: io::Error,
    },
    ParseReturnedNone {
        path: RelativePath,
        language: LanguageId,
    },
    SyntaxErrors {
        path: RelativePath,
        language: LanguageId,
    },
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigureParser {
                path,
                language,
                source,
            } => {
                write!(
                    f,
                    "failed to configure {language} parser for `{path}`: {source}"
                )
            }
            Self::ReadSource { path, .. } => {
                write!(f, "failed to read source file `{path}`")
            }
            Self::ParseReturnedNone { path, language } => {
                write!(
                    f,
                    "{language} parser did not return a syntax tree for `{path}`"
                )
            }
            Self::SyntaxErrors { path, language } => {
                write!(f, "{language} parser reported syntax errors in `{path}`")
            }
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ConfigureParser { source, .. } => Some(source),
            Self::ReadSource { source, .. } => Some(source),
            Self::ParseReturnedNone { .. } | Self::SyntaxErrors { .. } => None,
        }
    }
}

/// A language integration that can classify and parse repository files.
pub trait LanguagePlugin: Sync {
    fn language(&self) -> LanguageId;
    fn supports_path(&self, path: &Path) -> bool;
    fn parse_source(&self, source_text: &str) -> Result<Tree, LanguagePluginError>;
}

#[derive(Debug)]
pub enum LanguagePluginError {
    ConfigureParser { source: tree_sitter::LanguageError },
    ParseReturnedNone,
}

/// Compile-time registry of bundled language plugins.
#[derive(Default)]
pub struct LanguageRegistry {
    plugins: Vec<&'static dyn LanguagePlugin>,
}

impl LanguageRegistry {
    /// Creates an empty language registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a language plugin in priority order.
    pub fn register(&mut self, plugin: &'static dyn LanguagePlugin) {
        self.plugins.push(plugin);
    }

    /// Registers a language plugin and returns the updated registry.
    pub fn with_plugin(mut self, plugin: &'static dyn LanguagePlugin) -> Self {
        self.register(plugin);
        self
    }

    /// Returns whether the registry contains no language plugins.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Returns the number of registered language plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    fn plugin_for_relative_path(&self, path: &RelativePath) -> Option<&'static dyn LanguagePlugin> {
        let file_path = Path::new(path.as_str());

        self.plugins
            .iter()
            .copied()
            .find(|plugin| plugin.supports_path(file_path))
    }
}

/// Parses all supported source files from a repository scan.
pub fn parse_repository_scan(
    repository_root: &Path,
    scan: &RepositoryScan,
    registry: &LanguageRegistry,
) -> Result<Vec<ParsedFile>, ParseError> {
    let mut parsed_files = Vec::new();

    for file in &scan.files {
        let Some(plugin) = registry.plugin_for_relative_path(&file.relative_path) else {
            continue;
        };

        let source_path = repository_root.join(file.relative_path.as_str());
        let source_text =
            fs::read_to_string(&source_path).map_err(|source| ParseError::ReadSource {
                path: file.relative_path.clone(),
                source,
            })?;

        let language = plugin.language();
        let tree = plugin
            .parse_source(&source_text)
            .map_err(|error| map_plugin_error(error, file.relative_path.clone(), language))?;

        if tree.root_node().has_error() {
            return Err(ParseError::SyntaxErrors {
                path: file.relative_path.clone(),
                language,
            });
        }

        parsed_files.push(ParsedFile {
            file_id: file.id,
            relative_path: file.relative_path.clone(),
            language,
            source_text,
            tree,
        });
    }

    Ok(parsed_files)
}

fn map_plugin_error(
    error: LanguagePluginError,
    path: RelativePath,
    language: LanguageId,
) -> ParseError {
    match error {
        LanguagePluginError::ConfigureParser { source } => ParseError::ConfigureParser {
            path,
            language,
            source,
        },
        LanguagePluginError::ParseReturnedNone => ParseError::ParseReturnedNone { path, language },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use tree_sitter::Tree;

    use super::{
        LanguageId, LanguagePlugin, LanguagePluginError, LanguageRegistry, ParseError,
        parse_repository_scan,
    };

    static RS_PLUGIN: MatchByExtensionPlugin = MatchByExtensionPlugin {
        extension: "rs",
        language: LanguageId::new("rust-test"),
    };

    #[test]
    fn registry_selects_matching_plugin() {
        let path = leshy_core::RelativePath::new("src/lib.rs").expect("relative path should build");
        let registry = LanguageRegistry::new().with_plugin(&RS_PLUGIN);

        let plugin = registry
            .plugin_for_relative_path(&path)
            .expect("plugin should match");

        assert_eq!(plugin.language(), LanguageId::new("rust-test"));
    }

    #[test]
    fn skips_unsupported_files() {
        let tempdir = TestDir::new();
        tempdir.write_file("README.md", "# Leshy");
        let registry = LanguageRegistry::new().with_plugin(&RS_PLUGIN);

        let scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        let parsed =
            parse_repository_scan(tempdir.path(), &scan, &registry).expect("parse should succeed");

        assert!(parsed.is_empty());
    }

    #[test]
    fn propagates_plugin_parse_errors() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn meaning() {}\n");
        let mut registry = LanguageRegistry::new();
        registry.register(&ParseNonePlugin);

        let scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        let error =
            parse_repository_scan(tempdir.path(), &scan, &registry).expect_err("parse should fail");

        assert!(matches!(
            error,
            ParseError::ParseReturnedNone { ref path, language }
                if path.as_str() == "src/lib.rs" && language == LanguageId::new("parse-none")
        ));
    }

    #[test]
    fn skips_all_files_when_registry_is_empty() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn meaning() {}\n");

        let scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        let parsed = parse_repository_scan(tempdir.path(), &scan, &LanguageRegistry::new())
            .expect("parse should skip");

        assert!(parsed.is_empty());
    }

    struct MatchByExtensionPlugin {
        extension: &'static str,
        language: LanguageId,
    }

    impl LanguagePlugin for MatchByExtensionPlugin {
        fn language(&self) -> LanguageId {
            self.language
        }

        fn supports_path(&self, path: &Path) -> bool {
            matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some(extension) if extension == self.extension
            )
        }

        fn parse_source(&self, _source_text: &str) -> Result<Tree, LanguagePluginError> {
            Err(LanguagePluginError::ParseReturnedNone)
        }
    }

    struct ParseNonePlugin;

    impl LanguagePlugin for ParseNonePlugin {
        fn language(&self) -> LanguageId {
            LanguageId::new("parse-none")
        }

        fn supports_path(&self, path: &Path) -> bool {
            matches!(
                path.extension().and_then(std::ffi::OsStr::to_str),
                Some("rs")
            )
        }

        fn parse_source(&self, _source_text: &str) -> Result<Tree, LanguagePluginError> {
            Err(LanguagePluginError::ParseReturnedNone)
        }
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

            let unique = format!(
                "leshy-parser-test-{}-{}-{}",
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
}
