// Tests for passing complex types *into* java_fn! as input parameters.
// Each method echoes its argument back so the test verifies the full
// serialise → Java → deserialise round-trip.
//
// All tests in this file are currently Red (implementation not yet complete).

use inline_java::java_fn;

// List<Optional<String[]>> as input → Vec<Option<Vec<String>>>
#[test]
fn java_fn_arg_list_of_optional_string_array() {
	let input: Vec<Option<Vec<&str>>> = vec![
		Some(vec!["a", "b"]),
		None,
		Some(vec!["c"]),
	];
	let v: Vec<Option<Vec<String>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static List<Optional<String[]>> run(List<Optional<String[]>> v) {
			return v;
		}
	}(input).unwrap();
	assert_eq!(v, vec![
		Some(vec!["a".to_string(), "b".to_string()]),
		None,
		Some(vec!["c".to_string()]),
	]);
}

// Optional<List<Integer>> as input → Option<Vec<i32>> (present)
#[test]
fn java_fn_arg_optional_list_integer_present() {
	let v: Option<Vec<i32>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Integer>> run(Optional<List<Integer>> v) {
			return v;
		}
	}(Some(vec![1i32, 2, 3])).unwrap();
	assert_eq!(v, Some(vec![1, 2, 3]));
}

// Optional<List<Integer>> as input → Option<Vec<i32>> (absent)
#[test]
fn java_fn_arg_optional_list_integer_absent() {
	let v: Option<Vec<i32>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Integer>> run(Optional<List<Integer>> v) {
			return v;
		}
	}(None::<Vec<i32>>).unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<Integer>>> as input → Option<Vec<Option<i32>>> (present)
#[test]
fn java_fn_arg_optional_list_of_optional_integer_present() {
	let v: Option<Vec<Option<i32>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer>>> run(Optional<List<Optional<Integer>>> v) {
			return v;
		}
	}(Some(vec![Some(1i32), None, Some(3)])).unwrap();
	assert_eq!(v, Some(vec![Some(1), None, Some(3)]));
}

// Optional<List<Optional<Integer>>> as input → Option<Vec<Option<i32>>> (absent)
#[test]
fn java_fn_arg_optional_list_of_optional_integer_absent() {
	let v: Option<Vec<Option<i32>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer>>> run(Optional<List<Optional<Integer>>> v) {
			return v;
		}
	}(None::<Vec<Option<i32>>>).unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<Integer[]>>> as input → Option<Vec<Option<Vec<i32>>>> (present)
#[test]
fn java_fn_arg_optional_list_of_optional_integer_array_present() {
	let v: Option<Vec<Option<Vec<i32>>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer[]>>> run(Optional<List<Optional<Integer[]>>> v) {
			return v;
		}
	}(Some(vec![Some(vec![1i32, 2]), None, Some(vec![3, 4, 5])])).unwrap();
	assert_eq!(v, Some(vec![Some(vec![1, 2]), None, Some(vec![3, 4, 5])]));
}

// Optional<List<Optional<Integer[]>>> as input → Option<Vec<Option<Vec<i32>>>> (absent)
#[test]
fn java_fn_arg_optional_list_of_optional_integer_array_absent() {
	let v: Option<Vec<Option<Vec<i32>>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer[]>>> run(Optional<List<Optional<Integer[]>>> v) {
			return v;
		}
	}(None::<Vec<Option<Vec<i32>>>>).unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<String[][]>>> as input → Option<Vec<Option<Vec<Vec<String>>>>> (present)
#[test]
fn java_fn_arg_optional_list_of_optional_string_2d_array_present() {
	let input: Option<Vec<Option<Vec<Vec<&str>>>>> = Some(vec![
		Some(vec![vec!["a", "b"], vec!["c"]]),
		None,
	]);
	let v: Option<Vec<Option<Vec<Vec<String>>>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<String[][]>>> run(Optional<List<Optional<String[][]>>> v) {
			return v;
		}
	}(input).unwrap();
	assert_eq!(v, Some(vec![
		Some(vec![
			vec!["a".to_string(), "b".to_string()],
			vec!["c".to_string()],
		]),
		None,
	]));
}

// Optional<List<Optional<String[][]>>> as input → Option<Vec<Option<Vec<Vec<String>>>>> (absent)
#[test]
fn java_fn_arg_optional_list_of_optional_string_2d_array_absent() {
	let v: Option<Vec<Option<Vec<Vec<String>>>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<String[][]>>> run(Optional<List<Optional<String[][]>>> v) {
			return v;
		}
	}(None::<Vec<Option<Vec<Vec<&str>>>>>).unwrap();
	assert_eq!(v, None);
}
