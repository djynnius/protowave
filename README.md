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

**Phase 1 (single-server collaboration)** — see PRD §12 for the roadmap.

Working now: accounts (argon2id + sessions), wave creation and inbox,
real-time collaborative editing (yrs/Yjs CRDTs over the protobuf WebSocket
protocol), threaded blips, live collaborator cursors and presence, offline
reconnect convergence, and durable file-backed persistence with periodic
snapshots.

Try it: run the server, then `npm run dev` in `web/`, open two browsers,
register two accounts, create a wave, add the second account, and type in
both windows.

## License

Apache License 2.0 (see [LICENSE](LICENSE)), continuing the Apache Wave lineage.
