<script setup lang="ts">
// 左栏「目录」tab 内容：打开目录 / 收藏当前目录 / 当前目录卡片 / 收藏列表 / 最近打开列表。
// 收藏与最近经 '@/lib/ipc' 命令读取（mock 模式走内存态）；外壳（宽度/拖宽/tab 头）在 LeftPanel.vue。
import { computed, onMounted, ref, watch } from 'vue'
import { ChevronRightIcon, ClockIcon, CopyIcon, FolderIcon, FolderOpenIcon, ImportIcon, StarIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useDuplicatesStore } from '@/stores/duplicates'
import { useContextMenuStore, type ContextMenuItem } from '@/stores/contextMenu'
import { useImportDialogStore } from '@/stores/importDialog'
import type { SubdirInfo } from '@/lib/bindings'
import {
  addFavorite,
  getAppConfig,
  listFavorites,
  listRecent,
  listSubdirs,
  removeFavorite,
  setAppConfig,
} from '@/lib/ipc'

const captures = useCapturesStore()
const duplicates = useDuplicatesStore()
const contextMenu = useContextMenuStore()
const importDialog = useImportDialogStore()

// ── 收藏 / 最近打开 ──────────────────────────────────────────────
const favorites = ref<string[]>([])
const recents = ref<string[]>([])

async function loadLists() {
  const [favs, rc] = await Promise.all([listFavorites(), listRecent()])
  favorites.value = favs
  recents.value = rc
}

onMounted(() => {
  void loadLists()
})

/** 目录显示名（路径末段，对齐 App.vue dirName） */
function dirName(dir: string): string {
  return dir.split(/[\\/]/).filter(Boolean).pop() ?? dir
}

/** 当前目录是否已收藏（决定按钮文案/星标填充态） */
const isFav = computed(() =>
  captures.directory ? favorites.value.includes(captures.directory) : false,
)

/** 收藏/取消收藏指定目录（按钮与右键菜单共用）；失败重拉列表回滚 */
async function toggleFavFor(dir: string) {
  try {
    if (favorites.value.includes(dir)) {
      await removeFavorite(dir)
      favorites.value = favorites.value.filter((f) => f !== dir)
    } else {
      await addFavorite(dir)
      favorites.value.push(dir)
    }
  } catch (e) {
    console.error('收藏操作失败，重拉列表', e)
    await loadLists()
  }
}

/** 收藏当前目录（顶部按钮）：已收藏则取消，否则添加 */
async function toggleFavorite() {
  const dir = captures.directory
  if (!dir) return
  await toggleFavFor(dir)
}

/** 移除收藏（卡片小按钮 / 右键菜单） */
async function removeFav(dir: string) {
  try {
    await removeFavorite(dir)
    favorites.value = favorites.value.filter((f) => f !== dir)
  } catch (e) {
    console.error('取消收藏失败，重拉列表', e)
    await loadLists()
  }
}

/**
 * 从最近列表移除（右键菜单）。后端无独立 remove_recent 命令（对齐 GPUI
 * RemoveContextDir 直接改配置），经 getAppConfig/setAppConfig 持久化；
 * 本地先乐观移除保证列表即时刷新（mock 层 listRecent 不读配置，仅本地生效）。
 */
async function removeRecentDir(dir: string) {
  recents.value = recents.value.filter((r) => r !== dir)
  try {
    const cfg = await getAppConfig()
    await setAppConfig({
      ...cfg,
      recentDirectories: (cfg.recentDirectories ?? []).filter((r) => r !== dir),
    })
  } catch (e) {
    console.error('从最近移除失败，重拉列表', e)
    await loadLists()
  }
}

/**
 * 文件夹卡片右键菜单（对齐 GPUI folder_menu + 打开）：
 * 打开 / 加入收藏|取消收藏（按当前收藏态切换）/ 从最近移除（该目录在最近列表时显示）。
 */
function onFolderContextMenu(dir: string, e: MouseEvent) {
  const items: ContextMenuItem[] = [
    { kind: 'item', label: '打开', action: () => void openPath(dir) },
    {
      kind: 'item',
      label: favorites.value.includes(dir) ? '取消收藏' : '加入收藏',
      action: () => void toggleFavFor(dir),
    },
  ]
  if (recents.value.includes(dir)) {
    items.push(
      { kind: 'sep' },
      { kind: 'item', label: '从最近移除', action: () => void removeRecentDir(dir) },
    )
  }
  contextMenu.openMenu(items, e.clientX, e.clientY)
}

/** 当前目录卡片右键：收藏切换（对齐顶部「收藏当前目录」按钮语义） */
function onCurrentDirContextMenu(e: MouseEvent) {
  const dir = captures.directory
  if (!dir) return
  contextMenu.openMenu(
    [
      {
        kind: 'item',
        label: favorites.value.includes(dir) ? '取消收藏' : '加入收藏',
        action: () => void toggleFavFor(dir),
      },
    ],
    e.clientX,
    e.clientY,
  )
}

/** 单击收藏/最近卡片：直接打开该目录（复用 captures.openPath） */
async function openPath(dir: string) {
  await captures.openPath(dir)
  // 打开后重拉最近列表（真实后端扫描时会更新 recentDirectories）
  try {
    recents.value = await listRecent()
  } catch (e) {
    console.error('listRecent 失败', e)
  }
}

// ── 子目录树（当前目录卡片下，一层懒加载；点击目录 = 走现有 openPath 扫描流程） ──

/** 当前目录的一层子目录（null = 尚未展开/加载） */
const subdirs = ref<SubdirInfo[] | null>(null)
/** 树是否展开 */
const subdirExpanded = ref(false)
const subdirLoading = ref(false)
const subdirError = ref('')

/** 懒加载当前目录的一层子目录（list_subdirs；更深层在切换目录后再展开） */
async function loadSubdirs() {
  const dir = captures.directory
  if (!dir) return
  subdirLoading.value = true
  subdirError.value = ''
  try {
    subdirs.value = await listSubdirs(dir)
  } catch (e) {
    subdirError.value = '子目录加载失败'
    console.error('listSubdirs 失败', e)
  } finally {
    subdirLoading.value = false
  }
}

/** 展开/收起箭头：首次展开才请求（懒加载）；收起不清缓存，再展开直接渲染 */
async function toggleSubdirs() {
  if (!captures.directory) return
  subdirExpanded.value = !subdirExpanded.value
  if (subdirExpanded.value && subdirs.value === null) await loadSubdirs()
}

// 目录切换后重置树（子目录树属于当前目录；点击子目录 → openPath → directory 变化触发）
watch(
  () => captures.directory,
  () => {
    subdirExpanded.value = false
    subdirs.value = null
    subdirError.value = ''
  },
)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <!-- 操作区：导入（对话框内含「导入 SD 卡 / 添加目录直接打开」双选项）+ 收藏当前目录（无目录时隐藏） -->
    <div class="flex flex-col gap-1.5 border-b px-3 py-2">
      <Button size="sm" @click="importDialog.open = true">
        <ImportIcon data-icon="inline-start" />
        导入
      </Button>
      <Button v-if="captures.directory" size="sm" variant="ghost" @click="toggleFavorite">
        <StarIcon data-icon="inline-start" :class="isFav ? 'fill-current text-primary' : ''" />
        {{ isFav ? '取消收藏' : '收藏当前目录' }}
      </Button>
    </div>

    <!-- 滚动列表区 -->
    <div class="min-h-0 flex-1 overflow-y-auto px-3 py-2">
      <!-- 当前目录卡片：图标瓦片 + 目录名 + 照片计数（对齐 GPUI sidebar 目录行；右键收藏切换） -->
      <div
        class="dir-card-active mb-2 flex items-center gap-2 px-2.5 py-2"
        @contextmenu.prevent="onCurrentDirContextMenu($event)"
      >
        <div
          class="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary"
        >
          <FolderOpenIcon class="size-4" />
        </div>
        <div class="min-w-0 flex-1">
          <div
            v-if="captures.directory"
            class="truncate text-[13px] font-medium"
            :title="captures.directory"
          >
            {{ dirName(captures.directory) }}
          </div>
          <div v-else class="text-[13px] text-muted-foreground">未打开目录</div>
          <div
            v-if="captures.directory"
            class="truncate text-[11px] text-muted-foreground tabular-nums"
          >
            {{ captures.count }} 张
          </div>
        </div>
      </div>

      <!-- 子目录树：当前目录的一层子目录，箭头懒加载（list_subdirs）；点击行 = 走现有扫描流程 -->
      <div v-if="captures.directory" class="mb-2">
        <div
          class="flex cursor-pointer select-none items-center gap-1 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-accent"
          role="button"
          :aria-expanded="subdirExpanded"
          @click="toggleSubdirs"
        >
          <ChevronRightIcon
            class="size-3 shrink-0 transition-transform"
            :class="subdirExpanded ? 'rotate-90' : ''"
          />
          <FolderIcon class="size-3 shrink-0" />
          <span>子目录</span>
          <span v-if="subdirs" class="tabular-nums">({{ subdirs.length }})</span>
        </div>
        <div
          v-if="subdirExpanded"
          class="mt-0.5 ml-2 space-y-0.5 border-l border-border pl-1.5"
        >
          <div v-if="subdirLoading" class="px-2 py-0.5 text-xs text-muted-foreground/70">
            加载中…
          </div>
          <div v-else-if="subdirError" class="px-2 py-0.5 text-xs text-destructive">
            {{ subdirError }}
          </div>
          <div
            v-else-if="subdirs && subdirs.length === 0"
            class="px-2 py-0.5 text-xs text-muted-foreground/70"
          >
            无子目录
          </div>
          <div
            v-for="s in subdirs ?? []"
            :key="s.path"
            class="group flex cursor-pointer items-center gap-1.5 rounded-md px-2 py-1 transition-colors hover:bg-element-hover"
            :title="s.path"
            @click="openPath(s.path)"
          >
            <span class="min-w-0 flex-1 truncate text-xs">{{ s.name }}</span>
            <span class="tabular-nums text-[0.625rem] text-muted-foreground">
              {{ s.photoCount }} 张
            </span>
          </div>
        </div>
      </div>

      <!-- 收藏分区（分隔线 + 分区标题 + 轻列表行） -->
      <div class="mt-3 border-t border-border pt-3">
        <div class="section-header mb-1.5 flex items-center gap-1">
          <StarIcon class="size-3" />
          收藏
        </div>
        <div v-if="favorites.length === 0" class="text-xs text-muted-foreground">
          点「收藏当前目录」加入
        </div>
        <div
          v-for="dir in favorites"
          :key="dir"
          class="group mb-0.5 flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-element-hover"
          :title="dir"
          @click="openPath(dir)"
          @contextmenu.prevent="onFolderContextMenu(dir, $event)"
        >
          <div class="min-w-0 flex-1">
            <div class="truncate text-xs font-medium">{{ dirName(dir) }}</div>
            <div class="truncate text-[0.625rem] text-muted-foreground tabular-nums">{{ dir }}</div>
          </div>
          <!-- 移除按钮（悬浮显现） -->
          <button
            class="shrink-0 rounded-full p-1 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-element-active hover:text-foreground"
            title="移除收藏"
            @click.stop="removeFav(dir)"
          >
            <XIcon class="size-3" />
          </button>
        </div>
      </div>

      <!-- 最近打开分区（分隔线 + 分区标题 + panel-card 文件夹行） -->
      <!-- 最近打开分区（分隔线 + 分区标题 + 轻列表行） -->
      <div class="mt-3 border-t border-border pt-3">
        <div class="mb-1.5 flex items-center gap-1 text-[11px] font-semibold text-muted-foreground">
          <ClockIcon class="size-3" />
          最近打开
        </div>
        <div v-if="recents.length === 0" class="text-xs text-muted-foreground">
          暂无历史记录
        </div>
        <div
          v-for="dir in recents"
          :key="dir"
          class="group mb-0.5 flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 transition-colors hover:bg-element-hover"
          :title="dir"
          @click="openPath(dir)"
          @contextmenu.prevent="onFolderContextMenu(dir, $event)"
        >
          <div class="min-w-0 flex-1">
            <div class="truncate text-xs font-medium">{{ dirName(dir) }}</div>
            <div class="truncate text-[0.625rem] text-muted-foreground tabular-nums">{{ dir }}</div>
          </div>
        </div>
      </div>
    </div>

    <!-- 底部工具区：重复照片检测（pHash 近重复分组；打开面板后逐张哈希 → 分组列表） -->
    <div class="shrink-0 border-t px-3 py-2">
      <Button size="sm" variant="ghost" class="w-full" @click="duplicates.openPanel()">
        <CopyIcon data-icon="inline-start" />
        重复照片
      </Button>
    </div>
  </div>
</template>
