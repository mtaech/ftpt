import { ref, onUnmounted } from 'vue'
import { getThumbnail } from '@/types/tauri'

const cache = new Map<string, string>()

export function useThumbnail() {
  const url = ref<string>('')

  onUnmounted(() => {
    if (url.value && url.value.startsWith('blob:')) {
      URL.revokeObjectURL(url.value)
    }
  })

  async function load(path: string, size: number) {
    const key = `${path}@${size}`
    if (cache.has(key)) {
      url.value = cache.get(key)!
      return
    }
    try {
      const bytes = await getThumbnail(path, size)
      const blob = new Blob([new Uint8Array(bytes)], { type: 'image/jpeg' })
      const blobUrl = URL.createObjectURL(blob)
      cache.set(key, blobUrl)
      url.value = blobUrl
    } catch {
      url.value = ''
    }
  }

  return { url, load }
}
