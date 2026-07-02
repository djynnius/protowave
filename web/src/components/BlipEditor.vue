<script setup lang="ts">
// One blip: a Tiptap editor bound to the blip's Y.XmlFragment, with the
// author/time in the manuscript margin and live collaborator carets.
import { onBeforeUnmount } from 'vue'
import { Editor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import CollaborationCursor from '@tiptap/extension-collaboration-cursor'
import type * as Y from 'yjs'
import type { WaveletProvider } from '../lib/provider'
import { localPart, participantColor, type BlipEntry } from '../lib/wavemodel'

const props = defineProps<{
  entry: BlipEntry
  fragment: Y.XmlFragment
  provider: WaveletProvider
  me: string
  depth: number
  translation?: string
}>()

const emit = defineEmits<{ reply: [parent: string] }>()

const editor = new Editor({
  extensions: [
    // Collaboration ships its own CRDT-aware history.
    StarterKit.configure({ history: false }),
    Collaboration.configure({ fragment: props.fragment }),
    CollaborationCursor.configure({
      provider: { awareness: props.provider.awareness },
      user: { name: localPart(props.me), color: participantColor(props.me) },
    }),
  ],
})

onBeforeUnmount(() => editor.destroy())

function timeOf(ts: number): string {
  return new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}
</script>

<template>
  <article class="blip" :style="{ '--depth': depth }">
    <div class="margin mono">
      <span class="author" :style="{ color: participantColor(entry.author) }">
        {{ localPart(entry.author) }}
      </span>
      <time>{{ timeOf(entry.ts) }}</time>
    </div>
    <div class="body">
      <EditorContent :editor="editor" class="blip-editor" />
      <p v-if="translation" class="translation">
        <span class="tilde" aria-hidden="true">≈</span>{{ translation }}
      </p>
      <button class="reply mono" @click="emit('reply', entry.id)">↳ reply</button>
    </div>
  </article>
</template>

<style scoped>
.blip {
  display: grid;
  grid-template-columns: 6.5rem 1fr;
  gap: 1rem;
  margin-left: calc(var(--depth) * 1.6rem);
  padding: 0.7rem 0;
  border-bottom: 1px solid var(--hairline-soft);
  position: relative;
}

/* Thread tie-line for replies. */
.blip[style*='--depth: 0'] {
  margin-left: 0;
}

.blip:not([style*='--depth: 0'])::before {
  content: '';
  position: absolute;
  left: -0.9rem;
  top: 0;
  bottom: 0;
  border-left: 1px solid var(--hairline);
}

.margin {
  text-align: right;
  padding-top: 0.15rem;
  font-size: 0.68rem;
  letter-spacing: 0.04em;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
  user-select: none;
}

.author {
  font-weight: 500;
}

time {
  color: var(--ink-faint);
}

.body {
  position: relative;
  min-width: 0;
}

/* Translation overlay: derived, never the document itself (PRD §9). */
.translation {
  margin: 0.3rem 0 0;
  padding: 0.35rem 0.6rem;
  background: var(--tide-wash);
  border-left: 2px solid var(--tide);
  border-radius: 0 4px 4px 0;
  font-style: italic;
  color: var(--tide-deep);
  font-size: 0.95rem;
}

.tilde {
  font-family: var(--font-mono);
  margin-right: 0.4rem;
  opacity: 0.6;
}

.reply {
  opacity: 0;
  background: none;
  border: none;
  color: var(--tide-deep);
  font-size: 0.68rem;
  letter-spacing: 0.05em;
  cursor: pointer;
  padding: 0.2rem 0;
  transition: opacity 0.15s ease;
}

.blip:hover .reply,
.blip:focus-within .reply {
  opacity: 1;
}

@media (max-width: 640px) {
  .blip {
    grid-template-columns: 1fr;
    gap: 0.2rem;
  }

  .margin {
    flex-direction: row;
    gap: 0.6rem;
    text-align: left;
  }
}
</style>
