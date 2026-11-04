# Security

glasstarn runs on your own computer. Its browser view listens on `127.0.0.1`
only and refuses requests from other sites. Your files, projects and results
are not sent anywhere by glasstarn. The one request it makes on its own is a
version check: at most once a day it asks the npm registry
(registry.npmjs.org) for the latest glasstarn version number, with no data
about you or your projects. Turn it off with `GLASSTARN_NO_UPDATE_CHECK=1` or
`"update_check": false` in `~/.glasstarn/settings.json`. Data is pulled from a
system such as Salesforce only when you approve that pull.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private vulnerability reporting
on this repository ("Report a vulnerability" under the Security tab), with the
version (`glasstarn --version`), your operating system and the steps to
reproduce. We aim to acknowledge reports within three working days.

Never include real company data in a report.

## Supported versions

Security fixes are made in the latest release. Update with
`npx glasstarn setup` or `glasstarn update`.
