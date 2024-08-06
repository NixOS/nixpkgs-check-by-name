use std::fmt;
use std::process::ExitCode;

use colored::Colorize as _;

use crate::nixpkgs_problem::NixpkgsProblem;

pub enum Status {
    /// It's all green.
    ValidatedSuccessfully,

    /// The base branch is broken, but this PR fixes it. Nice job!
    BranchHealed,

    /// The base branch fails, the PR doesn't fix it, and the PR may also introduce additional
    /// problems.
    BranchStillBroken(Vec<NixpkgsProblem>),

    /// This PR introduces the problems listed. Please fix them before merging, otherwise the base
    /// branch would break.
    ProblemsIntroduced(Vec<NixpkgsProblem>),

    /// This PR introduces additional instances of discouraged patterns. Merging is discouraged but
    /// would not break the base branch.
    DiscouragedPatternedIntroduced(Vec<NixpkgsProblem>),

    /// Some other error occurred.
    Error(anyhow::Error),
}

impl Status {
    fn errors(&self) -> Option<&Vec<NixpkgsProblem>> {
        match self {
            Self::ValidatedSuccessfully | Self::BranchHealed | Self::Error(..) => None,
            Self::BranchStillBroken(errors)
            | Self::ProblemsIntroduced(errors)
            | Self::DiscouragedPatternedIntroduced(errors) => Some(errors),
        }
    }

    fn fmt(&self, f: &mut fmt::Formatter, use_color: bool) -> fmt::Result {
        // These all respect the NO_COLOR environment variable even if `use_color` is true.
        let maybe_green = |s: &str| if use_color { s.green() } else { s.into() };
        let maybe_yellow = |s: &str| if use_color { s.yellow() } else { s.into() };
        let maybe_red = |s: &str| if use_color { s.red() } else { s.into() };

        // If there are errors, print them all out first in red.
        if let Some(errors) = self.errors() {
            for error in errors {
                let error = format!("{error}\n");
                fmt::Display::fmt(&maybe_red(&error), f)?;
            }
        }

        // Then, print out the message for this status.
        let message = match self {
            Self::Error(error) => format!("{} {:#}", &maybe_yellow("I/O error: "), error).into(),
            Self::ValidatedSuccessfully => maybe_green("Validated successfully"),
            Self::BranchHealed => {
                maybe_green("The base branch is broken, but this PR fixes it. Nice job!")
            }
            Self::BranchStillBroken(..) => maybe_yellow(
                "The base branch is broken and still has above problems with this PR, which need \
                 to be fixed first.\nConsider reverting the PR that introduced these problems \
                 in order to prevent more failures of unrelated PRs.",
            ),
            Self::ProblemsIntroduced(..) => maybe_yellow(
                "This PR introduces the problems listed above. Please fix them before merging, \
                 otherwise the base branch would break.",
            ),
            Self::DiscouragedPatternedIntroduced(..) => maybe_yellow(
                "This PR introduces additional instances of discouraged patterns as listed above. \
                 Merging is discouraged but would not break the base branch.",
            ),
        };
        fmt::Display::fmt(&message, f)
    }
}

impl From<anyhow::Error> for Status {
    fn from(err: anyhow::Error) -> Self {
        Self::Error(err)
    }
}

impl From<Status> for ExitCode {
    fn from(status: Status) -> Self {
        match status {
            Status::ValidatedSuccessfully | Status::BranchHealed => ExitCode::SUCCESS,
            Status::BranchStillBroken(..)
            | Status::ProblemsIntroduced(..)
            | Status::DiscouragedPatternedIntroduced(..) => ExitCode::from(1),
            Status::Error(..) => ExitCode::from(2),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Status::fmt(self, f, /* use_color */ false)
    }
}

pub struct ColoredStatus(Status);

impl From<Status> for ColoredStatus {
    fn from(status: Status) -> Self {
        Self(status)
    }
}

impl fmt::Display for ColoredStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Status::fmt(&self.0, f, /* use_color */ true)
    }
}

impl From<ColoredStatus> for ExitCode {
    fn from(status: ColoredStatus) -> Self {
        status.0.into()
    }
}
