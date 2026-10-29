//! `cairn user ...` and `cairn backup`: managing users from the command line, mainly to create the first admin.

use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::auth::password::{POLICY, hash_blocking, policy_ok};
use crate::cli::UserCommand;
use crate::config::Config;
use crate::db::Db;
use crate::users::{self, NewUser, Perms};

pub fn db_path(config: &Config) -> PathBuf {
    config.data.join("cairn.db")
}

pub fn run_user(config: &Config, cmd: UserCommand) -> ExitCode {
    match user(config, cmd) {
        Ok(line) => {
            if !line.is_empty() {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

pub fn run_backup(config: &Config, dest: &Path) -> ExitCode {
    let result = Db::open(&db_path(config)).and_then(|db| db.backup_to(dest));
    match result {
        Ok(()) => {
            println!("Wrote a copy of the database to {}.", dest.display());
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn user(config: &Config, cmd: UserCommand) -> Result<String, String> {
    let db = Db::open(&db_path(config))?;
    let find = |email: &str| -> Result<users::User, String> {
        let email = users::normalize_email(email).ok_or("Enter a valid e-mail address.")?;
        match users::by_email(&db, &email).map_err(|e| e.message())? {
            Some((u, _)) => Ok(u),
            None => Err(format!("No user with the e-mail {email}.")),
        }
    };
    match cmd {
        UserCommand::Add {
            name,
            email,
            admin,
            password,
            scope,
        } => {
            let sso = config.auth.oauth.is_some() || !config.auth.oidc.is_empty();
            if !password && !sso {
                return Err("This server signs in with passwords only, so the user needs one. Add --password.".into());
            }
            let hash = if password {
                Some(hash_blocking(&read_password()?))
            } else {
                None
            };
            let perms = Perms {
                read: true,
                write: admin,
                share: admin,
                admin,
            };
            let new = NewUser {
                email,
                name,
                password_hash: hash,
                scope,
                perms,
            };
            let u = users::create(&db, new, now()).map_err(|e| e.message())?;
            let role = if u.perms.admin { "admin" } else { "user" };
            Ok(format!("Created {} ({role}, scope {}).", u.email, u.scope))
        }
        UserCommand::Passwd { email } => {
            let u = find(&email)?;
            let hash = hash_blocking(&read_password()?);
            users::set_password(&db, u.id, Some(hash)).map_err(|e| e.message())?;
            crate::auth::session::delete_for_user(&db, u.id, None).map_err(|e| e.to_string())?;
            Ok(format!(
                "Password changed for {}. Their sessions have ended.",
                u.email
            ))
        }
        UserCommand::List => {
            let lines: Vec<String> = users::list(&db)
                .map_err(|e| e.message())?
                .into_iter()
                .map(|u| {
                    let p = u.perms;
                    let flags: String = [
                        (p.read, 'r'),
                        (p.write, 'w'),
                        (p.share, 's'),
                        (p.admin, 'a'),
                    ]
                    .iter()
                    .map(|&(on, c)| if on { c } else { '-' })
                    .collect();
                    let pw = if u.has_password {
                        "password"
                    } else {
                        "no password"
                    };
                    format!("{}\t{}\t{}\t{flags}\t{pw}", u.email, u.name, u.scope)
                })
                .collect();
            Ok(lines.join("\n"))
        }
        UserCommand::Delete { email } => {
            let u = find(&email)?;
            users::delete(&db, u.id).map_err(|e| e.message())?;
            Ok(format!("Deleted {}.", u.email))
        }
    }
}

/// From the terminal without echo, or one line from stdin when piped. Never from an argument: those show up in
/// `ps` and in shell history.
fn read_password() -> Result<String, String> {
    let password = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Password: ")
            .map_err(|e| format!("Cannot read the password ({e})."))?
    } else {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| format!("Cannot read the password ({e})."))?;
        line.strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(&line)
            .to_string()
    };
    if !policy_ok(&password) {
        return Err(POLICY.into());
    }
    Ok(password)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
