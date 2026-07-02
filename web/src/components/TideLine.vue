<script setup lang="ts">
// The signature motif: a gently rolling tide-line. Doubles as an activity
// indicator — `active` quickens the swell.
import { computed } from 'vue'

const props = withDefaults(defineProps<{ active?: boolean; color?: string }>(), {
  active: false,
  color: 'var(--tide)',
})

// Two wavelengths of a smooth sine-ish path, tiled by the animation.
const path = computed(() => {
  let d = 'M0 6'
  for (let x = 0; x < 1240; x += 40) {
    d += ` C ${x + 10} 0.5, ${x + 30} 11.5, ${x + 40} 6`
  }
  return d
})
</script>

<template>
  <div class="tide-line" aria-hidden="true">
    <svg viewBox="0 0 620 12" preserveAspectRatio="none">
      <path
        :d="path"
        fill="none"
        :stroke="props.color"
        stroke-width="1.6"
        stroke-linecap="round"
        :class="{ active: props.active }"
      />
    </svg>
  </div>
</template>

<style scoped>
.tide-line {
  overflow: hidden;
  height: 12px;
}

svg {
  display: block;
  width: 200%;
  height: 12px;
}

path {
  animation: roll 7s linear infinite;
  opacity: 0.85;
}

path.active {
  animation-duration: 1.6s;
}

@keyframes roll {
  to {
    transform: translateX(-80px);
  }
}

@media (prefers-reduced-motion: reduce) {
  path {
    animation: none;
  }
}
</style>
