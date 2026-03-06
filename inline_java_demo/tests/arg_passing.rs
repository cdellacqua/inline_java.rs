use inline_java::{ct_java, java};

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

// classpath JAR via $INLINE_JAVA_CP

#[test]
fn java_runtime_javac_classpath_jar() {
	let val: Result<String, _> = java! {
		javac = "-cp \"demo.jar\"",
		java = "-cp $INLINE_JAVA_CP:demo.jar",
		import com.example.demo.*;
		static String run() {
			return new HelloWorld().greet();
		}
	};
	assert_eq!(val, Ok("Hello, World!".to_string()));
}

#[test]
fn java_runtime_javac_classpath_jar_long_arg_name() {
	let val: Result<String, _> = java! {
		javac = "-classpath \"demo.jar\"",
		java = "-classpath $INLINE_JAVA_CP:demo.jar",
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
	javac = "-sourcepath ./inline_java_demo",
	import com.example.demo.*;
	static String run() {
		return new HelloWorld().greet();
	}
};

#[test]
fn ct_java_javac_sourcepath() {
	assert_eq!(CT_JAVAC_SOURCEPATH, "Hello, World!");
}

// java! with int[] return type

#[test]
fn java_runtime_int_array() {
	let v: Vec<i32> = java! {
		static int[] run() {
			return new int[]{1, 2, 3, 4, 5};
		}
	}.unwrap();
	assert_eq!(v, vec![1i32, 2, 3, 4, 5]);
}

// java! with double[] return type

#[test]
fn java_runtime_double_array() {
	let v: Vec<f64> = java! {
		static double[] run() {
			return new double[]{1.5, 2.5, 3.5};
		}
	}.unwrap();
	assert_eq!(v, vec![1.5f64, 2.5, 3.5]);
}

// java! with boolean[] return type

#[test]
fn java_runtime_boolean_array() {
	let v: Vec<bool> = java! {
		static boolean[] run() {
			return new boolean[]{true, false, true};
		}
	}.unwrap();
	assert_eq!(v, vec![true, false, true]);
}

// java! with String[] return type

#[test]
fn java_runtime_string_array() {
	let v: Vec<String> = java! {
		static String[] run() {
			return new String[]{"hello", "world"};
		}
	}.unwrap();
	assert_eq!(v, vec!["hello".to_string(), "world".to_string()]);
}

// java! with List<Integer> return type

#[test]
fn java_runtime_list_integer() {
	let v: Vec<i32> = java! {
		import java.util.Arrays;
		import java.util.List;
		static List<Integer> run() {
			return Arrays.asList(10, 20, 30);
		}
	}.unwrap();
	assert_eq!(v, vec![10i32, 20, 30]);
}

// java! with List<String> return type

#[test]
fn java_runtime_list_string() {
	let v: Vec<String> = java! {
		import java.util.Arrays;
		static java.util.List<String> run() {
			return Arrays.asList("foo", "bar", "baz");
		}
	}.unwrap();
	assert_eq!(v, vec!["foo".to_string(), "bar".to_string(), "baz".to_string()]);
}

// java! with abstract class + subclass (OOP)

#[test]
fn java_runtime_abstract_class_override() {
	let sound: String = java! {
		abstract class Animal {
			abstract String sound();
		}
		class Dog extends Animal {
			@Override
			String sound() { return "woof"; }
		}
		static String run() {
			return new Dog().sound();
		}
	}.unwrap();
	assert_eq!(sound, "woof");
}

// java! with empty array

#[test]
fn java_runtime_empty_array() {
	let v: Vec<i32> = java! {
		static int[] run() {
			return new int[]{};
		}
	}.unwrap();
	assert!(v.is_empty());
}

// ct_java! with int[] return type

const CT_INT_ARRAY: [i32; 3] = ct_java! {
	static int[] run() {
		return new int[]{100, 200, 300};
	}
};

#[test]
fn ct_java_int_array() {
	assert_eq!(CT_INT_ARRAY, [100i32, 200, 300]);
}

// ct_java! with String[] return type

const CT_STRING_ARRAY: [&str; 2] = ct_java! {
	static String[] run() {
		return new String[]{"compile", "time"};
	}
};

#[test]
fn ct_java_string_array() {
	assert_eq!(CT_STRING_ARRAY, ["compile", "time"]);
}
