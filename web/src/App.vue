<script setup lang="ts">
// Phase 0 shell: connect, authenticate, exercise the echo channel.
// Replaced by the real inbox/wave UI in Phase 1.
import { ref } from 'vue'
import { useConnection } from './lib/connection'

const conn = useConnection()
const participant = ref('ada@example.org')
const token = ref('dev')
const message = ref('')

function send() {
  if (message.value.trim()) {
    conn.sendEcho(message.value)
    message.value = ''
  }
}
</script>

<template>
  <main class="shell">
    <h1>ProtoWave <span class="phase">Phase 0</span></h1>

    <section class="card">
      <h2>Session</h2>
      <label>
        Participant
        <input v-model="participant" placeholder="user@domain" />
      </label>
      <label>
        Token
        <input v-model="token" />
      </label>
      <div class="row">
        <button
          :disabled="conn.status.value === 'connecting'"
          @click="conn.connect(participant, token)"
        >
          Connect
        </button>
        <button :disabled="conn.status.value === 'disconnected'" @click="conn.disconnect()">
          Disconnect
        </button>
        <span class="status" :data-status="conn.status.value">{{ conn.status.value }}</span>
      </div>
      <p v-if="conn.sessionId.value" class="session">session {{ conn.sessionId.value }}</p>
      <p v-if="conn.lastError.value" class="error">{{ conn.lastError.value }}</p>
    </section>

    <section class="card">
      <h2>Echo channel</h2>
      <form class="row" @submit.prevent="send">
        <input
          v-model="message"
          :disabled="conn.status.value !== 'authenticated'"
          placeholder="say something…"
        />
        <button :disabled="conn.status.value !== 'authenticated'">Send</button>
      </form>
      <ul>
        <li v-for="(echo, i) in conn.echoes.value" :key="i">{{ echo }}</li>
      </ul>
    </section>
  </main>
</template>
