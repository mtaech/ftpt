<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { openFolderDialog, detectDrives, importCaptures } from '@/types/tauri'
import { Loader2 } from 'lucide-vue-next'
import AppDialog from '../AppDialog.vue'

const browse = useBrowseStore()
const ui = useUiStore()

const drives = ref<string[]>([])
const sourcePath = ref('')
const destPath = ref('')
const behavior = ref('copy')
const dateFormat = ref('year_month_day')
const overwrite = ref('skip')
const loading = ref(false)

onMounted(async () => {
  drives.value = await detectDrives()
  const { homeDir } = await import('@tauri-apps/api/path')
  destPath.value = await homeDir() + '/Pictures'
})

async function pickSource() {
  const dir = await openFolderDialog('选择源目录')
  if (dir) sourcePath.value = dir
}

async function pickDest() {
  const dir = await openFolderDialog('选择目标归档目录')
  if (dir) destPath.value = dir
}

async function doImport() {
  if (!sourcePath.value || !destPath.value) return
  loading.value = true
  try {
    await importCaptures([], { destRoot: destPath.value, behavior: behavior.value, dateFormat: dateFormat.value, overwriteStrategy: overwrite.value })
  } finally {
    loading.value = false
    ui.closeImport()
  }
}
</script>

<template>
  <AppDialog title="导入照片" width="520px" @close="ui.closeImport()">
    <template #body>
      <div class="field">
        <label>源设备</label>
        <div class="field__row">
          <select v-model="sourcePath" class="input">
            <option value="">— 选择设备 —</option>
            <option v-for="d in drives" :key="d" :value="d">{{ d }}</option>
          </select>
          <button class="btn btn--outline" @click="pickSource">浏览…</button>
        </div>
      </div>
      <div class="field">
        <label>源路径</label>
        <input class="input" v-model="sourcePath" placeholder="/media/card/DCIM" />
      </div>
      <div class="field">
        <label>目标归档目录</label>
        <div class="field__row">
          <input class="input" v-model="destPath" />
          <button class="btn btn--outline" @click="pickDest">浏览…</button>
        </div>
      </div>
      <div class="field-row">
        <div class="field">
          <label>操作</label>
          <select class="input" v-model="behavior">
            <option value="copy">复制</option>
            <option value="move">移动</option>
          </select>
        </div>
        <div class="field">
          <label>日期格式</label>
          <select class="input" v-model="dateFormat">
            <option value="year_month_day">年/月/日</option>
            <option value="iso_date">YYYY-MM-DD</option>
            <option value="year_iso">年/YYYY-MM-DD</option>
          </select>
        </div>
        <div class="field">
          <label>同名处理</label>
          <select class="input" v-model="overwrite">
            <option value="skip">跳过</option>
            <option value="overwrite">覆盖</option>
            <option value="rename">重命名</option>
          </select>
        </div>
      </div>
    </template>
    <template #footer>
      <button class="btn" @click="ui.closeImport()">取消</button>
      <button class="btn btn--primary" :disabled="loading" @click="doImport">
        <Loader2 v-if="loading" :size="14" class="spin" />
        {{ loading ? '导入中…' : '导入' }}
      </button>
    </template>
  </AppDialog>
</template>

<style scoped>
.field { display: flex; flex-direction: column; gap: 4px; flex: 1; }
.field label { font-size: 12px; font-weight: 500; color: var(--text-secondary); }
.field__row { display: flex; gap: 6px; }
.field__row .input { flex: 1; }
.field-row { display: flex; gap: 12px; }
.input { font-family: var(--font-body); font-size: 13px; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-page); color: var(--text); outline: none; transition: all var(--transition-fast); }
.input:focus { border-color: var(--border-focus); box-shadow: 0 0 0 3px var(--primary-subtle); background: var(--bg-surface); }
.btn { font-family: var(--font-body); font-size: 13px; font-weight: 500; padding: 7px 14px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); white-space: nowrap; }
.btn:hover { background: var(--bg-hover); color: var(--text); }
.btn--outline { border-color: var(--border); }
.btn--primary { background: var(--primary); color: white; border-color: var(--primary); }
.btn--primary:hover { background: var(--primary-hover); }
.btn:disabled { opacity: 0.5; cursor: not-allowed; }
.spin { animation: spin 1s linear infinite; }
@keyframes spin { 100% { transform: rotate(360deg); } }
</style>
