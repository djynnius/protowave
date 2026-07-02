<script setup lang="ts">
// Playback (FR-25..26): fetch the wavelet's full update log, scrub through
// history with a slider; the document is rebuilt at each version and
// rendered read-only.
import { computed, ref, watch } from 'vue'
import * as Y from 'yjs'
import { sanitizeFragmentXml } from '../lib/markdown'
import { threadOrder, type BlipEntry } from '../lib/wavemodel'
import WaveMesh from './WaveMesh.vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const props = defineProps<{ wavelet: string }>()
const emit = defineEmits<{ close: [] }>()

const updates = ref<Uint8Array[]>([])
const position = ref(0)
const loading = ref(true)
const error = ref('')

interface RenderedBlip {
  id: string
  author: string
  depth: number
  html: string
}

const rendered = ref<RenderedBlip[]>([])

async function load() {
  loading.value = true
  try {
    const res = await fetch(`/api/history?wavelet=${encodeURIComponent(props.wavelet)}`, {
      credentials: 'same-origin',
    })
    if (!res.ok) throw new Error(`history: ${res.status}`)
    const bytes = new Uint8Array(await res.arrayBuffer())
    const parsed: Uint8Array[] = []
    let offset = 0
    const view = new DataView(bytes.buffer)
    while (offset + 4 <= bytes.length) {
      const len = view.getUint32(offset, true)
      parsed.push(bytes.subarray(offset + 4, offset + 4 + len))
      offset += 4 + len
    }
    updates.value = parsed
    position.value = parsed.length
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'failed to load history'
  } finally {
    loading.value = false
  }
}

/// Rebuild the doc at version k. O(k) replay from the start — fine for
/// Phase 2 scale; snapshot-anchored seeks (NFR-C4) come with bigger logs.
function materialize(k: number) {
  const doc = new Y.Doc()
  for (let i = 0; i < k; i++) {
    Y.applyUpdate(doc, updates.value[i])
  }
  const entries = doc.getArray<BlipEntry>('manifest').toArray()
  const blips = doc.getMap<Y.XmlFragment>('blips')
  rendered.value = threadOrder(entries).map(({ entry, depth }) => ({
    id: entry.id,
    author: entry.author,
    depth,
    html: sanitizeFragmentXml(blips.get(entry.id)?.toString() ?? ''),
  }))
  doc.destroy()
}

watch(position, (k) => materialize(k))
watch(updates, () => materialize(position.value))

const label = computed(() =>
  loading.value
    ? t('loadingHistory')
    : t('updatesLabel', { k: position.value, n: updates.value.length }),
)

load()
</script>

<template>
  <aside class="playback">
    <header>
      <h3>{{ t('replayTitle') }}</h3>
      <button class="btn" @click="emit('close')">{{ t('close') }}</button>
    </header>
    <WaveMesh :active="loading" :width="400" />
    <p v-if="error" class="error-note">{{ error }}</p>

    <div class="scrubber">
      <input
        v-model.number="position"
        type="range"
        min="0"
        :max="updates.length"
        :disabled="loading"
      />
      <span class="mono position">{{ label }}</span>
    </div>

    <div class="frames">
      <article
        v-for="blip in rendered"
        :key="blip.id"
        class="frame"
        :style="{ marginLeft: `${blip.depth * 1.2}rem` }"
      >
        <span class="mono author">{{ blip.author.split('@')[0] }}</span>
        <!-- eslint-disable-next-line vue/no-v-html — sanitized fragment XML -->
        <div class="frame-body" v-html="blip.html" />
      </article>
      <p v-if="!loading && rendered.length === 0" class="mono empty">
        {{ t('emptyAtThisPoint') }}
      </p>
    </div>
  </aside>
</template>

<style scoped>
.playback {
  border: 1px solid var(--paper-edge);
  border-left: 3px solid var(--gold);
  border-radius: 8px;
  background: color-mix(in srgb, var(--paper) 70%, #fff);
  box-shadow: var(--shadow-card);
  padding: 1rem 1.2rem;
  margin-bottom: 1.4rem;
}

header {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 0.4rem;
}

h3 {
  font-style: italic;
  font-weight: 400;
  font-size: 1.05rem;
  color: var(--ink-soft);
}

.scrubber {
  display: flex;
  align-items: center;
  gap: 0.9rem;
  margin: 0.8rem 0 1rem;
}

input[type='range'] {
  flex: 1;
  accent-color: var(--tide);
}

.position {
  font-size: 0.7rem;
  color: var(--ink-soft);
  white-space: nowrap;
}

.frames {
  max-height: 20rem;
  overflow-y: auto;
}

.frame {
  display: grid;
  grid-template-columns: 5.5rem 1fr;
  gap: 0.8rem;
  padding: 0.4rem 0;
  border-bottom: 1px solid var(--hairline-soft);
}

.author {
  font-size: 0.65rem;
  color: var(--ink-faint);
  text-align: right;
  padding-top: 0.2rem;
}

.frame-body :deep(p) {
  margin: 0 0 0.4em;
}

.empty {
  text-align: center;
  color: var(--ink-faint);
  font-size: 0.72rem;
  padding: 1rem;
}
</style>
