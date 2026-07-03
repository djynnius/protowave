<script setup lang="ts">
// A conversation message, rendered read-only (the mockup's chat model — you
// compose new blips in the bottom composer, not by editing inline). Still a
// live CRDT view: a read-only Tiptap bound to the fragment updates in place
// as it syncs, and keeps #tag/@mention chips and the translation overlay.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Editor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import type * as Y from 'yjs'
import { useI18n } from 'vue-i18n'
import { isAgent, localPart, participantColor, type BlipEntry } from '../lib/wavemodel'
import { TagsMentions } from '../lib/tiptap-decorations'

const { t } = useI18n()

const props = defineProps<{
  entry: BlipEntry
  fragment: Y.XmlFragment
  me: string
  depth: number
  role?: string
  translation?: string
  replying?: boolean
  highlight?: boolean
}>()

const emit = defineEmits<{ reply: [id: string]; tag: [tag: string] }>()

const agent = computed(() => isAgent(props.entry.author))
const root = ref<HTMLElement | null>(null)

const editor = new Editor({
  editable: false,
  extensions: [
    StarterKit.configure({ history: false }),
    Collaboration.configure({ fragment: props.fragment }),
    TagsMentions,
  ],
})

onBeforeUnmount(() => editor.destroy())

// Centre this message when it becomes the reply target (or during autoplay).
watch(
  () => props.highlight,
  (on) => {
    if (on) root.value?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  },
)

function onClick(event: MouseEvent) {
  const el = (event.target as HTMLElement).closest('.pw-tag')
  if (el) emit('tag', (el.getAttribute('data-token') || '').replace(/^#/, ''))
}

function timeOf(ts: number): string {
  const delta = Date.now() - ts
  if (delta < 60_000) return t('justNow')
  if (delta < 3_600_000) return t('minutesAgo', { n: Math.floor(delta / 60_000) })
  if (delta < 86_400_000) return t('hoursAgo', { n: Math.floor(delta / 3_600_000) })
  return new Date(ts).toLocaleDateString()
}

defineExpose({ root })
</script>

<template>
  <article
    ref="root"
    class="msg"
    :class="{ agent, replying, highlight }"
    :style="{ '--depth': depth }"
  >
    <span
      class="avatar"
      :class="{ 'avatar-agent': agent }"
      :style="agent ? undefined : { background: participantColor(entry.author) }"
      :title="entry.author"
    >
      {{ agent ? '✳' : localPart(entry.author).slice(0, 1).toUpperCase() }}
    </span>
    <div class="body">
      <header class="byline">
        <span class="author">{{ localPart(entry.author) }}</span>
        <span v-if="agent" class="tag tag-agent">assistant</span>
        <span class="role caption">{{ role || 'editor' }} · {{ timeOf(entry.ts) }}</span>
      </header>
      <!-- eslint-disable-next-line vue/no-v-html -->
      <EditorContent :editor="editor" class="blip-editor" @click="onClick" />
      <div v-if="translation" class="translation">
        <span class="tag tag-dusk">{{ t('translated') }}</span>
        <p>{{ translation }}</p>
      </div>
      <div class="actions">
        <button class="act" @click="emit('reply', entry.id)">↳ {{ t('reply') }}</button>
      </div>
    </div>
  </article>
</template>

<style scoped>
.msg {
  display: flex;
  gap: 0.85rem;
  margin-left: calc(var(--depth) * 1.7rem);
  padding: 0.7rem 0.6rem;
  border-radius: 12px;
  position: relative;
  transition:
    background 0.25s ease,
    box-shadow 0.25s ease;
}

.msg:hover {
  background: color-mix(in srgb, var(--sky-t) 55%, transparent);
}

.msg:not([style*='--depth: 0'])::before {
  content: '';
  position: absolute;
  left: -0.9rem;
  top: 0;
  bottom: 0;
  border-left: 2px solid var(--sky-t);
}

.msg.replying {
  background: var(--sky-t);
  box-shadow: inset 3px 0 0 var(--deep);
}

.msg.highlight {
  background: color-mix(in srgb, var(--crest-t) 60%, transparent);
  box-shadow: 0 0 0 1px var(--crest);
}

.msg.agent {
  background: linear-gradient(90deg, color-mix(in srgb, var(--crest-t) 55%, transparent), transparent 72%);
}

.avatar {
  flex: none;
  width: 2.2rem;
  height: 2.2rem;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 0.9rem;
  box-shadow: 0 2px 6px rgba(11, 27, 43, 0.15);
  user-select: none;
}

.avatar-agent {
  background: linear-gradient(135deg, var(--crest), var(--deep));
}

.body {
  min-width: 0;
  flex: 1;
}

.byline {
  display: flex;
  align-items: baseline;
  gap: 0.55rem;
  margin-bottom: 0.15rem;
}

.author {
  font-weight: 700;
  font-size: 0.9rem;
}

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

.actions {
  margin-top: 0.15rem;
  opacity: 0;
  transition: opacity 0.15s ease;
}

.msg:hover .actions,
.msg:focus-within .actions {
  opacity: 1;
}

.act {
  background: none;
  border: none;
  color: var(--deep);
  font-family: var(--font-body);
  font-size: 0.76rem;
  font-weight: 600;
  cursor: pointer;
  padding: 0.15rem 0;
}
</style>
