# Security

OpenIRC talks to servers and to other people's clients (DCC), so parsing,
file handling, TLS, and what is shown or saved from untrusted text are
all treated as security-relevant.

## Reporting a problem

Please report a vulnerability through GitHub's private vulnerability
reporting on this repository (Security, Report a vulnerability) rather
than in a public issue. You will get an answer within a week, and a fix
will be released before the details are published.

## What is in place

- `scripts/check.sh`, run by CI on every push and pull request: formatting,
  clippy with warnings as errors for Linux and Windows, the tests under a
  memory cap, `cargo audit` and `cargo deny` against the RustSec database,
  and benchmarks compared with a baseline.
- A weekly scheduled run, so a new advisory shows up without a push.
- Dependabot alerts and security updates for Cargo and the workflow
  actions; CodeQL scanning of the sources and workflows.
- `REVIEW.md` records every review finding and its fix, with the test that
  covers it.
