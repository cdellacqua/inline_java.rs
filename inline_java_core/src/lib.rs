//! Core runtime support for `inline_java`.
//!
//! This crate is an implementation detail of `inline_java_macros`.  End users
//! should depend on `inline_java` instead of this crate directly.
//!
//! Public items:
//!
//! - [`JavaError`] — error type returned by [`run_java`] and by the `java!` /
//!   `java_fn!` macros at program runtime.
//! - [`run_java`] — compile (if needed) and run a generated Java class.
//! - [`expand_java_args`] — shell-expand an option string into individual args.
//! - [`cache_dir`] — compute the deterministic temp-dir path for a Java class.
//! - [`detect_java_version`] — probe `javac -version` and return the major version.

use shellexpand::full_with_context_no_errors;

/// All errors that `java!` and `java_fn!` can return at runtime (and that
/// `ct_java!` maps to `compile_error!` diagnostics at build time).
#[derive(Debug, thiserror::Error, PartialEq, Eq, Clone)]
pub enum JavaError {
	/// An I/O error while creating the temp directory, writing the source
	/// file, or spawning `javac`/`java` (e.g. the binary is not on `PATH`).
	#[error("inline_java: I/O error: {0}")]
	Io(String),

	/// `javac` exited with a non-zero status.  The `0` field contains the
	/// compiler diagnostic output (stderr).
	#[error("inline_java: javac compilation failed:\n{0}")]
	CompilationFailed(String),

	/// The JVM exited with a non-zero status (e.g. an unhandled exception).
	/// The `0` field contains the exception message and stack trace (stderr).
	#[error("inline_java: java runtime failed:\n{0}")]
	RuntimeFailed(String),

	/// The Java `run()` method returned a `String` whose bytes are not valid
	/// UTF-8.
	#[error("inline_java: Java String output is not valid UTF-8: {0}")]
	InvalidUtf8(#[from] std::string::FromUtf8Error),

	/// The Java `run()` method returned a `char` value that is not a valid
	/// Unicode scalar (i.e. a lone surrogate half).
	#[error("inline_java: Java char is not a valid Unicode scalar value")]
	InvalidChar,
}

/// Shell-expand `raw` (expanding env vars and `~`), then split into individual
/// arguments (respecting quotes).
/// Returns an empty vec if `raw` is empty.
///
/// # Examples
///
/// ```rust
/// use inline_java_core::expand_java_args;
///
/// let args = expand_java_args("-verbose:class -Xmx512m");
/// assert_eq!(args, vec!["-verbose:class", "-Xmx512m"]);
///
/// let empty = expand_java_args("");
/// assert!(empty.is_empty());
/// ```
#[must_use]
pub fn expand_java_args(raw: &str) -> Vec<String> {
	if raw.is_empty() {
		return Vec::new();
	}
	let expanded = full_with_context_no_errors(
		raw,
		|| std::env::var("HOME").ok(),
		|var| std::env::var(var).ok(),
	);
	split_args(&expanded)
}

/// Inject `extra_cp` into `args` by appending it to any existing `-cp`,
/// `-classpath`, or `--class-path` value, or by appending `-cp extra_cp`
/// if no classpath flag is present.
fn inject_classpath(args: &mut Vec<String>, extra_cp: &str) {
	const SPACE_FLAGS: &[&str] = &["-cp", "-classpath", "--class-path"];
	for i in 0..args.len() {
		if SPACE_FLAGS.contains(&args[i].as_str()) && i + 1 < args.len() {
			args[i + 1].push(CP_SEP);
			args[i + 1].push_str(extra_cp);
			return;
		}
		if let Some(val) = args[i].strip_prefix("--class-path=") {
			args[i] = format!("--class-path={val}{CP_SEP}{extra_cp}");
			return;
		}
	}
	args.push("-cp".to_owned());
	args.push(extra_cp.to_owned());
}

/// Split a shell-style argument string into individual arguments, respecting
/// single- and double-quoted spans.
fn split_args(s: &str) -> Vec<String> {
	let mut args: Vec<String> = Vec::new();
	let mut cur = String::new();
	let mut in_single = false;
	let mut in_double = false;

	for ch in s.chars() {
		match ch {
			'\'' if !in_double => in_single = !in_single,
			'"' if !in_single => in_double = !in_double,
			' ' | '\t' if !in_single && !in_double => {
				if !cur.is_empty() {
					args.push(std::mem::take(&mut cur));
				}
			}
			_ => cur.push(ch),
		}
	}
	if !cur.is_empty() {
		args.push(cur);
	}
	args
}

/// Classpath entry separator: `:` on Unix, `;` on Windows.
#[cfg(not(windows))]
const CP_SEP: char = ':';
#[cfg(windows)]
const CP_SEP: char = ';';

/// Detect the installed Java major version by running `javac -version`.
///
/// `javac` (not `java`) is used because the compiler determines what class-file
/// format is produced; the runtime is not guaranteed to be present.  Output is
/// read from stdout (most JDKs) with stderr as fallback.
///
/// # Errors
///
/// Returns [`JavaError::Io`] if `javac` cannot be spawned or its output cannot
/// be parsed as a version string.
pub fn detect_java_version() -> Result<String, JavaError> {
	let output = std::process::Command::new("javac")
		.arg("-version")
		.output()
		.map_err(|e| JavaError::Io(format!("failed to run `javac -version`: {e}")))?;
	// `javac -version` writes to stdout on most JDKs, e.g. "javac 21.0.10\n"
	// (some older JDKs write to stderr; check both, prefer stdout)
	let stdout = String::from_utf8_lossy(&output.stdout);
	let stderr = String::from_utf8_lossy(&output.stderr);
	let raw = if stdout.trim().is_empty() {
		&*stderr
	} else {
		&*stdout
	};
	let version_str = raw.trim().strip_prefix("javac ").unwrap_or(raw.trim());
	let major = version_str
		.split('.')
		.next()
		.and_then(|s| s.parse::<u32>().ok())
		.ok_or_else(|| {
			JavaError::Io(format!(
				"could not parse major version from `javac -version` output: {raw:?}"
			))
		})?;
	Ok(major.to_string())
}

/// Resolve the root directory used to cache compiled `.class` files.
///
/// Resolution order:
/// 1. `INLINE_JAVA_CACHE_DIR` environment variable, if set and non-empty.
/// 2. The XDG / platform cache directory (`~/.cache/inline_java` on Linux,
///    `~/Library/Caches/inline_java` on macOS, `%LOCALAPPDATA%\inline_java`
///    on Windows) via the [`dirs`] crate.
/// 3. `<system_temp>/inline_java` as a final fallback.
#[must_use]
pub fn base_cache_dir() -> std::path::PathBuf {
	if let Ok(v) = std::env::var("INLINE_JAVA_CACHE_DIR")
		&& !v.is_empty()
	{
		return std::path::PathBuf::from(v);
	}
	if let Some(cache) = dirs::cache_dir() {
		return cache.join("inline_java");
	}
	std::env::temp_dir().join("inline_java")
}

/// Compute the deterministic cache-dir path used to store compiled `.class` files.
///
/// The path is `<base_cache_dir>/<class_name>_<hex_hash>/` where `hex_hash` is a
/// 64-bit hash of:
/// - `java_class` — the complete Java source text
/// - `expand_java_args(javac_raw)` — shell-expanded javac args (env vars and
///   `~` substituted); relative paths in these args are anchored by the next
///   component below
/// - `std::env::current_dir()` — the process working directory at call time;
///   including it ensures that two invocations with the same `javac_raw`
///   containing relative paths (e.g. `-cp .`) but from different working
///   directories hash to different cache entries
/// - `java_raw` — hashed as a raw string (no expansion needed for cache
///   differentiation; the `java` step always re-runs fresh)
/// - [`detect_java_version()`] — the installed `javac` major version; ensures
///   that upgrading the JDK produces a fresh cache entry whose `.class` files
///   are compiled by the new compiler
///
/// The base directory is resolved by [`base_cache_dir`].
///
/// # Errors
///
/// Returns [`JavaError::Io`] if `javac -version` cannot be run or its output
/// cannot be parsed (see [`detect_java_version`]).
#[allow(clippy::similar_names)]
pub fn cache_dir(
	class_name: &str,
	java_class: &str,
	javac_raw: &str,
	java_raw: &str,
) -> Result<std::path::PathBuf, JavaError> {
	use std::collections::hash_map::DefaultHasher;
	use std::hash::{Hash, Hasher};

	let mut h = DefaultHasher::new();
	java_class.hash(&mut h);
	expand_java_args(javac_raw).hash(&mut h); // shell-expanded; CWD handles relative paths
	std::env::current_dir().ok().hash(&mut h); // anchors relative paths in javac_raw
	java_raw.hash(&mut h);
	detect_java_version()?.hash(&mut h); // avoids .class collisions across JDK major versions

	let hex = format!("{:016x}", h.finish());
	Ok(base_cache_dir().join(format!("{class_name}_{hex}")))
}

/// Compile (if needed) and run a generated Java class, returning raw stdout bytes.
///
/// Both the compile step (javac) and the run step (java) are guarded by a
/// per-class-name file lock so that concurrent invocations cooperate correctly.
/// A `.done` sentinel and an optimistic pre-check skip recompilation on
/// subsequent calls without acquiring the lock.
///
/// - `class_name`      — bare class name; used as the temp-dir name.
/// - `filename`        — `"<class_name>.java"`, written inside the temp dir.
/// - `java_class`      — complete `.java` source to write.
/// - `full_class_name` — package-qualified class name passed to `java`.
/// - `javac_raw`       — raw `javac = "..."` option string (shell-expanded).
/// - `java_raw`        — raw `java  = "..."` option string (shell-expanded).
/// - `stdin_bytes`     — bytes to pipe to the child process's stdin (may be empty).
///
/// # Errors
///
/// Returns [`JavaError::Io`] if the temp directory, source file, or lock file
/// cannot be created, or if `javac`/`java` cannot be spawned.
/// Returns [`JavaError::CompilationFailed`] if `javac` exits with a non-zero status.
/// Returns [`JavaError::RuntimeFailed`] if `java` exits with a non-zero status.
///
/// # Examples
///
/// ```rust,no_run
/// use inline_java_core::run_java;
///
/// let src = "public class Greet {
///     public static void main(String[] args) {
///         System.out.print(\"hi\");
///     }
/// }";
/// let output = run_java("Greet", "Greet.java", src, "Greet", "", "", &[]).unwrap();
/// assert_eq!(output, b"hi");
/// ```
#[allow(clippy::similar_names)]
pub fn run_java(
	class_name: &str,
	filename: &str,
	java_class: &str,
	full_class_name: &str,
	javac_raw: &str,
	java_raw: &str,
	stdin_bytes: &[u8],
) -> Result<Vec<u8>, JavaError> {
	use std::io::Write;
	use std::process::Stdio;

	let tmp_dir = cache_dir(class_name, java_class, javac_raw, java_raw)?;
	let javac_extra = expand_java_args(javac_raw);
	let mut java_extra = expand_java_args(java_raw);
	inject_classpath(&mut java_extra, &tmp_dir.to_string_lossy());

	if !tmp_dir.join(".done").exists() {
		std::fs::create_dir_all(&tmp_dir).map_err(|e| JavaError::Io(e.to_string()))?;

		let lock_file = std::fs::OpenOptions::new()
			.create(true)
			.truncate(false)
			.write(true)
			.open(tmp_dir.join(".lock"))
			.map_err(|e| JavaError::Io(e.to_string()))?;
		let mut lock = fd_lock::RwLock::new(lock_file);
		let _guard = lock.write().map_err(|e| JavaError::Io(e.to_string()))?;

		if !tmp_dir.join(".done").exists() {
			let src = tmp_dir.join(filename);
			std::fs::write(&src, java_class).map_err(|e| JavaError::Io(e.to_string()))?;

			let mut cmd = std::process::Command::new("javac");
			for arg in &javac_extra {
				cmd.arg(arg);
			}
			let out = cmd
				.arg("-d")
				.arg(&tmp_dir)
				.arg(&src)
				.output()
				.map_err(|e| JavaError::Io(e.to_string()))?;
			if !out.status.success() {
				return Err(JavaError::CompilationFailed(
					String::from_utf8_lossy(&out.stderr).into_owned(),
				));
			}

			std::fs::write(tmp_dir.join(".done"), b"").map_err(|e| JavaError::Io(e.to_string()))?;
		}
	}

	let mut cmd = std::process::Command::new("java");
	for arg in &java_extra {
		cmd.arg(arg);
	}
	let mut child = cmd
		.arg(full_class_name)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.map_err(|e| JavaError::Io(e.to_string()))?;

	// Write stdin bytes then drop the handle to signal EOF.
	if stdin_bytes.is_empty() {
		// Drop stdin handle even when empty so Java doesn't block waiting.
		drop(child.stdin.take());
	} else if let Some(mut stdin_handle) = child.stdin.take() {
		stdin_handle
			.write_all(stdin_bytes)
			.map_err(|e| JavaError::Io(e.to_string()))?;
	}

	let out = child
		.wait_with_output()
		.map_err(|e| JavaError::Io(e.to_string()))?;

	if !out.status.success() {
		return Err(JavaError::RuntimeFailed(
			String::from_utf8_lossy(&out.stderr).into_owned(),
		));
	}

	Ok(out.stdout)
}

#[cfg(test)]
mod tests {
	use super::cache_dir;

	// -----------------------------------------------------------------------
	// cache_dir is idempotent: two calls with identical arguments return the
	// same path.
	// -----------------------------------------------------------------------
	#[test]
	fn cache_dir_idempotent() {
		let a = cache_dir("MyClass", "class body", "-cp /usr/lib", "-verbose").unwrap();
		let b = cache_dir("MyClass", "class body", "-cp /usr/lib", "-verbose").unwrap();
		assert_eq!(
			a, b,
			"cache_dir must return the same path for identical args"
		);
	}

	// -----------------------------------------------------------------------
	// cache_dir produces different paths for javac_raw strings that expand to
	// different argument lists.
	// -----------------------------------------------------------------------
	#[test]
	fn cache_dir_differs_for_different_javac_raw() {
		let a = cache_dir("MyClass", "class body", "-cp /usr/lib/foo", "").unwrap();
		let b = cache_dir("MyClass", "class body", "-cp /usr/lib/bar", "").unwrap();
		assert_ne!(
			a, b,
			"cache_dir must differ when javac_raw expands to different args"
		);
	}

	// -----------------------------------------------------------------------
	// cache_dir produces different paths when java_class differs.
	// -----------------------------------------------------------------------
	#[test]
	fn cache_dir_differs_for_different_java_class() {
		let a = cache_dir("MyClass", "class body A", "", "").unwrap();
		let b = cache_dir("MyClass", "class body B", "", "").unwrap();
		assert_ne!(a, b, "cache_dir must differ when java_class differs");
	}

	// -----------------------------------------------------------------------
	// cache_dir produces different paths when java_raw differs.
	// -----------------------------------------------------------------------
	#[test]
	fn cache_dir_differs_for_different_java_raw() {
		let a = cache_dir("MyClass", "class body", "", "-Xmx256m").unwrap();
		let b = cache_dir("MyClass", "class body", "", "-Xmx512m").unwrap();
		assert_ne!(a, b, "cache_dir must differ when java_raw differs");
	}

	// -----------------------------------------------------------------------
	// cache_dir result is inside base_cache_dir and uses the class_name as a
	// prefix.
	// -----------------------------------------------------------------------
	#[test]
	fn cache_dir_path_structure() {
		let result = cache_dir("InlineJava_abc123", "src", "", "").unwrap();
		let base = super::base_cache_dir();
		assert!(
			result.starts_with(&base),
			"cache_dir result must be under base_cache_dir ({}); got: {}",
			base.display(),
			result.display()
		);
		let file_name = result.file_name().unwrap().to_string_lossy();
		assert!(
			file_name.starts_with("InlineJava_abc123_"),
			"cache_dir result filename must start with the class name; got: {file_name}"
		);
	}

	// -----------------------------------------------------------------------
	// detect_java_version returns a non-empty numeric string when javac is
	// available in PATH (as it is in this repo's dev environment).
	// -----------------------------------------------------------------------
	#[test]
	fn detect_java_version_returns_major() {
		let version = super::detect_java_version().expect("javac must be on PATH");
		assert!(
			version.parse::<u32>().is_ok(),
			"version string must be a plain integer (major); got: {version:?}"
		);
	}

	// -----------------------------------------------------------------------
	// INLINE_JAVA_CACHE_DIR env var overrides the base cache directory.
	// -----------------------------------------------------------------------
	#[test]
	fn base_cache_dir_respects_env_var() {
		unsafe { std::env::set_var("INLINE_JAVA_CACHE_DIR", "/custom/cache") };
		let base = super::base_cache_dir();
		unsafe { std::env::remove_var("INLINE_JAVA_CACHE_DIR") };
		assert_eq!(base, std::path::PathBuf::from("/custom/cache"));
	}
}
