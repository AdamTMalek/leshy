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
        resolve_imported_path(&path_prefix, namespace, use_aliases).unwrap_or(path_prefix);
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

type RepositorySymbolOwners = BTreeMap<String, CrateSymbolOwners>;
type RepositoryUseAliases = BTreeMap<String, UseAliases>;

fn collect_repository_symbol_owners(parsed_files: &[&ParsedFile]) -> RepositorySymbolOwners {
    let mut keys: RepositorySymbolOwners = BTreeMap::new();
    let mut pending_files = Vec::new();

    for parsed_file in parsed_files {
        let Some(crate_scope) = direct_crate_scope(parsed_file) else {
            pending_files.push(*parsed_file);
            continue;
        };
        let context = ExtractionContext::file(parsed_file, None);
        let crate_keys = keys.entry(crate_scope).or_default();
        collect_local_type_keys_into(
            parsed_file.tree.root_node(),
            parsed_file,
            &context,
            &mut crate_keys.type_owners,
        );
        collect_module_owners_into(
            parsed_file.tree.root_node(),
            parsed_file,
            &context,
            &mut crate_keys.module_owners,
        );
    }

    while !pending_files.is_empty() {
        let mut remaining = Vec::new();
        let mut resolved_in_pass = false;

        for parsed_file in pending_files {
            let Some(crate_scope) = resolved_crate_scope_for_file(parsed_file, &keys) else {
                remaining.push(parsed_file);
                continue;
            };
            let crate_keys = keys
                .get_mut(&crate_scope)
                .expect("resolved crate scope should exist");
            let context = ExtractionContext::file(
                parsed_file,
                module_owner_for_file(parsed_file, &crate_keys.module_owners),
            );
            collect_local_type_keys_into(
                parsed_file.tree.root_node(),
                parsed_file,
                &context,
                &mut crate_keys.type_owners,
            );
            collect_module_owners_into(
                parsed_file.tree.root_node(),
                parsed_file,
                &context,
                &mut crate_keys.module_owners,
            );
            resolved_in_pass = true;
        }

        if !resolved_in_pass {
            break;
        }

        pending_files = remaining;
    }

    keys
}

fn collect_repository_use_aliases(
    parsed_files: &[&ParsedFile],
    repository_keys: &RepositorySymbolOwners,
) -> RepositoryUseAliases {
    let mut aliases_by_scope = BTreeMap::new();

    for parsed_file in parsed_files {
        let Some(crate_scope) = resolved_crate_scope_for_file(parsed_file, repository_keys) else {
            continue;
        };
        let crate_keys = repository_keys
            .get(&crate_scope)
            .expect("resolved crate scope should exist");
        let context = ExtractionContext::file(
            parsed_file,
            module_owner_for_file(parsed_file, &crate_keys.module_owners),
        );
        merge_use_aliases(
            aliases_by_scope.entry(crate_scope).or_default(),
            collect_use_aliases(parsed_file, &context),
        );
    }

    aliases_by_scope
}

fn merge_use_aliases(target: &mut UseAliases, source: UseAliases) {
    for (scope, aliases) in source {
        target.entry(scope).or_default().extend(aliases);
    }
}

fn resolved_crate_scope_for_file(
    parsed_file: &ParsedFile,
    repository_keys: &RepositorySymbolOwners,
) -> Option<String> {
    if let Some(scope) = direct_crate_scope(parsed_file) {
        return Some(scope);
    }

    if let Some(scope) = path_anchored_crate_scope(parsed_file)
        && repository_keys.contains_key(&scope)
    {
        return Some(scope);
    }

    let package_prefix = package_prefix(parsed_file);
    let namespace = file_namespace(parsed_file);
    let module_key = (!namespace.is_empty()).then(|| format!("module:{}", namespace.join("::")));

    let mut candidates: Vec<String> = repository_keys
        .iter()
        .filter_map(|(scope, crate_keys)| {
            if !scope_matches_package(scope, &package_prefix) {
                return None;
            }

            if let Some(module_key) = &module_key
                && crate_keys.module_owners.contains_key(module_key)
            {
                return Some(scope.clone());
            }

            None
        })
        .collect();

    if candidates.len() == 1 {
        return candidates.pop();
    }

    None
}

fn path_anchored_crate_scope(parsed_file: &ParsedFile) -> Option<String> {
    let path = parsed_file.relative_path.as_str();
    let path_segments: Vec<&str> = path.split('/').collect();
    let src_index = path_segments
        .iter()
        .position(|segment| *segment == "src")
        .unwrap_or(0);
    let package_prefix = if path_segments.get(src_index) == Some(&"src") {
        join_layout_segments(&path_segments[..src_index])
    } else {
        String::new()
    };
    let crate_relative_segments = if path_segments.get(src_index) == Some(&"src") {
        &path_segments[src_index + 1..]
    } else {
        &path_segments[..]
    };

    match crate_relative_segments {
        ["bin", binary_name, rest @ ..] if !rest.is_empty() => Some(crate_scope_key(
            &package_prefix,
            &format!("bin/{binary_name}"),
        )),
        _ => None,
    }
}

fn collect_module_owners_into(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    keys: &mut ModuleOwners,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        if child.kind() != "mod_item" {
            continue;
        }

        let Some(name) = node_name(child, parsed_file) else {
            continue;
        };

        let module_path = join_path(namespace(context), &name);
        let stable_key = format!("module:{module_path}");
        keys.insert(
            stable_key.clone(),
            leshy_core::SymbolId::new(parsed_file.file_id, &stable_key),
        );

        if let Some(body) = child.child_by_field_name("body") {
            collect_module_owners_into(
                body,
                parsed_file,
                &context.module(
                    leshy_core::SymbolId::new(parsed_file.file_id, &stable_key),
                    &name,
                ),
                keys,
            );
        }
    }
}

fn module_owner_for_file(
    parsed_file: &ParsedFile,
    module_owners: &ModuleOwners,
) -> Option<leshy_core::SymbolId> {
    let namespace = file_namespace(parsed_file);
    if namespace.is_empty() {
        return None;
    }

    module_owners
        .get(&format!("module:{}", namespace.join("::")))
        .copied()
}

fn file_namespace(parsed_file: &ParsedFile) -> Vec<String> {
    rust_source_layout(parsed_file).namespace
}

fn package_prefix(parsed_file: &ParsedFile) -> String {
    rust_source_layout(parsed_file).package_prefix
}

fn direct_crate_scope(parsed_file: &ParsedFile) -> Option<String> {
    let path = parsed_file.relative_path.as_str();
    let path_segments: Vec<&str> = path.split('/').collect();
    let src_index = path_segments
        .iter()
        .position(|segment| *segment == "src")
        .unwrap_or(0);
    let package_prefix = if path_segments.get(src_index) == Some(&"src") {
        join_layout_segments(&path_segments[..src_index])
    } else {
        String::new()
    };
    let crate_relative_segments = if path_segments.get(src_index) == Some(&"src") {
        &path_segments[src_index + 1..]
    } else {
        &path_segments[..]
    };

    match crate_relative_segments {
        ["lib.rs"] => Some(crate_scope_key(&package_prefix, "lib")),
        ["main.rs"] => Some(crate_scope_key(&package_prefix, "main")),
        ["bin", file_name] if file_name.ends_with(".rs") => {
            let binary_name = file_name.strip_suffix(".rs")?;
            Some(crate_scope_key(
                &package_prefix,
                &format!("bin/{binary_name}"),
            ))
        }
        ["bin", binary_name, "main.rs"] => Some(crate_scope_key(
            &package_prefix,
            &format!("bin/{binary_name}"),
        )),
        _ => None,
    }
}

fn rust_source_layout(parsed_file: &ParsedFile) -> RustSourceLayout {
    let path = parsed_file.relative_path.as_str();
    let path_segments: Vec<&str> = path.split('/').collect();
    let src_index = path_segments
        .iter()
        .position(|segment| *segment == "src")
        .unwrap_or(0);
    let crate_prefix = if path_segments.get(src_index) == Some(&"src") {
        &path_segments[..src_index]
    } else {
        &[][..]
    };
    let crate_relative_segments = if path_segments.get(src_index) == Some(&"src") {
        &path_segments[src_index + 1..]
    } else {
        &path_segments[..]
    };

    if let Some(binary_layout) = binary_source_layout(crate_prefix, crate_relative_segments) {
        return binary_layout;
    }

    RustSourceLayout {
        package_prefix: join_layout_segments(crate_prefix),
        namespace: module_namespace_from_segments(crate_relative_segments),
    }
}

fn binary_source_layout(
    crate_prefix: &[&str],
    crate_relative_segments: &[&str],
) -> Option<RustSourceLayout> {
    if crate_relative_segments.first().copied() != Some("bin") {
        return None;
    }

    match crate_relative_segments {
        ["bin", file_name] if file_name.ends_with(".rs") => Some(RustSourceLayout {
            package_prefix: join_layout_segments(crate_prefix),
            namespace: Vec::new(),
        }),
        ["bin", _binary_name, rest @ ..] => Some(RustSourceLayout {
            package_prefix: join_layout_segments(crate_prefix),
            namespace: module_namespace_from_segments(rest),
        }),
        _ => None,
    }
}

fn module_namespace_from_segments(segments: &[&str]) -> Vec<String> {
    let mut namespace: Vec<String> = segments
        .iter()
        .map(|segment| (*segment).to_string())
        .collect();
    let Some(last) = namespace.pop() else {
        return Vec::new();
    };

    match last.as_str() {
        "lib.rs" | "main.rs" => namespace,
        "mod.rs" => namespace,
        _ => {
            if let Some(stem) = last.strip_suffix(".rs") {
                namespace.push(stem.to_string());
            }
            namespace
        }
    }
}

fn join_layout_segments(segments: &[&str]) -> String {
    segments.join("/")
}

fn crate_scope_key(package_prefix: &str, target: &str) -> String {
    if package_prefix.is_empty() {
        format!("#{target}")
    } else {
        format!("{package_prefix}#{target}")
    }
}

fn scope_matches_package(scope: &str, package_prefix: &str) -> bool {
    scope
        .split_once('#')
        .map(|(prefix, _)| prefix)
        .unwrap_or("")
        == package_prefix
}

fn starts_with_dyn_keyword(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("dyn") else {
        return false;
    };

    rest.chars().next().is_some_and(char::is_whitespace)
}

fn collect_use_aliases(parsed_file: &ParsedFile, context: &ExtractionContext) -> UseAliases {
    let mut aliases = BTreeMap::new();
    collect_use_aliases_into(
        parsed_file.tree.root_node(),
        parsed_file,
        context,
        &mut aliases,
    );
    aliases
}

fn collect_use_aliases_into(
    node: Node<'_>,
    parsed_file: &ParsedFile,
    context: &ExtractionContext,
    aliases: &mut UseAliases,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                let Ok(text) = child.utf8_text(parsed_file.source_text.as_bytes()) else {
                    continue;
                };
                for (alias, target) in parse_use_declaration(text, namespace(context)) {
                    aliases
                        .entry(scope_key(namespace(context)))
                        .or_default()
                        .insert(alias, target);
                }
            }
            "mod_item" => {
                if let Some(name) = node_name(child, parsed_file)
                    && let Some(body) = child.child_by_field_name("body")
                {
                    let module_context = ExtractionContext {
                        namespace: extend_namespace(context, &name),
                        owner: context.owner.clone(),
                        member_kind: MemberKind::FileLike,
                    };
                    collect_use_aliases_into(body, parsed_file, &module_context, aliases);
                }
            }
            _ => {}
        }
    }
}

fn scope_key(namespace: &[String]) -> String {
    namespace.join("::")
}

fn resolve_imported_path(
    path_prefix: &str,
    namespace: &[String],
    use_aliases: &UseAliases,
) -> Option<String> {
    let (alias_scope, mut resolved) = alias_scope_and_path(path_prefix, namespace)?;
    let scope = use_aliases.get(&scope_key(&alias_scope))?;
    let original = resolved.clone();
    let mut seen = std::collections::BTreeSet::new();

    loop {
        if !seen.insert(resolved.clone()) {
            return Some(resolved);
        }

        let (first, remainder) = resolved
            .split_once("::")
            .map_or((resolved.as_str(), None), |(head, tail)| (head, Some(tail)));
        let Some(target) = scope.get(first) else {
            return if resolved == original {
                None
            } else {
                Some(resolved)
            };
        };

        resolved = match remainder {
            Some(rest) => format!("{target}::{rest}"),
            None => target.clone(),
        };
    }
}

fn alias_scope_and_path(path_prefix: &str, namespace: &[String]) -> Option<(Vec<String>, String)> {
    let mut scope = namespace.to_vec();
    let mut remaining = path_prefix.trim();

    loop {
        if let Some(rest) = remaining.strip_prefix("self::") {
            remaining = rest;
            continue;
        }
        if let Some(rest) = remaining.strip_prefix("super::") {
            scope.pop()?;
            remaining = rest;
            continue;
        }
        if let Some(rest) = remaining.strip_prefix("crate::") {
            scope.clear();
            remaining = rest;
        }
        break;
    }

    (!remaining.is_empty()).then(|| (scope, remaining.to_string()))
}

fn parse_use_declaration(text: &str, namespace: &[String]) -> Vec<(String, String)> {
    let mut declaration = strip_use_visibility(text.trim());
    declaration = declaration
        .strip_prefix("use")
        .unwrap_or(declaration)
        .trim();
    declaration = declaration.strip_suffix(';').unwrap_or(declaration).trim();

    let mut aliases = Vec::new();
    expand_use_tree("", declaration, namespace, &mut aliases);
    aliases
}

fn expand_use_tree(
    prefix: &str,
    tree: &str,
    namespace: &[String],
    aliases: &mut Vec<(String, String)>,
) {
    let tree = tree.trim();
    if tree.is_empty() {
        return;
    }

    if let Some((group_prefix, group_items)) = split_use_group(tree) {
        let next_prefix = join_use_prefix(prefix, group_prefix.trim_end_matches("::"));
        for item in split_top_level(group_items, ',') {
            expand_use_tree(&next_prefix, item, namespace, aliases);
        }
        return;
    }

    let (path, alias_override) = split_use_alias(tree);
    let full_path = if path == "self" {
        prefix.to_string()
    } else {
        join_use_prefix(prefix, path)
    };
    let canonical_target = canonicalize_use_target(&full_path, namespace);

    if canonical_target.is_empty() {
        return;
    }

    let alias = alias_override.unwrap_or_else(|| {
        if path == "self" {
            canonical_target
                .rsplit("::")
                .next()
                .unwrap_or(canonical_target.as_str())
                .to_string()
        } else {
            path.rsplit("::").next().unwrap_or(path).to_string()
        }
    });

    aliases.push((alias, canonical_target));
}

fn strip_use_visibility(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("pub") else {
        return text;
    };
    let rest = rest.trim_start();

    if let Some(remainder) = rest.strip_prefix('(') {
        let mut depth = 1usize;
        for (index, ch) in remainder.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return remainder[index + 1..].trim_start();
                    }
                }
                _ => {}
            }
        }
        text
    } else {
        rest
    }
}

fn split_use_group(tree: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut group_start = None;

    for (index, ch) in tree.char_indices() {
        match ch {
            '{' if depth == 0 => {
                group_start = Some(index);
                depth = 1;
            }
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let start = group_start?;
                    return Some((&tree[..start], &tree[start + 1..index]));
                }
            }
            _ => {}
        }
    }

    None
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    parts.push(text[start..].trim());
    parts
}

fn split_use_alias(tree: &str) -> (&str, Option<String>) {
    let mut depth = 0usize;

    for (index, ch) in tree.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 && tree[index..].starts_with(" as ") => {
                return (
                    tree[..index].trim(),
                    Some(tree[index + 4..].trim().to_string()),
                );
            }
            _ => {}
        }
    }

    (tree.trim(), None)
}

fn join_use_prefix(prefix: &str, suffix: &str) -> String {
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, _) => suffix.to_string(),
        (_, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{suffix}"),
    }
}

fn canonicalize_use_target(path: &str, namespace: &[String]) -> String {
    let compact = compact_type_name(path);
    let Some((path_prefix, _)) = split_path_prefix_and_suffix(path) else {
        return compact;
    };

    let segments: Vec<&str> = path_prefix.split("::").collect();
    if let Some(explicit) = canonicalize_explicit_prefix(&segments, namespace) {
        explicit.join("::")
    } else if compact == "self" {
        namespace.join("::")
    } else {
        path_prefix
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use leshy_core::{FileId, RelativePath, RepositoryId, SymbolId, SymbolKind};
    use leshy_parser::{LanguageId, ParsedFile};

    use super::{extract_symbols, parse_source, parse_use_declaration, supports_path};

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
    fn strips_visibility_from_use_declarations() {
        assert_eq!(
            parse_use_declaration("pub use crate::model::Record;", &[]),
            vec![("Record".to_string(), "model::Record".to_string())]
        );
        assert_eq!(
            parse_use_declaration("pub(crate) use crate::model::Record;", &[]),
            vec![("Record".to_string(), "model::Record".to_string())]
        );
    }

    #[test]
    fn canonicalizes_grouped_self_import_aliases() {
        assert_eq!(
            parse_use_declaration("use crate::outer::{self as outer_mod, Widget};", &[]),
            vec![
                ("outer_mod".to_string(), "outer".to_string()),
                ("Widget".to_string(), "outer::Widget".to_string()),
            ]
        );
    }

    #[test]
    fn derives_crate_local_namespaces_from_workspace_relative_paths() {
        let source = r#"
pub struct Record;

impl crate::Record {
    fn from_crate() -> Self {
        Self
    }
}
"#;
        let tree = parse_source(source).expect("parse should succeed");
        let relative_path =
            RelativePath::new("crates/example/src/lib.rs").expect("relative path should build");
        let parsed_file = ParsedFile {
            file_id: FileId::new(RepositoryId::new("repository"), &relative_path),
            relative_path,
            language: LanguageId::new("rust"),
            source_text: source.to_string(),
            tree,
        };

        let symbols = extract_symbols(&parsed_file);
        let record = symbols
            .iter()
            .find(|symbol| symbol.display_name == "Record")
            .expect("type should exist");
        let from_crate = symbols
            .iter()
            .find(|symbol| symbol.display_name == "from_crate")
            .expect("method should exist");

        assert_eq!(record.stable_key, "type:Record");
        assert_eq!(from_crate.stable_key, "method:Record::from_crate");
        assert_eq!(
            from_crate.owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Record"))
        );
    }

    #[test]
    fn treats_src_bin_files_as_crate_roots() {
        let source = r#"
pub struct Tool;

impl crate::Tool {
    fn run() -> Self {
        Self
    }
}
"#;

        for path in [
            "crates/example/src/bin/tool.rs",
            "crates/example/src/bin/tool/main.rs",
        ] {
            let tree = parse_source(source).expect("parse should succeed");
            let relative_path = RelativePath::new(path).expect("relative path should build");
            let parsed_file = ParsedFile {
                file_id: FileId::new(RepositoryId::new("repository"), &relative_path),
                relative_path,
                language: LanguageId::new("rust"),
                source_text: source.to_string(),
                tree,
            };

            let symbols = extract_symbols(&parsed_file);
            let tool = symbols
                .iter()
                .find(|symbol| symbol.display_name == "Tool")
                .expect("type should exist");
            let run = symbols
                .iter()
                .find(|symbol| symbol.display_name == "run")
                .expect("method should exist");

            assert_eq!(tool.stable_key, "type:Tool");
            assert_eq!(run.stable_key, "method:Tool::run");
            assert_eq!(
                run.owner,
                leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Tool"))
            );
        }
    }

    #[test]
    fn keeps_dyn_prefixed_identifiers_path_like_for_owner_resolution() {
        let source = r#"
mod dynastore {
    pub struct Widget;
}

impl dynastore::Widget {
    fn build() -> Self {
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
        let build = symbols
            .iter()
            .find(|symbol| symbol.display_name == "build")
            .expect("method should exist");

        assert_eq!(build.stable_key, "method:dynastore::Widget::build");
        assert_eq!(
            build.owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(
                parsed_file.file_id,
                "type:dynastore::Widget",
            ))
        );
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
    fn resolves_parent_scope_use_aliases_for_impl_targets() {
        let source = r#"
mod outer {
    use crate::model::Record as Alias;

    mod inner {
        impl super::Alias {
            fn from_parent_alias() -> Self {
                Self
            }
        }
    }
}

mod model {
    pub struct Record;
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
            .find(|symbol| symbol.stable_key == "method:model::Record::from_parent_alias")
            .expect("parent-scope alias method should exist");

        assert_eq!(
            method.owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(
                parsed_file.file_id,
                "type:model::Record",
            ))
        );
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

    #[test]
    fn keeps_dyn_trait_fallback_keys_distinct_from_nominal_types() {
        let source = r#"
trait Trait {}

struct dynTrait;

impl dyn Trait {
    fn collide(&self) {}
}

impl dynTrait {
    fn collide(&self) {}
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
        let dyn_trait_method = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "method:dyn Trait::collide")
            .expect("dyn trait method should exist");
        let nominal_method = symbols
            .iter()
            .find(|symbol| symbol.stable_key == "method:dynTrait::collide")
            .expect("nominal dynTrait method should exist");

        assert_ne!(dyn_trait_method.id, nominal_method.id);
        assert_ne!(dyn_trait_method.stable_key, nominal_method.stable_key);
    }
}
