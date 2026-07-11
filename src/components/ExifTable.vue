<script setup lang="ts">
import { computed } from 'vue'
import type { ExifMetadata } from '@/types'

const props = defineProps<{ exif: ExifMetadata }>()

function fmtSize(bytes: number | null): string {
  if (!bytes) return ''
  if (bytes > 1024 * 1024 * 1024) return `${(bytes / (1024*1024*1024)).toFixed(1)} GB`
  if (bytes > 1024 * 1024) return `${(bytes / (1024*1024)).toFixed(1)} MB`
  if (bytes > 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${bytes} B`
}

interface Entry { label: string; value: string }

const entries = computed<Entry[]>(() => {
  const e = props.exif
  const list: Entry[] = []
  function add(l: string, v: string | null | undefined) { if (v) list.push({ label: l, value: v }) }
  add('相机', e.camera.make ? [e.camera.make, e.camera.model].filter(Boolean).join(' ') : null)
  add('镜头', e.camera.lens)
  add('拍摄时间', e.dateTimeOriginal)
  add('快门', e.shooting.exposureTime)
  add('光圈', e.shooting.fNumber)
  add('ISO', e.shooting.iso?.toString())
  add('焦距', e.shooting.focalLength)
  add('曝光补偿', e.shooting.exposureCompensation)
  add('白平衡', e.shooting.whiteBalance)
  if (e.imageWidth && e.imageHeight) add('分辨率', `${e.imageWidth}×${e.imageHeight}`)
  add('色彩空间', e.colorSpace)
  add('文件大小', fmtSize(e.fileSize))
  return list
})
</script>

<template>
  <div class="exif">
    <div v-for="entry in entries" :key="entry.label" class="exif__entry">
      <span class="exif__label">{{ entry.label }}</span>
      <span class="exif__value">{{ entry.value }}</span>
    </div>
    <div v-if="entries.length === 0" class="exif__empty">无 EXIF 数据</div>
  </div>
</template>

<style scoped>
.exif {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px 16px;
}

.exif__entry {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.exif__label {
  font-size: 11px;
  font-weight: 600;
 color: var(--text-muted);
  letter-spacing: 0.02em;
}

.exif__value {
  font-size: 13px;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.exif__empty {
  font-size: 12px;
  color: var(--text-muted);
  padding: 8px 0;
  grid-column: 1 / -1;
}
</style>
