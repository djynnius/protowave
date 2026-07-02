// Wavelet document structure (PRD §4.3):
//   blips:    Y.Map<blipId, Y.XmlFragment>  — rich text content
//   manifest: Y.Array<BlipEntry>            — thread structure (flat, parent
//                                             pointers; rendered as a tree)

import * as Y from 'yjs'

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

/// Deterministic collaborator color from a participant address.
const PALETTE = ['#E2603F', '#0E7C6B', '#C9A227', '#7D4A8E', '#3F6C8C', '#A34D68']

export function participantColor(participant: string): string {
  let hash = 0
  for (const ch of participant) hash = (hash * 31 + ch.charCodeAt(0)) | 0
  return PALETTE[Math.abs(hash) % PALETTE.length]
}

export function localPart(participant: string): string {
  return participant.split('@')[0] ?? participant
}
