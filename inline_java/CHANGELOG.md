# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/cdellacqua/inline_java.rs/compare/inline_java-v0.1.0...inline_java-v0.1.1) - 2026-04-12

### Added

- add cache invalidation mechanism + include max mtime in hash computation
- support boxed primitives as a return type of run

## [0.1.0] - Unreleased

### Added

- `java!` macro: compile and run inline Java at program runtime (zero-arg form)
- `java_fn!` macro: compile and run inline Java at program runtime, with typed parameters passed over stdin
- `ct_java!` macro: run Java during `rustc` macro expansion and splice the result as a Rust literal
- Support for primitive types: `byte`, `short`, `int`, `long`, `float`, `double`, `boolean`, `char`
- Support for `String`, `T[]`, `List<T>`, and `Optional<T>` as parameter and return types, arbitrarily nested
- `javac = "..."` and `java = "..."` options for passing extra flags to the compiler and runtime
- `import` and `package` directives for referencing project Java source files
- Cross-process file locking to avoid redundant recompilation across parallel test runners
- `.done` sentinel for skip-recompile optimisation
