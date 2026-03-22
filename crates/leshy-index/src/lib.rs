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
mod tests;
