import { onMounted, onUnmounted } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'

export function useKeyboard() {
  const browse = useBrowseStore()
  const ui = useUiStore()

  function handler(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey

    switch (e.key) {
      case 'ArrowUp':
      case 'ArrowLeft':
        e.preventDefault()
        browse.focusPrev()
        break
      case 'ArrowDown':
      case 'ArrowRight':
        e.preventDefault()
        browse.focusNext()
        break
      case 'Escape':
        browse.clearSelection()
        ui.closeContextMenu()
        break
      case 'Delete':
        e.preventDefault()
        // Delete handled by button invocation
        break
      case 'a':
      case 'A':
        if (ctrl) { e.preventDefault(); browse.selectAll() }
        break
      case 'i':
      case 'I':
        if (ctrl) { e.preventDefault(); browse.invertSelection() }
        break
    }
  }

  onMounted(() => window.addEventListener('keydown', handler))
  onUnmounted(() => window.removeEventListener('keydown', handler))
}
