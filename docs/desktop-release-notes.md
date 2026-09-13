# Hieronymus v0.9.1

<!-- Publication template: desktop-ci.ts replaces @@EVIDENCE_URL@@ with the verified immutable qualification-data URL before creating the GitHub release. -->

Give your writing agent a memory. Install Hieronymus, choose **Connect your agent**,
add its MCP connection and Hieronymus skills, then continue writing in your agent.

## Install

- **Windows:** [Download Setup](https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/Hieronymus-0.9.1-Setup.exe), open it, and follow the steps.
- **macOS:** [Download the installer](https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/Hieronymus-0.9.1.pkg), open it, and follow the steps. It chooses Apple Silicon or Intel automatically.
- **Linux x86_64:** paste this into a terminal:

```bash
curl -fsSL https://github.com/InkyQuill/hieronymus/releases/download/v0.9.1/install-hieronymus.sh | bash
```

The installers download the app and its memory model, verify the files, and install
for your user account. Keep your internet connection on during setup. No Git clone,
compiler, Bun, Node, Python, or separate PowerShell installation is needed.
On Windows, Setup also installs Microsoft's runtime if needed; Windows may ask
you to allow that step.

The web interface opens after installation. Use it to inspect your agent's memory,
add pointers, and flag stale or wrong entries. Browser authentication is optional
and off by default; MCP access remains authenticated.

This update adds the simple installers, fixes macOS installation failing on
allocator diagnostics reported by `launchctl`, and handles short-lived Windows
file locks during installation and removal.
It also fixes occasional feedback failures when an agent reports on recalled
memories while background processing updates the database.

## Platform notes

The Windows and macOS installers are unsigned. macOS may require **Open Anyway**
in Privacy & Security. Intel macOS is included but remains **unqualified for native
desktop use**. Other test gaps are listed in the [qualification records](@@EVIDENCE_URL@@).

Test captures and packaging evidence are maintainer records, not installation
downloads. The remaining archives and metadata support automatic downloads and
advanced offline use; you do not need to download them separately.
