//! The `apex-control` command-line surface beyond serving the control plane.
//!
//! `apex-control hash-password` seeds the operator file hermetically and is a
//! genuine operational feature: it reads one password line from standard
//! input, hashes it through the crate's argon2id module
//! ([`crate::passwords`], the same parameters the operators loader verifies
//! against) with a fresh random salt, and writes exactly one PHC string —
//! and nothing else — to standard output. The password is never echoed,
//! logged, or included in any error message.
//!
//! Usage: `apex-control hash-password` (password on stdin, PHC on stdout).
//! The line terminator (`\n` or `\r\n`) is stripped; every other byte of the
//! line is part of the password, so leading and trailing spaces are
//! significant. Empty or whitespace-only input is refused with exit code 1
//! and a message on stderr.

use std::io::{BufRead, Write};

use rand::RngCore;

use crate::passwords::{self, PasswordError};

/// Why the command-line entry point refused to proceed.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Standard input ended without a line, or the line was empty or
    /// whitespace-only. A blank input would silently mint a password nobody
    /// can know, so it is refused instead of hashed.
    #[error("empty password: refusing to hash blank input")]
    EmptyPassword,
    /// Reading standard input failed (including non-UTF-8 bytes).
    #[error("could not read the password: {0}")]
    Read(std::io::Error),
    /// Writing the PHC string failed.
    #[error("could not write the hash: {0}")]
    Write(std::io::Error),
    /// The argon2id hash could not be built.
    #[error("hashing failed: {0}")]
    Hash(PasswordError),
}

/// Read one line from `input`, hash it with argon2id through
/// [`passwords::hash_with_salt`], and write exactly one PHC string plus a
/// newline to `output`. Nothing else is written; the password never appears
/// in the output, the error, or any log.
pub fn hash_password_line<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
) -> Result<(), CliError> {
    let mut line = String::new();
    let read = input.read_line(&mut line).map_err(CliError::Read)?;
    let password = line.trim_end_matches(['\r', '\n']);
    if read == 0 || password.trim().is_empty() {
        return Err(CliError::EmptyPassword);
    }
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let phc = passwords::hash_with_salt(password, &salt).map_err(CliError::Hash)?;
    writeln!(output, "{phc}").map_err(CliError::Write)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(input: &str) -> Result<String, CliError> {
        let mut out = Vec::new();
        hash_password_line(&mut Cursor::new(input), &mut out)?;
        Ok(String::from_utf8(out).expect("stdout is a PHC string"))
    }

    #[test]
    fn the_hash_verifies_through_the_crate_verifier_and_stdout_is_one_line() {
        let out = run("correct horse battery\n").unwrap();
        assert_eq!(out.matches('\n').count(), 1);
        let phc = out.trim_end();
        // `$argon2id$v=19$m=...,t=...,p=...$salt$hash`
        assert_eq!(phc.matches('$').count(), 5);
        assert!(passwords::phc_is_argon2id(phc).is_ok());
        assert!(passwords::verify("correct horse battery", phc));
        assert!(!passwords::verify("wrong password", phc));
        assert!(!out.contains("correct horse battery"), "password echoed");
    }

    #[test]
    fn every_invocation_uses_a_fresh_salt() {
        let first = run("pw\n").unwrap();
        let second = run("pw\n").unwrap();
        assert_ne!(first, second);
        assert!(passwords::verify("pw", first.trim_end()));
        assert!(passwords::verify("pw", second.trim_end()));
    }

    #[test]
    fn empty_and_whitespace_only_input_is_refused() {
        assert!(matches!(run("").unwrap_err(), CliError::EmptyPassword));
        assert!(matches!(run("\n").unwrap_err(), CliError::EmptyPassword));
        assert!(matches!(
            run(" \t \n").unwrap_err(),
            CliError::EmptyPassword
        ));
        let rendered = run("\n").unwrap_err().to_string();
        assert_eq!(rendered, "empty password: refusing to hash blank input");
    }

    #[test]
    fn crlf_terminators_are_stripped_but_inner_spaces_are_part_of_the_password() {
        let crlf = run("pw\r\n").unwrap();
        assert!(passwords::verify("pw", crlf.trim_end()));
        let spaced = run("  pw  \n").unwrap();
        assert!(passwords::verify("  pw  ", spaced.trim_end()));
        assert!(!passwords::verify("pw", spaced.trim_end()));
    }
}
