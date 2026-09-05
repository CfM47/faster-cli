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
            Event::Transferring(progress) => frame(progress),
        };
        let _ = self.screen.paint(&text::center(&block, width, height));
    }
}

fn frame(progress: Progress) -> Vec<String> {
    let mut block = vec![progress.direction.to_uppercase(), String::new()];

    match progress.stage {
        // Showing the unsettled figure would mean displaying a number several
        // times the real one and then walking it back as the window fills.
        Stage::Warmup => block.push("warming up".to_owned()),
        Stage::Measuring => {
            let (value, magnitude) = progress.rate.scaled();
            block.extend(text::enlarge(&format!("{value:.*}", magnitude.decimals())));
            block.push(String::new());
            block.push(magnitude.symbol().to_owned());
        }
    }

    block.push(String::new());
    block.push(format!("{:.0}s", progress.elapsed.as_secs_f64()));
    block
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
        let drawn = frame(progress(Stage::Measuring));
        assert_eq!(drawn[0], "DOWNLOAD");
        assert!(drawn.contains(&"Mbps".to_owned()));
        assert!(
            drawn.iter().any(|line| line.contains('█')),
            "a settled reading is shown as block digits"
        );
    }

    #[test]
    fn a_warming_sample_shows_no_number_at_all() {
        let drawn = frame(progress(Stage::Warmup));
        assert!(drawn.contains(&"warming up".to_owned()));
        assert!(
            !drawn.iter().any(|line| line.contains('█')),
            "an unsettled rate must not be shown as a reading"
        );
    }
}
