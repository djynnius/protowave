<script setup lang="ts">
import { ref } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useSession } from '../stores/session'
import { ApiError } from '../lib/api'
import WaveMesh from '../components/WaveMesh.vue'

const session = useSession()
const router = useRouter()
const route = useRoute()

const mode = ref<'login' | 'register'>('login')
const name = ref('')
const password = ref('')
const error = ref('')
const busy = ref(false)

async function submit() {
  if (!name.value || !password.value) return
  busy.value = true
  error.value = ''
  try {
    if (mode.value === 'login') {
      await session.login(name.value, password.value)
    } else {
      await session.register(name.value, password.value)
    }
    router.push((route.query.next as string) ?? '/')
  } catch (e) {
    error.value = e instanceof ApiError ? e.message : 'something went wrong — try again'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <main class="shore">
    <section class="card reveal">
      <WaveMesh variant="mark" class="mark" :active="busy" :width="320" />
      <h1 class="wordmark brand">Proto<em>Wave</em></h1>
      <p class="tagline">Collaboration without boundaries.</p>

      <div class="mode-switch" role="tablist">
        <button
          role="tab"
          :aria-selected="mode === 'login'"
          :class="{ on: mode === 'login' }"
          @click="mode = 'login'"
        >
          Sign in
        </button>
        <button
          role="tab"
          :aria-selected="mode === 'register'"
          :class="{ on: mode === 'register' }"
          @click="mode = 'register'"
        >
          Create account
        </button>
      </div>

      <form @submit.prevent="submit">
        <label class="field">
          <span class="field-label">Name</span>
          <input
            v-model.trim="name"
            class="text-input"
            autocomplete="username"
            spellcheck="false"
            placeholder="ada"
          />
        </label>
        <label class="field">
          <span class="field-label">Passphrase</span>
          <input
            v-model="password"
            class="text-input"
            type="password"
            :autocomplete="mode === 'login' ? 'current-password' : 'new-password'"
            placeholder="········"
          />
        </label>
        <p v-if="error" class="error-note">{{ error }}</p>
        <button class="btn btn-tide submit" :disabled="busy">
          {{ mode === 'login' ? 'Start a wave' : 'Create account & dive in' }}
        </button>
      </form>

      <p class="caption footer-caption">Apache-2.0 · Self-hostable · Federated</p>
    </section>
  </main>
</template>

<style scoped>
.shore {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 2rem 1rem;
}

.card {
  width: min(24rem, 92vw);
  background: #fff;
  border: 1px solid var(--mist);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow-raise);
  padding: 2.4rem 2.2rem 1.8rem;
  text-align: center;
}

.mark {
  width: 9.5rem;
  margin: 0 auto 1.1rem;
}

.brand {
  font-size: 2.1rem;
}

.tagline {
  color: var(--slate);
  margin: 0.35rem 0 1.4rem;
  font-weight: 500;
}

.mode-switch {
  display: inline-flex;
  gap: 0.25rem;
  background: var(--sky-t);
  border-radius: var(--radius-pill);
  padding: 0.25rem;
  margin-bottom: 1.4rem;
}

.mode-switch button {
  font-family: var(--font-body);
  font-size: 0.82rem;
  font-weight: 600;
  color: var(--slate);
  background: none;
  border: none;
  border-radius: var(--radius-pill);
  padding: 0.4rem 1rem;
  cursor: pointer;
  transition:
    background 0.15s ease,
    color 0.15s ease;
}

.mode-switch button.on {
  background: #fff;
  color: var(--deep);
  box-shadow: var(--shadow-card);
}

form {
  text-align: left;
}

.submit {
  width: 100%;
  justify-content: center;
  margin-top: 0.4rem;
  padding: 0.7rem 1rem;
  font-size: 0.92rem;
}

.footer-caption {
  margin-top: 1.6rem;
}
</style>
