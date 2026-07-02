<script setup lang="ts">
import { useSession } from './stores/session'
import BuoyStatus from './components/BuoyStatus.vue'

const session = useSession()
</script>

<template>
  <router-view />
  <BuoyStatus v-if="session.participant" />
</template>
