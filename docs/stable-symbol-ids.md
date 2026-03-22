# Stable Symbol ID Rules

This document defines the normative language-agnostic rules for stable keys and stable IDs in Leshy.

It exists to separate the indexing contract from any particular implementation. If the implementation and this document disagree, this document is the source of truth.

Language-specific supplements must extend this document rather than redefine it. The current language-specific supplement is:

- [Rust Stable Symbol ID Rules](stable-symbol-ids-rust.md)

## Terms

- Stable key: the human-readable canonical identity string for an entity.
- Stable ID: the hashed typed ID derived from a stable key and, where applicable, its parent scope.
- Defining file: the file that contains the defining syntax for a symbol.
- Owner: the semantic container that defines another symbol. In Leshy this is either a file or another symbol.
- Canonical path: the normalized semantic path used in stable keys after language-specific normalization has been applied.
- Local symbol: a symbol defined inside the repository being indexed.
- External symbol: a symbol referenced by source text but not defined inside the repository being indexed.

## Language-Agnostic Rules

### 1. Identity model

1. Repository, directory, file, and symbol identities are distinct concepts and must not share one another's stable keys.
2. A stable key must identify semantic meaning, not storage accidents such as insertion order, traversal order, memory addresses, or parser node IDs.
3. A stable ID must be a deterministic function of the stable key and the scope rules already defined in the core model.
4. For symbols, the defining file is part of the stable ID but not part of the stable key text.
5. A symbol may have the same stable key text as another symbol in a different defining file if and only if the language rules treat those as distinct definitions.

### 2. Required invariants

Every symbol extraction must satisfy all of the following:

1. Determinism: the same repository contents must yield the same stable keys and stable IDs across repeated indexing runs.
2. Order independence: declaration order must not change stable keys, owners, or ownership edges.
3. Whitespace independence: insignificant formatting differences must not change stable keys.
4. Span independence: source spans describe location only and must never participate in stable key generation.
5. Path-style independence: equivalent filesystem separator styles or repository checkout locations must not change stable keys.
6. Minimality: a stable key must encode only the information needed to distinguish the definition from other valid peer definitions.
7. Completeness: if two valid definitions may coexist in one repository, their stable keys must differ.

### 3. Stability boundaries

Stable keys must remain unchanged across:

1. Re-indexing the same repository contents.
2. Moving the repository checkout to a different absolute path.
3. Parser traversal changes that preserve the same semantic model.
4. Internal refactors to extraction code that do not change the semantic rules in this document.

Stable keys may change only when at least one of the following changes:

1. The defining symbol's semantic name changes.
2. The defining symbol's semantic owner path changes.
3. The symbol kind changes in a way that changes semantic identity.
4. The language-specific canonicalization rules in this document are intentionally revised.

If the rules are intentionally revised in a breaking way, the change must be documented explicitly in this file before or alongside the implementation change.

### 4. Stable key content rules

1. Stable keys must be human-readable and structured.
2. Stable keys must start with a kind prefix such as `module:`, `type:`, `fn:`, `method:`, or `const:`.
3. The content after the prefix must use the canonical semantic path, not an arbitrary syntactic spelling.
4. Equivalent local spellings that name the same definition must canonicalize to one stable key.
5. Distinct definitions must not be collapsed merely because they share a short display name.
6. Display names are descriptive only and must not be treated as unique identifiers.
7. Stable keys must not encode non-semantic parser artifacts such as raw node kinds or byte offsets.

### 5. Ownership rules

1. Every symbol must have exactly one owner.
2. Ownership is semantic, not textual.
3. A symbol's owner may be another symbol defined in a different file when the language permits out-of-file member definitions or equivalent constructs.
4. Ownership must be based on the resolved semantic container, not on whether the container has already been visited.
5. If a local owner can be resolved unambiguously, the symbol must point to that owner symbol.
6. If no local owner can be resolved unambiguously, the symbol must fall back to the defining file as owner.
7. Implementations must not guess between multiple plausible owners.

### 6. Canonicalization rules

1. Canonicalization must be semantic, not purely textual.
2. Canonicalization may normalize equivalent local spellings into one canonical spelling.
3. Canonicalization must not erase information that distinguishes valid coexisting definitions.
4. Keyword handling must follow token boundaries or parser structure, not string prefixes.
5. Namespace derivation must be rooted at the language's crate/package/module root, not at arbitrary repository path prefixes.
6. Import or alias resolution may contribute to canonicalization only when the target can be resolved unambiguously.
7. Visibility modifiers, formatting, and syntactic sugar must not change the canonical identity of the imported or defined target.

### 7. Ambiguity and unsupported constructs

1. When a construct is unsupported, the extractor must preserve a deterministic fallback stable key rather than emitting unstable guesses.
2. When a local target cannot be resolved unambiguously, ownership must fall back to the file.
3. Fallback stable keys must preserve enough syntax to remain distinct from other valid coexisting definitions whenever possible.
4. Unsupported resolution in one construct must not degrade unrelated symbols in the same file.
5. The extractor must prefer a stable unresolved representation over an incorrect resolved one.

### 8. Cross-file rules

1. Cross-file ownership is valid when the language semantics permit a definition in one file to belong to a symbol defined in another file.
2. Cross-file ownership must refer to the owner's true defining symbol ID, not to a same-file surrogate.
3. Cross-file ownership must not depend on whether the owner file is indexed before or after the child file.
4. Repository-level resolution must use the repository's semantic module structure, not just per-file local declarations.
5. Repository-level resolution must preserve the language's own root scope boundaries, such as crate, package, or module-root boundaries.
6. Two symbols that share the same language-local path text but belong to different root scopes must not collide during owner resolution.

### 9. Review checklist for any language integration

A stable-key scheme is incomplete if any of the following can change the output incorrectly:

1. Moving a definition to another valid file that preserves the same semantic module path.
2. Reordering declarations.
3. Switching between equivalent local path spellings.
4. Importing a symbol and using the import alias instead of the fully qualified name.
5. Introducing a second valid definition that should remain distinct.
6. Adding generics, specialization, or other type arguments that distinguish valid coexisting definitions.
7. Using visibility modifiers or grouped import forms that preserve the same semantic target.

## Change Rule

Future implementation work for stable keys and symbol IDs must start by checking proposed behavior against this document and any applicable language-specific supplement. If the desired behavior is not already described there, update the documentation first and only then change the implementation.
