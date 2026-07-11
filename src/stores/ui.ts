import { defineStore } from 'pinia'
import { ref } from 'vue'

export type AppMode = 'browse' | 'compare' | 'import' | 'rename' | 'settings' | 'convert'

export const useUiStore = defineStore('ui', () => {
  const mode = ref<AppMode>('browse')
  const rightPanelVisible = ref(true)
  const leftPanelWidth = ref(260)
  const contextMenu = ref<{ index: number; x: number; y: number } | null>(null)
  const compareIndices = ref<[number, number]>([0, 0])

  // Dialog visibility
  const importOpen = ref(false)
  const renameOpen = ref(false)
  const settingsOpen = ref(false)
  const convertOpen = ref(false)

  function openContextMenu(index: number, x: number, y: number) {
    contextMenu.value = { index, x, y }
  }

  function closeContextMenu() {
    contextMenu.value = null
  }

  function enterCompare(left: number, right: number) {
    compareIndices.value = [left, right]
    mode.value = 'compare'
  }

  function exitCompare() {
    mode.value = 'browse'
  }

  function toggleRightPanel() {
    rightPanelVisible.value = !rightPanelVisible.value
  }

  function openImport() { importOpen.value = true; mode.value = 'import' }
  function closeImport() { importOpen.value = false; mode.value = 'browse' }
  function openRename() { renameOpen.value = true; mode.value = 'rename' }
  function closeRename() { renameOpen.value = false; mode.value = 'browse' }
  function openSettings() { settingsOpen.value = true; mode.value = 'settings' }
  function closeSettings() { settingsOpen.value = false; mode.value = 'browse' }
  function openConvert() { convertOpen.value = true; mode.value = 'convert' }
  function closeConvert() { convertOpen.value = false; mode.value = 'browse' }

  return {
    mode, rightPanelVisible, leftPanelWidth,
    contextMenu, compareIndices,
    importOpen, renameOpen, settingsOpen, convertOpen,
    openContextMenu, closeContextMenu,
    enterCompare, exitCompare, toggleRightPanel,
    openImport, closeImport, openRename, closeRename,
    openSettings, closeSettings, openConvert, closeConvert,
  }
})
