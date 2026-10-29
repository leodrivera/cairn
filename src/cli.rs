//! Command line: `cairn [serve]`, `cairn health`, `cairn user ...`, `cairn backup`, `cairn --version`, `cairn --help`.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Health,
    User(UserCommand),
    Backup(PathBuf),
    Version,
    Help,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UserCommand {
    Add {
        name: String,
        email: String,
        admin: bool,
        password: bool,
        scope: String,
    },
    Passwd {
        email: String,
    },
    List,
    Delete {
        email: String,
    },
}

pub const HELP: &str = "\
Usage: cairn [COMMAND]

Commands:
  serve                 Run the web server (the default)
  health                Exit 0 when the local server answers, 1 when not
  user add <name> <email> [--admin] [--password] [--scope /folder]
                        Create a user; --password asks for it without echo
  user passwd <email>   Set or change a password; ends that user's sessions
  user list             List users
  user delete <email>   Delete a user
  backup <file>         Write a consistent copy of the database

Options:
  -h, --help     Show this help
  -V, --version  Show the version
";

const MORE: &str = "Run cairn --help to see the commands.";
const ADD_USAGE: &str =
    "Usage: cairn user add <name> <email> [--admin] [--password] [--scope /folder]";

/// Reads the command line, without the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_args(args);
    let mut command = None;
    while let Some(arg) = parser.next().map_err(|e| format!("{e}. {MORE}"))? {
        let next = match arg {
            Short('h') | Long("help") => return Ok(Command::Help),
            Short('V') | Long("version") => return Ok(Command::Version),
            Value(value) if command.is_none() => match value.to_string_lossy().as_ref() {
                "serve" => Command::Serve,
                "health" => Command::Health,
                "user" => return parse_user(&mut parser),
                "backup" => {
                    let path = parser
                        .value()
                        .map_err(|_| "Usage: cairn backup <file>".to_string())?;
                    Command::Backup(PathBuf::from(path))
                }
                other => return Err(format!("Unknown command: {other}. {MORE}")),
            },
            other => return Err(format!("{}. {MORE}", other.unexpected())),
        };
        command = Some(next);
    }
    Ok(command.unwrap_or(Command::Serve))
}

fn parse_user(parser: &mut lexopt::Parser) -> Result<Command, String> {
    use lexopt::prelude::*;

    let mut words = Vec::new();
    let (mut admin, mut password, mut scope) = (false, false, "/".to_string());
    while let Some(arg) = parser.next().map_err(|e| format!("{e}. {MORE}"))? {
        match arg {
            Long("admin") => admin = true,
            Long("password") => password = true,
            Long("scope") => {
                scope = parser
                    .value()
                    .map_err(|_| ADD_USAGE.to_string())?
                    .to_string_lossy()
                    .into_owned();
            }
            Value(v) => words.push(v.to_string_lossy().into_owned()),
            other => return Err(format!("{}. {MORE}", other.unexpected())),
        }
    }
    let mut words = words.into_iter();
    let sub = words.next().unwrap_or_default();
    let rest: Vec<String> = words.collect();
    let command = match (sub.as_str(), rest.as_slice()) {
        ("add", [name, email]) => UserCommand::Add {
            name: name.clone(),
            email: email.clone(),
            admin,
            password,
            scope,
        },
        ("add", _) => return Err(ADD_USAGE.into()),
        ("passwd", [email]) => UserCommand::Passwd {
            email: email.clone(),
        },
        ("passwd", _) => return Err("Usage: cairn user passwd <email>".into()),
        ("list", []) => UserCommand::List,
        ("delete", [email]) => UserCommand::Delete {
            email: email.clone(),
        },
        ("delete", _) => return Err("Usage: cairn user delete <email>".into()),
        _ => {
            return Err(format!(
                "Unknown user command: {sub}. Use add, passwd, list or delete."
            ));
        }
    };
    Ok(Command::User(command))
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

    fn user(list: &[&str]) -> Result<Command, String> {
        let mut all = vec!["user"];
        all.extend_from_slice(list);
        parse(args(&all))
    }

    #[test]
    fn user_add() {
        assert_eq!(
            user(&["add", "Admin", "admin@example.com", "--admin", "--password"]),
            Ok(Command::User(UserCommand::Add {
                name: "Admin".into(),
                email: "admin@example.com".into(),
                admin: true,
                password: true,
                scope: "/".into(),
            }))
        );
    }

    #[test]
    fn user_add_scope() {
        assert_eq!(
            user(&["add", "Guest", "guest@example.com", "--scope", "/public"]),
            Ok(Command::User(UserCommand::Add {
                name: "Guest".into(),
                email: "guest@example.com".into(),
                admin: false,
                password: false,
                scope: "/public".into(),
            }))
        );
    }

    #[test]
    fn user_passwd() {
        assert_eq!(
            user(&["passwd", "guest@example.com"]),
            Ok(Command::User(UserCommand::Passwd {
                email: "guest@example.com".into()
            }))
        );
    }

    #[test]
    fn user_list() {
        assert_eq!(user(&["list"]), Ok(Command::User(UserCommand::List)));
    }

    #[test]
    fn user_delete() {
        assert_eq!(
            user(&["delete", "guest@example.com"]),
            Ok(Command::User(UserCommand::Delete {
                email: "guest@example.com".into()
            }))
        );
    }

    #[test]
    fn backup() {
        assert_eq!(
            parse(args(&["backup", "/data/backup.db"])),
            Ok(Command::Backup("/data/backup.db".into()))
        );
    }

    #[test]
    fn user_missing_args() {
        assert_eq!(
            user(&["add", "Admin"]),
            Err(
                "Usage: cairn user add <name> <email> [--admin] [--password] [--scope /folder]"
                    .into()
            )
        );
    }

    #[test]
    fn unknown() {
        assert_eq!(
            parse(args(&["frobnicate"])),
            Err("Unknown command: frobnicate. Run cairn --help to see the commands.".into())
        );
    }
}
