//! Command line: `cairn [serve]`, `cairn health`, `cairn --version`, `cairn --help`.

use std::ffi::OsString;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Health,
    Version,
    Help,
}

pub const HELP: &str = "\
Usage: cairn [COMMAND]

Commands:
  serve      Run the web server (the default)
  health     Exit 0 when the local server answers, 1 when not

Options:
  -h, --help     Show this help
  -V, --version  Show the version
";

/// Reads the command line, without the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_args(args);
    let mut command = None;
    while let Some(arg) = parser
        .next()
        .map_err(|e| format!("{e}. Run cairn --help to see the commands."))?
    {
        let next = match arg {
            Short('h') | Long("help") => Command::Help,
            Short('V') | Long("version") => Command::Version,
            Value(value) if command.is_none() => match value.to_string_lossy().as_ref() {
                "serve" => Command::Serve,
                "health" => Command::Health,
                other => {
                    return Err(format!(
                        "Unknown command: {other}. Run cairn --help to see the commands."
                    ));
                }
            },
            other => {
                return Err(format!(
                    "{}. Run cairn --help to see the commands.",
                    other.unexpected()
                ));
            }
        };
        if matches!(next, Command::Help | Command::Version) {
            return Ok(next);
        }
        command = Some(next);
    }
    Ok(command.unwrap_or(Command::Serve))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_args_is_serve() {
        assert_eq!(parse(args(&[])), Ok(Command::Serve));
    }

    #[test]
    fn serve() {
        assert_eq!(parse(args(&["serve"])), Ok(Command::Serve));
    }

    #[test]
    fn health() {
        assert_eq!(parse(args(&["health"])), Ok(Command::Health));
    }

    #[test]
    fn version_flag() {
        assert_eq!(parse(args(&["--version"])), Ok(Command::Version));
    }

    #[test]
    fn help_flag() {
        assert_eq!(parse(args(&["-h"])), Ok(Command::Help));
    }

    #[test]
    fn unknown() {
        assert_eq!(
            parse(args(&["frobnicate"])),
            Err("Unknown command: frobnicate. Run cairn --help to see the commands.".into())
        );
    }
}
