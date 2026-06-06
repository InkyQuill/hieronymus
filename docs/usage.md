# Hieronymus Usage

For the long-term memory workflow, see [Memory Dreaming](memory-dreaming.md).

## Data Root

By default, Hieronymus stores one global database at
`~/.config/hieronymus/hieronymus.sqlite`. Set `HIERONYMUS_DATA_ROOT` to use a
different data root:

```bash
export HIERONYMUS_DATA_ROOT=/home/inky/Yandex.Disk/Translation/.translation-memory
```

## Initialize a Series

```bash
hieronymus init-series only-sense-online --title "Only Sense Online" --source-language ja --target-language en
hieronymus init-series death-march --title "Death March to the Parallel World Rhapsody" --source-language ja --target-language en
```

## Propose a Term

```bash
hieronymus propose-term only-sense-online --category person_name --source "ユン" --translation "Yun" --tag name
```

## Validate a Chapter

```bash
cd /home/inky/Yandex.Disk/Translation
hieronymus validate only-sense-online --raw-file only-sense-online/vol01/raw/chapter-002.xhtml --translated-file only-sense-online/vol01/translated/chapter-002.md
```

## Remember Translation Context

```bash
hieronymus remember only-sense-online --kind translation_rationale --text "Use Yun for ユン." --source-ref only-sense-online/vol01/chapter-002
```

## Memory Dreaming Workflow

```bash
hieronymus init-series oso --title "Only Sense Online" --source-language ja --target-language en
hieronymus session-start oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
hieronymus remember-short 1 --role user --kind correction --text "Define obscure Japanese cultural terms when the average English reader may not know them."
hieronymus session-complete 1
hieronymus dream --provider deterministic
hieronymus session-start oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002
hieronymus recall 2 --series oso --source-language ja --target-language en --task-type translation --volume 01 --chapter 002 --query "cultural terms"
```

The final recall command uses session `2` because recall must run inside a new
active session after session `1` has been completed and dreamed.

## Service Commands

```bash
hiero
hiero status --json
hiero doctor
hiero install codex --dry-run
hiero stop
```

`hiero` is an alias for `hieronymus`; all subcommands work with either command.
