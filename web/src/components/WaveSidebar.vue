<script setup lang="ts">
// Left column of the app shell: wordmark, global search, the waves list
// (most-recent first, unread marked), a new-wave action, and the account.
import { onMounted, ref, watch } from 'vue'
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

onMounted(() => waves.refresh())

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
        <p class="section-label caption">{{ t('theLog') }} · {{ t('mostRecent') }}</p>
        <p v-if="!waves.loading && waves.list.length === 0" class="empty">{{ t('becalmed') }}</p>
        <button
          v-for="w in waves.list"
          :key="w.wave"
          class="wave-item"
          :class="{ active: w.wave === props.current, unread: w.unread }"
          @click="open(w.wave)"
        >
          <span class="wave-title">
            <span v-if="w.unread" class="dot" />
            {{ w.title }}
          </span>
          <span class="wave-sub caption">
            {{ w.participants.map(localPart).join(' · ') }} · {{ relative(w.lastActivityMs) }}
          </span>
        </button>
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
          ><b>{{ localPart(session.participant ?? '') }}</b></span
        >
        <button class="btn ghost" @click="signOut">{{ t('signOut') }}</button>
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
  flex-direction: column;
  gap: 0.15rem;
  width: 100%;
  text-align: left;
  background: none;
  border: none;
  border-radius: 10px;
  padding: 0.55rem 0.6rem;
  margin-bottom: 0.15rem;
  cursor: pointer;
  transition: background 0.12s ease;
}

.wave-item:hover {
  background: var(--sky-t);
}

.wave-item.active {
  background: var(--sky-t);
  box-shadow: inset 3px 0 0 var(--deep);
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
}

.dialog-actions {
  display: flex;
  gap: 0.5rem;
  justify-content: flex-end;
  margin-top: 0.5rem;
}
</style>
