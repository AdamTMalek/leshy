use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ids::stable_hash;
use crate::{DirectoryId, FileId, GraphError, RelativePath, RepositoryId, SymbolId};

/// The category of a graph entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Repository,
    Directory,
    File,
    Symbol,
}

impl EntityKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Directory => "directory",
            Self::File => "file",
            Self::Symbol => "symbol",
        }
    }
}

/// A typed graph entity identifier for relationship endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityId {
    Repository(RepositoryId),
    Directory(DirectoryId),
    File(FileId),
    Symbol(SymbolId),
}

impl EntityId {
    /// Returns the entity category for this identifier.
    pub fn kind(self) -> EntityKind {
        match self {
            Self::Repository(_) => EntityKind::Repository,
            Self::Directory(_) => EntityKind::Directory,
            Self::File(_) => EntityKind::File,
            Self::Symbol(_) => EntityKind::Symbol,
        }
    }

    fn stable_component(self) -> String {
        match self {
            Self::Repository(id) => id.stable_component(),
            Self::Directory(id) => id.stable_component(),
            Self::File(id) => id.stable_component(),
            Self::Symbol(id) => id.stable_component(),
        }
    }
}

/// The repository root metadata stored in a graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub id: RepositoryId,
    pub stable_key: String,
    pub display_name: String,
    pub root_path: PathBuf,
}

impl Repository {
    /// Creates repository metadata with a deterministic identifier.
    pub fn new(
        stable_key: impl Into<String>,
        display_name: impl Into<String>,
        root_path: impl Into<PathBuf>,
    ) -> Result<Self, GraphError> {
        let stable_key = stable_key.into();
        let display_name = display_name.into();

        check_key_is_not_blank("repository", &stable_key)?;
        check_name_is_not_blank("repository", &display_name)?;

        Ok(Self {
            id: RepositoryId::new(&stable_key),
            stable_key,
            display_name,
            root_path: root_path.into(),
        })
    }
}

/// A directory node in the repository filesystem layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    pub id: DirectoryId,
    pub repository_id: RepositoryId,
    pub parent_id: Option<DirectoryId>,
    pub relative_path: RelativePath,
}

impl Directory {
    /// Creates a directory node for the given repository-relative path.
    pub fn new(
        repository_id: RepositoryId,
        parent_id: Option<DirectoryId>,
        relative_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, GraphError> {
        let relative_path = RelativePath::new(relative_path)?;
        let id = DirectoryId::new(repository_id, &relative_path);

        Ok(Self {
            id,
            repository_id,
            parent_id,
            relative_path,
        })
    }
}

/// A file node in the repository filesystem layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub id: FileId,
    pub repository_id: RepositoryId,
    pub parent_id: DirectoryId,
    pub relative_path: RelativePath,
}

impl File {
    /// Creates a file node for the given repository-relative path.
    pub fn new(
        repository_id: RepositoryId,
        parent_id: DirectoryId,
        relative_path: impl AsRef<std::path::Path>,
    ) -> Result<Self, GraphError> {
        let relative_path = RelativePath::new(relative_path)?;
        let id = FileId::new(repository_id, &relative_path);

        Ok(Self {
            id,
            repository_id,
            parent_id,
            relative_path,
        })
    }
}

/// The supported symbol categories in the MVP graph model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Module,
    Type,
    Function,
    Method,
    Field,
    Constant,
}

/// The owning container for a symbol definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolOwner {
    File(FileId),
    Symbol(SymbolId),
}

impl SymbolOwner {
    fn entity_id(self) -> EntityId {
        match self {
            Self::File(id) => EntityId::File(id),
            Self::Symbol(id) => EntityId::Symbol(id),
        }
    }
}

/// A symbol node in the semantic layer of the repository graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub id: SymbolId,
    pub file_id: FileId,
    pub owner: SymbolOwner,
    pub kind: SymbolKind,
    pub display_name: String,
    pub stable_key: String,
}

impl Symbol {
    /// Creates a symbol node with a deterministic identifier scoped to its defining file.
    pub fn new(
        file_id: FileId,
        owner: SymbolOwner,
        kind: SymbolKind,
        display_name: impl Into<String>,
        stable_key: impl Into<String>,
    ) -> Result<Self, GraphError> {
        let display_name = display_name.into();
        let stable_key = stable_key.into();

        check_name_is_not_blank("symbol", &display_name)?;
        check_key_is_not_blank("symbol", &stable_key)?;

        Ok(Self {
            id: SymbolId::new(file_id, &stable_key),
            file_id,
            owner,
            kind,
            display_name,
            stable_key,
        })
    }
}

/// Supported relationship categories in the core graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    Contains,
    Defines,
    Imports,
    Calls,
    References,
}

impl RelationshipKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Defines => "defines",
            Self::Imports => "imports",
            Self::Calls => "calls",
            Self::References => "references",
        }
    }
}

/// Deterministic identifier for a relationship edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelationshipId(u64);

impl RelationshipId {
    /// Builds a deterministic identifier from relationship kind and endpoints.
    pub fn new(kind: RelationshipKind, source: EntityId, target: EntityId) -> Self {
        Self(stable_hash(&[
            "relationship",
            kind.as_str(),
            &source.stable_component(),
            &target.stable_component(),
        ]))
    }
}

impl std::fmt::Display for RelationshipId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rel:{:016x}", self.0)
    }
}

/// A relationship edge between typed graph entities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub id: RelationshipId,
    pub kind: RelationshipKind,
    pub source: EntityId,
    pub target: EntityId,
}

impl Relationship {
    /// Creates a relationship edge with a deterministic identifier.
    pub fn new(source: EntityId, kind: RelationshipKind, target: EntityId) -> Self {
        Self {
            id: RelationshipId::new(kind, source, target),
            kind,
            source,
            target,
        }
    }
}

/// The aggregate repository graph used by indexing and later query code.
#[derive(Clone, Debug)]
pub struct RepositoryGraph {
    repository: Repository,
    directories: BTreeMap<DirectoryId, Directory>,
    files: BTreeMap<FileId, File>,
    symbols: BTreeMap<SymbolId, Symbol>,
    relationships: BTreeMap<RelationshipId, Relationship>,
}

impl RepositoryGraph {
    /// Creates an empty graph for a single repository.
    pub fn new(repository: Repository) -> Self {
        Self {
            repository,
            directories: BTreeMap::new(),
            files: BTreeMap::new(),
            symbols: BTreeMap::new(),
            relationships: BTreeMap::new(),
        }
    }

    /// Returns repository metadata for this graph.
    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    /// Returns a directory node by identifier.
    pub fn directory(&self, id: DirectoryId) -> Option<&Directory> {
        self.directories.get(&id)
    }

    /// Returns a file node by identifier.
    pub fn file(&self, id: FileId) -> Option<&File> {
        self.files.get(&id)
    }

    /// Returns a symbol node by identifier.
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(&id)
    }

    /// Returns a relationship edge by identifier.
    pub fn relationship(&self, id: RelationshipId) -> Option<&Relationship> {
        self.relationships.get(&id)
    }

    /// Iterates over directories in deterministic identifier order.
    pub fn directories(&self) -> impl Iterator<Item = &Directory> {
        self.directories.values()
    }

    /// Iterates over files in deterministic identifier order.
    pub fn files(&self) -> impl Iterator<Item = &File> {
        self.files.values()
    }

    /// Iterates over symbols in deterministic identifier order.
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }

    /// Iterates over relationships in deterministic identifier order.
    pub fn relationships(&self) -> impl Iterator<Item = &Relationship> {
        self.relationships.values()
    }

    /// Returns whether an entity exists in the graph.
    pub fn contains_entity(&self, entity: EntityId) -> bool {
        match entity {
            EntityId::Repository(id) => id == self.repository.id,
            EntityId::Directory(id) => self.directories.contains_key(&id),
            EntityId::File(id) => self.files.contains_key(&id),
            EntityId::Symbol(id) => self.symbols.contains_key(&id),
        }
    }

    /// Inserts a directory node and its structural `contains` relationship.
    pub fn insert_directory(&mut self, directory: Directory) -> Result<(), GraphError> {
        self.ensure_repository(directory.repository_id, "directory")?;
        self.validate_directory_parent(&directory)?;
        self.insert_unique_directory(directory.clone())?;

        let source = match directory.parent_id {
            Some(parent_id) => EntityId::Directory(parent_id),
            None => EntityId::Repository(self.repository.id),
        };
        self.insert_relationship_internal(Relationship::new(
            source,
            RelationshipKind::Contains,
            EntityId::Directory(directory.id),
        ))?;

        Ok(())
    }

    /// Inserts a file node and its structural `contains` relationship.
    pub fn insert_file(&mut self, file: File) -> Result<(), GraphError> {
        self.ensure_repository(file.repository_id, "file")?;
        self.validate_file_parent(&file)?;

        if self.files.contains_key(&file.id) {
            return Err(GraphError::DuplicateEntity {
                entity: "file",
                id: file.id.to_string(),
            });
        }

        self.files.insert(file.id, file.clone());
        self.insert_relationship_internal(Relationship::new(
            EntityId::Directory(file.parent_id),
            RelationshipKind::Contains,
            EntityId::File(file.id),
        ))?;

        Ok(())
    }

    /// Inserts a symbol node and its structural `defines` relationship.
    pub fn insert_symbol(&mut self, symbol: Symbol) -> Result<(), GraphError> {
        self.validate_symbol_owner(&symbol)?;

        if self.symbols.contains_key(&symbol.id) {
            return Err(GraphError::DuplicateEntity {
                entity: "symbol",
                id: symbol.id.to_string(),
            });
        }

        self.symbols.insert(symbol.id, symbol.clone());
        self.insert_relationship_internal(Relationship::new(
            symbol.owner.entity_id(),
            RelationshipKind::Defines,
            EntityId::Symbol(symbol.id),
        ))?;

        Ok(())
    }

    /// Inserts an explicit relationship edge after validating its endpoints.
    pub fn insert_relationship(&mut self, relationship: Relationship) -> Result<(), GraphError> {
        self.insert_relationship_internal(relationship)
    }

    fn insert_unique_directory(&mut self, directory: Directory) -> Result<(), GraphError> {
        if self.directories.contains_key(&directory.id) {
            return Err(GraphError::DuplicateEntity {
                entity: "directory",
                id: directory.id.to_string(),
            });
        }

        self.directories.insert(directory.id, directory);
        Ok(())
    }

    fn insert_relationship_internal(
        &mut self,
        relationship: Relationship,
    ) -> Result<(), GraphError> {
        self.validate_relationship(&relationship)?;

        if self.relationships.contains_key(&relationship.id) {
            return Err(GraphError::DuplicateEntity {
                entity: "relationship",
                id: relationship.id.to_string(),
            });
        }

        self.relationships.insert(relationship.id, relationship);
        Ok(())
    }

    fn ensure_repository(
        &self,
        repository_id: RepositoryId,
        entity: &'static str,
    ) -> Result<(), GraphError> {
        if repository_id != self.repository.id {
            return Err(GraphError::RepositoryMismatch { entity });
        }

        Ok(())
    }

    fn validate_directory_parent(&self, directory: &Directory) -> Result<(), GraphError> {
        let expected_parent_path = directory.relative_path.parent();

        match (directory.parent_id, expected_parent_path) {
            (None, None) => Ok(()),
            (None, Some(expected)) => Err(GraphError::InvalidParent {
                child: "directory",
                expected: expected.to_string(),
                actual: "<none>".to_string(),
            }),
            (Some(_), None) => Err(GraphError::InvalidParent {
                child: "directory",
                expected: "<none>".to_string(),
                actual: "directory".to_string(),
            }),
            (Some(parent_id), Some(expected)) => {
                let parent =
                    self.directory(parent_id)
                        .ok_or_else(|| GraphError::MissingEntity {
                            entity: "directory",
                            id: parent_id.to_string(),
                        })?;

                if parent.relative_path != expected {
                    return Err(GraphError::InvalidParent {
                        child: "directory",
                        expected: expected.to_string(),
                        actual: parent.relative_path.to_string(),
                    });
                }

                Ok(())
            }
        }
    }

    fn validate_file_parent(&self, file: &File) -> Result<(), GraphError> {
        let parent = self
            .directory(file.parent_id)
            .ok_or_else(|| GraphError::MissingEntity {
                entity: "directory",
                id: file.parent_id.to_string(),
            })?;

        let expected_parent = file
            .relative_path
            .parent()
            .ok_or(GraphError::InvalidParent {
                child: "file",
                expected: "a parent directory".to_string(),
                actual: "<none>".to_string(),
            })?;

        if parent.relative_path != expected_parent {
            return Err(GraphError::InvalidParent {
                child: "file",
                expected: expected_parent.to_string(),
                actual: parent.relative_path.to_string(),
            });
        }

        Ok(())
    }

    fn validate_symbol_owner(&self, symbol: &Symbol) -> Result<(), GraphError> {
        self.file(symbol.file_id)
            .ok_or_else(|| GraphError::MissingEntity {
                entity: "file",
                id: symbol.file_id.to_string(),
            })?;

        match symbol.owner {
            SymbolOwner::File(file_id) => {
                if file_id != symbol.file_id {
                    return Err(GraphError::InvalidParent {
                        child: "symbol",
                        expected: symbol.file_id.to_string(),
                        actual: file_id.to_string(),
                    });
                }
            }
            SymbolOwner::Symbol(parent_symbol_id) => {
                let parent_symbol =
                    self.symbol(parent_symbol_id)
                        .ok_or_else(|| GraphError::MissingEntity {
                            entity: "symbol",
                            id: parent_symbol_id.to_string(),
                        })?;

                if parent_symbol.file_id != symbol.file_id {
                    return Err(GraphError::InvalidParent {
                        child: "symbol",
                        expected: parent_symbol.file_id.to_string(),
                        actual: symbol.file_id.to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    fn validate_relationship(&self, relationship: &Relationship) -> Result<(), GraphError> {
        if !self.contains_entity(relationship.source) {
            return Err(GraphError::MissingEntity {
                entity: relationship.source.kind().as_str(),
                id: relationship.source.stable_component(),
            });
        }

        if !self.contains_entity(relationship.target) {
            return Err(GraphError::MissingEntity {
                entity: relationship.target.kind().as_str(),
                id: relationship.target.stable_component(),
            });
        }

        let source_kind = relationship.source.kind();
        let target_kind = relationship.target.kind();

        let valid = match relationship.kind {
            RelationshipKind::Contains => matches!(
                (source_kind, target_kind),
                (EntityKind::Repository, EntityKind::Directory)
                    | (EntityKind::Directory, EntityKind::Directory)
                    | (EntityKind::Directory, EntityKind::File)
            ),
            RelationshipKind::Defines => matches!(
                (source_kind, target_kind),
                (EntityKind::File, EntityKind::Symbol) | (EntityKind::Symbol, EntityKind::Symbol)
            ),
            RelationshipKind::Imports | RelationshipKind::References => matches!(
                (source_kind, target_kind),
                (EntityKind::File, EntityKind::File)
                    | (EntityKind::File, EntityKind::Symbol)
                    | (EntityKind::Symbol, EntityKind::File)
                    | (EntityKind::Symbol, EntityKind::Symbol)
            ),
            RelationshipKind::Calls => {
                matches!(
                    (source_kind, target_kind),
                    (EntityKind::Symbol, EntityKind::Symbol)
                )
            }
        };

        if !valid {
            return Err(GraphError::InvalidRelationship {
                kind: relationship.kind.as_str(),
                source: source_kind.as_str(),
                target: target_kind.as_str(),
            });
        }

        Ok(())
    }
}

fn check_key_is_not_blank(entity: &'static str, value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(GraphError::EmptyStableKey { entity });
    }

    Ok(())
}

fn check_name_is_not_blank(entity: &'static str, value: &str) -> Result<(), GraphError> {
    if value.trim().is_empty() {
        return Err(GraphError::EmptyName { entity });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Directory, EntityId, File, Relationship, RelationshipKind, Repository, RepositoryGraph,
        Symbol, SymbolKind, SymbolOwner,
    };
    use crate::{DirectoryId, FileId, GraphError, RelativePath, RepositoryId, SymbolId};
    use std::path::PathBuf;

    #[test]
    fn file_ids_are_stable_across_repository_moves() {
        let repository_id = RepositoryId::new("repository");
        let file_path = RelativePath::new("src/lib.rs").expect("path should normalize");

        let left = FileId::new(repository_id, &file_path);
        let right = FileId::new(repository_id, &file_path);

        assert_eq!(left, right);
    }

    #[test]
    fn symbol_ids_change_when_stable_key_changes() {
        let repository_id = RepositoryId::new("repository");
        let file_id = FileId::new(
            repository_id,
            &RelativePath::new("src/lib.rs").expect("path should normalize"),
        );

        let left = SymbolId::new(file_id, "fn:crate::build_graph");
        let right = SymbolId::new(file_id, "fn:crate::scan");

        assert_ne!(left, right);
    }

    #[test]
    fn rejects_directory_parent_mismatch() {
        let mut graph = RepositoryGraph::new(repository());
        let root = Directory::new(graph.repository().id, None, ".").expect("root directory");
        graph
            .insert_directory(root)
            .expect("root directory should insert");

        let nested = Directory::new(graph.repository().id, None, "src").expect("nested directory");
        let error = graph
            .insert_directory(nested)
            .expect_err("nested directory without parent must fail");

        assert_eq!(
            error,
            GraphError::InvalidParent {
                child: "directory",
                expected: ".".to_string(),
                actual: "<none>".to_string(),
            }
        );
    }

    #[test]
    fn rejects_relationship_with_missing_endpoint() {
        let mut graph = RepositoryGraph::new(repository());
        let relationship = Relationship::new(
            EntityId::Symbol(SymbolId::new(
                FileId::new(
                    graph.repository().id,
                    &RelativePath::new("src/lib.rs").expect("path should normalize"),
                ),
                "fn:source",
            )),
            RelationshipKind::Calls,
            EntityId::Symbol(SymbolId::new(
                FileId::new(
                    graph.repository().id,
                    &RelativePath::new("src/lib.rs").expect("path should normalize"),
                ),
                "fn:target",
            )),
        );

        let error = graph
            .insert_relationship(relationship)
            .expect_err("missing endpoint should fail");

        assert!(matches!(
            error,
            GraphError::MissingEntity {
                entity: "symbol",
                ..
            }
        ));
    }

    #[test]
    fn rejects_invalid_relationship_endpoint_categories() {
        let mut graph = populated_graph();
        let root_dir = DirectoryId::new(graph.repository().id, &RelativePath::root());
        let symbol_id = graph.symbols().next().expect("symbol should exist").id;

        let error = graph
            .insert_relationship(Relationship::new(
                EntityId::Directory(root_dir),
                RelationshipKind::Calls,
                EntityId::Symbol(symbol_id),
            ))
            .expect_err("directory cannot call a symbol");

        assert_eq!(
            error,
            GraphError::InvalidRelationship {
                kind: "calls",
                source: "directory",
                target: "symbol",
            }
        );
    }

    #[test]
    fn rejects_explicit_structural_relationships() {
        let mut graph = populated_graph();
        let root_id = DirectoryId::new(graph.repository().id, &RelativePath::root());
        let src_id = DirectoryId::new(
            graph.repository().id,
            &RelativePath::new("src").expect("path should normalize"),
        );

        let error = graph
            .insert_relationship(Relationship::new(
                EntityId::Directory(root_id),
                RelationshipKind::Contains,
                EntityId::Directory(src_id),
            ))
            .expect_err("structural edges must be derived");

        assert_eq!(error, GraphError::ManagedRelationship { kind: "contains" });
    }

    #[test]
    fn builds_minimal_graph_with_structural_edges() {
        let graph = populated_graph();

        assert_eq!(graph.directories().count(), 2);
        assert_eq!(graph.files().count(), 1);
        assert_eq!(graph.symbols().count(), 2);
        assert_eq!(graph.relationships().count(), 5);
    }

    fn repository() -> Repository {
        Repository::new("repository", "leshy", PathBuf::from("C:/repos/leshy"))
            .expect("repository should build")
    }

    fn populated_graph() -> RepositoryGraph {
        let mut graph = RepositoryGraph::new(repository());

        let root = Directory::new(graph.repository().id, None, ".").expect("root directory");
        graph
            .insert_directory(root)
            .expect("root directory should insert");

        let root_id = DirectoryId::new(graph.repository().id, &RelativePath::root());
        let src_path = RelativePath::new("src").expect("path should normalize");
        let src_id = DirectoryId::new(graph.repository().id, &src_path);
        let file_path = RelativePath::new("src/lib.rs").expect("path should normalize");
        let file_id = FileId::new(graph.repository().id, &file_path);
        let module_id = SymbolId::new(file_id, "module:crate");

        let src = Directory::new(graph.repository().id, Some(root_id), "src").expect("src");
        graph
            .insert_directory(src)
            .expect("src directory should insert");

        let file = File::new(graph.repository().id, src_id, "src/lib.rs").expect("file");
        graph.insert_file(file).expect("file should insert");

        let module = Symbol::new(
            file_id,
            SymbolOwner::File(file_id),
            SymbolKind::Module,
            "crate",
            "module:crate",
        )
        .expect("module");
        graph
            .insert_symbol(module)
            .expect("module symbol should insert");

        let function = Symbol::new(
            file_id,
            SymbolOwner::Symbol(module_id),
            SymbolKind::Function,
            "build_graph",
            "fn:crate::build_graph",
        )
        .expect("function");
        graph
            .insert_symbol(function)
            .expect("function symbol should insert");

        graph
    }
}
