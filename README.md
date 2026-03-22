# Leshy

**Leshy** is a fast code intelligence engine for repositories.

It analyzes a codebase and builds a structured representation of it, combining:

- repository structure
- syntax trees
- symbol information
- code relationships (calls, imports, references)

The result is a **queryable code graph** that can power developer tools, analysis pipelines, and AI systems.

The project is written in **Rust** and designed to scale to very large repositories.

---

## Motivation

Understanding large codebases is hard.

Most tools expose only one layer of information:

| Tool type | Provides |
|-----------|---------|
| AST parsers | syntax trees |
| static analyzers | semantic information |
| repo tools | filesystem structure |

Leshy unifies these layers into a single model:
```text
Repository
↓
Syntax (AST)
↓
Symbols
↓
Relationships
↓
Code Graph
```

This allows powerful queries like:

- "Which functions call this function?"
- "What files define this symbol?"
- "Which modules depend on this module?"
- "What is the structure of this repository?"

---

## Features

- Fast repository indexing
- Tree-sitter based parsing
- Language-agnostic architecture
- Symbol and call graph extraction
- Queryable code graph
- Designed for large repositories

Future goals:

- multi-language support
- incremental indexing
- graph queries
- LLM-friendly repository summaries

---

## Architecture

Leshy builds a **multi-layer representation of a codebase**.
```text
Filesystem Layer
├─ Root
├─ Directory
└─ File

Symbol Layer
├─ Module
├─ Struct / Class
├─ Function / Method
└─ Variable / Constant

Relationship Layer
├─ calls
├─ imports
├─ references
└─ defines
```


These layers are connected internally using stable IDs to allow efficient graph traversal.

The stable-ID contract is specified in [docs/stable-symbol-ids.md](docs/stable-symbol-ids.md), with the Rust-specific supplement in [docs/stable-symbol-ids-rust.md](docs/stable-symbol-ids-rust.md).

---

## Project Structure

The repository is organized as a Rust workspace:
```text
crates/
    leshy-core # core data structures
    leshy-repo # repository scanning
    leshy-parser # tree-sitter integration
    leshy-symbols # symbol extraction
    leshy-graph # relationship graph
    leshy-index # indexing pipeline
    leshy-query # graph queries
    leshy-cli # command line interface
```

---

## Installation

```bash
cargo install triglav
```
Or build locally
```bash
git clone https://github.com/AdamTMalek/leshy
cd leshy
cargo build --release
```

---

## Usage

Index a repository:

```bash
leshy index .
```

Repository scans honor Git ignore rules such as `.gitignore` and `.git/info/exclude`, while still indexing hidden files that are not ignored by Git.

Query relationships:

```bash
leshy query callers <symbol>
```

Visualize structure:

```bash
leshy graph
```

---

## Example Use Cases

### Code exploration
Understand the structure of unfamiliar repositories.

### Static analysis
Build custom analysis tools on top of the code graph.

### Developer tooling
Provide navigation, dependency insights, or architectural views.

### AI-assisted development
Generate repository summaries or build better code retrieval pipelines.

---

## Design goals
leshy is built with several principles in mind:
* **Performance first** – handle large repositories efficiently
* **Language agnostic** – support multiple languages through parsers
* **Composable** – usable as a library or CLI
* **Incremental** – update indexes without rebuilding everything
* **Extensible** – new languages and analyses can be added easily

---

## Roadmap
Roadmap

-[ ] Core indexing pipeline
-[ ] Rust language support
-[ ] Graph query API
-[ ] Multi-language support
-[ ] Incremental indexing
-[ ] Visualization tools
-[ ] LLM-friendly exports
