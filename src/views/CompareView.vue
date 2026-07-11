<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useBrowseStore } from '@/stores/browse'
import { getThumbnail } from '@/types/tauri'
import { Minus, Plus, X, Image } from 'lucide-vue-next'

const route = useRoute()
const router = useRouter()
const browse = useBrowseStore()

const leftUrl = ref('')
const rightUrl = ref('')
const leftLabel = ref('')
const rightLabel = ref('')
const zoomLevel = ref(1.0)
const fitToWindow = ref(true)
const blobUrls: string[] = []

watch(() => [route.params.left, route.params.right], async ([l, r]) => {
  leftUrl.value = ''; rightUrl.value = ''
  const leftIdx = Number(l); const rightIdx = Number(r)
  const leftCap = browse.filteredCaptures[leftIdx]
  const rightCap = browse.filteredCaptures[rightIdx]
  if (!leftCap || !rightCap) { leftLabel.value = '-'; rightLabel.value = '-'; return }
  leftLabel.value = leftCap.baseName; rightLabel.value = rightCap.baseName
  try {
    const [lb, rb] = await Promise.all([
      getThumbnail(leftCap.primaryPath, 1600),
      getThumbnail(rightCap.primaryPath, 1600),
    ])
    const lUrl = URL.createObjectURL(new Blob([new Uint8Array(lb)], { type: 'image/jpeg' }))
    const rUrl = URL.createObjectURL(new Blob([new Uint8Array(rb)], { type: 'image/jpeg' }))
    blobUrls.push(lUrl, rUrl)
    leftUrl.value = lUrl; rightUrl.value = rUrl
  } catch {}
}, { immediate: true })

onUnmounted(() => blobUrls.forEach(u => URL.revokeObjectURL(u)))

function setZoom(delta: number) {
  zoomLevel.value = Math.max(0.25, Math.min(5, zoomLevel.value + delta))
  if (Math.abs(zoomLevel.value - 1) < 0.01) fitToWindow.value = true
  else fitToWindow.value = false
}

function toggleFit() {
  fitToWindow.value = !fitToWindow.value
  if (fitToWindow.value) zoomLevel.value = 1
}

function exit() { router.push('/browse') }
</script>

<template>
  <div class="compare">
    <div class="compare__images">
      <div class="compare__panel">
        <div class="compare__img">
          <img v-if="leftUrl" :src="leftUrl" :style="fitToWindow ? { maxWidth:'100%', maxHeight:'100%', objectFit:'contain' } : { transform:`scale(${zoomLevel})`, transformOrigin:'top left' }" />
        </div>
        <div class="compare__label">
          <Image :size="12" />
          {{ leftLabel }}
        </div>
      </div>
      <div class="compare__divider" />
      <div class="compare__panel">
        <div class="compare__img">
          <img v-if="rightUrl" :src="rightUrl" :style="fitToWindow ? { maxWidth:'100%', maxHeight:'100%', objectFit:'contain' } : { transform:`scale(${zoomLevel})`, transformOrigin:'top left' }" />
        </div>
        <div class="compare__label">
          <Image :size="12" />
          {{ rightLabel }}
        </div>
      </div>
    </div>
    <div class="compare__toolbar">
      <span class="compare__title">对比模式</span>
      <div class="compare__divider-v" />
      <div class="btn-group">
        <button class="ctrl-btn btn-group__first" @click="setZoom(-0.25)" :disabled="zoomLevel <= 0.25">
          <Minus :size="12" />
        </button>
        <span class="ctrl-pct">{{ Math.round(zoomLevel * 100) }}%</span>
        <button class="ctrl-btn btn-group__last" @click="setZoom(0.25)" :disabled="zoomLevel >= 5">
          <Plus :size="12" />
        </button>
      </div>
      <button class="ctrl-btn" :class="{ active: fitToWindow }" @click="toggleFit">适应窗口</button>
      <span class="spacer" />
      <button class="exit-btn" @click="exit">
        <X :size="16" />
        退出对比
      </button>
    </div>
  </div>
</template>

<style scoped>
.compare { display: flex; flex-direction: column; height: 100%; background: var(--bg-page); }
.compare__images { flex: 1; display: flex; gap: 0; overflow: hidden; }
.compare__panel { flex: 1; display: flex; flex-direction: column; align-items: center; padding: 12px; min-width: 0; }
.compare__img { flex: 1; display: flex; align-items: center; justify-content: center; overflow: auto; width: 100%; background: var(--bg-page); border: 1px solid var(--border); border-radius: var(--radius-md); box-shadow: var(--shadow-sm); }
.compare__img img { display: block; border-radius: 4px; }
.compare__divider { width: 1px; background: var(--border); margin: 12px 0; }
.compare__label { font-family: var(--font-heading); font-size: 13px; font-weight: 500; color: var(--text-muted); padding: 8px 0 0; text-align: center; display: flex; align-items: center; gap: 4px; }
.compare__toolbar { display: flex; align-items: center; gap: 6px; padding: 8px 16px; background: var(--bg-surface); border-top: 1px solid var(--border); }
.compare__title { font-family: var(--font-heading); font-size: 13px; font-weight: 600; color: var(--text-muted); }
.compare__divider-v { width: 1px; height: 20px; background: var(--border); flex-shrink: 0; }
.spacer { flex: 1; }

.btn-group {
  display: flex;
  align-items: center;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.btn-group__first {
  border: none !important;
  border-radius: 0 !important;
}

.btn-group__last {
  border: none !important;
  border-radius: 0 !important;
  border-left: 1px solid var(--border) !important;
}

.ctrl-btn { display: flex; align-items: center; justify-content: center; width: 28px; height: 28px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); font-size: 12px; }
.ctrl-btn:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
.ctrl-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.ctrl-btn.active { background: var(--primary); color: white; border-color: var(--primary); }
.ctrl-pct { font-size: 12px; font-weight: 500; width: 44px; text-align: center; color: var(--text-secondary); }
.exit-btn { display: flex; align-items: center; gap: 6px; font-family: var(--font-body); font-size: 13px; font-weight: 500; padding: 6px 14px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--danger); cursor: pointer; transition: all var(--transition-fast); }
.exit-btn:hover { background: var(--danger-subtle); border-color: var(--danger); }
</style>
