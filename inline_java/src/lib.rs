//! Embed Java directly in Rust — evaluated at program runtime ([`java!`]) or
//! at compile time ([`ct_java!`]).
//!
//! # Runtime usage
//!
//! [`java!`] compiles and runs Java each time the surrounding Rust code
//! executes.  It expands to `Result<T, `[`JavaError`]`>`.
//!
//! ```rust,no_run
//! use inline_java::java;
//!
//! let x: i32 = java! {
//!     static int run() {
//!         return 42;
//!     }
//! }.unwrap();
//! ```
//!
//! # Compile-time usage
//!
//! [`ct_java!`] runs Java during `rustc` macro expansion and splices the
//! result as a Rust literal at the call site.
//!
//! ```rust,no_run
//! use inline_java::ct_java;
//!
//! const PI: f64 = ct_java! {
//!     static double run() {
//!         return Math.PI;
//!     }
//! };
//! ```
//!
//! # Supported return types
//!
//! `byte`/`short`/`int`/`long`/`float`/`double`/`boolean`/`char`/`String`
//! map to the obvious Rust types.  `T[]` and `List<BoxedT>` both map to
//! `Vec<T>`.
//!
//! # Variable injection
//!
//! Inject Rust variables into `java!` using `'varname` syntax.  Each
//! `'varname` becomes the Java `String _RUST_varname` static field.
//!
//! ```rust,no_run
//! use inline_java::java;
//!
//! let n: i32 = 21;
//! let doubled: i32 = java! {
//!     static int run() {
//!         int value = Integer.parseInt('n);
//!         return value * 2;
//!     }
//! }.unwrap();
//! ```

/// Re-export the proc macros so users only need to depend on this crate.
pub use inline_java_macros::{ct_java, java};

/// Re-export the core error type and runtime helpers.
pub use inline_java_core::{JavaError, expand_java_args, run_java};
