<script setup lang="ts">
// One blip, styled after the Brand v2 conversation card: colored avatar
// circle, name + time caption, message body, and (when reading in another
// language) the Dusk "translated" overlay chip.
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
    <span
      class="avatar"
      :style="{ background: participantColor(entry.author) }"
      :title="entry.author"
    >
      {{ localPart(entry.author).slice(0, 1).toUpperCase() }}
    </span>
    <div class="body">
      <header class="byline">
        <span class="author">{{ localPart(entry.author) }}</span>
        <time class="caption">{{ timeOf(entry.ts) }}</time>
      </header>
      <EditorContent :editor="editor" class="blip-editor" />
      <div v-if="translation" class="translation">
        <span class="tag tag-dusk">✓ translated</span>
        <p>{{ translation }}</p>
      </div>
      <button class="reply" @click="emit('reply', entry.id)">↳ Reply</button>
    </div>
  </article>
</template>

<style scoped>
.blip {
  display: flex;
  gap: 0.75rem;
  margin-left: calc(var(--depth) * 1.7rem);
  padding: 0.85rem 0;
  border-bottom: 1px solid var(--hairline-soft);
  position: relative;
}

/* Thread tie-line for replies. */
.blip:not([style*='--depth: 0'])::before {
  content: '';
  position: absolute;
  left: -0.95rem;
  top: 0;
  bottom: 0;
  border-left: 2px solid var(--sky-t);
}

.avatar {
  flex: none;
  width: 2.1rem;
  height: 2.1rem;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: #fff;
  font-family: var(--font-display);
  font-size: 0.85rem;
  font-weight: 700;
  user-select: none;
  box-shadow: 0 2px 6px rgba(11, 27, 43, 0.15);
}

.body {
  min-width: 0;
  flex: 1;
}

.byline {
  display: flex;
  align-items: baseline;
  gap: 0.6rem;
  margin-bottom: 0.15rem;
}

.author {
  font-weight: 700;
  font-size: 0.88rem;
  color: var(--ink);
}

/* Translation overlay: derived, never the document itself (PRD §9). */
.translation {
  margin-top: 0.4rem;
  padding: 0.5rem 0.7rem;
  background: var(--dusk-t);
  border-radius: 10px;
}

.translation p {
  margin: 0.3rem 0 0;
  color: #4c53b8;
  font-size: 0.92rem;
}

.reply {
  opacity: 0;
  background: none;
  border: none;
  color: var(--deep);
  font-family: var(--font-body);
  font-size: 0.76rem;
  font-weight: 600;
  cursor: pointer;
  padding: 0.3rem 0;
  transition: opacity 0.15s ease;
}

.blip:hover .reply,
.blip:focus-within .reply {
  opacity: 1;
}
</style>
