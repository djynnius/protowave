// Yjs connection provider over the ProtoWave envelope protocol.
//
// One WaveSocket per app session multiplexes any number of wavelet
// subscriptions. WaveletProvider owns a Y.Doc + y-protocols Awareness and
// performs the subscribe/sync-state exchange: on (re)connect it sends its
// state vector, applies the server diff, and pushes back whatever the server
// was missing — offline edits merge with no special casing (NFR-21).

import { ref, type Ref } from 'vue'
import * as Y from 'yjs'
import {
  Awareness,
  applyAwarenessUpdate,
  encodeAwarenessUpdate,
  removeAwarenessStates,
} from 'y-protocols/awareness'
import {
  Channel,
  decodeAwareness,
  decodeControl,
  decodeEnvelope,
  decodeSync,
  encodeAwareness,
  encodeSubscribe,
  encodeUnsubscribe,
  encodeUpdate,
} from './protocol'

export type SocketStatus = 'offline' | 'connecting' | 'online'

export class WaveSocket {
  readonly status: Ref<SocketStatus> = ref('offline')
  private ws: WebSocket | null = null
  private providers = new Map<string, WaveletProvider>()
  private retries = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private closed = false

  constructor(private url = `ws://${location.host}/ws`) {}

  connect() {
    if (this.ws || this.closed) return
    this.status.value = 'connecting'
    const ws = new WebSocket(this.url)
    ws.binaryType = 'arraybuffer'
    this.ws = ws

    ws.onopen = () => {
      this.retries = 0
      this.status.value = 'online'
      for (const provider of this.providers.values()) provider.resubscribe()
    }
    ws.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      this.route(new Uint8Array(event.data))
    }
    ws.onclose = () => {
      this.ws = null
      this.status.value = 'offline'
      for (const provider of this.providers.values()) provider.markUnsynced()
      this.scheduleReconnect()
    }
    ws.onerror = () => ws.close()
  }

  close() {
    this.closed = true
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    this.ws?.close()
  }

  private scheduleReconnect() {
    if (this.closed || this.reconnectTimer) return
    const delay = Math.min(500 * 2 ** this.retries++, 10_000)
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this.connect()
    }, delay)
  }

  send(frame: Uint8Array): boolean {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(frame)
      return true
    }
    return false
  }

  register(provider: WaveletProvider) {
    this.providers.set(provider.wavelet, provider)
    if (this.status.value === 'online') provider.resubscribe()
    else this.connect()
  }

  unregister(wavelet: string) {
    this.providers.delete(wavelet)
    this.send(encodeUnsubscribe(wavelet))
  }

  private route(bytes: Uint8Array) {
    const env = decodeEnvelope(bytes)
    switch (env.channel) {
      case Channel.CONTROL: {
        const msg = decodeControl(env.payload)
        if (msg.error) {
          this.providers.get(msg.error.wavelet)?.onError(msg.error.code, msg.error.message)
        }
        break
      }
      case Channel.SYNC: {
        const msg = decodeSync(env.payload)
        const provider = this.providers.get(msg.wavelet)
        if (!provider) break
        if (msg.syncState) provider.onSyncState(msg.syncState.stateVector, msg.syncState.diff)
        if (msg.update) provider.onRemoteUpdate(msg.update.update)
        break
      }
      case Channel.AWARENESS: {
        const msg = decodeAwareness(env.payload)
        this.providers.get(msg.wavelet)?.onRemoteAwareness(msg.payload)
        break
      }
    }
  }
}

export class WaveletProvider {
  readonly doc = new Y.Doc()
  readonly awareness = new Awareness(this.doc)
  readonly synced = ref(false)
  readonly error: Ref<string | null> = ref(null)

  constructor(
    private socket: WaveSocket,
    readonly wavelet: string,
  ) {
    this.doc.on('update', this.onLocalUpdate)
    this.awareness.on('update', this.onAwarenessChange)
    socket.register(this)
  }

  resubscribe() {
    this.socket.send(encodeSubscribe(this.wavelet, Y.encodeStateVector(this.doc)))
  }

  markUnsynced() {
    this.synced.value = false
  }

  onSyncState(serverStateVector: Uint8Array, diff: Uint8Array) {
    if (diff.length > 0) {
      Y.applyUpdate(this.doc, diff, this)
    }
    // Push back what the server is missing (our offline edits).
    const missing = Y.encodeStateAsUpdate(this.doc, serverStateVector)
    if (missing.length > 2) {
      this.socket.send(encodeUpdate(this.wavelet, missing))
    }
    // Re-announce our presence to the new subscription.
    const local = encodeAwarenessUpdate(this.awareness, [this.doc.clientID])
    this.socket.send(encodeAwareness(this.wavelet, local))
    this.synced.value = true
    this.error.value = null
  }

  onRemoteUpdate(update: Uint8Array) {
    Y.applyUpdate(this.doc, update, this)
  }

  onRemoteAwareness(payload: Uint8Array) {
    applyAwarenessUpdate(this.awareness, payload, 'remote')
  }

  onError(code: string, message: string) {
    this.error.value = `${code}: ${message}`
  }

  private onLocalUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin !== this) {
      this.socket.send(encodeUpdate(this.wavelet, update))
    }
  }

  private onAwarenessChange = (
    changes: { added: number[]; updated: number[]; removed: number[] },
    origin: unknown,
  ) => {
    if (origin === 'remote') return
    const changed = changes.added.concat(changes.updated, changes.removed)
    this.socket.send(encodeAwareness(this.wavelet, encodeAwarenessUpdate(this.awareness, changed)))
  }

  destroy() {
    removeAwarenessStates(this.awareness, [this.doc.clientID], 'destroy')
    this.doc.off('update', this.onLocalUpdate)
    this.socket.unregister(this.wavelet)
    this.awareness.destroy()
    this.doc.destroy()
  }
}

// Shared app-wide socket.
export const socket = new WaveSocket()
