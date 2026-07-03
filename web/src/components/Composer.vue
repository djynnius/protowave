<script setup lang="ts">
// Bottom composer (the mockup's chat input). Type a message and send it;
// on send it becomes a new blip. When replying, a quote bar shows who/what
// you're replying to, and the sent blip is threaded under that message.
// Typing broadcasts an ephemeral "is writing…" flag over awareness.
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { Editor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import { useI18n } from 'vue-i18n'
import type { Node as PMNode } from '@tiptap/pm/model'
import type { WaveletProvider } from '../lib/provider'
import { localPart } from '../lib/wavemodel'
import { TagsMentions } from '../lib/tiptap-decorations'

const { t } = useI18n()

const props = defineProps<{
  provider: WaveletProvider
  me: string
  replyTo?: { author: string; snippet: string } | null
}>()

const emit = defineEmits<{ send: [content: PMNode]; cancelReply: [] }>()

const editor = new Editor({
  extensions: [StarterKit, TagsMentions],
  editorProps: {
    handleKeyDown(_view, event) {
      // Enter sends; Shift+Enter makes a newline.
      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault()
        send()
        return true
      }
      return false
    },
  },
  onUpdate: () => setTyping(!editor.isEmpty),
  onBlur: () => setTyping(false),
})

let typing = false
function setTyping(on: boolean) {
  if (on === typing) return
  typing = on
  props.provider.awareness.setLocalStateField('typing', on ? localPart(props.me) : null)
}

function send() {
  if (editor.isEmpty) return
  emit('send', editor.state.doc)
  editor.commands.clearContent()
  setTyping(false)
}

// Focus the input when a reply target is set.
watch(
  () => props.replyTo,
  (r) => {
    if (r) editor.commands.focus('end')
  },
)

onMounted(() => {
  // Expose a focus handle to the parent (reply button focuses the composer).
  ;(window as unknown as { __pwFocusComposer?: () => void }).__pwFocusComposer = () =>
    editor.commands.focus('end')
})
onBeforeUnmount(() => {
  setTyping(false)
  editor.destroy()
})
</script>

<template>
  <div class="composer">
    <div v-if="replyTo" class="reply-quote">
      <span class="reply-bar" />
      <div class="reply-text">
        <span class="reply-to caption">↳ replying to {{ localPart(replyTo.author) }}</span>
        <span class="reply-snippet">{{ replyTo.snippet }}</span>
      </div>
      <button class="reply-cancel" :title="t('cancel')" @click="emit('cancelReply')">×</button>
    </div>
    <div class="input-row">
      <EditorContent :editor="editor" class="blip-editor input" />
      <button class="send" :disabled="editor.isEmpty" :title="t('reply')" @click="send">
        <span aria-hidden="true">➤</span>
      </button>
    </div>
    <p class="hint caption">Enter to send · Shift+Enter for a new line · #tag · @mention</p>
  </div>
</template>

<style scoped>
.composer {
  border: 1px solid var(--mist);
  border-radius: 16px;
  background: #fff;
  box-shadow: var(--shadow-card);
  padding: 0.6rem 0.7rem 0.4rem;
}

.reply-quote {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.35rem 0.2rem 0.5rem;
  margin-bottom: 0.3rem;
  border-bottom: 1px solid var(--hairline-soft);
}

.reply-bar {
  width: 3px;
  align-self: stretch;
  background: var(--deep);
  border-radius: 2px;
}

.reply-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
}

.reply-snippet {
  font-size: 0.85rem;
  color: var(--slate);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.reply-cancel {
  background: none;
  border: none;
  color: var(--steel);
  font-size: 1.1rem;
  cursor: pointer;
  line-height: 1;
}

.input-row {
  display: flex;
  align-items: flex-end;
  gap: 0.5rem;
}

.input {
  flex: 1;
  max-height: 40vh;
  overflow-y: auto;
  padding: 0.45rem 0.5rem;
}

.send {
  flex: none;
  width: 2.4rem;
  height: 2.4rem;
  border-radius: 50%;
  border: none;
  background: var(--deep);
  color: #fff;
  font-size: 0.95rem;
  cursor: pointer;
  transition:
    transform 0.12s ease,
    background 0.12s ease;
}

.send:hover:not(:disabled) {
  transform: translateY(-1px);
  background: #2459ce;
}

.send:disabled {
  opacity: 0.4;
  cursor: default;
}

.hint {
  margin: 0.3rem 0 0.1rem 0.5rem;
  font-size: 0.62rem;
}
</style>
