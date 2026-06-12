# Keep Hieronymus In Alpha Until Explicit Release Approval

Hieronymus has premature `v1.0.0` and `v1.1.0` tags, but the product is still in
alpha and should not present itself as a stable 1.x release line. The version
line should be remapped to `0.x` (`1.0.0` becomes `0.1.0`, `1.1.0` becomes
`0.2.0`) and no `1.0.0` release should be created until Pavel explicitly
approves a major release. Human-facing prompts and headers should display a
Greek alpha marker, for example `v0.2.0α`, while package metadata, tags, and
update comparisons remain SemVer-compatible.
