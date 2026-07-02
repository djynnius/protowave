# ProtoWave — Product Requirements Document

**Version:** 1.0-draft
**Date:** 2026-07-02
**Status:** Draft for review
**Repository:** `protowave` (legacy Apache Wave Java codebase preserved on branch `legacy/apache-wave`)

---

## 1. Overview & Vision

ProtoWave is a revival of Google Wave, rebuilt on modern foundations: a **Rust** backend engineered for explicit space/time efficiency, a **Vue 3** frontend built for reactivity, **CRDT**-based real-time collaboration, and **federation** between independent servers as a first-class design goal.

Wave's core insight remains unmatched by anything that followed it: a **wave is a document that is a conversation**. It is simultaneously:

- a **message thread** (like email or Slack),
- a **collaboratively edited document** (like Google Docs),
- a **replayable history** (any participant can play back how the conversation evolved),
- a **federated object** (users on different servers collaborate on the same wave, like email but live).

ProtoWave keeps that model and adds three modern differentiators:

1. **A real-time language translation engine** — messages written in one language appear to each recipient in their own language, translated live as words are typed, with earlier words self-correcting as context accumulates.
2. **First-class markdown** — markdown authoring in messages and native rendering of `.md` files.
3. **Distributed, torrent-like file sharing** — a user can add a local folder to ProtoWave and share it peer-to-peer; content is chunked, content-addressed, and fetched from any online holder.

This is a **ground-up rewrite**, not a port. The retired Apache Wave Java codebase (on branch `legacy/apache-wave`) serves as an architectural reference: its data model, protocol definitions, and federation design are proven and documented in source, and this PRD maps each legacy concept to its modern replacement.

---

## 2. Goals & Non-Goals

### 2.1 Goals

| # | Goal |
|---|------|
| G1 | Real-time collaborative editing: multiple users concurrently edit the same rich-text content with live cursors and sub-100ms local echo. |
| G2 | Threaded conversations: blips (messages) organized in trees within a wave, including inline replies anchored to text. |
| G3 | Presence & awareness: who is online, who is viewing, live remote cursors and selections. |
| G4 | Playback: any wave can be replayed from its beginning; history is never discarded. |
| G5 | Federation: independent ProtoWave servers exchange waves so users on different servers collaborate as peers. Designed in from day one; shipped in Phase 3. |
| G6 | Live translation: streaming, context-revising machine translation between participants' languages, powered by a Gemini Flash-Lite-class model. |
| G7 | Markdown: authoring shortcuts in the editor and native rendering of shared `.md` files. |
| G8 | Document management: files attached to and shared within waves, with metadata, previews, and search. |
| G9 | Distributed file sharing: P2P, content-addressed folder sharing (libp2p/IPFS-style) with the home server as optional always-on mirror. |
| G10 | Efficiency as a requirement: every hot path carries an explicit Big-O budget (§6.1) and a wall-clock SLO (§6.2). |
| G11 | Self-hostability: a single-binary server that a hobbyist can run, and that scales to organizational deployments. |

### 2.2 Non-Goals (v1)

- **Wire compatibility with legacy Wave/XMPP federation.** The old XMPP transport was removed from Apache Wave before retirement; ProtoWave defines a new signed gRPC federation protocol.
- **Gadget/robot API parity.** Wave's extension mechanisms are deferred to Phase 6 as a redesigned, sandboxed extension API.
- **Native mobile apps.** The web client is a responsive PWA; native apps are future work.
- **End-to-end encryption.** E2EE fundamentally conflicts with server-side translation (the server must read content to translate it). E2EE is flagged as future work with the tension documented (§14, R9).
- **Interop bridges (Matrix, Slack, email).** Interesting, out of scope for v1.

---

## 3. Personas & User Stories

### 3.1 Personas

- **P1 — Dana, distributed team lead.** Runs a 12-person team across 5 time zones. Needs threaded discussion that becomes the document — decisions, specs, and the debate that produced them in one place. Uses playback to catch up on what happened overnight.
- **P2 — Miguel, multilingual community moderator.** Moderates an open-source community spanning Spanish, English, Mandarin, and French speakers. Needs conversation to flow across languages without members switching tools or copy-pasting into translators.
- **P3 — Ade, self-hoster / sysadmin.** Runs services for a co-op. Needs a single binary, sane defaults, low resource floor, and federation so members can collaborate with people on other servers without surrendering data to a central operator.
- **P4 — Priya, researcher sharing large datasets.** Collaborates on multi-GB folders of data and papers. Needs to share a folder from her machine without uploading everything to a server first, with integrity verification and resumable transfers.

### 3.2 User Stories

**Collaboration & conversation**

- US-1 (P1): As a team lead, I create a wave, add participants, and we co-edit the root blip simultaneously with visible cursors, so a meeting agenda becomes meeting notes becomes the decision record. *Acceptance: 5 concurrent editors, no lost edits, cursors visible, local echo < 100ms p95.*
- US-2 (P1): As a participant, I reply inline to a specific sentence, creating a thread anchored to that text, so context is never lost. *Acceptance: anchor survives concurrent edits to surrounding text.*
- US-3 (P1): As a late joiner, I replay the wave to watch how the discussion evolved, so I can catch up without asking anyone. *Acceptance: playback slider seeks to any historical version in < 500ms p95.*
- US-4 (P1): As a participant, I see unread blips marked per wave and an inbox ordered by recent activity. *Acceptance: unread state syncs across my sessions.*
- US-5 (any): As a user, I keep typing while offline or during a network blip, and my edits merge cleanly when I reconnect. *Acceptance: no conflict dialogs, ever; CRDT merge is automatic.*

**Translation**

- US-6 (P2): As a Spanish speaker, I read English messages in Spanish as they are being typed, with earlier words revising themselves as the sentence's meaning becomes clear. *Acceptance: first translated words < 1.5s after typing starts; revisions render without flicker.*
- US-7 (P2): As a moderator, I set my preferred language once and every wave respects it. *Acceptance: per-user language setting; per-wave translation opt-in toggle.*
- US-8 (P2): As a wave creator, I must explicitly enable translation for a wave, and participants are told content will be sent to a third-party model API. *Acceptance: translation off by default; disclosure shown on enable.*
- US-9 (P2): As a participant, I can always toggle between the translation and the original text. *Acceptance: one click/keystroke; original is the stored truth.*

**Markdown & documents**

- US-10 (P1): As a writer, I type markdown (`# `, `**bold**`, `- `, ``` ``` ```) and it converts live to rich text. *Acceptance: standard input rules; undo restores the literal text.*
- US-11 (P1): As a participant, I share a `.md` file into a wave and everyone sees it rendered with code highlighting. *Acceptance: CommonMark + GFM tables; sanitized output.*
- US-12 (P1): As a participant, I attach files to a blip; others preview images inline and download anything. *Acceptance: drag-and-drop upload; thumbnails; content-addressed dedup.*
- US-13 (P1): As a participant, I search across wave text and attachment names. *Acceptance: results < 300ms p95 at 1M waves.*

**P2P sharing**

- US-14 (P4): As a researcher, I add a local folder to ProtoWave and share it into a wave; recipients browse the manifest and fetch files from my machine directly. *Acceptance: no full upload required; transfer is chunk-verified.*
- US-15 (P4): As a recipient, I fetch a shared folder from *any* online holder — the sharer, another recipient, or the server mirror. *Acceptance: swarm download from ≥ 2 sources when available.*
- US-16 (P4): As a sharer, I update files in a shared folder and re-publish; recipients fetch only changed chunks. *Acceptance: content-defined chunking dedups unchanged data.*
- US-17 (P4): As a wave owner, I opt the home server in as an always-on mirror so shares survive my laptop sleeping. *Acceptance: mirror seeds when the origin is offline.*

**Federation & self-hosting**

- US-18 (P3): As a self-hoster, I download one binary, run it with one config file, and my server works — including later joining federation. *Acceptance: `protowave-server` starts with zero external services; embedded storage.*
- US-19 (P3): As a user on server A, I add `friend@server-b.example` to my wave and we co-edit live. *Acceptance: cross-server edit latency < 400ms p95 same-region.*
- US-20 (P3): As a server admin, I block abusive remote servers and remove local content. *Acceptance: server blocklist; admin removal tooling.*
- US-21 (P3): As a participant, my server keeps a full replica of every wave I'm in, so a remote server going down never locks me out of content. *Acceptance: reads and edits work during remote outage; changes reconcile on recovery.*

---

## 4. Concepts & Data Model

### 4.1 Vocabulary

| Term | Definition |
|------|-----------|
| **Wave** | A container for a conversation-document. Identified by `waveId = domain/random-id`. A wave contains one or more wavelets. |
| **Wavelet** | The unit of replication, access control, and federation. A wave has a root conversation wavelet plus optional private wavelets (e.g., a side-conversation among a subset of participants, or per-user data such as read state). |
| **Blip** | A single message/document node within a wavelet. Blips form a tree: the root blip, replies, and inline replies anchored to positions in a parent blip's text. |
| **Document** | A blip's content: rich text with structural nodes and formatting attributes (the successor of legacy Wave's XML-documents-with-ranged-annotations). |
| **Participant** | A user identified as `user@domain` — federation-shaped from day one, exactly as legacy Wave's `ParticipantId`. |
| **Update** | An atomic CRDT change (yrs binary update). The append-only signed **update log** per wavelet is the source of truth and the substrate for playback. |
| **Snapshot** | A serialized materialization of a wavelet's CRDT state at a version, taken every *k* updates to bound load and seek time. |
| **Folder share** | A signed, content-addressed manifest of a local folder published into a wave for P2P fetching. |

### 4.2 Legacy → ProtoWave mapping

| Legacy Apache Wave (Java) | ProtoWave |
|---|---|
| `org.waveprotocol.wave.model.wave` — Wave/Wavelet/Blip | Same hierarchy; wavelet = one yrs `Doc` |
| XML document + ranged annotations (`model.document`) | `Y.XmlFragment`/`XmlText` with format attributes |
| Operational Transformation (`model.operation`, concurrency control engine) | CRDT (yrs); no transformation, updates commute (ADR-1) |
| Wavelet deltas + hashed versions, signed for federation (`federation.protodevel`) | Signed update batches + state vectors (§8.3) |
| Conversation manifest (`model.conversation` — `DocumentBasedManifest`, thread tree) | CRDT conversation manifest: `Y.Map`/`Y.Array` of blip/thread structure |
| `waveclient-rpc.proto` — Open (streaming updates), Submit, Authenticate over WebSocket | Protobuf envelope over WebSocket: multiplexed sync/awareness/translation/notification channels (§8.1) |
| Pluggable persistence (file / memory / MongoDB / Lucene) | `WaveStore` trait: RocksDB embedded (default), PostgreSQL (Phase 2); tantivy for search |
| `AttachmentStore` + servlets | Content-addressed blob store (BLAKE3 CAS), shared with the P2P layer |
| Robots & gadgets | Deferred: sandboxed extension API (Phase 6) |
| GWT client (`EditorImpl`, wavepanel MVP) | Vue 3 SPA; Tiptap 2 + y-prosemirror editor |

### 4.3 Wavelet data model (normative)

Each wavelet is **one yrs `Doc`** containing:

- `blips: Y.Map<BlipId, Y.XmlFragment>` — blip content, rich text.
- `manifest: Y.Map` — conversation structure: thread tree, blip ordering, inline-reply anchors (as yrs relative positions, which survive concurrent edits — satisfies US-2).
- `meta: Y.Map` — title, tags, wave-level settings that participants may edit.

**Deliberately outside the CRDT** (server-authoritative, signed, versioned):

- **Participant list and ACLs.** Access control must not be merge-resolved by a CRDT; membership changes are ordered, signed control-plane events issued by the wavelet's home server (§8.3). This mirrors legacy Wave's authoritative wavelet host and is critical for federation security.
- **Attachment blobs.** The CRDT stores references (BLAKE3 hashes); bytes live in the CAS.
- **Translations.** Ephemeral derived state, never stored in the document (§9).

**Persistence per wavelet:** append-only signed update log + snapshot every *k* updates (default *k* = 500). The log is **never truncated** — playback (G4) is a feature, so compaction means "snapshot + keep log," not "discard history." Open cost is O(snapshot + tail); playback seek is O(k) from the nearest snapshot (§6.1).

---

## 5. Functional Requirements

Priorities: **P0** = MVP-blocking, **P1** = required for the full vision, **P2** = desirable. Phase tags reference §12.

### 5.1 Accounts & authentication

- **FR-1 (P0, Ph1)** Users register with `user@domain` identity; passwords hashed with argon2id.
- **FR-2 (P0, Ph1)** Cookie-based sessions over TLS; session revocation; concurrent sessions per user.
- **FR-3 (P1, Ph2)** OIDC login (Google/GitHub/generic) as an alternative to passwords.
- **FR-4 (P1, Ph2)** User profiles: display name, avatar, **preferred language** (drives translation targeting).

### 5.2 Wave lifecycle

- **FR-5 (P0, Ph1)** Create a wave; creator is first participant of the root wavelet.
- **FR-6 (P0, Ph1)** Add/remove participants by address; participants see the wave in their inbox.
- **FR-7 (P0, Ph1)** Open a wave: client receives snapshot + log tail, then live updates (streaming, like legacy `Open`).
- **FR-8 (P1, Ph2)** Per-user read/unread state per blip (private per-user wavelet).
- **FR-9 (P1, Ph2)** Wave-level tags and folders; archive/mute.
- **FR-10 (P2, Ph2)** Public waves (readable by anyone on the server; participation still explicit).

### 5.3 Editor & documents

- **FR-11 (P0, Ph1)** Rich-text collaborative editing: bold/italic/underline/strikethrough, headings, lists, links, code blocks, blockquotes.
- **FR-12 (P0, Ph1)** Concurrent edits from N clients converge with no user-visible conflicts (CRDT).
- **FR-13 (P0, Ph1)** Live remote cursors and selections (yrs awareness protocol).
- **FR-14 (P0, Ph1)** Undo/redo scoped to the local user's own edits.
- **FR-15 (P1, Ph2)** Mentions (`@user`) with autocomplete; mention notifications.
- **FR-16 (P1, Ph2)** Markdown input rules: typing markdown syntax converts live to rich text (US-10).
- **FR-17 (P2, Ph6)** Paste/import of markdown and HTML with sanitization.

### 5.4 Threading & conversation

- **FR-18 (P0, Ph1)** Reply blips forming a thread tree; collapse/expand threads.
- **FR-19 (P1, Ph2)** Inline replies anchored to a text position; anchors survive concurrent edits (relative positions).
- **FR-20 (P1, Ph2)** Blip-level metadata: author(s), contributors, timestamps, edit indicator.
- **FR-21 (P2, Ph2)** Blip deletion (tombstone in manifest; content removed from *rendered* state; log retention per §6.3 and admin redaction per FR-62).

### 5.5 Presence & awareness

- **FR-22 (P0, Ph1)** Per-wave presence: who has the wave open now.
- **FR-23 (P0, Ph1)** Awareness events (cursor, selection, typing) coalesced to ≤ 4 Hz per user before fanout.
- **FR-24 (P2, Ph2)** Inbox-level presence (online/idle/offline) for contacts.

### 5.6 Playback

- **FR-25 (P1, Ph2)** Playback UI: slider over wavelet history; seek to any version; step by update batch.
- **FR-26 (P1, Ph2)** Seek materializes state from nearest snapshot + replay, O(k) (§6.1).
- **FR-27 (P2, Ph2)** "Diff since my last visit" view built on the same machinery.

### 5.7 Search & inbox

- **FR-28 (P0, Ph1)** Inbox: waves ordered by last activity, with digest snippet (title + recent content).
- **FR-29 (P1, Ph2)** Full-text search over blip text via embedded tantivy index; incremental indexing on update.
- **FR-30 (P1, Ph2)** Search filters: participant, tag, date range, has:attachment.
- **FR-31 (P2, Ph3)** Search covers federated waves replicated to the local server.

### 5.8 Markdown rendering

- **FR-32 (P1, Ph2)** `.md` attachments render natively: CommonMark + GFM (tables, task lists, strikethrough, autolinks).
- **FR-33 (P1, Ph2)** All rendered markdown/HTML is sanitized (DOMPurify); no script execution.
- **FR-34 (P1, Ph2)** Code blocks syntax-highlighted (shiki), in both markdown files and editor code blocks.

### 5.9 Attachments & document management

- **FR-35 (P0, Ph2)** Upload attachments to a blip (drag-and-drop, picker); download; image/PDF inline preview + thumbnails.
- **FR-36 (P1, Ph2)** Attachments stored in a BLAKE3 content-addressed store; identical content deduplicates automatically.
- **FR-37 (P1, Ph2)** Attachment metadata (name, MIME type, size, uploader, hash) recorded in the wavelet; searchable (FR-30).
- **FR-38 (P1, Ph2)** Per-wave document panel listing all files across blips.
- **FR-39 (P2, Ph2)** Server-side size quotas per user/server; configurable.

### 5.10 Translation engine (§9 is normative for design)

- **FR-40 (P1, Ph4)** Per-wave translation toggle (off by default) with third-party-API disclosure (US-8).
- **FR-41 (P1, Ph4)** Viewers with a preferred language different from a blip's source language see a live translated overlay; original always one toggle away (US-9).
- **FR-42 (P1, Ph4)** Streaming translation begins while the author is still typing; already-rendered words are revised in place as context improves (US-6).
- **FR-43 (P1, Ph4)** Finalized sentence translations are cached; replays, rejoins, and scrollback are served from cache at zero model cost.
- **FR-44 (P1, Ph4)** Token budgets per user and per server; on exhaustion, translation degrades gracefully to on-demand ("translate this blip") with clear UI state.
- **FR-45 (P2, Ph4)** Per-wave glossary (term → preferred translation) injected into translation context.
- **FR-46 (P1, Ph4)** Translation provider behind a `Translator` trait; Gemini Flash-Lite-class is the reference implementation, swappable by config.

### 5.11 Federation (§8.3 is normative for protocol)

- **FR-47 (P1, Ph3)** Server discovery via `https://<domain>/.well-known/protowave`; server identity = ed25519 keypair.
- **FR-48 (P1, Ph3)** Adding a remote participant causes their server to receive and maintain a full replica of the wavelet (content plane).
- **FR-49 (P1, Ph3)** Every federated update batch is signed by the emitting server; receivers verify before applying.
- **FR-50 (P1, Ph3)** Anti-entropy: periodic state-vector exchange heals missed updates after partitions.
- **FR-51 (P1, Ph3)** Membership/ACL changes are issued only by the wavelet's home server as signed, versioned control events; remote servers reject content updates that violate the ACL version they carry.
- **FR-52 (P1, Ph3)** Server admin blocklist for remote domains; per-wave participant bans.
- **FR-53 (P2, Ph3)** Two-server interoperability test suite runs in CI (spin up two instances, federate a wave, assert convergence).

### 5.12 P2P distributed file sharing (§11 is normative for design)

- **FR-54 (P1, Ph5)** "Add folder" in the client registers a local folder with the local ProtoWave node: files are chunked (FastCDC), hashed (BLAKE3), and a signed manifest is produced.
- **FR-55 (P1, Ph5)** Share a folder manifest into a wave as a browsable card (file tree, sizes, hashes).
- **FR-56 (P1, Ph5)** Recipients fetch content P2P over QUIC with hole-punching and relay fallback; every chunk is incrementally verified against its hash before acceptance.
- **FR-57 (P1, Ph5)** Swarm behavior: fetch from any holder (origin, other recipients, server mirror); parallel multi-source download.
- **FR-58 (P1, Ph5)** Optional home-server mirroring of shared folders (opt-in per share) so content survives the origin going offline.
- **FR-59 (P1, Ph5)** Re-publishing a modified folder produces a new manifest version; unchanged chunks are not re-transferred (content-defined chunking dedup).
- **FR-60 (P2, Ph5)** Bandwidth limits (up/down) configurable per node.

### 5.13 Administration & moderation

- **FR-61 (P1, Ph2)** Admin role: user management, storage quotas, server settings.
- **FR-62 (P1, Ph3)** Admin content redaction: remove blip content and attachments from serving *and* from log replay (redaction event supersedes history for the redacted range; the only sanctioned exception to "never truncate").
- **FR-63 (P2, Ph6)** Basic anti-abuse: rate limits on wave creation, participant adds, and federation traffic.

---

## 6. Non-Functional Requirements

### 6.1 Complexity budgets (normative)

Every implementation of a hot path below must meet its Big-O budget; regressions are release blockers. *n* = blocks in a document, *k* = updates in a batch or since last snapshot, *s* = subscribed sessions on a wavelet, *N* = waves on the server.

| ID | Operation | Time budget | Space notes |
|----|-----------|------------|-------------|
| NFR-C1 | Apply local edit (yrs op) | O(log n) amortized | yrs block store |
| NFR-C2 | Integrate remote update batch | O(k log n) | |
| NFR-C3 | Open wave (client) | O(snapshot) + O(tail), tail ≤ k by snapshot policy | streamed, not buffered whole |
| NFR-C4 | Playback seek to version v | O(k) from nearest snapshot — **never** O(v) | snapshots every k updates |
| NFR-C5 | Presence/awareness fanout | O(s) per event; events coalesced ≤ 4 Hz per user | tokio broadcast per wavelet |
| NFR-C6 | Inbox query / full-text search | O(log N + page) via tantivy | incremental indexing O(changed terms) |
| NFR-C7 | Translation work per edit | O(changed sentences) — **never** O(document) | cache keyed by sentence+context digest |
| NFR-C8 | P2P chunk provider lookup | O(log peers) DHT-style hops; local chunk O(1) | |
| NFR-C9 | P2P chunk verification | O(chunk) incremental (BLAKE3/bao verified streaming) | no full-file buffering |
| NFR-C10 | Client wave render | O(viewport) — virtualized; **never** O(blips) | lazy-load blip fragments |

### 6.2 Latency & capacity SLOs

- **NFR-1** Local edit echo (keystroke → own screen): < 16ms. Edit propagation to same-server peers: < 100ms p95 same region.
- **NFR-2** Presence updates visible: < 250ms p95 same server.
- **NFR-3** Wave open (1 MB document, warm server): < 200ms p95 server time.
- **NFR-4** Search: < 300ms p95 at 1M indexed waves.
- **NFR-5** Cross-server (federated) edit propagation: < 400ms p95 same region.
- **NFR-6** First translated tokens visible: < 1.5s p95 after typing begins (model latency dominated).
- **NFR-7** Single-server capacity target (reference hardware, 8 vCPU / 16 GB): 10k concurrent WebSocket sessions, 1M stored waves, 100 updates/sec sustained per hot wavelet.
- **NFR-8** Server cold start: < 2s to accepting connections (embedded storage, no external services).

### 6.3 Storage & space

- **NFR-9** Update logs are append-only and retained indefinitely (playback is a feature). Compaction = add snapshots; never delete log entries. Sole exception: admin redaction (FR-62).
- **NFR-10** Snapshot interval k tunable per deployment; default 500 updates. Expected overhead documented: snapshot size ≈ document size; log growth ≈ bytes-per-update × edit rate.
- **NFR-11** yrs tombstone overhead is bounded by document edit history; wavelets are the granularity cap — a wave with pathological blip counts splits across wavelets (open question R8).
- **NFR-12** Translation cache bounded by LRU (memory) + RocksDB (disk) with configurable ceiling.
- **NFR-13** CAS blobs are deduplicated by hash; garbage collection removes blobs unreferenced by any wavelet or active share.

### 6.4 Security & privacy

- **NFR-14** All transport TLS; passwords argon2id; sessions httpOnly + SameSite cookies; CSRF protection on REST.
- **NFR-15** Federation: ed25519 server keys published via `.well-known`, pinned on first contact; all s2s batches signed; replay protection via monotonic batch sequence + state vectors.
- **NFR-16** Translation privacy: content leaves the server only for waves where translation is explicitly enabled; the disclosure names the provider. No content is sent for translation-disabled waves. Logged prompts must be excluded from provider-side training via API configuration where available.
- **NFR-17** All user-generated rendered content (markdown, links, file names) sanitized against XSS; attachments served with `Content-Disposition` and type sniffing protections.
- **NFR-18** P2P shares are readable only by holders of the manifest (capability-style: the manifest contains the content hashes required to fetch); server mirror enforces wave ACLs when serving.

### 6.5 Accessibility & i18n

- **NFR-19** WCAG 2.1 AA for the web client; full keyboard navigation of inbox, wave, and editor.
- **NFR-20** UI localization: en, de, es, fr, ru, zh-TW, sl at launch parity with the legacy client; string externalization from day one (vue-i18n).

### 6.6 Reliability

- **NFR-21** A client offline for any duration reconnects and converges without user intervention (CRDT property; must be tested with 24h-offline scenarios).
- **NFR-22** Crash safety: an update acknowledged to a client is durably in the log (fsync policy configurable; default on).
- **NFR-23** Federated wavelet replicas remain readable and editable during remote-server outage; control-plane changes (membership) queue until the home server returns (§8.3).

---

## 7. Architecture Overview

```
┌─────────────────────────────── Vue 3 SPA (PWA) ────────────────────────────────┐
│ Pinia stores │ Tiptap 2 editor + y-prosemirror │ virtualized wave/inbox views  │
│ markdown-it + DOMPurify + shiki │ translation overlay decorations              │
└───────────────────────────────┬────────────────────────────────────────────────┘
                                │ WebSocket (protobuf envelope, multiplexed) + REST
┌───────────────────────────────▼────────────────────────────────────────────────┐
│                        protowave-server (single Rust binary)                    │
│  axum gateway ── auth/sessions ── rate limiting                                 │
│  ┌───────────────┬───────────────┬───────────────┬────────────────────────┐    │
│  │ wave engine   │ presence      │ translation   │ federation service     │    │
│  │ (yrs docs +   │ (awareness    │ service       │ (tonic gRPC, ed25519,  │    │
│  │  update log)  │  broadcast)   │ (Translator   │  state-vector sync)    │    │
│  │               │               │  trait→Gemini)│                        │    │
│  ├───────────────┴───────────────┴───────────────┴────────────────────────┤    │
│  │ storage: WaveStore trait → RocksDB (default) / PostgreSQL (Phase 2)    │    │
│  │ search: tantivy (embedded)   blobs: BLAKE3 CAS (fs, S3 optional)       │    │
│  └────────────────────────────────────────────────────────────────────────┘    │
│  blob/P2P service (iroh): chunking, manifests, QUIC swarm, server mirror        │
└──────────────┬──────────────────────────────────────────────┬──────────────────┘
               │ tonic gRPC over TLS (federation)              │ iroh QUIC (P2P)
        other ProtoWave servers                     peer nodes (clients, mirrors)
```

**Components (one paragraph each):**

- **Gateway (axum + tokio).** Terminates HTTP/WebSocket, authenticates sessions, routes the multiplexed envelope channels. Tower middleware for rate limiting, compression, metrics.
- **Wave engine (yrs).** Owns in-memory yrs `Doc`s for hot wavelets (LRU-evicted), applies local and remote updates, appends to the signed update log, emits snapshots every k updates, fans out updates to subscribed sessions.
- **Presence service.** Ephemeral awareness state per wavelet via tokio broadcast channels; coalesces per-user events to ≤ 4 Hz; never persisted.
- **Storage layer.** `WaveStore` trait (mirrors legacy pluggable persistence): update logs, snapshots, accounts, wave metadata. Default RocksDB (embedded, single-binary, LSM suits append-heavy logs); PostgreSQL implementation in Phase 2 for operators who want managed SQL. redb is the pure-Rust fallback if RocksDB build friction warrants.
- **Search (tantivy).** Embedded full-text index (the Rust Lucene, replacing legacy Lucene); incrementally indexes blip text and attachment names on update.
- **Translation service.** Consumes document change streams, runs the §9 pipeline, emits overlay patches on the translation channel. Provider behind `Translator` trait.
- **Federation service (tonic gRPC).** Pushes signed update batches to peer servers, runs anti-entropy state-vector exchange, validates signatures and ACL versions on ingress. Protocol spec lives in `docs/federation-spec.md` (authored in Phase 0, implemented in Phase 3).
- **Blob/P2P service (iroh).** One content-addressed layer for both attachments and folder shares: BLAKE3 CAS, bao verified streaming, iroh QUIC with hole-punching and relay fallback; the server doubles as an optional always-on mirror.
- **Vue 3 SPA.** §10.

### 7.1 ADR-1 — CRDT (yrs) over OT, and over diamond-types

**Decision:** use CRDTs, specifically **yrs** (the Rust port of Yjs).

**Why CRDT over OT (legacy Wave's model):** OT requires an authoritative sequencer to totally order and transform operations — the root of legacy Wave's host/remote federation asymmetry and its client-side concurrency-control engine's complexity. CRDT updates commute: merges need no authority, which makes offline editing trivial (US-5), federation symmetric (§8.3), and eliminates the transform engine entirely.

**Why yrs over diamond-types:** (1) *Rich text:* yrs `XmlFragment`/`XmlText` with format attributes maps directly onto Wave's annotated-XML document model; diamond-types is plain-text only today — a non-starter for blips. (2) *Ecosystem:* yrs shares its binary wire format with Yjs (JS), so `y-prosemirror` + Tiptap gives a production-grade collaborative editor against our Rust server with zero translation layer. (3) *Awareness:* the ecosystem's presence/cursor protocol replaces a chunk of custom engineering. (4) *Federation:* yrs sync (state-vector exchange → diff) is symmetric and serves directly as the s2s anti-entropy mechanism.

**Costs accepted:** yrs carries tombstone/metadata overhead vs. diamond-types' raw plain-text speed; playback requires us to persist the update log ourselves (yrs does not retain seekable history) — hence the log+snapshot design in §4.3.

### 7.2 ADR-2 — Hybrid federation topology

**Decision:** content plane fully replicated Matrix-style; control plane home-server-authoritative Wave-style. See §8.3. **Why:** full content replication gives partition tolerance and local-first reads/writes (NFR-23); keeping membership/ACLs on a single authoritative server per wavelet deliberately avoids Matrix's state-resolution algorithm — historically its hardest and most CVE-prone component. **Cost accepted:** membership changes are unavailable while a wavelet's home server is down (content editing still works). Control-plane portability is an open question (R7).

### 7.3 ADR-3 — Editor: Tiptap 2 + y-prosemirror, not a custom editor

**Decision:** build the editor on Tiptap 2 (ProseMirror) with the y-prosemirror Yjs binding. **Why:** the custom editor was the single largest source of complexity in the legacy client (`EditorImpl` and the doodad framework, ~100+ files); Tiptap+Yjs erases that entire category. Wave-specific behavior lands as Tiptap extensions (§10). **Cost accepted:** ProseMirror's schema constrains document shapes; mapping legacy annotation semantics needs an early spike (R5).

### 7.4 ADR-4 — P2P stack: iroh, not raw rust-libp2p

**Decision:** iroh + iroh-blobs. **Why:** it *is* the IPFS-style architecture the requirement calls for — BLAKE3 content addressing, bao incremental verification, QUIC hole-punching with relay fallback — with roughly an order of magnitude less protocol assembly than rust-libp2p (Kademlia + custom bitswap + NAT traversal). **Fallback:** raw rust-libp2p only if interop with the public IPFS DHT becomes a requirement. **Cost accepted:** iroh API churn; pin versions and keep everything behind a blob-service trait (R6).

### 7.5 ADR-5 — Storage: RocksDB + PostgreSQL; no MongoDB

**Decision:** embedded RocksDB is the default `WaveStore`; PostgreSQL is the external option (Phase 2). MongoDB — a legacy Wave backend — is deliberately not carried forward. **Why:** the hot path is an append-only update log + snapshots (sequential writes, range reads — an LSM workload, not a document-query workload); blip content is opaque yrs binary that no database can usefully query into; full-text search belongs to tantivy; and the genuinely relational data (accounts, ACLs, metadata) is Postgres's home turf. Requiring MongoDB would also break the single-binary self-host goal (G11). **Cost accepted:** none identified — no workload in the system wants a document store.

---

## 8. Protocol & API Design

### 8.1 Client ↔ server

- **Transport:** one WebSocket per client session, protobuf envelope (prost), multiplexing logical channels: `sync` (yrs updates as opaque bytes — yrs's lib0 encoding is already optimal; never re-encode), `awareness`, `translation`, `notification`, `control` (subscribe/unsubscribe wavelets).
- **Semantics** (successor of legacy `waveclient-rpc.proto`): `Subscribe(waveletId, knownStateVector)` → server streams snapshot-or-diff, then live updates (like legacy `Open`); client sends updates as they occur (like legacy `Submit`, but commutative — no version negotiation); server acks with durable log position.
- **REST (axum)** for non-realtime: auth, registration, profile, wave CRUD/listing, search, attachment upload/download, folder-share manifests, admin.
- The protobuf schema lives in a dedicated `protowave-proto` crate (echoing legacy's protocol-first discipline and the PST codegen pipeline), consumed by server and generated TS client.

### 8.2 Identity & discovery

- Users: `user@domain`. Waves: `domain/waveId`; wavelets: `domain/waveId/waveletId` (federation-qualified from day one, exactly like legacy `WaveletName`).
- Servers publish `https://<domain>/.well-known/protowave`: gRPC endpoint, ed25519 public keys (with key rotation metadata), supported protocol versions.

### 8.3 Server ↔ server federation

- **Transport:** tonic gRPC over TLS. Versioned protobuf schema; the full spec is authored as `docs/federation-spec.md` in Phase 0 so the MVP's IDs, signatures, and log format are federation-shaped before federation ships.
- **Content plane (Matrix-style replication):** every server with ≥ 1 participant maintains a full replica of the wavelet. Emitting servers push signed update batches eagerly; periodic **state-vector exchange** (yrs native) performs anti-entropy after partitions or missed pushes. Any replica accepts local edits at any time — partition-tolerant by construction (NFR-23).
- **Control plane (Wave-style authority):** the wavelet's **home server** (its origin domain) is the sole authority for membership, ACLs, and policy, published as a signed, versioned *wavelet state document*. Content-plane batches carry the ACL version they were authored under.
- **Signing:** every federated batch signed with the emitting server's ed25519 key (the successor of legacy hashed-version delta signing); receivers verify signature, sender membership, and ACL version before applying.
- **Removed-participant window (CRDT-specific):** a removed user's server may hold concurrent unmerged edits. Rule: servers reject batches whose ACL version predates the removal once the removal event is seen; the bounded residual window (edits merged by third parties before the removal propagates) is documented behavior, mitigated by fast control-plane propagation.
- **Abuse controls:** per-server blocklists (FR-52), rate limits on ingress (FR-63), signed batches give an audit trail.

---

## 9. Translation Engine Design

**Principle: translations are ephemeral overlays, never document content.** The CRDT stores only what the author wrote. Translations are derived state keyed by `(waveletId, blipId, targetLang, sourceStateVector)`, delivered on the `translation` WebSocket channel, and rendered as ProseMirror decorations over the source blip. Toggling to the original (US-9) merely hides the overlay.

**Pipeline** — runs per (blip, targetLang) with ≥ 1 active viewer whose preferred language differs from the blip's detected source language, on translation-enabled waves only:

1. **Segment.** Blip text split into sentences (`unicode-segmentation` + language-aware rules). All downstream work is sentence-scoped: cost is **O(changed sentences), never O(document)** (NFR-C7).
2. **Debounce.** Source CRDT changes trigger translation on *400ms typing idle* OR *sentence-boundary commit*, whichever fires first.
3. **Assemble context.** The changed sentence(s) + preceding N source sentences + the **current draft translation** of those sentences + the wave glossary (FR-45). The prompt explicitly permits the model to *revise its previous draft* — this is the mechanism behind live self-correcting translation (US-6): as context accumulates, the model re-emits improved wording for words it already produced.
4. **Stream & diff.** Model tokens stream in; the service diffs the stream against the current shadow translation and emits **minimal patch ops** to viewers. Recipients see words appear, and earlier words revise in place without flicker.
5. **Freeze & cache.** Once a sentence's source has been stable for ~5s and the author's cursor has moved past it, its translation is marked final and cached under `hash(sentence ‖ context-digest ‖ langpair ‖ model-revision)` — RocksDB-backed with in-memory LRU. Cache hits serve replay, rejoin, and scrollback at zero model cost (FR-43).

**Cost controls (hard requirements, not optimizations)** — the live-revision loop multiplies token spend, so these are load-bearing (R3):

- Translate only **viewer-visible** blips: the client's viewport drives translation subscriptions (NFR-C10 synergy).
- Per-user and per-server **token budgets** (FR-44) with graceful degradation to on-demand translation.
- **Batch** adjacent dirty sentences into a single model request; cap context tokens.
- Optional **two-tier mode**: Flash-Lite-class model for live drafts, a stronger model for finalization pass.

**Provider abstraction.** `Translator` trait (async, streaming): the reference implementation targets a Gemini Flash-Lite-class model via API; provider, model name, and endpoint are config. Nothing outside the translation service knows which provider is in use (FR-46).

**Privacy.** Off by default per wave; enabling shows a disclosure naming the provider (FR-40, NFR-16).

---

## 10. Frontend Specification

- **Stack:** Vue 3 + Composition API + TypeScript + Vite; **Pinia** for state; vue-router; vue-i18n (NFR-20).
- **Editor:** **Tiptap 2** with **y-prosemirror** binding to yrs via a custom provider speaking our WebSocket envelope (a y-websocket-derived sync protocol on the `sync` + `awareness` channels). Custom Tiptap extensions: blip boundaries, inline-reply anchors, mentions, attachment cards, folder-share cards, translation overlay decorations.
- **Markdown:** `markdown-it` + **DOMPurify** for rendering `.md` files and read-only content; **tiptap-markdown** + input rules for authoring (FR-16); **shiki** for highlighting (FR-34).
- **Rendering discipline:** inbox and wave views virtualized with TanStack Virtual — a wave with 5k blips renders O(viewport), never O(blips) (NFR-C10); blip fragments lazy-load on scroll.
- **UI kit:** Reka UI (headless, accessible primitives — the Vue continuation of Radix UI, formerly "Radix Vue") + Tailwind CSS; WCAG 2.1 AA (NFR-19). Visual design must be distinctive and crafted, not template-generic.
- **PWA:** installable, offline shell; offline *editing* rides on yrs local persistence (y-indexeddb-equivalent over the same provider), reconciling on reconnect (US-5, NFR-21).
- **State boundaries:** Pinia owns app state (session, inbox, settings); yrs owns document state — never duplicate document content into Pinia.

---

## 11. P2P Folder Sharing Design

- **Node roles.** The user's client (via a local node component) and the home server both run the iroh-based blob service. Clients can fetch directly from each other; the server is one more peer that happens to be always-on.
- **Add folder (FR-54).** Files are chunked with **FastCDC** (content-defined chunking → cross-version dedup, FR-59), chunks hashed with **BLAKE3**, and the folder becomes a signed **manifest** (a merkle collection: paths → content hashes → chunk trees).
- **Share (FR-55).** The manifest reference is embedded in a wave as a browsable card. Possession of the manifest is the read capability (NFR-18); wave ACLs gate who ever sees it.
- **Fetch (FR-56, FR-57).** Recipients resolve providers for each hash and fetch over iroh QUIC (hole-punching, relay fallback) from any holder — origin, other recipients, or the server mirror — in parallel. Every chunk verifies incrementally via bao streaming: O(chunk) verification, no trust in any peer (NFR-C9).
- **Mirroring (FR-58).** Opt-in per share: the home server pins the manifest's content and seeds when the origin is offline — solving the classic torrent cold-start/seeder-offline problem while keeping the system functional with zero server storage if the user declines.
- **Updates (FR-59).** Re-publishing produces manifest v2 referencing mostly the same chunks; fetchers transfer only the delta.
- **Unification.** Ordinary attachments (FR-35) use the same BLAKE3 CAS — an attachment is simply server-mirrored-by-default single-file content. One storage and transfer layer serves both features.

---

## 12. Phasing & Milestones

| Phase | Scope | Exit criteria |
|-------|-------|---------------|
| **0 — Foundations** | Legacy Java preserved on `legacy/apache-wave` (done); master transitions to Cargo workspace (`server/`, `crates/protowave-proto`, `crates/protowave-core`) + `web/` Vue app; protobuf schemas; federation-shaped IDs and signing primitives; `docs/federation-spec.md` draft; CI (fmt, clippy, test, web build). | Empty-but-wired server & client build and talk (auth + echo channel) in CI. |
| **1 — MVP: single-server collaboration** | Accounts/auth (FR-1..2), wave lifecycle (FR-5..7), Tiptap+yrs co-editing (FR-11..14), threading (FR-18), presence/cursors (FR-22..23), RocksDB persistence, basic inbox (FR-28). | **Two users co-edit a threaded wave with live cursors**; NFR-1 met; offline reconnect converges. |
| **2 — Wave parity** | Search (FR-29..30), attachments (FR-35..39), markdown (FR-16, FR-32..34), playback (FR-25..27), read state (FR-8), inline replies (FR-19), participant mgmt/ACLs, admin basics (FR-61), PostgreSQL backend, OIDC (FR-3). | Feature parity with the legacy Wave-in-a-Box experience, modernized. |
| **3 — Federation** | s2s protocol (FR-47..52), key management, two-server interop suite in CI (FR-53), moderation basics. | Two independent servers co-edit one wave; partition test converges; NFR-5 met. |
| **4 — Translation** | §9 pipeline (FR-40..46), budgets, glossaries, overlay UI. | US-6 acceptance met across 3 language pairs within budget. |
| **5 — P2P folder sharing** | §11 (FR-54..60), iroh integration, mirroring. | US-14..17 acceptance met including origin-offline fetch. |
| **6 — Polish & extend** | PWA/offline hardening, i18n locales (NFR-20), rate limiting (FR-63), extension API (sandboxed iframes + postMessage — the gadget successor), admin tooling, HTML import (FR-17). | Public beta readiness. |
| **7 — Federated inference ("Hive Mind")** *(exploratory, implemented; §12.1)* | Agents as wave participants, RAG over accessible waves + shared files with provenance, signed federated inference (mixture-of-peers). | Shipped as exploratory: an agent answers as a blip grounded in wave context; a node can route inference to a peer's model. Constraints below. |

**Ordering rationale:** federation lands before translation and P2P because it constrains protocol design the most — "core goal" means it cannot accrete late. Translation and P2P are the differentiators but depend on a stable collaborative core. Phase 7 depends on federation (Ph3), the `Translator`/provider abstraction (Ph4), P2P content addressing (Ph5), and the extension API (Ph6) all being stable.

### 12.1 Phase 7 — Federated Inference & Agent Harness (exploratory, implemented)

**Status: exploratory — a first implementation shipped.** Requirements remain labeled FI-x (not FR-x): this is beyond the committed v1 surface and its guarantees are weaker. What's built:

- **Agent as participant (the harness).** `assistant@domain` reads a wave, assembles grounding context, and writes its answer as a real *blip* into the wavelet CRDT (`server/src/agent.rs`) — it persists, fans out to every subscriber, and federates like a human edit. Server-authored blips are constructed in yrs wire-compatibly with the web document model. Triggered by `POST /api/waves/ask` (the client's "✳ ask" bar); rate-limited (FI-2).
- **RAG with provenance (FI-5).** Context = recent blips + full-text retrieval (tantivy) scoped strictly to the *asker's* accessible waves + the wave's shared-file listing. Retrieval never crosses the asker's ACLs.
- **Federated inference (mixture-of-peers).** `POST /federation/v0/infer` (signed, participant-domain ACL) lets a wave's agent route to another node's model; each node advertises its model id in `.well-known/protowave` (FI-1). Provider is swappable behind `InferenceProvider` (FI-6).

**Honest constraints (do not overstate this phase):**
- Self-hosted local models are supported via **Ollama** (`OllamaInference`, selected by `PROTOWAVE_OLLAMA`): a node points at a local `ollama serve`, and its model auto-joins the Hive Mind. Gemini is the fallback when no local model is configured. GPU is optional — small models run on CPU (the live deployment runs `gemma3:270m`). Multi-model-per-node routing (capability manifests listing several models, a `model` field on InferRequest) is the remaining FI-1 step.
- **Answer verification (R11) remains unsolved.** Answers are advisory; the UI says "verify before relying on it." Redundant-sampling/attribution scaffolding is the only mitigation, and it is partial.
- No agent autonomy loop, tool use, or multi-step planning — an agent answers when asked, once.

Original design intent (still the north star) follows; candidate requirements are FI-x.

**Concept.** Community members host small LLMs (e.g., Gemma-class, Qwen-Coder-class) on their own machines and share inference capacity across the federation — a *mixture of peers*, not merged weights: a router directs each task to a suitable node (code → code model, general → largest available), optionally sampling two nodes for comparison. Federated folder shares double as a RAG corpus with **cryptographic provenance**: every retrieved passage traces to a BLAKE3 hash in a signed manifest, so answers cite exactly which shared document a claim came from.

**Reuse of existing primitives** (why this is Phase 7 and not a new product): node identity & discovery = ed25519 + `.well-known` (§8.2); capability advertisement = signed manifests (same mechanism as folder shares, §11); transport = iroh QUIC streaming (§11); provider abstraction = the `Translator`-style trait generalized to `InferenceProvider` (§9); retrieval = tantivy + an embeddings index over CAS content.

**ProtoWave as agent harness.** Waves are a natural orchestration substrate for these models — the modern successor of legacy Wave's **robots API** (`box/server/robots/` in `legacy/apache-wave`):

- **Agents are participants**: `agent@domain` joins a wave like any user, governed by the same ACLs, presence, and federation rules. No parallel permission system.
- **Blips are the I/O channel**: an `@mention` in a blip is the prompt; the agent's reply blip (or inline thread) is the response. Multi-step tool use is a thread — each step a blip.
- **CRDT documents are shared working memory**: an agent and humans co-edit the same document; the agent's edits merge like anyone else's (US-5 machinery, unchanged).
- **Playback is the audit log for free**: every agent action is in the update log, signed, replayable, attributable (G4) — a harness property most agent frameworks have to build; ProtoWave gets it from its core design.
- **Federation extends the harness across servers**: an agent hosted on server A can participate in a wave homed on server B under the standard content/control-plane rules (§8.3).

**Candidate requirements.**

- **FI-1** Capability manifests: model name, quantization, context length, throughput, availability window, and **license identifier** — signed by the hosting node.
- **FI-2** Serving is opt-in and scoped by the operator: own waves / own federation / public; operator-side acceptable-use policy, rate limits, and right to filter or log.
- **FI-3** License flow-down: the protocol surfaces each model's license and prohibited-use terms to consumers (Gemma-class terms are use-restricted and must propagate; Apache-2.0 models are unrestricted). Compliance responsibility sits with the hosting operator.
- **FI-4** Privacy disclosure: sending a prompt to a peer node requires the same explicit disclosure as third-party translation (NFR-16), naming the destination node/domain.
- **FI-5** RAG respects ACLs: retrieval never surfaces content from a share or wave the *asker* cannot access; enforced via the capability-manifest model (NFR-18).
- **FI-6** Tiered mind: routing may combine local/federated models (private, free, always-available) with API models (quality) per wave policy; local-model translation becomes an option that keeps content inside the federation (partially resolving R9).
- **FI-7** Agent identity: agents authenticate as `agent@domain` participants with their own keys; agent blips are visually attributed as automated.

**Open research question — inference verification (R11).** A file chunk verifies in O(chunk) against its hash; an inference result has no equivalent. A malicious or degraded node can return plausible garbage. Candidate mitigations: signed responses building node reputation; occasional redundant sampling across nodes; attestation of model identity. No known cheap, general solution — this gates the phase.

---

## 13. Success Metrics

| Metric | Target (12 months post-beta) |
|--------|------------------------------|
| Time-to-first-collaborative-edit (signup → first co-edit) | < 5 minutes median |
| DAU/WAU per active server | > 0.4 |
| Federated-wave ratio (waves with ≥ 2 domains) | > 10% of active waves |
| Translation adoption (translation-enabled waves among multilingual-participant waves) | > 50% |
| p95 latencies | Within §6.2 budgets, continuously measured |
| Self-host installs (unique reporting servers, opt-in telemetry) | 500+ |
| Playback usage (waves replayed / waves opened by late joiners) | > 15% |

---

## 14. Risks & Open Questions

| ID | Risk / question | Mitigation / plan |
|----|-----------------|-------------------|
| R1 | **CRDT history growth** — logs retained forever for playback; yrs tombstone memory on huge, old waves. | Snapshots bound open/seek cost; lazy log loading; wavelet granularity caps doc size; measure tombstone overhead in Phase 1 benchmarks. |
| R2 | **ACL enforcement vs. CRDT convergence** — removed participants' concurrent edits (§8.3). | ACL-versioned batches, fast control propagation; document the residual window honestly. |
| R3 | **Translation cost blow-up** — the live-revision loop multiplies tokens. | Budgets, viewport-driven subscription, sentence caching, batching are *requirements* (FR-43..44), not optimizations; load-test cost per active multilingual wave in Phase 4. |
| R4 | **Federation abuse** — spam waves, malicious servers. | Blocklists, rate limits, signed audit trail; study Matrix/Mastodon moderation history before Phase 3. |
| R5 | **Editor schema fidelity** — mapping Wave's annotation model onto ProseMirror marks/decorations (inline replies, diff highlighting). | Dedicated spike at Phase 1 start; legacy `model/conversation/` + `model/document/` are the reference. |
| R6 | **iroh maturity / API churn.** | Pin versions; isolate behind the blob-service trait; rust-libp2p is the documented fallback (ADR-4). |
| R7 | **Control-plane portability** — what happens to membership authority if a wavelet's home server dies permanently? | Open question; candidate: signed authority-transfer records. Decide before Phase 3 freeze. |
| R8 | **Wavelet-per-Y.Doc granularity** at 10k+ blips — memory and load cost of one giant doc. | Open question; candidate: yrs subdocuments or sharded wavelets. Benchmark in Phase 2. |
| R9 | **E2EE vs. translation tension** — server-side translation requires plaintext. | Documented non-goal for v1; future direction: per-wave choice between E2EE and translation (mutually exclusive). |
| R10 | **Gemini API dependency** — pricing/model changes. | `Translator` trait keeps the provider swappable (FR-46); cache reduces exposure. |
| R11 | **Inference verification (Phase 7)** — peer inference results cannot be cheaply verified the way content chunks can; malicious/degraded nodes can return plausible garbage. | Reputation via signed responses, redundant sampling, model attestation — all partial. Open research question; gates Phase 7 (§12.1). |
| R12 | **GPU free-riding & abuse (Phase 7)** — few operators subsidize many users; strangers' prompts run on private hardware. | Serving opt-in and scoped (FI-2); per-node quotas; operator AUP and filtering rights; revisit economics if usage grows. |

---

## 15. Glossary & Appendix

### 15.1 Glossary

**Wave** — container for a conversation-document. **Wavelet** — unit of replication/ACL/federation within a wave. **Blip** — one message node in a wavelet's thread tree. **CRDT** — conflict-free replicated data type; concurrent updates merge deterministically without coordination. **yrs** — Rust implementation of the Yjs CRDT. **State vector** — compact summary of which updates a replica has; exchanging them yields the missing diff. **Awareness** — ephemeral presence/cursor protocol in the Yjs ecosystem. **CAS** — content-addressed store (BLAKE3 hashes as keys). **FastCDC** — content-defined chunking algorithm. **bao** — BLAKE3-based verified streaming. **Home server** — the origin-domain server holding control-plane authority for a wavelet. **Overlay** — derived, non-authoritative rendering layered on a document (e.g., translations).

### 15.2 Legacy codebase references (branch `legacy/apache-wave`)

| Legacy artifact | Relevance |
|---|---|
| `wave/src/main/java/org/waveprotocol/wave/model/conversation/` | Thread-tree/manifest model → §4.3 conversation manifest |
| `wave/src/main/java/org/waveprotocol/wave/model/document/` | Annotated-document model → yrs XML mapping (ADR-1) |
| `wave/src/proto/proto/org/waveprotocol/box/common/comms/waveclient-rpc.proto` | Open/Submit/Authenticate semantics → §8.1 envelope |
| `wave/src/proto/proto/org/waveprotocol/wave/federation/federation.protodevel` | Signed deltas + hashed versions → §8.3 signed batches |
| `wave/src/main/java/org/waveprotocol/box/server/waveserver/` | Server coordinator & pluggable persistence → `WaveStore` trait |
| `wave/src/main/java/org/waveprotocol/wave/client/editor/` | Everything Tiptap+Yjs must replace (ADR-3) |

### 15.3 External references

- Yjs / yrs: <https://docs.yjs.dev/> · <https://github.com/y-crdt/y-crdt>
- ProseMirror / Tiptap: <https://prosemirror.net/> · <https://tiptap.dev/>
- Matrix federation (comparative reference for §8.3): <https://spec.matrix.org/latest/server-server-api/>
- iroh / BLAKE3 / bao: <https://iroh.computer/> · <https://github.com/BLAKE3-team/BLAKE3>
- FastCDC: Xia et al., USENIX ATC '16
- tantivy: <https://github.com/quickwit-oss/tantivy>
- Google Wave Federation Protocol (historical): waveprotocol.org drafts, preserved in `legacy/apache-wave` proto comments
