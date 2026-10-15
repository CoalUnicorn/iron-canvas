pub fn xml_escape(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

/// Escape text for a PDF literal string using the WinAnsi character encoding.
/// Characters without a WinAnsi representation are replaced with `?`.
pub fn pdf_string_escape(s: &str, out: &mut Vec<u8>) {
    for ch in s.chars() {
        let b = win_ansi_byte(ch).unwrap_or(b'?');
        match b {
            b'(' => out.extend_from_slice(b"\\("),
            b')' => out.extend_from_slice(b"\\)"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b if !(0x20..=0x7E).contains(&b) => {
                out.extend_from_slice(format!("\\{b:03o}").as_bytes());
            }
            b => out.push(b),
        }
    }
}

fn win_ansi_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    if code <= 0x7f || (0xa0..=0xff).contains(&code) {
        return Some(code as u8);
    }
    Some(match ch {
        '\u{20AC}' => 0x80,
        '\u{201A}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201E}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02C6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8a,
        '\u{2039}' => 0x8b,
        '\u{0152}' => 0x8c,
        '\u{017D}' => 0x8e,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201C}' => 0x93,
        '\u{201D}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02DC}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9a,
        '\u{203A}' => 0x9b,
        '\u{0153}' => 0x9c,
        '\u{017E}' => 0x9e,
        '\u{0178}' => 0x9f,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::pdf_string_escape;

    #[test]
    fn pdf_string_escape_encodes_accented_currency_and_punctuation_as_win_ansi() {
        let mut escaped = Vec::new();
        pdf_string_escape("café £ € ’", &mut escaped);

        assert_eq!(escaped, b"caf\\351 \\243 \\200 \\222");
    }

    #[test]
    fn pdf_string_escape_replaces_characters_outside_win_ansi() {
        let mut escaped = Vec::new();
        pdf_string_escape("漢", &mut escaped);

        assert_eq!(escaped, b"?");
    }
}
