import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api, type WaveDigest } from '../lib/api'

export const useWaves = defineStore('waves', () => {
  const list = ref<WaveDigest[]>([])
  const loading = ref(false)

  async function refresh() {
    loading.value = true
    try {
      list.value = await api.listWaves()
    } finally {
      loading.value = false
    }
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

  function byId(wave: string): WaveDigest | undefined {
    return list.value.find((w) => w.wave === wave)
  }

  function clearUnread(wave: string) {
    const digest = list.value.find((w) => w.wave === wave)
    if (digest) digest.unread = false
  }

  return { list, loading, refresh, create, addParticipant, byId, clearUnread }
})
