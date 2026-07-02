// ProtoWave wire protocol (Phase 0): parses the canonical .proto schema at
// runtime with protobufjs, so the schema in crates/protowave-proto stays the
// single source of truth. Replaced by generated static code in Phase 1.

import protobuf from 'protobufjs'
import envelopeProto from '../../../crates/protowave-proto/proto/protowave/v1/envelope.proto?raw'

const root = protobuf.parse(envelopeProto).root
const EnvelopeType = root.lookupType('protowave.v1.Envelope')
const AuthRequestType = root.lookupType('protowave.v1.AuthRequest')
const AuthResponseType = root.lookupType('protowave.v1.AuthResponse')

export const Channel = {
  CONTROL: 1,
  SYNC: 2,
  AWARENESS: 3,
  TRANSLATION: 4,
  NOTIFICATION: 5,
  ECHO: 6,
} as const

export interface Envelope {
  channel: number
  payload: Uint8Array
}

export interface AuthResponse {
  ok: boolean
  sessionId: string
  error: string
}

export function encodeEnvelope(channel: number, payload: Uint8Array): Uint8Array {
  return EnvelopeType.encode(EnvelopeType.create({ channel, payload })).finish()
}

export function decodeEnvelope(bytes: Uint8Array): Envelope {
  const msg = EnvelopeType.decode(bytes) as unknown as { channel: number; payload: Uint8Array }
  return { channel: msg.channel, payload: msg.payload ?? new Uint8Array() }
}

export function encodeAuthRequest(participant: string, token: string): Uint8Array {
  const payload = AuthRequestType.encode(AuthRequestType.create({ participant, token })).finish()
  return encodeEnvelope(Channel.CONTROL, payload)
}

export function decodeAuthResponse(payload: Uint8Array): AuthResponse {
  const msg = AuthResponseType.toObject(AuthResponseType.decode(payload), {
    defaults: true,
  }) as { ok: boolean; sessionId: string; error: string }
  return msg
}
