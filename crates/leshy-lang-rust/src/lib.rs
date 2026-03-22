use std::path::Path;

use leshy_core::{ExtractedSymbol, SourcePosition, SourceSpan, SymbolKind};
use leshy_parser::{LanguageId, LanguagePlugin, LanguagePluginError, ParsedFile};
use tree_sitter::{Node, Parser, Tree};

pub static RUST_LANGUAGE_PLUGIN: RustLanguagePlugin = RustLanguagePlugin;
pub const RUST_LANGUAGE_ID: LanguageId = LanguageId::new("rust");

pub struct RustLanguagePlugin;

pub fn supports_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(std::ffi::OsStr::to_str),
        Some("rs")
    )
}

pub fn parse_source(source_text: &str) -> Result<Tree, LanguagePluginError> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|source| LanguagePluginError::ConfigureParser { source })?;

    parser
        .parse(source_text, None)
        .ok_or(LanguagePluginError::ParseReturnedNone)
}

pub fn extract_symbols(parsed_file: &ParsedFile) -> Vec<ExtractedSymbol> {
    let mut symbols = Vec::new();
    visit_item_list(
        parsed_file.tree.root_node(),
        parsed_file,
        ExtractionContext::File,
        &mut symbols,
    );
    symbols
}

impl LanguagePlugin for RustLanguagePlugin {
    fn language(&self) -> LanguageId {
        RUST_LANGUAGE_ID
    }

    fn supports_path(&self, path: &Path) -> bool {
        supports_path(path)
    }

    fn parse_source(&self, source_text: &str) -> Result<Tree, LanguagePluginError> {
        parse_source(source_text)
    }

    fn extract_symbols(&self, parsed_file: &ParsedFile) -> Vec<ExtractedSymbol> {
        extract_symbols(parsed_file)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtractionContext {
    File,
    Trait,
    Impl,
}

fn visit_item_list(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        visit_item(child, parsed_file, context, symbols);
    }
}

fn visit_item(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    match node.kind() {
        "mod_item" => {
            push_symbol(symbols, parsed_file, node, SymbolKind::Module);

            if let Some(body) = node.child_by_field_name("body") {
                visit_item_list(body, parsed_file, ExtractionContext::File, symbols);
            }
        }
        "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item"
        | "associated_type" => {
            push_symbol(symbols, parsed_file, node, SymbolKind::Type);

            if node.kind() == "trait_item"
                && let Some(body) = node.child_by_field_name("body")
            {
                visit_item_list(body, parsed_file, ExtractionContext::Trait, symbols);
            }
        }
        "impl_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                visit_item_list(body, parsed_file, ExtractionContext::Impl, symbols);
            }
        }
        "function_item" | "function_signature_item" => {
            let kind = match context {
                ExtractionContext::Trait | ExtractionContext::Impl => SymbolKind::Method,
                ExtractionContext::File => SymbolKind::Function,
            };
            push_symbol(symbols, parsed_file, node, kind);
        }
        "const_item" | "static_item" => {
            push_symbol(symbols, parsed_file, node, SymbolKind::Constant);
        }
        _ => {}
    }
}

fn push_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    parsed_file: &ParsedFile,
    node: Node<'_>,
    kind: SymbolKind,
) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let Ok(display_name) = name_node.utf8_text(parsed_file.source_text.as_bytes()) else {
        return;
    };
    let range = node.range();

    if let Ok(symbol) = ExtractedSymbol::new(
        parsed_file.file_id,
        parsed_file.relative_path.clone(),
        kind,
        display_name,
        SourceSpan::new(
            range.start_byte,
            range.end_byte,
            SourcePosition::new(range.start_point.row, range.start_point.column),
            SourcePosition::new(range.end_point.row, range.end_point.column),
        ),
    ) {
        symbols.push(symbol);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use leshy_core::{FileId, RelativePath, RepositoryId, SymbolKind};
    use leshy_parser::{LanguageId, ParsedFile};

    use super::{extract_symbols, parse_source, supports_path};

    #[test]
    fn matches_rust_source_files() {
        assert!(supports_path(Path::new("src/lib.rs")));
        assert!(!supports_path(Path::new("README.md")));
    }

    #[test]
    fn parses_valid_rust_source() {
        let tree = parse_source("pub fn meaning() -> i32 { 42 }\n").expect("parse should succeed");

        assert_eq!(tree.root_node().kind(), "source_file");
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn returns_tree_with_errors_for_invalid_rust_source() {
        let tree = parse_source("fn broken( {\n").expect("parse should still return a tree");

        assert!(tree.root_node().has_error());
    }

    #[test]
    fn extracts_primary_rust_symbols_from_a_file() {
        let source = r#"
pub mod nested {
    pub struct Widget;
    pub enum Mode {
        Basic,
    }

    pub trait Runnable {
        fn run(&self);
    }

    pub const LIMIT: usize = 8;
}

pub type Alias = nested::Widget;
pub static CACHE: usize = 1;
pub fn build() {}

impl nested::Widget {
    pub fn new() -> Self {
        Self
    }
}
"#;
        let tree = parse_source(source).expect("parse should succeed");
        let relative_path = RelativePath::new("src/lib.rs").expect("relative path should build");
        let parsed_file = ParsedFile {
            file_id: FileId::new(RepositoryId::new("repository"), &relative_path),
            relative_path,
            language: LanguageId::new("rust"),
            source_text: source.to_string(),
            tree,
        };

        let symbols = extract_symbols(&parsed_file);
        let extracted: Vec<(String, SymbolKind)> = symbols
            .iter()
            .map(|symbol| (symbol.display_name.clone(), symbol.kind))
            .collect();

        assert_eq!(
            extracted,
            vec![
                ("nested".to_string(), SymbolKind::Module),
                ("Widget".to_string(), SymbolKind::Type),
                ("Mode".to_string(), SymbolKind::Type),
                ("Runnable".to_string(), SymbolKind::Type),
                ("run".to_string(), SymbolKind::Method),
                ("LIMIT".to_string(), SymbolKind::Constant),
                ("Alias".to_string(), SymbolKind::Type),
                ("CACHE".to_string(), SymbolKind::Constant),
                ("build".to_string(), SymbolKind::Function),
                ("new".to_string(), SymbolKind::Method),
            ]
        );

        let nested_module = &symbols[0];
        assert_eq!(nested_module.span.start.line, 1);
        assert_eq!(nested_module.span.start.column, 0);
        assert!(nested_module.span.end_byte > nested_module.span.start_byte);
    }
}
