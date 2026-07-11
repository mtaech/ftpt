<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useUiStore } from '@/stores/ui'
import AppDialog from '../AppDialog.vue'
import { loadConfig, saveConfig } from '@/types/tauri'

const ui = useUiStore()

const thumbnailSize = ref(220)
const defaultDeleteMode = ref('trash')

onMounted(async () => {
  try {
    const cfg = await loadConfig()
    thumbnailSize.value = cfg.thumbnailSize
    defaultDeleteMode.value = cfg.defaultDeleteMode
  } catch {}
})

async function doSave() {
  try {
    const cfg = await loadConfig()
    cfg.thumbnailSize = thumbnailSize.value
    cfg.defaultDeleteMode = defaultDeleteMode.value
    await saveConfig(cfg)
  } finally { ui.closeSettings() }
}
</script>

<template>
  <AppDialog title="设置" @close="ui.closeSettings()">
    <template #body>
      <div class="setting-group">
        <h3 class="setting-group__title">常规</h3>
        <div class="field">
          <label>缩略图大小</label>
          <div class="field__row">
            <input type="range" min="100" max="400" v-model.number="thumbnailSize" class="slider" />
            <span class="slider-val">{{ thumbnailSize }}px</span>
          </div>
        </div>
        <div class="field">
          <label>默认删除模式</label>
          <select class="input" v-model="defaultDeleteMode">
            <option value="trash">移到回收站</option>
            <option value="permanent">永久删除</option>
          </select>
        </div>
      </div>
      <div class="setting-group">
        <h3 class="setting-group__title">缓存</h3>
        <p class="setting-note">缩略图缓存在应用启动时根据配置上限自动清理。</p>
      </div>
    </template>
    <template #footer>
      <button class="btn" @click="ui.closeSettings()">取消</button>
      <button class="btn btn--primary" @click="doSave">保存设置</button>
    </template>
  </AppDialog>
</template>

<style scoped>
.setting-group { display: flex; flex-direction: column; gap: 10px; }
.setting-group__title { font-family: var(--font-heading); font-size: 14px; font-weight: 600; color: var(--text); padding-bottom: 4px; border-bottom: 1px solid var(--border-light); }
.setting-note { font-size: 12px; color: var(--text-muted); line-height: 1.5; }
.field { display: flex; flex-direction: column; gap: 4px; }
.field label { font-size: 12px; font-weight: 500; color: var(--text-secondary); }
.field__row { display: flex; align-items: center; gap: 10px; }
.slider { flex: 1; accent-color: var(--primary); }
.slider-val { font-size: 12px; font-weight: 500; color: var(--text-muted); min-width: 40px; }
.input { font-family: var(--font-body); font-size: 13px; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-page); color: var(--text); outline: none; transition: all var(--transition-fast); }
.input:focus { border-color: var(--border-focus); box-shadow: 0 0 0 3px var(--primary-subtle); background: var(--bg-surface); }
.btn { font-family: var(--font-body); font-size: 13px; font-weight: 500; padding: 7px 14px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); }
.btn:hover { background: var(--bg-hover); color: var(--text); }
.btn--primary { background: var(--primary); color: white; border-color: var(--primary); }
.btn--primary:hover { background: var(--primary-hover); }
</style>
