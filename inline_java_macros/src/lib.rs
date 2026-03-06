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
//   T[]  (array of any of the above)
//   List<BoxedT> (java.util.List of the boxed equivalent)
//
// The macro generates a `main()` that binary-serialises the return value of
// `run()` to stdout (via DataOutputStream / raw UTF-8 for String), reads those
// bytes, and produces the corresponding Rust value.  This avoids both text
// parsing and the need to manually quote strings for ct_java!.
//
// Arrays and Lists use a length-prefixed wire format:
//   4 bytes big-endian int32: number of elements
//   for each element:
//     - fixed-size primitives: serialized with DataOutputStream (same as scalar)
//     - String: 4-byte int32 length + UTF-8 bytes
//
// Optional key-value options
// Both macros accept zero or more `key = "value"` pairs before the Java body,
// separated by commas.  Recognised keys:
//
//   javac = "<args>"   extra command-line arguments passed verbatim to javac,
//                      shell-quoted (single/double quotes respected).
//   java  = "<args>"   extra command-line arguments passed verbatim to java,
//                      shell-quoted (single/double quotes respected).
//
// java!
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
// Runs Java at *compile time* (inside the proc-macro, while rustc is
// expanding macros).  The return value of `run()` is binary-deserialised and
// spliced as a Rust literal at the call site (e.g. 42, 3.14, true, 'x',
// "hello", [1, 2, 3]).
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
//
//   const PRIMES: [i32; 5] = ct_java! {
//       public static int[] run() {
//           return new int[]{2, 3, 5, 7, 11};
//       }
//   };

use proc_macro::TokenStream;
use proc_macro2::{Ident, LineColumn, Spacing, TokenTree};
use quote::quote;
use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

// ScalarType — the nine primitive / String base types

#[derive(Clone, Copy, PartialEq)]
enum ScalarType {
	Byte,
	Short,
	Int,
	Long,
	Float,
	Double,
	Boolean,
	Char,
	Str,
}

impl ScalarType {
	/// Parse a Java primitive type name or "String".
	fn from_primitive_name(s: &str) -> Option<Self> {
		match s {
			"byte" => Some(Self::Byte),
			"short" => Some(Self::Short),
			"int" => Some(Self::Int),
			"long" => Some(Self::Long),
			"float" => Some(Self::Float),
			"double" => Some(Self::Double),
			"boolean" => Some(Self::Boolean),
			"char" => Some(Self::Char),
			"String" => Some(Self::Str),
			_ => None,
		}
	}

	/// Parse a Java boxed type name (used in `List<T>`).
	fn from_boxed_name(s: &str) -> Option<Self> {
		match s {
			"Byte" => Some(Self::Byte),
			"Short" => Some(Self::Short),
			"Integer" => Some(Self::Int),
			"Long" => Some(Self::Long),
			"Float" => Some(Self::Float),
			"Double" => Some(Self::Double),
			"Boolean" => Some(Self::Boolean),
			"Character" => Some(Self::Char),
			"String" => Some(Self::Str),
			_ => None,
		}
	}

	/// Java primitive / String type name used in `T[]` declarations.
	fn java_prim_name(self) -> &'static str {
		match self {
			Self::Byte => "byte",
			Self::Short => "short",
			Self::Int => "int",
			Self::Long => "long",
			Self::Float => "float",
			Self::Double => "double",
			Self::Boolean => "boolean",
			Self::Char => "char",
			Self::Str => "String",
		}
	}

	/// Java boxed type name used in `List<T>` declarations.
	fn java_boxed_name(self) -> &'static str {
		match self {
			Self::Byte => "Byte",
			Self::Short => "Short",
			Self::Int => "Integer",
			Self::Long => "Long",
			Self::Float => "Float",
			Self::Double => "Double",
			Self::Boolean => "Boolean",
			Self::Char => "Character",
			Self::Str => "String",
		}
	}

	/// `DataOutputStream` write method name; `None` for String (special case).
	fn dos_write_method(self) -> Option<&'static str> {
		match self {
			Self::Byte => Some("writeByte"),
			Self::Short => Some("writeShort"),
			Self::Int => Some("writeInt"),
			Self::Long => Some("writeLong"),
			Self::Float => Some("writeFloat"),
			Self::Double => Some("writeDouble"),
			Self::Boolean => Some("writeBoolean"),
			Self::Char => Some("writeChar"),
			Self::Str => None,
		}
	}

	/// Rust type token stream corresponding to this scalar.
	fn rust_type_ts(self) -> proc_macro2::TokenStream {
		match self {
			Self::Byte => quote! { i8 },
			Self::Short => quote! { i16 },
			Self::Int => quote! { i32 },
			Self::Long => quote! { i64 },
			Self::Float => quote! { f32 },
			Self::Double => quote! { f64 },
			Self::Boolean => quote! { bool },
			Self::Char => quote! { char },
			Self::Str => quote! { ::std::string::String },
		}
	}

	/// Generates code for the body of the element-deserialization loop inside
	/// `rust_deser_array`.  Produces a statement that pushes one element onto
	/// `_v` and advances `_cur` by the element's byte width.
	fn rust_deser_one_ts(self) -> proc_macro2::TokenStream {
		match self {
			Self::Byte => quote! {
				_v.push(i8::from_be_bytes([_raw[_cur]]));
				_cur += 1;
			},
			Self::Short => quote! {
				_v.push(i16::from_be_bytes([_raw[_cur], _raw[_cur + 1]]));
				_cur += 2;
			},
			Self::Int => quote! {
				_v.push(i32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]));
				_cur += 4;
			},
			Self::Long => quote! {
				_v.push(i64::from_be_bytes([
					_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3],
					_raw[_cur + 4], _raw[_cur + 5], _raw[_cur + 6], _raw[_cur + 7],
				]));
				_cur += 8;
			},
			Self::Float => quote! {
				_v.push(f32::from_bits(u32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]])));
				_cur += 4;
			},
			Self::Double => quote! {
				_v.push(f64::from_bits(u64::from_be_bytes([
					_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3],
					_raw[_cur + 4], _raw[_cur + 5], _raw[_cur + 6], _raw[_cur + 7],
				])));
				_cur += 8;
			},
			Self::Boolean => quote! {
				_v.push(_raw[_cur] != 0);
				_cur += 1;
			},
			Self::Char => quote! {
				_v.push(
					::std::char::from_u32(u16::from_be_bytes([_raw[_cur], _raw[_cur + 1]]) as u32)
						.ok_or(::inline_java::JavaError::InvalidChar)?
				);
				_cur += 2;
			},
			Self::Str => quote! {
				let _slen = i32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]) as usize;
				_cur += 4;
				_v.push(::std::string::String::from_utf8(_raw[_cur.._cur + _slen].to_vec())?);
				_cur += _slen;
			},
		}
	}
}

// JavaType — allowed return types for run(), with serialisation/deserialisation

#[derive(Clone, Copy, PartialEq)]
enum JavaType {
	Scalar(ScalarType),
	/// Java `T[]` — returned as `Vec<T>` at runtime, `[T; N]` at compile time.
	Array(ScalarType),
	/// Java `List<BoxedT>` — same wire format / Rust type as `Array`.
	List(ScalarType),
}

impl JavaType {
	/// Generates the complete `main(String[] args)` method that binary-serialises
	/// `run()`'s return value to stdout.  `var_inits` is pre-formatted code that
	/// assigns `_RUST_*` static fields from `args[]` (empty for `ct_java!`).
	fn java_main(self, var_inits: &str) -> String {
		match self {
			Self::Scalar(s) => {
				let serialize = if s == ScalarType::Str {
					"byte[] _b = run().getBytes(java.nio.charset.StandardCharsets.UTF_8);\n\
  				 \t\tSystem.out.write(_b);\n\
  				 \t\tSystem.out.flush();"
						.to_string()
				} else {
					let method = s.dos_write_method().unwrap();
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
			Self::Array(s) => {
				let prim = s.java_prim_name();
				let loop_body = array_serialize_loop(s, prim);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {var_inits}\t\t{prim}[] _arr = run();\n\
					 \t\tjava.io.DataOutputStream _dos = new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.writeInt(_arr.length);\n\
					 \t\t{loop_body}\n\
					 \t\t_dos.flush();\n\
					 \t}}"
				)
			}
			Self::List(s) => {
				let boxed = s.java_boxed_name();
				let iter_type = if s == ScalarType::Str {
					"String"
				} else {
					boxed
				};
				let loop_body = array_serialize_loop(s, iter_type);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {var_inits}\t\tjava.util.List<{boxed}> _arr = run();\n\
					 \t\tjava.io.DataOutputStream _dos = new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.writeInt(_arr.size());\n\
					 \t\t{loop_body}\n\
					 \t\t_dos.flush();\n\
					 \t}}"
				)
			}
		}
	}

	/// Returns a Rust expression (as a token stream) that deserialises the raw
	/// stdout bytes `_raw: Vec<u8>` into the corresponding Rust type.
	/// Used by `java!` at program runtime.
	fn rust_deser(self) -> proc_macro2::TokenStream {
		match self {
			Self::Scalar(s) => match s {
				ScalarType::Byte => quote! { i8::from_be_bytes([_raw[0]]) },
				ScalarType::Short => quote! { i16::from_be_bytes([_raw[0], _raw[1]]) },
				ScalarType::Int => {
					quote! { i32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]]) }
				}
				ScalarType::Long => {
					quote! {
						i64::from_be_bytes([
							_raw[0], _raw[1], _raw[2], _raw[3],
							_raw[4], _raw[5], _raw[6], _raw[7],
						])
					}
				}
				ScalarType::Float => {
					quote! { f32::from_bits(u32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]])) }
				}
				ScalarType::Double => {
					quote! {
						f64::from_bits(u64::from_be_bytes([
							_raw[0], _raw[1], _raw[2], _raw[3],
							_raw[4], _raw[5], _raw[6], _raw[7],
						]))
					}
				}
				ScalarType::Boolean => quote! { _raw[0] != 0 },
				ScalarType::Char => {
					quote! {
						::std::char::from_u32(u16::from_be_bytes([_raw[0], _raw[1]]) as u32)
							.ok_or(::inline_java::JavaError::InvalidChar)?
					}
				}
				ScalarType::Str => {
					quote! {
						::std::string::String::from_utf8(_raw)?
					}
				}
			},
			Self::Array(s) | Self::List(s) => {
				let rust_type = s.rust_type_ts();
				let deser_one = s.rust_deser_one_ts();
				quote! {
					{
						let _n = i32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]]) as usize;
						let mut _cur = 4usize;
						let mut _v: Vec<#rust_type> = ::std::vec::Vec::with_capacity(_n);
						for _ in 0.._n {
							#deser_one
						}
						_v
					}
				}
			}
		}
	}

	/// Converts the raw stdout bytes produced by the generated `main()` into a
	/// Rust literal / expression token stream to splice at the `ct_java!` call site.
	/// Scalars produce literals (42, 3.14, true, 'x', "hello").
	/// Arrays/Lists produce array expressions ([e0, e1, e2]).
	fn ct_java_tokens(self, bytes: Vec<u8>) -> Result<proc_macro2::TokenStream, String> {
		match self {
			Self::Scalar(s) => {
				// Scalar String is serialised as raw UTF-8 (no length prefix) — special case.
				let lit = if s == ScalarType::Str {
					let s = String::from_utf8(bytes)
						.map_err(|_| "ct_java: Java String is not valid UTF-8".to_string())?;
					format!("{s:?}")
				} else {
					let (l, _) = scalar_ct_lit(s, &bytes)?;
					l
				};
				proc_macro2::TokenStream::from_str(&lit)
					.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
			}
			Self::Array(s) | Self::List(s) => {
				if bytes.len() < 4 {
					return Err("ct_java: array output too short (missing length)".to_string());
				}
				#[allow(clippy::cast_sign_loss)]
				let n = i32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
				let mut cur = 4;
				let mut lits: Vec<String> = Vec::with_capacity(n);
				for _ in 0..n {
					let (lit, consumed) = scalar_ct_lit(s, &bytes[cur..])?;
					lits.push(lit);
					cur += consumed;
				}
				let array_expr = format!("[{}]", lits.join(", "));
				proc_macro2::TokenStream::from_str(&array_expr)
					.map_err(|e| format!("ct_java: produced invalid Rust tokens: {e}"))
			}
		}
	}
}

// scalar_ct_lit — convert raw bytes to a Rust literal string for one element

/// Deserialise one element of type `s` from `bytes` and return a
/// `(rust_literal_string, bytes_consumed)` pair for use in `ct_java_tokens`.
fn scalar_ct_lit(s: ScalarType, bytes: &[u8]) -> Result<(String, usize), String> {
	match s {
		ScalarType::Byte => {
			if bytes.is_empty() {
				return Err("ct_java: truncated byte element".to_string());
			}
			Ok((format!("{}", i8::from_be_bytes([bytes[0]])), 1))
		}
		ScalarType::Short => {
			if bytes.len() < 2 {
				return Err("ct_java: truncated short element".to_string());
			}
			Ok((format!("{}", i16::from_be_bytes([bytes[0], bytes[1]])), 2))
		}
		ScalarType::Int => {
			let arr: [u8; 4] = bytes[..4]
				.try_into()
				.map_err(|_| "ct_java: truncated int element")?;
			Ok((format!("{}", i32::from_be_bytes(arr)), 4))
		}
		ScalarType::Long => {
			let arr: [u8; 8] = bytes[..8]
				.try_into()
				.map_err(|_| "ct_java: truncated long element")?;
			Ok((format!("{}", i64::from_be_bytes(arr)), 8))
		}
		ScalarType::Float => {
			let arr: [u8; 4] = bytes[..4]
				.try_into()
				.map_err(|_| "ct_java: truncated float element")?;
			let bits = u32::from_be_bytes(arr);
			Ok((format!("f32::from_bits(0x{bits:08x}_u32)"), 4))
		}
		ScalarType::Double => {
			let arr: [u8; 8] = bytes[..8]
				.try_into()
				.map_err(|_| "ct_java: truncated double element")?;
			let bits = u64::from_be_bytes(arr);
			Ok((format!("f64::from_bits(0x{bits:016x}_u64)"), 8))
		}
		ScalarType::Boolean => {
			if bytes.is_empty() {
				return Err("ct_java: truncated boolean element".to_string());
			}
			Ok((
				if bytes[0] != 0 {
					"true".to_string()
				} else {
					"false".to_string()
				},
				1,
			))
		}
		ScalarType::Char => {
			if bytes.len() < 2 {
				return Err("ct_java: truncated char element".to_string());
			}
			let code_unit = u16::from_be_bytes([bytes[0], bytes[1]]);
			let c = char::from_u32(u32::from(code_unit))
				.ok_or("ct_java: Java char is not a valid Unicode scalar value")?;
			Ok((format!("{c:?}"), 2))
		}
		ScalarType::Str => {
			if bytes.len() < 4 {
				return Err("ct_java: truncated String length prefix".to_string());
			}
			#[allow(clippy::cast_sign_loss)]
			let len = i32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
			if bytes.len() < 4 + len {
				return Err(format!(
					"ct_java: truncated String element (expected {len} bytes)"
				));
			}
			let s = String::from_utf8(bytes[4..4 + len].to_vec())
				.map_err(|_| "ct_java: String element is not valid UTF-8".to_string())?;
			Ok((format!("{s:?}"), 4 + len))
		}
	}
}

// array_serialize_loop — Java loop body for array/List serialisation

/// Returns the Java `for` loop that serialises the elements of `_arr` using
/// `_dos`.  `iter_type` is the element type used in the `for` declaration
/// (primitive for T[], boxed for List<T>).
fn array_serialize_loop(s: ScalarType, iter_type: &str) -> String {
	if s == ScalarType::Str {
		"for (String _e : _arr) {\n\
			 \t\t\tbyte[] _b = _e.getBytes(java.nio.charset.StandardCharsets.UTF_8);\n\
			 \t\t\t_dos.writeInt(_b.length);\n\
			 \t\t\t_dos.write(_b, 0, _b.length);\n\
			 \t\t}"
			.to_string()
	} else {
		let method = s.dos_write_method().unwrap();
		format!("for ({iter_type} _e : _arr) {{ _dos.{method}(_e); }}")
	}
}

// parse_java_source — merged import split + var extraction + return-type parse

/// Output of the unified Java source parser.
struct ParsedJava {
	/// The import/package section verbatim from the original source.
	imports: String,
	/// Any class/interface/enum declarations written before `run()`.
	/// Emitted as top-level Java types, outside the generated wrapper class.
	outer: String,
	/// The `run()` method and everything after it, verbatim from the original
	/// source, with every `'var` replaced by `_RUST_var`.
	/// Placed inside the generated wrapper class.
	body: String,
	/// Captured Rust variables: name → original Ident (for span / quoting).
	vars: BTreeMap<String, Ident>,
	/// Return type of the `public static T run()` method.
	java_type: JavaType,
}

/// Records one `'var` occurrence found while walking the token tree.
struct VarOccurrence {
	/// Source position of the leading `'` punctuation.
	quote_start: LineColumn,
	/// Source position one-past-end of the identifier that follows.
	ident_end: LineColumn,
	/// Variable name (without the leading `'`).
	name: String,
}

/// Walk `stream` recursively (into Groups) and collect every `'ident`
/// occurrence.  The first occurrence of each name is stored in `vars`; all
/// occurrences (for substitution) go into `occurrences`.
fn collect_vars(
	stream: proc_macro2::TokenStream,
	vars: &mut BTreeMap<String, Ident>,
	occurrences: &mut Vec<VarOccurrence>,
) {
	let tts: Vec<TokenTree> = stream.into_iter().collect();
	let mut i = 0;
	while i < tts.len() {
		if matches!(&tts[i], TokenTree::Punct(p)
				if p.as_char() == '\'' && p.spacing() == Spacing::Joint)
			&& let Some(TokenTree::Ident(id)) = tts.get(i + 1)
		{
			let name = id.to_string();
			vars.entry(name.clone()).or_insert_with(|| id.clone());
			occurrences.push(VarOccurrence {
				quote_start: tts[i].span().start(),
				ident_end: id.span().end(),
				name,
			});
			i += 2;
			continue;
		}
		if let TokenTree::Group(g) = &tts[i] {
			collect_vars(g.stream(), vars, occurrences);
		}
		i += 1;
	}
}

/// Convert an absolute source `LineColumn` to a byte offset within `text`,
/// given that the first character of `text` is at `text_start` in the source
/// file.  Returns `text.len()` if `target` is at or past the end.
///
/// `LineColumn::line` is 1-indexed; `LineColumn::column` is 0-indexed (chars).
fn offset_in_text(text: &str, text_start: LineColumn, target: LineColumn) -> usize {
	let mut byte = 0usize;
	let mut line = text_start.line;
	let mut col = text_start.column;
	for ch in text.chars() {
		if line == target.line && col == target.column {
			return byte;
		}
		byte += ch.len_utf8();
		if ch == '\n' {
			line += 1;
			col = 0;
		} else {
			col += 1;
		}
	}
	byte
}

/// Replace every `'var` occurrence in `body` with `_RUST_var` using the span
/// positions in `occurrences`.  `body_start` is the absolute source position
/// of the first character in `body`.
fn substitute_vars_in_body(
	body: &str,
	body_start: LineColumn,
	occurrences: &[VarOccurrence],
) -> String {
	let mut result = String::with_capacity(body.len() + occurrences.len() * 6);
	let mut last_byte = 0usize;
	for occ in occurrences {
		let start = offset_in_text(body, body_start, occ.quote_start);
		let end = offset_in_text(body, body_start, occ.ident_end);
		result.push_str(&body[last_byte..start]);
		result.push_str("_RUST_");
		result.push_str(&occ.name);
		last_byte = end;
	}
	result.push_str(&body[last_byte..]);
	result
}

/// Fallback body reconstruction from tokens when `source_text()` is
/// unavailable.  Performs `'var` → `_RUST_var` substitution at the token
/// level.
fn reconstruct_body_fallback(
	tts: &[TokenTree],
	start_idx: usize,
	occurrences: &[VarOccurrence],
) -> String {
	// Index the quote positions so we can skip them during reconstruction.
	let quote_positions: std::collections::HashSet<(usize, usize)> = occurrences
		.iter()
		.map(|o| (o.quote_start.line, o.quote_start.column))
		.collect();

	let mut parts: Vec<String> = Vec::new();
	let mut i = start_idx;
	while i < tts.len() {
		let lc = tts[i].span().start();
		if quote_positions.contains(&(lc.line, lc.column))
			&& let Some(TokenTree::Ident(id)) = tts.get(i + 1)
		{
			parts.push(format!("_RUST_{id}"));
			i += 2;
			continue;
		}
		parts.push(tts[i].to_string());
		i += 1;
	}
	parts.join(" ")
}

/// Scan `tts` for the first `public static <T> run` pattern and return the
/// corresponding `JavaType` together with the index of the `public` token
/// within `tts` (so the caller can split outer declarations from the method).
fn parse_run_return_type(tts: &[TokenTree]) -> Result<(JavaType, usize), String> {
	for i in 0..tts.len().saturating_sub(3) {
		if !matches!(&tts[i], TokenTree::Ident(id) if id == "public") {
			continue;
		}
		if !matches!(&tts[i + 1], TokenTree::Ident(id) if id == "static") {
			continue;
		}

		// Pattern 1: public static T run  (scalar)
		if let TokenTree::Ident(type_id) = &tts[i + 2] {
			let type_name = type_id.to_string();

			if matches!(&tts.get(i + 3), Some(TokenTree::Ident(id)) if id == "run") {
				return ScalarType::from_primitive_name(&type_name)
					.map(|s| (JavaType::Scalar(s), i))
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` return type `{type_name}` is not supported; \
							 scalar types: byte short int long float double boolean char String; \
							 array types: T[] or List<T> for any of those T"
						)
					});
			}

			// Pattern 2: public static T[] run  (array)
			let is_empty_bracket = matches!(
				&tts.get(i + 3),
				Some(TokenTree::Group(g))
					if g.delimiter() == proc_macro2::Delimiter::Bracket
					   && g.stream().is_empty()
			);
			if is_empty_bracket
				&& matches!(&tts.get(i + 4), Some(TokenTree::Ident(id)) if id == "run")
			{
				return ScalarType::from_primitive_name(&type_name)
					.map(|s| (JavaType::Array(s), i))
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` array element type `{type_name}` is not supported; \
								 supported types: byte short int long float double boolean char String"
						)
					});
			}
		}

		// Pattern 3: public static List < BoxedT > run  (List<T>)
		if matches!(&tts[i + 2], TokenTree::Ident(id) if id == "List")
			&& matches!(&tts.get(i + 3), Some(TokenTree::Punct(p)) if p.as_char() == '<')
			&& let Some(TokenTree::Ident(inner_id)) = tts.get(i + 4)
		{
			let inner_name = inner_id.to_string();
			if matches!(&tts.get(i + 5), Some(TokenTree::Punct(p)) if p.as_char() == '>')
				&& matches!(&tts.get(i + 6), Some(TokenTree::Ident(id)) if id == "run")
			{
				return ScalarType::from_boxed_name(&inner_name)
					.map(|s| (JavaType::List(s), i))
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` List element type `{inner_name}` is not supported; \
										 supported types: Byte Short Integer Long Float Double Boolean Character String"
						)
					});
			}
		}

		// Pattern 4: public static java.util.List < BoxedT > run
		if matches!(&tts[i + 2], TokenTree::Ident(id) if id == "java")
			&& matches!(&tts.get(i + 3), Some(TokenTree::Punct(p)) if p.as_char() == '.')
			&& matches!(&tts.get(i + 4), Some(TokenTree::Ident(id)) if id == "util")
			&& matches!(&tts.get(i + 5), Some(TokenTree::Punct(p)) if p.as_char() == '.')
			&& matches!(&tts.get(i + 6), Some(TokenTree::Ident(id)) if id == "List")
			&& matches!(&tts.get(i + 7), Some(TokenTree::Punct(p)) if p.as_char() == '<')
			&& let Some(TokenTree::Ident(inner_id)) = tts.get(i + 8)
			&& matches!(&tts.get(i + 9), Some(TokenTree::Punct(p)) if p.as_char() == '>')
			&& matches!(&tts.get(i + 10), Some(TokenTree::Ident(id)) if id == "run")
		{
			let inner_name = inner_id.to_string();
			return ScalarType::from_boxed_name(&inner_name)
				.map(|s| (JavaType::List(s), i))
				.ok_or_else(|| {
					format!(
						"inline_java: `run()` List element type `{inner_name}` is not supported; \
						 supported types: Byte Short Integer Long Float Double Boolean Character String"
					)
				});
		}
	}
	Err("inline_java: could not find `public static <type> run()` in Java body".to_string())
}

/// Unified parser: walks the token stream once to separate `import`/`package`
/// directives from the method body, identify the `run()` return type, and
/// collect `'var` injections.
///
/// Rather than reconstructing strings from the token tree (which loses
/// whitespace), it uses `Span::join` + `Span::source_text` to slice the
/// original source text directly.
fn parse_java_source(stream: proc_macro2::TokenStream) -> Result<ParsedJava, String> {
	let tts: Vec<TokenTree> = stream.into_iter().collect();

	// Separate imports from body
	let mut first_import_idx: Option<usize> = None;
	let mut last_import_end_idx: Option<usize> = None; // index of the last ';' in imports
	let mut first_body_idx: Option<usize> = None;
	let mut in_imports = true;
	let mut i = 0usize;

	while i < tts.len() && in_imports {
		match &tts[i] {
			TokenTree::Ident(id) if id == "import" || id == "package" => {
				first_import_idx.get_or_insert(i);
				// Scan forward for the terminating ';'.
				let semi = tts[i + 1..]
					.iter()
					.position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
					.map(|rel| i + 1 + rel);
				if let Some(semi_idx) = semi {
					last_import_end_idx = Some(semi_idx);
					i = semi_idx + 1;
				} else {
					// Malformed: no semicolon; treat remainder as body.
					in_imports = false;
					first_body_idx = Some(i);
				}
			}
			_ => {
				in_imports = false;
				first_body_idx = Some(i);
			}
		}
	}
	// If the loop ended because all tokens were imports, body starts at i.
	if first_body_idx.is_none() && i < tts.len() {
		first_body_idx = Some(i);
	}
	let body_start = first_body_idx.unwrap_or(tts.len());

	// Parse return type from body tokens
	// Also returns the index (relative to body_start) of the `public` token
	// so we can split outer class declarations from the run() method.
	let (java_type, run_rel_idx) = parse_run_return_type(&tts[body_start..])?;
	let run_abs_idx = body_start + run_rel_idx;

	// Collect vars from body (recursively into Groups)
	let mut vars: BTreeMap<String, Ident> = BTreeMap::new();
	let mut occurrences: Vec<VarOccurrence> = Vec::new();
	let body_stream: proc_macro2::TokenStream = tts[body_start..].iter().cloned().collect();
	collect_vars(body_stream, &mut vars, &mut occurrences);
	// Ensure occurrences are in source order for sequential substitution.
	occurrences.sort_by_key(|o| (o.quote_start.line, o.quote_start.column));

	// Extract text via source_text()

	// Helper: get source text for a contiguous slice of tts, with fallback.
	let slice_text = |lo: usize, hi: usize| -> String {
		if lo >= hi {
			return String::new();
		}
		tts[lo]
			.span()
			.join(tts[hi - 1].span())
			.and_then(|s| s.source_text())
			.unwrap_or_else(|| {
				tts[lo..hi]
					.iter()
					.map(std::string::ToString::to_string)
					.collect::<Vec<_>>()
					.join(" ")
			})
	};

	// imports: span from first import keyword to last ';'
	let imports = match (first_import_idx, last_import_end_idx) {
		(Some(fi), Some(le)) => slice_text(fi, le + 1),
		_ => String::new(),
	};

	// outer: any tokens between the end of imports and `public static T run`
	let outer = slice_text(body_start, run_abs_idx);

	// body: from `public static T run` to end, with var substitution.
	let body = if run_abs_idx < tts.len() {
		let start_span = tts[run_abs_idx].span();
		let end_span = tts.last().unwrap().span();
		let body_start_lc = start_span.start();

		// occurrences is sorted; skip any that fall in the outer section.
		let first_in_body = occurrences.partition_point(|o| {
			(o.quote_start.line, o.quote_start.column) < (body_start_lc.line, body_start_lc.column)
		});
		let body_occurrences = &occurrences[first_in_body..];

		match start_span.join(end_span).and_then(|s| s.source_text()) {
			Some(raw) if body_occurrences.is_empty() => raw,
			Some(raw) => substitute_vars_in_body(&raw, body_start_lc, body_occurrences),
			None => reconstruct_body_fallback(&tts, run_abs_idx, &occurrences),
		}
	} else {
		String::new()
	};

	Ok(ParsedJava {
		imports,
		outer,
		body,
		vars,
		java_type,
	})
}

#[proc_macro]
pub fn java(input: TokenStream) -> TokenStream {
	let input2 = proc_macro2::TokenStream::from(input);

	// Consume any leading `key = "value",` option pairs.
	let (opts, input2) = extract_opts(input2);

	let parsed = match parse_java_source(input2) {
		Ok(p) => p,
		Err(msg) => return quote! { compile_error!(#msg) }.into(),
	};

	let ParsedJava {
		imports,
		outer,
		body,
		vars,
		java_type,
	} = parsed;

	// Unique class name derived from source content and compilation options.
	// opts are included so that two call sites with the same body but different
	// javac/java args get separate temp dirs (and separate .done sentinels).
	let class_name = make_class_name("InlineJava", &imports, &outer, &body, &opts);
	let filename = format!("{class_name}.java");

	// If the user wrote a `package` declaration, the class must be run by its
	// fully-qualified name (e.g. `com.example.demo.InlineJava_xxx`).
	let full_class_name = qualify_class_name(&class_name, &imports);

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
	let java_class = format_java_class(&imports, &outer, &class_name, &var_fields, &body, &main_method);

	// Option strings are baked into the generated code as string literals;
	// shell expansion happens at program runtime via `inline_java::run_java`.
	let javac_raw = opts.javac_args.unwrap_or_default();
	let java_raw = opts.java_args.unwrap_or_default();
	let var_idents: Vec<Ident> = vars.values().cloned().collect();
	let deser = java_type.rust_deser();

	let generated = quote! {
		(|| -> ::std::result::Result<_, ::inline_java::JavaError> {
			let _raw = ::inline_java::run_java(
				#class_name,
				#filename,
				#java_class,
				#full_class_name,
				#javac_raw,
				#java_raw,
				&[#(::std::string::ToString::to_string(&#var_idents)),*],
			)?;
			::std::result::Result::Ok(#deser)
		})()
	};

	generated.into()
}

// ct_java! — compile-time Java evaluation

/// Run Java at *compile time* and splice its return value as a Rust literal.
///
/// Accepts optional `javac = "..."` and `java = "..."` key-value pairs before
/// the Java body.  The user provides a `public static <T> run()` method; its
/// binary-serialised return value is decoded and emitted as a Rust literal at
/// the call site (`42`, `3.14`, `true`, `'x'`, `"hello"`, `[1, 2, 3]`, …).
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

	let ParsedJava {
		imports,
		outer,
		body,
		java_type,
		..
	} = parse_java_source(input)?;

	let class_name = make_class_name("CtJava", &imports, &outer, &body, &opts);
	let filename = format!("{class_name}.java");
	let full_class_name = qualify_class_name(&class_name, &imports);

	let main_method = java_type.java_main("");
	let java_class = format_java_class(&imports, &outer, &class_name, "", &body, &main_method);

	let bytes = compile_run_java_now(
		&class_name,
		&filename,
		&java_class,
		&full_class_name,
		opts.javac_args,
		opts.java_args,
	)?;
	java_type.ct_java_tokens(bytes)
}

// Option extraction: `javac = "…"` / `java = "…"` before the Java body

struct JavaOpts {
	/// Extra args for `javac`, shell-split at use-site.  `None` → no extra args.
	javac_args: Option<String>,
	/// Extra args for `java`, shell-split at use-site.  `None` → no extra args.
	java_args: Option<String>,
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
	let Some(TokenTree::Literal(lit)) = tts.get(2) else {
		return None;
	};
	let value = litrs::StringLit::try_from(lit).ok()?.value().to_owned();
	Some((key, value, 3))
}

// Shared helpers used by both java! and ct_java!

/// Compute a deterministic class name by hashing the source and options.
/// `prefix` distinguishes runtime ("InlineJava") from compile-time ("CtJava").
fn make_class_name(prefix: &str, imports: &str, outer: &str, body: &str, opts: &JavaOpts) -> String {
	let mut h = DefaultHasher::new();
	imports.hash(&mut h);
	outer.hash(&mut h);
	body.hash(&mut h);
	opts.javac_args.hash(&mut h);
	opts.java_args.hash(&mut h);
	format!("{prefix}_{:016x}", h.finish())
}

/// Qualify `class_name` with its package if `imports` contains a `package`
/// declaration (e.g. `"com.example.InlineJava_xxx"`).
fn qualify_class_name(class_name: &str, imports: &str) -> String {
	match parse_package_name(imports) {
		Some(pkg) => format!("{pkg}.{class_name}"),
		None => class_name.to_owned(),
	}
}

/// Compile (if needed) and run a Java class at *compile time*, returning raw
/// stdout bytes.  Delegates to `inline_java_core::run_java` and maps
/// `JavaError` to `String` for use as a `compile_error!` diagnostic.
fn compile_run_java_now(
	class_name: &str,
	filename: &str,
	java_class: &str,
	full_class_name: &str,
	javac_raw: Option<String>,
	java_raw: Option<String>,
) -> Result<Vec<u8>, String> {
	inline_java_core::run_java(
		class_name,
		filename,
		java_class,
		full_class_name,
		javac_raw.as_deref().unwrap_or(""),
		java_raw.as_deref().unwrap_or(""),
		&[],
	)
	.map_err(|e| e.to_string())
}

/// Render the complete `.java` source file.  Pass `var_fields = ""` when
/// there are no injected Rust variables (i.e. for `ct_java!`).
fn format_java_class(
	imports: &str,
	outer: &str,
	class_name: &str,
	var_fields: &str,
	body: &str,
	main_method: &str,
) -> String {
	format!(
		"{imports}\n{outer}\npublic class {class_name} {{\n{var_fields}\n{body}\n\n{main_method}\n}}\n"
	)
}

// Package name extraction

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
