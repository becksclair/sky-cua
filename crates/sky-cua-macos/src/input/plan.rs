//! Pure event planning for CGEvent input: drag paths, scroll steps, text
//! segmentation, and chord key sequences. Nothing here touches CoreGraphics
//! posting, so every rule is unit-tested without injecting input.

use crate::keymap::{self, KeyChord, KeyTarget, ModifierKey};

/// `CGEventKeyboardSetUnicodeString` keeps at most 20 UTF-16 code units per
/// event and silently drops the rest.
pub(super) const MAX_UTF16_PER_EVENT: usize = 20;

/// Points of pointer travel per interpolated drag event.
const DRAG_POINTS_PER_STEP: f64 = 16.0;
/// Fewest dragged events per drag, so drag recognizers that need several
/// motion events past their threshold still start, even for short drags.
const DRAG_MIN_STEPS: usize = 8;
/// Most dragged events per drag, which bounds the duration of long drags.
const DRAG_MAX_STEPS: usize = 40;

/// Wheel lines per scroll event. Real wheels report a few lines per event;
/// some applications clamp or ignore very large single-event deltas.
const SCROLL_LINES_PER_EVENT: u64 = 4;
/// Most scroll events per call. Beyond `SCROLL_LINES_PER_EVENT *
/// MAX_SCROLL_EVENTS` lines the per-event delta grows instead, which bounds
/// the duration of absurd requests.
const MAX_SCROLL_EVENTS: u64 = 256;

/// CGEventFlags bits, kept as raw values so this module stays independent of
/// the CoreGraphics bindings. Values from `CGEventTypes.h`.
pub(super) mod flag {
    pub const SHIFT: u64 = 1 << 17;
    pub const CONTROL: u64 = 1 << 18;
    pub const ALTERNATE: u64 = 1 << 19;
    pub const COMMAND: u64 = 1 << 20;
    pub const NUMERIC_PAD: u64 = 1 << 21;
    pub const SECONDARY_FN: u64 = 1 << 23;
}

/// Interpolated `LeftMouseDragged` locations from `from` (exclusive) to `to`
/// (inclusive). The final point is exactly `to`.
pub(super) fn drag_path(from: (f64, f64), to: (f64, f64)) -> Vec<(f64, f64)> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let distance = dx.hypot(dy);
    let steps = if distance.is_finite() {
        // Saturating float-to-int cast; the result is clamped to a small range.
        ((distance / DRAG_POINTS_PER_STEP).ceil() as usize).clamp(DRAG_MIN_STEPS, DRAG_MAX_STEPS)
    } else {
        DRAG_MIN_STEPS
    };
    (1..=steps)
        .map(|step| {
            if step == steps {
                to
            } else {
                let t = step as f64 / steps as f64;
                (from.0 + dx * t, from.1 + dy * t)
            }
        })
        .collect()
}

/// Line-unit scroll events as `(wheel1, wheel2)` pairs for the contract
/// deltas, where positive `delta_y` moves content down (the view travels
/// toward the top) and positive `delta_x` moves content right (the view
/// travels toward the left).
///
/// That is the CoreGraphics wheel convention, so values pass through
/// unchanged: positive `wheel1` scrolls toward the top, positive `wheel2`
/// toward the left. Synthetic events are not subject to the user's
/// natural-scrolling preference, which only inverts device input. Each event
/// carries at most `SCROLL_LINES_PER_EVENT` lines per axis (up to
/// `MAX_SCROLL_EVENTS` events) and the per-axis sums are exact.
pub(super) fn scroll_steps(delta_x: i32, delta_y: i32) -> Vec<(i32, i32)> {
    let wheel1 = i64::from(delta_y);
    let wheel2 = i64::from(delta_x);
    let events = wheel1
        .unsigned_abs()
        .max(wheel2.unsigned_abs())
        .div_ceil(SCROLL_LINES_PER_EVENT)
        .min(MAX_SCROLL_EVENTS);
    let events = usize::try_from(events).unwrap_or(0);
    split_evenly(wheel1, events)
        .into_iter()
        .zip(split_evenly(wheel2, events))
        .collect()
}

/// Splits `total` into `parts` integers whose sum is exactly `total` and
/// whose magnitudes differ by at most one.
fn split_evenly(total: i64, parts: usize) -> Vec<i32> {
    if parts == 0 {
        return Vec::new();
    }
    let parts_i64 = i64::try_from(parts).unwrap_or(i64::MAX);
    let base = total / parts_i64;
    let remainder = (total % parts_i64).unsigned_abs();
    let sign = total.signum();
    (0..parts)
        .map(|index| {
            let extra = if (index as u64) < remainder { sign } else { 0 };
            // |total| <= 2^31 and parts >= 1, so every part fits in i32.
            i32::try_from(base + extra).unwrap_or(0)
        })
        .collect()
}

/// One unit of `type_text` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TextSegment {
    /// UTF-16 units posted on one key-down/key-up pair, never more than
    /// `MAX_UTF16_PER_EVENT` and never splitting a surrogate pair.
    Unicode(Vec<u16>),
    /// A physical key: Return for line breaks, Tab for tabs. Text views
    /// handle these as `insertNewline:`/`insertTab:` commands, which a
    /// Unicode `\n` or `\t` string does not reliably trigger.
    Key(u16),
}

/// Segments text for Unicode typing. `\r\n`, `\n`, and `\r` each become one
/// Return press; `\t` becomes one Tab press.
pub(super) fn text_segments(text: &str) -> Vec<TextSegment> {
    let mut segments = Vec::new();
    let mut pending: Vec<u16> = Vec::new();
    let mut chars = text.chars().peekable();
    let mut buffer = [0u16; 2];
    while let Some(ch) = chars.next() {
        let key = match ch {
            '\r' => {
                chars.next_if_eq(&'\n');
                Some(keymap::vk::RETURN)
            }
            '\n' => Some(keymap::vk::RETURN),
            '\t' => Some(keymap::vk::TAB),
            _ => None,
        };
        if let Some(key) = key {
            if !pending.is_empty() {
                segments.push(TextSegment::Unicode(std::mem::take(&mut pending)));
            }
            segments.push(TextSegment::Key(key));
            continue;
        }
        let units = ch.encode_utf16(&mut buffer);
        if pending.len() + units.len() > MAX_UTF16_PER_EVENT {
            segments.push(TextSegment::Unicode(std::mem::take(&mut pending)));
        }
        pending.extend_from_slice(units);
    }
    if !pending.is_empty() {
        segments.push(TextSegment::Unicode(pending));
    }
    segments
}

/// The CGEvent type a planned keystroke is posted as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StrokeKind {
    /// A modifier key changing state, as hardware reports it.
    FlagsChanged,
    KeyDown,
    KeyUp,
}

/// One planned keyboard event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeyStroke {
    pub kind: StrokeKind,
    /// True for a press, false for a release.
    pub down: bool,
    pub key_code: u16,
    /// Full CGEventFlags for the event, set explicitly so modifiers the user
    /// physically holds never leak into an injected chord.
    pub flags: u64,
    /// Unicode payload for a character with no virtual key.
    pub character: Option<char>,
}

/// Plans one chord: modifiers down in `Modifiers::press_order`, each with
/// the accumulated flags; the key down and up with every modifier flag plus
/// the flags hardware sets for that key; modifiers up in reverse order with
/// the remaining flags.
pub(super) fn chord_plan(chord: &KeyChord) -> Vec<KeyStroke> {
    let modifiers = chord.modifiers.press_order();
    let mut strokes = Vec::with_capacity(modifiers.len() * 2 + 2);
    let mut flags = 0;
    for (modifier, code) in &modifiers {
        flags |= modifier_flag(*modifier);
        strokes.push(KeyStroke {
            kind: StrokeKind::FlagsChanged,
            down: true,
            key_code: *code,
            flags,
            character: None,
        });
    }
    if let Some(target) = &chord.key {
        let (key_code, character, implied) = match target {
            KeyTarget::VirtualKey(code) => (*code, None, implied_key_flags(*code)),
            // Unicode events conventionally carry key code 0; the string, not
            // the code, is what the receiving application inserts.
            KeyTarget::Character(ch) => (0, Some(*ch), 0),
        };
        for down in [true, false] {
            strokes.push(KeyStroke {
                kind: if down {
                    StrokeKind::KeyDown
                } else {
                    StrokeKind::KeyUp
                },
                down,
                key_code,
                flags: flags | implied,
                character,
            });
        }
    }
    for (modifier, code) in modifiers.iter().rev() {
        flags &= !modifier_flag(*modifier);
        strokes.push(KeyStroke {
            kind: StrokeKind::FlagsChanged,
            down: false,
            key_code: *code,
            flags,
            character: None,
        });
    }
    strokes
}

fn modifier_flag(modifier: ModifierKey) -> u64 {
    match modifier {
        ModifierKey::Command => flag::COMMAND,
        ModifierKey::Control => flag::CONTROL,
        ModifierKey::Option => flag::ALTERNATE,
        ModifierKey::Shift => flag::SHIFT,
        ModifierKey::Function => flag::SECONDARY_FN,
    }
}

/// Flags hardware keyboards set on a key regardless of held modifiers.
pub(super) fn implied_key_flags(code: u16) -> u64 {
    let mut flags = 0;
    if keymap::is_numeric_pad_key(code) {
        flags |= flag::NUMERIC_PAD;
    }
    if keymap::is_secondary_fn_key(code) {
        flags |= flag::SECONDARY_FN;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{Modifiers, parse_chord, vk};

    #[test]
    fn drag_path_ends_exactly_at_target_and_moves_monotonically() {
        let path = drag_path((10.0, 20.0), (410.0, 320.0));
        // 500 points / 16 per step = 32 steps.
        assert_eq!(path.len(), 32);
        assert_eq!(*path.last().unwrap(), (410.0, 320.0));
        let mut previous = (10.0, 20.0);
        for point in &path {
            assert!(point.0 > previous.0 && point.1 > previous.1);
            previous = *point;
        }
    }

    #[test]
    fn drag_path_step_count_is_clamped() {
        assert_eq!(drag_path((0.0, 0.0), (5.0, 0.0)).len(), DRAG_MIN_STEPS);
        assert_eq!(drag_path((0.0, 0.0), (0.0, 0.0)).len(), DRAG_MIN_STEPS);
        assert_eq!(drag_path((0.0, 0.0), (5000.0, 0.0)).len(), DRAG_MAX_STEPS);
        assert_eq!(
            drag_path((0.0, 0.0), (f64::INFINITY, 0.0)).len(),
            DRAG_MIN_STEPS
        );
    }

    #[test]
    fn drag_path_handles_negative_direction() {
        let path = drag_path((300.0, 300.0), (100.0, 300.0));
        assert_eq!(*path.last().unwrap(), (100.0, 300.0));
        assert!(path.windows(2).all(|pair| pair[1].0 < pair[0].0));
        assert!(path.iter().all(|point| point.1 == 300.0));
    }

    fn sums(steps: &[(i32, i32)]) -> (i64, i64) {
        steps.iter().fold((0, 0), |(a, b), (w1, w2)| {
            (a + i64::from(*w1), b + i64::from(*w2))
        })
    }

    #[test]
    fn delta_y_maps_to_wheel1_unchanged() {
        // Positive: content down, view toward the top (scroll up).
        assert_eq!(scroll_steps(0, 3), vec![(3, 0)]);
        // Negative: view toward the bottom (scroll down).
        assert_eq!(scroll_steps(0, -2), vec![(-2, 0)]);
    }

    #[test]
    fn delta_x_maps_to_wheel2_unchanged() {
        // Positive: content right, view toward the left.
        assert_eq!(scroll_steps(2, 0), vec![(0, 2)]);
        assert_eq!(scroll_steps(-1, 0), vec![(0, -1)]);
    }

    #[test]
    fn scroll_zero_posts_nothing() {
        assert!(scroll_steps(0, 0).is_empty());
    }

    #[test]
    fn scroll_chunks_sum_exactly_and_respect_the_per_event_cap() {
        for (dx, dy) in [(0, 10), (7, -23), (-1, 1), (100, 3), (0, 4), (0, 5)] {
            let steps = scroll_steps(dx, dy);
            assert_eq!(sums(&steps), (i64::from(dy), i64::from(dx)), "{dx},{dy}");
            assert!(steps.iter().all(|(w1, w2)| {
                u64::from(w1.unsigned_abs()) <= SCROLL_LINES_PER_EVENT
                    && u64::from(w2.unsigned_abs()) <= SCROLL_LINES_PER_EVENT
            }));
        }
        assert_eq!(scroll_steps(0, -10), vec![(-4, 0), (-3, 0), (-3, 0)]);
    }

    #[test]
    fn scroll_handles_extreme_deltas() {
        let steps = scroll_steps(i32::MIN, i32::MAX);
        assert_eq!(steps.len(), 256);
        assert_eq!(sums(&steps), (i64::from(i32::MAX), i64::from(i32::MIN)));
    }

    fn unicode(text: &str) -> TextSegment {
        TextSegment::Unicode(text.encode_utf16().collect())
    }

    #[test]
    fn text_is_chunked_at_the_utf16_limit() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            text_segments(text),
            vec![unicode("abcdefghijklmnopqrst"), unicode("uvwxyz")]
        );
        assert!(text_segments("").is_empty());
    }

    #[test]
    fn surrogate_pairs_are_never_split() {
        // 19 ASCII units then an emoji (2 units): the emoji starts a new event.
        let text = format!("{}😀x", "a".repeat(19));
        let segments = text_segments(&text);
        assert_eq!(segments, vec![unicode(&"a".repeat(19)), unicode("😀x")]);
        for segment in &segments {
            let TextSegment::Unicode(units) = segment else {
                panic!("unexpected key segment");
            };
            assert!(units.len() <= MAX_UTF16_PER_EVENT);
            assert!(String::from_utf16(units).is_ok());
        }
    }

    #[test]
    fn line_breaks_and_tabs_become_keys() {
        assert_eq!(
            text_segments("a\r\nb\nc\rd\te"),
            vec![
                unicode("a"),
                TextSegment::Key(vk::RETURN),
                unicode("b"),
                TextSegment::Key(vk::RETURN),
                unicode("c"),
                TextSegment::Key(vk::RETURN),
                unicode("d"),
                TextSegment::Key(vk::TAB),
                unicode("e"),
            ]
        );
        assert_eq!(
            text_segments("\n\n"),
            vec![TextSegment::Key(vk::RETURN), TextSegment::Key(vk::RETURN)]
        );
        assert_eq!(
            text_segments("\r\r\n"),
            vec![TextSegment::Key(vk::RETURN), TextSegment::Key(vk::RETURN)]
        );
    }

    fn plan(names: &[&str]) -> Vec<KeyStroke> {
        let keys: Vec<String> = names.iter().map(|name| (*name).to_string()).collect();
        chord_plan(&parse_chord(&keys).unwrap())
    }

    #[test]
    fn chord_plan_orders_modifiers_and_flags() {
        let strokes = plan(&["Cmd", "Shift", "Z"]);
        let summary: Vec<_> = strokes
            .iter()
            .map(|stroke| (stroke.kind, stroke.down, stroke.key_code, stroke.flags))
            .collect();
        let both = flag::SHIFT | flag::COMMAND;
        assert_eq!(
            summary,
            vec![
                (StrokeKind::FlagsChanged, true, vk::SHIFT, flag::SHIFT),
                (StrokeKind::FlagsChanged, true, vk::COMMAND, both),
                (StrokeKind::KeyDown, true, vk::ANSI_Z, both),
                (StrokeKind::KeyUp, false, vk::ANSI_Z, both),
                (StrokeKind::FlagsChanged, false, vk::COMMAND, flag::SHIFT),
                (StrokeKind::FlagsChanged, false, vk::SHIFT, 0),
            ]
        );
    }

    #[test]
    fn ctrl_chord_sets_control_flag_not_command() {
        let strokes = plan(&["Ctrl+C"]);
        let key_down = strokes
            .iter()
            .find(|stroke| stroke.kind == StrokeKind::KeyDown)
            .unwrap();
        assert_eq!(key_down.key_code, vk::ANSI_C);
        assert_eq!(key_down.flags, flag::CONTROL);
    }

    #[test]
    fn plain_key_has_no_modifier_events() {
        let strokes = plan(&["Enter"]);
        assert_eq!(strokes.len(), 2);
        assert_eq!(strokes[0].kind, StrokeKind::KeyDown);
        assert_eq!(strokes[1].kind, StrokeKind::KeyUp);
        assert_eq!(strokes[0].flags, 0);
    }

    #[test]
    fn navigation_keys_carry_hardware_flags() {
        let strokes = plan(&["Shift", "Left"]);
        let key_down = &strokes[1];
        assert_eq!(key_down.key_code, vk::LEFT_ARROW);
        assert_eq!(
            key_down.flags,
            flag::SHIFT | flag::NUMERIC_PAD | flag::SECONDARY_FN
        );
        assert_eq!(implied_key_flags(vk::F5), flag::SECONDARY_FN);
        assert_eq!(implied_key_flags(vk::KEYPAD_1), flag::NUMERIC_PAD);
        assert_eq!(implied_key_flags(vk::ANSI_A), 0);
    }

    #[test]
    fn character_targets_post_unicode_with_modifiers() {
        let strokes = plan(&["Cmd", "+"]);
        let key_down = &strokes[1];
        assert_eq!(key_down.kind, StrokeKind::KeyDown);
        assert_eq!(key_down.character, Some('+'));
        assert_eq!(key_down.key_code, 0);
        assert_eq!(key_down.flags, flag::COMMAND);
    }

    #[test]
    fn modifier_only_chord_presses_and_releases() {
        let strokes = chord_plan(&KeyChord {
            modifiers: Modifiers {
                option: true,
                ..Modifiers::default()
            },
            key: None,
        });
        assert_eq!(strokes.len(), 2);
        assert!(strokes[0].down && !strokes[1].down);
        assert_eq!(strokes[0].flags, flag::ALTERNATE);
        assert_eq!(strokes[1].flags, 0);
        assert!(
            strokes
                .iter()
                .all(|stroke| stroke.kind == StrokeKind::FlagsChanged)
        );
    }
}
