# Runtime and installed-artifact checks

Use this guide when qualifying real inference or a packaged application. Ordinary
mock/transport tests cannot establish model quality, native GUI behavior or host
acceptance. Old cutover receipts remain in local archives/Git history and apply
only to their exact binaries, models and environment.

## Safe setup and reporting

Use disposable configuration, application, data and registration directories.
Never point a service-install test at the real user manager or persist a fixture
path into normal registration. Custom `--unit-dir` tests render definitions; they
do not establish actual manager behavior. Native registration/login checks need
a disposable OS account or VM.

Record source commit, exact artifact/model/runtime hashes, platform/host versions,
fixture, command and observed outcome in run artifacts or an issue/PR. State failed,
ignored and unavailable checks honestly. A different candidate cannot inherit an
older run's success by editing version metadata. Keep book content, credentials and
browser grants out of shared evidence.

## Final packaged inference and MCP

Use a verified disposable assembled payload, including its own pinned model/runtime:

```sh
HIERO_DESKTOP_INSTALLED_CLI=/absolute/disposable/payload/hiero \
CARGO_BUILD_JOBS=2 cargo test -p hiero --locked --test desktop_native -- --ignored --nocapture
```

The fixture runs real inference, authenticated MCP and shutdown. Missing inputs
fail. It does not qualify login, panel/tray, native manager, user-facing language
quality or a writing agent's actual workflow.

## Linux real upgrade, rollback and uninstall

The ignored `desktop_update` fixture requires a complete actual 0.8.0 baseline
version directory and a verified final split release directory:

```sh
HIERO_DESKTOP_BASELINE_DIR=/absolute/retained/0.8.0 \
HIERO_DESKTOP_RELEASE_DIR=/absolute/final/split-release \
CARGO_BUILD_JOBS=2 cargo test -p hiero --locked --test desktop_update -- --ignored --nocapture
```

It exercises disposable offline bootstrap, compiled-version upgrade/rollback and
uninstall preservation. It is Linux-only and does not substitute for live desktop
manager acceptance. Inspect the test's documented input layout before running;
never relabel one binary as two releases to satisfy it.

## Provider and semantic checks

The repository also has explicitly ignored real-model/provider tests. Find their
current fixture requirements in the owning test before selecting one; do not run
all ignored tests against a normal installation. Supply credentials securely and
use synthetic text unless a corpus disclosure is explicitly authorized.

Jev synthetic relevance/comparison probes are described in
[hook context](agent-hook-context.md) and [the adapter](jev-sdk-batching.md).
Live provider tests are local-only; CI uses mocks and disposable loopback servers.
A transport success is not calibrated literary accuracy. [Semantic validation](semantic-validation.md)
distinguishes storage agreement from text/query relevance.

## Broader checks and release consequences

Use [desktop checks](desktop-qualification.md) for visible native behavior and
[host checks](agent-host-acceptance.md) for actual MCP-and-skills workflows.
The normal contributor checklist remains in [AGENTS.md](../AGENTS.md#verification).

Only open P0 issues block a release; optional native evidence and non-P0 failed/unrun
checks are warnings with actionable issues/run links. This does not waive archive
integrity, data ownership or truthful runtime readiness. Reuse unchanged retained
binaries and rerun the affected check instead of restarting a full native build.
