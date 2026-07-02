<script setup lang="ts">
// One attachment in the wave's document panel: image preview inline,
// markdown files open a rendered view, everything downloads.
import { ref } from 'vue'
import {
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from 'reka-ui'
import { renderMarkdown, highlightCodeBlocks } from '../lib/markdown'
import { localPart } from '../lib/wavemodel'

const props = defineProps<{
  hash: string
  name: string
  mime: string
  size: number
  uploader: string
}>()

const mdOpen = ref(false)
const mdHtml = ref('')
const mdEl = ref<HTMLElement | null>(null)

const url = `/api/attachments/${props.hash}`
const isImage = props.mime.startsWith('image/')
const isMarkdown =
  props.mime === 'text/markdown' || props.name.toLowerCase().endsWith('.md')

function prettySize(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / 1048576).toFixed(1)} MB`
}

async function openMarkdown() {
  const res = await fetch(url, { credentials: 'same-origin' })
  mdHtml.value = renderMarkdown(await res.text())
  mdOpen.value = true
  requestAnimationFrame(() => {
    if (mdEl.value) highlightCodeBlocks(mdEl.value)
  })
}
</script>

<template>
  <div class="attachment">
    <a v-if="isImage" :href="url" target="_blank" class="thumb-link">
      <img :src="url" :alt="name" loading="lazy" />
    </a>
    <div class="info">
      <span class="name">{{ name }}</span>
      <span class="meta mono">{{ prettySize(size) }} · {{ localPart(uploader) }}</span>
      <span class="actions">
        <button v-if="isMarkdown" class="linkish mono" @click="openMarkdown">view</button>
        <a class="linkish mono" :href="url" :download="name">download</a>
      </span>
    </div>

    <DialogRoot v-model:open="mdOpen">
      <DialogPortal>
        <DialogOverlay class="dialog-overlay" />
        <DialogContent class="dialog-content md-dialog">
          <DialogTitle class="dialog-title mono">{{ name }}</DialogTitle>
          <!-- eslint-disable-next-line vue/no-v-html — sanitized by DOMPurify -->
          <div ref="mdEl" class="md-body" v-html="mdHtml" />
        </DialogContent>
      </DialogPortal>
    </DialogRoot>
  </div>
</template>

<style scoped>
.attachment {
  display: flex;
  gap: 0.7rem;
  align-items: center;
  border: 1px solid var(--hairline-soft);
  border-radius: 6px;
  padding: 0.5rem 0.7rem;
  background: color-mix(in srgb, var(--paper) 60%, #fff);
}

.thumb-link img {
  width: 3.2rem;
  height: 3.2rem;
  object-fit: cover;
  border-radius: 4px;
  display: block;
}

.info {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
  min-width: 0;
}

.name {
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta {
  font-size: 0.68rem;
  color: var(--ink-faint);
}

.actions {
  display: flex;
  gap: 0.7rem;
}

.linkish {
  background: none;
  border: none;
  padding: 0;
  font-size: 0.7rem;
  letter-spacing: 0.04em;
  color: var(--tide-deep);
  cursor: pointer;
  text-decoration: none;
}

.md-dialog {
  width: min(44rem, 94vw);
  max-height: 82vh;
  overflow-y: auto;
}

.md-body :deep(h1),
.md-body :deep(h2),
.md-body :deep(h3) {
  font-family: var(--font-display);
}

.md-body :deep(pre) {
  border-radius: 6px;
  padding: 0.8rem 1rem;
  overflow-x: auto;
  font-size: 0.85rem;
}

.md-body :deep(code) {
  font-family: var(--font-mono);
}

.md-body :deep(table) {
  border-collapse: collapse;
}

.md-body :deep(td),
.md-body :deep(th) {
  border: 1px solid var(--hairline);
  padding: 0.3rem 0.6rem;
}
</style>
