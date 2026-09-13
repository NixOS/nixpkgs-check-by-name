use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use relative_path::{RelativePath, RelativePathBuf};
use rnix::ast;
use rowan::ast::AstNode;

use crate::location::Location;
use crate::nix_file::{NixFile, NixFileStore, ResolvedPath};
use crate::ratchet::{self, RatchetState};

const NIXOS_TESTS: &str = "nixos/tests";

/// Finds NixOS test modules passed as literal paths to `runTest` or `runTestOn`.
pub fn find_nixos_tests(
    nixpkgs_path: &Path,
    nix_file_store: &mut NixFileStore,
    nix_files: &[RelativePathBuf],
) -> anyhow::Result<BTreeMap<RelativePathBuf, ratchet::NixosTest>> {
    let mut test_paths = BTreeSet::new();

    for caller_path in nix_files
        .iter()
        .filter(|path| path.starts_with(NIXOS_TESTS))
    {
        let caller = nix_file_store.get(&caller_path.to_path(nixpkgs_path))?;

        for application in caller
            .syntax_root
            .syntax()
            .descendants()
            .filter_map(ast::Apply::cast)
        {
            let Some(ast::Expr::PathRel(path)) = application.argument() else {
                continue;
            };
            let Some(function) = application.lambda().and_then(applied_function_name) else {
                continue;
            };
            if !matches!(function.as_str(), "runTest" | "runTestOn") {
                continue;
            }

            let path = ast::Path::PathRel(path);
            if let ResolvedPath::Within(mut relative_path) =
                caller.static_resolve_path(&path, nixpkgs_path)
            {
                if relative_path.to_path(nixpkgs_path).is_dir() {
                    relative_path.push("default.nix");
                }
                test_paths.insert(relative_path);
            }
        }
    }

    let mut tests = BTreeMap::new();
    for relative_path in test_paths {
        let nix_file = nix_file_store.get(&relative_path.to_path(nixpkgs_path))?;
        let uses_pkgs = pkgs_argument_location(&relative_path, nix_file)
            .map_or(RatchetState::Tight, RatchetState::Loose);
        tests.insert(relative_path, ratchet::NixosTest { uses_pkgs });
    }

    Ok(tests)
}

fn pkgs_argument_location(relative_path: &RelativePath, nix_file: &NixFile) -> Option<Location> {
    let pattern = outer_function_pattern(nix_file.syntax_root.expr()?)?;

    pattern.pat_entries().find_map(|entry| {
        let identifier = entry.ident()?;
        if identifier.ident_token()?.text() != "pkgs" {
            return None;
        }

        let index: usize = identifier.syntax().text_range().start().into();
        Some(Location::new(
            relative_path,
            nix_file.line_index.line(index),
            nix_file.line_index.column(index),
        ))
    })
}

fn outer_function_pattern(expression: ast::Expr) -> Option<ast::Pattern> {
    match expression {
        // ({ pkgs, ... }: ...) -> { pkgs, ... }: ...
        ast::Expr::Paren(parenthesized) => outer_function_pattern(parenthesized.expr()?),
        // { pkgs, ... }: ... -> { pkgs, ... }
        ast::Expr::Lambda(lambda) => match lambda.param()? {
            ast::Param::Pattern(pattern) => Some(pattern),
            ast::Param::IdentParam(_) => None,
        },
        _ => None,
    }
}

fn applied_function_name(expression: ast::Expr) -> Option<String> {
    match expression {
        // (runTestOn systems) -> runTestOn systems
        ast::Expr::Paren(parenthesized) => applied_function_name(parenthesized.expr()?),
        // runTestOn systems -> runTestOn
        ast::Expr::Apply(application) => applied_function_name(application.lambda()?),
        // runTestOn -> "runTestOn"
        ast::Expr::Ident(identifier) => Some(identifier.ident_token()?.text().to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::files;
    use crate::tests::tempdir;

    #[test]
    fn discovers_literal_test_modules() -> anyhow::Result<()> {
        let temp_nixpkgs = tempdir()?;
        let root = temp_nixpkgs.path();
        let tests_dir = root.join(NIXOS_TESTS);

        fs::create_dir_all(tests_dir.join("directory"))?;
        fs::create_dir_all(tests_dir.join("sub"))?;
        fs::create_dir_all(root.join("pkgs"))?;
        fs::write(
            tests_dir.join("all-tests.nix"),
            r#"
              { runTest, runTestOn, lib, other }:
              {
                direct = runTest ./direct.nix;
                on = runTestOn [ "x86_64-linux" ] ./on.nix;
                qualified = lib.runTest ./qualified.nix;
                directory = runTest ./directory;
                ignored = other ./ignored.nix;
                dynamic = runTest (if true then ./ignored.nix else ./direct.nix);
                text = "runTest ./ignored.nix";
              }
            "#,
        )?;
        fs::write(
            tests_dir.join("sub/default.nix"),
            "{ runTest }: runTest ../nested.nix",
        )?;
        fs::write(
            root.join("pkgs/default.nix"),
            "{ runTest }: runTest ../nixos/tests/outside.nix",
        )?;

        for path in [
            "direct.nix",
            "on.nix",
            "qualified.nix",
            "ignored.nix",
            "nested.nix",
            "outside.nix",
            "directory/default.nix",
        ] {
            fs::write(tests_dir.join(path), "{}")?;
        }

        let nix_files = files::collect_nix_files(root)?;
        let actual = find_nixos_tests(root, &mut NixFileStore::default(), &nix_files)?
            .into_keys()
            .collect::<BTreeSet<_>>();
        let expected = [
            "nixos/tests/direct.nix",
            "nixos/tests/on.nix",
            "nixos/tests/directory/default.nix",
            "nixos/tests/nested.nix",
        ]
        .into_iter()
        .map(RelativePathBuf::from)
        .collect();

        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn detects_pkgs_argument() -> anyhow::Result<()> {
        let temp_nixpkgs = tempdir()?;
        let mut nix_file_store = NixFileStore::default();

        for (name, contents, expected) in [
            ("direct.nix", "{ pkgs, ... }: {}", Some((1, 3))),
            ("parenthesized.nix", "({ pkgs, ... }: {})", Some((1, 4))),
            (
                "nested.nix",
                "{ config, ... }: { nodes.machine = { pkgs, ... }: {}; }",
                None,
            ),
            ("identifier.nix", "args: {}", None),
            ("attrs.nix", "{}", None),
        ] {
            let path = temp_nixpkgs.path().join(name);
            fs::write(&path, contents)?;

            let relative_path = RelativePathBuf::from(name);
            let actual = pkgs_argument_location(&relative_path, nix_file_store.get(&path)?);
            assert_eq!(
                actual
                    .as_ref()
                    .map(|location| (location.line, location.column)),
                expected,
            );
            if let Some(location) = actual {
                assert_eq!(location.file, relative_path);
            }
        }

        Ok(())
    }
}
