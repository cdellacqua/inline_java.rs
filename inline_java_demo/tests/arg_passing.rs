use inline_java_macros::{ct_java, java};

// ── java = "..." : single system property passed to the JVM ──────────────────

#[test]
fn java_runtime_single_java_arg() {
	let val: String = java! {
		java = "-Dinline.test=hello",
		public static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, "hello");
}


#[test]
fn java_runtime_single_java_arg_with_spaces() {
	let val: String = java! {
		java = "-Dinline.test='hello world'",
		public static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, "hello world");
}

// ── java = "..." : multiple args are split on whitespace ─────────────────────

#[test]
fn java_runtime_multiple_java_args() {
	let val: String = java! {
		java = "-Da=foo -Db=bar",
		public static String run() {
			return System.getProperty("a") + ":" + System.getProperty("b");
		}
	};
	assert_eq!(val, "foo:bar");
}

// ── javac = "..." : sourcepath lets javac resolve project Java files ──────────

#[test]
fn java_runtime_javac_sourcepath() {
	let val: String = java! {
		javac = "-sourcepath /home/ubuntu/Dev/inline_java/inline_java_demo",
		import com.example.demo.*;
		public static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, "Hello, World!");
}

// ── both opts together ────────────────────────────────────────────────────────

#[test]
fn java_runtime_javac_and_java_args() {
	let val: String = java! {
		javac = "-sourcepath /home/ubuntu/Dev/inline_java/inline_java_demo",
		java = "-Dinline.combined=yes",
		import com.example.demo.*;
		public static String run() {
			return new HelloWorld().greet() + "|" + System.getProperty("inline.combined");
		}
	};
	assert_eq!(val, "Hello, World!|yes");
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
	javac = "-sourcepath /home/ubuntu/Dev/inline_java/inline_java_demo",
	import com.example.demo.*;
	public static String run() {
		return new HelloWorld().greet();
	}
};

#[test]
fn ct_java_javac_sourcepath() {
	assert_eq!(CT_JAVAC_SOURCEPATH, "Hello, World!");
}
