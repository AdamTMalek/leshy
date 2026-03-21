use std::fmt::{Display, Formatter};
use std::fs;
use std::io;
use std::path::Path;

use tree_sitter::{Parser, Tree};

use crate::{FileId, RelativePath, RepositoryScan};

/// Supported source languages for parser dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLanguage {
    Rust,
}

impl SourceLanguage {
    fn from_relative_path(path: &RelativePath) -> Option<Self> {
        match Path::new(path.as_str())
            .extension()
            .and_then(std::ffi::OsStr::to_str)
        {
            Some("rs") => Some(Self::Rust),
            _ => None,
        }
    }

    fn parser(self) -> Result<Parser, ParseError> {
        let mut parser = Parser::new();

        match self {
            Self::Rust => {
                let language = tree_sitter_rust::LANGUAGE.into();
                parser
                    .set_language(&language)
                    .map_err(|source| ParseError::ConfigureParser {
                        language: self,
                        source,
                    })?;
            }
        }

        Ok(parser)
    }
}

impl Display for SourceLanguage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => f.write_str("Rust"),
        }
    }
}

/// Parsed syntax data for a repository file.
#[derive(Debug)]
pub struct ParsedFile {
    pub file_id: FileId,
    pub relative_path: RelativePath,
    pub language: SourceLanguage,
    pub source_text: String,
    pub tree: Tree,
}

/// Errors returned by repository parsing.
#[derive(Debug)]
pub enum ParseError {
    ConfigureParser {
        language: SourceLanguage,
        source: tree_sitter::LanguageError,
    },
    ReadSource {
        path: RelativePath,
        source: io::Error,
    },
    ParseReturnedNone {
        path: RelativePath,
        language: SourceLanguage,
    },
    SyntaxErrors {
        path: RelativePath,
        language: SourceLanguage,
    },
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigureParser { language, source } => {
                write!(f, "failed to configure {language} parser: {source}")
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

/// Parses all supported source files from a repository scan.
pub fn parse_repository_scan(
    repository_root: &Path,
    scan: &RepositoryScan,
) -> Result<Vec<ParsedFile>, ParseError> {
    let mut parsed_files = Vec::new();

    for file in &scan.files {
        let Some(language) = SourceLanguage::from_relative_path(&file.relative_path) else {
            continue;
        };

        let source_path = repository_root.join(file.relative_path.as_str());
        let source_text =
            fs::read_to_string(&source_path).map_err(|source| ParseError::ReadSource {
                path: file.relative_path.clone(),
                source,
            })?;

        let mut parser = language.parser()?;
        let tree =
            parser
                .parse(&source_text, None)
                .ok_or_else(|| ParseError::ParseReturnedNone {
                    path: file.relative_path.clone(),
                    language,
                })?;

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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{ParseError, SourceLanguage, parse_repository_scan};

    #[test]
    fn detects_rust_files_by_extension() {
        let path = crate::RelativePath::new("src/lib.rs").expect("relative path should build");

        assert_eq!(
            SourceLanguage::from_relative_path(&path),
            Some(SourceLanguage::Rust)
        );
    }

    #[test]
    fn skips_unsupported_files() {
        let tempdir = TestDir::new();
        tempdir.write_file("README.md", "# Leshy");

        let scan = crate::scan_repository(tempdir.path()).expect("scan should succeed");
        let parsed = parse_repository_scan(tempdir.path(), &scan).expect("parse should succeed");

        assert!(parsed.is_empty());
    }

    #[test]
    fn parses_valid_rust_files() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn meaning() -> i32 { 42 }\n");

        let scan = crate::scan_repository(tempdir.path()).expect("scan should succeed");
        let parsed = parse_repository_scan(tempdir.path(), &scan).expect("parse should succeed");

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].language, SourceLanguage::Rust);
        assert_eq!(parsed[0].relative_path.as_str(), "src/lib.rs");
        assert_eq!(parsed[0].tree.root_node().kind(), "source_file");
        assert_eq!(parsed[0].source_text, "pub fn meaning() -> i32 { 42 }\n");
    }

    #[test]
    fn surfaces_rust_syntax_errors() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "fn broken( {\n");

        let scan = crate::scan_repository(tempdir.path()).expect("scan should succeed");
        let error = parse_repository_scan(tempdir.path(), &scan).expect_err("parse should fail");

        assert!(matches!(
            error,
            ParseError::SyntaxErrors { ref path, language } if path.as_str() == "src/lib.rs"
                && language == SourceLanguage::Rust
        ));
        assert!(error.to_string().contains("syntax errors"));
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

            let unique = format!(
                "leshy-parse-test-{}-{}-{}",
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
