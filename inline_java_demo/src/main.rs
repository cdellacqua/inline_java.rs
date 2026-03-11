use inline_java::{ct_java, java, java_fn};
use std::process::Command;

fn build_demo_jar(jar_path: &str) {
	let manifest_dir = env!("CARGO_MANIFEST_DIR");
	let classes_dir = format!("{jar_path}.d");
	std::fs::create_dir_all(&classes_dir).expect("create classes dir");
	let status = Command::new("javac")
		.args([
			"-d",
			&classes_dir,
			&format!("{manifest_dir}/com/example/demo/Greetings.java"),
			&format!("{manifest_dir}/com/example/demo/HelloWorld.java"),
		])
		.status()
		.expect("javac");
	assert!(status.success(), "javac failed building demo jar");
	let status = Command::new("jar")
		.args(["cf", jar_path, "-C", &classes_dir, "."])
		.status()
		.expect("jar");
	assert!(status.success(), "jar failed building demo jar");
}

#[allow(clippy::too_many_lines)]
fn main() {
	// runtime, no input
	let x: i32 = java! {
		import java.util.concurrent.ThreadLocalRandom;

		static int run() {
			return ThreadLocalRandom.current().nextInt(0, 10);
		}
	}
	.unwrap();
	println!("Random from Java: {x}");

	// runtime, with int parameter (java_fn!)
	let n: i32 = 21;
	let doubled: i32 = java_fn! {
		static int run(int n) {
			return n * 2;
		}
	}(n)
	.unwrap();
	println!("{n} * 2 = {doubled}");

	// runtime, multiple String parameters (java_fn!)
	let greeting = "Hello";
	let target = "World";
	let msg: String = java_fn! {
		static String run(String greeting, String target) {
			return greeting + ", " + target + "!";
		}
	}(greeting, target)
	.unwrap();
	println!("{msg}");

	// compile-time constant
	println!("PI (baked at compile time): {PI_APPROX}");

	let imports: String = java! {
		javac = "-sourcepath .",
		import com.example.demo.*;

		static String run() {
			return new HelloWorld().greet();
		}
	}
	.unwrap();
	println!("{imports}");

	build_demo_jar("/tmp/inline_java_demo_jar/demo.jar");
	let imports_jar: String = java! {
		javac = "-classpath /tmp/inline_java_demo_jar/demo.jar",
		java = "-classpath /tmp/inline_java_demo_jar/demo.jar",
		import com.example.demo.*;

		static String run() {
			return new HelloWorld().greet();
		}
	}
	.unwrap();
	println!("{imports_jar}");

	let package: String = java! {
		javac = "-sourcepath .",
		package com.example.demo;

		static String run() {
			return new HelloWorld().greet();
		}
	}
	.unwrap();
	println!("{package}");

	// explicit javac + java flags (runtime)
	let explicit: String = java! {
		javac = "-sourcepath .",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet();
		}
	}
	.unwrap();
	println!("explicit javac sourcepath (java!): {explicit}");

	// explicit javac + java flags (compile-time)
	println!("explicit javac + java flags (ct_java!): {EXPLICIT_CT}");

	// runtime int[]
	let nums: Vec<i32> = java! {
		static int[] run() {
			return new int[]{10, 20, 30, 40, 50};
		}
	}
	.unwrap();
	println!("int[] from Java: {nums:?}");

	// runtime List<String>
	let words: Vec<String> = java! {
		import java.util.Arrays;
		import java.util.List;
		static List<String> run() {
			return Arrays.asList("alpha", "beta", "gamma");
		}
	}
	.unwrap();
	println!("List<String> from Java: {words:?}");

	// compile-time int array baked into binary
	println!("first 5 primes (ct_java): {PRIMES:?}");

	// compile-time String array baked into binary
	println!("days (ct_java): {DAYS:?}");

	// compile-time Optional constants
	assert_eq!(OPT_INT_SOME, Some(99));
	assert_eq!(OPT_INT_NONE, None::<i32>);
	assert_eq!(OPT_STR_SOME, Some("hello"));
	assert_eq!(OPT_STR_NONE, None::<&str>);
	println!("ct_java Optional<Integer> Some: {OPT_INT_SOME:?}");
	println!("ct_java Optional<Integer> None: {OPT_INT_NONE:?}");
	println!("ct_java Optional<String> Some: {OPT_STR_SOME:?}");
	println!("ct_java Optional<String> None: {OPT_STR_NONE:?}");

	// runtime Optional<Integer> return — present
	let opt_int_some: Option<i32> = java! {
		import java.util.Optional;
		static Optional<Integer> run() {
			return Optional.of(42);
		}
	}
	.unwrap();
	assert_eq!(opt_int_some, Some(42));
	println!("Optional<Integer> present: {opt_int_some:?}");

	// runtime Optional<Integer> return — empty
	let opt_int_none: Option<i32> = java! {
		import java.util.Optional;
		static Optional<Integer> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(opt_int_none, None);
	println!("Optional<Integer> empty: {opt_int_none:?}");

	// runtime Optional<String> return — present
	let opt_str_some: Option<String> = java! {
		import java.util.Optional;
		static Optional<String> run() {
			return Optional.of("world");
		}
	}
	.unwrap();
	assert_eq!(opt_str_some.as_deref(), Some("world"));
	println!("Optional<String> present: {opt_str_some:?}");

	// runtime Optional<String> return — empty
	let opt_str_none: Option<String> = java! {
		import java.util.Optional;
		static Optional<String> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(opt_str_none, None);
	println!("Optional<String> empty: {opt_str_none:?}");

	// Optional<Integer> parameter — Some
	let result_some: Option<i32> = java_fn! {
		import java.util.Optional;
		static Optional<Integer> run(Optional<Integer> val) {
			return val.map(x -> x * 2);
		}
	}(Some(21))
	.unwrap();
	assert_eq!(result_some, Some(42));
	println!("Optional<Integer> param Some -> {result_some:?}");

	// Optional<Integer> parameter — None
	let result_none: Option<i32> = java_fn! {
		import java.util.Optional;
		static Optional<Integer> run(Optional<Integer> val) {
			return val.map(x -> x * 2);
		}
	}(None)
	.unwrap();
	assert_eq!(result_none, None);
	println!("Optional<Integer> param None -> {result_none:?}");

	// Optional<String> parameter — Some
	let result_str_some: Option<String> = java_fn! {
		import java.util.Optional;
		static Optional<String> run(Optional<String> val) {
			return val.map(s -> s.toUpperCase());
		}
	}(Some("hello"))
	.unwrap();
	assert_eq!(result_str_some.as_deref(), Some("HELLO"));
	println!("Optional<String> param Some -> {result_str_some:?}");

	// Optional<String> parameter — None
	let result_str_none: Option<String> = java_fn! {
		import java.util.Optional;
		static Optional<String> run(Optional<String> val) {
			return val.map(s -> s.toUpperCase());
		}
	}(None)
	.unwrap();
	assert_eq!(result_str_none, None);
	println!("Optional<String> param None -> {result_str_none:?}");
}

// compile-time with explicit javac and java flags
const EXPLICIT_CT: i32 = ct_java! {
	javac = "-sourcepath /tmp",
	java = "-Xss512k",
	static int run() {
		return 1 + 1;
	}
};

// Compile-time constant: evaluated during rustc macro expansion
// Math.PI is baked into the binary; java is never invoked at runtime for this.
#[allow(clippy::approx_constant)]
const PI_APPROX: f64 = ct_java! {
	static double run() {
		return Math.PI;
	}
};

// compile-time int array
const PRIMES: [i32; 5] = ct_java! {
	static int[] run() {
		return new int[]{2, 3, 5, 7, 11};
	}
};

// compile-time String array
const DAYS: [&str; 3] = ct_java! {
	static String[] run() {
		return new String[]{"Mon", "Tue", "Wed"};
	}
};

// compile-time Optional<Integer> — present
const OPT_INT_SOME: Option<i32> = ct_java! {
	import java.util.Optional;
	static Optional<Integer> run() {
		return Optional.of(99);
	}
};

// compile-time Optional<Integer> — empty
const OPT_INT_NONE: Option<i32> = ct_java! {
	import java.util.Optional;
	static Optional<Integer> run() {
		return Optional.empty();
	}
};

// compile-time Optional<String> — present
const OPT_STR_SOME: Option<&str> = ct_java! {
	import java.util.Optional;
	static Optional<String> run() {
		return Optional.of("hello");
	}
};

// compile-time Optional<String> — empty
const OPT_STR_NONE: Option<&str> = ct_java! {
import java.util.Optional;
static Optional<String> run() {
	return Optional.empty();
}};
