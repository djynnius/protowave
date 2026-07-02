import { createRouter, createWebHistory } from 'vue-router'
import { useSession } from './stores/session'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', name: 'login', component: () => import('./views/LoginView.vue') },
    { path: '/', name: 'inbox', component: () => import('./views/InboxView.vue') },
    { path: '/w/:wave', name: 'wave', component: () => import('./views/WaveView.vue') },
  ],
})

router.beforeEach(async (to) => {
  const session = useSession()
  if (!session.checked) await session.refresh()
  if (!session.participant && to.name !== 'login') {
    return { name: 'login', query: { next: to.fullPath } }
  }
  if (session.participant && to.name === 'login') {
    return { name: 'inbox' }
  }
})

export default router
