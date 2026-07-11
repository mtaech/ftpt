<script setup lang="ts">
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { openFolderDialog } from '@/types/tauri'
import { Images, FolderPlus } from 'lucide-vue-next'
import ThumbnailCell from './ThumbnailCell.vue'

const browse = useBrowseStore()
const ui = useUiStore()

async function openFolder() {
  const dir = await openFolderDialog('选择照片目录')
  if (dir) browse.openDirectory(dir)
}

function handleClick(captureIdx: number) {
  browse.selectCapture(captureIdx)
}

function handleContextMenu(captureIdx: number, event: MouseEvent) {
  event.preventDefault()
  browse.selectCapture(captureIdx)
  ui.openContextMenu(captureIdx, event.clientX, event.clientY)
}
</script>

<template>
  <div class="grid-container" @contextmenu.prevent>
    <div v-if="browse.filteredCaptures.length === 0" class="grid-empty">
      <Images :size="56" />
      <div class="grid-empty__title">{{ browse.totalCount === 0 ? '还没有照片' : '无匹配结果' }}</div>
      <template v-if="browse.totalCount === 0">
        <div class="grid-empty__hint">从左侧目录树选择文件夹，或</div>
        <button class="grid-empty__btn" @click="openFolder">
          <FolderPlus :size="14" /> 选择目录…
        </button>
      </template>
      <div v-else class="grid-empty__hint">试试清除搜索关键词</div>
    </div>
    <TransitionGroup v-else name="grid" tag="div" class="grid">
      <ThumbnailCell
        v-for="(capture, idx) in browse.filteredCaptures"
        :key="browse.filteredIndices[idx]"
        :capture="capture"
        :is-selected="browse.selectedIndices.has(browse.filteredIndices[idx])"
        :is-focused="browse.focusedIndex === idx"
        :size="320"
        @click="handleClick(browse.filteredIndices[idx])"
        @contextmenu="handleContextMenu(browse.filteredIndices[idx], $event)"
      />
    </TransitionGroup>
  </div>
</template>

<style scoped>
.grid-container {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
}

.grid-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 100%;
  color: var(--text-muted);
}

.grid-empty__title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-top: 8px;
}

.grid-empty__hint {
  font-size: 13px;
  color: var(--text-muted);
}

.grid-empty__btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 8px;
  font-family: var(--font-body);
  font-size: 13px;
  font-weight: 500;
  padding: 7px 16px;
  border: 1px solid var(--primary);
  border-radius: var(--radius-sm);
  background: var(--primary-subtle);
  color: var(--primary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.grid-empty__btn:hover {
  background: var(--primary);
  color: white;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 14px;
  align-content: start;
}

.grid-move {
  transition: transform 0.3s ease;
}

.grid-enter-active {
  transition: all 0.3s ease;
}

.grid-leave-active {
  transition: all 0.2s ease;
}

.grid-enter-from {
  opacity: 0;
  transform: scale(0.9);
}

.grid-leave-to {
  opacity: 0;
  transform: scale(0.9);
}
</style>
