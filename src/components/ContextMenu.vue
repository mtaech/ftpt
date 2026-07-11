<script setup lang="ts">
import { ref, watch } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { openFolderDialog, confirmDialog } from '@/types/tauri'
import { deleteCaptures, moveCaptures, copyCaptures } from '@/types/tauri'
import { writeCaptureXmp, readCaptureXmp } from '@/types/tauri'
import type { XmpMetadata } from '@/types'
import { Trash2, ArrowUpToLine, Copy, Pencil, Star, Flag, FlagOff, Check } from 'lucide-vue-next'

const browse = useBrowseStore()
const ui = useUiStore()

const currentRating = ref(0)
const currentColorLabel = ref('')
const currentFlag = ref('')

watch(() => ui.contextMenu, async (menu) => {
  if (!menu) { currentRating.value = 0; currentColorLabel.value = ''; currentFlag.value = ''; return }
  const cap = browse.filteredCaptures[menu.index]
  if (!cap) return
  try {
    const xmp: XmpMetadata = await readCaptureXmp(cap.primaryPath)
    currentRating.value = xmp.rating || 0
    currentColorLabel.value = xmp.colorLabel || ''
    currentFlag.value = xmp.flag || ''
  } catch {
    currentRating.value = 0
    currentColorLabel.value = ''
    currentFlag.value = ''
  }
}, { immediate: false })

const colorDotLabels: Record<string, string> = { red: '红', yellow: '黄', green: '绿', blue: '蓝', purple: '紫' }

function getSelectedPaths(): string[][] {
  return Array.from(browse.selectedIndices)
    .map(i => [browse.captures[i]?.primaryPath].filter(Boolean)) as string[][]
}

function close() { ui.closeContextMenu() }

async function doDelete(permanent: boolean) {
  const paths = getSelectedPaths()
  if (paths.length === 0) { close(); return }
  if (permanent) {
    const ok = await confirmDialog(`确定要永久删除 ${paths.length} 个文件吗？此操作不可恢复。`, '确认永久删除')
    if (!ok) { close(); return }
  }
  await deleteCaptures(paths, permanent ? 'permanent' : 'trash')
  await browse.openDirectory(browse.currentPath)
  close()
}

async function doMove() {
  close()
  const dir = await openFolderDialog('选择目标目录')
  if (!dir) return
  await moveCaptures(getSelectedPaths(), dir)
  await browse.openDirectory(browse.currentPath)
}

async function doCopy() {
  close()
  const dir = await openFolderDialog('选择目标目录')
  if (!dir) return
  await copyCaptures(getSelectedPaths(), dir)
}

function doRename() { close(); ui.openRename() }

async function setRating(rating: number) {
  const idx = ui.contextMenu?.index
  if (idx === undefined) return
  const cap = browse.filteredCaptures[idx]
  if (!cap) return
  await writeCaptureXmp(cap.primaryPath, { rating, colorLabel: currentColorLabel.value, flag: currentFlag.value })
  currentRating.value = rating
}

async function setColorLabel(label: string) {
  const idx = ui.contextMenu?.index
  if (idx === undefined) return
  const cap = browse.filteredCaptures[idx]
  if (!cap) return
  await writeCaptureXmp(cap.primaryPath, { rating: currentRating.value, colorLabel: label, flag: currentFlag.value })
  currentColorLabel.value = label
}

async function toggleFlag(flag: string) {
  const idx = ui.contextMenu?.index
  if (idx === undefined) return
  const cap = browse.filteredCaptures[idx]
  if (!cap) return
  const newFlag = currentFlag.value === flag ? '' : flag
  await writeCaptureXmp(cap.primaryPath, { rating: currentRating.value, colorLabel: currentColorLabel.value, flag: newFlag })
  currentFlag.value = newFlag
}
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="ui.contextMenu" class="cm-backdrop" @click="close" @contextmenu.prevent="close" @keydown.escape="close">
        <div class="cm-menu" :style="{ left: ui.contextMenu.x + 'px', top: ui.contextMenu.y + 'px' }" @click.stop>
          <button class="cm-item" @click="doDelete(false)">
            <Trash2 :size="16" class="cm-icon" />
            删除（回收站）
          </button>
          <button class="cm-item cm-item--danger" @click="doDelete(true)">
            <Trash2 :size="16" class="cm-icon" />
            永久删除
          </button>
          <div class="cm-sep" />
          <button class="cm-item" @click="doMove">
            <ArrowUpToLine :size="16" class="cm-icon" />
            移动…
          </button>
          <button class="cm-item" @click="doCopy">
            <Copy :size="16" class="cm-icon" />
            复制…
          </button>
          <button class="cm-item" @click="doRename">
            <Pencil :size="16" class="cm-icon" />
            重命名…
          </button>
          <div class="cm-sep" />
          <div class="cm-section">
            <span class="cm-section__title">评分</span>
            <div class="cm-stars">
              <Star
                v-for="i in 5"
                :key="i"
                :size="14"
                class="star"
                :class="{ active: i <= currentRating }"
                :fill="i <= currentRating ? 'var(--star)' : 'none'"
                :stroke="i <= currentRating ? 'var(--star)' : 'var(--border)'"
                @click="setRating(i)"
              />
            </div>
          </div>
          <div class="cm-section">
            <span class="cm-section__title">颜色标签</span>
            <div class="cm-colors">
              <span class="dot dot--none" :class="{ active: currentColorLabel === '' }" @click="setColorLabel('')" title="无">
                <Check v-if="currentColorLabel === ''" :size="10" />
              </span>
              <span
                v-for="color in ['red', 'yellow', 'green', 'blue', 'purple']"
                :key="color"
                class="dot"
                :class="{ active: currentColorLabel === color }"
                :style="{ background: { red: '#EF4444', yellow: '#F59E0B', green: '#22C55E', blue: '#3B82F6', purple: '#A855F7' }[color] }"
                :title="colorDotLabels[color]"
                @click="setColorLabel(color)"
              >
                <Check v-if="currentColorLabel === color" :size="10" />
              </span>
            </div>
          </div>
          <div class="cm-section">
            <span class="cm-section__title">旗标</span>
            <div class="cm-flags">
              <button
                class="flag-btn"
                :class="{ 'flag-btn--pick': currentFlag !== 'pick', 'flag-btn--active': currentFlag === 'pick' }"
                @click="toggleFlag('pick')"
              >
                <Flag v-if="currentFlag !== 'pick'" :size="12" />
                <Flag v-else :size="12" fill="var(--flag-pick)" />
                Pick
              </button>
              <button
                class="flag-btn"
                :class="{ 'flag-btn--reject': currentFlag !== 'reject', 'flag-btn--active flag-btn--reject-active': currentFlag === 'reject' }"
                @click="toggleFlag('reject')"
              >
                <FlagOff v-if="currentFlag !== 'reject'" :size="12" />
                <FlagOff v-else :size="12" fill="var(--flag-reject)" />
                Reject
              </button>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.cm-backdrop { position: fixed; inset: 0; z-index: 1000; }
.cm-menu {
  position: absolute;
  z-index: 1001;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 6px;
  min-width: 200px;
  box-shadow: var(--shadow-xl);
  backdrop-filter: blur(8px);
}

.cm-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  text-align: left;
  font-family: var(--font-body);
  font-size: 13px;
  padding: 6px 10px;
  border: none;
  background: none;
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.cm-item:hover { background: var(--bg-hover); color: var(--text); }
.cm-item--danger { color: var(--danger); }
.cm-item--danger:hover { background: var(--danger-subtle); color: var(--danger); }

.cm-icon { flex-shrink: 0; }

.cm-sep { height: 1px; background: var(--border-light); margin: 4px 8px; }

.cm-section {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
}

.cm-section__title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  min-width: 44px;
}

.cm-stars { display: flex; gap: 2px; }
.star { cursor: pointer; transition: color var(--transition-fast), fill var(--transition-fast), stroke var(--transition-fast); }
.star:hover { fill: var(--star) !important; stroke: var(--star) !important; }

.cm-colors { display: flex; gap: 4px; align-items: center; }
.dot {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  cursor: pointer;
  transition: transform var(--transition-fast), border-color var(--transition-fast);
  display: flex;
  align-items: center;
  justify-content: center;
  color: white;
}
.dot:hover { transform: scale(1.25); }
.dot--none { background: transparent; border: 1.5px solid var(--border); color: var(--text-muted); }
.dot--none.active { border-color: var(--primary); }
.dot--none:hover { border-color: var(--text-muted); }

.cm-flags { display: flex; gap: 4px; }
.flag-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  font-weight: 600;
  padding: 2px 10px;
  border-radius: 4px;
  border: none;
  cursor: pointer;
  color: white;
  transition: opacity var(--transition-fast);
}
.flag-btn:hover { opacity: 0.8; }
.flag-btn--pick { background: var(--flag-pick); }
.flag-btn--reject { background: var(--flag-reject); }
.flag-btn--active { opacity: 1; }
.flag-btn--reject-active { background: var(--flag-reject); }

.fade-enter-active, .fade-leave-active { transition: opacity 150ms ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
</style>
