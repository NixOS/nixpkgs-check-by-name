let
  sources = import ./npins;
in
{
  system ? builtins.currentSystem,
  nixpkgs ? sources.nixpkgs,
  treefmt-nix ? sources.treefmt-nix,
  pkgs ? (
    import nixpkgs {
      inherit system;
      config = { };
      overlays = [ ];
    }
  ),
}:
let
  inherit (pkgs) lib;

  runtimeExprPath = ./src/eval.nix;
  testNixpkgsPath = ./tests/mock-nixpkgs.nix;
  nixpkgsLibPath = nixpkgs + "/lib";

  # Needed to make Nix evaluation work inside nix builds
  initNix = ''
    export TEST_ROOT=$(pwd)/test-tmp
    export NIX_CONF_DIR=$TEST_ROOT/etc
    export NIX_LOCALSTATE_DIR=$TEST_ROOT/var
    export NIX_LOG_DIR=$TEST_ROOT/var/log/nix
    export NIX_STATE_DIR=$TEST_ROOT/var/nix
    export NIX_STORE_DIR=$TEST_ROOT/store

    # Ensure that even if tests run in parallel, we don't get an error
    # We'd run into https://github.com/NixOS/nix/issues/2706 unless the store is initialised first
    nix-store --init
  '';

  # Determine version from Cargo.toml
  version = (lib.importTOML ./Cargo.toml).package.version;

  treefmtEval = (import treefmt-nix).evalModule pkgs {
    # Used to find the project root
    projectRootFile = ".git/config";

    programs.rustfmt.enable = true;
    programs.nixfmt-rfc-style.enable = true;
    programs.shfmt.enable = true;
    settings.formatter.shfmt.options = [ "--space-redirects" ];
  };

  packages = {
    build = pkgs.callPackage ./package.nix {
      inherit
        nixpkgsLibPath
        initNix
        runtimeExprPath
        testNixpkgsPath
        version
        ;
    };

    shell = pkgs.mkShell {
      env.NIX_CHECK_BY_NAME_EXPR_PATH = toString runtimeExprPath;
      env.NIX_PATH = "test-nixpkgs=${toString testNixpkgsPath}:test-nixpkgs/lib=${toString nixpkgsLibPath}";
      env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      inputsFrom = [ packages.build ];
      nativeBuildInputs = with pkgs; [
        cargo-audit
        cargo-edit
        cargo-outdated
        npins
        rust-analyzer
        treefmtEval.config.build.wrapper
      ];
    };

    # This checks that all Git-tracked files are formatted appropriately
    treefmt = treefmtEval.config.build.check (
      lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.gitTracked ./.;
      }
    );

    # Fits in with `nix fmt`.
    treefmtWrapper = treefmtEval.config.build.wrapper;

    # Run regularly by CI and turned into a PR
    autoPrUpdate =
      let
        updateScripts = {
          flake = pkgs.writeShellApplication {
            name = "update-flake";
            runtimeInputs = with pkgs; [
              git
              nix
            ];
            text = ''
              echo "<details><summary>flake.nix changes</summary>"
              # Needed because GitHub's rendering of the first body line breaks down otherwise
              echo ""
              echo '```'
              cd "$1"
              nix flake update 2>&1
              echo  '```'
              echo "</details>"
            '';
          };
          npins = pkgs.writeShellApplication {
            name = "update-npins";
            runtimeInputs = with pkgs; [ npins ];
            text = ''
              echo "<details><summary>npins changes</summary>"
              # Needed because GitHub's rendering of the first body line breaks down otherwise
              echo ""
              echo '```'
              npins --directory "$1/npins" import-flake 2>&1
              echo  '```'
              echo "</details>"
            '';
          };
          # These steps have to be in the same script because order matters.
          # `carge upgrade` should happen before `cargo update` and then check
          # `cargo outdated` and `cargo audit` after that.
          cargo = pkgs.writeShellApplication {
            name = "cargo";
            runtimeInputs = with pkgs; [
              cargo
              cargo-audit
              cargo-edit # provides `cargo upgrade`
              cargo-outdated
            ];
            text = ''
              echo "<details><summary>cargo changes</summary>"
              echo ""
              echo "### cargo upgrade"
              printf "\n\`\`\`\n"
              # --incompatible allows jumping to the next major version.
              cargo upgrade --incompatible --manifest-path "$1/Cargo.toml" 2>&1
              printf "\n\`\`\`\n"

              echo "### cargo update"
              printf "\n\`\`\`\n"
              cargo update --manifest-path "$1/Cargo.toml" 2>&1
              printf "\n\`\`\`\n"

              echo "### cargo outdated"
              printf "\n\`\`\`\n"
              cargo outdated --manifest-path "$1/Cargo.toml" 2>&1
              printf "\n\`\`\`\n"

              echo "### cargo audit"
              printf "\n\`\`\`\n"
              cargo audit --file "$1/Cargo.lock" 2>&1
              printf "\n\`\`\`\n"
              echo "</details>"
            '';
          };
          githubActions = pkgs.writeShellApplication {
            name = "update-github-actions";
            runtimeInputs = with pkgs; [
              dependabot-cli
              jq
              github-cli
              coreutils
            ];
            text = builtins.readFile ./scripts/update-github-actions.sh;
          };
        };
      in
      pkgs.writeShellApplication {
        name = "auto-pr-update";
        text = ''
          # Prevent impurities
          unset PATH
          ${lib.concatMapStringsSep "\n" (script: ''
            echo >&2 "Running ${script}"
            ${lib.getExe script} "$1"
          '') (lib.attrValues updateScripts)}
        '';
      };

    # Tests the tool on the pinned Nixpkgs tree, this is a good sanity check
    nixpkgsCheck =
      pkgs.runCommand "test-nixpkgs-check-by-name"
        {
          nativeBuildInputs = [
            packages.build
            pkgs.nix
          ];
          nixpkgsPath = nixpkgs;
        }
        ''
          ${initNix}
          nixpkgs-check-by-name --base "$nixpkgsPath" "$nixpkgsPath"
          touch $out
        '';
  };
in
packages
// {

  # Good for debugging
  inherit pkgs;

  # Built by CI
  ci = pkgs.linkFarm "ci" packages;

  # Used by CI to determine whether a new version should be released
  inherit version;
}
