use crate::nix_file::CallPackageArgumentInfo;
use crate::nixpkgs_problem::{
    ByNameError, ByNameErrorKind, ByNameOverrideError, ByNameOverrideErrorKind, NixEvalError,
    NixpkgsProblem,
};
use crate::ratchet::RatchetState::{Loose, Tight};
use crate::ratchet::{self, ManualDefinition, RatchetState};
use crate::structure;
use crate::utils;
use crate::validation::ResultIteratorExt as _;
use crate::validation::{self, Validation::Success};
use crate::NixFileStore;
use relative_path::RelativePathBuf;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use anyhow::Context;
use serde::Deserialize;
use tempfile::Builder;

const EVAL_NIX: &[u8] = include_bytes!("eval.nix");

/// Attribute set of this structure is returned by `./eval.nix`
#[derive(Deserialize)]
enum Attribute {
    /// An attribute that should be defined via `pkgs/by-name`.
    ByName(ByNameAttribute),
    /// An attribute not defined via `pkgs/by-name`.
    NonByName(NonByNameAttribute),
}

#[derive(Deserialize)]
enum NonByNameAttribute {
    /// The attribute doesn't evaluate.
    EvalFailure,
    EvalSuccess(AttributeInfo),
}

#[derive(Deserialize)]
enum ByNameAttribute {
    /// The attribute doesn't exist at all.
    Missing,
    Existing(AttributeInfo),
}

#[derive(Deserialize)]
struct AttributeInfo {
    /// The location of the attribute as returned by `builtins.unsafeGetAttrPos`.
    location: Option<Location>,
    attribute_variant: AttributeVariant,
}

/// The structure returned by a successful `builtins.unsafeGetAttrPos`.
#[derive(Deserialize, Clone, Debug)]
struct Location {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
}

impl Location {
    /// Returns the [file] field, but relative to Nixpkgs.
    fn relative_file(&self, nixpkgs_path: &Path) -> anyhow::Result<RelativePathBuf> {
        let path = self.file.strip_prefix(nixpkgs_path).with_context(|| {
            format!(
                "The file ({}) is outside Nixpkgs ({})",
                self.file.display(),
                nixpkgs_path.display()
            )
        })?;
        Ok(RelativePathBuf::from_path(path).expect("relative path"))
    }
}

#[derive(Deserialize)]
pub enum AttributeVariant {
    /// The attribute is not an attribute set, so we're limited in the amount of information we can
    /// get from it. Since all derivations are attribute sets, it's obviously not a derivation.
    NonAttributeSet,
    AttributeSet {
        /// Whether the attribute is a derivation (`lib.isDerivation`)
        derivation: Derivation,
        /// The type of `callPackage` used.
        definition_variant: DefinitionVariant,
    },
}

/// Info about a derivation
#[derive(Deserialize)]
pub enum Derivation {
    NonDerivation,
    Derivation {
        pname: String,
        description: Option<String>,
    },
}

#[derive(Deserialize)]
pub enum DefinitionVariant {
    /// An automatic definition by the `pkgs/by-name` overlay, though it's detected using the
    /// internal `_internalCallByNamePackageFile` attribute, which can in theory also be used by
    /// other code.
    AutoDefinition,
    /// A manual definition of the attribute, typically in `all-packages.nix`.
    ManualDefinition {
        /// Whether the attribute is defined as `pkgs.callPackage ...` or something else.
        is_semantic_call_package: bool,
    },
}

/// Pass through variables needed to make Nix evaluation work inside Nix build. See `initNix`.
/// If these variables don't exist, assume we're not in a Nix sandbox.
fn pass_through_environment_variables_for_nix_eval_in_nix_build(command: &mut process::Command) {
    for variable in [
        "NIX_CONF_DIR",
        "NIX_LOCALSTATE_DIR",
        "NIX_LOG_DIR",
        "NIX_STATE_DIR",
        "NIX_STORE_DIR",
    ] {
        if let Ok(value) = env::var(variable) {
            command.env(variable, value);
        }
    }
}

#[cfg(not(test))]
fn mutate_nix_instatiate_arguments_based_on_cfg(
    _work_dir_path: &Path,
    command: &mut process::Command,
) -> anyhow::Result<()> {
    command.arg("--show-trace");

    Ok(())
}

/// Tests need to be able to mock out `<nixpkgs>`; do that for them.
#[cfg(test)]
fn mutate_nix_instatiate_arguments_based_on_cfg(
    work_dir_path: &Path,
    command: &mut process::Command,
) -> anyhow::Result<()> {
    const MOCK_NIXPKGS: &[u8] = include_bytes!("../tests/mock-nixpkgs.nix");
    let mock_nixpkgs_path = work_dir_path.join("mock-nixpkgs.nix");
    fs::write(&mock_nixpkgs_path, MOCK_NIXPKGS)?;

    // Wire it up so that it can be imported as `import <test-nixpkgs> { }`.
    command.arg("-I");
    command.arg(&format!("test-nixpkgs={}", mock_nixpkgs_path.display()));

    // Retrieve the path to the real nixpkgs lib, then wire it up to `import <test-nixpkgs/lib>`.
    let nixpkgs_lib = env::var("NIXPKGS_VET_NIXPKGS_LIB")
        .with_context(|| "Could not get environment variable NIXPKGS_VET_NIXPKGS_LIB")?;

    command.arg("-I");
    command.arg(&format!("test-nixpkgs/lib={nixpkgs_lib}"));

    Ok(())
}

/// Check that the Nixpkgs attribute values corresponding to the packages in `pkgs/by-name` are of
/// the form `callPackage <package_file> { ... }`. See the `./eval.nix` file for how this is
/// achieved on the Nix side.
pub fn check_values(
    nixpkgs_path: &Path,
    nix_file_store: &mut NixFileStore,
    package_names: Vec<String>,
) -> validation::Result<ratchet::Nixpkgs> {
    let work_dir = Builder::new()
        .prefix("nixpkgs-vet")
        .tempdir()
        .with_context(|| "Failed to create a working directory")?;

    // Canonicalize the path so that if a symlink were returned, we wouldn't ask Nix to follow it.
    let work_dir_path = work_dir.path().canonicalize()?;

    // Write the list of packages we need to check into a temporary JSON file.
    let package_names_path = work_dir_path.join("package-names.json");
    let package_names_file = fs::File::create(&package_names_path)?;
    serde_json::to_writer(&package_names_file, &package_names).with_context(|| {
        format!(
            "Failed to serialise the package names to the work dir {}",
            work_dir_path.display()
        )
    })?;

    // Write the Nix file into the work directory.
    let eval_nix_path = work_dir_path.join("eval.nix");
    fs::write(&eval_nix_path, EVAL_NIX)?;

    // Pinning Nix in this way makes the tool more reproducible
    let nix_package = env::var("NIXPKGS_VET_NIX_PACKAGE")
        .with_context(|| "Could not get environment variable NIXPKGS_VET_NIX_PACKAGE")?;

    // With restrict-eval, only paths in NIX_PATH can be accessed. We explicitly specify them here.
    let mut command = process::Command::new(format!("{nix_package}/bin/nix-instantiate"));
    command
        // Capture stderr so that it can be printed later in case of failure
        .stderr(process::Stdio::piped())
        // Clear environment so that nothing from the outside influences this `nix-instantiate`.
        .env_clear()
        .args([
            "--eval",
            "--json",
            "--strict",
            "--readonly-mode",
            "--restrict-eval",
        ])
        // Add the work directory to the NIX_PATH so that it can be accessed in restrict-eval mode.
        .arg("-I")
        .arg(&work_dir_path)
        .args(["--arg", "attrsPath"])
        .arg(&package_names_path)
        // Same for the nixpkgs to test, adding it to the NIX_PATH so it can be accessed in
        // restrict-eval mode.
        .args(["--arg", "nixpkgsPath"])
        .arg(nixpkgs_path)
        .arg("-I")
        .arg(nixpkgs_path);

    pass_through_environment_variables_for_nix_eval_in_nix_build(&mut command);
    mutate_nix_instatiate_arguments_based_on_cfg(&work_dir_path, &mut command)?;

    command.arg(eval_nix_path);

    let result = command
        .output()
        .with_context(|| format!("Failed to run command {command:?}"))?;

    if !result.status.success() {
        // Early return in case evaluation fails
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        return Ok(NixpkgsProblem::NixEval(NixEvalError { stderr }).into());
    }

    // Parse the resulting JSON value
    let attributes: Vec<(String, Attribute)> = serde_json::from_slice(&result.stdout)
        .with_context(|| {
            format!(
                "Failed to deserialise {}",
                String::from_utf8_lossy(&result.stdout)
            )
        })?;

    let check_result = validation::sequence(
        attributes
            .into_iter()
            .map(|(attribute_name, attribute_value)| {
                let check_result = match attribute_value {
                    Attribute::NonByName(non_by_name_attribute) => handle_non_by_name_attribute(
                        nixpkgs_path,
                        nix_file_store,
                        &attribute_name,
                        non_by_name_attribute,
                    )?,
                    Attribute::ByName(by_name_attribute) => by_name(
                        nix_file_store,
                        nixpkgs_path,
                        &attribute_name,
                        by_name_attribute,
                    )?,
                };
                Ok::<_, anyhow::Error>(check_result.map(|value| (attribute_name.clone(), value)))
            })
            .collect_vec()?,
    );

    Ok(check_result.map(|elems| ratchet::Nixpkgs {
        package_names: elems.iter().map(|(name, _)| name.to_owned()).collect(),
        package_map: elems.into_iter().collect(),
    }))
}

/// Check the validity of a package description
fn check_description(pname: &str, description: &Option<String>) -> ratchet::PackageDescription {
    if let Some(text) = description {
        let lowercased = text.to_lowercase();
        ratchet::PackageDescription {
            not_capitalised: {
                if let Some(first) = text.chars().next() {
                    if first.is_lowercase() {
                        Loose(text.to_owned())
                    } else {
                        Tight
                    }
                } else {
                    Tight
                }
            },
            starts_with_article: {
                if lowercased.starts_with("a ")
                    || lowercased.starts_with("an ")
                    || lowercased.starts_with("the ")
                {
                    Loose(text.to_owned())
                } else {
                    Tight
                }
            },
            starts_with_package_name: {
                if lowercased.starts_with(&pname.to_lowercase()) {
                    Loose(text.to_owned())
                } else {
                    Tight
                }
            },
            ends_with_punctuation: {
                if let Some(last) = text.chars().last() {
                    if last.is_ascii_punctuation() {
                        Loose(text.to_owned())
                    } else {
                        Tight
                    }
                } else {
                    Tight
                }
            },
        }
    } else {
        ratchet::PackageDescription {
            not_capitalised: Tight,
            starts_with_article: Tight,
            starts_with_package_name: Tight,
            ends_with_punctuation: Tight,
        }
    }
}

/// Handle the evaluation result for an attribute in `pkgs/by-name`, making it a validation result.
fn by_name(
    nix_file_store: &mut NixFileStore,
    nixpkgs_path: &Path,
    attribute_name: &str,
    by_name_attribute: ByNameAttribute,
) -> validation::Result<ratchet::Package> {
    use ratchet::RatchetState::*;
    use ByNameAttribute::*;

    let to_validation = |kind| -> validation::Validation<RatchetState<ManualDefinition>> {
        NixpkgsProblem::ByName(ByNameError {
            attribute_name: attribute_name.to_owned(),
            kind,
        })
        .into()
    };

    let description_ratchet = match by_name_attribute {
        Existing(AttributeInfo {
            attribute_variant:
                AttributeVariant::AttributeSet {
                    derivation:
                        Derivation::Derivation {
                            pname: ref name,
                            ref description,
                        },
                    ..
                },
            ..
        }) => check_description(name, description),
        _ => ratchet::PackageDescription {
            not_capitalised: NonApplicable,
            starts_with_article: NonApplicable,
            starts_with_package_name: NonApplicable,
            ends_with_punctuation: NonApplicable,
        },
    };

    // At this point we know that `pkgs/by-name/fo/foo/package.nix` has to exists.  This match
    // decides whether the attribute `foo` is defined accordingly and whether a legacy manual
    // definition could be removed.
    let manual_definition_result = match by_name_attribute {
        // The attribute is missing.
        Missing => {
            // This indicates a bug in the `pkgs/by-name` overlay, because it's supposed to
            // automatically defined attributes in `pkgs/by-name`
            to_validation(ByNameErrorKind::UndefinedAttr)
        }
        // The attribute exists
        Existing(AttributeInfo {
            // But it's not an attribute set, which limits the amount of information we can get
            // about this attribute (see ./eval.nix)
            attribute_variant: AttributeVariant::NonAttributeSet,
            location: _location,
        }) => {
            // The only thing we know is that it's definitely not a derivation, since those are
            // always attribute sets.
            //
            // We can't know whether the attribute is automatically or manually defined for sure,
            // and while we could check the location, the error seems clear enough as is.
            to_validation(ByNameErrorKind::NonDerivation)
        }
        // The attribute exists
        Existing(AttributeInfo {
            // And it's an attribute set, which allows us to get more information about it
            attribute_variant:
                AttributeVariant::AttributeSet {
                    derivation,
                    definition_variant,
                },
            location,
        }) => {
            // Only derivations are allowed in `pkgs/by-name`.
            let is_derivation_result = match derivation {
                Derivation::Derivation { .. } => Success(()),
                Derivation::NonDerivation => {
                    to_validation(ByNameErrorKind::NonDerivation).map(|_| ())
                }
            };

            // If the definition looks correct
            let variant_result = match definition_variant {
                // An automatic `callPackage` by the `pkgs/by-name` overlay.
                // Though this gets detected by checking whether the internal
                // `_internalCallByNamePackageFile` was used
                DefinitionVariant::AutoDefinition => {
                    if let Some(_location) = location {
                        // Such an automatic definition should definitely not have a location.
                        // Having one indicates that somebody is using
                        // `_internalCallByNamePackageFile`,
                        to_validation(ByNameErrorKind::InternalCallPackageUsed)
                    } else {
                        Success(Tight)
                    }
                }
                // The attribute is manually defined, e.g. in `all-packages.nix`.
                // This means we need to enforce it to look like this:
                //   callPackage ../pkgs/by-name/fo/foo/package.nix { ... }
                DefinitionVariant::ManualDefinition {
                    is_semantic_call_package,
                } => {
                    // We should expect manual definitions to have a location, otherwise we can't
                    // enforce the expected format
                    if let Some(location) = location {
                        // Parse the Nix file in the location
                        let nix_file = nix_file_store.get(&location.file)?;

                        // The relative path of the Nix file, for error messages
                        let relative_location_file =
                            location.relative_file(nixpkgs_path).with_context(|| {
                                format!(
                                    "Failed to resolve the file where attribute {} is defined",
                                    attribute_name
                                )
                            })?;

                        // Figure out whether it's an attribute definition of the form
                        // `= callPackage <arg1> <arg2>`, returning the arguments if so.
                        let (optional_syntactic_call_package, definition) = nix_file
                            .call_package_argument_info_at(
                                location.line,
                                location.column,
                                nixpkgs_path,
                            )
                            .with_context(|| {
                                format!(
                                    "Failed to get the definition info for attribute {}",
                                    attribute_name
                                )
                            })?;

                        by_name_override(
                            attribute_name,
                            is_semantic_call_package,
                            optional_syntactic_call_package,
                            definition,
                            location,
                            relative_location_file,
                        )
                    } else {
                        // If manual definitions don't have a location, it's likely `mapAttrs`'d
                        // over, e.g. if it's defined in aliases.nix.
                        // We can't verify whether its of the expected `callPackage`, so error out.
                        to_validation(ByNameErrorKind::CannotDetermineAttributeLocation)
                    }
                }
            };

            // Independently report problems about whether it's a derivation and the callPackage
            // variant.
            is_derivation_result.and(variant_result)
        }
    };
    Ok(
        // Packages being checked in this function are _always_ already defined in `pkgs/by-name`,
        // so instead of repeating ourselves all the time to define `uses_by_name`, just set it
        // once at the end with a map.
        manual_definition_result.map(|manual_definition| ratchet::Package {
            manual_definition,
            description: description_ratchet,
            uses_by_name: Tight,
        }),
    )
}

/// Handles the case for packages in `pkgs/by-name` that are manually overridden,
/// e.g. in `pkgs/top-level/all-packages.nix`.
fn by_name_override(
    attribute_name: &str,
    is_semantic_call_package: bool,
    optional_syntactic_call_package: Option<CallPackageArgumentInfo>,
    definition: String,
    location: Location,
    relative_location_file: RelativePathBuf,
) -> validation::Validation<ratchet::RatchetState<ratchet::ManualDefinition>> {
    let expected_package_path = structure::relative_file_for_package(attribute_name);

    let to_problem = |kind| {
        NixpkgsProblem::ByNameOverride(ByNameOverrideError {
            package_name: attribute_name.to_owned(),
            expected_package_path: expected_package_path.to_owned(),
            file: relative_location_file,
            line: location.line,
            column: location.column,
            definition,
            kind,
        })
    };

    // At this point, we completed two different checks for whether it's a `callPackage`
    match (is_semantic_call_package, optional_syntactic_call_package) {
        // Something like `<attr> = foo`
        (_, None) => to_problem(ByNameOverrideErrorKind::NonSyntacticCallPackage).into(),

        // Something like `<attr> = pythonPackages.callPackage ...`
        (false, Some(_)) => to_problem(ByNameOverrideErrorKind::NonToplevelCallPackage).into(),

        // Something like `<attr> = pkgs.callPackage ...`
        (true, Some(syntactic_call_package)) => {
            if let Some(actual_package_path) = syntactic_call_package.relative_path {
                if actual_package_path != expected_package_path {
                    // Wrong path
                    to_problem(ByNameOverrideErrorKind::WrongCallPackagePath {
                        actual_path: actual_package_path,
                    })
                    .into()
                } else {
                    // Manual definitions with empty arguments are not allowed anymore,
                    // but existing ones should continue to be allowed.
                    let manual_definition_ratchet = if syntactic_call_package.empty_arg {
                        // This is the state to migrate away from.
                        Loose(to_problem(ByNameOverrideErrorKind::EmptyArgument))
                    } else {
                        // This is the state to migrate to.
                        Tight
                    };

                    Success(manual_definition_ratchet)
                }
            } else {
                // No path...
                to_problem(ByNameOverrideErrorKind::NonPath).into()
            }
        }
    }
}

/// Handles the evaluation result for an attribute _not_ in `pkgs/by-name`, turning it into a
/// validation result.
fn handle_non_by_name_attribute(
    nixpkgs_path: &Path,
    nix_file_store: &mut NixFileStore,
    attribute_name: &str,
    non_by_name_attribute: NonByNameAttribute,
) -> validation::Result<ratchet::Package> {
    use ratchet::RatchetState::*;
    use NonByNameAttribute::*;

    let description_ratchet = match non_by_name_attribute {
        EvalSuccess(AttributeInfo {
            attribute_variant:
                AttributeVariant::AttributeSet {
                    derivation:
                        Derivation::Derivation {
                            pname: ref name,
                            ref description,
                        },
                    ..
                },
            ..
        }) => check_description(name, description),
        _ => ratchet::PackageDescription {
            not_capitalised: NonApplicable,
            starts_with_article: NonApplicable,
            starts_with_package_name: NonApplicable,
            ends_with_punctuation: NonApplicable,
        },
    };

    // The ratchet state whether this attribute uses `pkgs/by-name`.
    //
    // This is never `Tight`, because we only either:
    // - Know that the attribute _could_ be migrated to `pkgs/by-name`, which is `Loose`
    // - Or we're unsure, in which case we use `NonApplicable`
    let uses_by_name =
        // This is a big ol' match on various properties of the attribute
        //
        // First, it needs to succeed evaluation. We can't know whether an attribute could be
        // migrated to `pkgs/by-name` if it doesn't evaluate, since we need to check that it's a
        // derivation.
        //
        // This only has the minor negative effect that if a PR that breaks evaluation gets merged,
        // fixing those failures won't force anything into `pkgs/by-name`.
        //
        // For now this isn't our problem, but in the future we might have another check to enforce
        // that evaluation must not be broken.
        //
        // The alternative of assuming that failing attributes would have been fit for
        // `pkgs/by-name` has the problem that if a package evaluation gets broken temporarily,
        // fixing it requires a move to pkgs/by-name, which could happen more often and isn't
        // really justified.
        if let EvalSuccess(AttributeInfo {
            // We're only interested in attributes that are attribute sets, which all derivations
            // are. Anything else can't be in `pkgs/by-name`.
            attribute_variant: AttributeVariant::AttributeSet {
                // As of today, non-derivation attribute sets can't be in `pkgs/by-name`.
                derivation: Derivation::Derivation { .. },
                // Of the two definition variants, really only the manual one makes sense here.
                //
                // Special cases are:
                //
                // - Manual aliases to auto-called packages are not treated as manual definitions,
                //   due to limitations in the semantic `callPackage` detection.
                //   So those should be ignored.
                //
                // - Manual definitions using the internal `_internalCallByNamePackageFile` are
                //   not treated as manual definitions, since `_internalCallByNamePackageFile` is
                //   used to detect automatic ones. We can't distinguish from the above case, so we
                //   just need to ignore this one too, even if that internal attribute should never
                //   be called manually.
                definition_variant: DefinitionVariant::ManualDefinition {
                    is_semantic_call_package
                }
            },
            // We need the location of the manual definition, because otherwise we can't figure out
            // whether it's a syntactic `callPackage`.
            location: Some(location),
        }) = non_by_name_attribute {

        // Parse the Nix file in the location
        let nix_file = nix_file_store.get(&location.file)?;

        // The relative path of the Nix file, for error messages
        let relative_location_file = location.relative_file(nixpkgs_path).with_context(|| {
            format!("Failed to resolve the file where attribute {attribute_name} is defined")
        })?;

        // Figure out whether it's an attribute definition of the form
        // `= callPackage <arg1> <arg2>`, returning the arguments if so.
        let (optional_syntactic_call_package, _definition) = nix_file
            .call_package_argument_info_at(
                location.line,
                location.column,
                // Passing the Nixpkgs path here both checks that the <arg1> is within Nixpkgs,
                // and strips the absolute Nixpkgs path from it, such that
                // syntactic_call_package.relative_path is relative to Nixpkgs
                nixpkgs_path
            )
            .with_context(|| {
                format!("Failed to get the definition info for attribute {}", attribute_name)
            })?;

        // At this point, we completed two different checks for whether it's a `callPackage`.
        match (is_semantic_call_package, optional_syntactic_call_package) {
            // Something like `<attr> = { }`
            (false, None)
            // Something like `<attr> = pythonPackages.callPackage ...`
            | (false, Some(_))
            // Something like `<attr> = bar` where `bar = pkgs.callPackage ...`
            | (true, None) => {
                // In all of these cases, it's not possible to migrate the package to
                // `pkgs/by-name`.
                NonApplicable
            }

            // Something like `<attr> = pkgs.callPackage ...`
            (true, Some(syntactic_call_package)) => {
                // It's only possible to migrate such a definitions if..
                match syntactic_call_package.relative_path {
                    Some(ref rel_path) if rel_path.starts_with(utils::BASE_SUBPATH) => {
                        // ..the path is not already within `pkgs/by-name` like
                        //
                        //   foo-variant = callPackage ../by-name/fo/foo/package.nix {
                        //     someFlag = true;
                        //   }
                        //
                        // While such definitions could be moved to `pkgs/by-name` by using
                        // `.override { someFlag = true; }` instead, this changes the semantics in
                        // relation with overlays, so migration is generally not possible.
                        //
                        // See also "package variants" in RFC 140:
                        // https://github.com/NixOS/rfcs/blob/master/rfcs/0140-simple-package-paths.md#package-variants
                        NonApplicable
                    }
                    _ => {
                        // Otherwise, the path is outside `pkgs/by-name`, which means it can be
                        // migrated.
                        Loose((syntactic_call_package, relative_location_file))
                    }
                }
            }
        }
    } else {
        // This catches all the cases not matched by the above `if let`, falling back to not being
        // able to migrate such attributes.
        NonApplicable
    };
    Ok(Success(ratchet::Package {
        // Packages being checked in this function _always_ need a manual definition, because
        // they're not using `pkgs/by-name` which would allow avoiding it. So instead of repeating
        // ourselves all the time to define `manual_definition`, just set it once at the end here.
        manual_definition: Tight,
        uses_by_name,
        description: description_ratchet,
    }))
}
