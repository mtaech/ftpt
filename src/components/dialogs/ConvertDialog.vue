<script setup lang="ts">
import { ref, computed } from 'vue'
import { useBrowseStore } from '@/stores/browse'
import { useUiStore } from '@/stores/ui'
import { openFolderDialog, convertImages } from '@/types/tauri'
import AppDialog from '../AppDialog.vue'

const browse = useBrowseStore()
const ui = useUiStore()

const outputDir = ref('')
const outputFormat = ref('jpg')
const jpegQuality = ref(90)
const maxDimension = ref(0)

const showQuality = computed(() => outputFormat.value === 'jpg')

async function pickOutput() {
  const dir = await openFolderDialog('选择输出目录')
  if (dir) outputDir.value = dir
}

async function doConvert() {
  if (!outputDir.value) return
  const paths = browse.selectedCaptures.map(c => c.primaryPath)
  try {
    await convertImages(paths, { outputDir: outputDir.value, outputFormat: outputFormat.value, jpegQuality: jpegQuality.value, maxDimension: maxDimension.value })
  } finally { ui.closeConvert() }
}
</script>

<template>
  <AppDialog title="转换" @close="ui.closeConvert()">
    <template #body>
      <div class="info-badge">已选择 <strong>{{ browse.selectedCount }}</strong> 张拍摄</div>
      <div class="field">
        <label>输出目录</label>
        <div class="field__row">
          <input class="input" v-model="outputDir" placeholder="选择输出目录" />
          <button class="btn btn--outline" @click="pickOutput">浏览…</button>
        </div>
      </div>
      <div class="field-row">
        <div class="field">
          <label>输出格式</label>
          <select class="input" v-model="outputFormat">
            <option value="jpg">JPEG</option>
            <option value="png">PNG</option>
          </select>
        </div>
        <div class="field">
          <label>最大尺寸</label>
          <input class="input" type="number" v-model.number="maxDimension" min="0" max="10000" placeholder="0=原尺寸" />
        </div>
      </div>
      <div v-if="showQuality" class="field">
        <label>JPEG 质量</label>
        <div class="field__row">
          <input type="range" min="1" max="100" v-model.number="jpegQuality" class="slider" />
          <span class="slider-val">{{ jpegQuality }}%</span>
        </div>
      </div>
    </template>
    <template #footer>
      <button class="btn" @click="ui.closeConvert()">取消</button>
      <button class="btn btn--primary" @click="doConvert">开始转换</button>
    </template>
  </AppDialog>
</template>

<style scoped>
.info-badge { font-size: 13px; color: var(--text-secondary); background: var(--primary-subtle); padding: 8px 12px; border-radius: var(--radius-sm); }
.field { display: flex; flex-direction: column; gap: 4px; flex: 1; }
.field label { font-size: 12px; font-weight: 500; color: var(--text-secondary); }
.field__row { display: flex; align-items: center; gap: 6px; }
.field__row .input { flex: 1; }
.field-row { display: flex; gap: 12px; }
.input { font-family: var(--font-body); font-size: 13px; padding: 7px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-page); color: var(--text); outline: none; transition: all var(--transition-fast); }
.input:focus { border-color: var(--border-focus); box-shadow: 0 0 0 3px var(--primary-subtle); background: var(--bg-surface); }
.slider { flex: 1; accent-color: var(--primary); }
.slider-val { font-size: 12px; font-weight: 500; color: var(--text-muted); min-width: 32px; }
.btn { font-family: var(--font-body); font-size: 13px; font-weight: 500; padding: 7px 14px; border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; transition: all var(--transition-fast); }
.btn:hover { background: var(--bg-hover); color: var(--text); }
.btn--outline { background: var(--bg-surface); }
.btn--primary { background: var(--primary); color: white; border-color: var(--primary); }
.btn--primary:hover { background: var(--primary-hover); }
</style>
