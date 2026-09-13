use std::fmt;

use derive_enum_from_into::EnumFrom;

pub mod npv_100;
pub mod npv_101;
pub mod npv_109;
pub mod npv_110;
pub mod npv_111;

pub mod npv_120;
pub mod npv_121;
pub mod npv_122;
pub mod npv_123;
pub mod npv_124;
pub mod npv_125;
pub mod npv_126;
pub mod npv_127;
pub mod npv_128;

pub mod npv_140;
pub mod npv_141;
pub mod npv_142;
pub mod npv_143;
pub mod npv_144;
pub mod npv_145;
pub mod npv_146;

pub mod npv_160;
pub mod npv_162;
pub mod npv_164;
pub mod npv_165;
pub mod npv_166;
pub mod npv_167;

pub mod npv_170;
pub mod npv_172;
pub mod npv_173;

const WIKI_BASE_URL: &str = "https://github.com/NixOS/nixpkgs-vet/wiki";

#[derive(Clone, Debug, EnumFrom)]
pub enum Problem {
    /// NPV-100: attribute is not defined but it should be defined automatically
    ByNameUndefinedAttribute(npv_100::ByNameUndefinedAttribute),

    /// NPV-101: attribute is not a derivation
    ByNameNonDerivation(npv_101::ByNameNonDerivation),

    /// NPV-109: by-name shard is not a directory
    ByNameShardIsNotDirectory(npv_109::ByNameShardIsNotDirectory),

    /// NPV-110: by-name shard is invalid
    ByNameShardIsInvalid(npv_110::ByNameShardIsInvalid),

    /// NPV-111: by-name shard is case-sensitive duplicate
    ByNameShardIsCaseSensitiveDuplicate(npv_111::ByNameShardIsCaseSensitiveDuplicate),

    /// NPV-120: Nix evaluation failed
    NixEvalError(npv_120::NixEvalError),

    /// NPV-121: Nix file contains interpolated path
    NixFileContainsPathInterpolation(npv_121::NixFileContainsPathInterpolation),

    /// NPV-122: Nix file contains search path
    NixFileContainsSearchPath(npv_122::NixFileContainsSearchPath),

    /// NPV-123: Nix file contains path expression outside of directory
    NixFileContainsPathOutsideDirectory(npv_123::NixFileContainsPathOutsideDirectory),

    /// NPV-124: Nix file contains unresolvable path expression
    NixFileContainsUnresolvablePath(npv_124::NixFileContainsUnresolvablePath),

    /// NPV-125: Package contains symlink pointing outside its directory
    PackageContainsSymlinkPointingOutside(npv_125::PackageContainsSymlinkPointingOutside),

    /// NPV-126: Package contains unresolvable symlink
    PackageContainsUnresolvableSymlink(npv_126::PackageContainsUnresolvableSymlink),

    /// NPV-127: Nix file contains absolute path expression
    NixFileContainsAbsolutePath(npv_127::NixFileContainsAbsolutePath),

    /// NPV-128: Nix file contains home-relative path expression
    NixFileContainsHomeRelativePath(npv_128::NixFileContainsHomeRelativePath),

    /// NPV-140: Package directory is not directory
    PackageDirectoryIsNotDirectory(npv_140::PackageDirectoryIsNotDirectory),

    /// NPV-141: Package name is not valid
    InvalidPackageDirectoryName(npv_141::InvalidPackageDirectoryName),

    /// NPV-142: Package is in the wrong by-name shard
    PackageInWrongShard(npv_142::PackageInWrongShard),

    /// NPV-143: `package.nix` is missing
    PackageNixMissing(npv_143::PackageNixMissing),

    /// NPV-144: `package.nix` is not a file
    PackageNixIsNotFile(npv_144::PackageNixIsNotFile),

    /// NPV-145: Nix file is executable without shebang
    NixFileIsExecutableWithoutShebang(npv_145::NixFileIsExecutableWithoutShebang),

    /// NPV-146: Nix file has shebang but is not executable
    NixFileHasShebangButNotExecutable(npv_146::NixFileHasShebangButNotExecutable),

    /// NPV-160: top-level package moved out of by-name
    TopLevelPackageMovedOutOfByName(npv_160::TopLevelPackageMovedOutOfByName),

    /// NPV-162: new top-level package should be in by-name
    NewTopLevelPackageShouldBeByName(npv_162::NewTopLevelPackageShouldBeByName),

    /// NPV-164: new top-level package must enable strictDeps
    NewTopLevelPackageMustEnableStrictDeps(npv_164::NewTopLevelPackageMustEnableStrictDeps),

    /// NPV-165: top-level package disabled strictDeps
    TopLevelPackageDisabledStrictDeps(npv_165::TopLevelPackageDisabledStrictDeps),

    /// NPV-166: new top-level package must enable __structuredAttrs
    NewTopLevelPackageMustEnableStructuredAttrs(
        npv_166::NewTopLevelPackageMustEnableStructuredAttrs,
    ),

    /// NPV-167: top-level package disabled __structuredAttrs
    TopLevelPackageDisabledStructuredAttrs(npv_167::TopLevelPackageDisabledStructuredAttrs),

    /// NPV-170: nix files should not contain useless escapes
    NixFileContainsUselessEscape(npv_170::NixFileContainsUselessEscape),

    /// NPV-172: new NixOS test uses the pkgs module argument
    NewNixosTestUsesPkgs(npv_172::NewNixosTestUsesPkgs),

    /// NPV-173: NixOS test started using the pkgs module argument
    NixosTestStartedUsingPkgs(npv_173::NixosTestStartedUsingPkgs),
}

impl Problem {
    /// Returns the NPV error code for this problem (e.g. "NPV-100").
    pub fn npv_code(&self) -> &'static str {
        match self {
            Self::ByNameUndefinedAttribute(..) => "NPV-100",
            Self::ByNameNonDerivation(..) => "NPV-101",
            Self::ByNameShardIsNotDirectory(..) => "NPV-109",
            Self::ByNameShardIsInvalid(..) => "NPV-110",
            Self::ByNameShardIsCaseSensitiveDuplicate(..) => "NPV-111",
            Self::NixEvalError(..) => "NPV-120",
            Self::NixFileContainsPathInterpolation(..) => "NPV-121",
            Self::NixFileContainsSearchPath(..) => "NPV-122",
            Self::NixFileContainsPathOutsideDirectory(..) => "NPV-123",
            Self::NixFileContainsUnresolvablePath(..) => "NPV-124",
            Self::PackageContainsSymlinkPointingOutside(..) => "NPV-125",
            Self::PackageContainsUnresolvableSymlink(..) => "NPV-126",
            Self::NixFileContainsAbsolutePath(..) => "NPV-127",
            Self::NixFileContainsHomeRelativePath(..) => "NPV-128",
            Self::PackageDirectoryIsNotDirectory(..) => "NPV-140",
            Self::InvalidPackageDirectoryName(..) => "NPV-141",
            Self::PackageInWrongShard(..) => "NPV-142",
            Self::PackageNixMissing(..) => "NPV-143",
            Self::PackageNixIsNotFile(..) => "NPV-144",
            Self::NixFileIsExecutableWithoutShebang(..) => "NPV-145",
            Self::NixFileHasShebangButNotExecutable(..) => "NPV-146",
            Self::TopLevelPackageMovedOutOfByName(..) => "NPV-160",
            Self::NewTopLevelPackageShouldBeByName(..) => "NPV-162",
            Self::NewTopLevelPackageMustEnableStrictDeps(..) => "NPV-164",
            Self::TopLevelPackageDisabledStrictDeps(..) => "NPV-165",
            Self::NewTopLevelPackageMustEnableStructuredAttrs(..) => "NPV-166",
            Self::TopLevelPackageDisabledStructuredAttrs(..) => "NPV-167",
            Self::NixFileContainsUselessEscape(..) => "NPV-170",
            Self::NewNixosTestUsesPkgs(..) => "NPV-172",
            Self::NixosTestStartedUsingPkgs(..) => "NPV-173",
        }
    }

    /// Returns the wiki URL for this problem's documentation.
    pub fn wiki_url(&self) -> String {
        format!("{WIKI_BASE_URL}/{}", self.npv_code())
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ByNameUndefinedAttribute(inner) => inner.fmt(f),
            Self::ByNameNonDerivation(inner) => inner.fmt(f),
            Self::ByNameShardIsNotDirectory(inner) => inner.fmt(f),
            Self::ByNameShardIsInvalid(inner) => inner.fmt(f),
            Self::ByNameShardIsCaseSensitiveDuplicate(inner) => inner.fmt(f),
            Self::NixEvalError(inner) => inner.fmt(f),
            Self::NixFileContainsPathInterpolation(inner) => inner.fmt(f),
            Self::NixFileContainsSearchPath(inner) => inner.fmt(f),
            Self::NixFileContainsPathOutsideDirectory(inner) => inner.fmt(f),
            Self::NixFileContainsUnresolvablePath(inner) => inner.fmt(f),
            Self::PackageContainsSymlinkPointingOutside(inner) => inner.fmt(f),
            Self::PackageContainsUnresolvableSymlink(inner) => inner.fmt(f),
            Self::NixFileContainsAbsolutePath(inner) => inner.fmt(f),
            Self::NixFileContainsHomeRelativePath(inner) => inner.fmt(f),
            Self::PackageDirectoryIsNotDirectory(inner) => inner.fmt(f),
            Self::InvalidPackageDirectoryName(inner) => inner.fmt(f),
            Self::PackageInWrongShard(inner) => inner.fmt(f),
            Self::PackageNixMissing(inner) => inner.fmt(f),
            Self::PackageNixIsNotFile(inner) => inner.fmt(f),
            Self::NixFileIsExecutableWithoutShebang(inner) => inner.fmt(f),
            Self::NixFileHasShebangButNotExecutable(inner) => inner.fmt(f),
            Self::TopLevelPackageMovedOutOfByName(inner) => inner.fmt(f),
            Self::NewTopLevelPackageShouldBeByName(inner) => inner.fmt(f),
            Self::NewTopLevelPackageMustEnableStrictDeps(inner) => inner.fmt(f),
            Self::TopLevelPackageDisabledStrictDeps(inner) => inner.fmt(f),
            Self::NewTopLevelPackageMustEnableStructuredAttrs(inner) => inner.fmt(f),
            Self::TopLevelPackageDisabledStructuredAttrs(inner) => inner.fmt(f),
            Self::NixFileContainsUselessEscape(inner) => inner.fmt(f),
            Self::NewNixosTestUsesPkgs(inner) => inner.fmt(f),
            Self::NixosTestStartedUsingPkgs(inner) => inner.fmt(f),
        }
    }
}
