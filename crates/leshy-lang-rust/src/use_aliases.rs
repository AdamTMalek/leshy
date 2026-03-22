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
    local_type_keys: &TypeOwners,
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
        let qualified_target = qualify_alias_target(target, &alias_scope, local_type_keys);

        resolved = match remainder {
            Some(rest) => format!("{qualified_target}::{rest}"),
            None => qualified_target,
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

fn qualify_alias_target(
    target: &str,
    alias_scope: &[String],
    local_type_keys: &TypeOwners,
) -> String {
    let Some((path_prefix, suffix)) = split_path_prefix_and_suffix(target) else {
        return target.to_string();
    };
    let resolved = resolve_local_path(&path_prefix, alias_scope, local_type_keys);

    match resolved.stable_prefix {
        Some(prefix) => format!("{prefix}{suffix}"),
        None => target.to_string(),
    }
}

