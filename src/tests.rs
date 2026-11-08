use std::{
	collections::hash_map::DefaultHasher,
	hash::{Hash, Hasher},
	mem::size_of,
};

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::Signed;

use super::*;

fn scaled(coefficient: i128) -> BigFixed {
	BigFixed::from_scaled_i128(coefficient)
}

fn big_scaled(coefficient: &str) -> BigFixed {
	BigFixed::from_scaled_bigint(coefficient.parse().unwrap())
}

fn reference_round_div(numerator: BigInt, denominator: BigInt) -> BigInt {
	let (mut quotient, remainder) = numerator.div_rem(&denominator);
	let remainder_abs = remainder.abs();
	let denominator_abs = denominator.abs();

	if &remainder_abs * 2 >= denominator_abs {
		if numerator.sign() == denominator.sign() {
			quotient += 1;
		} else {
			quotient -= 1;
		}
	}

	quotient
}

#[test]
fn constants_and_scaled_constructors_are_canonical() {
	assert_eq!(BigFixed::ZERO.as_scaled_i64(), Some(0));
	assert_eq!(BigFixed::ONE.as_scaled_i64(), Some(DEC_I64));
	assert!(BigFixed::from_scaled(i64::MIN).is_native());
	assert!(BigFixed::from_scaled(i64::MAX).is_native());

	let below = BigFixed::from_scaled_i128(i128::from(i64::MIN) - 1);
	let above = BigFixed::from_scaled_i128(i128::from(i64::MAX) + 1);
	assert!(!below.is_native());
	assert!(!above.is_native());

	assert_eq!(below.to_scaled_bigint(), BigInt::from(i64::MIN) - BigInt::from(1));
	assert_eq!(above.into_scaled_bigint(), BigInt::from(i64::MAX) + BigInt::from(1));
}

#[test]
fn integer_conversions_scale_the_input() {
	assert_eq!(BigFixed::from(1_i64), BigFixed::ONE);
	assert_eq!(BigFixed::from(-2_i64).as_scaled_i64(), Some(-2 * DEC_I64));

	let huge = BigInt::from(i64::MAX) + BigInt::from(1);
	assert_eq!(
		BigFixed::from(huge.clone()).to_scaled_bigint(),
		huge * BigInt::from(DEC_I64)
	);
}

#[test]
fn whole_numbers_use_the_full_native_i64_range() {
	for integer in [i64::MIN, -1, 0, 1, i64::MAX] {
		let value = BigFixed::from(integer);
		assert!(value.is_native(), "integer: {integer}");
		assert!(value.is_integer(), "integer: {integer}");
		assert_eq!(value.as_i64_integer(), Some(integer));
		assert_eq!(
			value.to_scaled_bigint(),
			BigInt::from(integer) * DEC_I64,
			"integer: {integer}"
		);
	}

	assert_eq!(BigFixed::from(i64::MAX).as_scaled_i64(), None);
	assert_eq!(BigFixed::from(i64::MIN).as_scaled_i64(), None);

	let max_from_i128 = BigFixed::from_scaled_i128(i128::from(i64::MAX) * DEC_I128);
	let min_from_bigint = BigFixed::from_scaled_bigint(BigInt::from(i64::MIN) * DEC_I64);
	assert_eq!(max_from_i128.as_i64_integer(), Some(i64::MAX));
	assert_eq!(min_from_bigint.as_i64_integer(), Some(i64::MIN));
}

#[test]
fn integer_queries_distinguish_fractional_and_out_of_range_values() {
	let fractional = BigFixed::from_scaled(DEC_I64 + 1);
	assert!(!fractional.is_integer());
	assert_eq!(fractional.as_i64_integer(), None);

	let beyond_i64 = BigFixed::from(BigInt::from(i64::MAX) + 1);
	assert!(!beyond_i64.is_native());
	assert!(beyond_i64.is_integer());
	assert_eq!(beyond_i64.as_i64_integer(), None);

	let large_fraction = BigFixed::from_scaled_bigint(BigInt::from(i64::MAX) + 1);
	assert!(!large_fraction.is_native());
	assert!(!large_fraction.is_integer());
	assert_eq!(large_fraction.as_i64_integer(), None);
}

#[test]
fn parsing_keeps_i64_integers_native() {
	let min: BigFixed = i64::MIN.to_string().parse().unwrap();
	let max: BigFixed = i64::MAX.to_string().parse().unwrap();
	let above: BigFixed = "9223372036854775808".parse().unwrap();

	assert_eq!(min.as_i64_integer(), Some(i64::MIN));
	assert_eq!(max.as_i64_integer(), Some(i64::MAX));
	assert!(min.is_native());
	assert!(max.is_native());
	assert!(!above.is_native());
	assert!(above.is_integer());
}

#[test]
#[cfg(target_pointer_width = "64")]
fn representation_stays_two_machine_words() {
	assert_eq!(size_of::<BigFixed>(), 16);
}

#[test]
fn parses_and_formats_valid_decimals() {
	let cases = [
		("0", "0", "0.000000"),
		("+0", "0", "0.000000"),
		("-0", "0", "0.000000"),
		("000.0000000", "0", "0.000000"),
		("1", "1", "1.000000"),
		("1.2", "1.2", "1.200000"),
		("1.234567", "1.234567", "1.234567"),
		("-12.340000", "-12.34", "-12.340000"),
	];

	for (source, display, _fixed) in cases {
		let value: BigFixed = source.parse().unwrap();
		assert_eq!(value.to_string(), display, "source: {source}");
		assert_eq!(value.to_string().parse::<BigFixed>().unwrap(), value);
	}
}

#[test]
fn formatting_honors_sign_width_alignment_and_zero_padding() {
	let positive: BigFixed = "12.34".parse().unwrap();
	let negative: BigFixed = "-12.34".parse().unwrap();

	assert_eq!(format!("{positive:+}"), "+12.34");
	assert_eq!(format!("{positive:>10}"), "     12.34");
	assert_eq!(format!("{positive:<10}"), "12.34     ");
	assert_eq!(format!("{positive:010}"), "0000012.34");
	assert_eq!(format!("{negative:010}"), "-000012.34");
}

#[test]
fn parsing_rounds_half_away_from_zero() {
	let cases = [
		("0.0000004", 0),
		("0.0000005", 1),
		("-0.0000004", 0),
		("-0.0000005", -1),
		("0.9999994", 999_999),
		("0.9999995", 1_000_000),
		("-0.9999995", -1_000_000),
		("1.23456749999", 1_234_567),
		("1.23456750000", 1_234_568),
	];

	for (source, expected) in cases {
		assert_eq!(
			source.parse::<BigFixed>().unwrap().as_scaled_i64(),
			Some(expected),
			"source: {source}"
		);
	}
}

#[test]
fn parsing_respects_native_boundaries() {
	let min: BigFixed = "-9223372036854.775808".parse().unwrap();
	let max: BigFixed = "9223372036854.775807".parse().unwrap();
	let below: BigFixed = "-9223372036854.775809".parse().unwrap();
	let above: BigFixed = "9223372036854.775808".parse().unwrap();

	assert_eq!(min.as_scaled_i64(), Some(i64::MIN));
	assert_eq!(max.as_scaled_i64(), Some(i64::MAX));
	assert!(!below.is_native());
	assert!(!above.is_native());
	assert_eq!(below.to_string(), "-9223372036854.775809");
	assert_eq!(above.to_string(), "9223372036854.775808");
}

#[test]
fn parses_arbitrarily_large_values_and_rounding_carry() {
	let source = "999999999999999999999999999999999999.9999995";
	let value: BigFixed = source.parse().unwrap();

	assert!(!value.is_native());
	assert_eq!(value.to_string(), "1000000000000000000000000000000000000");
}

#[test]
fn rejects_invalid_decimal_syntax() {
	assert_eq!("".parse::<BigFixed>(), Err(Error::Empty));

	for source in [
		"+",
		"-",
		".",
		".1",
		"1.",
		"1..0",
		"1e3",
		" 1",
		"1 ",
		"1_000",
		"١",
		"1.000000x",
		"--1",
		"+-1",
	] {
		assert_eq!(
			source.parse::<BigFixed>(),
			Err(Error::InvalidSyntax),
			"source: {source:?}"
		);
	}
}

#[test]
fn addition_promotes_and_demotes_at_native_boundaries() {
	let max = BigFixed::from_scaled(i64::MAX);
	let promoted = &max + BigFixed::from_scaled(1);
	assert!(!promoted.is_native());
	assert_eq!(promoted.to_scaled_bigint(), BigInt::from(i64::MAX) + BigInt::from(1));

	let demoted = promoted - BigFixed::from_scaled(1);
	assert_eq!(demoted.as_scaled_i64(), Some(i64::MAX));

	let min = BigFixed::from_scaled(i64::MIN);
	let promoted = &min - BigFixed::from_scaled(1);
	assert!(!promoted.is_native());
	assert_eq!((promoted + BigFixed::from_scaled(1)).as_scaled_i64(), Some(i64::MIN));
}

#[test]
fn big_arithmetic_can_canonicalize_back_to_native() {
	let positive = big_scaled("9223372036854775808");
	let negative = big_scaled("-9223372036854775808");

	assert_eq!(
		(&positive + &negative).as_scaled_i64(),
		Some(0),
		"Big + Big cancellation must canonicalize"
	);
	assert_eq!((&positive - BigFixed::from_scaled(1)).as_scaled_i64(), Some(i64::MAX));
}

#[test]
fn negation_handles_i64_min_and_demotes_again() {
	let min = BigFixed::from_scaled(i64::MIN);
	let positive = -&min;

	assert!(!positive.is_native());
	assert_eq!(positive.to_scaled_bigint(), BigInt::from(1_u64) << 63);
	assert_eq!((-positive).as_scaled_i64(), Some(i64::MIN));
	assert_eq!((-BigFixed::ZERO).as_scaled_i64(), Some(0));
}

#[test]
fn absolute_value_handles_native_and_big_values() {
	assert_eq!(scaled(-123).abs().as_scaled_i64(), Some(123));

	let abs_min = BigFixed::from_scaled(i64::MIN).abs();
	assert!(!abs_min.is_native());
	assert_eq!(abs_min.to_scaled_bigint(), BigInt::from(1_u64) << 63);
}

#[test]
fn multiplication_rounds_ties_away_from_zero() {
	let cases = [
		(1, 499_999, 0),
		(1, 500_000, 1),
		(-1, 500_000, -1),
		(-1, -500_000, 1),
		(1, -499_999, 0),
	];

	for (lhs, rhs, expected) in cases {
		assert_eq!(
			(scaled(lhs) * scaled(rhs)).as_scaled_i64(),
			Some(expected),
			"{lhs} * {rhs}"
		);
	}
}

#[test]
fn native_multiplication_uses_a_wide_intermediate() {
	let lhs: BigFixed = "10000000".parse().unwrap();
	let rhs: BigFixed = "1".parse().unwrap();
	assert!(lhs.is_native());
	assert!(rhs.is_native());

	let product = lhs * rhs;
	assert!(product.is_native());
	assert_eq!(product.to_string(), "10000000");
}

#[test]
fn lehmer_rng_intermediate_stays_native() {
	let seed = 2_147_483_646_i64;
	let multiplier = 48_271_i64;
	let modulus = 2_147_483_647_i64;

	let product = BigFixed::from(seed) * BigFixed::from(multiplier);
	assert!(product.is_native());
	assert_eq!(product.as_i64_integer(), Some(seed * multiplier));

	let next = product % BigFixed::from(modulus);
	assert!(next.is_native());
	assert_eq!(next.as_i64_integer(), Some((seed * multiplier) % modulus));
}

#[test]
fn mixed_native_arithmetic_preserves_scale_and_integer_queries() {
	let half: BigFixed = "0.5".parse().unwrap();
	let two = BigFixed::from(2_i64);

	let sum = &two + &half;
	assert!(sum.is_native());
	assert!(!sum.is_integer());
	assert_eq!(sum.to_string(), "2.5");

	let product = two * half;
	assert!(product.is_native());
	assert!(product.is_integer());
	assert_eq!(product.as_i64_integer(), Some(1));

	let promoted = BigFixed::from(i64::MAX) + BigFixed::ONE;
	assert!(!promoted.is_native());
	assert!(promoted.is_integer());
	assert_eq!((promoted - BigFixed::ONE).as_i64_integer(), Some(i64::MAX));
}

#[test]
fn coefficient_arithmetic_defers_whole_detection() {
	let scaled_whole = scaled(500_000) * BigFixed::from(2_i64);
	assert!(matches!(
		&scaled_whole.0,
		Repr::Scaled(coefficient) if *coefficient == DEC_I64
	));
	assert_eq!(scaled_whole, BigFixed::ONE);
	assert_eq!(scaled_whole.as_i64_integer(), Some(1));

	let scaled_zero = scaled(1) - scaled(1);
	assert!(matches!(&scaled_zero.0, Repr::Scaled(0)));
	assert!(scaled_zero.is_zero());
	assert_eq!(scaled_zero, BigFixed::ZERO);

	let large_whole =
		BigFixed::from_scaled(i64::MAX).checked_div(&BigFixed::from_scaled(1)).unwrap();
	assert!(matches!(&large_whole.0, Repr::Large(_)));
	assert_eq!(large_whole.as_i64_integer(), Some(i64::MAX));
	assert_eq!(large_whole, BigFixed::from(i64::MAX));
}

#[test]
fn all_representation_pairs_match_the_scaled_bigint_reference() {
	let scaled_whole = scaled(500_000) * BigFixed::from(2_i64);
	let scaled_zero = scaled(1) - scaled(1);
	let large_whole =
		BigFixed::from_scaled(i64::MAX).checked_div(&BigFixed::from_scaled(1)).unwrap();
	let values = [
		BigFixed::from(i64::MIN),
		BigFixed::from(-48_271_i64),
		BigFixed::ZERO,
		BigFixed::from(48_271_i64),
		BigFixed::from(i64::MAX),
		BigFixed::from_scaled(i64::MIN),
		BigFixed::from_scaled(-1),
		BigFixed::from_scaled(1),
		BigFixed::from_scaled(i64::MAX),
		BigFixed::from_scaled_bigint(BigInt::from(i64::MIN) - 1),
		BigFixed::from_scaled_bigint(BigInt::from(i64::MAX) + 1),
		BigFixed::from(BigInt::from(i64::MIN) - 1),
		BigFixed::from(BigInt::from(i64::MAX) + 1),
		scaled_whole,
		scaled_zero,
		large_whole,
	];

	for lhs in &values {
		for rhs in &values {
			let lhs_scaled = lhs.to_scaled_bigint();
			let rhs_scaled = rhs.to_scaled_bigint();
			let case = format!("lhs={lhs}, rhs={rhs}");

			assert_eq!(
				(lhs + rhs).into_scaled_bigint(),
				&lhs_scaled + &rhs_scaled,
				"add: {case}"
			);
			assert_eq!(
				(lhs - rhs).into_scaled_bigint(),
				&lhs_scaled - &rhs_scaled,
				"sub: {case}"
			);
			assert_eq!(
				(lhs * rhs).into_scaled_bigint(),
				reference_round_div(&lhs_scaled * &rhs_scaled, BigInt::from(DEC_I64),),
				"mul: {case}"
			);
			assert_eq!(lhs.cmp(rhs), lhs_scaled.cmp(&rhs_scaled), "cmp: {case}");

			if !rhs.is_zero() {
				assert_eq!(
					lhs.checked_div(rhs).unwrap().into_scaled_bigint(),
					reference_round_div(
						&lhs_scaled * BigInt::from(DEC_I64),
						rhs_scaled.clone(),
					),
					"div: {case}"
				);
				assert_eq!(
					lhs.checked_rem(rhs).unwrap().into_scaled_bigint(),
					&lhs_scaled % &rhs_scaled,
					"rem: {case}"
				);
			}
		}
	}
}

#[test]
fn native_multiplication_matches_bigint_reference_at_extremes() {
	for (lhs, rhs) in [(i64::MIN, i64::MIN), (i64::MIN, i64::MAX), (i64::MAX, i64::MAX)]
	{
		let expected = reference_round_div(
			BigInt::from(lhs) * BigInt::from(rhs),
			BigInt::from(DEC_I64),
		);
		let actual =
			(BigFixed::from_scaled(lhs) * BigFixed::from_scaled(rhs)).into_scaled_bigint();
		assert_eq!(actual, expected, "{lhs} * {rhs}");
	}
}

#[test]
fn big_multiplication_rounds_signed_ties_and_demotes() {
	let magnitude = i128::from(i64::MAX) + 2;
	let expected = 4_611_686_018_427_387_905_i64;

	for (lhs, rhs, result) in [
		(magnitude, 500_000, expected),
		(-magnitude, 500_000, -expected),
		(magnitude, -500_000, -expected),
		(-magnitude, -500_000, expected),
	] {
		let lhs = BigFixed::from_scaled_i128(lhs);
		assert!(!lhs.is_native());
		assert_eq!(
			(&lhs * BigFixed::from_scaled(rhs)).as_scaled_i64(),
			Some(result),
			"{lhs} * {rhs}"
		);
	}
}

#[test]
fn division_rounds_to_six_fractional_digits() {
	let cases = [
		(1, 2_000_001, 0),
		(1, 2_000_000, 1),
		(1, 1_999_999, 1),
		(-1, 2_000_000, -1),
		(1, -2_000_000, -1),
		(-1, -2_000_000, 1),
		(1, 3, 333_333),
		(2, 3, 666_667),
	];

	for (lhs, rhs, expected) in cases {
		assert_eq!(
			scaled(lhs).checked_div(&scaled(rhs)).unwrap().as_scaled_i64(),
			Some(expected),
			"{lhs} / {rhs}"
		);
	}
}

#[test]
fn native_division_handles_extreme_coefficients() {
	let quotient = BigFixed::from_scaled(i64::MAX)
		.checked_div(&BigFixed::from_scaled(i64::MIN))
		.unwrap();
	assert_eq!(quotient.as_scaled_i64(), Some(-DEC_I64));

	let huge =
		BigFixed::from_scaled(i64::MAX).checked_div(&BigFixed::from_scaled(1)).unwrap();
	assert!(!huge.is_native());
	assert_eq!(huge.as_i64_integer(), Some(i64::MAX));
	assert_eq!(huge, BigFixed::from(i64::MAX));
	assert_eq!(huge.to_scaled_bigint(), BigInt::from(i64::MAX) * BigInt::from(DEC_I64));
}

#[test]
fn big_division_rounds_signed_ties_and_demotes() {
	let magnitude = i128::from(i64::MAX) + 2;
	let expected = 4_611_686_018_427_387_905_i64;

	for (lhs, rhs, result) in [
		(magnitude, 2_000_000, expected),
		(-magnitude, 2_000_000, -expected),
		(magnitude, -2_000_000, -expected),
		(-magnitude, -2_000_000, expected),
	] {
		let lhs = BigFixed::from_scaled_i128(lhs);
		assert!(!lhs.is_native());
		assert_eq!(
			lhs.checked_div(&scaled(rhs)).unwrap().as_scaled_i64(),
			Some(result),
			"{lhs} / {rhs}"
		);
	}
}

#[test]
fn big_by_big_division_rounds_signed_ties_away_from_zero() {
	let magnitude = BigInt::from(i64::MAX) + BigInt::from(2);
	let denominator_magnitude = &magnitude * BigInt::from(2 * DEC_I64);

	for (lhs_sign, rhs_sign, expected) in
		[(1, 1, 1), (-1, 1, -1), (1, -1, -1), (-1, -1, 1)]
	{
		let lhs = BigFixed::from_scaled_bigint(&magnitude * lhs_sign);
		let rhs = BigFixed::from_scaled_bigint(&denominator_magnitude * rhs_sign);
		assert!(!lhs.is_native());
		assert!(!rhs.is_native());
		assert_eq!(lhs.checked_div(&rhs).unwrap().as_scaled_i64(), Some(expected));
	}
}

#[test]
fn mixed_and_big_division_match_the_bigint_reference() {
	let cases = [
		("9223372036854775808", "3"),
		("3", "9223372036854775808"),
		("999999999999999999999999999999", "777777777777777777777"),
		("-999999999999999999999999999999", "777777777777777777777"),
	];

	for (lhs, rhs) in cases {
		let lhs_value = big_scaled(lhs);
		let rhs_value = big_scaled(rhs);
		let expected = reference_round_div(
			lhs.parse::<BigInt>().unwrap() * BigInt::from(DEC_I64),
			rhs.parse().unwrap(),
		);
		let actual = lhs_value.checked_div(&rhs_value).unwrap().into_scaled_bigint();
		assert_eq!(actual, expected, "{lhs} / {rhs}");
	}
}

#[test]
fn checked_division_and_remainder_reject_zero() {
	assert_eq!(BigFixed::ONE.checked_div(&BigFixed::ZERO), Err(Error::DivisionByZero));
	assert_eq!(BigFixed::ONE.checked_rem(&BigFixed::ZERO), Err(Error::DivisionByZero));
}

#[test]
#[should_panic(expected = "attempt to divide BigFixed by zero")]
fn division_operator_panics_on_zero() {
	let _ = BigFixed::ONE / BigFixed::ZERO;
}

#[test]
#[should_panic(
	expected = "attempt to calculate BigFixed remainder with a divisor of zero"
)]
fn remainder_operator_panics_on_zero() {
	let _ = BigFixed::ONE % BigFixed::ZERO;
}

#[test]
fn remainder_uses_truncating_sign_rules() {
	let cases = [(7, 3, 1), (7, -3, 1), (-7, 3, -1), (-7, -3, -1)];

	for (lhs, rhs, expected) in cases {
		assert_eq!(
			scaled(lhs).checked_rem(&scaled(rhs)).unwrap().as_scaled_i64(),
			Some(expected),
			"{lhs} % {rhs}"
		);
	}

	assert_eq!(
		BigFixed::from_scaled(i64::MIN)
			.checked_rem(&BigFixed::from_scaled(-1))
			.unwrap()
			.as_scaled_i64(),
		Some(0)
	);
}

#[test]
fn scaled_by_whole_remainder_handles_native_denominator_boundaries() {
	for lhs in [i64::MIN, -7_000_001, 7_000_001, i64::MAX] {
		for rhs in [i64::MIN, -9_223_372_036_854, -1, 1, 9_223_372_036_854, i64::MAX] {
			let lhs_value = BigFixed(Repr::Scaled(lhs));
			let rhs_value = BigFixed(Repr::Whole(rhs));
			let denominator = i128::from(rhs) * DEC_I128;
			let expected = (i128::from(lhs) % denominator) as i64;

			assert_eq!(
				lhs_value.checked_rem(&rhs_value).unwrap().as_scaled_i64(),
				Some(expected),
				"{lhs} % {rhs}"
			);
		}
	}
}

#[test]
fn signed_big_remainders_demote_to_native() {
	let lhs_magnitude = i128::from(i64::MAX) + 3;
	let rhs_magnitude = i128::from(i64::MAX) + 2;

	for (lhs, rhs, expected) in [
		(lhs_magnitude, rhs_magnitude, 1),
		(lhs_magnitude, -rhs_magnitude, 1),
		(-lhs_magnitude, rhs_magnitude, -1),
		(-lhs_magnitude, -rhs_magnitude, -1),
	] {
		let lhs = BigFixed::from_scaled_i128(lhs);
		let rhs = BigFixed::from_scaled_i128(rhs);
		assert!(!lhs.is_native());
		assert!(!rhs.is_native());
		assert_eq!(lhs.checked_rem(&rhs).unwrap().as_scaled_i64(), Some(expected));
	}
}

#[test]
fn ordering_places_large_negative_and_positive_values_correctly() {
	let below = BigFixed::from_scaled_i128(i128::from(i64::MIN) - 1);
	let min = BigFixed::from_scaled(i64::MIN);
	let max = BigFixed::from_scaled(i64::MAX);
	let above = BigFixed::from_scaled_i128(i128::from(i64::MAX) + 1);

	assert!(below < min);
	assert!(min < max);
	assert!(max < above);
}

#[test]
fn ordering_compares_whole_values_against_scaled_coefficients() {
	let positive_whole = BigFixed::from(10_000_000_000_000_i64);
	let positive_large_fraction: BigFixed = "9223372036854.775808".parse().unwrap();
	let negative_whole = BigFixed::from(-10_000_000_000_000_i64);
	let negative_large_fraction: BigFixed = "-9223372036854.775809".parse().unwrap();

	assert!(positive_whole > BigFixed::from_scaled(i64::MAX));
	assert!(positive_whole > positive_large_fraction);
	assert!(negative_whole < BigFixed::from_scaled(i64::MIN));
	assert!(negative_whole < negative_large_fraction);
	assert!(BigFixed::from(1_i64) > "0.5".parse().unwrap());
}

#[test]
fn equal_values_have_equal_hashes() {
	let parsed: BigFixed = "1.230000".parse().unwrap();
	let calculated = BigFixed::from_scaled(1_000_000) + BigFixed::from_scaled(230_000);
	assert_eq!(parsed, calculated);

	let hash = |value: &BigFixed| {
		let mut hasher = DefaultHasher::new();
		value.hash(&mut hasher);
		hasher.finish()
	};
	assert_eq!(hash(&parsed), hash(&calculated));

	let whole_forms = [
		"1".parse::<BigFixed>().unwrap(),
		"1.0".parse().unwrap(),
		"1.000000".parse().unwrap(),
		BigFixed::from_scaled(DEC_I64),
		BigFixed::from(1_i64),
	];
	for value in &whole_forms {
		assert_eq!(value, &BigFixed::ONE);
		assert_eq!(hash(value), hash(&BigFixed::ONE));
	}

	let scaled_whole = scaled(500_000) * BigFixed::from(2_i64);
	assert!(matches!(
		&scaled_whole.0,
		Repr::Scaled(coefficient) if *coefficient == DEC_I64
	));
	assert_eq!(hash(&scaled_whole), hash(&BigFixed::ONE));

	let large_whole =
		BigFixed::from_scaled(i64::MAX).checked_div(&BigFixed::from_scaled(1)).unwrap();
	let max = BigFixed::from(i64::MAX);
	assert!(matches!(&large_whole.0, Repr::Large(_)));
	assert_eq!(large_whole, max);
	assert_eq!(hash(&large_whole), hash(&max));
}

#[test]
fn assignment_operators_preserve_canonicalization() {
	let mut value = BigFixed::from_scaled(i64::MAX);
	value += BigFixed::from_scaled(1);
	assert!(!value.is_native());
	value -= BigFixed::from_scaled(1);
	assert_eq!(value.as_scaled_i64(), Some(i64::MAX));

	value *= &BigFixed::from_scaled(DEC_I64);
	value /= &BigFixed::from_scaled(DEC_I64);
	value %= &BigFixed::from_scaled(i64::MAX);
	assert!(value.is_zero());
}

#[test]
fn sign_queries_work_for_native_and_big_values() {
	assert!(BigFixed::ZERO.is_zero());
	assert!(!BigFixed::ZERO.is_positive());
	assert!(!BigFixed::ZERO.is_negative());
	assert!(scaled(1).is_positive());
	assert!(scaled(-1).is_negative());
	assert!(big_scaled("9223372036854775808").is_positive());
	assert!(big_scaled("-9223372036854775809").is_negative());
}
