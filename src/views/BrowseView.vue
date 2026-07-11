<script setup lang="ts">
import Layout from '@/components/Layout.vue'
import StatusBar from '@/components/StatusBar.vue'
import DirectoryTree from '@/components/DirectoryTree.vue'
import Toolbar from '@/components/Toolbar.vue'
import ThumbnailGrid from '@/components/ThumbnailGrid.vue'
import PreviewPanel from '@/components/PreviewPanel.vue'
import ContextMenu from '@/components/ContextMenu.vue'
import ImportDialog from '@/components/dialogs/ImportDialog.vue'
import RenameDialog from '@/components/dialogs/RenameDialog.vue'
import SettingsDialog from '@/components/dialogs/SettingsDialog.vue'
import ConvertDialog from '@/components/dialogs/ConvertDialog.vue'
import { FolderSearch, ScanSearch, CheckCircle2 } from 'lucide-vue-next'
import { onMounted } from 'vue'
import { useKeyboard } from '@/composables/useKeyboard'
import { useUiStore } from '@/stores/ui'
import { useBrowseStore } from '@/stores/browse'
import { useConfigStore } from '@/stores/config'

const ui = useUiStore()
const browse = useBrowseStore()
const config = useConfigStore()
useKeyboard()

onMounted(async () => {
  // 1. 加载目录树结构
  await browse.loadDirectoryTree()

  // 2. 加载配置，如果上次有打开的目录则自动打开
  await config.load()
  if (config.config.lastDirectory) {
    browse.openDirectory(config.config.lastDirectory)
  }
})
</script>

<template>
  <div class="browse">
    <Layout>
      <template #left><DirectoryTree /></template>
      <template #center>
        <Toolbar />
        <ThumbnailGrid />
      </template>
      <template #right><PreviewPanel /></template>
    </Layout>
    <StatusBar />

    <!-- 右下角扫描进度条 -->
    <Transition name="progress">
      <div v-if="browse.isScanning && browse.scanProgress" class="scan-progress">
        <div class="progress-header">
          <FolderSearch v-if="browse.scanProgress.phase === 'scanning'" :size="14" />
          <ScanSearch v-else-if="browse.scanProgress.phase === 'building'" :size="14" />
          <CheckCircle2 v-else :size="14" />
          <span class="progress-label">
            {{ browse.scanProgress.phase === 'scanning' ? '扫描文件' : browse.scanProgress.phase === 'building' ? '构建元数据' : '完成' }}
          </span>
        </div>
        <div class="progress-bar-track">
          <div
            class="progress-bar-fill"
            :style="{ width: browse.scanProgress.percent + '%' }"
          />
        </div>
        <div class="progress-path" :title="browse.scanProgress.path">
          {{ browse.scanProgress.path }}
        </div>
      </div>
    </Transition>

    <ContextMenu />
    <ImportDialog v-if="ui.importOpen" />
    <RenameDialog v-if="ui.renameOpen" />
    <SettingsDialog v-if="ui.settingsOpen" />
    <ConvertDialog v-if="ui.convertOpen" />
  </div>
</template>

<style scoped>
.browse { display: flex; flex-direction: column; height: 100%; position: relative; }

/* ── 进度条 ── */
.scan-progress {
  position: fixed;
  right: 16px;
  bottom: 16px;
  z-index: 200;
  width: 280px;
  padding: 12px 16px;
  background: var(--color-surface, #fff);
  border: 1px solid var(--color-border, #e0e0e0);
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.1);
}

.progress-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
  font-size: 12px;
  color: var(--color-text-secondary, #666);
}

.progress-label { font-weight: 500; }

.progress-bar-track {
  height: 6px;
  background: var(--color-border, #e0e0e0);
  border-radius: 3px;
  overflow: hidden;
}

.progress-bar-fill {
  height: 100%;
  background: linear-gradient(90deg, #3b82f6, #2563eb);
  border-radius: 3px;
  transition: width 0.3s ease-out;
}

.progress-path {
  margin-top: 6px;
  font-size: 11px;
  color: var(--color-text-tertiary, #999);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ── 进出动画 ── */
.progress-enter-active { transition: all 0.25s ease-out; }
.progress-leave-active { transition: all 0.3s ease-in; }
.progress-enter-from,
.progress-leave-to {
  opacity: 0;
  transform: translateY(12px);
}
</style>
