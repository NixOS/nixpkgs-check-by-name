use std::fmt;

use derive_new::new;
use indoc::writedoc;

use crate::location::Location;

#[derive(Clone, Debug, new)]
pub struct NewNixosTestUsesPkgs {
    location: Location,
}

impl fmt::Display for NewNixosTestUsesPkgs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self { location } = self;
        writedoc!(
            f,
            "
            - {}: line {}, column {} is a new NixOS test that declares the obsolete `pkgs` module argument.
              Use `config.node.pkgs` for guest packages or `hostPkgs` for host packages.
            ",
            location.file,
            location.line,
            location.column,
        )
    }
}
