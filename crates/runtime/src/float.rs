//! Printing an `f64`: the shortest text that reads back as the same value.

use std::io::Write;

/// The longest text of an `f64`: `-2.2250738585072014e-308`.
const FLOAT_BYTES: usize = 24;

/// `x` in the shortest text that reads back as `x`, in `buffer`: Rust's `{:?}`, which spells
/// NaN `NaN`, with `nan` for it.
pub(crate) fn shortest(x: f64, buffer: &mut [u8; FLOAT_BYTES]) -> &[u8] {
    if x.is_nan() {
        return b"nan";
    }
    let mut rest = &mut buffer[..];
    write!(rest, "{x:?}").expect("24 bytes should hold every `f64`");
    let len = FLOAT_BYTES - rest.len();
    &buffer[..len]
}

/// Writes `x` as `print` does to `out`, and returns how many bytes it wrote: at most 24.
/// Interpolation calls this for an `f64`.
///
/// # Safety
///
/// `out` has room for the text of `x`, at most 24 bytes.
#[expect(
    unsafe_code,
    reason = "generated code calls this function by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sisu_write_f64(x: f64, out: *mut u8) -> usize {
    // SAFETY: the caller promises room for the text, at most the 24 bytes `shortest` gives.
    unsafe { crate::text::write(shortest(x, &mut [0; FLOAT_BYTES]), out) }
}

#[cfg(test)]
mod tests {
    use super::{FLOAT_BYTES, shortest, sisu_write_f64};

    /// Interpolation writes the text `print` does; the longest fills the 24 bytes it is given.
    #[test]
    fn write_f64_fills_its_bound() {
        for (x, want) in [
            (-2.225_073_858_507_201_4e-308, "-2.2250738585072014e-308"),
            (1.0, "1.0"),
            (f64::NAN, "nan"),
        ] {
            let mut out = [0; FLOAT_BYTES];
            // SAFETY: `out` has room for 24 bytes.
            #[expect(unsafe_code, reason = "calls the runtime's C ABI function")]
            let len = unsafe { sisu_write_f64(x, out.as_mut_ptr()) };
            assert_eq!(&out[..len], want.as_bytes());
        }
    }

    #[test]
    fn shortest_matches_the_spec() {
        let rows = [
            (1.0, "1.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e15, "1000000000000000.0"),
            (1e16, "1e16"),
            (0.0001, "0.0001"),
            (1e-5, "1e-5"),
            (-0.0, "-0.0"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (f64::MAX, "1.7976931348623157e308"),
            (5e-324, "5e-324"),
            (f64::NAN, "nan"),
            (-f64::NAN, "nan"),
            // The longest output.
            (-2.225_073_858_507_201_4e-308, "-2.2250738585072014e-308"),
        ];
        for (x, text) in rows {
            assert_eq!(shortest(x, &mut [0; FLOAT_BYTES]), text.as_bytes(), "{x}");
        }
        assert_eq!("-2.2250738585072014e-308".len(), FLOAT_BYTES);
    }

    #[test]
    fn shortest_reads_back() {
        let mut bits = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..100_000 {
            // A xorshift step gives bit patterns across every exponent and sign.
            bits ^= bits << 13;
            bits ^= bits >> 7;
            bits ^= bits << 17;
            let x = f64::from_bits(bits);
            if !x.is_finite() {
                continue;
            }
            let mut buffer = [0; FLOAT_BYTES];
            let text =
                std::str::from_utf8(shortest(x, &mut buffer)).expect("the text should be ASCII");
            let back: f64 = text.parse().expect("the text should parse as an `f64`");
            assert_eq!(back.to_bits(), x.to_bits(), "{text}");
        }
    }
}
