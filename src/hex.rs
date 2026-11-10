use std::{fmt::Display, ops::{Deref, DerefMut}};

#[derive(Clone, Copy, Debug, PartialEq)]
/// Tuple struct type that manages fractional hexadecimal numbers according to RetroStudio V4 rules:
/// - Whole part is hexadecimal.
/// - Fractional part is hexadecimal representing integer thousandths (value / 1000.0).
pub struct Hex(pub f64);

impl Hex {
    /// Gets a hexadecimal number in a string, returning a Hex.
    ///
    /// Examples:
    /// - "3c" -> 60.0
    /// - "C2" -> 194.0
    /// - "3E9" -> 1001.0
    /// - "a.1f" -> 10.031 (0x1F = 31 / 1000)
    /// - "0.64" -> 0.1 (0x64 = 100 / 1000)
    /// - "0.1F4" -> 0.5 (0x1F4 = 500 / 1000)
    /// - "0.12C" -> 0.3 (0x12C = 300 / 1000)
    /// - "0.2BC" -> 0.7 (0x2BC = 700 / 1000)
    pub fn from_hex(src: &str) -> Option<Self> {
        let s = src.trim();
        if s.is_empty() {
            return None;
        }

        let (neg, s) = if let Some(stripped) = s.strip_prefix('-') {
            (true, stripped)
        } else {
            (false, s)
        };

        if s.is_empty() {
            return None;
        }

        let val = if let Some((int_str, frac_str)) = s.split_once('.') {
            let int_val = if int_str.is_empty() {
                0.0
            } else {
                i64::from_str_radix(int_str, 16).ok()? as f64
            };

            let frac_val = if frac_str.is_empty() {
                0.0
            } else {
                let frac_int = i64::from_str_radix(frac_str, 16).ok()? as f64;
                frac_int / 1000.0
            };

            int_val + frac_val
        } else {
            i64::from_str_radix(s, 16).ok()? as f64
        };

        let res = if neg { -val } else { val };
        Some(Self(res))
    }

    /// Formats an f64 to RetroStudio hexadecimal string representation.
    pub fn to_hex_string(val: f64) -> String {
        let neg = val < 0.0;
        let abs_val = val.abs();
        let total_thousandths = (abs_val * 1000.0).round() as u64;
        let whole = total_thousandths / 1000;
        let frac = total_thousandths % 1000;

        let sign = if neg && total_thousandths > 0 { "-" } else { "" };
        if frac == 0 {
            format!("{}{:X}", sign, whole)
        } else {
            format!("{}{:X}.{:X}", sign, whole, frac)
        }
    }

    /// Formats a raw hex string for display in decoded reports matching formatNumberOut / python.py.
    pub fn format_num_display(s: Option<&str>) -> String {
        match s {
            None => "0".to_string(),
            Some(s) => {
                if let Some(h) = Self::from_hex(s) {
                    let v = h.0;
                    if v == (v as i64 as f64) {
                        format!("{}", v as i64)
                    } else {
                        let s = format!("{:.3}", v);
                        s.trim_end_matches('0').trim_end_matches('.').to_string()
                    }
                } else {
                    "0".to_string()
                }
            }
        }
    }
}

impl Into<isize> for Hex {
    fn into(self) -> isize {
        self.0 as isize
    }
}

impl<T> From<T> for Hex
where
    T: Into<f64>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl Deref for Hex {
    type Target = f64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Hex {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for Hex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", Self::to_hex_string(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_hex() {
        assert_eq!(Hex::from_hex("3c").unwrap().0, 60.0);
        assert_eq!(Hex::from_hex("C2").unwrap().0, 194.0);
        assert_eq!(Hex::from_hex("3E9").unwrap().0, 1001.0);
    }

    #[test]
    fn test_fractional_hex_thousandths() {
        // "a.1f" = 10 + 31/1000 = 10.031
        let a_1f = Hex::from_hex("a.1f").unwrap().0;
        assert!((a_1f - 10.031).abs() < 1e-6);

        // "0.64" = 0 + 100/1000 = 0.1
        let zero_64 = Hex::from_hex("0.64").unwrap().0;
        assert!((zero_64 - 0.1).abs() < 1e-6);

        // "0.1F4" = 0 + 500/1000 = 0.5
        let zero_1f4 = Hex::from_hex("0.1F4").unwrap().0;
        assert!((zero_1f4 - 0.5).abs() < 1e-6);

        // "0.12C" = 0 + 300/1000 = 0.3
        let zero_12c = Hex::from_hex("0.12C").unwrap().0;
        assert!((zero_12c - 0.3).abs() < 1e-6);

        // "0.2BC" = 0 + 700/1000 = 0.7
        let zero_2bc = Hex::from_hex("0.2BC").unwrap().0;
        assert!((zero_2bc - 0.7).abs() < 1e-6);
    }

    #[test]
    fn test_negative_hex() {
        let neg_a_1f = Hex::from_hex("-a.1f").unwrap().0;
        assert!((neg_a_1f - (-10.031)).abs() < 1e-6);
        assert_eq!(Hex::from_hex("-3c").unwrap().0, -60.0);
    }

    #[test]
    fn test_roundtrip_formatting() {
        assert_eq!(Hex::to_hex_string(60.0), "3C");
        assert_eq!(Hex::to_hex_string(194.0), "C2");
        assert_eq!(Hex::to_hex_string(1001.0), "3E9");
        assert_eq!(Hex::to_hex_string(10.031), "A.1F");
        assert_eq!(Hex::to_hex_string(0.1), "0.64");
        assert_eq!(Hex::to_hex_string(0.5), "0.1F4");
        assert_eq!(Hex::to_hex_string(0.3), "0.12C");
        assert_eq!(Hex::to_hex_string(0.7), "0.2BC");
        assert_eq!(Hex::to_hex_string(-10.031), "-A.1F");
    }

    #[test]
    fn test_format_num_display() {
        assert_eq!(Hex::format_num_display(Some("3c")), "60");
        assert_eq!(Hex::format_num_display(Some("C2")), "194");
        assert_eq!(Hex::format_num_display(Some("a.1f")), "10.031");
        assert_eq!(Hex::format_num_display(Some("0.1F4")), "0.5");
        assert_eq!(Hex::format_num_display(None), "0");
    }
}