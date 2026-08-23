#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

pub fn encode_key(key: &str, key_char: Option<&str>, modifiers: KeyModifiers, application_cursor: bool) -> Option<Vec<u8>> {
    let modifier = 1 + modifiers.shift as u8 + (modifiers.alt as u8 * 2) + (modifiers.control as u8 * 4);
    let cursor = |normal: &str, application: &str| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{normal}")
        } else if application_cursor {
            format!("\x1bO{application}")
        } else {
            format!("\x1b[{normal}")
        }
    };
    let mut bytes = match key {
        "enter" => b"\r".to_vec(),
        "tab" if modifiers.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" => b"\x7f".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "up" => cursor("A", "A").into_bytes(),
        "down" => cursor("B", "B").into_bytes(),
        "right" => cursor("C", "C").into_bytes(),
        "left" => cursor("D", "D").into_bytes(),
        "home" => cursor("H", "H").into_bytes(),
        "end" => cursor("F", "F").into_bytes(),
        "insert" => tilde_key(2, modifier).into_bytes(),
        "delete" => tilde_key(3, modifier).into_bytes(),
        "pageup" => tilde_key(5, modifier).into_bytes(),
        "pagedown" => tilde_key(6, modifier).into_bytes(),
        "f1" => function_key("P", modifier).into_bytes(),
        "f2" => function_key("Q", modifier).into_bytes(),
        "f3" => function_key("R", modifier).into_bytes(),
        "f4" => function_key("S", modifier).into_bytes(),
        "f5" => tilde_key(15, modifier).into_bytes(),
        "f6" => tilde_key(17, modifier).into_bytes(),
        "f7" => tilde_key(18, modifier).into_bytes(),
        "f8" => tilde_key(19, modifier).into_bytes(),
        "f9" => tilde_key(20, modifier).into_bytes(),
        "f10" => tilde_key(21, modifier).into_bytes(),
        "f11" => tilde_key(23, modifier).into_bytes(),
        "f12" => tilde_key(24, modifier).into_bytes(),
        _ => {
            let text = key_char.filter(|text| !text.is_empty()).unwrap_or(key);
            if modifiers.control { control_character(text)? } else { text.as_bytes().to_vec() }
        }
    };
    if modifiers.alt && !bytes.starts_with(b"\x1b") {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if bracketed {
        [b"\x1b[200~".as_slice(), normalized.as_bytes(), b"\x1b[201~".as_slice()].concat()
    } else {
        normalized.into_bytes()
    }
}

fn control_character(text: &str) -> Option<Vec<u8>> {
    let character = text.as_bytes().first().copied()?;
    let control = match character {
        b'@' | b'`' | b' ' => 0,
        b'a'..=b'z' => character - b'a' + 1,
        b'A'..=b'Z' => character - b'A' + 1,
        b'[' => 27,
        b'\\' => 28,
        b']' => 29,
        b'^' => 30,
        b'_' | b'/' => 31,
        b'?' => 127,
        _ => return None,
    };
    Some(vec![control])
}

fn tilde_key(number: u8, modifier: u8) -> String {
    if modifier == 1 {
        format!("\x1b[{number}~")
    } else {
        format!("\x1b[{number};{modifier}~")
    }
}

fn function_key(final_character: &str, modifier: u8) -> String {
    if modifier == 1 {
        format!("\x1bO{final_character}")
    } else {
        format!("\x1b[1;{modifier}{final_character}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_control_alt_navigation_and_function_keys() {
        assert_eq!(
            encode_key(
                "c",
                Some("c"),
                KeyModifiers {
                    control: true,
                    ..Default::default()
                },
                false
            ),
            Some(vec![3])
        );
        assert_eq!(encode_key("left", None, KeyModifiers::default(), true), Some(b"\x1bOD".to_vec()));
        assert_eq!(
            encode_key(
                "up",
                None,
                KeyModifiers {
                    control: true,
                    shift: true,
                    alt: false
                },
                false
            ),
            Some(b"\x1b[1;6A".to_vec())
        );
        assert_eq!(encode_key("x", Some("x"), KeyModifiers { alt: true, ..Default::default() }, false), Some(b"\x1bx".to_vec()));
        assert_eq!(encode_key("f12", None, KeyModifiers::default(), false), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn wraps_and_normalizes_bracketed_paste() {
        assert_eq!(encode_paste("a\r\nb\rc", true), b"\x1b[200~a\nb\nc\x1b[201~");
    }
}
