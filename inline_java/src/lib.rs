/// Re-export the proc macros so users only need to depend on this crate.
pub use inline_java_macros::{ct_java, java};

/// Re-export the core error type and runtime helpers.
pub use inline_java_core::{JavaError, expand_java_args, run_java};
