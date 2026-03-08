//! Proc-macro implementation for `inline_java`.
//!
//! Provides three proc macros for embedding Java in Rust:
//!
//! | Macro        | When it runs    |
//! |--------------|-----------------|
//! | [`java!`]    | program runtime |
//! | [`java_fn!`] | program runtime |
//! | [`ct_java!`] | compile time    |
//!
//! All macros require the user to write a `static <T> run(...)` method
//! where `T` is one of: `byte`, `short`, `int`, `long`, `float`, `double`,
//! `boolean`, `char`, `String`, `T[]`, `List<T>`, or `Optional<T>` —
//! including arbitrarily nested types like `List<List<Integer>>`,
//! `Optional<List<String>>`, `Optional<List<Optional<Integer[]>>>`, etc.
//!
//! # Wire format (Java → Rust, stdout)
//!
//! The macro generates a `main()` that binary-serialises `run()`'s return
//! value to stdout via `DataOutputStream` (raw UTF-8 for top-level `String`
//! scalars).
//!
//! Encoding per type:
//! - `Boxed(String)` at top level: raw UTF-8 (no length prefix)
//! - `Boxed(String)` inside a container: 4-byte BE length + UTF-8 bytes
//! - `Primitive` / `Boxed(non-String)`: fixed-width big-endian bytes via DataOutputStream
//! - `Array(T)` / `List(T)`: 4-byte BE count + N × encode(T)
//! - `Optional(T)`: 1-byte tag (0=absent, 1=present) + encode(T) if present
//!
//! # Wire format (Rust → Java, stdin)
//!
//! Parameters declared in `run(...)` are serialised by Rust and piped to the
//! child process's stdin. Java reads them with `DataInputStream`.
//!
//! # Options
//!
//! All three macros accept zero or more `key = "value"` pairs before the Java body,
//! comma-separated.  Recognised keys:
//!
//! - `javac = "<args>"` — extra arguments passed verbatim to `javac`
//!   (shell-quoted; single/double quotes respected).
//! - `java  = "<args>"` — extra arguments passed verbatim to `java`
//!   (shell-quoted; single/double quotes respected).

use proc_macro::TokenStream;
use proc_macro2::TokenTree;
use quote::{format_ident, quote};
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

// ── ScalarEnc ────────────────────────────────────────────────────────────────

/// Internal wire-encoding kind shared by `PrimitiveType` and `BoxedType`.
/// Determines byte layout, DataOutputStream method, and Rust decode logic.
#[derive(Clone, Copy, PartialEq)]
enum ScalarEnc {
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

impl ScalarEnc {
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
}

// ── PrimitiveType ────────────────────────────────────────────────────────────

/// Java primitive types (`byte`, `int`, `double`, …) — no `String`.
/// Used for scalars and array element types written with primitive names.
#[derive(Clone, Copy, PartialEq)]
enum PrimitiveType {
	Byte,
	Short,
	Int,
	Long,
	Float,
	Double,
	Boolean,
	Char,
}

impl PrimitiveType {
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
			_ => None,
		}
	}

	fn java_name(self) -> &'static str {
		match self {
			Self::Byte => "byte",
			Self::Short => "short",
			Self::Int => "int",
			Self::Long => "long",
			Self::Float => "float",
			Self::Double => "double",
			Self::Boolean => "boolean",
			Self::Char => "char",
		}
	}

	fn enc(self) -> ScalarEnc {
		match self {
			Self::Byte => ScalarEnc::Byte,
			Self::Short => ScalarEnc::Short,
			Self::Int => ScalarEnc::Int,
			Self::Long => ScalarEnc::Long,
			Self::Float => ScalarEnc::Float,
			Self::Double => ScalarEnc::Double,
			Self::Boolean => ScalarEnc::Boolean,
			Self::Char => ScalarEnc::Char,
		}
	}

	fn dos_write_method(self) -> &'static str {
		self.enc().dos_write_method().unwrap()
	}

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
		}
	}

	fn java_dis_read(self, param_name: &str) -> String {
		match self {
			Self::Byte => format!("byte {param_name} = _dis.readByte();"),
			Self::Short => format!("short {param_name} = _dis.readShort();"),
			Self::Int => format!("int {param_name} = _dis.readInt();"),
			Self::Long => format!("long {param_name} = _dis.readLong();"),
			Self::Float => format!("float {param_name} = _dis.readFloat();"),
			Self::Double => format!("double {param_name} = _dis.readDouble();"),
			Self::Boolean => format!("boolean {param_name} = _dis.readBoolean();"),
			Self::Char => format!("char {param_name} = _dis.readChar();"),
		}
	}
}

// ── BoxedType ────────────────────────────────────────────────────────────────

/// Java reference / boxed types (`Integer`, `String`, …).
/// Used for all types inside `<>` generics, and for `String` at any level.
#[derive(Clone, Copy, PartialEq)]
enum BoxedType {
	Byte,
	Short,
	Integer,
	Long,
	Float,
	Double,
	Boolean,
	Character,
	String,
}

impl BoxedType {
	/// Parse a boxed or primitive name (both accepted inside `<>`).
	fn from_name(s: &str) -> Option<Self> {
		match s {
			"Byte" | "byte" => Some(Self::Byte),
			"Short" | "short" => Some(Self::Short),
			"Integer" | "int" => Some(Self::Integer),
			"Long" | "long" => Some(Self::Long),
			"Float" | "float" => Some(Self::Float),
			"Double" | "double" => Some(Self::Double),
			"Boolean" | "boolean" => Some(Self::Boolean),
			"Character" | "char" => Some(Self::Character),
			"String" => Some(Self::String),
			_ => None,
		}
	}

	fn java_name(self) -> &'static str {
		match self {
			Self::Byte => "Byte",
			Self::Short => "Short",
			Self::Integer => "Integer",
			Self::Long => "Long",
			Self::Float => "Float",
			Self::Double => "Double",
			Self::Boolean => "Boolean",
			Self::Character => "Character",
			Self::String => "String",
		}
	}

	fn enc(self) -> ScalarEnc {
		match self {
			Self::Byte => ScalarEnc::Byte,
			Self::Short => ScalarEnc::Short,
			Self::Integer => ScalarEnc::Int,
			Self::Long => ScalarEnc::Long,
			Self::Float => ScalarEnc::Float,
			Self::Double => ScalarEnc::Double,
			Self::Boolean => ScalarEnc::Boolean,
			Self::Character => ScalarEnc::Char,
			Self::String => ScalarEnc::Str,
		}
	}

	fn dos_write_method(self) -> Option<&'static str> {
		self.enc().dos_write_method()
	}

	fn rust_type_ts(self) -> proc_macro2::TokenStream {
		match self {
			Self::Byte => quote! { i8 },
			Self::Short => quote! { i16 },
			Self::Integer => quote! { i32 },
			Self::Long => quote! { i64 },
			Self::Float => quote! { f32 },
			Self::Double => quote! { f64 },
			Self::Boolean => quote! { bool },
			Self::Character => quote! { char },
			Self::String => quote! { ::std::string::String },
		}
	}

	/// Read from DataInputStream — always uses primitive read ops; Java auto-boxes.
	fn java_dis_read(self, param_name: &str) -> String {
		match self {
			Self::Byte => format!("byte {param_name} = _dis.readByte();"),
			Self::Short => format!("short {param_name} = _dis.readShort();"),
			Self::Integer => format!("int {param_name} = _dis.readInt();"),
			Self::Long => format!("long {param_name} = _dis.readLong();"),
			Self::Float => format!("float {param_name} = _dis.readFloat();"),
			Self::Double => format!("double {param_name} = _dis.readDouble();"),
			Self::Boolean => format!("boolean {param_name} = _dis.readBoolean();"),
			Self::Character => format!("char {param_name} = _dis.readChar();"),
			Self::String => format!(
				"int _len_{param_name} = _dis.readInt();\n\
				 \t\tbyte[] _b_{param_name} = new byte[_len_{param_name}];\n\
				 \t\t_dis.readFully(_b_{param_name});\n\
				 \t\tString {param_name} = new String(_b_{param_name}, java.nio.charset.StandardCharsets.UTF_8);"
			),
		}
	}
}

// ── JavaType ─────────────────────────────────────────────────────────────────

/// Recursive composable Java type system.
#[derive(Clone, PartialEq)]
enum JavaType {
	/// Java primitive types (`int`, `double`, …).
	Primitive(PrimitiveType),
	/// Java reference / boxed types (`Integer`, `String`, …) — inside generics, or `String` at top level.
	Boxed(BoxedType),
	/// Java `T[]` — returned as `Vec<T>` at runtime.
	Array(Box<JavaType>),
	/// Java `List<T>` — same wire format / Rust type as `Array`.
	List(Box<JavaType>),
	/// Java `Optional<T>` — returned as `Option<T>`.
	Optional(Box<JavaType>),
}

impl JavaType {
	/// Returns the Java type name for use in generated code.
	/// The name reflects exactly how the type was parsed:
	/// `Primitive(Int)` → `"int"`, `Boxed(Integer)` → `"Integer"`,
	/// `Array(Boxed(Integer))` → `"Integer[]"`, etc.
	fn java_type_name(&self) -> String {
		match self {
			Self::Primitive(p) => p.java_name().to_string(),
			Self::Boxed(b) => b.java_name().to_string(),
			Self::Array(inner) => format!("{}[]", inner.java_type_name()),
			Self::List(inner) => format!("java.util.List<{}>", inner.java_type_name()),
			Self::Optional(inner) => format!("java.util.Optional<{}>", inner.java_type_name()),
		}
	}

	/// Returns the Rust return type token stream for this Java type.
	fn rust_return_type_ts(&self) -> proc_macro2::TokenStream {
		match self {
			Self::Primitive(p) => p.rust_type_ts(),
			Self::Boxed(b) => b.rust_type_ts(),
			Self::Array(inner) | Self::List(inner) => {
				let inner_ts = inner.rust_return_type_ts();
				quote! { ::std::vec::Vec<#inner_ts> }
			}
			Self::Optional(inner) => {
				let inner_ts = inner.rust_return_type_ts();
				quote! { ::std::option::Option<#inner_ts> }
			}
		}
	}

	/// Returns the Rust parameter type token stream.
	/// `Boxed(String)` leaf → `&str`; `Array`/`List` → `Vec<T>` (Fix A).
	fn rust_param_type_ts(&self) -> proc_macro2::TokenStream {
		match self {
			Self::Primitive(p) => p.rust_type_ts(),
			Self::Boxed(BoxedType::String) => quote! { &str },
			Self::Boxed(b) => b.rust_type_ts(),
			Self::Array(inner) | Self::List(inner) => {
				let inner_ts = inner.rust_param_type_ts();
				quote! { ::std::vec::Vec<#inner_ts> }
			}
			Self::Optional(inner) => {
				let inner_ts = inner.rust_param_type_ts();
				quote! { ::std::option::Option<#inner_ts> }
			}
		}
	}

	/// Generates Rust code to serialize a parameter value into `_stdin_bytes`.
	/// `param_ident` is the Rust identifier holding the value.
	/// `depth` is used to generate unique variable names for nested loops.
	fn rust_ser_ts(&self, param_ident: &proc_macro2::TokenStream, depth: usize) -> proc_macro2::TokenStream {
		match self {
			Self::Primitive(p) => scalar_enc_ser_ts(p.enc(), param_ident),
			Self::Boxed(b) => scalar_enc_ser_ts(b.enc(), param_ident),
			Self::Array(inner) | Self::List(inner) => {
				let item_var = format_ident!("_item{}", depth);
				let item_expr = quote! { #item_var };
				let inner_ser = inner.rust_ser_ts(&item_expr, depth + 1);
				quote! {
					{
						_stdin_bytes.extend_from_slice(&(#param_ident.len() as i32).to_be_bytes());
						for #item_var in #param_ident {
							#inner_ser
						}
					}
				}
			}
			Self::Optional(inner) => {
				let inner_var = format_ident!("_inner{}", depth);
				let inner_expr = quote! { #inner_var };
				let inner_ser = inner.rust_ser_ts(&inner_expr, depth + 1);
				quote! {
					match #param_ident {
						::std::option::Option::None => _stdin_bytes.push(0u8),
						::std::option::Option::Some(#inner_var) => {
							_stdin_bytes.push(1u8);
							#inner_ser
						}
					}
				}
			}
		}
	}

	/// Generates Java statement(s) to read this type from a `DataInputStream` named `_dis`.
	/// `param_name` is the Java variable name to declare.
	/// `depth` is used to generate unique names for temporaries.
	fn java_dis_read(&self, param_name: &str, depth: usize) -> String {
		match self {
			Self::Primitive(p) => p.java_dis_read(param_name),
			Self::Boxed(b) => b.java_dis_read(param_name),
			Self::Array(inner) => {
				let count_var = format!("_count_{param_name}_{depth}");
				let i_var = format!("_i_{param_name}_{depth}");
				let elem_var = format!("_elem_{param_name}_{depth}");
				let inner_java_type = inner.java_type_name();
				let new_expr = inner.java_new_outer_array(&count_var);
				let inner_read = inner.java_dis_read_for_elem(&elem_var, depth + 1);
				format!(
					"int {count_var} = _dis.readInt();\n\
					 \t\t{inner_java_type}[] {param_name} = {new_expr};\n\
					 \t\tfor (int {i_var} = 0; {i_var} < {count_var}; {i_var}++) {{\n\
					 \t\t\t{inner_read}\n\
					 \t\t\t{param_name}[{i_var}] = {elem_var};\n\
					 \t\t}}"
				)
			}
			Self::List(inner) => {
				let count_var = format!("_count_{param_name}_{depth}");
				let i_var = format!("_i_{param_name}_{depth}");
				let elem_var = format!("_elem_{param_name}_{depth}");
				let inner_java_type = inner.java_type_name();
				let inner_read = inner.java_dis_read_for_elem(&elem_var, depth + 1);
				format!(
					"int {count_var} = _dis.readInt();\n\
					 \t\tjava.util.List<{inner_java_type}> {param_name} = new java.util.ArrayList<>();\n\
					 \t\tfor (int {i_var} = 0; {i_var} < {count_var}; {i_var}++) {{\n\
					 \t\t\t{inner_read}\n\
					 \t\t\t{param_name}.add(({inner_java_type}) {elem_var});\n\
					 \t\t}}"
				)
			}
			Self::Optional(inner) => {
				let tag_var = format!("_tag_{param_name}_{depth}");
				let inner_var = format!("_inner_{param_name}_{depth}");
				let inner_java = inner.java_type_name();
				let inner_read = inner.java_dis_read_for_elem(&inner_var, depth + 1);
				format!(
					"int {tag_var} = _dis.readUnsignedByte();\n\
					 \t\tjava.util.Optional<{inner_java}> {param_name};\n\
					 \t\tif ({tag_var} != 0) {{\n\
					 \t\t\t{inner_read}\n\
					 \t\t\t{param_name} = java.util.Optional.of({inner_var});\n\
					 \t\t}} else {{\n\
					 \t\t\t{param_name} = java.util.Optional.empty();\n\
					 \t\t}}"
				)
			}
		}
	}

	/// Generate a Java `new T[count]` expression for an outer array whose elements
	/// are of type `self`.  Handles multi-dimensional arrays correctly:
	/// - `Boxed(String)`         → `new String[count]`
	/// - `Array(Boxed(String))`  → `new String[count][]`
	/// - `Array(Array(...))`     → `new Base[count][][]…`
	fn java_new_outer_array(&self, count_var: &str) -> String {
		let mut ty = self;
		let mut extra_dims = 0usize;
		while let JavaType::Array(inner) = ty {
			extra_dims += 1;
			ty = inner;
		}
		let base_name = ty.java_type_name(); // non-array base type name
		let trailing = "[]".repeat(extra_dims);
		format!("new {base_name}[{count_var}]{trailing}")
	}

	/// Like `java_dis_read` but for reading an inner element.
	fn java_dis_read_for_elem(&self, elem_name: &str, depth: usize) -> String {
		match self {
			Self::Primitive(p) => p.java_dis_read(elem_name),
			Self::Boxed(b) => b.java_dis_read(elem_name),
			_ => self.java_dis_read(elem_name, depth),
		}
	}

	/// Generates the complete `main(String[] args)` method that binary-serialises
	/// `run()`'s return value to stdout.  `params` lists the parameters declared
	/// in `run(...)` so the generated `main` can read them from stdin and forward
	/// them to `run`.
	fn java_main(&self, params: &[(JavaType, String)]) -> String {
		// Build DataInputStream setup + parameter reads (only if there are params).
		let param_reads = if params.is_empty() {
			String::new()
		} else {
			let mut s = String::from(
				"\t\tjava.io.DataInputStream _dis = new java.io.DataInputStream(System.in);\n",
			);
			for (ty, name) in params {
				writeln!(s, "\t\t{}", ty.java_dis_read(name, 0)).unwrap();
			}
			s
		};

		// Build the run() call argument list.
		let run_args: String = params
			.iter()
			.map(|(_, name)| name.as_str())
			.collect::<Vec<_>>()
			.join(", ");

		match self {
			Self::Primitive(p) => {
				let method = p.dos_write_method();
				let serialize = format!(
					"java.io.DataOutputStream _dos = \
					 new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.{method}(run({run_args}));\n\
					 \t\t_dos.flush();"
				);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\t{serialize}\n\
					 \t}}"
				)
			}
			Self::Boxed(BoxedType::String) => {
				let serialize = format!(
					"byte[] _b = run({run_args}).getBytes(java.nio.charset.StandardCharsets.UTF_8);\n\
  				 \t\tSystem.out.write(_b);\n\
  				 \t\tSystem.out.flush();"
				);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\t{serialize}\n\
					 \t}}"
				)
			}
			Self::Boxed(b) => {
				let method = b.dos_write_method().unwrap();
				let serialize = format!(
					"java.io.DataOutputStream _dos = \
					 new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.{method}(run({run_args}));\n\
					 \t\t_dos.flush();"
				);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\t{serialize}\n\
					 \t}}"
				)
			}
			Self::Array(inner) => {
				let elem_java_type = inner.java_type_name();
				let ser_body = java_ser_element(inner, "_e0", "_dos", 1);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\t{elem_java_type}[] _arr = run({run_args});\n\
					 \t\tjava.io.DataOutputStream _dos = new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.writeInt(_arr.length);\n\
					 \t\tfor ({elem_java_type} _e0 : _arr) {{\n\
					 \t\t\t{ser_body}\n\
					 \t\t}}\n\
					 \t\t_dos.flush();\n\
					 \t}}"
				)
			}
			Self::List(inner) => {
				let elem_java_type = inner.java_type_name();
				let ser_body = java_ser_element(inner, "_e0", "_dos", 1);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\tjava.util.List<{elem_java_type}> _arr = run({run_args});\n\
					 \t\tjava.io.DataOutputStream _dos = new java.io.DataOutputStream(System.out);\n\
					 \t\t_dos.writeInt(_arr.size());\n\
					 \t\tfor ({elem_java_type} _e0 : _arr) {{\n\
					 \t\t\t{ser_body}\n\
					 \t\t}}\n\
					 \t\t_dos.flush();\n\
					 \t}}"
				)
			}
			Self::Optional(inner) => {
				let inner_java_type = inner.java_type_name();
				let present_body = java_ser_element(inner, "_opt.get()", "_dos", 1);
				format!(
					"\tpublic static void main(String[] args) throws Exception {{\n\
					 {param_reads}\t\tjava.util.Optional<{inner_java_type}> _opt = run({run_args});\n\
					 \t\tjava.io.DataOutputStream _dos = new java.io.DataOutputStream(System.out);\n\
					 \t\tif (_opt.isPresent()) {{\n\
					 \t\t\t_dos.writeByte(1);\n\
					 \t\t\t{present_body}\n\
					 \t\t}} else {{\n\
					 \t\t\t_dos.writeByte(0);\n\
					 \t\t}}\n\
					 \t\t_dos.flush();\n\
					 \t}}"
				)
			}
		}
	}

	/// Returns a Rust expression (as a token stream) that deserialises the raw
	/// stdout bytes `_raw: Vec<u8>` into the corresponding Rust type.
	/// Used by `java!` and `java_fn!` at program runtime.
	fn rust_deser(&self) -> proc_macro2::TokenStream {
		match self {
			Self::Primitive(p) => scalar_enc_top_deser(p.enc()),
			Self::Boxed(BoxedType::String) => {
				// Top-level String: raw UTF-8, no length prefix
				quote! { ::std::string::String::from_utf8(_raw)? }
			}
			Self::Boxed(b) => scalar_enc_top_deser(b.enc()),
			_ => {
				// Container types: set up shared _cur and call recursive reader
				let rust_type = self.rust_return_type_ts();
				let read_expr = rust_read_element(self, 0);
				quote! {
					{
						let mut _cur = 0usize;
						let _result: #rust_type = #read_expr;
						_result
					}
				}
			}
		}
	}

	/// Converts the raw stdout bytes produced by the generated `main()` into a
	/// Rust literal / expression token stream to splice at the `ct_java!` call site.
	fn ct_java_tokens(&self, bytes: Vec<u8>) -> Result<proc_macro2::TokenStream, String> {
		match self {
			Self::Primitive(p) => {
				let (lit, _) = scalar_enc_ct_lit(p.enc(), &bytes)?;
				proc_macro2::TokenStream::from_str(&lit)
					.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
			}
			Self::Boxed(BoxedType::String) => {
				// Top-level String: raw UTF-8, no length prefix
				let s = String::from_utf8(bytes)
					.map_err(|_| "ct_java: Java String is not valid UTF-8".to_string())?;
				let lit = format!("{s:?}");
				proc_macro2::TokenStream::from_str(&lit)
					.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
			}
			Self::Boxed(b) => {
				let (lit, _) = scalar_enc_ct_lit(b.enc(), &bytes)?;
				proc_macro2::TokenStream::from_str(&lit)
					.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
			}
			_ => {
				let mut cur = 0usize;
				let ts = ct_java_tokens_recursive(self, &bytes, &mut cur)?;
				Ok(ts)
			}
		}
	}
}

// ── Scalar encoding helpers ───────────────────────────────────────────────────

/// Generate Rust code to serialise a scalar value into `_stdin_bytes`.
fn scalar_enc_ser_ts(enc: ScalarEnc, param_ident: &proc_macro2::TokenStream) -> proc_macro2::TokenStream {
	match enc {
		ScalarEnc::Byte => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as i8).to_be_bytes());
		},
		ScalarEnc::Short => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as i16).to_be_bytes());
		},
		ScalarEnc::Int => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as i32).to_be_bytes());
		},
		ScalarEnc::Long => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as i64).to_be_bytes());
		},
		ScalarEnc::Float => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as f32).to_bits().to_be_bytes());
		},
		ScalarEnc::Double => quote! {
			_stdin_bytes.extend_from_slice(&(#param_ident as f64).to_bits().to_be_bytes());
		},
		ScalarEnc::Boolean => quote! {
			_stdin_bytes.push(#param_ident as u8);
		},
		ScalarEnc::Char => quote! {
			{
				let _c = #param_ident as u32;
				assert!(_c <= 0xFFFF, "inline_java: char value exceeds u16 range");
				_stdin_bytes.extend_from_slice(&(_c as u16).to_be_bytes());
			}
		},
		ScalarEnc::Str => quote! {
			{
				let _b = #param_ident.as_bytes();
				let _len = _b.len() as i32;
				_stdin_bytes.extend_from_slice(&_len.to_be_bytes());
				_stdin_bytes.extend_from_slice(_b);
			}
		},
	}
}

/// Generate Rust expression to deserialise a top-level scalar from raw bytes.
fn scalar_enc_top_deser(enc: ScalarEnc) -> proc_macro2::TokenStream {
	match enc {
		ScalarEnc::Byte => quote! { i8::from_be_bytes([_raw[0]]) },
		ScalarEnc::Short => quote! { i16::from_be_bytes([_raw[0], _raw[1]]) },
		ScalarEnc::Int => {
			quote! { i32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]]) }
		}
		ScalarEnc::Long => {
			quote! {
				i64::from_be_bytes([
					_raw[0], _raw[1], _raw[2], _raw[3],
					_raw[4], _raw[5], _raw[6], _raw[7],
				])
			}
		}
		ScalarEnc::Float => {
			quote! { f32::from_bits(u32::from_be_bytes([_raw[0], _raw[1], _raw[2], _raw[3]])) }
		}
		ScalarEnc::Double => {
			quote! {
				f64::from_bits(u64::from_be_bytes([
					_raw[0], _raw[1], _raw[2], _raw[3],
					_raw[4], _raw[5], _raw[6], _raw[7],
				]))
			}
		}
		ScalarEnc::Boolean => quote! { _raw[0] != 0 },
		ScalarEnc::Char => {
			quote! {
				::std::char::from_u32(u16::from_be_bytes([_raw[0], _raw[1]]) as u32)
					.ok_or(::inline_java::JavaError::InvalidChar)?
			}
		}
		ScalarEnc::Str => {
			// Top-level Str: raw UTF-8 (caller handles separately for Boxed(String))
			quote! { ::std::string::String::from_utf8(_raw)? }
		}
	}
}

// ── scalar_enc_ct_lit ─────────────────────────────────────────────────────────

/// Deserialise one scalar element from `bytes` and return a
/// `(rust_literal_string, bytes_consumed)` pair for use in `ct_java_tokens`.
fn scalar_enc_ct_lit(enc: ScalarEnc, bytes: &[u8]) -> Result<(String, usize), String> {
	match enc {
		ScalarEnc::Byte => {
			if bytes.is_empty() {
				return Err("ct_java: truncated byte element".to_string());
			}
			Ok((format!("{}", i8::from_be_bytes([bytes[0]])), 1))
		}
		ScalarEnc::Short => {
			if bytes.len() < 2 {
				return Err("ct_java: truncated short element".to_string());
			}
			Ok((format!("{}", i16::from_be_bytes([bytes[0], bytes[1]])), 2))
		}
		ScalarEnc::Int => {
			let arr: [u8; 4] = bytes[..4]
				.try_into()
				.map_err(|_| "ct_java: truncated int element")?;
			Ok((format!("{}", i32::from_be_bytes(arr)), 4))
		}
		ScalarEnc::Long => {
			let arr: [u8; 8] = bytes[..8]
				.try_into()
				.map_err(|_| "ct_java: truncated long element")?;
			Ok((format!("{}", i64::from_be_bytes(arr)), 8))
		}
		ScalarEnc::Float => {
			let arr: [u8; 4] = bytes[..4]
				.try_into()
				.map_err(|_| "ct_java: truncated float element")?;
			let bits = u32::from_be_bytes(arr);
			Ok((format!("f32::from_bits(0x{bits:08x}_u32)"), 4))
		}
		ScalarEnc::Double => {
			let arr: [u8; 8] = bytes[..8]
				.try_into()
				.map_err(|_| "ct_java: truncated double element")?;
			let bits = u64::from_be_bytes(arr);
			Ok((format!("f64::from_bits(0x{bits:016x}_u64)"), 8))
		}
		ScalarEnc::Boolean => {
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
		ScalarEnc::Char => {
			if bytes.len() < 2 {
				return Err("ct_java: truncated char element".to_string());
			}
			let code_unit = u16::from_be_bytes([bytes[0], bytes[1]]);
			let c = char::from_u32(u32::from(code_unit))
				.ok_or("ct_java: Java char is not a valid Unicode scalar value")?;
			Ok((format!("{c:?}"), 2))
		}
		ScalarEnc::Str => {
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

// ── Recursive compile-time token generation ───────────────────────────────────

/// Recursively decode one value of `ty` from `bytes[*cur..]`, advance `*cur`,
/// and return a Rust literal/expression token stream.
fn ct_java_tokens_recursive(
	ty: &JavaType,
	bytes: &[u8],
	cur: &mut usize,
) -> Result<proc_macro2::TokenStream, String> {
	match ty {
		JavaType::Primitive(p) => {
			let (lit, consumed) = scalar_enc_ct_lit(p.enc(), &bytes[*cur..])?;
			*cur += consumed;
			proc_macro2::TokenStream::from_str(&lit)
				.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
		}
		JavaType::Boxed(b) => {
			let (lit, consumed) = scalar_enc_ct_lit(b.enc(), &bytes[*cur..])?;
			*cur += consumed;
			proc_macro2::TokenStream::from_str(&lit)
				.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
		}
		JavaType::Array(inner) | JavaType::List(inner) => {
			if bytes[*cur..].len() < 4 {
				return Err("ct_java: array/list output too short (missing length)".to_string());
			}
			#[allow(clippy::cast_sign_loss)]
			let n = i32::from_be_bytes(bytes[*cur..*cur + 4].try_into().unwrap()) as usize;
			*cur += 4;
			let mut lits: Vec<proc_macro2::TokenStream> = Vec::with_capacity(n);
			for _ in 0..n {
				lits.push(ct_java_tokens_recursive(inner, bytes, cur)?);
			}
			let array_ts = quote! { [#(#lits),*] };
			Ok(array_ts)
		}
		JavaType::Optional(inner) => {
			if bytes[*cur..].is_empty() {
				return Err("ct_java: optional output is empty".to_string());
			}
			let tag = bytes[*cur];
			*cur += 1;
			if tag == 0 {
				proc_macro2::TokenStream::from_str("::std::option::Option::None")
					.map_err(|e| format!("ct_java: produced invalid Rust token: {e}"))
			} else {
				let inner_ts = ct_java_tokens_recursive(inner, bytes, cur)?;
				let result = quote! { ::std::option::Option::Some(#inner_ts) };
				Ok(result)
			}
		}
	}
}

// ── Recursive Java serialization helper ──────────────────────────────────────

/// Generates Java code to serialize `var` of type `ty` to `DataOutputStream` named `dos`.
/// `depth` is used to generate unique local variable names for nested loops.
fn java_ser_element(ty: &JavaType, var: &str, dos: &str, depth: usize) -> String {
	match ty {
		JavaType::Primitive(p) => {
			let method = p.dos_write_method();
			format!("{dos}.{method}({var});")
		}
		JavaType::Boxed(BoxedType::String) => {
			format!(
				"{{ byte[] _b{depth} = ({var}).getBytes(java.nio.charset.StandardCharsets.UTF_8);\n\
				 \t\t\t{dos}.writeInt(_b{depth}.length);\n\
				 \t\t\t{dos}.write(_b{depth}, 0, _b{depth}.length); }}"
			)
		}
		JavaType::Boxed(b) => {
			let method = b.dos_write_method().unwrap();
			format!("{dos}.{method}({var});")
		}
		JavaType::Array(inner) => {
			let elem_java_type = inner.java_type_name();
			let elem_var = format!("_e{depth}");
			let inner_ser = java_ser_element(inner, &elem_var, dos, depth + 1);
			format!(
				"{dos}.writeInt(({var}).length);\n\
				 \t\t\tfor ({elem_java_type} {elem_var} : ({var})) {{\n\
				 \t\t\t\t{inner_ser}\n\
				 \t\t\t}}"
			)
		}
		JavaType::List(inner) => {
			let elem_java_type = inner.java_type_name();
			let elem_var = format!("_e{depth}");
			let inner_ser = java_ser_element(inner, &elem_var, dos, depth + 1);
			format!(
				"{dos}.writeInt(({var}).size());\n\
				 \t\t\tfor ({elem_java_type} {elem_var} : ({var})) {{\n\
				 \t\t\t\t{inner_ser}\n\
				 \t\t\t}}"
			)
		}
		JavaType::Optional(inner) => {
			let inner_java_type = inner.java_type_name();
			let inner_var = format!("_opt_inner{depth}");
			let inner_ser = java_ser_element(inner, &inner_var, dos, depth + 1);
			format!(
				"if (({var}).isPresent()) {{\n\
				 \t\t\t\t{dos}.writeByte(1);\n\
				 \t\t\t\t{inner_java_type} {inner_var} = ({var}).get();\n\
				 \t\t\t\t{inner_ser}\n\
				 \t\t\t}} else {{\n\
				 \t\t\t\t{dos}.writeByte(0);\n\
				 \t\t\t}}"
			)
		}
	}
}

// ── Recursive Rust deserialization helper ─────────────────────────────────────

/// Generates a Rust expression block that reads one value of type `ty` from `_raw`
/// using the shared mutable cursor `_cur`, and evaluates to the decoded Rust value.
/// `depth` is used to generate unique `_n{depth}` and `_v{depth}` variable names
/// for nested loops. All levels share the same `_cur` and `_raw` variables.
fn rust_read_element(ty: &JavaType, depth: usize) -> proc_macro2::TokenStream {
	match ty {
		JavaType::Primitive(p) => scalar_enc_read_element(p.enc()),
		JavaType::Boxed(b) => scalar_enc_read_element(b.enc()),
		JavaType::Array(inner) | JavaType::List(inner) => {
			let n_var = format_ident!("_n{}", depth);
			let v_var = format_ident!("_v{}", depth);
			let inner_rust_type = inner.rust_return_type_ts();
			let inner_read = rust_read_element(inner, depth + 1);
			quote! {{
				let #n_var = i32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]) as usize;
				_cur += 4;
				let mut #v_var: ::std::vec::Vec<#inner_rust_type> = ::std::vec::Vec::with_capacity(#n_var);
				for _ in 0..#n_var {
					let _item = #inner_read;
					#v_var.push(_item);
				}
				#v_var
			}}
		}
		JavaType::Optional(inner) => {
			let inner_rust_type = inner.rust_return_type_ts();
			let inner_read = rust_read_element(inner, depth + 1);
			quote! {{
				let _tag = _raw[_cur];
				_cur += 1;
				if _tag == 0 {
					::std::option::Option::None::<#inner_rust_type>
				} else {
					::std::option::Option::Some(#inner_read)
				}
			}}
		}
	}
}

/// Generate a Rust expression block that reads one scalar element from `_raw[_cur..]`.
fn scalar_enc_read_element(enc: ScalarEnc) -> proc_macro2::TokenStream {
	match enc {
		ScalarEnc::Byte => quote! {{
			let _val = i8::from_be_bytes([_raw[_cur]]);
			_cur += 1;
			_val
		}},
		ScalarEnc::Short => quote! {{
			let _val = i16::from_be_bytes([_raw[_cur], _raw[_cur + 1]]);
			_cur += 2;
			_val
		}},
		ScalarEnc::Int => quote! {{
			let _val = i32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]);
			_cur += 4;
			_val
		}},
		ScalarEnc::Long => quote! {{
			let _val = i64::from_be_bytes([
				_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3],
				_raw[_cur + 4], _raw[_cur + 5], _raw[_cur + 6], _raw[_cur + 7],
			]);
			_cur += 8;
			_val
		}},
		ScalarEnc::Float => quote! {{
			let _val = f32::from_bits(u32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]));
			_cur += 4;
			_val
		}},
		ScalarEnc::Double => quote! {{
			let _val = f64::from_bits(u64::from_be_bytes([
				_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3],
				_raw[_cur + 4], _raw[_cur + 5], _raw[_cur + 6], _raw[_cur + 7],
			]));
			_cur += 8;
			_val
		}},
		ScalarEnc::Boolean => quote! {{
			let _val = _raw[_cur] != 0;
			_cur += 1;
			_val
		}},
		ScalarEnc::Char => quote! {{
			let _val = ::std::char::from_u32(u16::from_be_bytes([_raw[_cur], _raw[_cur + 1]]) as u32)
				.ok_or(::inline_java::JavaError::InvalidChar)?;
			_cur += 2;
			_val
		}},
		// Str inside a container always has a 4-byte length prefix
		ScalarEnc::Str => quote! {{
			let _slen = i32::from_be_bytes([_raw[_cur], _raw[_cur + 1], _raw[_cur + 2], _raw[_cur + 3]]) as usize;
			_cur += 4;
			let _val = ::std::string::String::from_utf8(_raw[_cur.._cur + _slen].to_vec())?;
			_cur += _slen;
			_val
		}},
	}
}

// ── parse_java_source — merged import split + return-type parse ───────────────

/// Output of the unified Java source parser.
struct ParsedJava {
	/// The import/package section verbatim from the original source.
	imports: String,
	/// Any class/interface/enum declarations written before `run()`.
	/// Emitted as top-level Java types, outside the generated wrapper class.
	outer: String,
	/// The `run()` method and everything after it, verbatim from the original source.
	/// Placed inside the generated wrapper class.
	body: String,
	/// Parameters declared in `run(...)`, in order.
	params: Vec<(JavaType, String)>,
	/// Return type of the `static T run(...)` method.
	java_type: JavaType,
}

/// Recursively parse a `JavaType` from `tts` starting at index 0.
/// Returns `(java_type, tokens_consumed)` on success.
///
/// Recognises:
/// - Primitive: `T` where T is a Java primitive name (`int`, `double`, etc.)
/// - Boxed:     `String` at top level
/// - Array:     `T[]`, `T[][]`, … (Ident + one or more empty Bracket groups)
/// - List:      `List<T>` or `java.util.List<T>`
/// - Optional:  `Optional<T>` or `java.util.Optional<T>`
///
/// Inner types for List/Optional are parsed by `parse_java_type_inner`.
fn parse_java_type(tts: &[TokenTree]) -> Result<(JavaType, usize), String> {
	if tts.is_empty() {
		return Err("inline_java: unexpected end of tokens while parsing Java type".to_string());
	}

	// Handle `java.util.` qualified prefix — skip 4 tokens: `java`, `.`, `util`, `.`
	let (tts, offset) = if matches!(&tts[0], TokenTree::Ident(id) if id == "java")
		&& matches!(tts.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '.')
		&& matches!(tts.get(2), Some(TokenTree::Ident(id)) if id == "util")
		&& matches!(tts.get(3), Some(TokenTree::Punct(p)) if p.as_char() == '.')
	{
		(&tts[4..], 4usize)
	} else {
		(tts, 0usize)
	};

	match tts.first() {
		Some(TokenTree::Ident(id)) => {
			let name = id.to_string();
			match name.as_str() {
				"List" | "Optional" => {
					// Expect `<` inner_type `>`
					if !matches!(tts.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
						return Err(format!(
							"inline_java: expected `<` after `{name}`"
						));
					}
					// Parse inner type recursively starting at index 2
					let (inner_ty, inner_consumed) = parse_java_type_inner(&tts[2..])?;
					// After inner type, we need `>`
					let close_idx = 2 + inner_consumed;
					if !matches!(tts.get(close_idx), Some(TokenTree::Punct(p)) if p.as_char() == '>') {
						return Err(format!(
							"inline_java: expected `>` to close `{name}<...>`"
						));
					}
					let total_consumed = offset + close_idx + 1;
					if name == "List" {
						Ok((JavaType::List(Box::new(inner_ty)), total_consumed))
					} else {
						Ok((JavaType::Optional(Box::new(inner_ty)), total_consumed))
					}
				}
				_ => {
					// Try primitive name first, then String
					let mut consumed = offset + 1;
					let base_ty = if let Some(p) = PrimitiveType::from_name(&name) {
						JavaType::Primitive(p)
					} else if name == "String" {
						JavaType::Boxed(BoxedType::String)
					} else {
						return Err(format!(
							"inline_java: `{name}` is not a supported Java type; \
							 scalar types: byte short int long float double boolean char String"
						));
					};

					// Consume any trailing `[]` bracket groups, wrapping in Array each time.
					let mut ty = base_ty;
					while matches!(
						tts.get(consumed - offset),
						Some(TokenTree::Group(g))
							if g.delimiter() == proc_macro2::Delimiter::Bracket
							   && g.stream().is_empty()
					) {
						ty = JavaType::Array(Box::new(ty));
						consumed += 1;
					}
					Ok((ty, consumed))
				}
			}
		}
		_ => Err("inline_java: expected a Java type name".to_string()),
	}
}

/// Like `parse_java_type` but uses boxed names for scalars (for use inside `<>`).
/// This allows writing `List<Integer>` / `List<Integer[]>` etc.
/// Also accepts primitive names (`int`, `byte`, …) and maps them to boxed equivalents.
fn parse_java_type_inner(tts: &[TokenTree]) -> Result<(JavaType, usize), String> {
	if tts.is_empty() {
		return Err("inline_java: unexpected end of tokens while parsing Java type argument".to_string());
	}

	// Handle `java.util.` qualified prefix
	let (tts, offset) = if matches!(&tts[0], TokenTree::Ident(id) if id == "java")
		&& matches!(tts.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '.')
		&& matches!(tts.get(2), Some(TokenTree::Ident(id)) if id == "util")
		&& matches!(tts.get(3), Some(TokenTree::Punct(p)) if p.as_char() == '.')
	{
		(&tts[4..], 4usize)
	} else {
		(tts, 0usize)
	};

	match tts.first() {
		Some(TokenTree::Ident(id)) => {
			let name = id.to_string();
			match name.as_str() {
				"List" | "Optional" => {
					// Recursive container inside generics
					if !matches!(tts.get(1), Some(TokenTree::Punct(p)) if p.as_char() == '<') {
						return Err(format!(
							"inline_java: expected `<` after `{name}`"
						));
					}
					let (inner_ty, inner_consumed) = parse_java_type_inner(&tts[2..])?;
					let close_idx = 2 + inner_consumed;
					if !matches!(tts.get(close_idx), Some(TokenTree::Punct(p)) if p.as_char() == '>') {
						return Err(format!(
							"inline_java: expected `>` to close `{name}<...>`"
						));
					}
					let total_consumed = offset + close_idx + 1;
					if name == "List" {
						Ok((JavaType::List(Box::new(inner_ty)), total_consumed))
					} else {
						Ok((JavaType::Optional(Box::new(inner_ty)), total_consumed))
					}
				}
				_ => {
					// Inside `<>`, use boxed names (Integer, not int)
					let b = BoxedType::from_name(&name).ok_or_else(|| {
						format!(
							"inline_java: `{name}` is not a supported Java type argument; \
							 supported: Byte Short Integer Long Float Double Boolean Character String \
							 (or primitive names)"
						)
					})?;
					let mut consumed = offset + 1;
					let base_ty = JavaType::Boxed(b);

					// Consume any trailing `[]` bracket groups, wrapping in Array each time.
					while matches!(
						tts.get(consumed - offset),
						Some(TokenTree::Group(g))
							if g.delimiter() == proc_macro2::Delimiter::Bracket
							   && g.stream().is_empty()
					) {
						consumed += 1;
					}
					// Build Array wrapping by counting how many [] we consumed.
					let array_depth = consumed - offset - 1;
					let mut ty = base_ty;
					for _ in 0..array_depth {
						ty = JavaType::Array(Box::new(ty));
					}
					Ok((ty, consumed))
				}
			}
		}
		_ => Err("inline_java: expected a Java type name inside `<>`".to_string()),
	}
}

/// Scan `tts` for the first `[visibility] static <T> run` pattern and return the
/// corresponding `JavaType` together with the index of the first token of the
/// method declaration within `tts` (the visibility modifier if present, otherwise
/// `static`), and the index of the `run` identifier token.
///
/// Returns `(java_type, method_start_idx, run_idx)`.
///
/// The visibility modifier (`public`, `private`, `protected`) is optional; plain
/// `static <T> run()` is accepted in addition to `public static <T> run()`.
fn parse_run_return_type(tts: &[TokenTree]) -> Result<(JavaType, usize, usize), String> {
	for i in 0..tts.len().saturating_sub(2) {
		if !matches!(&tts[i], TokenTree::Ident(id) if id == "static") {
			continue;
		}

		// Include an optional preceding visibility modifier in the returned start index.
		let start = if i > 0
			&& matches!(&tts[i - 1], TokenTree::Ident(id)
				if matches!(id.to_string().as_str(), "public" | "private" | "protected"))
		{
			i - 1
		} else {
			i
		};

		// Try to parse a JavaType starting at tts[i+1]
		let type_start = i + 1;
		if type_start >= tts.len() {
			continue;
		}

		match parse_java_type(&tts[type_start..]) {
			Ok((java_type, consumed)) => {
				let run_idx = type_start + consumed;
				if matches!(tts.get(run_idx), Some(TokenTree::Ident(id)) if id == "run") {
					return Ok((java_type, start, run_idx));
				}
				// consumed tokens didn't lead to `run` — continue scanning
			}
			Err(_) => {
				// Not a valid type here — continue scanning
				continue;
			}
		}
	}
	Err("inline_java: could not find `static <type> run()` in Java body".to_string())
}

/// Parse the parameter list from the `Group(Parenthesis)` token immediately
/// after the `run` identifier.  Returns `Vec<(JavaType, param_name)>`.
///
/// Empty group → `Ok(vec![])`.
/// Unknown/unsupported type → `Err(...)` with a helpful message.
fn parse_run_params(tts: &[TokenTree]) -> Result<Vec<(JavaType, String)>, String> {
	// tts[0] must be the parenthesis group immediately after `run`.
	let group = match tts.first() {
		Some(TokenTree::Group(g)) if g.delimiter() == proc_macro2::Delimiter::Parenthesis => g,
		_ => return Ok(vec![]),
	};

	let inner: Vec<TokenTree> = group.stream().into_iter().collect();
	if inner.is_empty() {
		return Ok(vec![]);
	}

	// Split on ',' to get segments, one per parameter.
	// Note: commas inside angle brackets (e.g. Map<K,V>) would need special
	// handling, but we don't support those types so simple splitting is fine.
	let mut params = Vec::new();
	let mut segments: Vec<Vec<TokenTree>> = Vec::new();
	let mut current: Vec<TokenTree> = Vec::new();
	let mut angle_depth = 0i32;
	for tt in inner {
		if matches!(&tt, TokenTree::Punct(p) if p.as_char() == '<') {
			angle_depth += 1;
			current.push(tt);
		} else if matches!(&tt, TokenTree::Punct(p) if p.as_char() == '>') {
			angle_depth -= 1;
			current.push(tt);
		} else if matches!(&tt, TokenTree::Punct(p) if p.as_char() == ',') && angle_depth == 0 {
			segments.push(std::mem::take(&mut current));
		} else {
			current.push(tt);
		}
	}
	if !current.is_empty() {
		segments.push(current);
	}

	for seg in segments {
		if seg.is_empty() {
			continue;
		}

		// The last token in the segment is the parameter name (an Ident).
		// Everything before it is the type.
		let param_name = match seg.last() {
			Some(TokenTree::Ident(id)) => id.to_string(),
			_ => {
				return Err(
					"inline_java: unexpected token in run() parameter list: expected a parameter name"
						.to_string(),
				);
			}
		};

		// Parse the type from all tokens except the last (the param name).
		let type_tts = &seg[..seg.len() - 1];
		if type_tts.is_empty() {
			return Err(format!(
				"inline_java: missing type for parameter `{param_name}`"
			));
		}

		let (java_type, consumed) = parse_java_type(type_tts).map_err(|e| {
			format!("inline_java: error parsing type of parameter `{param_name}`: {e}")
		})?;

		// Make sure we consumed all type tokens (consumed should equal type_tts.len())
		if consumed != type_tts.len() {
			return Err(format!(
				"inline_java: unexpected tokens after type of parameter `{param_name}`"
			));
		}

		params.push((java_type, param_name));
	}

	Ok(params)
}

/// Unified parser: walks the token stream once to separate `import`/`package`
/// directives from the method body, identify the `run()` return type and
/// parameters.
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

	// Parse return type and run index from body tokens.
	let (java_type, run_rel_idx, run_rel_run_idx) =
		parse_run_return_type(&tts[body_start..])?;
	let run_abs_idx = body_start + run_rel_idx;
	let run_token_abs_idx = body_start + run_rel_run_idx;

	// Parse run() parameters from the token immediately after `run`.
	let params = parse_run_params(&tts[run_token_abs_idx + 1..])?;

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

	// outer: any tokens between the end of imports and the `run` method declaration
	let outer = slice_text(body_start, run_abs_idx);

	// body: from the `run` method declaration to end (verbatim, no substitution).
	let body = if run_abs_idx < tts.len() {
		let start_span = tts[run_abs_idx].span();
		let end_span = tts.last().unwrap().span();

		match start_span.join(end_span).and_then(|s| s.source_text()) {
			Some(raw) => raw,
			None => tts[run_abs_idx..]
				.iter()
				.map(std::string::ToString::to_string)
				.collect::<Vec<_>>()
				.join(" "),
		}
	} else {
		String::new()
	};

	Ok(ParsedJava {
		imports,
		outer,
		body,
		params,
		java_type,
	})
}

// Shared code-generation helper used by both java! and java_fn!

/// Generate a `fn __java_runner(...) -> Result<T, JavaError>` token stream
/// used by both `java!` and `java_fn!`.  The caller decides whether to emit
/// `__java_runner()` (immediate call, `java!`) or `__java_runner` (return
/// the function, `java_fn!`).
fn make_runner_fn(
	parsed: ParsedJava,
	opts: JavaOpts,
	prefix: &str,
) -> proc_macro2::TokenStream {
	let ParsedJava {
		imports,
		outer,
		body,
		params,
		java_type,
	} = parsed;

	let class_name = make_class_name(prefix, &imports, &outer, &body, &opts);
	let filename = format!("{class_name}.java");
	let full_class_name = qualify_class_name(&class_name, &imports);

	let main_method = java_type.java_main(&params);
	let java_class = format_java_class(&imports, &outer, &class_name, &body, &main_method);

	let javac_raw = opts.javac_args.unwrap_or_default();
	let java_raw = opts.java_args.unwrap_or_default();
	let deser = java_type.rust_deser();
	let ret_ty = java_type.rust_return_type_ts();

	// Build Rust parameter list for the generated function signature.
	// String params use `&str`.
	let fn_params: Vec<proc_macro2::TokenStream> = params
		.iter()
		.map(|(ty, name)| {
			let ident = proc_macro2::Ident::new(name, proc_macro2::Span::call_site());
			let param_ty = ty.rust_param_type_ts();
			quote! { #ident: #param_ty }
		})
		.collect();

	// Build serialization statements for each parameter.
	let ser_stmts: Vec<proc_macro2::TokenStream> = params
		.iter()
		.map(|(ty, name)| {
			let ident = proc_macro2::Ident::new(name, proc_macro2::Span::call_site());
			let ident_ts = quote! { #ident };
			ty.rust_ser_ts(&ident_ts, 0)
		})
		.collect();

	quote! {
		fn __java_runner(#(#fn_params),*) -> ::std::result::Result<#ret_ty, ::inline_java::JavaError> {
			let mut _stdin_bytes: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
			#(#ser_stmts)*
			let _raw = ::inline_java::run_java(
				#class_name,
				#filename,
				#java_class,
				#full_class_name,
				#javac_raw,
				#java_raw,
				&_stdin_bytes,
			)?;
			::std::result::Result::Ok(#deser)
		}
	}
}

/// Compile and run zero-argument Java code at *program runtime*.
///
/// Wraps the provided Java body in a generated class, compiles it with `javac`,
/// and executes it with `java`.  The return value of `static T run()` is
/// binary-serialised by the generated `main()` and deserialised to the inferred
/// Rust type.
///
/// Expands to `Result<T, inline_java::JavaError>`, so callers can propagate
/// errors with `?` or surface them with `.unwrap()`.
///
/// For `run()` methods that take parameters, use [`java_fn!`] instead.
///
/// # Options
///
/// Optional `key = "value"` pairs may appear before the Java body, separated by
/// commas:
///
/// - `javac = "<args>"` — extra arguments for `javac` (shell-quoted).
/// - `java  = "<args>"` — extra arguments for `java` (shell-quoted).
///
/// `$INLINE_JAVA_CP` in either option expands to the class-output directory.
///
/// # Examples
///
/// ```text
/// use inline_java::java;
///
/// // Scalar return value
/// let x: i32 = java! {
///     static int run() {
///         return 42;
///     }
/// }.unwrap();
///
/// // Array return
/// let primes: Vec<i32> = java! {
///     static int[] run() {
///         return new int[]{2, 3, 5, 7, 11};
///     }
/// }.unwrap();
///
/// // Extra javac flags
/// let greeting: String = java! {
///     javac = "-sourcepath .",
///     import com.example.demo.*;
///     static String run() {
///         return new HelloWorld().greet();
///     }
/// }.unwrap();
///
/// // Visibility modifiers are accepted — `public`, `private`, `protected` all work
/// let v: i32 = java! {
///     public static int run() { return 99; }
/// }.unwrap();
/// ```
#[proc_macro]
#[allow(clippy::similar_names)]
pub fn java(input: TokenStream) -> TokenStream {
	let input2 = proc_macro2::TokenStream::from(input);

	// Consume any leading `key = "value",` option pairs.
	let (opts, input2) = extract_opts(input2);

	let parsed = match parse_java_source(input2) {
		Ok(p) => p,
		Err(msg) => return quote! { compile_error!(#msg) }.into(),
	};

	let runner_fn = make_runner_fn(parsed, opts, "InlineJava");

	let generated = quote! {
		{
			#runner_fn
			__java_runner()
		}
	};

	generated.into()
}

/// Return a typed Rust function that compiles and runs Java at *program runtime*.
///
/// Like [`java!`], but supports parameters.  The parameters declared in the
/// Java `run(P1 p1, P2 p2, ...)` method become the Rust function's parameters.
/// Arguments are serialised by Rust and piped to the Java process via stdin;
/// Java reads them with `DataInputStream`.
///
/// Expands to a function value of type `fn(P1, P2, ...) -> Result<T, JavaError>`.
/// Call it immediately or store it in a variable.
///
/// # Supported parameter types
///
/// | Java type              | Rust type           |
/// |------------------------|---------------------|
/// | `byte`                 | `i8`                |
/// | `short`                | `i16`               |
/// | `int`                  | `i32`               |
/// | `long`                 | `i64`               |
/// | `float`                | `f32`               |
/// | `double`               | `f64`               |
/// | `boolean`              | `bool`              |
/// | `char`                 | `char`              |
/// | `String`               | `&str`              |
/// | `Optional<BoxedT>`     | `Option<T>`         |
/// | `Optional<String>`     | `Option<&str>`      |
///
/// # Options
///
/// Same `javac = "..."` / `java = "..."` key-value pairs as [`java!`].
///
/// # Examples
///
/// ```text
/// use inline_java::java_fn;
///
/// // Single int parameter
/// let double_it = java_fn! {
///     static int run(int n) {
///         return n * 2;
///     }
/// };
/// let result: i32 = double_it(21).unwrap();
/// assert_eq!(result, 42);
///
/// // Multiple parameters including String
/// let greet = java_fn! {
///     static String run(String greeting, String target) {
///         return greeting + ", " + target + "!";
///     }
/// };
/// let msg: String = greet("Hello", "World").unwrap();
/// assert_eq!(msg, "Hello, World!");
/// ```
#[proc_macro]
#[allow(clippy::similar_names)]
pub fn java_fn(input: TokenStream) -> TokenStream {
	let input2 = proc_macro2::TokenStream::from(input);

	// Consume any leading `key = "value",` option pairs.
	let (opts, input2) = extract_opts(input2);

	let parsed = match parse_java_source(input2) {
		Ok(p) => p,
		Err(msg) => return quote! { compile_error!(#msg) }.into(),
	};

	let runner_fn = make_runner_fn(parsed, opts, "InlineJava");

	let generated = quote! {
		{
			#runner_fn
			__java_runner
		}
	};

	generated.into()
}

// ct_java! — compile-time Java evaluation

/// Run Java at *compile time* and splice its return value as a Rust literal.
///
/// Accepts optional `javac = "..."` and `java = "..."` key-value pairs before
/// the Java body.  The user provides a `static <T> run()` method; its
/// binary-serialised return value is decoded and emitted as a Rust literal at
/// the call site (`42`, `3.14`, `true`, `'x'`, `"hello"`, `[1, 2, 3]`, …).
///
/// Java compilation/runtime errors become Rust `compile_error!` diagnostics.
///
/// # Examples
///
/// ```text
/// use inline_java::ct_java;
///
/// // Numeric constant computed at compile time
/// const PI_APPROX: f64 = ct_java! {
///     static double run() {
///         return Math.PI;
///     }
/// };
///
/// // String constant
/// const GREETING: &str = ct_java! {
///     static String run() {
///         return "Hello, World!";
///     }
/// };
///
/// // Array constant
/// const PRIMES: [i32; 5] = ct_java! {
///     static int[] run() {
///         return new int[]{2, 3, 5, 7, 11};
///     }
/// };
/// ```
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

	let main_method = java_type.java_main(&[]);
	let java_class = format_java_class(&imports, &outer, &class_name, &body, &main_method);

	let bytes = compile_run_java_now(
		&class_name,
		&filename,
		&java_class,
		&full_class_name,
		opts.javac_args.as_deref(),
		opts.java_args.as_deref(),
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

// Shared helpers used by java!, java_fn!, and ct_java!

/// Compute a deterministic class name by hashing the source and options.
/// `prefix` distinguishes runtime ("`InlineJava`") from compile-time ("`CtJava`").
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
#[allow(clippy::similar_names)]
fn compile_run_java_now(
	class_name: &str,
	filename: &str,
	java_class: &str,
	full_class_name: &str,
	javac_raw: Option<&str>,
	java_raw: Option<&str>,
) -> Result<Vec<u8>, String> {
	inline_java_core::run_java(
		class_name,
		filename,
		java_class,
		full_class_name,
		javac_raw.unwrap_or(""),
		java_raw.unwrap_or(""),
		&[],
	)
	.map_err(|e| e.to_string())
}

/// Render the complete `.java` source file.
fn format_java_class(
	imports: &str,
	outer: &str,
	class_name: &str,
	body: &str,
	main_method: &str,
) -> String {
	format!(
		"{imports}\n{outer}\npublic class {class_name} {{\n\n{body}\n\n{main_method}\n}}\n"
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
