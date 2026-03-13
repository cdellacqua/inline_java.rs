# Plan: Maven & Gradle Dependency Support

## Overview

Allow `java!`, `java_fn!`, and `ct_java!` to resolve third-party JVM dependencies
from a `pom.xml` (Maven) or `build.gradle` / `build.gradle.kts` (Gradle) file,
injecting the resolved classpath into both the `javac` and `java` invocations.

The feature is entirely optional. If no `deps = "..."` option is given, behaviour
is identical to today. If a deps file is specified but the required build tool is
not on `PATH`, a clear `JavaError::BuildToolNotFound` is returned instead of a
silent failure.

---

## New Macro Syntax

```rust
// Maven
let result: String = java! {
    deps = "pom.xml",
    import org.apache.commons.lang3.StringUtils;
    static String run() {
        return StringUtils.reverse("hello");
    }
}.unwrap();

// Gradle (Groovy DSL)
let result: String = java! {
    deps = "build.gradle",
    import com.google.gson.Gson;
    static String run() {
        return new Gson().toJson(42);
    }
}.unwrap();
```

The `deps` path is relative to `CARGO_MANIFEST_DIR` (the crate that contains the
`java!` call), consistent with how `-sourcepath` is typically used today.

---

## Tool Detection

The build tool is selected by inspecting the deps file name / extension:

| File pattern          | Tool   | Required binary on PATH |
|-----------------------|--------|-------------------------|
| `pom.xml`             | Maven  | `mvn`                   |
| `*.gradle`            | Gradle | `gradle` or `./gradlew` |
| `*.gradle.kts`        | Gradle | `gradle` or `./gradlew` |

For Gradle, a local `gradlew` / `gradlew.bat` wrapper in the same directory as the
build file takes precedence over the system `gradle` binary (mirrors standard
Gradle project convention).

---

## Dependency Resolution

### Maven

```sh
mvn dependency:build-classpath \
    -Dmdep.outputFile=<cache_dir>/deps.classpath \
    -f <abs_path_to_pom.xml> \
    -q --batch-mode
```

`-q --batch-mode` suppresses progress output so only errors reach stderr.

### Gradle

Because we do not want to modify the user's build file, we inject a
**temporary init script** that adds a one-shot task:

```groovy
// Written to a temp file, e.g. /tmp/inline_java_init_<hash>.gradle
allprojects {
    afterEvaluate {
        task inlineJavaPrintClasspath {
            doLast {
                def cp = configurations.findByName('compileClasspath')
                       ?: configurations.findByName('compile')
                       ?: configurations.findByName('runtimeClasspath')
                if (cp != null) {
                    println cp.resolve()
                              .collect { it.absolutePath }
                              .join(File.pathSeparator)
                }
            }
        }
    }
}
```

```sh
<gradle_binary> -b <abs_path_to_build_file> \
    --init-script <tmp_init_script> \
    inlineJavaPrintClasspath -q
```

The classpath is captured from stdout (the single printed line). The init script
file is written to `<base_cache_dir>/gradle_init_<hash>.gradle` so it persists and
does not need to be regenerated on each run.

---

## Caching

Dep resolution can be slow (network I/O, Gradle daemon startup). Results are
cached by a hash of the **deps file content** (not just the path):

```
<base_cache_dir>/deps_<hex_hash_of_file_content>/.classpath
```

If `.classpath` exists, `resolve_deps` returns its contents without invoking
`mvn`/`gradle`. This is intentionally aggressive: snapshot/dynamic versions
are not auto-refreshed.

**Force-refresh escape hatch**: set `INLINE_JAVA_REFRESH_DEPS=1` in the
environment to delete the `.classpath` sentinel before resolving, forcing a
fresh invocation of the build tool.

---

## Code Changes

### 1. `inline_java_core/src/lib.rs`

#### a. New `JavaError` variants

```rust
/// The required build tool (`mvn` or `gradle`) is not installed or not on PATH.
#[error("inline_java: build tool not found: {0}")]
BuildToolNotFound(String),

/// Dependency resolution via Maven or Gradle failed.
/// The `0` field contains the tool's stderr output.
#[error("inline_java: dependency resolution failed:\n{0}")]
DepsResolutionFailed(String),
```

#### b. New public function `resolve_deps`

```rust
/// Resolve dependencies from a Maven `pom.xml` or Gradle build file,
/// returning a classpath string (OS-appropriate separator).
///
/// Results are cached under `base_cache_dir()` keyed by the file's content
/// hash.  Set `INLINE_JAVA_REFRESH_DEPS=1` to force re-resolution.
pub fn resolve_deps(deps_file: &std::path::Path) -> Result<String, JavaError>
```

Internal steps:
1. Read `deps_file` contents → compute 64-bit content hash → check cache.
2. If cache hit and `INLINE_JAVA_REFRESH_DEPS` not set, return cached string.
3. Detect tool (Maven vs Gradle) and find binary (`which`-style search: try
   `./gradlew` adjacent to build file, then `gradle`; for Maven try `mvn`).
4. Run the resolution command (see above), capturing stdout/stderr.
5. On non-zero exit, return `DepsResolutionFailed(stderr)`.
6. Write classpath string to cache file; return it.

#### c. Modify `run_java` signature

Add one parameter after `java_raw`:

```rust
pub fn run_java(
    class_name: &str,
    filename: &str,
    java_class: &str,
    full_class_name: &str,
    javac_raw: &str,
    java_raw: &str,
    deps_file: &str,   // ← NEW: absolute path, or "" for no deps
    stdin_bytes: &[u8],
) -> Result<Vec<u8>, JavaError>
```

At the top of `run_java`, before the `.done` check:
```rust
let deps_cp = if deps_file.is_empty() {
    String::new()
} else {
    resolve_deps(std::path::Path::new(deps_file))?
};
```

Then inject `deps_cp` into both `javac_extra` and `java_extra`:
```rust
if !deps_cp.is_empty() {
    inject_classpath(&mut javac_extra, &deps_cp);
    inject_classpath(&mut java_extra, &deps_cp);
}
```

Note: `javac_extra` is currently `let` (immutable); change to `let mut`.

#### d. Update `cache_dir` to include deps_file

Include the resolved deps classpath in the cache-dir hash so that a change in
the deps file content produces a new compilation cache entry (avoiding stale
class files compiled against old jars):

Pass `deps_file: &str` to `cache_dir` as well; it hashes the file's content
if non-empty (read the file; on read failure, hash the path string as a
fallback).

---

### 2. `inline_java_macros/src/lib.rs`

#### a. Extend `JavaOpts`

```rust
struct JavaOpts {
    javac_args: Option<String>,
    java_args: Option<String>,
    deps: Option<String>,   // ← NEW: relative path to pom.xml / build.gradle
}
```

#### b. Update `extract_opts`

Add a `"deps"` arm in the key match:
```rust
"deps" => opts.deps = Some(val),
```

#### c. Update `make_class_name`

Hash `opts.deps` alongside the existing fields so that changing the deps file
path produces a different class name (and thus a different cache entry):
```rust
opts.deps.hash(&mut h);
```

#### d. Update `compile_run_java_now`

Add `deps_file: Option<&str>` parameter; pass it through to `run_java`.

Resolve the absolute path at macro expansion time (proc-macros run during
compilation, so `std::env::var("CARGO_MANIFEST_DIR")` is available):

```rust
let abs_deps = deps_file.map(|rel| {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_default();
    format!("{manifest_dir}/{rel}")
}).unwrap_or_default();
inline_java_core::run_java(..., &abs_deps, &[])
```

#### e. Update runtime macro expansion (`java!`, `java_fn!`)

In the generated `quote!` block, compute and embed the absolute deps path as a
string literal at macro expansion time (same technique: resolve
`CARGO_MANIFEST_DIR` in the proc-macro, concatenate with the relative user
path, embed the result as a `&str` literal):

```rust
// In the proc-macro (runs at compile time):
let deps_path_lit: proc_macro2::TokenStream = {
    let s = opts.deps.as_deref().map(|rel| {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        format!("{dir}/{rel}")
    }).unwrap_or_default();
    quote! { #s }
};

// In the generated code:
quote! {
    let _raw = ::inline_java::run_java(
        #class_name, #filename, #java_class, #full_class_name,
        #javac_raw, #java_raw, #deps_path_lit,
        &_stdin_bytes,
    )?;
}
```

---

### 3. `inline_java/src/lib.rs`

Re-export the two new error variants (they're already covered by re-exporting
`JavaError` itself; no change needed unless the existing re-export is selective).

---

## Error Handling Design

| Situation | Error returned |
|-----------|---------------|
| Tool not found on PATH | `JavaError::BuildToolNotFound("mvn not found on PATH")` |
| Tool exits non-zero | `JavaError::DepsResolutionFailed(<stderr>)` |
| deps file does not exist | `JavaError::Io(...)` (from `std::fs::read`) |
| Classpath contains non-UTF-8 | `JavaError::Io("deps classpath is not valid UTF-8")` |

For `ct_java!`, all `JavaError` variants are mapped to `compile_error!`
diagnostics, so a missing `mvn` becomes a hard build error with a clear message.

---

## Demo Update (`inline_java_demo`)

Add an optional demo entry in `main.rs` behind a feature flag or behind an
`#[cfg]` that checks for a pre-existing `pom.xml` in the demo directory.
Provide an example `demo/pom.xml` that depends on
`org.apache.commons:commons-lang3` to exercise the Maven path.

---

## Non-Goals (out of scope for this plan)

- Support for other build tools (SBT, Baht, Mill, etc.)
- Dependency version pinning / lockfiles managed by inline_java itself
- Automatic Gradle wrapper (`gradlew`) generation
- Multi-module Maven/Gradle projects (only single `pom.xml`/`build.gradle`)

---

## Implementation Order

1. `JavaError` new variants + `resolve_deps` + `run_java` signature in `inline_java_core`
2. Tests for `resolve_deps` (unit-test with a minimal `pom.xml` if `mvn` is
   available, otherwise skip with `#[ignore]`)
3. `cache_dir` update to accept and hash `deps_file`
4. `JavaOpts` + `extract_opts` + `make_class_name` in `inline_java_macros`
5. `compile_run_java_now` update
6. Runtime macro expansion update (`java!`, `java_fn!`)
7. Demo `pom.xml` + demo entry
8. Docs: crate-level doc comment additions, README section, CLAUDE.md update
