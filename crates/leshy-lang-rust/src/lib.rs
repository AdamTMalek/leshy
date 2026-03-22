use std::collections::BTreeSet;
use std::path::Path;

use leshy_core::{ExtractedSymbol, SourcePosition, SourceSpan, SymbolKind, SymbolOwner};
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
    let mut defined_symbols = BTreeSet::new();
    visit_item_list(
        parsed_file.tree.root_node(),
        parsed_file,
        &ExtractionContext::file(),
        &mut symbols,
        &mut defined_symbols,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExtractionContext {
    namespace: Vec<String>,
    owner: NestingOwner,
    member_kind: MemberKind,
}

impl ExtractionContext {
    fn file() -> Self {
        Self {
            namespace: Vec::new(),
            owner: NestingOwner::File,
            member_kind: MemberKind::FileLike,
        }
    }

    fn module(&self, owner_key: String, segment: &str) -> Self {
        Self {
            namespace: extend_namespace(self, segment),
            owner: NestingOwner::Symbol(owner_key),
            member_kind: MemberKind::FileLike,
        }
    }

    fn type_like(&self, owner_key: String, stable_owner: String) -> Self {
        Self {
            namespace: self.namespace.clone(),
            owner: NestingOwner::Symbol(owner_key),
            member_kind: MemberKind::TypeLike { stable_owner },
        }
    }

    fn impl_like(&self, owner: NestingOwner, stable_owner: String) -> Self {
        Self {
            namespace: self.namespace.clone(),
            owner,
            member_kind: MemberKind::TypeLike { stable_owner },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NestingOwner {
    File,
    Symbol(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MemberKind {
    FileLike,
    TypeLike { stable_owner: String },
}

fn visit_item_list(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
    defined_symbols: &mut BTreeSet<String>,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        visit_item(child, parsed_file, context, symbols, defined_symbols);
    }
}

fn visit_item(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
    defined_symbols: &mut BTreeSet<String>,
) {
    match node.kind() {
        "mod_item" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let module_path = join_path(namespace(context), &name);
            let stable_key = format!("module:{module_path}");
            push_symbol(
                symbols,
                parsed_file,
                owner(context, parsed_file.file_id),
                node,
                SymbolKind::Module,
                stable_key.clone(),
                defined_symbols,
            );

            if let Some(body) = node.child_by_field_name("body") {
                visit_item_list(
                    body,
                    parsed_file,
                    &context.module(stable_key, &name),
                    symbols,
                    defined_symbols,
                );
            }
        }
        "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item"
        | "associated_type" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let stable_key = type_stable_key(node, context, &name);
            push_symbol(
                symbols,
                parsed_file,
                owner(context, parsed_file.file_id),
                node,
                SymbolKind::Type,
                stable_key.clone(),
                defined_symbols,
            );

            if node.kind() == "trait_item"
                && let Some(body) = node.child_by_field_name("body")
            {
                visit_item_list(
                    body,
                    parsed_file,
                    &context.type_like(stable_key.clone(), stable_owner_name(&stable_key)),
                    symbols,
                    defined_symbols,
                );
            }
        }
        "impl_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let Some((impl_owner, nesting_owner)) =
                    impl_owner(node, parsed_file, namespace(context), defined_symbols)
                else {
                    return;
                };
                visit_item_list(
                    body,
                    parsed_file,
                    &context.impl_like(nesting_owner, impl_owner),
                    symbols,
                    defined_symbols,
                );
            }
        }
        "function_item" | "function_signature_item" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let (kind, stable_key) = function_symbol(context, &name);
            push_symbol(
                symbols,
                parsed_file,
                owner(context, parsed_file.file_id),
                node,
                kind,
                stable_key,
                defined_symbols,
            );
        }
        "const_item" | "static_item" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let stable_key = constant_stable_key(context, &name);
            push_symbol(
                symbols,
                parsed_file,
                owner(context, parsed_file.file_id),
                node,
                SymbolKind::Constant,
                stable_key,
                defined_symbols,
            );
        }
        _ => {}
    }
}

fn push_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    parsed_file: &ParsedFile,
    owner: SymbolOwner,
    node: Node<'_>,
    kind: SymbolKind,
    stable_key: String,
    defined_symbols: &mut BTreeSet<String>,
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
        owner,
        parsed_file.relative_path.clone(),
        kind,
        display_name,
        stable_key,
        SourceSpan::new(
            range.start_byte,
            range.end_byte,
            SourcePosition::new(range.start_point.row, range.start_point.column),
            SourcePosition::new(range.end_point.row, range.end_point.column),
        ),
    ) {
        defined_symbols.insert(symbol.stable_key.clone());
        symbols.push(symbol);
    }
}

fn namespace(context: &ExtractionContext) -> &[String] {
    &context.namespace
}

fn extend_namespace(context: &ExtractionContext, segment: &str) -> Vec<String> {
    let mut segments = namespace(context).to_vec();
    segments.push(segment.to_string());
    segments
}

fn join_path(namespace: &[String], name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{}::{name}", namespace.join("::"))
    }
}

fn node_name(node: Node<'_>, parsed_file: &ParsedFile) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let text = name_node
        .utf8_text(parsed_file.source_text.as_bytes())
        .ok()?;
    Some(text.to_string())
}

fn type_stable_key(node: Node<'_>, context: &ExtractionContext, name: &str) -> String {
    match (&context.member_kind, node.kind()) {
        (MemberKind::TypeLike { stable_owner }, "associated_type") => {
            format!("type:{stable_owner}::{name}")
        }
        _ => format!("type:{}", join_path(namespace(context), name)),
    }
}

fn function_symbol(context: &ExtractionContext, name: &str) -> (SymbolKind, String) {
    match &context.member_kind {
        MemberKind::FileLike => (
            SymbolKind::Function,
            format!("fn:{}", join_path(namespace(context), name)),
        ),
        MemberKind::TypeLike { stable_owner } => {
            (SymbolKind::Method, format!("method:{stable_owner}::{name}"))
        }
    }
}

fn impl_owner(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    namespace: &[String],
    defined_symbols: &BTreeSet<String>,
) -> Option<(String, NestingOwner)> {
    let type_node = node.child_by_field_name("type")?;
    let type_name = type_node
        .utf8_text(parsed_file.source_text.as_bytes())
        .ok()?
        .split_whitespace()
        .collect::<String>();
    let qualified_type = if type_name.contains("::") || namespace.is_empty() || type_name == "Self"
    {
        type_name
    } else {
        format!("{}::{type_name}", namespace.join("::"))
    };
    let type_key = format!("type:{qualified_type}");
    let nesting_owner = if defined_symbols.contains(&type_key) {
        NestingOwner::Symbol(type_key)
    } else {
        NestingOwner::File
    };

    if let Some(trait_node) = node.child_by_field_name("trait") {
        let trait_name = trait_node
            .utf8_text(parsed_file.source_text.as_bytes())
            .ok()?
            .split_whitespace()
            .collect::<String>();
        Some((format!("{trait_name} for {qualified_type}"), nesting_owner))
    } else {
        Some((qualified_type, nesting_owner))
    }
}

fn constant_stable_key(context: &ExtractionContext, name: &str) -> String {
    match &context.member_kind {
        MemberKind::FileLike => format!("const:{}", join_path(namespace(context), name)),
        MemberKind::TypeLike { stable_owner } => format!("const:{stable_owner}::{name}"),
    }
}

fn stable_owner_name(stable_key: &str) -> String {
    stable_key
        .split_once(':')
        .map(|(_, value)| value.to_string())
        .unwrap_or_else(|| stable_key.to_string())
}

fn owner(context: &ExtractionContext, file_id: leshy_core::FileId) -> SymbolOwner {
    match &context.owner {
        NestingOwner::File => SymbolOwner::File(file_id),
        NestingOwner::Symbol(stable_key) => {
            SymbolOwner::Symbol(leshy_core::SymbolId::new(file_id, stable_key))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use leshy_core::{FileId, RelativePath, RepositoryId, SymbolId, SymbolKind};
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
        assert_eq!(nested_module.stable_key, "module:nested");
        assert_eq!(
            nested_module.id,
            SymbolId::new(parsed_file.file_id, "module:nested")
        );
        assert_eq!(nested_module.span.start.line, 1);
        assert_eq!(nested_module.span.start.column, 0);
        assert!(nested_module.span.end_byte > nested_module.span.start_byte);
        assert_eq!(symbols[4].stable_key, "method:nested::Runnable::run");
        assert_eq!(symbols[9].stable_key, "method:nested::Widget::new");
        assert_eq!(
            symbols[9].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(
                parsed_file.file_id,
                "type:nested::Widget"
            ))
        );
    }

    #[test]
    fn keeps_functions_inside_modules_classified_as_functions() {
        let source = r#"
mod nested {
    fn helper() {}
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

        assert_eq!(symbols[0].stable_key, "module:nested");
        assert_eq!(symbols[1].kind, SymbolKind::Function);
        assert_eq!(symbols[1].stable_key, "fn:nested::helper");
        assert_eq!(
            symbols[1].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "module:nested"))
        );
    }

    #[test]
    fn distinguishes_methods_from_multiple_trait_impls_for_the_same_type() {
        let source = r#"
trait Read {
    fn read(&self);
}

trait Write {
    fn read(&self);
}

struct Stream;

impl Read for Stream {
    fn read(&self) {}
}

impl Write for Stream {
    fn read(&self) {}
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
        let read_methods: Vec<&leshy_core::ExtractedSymbol> = symbols
            .iter()
            .filter(|symbol| symbol.display_name == "read" && symbol.kind == SymbolKind::Method)
            .collect();

        assert_eq!(read_methods.len(), 4);
        assert_eq!(read_methods[0].stable_key, "method:Read::read");
        assert_eq!(read_methods[1].stable_key, "method:Write::read");
        assert_eq!(read_methods[2].stable_key, "method:Read for Stream::read");
        assert_eq!(read_methods[3].stable_key, "method:Write for Stream::read");
        assert_ne!(read_methods[2].id, read_methods[3].id);
    }
}
