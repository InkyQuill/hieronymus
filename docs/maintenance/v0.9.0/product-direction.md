# v0.9.0 product direction

The following requirements were clarified by the owner during release maintenance on 2026-09-12. They supersede earlier assumptions about mandatory browser authentication, tray ownership, and a manually curated project knowledge base.

Hieronymus is an agent-centered, multi-type memory server for writing projects. Agents do the writing work in Codex, Cowork, or pi and use Hieronymus to remember relevant context. The author uses the web interface to inspect what the agent remembers, add pointers, and flag stale or wrong memories. It is not intended to become a comprehensive, human-maintained project knowledge base.

“Connect your agent” means adding the Hieronymus MCP connection **and** installing the Hieronymus skills that teach the agent how to work with its memory. Setup must distinguish preparing installation files from successfully installing and verifying both components in the actual agent host. Preparing or copying a setup request does not establish a working connection.

Hieronymus owns the server and serves the web administration interface. `hiero` is a helper for quick command-line tasks. The tray belongs to the running server, either in process or as a sidecar, and should remain visible whenever the desktop environment supports it and the server is running.

The default deployment is a local desktop tool. Browser authentication is disabled by default and configurable for deployments that need it. This does not remove authentication from MCP or trusted agent ingress.

The CWS source checkout is `/home/inky/Development/creative-writing-skills/`. Project recognition does not authorize wholesale book ingestion or rewriting project instructions.

Release testing uses the machines actually available: CachyOS with KDE, Windows 11, and macOS 26.5 on M1. Build Intel macOS as well, explicitly labeled unqualified. Record unavailable sessions and incomplete native checks honestly; do not substitute automated runner tests for native author or agent-host acceptance.
