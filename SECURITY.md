# Security

Please report vulnerabilities privately, not in a public issue: use **Report a
vulnerability** on the repository's Security tab (GitHub private vulnerability
reporting). Include what's affected (the CLI or the desktop app, and the version) and
how to reproduce it. You'll get a reply as soon as possible, and a fix will be released
before the details are made public.

Worth knowing about DataDrop's design:

- Database and SSH passwords and key passphrases are kept in the OS keychain when
  there is one. Profiles set to keep them in the file (or saved where there's no
  keychain) **store them as plain text** in `~/.datadrop/profiles`, readable only by
  your user account: anyone (or anything) that can read your files can read them.
  See [Where things are kept](README.md#where-things-are-kept).
- Connections use TLS when the server offers it (`prefer`), without checking the
  certificate. Use `verify-full` (with a CA certificate if the server's isn't publicly
  trusted) to be protected from someone intercepting the connection.
- SSH tunnels only connect to servers whose host key is known (in
  `~/.ssh/known_hosts`, or trusted in DataDrop after you've checked its fingerprint),
  and refuse a server whose key has changed. While a tunnel is open, its local end
  listens on 127.0.0.1, so other programs on the same computer could connect through
  it, as with `ssh -L`.
- The query history (`~/.datadrop/history.jsonl`) keeps the SQL you've run, which may
  include values you typed into queries. Clear it with `datadrop history clear`.
- DataDrop runs the SQL you give it as written; it has no read-only mode. Use a database
  user with only the permissions you need.

Releases are signed when signing is set up: see [Install](README.md#install) for how to
check a download.
