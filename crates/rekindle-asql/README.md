# rekindle-asql

Async handle for [`rusqlite`] on a dedicated background thread.

This crate is **vendored verbatim from [`tokio-rusqlite`] 0.7.0** (MIT, © 2022 Eray Karatay;
see `LICENSE`). Changes versus upstream: the dependency pin (`rusqlite 0.37` → `0.39`;
upstream is unmaintained and its `rusqlite ^0.37` requirement conflicts with `veilid-core`
0.5.8's sqlite chain), two additions (`Connection::call_detached` and the call watch), and one
fix (finding AQ1): a successful `close` drops the queue's receiver before it resolves, so a call
made after `close` returns gets `Error::ConnectionClosed` (or `call_unwrap`'s documented panic)
instead of being queued and then discarded unrun.

Consuming crates in this workspace depend on it under the name `tokio_rusqlite` via a Cargo
`package`-rename, so existing `tokio_rusqlite::` call-sites compile unchanged:

```toml
tokio_rusqlite = { package = "rekindle-asql", path = "../rekindle-asql" }
```

Feature flags for `rusqlite` (e.g. `bundled`, `bundled-sqlcipher-vendored-openssl`) are supplied
by each consumer's own direct `rusqlite` dependency — Cargo unifies them across the graph, so
this crate stays featureless.

[`rusqlite`]: https://crates.io/crates/rusqlite
[`tokio-rusqlite`]: https://crates.io/crates/tokio-rusqlite
