#![forbid(unsafe_code)]

use std::hash::{Hash, Hasher};

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};

pub const SCALE: u32 = 6;
pub const DEC_I64: i64 = 10_i64.pow(SCALE);
pub const DEC_I128: i128 = DEC_I64 as i128;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	#[error("cannot parse BigFixed from an empty string")]
	Empty,

	#[error("invalid BigFixed decimal syntax")]
	InvalidSyntax,

	#[error("division by zero")]
	DivisionByZero,
}

/// An arbitrary-range fixed-point decimal with six fractional digits.
///
/// Whole values that fit in `i64`, and fractional values whose scaled
/// coefficient fits in `i64`, use allocation-free representations. All
/// integer constructors and all-whole operations preserve the whole-number
/// representation. Coefficient arithmetic does not eagerly test whether its
/// result is whole: that test itself requires the division this representation
/// exists to avoid, so integer queries and value comparisons normalize lazily.
#[derive(Clone)]
pub struct BigFixed(Repr);

#[derive(Clone)]
enum Repr {
	// Keep the two legacy coefficient representations first: coefficient-heavy
	// workloads retain the old Small=0, Large=1 dispatch order.
	/// A scaled coefficient, which may also denote a whole value.
	Scaled(i64),
	/// A scaled coefficient that does not fit in `i64`.
	Large(Box<BigInt>),
	/// The unscaled integer value.
	Whole(i64),
}

impl BigFixed {
	pub const ZERO: Self = Self(Repr::Whole(0));

	pub const ONE: Self = Self(Repr::Whole(1));

	pub const fn from_scaled(coefficient: i64) -> Self {
		if coefficient % DEC_I64 == 0 {
			Self(Repr::Whole(coefficient / DEC_I64))
		} else {
			Self(Repr::Scaled(coefficient))
		}
	}

	#[inline]
	pub fn from_scaled_i128(coefficient: i128) -> Self {
		if coefficient % DEC_I128 == 0
			&& let Ok(integer) = i64::try_from(coefficient / DEC_I128)
		{
			return Self(Repr::Whole(integer));
		}

		match i64::try_from(coefficient) {
			Ok(coefficient) => Self(Repr::Scaled(coefficient)),
			Err(_) => Self(Repr::Large(Box::new(BigInt::from(coefficient)))),
		}
	}

	pub fn from_scaled_bigint(coefficient: BigInt) -> Self {
		if let Some(coefficient) = coefficient.to_i64() {
			return Self::from_scaled(coefficient);
		}

		if let Some(coefficient) = coefficient.to_i128()
			&& coefficient % DEC_I128 == 0
			&& let Ok(integer) = i64::try_from(coefficient / DEC_I128)
		{
			return Self(Repr::Whole(integer));
		}

		Self(Repr::Large(Box::new(coefficient)))
	}

	pub fn from_integer(integer: BigInt) -> Self {
		match integer.to_i64() {
			Some(integer) => Self(Repr::Whole(integer)),
			None => Self(Repr::Large(Box::new(integer * BigInt::from(DEC_I64)))),
		}
	}

	/// Returns `true` when the value uses an allocation-free representation.
	///
	/// A native whole value can have a scaled coefficient too large for
	/// [`as_scaled_i64`](Self::as_scaled_i64).
	pub const fn is_native(&self) -> bool {
		matches!(self.0, Repr::Whole(_) | Repr::Scaled(_))
	}

	/// Returns the scaled coefficient when it fits in `i64`.
	pub const fn as_scaled_i64(&self) -> Option<i64> {
		match self.0 {
			Repr::Whole(integer) => integer.checked_mul(DEC_I64),
			Repr::Scaled(coefficient) => Some(coefficient),
			Repr::Large(_) => None,
		}
	}

	/// Returns the whole value when it is integer-valued and fits in `i64`.
	pub fn as_i64_integer(&self) -> Option<i64> {
		match &self.0 {
			Repr::Whole(integer) => Some(*integer),
			Repr::Scaled(coefficient) => {
				if *coefficient % DEC_I64 == 0 {
					Some(*coefficient / DEC_I64)
				} else {
					None
				}
			}
			Repr::Large(coefficient) => Self::large_as_i64_integer(coefficient),
		}
	}

	fn large_as_i64_integer(coefficient: &BigInt) -> Option<i64> {
		let coefficient = coefficient.to_i128()?;
		(coefficient % DEC_I128 == 0)
			.then(|| i64::try_from(coefficient / DEC_I128).ok())
			.flatten()
	}

	/// Returns `true` when this value has no fractional part.
	pub fn is_integer(&self) -> bool {
		match &self.0 {
			Repr::Whole(_) => true,
			Repr::Scaled(coefficient) => coefficient % DEC_I64 == 0,
			Repr::Large(coefficient) => coefficient.is_multiple_of(&BigInt::from(DEC_I64)),
		}
	}

	/// Clones the scaled coefficient into a [`BigInt`].
	pub fn to_scaled_bigint(&self) -> BigInt {
		match &self.0 {
			Repr::Whole(integer) => BigInt::from(*integer) * DEC_I64,
			Repr::Scaled(coefficient) => BigInt::from(*coefficient),
			Repr::Large(coefficient) => coefficient.as_ref().clone(),
		}
	}

	/// Consumes the value and returns its scaled coefficient.
	pub fn into_scaled_bigint(self) -> BigInt {
		match self.0 {
			Repr::Whole(integer) => BigInt::from(integer) * DEC_I64,
			Repr::Scaled(coefficient) => BigInt::from(coefficient),
			Repr::Large(coefficient) => *coefficient,
		}
	}

	/// Returns `true` when this value is zero.
	pub const fn is_zero(&self) -> bool {
		matches!(self.0, Repr::Whole(0) | Repr::Scaled(0))
	}

	/// Returns `true` when this value is greater than zero.
	pub fn is_positive(&self) -> bool {
		match &self.0 {
			Repr::Whole(integer) => *integer > 0,
			Repr::Scaled(coefficient) => *coefficient > 0,
			Repr::Large(coefficient) => coefficient.sign() == Sign::Plus,
		}
	}

	/// Returns `true` when this value is less than zero.
	pub fn is_negative(&self) -> bool {
		match &self.0 {
			Repr::Whole(integer) => *integer < 0,
			Repr::Scaled(coefficient) => *coefficient < 0,
			Repr::Large(coefficient) => coefficient.sign() == Sign::Minus,
		}
	}

	/// Returns the absolute value.
	#[inline]
	pub fn abs(&self) -> Self {
		match &self.0 {
			Repr::Whole(integer) => Self::from_integer_i128(i128::from(*integer).abs()),
			Repr::Scaled(coefficient) => {
				Self::from_scaled_i128_lazy(i128::from(*coefficient).abs())
			}
			Repr::Large(coefficient) => Self::from_scaled_bigint_lazy(coefficient.abs()),
		}
	}

	/// Divides by `rhs`, rounding to six fractional digits using nearest,
	/// ties away from zero.
	#[inline]
	pub fn checked_div(&self, rhs: &Self) -> Result<Self> {
		if rhs.is_zero() {
			return Err(Error::DivisionByZero);
		}

		let result = match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => {
				let numerator = i128::from(*lhs) * DEC_I128;
				return Ok(Self::from_scaled_i128_lazy(round_div_i128(
					numerator,
					i128::from(*rhs),
				)));
			}
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => {
				let numerator = i128::from(*lhs) * DEC_I128;
				return Ok(Self::from_scaled_i128_lazy(round_div_i128(
					numerator,
					i128::from(*rhs),
				)));
			}
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				let numerator = i128::from(*lhs) * DEC_I128 * DEC_I128;
				return Ok(Self::from_scaled_i128_lazy(round_div_i128(
					numerator,
					i128::from(*rhs),
				)));
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				return Ok(Self::from_scaled_i128_lazy(round_div_i128(
					i128::from(*lhs),
					i128::from(*rhs),
				)));
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => {
				let numerator = lhs.as_ref() * DEC_I64;
				round_div_big(numerator, rhs.as_ref())
			}
			(Repr::Large(lhs), Repr::Scaled(rhs)) => {
				let numerator = lhs.as_ref() * DEC_I64;
				round_div_big(numerator, &BigInt::from(*rhs))
			}
			(Repr::Scaled(lhs), Repr::Large(rhs)) => {
				let numerator = BigInt::from(i128::from(*lhs) * DEC_I128);
				round_div_big(numerator, rhs.as_ref())
			}
			(Repr::Large(lhs), Repr::Whole(rhs)) => {
				round_div_big(lhs.as_ref().clone(), &BigInt::from(*rhs))
			}
			(Repr::Whole(lhs), Repr::Large(rhs)) => {
				let numerator = BigInt::from(i128::from(*lhs) * DEC_I128 * DEC_I128);
				round_div_big(numerator, rhs.as_ref())
			}
		};
		Ok(Self::from_scaled_bigint_lazy(result))
	}

	/// Computes the truncating remainder of division by `rhs`.
	///
	/// This follows `a - trunc(a / b) * b`; the remainder has the sign of the
	/// left operand and does not use the rounded result returned by division.
	#[inline]
	pub fn checked_rem(&self, rhs: &Self) -> Result<Self> {
		if rhs.is_zero() {
			return Err(Error::DivisionByZero);
		}

		let result = match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => {
				// The only overflowing i64 remainder is MIN % -1, whose
				// mathematical result is zero, exactly as `wrapping_rem` returns.
				return Ok(Self(Repr::Whole(lhs.wrapping_rem(*rhs))));
			}
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => {
				return Ok(Self(Repr::Scaled(lhs.wrapping_rem(*rhs))));
			}
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				let remainder = i128::from(*lhs) * DEC_I128 % i128::from(*rhs);
				return Ok(Self(Repr::Scaled(remainder as i64)));
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				let remainder = match rhs.checked_mul(DEC_I64) {
					Some(denominator) => lhs.wrapping_rem(denominator),
					None => *lhs,
				};
				return Ok(Self(Repr::Scaled(remainder)));
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => {
				let (_, remainder) = lhs.as_ref().div_rem(rhs.as_ref());
				remainder
			}
			(Repr::Large(lhs), Repr::Scaled(rhs)) => lhs.as_ref() % *rhs,
			(Repr::Scaled(lhs), Repr::Large(rhs)) => BigInt::from(*lhs) % rhs.as_ref(),
			(Repr::Large(lhs), Repr::Whole(rhs)) => {
				lhs.as_ref() % (i128::from(*rhs) * DEC_I128)
			}
			(Repr::Whole(lhs), Repr::Large(rhs)) => {
				BigInt::from(i128::from(*lhs) * DEC_I128) % rhs.as_ref()
			}
		};
		Ok(Self::from_scaled_bigint_lazy(result))
	}

	#[inline]
	fn from_scaled_i128_lazy(coefficient: i128) -> Self {
		match i64::try_from(coefficient) {
			Ok(coefficient) => Self(Repr::Scaled(coefficient)),
			Err(_) => Self(Repr::Large(Box::new(BigInt::from(coefficient)))),
		}
	}

	fn from_scaled_bigint_lazy(coefficient: BigInt) -> Self {
		match coefficient.to_i64() {
			Some(coefficient) => Self(Repr::Scaled(coefficient)),
			None => Self(Repr::Large(Box::new(coefficient))),
		}
	}

	fn from_integer_i128(integer: i128) -> Self {
		match i64::try_from(integer) {
			Ok(integer) => Self(Repr::Whole(integer)),
			Err(_) => {
				Self(Repr::Large(Box::new(BigInt::from(integer) * BigInt::from(DEC_I64))))
			}
		}
	}

	#[inline]
	fn add_impl(&self, rhs: &Self) -> Self {
		match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => match lhs.checked_add(*rhs) {
				Some(sum) => Self(Repr::Whole(sum)),
				None => Self::from_integer_i128(i128::from(*lhs) + i128::from(*rhs)),
			},
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => match lhs.checked_add(*rhs) {
				Some(sum) => Self(Repr::Scaled(sum)),
				None => Self::from_scaled_i128_lazy(i128::from(*lhs) + i128::from(*rhs)),
			},
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				Self::from_scaled_i128_lazy(i128::from(*lhs) * DEC_I128 + i128::from(*rhs))
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				Self::from_scaled_i128_lazy(i128::from(*lhs) + i128::from(*rhs) * DEC_I128)
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => {
				Self::from_scaled_bigint_lazy(lhs.as_ref() + rhs.as_ref())
			}
			(Repr::Large(lhs), Repr::Scaled(rhs))
			| (Repr::Scaled(rhs), Repr::Large(lhs)) => {
				Self::from_scaled_bigint_lazy(lhs.as_ref() + *rhs)
			}
			(Repr::Large(lhs), Repr::Whole(rhs))
			| (Repr::Whole(rhs), Repr::Large(lhs)) => {
				let rhs = i128::from(*rhs) * DEC_I128;
				Self::from_scaled_bigint_lazy(lhs.as_ref() + rhs)
			}
		}
	}

	#[inline(always)]
	fn sub_impl(&self, rhs: &Self) -> Self {
		match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => match lhs.checked_sub(*rhs) {
				Some(difference) => Self(Repr::Whole(difference)),
				None => Self::from_integer_i128(i128::from(*lhs) - i128::from(*rhs)),
			},
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => match lhs.checked_sub(*rhs) {
				Some(difference) => Self(Repr::Scaled(difference)),
				None => Self::from_scaled_i128_lazy(i128::from(*lhs) - i128::from(*rhs)),
			},
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				Self::from_scaled_i128_lazy(i128::from(*lhs) * DEC_I128 - i128::from(*rhs))
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				Self::from_scaled_i128_lazy(i128::from(*lhs) - i128::from(*rhs) * DEC_I128)
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => {
				Self::from_scaled_bigint_lazy(lhs.as_ref() - rhs.as_ref())
			}
			(Repr::Large(lhs), Repr::Scaled(rhs)) => {
				Self::from_scaled_bigint_lazy(lhs.as_ref() - *rhs)
			}
			(Repr::Scaled(lhs), Repr::Large(rhs)) => {
				Self::from_scaled_bigint_lazy(*lhs - rhs.as_ref())
			}
			(Repr::Large(lhs), Repr::Whole(rhs)) => {
				let rhs = i128::from(*rhs) * DEC_I128;
				Self::from_scaled_bigint_lazy(lhs.as_ref() - rhs)
			}
			(Repr::Whole(lhs), Repr::Large(rhs)) => {
				let lhs = i128::from(*lhs) * DEC_I128;
				Self::from_scaled_bigint_lazy(lhs - rhs.as_ref())
			}
		}
	}

	// Keep the large value-operation dispatch out of SubAssign's native fast
	// path. `sub_impl` is forced inline for `a - b`, but pulling it into `a -= b`
	// makes the common checked-i64 branch substantially larger.
	#[inline(never)]
	fn sub_assign_fallback(&self, rhs: &Self) -> Self {
		self.sub_impl(rhs)
	}

	#[inline]
	fn mul_impl(&self, rhs: &Self) -> Self {
		match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => match lhs.checked_mul(*rhs) {
				Some(product) => Self(Repr::Whole(product)),
				None => Self::from_integer_i128(i128::from(*lhs) * i128::from(*rhs)),
			},
			(Repr::Whole(lhs), Repr::Scaled(rhs))
			| (Repr::Scaled(rhs), Repr::Whole(lhs)) => {
				Self::from_scaled_i128_lazy(i128::from(*lhs) * i128::from(*rhs))
			}
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => {
				let numerator = i128::from(*lhs) * i128::from(*rhs);
				Self::from_scaled_i128_lazy(round_div_i128(numerator, DEC_I128))
			}
			(Repr::Whole(lhs), Repr::Large(rhs))
			| (Repr::Large(rhs), Repr::Whole(lhs)) => {
				Self::from_scaled_bigint_lazy(rhs.as_ref() * *lhs)
			}
			(Repr::Scaled(lhs), Repr::Large(rhs))
			| (Repr::Large(rhs), Repr::Scaled(lhs)) => {
				let numerator = rhs.as_ref() * *lhs;
				Self::from_scaled_bigint_lazy(round_div_big(
					numerator,
					&BigInt::from(DEC_I64),
				))
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => {
				let numerator = lhs.as_ref() * rhs.as_ref();
				Self::from_scaled_bigint_lazy(round_div_big(
					numerator,
					&BigInt::from(DEC_I64),
				))
			}
		}
	}

	#[inline]
	fn div_impl(&self, rhs: &Self) -> Self {
		self
			.checked_div(rhs)
			.unwrap_or_else(|_| panic!("attempt to divide BigFixed by zero"))
	}

	#[inline]
	fn rem_impl(&self, rhs: &Self) -> Self {
		self.checked_rem(rhs).unwrap_or_else(|_| {
			panic!("attempt to calculate BigFixed remainder with a divisor of zero")
		})
	}

	#[inline]
	fn neg_impl(&self) -> Self {
		match &self.0 {
			Repr::Whole(integer) => match integer.checked_neg() {
				Some(negated) => Self(Repr::Whole(negated)),
				None => Self::from_integer_i128(-i128::from(*integer)),
			},
			Repr::Scaled(coefficient) => match coefficient.checked_neg() {
				Some(negated) => Self(Repr::Scaled(negated)),
				None => Self::from_scaled_i128_lazy(-i128::from(*coefficient)),
			},
			Repr::Large(coefficient) => {
				Self::from_scaled_bigint_lazy(-coefficient.as_ref())
			}
		}
	}
}

impl std::fmt::Debug for BigFixed {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.debug_tuple("BigFixed").field(&self.to_string()).finish()
	}
}

impl std::fmt::Display for BigFixed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let (mut digits, is_scaled) = match &self.0 {
			Repr::Whole(value) => (value.to_string(), false),
			Repr::Scaled(value) => (value.to_string(), true),
			Repr::Large(value) => (value.to_str_radix(10), true),
		};
		if is_scaled {
			let scale = SCALE as usize;
			let sign_len = usize::from(digits.starts_with('-'));
			let zero_count = (scale + 1 + sign_len).saturating_sub(digits.len());
			if zero_count != 0 {
				digits.insert_str(sign_len, &"0".repeat(zero_count));
			}
			digits.insert(digits.len() - scale, '.');
			digits.truncate(digits.trim_end_matches('0').len());
			if digits.ends_with('.') {
				digits.pop();
			}
		}
		let (is_nonnegative, magnitude) = match digits.strip_prefix('-') {
			Some(magnitude) => (false, magnitude),
			None => (true, digits.as_str()),
		};
		f.pad_integral(is_nonnegative, "", magnitude)
	}
}

impl Default for BigFixed {
	fn default() -> Self {
		Self::ZERO
	}
}

impl PartialEq for BigFixed {
	fn eq(&self, other: &Self) -> bool {
		match (&self.0, &other.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => lhs == rhs,
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => lhs == rhs,
			(Repr::Large(lhs), Repr::Large(rhs)) => lhs == rhs,
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				i128::from(*lhs) * DEC_I128 == i128::from(*rhs)
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				i128::from(*lhs) == i128::from(*rhs) * DEC_I128
			}
			(Repr::Large(lhs), Repr::Whole(rhs)) => {
				cmp_large_to_i128(lhs, i128::from(*rhs) * DEC_I128).is_eq()
			}
			(Repr::Whole(lhs), Repr::Large(rhs)) => {
				cmp_large_to_i128(rhs, i128::from(*lhs) * DEC_I128).is_eq()
			}
			(Repr::Scaled(_), Repr::Large(_)) | (Repr::Large(_), Repr::Scaled(_)) => false,
		}
	}
}

impl Eq for BigFixed {}

impl PartialOrd for BigFixed {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for BigFixed {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		match (&self.0, &other.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs)) => lhs.cmp(rhs),
			(Repr::Scaled(lhs), Repr::Scaled(rhs)) => lhs.cmp(rhs),
			(Repr::Whole(lhs), Repr::Scaled(rhs)) => {
				(i128::from(*lhs) * DEC_I128).cmp(&i128::from(*rhs))
			}
			(Repr::Scaled(lhs), Repr::Whole(rhs)) => {
				i128::from(*lhs).cmp(&(i128::from(*rhs) * DEC_I128))
			}
			(Repr::Large(lhs), Repr::Large(rhs)) => lhs.cmp(rhs),
			(Repr::Large(lhs), Repr::Scaled(_)) => match lhs.sign() {
				Sign::Plus => std::cmp::Ordering::Greater,
				Sign::Minus => std::cmp::Ordering::Less,
				_ => unreachable!("zero must use a native representation"),
			},
			(Repr::Scaled(_), Repr::Large(rhs)) => match rhs.sign() {
				Sign::Plus => std::cmp::Ordering::Less,
				Sign::Minus => std::cmp::Ordering::Greater,
				_ => unreachable!("zero must use a native representation"),
			},
			(Repr::Large(lhs), Repr::Whole(rhs)) => {
				cmp_large_to_i128(lhs, i128::from(*rhs) * DEC_I128)
			}
			(Repr::Whole(lhs), Repr::Large(rhs)) => {
				cmp_large_to_i128(rhs, i128::from(*lhs) * DEC_I128).reverse()
			}
		}
	}
}

impl Hash for BigFixed {
	fn hash<H: Hasher>(&self, state: &mut H) {
		match &self.0 {
			Repr::Whole(integer) => {
				hash_scaled_i128(i128::from(*integer) * DEC_I128, state)
			}
			Repr::Scaled(coefficient) => hash_scaled_i128(i128::from(*coefficient), state),
			Repr::Large(coefficient) => hash_scaled_bigint(coefficient, state),
		}
	}
}

fn cmp_large_to_i128(lhs: &BigInt, rhs: i128) -> std::cmp::Ordering {
	match lhs.to_i128() {
		Some(lhs) => lhs.cmp(&rhs),
		None => match lhs.sign() {
			Sign::Plus => std::cmp::Ordering::Greater,
			Sign::Minus => std::cmp::Ordering::Less,
			Sign::NoSign => unreachable!("zero must use a native representation"),
		},
	}
}

fn hash_scaled_i128<H: Hasher>(coefficient: i128, state: &mut H) {
	let sign = match coefficient.cmp(&0) {
		std::cmp::Ordering::Less => 2_u8,
		std::cmp::Ordering::Equal => 0_u8,
		std::cmp::Ordering::Greater => 1_u8,
	};
	sign.hash(state);

	let magnitude = coefficient.unsigned_abs();
	if magnitude != 0 {
		1_u8.hash(state);
		(magnitude as u64).hash(state);
		let high = (magnitude >> 64) as u64;
		if high != 0 {
			1_u8.hash(state);
			high.hash(state);
		}
	}
	0_u8.hash(state);
}

fn hash_scaled_bigint<H: Hasher>(coefficient: &BigInt, state: &mut H) {
	let sign = match coefficient.sign() {
		Sign::NoSign => 0_u8,
		Sign::Plus => 1_u8,
		Sign::Minus => 2_u8,
	};
	sign.hash(state);
	for digit in coefficient.iter_u64_digits() {
		1_u8.hash(state);
		digit.hash(state);
	}
	0_u8.hash(state);
}

impl std::str::FromStr for BigFixed {
	type Err = Error;

	fn from_str(source: &str) -> Result<Self> {
		if source.is_empty() {
			return Err(Error::Empty);
		}

		let (negative, unsigned) = if let Some(unsigned) = source.strip_prefix('-') {
			(true, unsigned)
		} else if let Some(unsigned) = source.strip_prefix('+') {
			(false, unsigned)
		} else {
			(false, source)
		};

		if unsigned.is_empty() {
			return Err(Error::InvalidSyntax);
		}

		let (integer, fraction) = match unsigned.split_once('.') {
			Some((integer, fraction)) => (integer, Some(fraction)),
			None => (unsigned, None),
		};

		if integer.is_empty()
			|| !integer.bytes().all(|byte| byte.is_ascii_digit())
			|| fraction.is_some_and(|fraction| {
				fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
			}) {
			return Err(Error::InvalidSyntax);
		}

		let fraction_coefficient = if let Some(fraction) = fraction {
			let bytes = fraction.as_bytes();
			let mut fraction_coefficient = 0_i64;
			for index in 0..SCALE as usize {
				fraction_coefficient *= 10;
				if let Some(digit) = bytes.get(index) {
					fraction_coefficient += i64::from(*digit - b'0');
				}
			}

			if bytes.get(SCALE as usize).is_some_and(|digit| *digit >= b'5') {
				fraction_coefficient += 1;
			}

			fraction_coefficient
		} else {
			0
		};

		if let Ok(integer) = integer.parse::<u64>() {
			let mut coefficient =
				i128::from(integer) * DEC_I128 + i128::from(fraction_coefficient);
			if negative {
				coefficient = -coefficient;
			}
			return Ok(Self::from_scaled_i128(coefficient));
		}

		let mut magnitude = BigInt::parse_bytes(integer.as_bytes(), 10)
			.expect("validated decimal digits must parse as BigInt");
		magnitude *= DEC_I64;
		magnitude += fraction_coefficient;

		if negative {
			magnitude = -magnitude;
		}

		Ok(Self::from_scaled_bigint(magnitude))
	}
}

impl TryFrom<&str> for BigFixed {
	type Error = Error;

	fn try_from(value: &str) -> Result<Self> {
		value.parse()
	}
}

impl From<i64> for BigFixed {
	fn from(integer: i64) -> Self {
		Self(Repr::Whole(integer))
	}
}

impl From<BigInt> for BigFixed {
	fn from(integer: BigInt) -> Self {
		Self::from_integer(integer)
	}
}

#[inline]
fn round_div_i128(numerator: i128, denominator: i128) -> i128 {
	debug_assert_ne!(denominator, 0);

	let mut quotient = numerator / denominator;
	let remainder = numerator - quotient * denominator;
	let remainder_abs = remainder.unsigned_abs();
	let denominator_abs = denominator.unsigned_abs();

	if remainder_abs >= denominator_abs - remainder_abs {
		if (numerator < 0) == (denominator < 0) {
			quotient += 1;
		} else {
			quotient -= 1;
		}
	}

	quotient
}

fn round_div_big(numerator: BigInt, denominator: &BigInt) -> BigInt {
	debug_assert!(!denominator.is_zero());

	let (mut quotient, remainder) = numerator.div_rem(denominator);
	let twice_remainder = remainder.magnitude() << 1_u32;

	if twice_remainder >= *denominator.magnitude() {
		if numerator.sign() == denominator.sign() {
			quotient += 1;
		} else {
			quotient -= 1;
		}
	}

	quotient
}

impl std::ops::Neg for BigFixed {
	type Output = BigFixed;

	#[inline]
	fn neg(self) -> Self::Output {
		self.neg_impl()
	}
}

impl std::ops::Neg for &BigFixed {
	type Output = BigFixed;

	#[inline]
	fn neg(self) -> Self::Output {
		self.neg_impl()
	}
}

macro_rules! impl_binary_operator {
	($trait:ident, $method:ident, $implementation:ident) => {
		impl std::ops::$trait for BigFixed {
			type Output = BigFixed;

			#[inline]
			fn $method(self, rhs: BigFixed) -> Self::Output {
				BigFixed::$implementation(&self, &rhs)
			}
		}

		impl<'rhs> std::ops::$trait<&'rhs BigFixed> for BigFixed {
			type Output = BigFixed;

			#[inline]
			fn $method(self, rhs: &'rhs BigFixed) -> Self::Output {
				BigFixed::$implementation(&self, rhs)
			}
		}

		impl<'lhs> std::ops::$trait<BigFixed> for &'lhs BigFixed {
			type Output = BigFixed;

			#[inline]
			fn $method(self, rhs: BigFixed) -> Self::Output {
				BigFixed::$implementation(self, &rhs)
			}
		}

		impl<'lhs, 'rhs> std::ops::$trait<&'rhs BigFixed> for &'lhs BigFixed {
			type Output = BigFixed;

			#[inline]
			fn $method(self, rhs: &'rhs BigFixed) -> Self::Output {
				BigFixed::$implementation(self, rhs)
			}
		}
	};
}

impl_binary_operator!(Add, add, add_impl);
impl_binary_operator!(Sub, sub, sub_impl);
impl_binary_operator!(Mul, mul, mul_impl);
impl_binary_operator!(Div, div, div_impl);
impl_binary_operator!(Rem, rem, rem_impl);

#[inline]
fn native_add(lhs: &Repr, rhs: &Repr) -> Option<Repr> {
	match (lhs, rhs) {
		(Repr::Whole(lhs), Repr::Whole(rhs)) => lhs.checked_add(*rhs).map(Repr::Whole),
		(Repr::Scaled(lhs), Repr::Scaled(rhs)) => lhs.checked_add(*rhs).map(Repr::Scaled),
		_ => None,
	}
}

#[inline]
fn native_sub(lhs: &Repr, rhs: &Repr) -> Option<Repr> {
	match (lhs, rhs) {
		(Repr::Whole(lhs), Repr::Whole(rhs)) => lhs.checked_sub(*rhs).map(Repr::Whole),
		(Repr::Scaled(lhs), Repr::Scaled(rhs)) => lhs.checked_sub(*rhs).map(Repr::Scaled),
		_ => None,
	}
}

#[inline]
fn native_mul(lhs: &Repr, rhs: &Repr) -> Option<Repr> {
	match (lhs, rhs) {
		(Repr::Whole(lhs), Repr::Whole(rhs)) => lhs.checked_mul(*rhs).map(Repr::Whole),
		(Repr::Scaled(lhs), Repr::Scaled(rhs)) => {
			let numerator = i128::from(*lhs) * i128::from(*rhs);
			i64::try_from(round_div_i128(numerator, DEC_I128)).ok().map(Repr::Scaled)
		}
		_ => None,
	}
}

#[inline]
fn native_rem(lhs: &Repr, rhs: &Repr) -> Option<Repr> {
	match (lhs, rhs) {
		(Repr::Whole(lhs), Repr::Whole(rhs)) if *rhs != 0 => {
			Some(Repr::Whole(lhs.wrapping_rem(*rhs)))
		}
		(Repr::Scaled(lhs), Repr::Scaled(rhs)) if *rhs != 0 => {
			Some(Repr::Scaled(lhs.wrapping_rem(*rhs)))
		}
		_ => None,
	}
}

macro_rules! impl_assignment_operator {
	($trait:ident, $method:ident, $implementation:ident, $native:ident) => {
		impl std::ops::$trait<BigFixed> for BigFixed {
			#[inline]
			fn $method(&mut self, rhs: BigFixed) {
				std::ops::$trait::$method(self, &rhs);
			}
		}

		impl<'rhs> std::ops::$trait<&'rhs BigFixed> for BigFixed {
			#[inline]
			fn $method(&mut self, rhs: &'rhs BigFixed) {
				if let Some(result) = $native(&self.0, &rhs.0) {
					self.0 = result;
					return;
				}
				*self = BigFixed::$implementation(self, rhs);
			}
		}
	};
}

impl_assignment_operator!(AddAssign, add_assign, add_impl, native_add);
impl_assignment_operator!(SubAssign, sub_assign, sub_assign_fallback, native_sub);
impl_assignment_operator!(MulAssign, mul_assign, mul_impl, native_mul);
impl_assignment_operator!(RemAssign, rem_assign, rem_impl, native_rem);

impl std::ops::DivAssign<BigFixed> for BigFixed {
	#[inline]
	fn div_assign(&mut self, rhs: BigFixed) {
		std::ops::DivAssign::div_assign(self, &rhs);
	}
}

impl std::ops::DivAssign<&BigFixed> for BigFixed {
	#[inline]
	fn div_assign(&mut self, rhs: &BigFixed) {
		let operands = match (&self.0, &rhs.0) {
			(Repr::Whole(lhs), Repr::Whole(rhs))
			| (Repr::Scaled(lhs), Repr::Scaled(rhs)) => Some((*lhs, *rhs)),
			_ => None,
		};
		if let Some((lhs, rhs)) = operands
			&& rhs != 0
		{
			let numerator = i128::from(lhs) * DEC_I128;
			if let Ok(result) = i64::try_from(round_div_i128(numerator, i128::from(rhs))) {
				self.0 = Repr::Scaled(result);
				return;
			}
		}
		*self = BigFixed::div_impl(self, rhs);
	}
}

#[cfg(test)]
mod tests;
