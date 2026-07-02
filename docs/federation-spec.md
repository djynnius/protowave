# ProtoWave Federation Protocol — v0 (implemented)

**Status:** v0 implemented (Phase 3). PRD §8.3 is the product-level summary. Message schemas live in `crates/protowave-proto/proto/protowave/v1/federation.proto`; the reference implementation is `server/src/federation.rs` with the interop suite in `server/tests/federation.rs`.

**v0 transport note:** messages travel as signed protobuf bodies over HTTP POST on each server's public URL (`/federation/v0/push|sync|announce`), not gRPC — the tonic stack does not build on the project's pinned rustc 1.75 toolchain, and security is message-level (ed25519 over `"protowave-fed-v0\n" ‖ path ‖ "\n" ‖ body`, keys pinned TOFU via `/.well-known/protowave`), so the transport carries no trust. The schema is transport-agnostic; migrating to gRPC/TLS is a contained change. Peer resolution v0: static map (`PROTOWAVE_PEERS`), with `.well-known` used for key discovery.

## 1. Scope

Server-to-server exchange of wavelets between independent ProtoWave deployments. Design goals, in priority order: partition tolerance for content, unambiguous access-control authority, verifiable provenance for every byte exchanged, and a versioned protocol that alternate implementations can target.

## 2. Identity

- **Servers** are identified by DNS domain. Each server holds one or more ed25519 keypairs; public keys, the gRPC federation endpoint, and supported protocol versions are published at `https://<domain>/.well-known/protowave` (JSON; schema TBD). Keys are pinned by peers on first contact; rotation is announced via overlapping validity windows.
- **Participants** are `local@domain` (see `protowave-core::ParticipantId`).
- **Waves** are `domain/wave-id`; **wavelets** are `(wave, wavelet)` pairs where the wavelet's own domain is its **home server** (see `protowave-core::WaveletName`).

## 3. Planes

### 3.1 Content plane (fully replicated)

Every server with ≥ 1 participant on a wavelet maintains a full replica of its yrs document and update log.

- **Eager push:** a server that accepts a local edit broadcasts a signed **update batch** to all peer servers on the wavelet.
- **Anti-entropy:** peers periodically exchange yrs **state vectors**; a receiver replies with the updates the sender is missing. This heals partitions and missed pushes.
- Updates are commutative (CRDT); no ordering negotiation is required or performed.

### 3.2 Control plane (home-server authoritative)

The wavelet's home server is the sole authority for membership, ACLs, and policy. It publishes a signed, versioned **wavelet state document**; every content batch carries the ACL version it was authored under. Receivers reject batches that violate the ACL version they have seen (removed-participant rule, PRD §8.3). Control-plane changes queue while the home server is unreachable; content editing continues.

## 4. Wire format (sketch)

Transport: gRPC over TLS (tonic). Messages (protobuf, `protowave.federation.v1` — to be added to `crates/protowave-proto`):

- `UpdateBatch { wavelet_name, yrs_update bytes, acl_version, origin_server, sequence, signature }`
- `StateVectorRequest / StateVectorResponse`
- `WaveletStateDocument { participants, acl, version, home_server_signature }`
- `ServerKeyAnnouncement`

Signatures are ed25519 over a canonical serialization (field order fixed by protobuf schema; exact signing input TBD — likely `SHA-512(domain-separator ‖ serialized-message)`).

## 5. Open questions (tracked in PRD §14)

- Control-plane authority transfer when a home server dies permanently (R7).
- Bounded semantics of the removed-participant merge window (R2).
- Abuse controls beyond blocklists: rate limits, proof-of-work greylisting? (R4)

## 6. Versioning

`protocol_version` integer negotiated at connection establishment; servers advertise supported ranges in `.well-known`. Breaking changes increment the major version; this draft is version 0 (pre-release, no compatibility promises).
