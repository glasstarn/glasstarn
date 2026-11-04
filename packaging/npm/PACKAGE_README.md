# glasstarn

See how your work really flows. glasstarn is free, open-source process mining
that runs on your own computer.

```sh
npx glasstarn setup
```

This installs the `glasstarn` command and connects it to Claude Code and Codex.
Restart your agent, open it on a folder with an export from your system, and
ask in plain words, for example "why are our invoices paid late?". Your agent
runs glasstarn, explains what it found and opens the results in your browser;
keep talking and that page follows the conversation.

Your files and every calculation stay on your computer. The only request
glasstarn makes on its own is a daily check of the latest version number on
npm; turn it off with `GLASSTARN_NO_UPDATE_CHECK=1`.

- Documentation: https://glasstarn.com/docs/start/install/
- Source: https://github.com/glasstarn/glasstarn
- Contact: hello@glasstarn.com

Apache-2.0 · Tropical Glacier LLC
