//! Laying out the text a frame is built from.

const GLYPH_HEIGHT: usize = 9;
const BLANK: Glyph = ["         "; GLYPH_HEIGHT];

/// One character drawn nine rows tall.
type Glyph = [&'static str; GLYPH_HEIGHT];

#[rustfmt::skip]
const GLYPHS: [(char, Glyph); 11] = [
    ('0', ["  █████  ", " ██   ██ ", "██     ██", "██     ██", "██     ██", "██     ██", "██     ██", " ██   ██ ", "  █████  "]),
    ('1', ["    ██   ", "  ████   ", "    ██   ", "    ██   ", "    ██   ", "    ██   ", "    ██   ", "    ██   ", " ███████ "]),
    ('2', [" ██████  ", "██    ██ ", "      ██ ", "     ██  ", "   ███   ", " ██      ", "██       ", "██       ", "████████ "]),
    ('3', ["███████  ", "      ██ ", "      ██ ", "   ████  ", "      ██ ", "      ██ ", "      ██ ", "      ██ ", "███████  "]),
    ('4', ["██    ██ ", "██    ██ ", "██    ██ ", "██    ██ ", "████████ ", "      ██ ", "      ██ ", "      ██ ", "      ██ "]),
    ('5', ["████████ ", "██       ", "██       ", "███████  ", "      ██ ", "      ██ ", "      ██ ", "      ██ ", "███████  "]),
    ('6', ["  █████  ", " ██      ", "██       ", "██       ", "███████  ", "██    ██ ", "██    ██ ", "██    ██ ", " ██████  "]),
    ('7', ["████████ ", "      ██ ", "     ██  ", "     ██  ", "    ██   ", "   ██    ", "   ██    ", "  ██     ", "  ██     "]),
    ('8', ["  █████  ", " ██   ██ ", " ██   ██ ", "  █████  ", " ██   ██ ", "██     ██", "██     ██", " ██   ██ ", "  █████  "]),
    ('9', ["  █████  ", " ██   ██ ", "██     ██", "██     ██", " ███████ ", "      ██ ", "      ██ ", "     ██  ", " █████   "]),
    ('.', ["         ", "         ", "         ", "         ", "         ", "         ", "         ", "   ██    ", "   ██    "]),
];

/// Draws `text` as block digits, nine rows tall.
///
/// Characters with no glyph render as blank space rather than as a substitute
/// digit, so a formatting slip shows up as a gap instead of a plausible wrong
/// number.
pub fn enlarge(text: &str) -> Vec<String> {
    let glyphs: Vec<Glyph> = text.chars().map(glyph).collect();
    (0..GLYPH_HEIGHT)
        .map(|row| {
            glyphs
                .iter()
                .map(|glyph| glyph[row])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn glyph(character: char) -> Glyph {
    GLYPHS
        .iter()
        .find(|(candidate, _)| *candidate == character)
        .map_or(BLANK, |(_, glyph)| *glyph)
}

/// Centres `lines` in a `width` by `height` frame.
///
/// Always returns exactly `height` rows so a caller can paint the frame
/// without tracking what the previous one left behind.
pub fn center(lines: &[String], width: u16, height: u16) -> Vec<String> {
    let (width, height) = (width as usize, height as usize);
    let top = height.saturating_sub(lines.len()) / 2;

    let mut frame: Vec<String> = vec![String::new(); top];
    frame.extend(
        lines
            .iter()
            .take(height - top)
            .map(|line| center_line(line, width)),
    );
    frame.resize(height, String::new());
    frame
}

/// Pads a line so it sits centred, measuring in characters rather than bytes.
fn center_line(line: &str, width: usize) -> String {
    let visible = line.chars().count();
    if visible == 0 {
        return String::new();
    }
    let left = width.saturating_sub(visible) / 2;
    format!("{}{line}", " ".repeat(left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enlarged_text_is_nine_rows_tall() {
        let drawn = enlarge("1.5");
        assert_eq!(drawn.len(), GLYPH_HEIGHT);
        for row in &drawn {
            assert_eq!(row.chars().count(), 9 * 3 + 2);
        }
    }

    #[test]
    fn a_character_without_a_glyph_leaves_a_gap() {
        let drawn = enlarge("?");
        assert!(
            drawn.iter().all(|row| row.trim().is_empty()),
            "an unknown character must not render as some other digit"
        );
    }

    #[test]
    fn a_frame_always_fills_its_height() {
        let lines = vec!["one".to_owned(), "two".to_owned()];
        let frame = center(&lines, 20, 10);
        assert_eq!(frame.len(), 10);
        assert_eq!(frame[4], format!("{}one", " ".repeat(8)));
        assert_eq!(frame[5], format!("{}two", " ".repeat(8)));
    }

    #[test]
    fn content_taller_than_the_frame_is_cut_to_fit() {
        let lines: Vec<String> = (0..40).map(|index| index.to_string()).collect();
        assert_eq!(center(&lines, 20, 5).len(), 5);
    }

    #[test]
    fn centring_counts_characters_not_bytes() {
        // Each block is three bytes; measuring in bytes would push this left.
        let frame = center(&["███".to_owned()], 11, 1);
        assert_eq!(frame[0], "    ███");
    }

    #[test]
    fn a_blank_line_stays_blank_rather_than_becoming_padding() {
        let frame = center(&[String::new()], 60, 1);
        assert_eq!(frame[0], "");
    }

    #[test]
    fn a_line_wider_than_the_frame_is_left_alone() {
        let frame = center(&["a very long line".to_owned()], 4, 1);
        assert_eq!(frame[0], "a very long line");
    }
}
