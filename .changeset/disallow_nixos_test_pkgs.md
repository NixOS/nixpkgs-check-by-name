---
default: minor
---

# Disallow new pkgs arguments in NixOS tests

NixOS test modules passed by path to `runTest` or `runTestOn` may no longer introduce the obsolete `pkgs` module argument.
Use `config.node.pkgs` for guest packages or `hostPkgs` for host packages.
