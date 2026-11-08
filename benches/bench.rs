//! Fixed-point arithmetic benchmarks for `BigFixed`.
//!
//! Run with `cargo bench --bench bench`.
//!
//! # Suites
//!
//! | suite    | operands                        | arms                 |
//! |----------|---------------------------------|----------------------|
//! | `native` | both fit `i64`                  | i64, i128, BigFixed  |
//! | `wide`   | both exceed `i64`, fit `i128`   | i128, BigFixed       |
//! | `mixed`  | left exceeds `i64`, right fits  | i128, BigFixed       |
//! | `assign` | in-place operators, `mid` band  | per-range arms       |
//!
//! `i64` is deliberately absent from `wide` and `mixed`: the operands are not
//! representable, so an `i64` arm would have to measure different inputs. In
//! those suites `i128` is the reference and `BigFixed` runs its `Repr::Big`
//! path (`mixed` additionally exercises `coefficient_cow`, which heap-allocates
//! a `BigInt` for the native-sided operand on every call).
//!
//! # Magnitude bands
//!
//! Each operation is swept across `small`, `mid`, and `limit`. The bands are
//! specified as target bit widths per operand and are chosen per operation,
//! because the binding constraint differs: addition is limited by operand
//! width, multiplication by the *sum* of operand widths (the product must not
//! overflow the intermediate), and division by `bits(lhs) - bits(rhs)` plus the
//! scale. `limit` pushes each operation as close to its own ceiling as the
//! reference type allows, which is where the 128-bit software divide routines
//! stop hitting their narrow-operand fast paths.
//!
//! # Two structural facts worth knowing before reading the numbers
//!
//! * **`wide` has no `mul`.** With both operands at 64+ bits the intermediate
//!   product needs 128+ bits and overflows `i128`, so no `i128` reference
//!   exists. Large-magnitude multiplication is covered by `mixed/mul`, where
//!   the right operand is bounded to keep the product representable.
//! * **`wide/div` always demotes.** The numerator is capped at 106 bits (any
//!   wider and `lhs * SCALE` overflows `i128`) and the divisor is at least 64
//!   bits, so the quotient never exceeds 62 bits and `BigFixed` canonicalizes
//!   back to `Repr::Native`. That suite measures the big-to-native demotion
//!   path, not big-to-big division.
//!
//! # Fairness rules
//!
//! * Every arm reads operands out of a slice in memory; none gets to keep them
//!   in registers while another pays for loads.
//! * Every arm performs `PAIRS` operations per Criterion iteration, so fixed
//!   loop overhead is amortised identically.
//! * `black_box` is applied at identical points: once on the input slice, once
//!   on each result.
//! * Assignment arms return the mutated accumulator buffer as the measured
//!   output. Criterion observes it after stopping the timer, so primitive
//!   writes cannot be removed as dead stores; reset cloning and final drops
//!   remain outside the measurement.
//! * Assignment uses Criterion's `LargeInput` batching for every arm. Each
//!   logical input is a 64-element buffer (and wide `BigFixed` buffers own heap
//!   allocations), so `SmallInput` would retain an excessive, backend-dependent
//!   amount of setup data between timed batches.
//! * `SCALE` stays a compile-time constant everywhere, matching what `BigFixed`
//!   sees internally.
//! * Inputs are deterministic (fixed seed, per-band) and every generated pair is
//!   validated against a checked reference before timing.
//!
//! # Runtime
//!
//! This is ~120 benchmarks, roughly a quarter hour at Criterion defaults.
//! Filter while iterating:
//!
//! ```text
//! cargo bench -- 'native/add'
//! cargo bench -- 'wide/'
//! cargo bench -- 'assign/'
//! cargo bench -- --measurement-time 2 --warm-up-time 1
//! ```
//!
//! Sub-nanosecond arms are sensitive to code layout across relinks; save a
//! baseline (`--save-baseline`) and compare against it rather than eyeballing
//! absolute numbers between builds.

use std::hint::black_box;

use bigfixed::{BigFixed, DEC_I64};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};

const SCALE_I128: i128 = DEC_I64 as i128;

/// Operand pairs per Criterion iteration.
const PAIRS: usize = 64;

/// Base seed; each (suite, op, band) derives its own stream from this.
const SEED: u64 = 0x5EED_1234_5678_9ABC;

// ---------------------------------------------------------------------------
// Reference implementations (mirroring `bigfixed`'s rounding exactly)
// ---------------------------------------------------------------------------

/// Round-half-away-from-zero division. Hot path, must stay branch-for-branch
/// equivalent to the library's `round_div_i128`.
fn round_div_i128(numerator: i128, denominator: i128) -> i128 {
	debug_assert_ne!(denominator, 0);

	let mut quotient = numerator / denominator;
	let remainder = numerator % denominator;
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

fn fixed_mul_i128(lhs: i128, rhs: i128) -> i128 {
	round_div_i128(lhs * rhs, SCALE_I128)
}

fn fixed_div_i128(lhs: i128, rhs: i128) -> i128 {
	round_div_i128(lhs * SCALE_I128, rhs)
}

fn narrow_to_i64(value: i128) -> i64 {
	i64::try_from(value).expect("generated inputs must produce an i64 result")
}

/// The i64 arms share the i128 core, so the two measurements differ only by
/// operand width and the narrowing check a real i64 fixed-point type must pay.
fn fixed_mul_i64(lhs: i64, rhs: i64) -> i64 {
	narrow_to_i64(fixed_mul_i128(i128::from(lhs), i128::from(rhs)))
}

fn fixed_div_i64(lhs: i64, rhs: i64) -> i64 {
	narrow_to_i64(fixed_div_i128(i128::from(lhs), i128::from(rhs)))
}

/// Setup-time validator. Returns `None` when the operation is undefined or
/// overflows `i128` anywhere along the way, including in the rounding step.
fn checked_round_div(numerator: i128, denominator: i128) -> Option<i128> {
	if denominator == 0 {
		return None;
	}

	let quotient = numerator.checked_div(denominator)?;
	let remainder = numerator.checked_rem(denominator)?;
	let remainder_abs = remainder.unsigned_abs();
	let denominator_abs = denominator.unsigned_abs();

	if remainder_abs >= denominator_abs - remainder_abs {
		if (numerator < 0) == (denominator < 0) {
			quotient.checked_add(1)
		} else {
			quotient.checked_sub(1)
		}
	} else {
		Some(quotient)
	}
}

// ---------------------------------------------------------------------------
// Operations, suites, bands
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Op {
	Add,
	Sub,
	Mul,
	Div,
	Rem,
}

impl Op {
	fn name(self) -> &'static str {
		match self {
			Self::Add => "add",
			Self::Sub => "sub",
			Self::Mul => "mul",
			Self::Div => "div",
			Self::Rem => "rem",
		}
	}

	/// Reference semantics, used to validate inputs and to check that every
	/// `BigFixed` result agrees before anything is timed.
	fn apply_checked(self, lhs: i128, rhs: i128) -> Option<i128> {
		match self {
			Self::Add => lhs.checked_add(rhs),
			Self::Sub => lhs.checked_sub(rhs),
			Self::Mul => checked_round_div(lhs.checked_mul(rhs)?, SCALE_I128),
			Self::Div => checked_round_div(lhs.checked_mul(SCALE_I128)?, rhs),
			Self::Rem => lhs.checked_rem(rhs),
		}
	}
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Suite {
	/// Both operands fit `i64`; results are constrained to fit too, so
	/// `BigFixed` never leaves `Repr::Native`.
	Native,
	/// Both operands exceed `i64`, so `BigFixed` starts on `Repr::Big`.
	Wide,
	/// Left operand exceeds `i64`, right operand fits. Exercises the
	/// native/big crossover and `coefficient_cow`.
	Mixed,
}

impl Suite {
	fn name(self) -> &'static str {
		match self {
			Self::Native => "native",
			Self::Wide => "wide",
			Self::Mixed => "mixed",
		}
	}
}

/// `(band name, lhs magnitude bits, rhs magnitude bits)`.
type Band = (&'static str, u32, u32);

/// Both operands must fit `i64` and so must the result, so the ceiling is 63
/// magnitude bits everywhere. `mul` spends that budget across the operand pair
/// (`bits(lhs) + bits(rhs) - log2(SCALE)`), `div` across their difference.
fn native_bands(op: Op) -> [Band; 3] {
	match op {
		Op::Add | Op::Sub => [("small", 20, 20), ("mid", 40, 40), ("limit", 62, 62)],
		Op::Mul => [("small", 26, 26), ("mid", 36, 36), ("limit", 52, 30)],
		Op::Div => [("small", 26, 26), ("mid", 46, 26), ("limit", 62, 20)],
		Op::Rem => [("small", 26, 26), ("mid", 46, 46), ("limit", 62, 62)],
	}
}

/// Both operands at 64+ bits, results held inside `i128`. `div` caps the
/// numerator at 106 bits because `lhs * SCALE` must stay representable.
fn wide_bands(op: Op) -> [Band; 3] {
	match op {
		Op::Add | Op::Sub => [("small", 64, 64), ("mid", 96, 96), ("limit", 125, 125)],
		Op::Div => [("small", 70, 66), ("mid", 96, 80), ("limit", 106, 64)],
		Op::Rem => [("small", 70, 68), ("mid", 100, 96), ("limit", 126, 124)],
		// See the module docs: both operands wide makes the product overflow
		// i128, leaving no reference to compare against.
		Op::Mul => unreachable!("wide/mul has no i128 reference"),
	}
}

/// Left operand wide, right operand native-sized. `mul` keeps the operand bit
/// sum under 127 so the intermediate product survives.
fn mixed_bands(op: Op) -> [Band; 3] {
	match op {
		Op::Add | Op::Sub => [("small", 64, 20), ("mid", 96, 40), ("limit", 125, 62)],
		Op::Mul => [("small", 64, 26), ("mid", 90, 30), ("limit", 100, 26)],
		Op::Div => [("small", 70, 30), ("mid", 96, 40), ("limit", 106, 24)],
		Op::Rem => [("small", 70, 30), ("mid", 96, 40), ("limit", 126, 62)],
	}
}

fn bands_for(suite: Suite, op: Op) -> [Band; 3] {
	match suite {
		Suite::Native => native_bands(op),
		Suite::Wide => wide_bands(op),
		Suite::Mixed => mixed_bands(op),
	}
}

/// Operations available in each suite. `wide` omits `mul`.
fn ops_for(suite: Suite) -> &'static [Op] {
	match suite {
		Suite::Native | Suite::Mixed => &[Op::Add, Op::Sub, Op::Mul, Op::Div, Op::Rem],
		Suite::Wide => &[Op::Add, Op::Sub, Op::Div, Op::Rem],
	}
}

// ---------------------------------------------------------------------------
// Deterministic input generation
// ---------------------------------------------------------------------------

/// SplitMix64: small, dependency-free, reproducible.
struct SplitMix64(u64);

impl SplitMix64 {
	fn next_u64(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^ (z >> 31)
	}

	fn next_u128(&mut self) -> u128 {
		(u128::from(self.next_u64()) << 64) | u128::from(self.next_u64())
	}
}

/// Samples a value whose magnitude occupies exactly `bits` bits, with a random
/// sign. Capped at 126 bits so the magnitude always fits `i128`.
fn sample_with_bits(rng: &mut SplitMix64, bits: u32) -> i128 {
	assert!(
		(1..=126).contains(&bits),
		"band bit width {bits} outside the representable range"
	);

	let low = 1_i128 << (bits - 1);
	let magnitude = low + (rng.next_u128() % (low as u128)) as i128;

	if rng.next_u64() & 1 == 0 { magnitude } else { -magnitude }
}

/// Derives an independent stream per (suite, op, band) so adding a band never
/// shifts the inputs of the others.
fn derive_seed(suite: Suite, op: Op, band: &str) -> u64 {
	let mut hash = SEED;
	for byte in suite.name().bytes().chain(op.name().bytes()).chain(band.bytes()) {
		hash = hash.rotate_left(7) ^ u64::from(byte).wrapping_mul(0x100_0000_01B3);
	}
	hash | 1
}

fn fits_i64(value: i128) -> bool {
	i64::try_from(value).is_ok()
}

fn generate_pairs(suite: Suite, op: Op, band: Band) -> Vec<(i128, i128)> {
	let (band_name, lhs_bits, rhs_bits) = band;
	let mut rng = SplitMix64(derive_seed(suite, op, band_name));
	let mut pairs = Vec::with_capacity(PAIRS);
	let mut attempts = 0_u32;

	while pairs.len() < PAIRS {
		attempts += 1;
		assert!(
			attempts < 1_000_000,
			"{}/{}/{band_name} at {lhs_bits}/{rhs_bits} bits is unsatisfiable; \
             the band table is wrong",
			suite.name(),
			op.name(),
		);

		let lhs = sample_with_bits(&mut rng, lhs_bits);
		let rhs = sample_with_bits(&mut rng, rhs_bits);

		if rhs == 0 {
			continue;
		}

		// Reject anything the reference cannot compute without overflow.
		let Some(result) = op.apply_checked(lhs, rhs) else {
			continue;
		};

		let ok = match suite {
			// Operands and result all stay native.
			Suite::Native => fits_i64(lhs) && fits_i64(rhs) && fits_i64(result),
			// Both operands must leave the native representation.
			Suite::Wide => !fits_i64(lhs) && !fits_i64(rhs),
			// Exactly the left operand leaves it.
			Suite::Mixed => !fits_i64(lhs) && fits_i64(rhs),
		};

		if ok {
			pairs.push((lhs, rhs));
		}
	}

	pairs
}

// ---------------------------------------------------------------------------
// Operand storage
// ---------------------------------------------------------------------------

struct Operands {
	/// Present only for the `native` suite, where the operands are
	/// representable. Omitting it elsewhere keeps every arm on identical
	/// inputs.
	scaled_i64: Option<Vec<(i64, i64)>>,
	scaled_i128: Vec<(i128, i128)>,
	big_fixed: Vec<(BigFixed, BigFixed)>,
}

impl Operands {
	fn build(suite: Suite, op: Op, band: Band) -> Self {
		let pairs = generate_pairs(suite, op, band);

		let scaled_i64 = (suite == Suite::Native)
			.then(|| pairs.iter().map(|&(lhs, rhs)| (lhs as i64, rhs as i64)).collect());

		let big_fixed = pairs
			.iter()
			.map(|&(lhs, rhs)| {
				(BigFixed::from_scaled_i128(lhs), BigFixed::from_scaled_i128(rhs))
			})
			.collect();

		let operands = Self { scaled_i64, scaled_i128: pairs, big_fixed };
		operands.verify(suite, op, band);
		operands
	}

	/// Confirms the representation assumptions the suite name promises, and
	/// that `BigFixed` agrees with the reference on every pair. A faster arm
	/// computing something else is not faster.
	fn verify(&self, suite: Suite, op: Op, band: Band) {
		let (band_name, ..) = band;
		let label = format!("{}/{}/{band_name}", suite.name(), op.name());

		for (&(lhs, rhs), (big_lhs, big_rhs)) in
			self.scaled_i128.iter().zip(&self.big_fixed)
		{
			match suite {
				Suite::Native => assert!(
					big_lhs.is_native() && big_rhs.is_native(),
					"{label}: operands must stay native ({lhs}, {rhs})"
				),
				Suite::Wide => assert!(
					!big_lhs.is_native() && !big_rhs.is_native(),
					"{label}: both operands must be big ({lhs}, {rhs})"
				),
				Suite::Mixed => assert!(
					!big_lhs.is_native() && big_rhs.is_native(),
					"{label}: expected big/native operands ({lhs}, {rhs})"
				),
			}

			let expected =
				op.apply_checked(lhs, rhs).expect("generation already validated this pair");
			let actual = match op {
				Op::Add => big_lhs + big_rhs,
				Op::Sub => big_lhs - big_rhs,
				Op::Mul => big_lhs * big_rhs,
				Op::Div => big_lhs / big_rhs,
				Op::Rem => big_lhs % big_rhs,
			};

			assert_eq!(
				actual,
				BigFixed::from_scaled_i128(expected),
				"{label}: BigFixed disagrees with the reference for ({lhs}, {rhs})"
			);
		}
	}
}

// ---------------------------------------------------------------------------
// Value-operator benchmarks
// ---------------------------------------------------------------------------

/// Generic over the closures so each arm is monomorphised and inlined. Passing
/// `fn` pointers would add an identical indirect call to all arms and mask the
/// differences being measured.
fn bench_value_arms<OpI64, OpI128, OpBig>(
	criterion: &mut Criterion,
	label: &str,
	operands: &Operands,
	op_i64: OpI64,
	op_i128: OpI128,
	op_big: OpBig,
) where
	OpI64: Fn(i64, i64) -> i64,
	OpI128: Fn(i128, i128) -> i128,
	OpBig: Fn(&BigFixed, &BigFixed) -> BigFixed,
{
	let mut group = criterion.benchmark_group(label);
	group.throughput(Throughput::Elements(PAIRS as u64));

	if let Some(pairs) = &operands.scaled_i64 {
		group.bench_function("i64", |bencher| {
			bencher.iter(|| {
				for &(lhs, rhs) in black_box(pairs.as_slice()) {
					black_box(op_i64(lhs, rhs));
				}
			})
		});
	}

	group.bench_function("i128", |bencher| {
		bencher.iter(|| {
			for &(lhs, rhs) in black_box(operands.scaled_i128.as_slice()) {
				black_box(op_i128(lhs, rhs));
			}
		})
	});

	group.bench_function("BigFixed", |bencher| {
		bencher.iter(|| {
			for (lhs, rhs) in black_box(operands.big_fixed.as_slice()) {
				black_box(op_big(lhs, rhs));
			}
		})
	});

	group.finish();
}

fn bench_value_op(criterion: &mut Criterion, suite: Suite, op: Op, band: Band) {
	let (band_name, ..) = band;
	let label = format!("{}/{}/{band_name}", suite.name(), op.name());
	let operands = Operands::build(suite, op, band);

	match op {
		Op::Add => bench_value_arms(
			criterion,
			&label,
			&operands,
			|lhs, rhs| lhs + rhs,
			|lhs, rhs| lhs + rhs,
			|lhs, rhs| lhs + rhs,
		),
		Op::Sub => bench_value_arms(
			criterion,
			&label,
			&operands,
			|lhs, rhs| lhs - rhs,
			|lhs, rhs| lhs - rhs,
			|lhs, rhs| lhs - rhs,
		),
		Op::Mul => bench_value_arms(
			criterion,
			&label,
			&operands,
			fixed_mul_i64,
			fixed_mul_i128,
			|lhs, rhs| lhs * rhs,
		),
		Op::Div => bench_value_arms(
			criterion,
			&label,
			&operands,
			fixed_div_i64,
			fixed_div_i128,
			|lhs, rhs| lhs / rhs,
		),
		Op::Rem => bench_value_arms(
			criterion,
			&label,
			&operands,
			|lhs, rhs| lhs % rhs,
			|lhs, rhs| lhs % rhs,
			|lhs, rhs| lhs % rhs,
		),
	}
}

// ---------------------------------------------------------------------------
// Assignment-operator benchmarks
// ---------------------------------------------------------------------------

/// One accumulator per operand pair, so each accumulator receives exactly one
/// operation per iteration and stays inside the same bounds the value suite
/// validated. `iter_batched` excludes setup from the measurement, then retains
/// the returned accumulator buffer as its output. Returning the buffer is
/// important: merely mutating a borrowed `Vec<i64>` or `Vec<i128>` and returning
/// `()` would allow those writes to be discarded as unobservable dead stores.
fn bench_assign_arms<OpI64, OpI128, OpBig>(
	criterion: &mut Criterion,
	label: &str,
	operands: &Operands,
	op_i64: OpI64,
	op_i128: OpI128,
	op_big: OpBig,
) where
	OpI64: Fn(&mut i64, i64),
	OpI128: Fn(&mut i128, i128),
	OpBig: Fn(&mut BigFixed, &BigFixed),
{
	let mut group = criterion.benchmark_group(label);
	group.throughput(Throughput::Elements(PAIRS as u64));

	if let Some(pairs) = &operands.scaled_i64 {
		let seeds: Vec<i64> = pairs.iter().map(|&(lhs, _)| lhs).collect();
		let divisors: Vec<i64> = pairs.iter().map(|&(_, rhs)| rhs).collect();
		group.bench_function("i64", |bencher| {
			bencher.iter_batched(
				|| seeds.clone(),
				|mut accumulators| {
					for (accumulator, &rhs) in
						accumulators.iter_mut().zip(black_box(divisors.as_slice()))
					{
						op_i64(accumulator, rhs);
					}
					accumulators
				},
				BatchSize::LargeInput,
			)
		});
	}

	let seeds: Vec<i128> = operands.scaled_i128.iter().map(|&(lhs, _)| lhs).collect();
	let divisors: Vec<i128> = operands.scaled_i128.iter().map(|&(_, rhs)| rhs).collect();
	group.bench_function("i128", |bencher| {
		bencher.iter_batched(
			|| seeds.clone(),
			|mut accumulators| {
				for (accumulator, &rhs) in
					accumulators.iter_mut().zip(black_box(divisors.as_slice()))
				{
					op_i128(accumulator, rhs);
				}
				accumulators
			},
			BatchSize::LargeInput,
		)
	});

	let big_seeds: Vec<BigFixed> =
		operands.big_fixed.iter().map(|(lhs, _)| lhs.clone()).collect();
	let big_divisors: Vec<BigFixed> =
		operands.big_fixed.iter().map(|(_, rhs)| rhs.clone()).collect();
	group.bench_function("BigFixed", |bencher| {
		bencher.iter_batched(
			|| big_seeds.clone(),
			|mut accumulators| {
				for (accumulator, rhs) in
					accumulators.iter_mut().zip(black_box(big_divisors.as_slice()))
				{
					op_big(accumulator, rhs);
				}
				accumulators
			},
			BatchSize::LargeInput,
		)
	});

	group.finish();
}

fn bench_assign_op(criterion: &mut Criterion, suite: Suite, op: Op) {
	// The `mid` band only: assignment is about the in-place path, not
	// magnitude scaling, which the value suites already sweep.
	let band = bands_for(suite, op)[1];
	let label = format!("assign/{}/{}", suite.name(), op.name());
	let operands = Operands::build(suite, op, band);

	match op {
		Op::Add => bench_assign_arms(
			criterion,
			&label,
			&operands,
			|acc, rhs| *acc += rhs,
			|acc, rhs| *acc += rhs,
			|acc, rhs| *acc += rhs,
		),
		Op::Sub => bench_assign_arms(
			criterion,
			&label,
			&operands,
			|acc, rhs| *acc -= rhs,
			|acc, rhs| *acc -= rhs,
			|acc, rhs| *acc -= rhs,
		),
		Op::Mul => bench_assign_arms(
			criterion,
			&label,
			&operands,
			|acc, rhs| *acc = fixed_mul_i64(*acc, rhs),
			|acc, rhs| *acc = fixed_mul_i128(*acc, rhs),
			|acc, rhs| *acc *= rhs,
		),
		Op::Div => bench_assign_arms(
			criterion,
			&label,
			&operands,
			|acc, rhs| *acc = fixed_div_i64(*acc, rhs),
			|acc, rhs| *acc = fixed_div_i128(*acc, rhs),
			|acc, rhs| *acc /= rhs,
		),
		Op::Rem => bench_assign_arms(
			criterion,
			&label,
			&operands,
			|acc, rhs| *acc %= rhs,
			|acc, rhs| *acc %= rhs,
			|acc, rhs| *acc %= rhs,
		),
	}
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn benchmark_config() -> Criterion {
	use std::time::Duration;
	Criterion::default()
		.warm_up_time(Duration::from_millis(300))
		.measurement_time(Duration::from_millis(500))
		.sample_size(100)
		.nresamples(10000)
}

fn benchmark_arithmetic(criterion: &mut Criterion) {
	for suite in [Suite::Native, Suite::Wide, Suite::Mixed] {
		for &op in ops_for(suite) {
			for band in bands_for(suite, op) {
				bench_value_op(criterion, suite, op, band);
			}
		}
	}

	for suite in [Suite::Native, Suite::Wide, Suite::Mixed] {
		for &op in ops_for(suite) {
			bench_assign_op(criterion, suite, op);
		}
	}
}

criterion_group! {
	name = benches;
	config = benchmark_config();
	targets = benchmark_arithmetic
}
criterion_main!(benches);
