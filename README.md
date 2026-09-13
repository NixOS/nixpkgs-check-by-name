# Nixpkgs vet (check) tool

This repository implements a program  which checks Nixpkgs for many sorts of defects, both major and minor. It's named `nixpkgs-vet`, since it '[vets](https://en.wiktionary.org/wiki/vet#Verb)' contributions to Nixpkgs.

It implements all checks for [Nixpkgs' `pkgs/by-name` directory](https://github.com/NixOS/nixpkgs/tree/master/pkgs/by-name) as part of [RFC 140](https://github.com/NixOS/rfcs/pull/140).

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for contributor documentation. Below is the user documentation.

Currently the only intended user for this program is [Nixpkgs](https://github.com/NixOS/nixpkgs).
Expect that the interface may be changed in breaking ways as long as Nixpkgs is adjusted to deal with it.
See [the `pkgs/by-name` Readme](https://github.com/NixOS/nixpkgs/blob/master/pkgs/by-name/README.md#validation) for how it's used in Nixpkgs.

## Nix derivations

The source code contains a `default.nix` file, which defines a Nix function.

The function takes an attribute set with at least these attributes as its argument:
- `system` (String, defaults to [`builtins.currentSystem`](https://nix.dev/manual/nix/stable/language/builtin-constants#builtins-currentSystem)):
  The [`system`](https://nixos.org/manual/nix/stable/language/derivations#attr-system)
  to build the resulting derivation with.

The function returns an attribute set with at least these attributes:
- `build` ([Package attribute set](https://nixos.org/manual/nix/stable/glossary#package-attribute-set)):
  A derivation that can be built with the given `system`.

There is no guarantee that the derivation succeeds on systems that don't have [prebuilt store paths](#prebuilt-store-paths),
but it can be attempted with

```bash
nix-build https://github.com/NixOS/nixpkgs-vet/tarball/master -A build
```

## Prebuilt store paths

The [GitHub releases](https://github.com/NixOS/nixpkgs-vet/releases)
contain a [gzip](https://en.wikipedia.org/wiki/Gzip)-compressed
[Nix Archive](https://nixos.org/manual/nix/stable/command-ref/nix-store/export.html)
of the [build closure](https://nixos.org/manual/nix/stable/glossary#gloss-closure)
of the [Nix derivation](#nix-derivations) with `x86_64-linux` as the `system`.

This release artifact is named `x86_64-linux.nar.gz`
and can be imported into a local Nix store using:

```bash
storePath=$(gzip -cd x86_64-linux.nar.gz | nix-store --import | tail -1)
# To prevent it from being garbage-collected
nix-store --realise "$storePath" --add-root result
```

Compared to building the [Nix derivations](#nix-derivations),
this has the benefit that no Nix evaluation needs to take place
and is therefore much faster and less storage intensive.

## Binary interface

The store path acquired from the above methods contains
a `system`-specific binary under `$storePath/bin/nixpkgs-vet`.

The public interface of this binary is printed by calling
```bash
result/bin/nixpkgs-vet --help
```

## Validity checks

The following checks are performed when calling the binary:

### File structure checks
- `pkgs/by-name` must only contain subdirectories of the form `${shard}/${name}`, called _package directories_.
- The `name`'s of package directories must be unique when lowercased.
- `name` is a string only consisting of the ASCII characters `a-z`, `A-Z`, `0-9`, `-` or `_`.
- `name` must not start with a digit or `-`.
- `shard` is the lowercased first two letters of `name`, expressed in Nix: `shard = toLower (substring 0 2 name)`.
- Each package directory must contain a `package.nix` file and may contain arbitrary other files.

### Nix parser checks
- Each package directory must not refer to files outside itself using symlinks or Nix path expressions.

### Nix evaluation checks

Evaluate Nixpkgs with `system` set to `x86_64-linux` and check that:
- For each package directory, the `pkgs.${name}` attribute must be defined,
  which the `pkgs/by-name` overlay does automatically.
- For each package directory, `pkgs.lib.isDerivation pkgs.${name}` must be `true`.

### Ratchet checks

Furthermore, this tool implements certain [ratchet](https://qntm.org/ratchet) checks.
This allows gradually phasing out deprecated patterns without breaking the base branch or having to migrate it all at once.
It works by not allowing new instances of the pattern to be introduced, but allowing already existing instances.
The existing instances are coming from `<BASE_NIXPKGS>`, which is then checked against `<NIXPKGS>` for new instances.
Ratchets should be removed eventually once the pattern is not used anymore.

The current ratchets are:

- New top-level packages defined using `pkgs.callPackage` must be defined with a package directory.
  - Once a top-level package uses `pkgs/by-name`, it also can't be moved back out of it.
- New top-level packages must evaluate with `strictDeps = true`.
  - Once a top-level package evaluates with `strictDeps = true`, it also can't regress to `false`.
- New top-level packages must evaluate with `__structuredAttrs = true`.
  - Once a top-level package evaluates with `__structuredAttrs = true`, it also can't regress to `false`.
- New NixOS test modules passed by path to `runTest` or `runTestOn` must not take the obsolete `pkgs` module argument.
  - Once a NixOS test module stops taking `pkgs`, it also can't regress to taking it.
  Use `config.node.pkgs` for guest packages or `hostPkgs` for host packages.
