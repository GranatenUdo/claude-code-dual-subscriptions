# LRU dependency follow-up

Reviewed 2026-10-14 separately from the subscription routing implementation.

`Cargo.lock` contains `lru 0.12.5`, pulled in by `ratatui 0.29.0`.
The upstream Ratatui manifest requires `lru ^0.12.0`.

- [RUSTSEC-2026-0002](https://rustsec.org/advisories/RUSTSEC-2026-0002.html):
  mutable iteration can violate Stacked Borrows; fixed in `lru >=0.16.3`.
- [RUSTSEC-2026-0253](https://rustsec.org/advisories/RUSTSEC-2026-0253.html):
  `pop` can leave dangling pointers if a key destructor panics; fixed in `lru >=0.18.2`.

Both advisories include the locked version. A lockfile-only update cannot select
either fix under Ratatui's current semver requirement. Adding a direct dependency
on `lru 0.18.2` would leave the vulnerable transitive version installed as well.

No dependency fix is claimed in this development copy. A separate Ratatui upgrade
or reviewed backport is needed, with monitor rendering, layout, and terminal
regression checks. Do not interpret mock provider tests as resolving these
advisories or proving they cannot be triggered in this application.

Source: [Ratatui 0.29.0 manifest](https://github.com/ratatui/ratatui/blob/v0.29.0/Cargo.toml).
