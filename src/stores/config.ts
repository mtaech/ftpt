import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { AppConfig } from '@/types'
import { loadConfig as loadCfg, saveConfig as saveCfg } from '@/types/tauri'

export const useConfigStore = defineStore('config', () => {
  const config = ref<AppConfig>({
    sidecarExtensions: ['xmp'],
    thumbnailSize: 220,
    favoriteDirs: [],
    lastDirectory: null,
    theme: 'Light',
    defaultDeleteMode: 'trash',
    importBehavior: 'copy',
    importDateFormat: 'year_month_day',
    overwriteStrategy: 'skip',
    windowWidth: 1400,
    windowHeight: 900,
    leftPanelWidth: 260,
    rightPanelVisible: true,
    thumbnailCacheDir: null,
    maxCacheSizeMb: 500,
  })

  async function load() {
    try { config.value = await loadCfg() } catch (e) { console.warn('Failed to load config:', e) }
  }

  async function save() {
    try { await saveCfg(config.value) } catch (e) { console.warn('Failed to save config:', e) }
  }

  return { config, load, save }
})
