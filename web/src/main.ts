import '@fontsource/archivo/500.css'
import '@fontsource/archivo/700.css'
import '@fontsource/archivo/900.css'
import '@fontsource/hanken-grotesk/400.css'
import '@fontsource/hanken-grotesk/500.css'
import '@fontsource/hanken-grotesk/600.css'
import '@fontsource/hanken-grotesk/700.css'
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/jetbrains-mono/500.css'
import './style.css'

import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import { i18n } from './i18n'

createApp(App).use(createPinia()).use(router).use(i18n).mount('#app')

// Offline shell (PRD §10): the CRDT layer handles document state; the
// service worker handles the app shell.
if ('serviceWorker' in navigator && !import.meta.env.DEV) {
  navigator.serviceWorker.register('/sw.js').catch(() => {})
}
