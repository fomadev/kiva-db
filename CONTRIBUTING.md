# Contributing to KivaDB

KivaDB is a high-performance database engine developed and maintained by FomaDev. We are active in its development and use it for critical internal infrastructure. While we are still refining our processes to make contributing as seamless as possible, we value the community's involvement in improving the engine.

This document clarifies the contribution process and answers common questions about how to get involved.

## Fordima Public License (FPL)

By contributing to KivaDB, you agree that your contributions will be licensed under the **[Fordima Public License (FPL)](LICENSE)**. 

Forks are authorized only for the purpose of contributing back to the official FomaDev repository via Pull Requests. Maintaining a permanent independent fork to bypass FomaDev's authority is strictly prohibited. For commercial use of the source code or integration into third-party products, please contact the lead maintainer for a commercial license.

## Code of Conduct

FomaDev expects all participants to adhere to a professional and respectful code of conduct. We expect contributors to act with integrity, technical excellence, and collaboration. Harassment or unprofessional behavior will not be tolerated.

## Open Development

All work on KivaDB happens directly on GitHub. Both the core team and external contributors follow the same process: submitting Pull Requests that go through a rigorous code review.

## Branch Organization

Submit all changes directly to the `main` branch. We strive to keep `main` in a stable, releasable state. All code submitted must pass existing tests and be compatible with the latest stable version.

## Bugs

### Finding Known Issues
We use GitHub Issues to track public bugs. Before filing a new issue, please search the existing ones to ensure the problem has not been reported yet.

### Reporting New Issues
The most effective way to get a bug fixed is to provide a reduced test case. Please include:
1. Your operating system (Windows/macOS/Linux).
2. The version of KivaDB you are using.
3. A minimal set of commands to reproduce the bug.

## Proposing Changes

If you intend to modify the public API, the wire protocol or make non-trivial changes to the storage engine or indexing logic, we recommend opening an issue first. This allows us to reach an agreement on the design before you invest significant effort.

## Sending a Pull Request

The core team monitors Pull Requests regularly. We will review your code and either merge it, request changes, or close it with an explanation.

### Pull Request Checklist
Before submitting, please ensure:
1. You have forked the repository and created your branch from `main`.
2. Your code is formatted with `cargo fmt` and passes `cargo clippy --all-targets` without warnings.
3. You have updated the documentation if you added new commands or flags.
4. You have added tests for your change and `cargo test` passes.

### Development Workflow
KivaDB is built with Cargo.
- To compile the project: `cargo build --release`
- To run the test suites: `cargo test`
- To try the shell on a throwaway database: `cargo run -- --home ./scratch-home`

### Continuous integration
Every push and pull request runs `.github/workflows/build.yml` on Windows, Linux and macOS: `cargo clippy --all-targets -- -D warnings`, `cargo test`, and the Node.js driver tests (`cd drivers/node && npm test`, on Linux). A pull request is merged only when all three systems are green.

### Releasing (maintainers)
1. Update the version in `Cargo.toml`, the Docker tag in `docker-compose.yml` and the documentation. The Node.js driver (`drivers/node/package.json`) has a version of its own, raised only when the driver changes, following semver.
2. Commit, push, and wait for the three systems to be green.
3. Tag the version and push the tag:
   ```bash
   git tag v1.6.0
   git push origin v1.6.0
   ```
The workflow then publishes a GitHub release with the Windows, Linux and macOS executables and their SHA-256 checksums, and pushes the Docker image `fomadev0/kiva-db:<version>` and `:latest`. Docker Hub access comes from the repository secrets `DOCKER_USERNAME` and `DOCKER_PASSWORD` (a Docker Hub access token, never a password).

## Style Guide

Consistency is key to a maintainable database engine.
- **Storage engine** (`src/engine.rs`, `format.rs`, `hint.rs`): any change to the on-disk format must stay readable by existing files, or bump the format number and say so in the pull request.
- **Command language** (`src/exec.rs`, `exec_admin.rs`): every new command needs an entry in `help`, in DOCUMENTATION.md (its chapter and the cheat sheet) and in the privilege table (`required`).
- **Errors**: return `KivaError` values; never panic on user input or on corrupted data.
- **Formatting**: `cargo fmt`. Use descriptive names and comment the reasoning, not the mechanics.

## Contributor License Agreement (CLA)

For us to accept your Pull Request, you must acknowledge that you have read and accepted the **[FPL](LICENSE)**. By submitting a Pull Request, you certify that you are the author of the code or have the right to contribute it under the [Fordima Public License](LICENSE).

## Contact

For technical discussions, feature requests, or commercial licensing inquiries, please contact:
- **Lead Maintainer**: [github.com/fordimalanda](https://github.com/fordimalanda)
- **GitHub**: [github.com/fomadev](https://github.com/fomadev)

Thank you for helping us improve KivaDB.