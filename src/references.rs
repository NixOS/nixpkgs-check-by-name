use crate::nixpkgs_problem::{
    NixFileError, NixFileErrorKind, NixpkgsProblem, PathError, PathErrorKind,
};
use crate::utils;
use crate::validation::{self, ResultIteratorExt, Validation::Success};
use crate::NixFileStore;
use relative_path::RelativePath;

use anyhow::Context;
use rowan::ast::AstNode;
use std::ffi::OsStr;
use std::path::Path;

/// Check that every package directory in pkgs/by-name doesn't link to outside that directory.
/// Both symlinks and Nix path expressions are checked.
pub fn check_references(
    nix_file_store: &mut NixFileStore,
    relative_package_dir: &RelativePath,
    absolute_package_dir: &Path,
) -> validation::Result<()> {
    // The first subpath to check is the package directory itself, which we can represent as an
    // empty path, since the absolute package directory gets prepended to this.
    // We don't use `./.` to keep the error messages cleaner, since there's no canonicalisation
    // going on underneath.
    let subpath = RelativePath::new("");
    check_path(
        nix_file_store,
        relative_package_dir,
        absolute_package_dir,
        subpath,
    )
    .with_context(|| {
        format!(
            "While checking the references in package directory {}",
            relative_package_dir
        )
    })
}

/// Checks for a specific path to not have references outside.
///
/// The subpath is the relative path within the package directory we're currently checking.
/// A relative path so that the error messages don't get absolute paths (which are messy in CI).
/// The absolute package directory gets prepended before doing anything with it though.
fn check_path(
    nix_file_store: &mut NixFileStore,
    relative_package_dir: &RelativePath,
    absolute_package_dir: &Path,
    subpath: &RelativePath,
) -> validation::Result<()> {
    let path = subpath.to_path(absolute_package_dir);
    let to_validation = |kind| -> validation::Validation<()> {
        NixpkgsProblem::Path(PathError {
            relative_package_dir: relative_package_dir.to_owned(),
            subpath: subpath.to_owned(),
            kind,
        })
        .into()
    };

    Ok(if path.is_symlink() {
        // Check whether the symlink resolves to outside the package directory.
        match path.canonicalize() {
            Ok(target) => {
                // No need to handle the case of it being inside the directory, since we scan
                // through the entire directory recursively in any case.
                if let Err(_prefix_error) = target.strip_prefix(absolute_package_dir) {
                    to_validation(PathErrorKind::OutsideSymlink)
                } else {
                    Success(())
                }
            }
            Err(io_error) => to_validation(PathErrorKind::UnresolvableSymlink {
                io_error: io_error.to_string(),
            }),
        }
    } else if path.is_dir() {
        // Recursively check each entry
        validation::sequence_(
            utils::read_dir_sorted(&path)?
                .into_iter()
                .map(|entry| {
                    check_path(
                        nix_file_store,
                        relative_package_dir,
                        absolute_package_dir,
                        // TODO: The relative_path crate doesn't seem to support OsStr
                        &subpath.join(entry.file_name().to_string_lossy().to_string()),
                    )
                })
                .collect_vec()
                .with_context(|| format!("Error while recursing into {}", subpath))?,
        )
    } else if path.is_file() {
        // Only check Nix files
        if let Some(ext) = path.extension() {
            if ext == OsStr::new("nix") {
                check_nix_file(
                    nix_file_store,
                    relative_package_dir,
                    absolute_package_dir,
                    subpath,
                )
                .with_context(|| format!("Error while checking Nix file {}", subpath))?
            } else {
                Success(())
            }
        } else {
            Success(())
        }
    } else {
        // This should never happen, git doesn't support other file types
        anyhow::bail!("Unsupported file type for path {}", subpath);
    })
}

/// Check whether a Nix file contains path expression references pointing outside the package
/// directory.
fn check_nix_file(
    nix_file_store: &mut NixFileStore,
    relative_package_dir: &RelativePath,
    absolute_package_dir: &Path,
    subpath: &RelativePath,
) -> validation::Result<()> {
    let path = subpath.to_path(absolute_package_dir);

    let nix_file = nix_file_store.get(&path)?;

    Ok(validation::sequence_(
        nix_file.syntax_root.syntax().descendants().map(|node| {
            let line = nix_file.line_index.line(node.text_range().start().into());
            let text = node.text().to_string();

            // We're only interested in Path expressions
            let Some(path) = rnix::ast::Path::cast(node) else {
                return Success(());
            };

            let to_validation = |kind| -> validation::Validation<()> {
                NixpkgsProblem::NixFile(NixFileError {
                    relative_package_dir: relative_package_dir.to_owned(),
                    subpath: subpath.to_owned(),
                    line,
                    text,
                    kind,
                })
                .into()
            };

            use crate::nix_file::ResolvedPath;

            match nix_file.static_resolve_path(path, absolute_package_dir) {
                ResolvedPath::Interpolated => to_validation(NixFileErrorKind::PathInterpolation),
                ResolvedPath::SearchPath => to_validation(NixFileErrorKind::SearchPath),
                ResolvedPath::Outside => to_validation(NixFileErrorKind::OutsidePathReference),
                ResolvedPath::Unresolvable(e) => {
                    to_validation(NixFileErrorKind::UnresolvablePathReference {
                        io_error: e.to_string(),
                    })
                }
                ResolvedPath::Within(..) => {
                    // No need to handle the case of it being inside the directory, since we scan
                    // through the entire directory recursively in any case.
                    Success(())
                }
            }
        }),
    ))
}
