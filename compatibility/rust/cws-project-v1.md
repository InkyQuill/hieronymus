# CWS structural compatibility examples

`cws-project-v1.json` is a byte-identical copy from creative-writing-skills
commit `d8ed8a2198b012af32dae2ace4feda196cb0560e`, at
`plugins/creative-writing-skills/skills/project-maintenance/resources/compatibility/cws-project-v1.json`.
The public specification is the adjacent producer `external-project-contract.md`.
Contract v1 supports project schemas 1 and 2. Tests use the portable
`technical_failure` field, not optional producer diagnostic codes.

The Hieronymus tests materialize these data-only examples without a CWS runtime.
Additional parser cases pin the scalar/list behavior of that commit's
`resources/cli/cwcli/documents.py`; this is not a general YAML parser contract.

The actionable direction cases were produced with `TranslationFixture`, source
transactions and `plan_direction`; producer tests exercise `project_settings`,
`load_catalog`, `effective_direction`, and `resolve_unit`. They pin same-language
directions, another target language in the same common work, volume source
replacement, independent unbound direction maps, and uncovered editions. The
`expect.selections` entries add read-only inspection expectations; they do not
change the project schema or grant memory authority.
