# inline_java

Embed Java directly in Rust — evaluated at program runtime (`java!`, `java_fn!`) or at
compile time (`ct_java!`).

## Prerequisites

Java 8+ with `javac` and `java` on `PATH`.

## Quick start

```toml
# Cargo.toml
[dependencies]
inline_java = "0.1.0"
```

## `java!` — runtime, no parameters

Compiles and runs Java each time the surrounding Rust code executes.  Expands
to `Result<T, inline_java::JavaError>`.

```rust
use inline_java::java;

// No type annotation needed — the macro infers `i32` from `static int run()`
let x = java! {
    static int run() {
        return 42;
    }
}.unwrap();
```

## `java_fn!` — runtime, with parameters

Like `java!`, but `run(...)` may declare parameters.  Expands to a Rust
function value `fn(P1, P2, …) -> Result<T, JavaError>`.  Parameters are
serialised by Rust and piped to the Java process over stdin.

```rust
use inline_java::java_fn;

// Single parameter — return type inferred from `static int run()`
let doubled = java_fn! {
    static int run(int n) {
        return n * 2;
    }
}(21).unwrap();

// Multiple parameters
let msg: String = java_fn! {
    static String run(String greeting, String target) {
        return greeting + ", " + target + "!";
    }
}("Hello", "World").unwrap();

// Optional parameter
use inline_java::java_fn;
let result: Option<i32> = java_fn! {
    import java.util.Optional;
    static Optional<Integer> run(Optional<Integer> val) {
        return val.map(x -> x * 2);
    }
}(Some(21)).unwrap();
```

## `ct_java!` — compile time

Runs Java during `rustc` macro expansion and splices the result as a Rust
literal at the call site.  No parameters are allowed (values must be
compile-time constants).

```rust
use inline_java::ct_java;

const PI: f64 = ct_java! {
    static double run() {
        return Math.PI;
    }
};

// Arrays work too — result is a Rust array literal baked into the binary
const PRIMES: [i32; 5] = ct_java! {
    static int[] run() {
        return new int[]{2, 3, 5, 7, 11};
    }
};
```

## Supported parameter types (`java_fn!`)

Declare parameters in the Java `run(...)` signature; Rust receives them with
the mapped types below.

| Java parameter type    | Rust parameter type  |
|------------------------|----------------------|
| `byte`                 | `i8`                 |
| `short`                | `i16`                |
| `int`                  | `i32`                |
| `long`                 | `i64`                |
| `float`                | `f32`                |
| `double`               | `f64`                |
| `boolean`              | `bool`               |
| `char`                 | `char`               |
| `String`               | `&str`               |
| `T[]` / `List<BoxedT>` | `&[T]`               |
| `Optional<BoxedT>`     | `Option<T>`          |

## Supported return types

| Java return type       | Rust return type  |
|------------------------|-------------------|
| `byte`                 | `i8`              |
| `short`                | `i16`             |
| `int`                  | `i32`             |
| `long`                 | `i64`             |
| `float`                | `f32`             |
| `double`               | `f64`             |
| `boolean`              | `bool`            |
| `char`                 | `char`            |
| `String`               | `String`          |
| `T[]` / `List<BoxedT>` | `Vec<T>`          |
| `Optional<BoxedT>`     | `Option<T>`       |

Types can be nested arbitrarily: `Optional<List<Integer>>` → `Option<Vec<i32>>`,
`List<String[]>` → `Vec<Vec<String>>`, etc.

## Options

The following optional `key = "value"` pairs may appear before the Java body, separated by
commas:

- `javac = "<args>"` — extra arguments for `javac` (shell-quoted).
- `java  = "<args>"` — extra arguments for `java` (shell-quoted).

```rust
use inline_java::java;

let result: String = java! {
    javac = "-cp ./my.jar",
    java  = "-cp ./my.jar",
    import com.example.MyClass;
    static String run() {
        return new MyClass().greet();
    }
}.unwrap();
```

## Using project Java source files

Use `import` or `package` directives together with `javac = "-sourcepath <path>"`
(or `-classpath`) to call into your own Java code:

```rust
// import style
let s: String = java! {
    javac = "-sourcepath .",
    import com.example.demo.*;
    static String run() {
        return new HelloWorld().greet();
    }
}.unwrap();

// package style — the generated class becomes part of the named package
let s: String = java! {
    javac = "-sourcepath .",
    package com.example.demo;
    static String run() {
        return new HelloWorld().greet();
    }
}.unwrap();
```

## Refactoring use case

`inline_java` is particularly well-suited for **incremental Java → Rust
migrations**.  The typical workflow is:

1. Keep the original Java logic intact.
2. Write the replacement in Rust.
3. Use `java_fn!` to call the original Java with the same inputs and assert
   that both implementations produce identical outputs.

```rust
use inline_java::java_fn;

fn my_rust_impl(n: i32) -> i32 {
    // … new Rust code …
    n * 2
}

#[test]
fn parity_with_java() {
    let java_impl = java_fn! {
        static int run(int n) {
            // original Java logic, verbatim
            return n * 2;
        }
    };

    for n in [0, 1, -1, 42, i32::MAX / 2] {
        let expected = java_impl(n).unwrap();
        assert_eq!(my_rust_impl(n), expected, "diverged for n={n}");
    }
}
```

## Crate layout

| Crate                | Purpose                                                     |
|----------------------|-------------------------------------------------------------|
| `inline_java`        | Public API — re-exports macros and core types               |
| `inline_java_macros` | Proc-macro implementation (`java!`, `java_fn!`, `ct_java!`) |
| `inline_java_core`   | Runtime helpers (`run_java`, `JavaError`)                   |
| `inline_java_demo`   | Demo binary                                                 |

---

<!-- TODO: move integration tests from `inline_java_demo/tests/` to `inline_java/tests/` so they live in the published crate -->

<!-- TODO: verify Java 8+ compatibility before releasing — test with OpenJDK 8 and update the Prerequisites section accordingly -->

<!-- TODO: consider replacing the doc-comment text-walls in `inline_java/src/lib.rs` and `inline_java_core/src/lib.rs` with `#[doc = include_str!("../../README.md")]` (and a crate-specific variant for the support crates) to keep docs in sync with the README automatically -->
