<script setup lang="ts">
// Right column: who's here, the reader's language, and the wave's trending
// tags. Presentational — the parent owns the state and handles the events.
import { useI18n } from 'vue-i18n'
import { localPart, participantColor, isAgent } from '../lib/wavemodel'

const { t } = useI18n()

defineProps<{
  participants: string[]
  owner: string
  online: Set<string>
  translationEnabled: boolean
  translationLang: string
  languages: [string, string][]
  tags: string[]
}>()

const emit = defineEmits<{
  setLang: [code: string]
  enableTranslation: []
  tag: [tag: string]
}>()
</script>

<template>
  <aside class="panel">
    <section>
      <p class="section-label caption">{{ t('participants') }}</p>
      <ul class="people">
        <li v-for="p in participants" :key="p" class="person">
          <span
            class="avatar"
            :class="{ 'avatar-agent': isAgent(p) }"
            :style="isAgent(p) ? undefined : { background: participantColor(p) }"
          >
            {{ isAgent(p) ? '✳' : localPart(p).slice(0, 1).toUpperCase() }}
            <span v-if="online.has(localPart(p))" class="presence" />
          </span>
          <span class="pname">
            {{ localPart(p) }}
            <span class="prole caption">{{ p === owner ? 'owner' : 'editor' }}</span>
          </span>
        </li>
      </ul>
    </section>

    <section>
      <p class="section-label caption">{{ t('translation') }}</p>
      <select
        v-if="translationEnabled"
        class="text-input lang"
        :value="translationLang"
        @change="emit('setLang', ($event.target as HTMLSelectElement).value)"
      >
        <option value="">{{ t('original') }}</option>
        <option v-for="[code, label] in languages" :key="code" :value="code">≈ {{ label }}</option>
      </select>
      <button v-else class="btn enable" @click="emit('enableTranslation')">
        {{ t('enableForWave') }}
      </button>
    </section>

    <section v-if="tags.length">
      <p class="section-label caption">{{ t('trending') }}</p>
      <div class="tags">
        <button v-for="tag in tags" :key="tag" class="tag-chip" @click="emit('tag', tag)">
          #{{ tag }}
        </button>
      </div>
    </section>
  </aside>
</template>

<style scoped>
.panel {
  height: 100vh;
  overflow-y: auto;
  padding: 1.4rem 1.1rem;
  border-left: 1px solid var(--mist);
  background: color-mix(in srgb, var(--cloud) 60%, #fff);
  display: flex;
  flex-direction: column;
  gap: 1.6rem;
}

.section-label {
  margin-bottom: 0.7rem;
}

.people {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.person {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.avatar {
  position: relative;
  flex: none;
  width: 2rem;
  height: 2rem;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: #fff;
  font-family: var(--font-display);
  font-weight: 700;
  font-size: 0.78rem;
}

.avatar-agent {
  background: linear-gradient(135deg, var(--crest), var(--deep));
}

.presence {
  position: absolute;
  right: -1px;
  bottom: -1px;
  width: 0.6rem;
  height: 0.6rem;
  border-radius: 50%;
  background: #22c55e;
  border: 2px solid #fff;
}

.pname {
  display: flex;
  flex-direction: column;
  font-weight: 600;
  font-size: 0.88rem;
}

.prole {
  font-weight: 500;
}

.lang {
  width: 100%;
  cursor: pointer;
}

.enable {
  width: 100%;
  justify-content: center;
}

.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 0.4rem;
}

.tag-chip {
  font-family: var(--font-mono);
  font-size: 0.72rem;
  font-weight: 500;
  color: var(--deep);
  background: var(--sky-t);
  border: none;
  border-radius: 999px;
  padding: 0.25rem 0.6rem;
  cursor: pointer;
}

.tag-chip:hover {
  background: color-mix(in srgb, var(--deep) 16%, var(--sky-t));
}
</style>
