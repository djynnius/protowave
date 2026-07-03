<script setup lang="ts">
// A conversation message, rendered read-only unless it's your own (then an
// ✎ Edit / ✓ Done toggle makes it editable). Own posts can also be deleted
// (tombstoned so replies survive). Reactions live in the CRDT. Names show
// real first/last when the author has set a profile.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Editor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Collaboration from '@tiptap/extension-collaboration'
import type * as Y from 'yjs'
import { useI18n } from 'vue-i18n'
import { useProfiles } from '../stores/profiles'
import {
  isAgent,
  isDeleted,
  localPart,
  participantColor,
  readReactions,
  toggleReaction,
  type BlipEntry,
} from '../lib/wavemodel'
import { TagsMentions } from '../lib/tiptap-decorations'

const { t } = useI18n()
const profiles = useProfiles()

const props = defineProps<{
  entry: BlipEntry
  fragment: Y.XmlFragment
  doc: Y.Doc
  me: string
  depth: number
  role?: string
  translation?: string
  replying?: boolean
  highlight?: boolean
}>()

const emit = defineEmits<{
  reply: [id: string]
  tag: [tag: string]
  delete: [id: string]
  profile: [participant: string]
}>()

const agent = computed(() => isAgent(props.entry.author))
const own = computed(() => !agent.value && props.entry.author === props.me)
const deleted = ref(isDeleted(props.doc, props.entry.id))
const editing = ref(false)
const showPicker = ref(false)
const root = ref<HTMLElement | null>(null)
const name = computed(() =>
  agent.value ? 'assistant' : profiles.nameOf(props.entry.author),
)

const EMOJI = ['👍', '🎉', '🌊', '❤️', '😂', '👀', '✅']
const reacts = ref(readReactions(props.doc, props.entry.id))
function refreshReactions() {
  reacts.value = readReactions(props.doc, props.entry.id)
  deleted.value = isDeleted(props.doc, props.entry.id)
}

const editor = new Editor({
  editable: false,
  extensions: [
    StarterKit.configure({ history: false }),
    Collaboration.configure({ fragment: props.fragment }),
    TagsMentions,
  ],
})

// Reactions/tombstones change outside this editor; observe the shared maps.
props.doc.getMap('reactions').observeDeep(refreshReactions)
props.doc.getMap('deleted').observe(refreshReactions)

watch(editing, (on) => {
  editor.setEditable(on)
  if (on) editor.commands.focus('end')
})
watch(
  () => props.highlight,
  (on) => {
    if (on) root.value?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  },
)

function toggleEdit() {
  editing.value = !editing.value
}
function react(emoji: string) {
  toggleReaction(props.doc, props.entry.id, emoji, localPart(props.me))
  showPicker.value = false
  refreshReactions()
}
function onClick(event: MouseEvent) {
  if (editing.value) return
  const el = (event.target as HTMLElement).closest('.pw-tag')
  if (el) emit('tag', (el.getAttribute('data-token') || '').replace(/^#/, ''))
}
function reactedByMe(r: { users: string[] }): boolean {
  return r.users.includes(localPart(props.me))
}

onBeforeUnmount(() => {
  props.doc.getMap('reactions').unobserveDeep(refreshReactions)
  props.doc.getMap('deleted').unobserve(refreshReactions)
  editor.destroy()
})

function timeOf(ts: number): string {
  const delta = Date.now() - ts
  if (delta < 60_000) return t('justNow')
  if (delta < 3_600_000) return t('minutesAgo', { n: Math.floor(delta / 60_000) })
  if (delta < 86_400_000) return t('hoursAgo', { n: Math.floor(delta / 3_600_000) })
  return new Date(ts).toLocaleDateString()
}
</script>

<template>
  <article
    ref="root"
    class="msg"
    :class="{ agent, replying, highlight, editing, deleted }"
    :style="{ '--depth': depth }"
  >
    <span
      class="avatar"
      :class="{ 'avatar-agent': agent }"
      :style="agent ? undefined : { background: participantColor(entry.author) }"
      :title="entry.author"
      role="button"
      @click="emit('profile', entry.author)"
    >
      {{ agent ? '✳' : localPart(entry.author).slice(0, 1).toUpperCase() }}
    </span>
    <div class="body">
      <header class="byline">
        <span class="author" role="button" @click="emit('profile', entry.author)">{{ name }}</span>
        <span v-if="agent" class="tag tag-agent">assistant</span>
        <span class="role caption">{{ role || 'editor' }} · {{ timeOf(entry.ts) }}</span>
      </header>

      <p v-if="deleted" class="tombstone">{{ t('messageDeleted') }}</p>
      <template v-else>
        <EditorContent :editor="editor" class="blip-editor" @click="onClick" />
        <div v-if="translation" class="translation">
          <span class="tag tag-dusk">{{ t('translated') }}</span>
          <p>{{ translation }}</p>
        </div>

        <div v-if="reacts.length" class="reactions">
          <button
            v-for="r in reacts"
            :key="r.emoji"
            class="reaction"
            :class="{ mine: reactedByMe(r) }"
            @click="react(r.emoji)"
          >
            {{ r.emoji }} {{ r.users.length }}
          </button>
        </div>

        <div class="actions">
          <button class="act" @click="emit('reply', entry.id)">↳ {{ t('reply') }}</button>
          <span class="react-wrap">
            <button class="act" @click="showPicker = !showPicker">☺ {{ t('react') }}</button>
            <span v-if="showPicker" class="picker">
              <button v-for="e in EMOJI" :key="e" class="emoji" @click="react(e)">{{ e }}</button>
            </span>
          </span>
          <button v-if="own" class="act" @click="toggleEdit">
            {{ editing ? '✓ ' + t('done') : '✎ ' + t('edit') }}
          </button>
          <button v-if="own" class="act danger" @click="emit('delete', entry.id)">
            🗑 {{ t('delete') }}
          </button>
        </div>
      </template>
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

.msg.editing {
  background: #fff;
  box-shadow: 0 0 0 1px var(--deep);
}

.msg.editing .actions {
  opacity: 1;
}

.msg.editing :deep(.ProseMirror) {
  outline: none;
  min-height: 1.4em;
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
  cursor: pointer;
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
  cursor: pointer;
}

.author:hover {
  text-decoration: underline;
}

.tombstone {
  color: var(--steel);
  font-style: italic;
  margin: 0.2rem 0;
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

.reactions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.3rem;
  margin-top: 0.4rem;
}

.reaction {
  border: 1px solid var(--mist);
  background: #fff;
  border-radius: 999px;
  padding: 0.1rem 0.55rem;
  font-size: 0.8rem;
  cursor: pointer;
}

.reaction.mine {
  border-color: var(--deep);
  background: var(--sky-t);
  color: var(--deep);
}

.actions {
  margin-top: 0.2rem;
  display: flex;
  align-items: center;
  gap: 0.7rem;
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

.act.danger {
  color: #d33;
}

.react-wrap {
  position: relative;
}

.picker {
  position: absolute;
  bottom: 1.4rem;
  left: 0;
  display: flex;
  gap: 0.1rem;
  background: #fff;
  border: 1px solid var(--mist);
  border-radius: 999px;
  padding: 0.2rem 0.35rem;
  box-shadow: var(--shadow-card);
  z-index: 5;
}

.emoji {
  background: none;
  border: none;
  font-size: 1rem;
  cursor: pointer;
  padding: 0.1rem;
}

.emoji:hover {
  transform: scale(1.2);
}
</style>
