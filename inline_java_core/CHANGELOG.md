# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/cdellacqua/inline_java.rs/releases/tag/inline_java_core-v0.1.0) - 2026-03-12

### Added

- use platform cache dir for compiled class files
- support multiple paths per entry via CP_SEP
- implement binary argument passing via stdin

### Fixed

- normalize paths to absolute before hashing

### Other

- update metadata in Cargo.toml
- format
- prepare workspace for publishing
- remove canonicalization of paths
- remove INLINE_JAVA_CP env variable
- apply clippy fixes
- update inline documentation
- apply clippy fixes
- apply clippy fixes
- begin unifying macro implementations
