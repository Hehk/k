use super::*;
use gpui::TestAppContext;

#[gpui::test]
fn editing_uses_graphemes_and_supports_selection_and_clipboard(cx: &mut TestAppContext) {
    cx.update(bind_keys);
    let (input, cx) = cx.add_window_view(|_, cx| TextInput::new("search", "Search", cx));
    input.update(cx, |input, cx| input.set_text("a\u{301}😀b", cx));
    cx.update(|window, cx| {
        window.focus(&input.focus_handle(cx));
        window.draw(cx).clear();
    });
    cx.simulate_keystrokes("left backspace backspace");
    input.read_with(cx, |input, _| assert_eq!(input.text, "b"));
    cx.simulate_keystrokes("cmd-a cmd-c cmd-x cmd-v");
    input.read_with(cx, |input, _| assert_eq!(input.text, "b"));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("branch/topic");
    input.read_with(cx, |input, _| assert_eq!(input.text, "branch/topic"));
}

#[gpui::test]
fn ime_ranges_are_relative_to_inserted_text_and_use_utf16(cx: &mut TestAppContext) {
    let (input, cx) = cx.add_window_view(|_, cx| TextInput::new("search", "Search", cx));
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.set_text("abZ", cx);
            input.replace_and_mark_text_in_range(Some(2..3), "😀é", Some(2..3), window, cx);
            assert_eq!(input.text, "ab😀é");
            assert_eq!(input.marked_text_range(window, cx), Some(2..5));
            assert_eq!(
                input.selected_text_range(false, window, cx).unwrap().range,
                4..5
            );
            input.replace_text_in_range(None, "語", window, cx);
            assert_eq!(input.text, "ab語");
            assert!(input.marked.is_none());
            input.replace_text_in_range(Some(0..100), "one\ntwo", window, cx);
            assert_eq!(input.text, "one two");
        })
    });
}
