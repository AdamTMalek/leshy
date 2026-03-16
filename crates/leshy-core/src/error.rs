use std::error::Error;
use std::fmt::{Display, Formatter};

/// Errors returned by the core repository graph model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    EmptyStableKey {
        entity: &'static str,
    },
    EmptyName {
        entity: &'static str,
    },
    InvalidRelativePath {
        path: String,
        reason: &'static str,
    },
    DuplicateEntity {
        entity: &'static str,
        id: String,
    },
    MissingEntity {
        entity: &'static str,
        id: String,
    },
    RepositoryMismatch {
        entity: &'static str,
    },
    ManagedRelationship {
        kind: &'static str,
    },
    InvalidParent {
        child: &'static str,
        expected: String,
        actual: String,
    },
    InvalidRelationship {
        kind: &'static str,
        source: &'static str,
        target: &'static str,
    },
}

impl Display for GraphError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyStableKey { entity } => {
                write!(f, "{entity} stable key must not be empty")
            }
            Self::EmptyName { entity } => write!(f, "{entity} name must not be empty"),
            Self::InvalidRelativePath { path, reason } => {
                write!(f, "invalid relative path `{path}`: {reason}")
            }
            Self::DuplicateEntity { entity, id } => {
                write!(f, "duplicate {entity} with id `{id}`")
            }
            Self::MissingEntity { entity, id } => {
                write!(f, "missing {entity} with id `{id}`")
            }
            Self::RepositoryMismatch { entity } => {
                write!(f, "{entity} belongs to a different repository")
            }
            Self::ManagedRelationship { kind } => {
                write!(f, "`{kind}` relationships are derived from graph structure")
            }
            Self::InvalidParent {
                child,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "invalid parent for {child}: expected `{expected}`, found `{actual}`"
                )
            }
            Self::InvalidRelationship {
                kind,
                source,
                target,
            } => {
                write!(
                    f,
                    "invalid `{kind}` relationship between `{source}` and `{target}`"
                )
            }
        }
    }
}

impl Error for GraphError {}
