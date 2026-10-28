use std::process::ExitCode;

use cairn::cli::{self, Command};
use cairn::config::{Config, vars_from_env};
use cairn::serve;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::Version => {
            println!("cairn {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Help => {
            print!("{}", cli::HELP);
            ExitCode::SUCCESS
        }
        Command::Serve => match Config::from_vars(&vars_from_env()) {
            Ok(config) => serve::run(config),
            Err(errors) => serve::report_invalid(&errors),
        },
        Command::Health => todo!("cairn health"),
    }
}
