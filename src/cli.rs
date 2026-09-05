//! The command line entry point.

use std::error::Error as _;
use std::process::ExitCode;

use clap::Parser;

use crate::error::{Error, Result};
use crate::render::live::Live;
use crate::render::screen::Screen;
use crate::render::summary::{Detail, summary};
use crate::report::Report;
use crate::session::{self, Event, Scope};

/// Drawn above the help text.
///
/// A raw string so the backslashes in the art are not read as escapes.
const BANNER: &str = r#"
__________             _____     __________________
___  ____/_____ _________  /_    __  ____/__  /__(_)
__  /_   _  __ `/_  ___/  __/    _  /    __  /__  /
_  __/   / /_/ /_(__  )/ /_      / /___  _  / _  /
/_/      \__,_/ /____/ \__/      \____/  /_/  /_/
"#;

/// Measure the speed of an internet connection.
#[derive(Debug, Parser)]
#[command(name = "fast", version, about, long_about = None, before_help = BANNER)]
pub struct Cli {
    /// Skip the upload measurement
    #[arg(long)]
    download_only: bool,

    /// Print the result as JSON, which always includes every field
    #[arg(long)]
    json: bool,

    /// Also show which server answered and how long the run took
    #[arg(short, long)]
    verbose: bool,
}

/// Where the result is written and in what shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Aligned text for a person, with a live display when attached to a terminal.
    Human,
    /// One JSON object for a script.
    Json,
}

impl Cli {
    /// Returns which phases to run.
    pub fn scope(&self) -> Scope {
        if self.download_only {
            Scope::DownloadOnly
        } else {
            Scope::Full
        }
    }

    /// Returns how much of the result to show.
    pub fn detail(&self) -> Detail {
        if self.verbose {
            Detail::Verbose
        } else {
            Detail::Plain
        }
    }

    /// Returns the shape of the output.
    pub fn output(&self) -> Output {
        if self.json {
            Output::Json
        } else {
            Output::Human
        }
    }
}

/// How the process reports its outcome to a shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The measurement completed.
    Success,
    /// The measurement could not be taken.
    Failed,
    /// The user interrupted the run.
    Interrupted,
}

impl Exit {
    fn for_error(error: &Error) -> Self {
        match error {
            Error::Cancelled => Self::Interrupted,
            _ => Self::Failed,
        }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        match exit {
            Exit::Success => Self::from(0),
            Exit::Failed => Self::from(1),
            // The shell convention for a process killed by signal N.
            Exit::Interrupted => Self::from(128 + 2),
        }
    }
}

/// Runs a measurement and prints it.
pub async fn run(cli: Cli) -> Exit {
    match measure_and_print(&cli).await {
        Ok(()) => Exit::Success,
        Err(error) => {
            report_error(&error);
            Exit::for_error(&error)
        }
    }
}

async fn measure_and_print(cli: &Cli) -> Result<()> {
    let report = measure(cli).await?;
    match cli.output() {
        Output::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        Output::Human => println!("{}", summary(&report, cli.detail()).join("\n")),
    }
    Ok(())
}

/// Takes the measurement, drawing it live when there is a terminal to draw on.
///
/// The display is dropped before this returns, so the terminal is restored
/// before the result is printed and the result survives in the scrollback
/// rather than vanishing with the alternate screen.
async fn measure(cli: &Cli) -> Result<Report> {
    let mut display = match cli.output() {
        Output::Json => None,
        Output::Human => Screen::enter().map(Live::new),
    };

    match &mut display {
        Some(live) => until_interrupted(cli.scope(), &mut |event| live.show(event)).await,
        None => until_interrupted(cli.scope(), &mut |_| {}).await,
    }
}

/// Races the run against the user pressing ctrl-c.
///
/// Losing the race drops the run, and dropping it cancels every request still
/// in flight, so there is nothing else to unwind.
async fn until_interrupted(scope: Scope, observe: &mut dyn FnMut(Event)) -> Result<Report> {
    tokio::select! {
        outcome = session::run(scope, observe) => outcome,
        _ = tokio::signal::ctrl_c() => Err(Error::Cancelled),
    }
}

fn report_error(error: &Error) {
    eprintln!("error: {error}");
    let mut cause = error.source();
    while let Some(reason) = cause {
        eprintln!("  caused by: {reason}");
        cause = reason.source();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(arguments: &[&str]) -> Cli {
        Cli::parse_from(std::iter::once("fast").chain(arguments.iter().copied()))
    }

    #[test]
    fn the_argument_definitions_are_sound() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_bare_invocation_measures_everything_for_a_person() {
        let cli = parse(&[]);
        assert_eq!(cli.scope(), Scope::Full);
        assert_eq!(cli.detail(), Detail::Plain);
        assert_eq!(cli.output(), Output::Human);
    }

    #[test]
    fn each_flag_maps_to_its_own_choice() {
        assert_eq!(parse(&["--download-only"]).scope(), Scope::DownloadOnly);
        assert_eq!(parse(&["--verbose"]).detail(), Detail::Verbose);
        assert_eq!(parse(&["-v"]).detail(), Detail::Verbose);
        assert_eq!(parse(&["--json"]).output(), Output::Json);
    }

    #[test]
    fn an_unknown_flag_is_rejected() {
        assert!(Cli::try_parse_from(["fast", "--turbo"]).is_err());
    }

    #[test]
    fn an_interruption_is_distinguishable_from_a_failure() {
        assert_eq!(Exit::for_error(&Error::Cancelled), Exit::Interrupted);
        assert_eq!(
            Exit::for_error(&Error::UnexpectedStatus { status: 503 }),
            Exit::Failed
        );
    }
}
