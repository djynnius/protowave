<script setup lang="ts">
// App settings: everyone can edit their profile (first/last name, which
// replaces their handle once set); the server owner additionally configures
// the hosted inference backend — architecture (Ollama / Gemini) then model.
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSession } from '../stores/session'
import { api } from '../lib/api'
import WaveSidebar from '../components/WaveSidebar.vue'

const { t } = useI18n()
const session = useSession()

// ---- profile (everyone) ----
const first = ref(session.firstName)
const last = ref(session.lastName)
const profileSaving = ref(false)
const profileSaved = ref(false)

async function saveProfile() {
  profileSaving.value = true
  profileSaved.value = false
  try {
    await session.saveProfile(first.value.trim(), last.value.trim())
    profileSaved.value = true
  } finally {
    profileSaving.value = false
  }
}

// ---- hosted model (owner only) ----
const provider = ref('ollama')
const base = ref('')
const model = ref('')
const activeModel = ref('')
const geminiKeyPresent = ref(false)
const modelSaving = ref(false)
const modelSaved = ref(false)
const loaded = ref(false)

onMounted(async () => {
  first.value = session.firstName
  last.value = session.lastName
  if (!session.isOwner) return
  try {
    const s = await api.getSettings()
    provider.value = s.inferenceProvider || 'ollama'
    base.value = s.inferenceBase || 'http://localhost:11434'
    model.value = s.inferenceModel || ''
    activeModel.value = s.activeModel || ''
    geminiKeyPresent.value = s.geminiKeyPresent
  } finally {
    loaded.value = true
  }
})

async function saveModel() {
  modelSaving.value = true
  modelSaved.value = false
  try {
    const r = await api.putSettings(provider.value, base.value.trim(), model.value.trim())
    activeModel.value = r.activeModel
    modelSaved.value = true
  } finally {
    modelSaving.value = false
  }
}
</script>

<template>
  <div class="settings-shell">
    <WaveSidebar current="" />
    <main class="content">
      <header class="head">
        <RouterLink class="back" :to="{ name: 'inbox' }">{{ t('back') }}</RouterLink>
        <h1 class="title">{{ t('settings') }}</h1>
      </header>

      <div class="cards">
        <section class="card">
          <h2 class="card-title">{{ t('yourProfile') }}</h2>
          <p class="hint">{{ t('profileHint') }}</p>
          <form class="form" @submit.prevent="saveProfile">
            <label class="field">
              <span class="field-label">{{ t('firstName') }}</span>
              <input v-model="first" class="text-input" autocomplete="given-name" />
            </label>
            <label class="field">
              <span class="field-label">{{ t('lastName') }}</span>
              <input v-model="last" class="text-input" autocomplete="family-name" />
            </label>
            <div class="row">
              <button type="submit" class="btn btn-tide" :disabled="profileSaving">
                {{ profileSaving ? t('saving') : t('save') }}
              </button>
              <span v-if="profileSaved" class="saved caption">✓ {{ t('saved') }}</span>
            </div>
          </form>
        </section>

        <section class="card">
          <h2 class="card-title">{{ t('hostedModel') }}</h2>
          <template v-if="session.isOwner">
            <p class="hint">{{ t('modelHint') }}</p>
            <form class="form" @submit.prevent="saveModel">
              <label class="field">
                <span class="field-label">{{ t('modelArchitecture') }}</span>
                <select v-model="provider" class="text-input">
                  <option value="ollama">Ollama (self-hosted)</option>
                  <option value="gemini">Gemini</option>
                </select>
              </label>

              <label v-if="provider === 'ollama'" class="field">
                <span class="field-label">{{ t('ollamaUrl') }}</span>
                <input v-model="base" class="text-input" placeholder="http://localhost:11434" />
              </label>

              <p v-if="provider === 'gemini'" class="key-status caption" :class="{ ok: geminiKeyPresent }">
                {{ geminiKeyPresent ? t('geminiKeyPresent') : t('geminiKeyMissing') }}
              </p>

              <label class="field">
                <span class="field-label">{{ t('modelName') }}</span>
                <input
                  v-model="model"
                  class="text-input"
                  :placeholder="provider === 'ollama' ? 'llama3.2' : 'gemini-2.0-flash-lite'"
                />
              </label>

              <div class="row">
                <button type="submit" class="btn btn-tide" :disabled="modelSaving">
                  {{ modelSaving ? t('saving') : t('save') }}
                </button>
                <span v-if="modelSaved" class="saved caption">✓ {{ t('saved') }}</span>
              </div>
            </form>
            <p class="active caption">
              {{ t('activeModel') }}: <b>{{ activeModel || t('noModel') }}</b>
            </p>
          </template>
          <p v-else class="hint locked">🔒 {{ t('ownerOnly') }}</p>
        </section>
      </div>
    </main>
  </div>
</template>

<style scoped>
.settings-shell {
  display: grid;
  grid-template-columns: 19rem 1fr;
  height: 100vh;
  overflow: hidden;
}

.content {
  overflow-y: auto;
  padding: 1.6rem 2rem;
}

.head {
  display: flex;
  align-items: baseline;
  gap: 1rem;
  margin-bottom: 1.4rem;
}

.back {
  color: var(--deep);
  font-weight: 600;
  text-decoration: none;
}

.back:hover {
  text-decoration: underline;
}

.title {
  font-size: 1.8rem;
}

.cards {
  display: flex;
  flex-direction: column;
  gap: 1.2rem;
  max-width: 34rem;
}

.card {
  border: 1px solid var(--mist);
  border-radius: 14px;
  background: color-mix(in srgb, var(--cloud) 45%, #fff);
  padding: 1.2rem 1.3rem;
}

.card-title {
  font-size: 1.1rem;
  margin-bottom: 0.3rem;
}

.hint {
  color: var(--steel);
  font-size: 0.85rem;
  margin-bottom: 1rem;
}

.hint.locked {
  margin-bottom: 0;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 0.9rem;
}

.row {
  display: flex;
  align-items: center;
  gap: 0.8rem;
}

.saved {
  color: var(--crest-deep, #0f766e);
  font-weight: 600;
}

.key-status {
  color: #b45309;
}

.key-status.ok {
  color: #0f766e;
}

.active {
  margin-top: 1rem;
  color: var(--steel);
}

@media (max-width: 60rem) {
  .settings-shell {
    grid-template-columns: 1fr;
  }
}
</style>
