// 入口：挂 Pinia + 全局样式。亮色（专业简洁·蓝）为默认；深色 = html.dark，由 config store applyDom 切换。
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import './style.css'

createApp(App).use(createPinia()).mount('#app')
