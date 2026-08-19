<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Contributing to luminate

Hello! **Thank you** for your interest in luminate. Contributions are always
appreciated. Whether you are fixing a small typo, documenting hardware,
improving a test, or making something new, your contribution is welcome.

You don't need to know the whole codebase before getting involved. Aim for a
focused change, explain the reasoning behind it, and ask questions when the
project's intent is unclear.

Participation in luminate is governed by our [Code of Conduct](CODE_OF_CONDUCT.md);
please help us keep the project a kind, constructive place to work together.
Please report suspected vulnerabilities according to the private process in
our [Security Policy](SECURITY.md), not in a public issue.

## A note on language

luminate uses Oxford English in its APIs, documentation, diagnostics, and
user-facing text. That means spellings such as `colour` and `behaviour`, along
with Oxford `-ize` forms such as `normalize` and `initialize`.

There is no need to rewrite terminology that belongs to a programming language,
dependency, standard, or external protocol. Preserve those spellings, along
with identifiers and quoted text owned by other projects.

## Licensing and signing off your work

The luminate daemon and official plugins are licensed under `GPL-3.0-or-later`.
libluminate and the crates compiled into it (luminate-core,
luminate-host-supervisor, luminate-platform, and luminate-protocol) are
licensed under `LGPL-3.0-or-later`. All documentation is licensed under
`CC-BY-SA-4.0`.

By contributing, you license your contribution under the licence of the
component and additionally grant the project permission to redistribute it
under GPL, LGPL, or CC-BY-SA-4.0 as necessary.

We use the [Developer Certificate of Origin
1.1](https://developercertificate.org/), usually called "the DCO." Signing off
a commit confirms that you created the work or otherwise have the right to
contribute it. Every commit needs a `Signed-off-by` trailer. Git can add one for
you:

```sh
git commit --signoff
```

Please use the primary name you use, and an email address by which you can be
identified:

```text
Signed-off-by: Ada Example <ada@example.com>
```

This sign-off records the provenance of the contribution; it is not the same as
cryptographically signing a commit. If you amend or rebase your work, take a
moment to check that every resulting commit still has its sign-off.

### Per-file licence and copyright notices

Project-authored source, documentation, configuration, packaging, example, and
test files carry machine-readable SPDX notices at the first line that can
contain a comment. A shebang or format-required preamble remains ahead of the
notices. New files should use:

```text
SPDX-License-Identifier: GPL-3.0-or-later
SPDX-FileCopyrightText: YEAR COPYRIGHT HOLDER
```

_Note: Change this to `LGPL-3.0-or-later` for code in libluminate or one of
the crates compiled into it (luminate-core,
luminate-host-supervisor, luminate-platform, luminate-protocol), or
`CC-BY-SA-4.0` for documentation. Examples under an LGPL crate's `examples/`
directory stay `GPL-3.0-or-later`, since they are consumer code rather than
part of the linked library._

Use the comment syntax appropriate to the file. Name the initial author as the
copyright holder when they own the work; if an employer, client, or other party
owns it instead, name that party. Do not copy an existing holder's name onto
work they did not own.

A contributor who adds substantial, independently copyrightable material that
they own may add another `SPDX-FileCopyrightText` line. Routine fixes, reviews,
formatting, mechanical changes, and small edits do not normally call for a new
notice. Contributors should add their own notice when appropriate rather than
asking a maintainer to infer ownership. Git history and DCO sign-offs remain
the authoritative contribution record, not the list of notices in a file.

Preserve notices on imported or vendored material. Generated files should
receive their notices from the generator or template so regeneration does not
discard them; do not claim copyright in upstream material merely because it is
checked into this repository.

## Why these guidelines are strict

luminate is a daemon that crosses boundaries between processes, protocols, FFI,
the operating system, and physical hardware. Small mistakes can become crashes,
security problems, corrupted state, incompatible clients, unnecessary device
writes, or behaviour that is difficult to reproduce without particular
hardware. The guidelines below make those risks visible before a change is
merged.

Consistent code is also easier to read, review, test, and maintain. Predictable
error handling, documentation, naming, and control flow reduce the amount of
local convention every contributor must rediscover. Focused changes and
deterministic tests help reviewers understand the important parts quickly,
which improves both review quality and contributor turnaround. Clear ownership
of generated data and compatibility boundaries prevents avoidable rework.

luminate's project style is consciously influenced by the Zen of Python, but
translated into Rust rather than copied literally. Readability counts;
explicit behaviour is preferable to hidden behaviour; simple designs are
preferable to needlessly clever ones; and errors should not disappear silently.
In Rust, those ideas usually mean strong domain types, explicit `Result`
handling, exhaustive matches, small composable units, conventional ownership,
and abstractions that clarify both cost and invariants.

Two linked principles are especially relevant because lighting hardware is
full of exceptions: special cases are not special enough to break the rules,
although practicality beats purity. Model unusual devices through the common
capability and state vocabulary wherever it remains truthful. When real
hardware cannot fit that model without distortion, prefer a narrow, documented,
and tested exception over either pretending the difference does not exist or
weakening the general rules for every device.

In the face of ambiguity, refuse the temptation to guess. If a protocol or
device does not establish an identity, capability, state, or guarantee, preserve
that uncertainty or return a useful error rather than manufacturing an answer.
Optimistic guesses about hardware tend to become compatibility promises and
unsafe writes later.

There should be one—and preferably only one—obvious way to do it, although that
may not be obvious at first unless you're from the Pacific Northwest. Prefer
canonical domain types, constructors, validation paths, and conversions over
parallel helpers with subtly different semantics. When more than one path is
genuinely needed, make the distinction explicit in names and documentation.

Strict does not mean inflexible. These rules are defaults chosen to reduce bugs
and cognitive overhead while keeping the software secure and efficient. When a
different approach is genuinely clearer, safer, or measurably better, explain
the tradeoff close to the code and in the change description. A well-reasoned,
narrow exception is preferable to forcing a rule where it does not fit.

## Planning a change

Keep contributions focused. Avoid mixing a behavioural change with unrelated
refactoring, formatting, dependency updates, or generated-file churn. Large
architectural changes, new public interfaces, and compatibility breaks are best
discussed before substantial implementation work begins.

Prefer changes that make invalid states difficult to represent. Use domain
types and validated constructors instead of passing loosely related primitive
values through the system. Preserve useful error sources and attach actionable
context instead of flattening errors into strings.

### Compatibility and public interfaces

Treat the Rust and C client APIs, CLI machine-readable output, D-Bus object
paths and interfaces, persisted state, daemon wire protocol, event protocol,
and plugin ABI as compatibility-sensitive. A type may be internal to the
workspace while its serialized representation is still an external contract.

Review additions, removals, renames, enum changes, numeric values, defaults,
and serialization formats for compatibility impact. When appropriate, update
the corresponding `PROTOCOL_ABI_VERSION`, `EVENT_PROTOCOL_VERSION`,
`PLUGIN_ABI_VERSION`, or persistence format version and provide a migration or
clear incompatibility path. Document intentional compatibility changes in the
change description and in the relevant architecture or user documentation.

### Dependencies and supported Rust version

Use existing workspace facilities when they are a clear fit; a small amount of
straightforward code may be preferable to a new dependency. New dependencies
should have a clear purpose, compatible licence, suitable maintenance posture,
and acceptable impact on build time and packaging.

luminate's minimum supported Rust version is declared in `Cargo.toml`. Do not
raise it accidentally. Keep `Cargo.lock` changes focused and include them when
changing dependencies. For dependency changes, run `cargo audit` when
available and report any unresolved advisory relevant to the change.

`rust-toolchain.toml` pins the contributor and default CI toolchain, including
rustfmt and Clippy, so formatting and lint results are reproducible. This is
newer than the minimum supported version by design; the separate MSRV CI job
continues to build the workspace with the version declared in `Cargo.toml`.

## Rust style and safety

`cargo fmt` is the authority on Rust formatting. Beyond formatting, the
workspace enables Clippy's `all` and `pedantic` groups plus a selection of
restriction lints. The complete configuration in [`Cargo.toml`](Cargo.toml)
and [`clippy.toml`](clippy.toml) is the source of truth. The sections below
cover the rules most likely to affect the shape of a contribution.

### Error handling and panics

Production code, including error paths, should not panic. Return or propagate
an error instead of using `unwrap`, `expect`, `panic!`, or unchecked indexing.
Prefer `?`, a deliberate `match`, or a checked accessor that lets the caller
decide how to handle failure. Functions that already return `Result` must not
hide a possible failure behind an unwrap.

The only exception to this policy: locks, unless on the C side of the FFI
boundary, should be unwrapped or expected upon. A poisoned lock means that
a panic occurred in another thread, and variants may be violated.

Tests are exempt from these panic-safety lints: a test may use `unwrap`,
`expect`, `panic!`, or indexing when an unmet assumption should fail the test
immediately. That exemption does not extend to production code merely because a
test exercises it.

Do not leave `todo!` or `unimplemented!` in submitted code. Document public
fallible APIs with a `# Errors` section, and document any intentionally
panicking public API with a `# Panics` section. The core and client libraries
also deny missing public API documentation generally.

### Unsafe code

`unsafe` code is denied by default and should remain confined to necessary FFI
boundaries. Any exception needs a narrow, reasoned `allow`, small unsafe blocks,
and the safety documentation and comments required by the lints. Prefer a safe
abstraction at the boundary so the rest of the codebase does not need to reason
about raw pointers or other unsafe invariants.

### Diagnostics and output

Use `tracing` for application and plugin diagnostics instead of `println!`,
`eprintln!`, or `dbg!`. Intentional user-facing CLI and example output is a
narrowly scoped exception; keep any corresponding lint allowance close to that
output.

### Documentation and comments

Every Rust source file, including binaries, examples, and integration tests,
should begin with crate- or module-level documentation describing its role.
Use `//!` comments for modules and crate roots, or an equivalent crate-level
`#![doc = ...]` attribute. A useful introduction tells the reader what belongs
in the module or explains context that its filename alone cannot provide.

Keep code reasonably well-commented. Comments are most valuable when they
record why a decision was made, an invariant that must be preserved, a safety
argument, a protocol or hardware constraint, or a surprising edge case. Do not
narrate straightforward code or restate names and types. Clearer names and
smaller functions are usually better in those cases. Keep comments current when
the code they describe changes.

### Design and readability

Keep functions and modules focused enough that their responsibilities are easy
to state. Prefer direct control flow and early returns when they reduce nesting.
Choose names from luminate's domain vocabulary and favour clear, unsurprising
code over compressed or clever expressions. When a comment is needed to
explain what an expression does, first consider whether a named helper or type
would communicate the intent better.

### General Rust style

Prefer explicit, portable APIs. The lints reject or discourage lossy numeric
conversions, ambiguous operator precedence, string slicing, assumptions about
hash iteration order, and hand-written substitutes for purpose-built
filesystem APIs. They also favour simple imports, paths, patterns, and modules:
avoid long absolute paths, wildcard enum fallbacks, redundant type annotations,
and fully qualified names when an import is clearer.

These rules apply to every workspace crate and plugin. Running the full Clippy
command below is the best way to catch rules relevant to a particular change.

### Lint suppressions

Treat lint warnings as useful design feedback. Sometimes a suppression really
is the clearest choice, but it should be exceptional, narrowly scoped, and easy
for the next person (including future you) to understand. Do not add an `allow`
just to make a warning disappear. Include a specific reason close to every new
suppression, and prefer fixing the underlying code when practical. For example:

```rust
#[allow(
    clippy::too_many_lines,
    reason = "This function enumerates all members of Foo enum"
)]
```

Blanket `allow`s over an entire file require much more justification than a
targeted one. Please use whole-file `allow`s sparingly, if at all.

## C style

`clang-format` is the authority on formatting for the hand-written C and C++
consumer code under `crates/libluminate/tests/c` and
`crates/libluminate/examples`, using the `.clang-format` at the repository
root. Run it before submitting a change that touches those files:

```sh
clang-format -i crates/libluminate/tests/c/*.c crates/libluminate/tests/c/*.cpp \
  crates/libluminate/examples/*.c
```

This does not apply to `crates/libluminate/luminate.h`, which is generated by
cbindgen; see [Generated files](#generated-files).

## Testing expectations

Add a regression test for a bug fix when practical. Tests should exercise
observable behaviour rather than incidental implementation details, and their
names should state the behaviour or condition they cover. Do not weaken an
assertion merely to make a failing test pass.

Keep ordinary tests deterministic and hermetic: avoid real hardware, external
network services, wall-clock timing assumptions, execution-order dependencies,
and global machine state. Tests that necessarily require hardware, network
access, installed artefacts, elevated privileges, or unusual host integration
must be explicitly gated or ignored with a reason and a documented way to run
them.

### libluminate coverage

Run libluminate's combined Rust and C coverage workflow with:

```sh
scripts/coverage.sh --libluminate
```

The script requires `cargo-llvm-cov` and `jq`. It builds the instrumented Rust
test executable and `libluminate.so` in the same LLVM coverage target directory,
runs the Rust tests and every C/C++ consumer, and verifies that both coverage
objects exist. Reports are written beneath
`target/llvm-cov-target/libluminate-report/`.

The summary reports production-source coverage and the separately excluded test
source inventory. Files named `*_tests.rs` and Rust integration-test sources are
test sources; `cargo-llvm-cov` excludes them from its report by default. Other
files beneath `crates/libluminate/src/` are production sources. This keeps test
implementation lines from inflating either the covered-line count or its
denominator. A run is not valid if a C consumer fails, either instrumented
object is missing, or LLVM cannot merge any raw profile.

The Rust test executable and shared library contain a few identical exported
functions compiled under different test configurations, so LLVM may report a
small number of mismatched function records while combining them. The script
uses the test executable as the primary object and the shared library as an
explicit secondary object; the summary is valid when profile merging succeeds
and both named objects are present.

Read the production percentage as the coverage of shipped Rust source, not as
directly comparable with older reports that counted inline tests in the same
files. When reviewing a change, compare covered production lines as well as the
percentage: moving code or changing the denominator must not disguise a loss of
executed behaviour. Eighty per cent is a useful soft floor, but deterministic
tests of observable behaviour take precedence over percentage-only assertions.

### `luminated` coverage

Run the daemon and its internal Rust dependency closure with:

```sh
scripts/coverage.sh --luminated
```

The script derives the workspace-local dependency closure from Cargo metadata,
runs its all-feature tests, and adds the hermetic process, shared-memory, CLI,
plugin, and private D-Bus scenarios that exercise the daemon boundary. It
measures production Rust sources only and fails below 85% line coverage.
Reports are written beneath `target/llvm-cov-target/luminated-report/`.

Failure-path scenarios can terminate a deliberately wedged instrumented child
before LLVM finishes its profile. The script checks each generated raw profile
before the aggregate merge and retains malformed profiles under
`luminated-report/invalid-profraw/` for diagnosis; valid sibling process data
is still included.

### Workspace coverage

Run the combined workspace coverage workflow with either command:

```sh
scripts/coverage.sh
scripts/coverage.sh --workspace
```

This runs the ordinary all-feature workspace tests followed by the hermetic
ignored suites which exercise supervised processes, local network peers, and a
private D-Bus session. It does not opt in to Alienware hardware writes. The
report includes the plugin and policy shared objects loaded by those tests and
fails when production line coverage falls below 80%. Reports are written under
`target/llvm-cov-target/workspace-report/`.

## Protocol and hardware contributions

Cite specifications, upstream implementations, captures, or direct
observations used to support protocol and hardware behaviour. Distinguish
documented facts, observed behaviour, and inference. Record the
exact hardware models and firmware revisions tested, along with important paths
that could not be tested.

Sanitize packet captures, fixtures, logs, and examples. Remove credentials,
network details, serial numbers, MAC addresses, stable device identifiers, and
other personal or device-specific data unless a value is intentionally public
and necessary. Do not contribute proprietary material or captures you do not
have the right to redistribute.

## Generated files

Do not edit generated artefacts as though they were primary sources. Change the
generator, template, source data, or configuration and regenerate the output.
For example, changes to `crates/libluminate/luminate.h` originate in the Rust
FFI surface or `crates/libluminate/cbindgen.toml`; regenerate it with:

```sh
cbindgen --config crates/libluminate/cbindgen.toml \
  --crate libluminate \
  --output crates/libluminate/luminate.h
```

Commit generated output when the repository tracks it, and keep its provenance,
source version or hash, and SPDX notices reproducible from the generation
process.

## Before you submit a change

Please make sure your change builds cleanly and passes the project's formatting,
lint, and test checks. These are the usual commands to run before submitting:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

After a major API or ABI change, run the repository's full local major-change
workflow as well:

```sh
scripts/major-check.local.sh
```

This includes the ordinary workspace checks plus process integration and
conformance tests, the dependency audit, generated-header verification,
packaging builds, and packaging lifecycle smoke tests. Report any check that
the host cannot run and why.

Run the smoke test for the current platform locally:

```sh
scripts/smoke-test-linux.sh
scripts/smoke-test-macos.sh
```

```powershell
scripts/smoke-test-windows.ps1
```

These local entry points run independently useful platform phases and do not
alter machine-wide service state unless the privileged option is supplied.
Maintainers with the disposable macOS and Windows libvirt guests can instead
run `scripts/vm-smoke-test-macos.sh` and
`scripts/vm-smoke-test-windows.sh`. Each VM runner copies the current workspace
over SSH, invokes the same platform-local smoke recipe with service tests
enabled, and shuts its guest down gracefully even after a failed check. Run
them sequentially on hosts which do not have enough memory for both guests.
The local and VM interfaces, prerequisites, test matrix, D-Bus support tiers,
and recovery behaviour are documented in the
[cross-platform testing guide](docs/development/cross-platform-testing.md).

Depending on what you touched, there may also be relevant packaging,
integration, hardware, or documentation checks elsewhere in the repository. If
you cannot run a hardware-specific check, say so when submitting the change and
in any relevant documentation; an honest account of what was and was not tested
is useful.

Keep commits reviewable and logically coherent, with messages that explain the
reason for the change. When submitting, summarize the problem and approach,
call out compatibility and security implications, link relevant issues or
research, list the exact checks performed, and identify anything that remains
untested. Update user, API, protocol, and architecture documentation alongside
the behaviour it describes.

### Continuous Integration

Contributions also need to pass every required CI check. If a check behaves
unexpectedly, please call it out rather than wrestling with it in silence.
