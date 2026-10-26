# Security

Please report vulnerabilities privately, not in a public issue: use **Report a
vulnerability** on the repository's Security tab (GitHub private vulnerability
reporting). Include what's affected (the CLI or the desktop app, and the version) and
how to reproduce it. You'll get a reply as soon as possible, and a fix will be released
before the details are made public.

Worth knowing about DataDrop's design:

- Connection profiles, **including passwords, are stored as plain text** in
  `~/.datadrop/profiles`, readable only by your user account. Anyone (or anything) that
  can read your files can read them. See [Where things are kept](README.md#where-things-are-kept).
- Connections use TLS when the server offers it (`prefer`), without checking the
  certificate. Use `verify-full` (with a CA certificate if the server's isn't publicly
  trusted) to be protected from someone intercepting the connection.
- DataDrop runs the SQL you give it as written; it has no read-only mode. Use a database
  user with only the permissions you need.

Releases are signed when signing is set up: see [Install](README.md#install) for how to
check a download.
