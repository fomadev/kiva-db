# KivaDB 2.0 — releasing

What to check before each tag of 2.0, and what the tag does. Nothing here runs by itself: a tag is pushed by the maintainer, when the maintainer decides.

## A release candidate (`v2.0.0-rc.N`)

Before the tag:

1. `version = "2.0.0-rc.N"` in `Cargo.toml` (and `Cargo.lock` rebuilt). The release stops if the tag and this version differ.
2. `main` is green on Windows, Linux and macOS, with `KIVADB_V1_BIN` (the real 1.7.1) on Linux.
3. `CHANGELOG.md` has its section `## Unreleased (2.0.0)`, with its `### Known limits`: the notes of the candidate are taken from it.
4. The upgrade has been tried on a copy of a real 1.7.1 home, following [Chapter 33](../../DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20).

What the tag does (`.github/workflows/build.yml`):

* the executables for Windows, Linux and macOS, and a GitHub **pre-release** with them, `SHA256SUMS.txt` and the notes;
* the Docker image `fomadev0/kiva-db:2.0.0-rc.N`, and not `latest`;
* nothing else: `kivadb update`, the install scripts and `latest` keep 1.7.1. A candidate is installed on purpose: `KIVADB_VERSION=2.0.0-rc.N` for the install scripts, or the archive of the pre-release.

## The release (`v2.0.0`)

1. `version = "2.0.0"` in `Cargo.toml`.
2. `## Unreleased (2.0.0)` renamed `## 2.0.0 - <date>` in `CHANGELOG.md`.
3. The version named in the manual and the READMEs updated: the examples of `serve`, `update`, `doctor` and `stats`, the Docker tags ([29.6](../../DOCUMENTATION.md#296-docker)), `docker-compose.yml`, and [Chapter 32](../../DOCUMENTATION.md#chapter-32--limits-and-scope).
4. A version of [Kiva Studio](https://github.com/fomadev/kiva-studio) that knows 2.0 released, since the upgrade guide points to it.
5. The Python driver: published on PyPI, or still installed from the repository, as the maintainer decides; [24.4](../../DOCUMENTATION.md#244-python) says which.
6. This folder (`docs/v2`) archived or removed, as [IMPLEMENTATION_MATRIX.md](IMPLEMENTATION_MATRIX.md) says.

The tag then publishes the release, `latest` on Docker Hub, and what `kivadb update` installs: from then on, a 1.7.1 that runs `kivadb update` moves to 2.0.
