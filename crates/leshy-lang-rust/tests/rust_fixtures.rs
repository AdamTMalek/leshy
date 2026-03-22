use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use leshy_core::{SymbolKind, SymbolOwner, scan_repository};
use leshy_lang_rust::RUST_LANGUAGE_PLUGIN;
use leshy_parser::{LanguageRegistry, ParseError, extract_symbols, parse_repository_scan};

#[test]
fn extracts_expected_symbols_from_shared_fixture_crate() {
    let root = fixture_root("mini_crate");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);
    let scan = scan_repository(&root).expect("fixture crate should scan");
    let parsed_files =
        parse_repository_scan(&root, &scan, &registry).expect("fixture crate should parse");

    let parsed_paths: Vec<&str> = parsed_files
        .iter()
        .map(|parsed_file| parsed_file.relative_path.as_str())
        .collect();
    assert_eq!(
        parsed_paths,
        vec![
            "src/lib.rs",
            "src/model.rs",
            "src/service/cache.rs",
            "src/service/mod.rs",
        ]
    );

    let symbols = extract_symbols(&parsed_files, &registry);
    let cases = [
        FixtureCase {
            relative_path: "src/lib.rs",
            expected: &[
                ExpectedSymbol::new("model", SymbolKind::Module, "module:model"),
                ExpectedSymbol::new("service", SymbolKind::Module, "module:service"),
                ExpectedSymbol::new(
                    "DEFAULT_BATCH_SIZE",
                    SymbolKind::Constant,
                    "const:DEFAULT_BATCH_SIZE",
                ),
                ExpectedSymbol::new("bootstrap", SymbolKind::Function, "fn:bootstrap"),
            ],
        },
        FixtureCase {
            relative_path: "src/model.rs",
            expected: &[
                ExpectedSymbol::new("RecordId", SymbolKind::Type, "type:model::RecordId"),
                ExpectedSymbol::new("Record", SymbolKind::Type, "type:model::Record"),
                ExpectedSymbol::new("Status", SymbolKind::Type, "type:model::Status"),
                ExpectedSymbol::new(
                    "DEFAULT_NAME",
                    SymbolKind::Constant,
                    "const:model::DEFAULT_NAME",
                ),
                ExpectedSymbol::new("new", SymbolKind::Method, "method:model::Record::new"),
                ExpectedSymbol::new(
                    "fmt",
                    SymbolKind::Method,
                    "method:std::fmt::Display for model::RecordId::fmt",
                ),
            ],
        },
        FixtureCase {
            relative_path: "src/service/cache.rs",
            expected: &[
                ExpectedSymbol::new(
                    "CacheEntry",
                    SymbolKind::Type,
                    "type:service::cache::CacheEntry",
                ),
                ExpectedSymbol::new(
                    "CACHE_CAPACITY",
                    SymbolKind::Constant,
                    "const:service::cache::CACHE_CAPACITY",
                ),
                ExpectedSymbol::new(
                    "new",
                    SymbolKind::Method,
                    "method:service::cache::CacheEntry::new",
                ),
            ],
        },
        FixtureCase {
            relative_path: "src/service/mod.rs",
            expected: &[
                ExpectedSymbol::new("cache", SymbolKind::Module, "module:service::cache"),
                ExpectedSymbol::new("Repository", SymbolKind::Type, "type:service::Repository"),
                ExpectedSymbol::new("Error", SymbolKind::Type, "type:service::Repository::Error"),
                ExpectedSymbol::new(
                    "load",
                    SymbolKind::Method,
                    "method:service::Repository::load",
                ),
                ExpectedSymbol::new("Store", SymbolKind::Type, "type:service::Store"),
                ExpectedSymbol::new("new", SymbolKind::Method, "method:service::Store::new"),
                ExpectedSymbol::new("fetch", SymbolKind::Method, "method:service::Store::fetch"),
            ],
        },
    ];

    for case in cases {
        assert_symbols_for_path(&symbols, case);
    }

    let anchors = [
        SpanAnchor::new("src/lib.rs", "bootstrap", 8, 0),
        SpanAnchor::new("src/model.rs", "Record", 4, 0),
        SpanAnchor::new("src/model.rs", "fmt", 23, 4),
        SpanAnchor::new("src/service/mod.rs", "Repository", 4, 0),
        SpanAnchor::new("src/service/cache.rs", "CacheEntry", 2, 0),
    ];

    for anchor in anchors {
        assert_span_anchor(&symbols, anchor);
    }
}

#[test]
fn reports_syntax_errors_for_invalid_fixture_crate() {
    let root = fixture_root("invalid_crate");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);
    let scan = scan_repository(&root).expect("fixture crate should scan");
    let error =
        parse_repository_scan(&root, &scan, &registry).expect_err("invalid fixture should fail");

    assert!(matches!(
        error,
        ParseError::SyntaxErrors { ref path, .. } if path.as_str() == "src/lib.rs"
    ));
}

#[test]
fn resolves_cross_file_impl_targets_and_imported_type_names() {
    let tempdir = TestDir::new();
    tempdir.write_file(
        "src/lib.rs",
        "mod model;\nuse crate::model::Record;\nimpl model::Record { fn from_module() -> Self { Self } }\nimpl Record { fn from_import() -> Self { Self } }\n",
    );
    tempdir.write_file("src/model.rs", "pub struct Record;\n");

    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);
    let scan = scan_repository(tempdir.path()).expect("crate should scan");
    let parsed_files =
        parse_repository_scan(tempdir.path(), &scan, &registry).expect("crate should parse");
    let symbols = extract_symbols(&parsed_files, &registry);
    let lib_file = parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");
    let model_file = parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/model.rs")
        .expect("model file should be parsed");

    let from_module = symbols
        .iter()
        .find(|symbol| symbol.stable_key == "method:model::Record::from_module")
        .expect("module-qualified impl method should exist");
    let from_import = symbols
        .iter()
        .find(|symbol| symbol.stable_key == "method:model::Record::from_import")
        .expect("import-qualified impl method should exist");

    let owner = SymbolOwner::Symbol(leshy_core::SymbolId::new(
        model_file.file_id,
        "type:model::Record",
    ));

    assert_eq!(from_module.file_id, lib_file.file_id);
    assert_eq!(from_import.file_id, lib_file.file_id);
    assert_eq!(from_module.owner, owner);
    assert_eq!(from_import.owner, owner);
}

fn assert_symbols_for_path(symbols: &[leshy_core::ExtractedSymbol], case: FixtureCase<'_>) {
    let extracted: Vec<(String, SymbolKind, String)> = symbols
        .iter()
        .filter(|symbol| symbol.relative_path.as_str() == case.relative_path)
        .map(|symbol| {
            (
                symbol.display_name.clone(),
                symbol.kind,
                symbol.stable_key.clone(),
            )
        })
        .collect();
    let expected: Vec<(String, SymbolKind, String)> = case
        .expected
        .iter()
        .map(|symbol| {
            (
                symbol.display_name.to_string(),
                symbol.kind,
                symbol.stable_key.to_string(),
            )
        })
        .collect();

    assert_eq!(
        extracted, expected,
        "unexpected symbols for {}",
        case.relative_path
    );
}

fn assert_span_anchor(symbols: &[leshy_core::ExtractedSymbol], anchor: SpanAnchor<'_>) {
    let symbol = symbols
        .iter()
        .find(|symbol| {
            symbol.relative_path.as_str() == anchor.relative_path
                && symbol.display_name == anchor.display_name
        })
        .unwrap_or_else(|| {
            panic!(
                "missing anchor {} in {}",
                anchor.display_name, anchor.relative_path
            )
        });

    assert_eq!(
        symbol.span.start.line, anchor.line,
        "wrong line for {} in {}",
        anchor.display_name, anchor.relative_path
    );
    assert_eq!(
        symbol.span.start.column, anchor.column,
        "wrong column for {} in {}",
        anchor.display_name, anchor.relative_path
    );
    assert!(symbol.span.end_byte > symbol.span.start_byte);
}

fn fixture_root(name: &str) -> PathBuf {
    workspace_root()
        .join("tests")
        .join("fixtures")
        .join("rust")
        .join(name)
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate should live in the workspace")
}

#[derive(Clone, Copy)]
struct FixtureCase<'a> {
    relative_path: &'a str,
    expected: &'a [ExpectedSymbol],
}

#[derive(Clone, Copy)]
struct ExpectedSymbol {
    display_name: &'static str,
    kind: SymbolKind,
    stable_key: &'static str,
}

impl ExpectedSymbol {
    const fn new(display_name: &'static str, kind: SymbolKind, stable_key: &'static str) -> Self {
        Self {
            display_name,
            kind,
            stable_key,
        }
    }
}

#[derive(Clone, Copy)]
struct SpanAnchor<'a> {
    relative_path: &'a str,
    display_name: &'a str,
    line: usize,
    column: usize,
}

impl<'a> SpanAnchor<'a> {
    const fn new(
        relative_path: &'a str,
        display_name: &'a str,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            relative_path,
            display_name,
            line,
            column,
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let unique = format!(
            "leshy-lang-rust-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir(&path).expect("temporary directory should be created");

        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_file(&self, relative_path: &str, contents: &str) {
        let file_path = self.path.join(relative_path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("parent directories should be created");
        }

        std::fs::write(file_path, contents).expect("file contents should be written");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
