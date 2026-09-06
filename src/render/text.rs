//! Laying out the text a frame is built from.

use std::sync::OnceLock;

/// Columns an unknown character, including a space, occupies.
const BLANK_WIDTH: usize = 4;

/// The block font, kept as art rather than as string arrays in this file.
///
/// Editing nine rows of escaped strings in Rust source means counting columns
/// by hand; editing `font.txt` means pasting the art in.
const FONT_SOURCE: &str = include_str!("font.txt");

/// One character drawn as rows of equal width.
type Glyph = Vec<String>;

/// The block font in the form the renderer uses.
pub struct Font {
    glyphs: Vec<(char, Glyph)>,
    height: usize,
}

impl Font {
    /// Reads the font from its art file.
    ///
    /// Rows are padded to the widest row in their own glyph and glyphs to the
    /// tallest in the file, so art that an editor has stripped trailing spaces
    /// from still lines up.
    fn parse(source: &str) -> Self {
        let mut glyphs: Vec<(char, Glyph)> = Vec::new();

        for line in source.lines() {
            match line.strip_prefix('@') {
                Some(declaration) => {
                    if let Some(character) = declaration.trim().chars().next() {
                        glyphs.push((character, Vec::new()));
                    }
                }
                None if line.starts_with('#') => {}
                None => {
                    if let Some((_, rows)) = glyphs.last_mut() {
                        rows.push(line.to_owned());
                    }
                }
            }
        }

        for (_, rows) in &mut glyphs {
            while rows.last().is_some_and(|row| row.trim().is_empty()) {
                rows.pop();
            }
            let width = rows
                .iter()
                .map(|row| row.chars().count())
                .max()
                .unwrap_or(0);
            for row in rows.iter_mut() {
                let padding = width - row.chars().count();
                row.push_str(&" ".repeat(padding));
            }
        }

        // Short glyphs are padded at the top so they sit on the same baseline
        // as the tall ones. A full stop is two rows of art; padding it at the
        // bottom instead would float it level with the top of the digits.
        let height = glyphs.iter().map(|(_, rows)| rows.len()).max().unwrap_or(0);
        for (_, rows) in &mut glyphs {
            let width = rows.first().map_or(0, |row| row.chars().count());
            let mut lifted = vec![" ".repeat(width); height.saturating_sub(rows.len())];
            lifted.append(rows);
            *rows = lifted;
        }

        Self { glyphs, height }
    }

    /// Returns how many rows tall every glyph is.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns whether every character in `text` has art to draw it with.
    ///
    /// A glyph declared in the font file but left empty counts as missing, so
    /// a caller can choose a smaller layout until the art is pasted in rather
    /// than drawing a reading with holes in it.
    pub fn can_draw(&self, text: &str) -> bool {
        text.chars().all(|character| {
            character == ' '
                || self
                    .glyphs
                    .iter()
                    .any(|(known, rows)| *known == character && has_art(rows))
        })
    }

    fn glyph(&self, character: char) -> Glyph {
        self.glyphs
            .iter()
            .find(|(candidate, _)| *candidate == character)
            .map_or_else(
                || vec![" ".repeat(BLANK_WIDTH); self.height],
                |(_, rows)| rows.clone(),
            )
    }
}

fn has_art(rows: &[String]) -> bool {
    rows.iter().any(|row| !row.trim().is_empty())
}

/// Returns the block font, read once.
pub fn font() -> &'static Font {
    static PARSED: OnceLock<Font> = OnceLock::new();
    PARSED.get_or_init(|| Font::parse(FONT_SOURCE))
}

/// Draws `text` in the block font.
///
/// Characters the font has no glyph for render as blank space rather than as
/// a substitute, so a formatting slip shows up as a gap instead of a plausible
/// wrong number.
pub fn enlarge(text: &str) -> Vec<String> {
    let font = font();
    let glyphs: Vec<Glyph> = text
        .chars()
        .map(|character| font.glyph(character))
        .collect();

    (0..font.height())
        .map(|row| {
            glyphs
                .iter()
                .map(|glyph| glyph[row].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Returns how many columns [`enlarge`] would need for `text`.
///
/// Lets a caller choose a layout that fits before drawing one that does not.
pub fn enlarged_width(text: &str) -> usize {
    let font = font();
    let glyphs = text.chars().count();
    let art: usize = text
        .chars()
        .map(|character| font.glyph(character)[0].chars().count())
        .sum();
    art + glyphs.saturating_sub(1)
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

    /// Every character a number can be made of.
    const REQUIRED: &str = "0123456789.";

    #[test]
    fn the_font_file_can_draw_any_number() {
        assert!(
            font().can_draw(REQUIRED),
            "font.txt is missing art for a character a reading can contain, \
             which would draw the number with a hole in it"
        );
    }

    #[test]
    fn an_empty_glyph_counts_as_missing_rather_than_drawable() {
        let font = Font::parse("@ 7\n ███\n░░░\n@ M\n");
        assert!(font.can_draw("7"));
        assert!(
            !font.can_draw("M"),
            "a declared but empty glyph must not pass for art, or the unit \
             would render as a blank space next to the number"
        );
    }

    #[test]
    fn a_short_glyph_sits_on_the_baseline() {
        let font = Font::parse("@ 8\n ██\n ██\n ██\n ██\n@ .\n ██\n");
        let (_, stop) = font
            .glyphs
            .iter()
            .find(|(character, _)| *character == '.')
            .expect("the full stop was declared");

        assert!(
            stop[..stop.len() - 1]
                .iter()
                .all(|row| row.trim().is_empty()),
            "a full stop padded at the bottom would float level with the digits"
        );
        assert_eq!(stop.last().map(String::as_str), Some(" ██"));
    }

    #[test]
    fn every_glyph_is_the_same_height() {
        let font = font();
        for (character, rows) in &font.glyphs {
            assert_eq!(
                rows.len(),
                font.height(),
                "the glyph for {character:?} is {} rows against a font height of {}, \
                 which would misalign every frame",
                rows.len(),
                font.height()
            );
        }
    }

    #[test]
    fn every_row_of_a_glyph_is_the_same_width() {
        for (character, rows) in &font().glyphs {
            let widths: Vec<usize> = rows.iter().map(|row| row.chars().count()).collect();
            assert!(
                widths.windows(2).all(|pair| pair[0] == pair[1]),
                "the glyph for {character:?} has ragged rows: {widths:?}"
            );
        }
    }

    #[test]
    fn enlarged_text_is_one_font_tall() {
        let drawn = enlarge("1.5");
        assert_eq!(drawn.len(), font().height());
        let widths: Vec<usize> = drawn.iter().map(|row| row.chars().count()).collect();
        assert!(widths.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn the_predicted_width_matches_what_is_drawn() {
        for text in ["1", "947.3", "947.3 MBPS", "0.05 KBPS", "8"] {
            assert_eq!(
                enlarged_width(text),
                enlarge(text)[0].chars().count(),
                "width prediction disagrees with the drawing for {text:?}"
            );
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
