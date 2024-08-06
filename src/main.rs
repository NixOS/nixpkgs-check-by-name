mod eval;
mod nix_file;
mod nixpkgs_problem;
mod ratchet;
mod references;
mod status;
mod structure;
mod utils;
mod validation;

use anyhow::Context as _;
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{panic, thread};

use crate::nix_file::NixFileStore;
use crate::status::{ColoredStatus, Status};
use crate::structure::check_structure;
use crate::validation::Validation::Failure;
use crate::validation::Validation::Success;

/// Program to check the validity of pkgs/by-name
///
/// This CLI interface may be changed over time if the CI workflow making use of it is adjusted to
/// deal with the change appropriately.
///
/// Exit code:
/// - `0`: If the validation is successful
/// - `1`: If the validation is not successful
/// - `2`: If an unexpected I/O error occurs
///
/// Standard error:
/// - Informative messages
/// - Detected problems if validation is not successful
#[derive(Parser, Debug)]
#[command(about, verbatim_doc_comment)]
pub struct Args {
    /// Path to the main Nixpkgs to check. For PRs, set this to a checkout of the PR branch.
    nixpkgs: PathBuf,

    /// Path to the base Nixpkgs to run ratchet checks against.
    /// For PRs, set this to a checkout of the PRs base branch.
    #[arg(long)]
    base: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let status: ColoredStatus = process(args.base, args.nixpkgs).into();
    eprintln!("{status}");
    status.into()
}

/// Does the actual work. This is the abstraction used both by `main` and the tests.
///
/// # Arguments
/// - `base_nixpkgs`: Path to the base Nixpkgs to run ratchet checks against.
/// - `main_nixpkgs`: Path to the main Nixpkgs to check.
fn process(base_nixpkgs: PathBuf, main_nixpkgs: PathBuf) -> Status {
    // Very easy to parallelise this, since both operations are totally independent of each other.
    let base_thread = thread::spawn(move || check_nixpkgs(&base_nixpkgs));
    let main_result = match check_nixpkgs(&main_nixpkgs) {
        Ok(result) => result,
        Err(error) => {
            return error.into();
        }
    };

    let base_result = match base_thread.join() {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return error.into();
        }
        Err(e) => panic::resume_unwind(e),
    };

    match (base_result, main_result) {
        (Failure(..), Failure(errors)) => Status::BranchStillBroken(errors),
        (Success(..), Failure(errors)) => Status::ProblemsIntroduced(errors),
        (Failure(..), Success(..)) => Status::BranchHealed,
        (Success(base), Success(main)) => {
            // Both base and main branch succeed. Check ratchet state between them...
            match ratchet::Nixpkgs::compare(base, main) {
                Failure(errors) => Status::DiscouragedPatternedIntroduced(errors),
                Success(..) => Status::ValidatedSuccessfully,
            }
        }
    }
}

/// Checks whether the pkgs/by-name structure in Nixpkgs is valid.
///
/// This does not include ratchet checks, see ../README.md#ratchet-checks
/// Instead a `ratchet::Nixpkgs` value is returned, whose `compare` method allows performing the
/// ratchet check against another result.
fn check_nixpkgs(nixpkgs_path: &Path) -> validation::Result<ratchet::Nixpkgs> {
    let nixpkgs_path = nixpkgs_path.canonicalize().with_context(|| {
        format!(
            "Nixpkgs path {} could not be resolved",
            nixpkgs_path.display()
        )
    })?;

    if !nixpkgs_path.join(utils::BASE_SUBPATH).exists() {
        // No pkgs/by-name directory, always valid
        return Ok(Success(ratchet::Nixpkgs::default()));
    }

    let mut nix_file_store = NixFileStore::default();
    let structure = check_structure(&nixpkgs_path, &mut nix_file_store)?;

    // Only if we could successfully parse the structure, we do the evaluation checks
    let result = structure.result_map(|package_names| {
        eval::check_values(&nixpkgs_path, &mut nix_file_store, package_names)
    })?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use std::fs;
    use std::path::Path;
    use tempfile::{tempdir_in, TempDir};

    use super::{process, utils::BASE_SUBPATH};

    #[test]
    fn tests_dir() -> anyhow::Result<()> {
        for entry in Path::new("tests").read_dir()? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            if !path.is_dir() {
                continue;
            }

            let expected_errors = fs::read_to_string(path.join("expected"))
                .with_context(|| format!("No expected file for test {name}"))?;

            test_nixpkgs(&name, &path, &expected_errors);
        }
        Ok(())
    }

    // tempfile::tempdir needs to be wrapped in temp_env lock
    // because it accesses TMPDIR environment variable.
    pub fn tempdir() -> anyhow::Result<TempDir> {
        let empty_list: [(&str, Option<&str>); 0] = [];
        Ok(temp_env::with_vars(empty_list, tempfile::tempdir)?)
    }

    // We cannot check case-conflicting files into Nixpkgs (the channel would fail to build),
    // so we generate the case-conflicting file instead.
    #[test]
    fn test_case_sensitive() -> anyhow::Result<()> {
        let temp_nixpkgs = tempdir()?;
        let path = temp_nixpkgs.path();

        if is_case_insensitive_fs(path)? {
            eprintln!("We're on a case-insensitive filesystem, skipping case-sensitivity test");
            return Ok(());
        }

        let base = path.join(BASE_SUBPATH);

        fs::create_dir_all(base.join("fo/foo"))?;
        fs::write(base.join("fo/foo/package.nix"), "{ someDrv }: someDrv")?;

        fs::create_dir_all(base.join("fo/foO"))?;
        fs::write(base.join("fo/foO/package.nix"), "{ someDrv }: someDrv")?;

        test_nixpkgs(
            "case_sensitive",
            path,
            "pkgs/by-name/fo: Duplicate case-sensitive package directories \"foO\" and \"foo\".\n\
            This PR introduces the problems listed above. Please fix them before merging, \
            otherwise the base branch would break.\n",
        );
        Ok(())
    }

    /// Tests symlinked temporary directories.
    ///
    /// This is needed because on Darwin, `/tmp` is a symlink to `/private/tmp`, and Nix's
    /// restrict-eval doesn't also allow access to the canonical path when you allow the
    /// non-canonical one.
    ///
    /// The error if we didn't do this would look like this:
    /// error: access to canonical path
    /// '/private/var/folders/[...]/.tmpFbcNO0' is forbidden in restricted mode
    #[test]
    fn test_symlinked_tmpdir() -> anyhow::Result<()> {
        // Create a directory with two entries:
        // - actual (dir)
        // - symlinked -> actual (symlink)
        let temp_root = tempdir()?;
        fs::create_dir(temp_root.path().join("actual"))?;
        std::os::unix::fs::symlink("actual", temp_root.path().join("symlinked"))?;
        let tmpdir = temp_root.path().join("symlinked");

        temp_env::with_var("TMPDIR", Some(&tmpdir), || {
            test_nixpkgs(
                "symlinked_tmpdir",
                Path::new("tests/success"),
                "Validated successfully\n",
            )
        });
        Ok(())
    }

    fn test_nixpkgs(name: &str, path: &Path, expected_errors: &str) {
        // Match the expected errors almost verbatim -- `@REDACTED@` turns into `.*`.
        let pattern = format!(
            "^{}$",
            regex::escape(expected_errors).replace("@REDACTED@", ".*")
        );

        let expected_errors_regex = regex::RegexBuilder::new(&pattern)
            .dot_matches_new_line(true)
            .build()
            .expect("valid regex");

        let path = path.to_owned();
        let base_path = path.join("base");
        let base_nixpkgs = if base_path.exists() {
            base_path
        } else {
            Path::new("tests/empty-base").to_owned()
        };

        // Empty dir, needed so that no warnings are printed when testing older Nix versions
        // that don't recognise certain newer keys in nix.conf
        let nix_conf_dir = tempdir().expect("directory");
        let nix_conf_dir = nix_conf_dir.path().as_os_str();

        let status = temp_env::with_var("NIX_CONF_DIR", Some(nix_conf_dir), || {
            process(base_nixpkgs, path)
        });

        let actual_errors = format!("{status}\n");

        if !expected_errors_regex.is_match(&actual_errors) {
            panic!(
                "Failed test case {name}, expected these errors:\n=======\n{}\n=======\n\
                but got these:\n=======\n{}\n=======",
                expected_errors, actual_errors
            );
        }
    }

    /// Check whether a path is in a case-insensitive filesystem
    fn is_case_insensitive_fs(path: &Path) -> anyhow::Result<bool> {
        let dir = tempdir_in(path)?;
        let base = dir.path();
        fs::write(base.join("aaa"), "")?;
        Ok(base.join("AAA").exists())
    }
}
