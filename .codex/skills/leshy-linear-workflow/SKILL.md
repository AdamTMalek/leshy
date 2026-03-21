---
name: leshy-linear-workflow
description: Work with Linear issues in the Leshy repository, including fetching issue context, creating the working branch, implementing the change, validating it with Cargo, committing with the repository's issue-aware commit format, and creating follow-up Linear issues for newly discovered bugs or missing features. Use when the user asks to work on a Leshy Linear ticket such as "Work on LSHY-123", "Work on LSHY-123 from Linear", or "Fix LSHY-123".
---

# Leshy Linear Workflow

## Overview

Use this skill to handle issue-driven work in this repository from intake through commit. Build context from Linear and the local codebase, then follow the repository's branching, validation, and commit rules exactly.

## Intake

Start by reading the Linear issue and extracting:

- Issue key, title, description, and acceptance criteria
- Current status, assignee, related issues, and blocking context when available
- Whether the work is a bug fix or a feature/task, because that determines commit wording

Then inspect the repository before editing. Respect [AGENTS.md](../../../AGENTS.md) and keep reusable logic in `crates/leshy-core` while keeping `crates/leshy-cli` focused on CLI orchestration.

## Branch Setup

Create a fresh branch before making issue-specific changes.

Branch format:

```text
<git-username>/<linear-issue>
```

Example:

```text
malekadas/LSHY-123
```

Resolve `<git-username>` from local git configuration if the repository or global config exposes it. If no reliable git username is configured, stop and ask the user instead of guessing.

Prefer commands equivalent to:

```bash
git checkout main
git pull --ff-only
git checkout -b <git-username>/<linear-issue>
```

If the local checkout already contains unrelated user changes, do not disturb them. Work with the existing tree carefully and avoid destructive git commands.

## Implementation

Make the smallest coherent change that satisfies the issue. Commit incrementally when a unit of work is complete and in a good state to preserve reviewable history.

Separate unrelated work from the ticket whenever possible:

- Formatting-only changes belong in a dedicated `format:` commit.
- Typo-only changes belong in a dedicated `typo:` commit.
- Other unrelated changes belong in a dedicated `misc:` commit.

If you discover a distinct bug or missing feature that is outside the scope of the current issue, create a new Linear issue for it instead of silently folding it into the current ticket.

## Commit Rules

Use these commit subject formats exactly:

For bug-fixing commits that fix the issue:

```text
fix LSHY-xxx: <title>
```

For feature/task commits that implement or complete the issue:

```text
completes LSHY-xxx: <title>
```

For issue-related commits that support the work but do not fix or complete it:

```text
ref LSHY-xxx: <title>
```

For unrelated formatting commits:

```text
format: <title>
```

For unrelated typo commits:

```text
typo: <title>
```

For unrelated miscellaneous commits:

```text
misc: <title>
```

An optional commit body is allowed when extra context is useful.

## Validation

Before making the final issue-resolving commit, run the standard Rust checks from the repository root:

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features -D warnings
```

If a command fails because of a real regression you introduced, fix it before committing. If a command fails for a pre-existing or environment-specific reason that you cannot resolve safely, capture that clearly in your final handoff.

## Final Handoff

When finishing the task:

- Summarize what changed and why
- Report the validation commands you ran and their results
- List each commit you created
- Mention any new Linear issue you filed for follow-up work
