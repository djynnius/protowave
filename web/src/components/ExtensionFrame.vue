<script setup lang="ts">
// One wave app in a sandboxed iframe — the successor of Wave's gadgets.
// The frame gets no same-origin powers (sandbox="allow-scripts" only); it
// talks over postMessage: 'pw-ready' → we send 'pw-init'; 'pw-set' writes
// a key into the extension's collaborative Y.Map; every remote change
// streams back as 'pw-state'. Everyone on the wave sees the same state.
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type * as Y from 'yjs'
import { localPart } from '../lib/wavemodel'

const props = defineProps<{
  id: string
  url: string
  state: Y.Map<unknown>
  me: string
}>()

const emit = defineEmits<{ remove: [id: string] }>()
const frame = ref<HTMLIFrameElement | null>(null)

function stateJson(): Record<string, unknown> {
  return props.state.toJSON() as Record<string, unknown>
}

function post(msg: Record<string, unknown>) {
  // Sandboxed frames have an opaque origin; '*' is required and safe here
  // because the payload is only this extension's own shared state.
  frame.value?.contentWindow?.postMessage(msg, '*')
}

function onStateChange() {
  post({ type: 'pw-state', state: stateJson() })
}

function onMessage(event: MessageEvent) {
  if (event.source !== frame.value?.contentWindow) return
  const msg = event.data
  if (!msg || typeof msg !== 'object') return
  if (msg.type === 'pw-ready') {
    post({ type: 'pw-init', state: stateJson(), me: localPart(props.me) })
  } else if (msg.type === 'pw-set' && typeof msg.key === 'string') {
    props.state.set(msg.key, msg.value)
  }
}

onMounted(() => {
  window.addEventListener('message', onMessage)
  props.state.observe(onStateChange)
})

onBeforeUnmount(() => {
  window.removeEventListener('message', onMessage)
  props.state.unobserve(onStateChange)
})
</script>

<template>
  <div class="extension">
    <header>
      <span class="caption">⊞ {{ url.split('/').pop() }}</span>
      <button class="remove" :title="'remove'" @click="emit('remove', id)">×</button>
    </header>
    <iframe ref="frame" :src="url" sandbox="allow-scripts" :title="`wave app ${id}`" />
  </div>
</template>

<style scoped>
.extension {
  border: 1px solid var(--mist);
  border-radius: 12px;
  background: #fff;
  overflow: hidden;
}

header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.35rem 0.7rem;
  background: var(--sky-t);
}

.remove {
  background: none;
  border: none;
  color: var(--steel);
  font-size: 1rem;
  cursor: pointer;
  line-height: 1;
}

iframe {
  display: block;
  width: 100%;
  height: 14rem;
  border: none;
}
</style>
