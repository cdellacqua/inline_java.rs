// Tests for return types where arrays appear *inside* generic type parameters,
// e.g. List<Optional<String[]>>. The macro's type parser must handle `[]`
// suffixes that appear before a closing `>` or `>>`.
//
// All tests in this file are currently Red (implementation not yet complete).

use inline_java::java;

// List<Optional<String[]>> → Vec<Option<Vec<String>>>
#[test]
fn java_runtime_list_of_optional_string_array() {
	let v: Vec<Option<Vec<String>>> = java! {
		import java.util.Arrays;
		import java.util.List;
		import java.util.Optional;
		static List<Optional<String[]>> run() {
			return Arrays.asList(
				Optional.of(new String[]{"a", "b"}),
				Optional.empty(),
				Optional.of(new String[]{"c"})
			);
		}
	}
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

// Optional<List<Integer>> → Option<Vec<i32>>
#[test]
fn java_runtime_optional_list_integer_present() {
	let v: Option<Vec<i32>> = java! {
		import java.util.Arrays;
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Integer>> run() {
			return Optional.of(Arrays.asList(1, 2, 3));
		}
	}
	.unwrap();
	assert_eq!(v, Some(vec![1, 2, 3]));
}

#[test]
fn java_runtime_optional_list_integer_absent() {
	let v: Option<Vec<i32>> = java! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Integer>> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<Integer>>> → Option<Vec<Option<i32>>>
#[test]
fn java_runtime_optional_list_of_optional_integer_present() {
	let v: Option<Vec<Option<i32>>> = java! {
		import java.util.Arrays;
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer>>> run() {
			return Optional.of(Arrays.asList(
				Optional.of(1),
				Optional.empty(),
				Optional.of(3)
			));
		}
	}
	.unwrap();
	assert_eq!(v, Some(vec![Some(1), None, Some(3)]));
}

#[test]
fn java_runtime_optional_list_of_optional_integer_absent() {
	let v: Option<Vec<Option<i32>>> = java! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer>>> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<Integer[]>>> → Option<Vec<Option<Vec<i32>>>>
#[test]
fn java_runtime_optional_list_of_optional_integer_array_present() {
	let v: Option<Vec<Option<Vec<i32>>>> = java! {
		import java.util.Arrays;
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer[]>>> run() {
			return Optional.of(Arrays.asList(
				Optional.of(new Integer[]{1, 2}),
				Optional.empty(),
				Optional.of(new Integer[]{3, 4, 5})
			));
		}
	}
	.unwrap();
	assert_eq!(v, Some(vec![Some(vec![1, 2]), None, Some(vec![3, 4, 5])]));
}

#[test]
fn java_runtime_optional_list_of_optional_integer_array_absent() {
	let v: Option<Vec<Option<Vec<i32>>>> = java! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<Integer[]>>> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(v, None);
}

// Optional<List<Optional<String[][]>>> → Option<Vec<Option<Vec<Vec<String>>>>>
#[test]
fn java_runtime_optional_list_of_optional_string_2d_array_present() {
	let v: Option<Vec<Option<Vec<Vec<String>>>>> = java! {
		import java.util.Arrays;
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<String[][]>>> run() {
			return Optional.of(Arrays.asList(
				Optional.of(new String[][]{{"a", "b"}, {"c"}}),
				Optional.empty()
			));
		}
	}
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

#[test]
fn java_runtime_optional_list_of_optional_string_2d_array_absent() {
	let v: Option<Vec<Option<Vec<Vec<String>>>>> = java! {
		import java.util.List;
		import java.util.Optional;
		static Optional<List<Optional<String[][]>>> run() {
			return Optional.empty();
		}
	}
	.unwrap();
	assert_eq!(v, None);
}
