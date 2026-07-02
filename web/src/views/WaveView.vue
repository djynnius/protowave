<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  DialogClose,
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
  DialogTrigger,
} from 'reka-ui'
import { socket, WaveletProvider } from '../lib/provider'
import { useSession } from '../stores/session'
import { useWaves } from '../stores/waves'
import { api, type AttachmentMeta, type ShareMeta } from '../lib/api'
import {
  addReply,
  blips,
  ensureRootBlip,
  localPart,
  manifest,
  participantColor,
  threadOrder,
  type BlipEntry,
} from '../lib/wavemodel'
import WaveMesh from '../components/WaveMesh.vue'
import BlipEditor from '../components/BlipEditor.vue'
import AttachmentCard from '../components/AttachmentCard.vue'
import FolderShareCard from '../components/FolderShareCard.vue'
import PlaybackDrawer from '../components/PlaybackDrawer.vue'

const route = useRoute()
const router = useRouter()
const session = useSession()
const waves = useWaves()

const waveId = decodeURIComponent(route.params.wave as string)
const rootWavelet = `${waveId}/conv+root`
const me = computed(() => session.participant ?? '')
const digest = computed(() => waves.byId(waveId))

const provider = new WaveletProvider(socket, rootWavelet)
const entries = shallowRef<BlipEntry[]>([])
const presence = ref<{ name: string; color: string }[]>([])
const attachments = ref<AttachmentMeta[]>([])
const shares = ref<ShareMeta[]>([])
const folderInput = ref<HTMLInputElement | null>(null)
const sharingFolder = ref(false)
const showPlayback = ref(false)
const translationLang = ref(localStorage.getItem('pw-lang') ?? '')
const enableTranslationOpen = ref(false)

const LANGUAGES: [string, string][] = [
  ['es', 'Español'],
  ['fr', 'Français'],
  ['de', 'Deutsch'],
  ['pt', 'Português'],
  ['yo', 'Yorùbá'],
  ['ig', 'Igbo'],
  ['ha', 'Hausa'],
  ['sw', 'Kiswahili'],
  ['ar', 'العربية'],
  ['zh', '中文'],
  ['en', 'English'],
]

function applyTranslationLang() {
  localStorage.setItem('pw-lang', translationLang.value)
  provider.setTranslationLang(translationLang.value || null)
}

async function enableTranslation() {
  await waves.setTranslation(waveId, true)
  enableTranslationOpen.value = false
  if (translationLang.value) applyTranslationLang()
}
const showDocuments = ref(false)
const uploading = ref(false)
const addOpen = ref(false)
const addName = ref('')
const addError = ref('')
const fileInput = ref<HTMLInputElement | null>(null)

const thread = computed(() => threadOrder(entries.value))

function readManifest() {
  entries.value = manifest(provider.doc).toArray()
}

function readPresence() {
  const seen = new Map<string, { name: string; color: string }>()
  for (const state of provider.awareness.getStates().values()) {
    const user = (state as { user?: { name: string; color: string } }).user
    if (user?.name) seen.set(user.name, user)
  }
  presence.value = [...seen.values()]
}

const stopSync = watch(
  provider.synced,
  (ok) => {
    if (ok) ensureRootBlip(provider.doc, me.value)
  },
  { immediate: true },
)

async function refreshAttachments() {
  try {
    attachments.value = await api.listAttachments(waveId)
  } catch {
    attachments.value = []
  }
  try {
    shares.value = await api.listShares(waveId)
  } catch {
    shares.value = []
  }
}

async function shareFolder(event: Event) {
  const input = event.target as HTMLInputElement
  const files = [...(input.files ?? [])]
  if (files.length === 0) return
  const first = files[0] as File & { webkitRelativePath?: string }
  const folderName = first.webkitRelativePath?.split('/')[0] || 'shared-folder'
  sharingFolder.value = true
  try {
    await api.uploadFolder(waveId, folderName, files)
    await refreshAttachments()
    showDocuments.value = true
  } finally {
    sharingFolder.value = false
    input.value = ''
  }
}

function onShareMirrored(updated: ShareMeta) {
  const i = shares.value.findIndex((s) => s.manifest_hash === updated.manifest_hash)
  if (i >= 0) shares.value[i] = updated
}

onMounted(async () => {
  if (!digest.value) await waves.refresh()
  manifest(provider.doc).observe(readManifest)
  provider.awareness.on('change', readPresence)
  readManifest()
  readPresence()
  refreshAttachments()
  // Resume the reader's preferred language on waves that opted in.
  if (digest.value?.translationEnabled && translationLang.value) applyTranslationLang()
  // Opening the wave marks it read (FR-8).
  api.markRead(waveId).catch(() => {})
  waves.clearUnread(waveId)
})

onBeforeUnmount(() => {
  stopSync()
  api.markRead(waveId).catch(() => {})
  manifest(provider.doc).unobserve(readManifest)
  provider.awareness.off('change', readPresence)
  provider.destroy()
})

function fragmentOf(id: string) {
  return blips(provider.doc).get(id)
}

function reply(parent: string) {
  addReply(provider.doc, me.value, parent)
}

async function addParticipant() {
  addError.value = ''
  const name = addName.value.trim()
  if (!name) return
  try {
    const address = name.includes('@') ? name : `${name}@${me.value.split('@')[1]}`
    await waves.addParticipant(waveId, address)
    addOpen.value = false
    addName.value = ''
  } catch (e) {
    addError.value = e instanceof Error ? e.message : 'could not add'
  }
}

async function uploadFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  uploading.value = true
  try {
    await api.uploadAttachment(waveId, file)
    await refreshAttachments()
    showDocuments.value = true
  } finally {
    uploading.value = false
    input.value = ''
  }
}
</script>

<template>
  <main class="deck">
    <header class="bridge reveal">
      <button class="btn back" @click="router.push({ name: 'inbox' })">← the log</button>

      <div class="crew">
        <span
          v-for="p in presence"
          :key="p.name"
          class="crew-buoy"
          :style="{ background: p.color }"
          :title="p.name"
        >
          {{ p.name.slice(0, 2) }}
        </span>

        <button class="btn" :data-on="showPlayback || undefined" @click="showPlayback = !showPlayback">
          ↺ replay
        </button>
        <button class="btn" :data-on="showDocuments || undefined" @click="showDocuments = !showDocuments">
          ⛁ files<template v-if="attachments.length"> ({{ attachments.length }})</template>
        </button>

        <select
          v-if="digest?.translationEnabled"
          v-model="translationLang"
          class="lang-select mono"
          title="read this wave in…"
          @change="applyTranslationLang"
        >
          <option value="">original</option>
          <option v-for="[code, label] in LANGUAGES" :key="code" :value="code">
            ≈ {{ label }}
          </option>
        </select>
        <DialogRoot v-else v-model:open="enableTranslationOpen">
          <DialogTrigger as-child>
            <button class="btn">≈ translation</button>
          </DialogTrigger>
          <DialogPortal>
            <DialogOverlay class="dialog-overlay" />
            <DialogContent class="dialog-content">
              <DialogTitle class="dialog-title">Enable live translation?</DialogTitle>
              <p class="disclosure">
                When translation is on, the text of this wave is sent to a
                third-party model API (Google Gemini) to produce translations
                for participants reading in other languages. Translations are
                overlays — the original text is always what's stored, and
                every reader can switch back to it at any time.
              </p>
              <div class="dialog-actions">
                <DialogClose as-child>
                  <button type="button" class="btn">not now</button>
                </DialogClose>
                <button type="button" class="btn btn-tide" @click="enableTranslation">
                  enable for this wave
                </button>
              </div>
            </DialogContent>
          </DialogPortal>
        </DialogRoot>

        <DialogRoot v-model:open="addOpen">
          <DialogTrigger as-child>
            <button class="btn">+ crew</button>
          </DialogTrigger>
          <DialogPortal>
            <DialogOverlay class="dialog-overlay" />
            <DialogContent class="dialog-content">
              <DialogTitle class="dialog-title">Add to the wave</DialogTitle>
              <form @submit.prevent="addParticipant">
                <label class="field">
                  <span class="field-label">name or address</span>
                  <input v-model="addName" class="text-input" placeholder="bob" autofocus />
                </label>
                <p v-if="addError" class="error-note">{{ addError }}</p>
                <div class="dialog-actions">
                  <DialogClose as-child>
                    <button type="button" class="btn">cancel</button>
                  </DialogClose>
                  <button type="submit" class="btn btn-tide">add</button>
                </div>
              </form>
            </DialogContent>
          </DialogPortal>
        </DialogRoot>
      </div>
    </header>

    <PlaybackDrawer v-if="showPlayback" :wavelet="rootWavelet" @close="showPlayback = false" />

    <section class="manuscript reveal" style="animation-delay: 0.06s">
      <h1 class="title">{{ digest?.title ?? '…' }}</h1>
      <p class="roster mono">
        <span
          v-for="p in digest?.participants ?? []"
          :key="p"
          :style="{ color: participantColor(p) }"
        >
          {{ localPart(p) }}&nbsp;
        </span>
      </p>
      <WaveMesh class="rule" :active="!provider.synced.value" />
      <p v-if="provider.error.value" class="error-note">{{ provider.error.value }}</p>

      <aside v-if="showDocuments" class="documents">
        <div class="documents-head">
          <h2>shared files</h2>
          <span class="doc-actions">
            <label class="btn" :aria-busy="uploading">
              {{ uploading ? 'uploading…' : '+ file' }}
              <input ref="fileInput" type="file" hidden :disabled="uploading" @change="uploadFile" />
            </label>
            <label class="btn" :aria-busy="sharingFolder">
              {{ sharingFolder ? 'chunking…' : '▤ share a folder' }}
              <input
                ref="folderInput"
                type="file"
                webkitdirectory
                multiple
                hidden
                :disabled="sharingFolder"
                @change="shareFolder"
              />
            </label>
          </span>
        </div>
        <p v-if="attachments.length === 0 && shares.length === 0" class="mono no-files">
          nothing shared yet
        </p>
        <div v-if="shares.length" class="documents-grid shares-grid">
          <FolderShareCard
            v-for="s in shares"
            :key="s.manifest_hash"
            :share="s"
            @mirrored="onShareMirrored"
          />
        </div>
        <div class="documents-grid">
          <AttachmentCard
            v-for="a in attachments"
            :key="a.hash"
            :hash="a.hash"
            :name="a.name"
            :mime="a.mime"
            :size="a.size"
            :uploader="a.uploader"
          />
        </div>
      </aside>

      <div class="thread">
        <BlipEditor
          v-for="node in thread"
          :key="node.entry.id"
          v-show="fragmentOf(node.entry.id)"
          :entry="node.entry"
          :fragment="fragmentOf(node.entry.id)!"
          :provider="provider"
          :me="me"
          :depth="node.depth"
          :translation="translationLang ? provider.translations.value[node.entry.id] : undefined"
          @reply="reply"
        />
      </div>
    </section>
  </main>
</template>

<style scoped>
.deck {
  max-width: 52rem;
  margin: 0 auto;
  padding: 1.6rem 1.25rem 6rem;
}

.bridge {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 2rem;
  gap: 0.6rem;
  flex-wrap: wrap;
}

.back {
  border: none;
  padding-left: 0;
}

.crew {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-wrap: wrap;
}

.crew-buoy {
  width: 1.7rem;
  height: 1.7rem;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: var(--paper);
  font-family: var(--font-mono);
  font-size: 0.62rem;
  font-weight: 500;
  text-transform: uppercase;
  animation: bob 2.8s ease-in-out infinite;
  box-shadow: 0 1px 3px rgba(30, 42, 50, 0.3);
}

.crew-buoy:nth-child(2n) {
  animation-delay: 0.7s;
}

.btn[data-on] {
  border-color: var(--tide);
  color: var(--tide-deep);
}

.lang-select {
  font-size: 0.78rem;
  padding: 0.45rem 0.5rem;
  border: 1px solid var(--tide);
  border-radius: 4px;
  background: var(--tide-wash);
  color: var(--tide-deep);
  cursor: pointer;
}

.disclosure {
  font-size: 0.92rem;
  line-height: 1.5;
  color: var(--ink-soft);
  margin: 0 0 1rem;
}

.title {
  font-size: 2.2rem;
  font-weight: 900;
  letter-spacing: -0.015em;
}

.roster {
  font-size: 0.72rem;
  letter-spacing: 0.05em;
  margin: 0.3rem 0 0.5rem;
}

.rule {
  margin-bottom: 1.6rem;
}

.documents {
  border: 1px solid var(--paper-edge);
  border-left: 3px solid var(--coral);
  border-radius: 8px;
  background: color-mix(in srgb, var(--paper) 70%, #fff);
  padding: 1rem 1.2rem;
  margin-bottom: 1.6rem;
}

.documents-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 0.8rem;
}

.documents-head h2 {
  font-style: italic;
  font-weight: 400;
  font-size: 1.05rem;
  color: var(--ink-soft);
}

.no-files {
  font-size: 0.72rem;
  color: var(--ink-faint);
}

.doc-actions {
  display: flex;
  gap: 0.5rem;
}

.shares-grid {
  margin-bottom: 0.6rem;
}

.documents-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
  gap: 0.6rem;
}

.thread {
  display: flex;
  flex-direction: column;
}
</style>
