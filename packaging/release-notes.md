glasstarn 1.0.1 smooths three rough edges people met in their first hour.

## What changed

- **Your results page stays on while you use it.** Every open glasstarn tab now tells glasstarn it is still in use, even in the background, so it no longer stops while you are reading. It closes by itself 15 minutes after the last tab is closed. If it ever is not running, the page says so plainly: "glasstarn isn't running. Ask your agent to open it again, or run glasstarn in Terminal."
- **Deleting the project you are in** now takes you to the project list with a note that it is in the Trash, instead of leaving you on a page that had closed. Renaming it from there keeps you where you are.
- **Setup ends with one plain next step:** restart your agent, then just ask.

## Install or update

```sh
npx glasstarn setup
```

Already installed? Ask your agent to update glasstarn, or run `glasstarn update`.

The attached `glasstarn-1.0.1.tgz` is the exact package published to npm, tested on macOS (Apple silicon and Intel), Linux (x64 and ARM64) and Windows x64; `glasstarn.sha256` holds its checksum.

Documentation: https://glasstarn.com · Contact: hello@glasstarn.com · Apache-2.0
