use inline_java::{ct_java, java};

fn main() {
	// runtime, no input
	let x: i32 = java! {
		import java.util.concurrent.ThreadLocalRandom;

		public static int run() {
			return ThreadLocalRandom.current().nextInt(0, 10);
		}
	}.unwrap();
	println!("Random from Java: {x}");

	// runtime, with input ('var syntax)
	let n: i32 = 21;
	let doubled: i32 = java! {
		public static int run() {
			int value = Integer.parseInt('n);
			return value * 2;
		}
	}.unwrap();
	println!("{n} * 2 = {doubled}");

	// runtime, multiple inputs
	let greeting = "Hello";
	let target = "World";
	let msg: String = java! {
		public static String run() {
			return 'greeting + ", " + 'target + "!";
		}
	}.unwrap();
	println!("{msg}");

	// compile-time constant
	println!("PI (baked at compile time): {PI_APPROX}");

	let imports: String = java! {
		javac = "-sourcepath .",
		import com.example.demo.*;

		public static String run() {
			return new HelloWorld().greet();
		}
	}.unwrap();
	println!("{imports}");

	let package: String = java! {
		javac = "-sourcepath .",
		package com.example.demo;

		public static String run() {
			return new HelloWorld().greet();
		}
	}.unwrap();
	println!("{package}");

	// explicit javac + java flags (runtime)
	let explicit: String = java! {
		javac = "-sourcepath .",
		import com.example.demo.*;
		public static String run() {
			return new HelloWorld().greet();
		}
	}.unwrap();
	println!("explicit javac sourcepath (java!): {explicit}");

	// explicit javac + java flags (compile-time)
	println!("explicit javac + java flags (ct_java!): {EXPLICIT_CT}");

	// runtime int[]
	let nums: Vec<i32> = java! {
		public static int[] run() {
			return new int[]{10, 20, 30, 40, 50};
		}
	}.unwrap();
	println!("int[] from Java: {nums:?}");

	// runtime List<String>
	let words: Vec<String> = java! {
		import java.util.Arrays;
		import java.util.List;
		public static List<String> run() {
			return Arrays.asList("alpha", "beta", "gamma");
		}
	}.unwrap();
	println!("List<String> from Java: {words:?}");

	// compile-time int array baked into binary
	println!("first 5 primes (ct_java): {PRIMES:?}");

	// compile-time String array baked into binary
	println!("days (ct_java): {DAYS:?}");
}

// compile-time with explicit javac and java flags
const EXPLICIT_CT: i32 = ct_java! {
	javac = "-sourcepath /tmp",
	java = "-Xss512k",
	public static int run() {
		return 1 + 1;
	}
};

// Compile-time constant: evaluated during rustc macro expansion
// Math.PI is baked into the binary; java is never invoked at runtime for this.
#[allow(clippy::approx_constant)]
const PI_APPROX: f64 = ct_java! {
	public static double run() {
		return Math.PI;
	}
};

// compile-time int array
const PRIMES: [i32; 5] = ct_java! {
	public static int[] run() {
		return new int[]{2, 3, 5, 7, 11};
	}
};

// compile-time String array
const DAYS: [&str; 3] = ct_java! {
	public static String[] run() {
		return new String[]{"Mon", "Tue", "Wed"};
	}
};
