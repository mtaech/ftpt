import { createRouter, createWebHashHistory } from 'vue-router'
import BrowseView from '@/views/BrowseView.vue'
import CompareView from '@/views/CompareView.vue'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/browse' },
    { path: '/browse', name: 'browse', component: BrowseView },
    { path: '/compare/:left/:right', name: 'compare', component: CompareView },
  ],
})

export default router
