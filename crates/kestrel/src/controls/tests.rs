use super::single_line;

#[test]
fn single_line_escapes_controls_and_unicode_line_separators() {
    assert_eq!(
        single_line("repo\nname\r\t\0\u{7}\u{1b}\u{7f}\u{85}\u{2028}\u{2029}"),
        r"repo\nname\r\t\u{0}\u{7}\u{1b}\u{7f}\u{85}\u{2028}\u{2029}"
    );
    for ch in ('\0'..='\u{9f}').filter(|ch| ch.is_control()) {
        let label = single_line(&ch.to_string());
        assert!(!label.chars().any(char::is_control));
        assert_eq!(single_line(&label), label);
    }
}

#[test]
fn single_line_preserves_ordinary_unicode_spaces_and_literal_escapes() {
    let text = " repo 日本語 🦅 e\u{301} 'quoted' \\\" \\n.rs ";
    assert_eq!(single_line(text), text);
}
