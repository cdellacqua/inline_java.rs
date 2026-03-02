// inline_java/src/lib.rs
//
// Two proc macros for embedding Java in Rust, inspired by `inline_python` /
// `ct_python`.
//
// ┌─────────────┬──────────┬───────────────┐
// │             │ runtime  │ compile-time  │
// ├─────────────┼──────────┼───────────────┤
// │ input       │ java!    │ ct_java!      │
// └─────────────┴──────────┴───────────────┘
//
// Both macros share the same Java-side API: the user writes a
// `public static <T> run()` method where T must be one of:
//   byte, short, int, long, float, double, boolean, char, String
//
// The macro generates a `main()` that binary-serialises the return value of
// `run()` to stdout (via DataOutputStream / raw UTF-8 for String), reads those
// bytes, and produces the corresponding Rust value.  This avoids both text
// parsing and the need to manually quote strings for ct_java!.
//
// Optional key-value options
// ──────────────────────────
// Both macros accept zero or more `key = "value"` pairs before the Java body,
// separated by commas.  Recognised keys:
//
//   javac = "<args>"   extra command-line arguments passed verbatim to javac,
//                      shell-quoted (single/double quotes respected).
//   java  = "<args>"   extra command-line arguments passed verbatim to java,
//                      shell-quoted (single/double quotes respected).
//
// java!
// ────────────
// Runs Java at *program runtime*.  The user provides a `run()` method; the
// macro wraps it in a class, compiles it with `javac`, and runs it with
// `java`.  The binary-encoded return value is decoded into the inferred Rust
// type at the call site.
//
// Expands to `Result<T, inline_java::JavaError>`, so callers can propagate
// errors with `?` or surface them with `.unwrap()`.
//
// Rust variables can be injected using `'var` syntax (same convention as
// inline_python).  Each `'var` becomes the Java String `_RUST_var`, passed
// via args[].
//
//   let n = 42i32;
//   let s: String = java! {
//       public static String run() {
//           int x = Integer.parseInt('n);
//           return "double is " + (x * 2);
//       }
//   }.unwrap();
//
// ct_java!
// ────────
// Runs Java at *compile time* (inside the proc-macro, while rustc is
// expanding macros).  The return value of `run()` is binary-deserialised and
// spliced as a Rust literal at the call site (e.g. 42, 3.14, true, 'x',
// "hello").
//
//   const PI_APPROX: f64 = ct_java! {
//       public static double run() {
//           return Math.PI;
//       }
//   };
//
//   const GREETING: &str = ct_java! {
//       public static String run() {
//           return "Hello, World!";
//       }
//   };

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Spacing, TokenTree};
use quote::quote;

// ---------------------------------------------------------------------------
// JavaType — allowed return types for run(), with serialisation/deserialisation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum JavaType {
	Byte,
	Short,
	Int,
	Long,
	Float,
	Double,
	Boolean,
	Char,
	JavaString,
}

impl JavaType {
	fn from_name(s: &str) -> Option<Self> {
		match s {
			"byte" => Some(Self::Byte),
			"short" => Some(Self::Short),
			"int" => Some(Self::Int),
			"long" => Some(Self::Long),
			"float" => Some(Self::Float),
			"double" => Some(Self::Double),
			"boolean" => Some(Self::Boolean),
			"char" => Some(Self::Char),
			"String" => Some(Self::JavaString),
			_ => None,
		}
	}

	/// Generates the complete `main(String[] args)` method that binary-serialises
	/// `run()`'s return value to stdout.  `var_inits` is pre-formatted code that
	/// assigns `_RUST_*` static fields from `args[]` (empty for `ct_java!`).
	fn java_main(self, var_inits: &str) -> String {
		let serialize = if self == Self::JavaString {
			"byte[] _b = run().getBytes(java.nio.charset.StandardCharsets.UTF_8);\n\
  				 \t\tSystem.out.write(_b);\n\
  				 \t\tSystem.out.flush();"
				.to_string()
		} else {
			let method = match self {
				Self::Byte => "writeByte",
				Self::Short => "writeShort",
				Self::Int => "writeInt",
				Self::Long => "writeLong",
				Self::Float => "writeFloat",
				Self::Double => "writeDouble",
				Self::Boolean => "writeBoolean",
				Self::Char => "writeChar",
				Self::JavaString => unreachable!(),
			};
			format!(
				"java.io.DataOutputStream _dos = \
  					 new java.io.DataOutputStream(System.out);\n\
  					 \t\t_dos.{method}(run());\n\
  					 \t\t_dos.flush();"
			)
		};
		format!(
			"\tpublic static void main(String[] args) throws Exception {{\n\
			 {var_inits}\t\t{serialize}\n\
			 \t}}"
		)
	}

	/// Returns a Rust expression (as a token stream) that deserialises the raw
	/// stdout bytes `_raw: Vec<u8>` into the corresponding Rust type.
	/// Used by `java!` at program runtime.
	fn rust_deser(self) -> proc_macro2::TokenStream {
		match self {
			Self::Byte => quote! { i8::from_be_bytes([_raw[0]]) },
			Self::Short => quote! { i16::from_be_bytes([_raw[0], _raw[1]]) },
			Self::Int => {
				quote! { i32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]]) }
			}
			Self::Long => {
				quote! {
					i64::from_be_bytes([
						_raw[0], _raw[1], _raw[2], _raw[3],
						_raw[4], _raw[5], _raw[6], _raw[7],
					])
				}
			}
			Self::Float => {
				quote! { f32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]]) }
			}
			Self::Double => {
				quote! {
					f64::from_be_bytes([
						_raw[0], _raw[1], _raw[2], _raw[3],
						_raw[4], _raw[5], _raw[6], _raw[7],
					])
				}
			}
			Self::Boolean => quote! { _raw[0] != 0 },
			Self::Char => {
				quote! {
					::std::char::from_u32(u16::from_be_bytes([_raw[0], _raw[1]]) as u32)
						.ok_or(::inline_java::JavaError::InvalidChar)?
				}
			}
			Self::JavaString => {
				quote! {
					::std::string::String::from_utf8(_raw)?
				}
			}
		}
	}

	/// Converts the raw stdout bytes produced by the generated `main()` into a
	/// Rust literal token stream to splice at the `ct_java!` call site.
	fn ct_java_tokens(self, bytes: Vec<u8>) -> Result<proc_macro2::TokenStream, String> {
		let lit = match self {
			Self::Byte => format!("{}", i8::from_be_bytes([bytes[0]])),
			Self::Short => format!("{}", i16::from_be_bytes([bytes[0], bytes[1]])),
			Self::Int => {
				let arr: [u8; 4] = bytes[..4]
					.try_into()
					.map_err(|_| "ct_java: truncated output for int")?;
				format!("{}", i32::from_be_bytes(arr))
			}
			Self::Long => {
				let arr: [u8; 8] = bytes[..8]
					.try_into()
					.map_err(|_| "ct_java: truncated output for long")?;
				format!("{}", i64::from_be_bytes(arr))
			}
			// For floats use from_bits so all values (including NaN/±Infinity) round-trip
			// correctly, and because f32::from_bits / f64::from_bits are const fn.
			Self::Float => {
				let arr: [u8; 4] = bytes[..4]
					.try_into()
					.map_err(|_| "ct_java: truncated output for float")?;
				let bits = u32::from_be_bytes(arr);
				format!("f32::from_bits(0x{bits:08x}_u32)")
			}
			Self::Double => {
				let arr: [u8; 8] = bytes[..8]
					.try_into()
					.map_err(|_| "ct_java: truncated output for double")?;
				let bits = u64::from_be_bytes(arr);
				format!("f64::from_bits(0x{bits:016x}_u64)")
			}
			Self::Boolean => {
				if bytes[0] != 0 {
					"true".to_string()
				} else {
					"false".to_string()
				}
			}
			Self::Char => {
				let code_unit = u16::from_be_bytes([bytes[0], bytes[1]]);
				let c = char::from_u32(u32::from(code_unit))
					.ok_or("ct_java: Java char is not a valid Unicode scalar value")?;
				format!("{c:?}") // Rust char literal: 'A', '\n', '\u{1f600}', …
			}
			Self::JavaString => {
				let s = String::from_utf8(bytes)
					.map_err(|_| "ct_java: Java String is not valid UTF-8".to_string())?;
				format!("{s:?}") // Rust string literal: "hello", "line\none", …
			}
		};
		proc_macro2::TokenStream::from_str(&lit)
			.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
	}
}

/// Scan the body token stream for the first `public static <type> run` pattern
/// and return the corresponding `JavaType`.  Emits a clear error for unsupported
/// return types and if no `run()` method is found at all.
fn parse_run_return_type(body: &proc_macro2::TokenStream) -> Result<JavaType, String> {
	let tts: Vec<TokenTree> = body.clone().into_iter().collect();
	for i in 0..tts.len().saturating_sub(3) {
		if !matches!(&tts[i],   TokenTree::Ident(id) if id == "public") {
			continue;
		}
		if !matches!(&tts[i+1], TokenTree::Ident(id) if id == "static") {
			continue;
		}
		let type_name = match &tts[i + 2] {
			TokenTree::Ident(id) => id.to_string(),
			_ => continue,
		};
		if !matches!(&tts[i+3], TokenTree::Ident(id) if id == "run") {
			continue;
		}
		return JavaType::from_name(&type_name).ok_or_else(|| {
			format!(
				"inline_java: `run()` return type `{type_name}` is not supported; \
				 allowed: byte, short, int, long, float, double, boolean, char, String"
			)
		});
	}
	Err("inline_java: could not find `public static <type> run()` in Java body".to_string())
}

#[proc_macro]
pub fn java(input: TokenStream) -> TokenStream {
	let input2 = proc_macro2::TokenStream::from(input);

	// Consume any leading `key = "value",` option pairs.
	let (opts, input2) = extract_opts(input2);

	// Replace 'var tokens with _RUST_var idents and collect the variable names.
	let (substituted, vars) = extract_vars(input2);

	// Split the substituted stream into import/package statements and the method body.
	let (imports_ts, body_ts) = split_imports(substituted);
	let imports = imports_ts.to_string();
	let body = body_ts.to_string();

	// Parse the return type of run() to drive serialisation/deserialisation.
	let java_type = match parse_run_return_type(&body_ts) {
		Ok(t) => t,
		Err(msg) => return quote! { compile_error!(#msg) }.into(),
	};

	// Unique class name derived from the source content.
	let mut h = DefaultHasher::new();
	imports.hash(&mut h);
	body.hash(&mut h);
	let class_name = format!("InlineJava_{:016x}", h.finish());
	let filename = format!("{class_name}.java");

	// If the user wrote a `package` declaration, the class must be run by its
	// fully-qualified name (e.g. `com.example.demo.InlineJava_xxx`).
	let package_name = parse_package_name(&imports);
	let full_class_name = match &package_name {
		Some(pkg) => format!("{pkg}.{class_name}"),
		None => class_name.clone(),
	};

	// `static String _RUST_foo;` declarations, one per captured variable.
	let var_fields: String = vars.keys().fold(String::new(), |mut s, name| {
		writeln!(s, "\tstatic String _RUST_{name};").unwrap();
		s
	});

	// Assignments inside main: `_RUST_foo = args[0];` in alphabetical order.
	let var_inits: String = vars
		.keys()
		.enumerate()
		.fold(String::new(), |mut s, (i, name)| {
			writeln!(s, "\t\t_RUST_{name} = args[{i}];").unwrap();
			s
		});

	let main_method = java_type.java_main(&var_inits);
	let java_class = format!(
		"{imports}\npublic class {class_name} {{\n{var_fields}\n{body}\n\n{main_method}\n}}\n"
	);

	let java_compiler_extra: Vec<String> = opts
		.javac_args
		.map(|a| split_args(&shellexpand::full(&a).map(std::borrow::Cow::into_owned).unwrap_or(a)))
		.unwrap_or_default();
	let java_runtime_extra: Vec<String> = opts
		.java_args
		.map(|a| split_args(&shellexpand::full(&a).map(std::borrow::Cow::into_owned).unwrap_or(a)))
		.unwrap_or_default();

	let var_idents: Vec<Ident> = vars.values().cloned().collect();
	let deser = java_type.rust_deser();

	let generated = quote! {
		(|| -> ::std::result::Result<_, ::inline_java::JavaError> {
			// Deterministic temp dir keyed by the class name (= hash of source).
			let _tmp_dir = ::std::env::temp_dir().join(#class_name);
			::std::fs::create_dir_all(&_tmp_dir)
				.map_err(|e| ::inline_java::JavaError::Io(e.to_string()))?;

			let _src = _tmp_dir.join(#filename);
			::std::fs::write(&_src, #java_class)
				.map_err(|e| ::inline_java::JavaError::Io(e.to_string()))?;

			// Compile phase.
			let _javac = ::std::process::Command::new("javac")
				#(.arg(#java_compiler_extra))*
				.arg("-d").arg(&_tmp_dir)
				.arg(&_src)
				.output()
				.map_err(|e| ::inline_java::JavaError::Io(e.to_string()))?;
			if !_javac.status.success() {
				return Err(::inline_java::JavaError::CompilationFailed(
					::std::string::String::from_utf8_lossy(&_javac.stderr).into_owned()
				));
			}

			// Run phase.
			let _java = ::std::process::Command::new("java")
				#(.arg(#java_runtime_extra))*
				.arg("-cp").arg(&_tmp_dir)
				.arg(#full_class_name)
				#(.arg(::std::string::ToString::to_string(&#var_idents)))*
				.output()
				.map_err(|e| ::inline_java::JavaError::Io(e.to_string()))?;
			if !_java.status.success() {
				return Err(::inline_java::JavaError::RuntimeFailed(
					::std::string::String::from_utf8_lossy(&_java.stderr).into_owned()
				));
			}

			let _raw = _java.stdout;
			::std::result::Result::Ok(#deser)
		})()
	};

	generated.into()
}

// ---------------------------------------------------------------------------
// ct_java! — compile-time Java evaluation
// ---------------------------------------------------------------------------

/// Run Java at *compile time* and splice its return value as a Rust literal.
///
/// Accepts optional `javac = "..."` and `java = "..."` key-value pairs before
/// the Java body.  The user provides a `public static <T> run()` method; its
/// binary-serialised return value is decoded and emitted as a Rust literal at
/// the call site (`42`, `3.14`, `true`, `'x'`, `"hello"`, …).
///
/// Java compilation/runtime errors become Rust `compile_error!` diagnostics.
#[proc_macro]
pub fn ct_java(input: TokenStream) -> TokenStream {
	match ct_java_impl(proc_macro2::TokenStream::from(input)) {
		Ok(ts) => ts.into(),
		Err(msg) => quote! { compile_error!(#msg) }.into(),
	}
}

fn ct_java_impl(input: proc_macro2::TokenStream) -> Result<proc_macro2::TokenStream, String> {
	let (opts, input) = extract_opts(input);

	let (imports_ts, body_ts) = split_imports(input);
	let imports = imports_ts.to_string();
	let body = body_ts.to_string();

	let java_type = parse_run_return_type(&body_ts)?;

	let mut h = DefaultHasher::new();
	imports.hash(&mut h);
	body.hash(&mut h);
	let class_name = format!("CtJava_{:016x}", h.finish());
	let filename = format!("{class_name}.java");

	let package_name = parse_package_name(&imports);
	let full_class_name = match &package_name {
		Some(pkg) => format!("{pkg}.{class_name}"),
		None => class_name.clone(),
	};

	let main_method = java_type.java_main("");
	let java_class =
		format!("{imports}\npublic class {class_name} {{\n{body}\n\n{main_method}\n}}\n");

	let tmp_dir = std::env::temp_dir().join(&class_name);
	std::fs::create_dir_all(&tmp_dir)
		.map_err(|e| format!("ct_java: failed to create temp dir: {e}"))?;

	let src = tmp_dir.join(&filename);
	std::fs::write(&src, &java_class)
		.map_err(|e| format!("ct_java: failed to write .java source: {e}"))?;

	let java_compiler_extra: Vec<String> = opts
		.javac_args
		.map(|a| split_args(&shellexpand::full(&a).map(std::borrow::Cow::into_owned).unwrap_or(a)))
		.unwrap_or_default();
	let java_runtime_extra: Vec<String> = opts
		.java_args
		.map(|a| split_args(&shellexpand::full(&a).map(std::borrow::Cow::into_owned).unwrap_or(a)))
		.unwrap_or_default();

	// Compile phase.
	let mut java_compiler_cmd = std::process::Command::new("javac");
	for arg in &java_compiler_extra {
		java_compiler_cmd.arg(arg);
	}
	let java_compiler_output = java_compiler_cmd
		.arg("-d")
		.arg(&tmp_dir)
		.arg(&src)
		.output()
		.map_err(|e| format!("ct_java: could not invoke javac (is it on PATH?): {e}"))?;
	if !java_compiler_output.status.success() {
		return Err(format!(
			"ct_java: javac failed:\n{}",
			String::from_utf8_lossy(&java_compiler_output.stderr)
		));
	}

	// Run phase.
	let mut java_cmd = std::process::Command::new("java");
	for arg in &java_runtime_extra {
		java_cmd.arg(arg);
	}
	let java_output = java_cmd
		.arg("-cp")
		.arg(&tmp_dir)
		.arg(&full_class_name)
		.output()
		.map_err(|e| format!("ct_java: could not invoke java (is it on PATH?): {e}"))?;
	if !java_output.status.success() {
		return Err(format!(
			"ct_java: java failed:\n{}",
			String::from_utf8_lossy(&java_output.stderr)
		));
	}

	java_type.ct_java_tokens(java_output.stdout)
}

// ---------------------------------------------------------------------------
// Option extraction: `javac = "…"` / `java = "…"` before the Java body
// ---------------------------------------------------------------------------

struct JavaOpts {
	/// Extra args for `javac`, shell-split at use-site.  `None` → no extra args.
	javac_args: Option<String>,
	/// Extra args for `java`, shell-split at use-site.  `None` → no extra args.
	java_args: Option<String>,
}

/// Split a shell-style argument string into individual arguments, respecting
/// single- and double-quoted spans.  Quotes are stripped; content inside them
/// is treated as a single token even if it contains whitespace.
///
/// Examples:
/// ```text
///   "-sourcepath /tmp"     → ["-sourcepath", "/tmp"]
///   "-Dprop='hello world'" → ["-Dprop=hello world"]
///   "-Da=\"x y\" -Db=z"    → ["-Da=x y", "-Db=z"]
/// ```
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

/// Consume leading `javac = "…"` / `java = "…"` option pairs (comma-separated,
/// trailing comma optional) and return the remaining token stream as the Java
/// body.  Unrecognised leading tokens are left untouched.
fn extract_opts(input: proc_macro2::TokenStream) -> (JavaOpts, proc_macro2::TokenStream) {
	let mut tts: Vec<TokenTree> = input.into_iter().collect();
	let mut opts = JavaOpts {
		javac_args: None,
		java_args: None,
	};
	let mut cursor = 0;

	loop {
		match try_parse_opt(&tts[cursor..]) {
			None => break,
			Some((key, val, consumed)) => {
				match key.as_str() {
					"javac" => opts.javac_args = Some(val),
					"java" => opts.java_args = Some(val),
					_ => break,
				}
				cursor += consumed;
				if let Some(TokenTree::Punct(p)) = tts.get(cursor)
					&& p.as_char() == ','
				{
					cursor += 1;
				}
			}
		}
	}

	let rest = tts.drain(cursor..).collect();
	(opts, rest)
}

/// Try to parse `Ident("javac"|"java") Punct("=") Literal(string)` at the
/// start of `tts`.  Returns `(key, unquoted_value, tokens_consumed)` or
/// `None` if the pattern doesn't match.
fn try_parse_opt(tts: &[TokenTree]) -> Option<(String, String, usize)> {
	let key = match tts.first() {
		Some(TokenTree::Ident(id)) => id.to_string(),
		_ => return None,
	};
	let Some(TokenTree::Punct(eq)) = tts.get(1) else {
		return None;
	};
	if eq.as_char() != '=' {
		return None;
	}
	let val_str = match tts.get(2) {
		Some(TokenTree::Literal(lit)) => lit.to_string(),
		_ => return None,
	};
	if val_str.starts_with('"') && val_str.ends_with('"') && val_str.len() >= 2 {
		Some((key, val_str[1..val_str.len() - 1].to_string(), 3))
	} else {
		None
	}
}

// ---------------------------------------------------------------------------
// Package name extraction
// ---------------------------------------------------------------------------

/// Extract the package name from the string representation of the imports
/// token stream.  `proc_macro2` serialises `package com.example.demo;` as a
/// compact string (dots and semicolon not separated by spaces), so we use
/// substring search rather than splitting on whitespace.
fn parse_package_name(imports: &str) -> Option<String> {
	let marker = "package ";
	let i = imports.find(marker)?;
	if i > 0 && !imports[..i].ends_with(|c: char| c.is_whitespace()) {
		return None;
	}
	let rest = imports[i + marker.len()..].trim_start();
	let semi = rest.find(';')?;
	let pkg = rest[..semi].trim().replace(|c: char| c.is_whitespace(), "");
	if pkg.is_empty() { None } else { Some(pkg) }
}

// ---------------------------------------------------------------------------
// Variable extraction: replace `'var` with `_RUST_var`
// ---------------------------------------------------------------------------

/// Walk the token stream and replace every `'var` occurrence (a `'` Punct
/// with `Joint` spacing immediately followed by an Ident) with a single Ident
/// `_RUST_var`.  Recurse into groups.  Returns the substituted stream and a
/// `BTreeMap` of variable names → their first-occurrence Ident (for spans).
fn extract_vars(
	input: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, BTreeMap<String, Ident>) {
	let mut vars: BTreeMap<String, Ident> = BTreeMap::new();
	let mut output: Vec<TokenTree> = Vec::new();
	let mut iter = input.into_iter().peekable();

	while let Some(tt) = iter.next() {
		let is_quote_punct = matches!(
			&tt,
			TokenTree::Punct(p) if p.as_char() == '\'' && p.spacing() == Spacing::Joint
		);

		if is_quote_punct {
			if matches!(iter.peek(), Some(TokenTree::Ident(_))) {
				let TokenTree::Ident(ident) = iter.next().unwrap() else {
					unreachable!()
				};
				let name = ident.to_string();
				let span = ident.span();
				vars.entry(name.clone()).or_insert_with(|| ident);
				output.push(TokenTree::Ident(Ident::new(&format!("_RUST_{name}"), span)));
			} else {
				output.push(tt);
			}
		} else {
			match tt {
				TokenTree::Group(g) => {
					let (inner, inner_vars) = extract_vars(g.stream());
					for (k, v) in inner_vars {
						vars.entry(k).or_insert(v);
					}
					let mut new_group = proc_macro2::Group::new(g.delimiter(), inner);
					new_group.set_span(g.span());
					output.push(TokenTree::Group(new_group));
				}
				other => output.push(other),
			}
		}
	}

	(output.into_iter().collect(), vars)
}

// ---------------------------------------------------------------------------
// Token-level import / body split
// ---------------------------------------------------------------------------

/// Partition the token stream into (`import_statements`, rest).
/// Detects `import …;` and `package …;` at the top level.
fn split_imports(
	input: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
	let mut iter = input.into_iter();
	let mut imports: Vec<TokenTree> = Vec::new();
	let mut body: Vec<TokenTree> = Vec::new();

	while let Some(tt) = iter.next() {
		let is_directive = matches!(
			&tt,
			TokenTree::Ident(id) if *id == "import" || *id == "package"
		);

		if is_directive {
			imports.push(tt);
			for tt in iter.by_ref() {
				let is_semi = matches!(&tt, TokenTree::Punct(p) if p.as_char() == ';');
				imports.push(tt);
				if is_semi {
					break;
				}
			}
		} else {
			body.push(tt);
		}
	}

	(imports.into_iter().collect(), body.into_iter().collect())
}
