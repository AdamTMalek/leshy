use std::path::{Path, PathBuf};

use leshy_core::{SymbolKind, scan_repository};
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
                ExpectedSymbol::new("model", SymbolKind::Module),
                ExpectedSymbol::new("service", SymbolKind::Module),
                ExpectedSymbol::new("DEFAULT_BATCH_SIZE", SymbolKind::Constant),
                ExpectedSymbol::new("bootstrap", SymbolKind::Function),
            ],
        },
        FixtureCase {
            relative_path: "src/model.rs",
            expected: &[
                ExpectedSymbol::new("RecordId", SymbolKind::Type),
                ExpectedSymbol::new("Record", SymbolKind::Type),
                ExpectedSymbol::new("Status", SymbolKind::Type),
                ExpectedSymbol::new("DEFAULT_NAME", SymbolKind::Constant),
                ExpectedSymbol::new("new", SymbolKind::Method),
                ExpectedSymbol::new("fmt", SymbolKind::Method),
            ],
        },
        FixtureCase {
            relative_path: "src/service/cache.rs",
            expected: &[
                ExpectedSymbol::new("CacheEntry", SymbolKind::Type),
                ExpectedSymbol::new("CACHE_CAPACITY", SymbolKind::Constant),
                ExpectedSymbol::new("new", SymbolKind::Method),
            ],
        },
        FixtureCase {
            relative_path: "src/service/mod.rs",
            expected: &[
                ExpectedSymbol::new("cache", SymbolKind::Module),
                ExpectedSymbol::new("Repository", SymbolKind::Type),
                ExpectedSymbol::new("Error", SymbolKind::Type),
                ExpectedSymbol::new("load", SymbolKind::Method),
                ExpectedSymbol::new("Store", SymbolKind::Type),
                ExpectedSymbol::new("new", SymbolKind::Method),
                ExpectedSymbol::new("fetch", SymbolKind::Method),
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

fn assert_symbols_for_path(symbols: &[leshy_core::ExtractedSymbol], case: FixtureCase<'_>) {
    let extracted: Vec<(String, SymbolKind)> = symbols
        .iter()
        .filter(|symbol| symbol.relative_path.as_str() == case.relative_path)
        .map(|symbol| (symbol.display_name.clone(), symbol.kind))
        .collect();
    let expected: Vec<(String, SymbolKind)> = case
        .expected
        .iter()
        .map(|symbol| (symbol.display_name.to_string(), symbol.kind))
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
}

impl ExpectedSymbol {
    const fn new(display_name: &'static str, kind: SymbolKind) -> Self {
        Self { display_name, kind }
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
