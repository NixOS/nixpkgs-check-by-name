use std::fmt;

use indoc::writedoc;

use crate::location::Location;
use crate::structure;

use super::{create_path_expr, indent_definition};

#[derive(Clone)]
pub struct ByNameOverrideOfNonTopLevelPackage {
    package_name: String,
    location: Location,
    definition: String,
}

impl ByNameOverrideOfNonTopLevelPackage {
    pub fn new(
        package_name: impl Into<String>,
        location: impl Into<Location>,
        definition: impl Into<String>,
    ) -> Self {
        Self {
            package_name: package_name.into(),
            location: location.into(),
            definition: definition.into(),
        }
    }
}

impl fmt::Display for ByNameOverrideOfNonTopLevelPackage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self {
            package_name,
            location,
            definition,
        } = self;
        let Location { file, line, column } = location;
        let relative_package_dir = structure::relative_dir_for_package(package_name);
        let expected_package_path = structure::relative_file_for_package(package_name);
        let expected_path_expr = create_path_expr(file, expected_package_path);
        let indented_definition = indent_definition(*column, definition);

        writedoc!(
            f,
            "
            - Because {relative_package_dir} exists, the attribute `pkgs.{package_name}` must be defined like

                {package_name} = callPackage {expected_path_expr} {{ /* ... */ }};

              However, in this PR, a different `callPackage` is used. See the definition in {file}:{line}:

            {indented_definition}
            ",
        )
    }
}