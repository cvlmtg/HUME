# Security Policy

## Supported versions

Security fixes go into the next release. Upgrade to the [latest release](https://github.com/cvlmtg/HUME/releases/latest) to get them. An older release line gets a patch release only when a fix is worth backporting to it.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through [GitHub's vulnerability reporting](https://github.com/cvlmtg/HUME/security/advisories/new) (the **Report a vulnerability** button on the repository's Security tab).

Include:

- the HUME version (`hume --version`) and your OS
- the steps or the file that trigger the problem
- what an attacker gains from it

HUME is maintained by one person in their spare time. Reports get a reply as soon as possible, but there is no fixed response time. Once a fix is released, the advisory is published with credit to the reporter, unless they ask otherwise.

## Scope

Plugins and `init.scm` are trusted code. They run with your user's privileges and can read and write any file and run any program, so a plugin doing that is not a vulnerability. Install third-party plugins only from sources you trust.

In scope is anything that lets content you did not choose to trust run code or touch files outside where it should, for example:

- opening a crafted file
- installing a grammar or language server whose archive writes outside its install directory
- a language server response that makes HUME write outside the edit it asked for
