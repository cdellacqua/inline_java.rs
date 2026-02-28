use inline_java_macros::{ct_java, java};

fn main() {
	// ── runtime, no input ────────────────────────────────────────────────────
	let x: i32 = java! {
		import java.util.concurrent.ThreadLocalRandom;

		public static int run() {
			return ThreadLocalRandom.current().nextInt(0, 10);
		}
	};
	println!("Random from Java: {x}");

	// ── runtime, with input ('var syntax) ───────────────────────────────────
	let n: i32 = 21;
	let doubled: i32 = java! {
		public static int run() {
			int value = Integer.parseInt('n);
			return value * 2;
		}
	};
	println!("{n} * 2 = {doubled}");

	// ── runtime, multiple inputs ─────────────────────────────────────────────
	let greeting = "Hello";
	let target = "World";
	let msg: String = java! {
		public static String run() {
			return 'greeting + ", " + 'target + "!";
		}
	};
	println!("{msg}");

	// ── compile-time constant ────────────────────────────────────────────────
	println!("PI (baked at compile time): {PI_APPROX}");
}

// ── Compile-time constant: evaluated during rustc macro expansion ────────────
// Math.PI is baked into the binary; java is never invoked at runtime for this.
#[allow(clippy::approx_constant)]
const PI_APPROX: f64 = ct_java! {
	public static void run() {
		System.out.println(Math.PI);
	}
};
