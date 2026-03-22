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
fn treats_build_rs_as_a_crate_root_namespace() {
    let source = "fn main() {}\n";
    let tree = parse_source(source).expect("parse should succeed");
    let relative_path =
        RelativePath::new("crates/example/build.rs").expect("relative path should build");
    let parsed_file = ParsedFile {
        file_id: FileId::new(RepositoryId::new("repository"), &relative_path),
        relative_path,
        language: LanguageId::new("rust"),
        source_text: source.to_string(),
        tree,
    };

    let symbols = extract_symbols(&parsed_file);
    let main = symbols
        .iter()
        .find(|symbol| symbol.display_name == "main")
        .expect("build script main should exist");

    assert_eq!(main.stable_key, "fn:main");
    assert_eq!(
        main.owner,
        leshy_core::SymbolOwner::File(parsed_file.file_id)
    );
}

#[test]
fn treats_tests_examples_and_benches_as_crate_roots() {
    let source = "pub struct Widget;\nimpl Widget { fn build() -> Self { Self } }\n";

    for path in [
        "tests/sample.rs",
        "examples/demo.rs",
        "benches/benchy.rs",
        "tests/sample/main.rs",
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
        let widget = symbols
            .iter()
            .find(|symbol| symbol.display_name == "Widget")
            .expect("type should exist");
        let build = symbols
            .iter()
            .find(|symbol| symbol.display_name == "build")
            .expect("method should exist");

        assert_eq!(widget.stable_key, "type:Widget");
        assert_eq!(build.stable_key, "method:Widget::build");
        assert_eq!(
            build.owner,
            leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:Widget"))
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
        leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:nested::Widget"))
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

    let widget_owner =
        leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:outer::Widget"));

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

    let wrapper_owner =
        leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:outer::Wrapper"));

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
        leshy_core::SymbolOwner::Symbol(SymbolId::new(parsed_file.file_id, "type:model::Record",))
    );
}

#[test]
fn qualifies_relative_use_aliases_to_the_alias_scope() {
    let source = r#"
mod outer {
    mod model {
        pub struct Record;
    }

    use model::Record as Alias;

    mod inner {
        impl super::Alias {
            fn from_relative_alias() -> Self {
                Self
            }
        }
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
        .find(|symbol| symbol.stable_key == "method:outer::model::Record::from_relative_alias")
        .expect("relative parent-scope alias method should exist");

    assert_eq!(
        method.owner,
        leshy_core::SymbolOwner::Symbol(SymbolId::new(
            parsed_file.file_id,
            "type:outer::model::Record",
        ))
    );
}

#[test]
fn keeps_external_use_aliases_unqualified() {
    let source = r#"
use std::fmt::Display;

struct RecordId;

impl Display for RecordId {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Ok(())
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
    let fmt = symbols
        .iter()
        .find(|symbol| symbol.display_name == "fmt")
        .expect("fmt method should exist");

    assert_eq!(fmt.stable_key, "method:std::fmt::Display for RecordId::fmt");
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
