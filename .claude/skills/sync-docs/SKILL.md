---
name: sync-docs
description: Audit inline documentation (doc comments, module-level comments, README) against the current implementation and fix any stale, missing, or misleading content.
---

Review all inline documentation in the workspace and align it with the current implementation:

1. Read each source file and compare doc comments against what the code actually does.
2. Fix stale parameter descriptions, return-type docs, and `# Errors` / `# Panics` sections.
3. Update or remove examples that no longer compile or reflect current behaviour.
4. Ensure every public function and exported macro has a `# Examples` section with at least one fenced Rust code block (` ```rust `) showing a realistic invocation.
5. Ensure module-level and crate-level comments accurately describe the current architecture.
6. Do **not** add doc comments where the code is self-evident — only fix what is wrong or missing.
