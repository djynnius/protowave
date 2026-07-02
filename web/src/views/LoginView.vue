<script setup lang="ts">
import { ref } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useSession } from '../stores/session'
import { ApiError } from '../lib/api'
import TideLine from '../components/TideLine.vue'

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
    error.value = e instanceof ApiError ? e.message : 'something went adrift — try again'
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <main class="shore">
    <div class="swell" aria-hidden="true">~</div>

    <section class="card reveal">
      <h1 class="wordmark">protowave</h1>
      <p class="tagline">a document that is a conversation</p>
      <TideLine class="rule" :active="busy" />

      <div class="mode-switch" role="tablist">
        <button
          role="tab"
          :aria-selected="mode === 'login'"
          :class="{ on: mode === 'login' }"
          @click="mode = 'login'"
        >
          sign in
        </button>
        <button
          role="tab"
          :aria-selected="mode === 'register'"
          :class="{ on: mode === 'register' }"
          @click="mode = 'register'"
        >
          come aboard
        </button>
      </div>

      <form @submit.prevent="submit">
        <label class="field">
          <span class="field-label">name</span>
          <input
            v-model.trim="name"
            class="text-input"
            autocomplete="username"
            spellcheck="false"
            placeholder="ada"
          />
        </label>
        <label class="field">
          <span class="field-label">passphrase</span>
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
          {{ mode === 'login' ? 'set sail' : 'register & set sail' }}
        </button>
      </form>
    </section>
  </main>
</template>

<style scoped>
.shore {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: 2rem 1rem;
  position: relative;
  overflow: hidden;
}

/* A giant, faint tilde looming behind the card — the wave itself. */
.swell {
  position: absolute;
  font-family: var(--font-display);
  font-weight: 900;
  font-size: min(70vw, 44rem);
  line-height: 0.6;
  color: transparent;
  -webkit-text-stroke: 1.5px rgba(14, 124, 107, 0.14);
  transform: rotate(-8deg) translateY(-4%);
  user-select: none;
  pointer-events: none;
}

.card {
  position: relative;
  width: min(23rem, 92vw);
  background: color-mix(in srgb, var(--paper) 65%, #fff);
  border: 1px solid var(--paper-edge);
  border-radius: 10px;
  box-shadow: var(--shadow-raise);
  padding: 2.2rem 2rem 2rem;
}

.wordmark {
  font-size: 2.6rem;
  font-weight: 900;
  letter-spacing: -0.02em;
}

.tagline {
  font-style: italic;
  color: var(--ink-soft);
  margin: 0.2rem 0 0.9rem;
}

.rule {
  margin-bottom: 1.4rem;
}

.mode-switch {
  display: flex;
  gap: 0.4rem;
  margin-bottom: 1.2rem;
}

.mode-switch button {
  font-family: var(--font-mono);
  font-size: 0.78rem;
  letter-spacing: 0.05em;
  color: var(--ink-soft);
  background: none;
  border: none;
  border-bottom: 2px solid transparent;
  padding: 0.25rem 0.1rem;
  cursor: pointer;
}

.mode-switch button.on {
  color: var(--tide-deep);
  border-bottom-color: var(--tide);
}

.submit {
  width: 100%;
  justify-content: center;
  margin-top: 0.4rem;
}
</style>
