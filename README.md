# bigfixed

`bigfixed` is a safe (`#![forbid(unsafe_code)]`) Rust crate providing `BigFixed`, an arbitrary-range fixed-point decimal type with a fixed scale of 6 fractional digits (`SCALE = 6`, scale factor `1_000_000`).

## Design

`BigFixed` is designed around an allocation-free native fast path with seamless fallback to arbitrary-precision integers (`num_bigint::BigInt`) when values exceed 64-bit bounds:

- **Three-variant internal representation** (16 bytes on 64-bit targets):
  - **Unscaled whole integers (`Whole(i64)`)**: Whole numbers in `i64::MIN..=i64::MAX` are stored directly without scaling, preserving the full `i64` range and keeping all-integer arithmetic on fast checked `i64` operations.
  - **Scaled native decimals (`Scaled(i64)`)**: Fractional values (and scaled results) whose $10^6$-scaled coefficient fits in `i64` are stored inline without heap allocation, using `i128` intermediates during multiplication, division, and mixed whole/scaled arithmetic.
  - **Arbitrary-precision fallback (`Large(Box<BigInt>)`)**: Values whose scaled coefficient exceeds `i64` automatically promote to a boxed `BigInt` and demote back to `Scaled(i64)` whenever a subsequent result fits in `i64` again.
- **Lazy normalization**: Scaled coefficient arithmetic does not eagerly divide by `1_000_000` to test whether a result became a whole number. Instead, equality (`Eq`), ordering (`Ord`), hashing (`Hash`), formatting (`Display`), and integer queries (`is_integer`, `as_i64_integer`) reconcile representations lazily.
- **Deterministic rounding**: Decimal parsing, multiplication, and division (`/` and `checked_div`) round to 6 fractional digits using **round half away from zero** (nearest, ties away from zero). Remainder (`%` and `checked_rem`) uses truncating division rules (`a - trunc(a / b) * b`).

## Usage

The package is published as `pka-bigfixed`, but the library is imported as `bigfixed`:

```toml
[dependencies]
pka-bigfixed = "0.1"
```

```rust
use bigfixed::{BigFixed, Error};

fn main() -> bigfixed::Result<()> {
    // Construct from integers or parse from decimal strings
    let whole = BigFixed::from(42_i64);
    let price: BigFixed = "19.95".parse()?;

    // Standard arithmetic operators (by value or reference)
    let total = &whole * &price;
    assert_eq!(total.to_string(), "837.9");
    assert!(total.is_native());

    // Division rounds to 6 fractional digits (nearest, ties away from zero)
    let third = BigFixed::ONE.checked_div(&BigFixed::from(3_i64))?;
    assert_eq!(third.to_string(), "0.333333");

    // Checked division by zero returns an Error
    assert_eq!(
        BigFixed::ONE.checked_div(&BigFixed::ZERO),
        Err(Error::DivisionByZero)
    );

    // Seamless promotion beyond i64 and demotion back to native
    let max_whole = BigFixed::from(i64::MAX);
    let promoted = &max_whole + BigFixed::ONE;
    assert!(!promoted.is_native());

    let demoted = promoted - BigFixed::ONE;
    assert_eq!(demoted.as_i64_integer(), Some(i64::MAX));

    Ok(())
}
```

## Development

Common development workflows are automated via `cargo xtask`:

```sh
cargo xtask fmt     # Format all code
cargo xtask lint    # Run clippy with warnings denied
cargo xtask test    # Run all tests
cargo xtask bench   # Run Criterion benchmarks
cargo xtask ci      # Check formatting, lint, and run tests
```

## License

BSD 3-Clause. See [LICENSE](LICENSE.md) for details.
