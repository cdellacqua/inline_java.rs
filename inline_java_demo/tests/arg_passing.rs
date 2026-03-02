use inline_java::{ct_java, java};

// ── java = "..." : single system property passed to the JVM ──────────────────

#[test]
fn java_runtime_single_java_arg() {
	let val: Result<String, _> = java! {
		java = "-Dinline.test=hello",
		public static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, Ok("hello".to_string()));
}


#[test]
fn java_runtime_single_java_arg_with_spaces() {
	let val: Result<String, _> = java! {
		java = "-Dinline.test='hello world'",
		public static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, Ok("hello world".to_string()));
}

// ── java = "..." : multiple args are split on whitespace ─────────────────────

#[test]
fn java_runtime_multiple_java_args() {
	let val: Result<String, _> = java! {
		java = "-Da=foo -Db=bar",
		public static String run() {
			return System.getProperty("a") + ":" + System.getProperty("b");
		}
	};
	assert_eq!(val, Ok("foo:bar".to_string()));
}

// ── javac = "..." : sourcepath lets javac resolve project Java files ──────────

#[test]
fn java_runtime_javac_sourcepath() {
	let val: Result<String, _> = java! {
		javac = "-sourcepath $CARGO_MANIFEST_DIR",
		import com.example.demo.*;
		public static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, Ok("Hello, World!".to_string()));
}

// ── both opts together ────────────────────────────────────────────────────────

#[test]
fn java_runtime_javac_and_java_args() {
	let val: Result<String, _> = java! {
		javac = "-sourcepath $CARGO_MANIFEST_DIR",
		java = "-Dinline.combined=yes",
		import com.example.demo.*;
		public static String run() {
			return new HelloWorld().greet() + "|" + System.getProperty("inline.combined");
		}
	};
	assert_eq!(val, Ok("Hello, World!|yes".to_string()));
}

// ── ct_java! with java = "..." ────────────────────────────────────────────────

const CT_JAVA_ARG: &str = ct_java! {
	java = "-Dinline.ct=compile-time",
	public static String run() {
		return System.getProperty("inline.ct");
	}
};

#[test]
fn ct_java_java_arg() {
	assert_eq!(CT_JAVA_ARG, "compile-time");
}

// ── ct_java! with javac = "..." ───────────────────────────────────────────────

const CT_JAVAC_SOURCEPATH: &str = ct_java! {
	javac = "-sourcepath $CARGO_MANIFEST_DIR",
	import com.example.demo.*;
	public static String run() {
		return new HelloWorld().greet();
	}
};

#[test]
fn ct_java_javac_sourcepath() {
	assert_eq!(CT_JAVAC_SOURCEPATH, "Hello, World!");
}
