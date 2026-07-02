// Phase 0 client connection: WebSocket + auth handshake + echo channel.

import { ref, type Ref } from 'vue'
import {
  Channel,
  decodeAuthResponse,
  decodeEnvelope,
  encodeAuthRequest,
  encodeEnvelope,
} from './protocol'

export type ConnectionStatus = 'disconnected' | 'connecting' | 'authenticated' | 'error'

export interface Connection {
  status: Ref<ConnectionStatus>
  sessionId: Ref<string>
  lastError: Ref<string>
  echoes: Ref<string[]>
  connect: (participant: string, token: string) => void
  sendEcho: (text: string) => void
  disconnect: () => void
}

export function useConnection(url = `ws://${location.host}/ws`): Connection {
  const status = ref<ConnectionStatus>('disconnected')
  const sessionId = ref('')
  const lastError = ref('')
  const echoes = ref<string[]>([])
  let socket: WebSocket | null = null

  function connect(participant: string, token: string) {
    disconnect()
    status.value = 'connecting'
    lastError.value = ''
    socket = new WebSocket(url)
    socket.binaryType = 'arraybuffer'

    socket.onopen = () => {
      socket?.send(encodeAuthRequest(participant, token))
    }
    socket.onmessage = (event: MessageEvent<ArrayBuffer>) => {
      const envelope = decodeEnvelope(new Uint8Array(event.data))
      if (envelope.channel === Channel.CONTROL) {
        const auth = decodeAuthResponse(envelope.payload)
        if (auth.ok) {
          status.value = 'authenticated'
          sessionId.value = auth.sessionId
        } else {
          status.value = 'error'
          lastError.value = auth.error
          socket?.close()
        }
      } else if (envelope.channel === Channel.ECHO) {
        echoes.value.push(new TextDecoder().decode(envelope.payload))
      }
    }
    socket.onerror = () => {
      status.value = 'error'
      lastError.value = 'websocket error'
    }
    socket.onclose = () => {
      if (status.value !== 'error') status.value = 'disconnected'
      socket = null
    }
  }

  function sendEcho(text: string) {
    if (socket?.readyState === WebSocket.OPEN) {
      socket.send(encodeEnvelope(Channel.ECHO, new TextEncoder().encode(text)))
    }
  }

  function disconnect() {
    socket?.close()
    socket = null
    status.value = 'disconnected'
    sessionId.value = ''
  }

  return { status, sessionId, lastError, echoes, connect, sendEcho, disconnect }
}
