use std::collections::BTreeMap;
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
    let context = ExtractionContext::file(parsed_file, None);
    let use_aliases = collect_use_aliases(parsed_file, &context);
    extract_symbols_with_resolution(
        parsed_file,
        &context,
        &collect_local_type_keys(parsed_file.tree.root_node(), parsed_file, &context),
        &use_aliases,
    )
}

fn extract_symbols_with_resolution(
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    local_type_keys: &TypeOwners,
    use_aliases: &UseAliases,
) -> Vec<ExtractedSymbol> {
    let mut symbols = Vec::new();
    visit_item_list(
        parsed_file.tree.root_node(),
        parsed_file,
        context,
        &mut symbols,
        local_type_keys,
        use_aliases,
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

    fn finalize_symbols(
        &self,
        parsed_files: &[&ParsedFile],
        symbols_by_file: &mut BTreeMap<leshy_core::FileId, Vec<ExtractedSymbol>>,
    ) {
        let repository_keys = collect_repository_symbol_owners(parsed_files);
        let repository_aliases = collect_repository_use_aliases(parsed_files, &repository_keys);

        for parsed_file in parsed_files {
            let Some(crate_scope) = resolved_crate_scope_for_file(parsed_file, &repository_keys)
            else {
                continue;
            };
            let crate_keys = repository_keys
                .get(&crate_scope)
                .expect("crate scope should be collected");
            let context = ExtractionContext::file(
                parsed_file,
                module_owner_for_file(parsed_file, &crate_keys.module_owners),
            );
            let use_aliases = repository_aliases
                .get(&crate_scope)
                .cloned()
                .unwrap_or_default();
            let symbols = extract_symbols_with_resolution(
                parsed_file,
                &context,
                &crate_keys.type_owners,
                &use_aliases,
            );
            symbols_by_file.insert(parsed_file.file_id, symbols);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExtractionContext {
    namespace: Vec<String>,
    owner: NestingOwner,
    member_kind: MemberKind,
}

impl ExtractionContext {
    fn file(parsed_file: &ParsedFile, owner: Option<leshy_core::SymbolId>) -> Self {
        Self {
            namespace: file_namespace(parsed_file),
            owner: owner.map_or(NestingOwner::File, NestingOwner::Symbol),
            member_kind: MemberKind::FileLike,
        }
    }

    fn module(&self, owner_id: leshy_core::SymbolId, segment: &str) -> Self {
        Self {
            namespace: extend_namespace(self, segment),
            owner: NestingOwner::Symbol(owner_id),
            member_kind: MemberKind::FileLike,
        }
    }

    fn type_like(&self, owner_id: leshy_core::SymbolId, stable_owner: String) -> Self {
        Self {
            namespace: self.namespace.clone(),
            owner: NestingOwner::Symbol(owner_id),
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
struct RustSourceLayout {
    package_prefix: String,
    namespace: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CrateSymbolOwners {
    type_owners: TypeOwners,
    module_owners: ModuleOwners,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NestingOwner {
    File,
    Symbol(leshy_core::SymbolId),
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
    local_type_keys: &TypeOwners,
    use_aliases: &UseAliases,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        visit_item(
            child,
            parsed_file,
            context,
            symbols,
            local_type_keys,
            use_aliases,
        );
    }
}

fn visit_item(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    symbols: &mut Vec<ExtractedSymbol>,
    local_type_keys: &TypeOwners,
    use_aliases: &UseAliases,
) {
    match node.kind() {
        "mod_item" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let module_path = join_path(namespace(context), &name);
            let stable_key = format!("module:{module_path}");
            let module_id = leshy_core::SymbolId::new(parsed_file.file_id, &stable_key);
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
                    &context.module(module_id, &name),
                    symbols,
                    local_type_keys,
                    use_aliases,
                );
            }
        }
        "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item"
        | "associated_type" => {
            let Some(name) = node_name(node, parsed_file) else {
                return;
            };
            let stable_key = type_stable_key(node, context, &name);
            let symbol_id = leshy_core::SymbolId::new(parsed_file.file_id, &stable_key);
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
                    &context.type_like(symbol_id, stable_owner_name(&stable_key)),
                    symbols,
                    local_type_keys,
                    use_aliases,
                );
            }
        }
        "impl_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                let Some((impl_owner, nesting_owner)) = impl_owner(
                    node,
                    parsed_file,
                    namespace(context),
                    local_type_keys,
                    use_aliases,
                ) else {
                    return;
                };
                visit_item_list(
                    body,
                    parsed_file,
                    &context.impl_like(nesting_owner, impl_owner),
                    symbols,
                    local_type_keys,
                    use_aliases,
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
) -> TypeOwners {
    let mut keys = BTreeMap::new();
    collect_local_type_keys_into(node, parsed_file, context, &mut keys);
    keys
}

fn collect_local_type_keys_into(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    keys: &mut TypeOwners,
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
                        &context.module(
                            leshy_core::SymbolId::new(parsed_file.file_id, &module_key),
                            &name,
                        ),
                        keys,
                    );
                }
            }
            "struct_item" | "enum_item" | "union_item" | "trait_item" | "type_item" => {
                if let Some(name) = node_name(child, parsed_file) {
                    let stable_key = format!("type:{}", join_path(namespace(context), &name));
                    keys.insert(
                        stable_key.clone(),
                        leshy_core::SymbolId::new(parsed_file.file_id, &stable_key),
                    );
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
    local_type_keys: &TypeOwners,
    use_aliases: &UseAliases,
) -> Option<(String, NestingOwner)> {
    let type_node = node.child_by_field_name("type")?;
    let target = canonicalize_type_like_target(
        type_node
            .utf8_text(parsed_file.source_text.as_bytes())
            .ok()?,
        namespace,
        local_type_keys,
        use_aliases,
    );
    let nesting_owner = target
        .local_owner
        .map(NestingOwner::Symbol)
        .unwrap_or(NestingOwner::File);

    if let Some(trait_node) = node.child_by_field_name("trait") {
        let trait_name = canonicalize_type_like_target(
            trait_node
                .utf8_text(parsed_file.source_text.as_bytes())
                .ok()?,
            namespace,
            local_type_keys,
            use_aliases,
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
        NestingOwner::Symbol(symbol_id) => SymbolOwner::Symbol(*symbol_id),
    }
}

type TypeOwners = BTreeMap<String, leshy_core::SymbolId>;
type ModuleOwners = BTreeMap<String, leshy_core::SymbolId>;
type UseAliases = BTreeMap<String, BTreeMap<String, String>>;

fn compact_type_name(raw: &str) -> String {
    raw.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CanonicalTypeTarget {
    stable_target: String,
    local_owner: Option<leshy_core::SymbolId>,
}

fn canonicalize_type_like_target(
    raw: &str,
    namespace: &[String],
    local_type_keys: &TypeOwners,
    use_aliases: &UseAliases,
) -> CanonicalTypeTarget {
    let Some((path_prefix, suffix)) = split_path_prefix_and_suffix(raw) else {
        return CanonicalTypeTarget {
            stable_target: unresolved_type_like_target(raw),
            local_owner: None,
        };
    };

    let resolved_path =
        resolve_imported_path(&path_prefix, namespace, use_aliases, local_type_keys)
            .unwrap_or(path_prefix);
    let resolved = resolve_local_path(&resolved_path, namespace, local_type_keys);
    let stable_prefix = resolved.stable_prefix.unwrap_or(resolved_path);

    CanonicalTypeTarget {
        stable_target: format!("{stable_prefix}{suffix}"),
        local_owner: resolved.local_owner,
    }
}

fn unresolved_type_like_target(raw: &str) -> String {
    let trimmed = raw.trim();
    if starts_with_dyn_keyword(trimmed) {
        trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        compact_type_name(trimmed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedLocalPath {
    stable_prefix: Option<String>,
    local_owner: Option<leshy_core::SymbolId>,
}

fn resolve_local_path(
    path_prefix: &str,
    namespace: &[String],
    local_type_keys: &TypeOwners,
) -> ResolvedLocalPath {
    let segments: Vec<&str> = path_prefix.split("::").collect();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return ResolvedLocalPath {
            stable_prefix: None,
            local_owner: None,
        };
    }

    if let Some(explicit) = canonicalize_explicit_prefix(&segments, namespace) {
        let path = explicit.join("::");
        return ResolvedLocalPath {
            stable_prefix: Some(path.clone()),
            local_owner: local_type_keys.get(&format!("type:{path}")).copied(),
        };
    }

    let relative = join_candidate(namespace, &segments);
    let rooted = segments.join("::");

    if let Some(owner) = local_type_keys.get(&format!("type:{relative}")) {
        return ResolvedLocalPath {
            stable_prefix: Some(relative.clone()),
            local_owner: Some(*owner),
        };
    }

    if let Some(owner) = local_type_keys.get(&format!("type:{rooted}")) {
        return ResolvedLocalPath {
            stable_prefix: Some(rooted.clone()),
            local_owner: Some(*owner),
        };
    }

    ResolvedLocalPath {
        stable_prefix: None,
        local_owner: None,
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

fn split_path_prefix_and_suffix(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    let compact = compact_type_name(trimmed);

    if compact.is_empty()
        || trimmed.starts_with('&')
        || trimmed.starts_with('*')
        || trimmed.starts_with('(')
        || trimmed.starts_with('[')
        || compact.starts_with("fn(")
        || compact.starts_with("extern\"")
        || compact.starts_with("unsafefn(")
        || trimmed.starts_with('<')
        || starts_with_dyn_keyword(trimmed)
    {
        return None;
    }

    let mut angle_depth = 0usize;
    for (index, ch) in compact.char_indices() {
        match ch {
            '<' if angle_depth == 0 => {
                return Some((compact[..index].to_string(), compact[index..].to_string()));
            }
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ => {}
        }
    }

    Some((compact, String::new()))
}

include!("layout.rs");

fn starts_with_dyn_keyword(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("dyn") else {
        return false;
    };

    rest.chars().next().is_some_and(char::is_whitespace)
}

include!("use_aliases.rs");

#[cfg(test)]
mod tests;
