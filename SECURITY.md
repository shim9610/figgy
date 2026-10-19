# Security policy

## Report a vulnerability privately

Use [GitHub's private vulnerability reporting form](https://github.com/shim9610/figgy/security/advisories/new)
for suspected vulnerabilities in figgy. Please do not disclose an unpatched
vulnerability in a public issue or pull request.

Include the affected package with its crates.io version, or the commit hash for a
Git checkout, because master can differ from the published release that carries
the same version number. Also include a minimal reproducer, the expected security
impact, and the steps needed to trigger it. For GPU-related reports, include the
OS, backend, adapter and driver version when available.
Use synthetic data and remove credentials and personal or proprietary data from
logs and attachments.

The maintainer will use the private report to discuss reproduction, a fix and
disclosure. Response and release times depend on maintainer availability; there
is no guaranteed response deadline.

## Supported code

Security fixes target the current default branch and the latest crates.io release
of `figgy-renderer` and `figgy-model`. Older releases and pinned Git revisions do
not have separate maintenance branches. Reports affecting older versions are still
welcome; please state whether the issue also reproduces on the current code.

This repository contains the `figgy-model` and `figgy-renderer` Rust libraries and
their examples, including the WebAssembly integration example. It does not
operate or distribute the separate figgyplot.com Studio application. If the
affected component is unclear, describe the application and the renderer revision
in the private report so the scope can be determined.

Rendering defects without a security impact belong in the
[public bug tracker](https://github.com/shim9610/figgy/issues/new/choose).

## Automated checks

Dependabot alerts track known dependency vulnerabilities. Secret scanning and
push protection help detect accidentally committed credentials. CodeQL analyzes
public, non-draft pull requests; see [CI coverage](ci/README.md#security-analysis).
These checks supplement review and tests; they do not guarantee that the code is
free of vulnerabilities.
