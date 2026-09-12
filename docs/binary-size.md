# Hieronymus v0.8.0 binary-size investigation

Date: 2026-09-11
Scope: bounded, read-only inspection of the published/installed Linux artifact,
the current shared Cargo target, release sources, and symbols. No build or daemon
was run and no user/book runtime data was opened. A disposable copy of the
published executable was stripped inside this SDD directory for measurement and
then removed; the published artifact was not changed.

## Answer

The published `hiero` executable really is 280,165,088 bytes (267.19 MiB). It
does **not** contain the 470,301,610-byte ONNX model or the 24,268,848-byte ONNX
Runtime library. The release archive carries those as separate files. The binary
is large mainly because it statically links the Rust semantic-storage stack,
including Lance/LanceDB, DataFusion, Arrow, Tokio, tokenizers, bundled SQLite,
and the rest of the CLI/daemon. It also ships with 54.56 MiB of ordinary ELF
symbol/name tables because the release is not stripped.

The only substantial product asset intentionally embedded in the executable is
the production web console. The local bundle measured during this investigation totaled 335,555 bytes
(0.320 MiB, about 0.120% of the published executable size); this local bundle
measurement must not be presented as the exact embedded bytes of v0.8.0.
SQL schema migrations and small compile-time strings/constants are also embedded.
The model URL, expected byte count, and SHA-256 are constants, but the model bytes
are not.

## Exact published artifact

Measured installed rehearsal copy (the exact member of the published archive):

`/home/inky/Development/hieronymus/qualification/.artifacts/published-v0.8.0/rehearsal/app/versions/0.8.0/hiero`

- Size: 280,165,088 B = 267.19 MiB
- SHA-256: `630b3f36032580672e8f5d124db8ebe3ac650f8e1ca7f7b05ac2134828f9ca75`
- Build ID: `329025af5b207c3483fd009621c31e3e88b09f0a`
- ELF x86-64 PIE, dynamically linked, GNU/Linux 3.2 ABI marker
- `file`: `not stripped`
- Dynamic dependencies: `libgcc_s.so.1`, `libm.so.6`, `libc.so.6`, and the ELF
  loader. `libonnxruntime.so` is absent from `DT_NEEDED` because `ort` is built
  with `load-dynamic` and loads the separately packaged runtime at runtime.
- There are no DWARF `.debug_*` sections. “Not stripped” here means ordinary
  `.symtab` and `.strtab` are retained, not that 57 MiB of source-level DWARF is
  present.

The nearby shared-target release binary is a different later/local build:

`/home/inky/Development/hieronymus/target/x86_64-unknown-linux-gnu/release/hiero`

- Size: 280,394,688 B = 267.40 MiB
- SHA-256: `7e43098df16881baacd123da06e754d89cc989e7f952a33045d277790d8f9d78`
- It is also unstripped and has nearly identical section proportions.

Task 6's copied CLI and tray helper are developer/test-profile artifacts, not
shipping-size evidence. Their report hashes match files under
`.superpowers/sdd/2026-09-11-desktop-tray/task6-native/bin/`: `hiero` is
408,853,608 B and `hiero-desktop` is 29,229,720 B. Both are unstripped; the CLI
came from the shared `target/debug` path used by focused tests.

## ELF section accounting

Published v0.8.0 executable:

| Section(s) | Bytes | MiB | Share of file | Meaning |
|---|---:|---:|---:|---|
| `.text` | 162,808,400 | 155.27 | 58.1% | compiled machine code |
| `.symtab` + `.strtab` | 57,206,855 | 54.56 | 20.4% | non-runtime symbol records and names |
| `.gcc_except_table` + `.eh_frame_hdr` + `.eh_frame` | 32,100,288 | 30.61 | 11.5% | unwind/exception metadata |
| `.rodata` | 12,050,168 | 11.49 | 4.3% | constants, lookup tables, embedded console |
| `.rela.dyn` | 9,178,272 | 8.75 | 3.3% | dynamic relocations |
| `.data.rel.ro` | 6,584,000 | 6.28 | 2.4% | relocated read-only data/vtables |

The GNU `size` allocated total is 222,966,032 bytes (`text` 216,150,020,
`data` 6,803,056, `bss` 12,956); its “text” bucket groups several read-only
sections and is not the ELF `.text` section alone.

## Strip probe

On a disposable copy only, `strip --strip-unneeded` changed the executable from
280,165,088 B to 222,958,080 B (212.63 MiB), a reduction of 57,207,008 B =
54.56 MiB = 20.4%. The stripped copy retained the same Build ID, GNU `size`
totals, section offsets/sizes, and byte-for-byte SHA-256 for representative
allocated sections:

- `.text`: `24d40eeefcda870bb0080f70d506b2cab9a08eaae5e71029c519efc8b5db45a1`
- `.rodata`: `3f087329584477f84f04d172c6c204b283e19d0c76d6d6e3b3b66dd45c1090c2`
- `.data.rel.ro`: `b5d2b3f88d9d7e33c3e454d926d992a73bbd1fc5012ab6da10ecdafc60e68a73`

The removed sections were `.symtab` and `.strtab`; required dynamic symbols
remained. This is a concrete low-risk distribution saving, subject to preserving
symbols separately if readable native backtraces or postmortem symbolication are
required.

## Static dependency evidence

The workspace already uses `lancedb = 0.37.1` with `default-features = false`.
LanceDB 0.37.1 declares an empty default feature set, but its core dependencies
unconditionally include Lance 10, DataFusion 54, and the Arrow 58 family. Thus a
simple “turn off LanceDB defaults” change is already exhausted.

A heuristic pass over the published binary's demangled sized symbols matched
the following symbol extents. This is directional evidence rather than precise
crate-size accounting: generic monomorphizations can be named for traits or
callers, shared code has ambiguous ownership, and aliases can overlap.

| Name match | Matched bytes | MiB | Symbols |
|---|---:|---:|---:|
| Lance family | 36,444,884 | 34.76 | 44,187 |
| DataFusion family | 25,065,632 | 23.90 | 40,377 |
| Arrow family | 18,257,868 | 17.41 | 50,173 |
| LanceDB wrapper | 1,167,918 | 1.11 | 1,300 |

Together those names match 80,936,302 bytes (77.19 MiB) of symbol extents. The
largest named functions are overwhelmingly Lance index/dataset/scanner/write
specializations and DataFusion planning/functions. This supports the dependency
graph explanation without pretending it is an exact `cargo-bloat` apportionment.
The direct `ort` wrapper names match only 64,640 bytes because ONNX Runtime itself
is a separate dynamically loaded file. Bundled SQLite names match about 1.01 MiB.

## What the release archive contains

Published archive:

`/home/inky/Development/hieronymus/qualification/.artifacts/published-v0.8.0/download/hieronymus-0.8.0-x86_64-unknown-linux-gnu.tar.gz`

- Compressed archive: 533,963,914 B = 509.23 MiB
- Uncompressed tar: 784,179,200 B
- Archive SHA-256: `523f6530a454aedccc1b0ec32f77878f752202f900e275e4584530e1f1344d5a`
- `hiero`: 280,165,088 B
- `models/minilm/model.onnx`: 470,301,610 B
- `models/minilm/tokenizer.json`: 9,081,518 B
- `lib/libonnxruntime.so`: 24,268,848 B
- Model/runtime licenses, notices, `assets.json`, and three symlinks to the one
  `hiero` executable make up the remainder.

The release script explicitly copies the runtime and model into the package
*after* building the executable. Future per-platform packages can therefore
refer to a shared versioned model artifact without changing what is compiled
into `hiero`; that reduces duplicated downloads/packages, not the 267 MiB binary
itself.

## Practical optimization order

1. Strip release symbols and retain a Build-ID-addressed symbol artifact when
   diagnostics require it. The measured saving is 54.56 MiB, with no allocated
   code/data changes in the probe.
2. Add an explicit release profile and measure LTO plus fewer codegen units.
   There is currently no `[profile.release]`, so Cargo's defaults apply
   (`opt-level = 3`, no LTO, 16 codegen units, symbols not stripped). LTO can
   remove cross-crate duplication but must be rebuilt and qualified before any
   size claim.
3. Profile the remaining 212.63 MiB stripped executable with a controlled
   release build. The strongest current lead is the unconditional Lance +
   DataFusion + Arrow stack. Material savings there require narrower upstream
   linkage, another storage boundary, or process/library separation; current
   feature flags alone do not remove it.
4. Consider `panic = "abort"` or size-focused optimization only after auditing
   unwind/catch behavior and measuring semantic/storage performance. The 30.61
   MiB unwind/exception metadata makes this worth investigating, but this report
   makes no compatibility or savings claim.
5. Keep the web console embedded. The local bundle measured during this investigation was 335,555 bytes,
   about 0.12% of the published executable size, and does not explain its bulk.


## Historical Task 12 unpublished 0.9.0 candidate — 2026-09-11

The historical Task 12 Linux package receipt is [desktop-task12-linux-receipt.json](desktop-task12-linux-receipt.json). It binds the actual rebuilt binaries, platform archive, shared model and final installed assets. The real disposable test passed offline install, genuine stopped 0.8.0→0.9.0 upgrade, complete-pair rollback and uninstall preservation. The stripped installed CLI also ran its actual semantic lane and completed authenticated shutdown. Native managed active-update and live tray replacement were not qualified here.

| Executable | Before strip | Shipped | Saving |
|---|---:|---:|---:|
| hiero | 280,788,536 B | 223,470,088 B | 54.66 MiB (20.41%) |
| hiero-desktop | 21,291,504 B | 13,009,480 B | 7.90 MiB (38.90%) |

The platform archive is 97,753,102 bytes (93.22 MiB); the one unchanged model archive is 435,109,879 bytes (414.95 MiB). No model payload appears in the platform archive. The mandatory upstream runtime is unchanged at its pinned digest. Linux keeps separate diagnostic symbol files keyed by the original executable digest. The release profile remains Cargo defaults; this task measured native stripping, not LTO or panic-mode changes. The CLI dynamic dependencies are libc, libm, libgcc and the ELF loader; native GTK dependencies remain in the helper.


## Historical Task 14 local Linux candidate — 2026-09-11

[The Task 14 receipt](desktop-task14-linux-receipt.json) replaced Task 12 bytes for
that qualification. It does not qualify the later final-review fixes. This deliberate rebuild includes the Task 13 embedded
console and Task 14 ownership correction. It is an unpublished `dev` candidate;
there is no remote candidate-run or release-promotion claim.

| Executable | Before strip | Shipped | Saving |
|---|---:|---:|---:|
| hiero | 280,823,064 B | 223,518,824 B | 54.65 MiB (20.41%) |
| hiero-desktop | 21,295,008 B | 13,012,840 B | 7.90 MiB (38.89%) |

The final platform archive is **97,766,207 bytes (93.24 MiB)**. The canonical model
archive remains **435,109,879 bytes (414.95 MiB)** with SHA256
`4a23a216615c6224b1dba9fcd067e2da0d0be2b460b39dbb468b185f152911c0`.
The platform archive contains no model payload. The runtime remains pinned and
unchanged. Separate diagnostic files are 41,846,000 bytes for the CLI and
4,912,464 bytes for the helper; the receipt binds their SHA256 and executable ELF
Build IDs. These diagnostic bytes are outside the installed platform archive.
Cargo release defaults remain unchanged; no LTO or panic-mode saving is claimed.

The final CLI has only libc/libm/libgcc/loader dynamic dependencies; GTK remains
in the helper. The assembled stripped CLI passed real pinned inference and
authenticated MCP/shutdown. The actual final split archive also passed disposable
offline installation, genuine stopped 0.8.0→0.9.0 upgrade, rollback and uninstall.
Native manager and interactive panel/session checks remain unavailable. The
published 0.8.0 measurement above, the historical Task 12 candidate, this final
candidate and separate diagnostic files are distinct measurements.


## Final review fixes: current local Linux candidate — 2026-09-11

[The new exact-byte receipt](desktop-final-fixes-linux-receipt.json) binds source
commit `1da8c61a72c370ba68917aac93aa7c44a088e92a` and the separate immutable
`qualification/.artifacts/desktop-final-fixes/final-release/` output. The Task 14
archive above remains unchanged and is historical evidence.

| Executable | Before strip | Shipped | Saving |
|---|---:|---:|---:|
| hiero | 280,830,112 B | 223,520,296 B | 54.65 MiB (20.41%) |
| hiero-desktop | 21,298,432 B | 13,013,064 B | 7.90 MiB (38.90%) |

The platform archive is **97,765,062 bytes (93.24 MiB)**, SHA256
`f075fd95fd125fd796c825c49b726c5eb6e1a57f5ed2d63e469ea945faee8b1e`. The canonical common model
remains **435,109,879 bytes**, SHA256
`4a23a216615c6224b1dba9fcd067e2da0d0be2b460b39dbb468b185f152911c0`. No model payload is present
in the platform archive. The pinned ONNX runtime is unchanged. Separate diagnostic
files are **41,851,952 bytes** for the CLI and **4,915,048 bytes** for the
helper; the receipt binds their hashes and ELF Build IDs. Cargo release defaults,
no-LTO, unwind policy and upstream dependencies remain unchanged.

These final stripped bytes passed real model inference, authenticated MCP status
and shutdown, plus disposable fresh installation, genuine stopped 0.8.0 upgrade,
rollback and uninstall. Receipt verification rehashed 38 logs, all 10 release files,
4 installed members and 344 source-input records. This partial source inventory
is not a toolchain/dependency closure. Native interactive and native-manager
acceptance on Linux, Windows and both macOS targets remain separate unavailable
gates; these local results do not authorize release promotion.
