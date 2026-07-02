<script setup lang="ts">
// Connection indicator: a buoy that bobs while online, sits still offshore
// while reconnecting.
import { computed } from 'vue'
import { socket } from '../lib/provider'

const label = computed(
  () =>
    ({
      online: 'in sync',
      connecting: 'hailing…',
      offline: 'adrift — edits kept locally',
    })[socket.status.value],
)
</script>

<template>
  <div class="buoy" :data-status="socket.status.value" :title="label">
    <span class="dot" />
    <span class="label">{{ label }}</span>
  </div>
</template>

<style scoped>
.buoy {
  position: fixed;
  right: 1.1rem;
  bottom: 1rem;
  z-index: 50;
  display: flex;
  align-items: center;
  gap: 0.45rem;
  font-family: var(--font-mono);
  font-size: 0.68rem;
  letter-spacing: 0.05em;
  color: var(--ink-soft);
  background: color-mix(in srgb, var(--paper) 85%, transparent);
  border: 1px solid var(--hairline-soft);
  border-radius: 999px;
  padding: 0.3rem 0.7rem;
  backdrop-filter: blur(4px);
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--ink-faint);
}

.buoy[data-status='online'] .dot {
  background: var(--crest);
  animation:
    bob 2.4s ease-in-out infinite,
    ripple 2.4s ease-out infinite;
}

.buoy[data-status='connecting'] .dot {
  background: var(--dusk);
  animation: bob 1s ease-in-out infinite;
}

.buoy[data-status='offline'] .dot {
  background: #f43f5e;
}
</style>
