This directory contains compact Rust fixture repositories for parser and indexing tests.

The fixtures are checked in on purpose instead of being cloned during test execution, so
the test suite stays hermetic and reproducible. The `mini_crate` fixture is intentionally
small, but it includes imports, module boundaries, nested files, and stable definition
locations so it can support LSHY-12, LSHY-13, and LSHY-14 together.
