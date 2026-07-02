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
- **Phase 5 (P2P folder sharing) is next** per PRD §12.
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
- Design language: **"Tidewriter's Desk"** — ivory paper, ink navy, tidal
  teal, coral; Fraunces/Newsreader/Spline Sans Mono (@fontsource, bundled).
  Use the frontend-design skill for UI work; no generic AI aesthetics.

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
