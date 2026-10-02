# Project Conventions

## Local Agent Files

- `.superpowers/` is temporary brainstorming companion state and stays ignored.
- `.agents/` is trackable because project-local agent skills and configuration
  may live there.

## Local build residue

The application no longer has a Python environment or Python package setup.
Remove obsolete project-local virtual environments and Python test/lint/bytecode
caches left by pre-Rust checkouts; do not retain them as historical evidence.
Ignore rules remain defensive for anyone inspecting an old revision.

Legacy SQLite fixtures under compatibility/ are still inputs to Rust migration
and installer tests. They are not a Python runtime dependency. The optional
source build of the upstream Intel macOS ONNX Runtime has its own Python/toolchain
requirements; do not confuse that upstream build with the application runtime.
