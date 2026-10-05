# Using Hieronymus

Install Hieronymus, connect your writing agent, and keep working in that agent.
Use the local web console when you want to inspect memory or change settings.

## Install and connect

1. Use the [installer for your platform](../README.md#install). Setup downloads
   and verifies the app and memory model, starts the server and opens the console.
2. Choose **Connect your agent**. Add both the MCP connection and Hieronymus skills
   using the instructions for your host.
3. Ask the agent to verify the connection and skills before using memory in a book.
   Generated configuration alone does not prove that the host loaded it.

No source checkout or developer tools are needed. Windows and macOS installers
are unsigned; macOS may require **Open Anyway** in Privacy & Security.
The [release notes](https://github.com/InkyQuill/hieronymus/releases/latest)
state platform qualification limits.

The tray opens the console while the server runs. `hiero config` opens settings
and `hiero admin` opens memory administration. Browser authentication is optional
and off by default; MCP remains authenticated.

## Work with your agent

The agent recalls context, captures consequential observations and completes its
memory session when the task ends. [Business logic](business-logic.md) explains
how observations, source text, terminology and story context differ.

Tell the agent what your project permits: whether memory is supplemental or
primary, and whether recording/importing is allowed. Source import and prompt
capture must respect that agreement. A technical project binding is not permission
to import a whole manuscript.

In **Memory**, choose a book, search records and inspect source locations or
processing history. Search covers full record text before pagination. Technical
details are available by disclosure. Optional duplicate combining is a maintenance
convenience; automatic consolidation does not require it. Select at least two
compatible crystals and choose **Combine selected memories**. The knowledge
model assigned in Dreaming prepares a title and combined text for you to review
and edit. Confirm only when ready to save: preparation and cancellation leave
originals untouched. If a selected memory changes, prepare a fresh suggestion.
Model errors appear in the dialog with a retry action. Concept combining instead
keeps the first selected concept as the target.

Use **Correct this memory** for an exact statement, or **Correct a rendering**
for a selected source occurrence. Results distinguish applied, tentative and
conflict. Refresh a stale selection explicitly. Ordinary model calls cannot
turn quoted user text into a trusted correction. Automatic prompt capture and its
supported command grammar are described in [hook context](agent-hook-context.md).

## Configure processing

In **Settings → Providers**, create a hosted or local model profile and check it.
OpenAI-compatible, Gemini, Anthropic and Ollama profiles are supported. These
providers process memory; they are separate from your writing agent.

In **Dreaming**, assign the workflow models, enable scheduling and adjust pending
memory thresholds. Dreaming combines observations across sessions within each
project/language scope, including active tasks. A low pending count can
legitimately leave scheduled work waiting; a manual run drains eligible batches,
including the final small batch. A run with no progress leaves pending work visible.

Provider timeouts are editable; long-thinking models may need several minutes.
Memory comparison has its own provider, timeout and optional explicit backup.
It is unassigned by default, and no cloud fallback is implicit. See
[Memory and Dreaming](memory-dreaming.md).

Optional **Ingest → Prompt relevance** can send the current user message to Jev
for classification. Without a saved key, classification uses the local heuristic.
A skipped message may already have been transmitted to Jev; skipping retention
does not undo transmission. [Hook context](agent-hook-context.md#capture-and-stop-capture)
explains configuration, pausing and retry behavior.

Save settings to persist edits. Reload discards unsaved edits. API keys are stored
in private local configuration files and redacted from diagnostic projections.
The [provider decision](adr/0007-provider-catalog-and-workflow-assignments.md)
explains why profiles and workflow assignments are separate.

## Data and privacy

| Platform | Default configuration and memory directory |
| --- | --- |
| Linux | `$XDG_CONFIG_HOME/hieronymus`, falling back to `~/.config/hieronymus` |
| macOS | `~/Library/Application Support/Hieronymus` |
| Windows | `%APPDATA%/Hieronymus` |

The SQLite database is `hieronymus.sqlite` under that directory. An explicit
`--data-root DIR` overrides `HIERONYMUS_DATA_ROOT`, which overrides the platform
default. Book files should stay outside the application data directory.

Local storage does not mean all processing is offline: configured cloud providers
receive the relevant processing inputs. Diagnostics must not expose credentials
or browser grants. Read [service operations](service-toolkit.md) for configuration
files, export and startup diagnosis.

## Update and remove

Run the latest installer again to update, or use `hiero update`. Memories and
configuration are preserved. **Settings → Updates** chooses stable or development
releases; it does not itself install an update. Development means published
prereleases. `hiero update --check --json` checks without downloading the app.

On Windows, remove Hieronymus through **Settings → Apps → Installed apps**.
On Linux or macOS, run `hiero uninstall --yes`. Data is kept by default.
`--delete-data` explicitly clears contents of the selected data root, including
models, backups and audits; check that root before using it. Coordination lock
files may remain to keep concurrent operations on the same locks.

For offline releases, custom application paths and recovery, use
[Distribution](distribution.md). A schema upgrade is explicit and backed up;
returning to an older binary after an incompatible schema upgrade is unsupported.

## If something is wrong

- Check **Overview** for pending memories, indexing progress and processing errors.
- Verify the MCP connection and installed/loaded skills separately. After an
  upgrade, reopen the conversation if the host cached an older plugin.
- Run `hiero doctor --json` for configuration and runtime diagnosis. Keep private
  source text, credentials and grants out of shared reports.
- Use [service operations](service-toolkit.md) for startup problems,
  [agent workflows](agent-workflows.md) for integration/cache problems and
  [Memory and Dreaming](memory-dreaming.md) for unfinished processing.

A healthy connection is not proof that a model completed a Dream run or that
newly added hooks work in your installed host version.
