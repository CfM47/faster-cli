//! Drawing a run while it happens.

use crate::probe::transfer::{Progress, Stage};
use crate::render::screen::Screen;
use crate::render::text;
use crate::session::Event;

/// Paints a run onto a terminal it has taken over.
pub struct Live {
    screen: Screen,
}

impl Live {
    /// Draws onto `screen` until dropped, restoring the terminal with it.
    pub fn new(screen: Screen) -> Self {
        Self { screen }
    }

    /// Redraws the frame for `event`.
    ///
    /// A terminal that has gone away mid-run is ignored rather than reported:
    /// losing the display is no reason to abandon a measurement that is
    /// otherwise still running.
    pub fn show(&mut self, event: Event) {
        let (width, height) = self.screen.size();
        let block = match event {
            Event::Probing => vec!["Measuring latency".to_owned()],
            Event::Transferring(progress) => frame(progress, width),
        };
        let _ = self.screen.paint(&text::center(&block, width, height));
    }
}

fn frame(progress: Progress, width: u16) -> Vec<String> {
    let mut block = vec![progress.direction.to_uppercase(), String::new()];

    match progress.stage {
        // Showing the unsettled figure would mean displaying a number several
        // times the real one and then walking it back as the window fills.
        Stage::Warmup => block.push("warming up".to_owned()),
        Stage::Measuring => block.extend(reading(progress, width)),
    }

    block.push(String::new());
    block.push(format!("{:.0}s", progress.elapsed.as_secs_f64()));
    block
}

/// Draws the rate as large as the screen and the font allow.
///
/// Three layouts, each the fallback for the one above it: the unit beside the
/// number, the unit as small text under it, and both as small text. A unit set
/// beside the number nearly doubles its width, which overruns a normal
/// terminal, and a narrow window cannot hold even the number. Wrapping the art
/// would look like a fault, so the display gives up size instead.
fn reading(progress: Progress, width: u16) -> Vec<String> {
    let (value, magnitude) = progress.rate.scaled();
    let number = format!("{value:.*}", magnitude.decimals());
    let unit = magnitude.symbol();
    let beside = format!("{number} {}", unit.to_uppercase());
    let (font, width) = (text::font(), width as usize);

    if font.can_draw(&beside) && text::enlarged_width(&beside) <= width {
        return text::enlarge(&beside);
    }

    if font.can_draw(&number) && text::enlarged_width(&number) <= width {
        let mut drawn = text::enlarge(&number);
        drawn.push(String::new());
        drawn.push(unit.to_owned());
        return drawn;
    }

    vec![format!("{number} {unit}")]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::{Bitrate, Bytes};
    use std::time::Duration;

    fn progress(stage: Stage) -> Progress {
        Progress {
            direction: "Download",
            stage,
            transferred: Bytes::new(125_000_000),
            elapsed: Duration::from_secs(5),
            rate: Bitrate::from_transfer(Bytes::new(125_000_000), Duration::from_secs(5)),
        }
    }

    #[test]
    fn a_settled_sample_is_drawn_large() {
        let drawn = frame(progress(Stage::Measuring), 80);
        assert_eq!(drawn[0], "DOWNLOAD");
        assert!(
            drawn.iter().any(|line| line.contains('█')),
            "a settled reading is shown as block digits"
        );
    }

    #[test]
    fn a_warming_sample_shows_no_number_at_all() {
        let drawn = frame(progress(Stage::Warmup), 80);
        assert!(drawn.contains(&"warming up".to_owned()));
        assert!(
            !drawn.iter().any(|line| line.contains('█')),
            "an unsettled rate must not be shown as a reading"
        );
    }

    #[test]
    fn the_unit_drops_below_the_number_when_it_will_not_fit_beside_it() {
        let drawn = frame(progress(Stage::Measuring), 80);
        assert!(
            drawn.contains(&"Mbps".to_owned()),
            "a unit that cannot be set beside the number is still named"
        );
        assert!(drawn.iter().any(|line| line.contains('█')));
    }

    #[test]
    fn a_window_too_narrow_for_the_art_still_shows_the_reading() {
        let drawn = frame(progress(Stage::Measuring), 20);
        assert!(
            drawn.iter().any(|line| line.contains("200.0 Mbps")),
            "giving up the block digits must not mean giving up the number"
        );
    }

    #[test]
    fn no_frame_is_wider_than_the_screen_it_was_built_for() {
        for width in [20u16, 40, 80, 200] {
            let drawn = frame(progress(Stage::Measuring), width);
            let widest = drawn.iter().map(|line| line.chars().count()).max().unwrap();
            assert!(
                widest <= width as usize,
                "a {width} column screen was given a {widest} column frame, which wraps"
            );
        }
    }
}
