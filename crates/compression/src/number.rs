//! JavaScript's number rounding and formatting, which every count and
//! percentage the kernel shows the model goes through: `Math.round` rounds
//! halves up, and `toFixed` rounds an exact tie away from zero where Rust's
//! formatting rounds it to even.

/// `Math.round`.
pub(crate) fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// `value.toFixed(digits)`.
pub(crate) fn to_fixed(value: f64, digits: usize) -> String {
    // Enough digits to hold any f64 exactly, so the tie is seen as a tie.
    let exact = format!("{:.1100}", value.abs());
    let (whole, fraction) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut kept: Vec<u8> = whole.bytes().chain(fraction.bytes().take(digits)).collect();
    let rest = &fraction.as_bytes()[digits.min(fraction.len())..];
    if rest.first().is_some_and(|digit| *digit >= b'5') {
        round_up(&mut kept);
    }
    let point = kept.len() - digits;
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&kept[..point]).unwrap_or("0"));
    if digits > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&kept[point..]).unwrap_or(""));
    }
    out
}

fn round_up(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            return;
        }
    }
    digits.insert(0, b'1');
}

/// A count of tokens as `12.3K` from a thousand on, as the nudge, the
/// status line and the status report show it.
pub(crate) fn format_k(tokens: f64) -> String {
    if tokens >= 1000.0 {
        format!("{}K", to_fixed(tokens / 1000.0, 1))
    } else {
        format_number(tokens)
    }
}

/// A number as JavaScript's `String(n)` writes it, for the magnitudes the
/// texts show.
pub(crate) fn format_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e21 {
        format!("{}", value as i128)
    } else {
        format!("{value}")
    }
}
