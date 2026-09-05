//! Borrowing the terminal for the duration of a run.

use std::io::{self, IsTerminal, Stdout, Write};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, size};
use crossterm::{ExecutableCommand, QueueableCommand};

const FALLBACK_SIZE: (u16, u16) = (80, 24);

/// A terminal switched to its alternate screen.
///
/// Holding one is the proof that the switch happened: there is no way to paint
/// without having entered, and no way to leave without dropping it. The
/// restore lives in [`Drop`] rather than at the end of the run so that an
/// error path, or a panic while unwinding, cannot strand the user in a screen
/// with no cursor.
pub struct Screen {
    output: Stdout,
}

impl Screen {
    /// Takes over the terminal.
    ///
    /// Returns `None` when stdout is not a terminal, where the escape codes
    /// would land in whatever is reading the output, and also when the
    /// terminal refuses the switch. Both mean the same thing to a caller:
    /// print plainly instead.
    pub fn enter() -> Option<Self> {
        let mut output = io::stdout();
        if !output.is_terminal() {
            return None;
        }
        output.execute(EnterAlternateScreen).ok()?;
        output.execute(Hide).ok()?;
        Some(Self { output })
    }

    /// Returns the frame size in columns and rows.
    pub fn size(&self) -> (u16, u16) {
        size().unwrap_or(FALLBACK_SIZE)
    }

    /// Draws one frame, one line per row from the top.
    ///
    /// Each line is cleared to its right as it is written rather than wiping
    /// the whole screen first, which would show a blank frame between paints.
    pub fn paint(&mut self, frame: &[String]) -> io::Result<()> {
        for (row, line) in frame.iter().enumerate().take(u16::MAX as usize) {
            self.output.queue(MoveTo(0, row as u16))?;
            self.output.write_all(line.as_bytes())?;
            self.output.queue(Clear(ClearType::UntilNewLine))?;
        }
        self.output.flush()
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.output.execute(Show);
        let _ = self.output.execute(LeaveAlternateScreen);
    }
}
