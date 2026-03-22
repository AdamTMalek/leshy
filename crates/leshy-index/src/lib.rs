use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use leshy_core::{
    DirectoryId, ExtractedSymbol, FileId, GraphError, RepositoryGraph, RepositoryScan, ScanError,
    Symbol, SymbolId, scan_repository,
};
use leshy_parser::{
    LanguageRegistry, ParseError, ParsedFile, extract_symbols, parse_repository_scan,
};

/// The end-to-end indexing result for a repository root.
#[derive(Debug)]
pub struct RepositoryIndex {
    pub scan: RepositoryScan,
    pub parsed_files: Vec<ParsedFile>,
    pub symbols: Vec<ExtractedSymbol>,
    pub graph: RepositoryGraph,
}

/// Errors returned by the indexing orchestration pipeline.
#[derive(Debug)]
pub enum IndexError {
    Scan {
        source: ScanError,
    },
    Parse {
        source: ParseError,
    },
    InsertDirectory {
        directory_id: DirectoryId,
        source: GraphError,
    },
    InsertFile {
        file_id: FileId,
        source: GraphError,
    },
    InsertSymbol {
        symbol_id: SymbolId,
        source: GraphError,
    },
}

impl Display for IndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scan { source } => write!(f, "failed to scan repository: {source}"),
            Self::Parse { source } => write!(f, "failed to parse repository: {source}"),
            Self::InsertDirectory {
                directory_id,
                source,
            } => {
                write!(f, "failed to populate directory `{directory_id}`: {source}")
            }
            Self::InsertFile { file_id, source } => {
                write!(f, "failed to populate file `{file_id}`: {source}")
            }
            Self::InsertSymbol { symbol_id, source } => {
                write!(f, "failed to populate symbol `{symbol_id}`: {source}")
            }
        }
    }
}

impl Error for IndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Scan { source } => Some(source),
            Self::Parse { source } => Some(source),
            Self::InsertDirectory { source, .. } => Some(source),
            Self::InsertFile { source, .. } => Some(source),
            Self::InsertSymbol { source, .. } => Some(source),
        }
    }
}

/// Scans a repository root, parses supported source files, and populates a repository graph.
pub fn index_repository(
    root: &Path,
    registry: &LanguageRegistry,
) -> Result<RepositoryIndex, IndexError> {
    let scan = scan_repository(root).map_err(|source| IndexError::Scan { source })?;
    let parsed_files = parse_repository_scan(root, &scan, registry)
        .map_err(|source| IndexError::Parse { source })?;
    let symbols = extract_symbols(&parsed_files, registry);
    let graph = build_graph_from_scan(&scan, &symbols)?;

    Ok(RepositoryIndex {
        scan,
        parsed_files,
        symbols,
        graph,
    })
}

fn build_graph_from_scan(
    scan: &RepositoryScan,
    symbols: &[ExtractedSymbol],
) -> Result<RepositoryGraph, IndexError> {
    let mut graph = RepositoryGraph::new(scan.repository.clone());

    for directory in &scan.directories {
        graph
            .insert_directory(directory.clone())
            .map_err(|source| IndexError::InsertDirectory {
                directory_id: directory.id,
                source,
            })?;
    }

    for file in &scan.files {
        graph
            .insert_file(file.clone())
            .map_err(|source| IndexError::InsertFile {
                file_id: file.id,
                source,
            })?;
    }

    let mut pending = Vec::with_capacity(symbols.len());
    for extracted in symbols {
        pending.push(
            Symbol::try_from(extracted).map_err(|source| IndexError::InsertSymbol {
                symbol_id: extracted.id,
                source,
            })?,
        );
    }

    while !pending.is_empty() {
        let mut deferred = Vec::new();
        let mut inserted_any = false;

        for symbol in pending {
            match graph.insert_symbol(symbol.clone()) {
                Ok(()) => inserted_any = true,
                Err(GraphError::MissingEntity {
                    entity: "symbol", ..
                }) => deferred.push(symbol),
                Err(source) => {
                    return Err(IndexError::InsertSymbol {
                        symbol_id: symbol.id,
                        source,
                    });
                }
            }
        }

        if !inserted_any {
            let symbol = deferred
                .into_iter()
                .next()
                .expect("pending symbols should not be empty");
            return Err(IndexError::InsertSymbol {
                symbol_id: symbol.id,
                source: GraphError::MissingEntity {
                    entity: "symbol",
                    id: symbol.id.to_string(),
                },
            });
        }

        pending = deferred;
    }

    Ok(graph)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use leshy_lang_rust::RUST_LANGUAGE_PLUGIN;
    use leshy_parser::{LanguageId, LanguageRegistry, ParseError};

    use super::{IndexError, build_graph_from_scan, index_repository};
    use leshy_core::{DirectoryId, RelativePath, ScanError, SourcePosition, SourceSpan};

    #[test]
    fn indexes_repository_end_to_end() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn library() {}\n");
        tempdir.write_file("src/bin/app.rs", "");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");

        assert_eq!(index.scan.directories.len(), 3);
        assert_eq!(index.scan.files.len(), 2);
        assert_eq!(index.parsed_files.len(), 2);
        assert_eq!(index.symbols.len(), 1);
        assert_eq!(index.parsed_files[0].language, LanguageId::new("rust"));
        assert_eq!(index.symbols[0].display_name, "library");
        assert_eq!(index.symbols[0].stable_key, "fn:library");
        assert_eq!(index.graph.directories().count(), 3);
        assert_eq!(index.graph.files().count(), 2);
        assert_eq!(index.graph.symbols().count(), 1);
        assert_eq!(
            index
                .graph
                .symbols()
                .next()
                .expect("graph symbol")
                .stable_key,
            "fn:library"
        );
        assert_eq!(index.graph.relationships().count(), 6);
        assert_eq!(index.graph.repository().id, index.scan.repository.id);
    }

    #[test]
    fn wraps_scan_failures() {
        let missing_path = unique_temp_path("missing");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);
        let error = index_repository(&missing_path, &registry).expect_err("indexing should fail");

        assert!(matches!(
            error,
            IndexError::Scan {
                source: ScanError::ReadPath { .. }
            }
        ));
    }

    #[test]
    fn wraps_parse_failures() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "fn broken( {\n");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let error = index_repository(tempdir.path(), &registry).expect_err("indexing should fail");

        assert!(matches!(
            error,
            IndexError::Parse {
                source: ParseError::SyntaxErrors { .. }
            }
        ));
        assert!(error.to_string().contains("failed to parse repository"));
        assert!(error.to_string().contains("src/lib.rs"));
    }

    #[test]
    fn reports_directory_population_stage_failures() {
        let tempdir = TestDir::new();
        fs::create_dir_all(tempdir.path().join("src/nested")).expect("nested directories");

        let mut scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        scan.directories[1].parent_id = None;
        let failing_directory_id = scan.directories[1].id;

        let error = build_graph_from_scan(&scan, &[]).expect_err("graph population should fail");

        assert!(matches!(
            error,
            IndexError::InsertDirectory { directory_id, .. } if directory_id == failing_directory_id
        ));
    }

    #[test]
    fn reports_file_population_stage_failures() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "");

        let mut scan = leshy_core::scan_repository(tempdir.path()).expect("scan should succeed");
        scan.files[0].parent_id = DirectoryId::new(
            scan.repository.id,
            &RelativePath::new("missing").expect("relative path should build"),
        );
        let failing_file_id = scan.files[0].id;

        let error = build_graph_from_scan(&scan, &[]).expect_err("graph population should fail");

        assert!(matches!(
            error,
            IndexError::InsertFile { file_id, .. } if file_id == failing_file_id
        ));
    }

    #[test]
    fn indexes_without_parsed_files_when_no_plugins_are_registered() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn library() {}\n");

        let index = index_repository(tempdir.path(), &LanguageRegistry::new())
            .expect("indexing should succeed");

        assert!(index.parsed_files.is_empty());
        assert!(index.symbols.is_empty());
        assert_eq!(index.scan.files.len(), 1);
        assert_eq!(index.graph.files().count(), 1);
    }

    #[test]
    fn populates_graph_symbols_with_definition_spans() {
        let tempdir = TestDir::new();
        tempdir.write_file("src/lib.rs", "pub fn library() {}\n");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let symbol = index.graph.symbols().next().expect("symbol should exist");

        assert_eq!(symbol.id, index.symbols[0].id);
        assert_eq!(symbol.display_name, "library");
        assert_eq!(
            symbol.span,
            SourceSpan::new(0, 19, SourcePosition::new(0, 0), SourcePosition::new(0, 19))
        );
    }

    #[test]
    fn preserves_nested_symbol_ownership_in_the_graph() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "mod nested { struct Widget; impl Widget { fn new() -> Self { Self } } }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file_id = index.parsed_files[0].file_id;
        let widget_id = leshy_core::SymbolId::new(file_id, "type:nested::Widget");
        let new_symbol = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:nested::Widget::new",
            ))
            .expect("method symbol should exist");

        assert_eq!(new_symbol.owner, leshy_core::SymbolOwner::Symbol(widget_id));
    }

    #[test]
    fn preserves_type_ownership_when_impl_appears_before_type_definition() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "impl Widget { fn new() -> Self { Self } }\nstruct Widget;\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file_id = index.parsed_files[0].file_id;
        let widget_id = leshy_core::SymbolId::new(file_id, "type:Widget");
        let new_symbol = index
            .graph
            .symbol(leshy_core::SymbolId::new(file_id, "method:Widget::new"))
            .expect("method symbol should exist");

        assert_eq!(new_symbol.owner, leshy_core::SymbolOwner::Symbol(widget_id));
    }

    #[test]
    fn indexes_multiple_trait_impl_associated_types_without_symbol_collisions() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "trait A { type Item; }\ntrait B { type Item; }\nstruct Stream;\nimpl A for Stream { type Item = u8; }\nimpl B for Stream { type Item = u16; }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file_id = index.parsed_files[0].file_id;

        assert!(
            index
                .graph
                .symbol(leshy_core::SymbolId::new(
                    file_id,
                    "type:A for Stream::Item"
                ))
                .is_some()
        );
        assert!(
            index
                .graph
                .symbol(leshy_core::SymbolId::new(
                    file_id,
                    "type:B for Stream::Item"
                ))
                .is_some()
        );
    }

    #[test]
    fn indexes_specialized_inherent_impl_members_without_symbol_collisions() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "struct Wrapper<T>(T);\nimpl Wrapper<u8> { const KIND: u8 = 1; fn new() -> Self { Self(0) } }\nimpl Wrapper<String> { const KIND: u8 = 2; fn new() -> Self { Self(String::new()) } }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file_id = index.parsed_files[0].file_id;
        let wrapper_id = leshy_core::SymbolId::new(file_id, "type:Wrapper");

        let new_u8 = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:Wrapper<u8>::new",
            ))
            .expect("u8 constructor should exist");
        let new_string = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:Wrapper<String>::new",
            ))
            .expect("string constructor should exist");
        let kind_u8 = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "const:Wrapper<u8>::KIND",
            ))
            .expect("u8 constant should exist");
        let kind_string = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "const:Wrapper<String>::KIND",
            ))
            .expect("string constant should exist");

        assert_eq!(new_u8.owner, leshy_core::SymbolOwner::Symbol(wrapper_id));
        assert_eq!(
            new_string.owner,
            leshy_core::SymbolOwner::Symbol(wrapper_id)
        );
        assert_eq!(kind_u8.owner, leshy_core::SymbolOwner::Symbol(wrapper_id));
        assert_eq!(
            kind_string.owner,
            leshy_core::SymbolOwner::Symbol(wrapper_id)
        );
    }

    #[test]
    fn resolves_same_crate_qualified_impl_targets_to_local_type_owners() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "trait Assoc { type Item; }\nmod outer {\n    pub struct Widget;\n    impl self::Widget { fn from_self() -> Self { Self } }\n    pub struct Wrapper<T>(pub T);\n    mod inner { impl super::Widget { const LABEL: &'static str = \"inner\"; } }\n}\nimpl crate::outer::Widget { fn from_crate() -> Self { crate::outer::Widget } }\nimpl Assoc for crate::outer::Widget { type Item = u8; }\nimpl crate::outer::Wrapper<u8> { fn from_u8() -> Self { crate::outer::Wrapper(0) } }\nimpl self::outer::Wrapper<String> { const KIND: &'static str = \"string\"; }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file_id = index.parsed_files[0].file_id;
        let widget_id = leshy_core::SymbolId::new(file_id, "type:outer::Widget");
        let wrapper_id = leshy_core::SymbolId::new(file_id, "type:outer::Wrapper");

        let from_self = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:outer::Widget::from_self",
            ))
            .expect("self-qualified method should exist");
        let from_crate = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:outer::Widget::from_crate",
            ))
            .expect("crate-qualified method should exist");
        let label = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "const:outer::Widget::LABEL",
            ))
            .expect("super-qualified constant should exist");
        let assoc_item = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "type:Assoc for outer::Widget::Item",
            ))
            .expect("qualified associated type should exist");
        let from_u8 = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "method:outer::Wrapper<u8>::from_u8",
            ))
            .expect("specialized qualified method should exist");
        let kind = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file_id,
                "const:outer::Wrapper<String>::KIND",
            ))
            .expect("specialized qualified constant should exist");

        assert_eq!(from_self.owner, leshy_core::SymbolOwner::Symbol(widget_id));
        assert_eq!(from_crate.owner, leshy_core::SymbolOwner::Symbol(widget_id));
        assert_eq!(label.owner, leshy_core::SymbolOwner::Symbol(widget_id));
        assert_eq!(assoc_item.owner, leshy_core::SymbolOwner::Symbol(widget_id));
        assert_eq!(from_u8.owner, leshy_core::SymbolOwner::Symbol(wrapper_id));
        assert_eq!(kind.owner, leshy_core::SymbolOwner::Symbol(wrapper_id));
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

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let lib_file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
            .expect("lib file should be parsed");
        let model_file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "src/model.rs")
            .expect("model file should be parsed");
        let model_type_id = leshy_core::SymbolId::new(model_file.file_id, "type:model::Record");

        let from_module = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:model::Record::from_module",
            ))
            .expect("module-qualified method should exist");
        let from_import = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:model::Record::from_import",
            ))
            .expect("import-qualified method should exist");

        assert_eq!(
            from_module.owner,
            leshy_core::SymbolOwner::Symbol(model_type_id)
        );
        assert_eq!(
            from_import.owner,
            leshy_core::SymbolOwner::Symbol(model_type_id)
        );
    }

    #[test]
    fn resolves_pub_use_reexports_and_grouped_self_aliases() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "src/lib.rs",
            "mod model;\nmod outer;\npub use crate::model::Record;\nuse crate::outer::{self as outer_mod};\nimpl Record { fn from_reexport() -> Self { Self } }\nimpl outer_mod::Widget { fn from_alias() -> Self { Self } }\n",
        );
        tempdir.write_file("src/model.rs", "pub struct Record;\n");
        tempdir.write_file("src/outer.rs", "pub struct Widget;\n");
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let lib_file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
            .expect("lib file should be parsed");
        let model_file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "src/model.rs")
            .expect("model file should be parsed");
        let outer_file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "src/outer.rs")
            .expect("outer file should be parsed");

        let from_reexport = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:model::Record::from_reexport",
            ))
            .expect("pub use method should exist");
        let from_alias = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:outer::Widget::from_alias",
            ))
            .expect("grouped self alias method should exist");

        assert_eq!(
            from_reexport.owner,
            leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
                model_file.file_id,
                "type:model::Record",
            ))
        );
        assert_eq!(
            from_alias.owner,
            leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
                outer_file.file_id,
                "type:outer::Widget",
            ))
        );
    }

    #[test]
    fn treats_workspace_crate_src_as_the_module_root() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "crates/example/src/lib.rs",
            "pub struct Record;\nimpl crate::Record { fn from_crate() -> Self { Self } }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "crates/example/src/lib.rs")
            .expect("workspace crate file should be parsed");
        let record_id = leshy_core::SymbolId::new(file.file_id, "type:Record");

        let from_crate = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                file.file_id,
                "method:Record::from_crate",
            ))
            .expect("crate-root method should exist");

        assert_eq!(from_crate.owner, leshy_core::SymbolOwner::Symbol(record_id));
    }

    #[test]
    fn keeps_repository_type_resolution_scoped_to_each_crate() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "crates/a/src/lib.rs",
            "pub struct Record;\nimpl Record { fn new() -> Self { Self } }\n",
        );
        tempdir.write_file(
            "crates/b/src/lib.rs",
            "pub struct Record;\nimpl Record { fn new() -> Self { Self } }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let crate_a = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "crates/a/src/lib.rs")
            .expect("crate a file should be parsed");
        let crate_b = index
            .parsed_files
            .iter()
            .find(|parsed_file| parsed_file.relative_path.as_str() == "crates/b/src/lib.rs")
            .expect("crate b file should be parsed");

        let new_in_a = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                crate_a.file_id,
                "method:Record::new",
            ))
            .expect("crate a method should exist");
        let new_in_b = index
            .graph
            .symbol(leshy_core::SymbolId::new(
                crate_b.file_id,
                "method:Record::new",
            ))
            .expect("crate b method should exist");

        assert_eq!(
            new_in_a.owner,
            leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
                crate_a.file_id,
                "type:Record",
            ))
        );
        assert_eq!(
            new_in_b.owner,
            leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
                crate_b.file_id,
                "type:Record",
            ))
        );
    }

    #[test]
    fn treats_src_bin_files_as_crate_roots() {
        let tempdir = TestDir::new();
        tempdir.write_file(
            "crates/example/src/bin/tool.rs",
            "pub struct Tool;\nimpl crate::Tool { fn run() -> Self { Self } }\n",
        );
        let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

        let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
        let file = index
            .parsed_files
            .iter()
            .find(|parsed_file| {
                parsed_file.relative_path.as_str() == "crates/example/src/bin/tool.rs"
            })
            .expect("binary crate file should be parsed");

        let run = index
            .graph
            .symbol(leshy_core::SymbolId::new(file.file_id, "method:Tool::run"))
            .expect("binary crate method should exist");

        assert_eq!(
            run.owner,
            leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(file.file_id, "type:Tool"))
        );
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

            let unique = format!(
                "leshy-index-test-{}-{}-{}",
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

    fn unique_temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "leshy-index-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos()
        ))
    }
}
