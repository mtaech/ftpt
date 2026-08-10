<script setup lang="ts">
// 左侧栏：打开目录 / 收藏当前目录 / 当前目录卡片 / 收藏列表 / 最近打开列表。
// 收藏与最近经 '@/lib/ipc' 命令读取（mock 模式走内存态）；右缘把手可拖宽，宽度持久化 localStorage。
import { computed, onMounted, ref } from 'vue'
import { useStorage } from '@vueuse/core'
import { ClockIcon, FolderOpenIcon, StarIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useCapturesStore } from '@/stores/captures'
import { useContextMenuStore, type ContextMenuItem } from '@/stores/contextMenu'
import {
  addFavorite,
  getAppConfig,
  listFavorites,
  listRecent,
  removeFavorite,
  setAppConfig,
} from '@/lib/ipc'

const captures = useCapturesStore()
const contextMenu = useContextMenuStore()

// ── 宽度：可拖拽，localStorage 持久化，范围 200–480（对齐 GPUI 左栏 size_range）──
const width = useStorage('ftpt.leftPanelWidth', 180)
const clampedWidth = computed(() => Math.min(480, Math.max(200, width.value)))
/** 拖拽起始状态（指针捕获在把手上，move/up 仍持续收到） */
let dragStartX = 0
let dragStartW = 0
function onHandleDown(e: PointerEvent) {
  e.preventDefault()
  dragStartX = e.clientX
  dragStartW = width.value
  ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
}
function onHandleMove(e: PointerEvent) {
  if (e.buttons === 0) return
  width.value = Math.min(480, Math.max(200, dragStartW + (e.clientX - dragStartX)))
}
/** 拖拽结束：指针捕获在 pointerup 后由浏览器自动释放，无需清理 */
function onHandleUp() {}

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
</script>

<template>
  <aside
    class="relative flex h-full shrink-0 flex-col border-r bg-sidebar"
    :style="{ width: `${clampedWidth}px` }"
  >
    <!-- 操作区：打开目录 + 收藏当前目录 -->
    <div class="flex flex-col gap-1.5 border-b p-2">
      <Button
        size="sm"
        variant="secondary"
        :disabled="captures.scanning"
        @click="captures.openDirectory()"
      >
        <FolderOpenIcon data-icon="inline-start" />
        打开目录
      </Button>
      <Button v-if="captures.directory" size="sm" variant="ghost" @click="toggleFavorite">
        <StarIcon data-icon="inline-start" :class="isFav ? 'fill-current text-primary' : ''" />
        {{ isFav ? '取消收藏' : '收藏当前目录' }}
      </Button>
    </div>

    <!-- 滚动列表区 -->
    <div class="min-h-0 flex-1 overflow-y-auto p-2">
      <!-- 当前目录卡片：目录名 + 照片计数（对齐 GPUI sidebar 目录行；右键收藏切换） -->
      <div
        class="mb-2 flex items-center gap-1 rounded-md border bg-card px-2 py-1.5"
        @contextmenu.prevent="onCurrentDirContextMenu($event)"
      >
        <div class="min-w-0 flex-1">
          <div v-if="captures.directory" class="truncate text-sm" :title="captures.directory">
            {{ dirName(captures.directory) }}
          </div>
          <div v-else class="text-sm text-muted-foreground">未打开目录</div>
          <div
            v-if="captures.directory"
            class="truncate text-[10px] text-muted-foreground font-mono-num"
          >
            {{ captures.count }} 张
          </div>
        </div>
      </div>

      <!-- 收藏列表 -->
      <div class="px-1 pb-1 text-[11px] font-medium text-muted-foreground">收藏</div>
      <div v-if="favorites.length === 0" class="px-1 pb-1 text-[11px] text-muted-foreground/70">
        点「收藏当前目录」加入
      </div>
      <div
        v-for="dir in favorites"
        :key="dir"
        class="group mb-0.5 flex cursor-pointer items-center gap-1 rounded-md border border-transparent px-2 py-1 hover:border-border hover:bg-accent"
        :title="dir"
        @click="openPath(dir)"
        @contextmenu.prevent="onFolderContextMenu(dir, $event)"
      >
        <StarIcon class="size-3 shrink-0 text-primary" />
        <div class="min-w-0 flex-1">
          <div class="truncate text-xs">{{ dirName(dir) }}</div>
          <div class="truncate text-[10px] text-muted-foreground font-mono-num">{{ dir }}</div>
        </div>
        <!-- 移除按钮（悬浮显现） -->
        <button
          class="shrink-0 rounded p-0.5 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted hover:text-foreground"
          title="移除收藏"
          @click.stop="removeFav(dir)"
        >
          <XIcon class="size-3" />
        </button>
      </div>

      <!-- 最近打开列表 -->
      <div class="mt-3 flex items-center gap-1 px-1 pb-1 text-[11px] font-medium text-muted-foreground">
        <ClockIcon class="size-3" />
        最近打开
      </div>
      <div v-if="recents.length === 0" class="px-1 pb-1 text-[11px] text-muted-foreground/70">
        暂无历史记录
      </div>
      <div
        v-for="dir in recents"
        :key="dir"
        class="group mb-0.5 flex cursor-pointer items-center gap-1 rounded-md border border-transparent px-2 py-1 hover:border-border hover:bg-accent"
        :title="dir"
        @click="openPath(dir)"
        @contextmenu.prevent="onFolderContextMenu(dir, $event)"
      >
        <FolderOpenIcon class="size-3 shrink-0 text-muted-foreground" />
        <div class="min-w-0 flex-1">
          <div class="truncate text-xs">{{ dirName(dir) }}</div>
          <div class="truncate text-[10px] text-muted-foreground font-mono-num">{{ dir }}</div>
        </div>
      </div>
    </div>

    <!-- 拖宽把手（右缘，指针捕获保证拖出侧栏仍生效） -->
    <div
      class="absolute inset-y-0 right-0 w-1 cursor-col-resize touch-none select-none hover:bg-primary/40"
      @pointerdown="onHandleDown"
      @pointermove="onHandleMove"
      @pointerup="onHandleUp"
      @pointercancel="onHandleUp"
    />
  </aside>
</template>
