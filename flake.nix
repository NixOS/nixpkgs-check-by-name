{
  description = "nixpkgs-check-by-name: a linter for nixpkgs";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      self,
      nixpkgs,
      treefmt-nix,
    }:
    let
      # Until https://github.com/NixOS/nixpkgs/pull/295083 is accepted and merged.
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      eachSystem =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import ./default.nix {
              inherit nixpkgs system treefmt-nix;
              pkgs = nixpkgs.legacyPackages.${system};
            }
          )
        );
    in
    {
      packages = eachSystem (this: {
        default = this.build;
      });

      devShells = eachSystem (this: {
        default = this.shell;
      });

      checks = eachSystem (this: {
        formatting = this.treefmt;
        nixpkgs = this.nixpkgsCheck;
      });

      formatter = eachSystem (this: this.treefmtWrapper);
    };
}
