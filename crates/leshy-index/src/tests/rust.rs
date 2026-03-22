use super::*;

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
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(crate_a.file_id, "type:Record",))
    );
    assert_eq!(
        new_in_b.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(crate_b.file_id, "type:Record",))
    );
}

#[test]
fn keeps_lib_and_main_crates_in_one_package_scoped_separately() {
    let tempdir = TestDir::new();
    tempdir.write_file(
        "src/lib.rs",
        "pub struct Widget;\nimpl Widget { fn from_lib() -> Self { Self } }\n",
    );
    tempdir.write_file(
        "src/main.rs",
        "pub struct Widget;\nimpl Widget { fn from_main() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let lib_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");
    let main_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/main.rs")
        .expect("main file should be parsed");

    let from_lib = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            lib_file.file_id,
            "method:Widget::from_lib",
        ))
        .expect("lib method should exist");
    let from_main = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            main_file.file_id,
            "method:Widget::from_main",
        ))
        .expect("main method should exist");

    assert_eq!(
        from_lib.owner,
        leshy_core::SymbolOwner::Symbol(
            leshy_core::SymbolId::new(lib_file.file_id, "type:Widget",)
        )
    );
    assert_eq!(
        from_main.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            main_file.file_id,
            "type:Widget",
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
        .find(|parsed_file| parsed_file.relative_path.as_str() == "crates/example/src/bin/tool.rs")
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

#[test]
fn owns_out_of_line_module_items_by_the_module_symbol() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "mod nested;\n");
    tempdir.write_file("src/nested.rs", "pub fn helper() {}\n");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let lib_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");
    let helper = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            index
                .parsed_files
                .iter()
                .find(|parsed_file| parsed_file.relative_path.as_str() == "src/nested.rs")
                .expect("nested file should be parsed")
                .file_id,
            "fn:nested::helper",
        ))
        .expect("module file function should exist");

    assert_eq!(
        helper.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            lib_file.file_id,
            "module:nested",
        ))
    );
}

#[test]
fn resolves_transitive_import_aliases_for_impl_targets() {
    let tempdir = TestDir::new();
    tempdir.write_file(
            "src/lib.rs",
            "mod outer;\nuse crate::outer as o;\nuse o::Widget as W;\nimpl W { fn from_alias_chain() -> Self { Self } }\n",
        );
    tempdir.write_file("src/outer.rs", "pub struct Widget;\n");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let lib_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");
    let outer_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/outer.rs")
        .expect("outer file should be parsed");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            lib_file.file_id,
            "method:outer::Widget::from_alias_chain",
        ))
        .expect("transitively aliased impl method should exist");

    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            outer_file.file_id,
            "type:outer::Widget",
        ))
    );
}

#[test]
fn resolves_parent_scope_aliases_for_impl_targets() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "mod outer;\nmod model;\n");
    tempdir.write_file(
        "src/outer.rs",
        "use crate::model::Record as Alias;\npub mod inner;\n",
    );
    tempdir.write_file(
        "src/outer/inner.rs",
        "impl super::Alias { fn from_parent_alias() -> Self { Self } }\n",
    );
    tempdir.write_file("src/model.rs", "pub struct Record;\n");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let inner_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/outer/inner.rs")
        .expect("inner file should be parsed");
    let model_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/model.rs")
        .expect("model file should be parsed");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            inner_file.file_id,
            "method:model::Record::from_parent_alias",
        ))
        .expect("parent-scope alias impl method should exist");

    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            model_file.file_id,
            "type:model::Record",
        ))
    );
}

#[test]
fn resolves_relative_parent_scope_aliases_for_impl_targets() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "mod outer;\n");
    tempdir.write_file(
        "src/outer.rs",
        "mod model;\nuse model::Record as Alias;\npub mod inner;\n",
    );
    tempdir.write_file("src/outer/model.rs", "pub struct Record;\n");
    tempdir.write_file(
        "src/outer/inner.rs",
        "impl super::Alias { fn from_relative_alias() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let inner_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/outer/inner.rs")
        .expect("inner file should be parsed");
    let model_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/outer/model.rs")
        .expect("model file should be parsed");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            inner_file.file_id,
            "method:outer::model::Record::from_relative_alias",
        ))
        .expect("relative parent-scope alias impl method should exist");

    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            model_file.file_id,
            "type:outer::model::Record",
        ))
    );
}

#[test]
fn resolves_nested_mod_rs_children_even_when_the_child_file_sorts_first() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub mod foo;\n");
    tempdir.write_file("src/foo/mod.rs", "pub mod bar;\n");
    tempdir.write_file(
        "src/foo/bar.rs",
        "pub struct Widget;\nimpl Widget { fn build() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let foo_mod_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/foo/mod.rs")
        .expect("foo mod file should be parsed");
    let bar_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/foo/bar.rs")
        .expect("bar file should be parsed");
    let widget = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            bar_file.file_id,
            "type:foo::bar::Widget",
        ))
        .expect("nested module type should exist");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            bar_file.file_id,
            "method:foo::bar::Widget::build",
        ))
        .expect("nested module method should exist");

    assert_eq!(
        widget.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            foo_mod_file.file_id,
            "module:foo::bar",
        ))
    );
    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            bar_file.file_id,
            "type:foo::bar::Widget",
        ))
    );
}

#[test]
fn prefers_binary_crate_scope_for_nested_module_files_under_src_bin() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub mod helper;\n");
    tempdir.write_file("src/helper.rs", "pub struct LibHelper;\n");
    tempdir.write_file("src/bin/tool.rs", "pub mod helper;\n");
    tempdir.write_file(
        "src/bin/tool/helper.rs",
        "pub struct Widget;\nimpl Widget { fn build() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let tool_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/bin/tool.rs")
        .expect("binary root file should be parsed");
    let helper_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/bin/tool/helper.rs")
        .expect("binary helper file should be parsed");
    let widget = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            helper_file.file_id,
            "type:helper::Widget",
        ))
        .expect("binary helper type should exist");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            helper_file.file_id,
            "method:helper::Widget::build",
        ))
        .expect("binary helper method should exist");

    assert_eq!(
        widget.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            tool_file.file_id,
            "module:helper",
        ))
    );
    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            helper_file.file_id,
            "type:helper::Widget",
        ))
    );
}

#[test]
fn prefers_lib_scope_for_module_files_shared_by_lib_and_main() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub mod util;\n");
    tempdir.write_file("src/main.rs", "pub mod util;\n");
    tempdir.write_file(
        "src/util.rs",
        "pub struct Widget;\nimpl Widget { fn build() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let lib_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");
    let util_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/util.rs")
        .expect("shared util file should be parsed");
    let widget = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            util_file.file_id,
            "type:util::Widget",
        ))
        .expect("shared util type should exist");
    let method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            util_file.file_id,
            "method:util::Widget::build",
        ))
        .expect("shared util method should exist");

    assert_eq!(
        widget.owner,
        leshy_core::SymbolOwner::Symbol(
            leshy_core::SymbolId::new(lib_file.file_id, "module:util",)
        )
    );
    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            util_file.file_id,
            "type:util::Widget",
        ))
    );
}

#[test]
fn treats_build_rs_as_a_separate_crate_root_from_src_build_modules() {
    let tempdir = TestDir::new();
    tempdir.write_file("build.rs", "fn main() {}\n");
    tempdir.write_file("src/lib.rs", "pub mod build;\n");
    tempdir.write_file("src/build.rs", "pub fn helper() {}\n");
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let build_script = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "build.rs")
        .expect("build script should be parsed");
    let main = index
        .graph
        .symbol(leshy_core::SymbolId::new(build_script.file_id, "fn:main"))
        .expect("build script main should exist");

    assert_eq!(
        main.owner,
        leshy_core::SymbolOwner::File(build_script.file_id)
    );
}

#[test]
fn treats_tests_rs_files_as_crate_roots_not_library_modules() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub mod tests;\n");
    tempdir.write_file("src/tests.rs", "pub struct LibraryHelper;\n");
    tempdir.write_file(
        "tests/sample.rs",
        "pub struct Widget;\nimpl Widget { fn build() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let test_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "tests/sample.rs")
        .expect("integration test file should be parsed");
    let build = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            test_file.file_id,
            "method:Widget::build",
        ))
        .expect("integration test method should exist");

    assert_eq!(
        build.owner,
        leshy_core::SymbolOwner::Symbol(leshy_core::SymbolId::new(
            test_file.file_id,
            "type:Widget",
        ))
    );
}

#[test]
fn leaves_orphan_src_files_out_of_crate_owner_resolution() {
    let tempdir = TestDir::new();
    tempdir.write_file("src/lib.rs", "pub struct Widget;\n");
    tempdir.write_file(
        "src/orphan.rs",
        "impl Widget { fn orphaned() -> Self { Self } }\n",
    );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let orphan_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/orphan.rs")
        .expect("orphan file should be parsed");
    let orphan_method = index
        .graph
        .symbol(leshy_core::SymbolId::new(
            orphan_file.file_id,
            "method:Widget::orphaned",
        ))
        .expect("orphan method should exist");

    assert_eq!(
        orphan_method.owner,
        leshy_core::SymbolOwner::File(orphan_file.file_id)
    );
}

#[test]
fn indexes_dyn_trait_and_nominal_dyn_type_impls_without_key_collisions() {
    let tempdir = TestDir::new();
    tempdir.write_file(
            "src/lib.rs",
            "trait Trait {}\nstruct dynTrait;\nimpl dyn Trait { fn collide(&self) {} }\nimpl dynTrait { fn collide(&self) {} }\n",
        );
    let registry = LanguageRegistry::new().with_plugin(&RUST_LANGUAGE_PLUGIN);

    let index = index_repository(tempdir.path(), &registry).expect("indexing should succeed");
    let lib_file = index
        .parsed_files
        .iter()
        .find(|parsed_file| parsed_file.relative_path.as_str() == "src/lib.rs")
        .expect("lib file should be parsed");

    assert!(
        index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:dyn Trait::collide",
            ))
            .is_some(),
        "dyn trait method should exist"
    );
    assert!(
        index
            .graph
            .symbol(leshy_core::SymbolId::new(
                lib_file.file_id,
                "method:dynTrait::collide",
            ))
            .is_some(),
        "nominal dynTrait method should exist"
    );
}
