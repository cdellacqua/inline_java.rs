# inline_java

Embed Java directly in Rust — evaluated at program runtime (`java!`) or at
compile time (`ct_java!`).

## Prerequisites

Java 8+ with `javac` and `java` on `PATH`.

## Quick start

```toml
# Cargo.toml
[dependencies]
inline_java = { path = "inline_java" }
```

## `java!` — runtime

Compiles and runs Java each time the surrounding Rust code executes.  Expands
to `Result<T, inline_java::JavaError>`.

```rust
use inline_java::java;

let x: i32 = java! {
    static int run() {
        return 42;
    }
}.unwrap();
```

## `ct_java!` — compile time

Runs Java during `rustc` macro expansion and splices the result as a Rust
literal at the call site.

```rust
use inline_java::ct_java;

const PI: f64 = ct_java! {
    static double run() {
        return Math.PI;
    }
};
```

## Supported return types

| Java type       | Rust type   |
|-----------------|-------------|
| `byte`          | `i8`        |
| `short`         | `i16`       |
| `int`           | `i32`       |
| `long`          | `i64`       |
| `float`         | `f32`       |
| `double`        | `f64`       |
| `boolean`       | `bool`      |
| `char`          | `char`      |
| `String`        | `String`    |
| `T[]`           | `Vec<T>`    |
| `List<BoxedT>`  | `Vec<T>`    |

## Variable injection

Inject Rust variables into Java using `'varname` syntax.  Each `'varname`
becomes the Java `String _RUST_varname` static field, passed via `args[]`.

```rust
use inline_java::java;

let greeting = "Hello";
let target = "World";
let msg: String = java! {
    static String run() {
        return 'greeting + ", " + 'target + "!";
    }
}.unwrap();
```

Variable injection is only supported in `java!`; `ct_java!` has no runtime
variables to capture.

## Options

Optional `key = "value"` pairs may appear before the Java body, separated by
commas:

- `javac = "<args>"` — extra arguments for `javac` (shell-quoted).
- `java  = "<args>"` — extra arguments for `java` (shell-quoted).

The special variable `$INLINE_JAVA_CP` in either option expands to the
class-output directory for the generated class.

```rust
use inline_java::java;

let result: String = java! {
    javac = "-sourcepath .",
    import com.example.MyClass;
    static String run() {
        return new MyClass().greet();
    }
}.unwrap();
```

## Crate layout

| Crate                | Purpose                                             |
|----------------------|-----------------------------------------------------|
| `inline_java`        | Public API — re-exports macros and core types       |
| `inline_java_macros` | Proc-macro implementation (`java!`, `ct_java!`)     |
| `inline_java_core`   | Runtime helpers (`run_java`, `JavaError`)           |
| `inline_java_demo`   | Demo binary                                         |
