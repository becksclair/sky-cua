use std::collections::HashMap;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use xkbcommon::xkb;

#[derive(Debug, Clone, Copy)]
pub(crate) struct EisKeyStroke {
    pub(crate) keycode: u32,
    pub(crate) modifiers: EisKeyModifiers,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EisKeyModifiers {
    pub(crate) shift: bool,
    pub(crate) level3: bool,
}

impl EisKeyModifiers {
    pub(crate) fn for_xkb_level(level: u32) -> Option<Self> {
        match level {
            0 => Some(Self::default()),
            1 => Some(Self {
                shift: true,
                level3: false,
            }),
            2 => Some(Self {
                shift: false,
                level3: true,
            }),
            3 => Some(Self {
                shift: true,
                level3: true,
            }),
            _ => None,
        }
    }

    fn from_xkb_mask(keymap: &xkb::Keymap, mask: xkb::ModMask) -> Option<Self> {
        let shift_bit = xkb_modifier_bit(keymap, xkb::MOD_NAME_SHIFT);
        let level3_bit = xkb_modifier_bit(keymap, xkb::MOD_NAME_ISO_LEVEL3_SHIFT);
        let mut supported_mask = 0;

        if let Some(bit) = shift_bit {
            supported_mask |= bit;
        }
        if let Some(bit) = level3_bit {
            supported_mask |= bit;
        }

        if mask & !supported_mask != 0 {
            return None;
        }

        Some(Self {
            shift: shift_bit.is_some_and(|bit| mask & bit != 0),
            level3: level3_bit.is_some_and(|bit| mask & bit != 0),
        })
    }

    fn weight(self) -> u8 {
        u8::from(self.shift) + u8::from(self.level3)
    }
}

pub(crate) fn resolve_eis_keystroke(
    keysym_cache: &HashMap<u32, EisKeyStroke>,
    keysym: i32,
) -> Result<EisKeyStroke, BackendError> {
    keysym_cache.get(&(keysym as u32)).copied().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::ActionUnsupportedForEnvironment,
            format!("EIS keyboard keymap cannot produce keysym 0x{keysym:x}"),
        )
    })
}

pub(crate) fn build_keysym_cache(keymap: &xkb::Keymap) -> HashMap<u32, EisKeyStroke> {
    let mut cache: HashMap<u32, EisKeyStroke> = HashMap::with_capacity(1024);
    let min_keycode = keymap.min_keycode().raw();
    let max_keycode = keymap.max_keycode().raw();
    // NOTE: XKB keycodes use an 8-value offset from evdev scancodes.
    // Keycodes 1–7 are below the evdev range and are silently skipped here.
    // This is standard; custom keymaps that place symbols on those keycodes
    // will not be reachable through EIS injection.
    for raw_keycode in min_keycode..=max_keycode {
        let keycode = xkb::Keycode::new(raw_keycode);
        let layout_count = keymap.num_layouts_for_key(keycode).max(1);
        for layout in 0..layout_count {
            let level_count = keymap.num_levels_for_key(keycode, layout).max(1);
            for level in 0..level_count {
                let Some(modifiers) = modifiers_for_level(keymap, keycode, layout, level) else {
                    continue;
                };
                for keysym in keymap.key_get_syms_by_level(keycode, layout, level) {
                    if let Some(evdev_keycode) = raw_keycode.checked_sub(8) {
                        let stroke = EisKeyStroke {
                            keycode: evdev_keycode,
                            modifiers,
                        };
                        cache
                            .entry(keysym.raw())
                            .and_modify(|existing| {
                                if stroke.modifiers.weight() < existing.modifiers.weight() {
                                    *existing = stroke;
                                }
                            })
                            .or_insert(stroke);
                    }
                }
            }
        }
    }
    cache
}

#[allow(dead_code)]
pub(crate) fn find_eis_keycode_for_keysym(
    keymap: &xkb::Keymap,
    keysym: i32,
) -> Option<EisKeyStroke> {
    let keysym = xkb::Keysym::new(u32::try_from(keysym).ok()?);
    let min_keycode = keymap.min_keycode().raw();
    let max_keycode = keymap.max_keycode().raw();
    // See `build_keysym_cache` for the evdev offset note on keycodes 1–7.
    for raw_keycode in min_keycode..=max_keycode {
        let keycode = xkb::Keycode::new(raw_keycode);
        let layout_count = keymap.num_layouts_for_key(keycode).max(1);
        for layout in 0..layout_count {
            let level_count = keymap.num_levels_for_key(keycode, layout).max(1);
            for level in 0..level_count {
                let Some(modifiers) = modifiers_for_level(keymap, keycode, layout, level) else {
                    continue;
                };
                if keymap
                    .key_get_syms_by_level(keycode, layout, level)
                    .contains(&keysym)
                {
                    return raw_keycode
                        .checked_sub(8)
                        .map(|keycode| EisKeyStroke { keycode, modifiers });
                }
            }
        }
    }
    None
}

/// Look up keycodes for a set of keysyms using a pre-built cache.
/// This avoids the full keymap scan that `find_eis_keycode_for_keysym` performs.
pub(crate) fn find_keycodes_from_cache(
    cache: &HashMap<u32, EisKeyStroke>,
    keysyms: &[u32],
) -> Vec<u32> {
    let mut keycodes = Vec::new();
    for keysym in keysyms {
        let Some(stroke) = cache.get(keysym).copied() else {
            continue;
        };
        if !keycodes.contains(&stroke.keycode) {
            keycodes.push(stroke.keycode);
        }
    }
    keycodes
}

pub(crate) fn clear_modifiers_already_present_in_chord(
    resolved: &mut [EisKeyStroke],
    shift_keycodes: &[u32],
    level3_keycodes: &[u32],
) {
    let explicit_shift = resolved
        .iter()
        .any(|stroke| shift_keycodes.contains(&stroke.keycode));
    let explicit_level3 = resolved
        .iter()
        .any(|stroke| level3_keycodes.contains(&stroke.keycode));

    if !explicit_shift && !explicit_level3 {
        return;
    }

    for stroke in resolved {
        if explicit_shift && !shift_keycodes.contains(&stroke.keycode) {
            stroke.modifiers.shift = false;
        }
        if explicit_level3 && !level3_keycodes.contains(&stroke.keycode) {
            stroke.modifiers.level3 = false;
        }
    }
}

/// Return the modifier keycodes that must be pressed alongside `stroke`.
///
/// When multiple modifier keys exist (e.g. left and right Shift), the first
/// available one is used. If that keycode is later discovered to be non-functional
/// the caller must handle the failure; this function does not attempt every
/// keycode because EIS key state emission happens on a different thread.
pub(crate) fn required_modifier_keycodes(
    stroke: EisKeyStroke,
    shift_keycodes: &[u32],
    level3_keycodes: &[u32],
) -> Result<Vec<u32>, BackendError> {
    let mut keycodes = Vec::with_capacity(2);
    if stroke.modifiers.shift && !shift_keycodes.contains(&stroke.keycode) {
        let shift_keycode = shift_keycodes.first().copied().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "EIS keymap needs Shift for this key but did not expose a Shift keycode",
            )
        })?;
        keycodes.push(shift_keycode);
    }
    if stroke.modifiers.level3 && !level3_keycodes.contains(&stroke.keycode) {
        let level3_keycode = level3_keycodes.first().copied().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                "EIS keymap needs AltGr/Level3 for this key but did not expose a Level3 keycode",
            )
        })?;
        keycodes.push(level3_keycode);
    }
    Ok(keycodes)
}

fn modifiers_for_level(
    keymap: &xkb::Keymap,
    keycode: xkb::Keycode,
    layout: xkb::LayoutIndex,
    level: xkb::LevelIndex,
) -> Option<EisKeyModifiers> {
    let mask_count = keymap.key_get_mods_for_level(keycode, layout, level, &mut []);
    let mut masks = vec![xkb::ModMask::default(); mask_count];
    let _ = keymap.key_get_mods_for_level(keycode, layout, level, &mut masks);
    masks
        .iter()
        .filter_map(|mask| EisKeyModifiers::from_xkb_mask(keymap, *mask))
        .min_by_key(|modifiers| modifiers.weight())
        .or_else(|| EisKeyModifiers::for_xkb_level(level))
}

fn xkb_modifier_bit(keymap: &xkb::Keymap, name: &str) -> Option<xkb::ModMask> {
    let index = keymap.mod_get_index(name);
    if index == xkb::MOD_INVALID || index >= u32::BITS {
        None
    } else {
        Some(1_u32 << index)
    }
}

pub fn keysym_for_char(character: char) -> Option<i32> {
    match character {
        '\n' | '\r' => Some(0xff0d),
        '\t' => Some(0xff09),
        _ => {
            let keysym = xkb::utf32_to_keysym(u32::from(character)).raw();
            if keysym == xkb::keysyms::KEY_NoSymbol {
                None
            } else {
                Some(keysym as i32)
            }
        }
    }
}

pub fn keysym_for_key_name(key: &str) -> Option<i32> {
    let key = key.trim();
    // Single-character shortcut (after trimming).
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return keysym_for_char(c);
    }

    // Helper: compare `key` against `name` case-insensitively, ignoring underscores in `key`.
    let eq = |name: &str| {
        let mut key_chars = key.chars().filter(|&c| c != '_');
        let mut name_chars = name.chars();
        loop {
            match (key_chars.next(), name_chars.next()) {
                (None, None) => return true,
                (Some(k), Some(n)) if k.eq_ignore_ascii_case(&n) => continue,
                _ => return false,
            }
        }
    };

    if eq("enter") || eq("return") {
        return Some(0xff0d);
    }
    if eq("tab") {
        return Some(0xff09);
    }
    if eq("backspace") {
        return Some(0xff08);
    }
    if eq("escape") || eq("esc") {
        return Some(0xff1b);
    }
    if eq("space") {
        return Some(0x20);
    }
    if eq("delete") || eq("del") {
        return Some(0xffff);
    }
    if eq("left") {
        return Some(0xff51);
    }
    if eq("up") {
        return Some(0xff52);
    }
    if eq("right") {
        return Some(0xff53);
    }
    if eq("down") {
        return Some(0xff54);
    }
    if eq("home") {
        return Some(0xff50);
    }
    if eq("end") {
        return Some(0xff57);
    }
    if eq("pageup") {
        return Some(0xff55);
    }
    if eq("pagedown") {
        return Some(0xff56);
    }
    if eq("shift") || eq("shiftl") {
        return Some(0xffe1);
    }
    if eq("shiftr") || eq("rightshift") {
        return Some(0xffe2);
    }
    if eq("control") || eq("ctrl") || eq("ctrll") {
        return Some(0xffe3);
    }
    if eq("controlr") || eq("ctrlr") || eq("rightcontrol") || eq("rightctrl") {
        return Some(0xffe4);
    }
    if eq("alt") || eq("altl") {
        return Some(0xffe9);
    }
    if eq("altr") || eq("rightalt") {
        return Some(0xffea);
    }
    if eq("altgr") || eq("level3") || eq("isolevel3shift") || eq("modeswitch") {
        return Some(0xfe03);
    }
    if eq("meta") || eq("super") || eq("superl") || eq("metal") {
        return Some(0xffeb);
    }
    if eq("capslock") {
        return Some(0xffe5);
    }
    if eq("f1") {
        return Some(0xffbe);
    }
    if eq("f2") {
        return Some(0xffbf);
    }
    if eq("f3") {
        return Some(0xffc0);
    }
    if eq("f4") {
        return Some(0xffc1);
    }
    if eq("f5") {
        return Some(0xffc2);
    }
    if eq("f6") {
        return Some(0xffc3);
    }
    if eq("f7") {
        return Some(0xffc4);
    }
    if eq("f8") {
        return Some(0xffc5);
    }
    if eq("f9") {
        return Some(0xffc6);
    }
    if eq("f10") {
        return Some(0xffc7);
    }
    if eq("f11") {
        return Some(0xffc8);
    }
    if eq("f12") {
        return Some(0xffc9);
    }
    None
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::diagnostics::BackendErrorCode;
    use xkbcommon::xkb;

    use super::{
        EisKeyModifiers, EisKeyStroke, clear_modifiers_already_present_in_chord,
        find_eis_keycode_for_keysym, keysym_for_char, keysym_for_key_name,
        required_modifier_keycodes,
    };

    #[test]
    fn resolves_ascii_character_keysyms() {
        assert_eq!(keysym_for_char('a'), Some(i32::from(b'a')));
        assert_eq!(keysym_for_char('\n'), Some(0xff0d));
        assert_eq!(
            keysym_for_char('ä'),
            Some(xkb::keysyms::KEY_adiaeresis as i32)
        );
        assert_eq!(
            keysym_for_char('€'),
            Some(xkb::keysyms::KEY_EuroSign as i32)
        );
    }

    #[test]
    fn resolves_named_keysyms() {
        assert_eq!(keysym_for_key_name("Enter"), Some(0xff0d));
        assert_eq!(keysym_for_key_name("Ctrl"), Some(0xffe3));
        assert_eq!(keysym_for_key_name("AltGr"), Some(0xfe03));
        assert_eq!(keysym_for_key_name("f5"), Some(0xffc2));
    }

    #[test]
    fn resolves_shift_modifier_from_xkb_level_masks() {
        let context = xkb::Context::new(0);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            Some("".to_string()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("standard US keymap should compile");

        let stroke = find_eis_keycode_for_keysym(&keymap, xkb::keysyms::KEY_A as i32)
            .expect("US keymap should produce uppercase A");

        assert!(stroke.modifiers.shift);
        assert!(!stroke.modifiers.level3);
    }

    #[test]
    fn resolves_level3_modifier_from_xkb_level_masks() {
        let context = xkb::Context::new(0);
        let Some(keymap) = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "de",
            "",
            Some("".to_string()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        ) else {
            return;
        };

        let stroke = find_eis_keycode_for_keysym(&keymap, xkb::keysyms::KEY_EuroSign as i32)
            .expect("German keymap should produce EuroSign");

        assert!(!stroke.modifiers.shift);
        assert!(stroke.modifiers.level3);
    }

    #[test]
    fn explicit_chord_modifiers_suppress_auto_modifiers() {
        let mut strokes = [
            EisKeyStroke {
                keycode: 42,
                modifiers: EisKeyModifiers::default(),
            },
            EisKeyStroke {
                keycode: 100,
                modifiers: EisKeyModifiers::default(),
            },
            EisKeyStroke {
                keycode: 30,
                modifiers: EisKeyModifiers {
                    shift: true,
                    level3: true,
                },
            },
        ];

        clear_modifiers_already_present_in_chord(&mut strokes, &[42], &[100]);

        assert_eq!(strokes[2].modifiers, EisKeyModifiers::default());
    }

    #[test]
    fn required_modifier_keycodes_are_stable_and_report_missing_modifiers() {
        let stroke = EisKeyStroke {
            keycode: 30,
            modifiers: EisKeyModifiers {
                shift: true,
                level3: true,
            },
        };

        assert_eq!(
            required_modifier_keycodes(stroke, &[42], &[100]).unwrap(),
            vec![42, 100]
        );

        let error = required_modifier_keycodes(stroke, &[], &[100]).unwrap_err();
        assert_eq!(
            error.code,
            BackendErrorCode::ActionUnsupportedForEnvironment.as_str()
        );
        assert!(error.message.contains("Shift"));
    }
}
