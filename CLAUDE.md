# CLAUDE.md — ProtoWave

Revival of Google Wave: Rust backend, Vue 3 frontend, CRDT collaboration.
**PRD.md is the source of truth** for requirements (FR-xx), complexity
budgets (NFR-Cx), architecture decisions (ADR-1..5), and the phase roadmap
(§12). The retired Apache Wave Java codebase lives on branch
`legacy/apache-wave` as an architectural reference.

## Status

- **Phases 0–4 complete**: foundations, real-time collaboration, Wave
  parity (search/attachments/markdown/playback/read-state/PostgreSQL),
  **federation** (signed s2s over HTTP — `server/src/federation.rs`, spec
  in `docs/federation-spec.md`), and **translation**
  (`server/src/translate.rs`: `Translator` trait, Gemini reference impl
  over hyper-rustls, TranslationHub with content-hash cache + call cap;
  overlays on CHANNEL_TRANSLATION; wave opt-in via
  `/api/waves/translation`). Gemini key comes from the `PROTOMOLECULE`
  env var (set in ~/.bashrc behind the interactive guard — extract with
  `eval "$(grep PROTOMOLECULE ~/.bashrc | head -1)"`).
- **Phase 5 (folder sharing) complete**: `server/src/shares.rs` — FastCDC
  chunking into the BLAKE3 CAS, FolderManifest protobuf (manifest hash =
  share id/capability), federated multi-source chunk fetch with per-chunk
  BLAKE3 verification, share announcements, mirroring. v0 transport is the
  signed federation HTTP channel — iroh/QUIC deferred (does not build on
  rustc 1.75), architecture is transport-agnostic.
- **Phase 6 complete — v1 roadmap done (public beta ready)**: PWA
  (manifest + hand-rolled sw.js in `web/public/`, network-first shell),
  i18n ×7 locales (`web/src/i18n.ts`, NFR-20), rate limiting
  (`server/src/limits.rs`, FR-63; per-IP on auth via optional ConnectInfo,
  per-user on wave ops), admin stats endpoint (PROTOWAVE_ADMIN env), and
  the **extension host** (`web/src/components/ExtensionFrame.vue`):
  sandboxed iframes + postMessage bridge to a collaborative Y.Map —
  Wave's gadget successor; sample app `web/public/extensions/tally.html`.
- **Phase 7 (Hive Mind) shipped as exploratory** (`server/src/agent.rs`):
  `InferenceProvider` trait + Gemini impl; agents author real blips into the
  wavelet CRDT (yrs construction wire-compatible with web wavemodel.ts) via
  POST /api/waves/ask; RAG over the asker's accessible waves + shared files
  with provenance; signed federated inference `/federation/v0/infer`
  (mixture-of-peers), model advertised in .well-known. FI-x not FR-x.
  Constraints: no local GPU → provider stand-in (Gemini); R11 verification
  unsolved (answers advisory). PROTOWAVE_INFER_MODEL overrides model.
- Deferred: OIDC login (FR-3), in-text anchored inline replies (FR-19),
  participant *removal* + federated blob fetch for attachments, gRPC/TLS
  transport for federation.

## Critical toolchain constraint (read before touching Cargo.toml)

Host has **rustc/cargo 1.75.0 from a source tarball — no rustup, no
protoc**. Consequences:

- `Cargo.lock` pins ~30 pre-edition2024 transitive versions. **Always
  build/test with `--locked`. Never run bare `cargo update`** — it will
  break the build with `edition2024` manifest errors. To add a dependency,
  pin `=x.y.z` (2024-era versions) in the workspace `Cargo.toml`, then fix
  offenders one at a time with `cargo update -p <crate> --precise <older>`
  (probe new dep stacks in a scratch crate first).
- tantivy runs **mmap-only features** (no lz4/zstd codecs — those crates
  no longer build on 1.75; lz4_flex 0.11.x was yanked).
- Protobuf codegen uses **protox** (pure Rust) — no system protoc needed.
- rustfmt/clippy are not installed system-wide. An assembled official
  1.75 toolchain (rustc+std+clippy+rustfmt from static.rust-lang.org/dist)
  reproduces CI exactly — see the memory note `toolchain-rustc-175-no-rustup`
  for the recipe. **Run fmt + clippy locally before pushing; CI enforces
  `-D warnings`.**

## Layout & key facts

- `crates/protowave-core` — federation-qualified IDs (`user@domain`,
  `domain/w+id/conv+root`), ed25519 signing.
- `crates/protowave-proto` — **canonical wire schema**
  (`proto/protowave/v1/envelope.proto`). Rust types generated at build;
  the web client parses the same file at runtime (protobufjs). One schema,
  two languages.
- `server/` — axum. Modules: `store` (async `WaveStore` trait; `FileStore`
  embedded default, `store_pg::PgStore` for PostgreSQL via `PROTOWAVE_PG`),
  `engine` (yrs docs, subscribe = state-vector/diff exchange, append-only
  update logs — **never truncated**, playback depends on this), `search`
  (tantivy behind `SearchIndex` trait, fed by engine change stream), `cas`
  (BLAKE3 content-addressed blobs), `ws`, `auth` (argon2id + cookie
  sessions; WS authenticates at upgrade), `attachments`, `api`.
- `web/` — Vue 3 + TS + Pinia + Reka UI. Editor: Tiptap 2 + y-prosemirror
  bound to `Y.XmlFragment`s in the `blips` map; thread tree in `manifest`
  (see `src/lib/wavemodel.ts`, mirrors PRD §4.3). Custom provider over the
  envelope protocol in `src/lib/provider.ts`.
- Design language: **ProtoWave Brand v2** (source of truth: `pw-theme/`) —
  luminous blues (Crest/Spray/Deep/Dusk over ink/slate/cloud), Archivo 900
  display + Hanken Grotesk body + JetBrains Mono captions, pill buttons,
  soft white cards, animated triangulated wave mesh
  (`web/src/components/WaveMesh.vue` — the user's favorite element). Use
  the frontend-design skill for UI work; no generic AI aesthetics.

## Commands

```sh
cargo test --workspace --locked          # all tests (--locked always!)
PROTOWAVE_TEST_PG="host=10.102.109.186 user=protowave password=protowave-dev dbname=protowave_test" \
  cargo test --workspace --locked       # includes live-PG suite
cargo run -p protowave-server            # dev server on 127.0.0.1:9898
cd web && npm run dev                    # Vite dev server (proxies /api, /ws)
cd web && npm run build                  # type-check + bundle
```

Env: `PROTOWAVE_ADDR`, `PROTOWAVE_DOMAIN`, `PROTOWAVE_DATA_DIR`,
`PROTOWAVE_FSYNC` (0/1), `PROTOWAVE_PG` (Postgres conn string; omit for
file store), `PROTOWAVE_WEB_DIST` (SPA dir; server serves it when present).

## Deployment (live)

- LXC container **protowave** (`10.102.109.183`): systemd unit
  `protowave.service`, binary at `/opt/protowave/protowave-server`, SPA at
  `/opt/protowave/dist`, data in `/var/lib/protowave`, port **80** via
  `CAP_NET_BIND_SERVICE`, domain `protowave.local`. LXD proxy device maps
  **host :9898 → container :80**. Deploy = build release on host (glibc
  2.35 < container 2.39, so host binaries run), `lxc file push`, restart.
  Gotcha: quote `Environment="PROTOWAVE_PG=..."` in the unit — the conn
  string has spaces.
- LXC container **protowave2** (`10.102.109.126`): second federation node,
  domain `protowave2.local`, file-backed store, port 80, host proxy
  **:9797 → container :80**. The two nodes federate via `PROTOWAVE_PEERS`
  pointing at each other's container IPs.
- LXC container **postgres** (`10.102.109.186`): Postgres 16, role
  `protowave` / `protowave-dev`, DBs `protowave` (prod) and
  `protowave_test` (tests).

## Conventions

- CI (GitHub Actions): fmt, clippy `-D warnings`, tests (with postgres:16
  service), web build. Watch runs via the GitHub API (no `gh` CLI on host).
- Commit style: imperative summary + body explaining what/why; end with
  the Claude Co-Authored-By trailer.
- The wire protocol changes in `envelope.proto` must keep Rust and web in
  sync — the integration tests in `server/tests/` are the contract.
