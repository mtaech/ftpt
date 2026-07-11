<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue'
import type { CaptureMeta } from '@/types'
import { getThumbnail } from '@/types/tauri'
import { Image } from 'lucide-vue-next'

const props = defineProps<{
  capture: CaptureMeta
  isSelected: boolean
  isFocused: boolean
  size: number
}>()

const emit = defineEmits<{ click: []; contextmenu: [MouseEvent] }>()

const thumbUrl = ref('')
const loading = ref(true)
const blobUrls = new Set<string>()

watch(() => props.capture.primaryPath, async (path) => {
  if (!path) return
  loading.value = true
  const key = `${path}@${props.size}`
  try {
    const bytes = await getThumbnail(path, props.size)
    const blob = new Blob([new Uint8Array(bytes)], { type: 'image/jpeg' })
    const url = URL.createObjectURL(blob)
    blobUrls.add(url)
    thumbUrl.value = url
  } catch { thumbUrl.value = '' }
  loading.value = false
}, { immediate: true })

onUnmounted(() => {
  blobUrls.forEach(u => URL.revokeObjectURL(u))
  blobUrls.clear()
})
</script>

<template>
  <div
    class="cell"
    :class="{ 'cell--selected': isSelected, 'cell--focused': isFocused }"
    @click="emit('click')"
    @contextmenu="emit('contextmenu', $event)"
  >
    <div class="cell__thumb">
      <img v-if="thumbUrl && !loading" :src="thumbUrl" class="cell__img" loading="lazy" />
      <div v-else class="cell__placeholder">
        <Image :size="32" />
      </div>
      <span v-if="capture.stackCount > 0" class="cell__badge">{{ capture.stackCount }}</span>
      <span class="cell__format">{{ capture.primaryFormat }}</span>
      <div v-if="loading" class="cell__loading" />
    </div>
    <div class="cell__label">{{ capture.baseName }}</div>
  </div>
</template>

<style scoped>
.cell {
  border-radius: var(--radius-md);
  overflow: hidden;
  cursor: pointer;
  transition: all var(--transition-fast);
  user-select: none;
  border: 1px solid var(--border-light);
  will-change: transform;
}

.cell:hover {
  box-shadow: var(--shadow-md);
  transform: translateY(-1px);
  border-color: transparent;
}

.cell--selected {
  box-shadow: 0 0 0 2px var(--bg-surface), 0 0 0 4px var(--primary);
  border-color: transparent;
}

.cell--focused:not(.cell--selected) {
  outline: 2px solid var(--primary);
  outline-offset: 2px;
}

.cell__thumb {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  aspect-ratio: 1 / 1;
  background: var(--bg-page);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.cell__img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform var(--transition-normal);
}

.cell:hover .cell__img {
  transform: scale(1.03);
}

.cell__placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  color: var(--border);
}

.cell__badge {
  position: absolute;
  bottom: 4px;
  right: 4px;
  background: rgba(239, 68, 68, 0.9);
  color: white;
  border-radius: 50%;
  width: 20px;
  height: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-body);
  font-size: 10px;
  font-weight: 700;
  line-height: 1;
  backdrop-filter: blur(4px);
}

.cell__format {
  position: absolute;
  top: 4px;
  right: 4px;
  background: var(--bg-page);
  color: var(--text-secondary);
  border: 1px solid var(--border);
  border-radius: var(--radius-xs);
  padding: 1px 5px;
  font-size: 9px;
  font-weight: 600;
  letter-spacing: 0.03em;
}

.cell__loading {
  position: absolute;
  inset: 0;
  background: linear-gradient(110deg, transparent 30%, rgba(255,255,255,0.15) 50%, transparent 70%);
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

.cell__label {
  font-size: 12px;
  color: var(--text-muted);
  text-align: center;
  padding: 4px 6px 2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
