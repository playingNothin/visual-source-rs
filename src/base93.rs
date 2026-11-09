pub const ALPHABET: &[u8; 93] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!#$%&'()*+,-./:;<=>?@[]^_`{|}~ ";

#[derive(Debug, PartialEq, Eq)]
pub enum Base93Error {
    InvalidCharacter(char, usize),
}

impl std::fmt::Display for Base93Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCharacter(c, pos) => {
                write!(f, "Invalid Base93 character {:?} at position {}", c, pos)
            }
        }
    }
}

impl std::error::Error for Base93Error {}

fn char_to_val(b: u8, pos: usize) -> Result<u32, Base93Error> {
    for (i, &ch) in ALPHABET.iter().enumerate() {
        if ch == b {
            return Ok(i as u32);
        }
    }
    Err(Base93Error::InvalidCharacter(b as char, pos))
}

/// Decodes a Base93 string to bytes.
pub fn decode(s: &str) -> Result<Vec<u8>, Base93Error> {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let even = (n / 2) * 2;

    let mut out = Vec::with_capacity(n);
    let mut bitbuf: u32 = 0;
    let mut bitcnt: u32 = 0;

    let mut i = 0;
    while i < even {
        let v1 = char_to_val(bytes[i], i)?;
        let v2 = char_to_val(bytes[i + 1], i + 1)?;
        let v = v1 + v2 * 93;

        bitbuf += v << bitcnt;
        bitcnt += if (v % 8192) > 456 { 13 } else { 14 };

        while bitcnt > 7 {
            out.push((bitbuf & 0xFF) as u8);
            bitbuf >>= 8;
            bitcnt -= 8;
        }

        i += 2;
    }

    if even < n {
        let c = char_to_val(bytes[n - 1], n - 1)?;
        out.push(((bitbuf + (c << bitcnt)) & 0xFF) as u8);
    }

    Ok(out)
}

/// Encodes raw bytes to a Base93 string.
pub fn encode(data: &[u8]) -> String {
    let mut b: u32 = 0;
    let mut n: u32 = 0;
    let mut out = Vec::with_capacity(data.len() * 12 / 10 + 4);

    for &byte in data {
        b |= (byte as u32) << n;
        n += 8;
        if n > 13 {
            let v13 = b & 8191;
            let v = if v13 > 456 {
                b >>= 13;
                n -= 13;
                v13
            } else {
                let v14 = b & 16383;
                b >>= 14;
                n -= 14;
                v14
            };
            out.push(ALPHABET[(v % 93) as usize]);
            out.push(ALPHABET[(v / 93) as usize]);
        }
    }

    if n > 0 {
        out.push(ALPHABET[(b % 93) as usize]);
        if n > 7 || b > 92 {
            out.push(ALPHABET[(b / 93) as usize]);
        }
    }

    // Safety: ALPHABET is purely ASCII
    unsafe { String::from_utf8_unchecked(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_vectors() {
        assert_eq!(decode("%A").unwrap(), b"A");
        assert_eq!(encode(b"A"), "%A");

        assert_eq!(decode("wFA").unwrap(), b"\x01\x02");
        assert_eq!(encode(b"\x01\x02"), "wFA");
    }

    #[test]
    fn test_roundtrips() {
        let test_cases: Vec<&[u8]> = vec![
            b"",
            b"Hello World!",
            b"\x00\x00\x00\x00",
            b"\xFF\xFE\xFD\xFC\xFB",
            b"VisualSource V4 RetroStudio",
        ];

        for tc in test_cases {
            let enc = encode(tc);
            let dec = decode(&enc).unwrap();
            assert_eq!(&dec[..], tc);
        }

        // Pseudo-random roundtrips
        for len in 1..256 {
            let pattern: Vec<u8> = (0..len).map(|i| ((i * 37 + 13) % 256) as u8).collect();
            let enc = encode(&pattern);
            let dec = decode(&enc).unwrap();
            assert_eq!(dec, pattern, "failed at length {}", len);
        }
    }
}
