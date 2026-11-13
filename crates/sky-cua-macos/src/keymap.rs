//! Key-name parsing for physical keyboard input.
//!
//! Pure data and parsing only, so it builds and tests on every target. The
//! CGEvent posting lives in `input.rs`.
//!
//! Virtual key codes are the `kVK_*` values from Carbon `Events.h`. The
//! `kVK_ANSI_*` codes name physical key positions on an ANSI keyboard, so a
//! letter, digit, or punctuation key resolves to the key at that US-layout
//! position; under another layout the same position may produce a different
//! character. Text entry that must be layout-independent goes through
//! `input::type_text`, which posts Unicode strings instead.

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

pub mod vk;

/// Modifier keys held for a chord.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub function: bool,
}

impl Modifiers {
    /// True when no modifier is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The modifier keys to press, in press order (release in reverse).
    ///
    /// The order is fixed (Control, Option, Shift, Command, Fn) rather than
    /// the order the caller named them: applications read the accumulated
    /// modifier flags on the final key event, not the press sequence.
    #[must_use]
    pub fn press_order(&self) -> Vec<(ModifierKey, u16)> {
        [
            (self.control, ModifierKey::Control, vk::CONTROL),
            (self.option, ModifierKey::Option, vk::OPTION),
            (self.shift, ModifierKey::Shift, vk::SHIFT),
            (self.command, ModifierKey::Command, vk::COMMAND),
            (self.function, ModifierKey::Function, vk::FUNCTION),
        ]
        .into_iter()
        .filter_map(|(held, key, code)| held.then_some((key, code)))
        .collect()
    }

    fn set(&mut self, key: ModifierKey) {
        match key {
            ModifierKey::Command => self.command = true,
            ModifierKey::Control => self.control = true,
            ModifierKey::Option => self.option = true,
            ModifierKey::Shift => self.shift = true,
            ModifierKey::Function => self.function = true,
        }
    }
}

/// One modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Command,
    Control,
    Option,
    Shift,
    Function,
}

/// What a chord presses once its modifiers are held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyTarget {
    /// A macOS virtual key code (`kVK_*`), layout-independent.
    VirtualKey(u16),
    /// A character with no dedicated virtual key, typed as Unicode.
    Character(char),
}

/// One parsed key chord, for example `cmd+shift+z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub modifiers: Modifiers,
    /// `None` for a modifier-only chord.
    pub key: Option<KeyTarget>,
}

/// Parses the key names of one chord in press order.
///
/// Accepts the names the shared tool contract uses on the other backends
/// (`Ctrl`, `Control`, `Alt`, `Shift`, `Return`, `Enter`, `Escape`, `Tab`,
/// arrows, function keys, single characters) plus the macOS spellings (`Cmd`,
/// `Command`, `Meta`, `Super`, `Option`, `Opt`, `Fn`). Matching is
/// case-insensitive.
///
/// Elements may be pre-split names (`["Ctrl", "A"]`) or whole chord strings
/// (`["Ctrl+A"]`); both forms are joined with `+` and re-split, so a literal
/// plus key survives either form (`Ctrl++`, `["Ctrl", "", ""]`). `Ctrl` is
/// always Control, never Command. A single letter never implies Shift
/// (`Ctrl+A` is Control and the A key). `Delete`/`Del` is forward delete, as
/// on the other backends; `Backspace` is the key macOS labels Delete. At most
/// one non-modifier key is accepted per chord.
pub fn parse_chord(keys: &[String]) -> Result<KeyChord, BackendError> {
    if keys.is_empty() {
        return Err(invalid("press_key requires at least one key"));
    }
    let joined = keys.join("+");
    let mut chord = KeyChord {
        modifiers: Modifiers::default(),
        key: None,
    };
    for name in split_chord_text(&joined)? {
        if let Some(modifier) = modifier_key(&name) {
            chord.modifiers.set(modifier);
            continue;
        }
        let target = key_target(&name)
            .ok_or_else(|| invalid(format!("unsupported macOS key name '{name}' in '{joined}'")))?;
        if chord.key.is_some() {
            return Err(invalid(format!(
                "a macOS key chord presses at most one non-modifier key: '{joined}'"
            )));
        }
        chord.key = Some(target);
    }
    Ok(chord)
}

/// Splits `Ctrl+Shift+A` into names, keeping a literal `+` key (`Ctrl++`).
fn split_chord_text(text: &str) -> Result<Vec<String>, BackendError> {
    if text.chars().count() == 1 {
        return Ok(vec![text.to_string()]);
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid("press_key requires a non-empty key"));
    }
    if text == "+" {
        return Ok(vec!["+".to_string()]);
    }
    let (body, plus_key) = match text.strip_suffix("++") {
        Some(body) => (body, true),
        None => (text, false),
    };
    let mut names = Vec::new();
    if !(plus_key && body.trim().is_empty()) {
        for part in body.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return Err(invalid(format!("empty key name in chord '{text}'")));
            }
            names.push(part.to_string());
        }
    }
    if plus_key {
        names.push("+".to_string());
    }
    Ok(names)
}

/// Lowercases a multi-character key name and drops `_`, `-`, and spaces, so
/// `Page_Up`, `page-up`, and `PageUp` match. Single characters are kept.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, '_' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn modifier_key(name: &str) -> Option<ModifierKey> {
    if name.chars().count() == 1 {
        return None;
    }
    let name = normalize(name);
    base_modifier(&name).or_else(|| {
        // Side-specific spellings (`ShiftL`, `Ctrl_R`, `LeftAlt`, `RightCmd`)
        // press the same modifier; the chord model carries no side.
        let unsided = name
            .strip_prefix("left")
            .or_else(|| name.strip_prefix("right"))
            .or_else(|| name.strip_suffix('l'))
            .or_else(|| name.strip_suffix('r'))?;
        base_modifier(unsided)
    })
}

fn base_modifier(name: &str) -> Option<ModifierKey> {
    Some(match name {
        "cmd" | "command" | "meta" | "super" => ModifierKey::Command,
        "ctrl" | "control" => ModifierKey::Control,
        "alt" | "option" | "opt" => ModifierKey::Option,
        "shift" => ModifierKey::Shift,
        "fn" | "function" => ModifierKey::Function,
        _ => return None,
    })
}

fn key_target(name: &str) -> Option<KeyTarget> {
    let mut chars = name.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        return Some(
            ansi_character_key(ch).map_or(KeyTarget::Character(ch), KeyTarget::VirtualKey),
        );
    }
    let name = normalize(name);
    if name == "plus" {
        return Some(KeyTarget::Character('+'));
    }
    named_key(&name).map(KeyTarget::VirtualKey)
}

/// The ANSI virtual key for a character typed without Shift. Letters match
/// case-insensitively and never imply Shift.
#[must_use]
pub fn ansi_character_key(ch: char) -> Option<u16> {
    Some(match ch.to_ascii_lowercase() {
        'a' => vk::ANSI_A,
        'b' => vk::ANSI_B,
        'c' => vk::ANSI_C,
        'd' => vk::ANSI_D,
        'e' => vk::ANSI_E,
        'f' => vk::ANSI_F,
        'g' => vk::ANSI_G,
        'h' => vk::ANSI_H,
        'i' => vk::ANSI_I,
        'j' => vk::ANSI_J,
        'k' => vk::ANSI_K,
        'l' => vk::ANSI_L,
        'm' => vk::ANSI_M,
        'n' => vk::ANSI_N,
        'o' => vk::ANSI_O,
        'p' => vk::ANSI_P,
        'q' => vk::ANSI_Q,
        'r' => vk::ANSI_R,
        's' => vk::ANSI_S,
        't' => vk::ANSI_T,
        'u' => vk::ANSI_U,
        'v' => vk::ANSI_V,
        'w' => vk::ANSI_W,
        'x' => vk::ANSI_X,
        'y' => vk::ANSI_Y,
        'z' => vk::ANSI_Z,
        '0' => vk::ANSI_0,
        '1' => vk::ANSI_1,
        '2' => vk::ANSI_2,
        '3' => vk::ANSI_3,
        '4' => vk::ANSI_4,
        '5' => vk::ANSI_5,
        '6' => vk::ANSI_6,
        '7' => vk::ANSI_7,
        '8' => vk::ANSI_8,
        '9' => vk::ANSI_9,
        '-' => vk::ANSI_MINUS,
        '=' => vk::ANSI_EQUAL,
        '[' => vk::ANSI_LEFT_BRACKET,
        ']' => vk::ANSI_RIGHT_BRACKET,
        '\\' => vk::ANSI_BACKSLASH,
        ';' => vk::ANSI_SEMICOLON,
        '\'' => vk::ANSI_QUOTE,
        ',' => vk::ANSI_COMMA,
        '.' => vk::ANSI_PERIOD,
        '/' => vk::ANSI_SLASH,
        '`' => vk::ANSI_GRAVE,
        ' ' => vk::SPACE,
        '\t' => vk::TAB,
        '\n' | '\r' => vk::RETURN,
        _ => return None,
    })
}

/// The virtual key for a normalized multi-character key name.
fn named_key(name: &str) -> Option<u16> {
    if let Some(code) = function_key(name) {
        return Some(code);
    }
    if let Some(code) = ["keypad", "numpad", "kp"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .and_then(keypad_key)
    {
        return Some(code);
    }
    Some(match name {
        "return" | "enter" => vk::RETURN,
        "tab" => vk::TAB,
        "space" | "spacebar" => vk::SPACE,
        "backspace" => vk::DELETE,
        "delete" | "del" | "forwarddelete" => vk::FORWARD_DELETE,
        "escape" | "esc" => vk::ESCAPE,
        "left" | "arrowleft" | "leftarrow" => vk::LEFT_ARROW,
        "right" | "arrowright" | "rightarrow" => vk::RIGHT_ARROW,
        "up" | "arrowup" | "uparrow" => vk::UP_ARROW,
        "down" | "arrowdown" | "downarrow" => vk::DOWN_ARROW,
        "home" => vk::HOME,
        "end" => vk::END,
        "pageup" | "pgup" => vk::PAGE_UP,
        "pagedown" | "pgdn" | "pgdown" => vk::PAGE_DOWN,
        "help" => vk::HELP,
        "capslock" => vk::CAPS_LOCK,
        "minus" | "hyphen" => vk::ANSI_MINUS,
        "equal" | "equals" => vk::ANSI_EQUAL,
        "leftbracket" | "bracketleft" => vk::ANSI_LEFT_BRACKET,
        "rightbracket" | "bracketright" => vk::ANSI_RIGHT_BRACKET,
        "backslash" => vk::ANSI_BACKSLASH,
        "semicolon" => vk::ANSI_SEMICOLON,
        "quote" | "apostrophe" => vk::ANSI_QUOTE,
        "comma" => vk::ANSI_COMMA,
        "period" | "dot" => vk::ANSI_PERIOD,
        "slash" => vk::ANSI_SLASH,
        "grave" | "backtick" | "backquote" => vk::ANSI_GRAVE,
        _ => return None,
    })
}

fn function_key(name: &str) -> Option<u16> {
    const CODES: [u16; 20] = [
        vk::F1,
        vk::F2,
        vk::F3,
        vk::F4,
        vk::F5,
        vk::F6,
        vk::F7,
        vk::F8,
        vk::F9,
        vk::F10,
        vk::F11,
        vk::F12,
        vk::F13,
        vk::F14,
        vk::F15,
        vk::F16,
        vk::F17,
        vk::F18,
        vk::F19,
        vk::F20,
    ];
    let digits = name.strip_prefix('f')?;
    if digits.starts_with('0') {
        return None;
    }
    let number: usize = digits.parse().ok()?;
    CODES.get(number.checked_sub(1)?).copied()
}

fn keypad_key(suffix: &str) -> Option<u16> {
    Some(match suffix {
        "0" => vk::KEYPAD_0,
        "1" => vk::KEYPAD_1,
        "2" => vk::KEYPAD_2,
        "3" => vk::KEYPAD_3,
        "4" => vk::KEYPAD_4,
        "5" => vk::KEYPAD_5,
        "6" => vk::KEYPAD_6,
        "7" => vk::KEYPAD_7,
        "8" => vk::KEYPAD_8,
        "9" => vk::KEYPAD_9,
        "enter" | "return" => vk::KEYPAD_ENTER,
        "decimal" | "period" | "dot" | "." => vk::KEYPAD_DECIMAL,
        "plus" | "add" | "+" => vk::KEYPAD_PLUS,
        "minus" | "subtract" => vk::KEYPAD_MINUS,
        "multiply" | "asterisk" | "*" => vk::KEYPAD_MULTIPLY,
        "divide" | "slash" | "/" => vk::KEYPAD_DIVIDE,
        "equal" | "equals" | "=" => vk::KEYPAD_EQUALS,
        "clear" => vk::KEYPAD_CLEAR,
        _ => return None,
    })
}

/// True for keys whose hardware events carry `kCGEventFlagMaskNumericPad`:
/// the keypad and the arrow keys.
#[must_use]
pub fn is_numeric_pad_key(code: u16) -> bool {
    matches!(
        code,
        vk::KEYPAD_0
            | vk::KEYPAD_1
            | vk::KEYPAD_2
            | vk::KEYPAD_3
            | vk::KEYPAD_4
            | vk::KEYPAD_5
            | vk::KEYPAD_6
            | vk::KEYPAD_7
            | vk::KEYPAD_8
            | vk::KEYPAD_9
            | vk::KEYPAD_ENTER
            | vk::KEYPAD_DECIMAL
            | vk::KEYPAD_PLUS
            | vk::KEYPAD_MINUS
            | vk::KEYPAD_MULTIPLY
            | vk::KEYPAD_DIVIDE
            | vk::KEYPAD_EQUALS
            | vk::KEYPAD_CLEAR
    ) || is_arrow_key(code)
}

/// True for keys whose hardware events carry `kCGEventFlagMaskSecondaryFn`:
/// function keys, the navigation cluster, and the arrow keys.
#[must_use]
pub fn is_secondary_fn_key(code: u16) -> bool {
    matches!(
        code,
        vk::F1
            | vk::F2
            | vk::F3
            | vk::F4
            | vk::F5
            | vk::F6
            | vk::F7
            | vk::F8
            | vk::F9
            | vk::F10
            | vk::F11
            | vk::F12
            | vk::F13
            | vk::F14
            | vk::F15
            | vk::F16
            | vk::F17
            | vk::F18
            | vk::F19
            | vk::F20
            | vk::HOME
            | vk::END
            | vk::PAGE_UP
            | vk::PAGE_DOWN
            | vk::FORWARD_DELETE
            | vk::HELP
    ) || is_arrow_key(code)
}

fn is_arrow_key(code: u16) -> bool {
    matches!(
        code,
        vk::LEFT_ARROW | vk::RIGHT_ARROW | vk::UP_ARROW | vk::DOWN_ARROW
    )
}

fn invalid(message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn chord(names: &[&str]) -> KeyChord {
        parse_chord(&keys(names)).unwrap_or_else(|error| panic!("{names:?}: {error}"))
    }

    fn mods(f: impl FnOnce(&mut Modifiers)) -> Modifiers {
        let mut modifiers = Modifiers::default();
        f(&mut modifiers);
        modifiers
    }

    fn rejects(names: &[&str]) -> BackendError {
        parse_chord(&keys(names)).expect_err(&format!("{names:?} should be rejected"))
    }

    #[test]
    fn ctrl_is_control_never_command() {
        for spelling in ["Ctrl", "ctrl", "CONTROL", "Control", "ctrl_l", "RightCtrl"] {
            let parsed = chord(&[spelling, "a"]);
            assert_eq!(parsed.modifiers, mods(|m| m.control = true), "{spelling}");
            assert_eq!(parsed.key, Some(KeyTarget::VirtualKey(vk::ANSI_A)));
        }
    }

    #[test]
    fn command_spellings() {
        for spelling in ["Cmd", "command", "Meta", "SUPER", "super_l", "RightCmd"] {
            assert_eq!(
                chord(&[spelling, "c"]).modifiers,
                mods(|m| m.command = true),
                "{spelling}"
            );
        }
    }

    #[test]
    fn option_shift_fn_spellings() {
        for spelling in ["Alt", "alt_r", "Option", "opt", "LeftAlt"] {
            assert_eq!(chord(&[spelling, "x"]).modifiers, mods(|m| m.option = true));
        }
        for spelling in ["Shift", "shift_l", "ShiftR", "RightShift"] {
            assert_eq!(chord(&[spelling, "x"]).modifiers, mods(|m| m.shift = true));
        }
        for spelling in ["Fn", "function"] {
            assert_eq!(
                chord(&[spelling, "Left"]).modifiers,
                mods(|m| m.function = true)
            );
        }
    }

    #[test]
    fn whole_chord_strings_split_on_plus() {
        let parsed = chord(&["Cmd+Shift+Z"]);
        assert_eq!(
            parsed.modifiers,
            mods(|m| {
                m.command = true;
                m.shift = true;
            })
        );
        assert_eq!(parsed.key, Some(KeyTarget::VirtualKey(vk::ANSI_Z)));
        assert_eq!(chord(&[" ctrl + a "]), chord(&["Ctrl", "A"]));
    }

    #[test]
    fn literal_plus_key_survives_both_forms() {
        let expected = KeyChord {
            modifiers: mods(|m| m.command = true),
            key: Some(KeyTarget::Character('+')),
        };
        assert_eq!(chord(&["Cmd++"]), expected);
        assert_eq!(chord(&["Cmd", "+"]), expected);
        assert_eq!(chord(&["Cmd", "", ""]), expected);
        assert_eq!(chord(&["Cmd", "Plus"]), expected);
        assert_eq!(
            chord(&["+"]),
            KeyChord {
                modifiers: Modifiers::default(),
                key: Some(KeyTarget::Character('+')),
            }
        );
    }

    #[test]
    fn letters_are_case_insensitive_and_never_imply_shift() {
        let upper = chord(&["A"]);
        assert_eq!(upper.modifiers, Modifiers::default());
        assert_eq!(upper.key, Some(KeyTarget::VirtualKey(vk::ANSI_A)));
        assert_eq!(chord(&["a"]), upper);
    }

    #[test]
    fn full_ansi_character_table() {
        let expected = [
            ('a', 0x00),
            ('s', 0x01),
            ('d', 0x02),
            ('f', 0x03),
            ('h', 0x04),
            ('g', 0x05),
            ('z', 0x06),
            ('x', 0x07),
            ('c', 0x08),
            ('v', 0x09),
            ('b', 0x0B),
            ('q', 0x0C),
            ('w', 0x0D),
            ('e', 0x0E),
            ('r', 0x0F),
            ('y', 0x10),
            ('t', 0x11),
            ('1', 0x12),
            ('2', 0x13),
            ('3', 0x14),
            ('4', 0x15),
            ('6', 0x16),
            ('5', 0x17),
            ('=', 0x18),
            ('9', 0x19),
            ('7', 0x1A),
            ('-', 0x1B),
            ('8', 0x1C),
            ('0', 0x1D),
            (']', 0x1E),
            ('o', 0x1F),
            ('u', 0x20),
            ('[', 0x21),
            ('i', 0x22),
            ('p', 0x23),
            ('l', 0x25),
            ('j', 0x26),
            ('\'', 0x27),
            ('k', 0x28),
            (';', 0x29),
            ('\\', 0x2A),
            (',', 0x2B),
            ('/', 0x2C),
            ('n', 0x2D),
            ('m', 0x2E),
            ('.', 0x2F),
            ('`', 0x32),
            (' ', 0x31),
        ];
        for (ch, code) in expected {
            assert_eq!(ansi_character_key(ch), Some(code), "{ch:?}");
            assert_eq!(
                chord(&[&ch.to_string()]).key,
                Some(KeyTarget::VirtualKey(code)),
                "{ch:?}"
            );
        }
        // Every letter and digit resolves.
        for ch in ('a'..='z').chain('0'..='9') {
            assert!(ansi_character_key(ch).is_some(), "{ch}");
        }
    }

    #[test]
    fn named_keys_resolve() {
        let expected: &[(&str, u16)] = &[
            ("Return", vk::RETURN),
            ("Enter", vk::RETURN),
            ("Tab", vk::TAB),
            ("Space", vk::SPACE),
            ("Backspace", vk::DELETE),
            ("Delete", vk::FORWARD_DELETE),
            ("Del", vk::FORWARD_DELETE),
            ("ForwardDelete", vk::FORWARD_DELETE),
            ("Escape", vk::ESCAPE),
            ("Esc", vk::ESCAPE),
            ("Left", vk::LEFT_ARROW),
            ("ArrowLeft", vk::LEFT_ARROW),
            ("Right", vk::RIGHT_ARROW),
            ("Up", vk::UP_ARROW),
            ("ArrowUp", vk::UP_ARROW),
            ("Down", vk::DOWN_ARROW),
            ("Home", vk::HOME),
            ("End", vk::END),
            ("PageUp", vk::PAGE_UP),
            ("Page_Up", vk::PAGE_UP),
            ("page-down", vk::PAGE_DOWN),
            ("PgDn", vk::PAGE_DOWN),
            ("Help", vk::HELP),
            ("CapsLock", vk::CAPS_LOCK),
            ("Minus", vk::ANSI_MINUS),
            ("Equal", vk::ANSI_EQUAL),
            ("BracketLeft", vk::ANSI_LEFT_BRACKET),
            ("BracketRight", vk::ANSI_RIGHT_BRACKET),
            ("Backslash", vk::ANSI_BACKSLASH),
            ("Semicolon", vk::ANSI_SEMICOLON),
            ("Quote", vk::ANSI_QUOTE),
            ("Comma", vk::ANSI_COMMA),
            ("Period", vk::ANSI_PERIOD),
            ("Slash", vk::ANSI_SLASH),
            ("Backquote", vk::ANSI_GRAVE),
            ("KP_Enter", vk::KEYPAD_ENTER),
            ("Numpad5", vk::KEYPAD_5),
            ("keypad0", vk::KEYPAD_0),
            ("KP_Add", vk::KEYPAD_PLUS),
            ("NumpadSubtract", vk::KEYPAD_MINUS),
            ("KP_Multiply", vk::KEYPAD_MULTIPLY),
            ("NumpadDivide", vk::KEYPAD_DIVIDE),
            ("KP_Decimal", vk::KEYPAD_DECIMAL),
            ("KeypadEquals", vk::KEYPAD_EQUALS),
            ("KeypadClear", vk::KEYPAD_CLEAR),
        ];
        for (name, code) in expected {
            assert_eq!(
                chord(&[name]).key,
                Some(KeyTarget::VirtualKey(*code)),
                "{name}"
            );
        }
    }

    #[test]
    fn function_keys_f1_to_f20() {
        let expected = [
            0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B,
            0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
        ];
        for (index, code) in expected.into_iter().enumerate() {
            let name = format!("F{}", index + 1);
            assert_eq!(
                chord(&[&name]).key,
                Some(KeyTarget::VirtualKey(code)),
                "{name}"
            );
        }
        for name in ["F0", "F21", "F01", "Fx"] {
            rejects(&[name]);
        }
    }

    #[test]
    fn non_ansi_characters_type_as_unicode() {
        for ch in ['!', '?', 'é', '€', '😀'] {
            let parsed = chord(&[&ch.to_string()]);
            assert_eq!(parsed.key, Some(KeyTarget::Character(ch)), "{ch}");
        }
        assert_eq!(
            chord(&["Shift", "1"]).key,
            Some(KeyTarget::VirtualKey(vk::ANSI_1))
        );
    }

    #[test]
    fn modifier_only_chords() {
        let parsed = chord(&["Shift"]);
        assert_eq!(parsed.modifiers, mods(|m| m.shift = true));
        assert_eq!(parsed.key, None);
        let parsed = chord(&["Cmd+Option"]);
        assert_eq!(
            parsed.modifiers,
            mods(|m| {
                m.command = true;
                m.option = true;
            })
        );
        assert_eq!(parsed.key, None);
    }

    #[test]
    fn repeated_modifier_is_idempotent() {
        assert_eq!(chord(&["shift", "Shift", "a"]), chord(&["Shift", "a"]));
    }

    #[test]
    fn rejections_are_invalid_requests() {
        for names in [
            &[][..],
            &["Ctrl", "A", "B"][..],
            &["Ctrl+Nope"][..],
            &["Ctrl++A"][..],
            &["Ctrl+"][..],
            &["  "][..],
            &["Windows"][..],
            &["Insert"][..],
        ] {
            assert_eq!(rejects(names).code, "InvalidRequest", "{names:?}");
        }
    }

    #[test]
    fn press_order_is_fixed() {
        let all = Modifiers {
            command: true,
            control: true,
            option: true,
            shift: true,
            function: true,
        };
        assert_eq!(
            all.press_order(),
            vec![
                (ModifierKey::Control, vk::CONTROL),
                (ModifierKey::Option, vk::OPTION),
                (ModifierKey::Shift, vk::SHIFT),
                (ModifierKey::Command, vk::COMMAND),
                (ModifierKey::Function, vk::FUNCTION),
            ]
        );
        assert!(Modifiers::default().press_order().is_empty());
        assert!(Modifiers::default().is_empty());
        assert!(!all.is_empty());
    }

    #[test]
    fn hardware_flag_classes() {
        assert!(is_numeric_pad_key(vk::LEFT_ARROW));
        assert!(is_secondary_fn_key(vk::LEFT_ARROW));
        assert!(is_numeric_pad_key(vk::KEYPAD_7));
        assert!(!is_secondary_fn_key(vk::KEYPAD_7));
        assert!(is_secondary_fn_key(vk::F5));
        assert!(is_secondary_fn_key(vk::HOME));
        assert!(is_secondary_fn_key(vk::FORWARD_DELETE));
        assert!(!is_numeric_pad_key(vk::HOME));
        assert!(!is_numeric_pad_key(vk::ANSI_A));
        assert!(!is_secondary_fn_key(vk::DELETE));
        assert!(!is_secondary_fn_key(vk::RETURN));
    }
}
