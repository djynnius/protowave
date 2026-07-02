<script setup lang="ts">
// The ProtoWave mark, alive: "a triangulated wireframe crests like a wave,
// its vertices lighting up as crystalline nodes" (Brand v2). Two phased
// sine polylines joined by struts form the mesh; nodes pulse in sequence;
// the whole crest rolls gently. `variant="line"` is the slim divider used
// throughout the app; `variant="mark"` is the standalone logo crest.
import { computed } from 'vue'

const props = withDefaults(
  defineProps<{ variant?: 'line' | 'mark'; active?: boolean; width?: number }>(),
  { variant: 'line', active: false, width: 640 },
)

interface Node {
  x: number
  y: number
}

const H = computed(() => (props.variant === 'mark' ? 72 : 18))
const SEGS = computed(() => (props.variant === 'mark' ? 7 : Math.max(6, Math.round(props.width / 46))))

// Two offset crest lines: top runs a half-phase ahead of the bottom.
function crest(segments: number, height: number, phase: number, amp: number): Node[] {
  const nodes: Node[] = []
  for (let i = 0; i <= segments; i++) {
    const t = i / segments
    const y = height / 2 + Math.sin(t * Math.PI * 2 + phase) * amp
    nodes.push({ x: t * props.width, y })
  }
  return nodes
}

const top = computed(() => crest(SEGS.value, H.value, 0.6, H.value * 0.26))
const bottom = computed(() => crest(SEGS.value, H.value, 2.2, H.value * 0.3))

const points = (nodes: Node[]) => nodes.map((n) => `${n.x.toFixed(1)},${n.y.toFixed(1)}`).join(' ')

// Triangulation struts: top[i] → bottom[i] and top[i] → bottom[i+1].
const struts = computed(() => {
  const out: [Node, Node][] = []
  for (let i = 0; i <= SEGS.value; i++) {
    out.push([top.value[i], bottom.value[i]])
    if (i < SEGS.value) out.push([top.value[i], bottom.value[i + 1]])
  }
  return out
})

const nodes = computed(() => [...top.value, ...bottom.value])
</script>

<template>
  <div class="wave-mesh" :data-variant="variant" aria-hidden="true">
    <svg
      :viewBox="`0 0 ${width} ${H}`"
      :style="{ aspectRatio: `${width} / ${H}` }"
      preserveAspectRatio="xMidYMid meet"
    >
      <g class="crest" :class="{ active }">
        <line
          v-for="([a, b], i) in struts"
          :key="`s${i}`"
          class="strut"
          :x1="a.x"
          :y1="a.y"
          :x2="b.x"
          :y2="b.y"
        />
        <polyline class="wire top" :points="points(top)" />
        <polyline class="wire" :points="points(bottom)" />
        <circle
          v-for="(n, i) in nodes"
          :key="`n${i}`"
          class="node"
          :cx="n.x"
          :cy="n.y"
          :r="variant === 'mark' ? 2.4 : 1.6"
          :style="{ animationDelay: `${(i * 0.35) % 2.8}s` }"
        />
      </g>
    </svg>
  </div>
</template>

<style scoped>
.wave-mesh svg {
  display: block;
  width: 100%;
  overflow: visible;
}

.crest {
  animation: crest-roll 6s ease-in-out infinite;
}

.crest.active {
  animation-duration: 1.8s;
}

.wire {
  fill: none;
  stroke: var(--deep);
  stroke-width: 1.3;
  stroke-linecap: round;
  opacity: 0.75;
}

.wire.top {
  stroke: var(--spray);
}

.strut {
  stroke: var(--deep);
  stroke-width: 0.7;
  opacity: 0.28;
}

.node {
  fill: var(--crest);
  animation: node-glow 2.8s ease-in-out infinite;
}

.wave-mesh[data-variant='mark'] .node {
  fill: var(--deep);
}

@keyframes crest-roll {
  0%,
  100% {
    transform: translateY(0);
  }
  50% {
    transform: translateY(-6%);
  }
}

@keyframes node-glow {
  0%,
  100% {
    opacity: 0.35;
    fill: var(--deep);
  }
  50% {
    opacity: 1;
    fill: var(--crest);
  }
}

@media (prefers-reduced-motion: reduce) {
  .crest,
  .node {
    animation: none;
  }
}
</style>
