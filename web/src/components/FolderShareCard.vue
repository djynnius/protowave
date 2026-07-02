<script setup lang="ts">
// A shared folder in the wave's file panel (PRD §11): brand card with the
// folder's stats, a browse dialog listing files (each downloadable — chunks
// federate transparently), and a mirror toggle (FR-58).
import { ref } from 'vue'
import {
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from 'reka-ui'
import { api, type ShareManifest, type ShareMeta } from '../lib/api'
import { localPart } from '../lib/wavemodel'

const props = defineProps<{ share: ShareMeta }>()
const emit = defineEmits<{ mirrored: [ShareMeta] }>()

const open = ref(false)
const manifest = ref<ShareManifest | null>(null)
const mirroring = ref(false)

function prettySize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`
  return `${(n / 1073741824).toFixed(2)} GB`
}

async function browse() {
  manifest.value = await api.shareManifest(props.share.manifest_hash)
  open.value = true
}

function fileUrl(path: string): string {
  return `/api/shares/${props.share.manifest_hash}/file?path=${encodeURIComponent(path)}`
}

async function mirror() {
  mirroring.value = true
  try {
    emit('mirrored', await api.mirrorShare(props.share.manifest_hash))
  } finally {
    mirroring.value = false
  }
}
</script>

<template>
  <div class="share-card">
    <span class="folder-glyph" aria-hidden="true">▤</span>
    <div class="info">
      <span class="name">{{ share.name }}</span>
      <span class="meta caption">
        {{ share.file_count }} files · {{ prettySize(share.total_size) }} ·
        {{ localPart(share.uploader) }}
      </span>
      <span class="row">
        <button class="linkish" @click="browse">browse</button>
        <span v-if="share.mirrored" class="tag tag-live" title="all chunks held locally">
          ⬤ mirrored
        </span>
        <button v-else class="linkish" :disabled="mirroring" @click="mirror">
          {{ mirroring ? 'pinning…' : 'mirror here' }}
        </button>
      </span>
    </div>

    <DialogRoot v-model:open="open">
      <DialogPortal>
        <DialogOverlay class="dialog-overlay" />
        <DialogContent class="dialog-content share-dialog">
          <DialogTitle class="dialog-title">▤ {{ share.name }}</DialogTitle>
          <p class="caption origin">
            shared from {{ share.origin_domain }} · verified chunk-by-chunk
          </p>
          <ul v-if="manifest" class="files">
            <li v-for="f in manifest.files" :key="f.path">
              <span class="path">{{ f.path }}</span>
              <span class="caption">{{ prettySize(f.size) }}</span>
              <a class="linkish" :href="fileUrl(f.path)">download</a>
            </li>
          </ul>
        </DialogContent>
      </DialogPortal>
    </DialogRoot>
  </div>
</template>

<style scoped>
.share-card {
  display: flex;
  gap: 0.7rem;
  align-items: flex-start;
  border: 1px solid var(--mist);
  border-radius: 12px;
  padding: 0.6rem 0.8rem;
  background: #fff;
}

.folder-glyph {
  font-size: 1.5rem;
  line-height: 1.2;
  color: var(--deep);
}

.info {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  min-width: 0;
}

.name {
  font-weight: 700;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row {
  display: flex;
  align-items: center;
  gap: 0.7rem;
}

.linkish {
  background: none;
  border: none;
  padding: 0;
  font-family: var(--font-body);
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--deep);
  cursor: pointer;
  text-decoration: none;
}

.linkish:disabled {
  opacity: 0.5;
}

.share-dialog {
  width: min(38rem, 94vw);
  max-height: 80vh;
  overflow-y: auto;
}

.origin {
  margin: -0.5rem 0 0.9rem;
}

.files {
  list-style: none;
  margin: 0;
  padding: 0;
}

.files li {
  display: flex;
  align-items: baseline;
  gap: 0.8rem;
  padding: 0.45rem 0;
  border-bottom: 1px solid var(--hairline-soft);
}

.path {
  font-family: var(--font-mono);
  font-size: 0.8rem;
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
