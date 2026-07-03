import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api, ApiError } from '../lib/api'

export const useSession = defineStore('session', () => {
  const participant = ref<string | null>(null)
  const firstName = ref('')
  const lastName = ref('')
  const isOwner = ref(false)
  const checked = ref(false)

  // Full name when a profile is set, otherwise the bare handle — the label
  // shown wherever the signed-in user sees themselves.
  const displayName = computed(() => {
    const full = `${firstName.value} ${lastName.value}`.trim()
    return full || (participant.value?.split('@')[0] ?? '')
  })

  function applyMe(me: {
    participant: string
    firstName?: string
    lastName?: string
    isOwner?: boolean
  }) {
    participant.value = me.participant
    firstName.value = me.firstName ?? ''
    lastName.value = me.lastName ?? ''
    isOwner.value = me.isOwner ?? false
  }

  async function refresh() {
    try {
      applyMe(await api.me())
    } catch {
      participant.value = null
    } finally {
      checked.value = true
    }
  }

  async function login(name: string, password: string) {
    await api.login(name, password)
    await refresh()
  }

  async function register(name: string, password: string) {
    await api.register(name, password)
    await refresh()
  }

  async function saveProfile(first: string, last: string) {
    const r = await api.setProfile(first, last)
    firstName.value = r.firstName
    lastName.value = r.lastName
  }

  async function logout() {
    try {
      await api.logout()
    } catch {
      // Session is gone either way.
    }
    participant.value = null
    firstName.value = ''
    lastName.value = ''
    isOwner.value = false
  }

  return {
    participant,
    firstName,
    lastName,
    isOwner,
    displayName,
    checked,
    refresh,
    login,
    register,
    saveProfile,
    logout,
  }
})

export { ApiError }
