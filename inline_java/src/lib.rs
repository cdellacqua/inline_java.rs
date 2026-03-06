/// Re-export the proc macros so users only need to depend on this crate.
pub use inline_java_macros::{ct_java, java};

/// Re-exported for use by `java!`-generated code.
pub use fd_lock;

/// Expand shell variables in `raw` (with `INLINE_JAVA_CP` resolved to
/// `inline_java_cp`), then split the result into individual arguments.
/// Returns an empty vec if `raw` is empty.
///
/// Called from `java!`-generated code at runtime so that `$INLINE_JAVA_CP`
/// resolves to the actual (PID-qualified) temp directory.
pub fn expand_java_args(raw: &str, inline_java_cp: &str) -> Vec<String> {
	if raw.is_empty() {
		return Vec::new();
	}
	let cp = inline_java_cp.to_owned();
	let expanded = shellexpand::full_with_context_no_errors(
		raw,
		|| std::env::var("HOME").ok(),
		move |var| match var {
			"INLINE_JAVA_CP" => Some(cp.clone()),
			other => std::env::var(other).ok(),
		},
	);
	split_args(&expanded)
}

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

/// All errors that `java!` can return at runtime.
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
