# Changelog

Notable changes to macho, following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- System fetch: user and host, OS, kernel, uptime, packages (pacman, dpkg, apk, xbps,
  flatpak, nix), shell, window manager, terminal, CPU and memory, read without
  spawning processes.
- Half-block pixel logos in the terminal's own palette, chosen from `os-release`
  `ID`/`ID_LIKE` or with `--logo`, and listed with `--list-logos`.
- `NO_COLOR` support.
- A static binary that links neither the standard library nor libc, for x86-64 and
  aarch64 Linux.
- Logos are drawn with as few colour changes as their pixels allow, flipping half-block
  glyphs instead of switching colours, which cuts the output by 9–25%.

[Unreleased]: https://github.com/RossAnder/macho/commits/master
