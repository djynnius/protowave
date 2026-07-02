# ProtoWave

A revival of Google Wave on modern foundations: a **Rust** backend, a **Vue 3** frontend, **CRDT**-based real-time collaboration, and **federation** between independent servers.

A wave is a document that is a conversation — simultaneously a message thread, a collaboratively edited document, a replayable history, and a federated object. ProtoWave keeps that model and adds live language translation, first-class markdown, and distributed P2P file sharing.

See **[PRD.md](PRD.md)** for the full product requirements document.

## Repository layout

```
crates/protowave-core     Core types: federation-qualified IDs, ed25519 signing
crates/protowave-proto    Protocol schemas (protobuf) + generated Rust types
server/                   protowave-server binary (axum gateway)
web/                      Vue 3 SPA (Vite + TypeScript)
docs/                     Protocol specifications
```

The retired Apache Wave (Java) codebase this project descends from is preserved on the [`legacy/apache-wave`](../../tree/legacy/apache-wave) branch and serves as an architectural reference.

## Development

Rust workspace:

```sh
cargo build            # build everything
cargo test             # run tests (includes the auth+echo integration test)
cargo run -p protowave-server   # run the server (default 127.0.0.1:9898)
```

Web client:

```sh
cd web
npm install
npm run dev            # Vite dev server, proxies /ws to the Rust server
npm run build          # type-check + production build
```

## Status

**Phase 4 (Translation)** — see PRD §12 for the roadmap.

Waves can opt in to live translation (with an explicit third-party-API
disclosure): readers pick a language and every blip gains a translated
overlay that updates as people type — the original text is always what's
stored, and switching back is one click. Provider is swappable
(`Translator` trait); the reference implementation is Gemini Flash-Lite
(`PROTOMOLECULE` env for the API key, `PROTOWAVE_TRANSLATE_MODEL` to
change models). Translations are cached by content hash and capped by
`PROTOWAVE_TRANSLATE_CAP`.

Two ProtoWave servers can now federate: add `bob@other.server` to a wave
and both servers hold live replicas — cross-server co-editing converges,
membership is distributed by the wave's home server (signed announcements),
every s2s message is ed25519-signed with TOFU key pinning, and
state-vector anti-entropy back-fills anything missed. Configure with
`PROTOWAVE_PEERS="other.server=http://host:port"` and
`PROTOWAVE_PUBLIC_URL`.

Working now: accounts (argon2id + sessions), wave creation and inbox with
unread badges, real-time collaborative editing (yrs/Yjs CRDTs over the
protobuf WebSocket protocol), threaded blips, live collaborator cursors and
presence, offline reconnect convergence, **playback** (replay any wave from
the beginning), **full-text search** (embedded tantivy), **attachments**
(BLAKE3 content-addressed, deduplicated, ACL-checked) with **markdown
rendering** of shared `.md` files, and pluggable persistence: embedded
file store (default) or **PostgreSQL** (`PROTOWAVE_PG=<conn-string>`).

The server also serves the built SPA (`web/dist`) when present — a single
binary is a complete deployment.

Try it: run the server, then `npm run dev` in `web/`, open two browsers,
register two accounts, create a wave, add the second account, and type in
both windows. Hit ↺ replay to scrub through history.

## License

Apache License 2.0 (see [LICENSE](LICENSE)), continuing the Apache Wave lineage.
