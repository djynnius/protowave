<script setup lang="ts">
// Landing at "/": the app shell with the sidebar, but no wave selected yet.
// If the user has waves, jump to the most recent; otherwise show a prompt to
// start one (the sidebar's + new wave).
import { onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { useWaves } from '../stores/waves'
import WaveSidebar from '../components/WaveSidebar.vue'
import WaveMesh from '../components/WaveMesh.vue'

const { t } = useI18n()
const router = useRouter()
const waves = useWaves()

onMounted(async () => {
  await waves.refresh()
  if (waves.list.length > 0) {
    router.replace({ name: 'wave', params: { wave: encodeURIComponent(waves.list[0].wave) } })
  }
})
</script>

<template>
  <div class="landing">
    <WaveSidebar current="" />
    <main class="empty">
      <WaveMesh variant="mark" :width="200" class="mark" />
      <h1 class="wordmark">Proto<em>Wave</em></h1>
      <p class="prompt">{{ t('pickAWave') }}</p>
    </main>
  </div>
</template>

<style scoped>
.landing {
  display: grid;
  grid-template-columns: 19rem 1fr;
  height: 100vh;
  overflow: hidden;
}

.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  text-align: center;
}

.mark {
  width: 9rem;
}

.wordmark {
  font-size: 2rem;
}

.prompt {
  color: var(--slate);
  font-size: 1.05rem;
}

@media (max-width: 60rem) {
  .landing {
    grid-template-columns: 1fr;
  }
  .empty {
    display: none;
  }
}
</style>
