import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '../lib/api'

// Caches participant → display name so blips and rosters can show real names
// (when set) instead of handles, without re-fetching per render.
export const useProfiles = defineStore('profiles', () => {
  const names = ref<Record<string, string>>({})
  const inflight = new Set<string>()

  function handle(participant: string): string {
    return participant.split('@')[0] ?? participant
  }

  function ensure(participant: string) {
    if (!participant || names.value[participant] || inflight.has(participant)) return
    inflight.add(participant)
    api
      .userProfile(participant)
      .then((p) => {
        names.value = { ...names.value, [participant]: p.displayName || handle(participant) }
      })
      .catch(() => {})
      .finally(() => inflight.delete(participant))
  }

  function nameOf(participant: string): string {
    ensure(participant)
    return names.value[participant] ?? handle(participant)
  }

  return { names, ensure, nameOf }
})
