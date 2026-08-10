// 入口：挂 Pinia + 全局样式。深色（Catppuccin Mocha）为默认，html.dark 见 index.html。
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './style.css'

createApp(App).use(createPinia()).mount('#app')
