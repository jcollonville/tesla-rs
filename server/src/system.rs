// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

/// Warn when the default partition has rows, or the 2035 horizon is close.
pub(crate) fn storage_alert(default_rows: i64, year: i32) -> Option<&'static str> {
    if default_rows > 0 {
        return Some("positions_default is not empty");
    }
    if year >= 2034 {
        return Some("the 2035 partition horizon is near");
    }
    None
}

pub(crate) fn proxy_ip_allowed(allowlist: &[String], ip: &str) -> bool {
    allowlist.iter().any(|entry| entry == ip)
}

/// `pg_dump` invocation for a Postgres URL.
///
/// Host, port, user, and database are arguments. The password is `PGPASSWORD`
/// on this command only, never an argument, so it does not appear in the
/// process list. When the URL has no password, an inherited `PGPASSWORD` is
/// removed.
pub(crate) fn pg_dump_command(
    database_url: &str,
) -> Result<std::process::Command, crate::error::AppError> {
    let target = dump_target(database_url)?;
    let mut command = std::process::Command::new("pg_dump");
    command
        .arg("--no-password")
        .arg("--host")
        .arg(target.host)
        .arg("--port")
        .arg(target.port)
        .arg("--username")
        .arg(target.user)
        .arg("--dbname")
        .arg(target.dbname);
    match target.password {
        Some(password) => {
            command.env("PGPASSWORD", password);
        }
        None => {
            command.env_remove("PGPASSWORD");
        }
    }
    Ok(command)
}

struct DumpTarget {
    host: String,
    port: String,
    user: String,
    dbname: String,
    password: Option<String>,
}

fn dump_target(database_url: &str) -> Result<DumpTarget, crate::error::AppError> {
    let parsed = url::Url::parse(database_url)
        .map_err(|_| crate::error::AppError::message("database URL cannot be used for backup"))?;
    if parsed.scheme() != "postgres" && parsed.scheme() != "postgresql" {
        return Err(crate::error::AppError::message(
            "database URL cannot be used for backup",
        ));
    }
    let host = parsed
        .host_str()
        .map(percent_decode)
        .transpose()?
        .filter(|host| !host.is_empty())
        .ok_or_else(|| crate::error::AppError::message("database URL cannot be used for backup"))?;
    let user = percent_decode(parsed.username())?;
    if user.is_empty() {
        return Err(crate::error::AppError::message(
            "database URL cannot be used for backup",
        ));
    }
    let dbname = percent_decode(parsed.path().trim_matches('/'))?;
    if dbname.is_empty() || dbname.contains('/') {
        return Err(crate::error::AppError::message(
            "database URL cannot be used for backup",
        ));
    }
    let password = parsed
        .password()
        .map(percent_decode)
        .transpose()?
        .filter(|password| !password.is_empty());
    Ok(DumpTarget {
        host,
        port: parsed.port().unwrap_or(5432).to_string(),
        user,
        dbname,
        password,
    })
}

fn percent_decode(value: &str) -> Result<String, crate::error::AppError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3).ok_or_else(|| {
                crate::error::AppError::message("database URL cannot be used for backup")
            })?;
            let hex = std::str::from_utf8(hex).map_err(|_| {
                crate::error::AppError::message("database URL cannot be used for backup")
            })?;
            let byte = u8::from_str_radix(hex, 16).map_err(|_| {
                crate::error::AppError::message("database URL cannot be used for backup")
            })?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| crate::error::AppError::message("database URL cannot be used for backup"))
}

#[cfg(test)]
mod tests {
    use super::{proxy_ip_allowed, storage_alert};

    #[test]
    fn alerts_on_the_default_partition_and_the_horizon() {
        assert_eq!(
            storage_alert(1, 2026),
            Some("positions_default is not empty")
        );
        assert_eq!(
            storage_alert(0, 2034),
            Some("the 2035 partition horizon is near")
        );
        assert_eq!(storage_alert(0, 2026), None);
        assert!(proxy_ip_allowed(&["10.0.0.1".into()], "10.0.0.1"));
        assert!(!proxy_ip_allowed(&["10.0.0.1".into()], "10.0.0.2"));
    }

    #[test]
    fn pg_dump_keeps_the_password_out_of_the_arguments() {
        let command =
            super::pg_dump_command("postgres://tesla:s%40cret@db.internal:5433/tesla_logger")
                .unwrap();
        assert_eq!(command.get_program(), "pg_dump");
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--no-password",
                "--host",
                "db.internal",
                "--port",
                "5433",
                "--username",
                "tesla",
                "--dbname",
                "tesla_logger",
            ]
        );
        let rendered = args.join(" ");
        assert!(!rendered.contains("s@cret"));
        assert!(!rendered.contains("s%40cret"));
        assert!(!rendered.contains("postgres://"));
        let password = command
            .get_envs()
            .find(|(key, _)| *key == "PGPASSWORD")
            .and_then(|(_, value)| value);
        assert_eq!(password, Some(std::ffi::OsStr::new("s@cret")));
    }

    #[test]
    fn pg_dump_drops_an_inherited_password_when_the_url_has_none() {
        let command = super::pg_dump_command("postgresql://tesla@db/tesla_logger").unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--port".to_owned()));
        assert!(args.contains(&"5432".to_owned()));
        let password = command
            .get_envs()
            .find(|(key, _)| *key == "PGPASSWORD")
            .map(|(_, value)| value);
        assert_eq!(password, Some(None));
    }
}
