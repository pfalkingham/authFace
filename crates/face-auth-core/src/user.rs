//! User identity resolution and validation.
//!
//! Usernames reaching this crate end up as path components under the
//! embeddings directory, so they are validated before use rather than
//! trusted. Lookups go through `getent` so NSS sources (LDAP, SSSD,
//! systemd-homed) resolve the same way the rest of the system sees them.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

/// A resolved system account.
#[derive(Debug, Clone)]
pub struct UserInfo {
    /// Canonical account name as NSS reports it (never a UID string).
    pub name: String,
    pub uid: u32,
    pub home: PathBuf,
}

/// Reject anything that could escape the embeddings directory or confuse
/// path handling. Deliberately stricter than POSIX allows: the accounts
/// this tool is used with are ordinary local logins.
pub fn validate_username(user: &str) -> Result<&str> {
    if user.is_empty() || user.len() > 32 {
        bail!("invalid username: must be 1-32 characters");
    }
    if user == "." || user == ".." {
        bail!("invalid username: reserved path component");
    }
    // Leading '-' would be read as a flag by anything we shell out to.
    if user.starts_with('-') {
        bail!("invalid username: may not start with '-'");
    }
    if !user
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
    {
        bail!("invalid username: only [A-Za-z0-9_.-] permitted");
    }
    Ok(user)
}

fn parse_passwd_line(line: &str) -> Option<UserInfo> {
    // name:passwd:uid:gid:gecos:home:shell
    let parts: Vec<&str> = line.trim_end_matches('\n').split(':').collect();
    if parts.len() < 6 {
        return None;
    }
    Some(UserInfo {
        name: parts[0].to_string(),
        uid: parts[2].parse().ok()?,
        home: PathBuf::from(parts[5]),
    })
}

/// Look up an account by name. `getent passwd` also accepts a UID, which
/// would silently enrol under a directory named after the number rather
/// than the account, so the argument is validated as a name first and the
/// canonical name is taken from the lookup result.
pub fn lookup(user: &str) -> Result<UserInfo> {
    validate_username(user)?;

    let output = Command::new("getent")
        .arg("passwd")
        .arg(user)
        .output()
        .context("failed to run getent")?;
    if !output.status.success() {
        bail!("user '{}' does not exist on this system", user);
    }
    let stdout = String::from_utf8(output.stdout).context("getent returned non-UTF-8 output")?;
    let info = stdout
        .lines()
        .next()
        .and_then(parse_passwd_line)
        .with_context(|| format!("could not parse passwd entry for '{}'", user))?;

    // Guard against `getent passwd 0` resolving to root: the caller asked
    // for a name, so the name it asked for must be what came back.
    if info.name != user {
        bail!(
            "user lookup mismatch: asked for '{}', NSS returned '{}'",
            user,
            info.name
        );
    }
    Ok(info)
}

/// Resolve the human behind this session, not the account the process happens
/// to run as.
///
/// A settings GUI started with `sudo` or `pkexec` runs as root, so `current()`
/// would report `root` and the app would silently enrol and test root's face
/// while PAM authenticates the desktop user — two templates, both "working",
/// neither helping. `sudo` and `pkexec` both record who invoked them, so
/// prefer that and fall back to the real UID.
pub fn invoking() -> Result<UserInfo> {
    for var in ["SUDO_UID", "PKEXEC_UID"] {
        let Ok(raw) = std::env::var(var) else { continue };
        let Ok(uid) = raw.trim().parse::<u32>() else { continue };
        if uid == 0 {
            continue; // root invoking root tells us nothing new
        }
        if let Ok(info) = by_uid(uid) {
            tracing::debug!(%var, uid, user = %info.name, "resolved invoking user");
            return Ok(info);
        }
    }
    current()
}

/// Resolve the account this process is running as, via its real UID.
/// Avoids trusting `$USER`/`$LOGNAME`, which are just environment strings.
pub fn current() -> Result<UserInfo> {
    by_uid(unsafe { libc::getuid() })
}

fn by_uid(uid: u32) -> Result<UserInfo> {
    let output = Command::new("getent")
        .arg("passwd")
        .arg(uid.to_string())
        .output()
        .context("failed to run getent")?;
    if !output.status.success() {
        bail!("no passwd entry for uid {}", uid);
    }
    let stdout = String::from_utf8(output.stdout).context("getent returned non-UTF-8 output")?;
    stdout
        .lines()
        .next()
        .and_then(parse_passwd_line)
        .with_context(|| format!("could not parse passwd entry for uid {}", uid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        for name in ["alice", "bob.smith", "svc-account", "user_1", "a"] {
            assert!(validate_username(name).is_ok(), "rejected {name}");
        }
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for name in [
            "..",
            ".",
            "../root",
            "../../etc/passwd",
            "a/b",
            "a\0b",
            "",
            "-rf",
            "with space",
        ] {
            assert!(validate_username(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn rejects_overlong_names() {
        assert!(validate_username(&"a".repeat(33)).is_err());
        assert!(validate_username(&"a".repeat(32)).is_ok());
    }

    #[test]
    fn invoking_prefers_the_sudo_caller_over_root() {
        // Guards the bug this fixes: `sudo face-auth-gtk` runs as root, so
        // using the process UID enrolled root while PAM authenticated the
        // desktop user. Both halves "worked"; neither helped.
        let me = current().expect("current user");
        if me.uid == 0 {
            return; // meaningless when the test itself runs as root
        }
        std::env::set_var("SUDO_UID", me.uid.to_string());
        let got = invoking().expect("invoking user");
        std::env::remove_var("SUDO_UID");
        assert_eq!(got.name, me.name);
    }

    #[test]
    fn invoking_ignores_a_root_sudo_uid() {
        std::env::set_var("SUDO_UID", "0");
        let got = invoking();
        std::env::remove_var("SUDO_UID");
        // Falls through to the process UID rather than claiming root.
        assert_eq!(got.unwrap().uid, unsafe { libc::getuid() });
    }

    #[test]
    fn invoking_ignores_junk_in_the_environment() {
        std::env::set_var("SUDO_UID", "not-a-number");
        let got = invoking();
        std::env::remove_var("SUDO_UID");
        assert_eq!(got.unwrap().uid, unsafe { libc::getuid() });
    }

    #[test]
    fn parses_passwd_line() {
        let info = parse_passwd_line("alice:x:1000:1000:Alice:/home/alice:/bin/bash").unwrap();
        assert_eq!(info.name, "alice");
        assert_eq!(info.uid, 1000);
        assert_eq!(info.home, PathBuf::from("/home/alice"));
    }

    #[test]
    fn rejects_truncated_passwd_line() {
        assert!(parse_passwd_line("alice:x:1000").is_none());
    }
}
