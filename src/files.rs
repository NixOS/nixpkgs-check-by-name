use relative_path::RelativePath;
use relative_path::RelativePathBuf;
use rnix::ast;
use rnix::ast::AstToken;
use rowan::ast::AstNode;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::location;
use crate::nix_file::{NixFile, NixFileStore};
use crate::problem::{Problem, npv_145, npv_146, npv_170};
use crate::validation::ResultIteratorExt;
use crate::validation::Validation::{Failure, Success};
use crate::validation::sequence_;
use crate::{ratchet, structure, validation};

/// Runs check on all Nix files, returning a ratchet result for each
pub fn check_files(
    nixpkgs_path: &Path,
    nix_file_store: &mut NixFileStore,
    files: &[RelativePathBuf],
) -> validation::Result<BTreeMap<RelativePathBuf, ratchet::File>> {
    process_nix_files(
        nixpkgs_path,
        nix_file_store,
        files,
        |relative_path, nix_file| {
            let result = sequence_([
                check_executable_iff_shebang(relative_path, &nix_file.path)?,
                check_invalid_escapes(relative_path, nix_file)?,
            ]);
            Ok(result.map(|()| ratchet::File {}))
        },
    )
}

/// Processes all Nix files in a Nixpkgs directory according to a given function `f`, collecting the
/// results into a mapping from each file to a ratchet value.
fn process_nix_files(
    nixpkgs_path: &Path,
    nix_file_store: &mut NixFileStore,
    files: &[RelativePathBuf],
    f: impl Fn(&RelativePath, &NixFile) -> validation::Result<ratchet::File>,
) -> validation::Result<BTreeMap<RelativePathBuf, ratchet::File>> {
    let results = ResultIteratorExt::collect_vec(files.iter().map(|path| {
        // Get the (optionally-cached) parsed Nix file
        let nix_file = nix_file_store.get(&path.to_path(nixpkgs_path))?;
        let result = f(path, nix_file)?;
        let val = result.map(|ratchet| (path.clone(), ratchet));
        Ok::<_, anyhow::Error>(val)
    }))?;

    Ok(validation::sequence(results).map(|entries| {
        // Convert the Vec to a BTreeMap
        entries.into_iter().collect()
    }))
}

/// Check that a Nix file is executable if and only if it has a shebang (`#!`) line.
fn check_executable_iff_shebang(
    relative_path: &RelativePath,
    absolute_path: &Path,
) -> validation::Result<()> {
    let metadata = fs::metadata(absolute_path)?;
    let mode = metadata.permissions().mode();
    let is_executable = mode & 0o111 != 0;

    let mut file = fs::File::open(absolute_path)?;
    let mut buf = [0u8; 2];
    let bytes_read = file.read(&mut buf)?;
    let has_shebang = bytes_read >= 2 && buf == *b"#!";

    match (is_executable, has_shebang) {
        // Executable without shebang: error
        (true, false) => Ok(npv_145::NixFileIsExecutableWithoutShebang::new(relative_path).into()),
        // Shebang without executable: error
        (false, true) => Ok(npv_146::NixFileHasShebangButNotExecutable::new(relative_path).into()),
        // Both or neither: fine
        _ => Ok(Success(())),
    }
}

/// Check that a Nix file does not contain useless escape sequences.
fn check_invalid_escapes(
    relative_path: &RelativePath,
    nix_file: &NixFile,
) -> validation::Result<()> {
    let mut problems: Vec<Problem> = Vec::new();

    let mut report = |index: usize, prefix: &str, c: char, fixed: Option<String>| {
        problems.push(
            npv_170::NixFileContainsUselessEscape::new(
                location::Location::new(
                    relative_path,
                    nix_file.line_index.line(index),
                    nix_file.line_index.column(index),
                ),
                format!("{prefix}{c}"),
                c.to_string(),
                fixed,
            )
            .into(),
        );
    };

    for str_node in nix_file
        .syntax_root
        .syntax()
        .descendants()
        .filter_map(ast::Str::cast)
    {
        let is_multiline = str_node
            .syntax()
            .children_with_tokens()
            .filter_map(rnix::SyntaxElement::into_token)
            .next()
            .is_some_and(|t| t.text() == "''");

        for part in str_node.parts() {
            let ast::InterpolPart::Literal(lit) = part else {
                continue;
            };
            let base: usize = lit.syntax().text_range().start().into();
            let mut chars = lit.syntax().text().char_indices();

            while let Some((_, ch)) = chars.next() {
                if ch == '\\'
                    && !is_multiline
                    && let Some((i, c)) = chars.next()
                    && !matches!(c, '\\' | '$' | '"' | 'r' | 'n' | 't')
                {
                    report(base + i, "\\", c, Some(format!("\\\\{c}")));
                } else if ch == '\''
                    && is_multiline
                    && let Some((_, '\'')) = chars.next()
                {
                    match chars.next() {
                        Some((_, '\'' | '$')) => continue,
                        Some((_, '\\')) => match chars.next() {
                            Some((_, 'n' | 'r' | 't' | '\'')) => continue,
                            Some((i, c)) => report(base + i, "''\\", c, None),
                            None => break,
                        },
                        _ => break,
                    }
                }
            }
        }
    }

    if problems.is_empty() {
        Ok(Success(()))
    } else {
        Ok(Failure(problems))
    }
}

/// Collects all Nix files in `base` recursively.
pub fn collect_nix_files(base: &Path) -> anyhow::Result<Vec<RelativePathBuf>> {
    let mut files = vec![];
    collect_nix_files_in(base, &RelativePathBuf::new(), &mut files)?;
    Ok(files)
}

/// Recursively collects all Nix files in the relative `dir` within `base` into `files`.
fn collect_nix_files_in(
    base: &Path,
    dir: &RelativePath,
    files: &mut Vec<RelativePathBuf>,
) -> anyhow::Result<()> {
    for entry in structure::read_dir_sorted(&dir.to_path(base))? {
        let mut relative_path = dir.to_relative_path_buf();
        relative_path.push(entry.file_name().to_string_lossy().into_owned());

        let absolute_path = entry.path();

        // We'll get to every file based on directory recursion, no need to follow symlinks.
        if absolute_path.is_symlink() {
            continue;
        }
        if absolute_path.is_dir() {
            collect_nix_files_in(base, &relative_path, files)?
        } else if absolute_path.extension().is_some_and(|x| x == "nix") {
            files.push(relative_path)
        }
    }
    Ok(())
}
