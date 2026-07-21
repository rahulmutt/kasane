/// PalmDoc LZ77 decompression. Opcodes: 0x00 literal NUL; 0x01..=0x08 copy N
/// literal bytes; 0x09..=0x7F literal; 0x80..=0xBF two-byte back-reference
/// (11-bit distance, 3-bit length-3); 0xC0..=0xFF space + (byte ^ 0x80).
/// Malformed input (short pair, bad distance) truncates the output at that
/// point rather than panicking: degrade, don't die.
pub(crate) fn decompress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        i += 1;
        match c {
            0x00 => out.push(0),
            0x01..=0x08 => {
                let n = (c as usize).min(data.len() - i);
                out.extend_from_slice(&data[i..i + n]);
                i += n;
            }
            0x09..=0x7F => out.push(c),
            0x80..=0xBF => {
                let Some(&lo) = data.get(i) else { break };
                i += 1;
                let pair = ((c as usize & 0x3F) << 8) | lo as usize;
                let distance = pair >> 3;
                let length = (pair & 0x07) + 3;
                if distance == 0 || distance > out.len() {
                    break;
                }
                for _ in 0..length {
                    let b = out[out.len() - distance];
                    out.push(b);
                }
            }
            0xC0..=0xFF => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
    out
}

// WHATWG windows-1252: 0x80..=0x9F remapped; everything else is the
// identical Unicode codepoint.
const CP1252_HIGH: [u32; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039,
    0x0152, 0x008D, 0x017D, 0x008F, 0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014,
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

pub(crate) fn cp1252_to_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => char::from_u32(CP1252_HIGH[(b - 0x80) as usize]).unwrap_or('\u{FFFD}'),
            _ => b as char,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_plain_ascii_through() {
        assert_eq!(decompress(b"hello"), b"hello");
    }

    #[test]
    fn literal_run_prefix() {
        // 0x01..=0x08: copy that many bytes verbatim (used to escape bytes
        // that would otherwise be opcodes).
        assert_eq!(decompress(&[0x02, 0x80, 0xC1, b'x']), &[0x80, 0xC1, b'x']);
    }

    #[test]
    fn lz77_back_reference() {
        // "abc" + pair(distance=3, length=6) -> "abcabcabc".
        // pair = (3 << 3) | (6 - 3) = 27 -> bytes 0x80, 0x1B
        assert_eq!(decompress(&[b'a', b'b', b'c', 0x80, 0x1B]), b"abcabcabc");
    }

    #[test]
    fn space_char_compression() {
        // 0xC0..=0xFF: a space followed by (byte ^ 0x80)
        assert_eq!(decompress(&[b'a', 0xE2]), b"a b");
    }

    #[test]
    fn malformed_backref_truncates_instead_of_panicking() {
        // distance 100 with only 1 byte of output so far: stop cleanly.
        #[allow(clippy::identity_op)]
        let pair = 100u16 << 3 | 0; // length 3
        let hi = 0x80 | ((pair >> 8) as u8 & 0x3F);
        let out = decompress(&[b'a', hi, pair as u8]);
        assert_eq!(out, b"a");
    }

    #[test]
    fn cp1252_maps_high_range() {
        assert_eq!(
            cp1252_to_string(b"caf\xE9 \x93q\x94"),
            "caf\u{e9} \u{201c}q\u{201d}"
        );
    }
}
