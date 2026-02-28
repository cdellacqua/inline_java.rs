// inline_java/src/lib.rs
//
// Two proc macros for embedding Java in Rust, inspired by `inline_python` /
// `ct_python`.
//
// ┌─────────────┬──────────┬───────────────┐
// │             │ runtime  │ compile-time  │
// ├─────────────┼──────────┼───────────────┤
// │ input       │ inline_java (via 'var) │ —  │
// │ output      │ inline_java            │ ct_java │
// └─────────────┴──────────┴───────────────┘
//
// java
// ────────────
// Runs Java at *program runtime*.  The user provides a `run()` method; the
// macro wraps it in a class, writes it to a temp file, spawns `java`, and
// parses the printed output back into the Rust return type.
//
// Rust variables can be injected using `'var` syntax (same convention as
// inline_python).  Each `'var` becomes the Java String `_RUST_var`, passed
// via args[].
//
//   let n = 42i32;
//   let s: String = java {
//       public static String run() {
//           int x = Integer.parseInt('n);
//           return "double is " + (x * 2);
//       }
//   };
//
// ct_java!
// ────────
// Runs Java at *compile time* (inside the proc-macro, while rustc is
// expanding macros).  Whatever the Java code prints to stdout becomes the
// Rust token stream at the call site.
//
// Use it wherever a Rust literal or expression is needed but the value is
// easier to compute in Java:
//
//   const PI_APPROX: f64 = ct_java! {
//       public static void run() {
//           System.out.println(Math.PI);
//       }
//   };
//
// Or to emit full Rust items:
//
//   ct_java! {
//       public static void run() {
//           System.out.println("static GREETING: &str = \"hello\";");
//       }
//   }

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fmt::Write as FmtWrite;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Spacing, TokenTree};
use quote::quote;

#[proc_macro]
pub fn java(input: TokenStream) -> TokenStream {
	let input2 = proc_macro2::TokenStream::from(input);

	// Replace 'var tokens with _RUST_var idents and collect the variable names.
	let (substituted, vars) = extract_vars(input2);

	// Split the substituted stream into import statements and the method body.
	let (imports_ts, body_ts) = split_imports(substituted);
	let imports = imports_ts.to_string();
	let body = body_ts.to_string();

	// Unique class name derived from the source content.
	let mut h = DefaultHasher::new();
	imports.hash(&mut h);
	body.hash(&mut h);
	let class_name = format!("InlineJava_{:016x}", h.finish());
	let filename = format!("{class_name}.java");

	// `static String _RUST_foo;` declarations, one per captured variable.
	let var_fields: String = vars.keys().fold(String::new(), |mut s, name| {
		writeln!(s, "\tstatic String _RUST_{name};").unwrap();
		s
	});

	// Assignments inside main: `_RUST_foo = args[0];` in alphabetical order.
	let var_inits: String =
		vars.keys()
			.enumerate()
			.fold(String::new(), |mut s, (i, name)| {
				writeln!(s, "\t\t_RUST_{name} = args[{i}];").unwrap();
				s
			});

	// Assemble the complete Java source file.
	let java_class = format!(
		"{imports}\npublic class {class_name} {{\n{var_fields}\n{body}\n\n\
         public static void main(String[] args) {{\n\
         {var_inits}\
         \t\tSystem.out.println(run());\n\
         }}\n}}\n"
	);

	// Idents for the captured Rust variables, in sorted order, used in quote!
	// to emit `.arg(var.to_string())` calls.
	let var_idents: Vec<Ident> = vars.values().cloned().collect();

	let generated = quote! {
		{
			// Java 11+ (JEP 330): `java Foo.java` compiles and runs in one step.
			let _src = ::std::env::temp_dir().join(#filename);
			::std::fs::write(&_src, #java_class)
				.expect("inline_java: failed to write .java source");

			let _java = ::std::process::Command::new("java")
				.arg(&_src)
				#(.arg(::std::string::ToString::to_string(&#var_idents)))*
				.output()
				.expect("inline_java: could not invoke java (is it on PATH?)");
			if !_java.status.success() {
				panic!(
					"inline_java: java failed:\n{}",
					::std::string::String::from_utf8_lossy(&_java.stderr)
				);
			}

			let _out = ::std::string::String::from_utf8(_java.stdout)
				.expect("inline_java: Java output was not valid UTF-8");
			// T is inferred from the binding at the call site, e.g. `let x: i32 = …`
			_out.trim().parse().expect("inline_java: could not parse Java output")
		}
	};

	generated.into()
}

// ---------------------------------------------------------------------------
// ct_java! — compile-time Java evaluation
// ---------------------------------------------------------------------------

/// Run Java at *compile time* and splice its stdout into the Rust token stream.
///
/// The Java code should contain a `public static void run()` method that
/// prints the desired Rust code/literal to stdout.  Whatever it prints is
/// parsed as a Rust token stream and substituted at the call site.
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
	let (imports_ts, body_ts) = split_imports(input);
	let imports = imports_ts.to_string();
	let body = body_ts.to_string();

	let mut h = DefaultHasher::new();
	imports.hash(&mut h);
	body.hash(&mut h);
	let class_name = format!("CtJava_{:016x}", h.finish());
	let filename = format!("{class_name}.java");

	// The user writes a `run()` method; main just calls it.
	let java_class = format!(
		"{imports}\npublic class {class_name} {{\n{body}\n\n\
         public static void main(String[] args) {{\n\
         \t\trun();\n\
         }}\n}}\n"
	);

	let src = std::env::temp_dir().join(&filename);
	std::fs::write(&src, &java_class)
		.map_err(|e| format!("ct_java: failed to write .java source: {e}"))?;

	let out = std::process::Command::new("java")
		.arg(&src)
		.output()
		.map_err(|e| format!("ct_java: could not invoke java (is it on PATH?): {e}"))?;

	if !out.status.success() {
		let stderr = String::from_utf8_lossy(&out.stderr);
		return Err(format!("ct_java: java failed:\n{stderr}"));
	}

	let stdout = String::from_utf8(out.stdout)
		.map_err(|_| "ct_java: Java output was not valid UTF-8".to_string())?;

	// The printed output IS the Rust token stream injected at the call site.
	proc_macro2::TokenStream::from_str(stdout.trim())
		.map_err(|e| format!("ct_java: produced invalid Rust code: {e}"))
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
		// Check for the `'` Joint punct that starts a `'var` capture.
		let is_quote_punct = matches!(
			&tt,
			TokenTree::Punct(p) if p.as_char() == '\'' && p.spacing() == Spacing::Joint
		);

		if is_quote_punct {
			// Only treat as a variable capture if the very next token is an ident.
			if matches!(iter.peek(), Some(TokenTree::Ident(_))) {
				let TokenTree::Ident(ident) = iter.next().unwrap() else {
					unreachable!()
				};
				let name = ident.to_string();
				let span = ident.span();
				vars.entry(name.clone()).or_insert_with(|| ident);
				output.push(TokenTree::Ident(Ident::new(&format!("_RUST_{name}"), span)));
			} else {
				// Not followed by an ident — leave the `'` as-is.
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
/// Detects `import …;` and `package …;` at the top level by looking for
/// an `import`/`package` identifier followed by tokens up to the next `;`.
fn split_imports(
	input: proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
	let mut iter = input.into_iter();
	let mut imports: Vec<TokenTree> = Vec::new();
	let mut body: Vec<TokenTree> = Vec::new();

	while let Some(tt) = iter.next() {
		let is_directive = matches!(
			&tt,
			TokenTree::Ident(id)
				if *id == "import" || *id == "package"
		);

		if is_directive {
			imports.push(tt);
			// Drain into imports up to and including the terminating `;`
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

	let import_ts: proc_macro2::TokenStream = imports.into_iter().collect();
	let body_ts: proc_macro2::TokenStream = body.into_iter().collect();
	(import_ts, body_ts)
}
