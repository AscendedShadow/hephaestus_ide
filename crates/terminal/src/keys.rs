//! Keyboard input encoding, following xterm's conventions.

/// Modifier keys held with a key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

/// See [`crate::Terminal::key_sequence`]. `app_cursor` is the mode in which programs such as
/// full-screen editors ask for SS3-prefixed cursor keys.
pub(crate) fn sequence(
    key: &str,
    text: Option<&str>,
    modifiers: Modifiers,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let Modifiers {
        control,
        alt,
        shift,
    } = modifiers;
    // xterm's modifier parameter: 1 + Shift + 2 × Alt + 4 × Control.
    let parameter = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let cursor = |end: char| {
        if parameter > 1 {
            format!("\x1b[1;{parameter}{end}")
        } else if app_cursor {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let function = |end: char| {
        if parameter > 1 {
            format!("\x1b[1;{parameter}{end}")
        } else {
            format!("\x1bO{end}")
        }
    };
    let tilde = |code: u8| {
        if parameter > 1 {
            format!("\x1b[{code};{parameter}~")
        } else {
            format!("\x1b[{code}~")
        }
    };
    let meta = |bytes: &[u8]| {
        let mut sequence = if alt { vec![0x1b] } else { Vec::new() };
        sequence.extend_from_slice(bytes);
        sequence
    };

    let sequence = match key {
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "tab" if shift => "\x1b[Z".into(),
        "tab" => return Some(meta(b"\t")),
        "enter" => return Some(meta(b"\r")),
        "escape" => return Some(meta(b"\x1b")),
        "backspace" if control => return Some(meta(b"\x08")),
        "backspace" => return Some(meta(b"\x7f")),
        // Control+Alt is also AltGr on Windows, which types characters: leave those as text.
        _ if control && !alt => return control_code(key).map(|code| vec![code]),
        // macOS Option composes characters rather than acting as Meta.
        _ if alt && !control && !cfg!(target_os = "macos") => {
            return Some(meta(text.unwrap_or(key).as_bytes()));
        }
        _ => return None,
    };
    Some(sequence.into_bytes())
}

/// The C0 control code a Control chord sends, as in xterm.
fn control_code(key: &str) -> Option<u8> {
    let [byte] = *key.as_bytes() else {
        return (key == "space").then_some(0);
    };
    Some(match byte {
        b'a'..=b'z' => byte - b'a' + 1,
        b'@' | b'2' => 0,
        b'[' | b'3' => 0x1b,
        b'\\' | b'4' => 0x1c,
        b']' | b'5' => 0x1d,
        b'^' | b'6' => 0x1e,
        b'_' | b'-' | b'/' | b'7' => 0x1f,
        b'?' | b'8' => 0x7f,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(key: &str, text: Option<&str>, modifiers: Modifiers, app_cursor: bool) -> String {
        String::from_utf8(sequence(key, text, modifiers, app_cursor).unwrap()).unwrap()
    }

    const NONE: Modifiers = Modifiers {
        control: false,
        alt: false,
        shift: false,
    };
    const CONTROL: Modifiers = Modifiers {
        control: true,
        ..NONE
    };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };

    #[test]
    fn cursor_keys_follow_application_cursor_mode_and_modifiers() {
        assert_eq!(keys("up", None, NONE, false), "\x1b[A");
        assert_eq!(keys("up", None, NONE, true), "\x1bOA");
        assert_eq!(keys("left", None, CONTROL, true), "\x1b[1;5D");
        assert_eq!(keys("home", None, SHIFT, false), "\x1b[1;2H");
        assert_eq!(keys("delete", None, NONE, false), "\x1b[3~");
        assert_eq!(keys("pageup", None, ALT, false), "\x1b[5;3~");
        assert_eq!(keys("f1", None, NONE, false), "\x1bOP");
        assert_eq!(keys("f5", None, CONTROL, false), "\x1b[15;5~");
    }

    #[test]
    fn control_and_editing_keys() {
        assert_eq!(keys("c", None, CONTROL, false), "\x03");
        assert_eq!(keys("[", None, CONTROL, false), "\x1b");
        assert_eq!(keys("space", Some(" "), CONTROL, false), "\0");
        assert_eq!(keys("enter", None, NONE, false), "\r");
        assert_eq!(keys("backspace", None, NONE, false), "\x7f");
        assert_eq!(keys("backspace", None, CONTROL, false), "\x08");
        assert_eq!(keys("tab", None, SHIFT, false), "\x1b[Z");
        assert_eq!(keys("escape", None, NONE, false), "\x1b");
    }

    #[test]
    fn plain_text_and_altgr_are_left_to_text_input() {
        assert_eq!(sequence("a", Some("a"), NONE, false), None);
        assert_eq!(sequence("a", Some("A"), SHIFT, false), None);
        let altgr = Modifiers {
            control: true,
            alt: true,
            shift: false,
        };
        assert_eq!(sequence("q", Some("@"), altgr, false), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn alt_sends_escape_prefixed_text() {
        assert_eq!(keys("b", Some("b"), ALT, false), "\x1bb");
        assert_eq!(keys("enter", None, ALT, false), "\x1b\r");
    }
}
