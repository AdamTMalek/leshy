# Rust Stable Symbol ID Rules

This document defines the Rust-specific stable-key and ownership rules for Leshy.

It complements, and does not replace, the language-agnostic contract in [Stable Symbol ID Rules](stable-symbol-ids.md). If a Rust rule here conflicts with the general contract, the conflict must be resolved in the general contract first.

## Scope

These rules apply to Rust extraction, canonicalization, stable-key generation, and ownership resolution.

## 1. Crate root and namespace derivation

1. Rust symbol namespaces are crate-local, not repository-relative.
2. The module root is the crate's `src/` directory, even when the crate lives inside a workspace path such as `crates/<name>/src/`.
3. For Rust stable keys, repository path prefixes outside the crate root must never appear in canonical symbol paths.
4. `src/lib.rs` and `src/main.rs` each define a crate root namespace, but they are distinct crate scopes for owner resolution.
5. `src/bin/<name>.rs` defines a binary crate root namespace, not `bin::<name>`.
6. `src/bin/<name>/main.rs` defines a binary crate root namespace, not `bin::<name>`.
7. `build.rs` defines a build-script crate root namespace and crate scope, not `build`.
8. Files under `src/bin/<name>/...` are namespaced relative to that binary crate root.
9. Out-of-line module files under `src/bin/<name>/...` belong to that binary crate scope even when another crate root in the same package exposes the same module path.
10. `src/foo.rs` defines module namespace `foo`.
11. `src/foo/mod.rs` defines module namespace `foo`.
12. `src/foo/bar.rs` defines module namespace `foo::bar`.
13. Inline modules extend the current module namespace from the point where they are defined.
14. If one Rust source file is reused by multiple crate roots in the same package and Leshy cannot represent multiple semantic owners for that file, owner resolution must choose one crate scope deterministically rather than falling back to file ownership.
15. The current deterministic preference order for shared module files is `lib`, then `main`, then `build`, then binary crate scopes in lexical order.

## 2. Canonical Rust path spellings

1. Canonical local Rust paths are crate-local paths without a leading `crate::`.
2. `self::` and `super::` must be resolved relative to the current module namespace before stable keys are produced.
3. Equivalent local spellings such as `Widget`, `self::Widget`, `crate::outer::Widget`, or `super::Widget` must canonicalize to the same local path when they refer to the same definition.
4. Imported aliases must canonicalize to the same target path as the imported definition when the alias resolves unambiguously.
5. External paths may retain their fully qualified language spelling when no local repository definition exists.
6. Repository-wide local-owner resolution must stay within the current crate scope unless Rust semantics explicitly cross that boundary.

## 3. Rust `use` rules

1. `use`, `pub use`, and visibility-qualified forms such as `pub(crate) use` must resolve to the same imported target.
2. Grouped imports must preserve the semantics of the group prefix.
3. Grouped `self` entries, such as `use crate::outer::{self as outer_mod};`, refer to the group prefix itself, not to a fictitious `outer::self` path.
4. `as` aliases must affect only the local lookup name, not the canonical target path.
5. Import resolution must be scope-aware.
6. Relative `use` targets without an explicit `crate::`, `self::`, or `super::` prefix must be interpreted relative to the defining module scope when they resolve to local repository symbols.
7. Import resolution may be used for stable key canonicalization only when the target is unambiguous.
8. If a `use` form is unsupported, the fallback must remain deterministic and must not corrupt unrelated imported targets.

## 4. Rust impl target rules

1. The stable owner text for impl members must reflect the concrete impl target that distinguishes valid coexisting impls.
2. The semantic owner symbol for impl members must resolve to the nominal local type symbol when that symbol exists in the repository and the resolution is unambiguous.
3. These two concepts are distinct and must not be conflated:
   - stable owner text used inside stable keys
   - resolved owner symbol used for ownership edges
4. Inherent impls on distinct concrete targets such as `Wrapper<u8>` and `Wrapper<String>` must receive distinct stable keys.
5. Trait impls that differ by implemented trait, target type, or concrete type arguments must receive distinct stable keys.
6. If an impl target resolves to a local repository type through an import, module path, or equivalent local spelling, the owner must point to that local type symbol.
7. Impl ownership must not depend on whether the type definition appears earlier or later in source.
8. Impl ownership resolution must not bind a symbol in one crate to a type symbol from another crate merely because the crate-local path text matches.
9. A package's library crate and binary crate must be treated as separate crate scopes even when they share the same `src/` directory.

## 5. Rust associated item rules

1. Methods inside inherent impls are `method:` symbols.
2. Methods inside trait definitions are `method:` symbols scoped to the trait symbol.
3. Methods inside trait impls are `method:` symbols scoped to the implemented trait plus concrete target.
4. Associated constants inside impls or traits are `const:` symbols scoped to the same canonical owner text used for sibling methods.
5. Associated types inside traits or trait impls are `type:` symbols scoped to the enclosing trait or trait impl owner.
6. Free functions inside modules remain `fn:` symbols even when nested inside inline modules.
7. Nested items must remain attached to their semantic container even when the container is reached through imports or another module file.

## 6. Generic and specialized target rules

1. Generic parameters and concrete type arguments must be preserved in stable owner text whenever they distinguish valid coexisting impls.
2. Generic parameters and concrete type arguments must not be erased merely to make local owner lookup easier.
3. Nominal local owner lookup may normalize a local impl target to its nominal type symbol, but only for ownership resolution, not for stable key text.
4. Tuple, array, reference, raw pointer, function pointer, and other non-nominal impl targets may remain file-owned unless and until Leshy introduces first-class symbols for those target forms.

## 7. Keyword and token-boundary rules

1. Rust keyword handling must follow actual token boundaries.
2. An identifier that merely begins with keyword text is still an ordinary identifier.
3. For example, names such as `dynastore`, `dynmap`, or `structural` must not be treated as `dyn`, `dyn`, or `struct` keywords.
4. Heuristics based on `starts_with` are invalid when they can misclassify ordinary identifiers.

## 8. Rust fallback rules

1. If a Rust path cannot be resolved to a unique local repository symbol, the extractor must keep a deterministic unresolved stable spelling and fall back to file ownership.
2. Unsupported import forms must not silently collapse multiple distinct impls onto the same stable key.
3. Unsupported module-layout cases must not introduce workspace path prefixes into stable keys.
4. When adding support for a new Rust spelling, the implementation must include regression tests for:
   - direct path spelling
   - equivalent imported spelling
   - grouped-import spelling when applicable
   - cross-file module-file layout when applicable
