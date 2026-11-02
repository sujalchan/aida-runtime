use std::{borrow::Cow, cmp::Ordering};

use super::{ScratchInput, ScratchValueView};

// convert a scratch value to a number for arithmetic
fn to_number(value: ScratchValueView<'_>) -> f64 {
    let number = match value {
        ScratchValueView::Number(number) => number,

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
pub fn scratch_to_number(value: impl ScratchInput) -> f64 {
    to_number(value.scratch_view())
}

fn scratch_trig_round(value: f64) -> f64 {
    if value == 0.0 {
        return 0.0;
    }
    if !value.is_finite() || value.abs() >= 1.0e21 {
        return value;
    }
    format!("{value:.10}").parse().unwrap_or(value)
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
    }
}

// convert a scratch value to a boolean
pub fn scratch_bool(value: impl ScratchInput) -> bool {
    match value.scratch_view() {
        ScratchValueView::Boolean(value) => value,

        ScratchValueView::Number(number) => number != 0.0 && !number.is_nan(),

        ScratchValueView::Text(text) => {
            !text.is_empty() && text != "0" && !text.eq_ignore_ascii_case("false")
        }
    }
}

pub fn scratch_add(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) + to_number(right.scratch_view())
}

pub fn scratch_subtract(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) - to_number(right.scratch_view())
}

pub fn scratch_multiply(left: impl ScratchInput, right: impl ScratchInput) -> f64 {
    to_number(left.scratch_view()) * to_number(right.scratch_view())
}

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
        assert_eq!(scratch_tan(45.0), 1.0);
        assert_eq!(scratch_tan(90.0), f64::INFINITY);
        assert_eq!(scratch_tan(-270.0), f64::INFINITY);
        assert_eq!(scratch_tan(-90.0), f64::NEG_INFINITY);
        assert_eq!(scratch_tan(270.0), f64::NEG_INFINITY);
    }
}
