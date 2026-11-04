use std::{borrow::Cow, cmp::Ordering};

use super::{ScratchInput, ScratchValueView};

// convert a scratch value to a number for arithmetic
#[inline]
fn to_number(value: ScratchValueView<'_>) -> f64 {
    let number = match value {
        ScratchValueView::Number(number) => number,
        ScratchValueView::Undefined => 0.0,

        ScratchValueView::Boolean(value) => {
            if value {
                1.0
            } else {
                0.0
            }
        }

        ScratchValueView::Text(text) => {
            let text = text.trim();

            if text.is_empty() {
                0.0
            } else {
                text.parse::<f64>().unwrap_or(0.0)
            }
        }
    };

    if number.is_nan() { 0.0 } else { number }
}

/// Convert a Scratch value to a number using Scratch's arithmetic rules.
#[inline]
pub fn scratch_to_number(value: impl ScratchInput) -> f64 {
    to_number(value.scratch_view())
}

// preserve scratch's nan-to-zero conversion for already numeric operands.
#[inline(always)]
pub fn scratch_number_operand(value: f64) -> f64 {
    if value.is_nan() { 0.0 } else { value }
}

/// convert a Scratch value to the text used by string menus and broadcasts.
pub fn scratch_string(value: impl ScratchInput) -> String {
    to_text(value.scratch_view()).into_owned()
}

fn scratch_trig_round(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    if !value.is_finite() || value.abs() >= 1.0e21 {
        return value;
    }

    const SCALE: f64 = 10_000_000_000.0;
    const SAFE_LIMIT: f64 = 450_359.0;
    if value.abs() >= SAFE_LIMIT {
        return format!("{value:.10}").parse().unwrap_or(value);
    }

    let scaled = value * SCALE;
    let scaled_abs = scaled.abs();
    let fraction = scaled_abs.fract();
    let tie_distance = (fraction - 0.5).abs();
    let multiplication_error_bound = 2.0 * f64::EPSILON * scaled_abs.max(1.0);
    if tie_distance <= multiplication_error_bound {
        return format!("{value:.10}").parse().unwrap_or(value);
    }

    scaled.round_ties_even() / SCALE
}

/// Scratch's sine reporter: degrees in, rounded to 10 decimal places.
pub fn scratch_sin(value: impl ScratchInput) -> f64 {
    let degrees = scratch_to_number(value);
    scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).sin())
}

/// Scratch's cosine reporter: degrees in, rounded to 10 decimal places.
pub fn scratch_cos(value: impl ScratchInput) -> f64 {
    let degrees = scratch_to_number(value);
    scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).cos())
}

/// Scratch's tangent reporter: degrees in, rounded to 10 decimal places,
/// with Scratch VM's exact singularity handling.
pub fn scratch_tan(value: impl ScratchInput) -> f64 {
    let degrees = scratch_to_number(value) % 360.0;
    match degrees {
        -270.0 | 90.0 => f64::INFINITY,
        -90.0 | 270.0 => f64::NEG_INFINITY,
        _ => scratch_trig_round(((std::f64::consts::PI * degrees) / 180.0).tan()),
    }
}

/// fast sine reporter: scratch conversion and degrees, without decimal rounding.
pub fn fast_sin(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).to_radians().sin()
}

/// fast cosine reporter: scratch conversion and degrees, without decimal rounding.
pub fn fast_cos(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).to_radians().cos()
}

/// fast tangent reporter without scratch's exact singularity handling.
pub fn fast_tan(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).to_radians().tan()
}

/// fast inverse sine in degrees using the platform's native operation.
pub fn fast_asin_degrees(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).asin().to_degrees()
}

/// fast inverse cosine in degrees using the platform's native operation.
pub fn fast_acos_degrees(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).acos().to_degrees()
}

/// fast inverse tangent in degrees using the platform's native operation.
pub fn fast_atan_degrees(value: impl ScratchInput) -> f64 {
    scratch_to_number(value).atan().to_degrees()
}

/// Scratch's inverse sine in degrees, using a stable atan2 form.
pub fn scratch_asin_degrees(value: impl ScratchInput) -> f64 {
    let number = scratch_to_number(value);
    let radians = scratch_asin_radians(number);
    radians * 180.0 / std::f64::consts::PI
}

/// Scratch's inverse cosine in degrees, using the half-angle identity.
pub fn scratch_acos_degrees(value: impl ScratchInput) -> f64 {
    let number = scratch_to_number(value);
    let radians = 2.0 * scratch_asin_radians(((1.0 - number) / 2.0).sqrt());
    radians * 180.0 / std::f64::consts::PI
}

/// Scratch's inverse tangent in degrees.
pub fn scratch_atan_degrees(value: impl ScratchInput) -> f64 {
    let radians = scratch_to_number(value).atan();
    radians * 180.0 / std::f64::consts::PI
}

fn scratch_asin_radians(number: f64) -> f64 {
    number.atan2((1.0 - number * number).sqrt())
}

// convert a scratch value to text
pub(super) fn to_text(value: ScratchValueView<'_>) -> Cow<'_, str> {
    match value {
        ScratchValueView::Number(number) => {
            if number.is_nan() {
                Cow::Borrowed("NaN")
            } else if number == f64::INFINITY {
                Cow::Borrowed("Infinity")
            } else if number == f64::NEG_INFINITY {
                Cow::Borrowed("-Infinity")
            } else if number == 0.0 {
                Cow::Borrowed("0")
            } else {
                Cow::Owned(number.to_string())
            }
        }

        ScratchValueView::Text(text) => Cow::Borrowed(text),

        ScratchValueView::Boolean(value) => Cow::Borrowed(if value { "true" } else { "false" }),

        ScratchValueView::Undefined => Cow::Borrowed("undefined"),
    }
}

// convert a scratch value to a boolean
pub fn scratch_bool(value: impl ScratchInput) -> bool {
    match value.scratch_view() {
        ScratchValueView::Boolean(value) => value,

        ScratchValueView::Undefined => false,

        ScratchValueView::Number(number) => number != 0.0 && !number.is_nan(),

        ScratchValueView::Text(text) => {
            !text.is_empty() && text != "0" && !text.eq_ignore_ascii_case("false")
        }
    }
}

#[inline]
pub fn scratch_add(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) + to_number(right.scratch_view())
}

#[inline]
pub fn scratch_subtract(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) - to_number(right.scratch_view())
}

#[inline]
pub fn scratch_multiply(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) * to_number(right.scratch_view())
}

#[inline]
pub fn scratch_divide(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) / to_number(right.scratch_view())
}

// try a numeric conversion for comparisons
//
// whitespace is treated as text here, rather than as zero
fn comparison_number(value: ScratchValueView<'_>) -> Option<f64> {
    match value {
        ScratchValueView::Number(number) => {
            if number.is_nan() {
                None
            } else {
                Some(number)
            }
        }

        ScratchValueView::Boolean(value) => Some(if value { 1.0 } else { 0.0 }),

        ScratchValueView::Undefined => None,

        ScratchValueView::Text(text) => {
            let text = text.trim();

            if text.is_empty() {
                return None;
            }

            text.parse::<f64>().ok().filter(|number| !number.is_nan())
        }
    }
}

// compare numerically when both values can be read as numbers;
// otherwise compare their lowercase text
pub fn scratch_compare(left: impl ScratchInput, right: impl ScratchInput) -> Ordering {
    let left = left.scratch_view();
    let right = right.scratch_view();

    if let (Some(left_number), Some(right_number)) =
        (comparison_number(left), comparison_number(right))
    {
        return left_number
            .partial_cmp(&right_number)
            .unwrap_or(Ordering::Equal);
    }

    let left_text = to_text(left).to_lowercase();
    let right_text = to_text(right).to_lowercase();

    // javascript compares strings by utf-16 code units
    left_text.encode_utf16().cmp(right_text.encode_utf16())
}

pub fn scratch_equals(left: impl ScratchInput, right: impl ScratchInput) -> bool {
    scratch_compare(left, right) == Ordering::Equal
}

pub fn scratch_greater_than(left: impl ScratchInput, right: impl ScratchInput) -> bool {
    scratch_compare(left, right) == Ordering::Greater
}

pub fn scratch_less_than(left: impl ScratchInput, right: impl ScratchInput) -> bool {
    scratch_compare(left, right) == Ordering::Less
}

pub fn scratch_and(left: impl ScratchInput, right: impl ScratchInput) -> bool {
    scratch_bool(left) && scratch_bool(right)
}

pub fn scratch_or(left: impl ScratchInput, right: impl ScratchInput) -> bool {
    scratch_bool(left) || scratch_bool(right)
}

pub fn scratch_not(value: impl ScratchInput) -> bool {
    !scratch_bool(value)
}

pub fn scratch_join(left: impl ScratchInput, right: impl ScratchInput) -> String {
    let left = to_text(left.scratch_view());
    let right = to_text(right.scratch_view());
    let mut result = String::with_capacity(left.len() + right.len());
    result.push_str(&left);
    result.push_str(&right);
    result
}

// scratch counts letters starting at 1
pub fn scratch_letter(index: impl ScratchInput, text: impl ScratchInput) -> String {
    let index = to_number(index.scratch_view());
    let text = to_text(text.scratch_view());

    if !index.is_finite() || index < 1.0 {
        return String::new();
    }

    let index = index.floor() as usize - 1;

    match text.encode_utf16().nth(index) {
        Some(code_unit) => String::from_utf16_lossy(&[code_unit]),
        None => String::new(),
    }
}

pub fn scratch_length(value: impl ScratchInput) -> f64 {
    let text = to_text(value.scratch_view());
    text.encode_utf16().count() as f64
}

pub fn scratch_contains(text: impl ScratchInput, search: impl ScratchInput) -> bool {
    let text = to_text(text.scratch_view()).to_lowercase();
    let search = to_text(search.scratch_view()).to_lowercase();

    text.contains(&search)
}

// scratch modulo follows the sign of the divisor
pub fn scratch_mod(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    let left = to_number(left.scratch_view());
    let right = to_number(right.scratch_view());

    let remainder = left % right;

    if remainder / right < 0.0 {
        remainder + right
    } else {
        remainder
    }
}

// halfway values round toward positive infinity
pub fn scratch_round(value: impl ScratchInput) -> f64 {
    let number = to_number(value.scratch_view());

    if !number.is_finite() || number == 0.0 {
        return number;
    }

    let lower = number.floor();

    let result = if number - lower >= 0.5 {
        lower + 1.0
    } else {
        lower
    };

    if result == 0.0 && number.is_sign_negative() {
        -0.0
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ScratchValue;

    #[test]
    fn compares_numbers_and_text() {
        assert!(scratch_equals("5", 5.0));
        assert!(scratch_equals("APPLE", "apple"));
        assert!(scratch_greater_than("banana", 0.0));
        assert!(!scratch_equals("", 0.0));
    }

    #[test]
    fn handles_text_operators() {
        assert_eq!(scratch_join("apple ", "banana"), "apple banana");
        assert_eq!(scratch_letter(1.0, "apple"), "a");
        assert_eq!(scratch_letter(10.0, "apple"), "");
        assert_eq!(scratch_length("Scratch"), 7.0);
        assert!(scratch_contains("Apple", "a"));
        assert!(scratch_contains(1234.0, 34.0));
    }

    #[test]
    fn handles_modulo_and_rounding() {
        assert_eq!(scratch_mod(4.0, 5.0), 4.0);
        assert_eq!(scratch_mod(-4.0, 5.0), 1.0);
        assert_eq!(scratch_round(1.5), 2.0);
        assert_eq!(scratch_round(-1.5), -1.0);
    }

    #[test]
    fn handles_boolean_conversion() {
        assert!(!scratch_bool("false"));
        assert!(!scratch_bool("0"));
        assert!(scratch_bool("hello"));
        assert!(scratch_not(false));
    }

    #[test]
    fn converts_scratch_values_to_numbers() {
        assert_eq!(scratch_to_number(5.5), 5.5);
        assert_eq!(scratch_to_number(" 5.5 "), 5.5);
        assert_eq!(scratch_to_number(true), 1.0);
        assert_eq!(scratch_to_number("not a number"), 0.0);
        assert_eq!(scratch_to_number(f64::NAN), 0.0);
        assert_eq!(scratch_add(f64::NAN, 1.0), 1.0);
    }

    #[test]
    fn preserves_scratch_undefined_results_from_legacy_reporters() {
        let value = ScratchValue::Undefined;
        assert_eq!(scratch_to_number(&value), 0.0);
        assert_eq!(scratch_string(&value), "undefined");
        assert!(!scratch_bool(&value));
        assert!(scratch_equals(&value, "undefined"));
    }

    #[test]
    fn operators_borrow_dynamic_values_across_type_changes() {
        let mut value = ScratchValue::Number(2.0);
        assert_eq!(scratch_add(&value, 3.0), 5.0);

        value = ScratchValue::Text(" 4.5 ".to_owned());
        assert_eq!(scratch_multiply(&value, 2.0), 9.0);
        assert_eq!(scratch_join(&value, "!"), " 4.5 !");

        value = ScratchValue::Boolean(true);
        assert_eq!(scratch_add(&value, 2.0), 3.0);
        assert_eq!(scratch_join(&value, "!"), "true!");
    }

    #[test]
    fn loop_style_conversions_keep_text_and_mutated_variable_values_dynamic() {
        let mut value = ScratchValue::Text("3".to_owned());
        let mut iterations = 0;
        while scratch_to_number(&value) > 0.0 {
            iterations += 1;
            value = ScratchValue::Number(scratch_to_number(&value) - 1.0);
        }
        assert_eq!(iterations, 3);
        assert_eq!(value, ScratchValue::Number(0.0));

        value = ScratchValue::Text("false".to_owned());
        assert!(!scratch_bool(&value));
        value = ScratchValue::Boolean(true);
        assert!(scratch_bool(&value));
    }

    #[test]
    fn matches_scratch_trigonometry_precision_and_tangent_singularities() {
        assert_eq!(scratch_sin(30.0), 0.5);
        assert_eq!(scratch_cos(60.0), 0.5);
        assert_eq!(scratch_sin("30"), 0.5);
        assert_eq!(scratch_cos("60"), 0.5);
        assert_eq!(scratch_tan(45.0), 1.0);
        assert_eq!(scratch_tan(90.0), f64::INFINITY);
        assert_eq!(scratch_tan(-270.0), f64::INFINITY);
        assert_eq!(scratch_tan(-90.0), f64::NEG_INFINITY);
        assert_eq!(scratch_tan(270.0), f64::NEG_INFINITY);
    }

    #[test]
    fn fast_trigonometry_keeps_conversion_but_skips_scratch_edge_adjustments() {
        assert_eq!(fast_sin("30"), 30.0_f64.to_radians().sin());
        assert_eq!(fast_cos(true), 1.0_f64.to_radians().cos());
        assert_eq!(fast_tan(90.0), 90.0_f64.to_radians().tan());
        assert!(fast_tan(90.0).is_finite());
        assert_ne!(fast_sin(30.0), scratch_sin(30.0));
        assert_eq!(fast_asin_degrees(0.5), 0.5_f64.asin().to_degrees());
        assert_eq!(fast_acos_degrees(0.5), 0.5_f64.acos().to_degrees());
        assert_eq!(fast_atan_degrees(1.0), 45.0);
        assert!(fast_sin(f64::INFINITY).is_nan());
        assert_eq!(fast_sin(f64::NAN), 0.0);
        assert_eq!(fast_sin(-0.0).to_bits(), (-0.0_f64).to_bits());
    }

    #[test]
    fn native_fast_arithmetic_keeps_ieee_nan_while_default_normalizes_operands() {
        let default = scratch_number_operand(f64::NAN) + scratch_number_operand(1.0);
        let fast = f64::NAN + 1.0;
        assert_eq!(default, 1.0);
        assert!(fast.is_nan());
        assert_eq!(scratch_add("2.5", true), 3.5);
        assert_eq!(scratch_add(f64::INFINITY, false), f64::INFINITY);
    }

    #[test]
    #[ignore = "manual release-mode dynamic value benchmark"]
    fn benchmark_dynamic_numeric_conversion() {
        use std::{hint::black_box, time::Instant};

        const ITERATIONS: usize = 10_000_000;
        const SAMPLES: usize = 5;
        let values = [
            ScratchValue::Number(2.5),
            ScratchValue::Text("2.5".to_owned()),
            ScratchValue::Boolean(true),
            ScratchValue::Text("not a number".to_owned()),
        ];
        let mut elapsed = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let mut sum = 0.0;
            for index in 0..ITERATIONS {
                sum += scratch_add(black_box(&values[index % values.len()]), 1.0);
            }
            black_box(sum);
            elapsed.push(started.elapsed());
        }
        elapsed.sort_unstable();
        println!(
            "dynamic numeric conversion: iterations={ITERATIONS}, samples={SAMPLES}, median={:?}",
            elapsed[SAMPLES / 2]
        );
    }

    #[test]
    fn fast_trig_rounding_matches_the_decimal_reference() {
        let reference = |value: f64| {
            if value == 0.0 || !value.is_finite() || value.abs() >= 1.0e21 {
                value
            } else {
                format!("{value:.10}").parse().unwrap_or(value)
            }
        };

        for value in [
            -1.0,
            -0.999_999_999_95,
            -0.500_000_000_05,
            -0.000_000_000_05,
            0.000_000_000_05,
            0.123_456_789_05,
            0.500_000_000_05,
            0.999_999_999_95,
            1.0,
            89_999.999_999_999_9,
            450_358.999_999_999_94,
            450_359.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ] {
            let actual = scratch_trig_round(value);
            let expected = reference(value);
            assert!(
                actual == expected || (actual.is_nan() && expected.is_nan()),
                "rounding mismatch for {value:?}: {actual:?} != {expected:?}"
            );
        }

        let mut seed = 0x9e37_79b9_u64;
        for _ in 0..100_000 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let value = (seed as i64 as f64 / i64::MAX as f64) * 450_358.0;
            assert_eq!(scratch_trig_round(value), reference(value));
        }
    }
}
