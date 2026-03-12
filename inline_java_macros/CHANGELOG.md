# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/cdellacqua/inline_java.rs/releases/tag/inline_java_macros-v0.1.0) - 2026-03-12

### Added

- support Optional<T> in parameters and return types
- implement binary argument passing via stdin
- use file-locking to synchronize concurrent compilation
- add overridable classpath
- support extra declarations in Java class body
- add list and array type support
- add shell expansion and error reporting
- add binary serialization for Java-Rust data exchange
- add argument passing from Rust to Java

### Fixed

- prevent race conditions on concurrent same-hash invocations

### Other

- update metadata in Cargo.toml
- format
- prepare workspace for publishing
- remove INLINE_JAVA_CP env variable
- apply clippy fixes
- borrow inputs, own outputs
- add recursive I/O serialization
- make run() visibility modifier optional
- update inline documentation
- apply clippy fixes
- apply clippy fixes
- begin unifying macro implementations
- remove _COMPILED sentinel; recycle javac outputs across runs
- clean up and apply linting
- improve source code handling
- initial commit
