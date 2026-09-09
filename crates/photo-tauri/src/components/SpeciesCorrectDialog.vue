<script setup lang="ts">
// 识别结果人工纠错对话框：显示当前照片 Top-5 模型候选（Recognition.candidates，
// 仅 bird 非空项可选）+ 名录搜索（300ms 防抖调 search_catalog，中文名/拼音/拉丁名），
// 选择鸟种后批量调 correct_recognition（作用于打开时传入的路径集合，多选批量应用），
// 成功后在 captures store 本地同步识别摘要字段。
// 入口：InfoPanel 识别卡「纠正…」按钮 / 网格右键「纠正鸟种…」（recognition store 管理显隐）。
import { computed, onUnmounted, ref, watch } from 'vue'
import { CheckIcon, SearchIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog'
import type { BirdCandidate, BirdMatch, CatalogEntry, Recognition } from '@/lib/bindings'
import { getFrequentSpecies, getRecognition, searchCatalog } from '@/lib/ipc'
import { useRecognitionStore } from '@/stores/recognition'

const recognition = useRecognitionStore()

/** 目标路径集合（打开时由 store 传入；空 = 无目标） */
const paths = computed(() => recognition.correctionPaths)
/** 当前照片完整识别结果（Top-5 候选数据源；无识别记录为 null） */
const fullRecognition = ref<Recognition | null>(null)
/** 名录搜索结果（防抖后 search_catalog 返回） */
const results = ref<CatalogEntry[]>([])
/** 高频鸟种「常用」快捷项（本机使用频次降序，空搜索词时显示；沿用原修正下拉） */
const frequent = ref<string[]>([])
/** 常用项反查进行中（chip 点击后按中文名精确反查名录，防连点） */
const frequentLoading = ref<string | null>(null)
/** 搜索词（300ms 防抖触发名录搜索） */
const query = ref('')
/** 名录搜索进行中 */
const searching = ref(false)
/** 当前选中的名录条目（应用按钮启用条件） */
const selected = ref<CatalogEntry | null>(null)
/** 应用错误信息（失败时展示） */
const errorMsg = ref('')
/** 防抖计时器（卸载时清理） */
let debounceTimer: ReturnType<typeof setTimeout> | null = null
/** 加载序号：切图后旧请求结果直接丢弃（防异步回填串图，同 InfoPanel loadRecognition 模式） */
let loadSeq = 0

/** Top-5 候选（仅已映射项：bird 非空；未映射类别不出现在候选列表，用户可走「名录搜索」手动选） */
const candidates = computed(() =>
  (fullRecognition.value?.candidates ?? []).filter((c) => c.bird != null),
)

/** 置信度归一化：mock 为 0–1 小数、真实后端 0–100，统一到 0–100 */
function confPercent(c: number | null): number {
  if (c === null) return 0
  return c <= 1 ? Math.round(c * 100) : Math.round(c)
}

/** 打开：清空旧状态 + 拉取首张完整识别结果（Top-5 候选数据源） */
watch(
  () => recognition.correctionOpen,
  (open) => {
    if (!open) return
    query.value = ''
    results.value = []
    searching.value = false
    selected.value = null
    errorMsg.value = ''
    fullRecognition.value = null
    const path = paths.value[0]
    if (!path) return
    const seq = ++loadSeq
    // 高频「常用」快捷项（失败静默：仅少一组 chips，不阻塞对话框）
    void getFrequentSpecies(10)
      .then((list) => {
        if (seq === loadSeq) frequent.value = list
      })
      .catch(() => {})
    void getRecognition(path)
      .then((r) => {
        if (seq !== loadSeq) return
        fullRecognition.value = r
      })
      .catch(() => {
        if (seq === loadSeq) fullRecognition.value = null
      })
  },
)

/** 搜索词防抖（300ms）：空词不发请求 */
watch(query, () => {
  if (debounceTimer) clearTimeout(debounceTimer)
  debounceTimer = setTimeout(runSearch, 300)
})

async function runSearch() {
  const q = query.value.trim()
  if (q === '') {
    results.value = []
    return
  }
  searching.value = true
  const seq = ++loadSeq
  try {
    const list = await searchCatalog(q, 50)
    if (seq !== loadSeq) return
    results.value = list
  } catch (e) {
    if (seq === loadSeq) errorMsg.value = `名录搜索失败：${e}`
  } finally {
    if (seq === loadSeq) searching.value = false
  }
}

/** 模型候选选中：bird 转 CatalogEntry 形态（birdId/cnName/latinName） */
function selectFromBird(bird: BirdMatch) {
  selected.value = { birdId: bird.birdId, cnName: bird.cnName, latinName: bird.latinName }
}

/** 候选点击：选中已映射候选（候选列表已过滤，bird 均非空） */
function pickCandidate(c: BirdCandidate) {
  if (c.bird) selectFromBird(c.bird)
}

/** 候选是否已选中（列表样式与勾选标记共用判断） */
function isCandidateSelected(c: BirdCandidate): boolean {
  return selected.value?.birdId === c.bird!.birdId
}

/** 候选按钮样式：选中高亮（候选已过滤，无未映射置灰态） */
function candidateCls(c: BirdCandidate): string {
  return isCandidateSelected(c)
    ? 'border-primary/40 bg-primary/10 text-primary'
    : 'border-transparent bg-muted/50 text-foreground hover:bg-element-hover'
}

/** 候选按钮文案：中文明（候选已过滤，无「未映射（类别 #）」文案） */
function candidateLabel(c: BirdCandidate): string {
  return c.bird!.cnName
}

/** 名录条目选中 */
function selectEntry(e: CatalogEntry) {
  selected.value = e
}

/** 常用 chip 点击：按中文名精确反查名录（cnName 全等）后直接选中；未命中回填搜索词 */
async function pickFrequent(name: string) {
  if (frequentLoading.value) return
  frequentLoading.value = name
  try {
    const list = await searchCatalog(name, 10)
    const hit = list.find((e) => e.cnName === name)
    if (hit) {
      selectEntry(hit)
    } else {
      query.value = name
    }
  } catch {
    query.value = name
  } finally {
    frequentLoading.value = null
  }
}

/** 应用纠正：批量写 folder_db + global_db 日志，成功后本地同步并关闭 */
async function apply() {
  const entry = selected.value
  if (!entry || recognition.correcting) return
  errorMsg.value = ''
  try {
    await recognition.correct(paths.value, entry.birdId, entry.cnName, entry.latinName)
    recognition.setNotice(`已纠正 ${paths.value.length} 张为「${entry.cnName}」`)
    recognition.closeCorrection()
  } catch (e) {
    errorMsg.value = `纠正失败：${e}`
  }
}

onUnmounted(() => {
  if (debounceTimer) clearTimeout(debounceTimer)
})
</script>

<template>
  <Dialog
    :open="recognition.correctionOpen"
    @update:open="(v: boolean) => !v && recognition.closeCorrection()"
  >
    <DialogContent
      :show-close-button="false"
      class="flex max-h-[85vh] w-[26rem] flex-col gap-0 p-0 sm:max-w-[26rem]"
    >
      <!-- 头栏 -->
      <div class="flex shrink-0 items-center justify-between gap-2 border-b border-border/70 px-4 py-3.5">
        <DialogTitle class="text-sm font-semibold tracking-tight">
          纠正鸟种<span class="ml-1 font-normal text-muted-foreground">（{{ paths.length }} 张）</span>
        </DialogTitle>
        <DialogClose as-child>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="关闭"
            @click="recognition.closeCorrection()"
          >
            <XIcon />
          </Button>
        </DialogClose>
      </div>

      <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
        <DialogDescription class="sr-only">
          从模型候选或名录搜索中选择鸟种，批量写回识别结果
        </DialogDescription>

        <!-- 当前识别（无识别记录时不显示，直接从候选/搜索选择） -->
        <div
          v-if="fullRecognition?.bird"
          class="flex items-center justify-between gap-2 rounded-md border border-primary/20 bg-primary/5 px-3 py-2.5"
        >
          <div class="flex min-w-0 items-center gap-1.5">
            <span class="shrink-0 text-[11px] text-muted-foreground">当前</span>
            <span class="truncate text-xs font-medium text-primary">{{ fullRecognition.bird.cnName }}</span>
          </div>
          <span v-if="fullRecognition.confidence != null" class="shrink-0 text-xs tabular-nums text-muted-foreground">
            {{ confPercent(fullRecognition.confidence) }}%
          </span>
        </div>

        <!-- Top-5 模型候选：仅展示已映射 bird 的候选（未映射类别已在 candidates 过滤，不出现）；点击即选中 -->
        <div class="space-y-1.5">
          <label class="text-[11px] font-semibold text-muted-foreground">模型候选</label>
          <template v-if="candidates.length > 0">
            <button
              v-for="(c, i) in candidates"
              :key="i"
              type="button"
              class="flex w-full items-center justify-between gap-2 rounded-md border px-2.5 py-2 text-left text-xs transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              :class="candidateCls(c)"
              @click="pickCandidate(c)"
            >
              <span class="truncate">{{ candidateLabel(c) }}</span>
              <span class="flex shrink-0 items-center gap-1.5">
                <CheckIcon v-if="isCandidateSelected(c)" class="size-3.5 text-primary" />
                <span class="tabular-nums text-muted-foreground">{{ confPercent(c.confidence) }}%</span>
              </span>
            </button>
          </template>
          <p v-else class="text-xs text-muted-foreground">无模型候选（可从下方名录搜索选择）</p>
        </div>

        <!-- 常用：高频鸟种快捷项（空搜索词时显示；本机使用频次降序，沿用原修正下拉） -->
        <div v-if="frequent.length > 0 && !query.trim()" class="space-y-1.5">
          <label class="text-[11px] font-semibold text-muted-foreground">常用</label>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="name in frequent"
              :key="name"
              type="button"
              :disabled="frequentLoading === name"
              class="rounded-full border px-2.5 py-1 text-xs transition-colors"
              :class="
                selected?.cnName === name
                  ? 'border-primary/40 bg-primary/10 text-primary'
                  : 'border-border/70 bg-muted/40 hover:bg-element-hover hover:text-foreground'
              "
              @click="pickFrequent(name)"
            >
              {{ name }}
            </button>
          </div>
        </div>

        <!-- 名录搜索：300ms 防抖调 search_catalog -->
        <div class="space-y-1.5">
          <label class="text-[11px] font-semibold text-muted-foreground">名录搜索</label>
          <div class="relative">
            <SearchIcon
              class="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            />
            <input
              v-model="query"
              type="text"
              placeholder="中文名 / 拼音 / 拉丁名…"
              class="h-8 w-full rounded-md border border-input bg-background pr-2 pl-8 text-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:border-primary focus-visible:ring-2 focus-visible:ring-ring/40"
            />
          </div>
          <div
            v-if="query.trim()"
            class="max-h-44 overflow-y-auto rounded-md border border-border bg-popover"
          >
            <p v-if="searching" class="px-2.5 py-1.5 text-xs text-muted-foreground">搜索中…</p>
            <template v-else>
              <button
                v-for="e in results"
                :key="e.birdId"
                type="button"
                class="flex w-full items-center justify-between gap-2 px-2.5 py-1.5 text-left text-xs transition-colors"
                :class="
                  selected?.birdId === e.birdId
                    ? 'bg-primary/10 text-primary'
                    : 'hover:bg-accent hover:text-accent-foreground'
                "
                @click="selectEntry(e)"
              >
                <span class="truncate">{{ e.cnName }}</span>
                <span class="shrink-0 text-muted-foreground/80 italic">{{ e.latinName }}</span>
              </button>
              <p
                v-if="!searching && results.length === 0"
                class="px-2.5 py-1.5 text-xs text-muted-foreground"
              >
                无匹配鸟种
              </p>
            </template>
          </div>
        </div>

        <!-- 应用错误 -->
        <p v-if="errorMsg" class="text-xs text-label-red">{{ errorMsg }}</p>
      </div>

      <!-- 底部：取消 / 应用（选中条目后启用） -->
      <div class="flex shrink-0 items-center justify-end gap-2 border-t border-border/70 px-4 py-3">
        <Button
          variant="ghost"
          size="sm"
          :disabled="recognition.correcting"
          @click="recognition.closeCorrection()"
        >
          取消
        </Button>
        <Button size="sm" :disabled="!selected || recognition.correcting" @click="apply">
          {{ recognition.correcting ? '纠正中…' : `应用纠正（${paths.length} 张）` }}
        </Button>
      </div>
    </DialogContent>
  </Dialog>
</template>
