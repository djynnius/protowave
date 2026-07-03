// Wavelet document structure (PRD §4.3):
//   blips:    Y.Map<blipId, Y.XmlFragment>  — rich text content
//   manifest: Y.Array<BlipEntry>            — thread structure (flat, parent
//                                             pointers; rendered as a tree)

import * as Y from 'yjs'
import { prosemirrorToYXmlFragment } from 'y-prosemirror'
import type { Node as PMNode } from '@tiptap/pm/model'

export const ROOT_BLIP = 'b+root'

export interface BlipEntry {
  id: string
  author: string
  ts: number
  parent: string | null
}

export function blips(doc: Y.Doc): Y.Map<Y.XmlFragment> {
  return doc.getMap<Y.XmlFragment>('blips')
}

export function manifest(doc: Y.Doc): Y.Array<BlipEntry> {
  return doc.getArray<BlipEntry>('manifest')
}

/// Create the root blip if the wavelet is brand new.
export function ensureRootBlip(doc: Y.Doc, author: string) {
  if (manifest(doc).length > 0) return
  doc.transact(() => {
    blips(doc).set(ROOT_BLIP, new Y.XmlFragment())
    manifest(doc).push([{ id: ROOT_BLIP, author, ts: Date.now(), parent: null }])
  })
}

export function addReply(doc: Y.Doc, author: string, parent: string): string {
  const id = `b+${Math.random().toString(36).slice(2, 10)}`
  doc.transact(() => {
    blips(doc).set(id, new Y.XmlFragment())
    manifest(doc).push([{ id, author, ts: Date.now(), parent }])
  })
  return id
}

/// Post a new blip from composed ProseMirror content (the bottom composer's
/// "send"). The PM doc is converted to a Y.XmlFragment and dropped into the
/// blips map, so it syncs, plays back, translates and federates like any
/// blip. `parent` is the replied-to blip, or the root for a top-level post.
export function postBlip(
  doc: Y.Doc,
  author: string,
  parent: string | null,
  content: PMNode,
): string {
  const id = `b+${Math.random().toString(36).slice(2, 10)}`
  const fragment = prosemirrorToYXmlFragment(content) as Y.XmlFragment
  doc.transact(() => {
    blips(doc).set(id, fragment)
    manifest(doc).push([{ id, author, ts: Date.now(), parent }])
  })
  return id
}

export interface BlipNode {
  entry: BlipEntry
  depth: number
}

/// Flatten the manifest into depth-first render order, deduped by id
/// (concurrent creation can double-insert; first entry wins).
export function threadOrder(entries: BlipEntry[]): BlipNode[] {
  const seen = new Set<string>()
  const unique = entries.filter((e) => {
    if (seen.has(e.id)) return false
    seen.add(e.id)
    return true
  })
  const children = new Map<string | null, BlipEntry[]>()
  for (const entry of unique) {
    const list = children.get(entry.parent) ?? []
    list.push(entry)
    children.set(entry.parent, list)
  }
  for (const list of children.values()) {
    list.sort((a, b) => a.ts - b.ts)
  }
  const out: BlipNode[] = []
  const walk = (parent: string | null, depth: number) => {
    for (const entry of children.get(parent) ?? []) {
      out.push({ entry, depth })
      walk(entry.id, depth + 1)
    }
  }
  walk(null, 0)
  return out
}

// ---- extensions: the gadget successor (Phase 6) ----
// Each extension instance = { url, state: Y.Map } in the `extensions` map.
// The state map is collaborative: every participant's frame sees the same
// live object, exactly like Wave's gadget state.

export interface ExtensionEntry {
  id: string
  url: string
}

export function extensions(doc: Y.Doc): Y.Map<Y.Map<unknown>> {
  return doc.getMap<Y.Map<unknown>>('extensions')
}

export function addExtension(doc: Y.Doc, url: string): string {
  const id = `x+${Math.random().toString(36).slice(2, 10)}`
  doc.transact(() => {
    const inner = new Y.Map<unknown>()
    inner.set('url', url)
    inner.set('state', new Y.Map())
    extensions(doc).set(id, inner)
  })
  return id
}

export function removeExtension(doc: Y.Doc, id: string) {
  extensions(doc).delete(id)
}

export function listExtensions(doc: Y.Doc): ExtensionEntry[] {
  const out: ExtensionEntry[] = []
  extensions(doc).forEach((inner, id) => {
    const url = inner.get('url')
    if (typeof url === 'string') out.push({ id, url })
  })
  return out
}

export function extensionState(doc: Y.Doc, id: string): Y.Map<unknown> | undefined {
  return extensions(doc).get(id)?.get('state') as Y.Map<unknown> | undefined
}

/// Deterministic collaborator color from a participant address —
/// brand blues (crest → dusk) plus deep accents so avatars sit in harmony.
const PALETTE = ['#2E6BF0', '#2DD4BF', '#818CF8', '#38BDF8', '#0F9D8A', '#5B63D6']

export function participantColor(participant: string): string {
  let hash = 0
  for (const ch of participant) hash = (hash * 31 + ch.charCodeAt(0)) | 0
  return PALETTE[Math.abs(hash) % PALETTE.length]
}

export function localPart(participant: string): string {
  return participant.split('@')[0] ?? participant
}

/// The wave agent (Hive Mind harness) authors blips as `assistant@domain`.
export function isAgent(participant: string): boolean {
  return localPart(participant) === 'assistant'
}

// ---- deletion (tombstones, so threaded replies survive) ----

function deletedSet(doc: Y.Doc): Y.Map<boolean> {
  return doc.getMap<boolean>('deleted')
}

export function isDeleted(doc: Y.Doc, id: string): boolean {
  return deletedSet(doc).get(id) === true
}

/// Delete a blip: empty its content and tombstone it. The manifest entry
/// stays so any replies remain threaded under a "(deleted)" placeholder.
export function deleteBlip(doc: Y.Doc, id: string) {
  doc.transact(() => {
    const frag = blips(doc).get(id)
    if (frag && frag.length > 0) frag.delete(0, frag.length)
    deletedSet(doc).set(id, true)
    reactions(doc).delete(id)
  })
}

// ---- reactions (in the CRDT, so they sync/federate/replay) ----
// reactions: Y.Map<blipId, Y.Map<emoji, Y.Map<user, true>>>

export function reactions(doc: Y.Doc): Y.Map<Y.Map<Y.Map<boolean>>> {
  return doc.getMap('reactions')
}

export interface ReactionSummary {
  emoji: string
  users: string[]
}

export function readReactions(doc: Y.Doc, blipId: string): ReactionSummary[] {
  const forBlip = reactions(doc).get(blipId)
  if (!forBlip) return []
  const out: ReactionSummary[] = []
  forBlip.forEach((byUser, emoji) => {
    const users = [...byUser.keys()]
    if (users.length) out.push({ emoji, users })
  })
  return out
}

export function toggleReaction(doc: Y.Doc, blipId: string, emoji: string, user: string) {
  doc.transact(() => {
    const all = reactions(doc)
    let forBlip = all.get(blipId)
    if (!forBlip) {
      forBlip = new Y.Map()
      all.set(blipId, forBlip)
    }
    let byUser = forBlip.get(emoji)
    if (!byUser) {
      byUser = new Y.Map()
      forBlip.set(emoji, byUser)
    }
    if (byUser.get(user)) byUser.delete(user)
    else byUser.set(user, true)
  })
}
