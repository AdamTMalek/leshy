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

    preferred_crate_scope(candidates)
}

fn preferred_crate_scope(candidates: Vec<String>) -> Option<String> {
    candidates
        .into_iter()
        .min_by_key(|scope| crate_scope_preference(scope))
}

fn crate_scope_preference(scope: &str) -> (u8, String) {
    let target = scope
        .rsplit_once('#')
        .map(|(_, target)| target)
        .unwrap_or(scope);
    let rank = match target {
        "lib" => 0,
        "main" => 1,
        "build" => 2,
        _ => 3,
    };

    (rank, target.to_string())
}

fn path_anchored_crate_scope(parsed_file: &ParsedFile) -> Option<String> {
    let path = parsed_file.relative_path.as_str();
    let path_segments: Vec<&str> = path.split('/').collect();
    if let Some(scope) = non_src_crate_scope(&path_segments) {
        return Some(scope);
    }
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
    if let Some(build_scope) = build_script_scope(&path_segments) {
        return Some(build_scope);
    }
    if let Some(non_src_scope) = non_src_crate_scope(&path_segments) {
        return Some(non_src_scope);
    }
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
    if let Some(build_layout) = build_script_layout(&path_segments) {
        return build_layout;
    }
    if let Some(non_src_layout) = non_src_crate_layout(&path_segments) {
        return non_src_layout;
    }
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

fn build_script_scope(path_segments: &[&str]) -> Option<String> {
    match path_segments {
        [package_prefix @ .., "build.rs"] => Some(crate_scope_key(
            &join_layout_segments(package_prefix),
            "build",
        )),
        _ => None,
    }
}

fn build_script_layout(path_segments: &[&str]) -> Option<RustSourceLayout> {
    match path_segments {
        [package_prefix @ .., "build.rs"] => Some(RustSourceLayout {
            package_prefix: join_layout_segments(package_prefix),
            namespace: Vec::new(),
        }),
        _ => None,
    }
}

fn non_src_crate_scope(path_segments: &[&str]) -> Option<String> {
    let (package_prefix, target) = non_src_crate_target(path_segments)?;
    Some(crate_scope_key(
        &join_layout_segments(package_prefix),
        &target,
    ))
}

fn non_src_crate_layout(path_segments: &[&str]) -> Option<RustSourceLayout> {
    let (package_prefix, _target) = non_src_crate_target(path_segments)?;
    let crate_relative_segments = &path_segments[package_prefix.len()..];

    match crate_relative_segments {
        [_, file_name] if file_name.ends_with(".rs") => Some(RustSourceLayout {
            package_prefix: join_layout_segments(package_prefix),
            namespace: Vec::new(),
        }),
        [_, _crate_name, rest @ ..] => Some(RustSourceLayout {
            package_prefix: join_layout_segments(package_prefix),
            namespace: module_namespace_from_segments(rest),
        }),
        _ => None,
    }
}

fn non_src_crate_target<'a>(path_segments: &'a [&'a str]) -> Option<(&'a [&'a str], String)> {
    let crate_dir_index = path_segments
        .iter()
        .position(|segment| matches!(*segment, "tests" | "examples" | "benches"))?;
    let package_prefix = &path_segments[..crate_dir_index];
    let crate_relative_segments = &path_segments[crate_dir_index..];

    match crate_relative_segments {
        [kind, file_name] if file_name.ends_with(".rs") => {
            let crate_name = file_name.strip_suffix(".rs")?;
            Some((package_prefix, format!("{kind}/{crate_name}")))
        }
        [kind, crate_name, "main.rs"] => Some((package_prefix, format!("{kind}/{crate_name}"))),
        [kind, crate_name, rest @ ..] if !rest.is_empty() => {
            Some((package_prefix, format!("{kind}/{crate_name}")))
        }
        _ => None,
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

