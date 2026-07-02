// ProtoWave wire protocol: parses the canonical .proto schema at runtime
// with protobufjs, so the schema in crates/protowave-proto stays the single
// source of truth. Replaced by generated static code when the protocol
// stabilizes.

import protobuf from 'protobufjs'
import envelopeProto from '../../../crates/protowave-proto/proto/protowave/v1/envelope.proto?raw'

const root = protobuf.parse(envelopeProto).root
const EnvelopeType = root.lookupType('protowave.v1.Envelope')
const ControlType = root.lookupType('protowave.v1.ControlMessage')
const SyncType = root.lookupType('protowave.v1.SyncMessage')
const AwarenessType = root.lookupType('protowave.v1.AwarenessMessage')
const TranslationType = root.lookupType('protowave.v1.TranslationMessage')

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

export interface ControlIn {
  subscribed?: { wavelet: string }
  error?: { wavelet: string; code: string; message: string }
}

export interface SyncIn {
  wavelet: string
  syncState?: { stateVector: Uint8Array; diff: Uint8Array }
  update?: { update: Uint8Array }
}

export interface AwarenessIn {
  wavelet: string
  payload: Uint8Array
}

export interface TranslationIn {
  wavelet: string
  targetLang: string
  entries: { blip: string; text: string; pending?: boolean }[]
}

function envelope(channel: number, payload: Uint8Array): Uint8Array {
  return EnvelopeType.encode(EnvelopeType.create({ channel, payload })).finish()
}

export function decodeEnvelope(bytes: Uint8Array): Envelope {
  const msg = EnvelopeType.decode(bytes) as unknown as { channel: number; payload: Uint8Array }
  return { channel: msg.channel, payload: msg.payload ?? new Uint8Array() }
}

export function encodeSubscribe(wavelet: string, stateVector: Uint8Array): Uint8Array {
  const payload = ControlType.encode(
    ControlType.create({ subscribe: { wavelet, stateVector } }),
  ).finish()
  return envelope(Channel.CONTROL, payload)
}

export function encodeUnsubscribe(wavelet: string): Uint8Array {
  const payload = ControlType.encode(ControlType.create({ unsubscribe: { wavelet } })).finish()
  return envelope(Channel.CONTROL, payload)
}

export function encodeUpdate(wavelet: string, update: Uint8Array): Uint8Array {
  const payload = SyncType.encode(SyncType.create({ wavelet, update: { update } })).finish()
  return envelope(Channel.SYNC, payload)
}

export function encodeAwareness(wavelet: string, payload: Uint8Array): Uint8Array {
  const body = AwarenessType.encode(AwarenessType.create({ wavelet, payload })).finish()
  return envelope(Channel.AWARENESS, body)
}

export function decodeControl(payload: Uint8Array): ControlIn {
  return ControlType.toObject(ControlType.decode(payload)) as ControlIn
}

export function decodeSync(payload: Uint8Array): SyncIn {
  return SyncType.toObject(SyncType.decode(payload)) as SyncIn
}

export function decodeAwareness(payload: Uint8Array): AwarenessIn {
  return AwarenessType.toObject(AwarenessType.decode(payload)) as AwarenessIn
}

export function encodeTranslateSubscribe(wavelet: string, targetLang: string): Uint8Array {
  const payload = ControlType.encode(
    ControlType.create({ translateSubscribe: { wavelet, targetLang } }),
  ).finish()
  return envelope(Channel.CONTROL, payload)
}

export function encodeTranslateUnsubscribe(wavelet: string): Uint8Array {
  const payload = ControlType.encode(
    ControlType.create({ translateUnsubscribe: { wavelet } }),
  ).finish()
  return envelope(Channel.CONTROL, payload)
}

export function decodeTranslation(payload: Uint8Array): TranslationIn {
  return TranslationType.toObject(TranslationType.decode(payload), {
    defaults: true,
  }) as TranslationIn
}
