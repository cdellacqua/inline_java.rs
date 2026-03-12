# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/cdellacqua/inline_java.rs/releases/tag/inline_java-v0.1.0) - 2026-03-12

### Added

- use platform cache dir for compiled class files
- support Optional<T> in parameters and return types
- implement binary argument passing via stdin
- use file-locking to synchronize concurrent compilation
- add overridable classpath
- add shell expansion and error reporting

### Fixed

- handle relative paths
- prevent race conditions on concurrent same-hash invocations

### Other

- update metadata in Cargo.toml
- restructure README for rustdoc compatibility
- move integration tests into inline_java_core
- prepare workspace for publishing
- showcase type inference
- remove canonicalization of paths
- remove INLINE_JAVA_CP env variable
- add README
- update inline documentation
- remove var injection pattern mentions
- make run() visibility modifier optional
- update inline documentation
- begin unifying macro implementations
