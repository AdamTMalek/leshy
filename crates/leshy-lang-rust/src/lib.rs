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
    let local_type_keys = collect_local_type_keys(
        parsed_file.tree.root_node(),
        parsed_file,
        &ExtractionContext::file(),
    );
    visit_item_list(
        parsed_file.tree.root_node(),
        parsed_file,
        &ExtractionContext::file(),
        &mut symbols,
        &local_type_keys,
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
    local_type_keys: &BTreeSet<String>,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        visit_item(child, parsed_file, context, symbols, local_type_keys);
    }
}

fn visit_item(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
    local_type_keys: &BTreeSet<String>,
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
            );

            if let Some(body) = node.child_by_field_name("body") {
                visit_item_list(
                    body,
                    parsed_file,
                    &context.module(stable_key, &name),
                    symbols,
                    local_type_keys,
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
            );

            if node.kind() == "trait_item"
                && let Some(body) = node.child_by_field_name("body")
            {
                visit_item_list(
                    body,
                    parsed_file,
                    &context.type_like(stable_key.clone(), stable_owner_name(&stable_key)),
                    symbols,
                    local_type_keys,
                );
            }
        }
        "impl_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let Some((impl_owner, nesting_owner)) =
                    impl_owner(node, parsed_file, namespace(context), local_type_keys)
                else {
                    return;
                };
                visit_item_list(
                    body,
                    parsed_file,
                    &context.impl_like(nesting_owner, impl_owner),
                    symbols,
                    local_type_keys,
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
        symbols.push(symbol);
    }
}

fn collect_local_type_keys(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    collect_local_type_keys_into(node, parsed_file, context, &mut keys);
    keys
}

fn collect_local_type_keys_into(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    keys: &mut BTreeSet<String>,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "mod_item" => {
                if let Some(name) = node_name(child, parsed_file)
                    && let Some(body) = child.child_by_field_name("body")
                {
                    let module_key = format!("module:{}", join_path(namespace(context), &name));
                    collect_local_type_keys_into(
                        body,
                        parsed_file,
                        &context.module(module_key, &name),
                        keys,
                    );
                }
            }
            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                if let Some(name) = node_name(child, parsed_file) {
                    keys.insert(format!("type:{}", join_path(namespace(context), &name)));
                }
            }
            _ => {}
        }
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
        (MemberKind::TypeLike { stable_owner }, "associated_type" | "type_item") => {
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
    local_type_keys: &BTreeSet<String>,
) -> Option<(String, NestingOwner)> {
    let type_node = node.child_by_field_name("type")?;
    let target = canonicalize_type_like_target(
        type_node
            .utf8_text(parsed_file.source_text.as_bytes())
            .ok()?,
        namespace,
        local_type_keys,
    );
    let nesting_owner = target
        .local_owner_key
        .clone()
        .map(NestingOwner::Symbol)
        .unwrap_or(NestingOwner::File);

    if let Some(trait_node) = node.child_by_field_name("trait") {
        let trait_name = canonicalize_type_like_target(
            trait_node
                .utf8_text(parsed_file.source_text.as_bytes())
                .ok()?,
            namespace,
            local_type_keys,
        );
        Some((
            format!("{} for {}", trait_name.stable_target, target.stable_target),
            nesting_owner,
        ))
    } else {
        Some((target.stable_target, nesting_owner))
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

fn compact_type_name(raw: &str) -> String {
    raw.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CanonicalTypeTarget {
    stable_target: String,
    local_owner_key: Option<String>,
}

fn canonicalize_type_like_target(
    raw: &str,
    namespace: &[String],
    local_type_keys: &BTreeSet<String>,
) -> CanonicalTypeTarget {
    let compact = compact_type_name(raw);
    let Some((path_prefix, suffix)) = split_path_prefix_and_suffix(&compact) else {
        return CanonicalTypeTarget {
            stable_target: compact,
            local_owner_key: None,
        };
    };

    let resolved = resolve_local_path(path_prefix, namespace, local_type_keys);
    let stable_prefix = resolved
        .stable_prefix
        .unwrap_or_else(|| path_prefix.to_string());

    CanonicalTypeTarget {
        stable_target: format!("{stable_prefix}{suffix}"),
        local_owner_key: resolved.local_owner_path.map(|path| format!("type:{path}")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedLocalPath {
    stable_prefix: Option<String>,
    local_owner_path: Option<String>,
}

fn resolve_local_path(
    path_prefix: &str,
    namespace: &[String],
    local_type_keys: &BTreeSet<String>,
) -> ResolvedLocalPath {
    let segments: Vec<&str> = path_prefix.split("::").collect();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return ResolvedLocalPath {
            stable_prefix: None,
            local_owner_path: None,
        };
    }

    if let Some(explicit) = canonicalize_explicit_prefix(&segments, namespace) {
        let path = explicit.join("::");
        return ResolvedLocalPath {
            stable_prefix: Some(path.clone()),
            local_owner_path: local_type_keys
                .contains(&format!("type:{path}"))
                .then_some(path),
        };
    }

    let relative = join_candidate(namespace, &segments);
    let rooted = segments.join("::");
    let relative_is_local = local_type_keys.contains(&format!("type:{relative}"));
    let rooted_is_local = local_type_keys.contains(&format!("type:{rooted}"));

    if relative_is_local {
        return ResolvedLocalPath {
            stable_prefix: Some(relative.clone()),
            local_owner_path: Some(relative),
        };
    }

    if rooted_is_local {
        return ResolvedLocalPath {
            stable_prefix: Some(rooted.clone()),
            local_owner_path: Some(rooted),
        };
    }

    ResolvedLocalPath {
        stable_prefix: None,
        local_owner_path: None,
    }
}

fn canonicalize_explicit_prefix(segments: &[&str], namespace: &[String]) -> Option<Vec<String>> {
    let mut index = 0usize;
    let mut resolved = if segments.first().copied() == Some("crate") {
        index = 1;
        Vec::new()
    } else if segments.first().copied() == Some("self") {
        index = 1;
        namespace.to_vec()
    } else if segments.first().copied() == Some("super") {
        namespace.to_vec()
    } else {
        return None;
    };

    while segments.get(index).copied() == Some("super") {
        resolved.pop()?;
        index += 1;
    }

    while segments.get(index).copied() == Some("self") {
        index += 1;
    }

    if index >= segments.len() {
        return None;
    }

    resolved.extend(
        segments[index..]
            .iter()
            .map(|segment| (*segment).to_string()),
    );
    Some(resolved)
}

fn join_candidate(namespace: &[String], segments: &[&str]) -> String {
    if namespace.is_empty() {
        segments.join("::")
    } else {
        let mut joined = namespace.join("::");
        joined.push_str("::");
        joined.push_str(&segments.join("::"));
        joined
    }
}

fn split_path_prefix_and_suffix(compact: &str) -> Option<(&str, &str)> {
    if compact.is_empty()
        || compact.starts_with('&')
        || compact.starts_with('*')
        || compact.starts_with('(')
        || compact.starts_with('[')
        || compact.starts_with("fn(")
        || compact.starts_with("extern\"")
        || compact.starts_with("unsafefn(")
        || compact.starts_with('<')
        || compact.starts_with("dyn")
    {
        return None;
    }

    let mut angle_depth = 0usize;
    for (index, ch) in compact.char_indices() {
        match ch {
            '<' if angle_depth == 0 => return Some((&compact[..index], &compact[index..])),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ => {}
        }
    }

    Some((compact, ""))
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

    #[test]
    fn preserves_generic_impl_targets_in_method_ids_and_local_owners() {
        let source = r#"
struct Wrapper<T>(T);

impl<T> Wrapper<T> {
    fn into_inner(self) -> T {
        self.0
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
        let method = symbols
            .iter()
            .find(|symbol| symbol.display_name == "into_inner")
            .expect("method should exist");

        assert_eq!(method.kind, SymbolKind::Method);
        assert_eq!(method.stable_key, "method:Wrapper<T>::into_inner");
        assert_eq!(
            method.owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
    }

    #[test]
    fn preserves_concrete_inherent_impl_targets_in_member_ids() {
        let source = r#"
struct Wrapper<T>(T);

impl Wrapper<u8> {
    const KIND: u8 = 1;

    fn new() -> Self {
        Self(0)
    }
}

impl Wrapper<String> {
    const KIND: u8 = 2;

    fn new() -> Self {
        Self(String::new())
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
        let new_methods: Vec<&leshy_core::ExtractedSymbol> = symbols
            .iter()
            .filter(|symbol| symbol.display_name == "new" && symbol.kind == SymbolKind::Method)
            .collect();
        let kind_constants: Vec<&leshy_core::ExtractedSymbol> = symbols
            .iter()
            .filter(|symbol| symbol.display_name == "KIND" && symbol.kind == SymbolKind::Constant)
            .collect();

        assert_eq!(new_methods.len(), 2);
        assert_eq!(new_methods[0].stable_key, "method:Wrapper<u8>::new");
        assert_eq!(new_methods[1].stable_key, "method:Wrapper<String>::new");
        assert_ne!(new_methods[0].id, new_methods[1].id);
        assert_eq!(
            new_methods[0].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
        assert_eq!(
            new_methods[1].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );

        assert_eq!(kind_constants.len(), 2);
        assert_eq!(kind_constants[0].stable_key, "const:Wrapper<u8>::KIND");
        assert_eq!(kind_constants[1].stable_key, "const:Wrapper<String>::KIND");
        assert_ne!(kind_constants[0].id, kind_constants[1].id);
        assert_eq!(
            kind_constants[0].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
        assert_eq!(
            kind_constants[1].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
    }

    #[test]
    fn preserves_concrete_trait_impl_targets_in_method_ids() {
        let source = r#"
trait Marker {
    fn mark(&self);
}

struct Wrapper<T>(T);

impl Marker for Wrapper<u8> {
    fn mark(&self) {}
}

impl Marker for Wrapper<String> {
    fn mark(&self) {}
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
        let mark_methods: Vec<&leshy_core::ExtractedSymbol> = symbols
            .iter()
            .filter(|symbol| symbol.display_name == "mark" && symbol.kind == SymbolKind::Method)
            .collect();

        assert_eq!(mark_methods.len(), 3);
        assert_eq!(mark_methods[0].stable_key, "method:Marker::mark");
        assert_eq!(
            mark_methods[1].stable_key,
            "method:Marker for Wrapper<u8>::mark"
        );
        assert_eq!(
            mark_methods[2].stable_key,
            "method:Marker for Wrapper<String>::mark"
        );
        assert_ne!(mark_methods[1].id, mark_methods[2].id);
        assert_eq!(
            mark_methods[1].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
        assert_eq!(
            mark_methods[2].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Wrapper"))
        );
    }

    #[test]
    fn canonicalizes_same_crate_qualified_impl_targets() {
        let source = r#"
trait Assoc {
    type Item;
}

mod outer {
    pub struct Widget;

    impl self::Widget {
        fn from_self() -> Self {
            Self
        }
    }

    mod inner {
        impl super::Widget {
            const LABEL: &'static str = "inner";
        }
    }
}

impl crate::outer::Widget {
    fn from_crate() -> Self {
        crate::outer::Widget
    }
}

impl Assoc for crate::outer::Widget {
    type Item = u8;
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
        let from_self = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "method:outer::Widget::from_self")
            .expect("self-qualified method should exist");
        let from_crate = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "method:outer::Widget::from_crate")
            .expect("crate-qualified method should exist");
        let label = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "const:outer::Widget::LABEL")
            .expect("super-qualified constant should exist");
        let assoc_item = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "type:Assoc for outer::Widget::Item")
            .expect("crate-qualified associated type should exist");

        let widget_owner = leshy_core::SymbolOwner::Symbol(SymbolId::new(
            parsed_file.file_id,
            "type:outer::Widget",
        ));

        assert_eq!(from_self.owner, widget_owner);
        assert_eq!(from_crate.owner, widget_owner);
        assert_eq!(label.owner, widget_owner);
        assert_eq!(assoc_item.owner, widget_owner);
    }

    #[test]
    fn canonicalizes_specialized_same_crate_impl_targets() {
        let source = r#"
trait Assoc {
    type Item;
}

mod outer {
    pub struct Wrapper<T>(pub T);
}

impl crate::outer::Wrapper<u8> {
    fn from_u8() -> Self {
        crate::outer::Wrapper(0)
    }
}

impl self::outer::Wrapper<String> {
    const KIND: &'static str = "string";
}

impl Assoc for crate::outer::Wrapper<u8> {
    type Item = u8;
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
        let from_u8 = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "method:outer::Wrapper<u8>::from_u8")
            .expect("specialized method should exist");
        let kind = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "const:outer::Wrapper<String>::KIND")
            .expect("specialized constant should exist");
        let assoc_item = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "type:Assoc for outer::Wrapper<u8>::Item")
            .expect("specialized associated type should exist");

        let wrapper_owner = leshy_core::SymbolOwner::Symbol(SymbolId::new(
            parsed_file.file_id,
            "type:outer::Wrapper",
        ));

        assert_eq!(from_u8.owner, wrapper_owner);
        assert_eq!(kind.owner, wrapper_owner);
        assert_eq!(assoc_item.owner, wrapper_owner);
    }

    #[test]
    fn scopes_associated_type_keys_to_the_enclosing_trait_impl() {
        let source = r#"
trait A {
    type Item;
}

trait B {
    type Item;
}

struct Stream;

impl A for Stream {
    type Item = u8;
}

impl B for Stream {
    type Item = u16;
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
        let associated_types: Vec<&leshy_core::ExtractedSymbol> = symbols
            .iter()
            .filter(|symbol| symbol.display_name == "Item" && symbol.kind == SymbolKind::Type)
            .collect();

        assert_eq!(associated_types.len(), 4);
        assert_eq!(associated_types[0].stable_key, "type:A::Item");
        assert_eq!(associated_types[1].stable_key, "type:B::Item");
        assert_eq!(associated_types[2].stable_key, "type:A for Stream::Item");
        assert_eq!(associated_types[3].stable_key, "type:B for Stream::Item");
        assert_ne!(associated_types[2].id, associated_types[3].id);
        assert_eq!(
            associated_types[2].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Stream"))
        );
        assert_eq!(
            associated_types[3].owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Stream"))
        );
    }
}
