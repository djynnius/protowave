<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import {
  DialogClose,
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from 'reka-ui'
import type { Node as PMNode } from '@tiptap/pm/model'
import { socket, WaveletProvider } from '../lib/provider'
import { useSession } from '../stores/session'
import { useWaves } from '../stores/waves'
import { api, type AttachmentMeta, type ShareMeta } from '../lib/api'
import {
  blips,
  deleteBlip,
  isDeleted,
  localPart,
  manifest,
  participantColor,
  postBlip,
  threadOrder,
  type BlipEntry,
} from '../lib/wavemodel'
import WaveMesh from '../components/WaveMesh.vue'
import WaveSidebar from '../components/WaveSidebar.vue'
import WavePanel from '../components/WavePanel.vue'
import BlipMessage from '../components/BlipMessage.vue'
import Composer from '../components/Composer.vue'
import AttachmentCard from '../components/AttachmentCard.vue'
import FolderShareCard from '../components/FolderShareCard.vue'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()
const session = useSession()
const waves = useWaves()

const waveId = computed(() => decodeURIComponent(route.params.wave as string))
const rootWavelet = computed(() => `${waveId.value}/conv+root`)
const me = computed(() => session.participant ?? '')
const digest = computed(() => waves.byId(waveId.value))

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

// A fresh provider per opened wave, swapped when the route param changes.
let provider = new WaveletProvider(socket, rootWavelet.value)
const entries = shallowRef<BlipEntry[]>([])
const online = ref<Set<string>>(new Set())
const typingNames = ref<string[]>([])
const attachments = ref<AttachmentMeta[]>([])
const shares = ref<ShareMeta[]>([])
const showFiles = ref(false)
const uploading = ref(false)
const sharingFolder = ref(false)
const translationLang = ref(localStorage.getItem('pw-lang') ?? '')
const replyTarget = ref<{ id: string; author: string; snippet: string } | null>(null)
const threadEl = ref<HTMLElement | null>(null)
const addOpen = ref(false)
const addName = ref('')
const addError = ref('')

async function addParticipant() {
  addError.value = ''
  const name = addName.value.trim()
  if (!name) return
  try {
    // Bare names are treated as local to this server; full user@domain
    // addresses (including remote ones) are accepted as-is.
    const address = name.includes('@') ? name : `${name}@${me.value.split('@')[1]}`
    await waves.addParticipant(waveId.value, address)
    addOpen.value = false
    addName.value = ''
  } catch (e) {
    addError.value = e instanceof Error ? e.message : t('couldNotAdd')
  }
}

function blipText(id: string): string {
  const frag = blips(provider.doc).get(id)
  return (frag?.toString().replace(/<[^>]*>/g, ' ') ?? '').trim()
}

// Render blips with content, plus deleted ones as tombstones (so their
// replies stay threaded); skip only empty non-deleted drafts.
const thread = computed(() =>
  threadOrder(entries.value).filter(
    (n) => blipText(n.entry.id).length > 0 || isDeleted(provider.doc, n.entry.id),
  ),
)

// ---- autoplay: reveal the conversation message by message ----
const playing = ref(false)
const revealCount = ref(0)
let playTimer: ReturnType<typeof setInterval> | null = null

// Autoplay reveals messages in the order they were actually written (by
// timestamp), not thread order — a late reply to an early post appears at
// its real moment, still slotted into its threaded position.
const chrono = computed(() => [...thread.value].sort((a, b) => a.entry.ts - b.entry.ts))
const revealed = computed(
  () => new Set(chrono.value.slice(0, revealCount.value).map((n) => n.entry.id)),
)
const visible = computed(() =>
  playing.value ? thread.value.filter((n) => revealed.value.has(n.entry.id)) : thread.value,
)
const highlightId = computed(() =>
  playing.value && revealCount.value > 0
    ? chrono.value[revealCount.value - 1]?.entry.id
    : replyTarget.value?.id,
)

function togglePlay() {
  if (playing.value) return stopPlay()
  if (thread.value.length === 0) return
  playing.value = true
  revealCount.value = 1
  playTimer = setInterval(() => {
    if (revealCount.value >= thread.value.length) stopPlay()
    else revealCount.value += 1
  }, 1600)
}
function stopPlay() {
  playing.value = false
  if (playTimer) clearInterval(playTimer)
  playTimer = null
}

// ---- trending tags derived from the conversation ----
const trending = computed(() => {
  const counts = new Map<string, number>()
  for (const n of thread.value) {
    for (const m of blipText(n.entry.id).matchAll(/(^|\s)#([\p{L}\p{N}_-]+)/gu)) {
      const tag = m[2].toLowerCase()
      counts.set(tag, (counts.get(tag) ?? 0) + 1)
    }
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 10)
    .map(([tag]) => tag)
})

let readTimer: ReturnType<typeof setTimeout> | null = null
function readManifest() {
  entries.value = manifest(provider.doc).toArray()
  // A post arrived while this wave is open — keep it marked read (debounced),
  // both locally and on the server, so it never shows an unread indicator.
  waves.clearUnread(waveId.value)
  if (readTimer) clearTimeout(readTimer)
  readTimer = setTimeout(() => api.markRead(waveId.value).catch(() => {}), 800)
}
function readPresence() {
  const seen = new Set<string>()
  const typing: string[] = []
  for (const state of provider.awareness.getStates().values()) {
    const s = state as { user?: { name: string }; typing?: string | null }
    if (s.user?.name) seen.add(s.user.name)
    if (s.typing && s.typing !== localPart(me.value)) typing.push(s.typing)
  }
  online.value = seen
  typingNames.value = [...new Set(typing)]
}

async function refreshFiles() {
  attachments.value = await api.listAttachments(waveId.value).catch(() => [])
  shares.value = await api.listShares(waveId.value).catch(() => [])
}

function wireProvider() {
  manifest(provider.doc).observe(readManifest)
  provider.awareness.on('change', readPresence)
  readManifest()
  readPresence()
  if (digest.value?.translationEnabled && translationLang.value) {
    provider.setTranslationLang(translationLang.value)
  }
}
function unwireProvider() {
  manifest(provider.doc).unobserve(readManifest)
  provider.awareness.off('change', readPresence)
  provider.destroy()
}

onMounted(async () => {
  if (!digest.value) await waves.refresh()
  wireProvider()
  refreshFiles()
  waves.setActive(waveId.value)
  api.markRead(waveId.value).catch(() => {})
})

// Switching waves in the sidebar swaps the provider without leaving.
watch(rootWavelet, (next, prev) => {
  if (next === prev) return
  // Mark the wave we're leaving as read before switching.
  const oldWave = prev.replace('/conv+root', '')
  api.markRead(oldWave).catch(() => {})
  stopPlay()
  unwireProvider()
  replyTarget.value = null
  provider = new WaveletProvider(socket, next)
  wireProvider()
  refreshFiles()
  waves.setActive(waveId.value)
  api.markRead(waveId.value).catch(() => {})
})

onBeforeUnmount(() => {
  stopPlay()
  if (readTimer) clearTimeout(readTimer)
  api.markRead(waveId.value).catch(() => {})
  waves.setActive(null)
  unwireProvider()
})

function fragmentOf(id: string) {
  return blips(provider.doc).get(id)
}
function roleOf(author: string): string {
  return author === digest.value?.createdBy ? 'owner' : 'editor'
}

function startReply(id: string) {
  const entry = entries.value.find((e) => e.id === id)
  replyTarget.value = { id, author: entry?.author ?? '', snippet: blipText(id).slice(0, 80) }
  ;(window as unknown as { __pwFocusComposer?: () => void }).__pwFocusComposer?.()
}

function onSend(content: PMNode) {
  const parent = replyTarget.value?.id ?? null
  postBlip(provider.doc, me.value, parent, content)
  replyTarget.value = null
  requestAnimationFrame(() =>
    threadEl.value?.scrollTo({ top: threadEl.value.scrollHeight, behavior: 'smooth' }),
  )
}

function onTag(tag: string) {
  router.push({ name: 'wave', params: { wave: route.params.wave }, query: { q: `#${tag}` } })
}

function onDelete(id: string) {
  deleteBlip(provider.doc, id)
}

// Participant profile card.
const profileOpen = ref(false)
const profileData = ref<{
  participant: string
  displayName: string
  sharedWaves: { wave: string; title: string }[]
} | null>(null)
async function openProfile(participant: string) {
  profileOpen.value = true
  profileData.value = null
  try {
    profileData.value = await api.userProfile(participant)
  } catch {
    profileData.value = { participant, displayName: localPart(participant), sharedWaves: [] }
  }
}
function goToWave(wave: string) {
  profileOpen.value = false
  router.push({ name: 'wave', params: { wave: encodeURIComponent(wave) } })
}

// Up to 5 participant avatars in the header, then a +N overflow.
const shownParticipants = computed(() => (digest.value?.participants ?? []).slice(0, 5))
const overflowParticipants = computed(() => Math.max(0, (digest.value?.participants?.length ?? 0) - 5))

function setLang(code: string) {
  translationLang.value = code
  localStorage.setItem('pw-lang', code)
  provider.setTranslationLang(code || null)
}
async function enableTranslation() {
  await waves.setTranslation(waveId.value, true)
  if (translationLang.value) provider.setTranslationLang(translationLang.value)
}

async function uploadFile(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  uploading.value = true
  try {
    await api.uploadAttachment(waveId.value, file)
    await refreshFiles()
  } finally {
    uploading.value = false
    input.value = ''
  }
}
async function shareFolder(event: Event) {
  const input = event.target as HTMLInputElement
  const files = [...(input.files ?? [])]
  if (!files.length) return
  const first = files[0] as File & { webkitRelativePath?: string }
  const name = first.webkitRelativePath?.split('/')[0] || 'shared-folder'
  sharingFolder.value = true
  try {
    await api.uploadFolder(waveId.value, name, files)
    await refreshFiles()
  } finally {
    sharingFolder.value = false
    input.value = ''
  }
}
</script>

<template>
  <div class="app-shell">
    <WaveSidebar :current="waveId" />

    <main class="conversation">
      <header class="wave-header">
        <div class="head-main">
          <h2 class="wave-title">{{ digest?.title ?? '…' }}</h2>
          <WaveMesh class="rule" :active="!provider.synced.value || playing" :width="200" />
        </div>
        <div class="head-actions">
          <div class="avatar-stack">
            <span
              v-for="p in shownParticipants"
              :key="p"
              class="stack-avatar"
              :style="{ background: participantColor(p) }"
              :title="p"
              role="button"
              @click="openProfile(p)"
            >
              {{ localPart(p).slice(0, 1).toUpperCase() }}
            </span>
            <span v-if="overflowParticipants" class="stack-avatar overflow">
              +{{ overflowParticipants }}
            </span>
          </div>
          <button class="btn" :class="{ on: playing }" @click="togglePlay">
            {{ playing ? '⏸ ' + t('stop') : '▶ ' + t('play') }}
          </button>
          <button class="btn" :class="{ on: showFiles }" @click="showFiles = !showFiles">
            ⛁ {{ shares.length + attachments.length || '' }}
          </button>
        </div>
      </header>

      <section v-if="showFiles" class="files">
        <div class="files-actions">
          <label class="btn">
            {{ uploading ? t('uploading') : t('shareFile') }}
            <input type="file" hidden :disabled="uploading" @change="uploadFile" />
          </label>
          <label class="btn">
            {{ sharingFolder ? t('chunking') : t('shareFolder') }}
            <input
              type="file"
              webkitdirectory
              multiple
              hidden
              :disabled="sharingFolder"
              @change="shareFolder"
            />
          </label>
        </div>
        <div class="files-grid">
          <FolderShareCard
            v-for="s in shares"
            :key="s.manifest_hash"
            :share="s"
            @mirrored="refreshFiles"
          />
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
      </section>

      <div ref="threadEl" class="thread">
        <p v-if="thread.length === 0" class="thread-empty">{{ t('becalmed') }}</p>
        <BlipMessage
          v-for="node in visible"
          :key="node.entry.id"
          :entry="node.entry"
          :fragment="fragmentOf(node.entry.id)!"
          :doc="provider.doc"
          :me="me"
          :depth="node.depth"
          :role="roleOf(node.entry.author)"
          :translation="translationLang ? provider.translations.value[node.entry.id] : undefined"
          :replying="replyTarget?.id === node.entry.id"
          :highlight="highlightId === node.entry.id"
          @reply="startReply"
          @tag="onTag"
          @delete="onDelete"
          @profile="openProfile"
        />
      </div>

      <div class="composer-dock">
        <p v-if="typingNames.length" class="typing caption">
          {{ typingNames.join(', ') }} {{ t('isWriting') }}
        </p>
        <Composer
          :provider="provider"
          :me="me"
          :reply-to="replyTarget"
          @send="onSend"
          @cancel-reply="replyTarget = null"
        />
      </div>
    </main>

    <WavePanel
      :participants="digest?.participants ?? []"
      :owner="digest?.createdBy ?? ''"
      :online="online"
      :translation-enabled="digest?.translationEnabled ?? false"
      :translation-lang="translationLang"
      :languages="LANGUAGES"
      :tags="trending"
      @set-lang="setLang"
      @enable-translation="enableTranslation"
      @tag="onTag"
      @add-person="addOpen = true"
      @profile="openProfile"
    />

    <DialogRoot v-model:open="profileOpen">
      <DialogPortal>
        <DialogOverlay class="dialog-overlay" />
        <DialogContent class="dialog-content">
          <DialogTitle class="dialog-title">
            {{ profileData?.displayName ?? '…' }}
          </DialogTitle>
          <p v-if="profileData" class="addr">{{ profileData.participant }}</p>
          <p class="section-label caption" style="margin-top: 1rem">{{ t('sharedWaves') }}</p>
          <ul v-if="profileData?.sharedWaves.length" class="shared-list">
            <li v-for="w in profileData.sharedWaves" :key="w.wave">
              <button class="linkish" @click="goToWave(w.wave)">
                {{ w.title }}
              </button>
            </li>
          </ul>
          <p v-else class="empty caption">{{ t('none') }}</p>
        </DialogContent>
      </DialogPortal>
    </DialogRoot>

    <DialogRoot v-model:open="addOpen">
      <DialogPortal>
        <DialogOverlay class="dialog-overlay" />
        <DialogContent class="dialog-content">
          <DialogTitle class="dialog-title">{{ t('addToWave') }}</DialogTitle>
          <form @submit.prevent="addParticipant">
            <label class="field">
              <span class="field-label">{{ t('nameOrAddress') }}</span>
              <input v-model="addName" class="text-input" placeholder="ada  ·  bob@other.server" autofocus />
            </label>
            <p v-if="addError" class="error-note">{{ addError }}</p>
            <div class="dialog-actions">
              <DialogClose as-child>
                <button type="button" class="btn">{{ t('cancel') }}</button>
              </DialogClose>
              <button type="submit" class="btn btn-tide" :disabled="!addName.trim()">
                {{ t('add') }}
              </button>
            </div>
          </form>
        </DialogContent>
      </DialogPortal>
    </DialogRoot>
  </div>
</template>

<style scoped>
.app-shell {
  display: grid;
  grid-template-columns: 19rem 1fr 17rem;
  height: 100vh;
  overflow: hidden;
}

.conversation {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-width: 0;
}

.wave-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 1.1rem 1.6rem 0.6rem;
  border-bottom: 1px solid var(--mist);
}

.wave-title {
  font-size: 1.5rem;
}

.rule {
  width: 9rem;
  margin-top: 0.15rem;
}

.head-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.avatar-stack {
  display: flex;
  margin-right: 0.4rem;
}

.stack-avatar {
  width: 1.9rem;
  height: 1.9rem;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 0.72rem;
  border: 2px solid var(--cloud);
  margin-left: -0.5rem;
  cursor: pointer;
}

.stack-avatar:first-child {
  margin-left: 0;
}

.stack-avatar.overflow {
  background: var(--steel);
  cursor: default;
}

.shared-list {
  list-style: none;
  margin: 0.3rem 0 0;
  padding: 0;
}

.shared-list li {
  padding: 0.2rem 0;
}

.linkish {
  background: none;
  border: none;
  padding: 0;
  color: var(--deep);
  font-weight: 600;
  cursor: pointer;
}

.empty {
  color: var(--steel);
}

.btn.on {
  border-color: var(--deep);
  color: var(--deep);
}

.files {
  padding: 0.9rem 1.6rem;
  border-bottom: 1px solid var(--mist);
  background: color-mix(in srgb, var(--cloud) 50%, #fff);
}

.files-actions {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 0.7rem;
}

.files-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
  gap: 0.6rem;
}

.thread {
  flex: 1;
  overflow-y: auto;
  padding: 1rem 1.4rem;
  display: flex;
  flex-direction: column;
}

.thread-empty {
  margin: auto;
  color: var(--steel);
  font-style: italic;
}

.composer-dock {
  padding: 0.5rem 1.4rem 1.1rem;
  border-top: 1px solid var(--mist);
  background: color-mix(in srgb, var(--cloud) 40%, #fff);
}

.typing {
  margin: 0 0 0.4rem 0.6rem;
  color: var(--tide-deep);
  font-style: italic;
}

.dialog-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
  margin-top: 0.5rem;
}

@media (max-width: 60rem) {
  .app-shell {
    grid-template-columns: 1fr;
  }
  .app-shell > :first-child,
  .app-shell > :last-child {
    display: none;
  }
}
</style>
