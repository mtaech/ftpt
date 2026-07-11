<script setup lang="ts">
import { ref, computed } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { renameCaptures } from '@/types/tauri'
import AppDialog from '../AppDialog.vue'

const browse = useBrowseStore()
const ui = useUiStore()

const prefix = ref('')
const startSeq = ref(1)
const digitCount = ref(3)

const preview = computed(() => {
  const items = browse.selectedCaptures.slice(0, 5)
  const lines = items.map((c, i) => {
    const seq = startSeq.value + i
    const newName = `${prefix.value}${String(seq).padStart(digitCount.value, '0')}`
    return `${c.baseName}  →  ${newName}`
  })
  if (browse.selectedCount > 5) lines.push(`… 共 ${browse.selectedCount} 个文件`)
  return lines.join('\n')
})

async function doRename() {
  const items: Array<[string, string]> = browse.selectedCaptures.map((c, i) => {
    const seq = startSeq.value + i
    const newBase = `${prefix.value}${String(seq).padStart(digitCount.value, '0')}`
    const ext = c.primaryFormat.toLowerCase()
    return [c.primaryPath, `${newBase}.${ext}`]
  })
  try { await renameCaptures(items); await browse.openDirectory(browse.currentPath) }
  finally { ui.closeRename() }
}
</script>

<template>
  <AppDialog title="批量重命名" @close="ui.closeRename()">
    <template #body>
      <div class="field">
        <label>前缀</label>
        <input class="input" v-model="prefix" placeholder="例如：旅行_2025_" />
      </div>
      <div class="field-row">
        <div class="field">
          <label>起始序号</label>
          <input class="input" type="number" v-model.number="startSeq" min="1" style="width:100px" />
        </div>
        <div class="field">
          <label>位数</label>
          <select class="input" v-model.number="digitCount" style="width:100px">
            <option :value="2">2 (01)</option>
            <option :value="3">3 (001)</option>
            <option :value="4">4 (0001)</option>
          </select>
        </div>
      </div>
      <div class="field">
        <label>预览</label>
        <pre class="preview-box">{{ preview }}</pre>
      </div>
    </template>
    <template #footer>
      <button class="btn" @click="ui.closeRename()">取消</button>
      <button class="btn btn--primary" @click="doRename">重命名</button>
    </template>
  </AppDialog>
</template>

<style scoped>
.field { display: flex; flex-direction: column; gap: 4px; flex: 1; }
.field label { font-size: 12px; font-weight: 500; color: var(--text-secondary); }
.field-row { display: flex; gap: 12px; }
.input { font-family: var(--font-body); font-size: 13px; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-page); color: var(--text); outline: none; transition: all var(--transition-fast); }
.input:focus { border-color: var(--border-focus); box-shadow: 0 0 0 3px var(--primary-subtle); background: var(--bg-surface); }
.preview-box { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 12px; padding: 10px; background: var(--bg-page); border-radius: var(--radius-sm); border: 1px solid var(--border-light); white-space: pre-wrap; max-height: 120px; overflow-y: auto; line-height: 1.6; color: var(--text-secondary); }
.btn { font-family: var(--font-body); font-size: 13px; font-weight: 500; padding: 7px 14px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); }
.btn:hover { background: var(--bg-hover); color: var(--text); }
.btn--primary { background: var(--primary); color: white; border-color: var(--primary); }
.btn--primary:hover { background: var(--primary-hover); }
</style>
