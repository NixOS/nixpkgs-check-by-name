//! This module implements the ratchet checks, see ../README.md#ratchet-checks
//!
//! Each type has a `compare` method that validates the ratchet checks for that item.

use std::marker::PhantomData;

use relative_path::RelativePath;
use std::collections::BTreeMap;

use relative_path::RelativePathBuf;

use crate::nix_file::CallPackageArgumentInfo;
use crate::problem::{Problem, npv_160, npv_162, npv_164, npv_165, npv_166, npv_167};
use crate::validation::{self, Validation, Validation::Success};

/// The ratchet value for the entirety of Nixpkgs.
#[derive(Default)]
pub struct Nixpkgs {
    /// The ratchet values for all packages
    pub packages: BTreeMap<String, Package>,
    pub files: BTreeMap<RelativePathBuf, File>,
    pub nixos_tests: BTreeMap<RelativePathBuf, NixosTest>,
}

impl Nixpkgs {
    /// Validates the ratchet checks for Nixpkgs
    pub fn compare(from: &Self, to: Self) -> Validation<()> {
        validation::sequence_(
            // We only loop over the current attributes,
            // we don't need to check ones that were removed
            to.packages
                .into_iter()
                .map(|(name, pkg)| Package::compare(&name, from.packages.get(&name), &pkg)),
        )
        .and_(validation::sequence_(to.files.into_iter().map(
            |(name, file)| File::compare(&name, from.files.get(&name), &file),
        )))
        .and_(validation::sequence_(to.nixos_tests.into_iter().map(
            |(name, test)| NixosTest::compare(&name, from.nixos_tests.get(&name), &test),
        )))
    }
}

/// The ratchet value for a top-level package
pub struct Package {
    /// The ratchet value for the check for new packages using pkgs/by-name
    pub uses_by_name: RatchetState<UsesByName>,

    /// The ratchet value for the check for enabling `strictDeps`.
    pub strict_deps: RatchetState<StrictDeps>,

    /// The ratchet value for the check for enabling `__structuredAttrs`.
    pub structured_attrs: RatchetState<StructuredAttrs>,
}

impl Package {
    /// Validates the ratchet checks for a top-level package
    pub fn compare(name: &str, optional_from: Option<&Self>, to: &Self) -> Validation<()> {
        validation::sequence_([
            RatchetState::<UsesByName>::compare(
                name,
                optional_from.map(|x| &x.uses_by_name),
                &to.uses_by_name,
            ),
            RatchetState::<StrictDeps>::compare(
                name,
                optional_from.map(|x| &x.strict_deps),
                &to.strict_deps,
            ),
            RatchetState::<StructuredAttrs>::compare(
                name,
                optional_from.map(|x| &x.structured_attrs),
                &to.structured_attrs,
            ),
        ])
    }
}

pub struct File {}

impl File {
    /// Validates the ratchet checks for a top-level package
    pub fn compare(
        _name: &RelativePath,
        _optional_from: Option<&Self>,
        _to: &Self,
    ) -> Validation<()> {
        Success(())
    }
}

pub struct NixosTest {}

impl NixosTest {
    pub fn compare(
        _name: &RelativePath,
        _optional_from: Option<&Self>,
        _to: &Self,
    ) -> Validation<()> {
        Success(())
    }
}

/// The ratchet state of a generic ratchet check.
pub enum RatchetState<Ratchet: ToProblem> {
    /// The ratchet is loose. It can be tightened more. In other words, this is the legacy state
    /// we're trying to move away from.
    ///
    /// Introducing new instances is not allowed but previous instances will continue to be
    /// allowed. The `Context` is context for error messages in case a new instance of this state
    /// is introduced.
    Loose(Ratchet::ToContext),

    /// The ratchet is tight. It can't be tightened any further. This is either because we already
    /// use the latest state, or because the ratchet isn't relevant.
    Tight,

    /// This ratchet can't be applied. State transitions from/to `NonApplicable` are always allowed.
    NonApplicable,
}

/// A trait that can convert an attribute-specific error context into a Problem.
pub trait ToProblem {
    /// Context relating to the Nixpkgs that is being transitioned _to_.
    type ToContext;

    /// How to convert an attribute-specific error context into a Problem.
    fn to_problem(name: &str, optional_from: Option<()>, to: &Self::ToContext) -> Problem;
}

impl<Context: ToProblem> RatchetState<Context> {
    /// Compare the previous ratchet state of an attribute to the new state.
    /// The previous state may be `None` in case the attribute is new.
    fn compare(name: &str, optional_from: Option<&Self>, to: &Self) -> Validation<()> {
        match (optional_from, to) {
            // Loosening a ratchet is not allowed.
            (Some(RatchetState::Tight), RatchetState::Loose(loose_context)) => {
                Context::to_problem(name, Some(()), loose_context).into()
            }

            // Introducing a loose ratchet is also not allowed.
            (None, RatchetState::Loose(loose_context)) => {
                Context::to_problem(name, None, loose_context).into()
            }

            // Everything else is allowed, including:
            // - Loose -> Loose (grandfathering policy for a loose ratchet)
            // - -> Tight (always okay to keep or make the ratchet tight)
            // - Anything involving NotApplicable, where we can't really make any good calls
            _ => Success(()),
        }
    }
}

/// The ratchet value of an attribute for the check that new packages use `pkgs/by-name`.
///
/// This checks that all new package defined using `callPackage` must be defined via
/// `pkgs/by-name`. It also checks that once a package uses `pkgs/by-name`, it can't switch back
/// to `pkgs/top-level/all-packages.nix`.
pub enum UsesByName {}

impl ToProblem for UsesByName {
    type ToContext = (CallPackageArgumentInfo, RelativePathBuf);

    fn to_problem(name: &str, optional_from: Option<()>, (to, file): &Self::ToContext) -> Problem {
        let is_new = optional_from.is_none();
        if is_new {
            npv_162::NewTopLevelPackageShouldBeByName::new(name, to.relative_path.clone()).into()
        } else {
            npv_160::TopLevelPackageMovedOutOfByName::new(name, to.relative_path.clone(), file)
                .into()
        }
    }
}

pub trait EnabledAttributeProblem {
    fn introduced_problem(name: &str, file: RelativePathBuf) -> Problem;
    fn regressed_problem(name: &str, file: RelativePathBuf) -> Problem;
}

/// The ratchet value of an evaluated boolean attribute that must be enabled for new packages and
/// must not regress to `false` once enabled.
pub struct EnabledAttribute<ProblemKind>(PhantomData<ProblemKind>);

impl<ProblemKind: EnabledAttributeProblem> ToProblem for EnabledAttribute<ProblemKind> {
    type ToContext = RelativePathBuf;

    fn to_problem(name: &str, optional_from: Option<()>, file: &Self::ToContext) -> Problem {
        if optional_from.is_some() {
            ProblemKind::regressed_problem(name, file.clone())
        } else {
            ProblemKind::introduced_problem(name, file.clone())
        }
    }
}

/// The ratchet value of an attribute for enabling `strictDeps`.
///
/// New packages must evaluate with `strictDeps = true` unless their package set already makes
/// that the default. Once a package evaluates with `strictDeps = true`, it must not regress.
pub type StrictDeps = EnabledAttribute<StrictDepsProblem>;

pub enum StrictDepsProblem {}

impl EnabledAttributeProblem for StrictDepsProblem {
    fn introduced_problem(name: &str, file: RelativePathBuf) -> Problem {
        npv_164::NewTopLevelPackageMustEnableStrictDeps::new(name, file).into()
    }

    fn regressed_problem(name: &str, file: RelativePathBuf) -> Problem {
        npv_165::TopLevelPackageDisabledStrictDeps::new(name, file).into()
    }
}

/// The ratchet value of an attribute for enabling `__structuredAttrs`.
///
/// New packages must evaluate with `__structuredAttrs = true` unless their package set already
/// makes that the default. Once a package evaluates with `__structuredAttrs = true`, it must not
/// regress.
pub type StructuredAttrs = EnabledAttribute<StructuredAttrsProblem>;

pub enum StructuredAttrsProblem {}

impl EnabledAttributeProblem for StructuredAttrsProblem {
    fn introduced_problem(name: &str, file: RelativePathBuf) -> Problem {
        npv_166::NewTopLevelPackageMustEnableStructuredAttrs::new(name, file).into()
    }

    fn regressed_problem(name: &str, file: RelativePathBuf) -> Problem {
        npv_167::TopLevelPackageDisabledStructuredAttrs::new(name, file).into()
    }
}
