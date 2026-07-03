<script setup lang="ts">
// Left column of the app shell: wordmark, global search, the waves list
// (most-recent first, unread marked), a new-wave action, and the account.
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  DialogClose,
  DialogContent,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
  DialogTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from 'reka-ui'
import { useI18n } from 'vue-i18n'
import { useSession } from '../stores/session'
import { useWaves } from '../stores/waves'
import { api, type SearchHit } from '../lib/api'
import { localPart } from '../lib/wavemodel'
import WaveMesh from './WaveMesh.vue'

const props = defineProps<{ current: string }>()
const { t } = useI18n()
const session = useSession()
const waves = useWaves()
const router = useRouter()
const route = useRoute()

const query = ref('')

// A tag chip elsewhere navigates with ?q=#tag — reflect it in the search.
watch(
  () => route.query.q,
  (q) => {
    if (typeof q === 'string' && q) query.value = q
  },
  { immediate: true },
)
const hits = ref<SearchHit[] | null>(null)
const dialogOpen = ref(false)
const newTitle = ref('')
let timer: ReturnType<typeof setTimeout> | null = null

// Keep unread indicators live: poll the inbox, and refresh on tab focus.
let poll: ReturnType<typeof setInterval> | null = null
function onVisible() {
  if (document.visibilityState === 'visible') waves.refresh()
}
onMounted(() => {
  waves.refresh()
  poll = setInterval(() => {
    if (document.visibilityState === 'visible') waves.refresh()
  }, 15000)
  document.addEventListener('visibilitychange', onVisible)
})
onBeforeUnmount(() => {
  if (poll) clearInterval(poll)
  document.removeEventListener('visibilitychange', onVisible)
})

watch(query, (q) => {
  if (timer) clearTimeout(timer)
  if (!q.trim()) {
    hits.value = null
    return
  }
  timer = setTimeout(async () => {
    hits.value = await api.search(q).catch(() => [])
  }, 250)
})

function open(wave: string) {
  router.push({ name: 'wave', params: { wave: encodeURIComponent(wave) } })
}

async function createWave() {
  const title = newTitle.value.trim()
  if (!title) return
  const digest = await waves.create(title)
  dialogOpen.value = false
  newTitle.value = ''
  open(digest.wave)
}

async function archiveWave(wave: string) {
  await waves.archive(wave).catch(() => {})
  if (wave === props.current) router.push({ name: 'inbox' })
}
async function deleteWave(wave: string) {
  if (!window.confirm(t('deleteWaveConfirm'))) return
  await waves.remove(wave).catch(() => {})
  if (wave === props.current) router.push({ name: 'inbox' })
}

// >9 unread shows as "9+"; a wave flagged unread but with no countable new
// blips (activity without a posted blip) still shows a single dot.
function unreadLabel(count: number): string {
  return count > 9 ? '9+' : String(count)
}

async function signOut() {
  await session.logout()
  router.push({ name: 'login' })
}

function relative(ms: number): string {
  const d = Date.now() - ms
  if (d < 60_000) return t('justNow')
  if (d < 3_600_000) return t('minutesAgo', { n: Math.floor(d / 60_000) })
  if (d < 86_400_000) return t('hoursAgo', { n: Math.floor(d / 3_600_000) })
  return t('daysAgo', { n: Math.floor(d / 86_400_000) })
}
</script>

<template>
  <aside class="sidebar">
    <header class="brand-row">
      <WaveMesh variant="mark" class="mark" :width="120" />
      <h1 class="wordmark">Proto<em>Wave</em></h1>
    </header>

    <input
      v-model="query"
      class="text-input search"
      type="search"
      :placeholder="t('searchPlaceholder')"
      aria-label="search"
    />

    <div class="scroll">
      <template v-if="hits !== null">
        <p class="section-label caption">{{ t('resultsFor', { q: query }) }}</p>
        <p v-if="hits.length === 0" class="empty">{{ t('noResults') }}</p>
        <button v-for="h in hits" :key="h.wave" class="wave-item" @click="open(h.wave)">
          <span class="wave-title">{{ h.title }}</span>
          <!-- eslint-disable-next-line vue/no-v-html -->
          <span class="wave-sub snippet" v-html="h.snippet" />
        </button>
      </template>
      <template v-else>
        <p class="section-label caption">
          {{ t('theLog') }} · {{ t('mostRecent') }}
          <span v-if="waves.unreadCount" class="unread-badge">{{ waves.unreadCount }}</span>
        </p>
        <p v-if="!waves.loading && waves.list.length === 0" class="empty">{{ t('becalmed') }}</p>
        <div
          v-for="w in waves.list"
          :key="w.wave"
          class="wave-item"
          :class="{ active: w.wave === props.current, unread: w.unread }"
        >
          <button class="wave-open" @click="open(w.wave)">
            <span class="wave-title">
              <span v-if="w.unread && !w.unreadCount" class="dot" />
              {{ w.title }}
            </span>
            <span class="wave-sub caption">
              {{ w.participants.map(localPart).join(' · ') }} · {{ relative(w.lastActivityMs) }}
            </span>
          </button>
          <span v-if="w.unread && w.unreadCount" class="count-badge">{{
            unreadLabel(w.unreadCount)
          }}</span>
          <DropdownMenuRoot>
            <DropdownMenuTrigger class="kebab" :aria-label="t('waveMenu')" @click.stop>⋯</DropdownMenuTrigger>
            <DropdownMenuPortal>
              <DropdownMenuContent class="menu" align="end" :side-offset="4">
                <DropdownMenuItem class="menu-item" @select="archiveWave(w.wave)">
                  ⌸ {{ t('archive') }}
                </DropdownMenuItem>
                <DropdownMenuItem class="menu-item danger" @select="deleteWave(w.wave)">
                  🗑 {{ t('delete') }}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuPortal>
          </DropdownMenuRoot>
        </div>
      </template>
    </div>

    <div class="foot">
      <DialogRoot v-model:open="dialogOpen">
        <DialogTrigger as-child>
          <button class="btn btn-tide new-wave">{{ t('newWave') }}</button>
        </DialogTrigger>
        <DialogPortal>
          <DialogOverlay class="dialog-overlay" />
          <DialogContent class="dialog-content">
            <DialogTitle class="dialog-title">{{ t('startAWave') }}</DialogTitle>
            <form @submit.prevent="createWave">
              <label class="field">
                <span class="field-label">{{ t('title') }}</span>
                <input v-model="newTitle" class="text-input" :placeholder="t('titlePlaceholder')" autofocus />
              </label>
              <div class="dialog-actions">
                <DialogClose as-child>
                  <button type="button" class="btn">{{ t('cancel') }}</button>
                </DialogClose>
                <button type="submit" class="btn btn-tide" :disabled="!newTitle.trim()">
                  {{ t('launch') }}
                </button>
              </div>
            </form>
          </DialogContent>
        </DialogPortal>
      </DialogRoot>

      <div class="account">
        <span class="addr"
          ><b>{{ session.displayName }}</b></span
        >
        <div class="account-actions">
          <button class="icon-btn" :title="t('settings')" @click="router.push({ name: 'settings' })">
            ⚙
          </button>
          <button class="btn ghost" @click="signOut">{{ t('signOut') }}</button>
        </div>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  display: flex;
  flex-direction: column;
  height: 100vh;
  padding: 1.1rem 0.9rem;
  border-right: 1px solid var(--mist);
  background: color-mix(in srgb, var(--cloud) 60%, #fff);
}

.brand-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.9rem;
}

.mark {
  width: 2.1rem;
}

.wordmark {
  font-size: 1.25rem;
}

.search {
  margin-bottom: 0.9rem;
}

.scroll {
  flex: 1;
  overflow-y: auto;
  margin: 0 -0.3rem;
  padding: 0 0.3rem;
}

.section-label {
  margin: 0.3rem 0 0.5rem;
}

.empty {
  font-size: 0.82rem;
  color: var(--steel);
  padding: 0.5rem 0.2rem;
}

.wave-item {
  display: flex;
  align-items: center;
  gap: 0.2rem;
  width: 100%;
  border-radius: 10px;
  padding: 0.15rem 0.2rem 0.15rem 0.4rem;
  margin-bottom: 0.15rem;
  transition: background 0.12s ease;
}

.wave-item:hover {
  background: var(--sky-t);
}

.wave-item.active {
  background: var(--sky-t);
  box-shadow: inset 3px 0 0 var(--deep);
}

.wave-open {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  text-align: left;
  background: none;
  border: none;
  padding: 0.4rem 0.4rem;
  cursor: pointer;
}

.count-badge {
  flex: none;
  min-width: 1.25rem;
  height: 1.25rem;
  padding: 0 0.3rem;
  border-radius: 999px;
  background: var(--crest);
  color: #06302b;
  font-size: 0.66rem;
  font-weight: 800;
  display: grid;
  place-items: center;
}

.kebab {
  flex: none;
  width: 1.5rem;
  height: 1.5rem;
  border-radius: 6px;
  border: none;
  background: none;
  color: var(--steel);
  font-size: 1.1rem;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.wave-item:hover .kebab,
.kebab[data-state='open'] {
  opacity: 1;
}

.kebab:hover {
  background: color-mix(in srgb, var(--deep) 12%, transparent);
  color: var(--deep);
}

.menu {
  min-width: 9rem;
  background: #fff;
  border: 1px solid var(--mist);
  border-radius: 10px;
  box-shadow: var(--shadow-card);
  padding: 0.3rem;
  z-index: 40;
}

.menu-item {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.4rem 0.5rem;
  border-radius: 6px;
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--ink);
  cursor: pointer;
  outline: none;
}

.menu-item[data-highlighted] {
  background: var(--sky-t);
}

.menu-item.danger {
  color: #d33;
}

.menu-item.danger[data-highlighted] {
  background: color-mix(in srgb, #d33 12%, #fff);
}

.wave-title {
  font-weight: 600;
  font-size: 0.92rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wave-sub {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--steel);
}

.snippet :deep(b) {
  color: var(--deep);
}

.dot {
  display: inline-block;
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--crest);
  margin-right: 0.3rem;
  vertical-align: 0.1em;
  animation: ripple 2.4s ease-out infinite;
}

.wave-item.unread .wave-title {
  font-weight: 700;
}

.unread-badge {
  display: inline-block;
  min-width: 1.1rem;
  padding: 0 0.35rem;
  margin-left: 0.35rem;
  border-radius: 999px;
  background: var(--deep);
  color: #fff;
  font-family: var(--font-body);
  font-size: 0.66rem;
  font-weight: 700;
  text-align: center;
  vertical-align: 0.05em;
}

.foot {
  padding-top: 0.7rem;
  border-top: 1px solid var(--mist);
}

.new-wave {
  width: 100%;
  justify-content: center;
  margin-bottom: 0.6rem;
}

.account {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.4rem;
}

.account .addr {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account-actions {
  display: flex;
  align-items: center;
  gap: 0.3rem;
  flex: none;
}

.icon-btn {
  width: 1.9rem;
  height: 1.9rem;
  border-radius: 8px;
  border: 1px solid var(--mist);
  background: #fff;
  color: var(--deep);
  font-size: 1rem;
  line-height: 1;
  cursor: pointer;
  transition:
    background 0.12s ease,
    border-color 0.12s ease;
}

.icon-btn:hover {
  background: var(--sky-t);
  border-color: var(--deep);
}

.dialog-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
  margin-top: 0.5rem;
}
</style>
