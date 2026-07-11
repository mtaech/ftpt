<script setup lang="ts">
import { ref, watch } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { getThumbnail, getExif } from '@/types/tauri'
import type { ExifMetadata } from '@/types'
import { Minus, Plus, Image, ArrowRight } from 'lucide-vue-next'
import ExifTable from './ExifTable.vue'

const browse = useBrowseStore()

const thumbUrl = ref('')
const exif = ref<ExifMetadata | null>(null)
const loading = ref(false)

watch(() => [...browse.selectedIndices], async () => {
  if (browse.selectedCaptures.length === 0) { thumbUrl.value = ''; exif.value = null; return }
  const cap = browse.selectedCaptures[0]
  loading.value = true
  try {
    const bytes = await getThumbnail(cap.primaryPath, 1200)
    thumbUrl.value = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: 'image/jpeg' }))
  } catch { thumbUrl.value = '' }
  try { exif.value = await getExif(cap.primaryPath) } catch { exif.value = null }
  loading.value = false
}, { immediate: true })
</script>

<template>
  <div class="panel">
    <template v-if="browse.selectedCaptures.length === 0">
      <div class="panel__empty">
        <ArrowRight :size="40" />
        <div class="panel__empty-title">查看拍摄详情</div>
        <div class="panel__empty-hint">在网格中选中一张照片<br />这里会显示预览与 EXIF 信息</div>
      </div>
    </template>
    <template v-else>
      <div class="panel__header">
        <div class="panel__title">{{ browse.selectedCaptures[0].baseName }}</div>
      </div>
      <div class="panel__controls">
        <button class="ctrl-btn" @click="browse.setZoom(-0.25)" :disabled="browse.zoomLevel <= 0.25">
          <Minus :size="12" />
        </button>
        <span class="ctrl-pct">{{ Math.round(browse.zoomLevel * 100) }}%</span>
        <button class="ctrl-btn" @click="browse.setZoom(0.25)" :disabled="browse.zoomLevel >= 5">
          <Plus :size="12" />
        </button>
        <button class="ctrl-btn" :class="{ active: browse.fitToWindow }" @click="browse.toggleFitToWindow()">适应</button>
      </div>
      <div class="panel__image" :class="{ 'panel__image--zoom': !browse.fitToWindow }">
        <img
          v-if="thumbUrl && !loading"
          :src="thumbUrl"
          class="panel__img"
          :class="{ 'panel__img--fit': browse.fitToWindow }"
          :style="browse.fitToWindow ? null : { transform: `scale(${browse.zoomLevel})`, transformOrigin: 'top left' }"
        />
        <div v-else class="panel__image-placeholder">
          <Image :size="24" />
          <span v-if="loading">加载中…</span>
        </div>
      </div>
      <div class="panel__exif">
        <ExifTable v-if="exif" :exif="exif" />
        <div v-else class="panel__exif-empty">
          <span v-if="loading">读取 EXIF…</span>
          <span v-else>无 EXIF 信息</span>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 0;
}

.panel__empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 100%;
  padding: 24px;
  text-align: center;
  color: var(--text-muted);
}

.panel__empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-top: 8px;
}

.panel__empty-hint {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.6;
}

.panel__header {
  padding: 14px 16px 8px;
  border-bottom: 1px solid var(--border-light);
}

.panel__title {
  font-family: var(--font-heading);
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.panel__controls {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-light);
}

.ctrl-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
  font-size: 12px;
}

.ctrl-btn:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.ctrl-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.ctrl-btn.active { background: var(--primary); color: white; border-color: var(--primary); }

.ctrl-pct {
  font-family: var(--font-body);
  font-size: 12px;
  font-weight: 500;
  width: 44px;
  text-align: center;
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.panel__image {
  display: flex;
  justify-content: center;
  align-items: center;
  padding: 16px;
  flex: 1 1 auto;
  min-height: 280px;
  border-bottom: 1px solid var(--border-light);
  background: var(--bg-page);
  overflow: auto;
}

.panel__image--zoom {
  justify-content: flex-start;
  align-items: flex-start;
}

.panel__img {
  display: block;
  flex-shrink: 0;
  border-radius: var(--radius-md);
}

.panel__img--fit {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
  border: 1px solid var(--border);
}

.panel__image-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  color: var(--text-muted);
  font-size: 12px;
}

.panel__exif {
  flex: 0 0 auto;
  max-height: 42vh;
  overflow-y: auto;
  padding: 14px 16px;
}

.panel__exif-empty {
  color: var(--text-muted);
  font-size: 12px;
  padding: 8px 0;
}
</style>
