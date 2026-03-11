// Tests for passing complex types *into* java_fn! as input parameters.
// Each method echoes its argument back so the test verifies the full
// serialise → Java → deserialise round-trip.

use inline_java::java_fn;

// List<Optional<String[]>> as input → Vec<Option<Vec<String>>>
#[test]
fn java_fn_arg_list_of_optional_string_array() {
	let input: &[Option<&[&str]>] = &[Some(&["a", "b"]), None, Some(&["c"])];
	let v: Vec<Option<Vec<String>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static List<Optional<String[]>> run(List<Optional<String[]>> v) {
			return v;
		}
	}(input)
	.unwrap();
	assert_eq!(
		v,
		vec![
			Some(vec!["a".to_string(), "b".to_string()]),
			None,
			Some(vec!["c".to_string()]),
		]
	);
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
	}(Some(&[1i32, 2, 3]))
	.unwrap();
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
	}(None::<&[i32]>)
	.unwrap();
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
	}(Some(&[Some(1i32), None, Some(3i32)]))
	.unwrap();
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
	}(None::<&[Option<i32>]>)
	.unwrap();
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
	}(Some(&[
		Some(&[1i32, 2] as &[_]),
		None,
		Some(&[3i32, 4, 5] as &[_]),
	]))
	.unwrap();
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
	}(None::<&[Option<&[i32]>]>)
	.unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<String[][]>>> as input → Option<Vec<Option<Vec<Vec<String>>>>> (present)
#[test]
fn java_fn_arg_optional_list_of_optional_string_2d_array_present() {
	let a: &[&str] = &["a", "b"];
	let b: &[&str] = &["c"];
	let row: &[&[&str]] = &[a, b];
	let input: Option<&[Option<&[&[&str]]>]> = Some(&[Some(row), None]);
	let v: Option<Vec<Option<Vec<Vec<String>>>>> = java_fn! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<String[][]>>> run(Optional<List<Optional<String[][]>>> v) {
			return v;
		}
	}(input)
	.unwrap();
	assert_eq!(
		v,
		Some(vec![
			Some(vec![
				vec!["a".to_string(), "b".to_string()],
				vec!["c".to_string()],
			]),
			None,
		])
	);
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
	}(None::<&[Option<&[&[&str]]>]>)
	.unwrap();
	assert_eq!(v, None);
}
