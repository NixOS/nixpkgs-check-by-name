use std::fmt;

use derive_new::new;
use indoc::writedoc;

use crate::location::Location;

#[derive(Clone, Debug, new)]
pub struct NixosTestStartedUsingPkgs {
    location: Location,
}

impl fmt::Display for NixosTestStartedUsingPkgs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self { location } = self;
        writedoc!(
            f,
            "
            - {}: line {}, column {} previously did not declare the obsolete `pkgs` module argument, but now does.
              Use `config.node.pkgs` for guest packages or `hostPkgs` for host packages.
            ",
            location.file,
            location.line,
            location.column,
        )
    }
}
