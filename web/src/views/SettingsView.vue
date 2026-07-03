<script setup lang="ts">
// App settings: everyone can edit their profile (first/last name, which
// replaces their handle once set); the server owner additionally configures
// the hosted inference backend — architecture (Ollama / Gemini) then model.
import { onMounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useSession } from '../stores/session'
import { api, type PoolModel } from '../lib/api'
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
  loadModels()
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

// ---- password (everyone) ----
const pw = reactive({ current: '', next: '', confirm: '' })
const pwBusy = ref(false)
const pwMsg = ref<{ ok: boolean; text: string } | null>(null)

async function changePassword() {
  pwMsg.value = null
  if (pw.next.length < 8) {
    pwMsg.value = { ok: false, text: t('passwordTooShort') }
    return
  }
  if (pw.next !== pw.confirm) {
    pwMsg.value = { ok: false, text: t('passwordMismatch') }
    return
  }
  pwBusy.value = true
  try {
    await api.changePassword(pw.current, pw.next)
    pw.current = ''
    pw.next = ''
    pw.confirm = ''
    pwMsg.value = { ok: true, text: t('passwordChanged') }
  } catch (e) {
    pwMsg.value = { ok: false, text: e instanceof Error ? e.message : t('genericError') }
  } finally {
    pwBusy.value = false
  }
}

// ---- inference pool (everyone) ----
const myModels = ref<PoolModel[]>([])
const poolModels = ref<PoolModel[]>([])
const draft = reactive({ label: '', base: 'http://localhost:11434', model: '', scope: 'wave' })
const poolBusy = ref(false)
const testing = ref(false)
const testResult = ref<{ ok: boolean; text: string } | null>(null)

async function loadModels() {
  const r = await api.listModels().catch(() => ({ mine: [], pool: [] }))
  myModels.value = r.mine
  poolModels.value = r.pool
}

async function testDraft() {
  if (!draft.base.trim() || !draft.model.trim()) return
  testing.value = true
  testResult.value = null
  try {
    const r = await api.testModel(draft.base.trim(), draft.model.trim())
    testResult.value = { ok: r.ok, text: r.ok ? t('reachable') : r.error || t('unreachable') }
  } finally {
    testing.value = false
  }
}

async function addModel() {
  if (!draft.label.trim() || !draft.base.trim() || !draft.model.trim()) return
  poolBusy.value = true
  try {
    await api.putModel({
      label: draft.label.trim(),
      base: draft.base.trim(),
      model: draft.model.trim(),
      scope: draft.scope,
    })
    draft.label = ''
    draft.model = ''
    testResult.value = null
    await loadModels()
  } finally {
    poolBusy.value = false
  }
}

async function removeModel(id: string) {
  await api.deleteModel(id).catch(() => {})
  await loadModels()
}

function scopeLabel(scope: string): string {
  return scope === 'private'
    ? t('scopePrivate')
    : scope === 'federation'
      ? t('scopeFederation')
      : t('scopeWave')
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
          <h2 class="card-title">{{ t('security') }}</h2>
          <p class="hint">{{ t('securityHint') }}</p>
          <form class="form" @submit.prevent="changePassword">
            <label class="field">
              <span class="field-label">{{ t('currentPassword') }}</span>
              <input
                v-model="pw.current"
                type="password"
                class="text-input"
                autocomplete="current-password"
              />
            </label>
            <label class="field">
              <span class="field-label">{{ t('newPassword') }}</span>
              <input
                v-model="pw.next"
                type="password"
                class="text-input"
                autocomplete="new-password"
              />
            </label>
            <label class="field">
              <span class="field-label">{{ t('confirmPassword') }}</span>
              <input
                v-model="pw.confirm"
                type="password"
                class="text-input"
                autocomplete="new-password"
              />
            </label>
            <div class="row">
              <button
                type="submit"
                class="btn btn-tide"
                :disabled="pwBusy || !pw.current || !pw.next"
              >
                {{ pwBusy ? t('saving') : t('updatePassword') }}
              </button>
              <span v-if="pwMsg" class="caption" :class="pwMsg.ok ? 'ok-text' : 'bad-text'">
                {{ pwMsg.ok ? '✓' : '✕' }} {{ pwMsg.text }}
              </span>
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

        <section class="card">
          <h2 class="card-title">{{ t('yourModels') }}</h2>
          <p class="hint">{{ t('poolHint') }}</p>

          <ul v-if="myModels.length" class="model-list">
            <li v-for="m in myModels" :key="m.id" class="model-row">
              <span class="model-dot" :class="{ off: !m.enabled }" />
              <span class="model-main">
                <b>{{ m.label || m.model }}</b>
                <span class="model-meta caption">{{ m.model }} · {{ scopeLabel(m.scope) }}</span>
              </span>
              <button class="link-danger" @click="removeModel(m.id)">{{ t('remove') }}</button>
            </li>
          </ul>
          <p v-else class="hint">{{ t('noModelsYet') }}</p>

          <form class="form add-model" @submit.prevent="addModel">
            <p class="field-label">{{ t('addModel') }}</p>
            <div class="grid2">
              <input v-model="draft.label" class="text-input" :placeholder="t('modelLabel')" />
              <select v-model="draft.scope" class="text-input">
                <option value="private">{{ t('scopePrivate') }}</option>
                <option value="wave">{{ t('scopeWave') }}</option>
                <option value="federation">{{ t('scopeFederation') }}</option>
              </select>
              <input v-model="draft.base" class="text-input" :placeholder="t('ollamaUrl')" />
              <input v-model="draft.model" class="text-input" placeholder="gemma3:270m" />
            </div>
            <div class="row">
              <button type="submit" class="btn btn-tide" :disabled="poolBusy">
                {{ t('add') }}
              </button>
              <button type="button" class="btn" :disabled="testing" @click="testDraft">
                {{ testing ? t('testing') : t('test') }}
              </button>
              <span v-if="testResult" class="caption" :class="testResult.ok ? 'ok-text' : 'bad-text'">
                {{ testResult.ok ? '✓' : '✕' }} {{ testResult.text }}
              </span>
            </div>
          </form>

          <div v-if="poolModels.length" class="pool">
            <p class="field-label">{{ t('sharedPool') }}</p>
            <ul class="model-list">
              <li v-for="m in poolModels" :key="m.id" class="model-row">
                <span class="model-dot" />
                <span class="model-main">
                  <b>{{ m.label || m.model }}</b>
                  <span class="model-meta caption">
                    {{ m.model }} · {{ t('hostedBy') }} @{{ m.ownerName }}
                  </span>
                </span>
              </li>
            </ul>
          </div>
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

.model-list {
  list-style: none;
  margin: 0 0 1rem;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.model-row {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.model-dot {
  flex: none;
  width: 0.55rem;
  height: 0.55rem;
  border-radius: 50%;
  background: var(--crest);
}

.model-dot.off {
  background: var(--mist);
}

.model-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.model-meta {
  color: var(--steel);
}

.link-danger {
  background: none;
  border: none;
  color: #d33;
  font-weight: 600;
  font-size: 0.8rem;
  cursor: pointer;
  flex: none;
}

.add-model {
  border-top: 1px solid var(--mist);
  padding-top: 1rem;
}

.grid2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.6rem;
}

.pool {
  border-top: 1px solid var(--mist);
  padding-top: 1rem;
  margin-top: 1rem;
}

.ok-text {
  color: #0f766e;
  font-weight: 600;
}

.bad-text {
  color: #b45309;
  font-weight: 600;
}

@media (max-width: 40rem) {
  .grid2 {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 60rem) {
  .settings-shell {
    grid-template-columns: 1fr;
  }
}
</style>
