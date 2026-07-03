import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api, type WaveDigest } from '../lib/api'

export const useWaves = defineStore('waves', () => {
  const list = ref<WaveDigest[]>([])
  const loading = ref(false)
  // The wave currently open; it's always treated as read so a poll refresh
  // never lights an unread dot on the wave you're looking at.
  const activeWave = ref<string | null>(null)

  const unreadCount = computed(() => list.value.filter((w) => w.unread).length)

  async function refresh() {
    loading.value = true
    try {
      list.value = await api.listWaves()
      if (activeWave.value) clearUnread(activeWave.value)
    } finally {
      loading.value = false
    }
  }

  function setActive(wave: string | null) {
    activeWave.value = wave
    if (wave) clearUnread(wave)
  }

  async function create(title: string): Promise<WaveDigest> {
    const digest = await api.createWave(title)
    list.value = [digest, ...list.value]
    return digest
  }

  async function addParticipant(wave: string, participant: string) {
    const digest = await api.addParticipant(wave, participant)
    const i = list.value.findIndex((w) => w.wave === digest.wave)
    if (i >= 0) list.value[i] = digest
    return digest
  }

  async function setTranslation(wave: string, enabled: boolean) {
    const digest = await api.setTranslation(wave, enabled)
    const i = list.value.findIndex((w) => w.wave === digest.wave)
    if (i >= 0) list.value[i] = digest
    return digest
  }

  function byId(wave: string): WaveDigest | undefined {
    return list.value.find((w) => w.wave === wave)
  }

  function clearUnread(wave: string) {
    const digest = list.value.find((w) => w.wave === wave)
    if (digest) digest.unread = false
  }

  return {
    list,
    loading,
    unreadCount,
    activeWave,
    refresh,
    setActive,
    create,
    addParticipant,
    setTranslation,
    byId,
    clearUnread,
  }
})
