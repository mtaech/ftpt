// 入口：挂 Pinia + 全局样式。亮色（专业简洁·蓝）为默认；深色 = html.dark，由 config store applyDom 切换。
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './style.css'
import { applyMaterialTheme, DEFAULT_ACCENT } from '@/lib/m3Theme'

// 首帧即应用默认 M3 主题（消除 JS 接线前的无色窗口；config store load 后会按持久化配置覆盖）
applyMaterialTheme(DEFAULT_ACCENT, false)

createApp(App).use(createPinia()).mount('#app')
