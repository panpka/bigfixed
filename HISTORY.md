# HISTORY

## [Unreleased]

## [0.1.0] - 2026-11-08

Initial release.

### Added

- `BigFixed`, an arbitrary-range fixed-point decimal type with 6 fractional
  digits (`SCALE`), stored in two machine words.
- Allocation-free native representations for whole and scaled values that fit
  in `i64`, with automatic promotion to `BigInt` and demotion back.
- Arithmetic operators (`+`, `-`, `*`, `/`, `%`, `-x` and their assign forms)
  for owned and borrowed operands; `checked_div` and `checked_rem` returning
  `Error::DivisionByZero`.
- Round-half-away-from-zero rounding for parsing, multiplication and division;
  truncating remainder.
- Decimal parsing via `FromStr` / `TryFrom<&str>`, and `Display`, `Eq`, `Ord`,
  `Hash` consistent across representations.
- Conversions from `i64` and `BigInt`, and scaled-coefficient constructors and
  accessors (`from_scaled*`, `as_scaled_i64`, `to_scaled_bigint`, ...).
