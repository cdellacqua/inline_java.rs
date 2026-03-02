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

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Spacing, TokenTree};
use quote::quote;

// ---------------------------------------------------------------------------
// ScalarType — the nine primitive / String base types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// JavaType — allowed return types for run(), with serialisation/deserialisation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// scalar_ct_lit — convert raw bytes to a Rust literal string for one element
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// array_serialize_loop — Java loop body for array/List serialisation
// ---------------------------------------------------------------------------

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

/// Scan the body token stream for the first `public static <type> run` pattern
/// and return the corresponding `JavaType`.  Handles:
///   - `public static T run`         → Scalar(T)
///   - `public static T[] run`       → Array(T)
///   - `public static List < T > run` → List(T)
fn parse_run_return_type(body: &proc_macro2::TokenStream) -> Result<JavaType, String> {
	let tts: Vec<TokenTree> = body.clone().into_iter().collect();
	for i in 0..tts.len().saturating_sub(3) {
		if !matches!(&tts[i], TokenTree::Ident(id) if id == "public") {
			continue;
		}
		if !matches!(&tts[i + 1], TokenTree::Ident(id) if id == "static") {
			continue;
		}

		// ── Pattern 1: public static T run  (scalar) ───────────────────────
		if let TokenTree::Ident(type_id) = &tts[i + 2] {
			let type_name = type_id.to_string();

			if matches!(&tts.get(i + 3), Some(TokenTree::Ident(id)) if id == "run") {
				return ScalarType::from_primitive_name(&type_name)
					.map(JavaType::Scalar)
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` return type `{type_name}` is not supported; \
							 scalar types: byte short int long float double boolean char String; \
							 array types: T[] or List<T> for any of those T"
						)
					});
			}

			// ── Pattern 2: public static T[] run  (array) ──────────────────
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
					.map(JavaType::Array)
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` array element type `{type_name}` is not supported; \
								 supported types: byte short int long float double boolean char String"
						)
					});
			}
		}

		// ── Pattern 3: public static List < BoxedT > run  (List<T>) ────────
		if matches!(&tts[i + 2], TokenTree::Ident(id) if id == "List")
			&& matches!(&tts.get(i + 3), Some(TokenTree::Punct(p)) if p.as_char() == '<')
			&& let Some(TokenTree::Ident(inner_id)) = tts.get(i + 4)
		{
			let inner_name = inner_id.to_string();
			if matches!(&tts.get(i + 5), Some(TokenTree::Punct(p)) if p.as_char() == '>')
				&& matches!(&tts.get(i + 6), Some(TokenTree::Ident(id)) if id == "run")
			{
				return ScalarType::from_boxed_name(&inner_name)
					.map(JavaType::List)
					.ok_or_else(|| {
						format!(
							"inline_java: `run()` List element type `{inner_name}` is not supported; \
										 supported types: Byte Short Integer Long Float Double Boolean Character String"
						)
					});
			}
		}
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
		.map(|a| {
			split_args(
				&shellexpand::full(&a)
					.map(std::borrow::Cow::into_owned)
					.unwrap_or(a),
			)
		})
		.unwrap_or_default();
	let java_runtime_extra: Vec<String> = opts
		.java_args
		.map(|a| {
			split_args(
				&shellexpand::full(&a)
					.map(std::borrow::Cow::into_owned)
					.unwrap_or(a),
			)
		})
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
		.map(|a| {
			split_args(
				&shellexpand::full(&a)
					.map(std::borrow::Cow::into_owned)
					.unwrap_or(a),
			)
		})
		.unwrap_or_default();
	let java_runtime_extra: Vec<String> = opts
		.java_args
		.map(|a| {
			split_args(
				&shellexpand::full(&a)
					.map(std::borrow::Cow::into_owned)
					.unwrap_or(a),
			)
		})
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
