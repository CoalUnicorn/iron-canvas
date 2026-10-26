/// Bold and italic flags from the prefix of a CSS font shorthand.
///
/// The workbook compiler writes these tokens before the size, for example
/// `bold italic 12px Calibri`. Family names do not affect the style result.
pub fn bold_italic(font_css: &str) -> (bool, bool) {
    let prefix = font_css
        .split_once("px")
        .map_or(font_css, |(prefix, _)| prefix);
    let mut bold = false;
    let mut italic = false;
    for token in prefix.split_whitespace() {
        match token {
            "bold" => bold = true,
            "italic" => italic = true,
            _ => {}
        }
    }
    (bold, italic)
}

#[cfg(test)]
mod tests {
    use super::bold_italic;

    #[test]
    fn parses_bold_and_italic_tokens_before_the_size() {
        assert_eq!(bold_italic("bold italic 12px Calibri"), (true, true));
        assert_eq!(bold_italic("italic 12px Calibri"), (false, true));
        assert_eq!(bold_italic("12px Boldish"), (false, false));
    }
}
