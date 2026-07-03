<script setup lang="ts">
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
import { useSession } from '../stores/session'
import { useWaves } from '../stores/waves'
import { api, type SearchHit } from '../lib/api'
import { localPart } from '../lib/wavemodel'
import WaveMesh from '../components/WaveMesh.vue'
import { useI18n } from 'vue-i18n'
import { LOCALES, setLocale } from '../i18n'

const { t, locale } = useI18n()
const session = useSession()
const waves = useWaves()
const router = useRouter()
const route = useRoute()

const newTitle = ref('')
const dialogOpen = ref(false)
const creating = ref(false)
const query = ref('')
const hits = ref<SearchHit[] | null>(null)
const searching = ref(false)
let searchTimer: ReturnType<typeof setTimeout> | null = null

onMounted(() => {
  waves.refresh()
  if (typeof route.query.q === 'string') query.value = route.query.q
})

watch(query, (q) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q.trim()) {
    hits.value = null
    return
  }
  searchTimer = setTimeout(async () => {
    searching.value = true
    try {
      hits.value = await api.search(q)
    } finally {
      searching.value = false
    }
  }, 250)
})

async function createWave() {
  const title = newTitle.value.trim()
  if (!title) return
  creating.value = true
  try {
    const digest = await waves.create(title)
    dialogOpen.value = false
    newTitle.value = ''
    router.push({ name: 'wave', params: { wave: encodeURIComponent(digest.wave) } })
  } finally {
    creating.value = false
  }
}

function openWave(wave: string) {
  router.push({ name: 'wave', params: { wave: encodeURIComponent(wave) } })
}

async function signOut() {
  await session.logout()
  router.push({ name: 'login' })
}

function relative(ms: number): string {
  const delta = Date.now() - ms
  if (delta < 60_000) return t('justNow')
  if (delta < 3_600_000) return t('minutesAgo', { n: Math.floor(delta / 60_000) })
  if (delta < 86_400_000) return t('hoursAgo', { n: Math.floor(delta / 3_600_000) })
  return t('daysAgo', { n: Math.floor(delta / 86_400_000) })
}
</script>

<template>
  <main class="harbor">
    <header class="masthead reveal">
      <div>
        <h1 class="wordmark">Proto<em>Wave</em></h1>
        <WaveMesh class="rule" :active="searching" :width="240" />
      </div>
      <div class="helm">
        <span class="addr"
          ><b>{{ localPart(session.participant ?? '') }}</b
          >@{{ (session.participant ?? '').split('@')[1] }}</span
        >
        <select
          class="locale-select mono"
          :value="locale"
          aria-label="language"
          @change="setLocale(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="[code, label] in LOCALES" :key="code" :value="code">{{ label }}</option>
        </select>
        <button class="btn" @click="signOut">{{ t('signOut') }}</button>
      </div>
    </header>

    <div class="soundings reveal" style="animation-delay: 0.03s">
      <input
        v-model="query"
        class="text-input"
        type="search"
        :placeholder="t('searchPlaceholder')"
        aria-label="search waves"
      />
    </div>

    <section v-if="hits !== null" class="log">
      <div class="log-head">
        <h2>{{ t('resultsFor', { q: query }) }}</h2>
        <button class="btn" @click="query = ''">{{ t('clear') }}</button>
      </div>
      <p v-if="hits.length === 0" class="becalmed">{{ t('noResults') }}</p>
      <ol class="wave-list">
        <li
          v-for="(hit, i) in hits"
          :key="hit.wave"
          class="wave-card reveal"
          :style="{ animationDelay: `${i * 0.04}s` }"
          @click="openWave(hit.wave)"
        >
          <div class="wave-main">
            <h3 class="wave-title">{{ hit.title }}</h3>
            <!-- eslint-disable-next-line vue/no-v-html — tantivy snippet, <b> tags only -->
            <p class="wave-crew snippet" v-html="hit.snippet" />
          </div>
        </li>
      </ol>
    </section>

    <section v-else class="log">
      <div class="log-head reveal" style="animation-delay: 0.05s">
        <h2>{{ t('theLog') }}</h2>

        <DialogRoot v-model:open="dialogOpen">
          <DialogTrigger as-child>
            <button class="btn btn-tide">{{ t('newWave') }}</button>
          </DialogTrigger>
          <DialogPortal>
            <DialogOverlay class="dialog-overlay" />
            <DialogContent class="dialog-content">
              <DialogTitle class="dialog-title">{{ t('startAWave') }}</DialogTitle>
              <form @submit.prevent="createWave">
                <label class="field">
                  <span class="field-label">{{ t('title') }}</span>
                  <input
                    v-model="newTitle"
                    class="text-input"
                    :placeholder="t('titlePlaceholder')"
                    autofocus
                  />
                </label>
                <div class="dialog-actions">
                  <DialogClose as-child>
                    <button type="button" class="btn">{{ t('cancel') }}</button>
                  </DialogClose>
                  <button type="submit" class="btn btn-tide" :disabled="creating || !newTitle.trim()">
                    {{ t('launch') }}
                  </button>
                </div>
              </form>
            </DialogContent>
          </DialogPortal>
        </DialogRoot>
      </div>

      <p v-if="!waves.loading && waves.list.length === 0" class="becalmed reveal">
        {{ t('becalmed') }}
      </p>

      <ol class="wave-list">
        <li
          v-for="(w, i) in waves.list"
          :key="w.wave"
          class="wave-card reveal"
          :class="{ unread: w.unread }"
          :style="{ animationDelay: `${0.08 + i * 0.045}s` }"
          @click="openWave(w.wave)"
        >
          <div class="wave-main">
            <h3 class="wave-title">
              <span v-if="w.unread" class="unread-buoy" title="new activity" />
              {{ w.title }}
            </h3>
            <p class="wave-crew mono">
              {{ w.participants.map(localPart).join(' · ') }}
            </p>
          </div>
          <time class="wave-time mono">{{ relative(w.lastActivityMs) }}</time>
        </li>
      </ol>
    </section>
  </main>
</template>

<style scoped>
.harbor {
  max-width: 46rem;
  margin: 0 auto;
  padding: 2.5rem 1.25rem 4rem;
}

.masthead {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1.6rem;
}

.wordmark {
  font-size: 2rem;
  font-weight: 900;
  letter-spacing: -0.02em;
}

.rule {
  width: 9.5rem;
  margin-top: 0.2rem;
}

.locale-select {
  font-size: 0.74rem;
  padding: 0.4rem 0.5rem;
  border: 1px solid var(--mist);
  border-radius: 999px;
  background: #fff;
  color: var(--slate);
  cursor: pointer;
}

.helm {
  display: flex;
  align-items: center;
  gap: 0.9rem;
}

.soundings {
  margin-bottom: 2rem;
}

.soundings input {
  width: 100%;
  font-size: 1rem;
  padding: 0.65rem 0.9rem;
}

.log-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 1.1rem;
}

.log-head h2 {
  font-size: 1.15rem;
  font-style: italic;
  font-weight: 400;
  color: var(--ink-soft);
}

.becalmed {
  font-style: italic;
  color: var(--ink-faint);
  border: 1px dashed var(--hairline);
  border-radius: 8px;
  padding: 2rem;
  text-align: center;
}

.wave-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.wave-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  background: #fff;
  border: 1px solid var(--paper-edge);
  border-left: 3px solid var(--deep);
  border-radius: 14px;
  padding: 0.9rem 1.1rem;
  cursor: pointer;
  transition:
    transform 0.14s ease,
    box-shadow 0.14s ease,
    border-color 0.14s ease;
}

.wave-card:hover {
  transform: translateX(3px);
  box-shadow: var(--shadow-card);
  border-left-color: var(--coral);
}

.wave-card.unread {
  border-left-color: var(--coral);
  background: color-mix(in srgb, var(--coral-wash) 40%, #fff);
}

.unread-buoy {
  display: inline-block;
  width: 0.55rem;
  height: 0.55rem;
  border-radius: 50%;
  background: var(--coral);
  margin-right: 0.4rem;
  vertical-align: 0.12em;
  animation: ripple 2.4s ease-out infinite;
}

.wave-title {
  font-size: 1.18rem;
  font-weight: 600;
}

.wave-crew {
  margin: 0.15rem 0 0;
  font-size: 0.72rem;
  letter-spacing: 0.04em;
  color: var(--ink-soft);
}

.snippet {
  font-family: var(--font-body);
  font-size: 0.85rem;
  font-style: italic;
}

.snippet :deep(b) {
  color: var(--tide-deep);
  font-style: normal;
}

.wave-time {
  font-size: 0.7rem;
  color: var(--ink-faint);
  white-space: nowrap;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.5rem;
  margin-top: 0.5rem;
}
</style>
