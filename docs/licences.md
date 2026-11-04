# Licences

Glasstarn is licensed under the [Apache License 2.0](../LICENSE).

## What the app is built from

`deny.toml` sets the dependency policy and `cargo deny check` enforces it in
CI: only permissive licences, crates only from crates.io, and no withdrawn
(yanked) versions or known vulnerabilities.

| Part | Licence | Notes |
|---|---|---|
| Rust libraries (about 130 crates) | MIT, Apache-2.0, BSD-2/3-Clause, BSL-1.0, CC0-1.0, 0BSD, Unicode-3.0, Unlicense, Zlib | All permissive. Where a crate offers a choice that includes LGPL (`r-efi`, used only for UEFI targets), the permissive option applies. |
| `calamine` and what it brings (`zip`, `encoding_rs`, `codepage`, `zopfli`) | MIT; `encoding_rs` also BSD-3-Clause for its data; `zopfli` Apache-2.0 | Reads old Excel (.xls, .xlsb) and LibreOffice (.ods) workbooks. |
| SQLite, bundled through `rusqlite` | Public domain | |
| Zstandard, bundled through `zstd-sys` | BSD-3-Clause | |
| Geist and Geist Mono fonts (Vercel) | SIL Open Font License 1.1 | Text in `ui/fonts/OFL.txt`, shipped beside the app as `FONTS-OFL.txt`. The fonts may be bundled; they may not be sold on their own. |
| Browser code | Apache-2.0 (this project) | No npm packages or third-party scripts. The UI tests use only Node's built-in test runner. |

Release packages include `LICENSE`, `FONTS-OFL.txt` and
`THIRD_PARTY_NOTICES.txt`, which lists every crate compiled into the program
with the licence texts it ships, as MIT and BSD licences ask for binary
distributions. Regenerate it after changing dependencies:
`python3 packaging/notices/generate.py`.

## Data

The test fixtures in `testdata/` and the files in `examples/` are invented for
this project. Public sample logs and the BPI Challenge data are downloaded
separately and are not part of the repository; their sources, licences and
checksums are listed beside the downloaded files.
