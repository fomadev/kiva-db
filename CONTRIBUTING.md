# Contributing to KivaDB

KivaDB is a high-performance database engine developed and maintained by FomaDev. We are active in its development and use it for critical internal infrastructure. While we are still refining our processes to make contributing as seamless as possible, we value the community's involvement in improving the engine.

This document clarifies the contribution process and answers common questions about how to get involved.

## FomaDev Public License (FPL)

By contributing to KivaDB, you agree that your contributions will be licensed under the **[FomaDev Public License (FPL)](LICENSE)**. 

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

If you intend to modify the public API (kivadb.h) or make non-trivial changes to the storage engine or indexing logic, we recommend opening an issue first. This allows us to reach an agreement on the design before you invest significant effort.

## Sending a Pull Request

The core team monitors Pull Requests regularly. We will review your code and either merge it, request changes, or close it with an explanation.

### Pull Request Checklist
Before submitting, please ensure:
1. You have forked the repository and created your branch from `main`.
2. Your code adheres to the project's style (C11 for core, C++17 for CLI/Index).
3. You have updated the documentation if you added new commands or flags.
4. You have verified your changes by running the build process.

### Development Workflow
KivaDB uses a `Makefile` for its build system.
- To compile the project: `make`
- To clean build artifacts: `make clean`
- To test the shell: `./kivadb`

## Style Guide

Consistency is key to a maintainable database engine.
- **C Core**: Follow a strict procedural style with clear manual memory management. Use `kiva_` prefixes for public API functions.
- **C++ CLI**: Use modern C++17 features where appropriate, but keep the interface with the C core through `extern "C"` blocks.
- **Formatting**: Keep indentation consistent (4 spaces). Use descriptive variable names.

## Contributor License Agreement (CLA)

For us to accept your Pull Request, you must acknowledge that you have read and accepted the **[FPL](LICENSE)**. By submitting a Pull Request, you certify that you are the author of the code or have the right to contribute it under the [FomaDev Public License](LICENSE).

## Contact

For technical discussions, feature requests, or commercial licensing inquiries, please contact:
- **Lead Maintainer**: [github.com/fordimalanda](https://github.com/fordimalanda)
- **GitHub**: [github.com/fomadev](https://github.com/fomadev)

Thank you for helping us make KivaDB the highest-performing storage engine for the technology ecosystem.