<script setup lang="ts">
import { computed } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { Image } from 'lucide-vue-next'

const browse = useBrowseStore()

const pathDisplay = computed(() => {
  return browse.currentPath || ''
})

const statusText = computed(() => {
  const parts: string[] = []
  if (browse.totalCount > 0) {
    parts.push(`共 ${browse.totalCount} 张拍摄`)
    if (browse.filteredCount < browse.totalCount) {
      parts.push(`筛选后 ${browse.filteredCount} 张`)
    }
  }
  return parts.join(' · ') || '就绪'
})

async function copyPath() {
  if (browse.currentPath) {
    try {
      await navigator.clipboard.writeText(browse.currentPath)
    } catch {}
  }
}
</script>

<template>
  <div class="statusbar">
    <span class="statusbar__path" :title="browse.currentPath" @click="copyPath">
      {{ pathDisplay || '未打开目录' }}
    </span>
    <span class="statusbar__selected" v-if="browse.selectedCount > 0">
      已选 {{ browse.selectedCount }}
    </span>
    <div class="statusbar__spacer" />
    <span class="statusbar__text">
      <Image :size="12" class="statusbar__icon" />
      {{ statusText }}
    </span>
  </div>
</template>

<style scoped>
.statusbar {
  height: var(--statusbar-height);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 16px;
  background: var(--bg-surface);
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}

.statusbar__path {
  font-size: 12px;
  color: var(--text-muted);
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 300px;
  transition: color var(--transition-fast);
}

.statusbar__path:hover {
  color: var(--text-secondary);
}

.statusbar__selected {
  font-size: 12px;
  font-weight: 500;
  color: var(--primary);
  padding: 2px 10px;
  background: var(--primary-subtle);
  border-radius: var(--radius-sm);
}

.statusbar__spacer {
  flex: 1;
}

.statusbar__text {
  font-size: 12px;
  color: var(--text-muted);
  display: flex;
  align-items: center;
  gap: 4px;
}

.statusbar__icon {
  flex-shrink: 0;
}
</style>
