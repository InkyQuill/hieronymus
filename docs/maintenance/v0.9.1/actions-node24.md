# GitHub Actions Node 24 migration

The owner reported Node 20 deprecation warnings in installer run 34747086141.
All five active workflows now use immutable pins for the current official releases:

| Action | Release | Commit |
| --- | --- | --- |
| actions/checkout | v7.0.1 | 3d3c42e5aac5ba805825da76410c181273ba90b1 |
| actions/upload-artifact | v7.0.1 | 043fb46d1a93c77aae656e7c1c64a875d1fc6a0a |
| actions/download-artifact | v8.0.1 | 3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c |

Each pinned `action.yml` was read from its official repository and declares
`runs.using: node24`. The existing setup-bun pin is already v2.2.0 and Node 24;
rust-toolchain is a composite action, so neither requires this replacement.

Compatibility review: checkout's new unsafe-fork restriction does not affect our
`pull_request` and explicit dispatch/tag workflows. Existing credential options
remain intact. Upload keeps its default ZIP archive behavior. Downloads keep
name/pattern extraction and now fail on transport digest mismatch by default.
No Node 20 opt-out or warning suppression is introduced.

Sources: [GitHub's migration notice](https://github.blog/changelog/2025-09-19-deprecation-of-node-20-on-github-actions-runners/),
[checkout](https://github.com/actions/checkout/releases/tag/v7.0.1),
[upload-artifact](https://github.com/actions/upload-artifact/releases/tag/v7.0.1),
[download-artifact](https://github.com/actions/download-artifact/releases/tag/v8.0.1).

The installer workflow will verify the new upload/download path on Linux,
Windows, and macOS; its known v0.9.0 macOS runtime failures are distinct from
action compatibility. New release candidates must use the updated source.
