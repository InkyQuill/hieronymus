# CWS structural compatibility examples

`cws-project-v1.json` is a byte-identical copy from creative-writing-skills
commit `68b95b6`, at
`plugins/creative-writing-skills/skills/project-maintenance/resources/compatibility/cws-project-v1.json`.
The public specification is the adjacent producer `external-project-contract.md`.
Contract v1 supports project schemas 1 and 2. Tests use the portable
`technical_failure` field, not optional producer diagnostic codes.

The Hieronymus tests materialize these data-only examples without a CWS runtime.
Additional parser cases pin the scalar/list behavior of that commit's
`resources/cli/cwcli/documents.py`; this is not a general YAML parser contract.
