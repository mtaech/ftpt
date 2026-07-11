<script setup lang="ts">
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { openFolderDialog, deleteCaptures, moveCaptures, copyCaptures } from '@/types/tauri'
import { Search, ArrowUpDown, ArrowUp, ArrowDown, Trash2, ArrowUpToLine, Copy, Pencil, Download, RefreshCw, Settings } from 'lucide-vue-next'

const browse = useBrowseStore()
const ui = useUiStore()

const sortLabels: Record<string, string> = { FileName: '文件名', DateTaken: '拍摄日期', FileSize: '文件大小' }

function cycleSort() {
  const next: Record<string, string> = { FileName: 'DateTaken', DateTaken: 'FileSize', FileSize: 'FileName' }
  browse.setSort(next[browse.sortBy] as any, browse.sortDirection)
}

function toggleDirection() {
  const dir = browse.sortDirection === 'Ascending' ? 'Descending' : 'Ascending'
  browse.setSort(browse.sortBy, dir as any)
}

async function doDelete() {
  const paths = Array.from(browse.selectedIndices).map(i => [browse.captures[i]?.primaryPath].filter(Boolean)) as string[][]
  if (paths.length === 0) return
  await deleteCaptures(paths, 'trash')
  await browse.openDirectory(browse.currentPath)
}

async function doMove() {
  const dir = await openFolderDialog('选择目标目录')
  if (!dir) return
  const paths = Array.from(browse.selectedIndices).map(i => [browse.captures[i]?.primaryPath].filter(Boolean)) as string[][]
  if (paths.length === 0) return
  await moveCaptures(paths, dir)
  await browse.openDirectory(browse.currentPath)
}

async function doCopy() {
  const dir = await openFolderDialog('选择目标目录')
  if (!dir) return
  const paths = Array.from(browse.selectedIndices).map(i => [browse.captures[i]?.primaryPath].filter(Boolean)) as string[][]
  if (paths.length === 0) return
  await copyCaptures(paths, dir)
}

function doRename() { ui.openRename() }
function doImport() { ui.openImport() }
function doConvert() { ui.openConvert() }
function doSettings() { ui.openSettings() }
</script>

<template>
  <div class="toolbar">
    <div class="toolbar__section">
      <span class="toolbar__label">排序</span>
      <div class="btn-group">
        <button class="btn-toolbar btn-group__first" @click="cycleSort">
          <ArrowUpDown :size="14" />
          {{ sortLabels[browse.sortBy] }}
        </button>
        <button class="btn-toolbar btn-group__last btn-toolbar--icon" @click="toggleDirection" :title="browse.sortDirection === 'Ascending' ? '升序' : '降序'">
          <ArrowUp v-if="browse.sortDirection === 'Ascending'" :size="14" />
          <ArrowDown v-else :size="14" />
        </button>
      </div>
    </div>

    <div class="toolbar__divider" />

    <div class="toolbar__section toolbar__section--search">
      <input class="toolbar__search" type="text" placeholder="搜索文件名…" :value="browse.searchText" @input="e => browse.setSearch((e.target as HTMLInputElement).value)" />
      <Search :size="14" class="icon--search" />
    </div>

    <div class="toolbar__spacer" />

    <div class="toolbar__section">
      <template v-if="browse.selectedCount > 0">
        <button class="btn-toolbar btn-toolbar--danger" @click="doDelete" title="删除所选">
          <Trash2 :size="14" />
          删除 {{ browse.selectedCount }}
        </button>
        <button class="btn-toolbar btn-toolbar--icon" @click="doMove" title="移动所选到…">
          <ArrowUpToLine :size="14" />
        </button>
        <button class="btn-toolbar btn-toolbar--icon" @click="doCopy" title="复制所选到…">
          <Copy :size="14" />
        </button>
        <button class="btn-toolbar btn-toolbar--icon" @click="doRename" title="重命名所选">
          <Pencil :size="14" />
        </button>
        <button class="btn-toolbar" @click="doImport" title="导入照片">
          <Download :size="14" />
          导入
        </button>
        <button class="btn-toolbar" @click="doConvert" title="格式转换">
          <RefreshCw :size="14" />
          转换
        </button>
      </template>
      <template v-else>
        <button class="btn-toolbar" @click="doImport">
          <Download :size="14" />
          导入
        </button>
        <button class="btn-toolbar" @click="doSettings">
          <Settings :size="14" />
          设置
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
  min-height: var(--toolbar-height);
}

.toolbar__section {
  display: flex;
  align-items: center;
  gap: 4px;
}

.toolbar__section--search {
  position: relative;
}

.toolbar__divider {
  width: 1px;
  height: 24px;
  background: var(--border);
  flex-shrink: 0;
}

.toolbar__label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-muted);
  letter-spacing: 0.02em;
  margin-right: 4px;
}

.toolbar__spacer {
  flex: 1;
}

.toolbar__search {
  font-family: var(--font-body);
  font-size: 13px;
  padding: 5px 10px 5px 32px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-page);
  color: var(--text);
  flex: 1;
  max-width: 360px;
  outline: none;
  transition: all var(--transition-fast);
}

.toolbar__search:focus {
  border-color: var(--border-focus);
  box-shadow: 0 0 0 3px var(--primary-subtle);
  background: var(--bg-surface);
}

.toolbar__search::placeholder {
  color: var(--text-muted);
}

.icon--search {
  position: absolute;
  left: 8px;
  color: var(--text-muted);
  pointer-events: none;
}

.toolbar__section:has(.toolbar__search) {
  position: relative;
}

.btn-group {
  display: flex;
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

.btn-toolbar {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-family: var(--font-body);
  font-size: 13px;
  font-weight: 500;
  padding: 6px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: var(--text-secondary);
  cursor: pointer;
  white-space: nowrap;
  transition: all var(--transition-fast);
}

.btn-toolbar:hover {
  background: var(--bg-hover);
  border-color: var(--border);
  color: var(--text);
}

.btn-toolbar:active {
  transform: scale(0.97);
}

.btn-toolbar--icon {
  padding: 6px 10px;
}

.btn-toolbar--danger {
  color: var(--danger);
  border-color: rgba(239, 68, 68, 0.2);
}

.btn-toolbar--danger:hover {
  background: var(--danger-subtle);
  border-color: var(--danger);
  color: var(--danger);
}
</style>
