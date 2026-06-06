# Hieronymus Usage

## Data Root

Set `HIERONYMUS_DATA_ROOT` to the translation workspace memory directory:

```bash
export HIERONYMUS_DATA_ROOT=/home/inky/Yandex.Disk/Translation/.translation-memory
```

## Initialize a Series

```bash
hieronymus init-series only-sense-online --title "Only Sense Online" --source-language ja --target-language en
hieronymus init-series death-march --title "Death March to the Parallel World Rhapsody" --source-language ja --target-language en
```

## Validate a Chapter

```bash
cd /home/inky/Yandex.Disk/Translation
hieronymus validate only-sense-online --raw-file only-sense-online/vol01/raw/chapter-002.xhtml --translated-file only-sense-online/vol01/translated/chapter-002.md
```
