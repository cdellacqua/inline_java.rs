use inline_java::{ct_java, java};
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

// java = "..." : single system property passed to the JVM

#[test]
fn java_runtime_single_java_arg() {
	let val: Result<String, _> = java! {
		java = "-Dinline.test=hello",
		static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, Ok("hello".to_string()));
}

#[test]
fn java_runtime_single_java_arg_with_spaces() {
	let val: Result<String, _> = java! {
		java = "-Dinline.test='hello world'",
		static String run() {
			return System.getProperty("inline.test");
		}
	};
	assert_eq!(val, Ok("hello world".to_string()));
}

// java = "..." : multiple args are split on whitespace

#[test]
fn java_runtime_multiple_java_args() {
	let val: Result<String, _> = java! {
		java = "-Da=foo -Db=bar",
		static String run() {
			return System.getProperty("a") + ":" + System.getProperty("b");
		}
	};
	assert_eq!(val, Ok("foo:bar".to_string()));
}

// classpath JAR — tmp_dir is appended automatically, no need to include it manually

#[test]
fn java_runtime_javac_classpath_jar() {
	build_demo_jar("/tmp/inline_java_flags_cp_jar/demo.jar");
	let val: Result<String, _> = java! {
		javac = "-cp \"/tmp/inline_java_flags_cp_jar/demo.jar\"",
		java = "-cp /tmp/inline_java_flags_cp_jar/demo.jar",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, Ok("Hello, World!".to_string()));
}

#[test]
fn java_runtime_javac_classpath_jar_long_arg_name() {
	build_demo_jar("/tmp/inline_java_flags_cp_long_jar/demo.jar");
	let val: Result<String, _> = java! {
		javac = "-classpath \"/tmp/inline_java_flags_cp_long_jar/demo.jar\"",
		java = "-classpath /tmp/inline_java_flags_cp_long_jar/demo.jar",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, Ok("Hello, World!".to_string()));
}

// javac = "..." : sourcepath lets javac resolve project Java files

#[test]
fn java_runtime_javac_sourcepath() {
	let val: Result<String, _> = java! {
		javac = "-sourcepath .",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, Ok("Hello, World!".to_string()));
}

// both opts together

#[test]
fn java_runtime_javac_and_java_args() {
	let val: Result<String, _> = java! {
		javac = "-sourcepath .",
		java = "-Dinline.combined=yes",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet() + "|" + System.getProperty("inline.combined");
		}
	};
	assert_eq!(val, Ok("Hello, World!|yes".to_string()));
}

// ct_java! with java = "..."

const CT_JAVA_ARG: &str = ct_java! {
	java = "-Dinline.ct=compile-time",
	static String run() {
		return System.getProperty("inline.ct");
	}
};

#[test]
fn ct_java_java_arg() {
	assert_eq!(CT_JAVA_ARG, "compile-time");
}

// ct_java! with javac = "..."

const CT_JAVAC_SOURCEPATH: &str = ct_java! {
	javac = "-sourcepath ./inline_java",
	import com.example.demo.*;
	static String run() {
		return new HelloWorld().greet();
	}
};

#[test]
fn ct_java_javac_sourcepath() {
	assert_eq!(CT_JAVAC_SOURCEPATH, "Hello, World!");
}
