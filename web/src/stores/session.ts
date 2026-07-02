import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api, ApiError } from '../lib/api'

export const useSession = defineStore('session', () => {
  const participant = ref<string | null>(null)
  const checked = ref(false)

  async function refresh() {
    try {
      participant.value = (await api.me()).participant
    } catch {
      participant.value = null
    } finally {
      checked.value = true
    }
  }

  async function login(name: string, password: string) {
    participant.value = (await api.login(name, password)).participant
  }

  async function register(name: string, password: string) {
    participant.value = (await api.register(name, password)).participant
  }

  async function logout() {
    try {
      await api.logout()
    } catch {
      // Session is gone either way.
    }
    participant.value = null
  }

  return { participant, checked, refresh, login, register, logout }
})

export { ApiError }
